//! Conflict detection and the staged resolution file-operations.
//!
//! When both sides change, the engine keeps the local file and writes the remote copy beside it as
//! a sidecar. When the remote copy was *deleted* while the local file was edited, there is nothing
//! to download, so the sidecar is a copy of the local file instead — same name, same resolutions,
//! and the same reason it exists: without a sidecar that state has no exit and never reaches this
//! scanner. Resolution needs **no IPC verb** — it is ordinary file work the GUI performs on
//! disk, after which the daemon reconciles from the resulting on-disk state.
//!
//! The sidecar name has two forms and a correct scanner must match **both**:
//! `{stem}.{suffix}.{ext}` (files with an extension) and the extensionless `{name}.{suffix}`
//! (dotfiles / no extension). We reuse the engine's own [`ConflictNaming`] so detection can never
//! disagree with what the daemon wrote — **including which suffix it wrote**, which
//! `conflict_suffix` makes configurable. The caller passes the naming read from the same config
//! file the daemon runs on; a scanner holding a different suffix silently finds no conflicts.

use proton_drive_sync_engine::sync::ConflictNaming;
use std::path::{Path, PathBuf};

/// What kind of disagreement a sidecar records.
///
/// The conflicts screen has to tell these apart and **nothing downstream can work it out**. A type
/// conflict has no diff to show — `04-conflicts.md` requires the disclosure hidden and different
/// card copy — but by the time the UI holds a [`Conflict`] the distinction is gone:
/// [`read_conflict_pair`] reads the original with `fs::read`, a directory answers `EISDIR`, and that
/// lands in the same `binary_or_large: true` arm as a JPEG. The screen would have had to choose
/// between showing a diff disclosure over a folder and hiding it from every binary file.
///
/// The scanner is the one place that knows, because it is already standing at the path with an
/// `original_abs` in hand — so this costs one `symlink_metadata` per conflict, not a command.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Both sides are files: the ordinary case, and the only one with a diff to disclose.
    Content,
    /// A **folder** here and a **file** on Proton Drive — `photos/trip` in the frames. The engine
    /// downloads the remote file beside the local directory, so a sidecar exists for these too.
    Type,
}

/// One detected, unresolved conflict. Both paths are **relative to the local root**.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Conflict {
    /// The user's local file the sidecar sits beside (relative to the local root).
    pub original: PathBuf,
    /// The `*.{suffix}[.ext]` sidecar the engine wrote (relative to the local root).
    pub sidecar: PathBuf,
    /// Whether this is a content disagreement or a type one. See [`ConflictKind`].
    ///
    /// `#[serde(default)]` so a reply written before this field existed still decodes — and the
    /// default is [`ConflictKind::Content`], which is both the common case and the safe one: it
    /// shows a disclosure that may be empty rather than hiding one that had something to say.
    #[serde(default = "content_kind")]
    pub kind: ConflictKind,
}

fn content_kind() -> ConflictKind {
    ConflictKind::Content
}

/// The four resolution choices. All are staged in the UI and applied together; nothing touches
/// disk until [`apply_resolution`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Delete the sidecar; the local file uploads on the next pass.
    KeepMine,
    /// Move the sidecar over the original, replacing it.
    UseProton,
    /// Rename the local file to `name.local.ext`, then move the sidecar into place. Both survive.
    KeepBoth,
    /// Do nothing on disk; still counts as outstanding everywhere in the UI.
    DecideLater,
}

/// Scan `local_root` for unresolved conflict sidecars (both name forms), skipping the top-level
/// `.sync/` state directory and not following symlinked directories. Results are sorted for a
/// deterministic order.
pub fn scan_conflicts(
    local_root: &Path,
    naming: &ConflictNaming,
) -> std::io::Result<Vec<Conflict>> {
    let mut out = Vec::new();
    walk(local_root, local_root, naming, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    naming: &ConflictNaming,
    out: &mut Vec<Conflict>,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // A directory that vanished or is unreadable mid-scan is skipped, not fatal.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Ok(()),
        Err(e) => return Err(e),
    };

    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();

        if file_type.is_dir() {
            // Skip only the top-level `.sync/` state dir (the engine treats just the top-level
            // one as state); recurse everywhere else. Symlinked dirs report `is_dir() == false`,
            // so this loop never follows them.
            if dir == root && entry.file_name() == std::ffi::OsStr::new(".sync") {
                continue;
            }
            walk(root, &path, naming, out)?;
        } else if file_type.is_file() && naming.is_conflict_copy(&path) {
            let sidecar = relative(root, &path);
            if let Some(original_abs) = naming.original_from_conflict_copy(&path) {
                // The one moment the kind is knowable. `symlink_metadata` rather than `metadata`
                // for the same reason the walk above uses `file_type()`: a symlink is classified as
                // itself, never as whatever it points at. An unreadable original falls back to
                // `Content` — the safe default, since it shows a disclosure that may be empty
                // rather than hiding one that had something to say.
                let kind = match std::fs::symlink_metadata(&original_abs) {
                    Ok(meta) if meta.is_dir() => ConflictKind::Type,
                    _ => ConflictKind::Content,
                };
                out.push(Conflict {
                    original: relative(root, &original_abs),
                    sidecar,
                    kind,
                });
            }
        }
    }
    Ok(())
}

fn relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Apply a single resolution as file operations under `local_root`. Called when the user hits
/// Apply on the staged set. [`Resolution::DecideLater`] is a no-op.
pub fn apply_resolution(
    local_root: &Path,
    conflict: &Conflict,
    choice: Resolution,
) -> std::io::Result<()> {
    let sidecar = local_root.join(&conflict.sidecar);
    let original = local_root.join(&conflict.original);
    match choice {
        Resolution::KeepMine => std::fs::remove_file(&sidecar),
        Resolution::UseProton => std::fs::rename(&sidecar, &original),
        Resolution::KeepBoth => {
            let local_copy = local_sibling_path(&original);
            std::fs::rename(&original, &local_copy)?;
            std::fs::rename(&sidecar, &original)
        }
        Resolution::DecideLater => Ok(()),
    }
}

/// `notes.txt` → `notes.local.txt`; `README` → `README.local`; `.env` → `.env.local`.
///
/// The unix path is byte-safe (no UTF-8 assumption), mirroring the engine's own extension
/// semantics for dotfiles. A non-unix fallback keeps the crate type-checking off-unix (consistent
/// with the `cfg(not(unix))` stubs elsewhere), accepting lossy handling of non-UTF-8 names there.
#[cfg(unix)]
fn local_sibling_path(original: &Path) -> PathBuf {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let Some(file_name) = original.file_name() else {
        return original.to_path_buf();
    };
    let as_path = Path::new(file_name);
    let stem = as_path.file_stem().unwrap_or(file_name);
    let mut bytes = stem.as_bytes().to_vec();
    bytes.extend_from_slice(b".local");
    if let Some(extension) = as_path.extension() {
        bytes.push(b'.');
        bytes.extend_from_slice(extension.as_bytes());
    }
    original.with_file_name(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn local_sibling_path(original: &Path) -> PathBuf {
    let Some(file_name) = original.file_name() else {
        return original.to_path_buf();
    };
    let as_path = Path::new(file_name);
    let stem = as_path
        .file_stem()
        .unwrap_or(file_name)
        .to_string_lossy()
        .into_owned();
    let renamed = match as_path.extension() {
        Some(extension) => format!("{stem}.local.{}", extension.to_string_lossy()),
        None => format!("{stem}.local"),
    };
    original.with_file_name(renamed)
}

/// One side of a conflict for the compare view.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConflictSide {
    pub exists: bool,
    pub size: u64,
    pub mtime_epoch_secs: Option<i64>,
    /// UTF-8 text, only for small files that look like text; `None` for binary/large/missing.
    pub text: Option<String>,
    /// `true` when the file is binary or too large to diff — the UI shows size + time only,
    /// never a fabricated preview (design §3.3).
    pub binary_or_large: bool,
}

/// One digest per line of the last agreed version, re-exported so a caller needs no direct engine
/// dependency for the one type it has to carry between the index read and this reader.
pub use proton_drive_sync_engine::ancestor::LineSummary;

/// What one side did to the last agreed version (#217) — the three counts the card's first line is
/// built from, and nothing else. The words are the webview's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SideChange {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

impl From<proton_drive_sync_engine::ancestor::VersionFacts> for SideChange {
    fn from(facts: proton_drive_sync_engine::ancestor::VersionFacts) -> Self {
        Self {
            added: facts.added,
            changed: facts.changed,
            removed: facts.removed,
        }
    }
}

/// Both sides of a conflict: the user's local `original` and the `sidecar` (Proton's copy).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConflictPair {
    pub original: ConflictSide,
    pub sidecar: ConflictSide,
    /// How each side moved from the version they last agreed on.
    ///
    /// **`None` is the ordinary answer and the card must draw one line fewer for it** (#347). It
    /// means there is no ancestor to compare against — the file was never summarised (binary, or
    /// past the engine's caps), the summary aged out, one side is unreadable, or the diff was too
    /// far apart. Every one of those is a case where the drawn sentence `You added a line` was
    /// being invented, which is what this replaces.
    pub happened: Option<PairChange>,
}

/// The pair of side changes, present only when both could be computed.
///
/// Both or neither, deliberately: one side's verb beside the other side's silence reads as "and
/// Proton did nothing", which is a claim about Proton's copy that nothing here checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PairChange {
    pub mine: SideChange,
    pub theirs: SideChange,
}

/// Files above this size are shown as metadata only (no diff).
const MAX_DIFF_BYTES: u64 = 512 * 1024;

/// Read both sides of a conflict for the compare view. **Path-safe**: each relative path is passed
/// through the engine's `validate_relative_path` guard (rejecting `..`/absolute/prefix components)
/// before being joined onto `local_root`. Text is returned only for small, valid-UTF-8, NUL-free
/// files; everything else comes back as metadata with `binary_or_large = true`.
pub fn read_conflict_pair(
    local_root: &Path,
    conflict: &Conflict,
    ancestor: Option<&LineSummary>,
) -> Result<ConflictPair, String> {
    let original = read_side(local_root, &conflict.original)?;
    let sidecar = read_side(local_root, &conflict.sidecar)?;
    let happened = ancestor.and_then(|ancestor| pair_change(ancestor, &original, &sidecar));
    Ok(ConflictPair {
        original,
        sidecar,
        happened,
    })
}

/// Compare both live sides against the agreed version, or answer `None`.
///
/// Reads the text `read_side` already returned rather than the files again: two reads of a file the
/// user may be editing can disagree, and the sentence has to describe the same bytes the panel
/// beneath it draws.
fn pair_change(
    ancestor: &LineSummary,
    original: &ConflictSide,
    sidecar: &ConflictSide,
) -> Option<PairChange> {
    use proton_drive_sync_engine::ancestor::compare_to_ancestor;
    let mine_text = original.text.as_deref()?;
    let theirs_text = sidecar.text.as_deref()?;

    // A SIDECAR CAN BE A COPY OF THE LOCAL FILE, and then nothing here knows anything about
    // Proton's copy. `sync.rs` writes one for `(Changed, Missing)` — a local edit whose remote copy
    // was deleted elsewhere (#46) — because a conflict with no sidecar has no exit. The GUI's disk
    // walk cannot tell that sidecar from a downloaded one, so comparing it as "Proton's version"
    // reports the user's OWN edit as something Proton did. Identical bytes are the case this can
    // see, and it is the state such a conflict is recorded in.
    //
    // The residual is honest and filed rather than papered over: if the local file is edited AFTER
    // such a sidecar is written, the two differ and this test passes, and `theirs` then describes
    // the user's earlier edit. Telling them apart needs provenance the wire does not carry — the
    // daemon knows (`PlannedAction::sidecar_from_local_copy`) and nothing records it.
    if mine_text == theirs_text {
        return None;
    }

    let mine = LineSummary::of(mine_text)?;
    let theirs = LineSummary::of(theirs_text)?;
    Some(PairChange {
        mine: compare_to_ancestor(ancestor, &mine)?.into(),
        theirs: compare_to_ancestor(ancestor, &theirs)?.into(),
    })
}

fn read_side(local_root: &Path, relative: &Path) -> Result<ConflictSide, String> {
    let safe = proton_drive_sync_engine::validate_relative_path(relative)
        .ok_or_else(|| format!("unsafe conflict path: {}", relative.display()))?;
    let full = local_root.join(safe);

    let meta = match std::fs::metadata(&full) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConflictSide {
                exists: false,
                size: 0,
                mtime_epoch_secs: None,
                text: None,
                binary_or_large: false,
            });
        }
        Err(e) => return Err(e.to_string()),
    };
    let size = meta.len();
    let mtime_epoch_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    if size > MAX_DIFF_BYTES {
        return Ok(ConflictSide {
            exists: true,
            size,
            mtime_epoch_secs,
            text: None,
            binary_or_large: true,
        });
    }
    let bytes = match std::fs::read(&full) {
        Ok(bytes) => bytes,
        // The file vanished between `metadata()` and `read()` — treat as missing, not an error, so
        // the whole pair doesn't fail over one racy side.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConflictSide {
                exists: false,
                size: 0,
                mtime_epoch_secs: None,
                text: None,
                binary_or_large: false,
            });
        }
        // Unreadable for another reason (e.g. permissions) — show metadata only rather than failing.
        Err(_) => {
            return Ok(ConflictSide {
                exists: true,
                size,
                mtime_epoch_secs,
                text: None,
                binary_or_large: true,
            });
        }
    };
    // Binary heuristic: invalid UTF-8, or an embedded NUL (common in binary formats).
    match String::from_utf8(bytes) {
        Ok(text) if !text.contains('\u{0}') => Ok(ConflictSide {
            exists: true,
            size,
            mtime_epoch_secs,
            text: Some(text),
            binary_or_large: false,
        }),
        _ => Ok(ConflictSide {
            exists: true,
            size,
            mtime_epoch_secs,
            text: None,
            binary_or_large: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn the_walker_finds_the_configured_suffix_and_only_that_one() {
        // The GUI's conflicts list is a DISK WALK, so it is the one consumer that can silently
        // disagree with the daemon about what a sidecar is called: a scanner holding the default
        // while `conflict_suffix` says otherwise reports "no conflicts" on a folder full of them,
        // and reports the user's own `.proton-cloud`-named files as conflicts on top. Both halves
        // are asserted here because a naming threaded to only one of them passes the other.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.from-cloud.txt"), "theirs");
        write(&root.join("legacy.txt"), "mine");
        write(
            &root.join("legacy.proton-cloud.txt"),
            "an ordinary file under this suffix",
        );

        let naming = ConflictNaming::new("from-cloud").unwrap();
        let conflicts = scan_conflicts(root, &naming).unwrap();

        assert_eq!(
            conflicts,
            vec![Conflict {
                original: "notes.txt".into(),
                sidecar: "notes.from-cloud.txt".into(),
                kind: ConflictKind::Content,
            }],
            "only the configured suffix names a conflict"
        );
    }

    #[test]
    fn scan_finds_both_sidecar_forms_and_skips_dot_sync() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs"); // extension form
        write(&root.join("README"), "mine");
        write(&root.join("README.proton-cloud"), "theirs"); // extensionless form
        write(&root.join("sub/a.md"), "mine");
        write(&root.join("sub/a.proton-cloud.md"), "theirs"); // nested
        write(
            &root.join(".sync/sync_index.proton-cloud.db"),
            "not a conflict",
        ); // must be ignored

        let conflicts = scan_conflicts(root, &ConflictNaming::default()).unwrap();
        let originals: Vec<_> = conflicts.iter().map(|c| c.original.clone()).collect();
        assert!(originals.contains(&PathBuf::from("notes.txt")));
        assert!(originals.contains(&PathBuf::from("README")));
        assert!(originals.contains(&PathBuf::from("sub/a.md")));
        assert_eq!(
            conflicts.len(),
            3,
            "the .sync/ entry must be skipped: {conflicts:?}"
        );
    }

    #[test]
    fn a_folder_here_and_a_file_on_proton_is_a_type_conflict() {
        // `04-conflicts.md` hides the diff disclosure for these and uses different card copy, and
        // this is the only moment the distinction is visible: `read_conflict_pair` reads the
        // original with `fs::read`, a directory answers EISDIR, and that lands in the same
        // `binary_or_large: true` arm as a JPEG. Without a kind, S2 must either draw a diff
        // disclosure over a folder or hide it from every binary file.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::create_dir_all(root.join("photos/trip")).unwrap();
        write(&root.join("photos/trip.proton-cloud"), "the remote file");
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs");

        let conflicts = scan_conflicts(root, &ConflictNaming::default()).unwrap();
        let kind_of = |p: &str| {
            conflicts
                .iter()
                .find(|c| c.original == Path::new(p))
                .unwrap_or_else(|| panic!("{p} not found in {conflicts:?}"))
                .kind
        };
        assert_eq!(kind_of("photos/trip"), ConflictKind::Type);
        assert_eq!(kind_of("notes.txt"), ConflictKind::Content);
    }

    #[test]
    fn a_sidecar_whose_original_has_vanished_reads_as_a_content_conflict() {
        // The safe default: a disclosure that may be empty beats one hidden from a file that had
        // something to say. (Reachable — the original can be removed between the write and the
        // scan.)
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("gone.proton-cloud.txt"), "theirs");

        let conflicts = scan_conflicts(root, &ConflictNaming::default()).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::Content);
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_to_a_directory_is_not_a_type_conflict() {
        // `symlink_metadata`, so the link is classified as itself — the same rule the walk uses for
        // `file_type()`. Following it would call a link to a folder a folder.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        write(&root.join("link.proton-cloud"), "theirs");

        let conflicts = scan_conflicts(root, &ConflictNaming::default()).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::Content);
    }

    #[test]
    fn a_reply_written_before_kind_existed_still_decodes() {
        // `scan_conflicts` is a Tauri command reply, and the frontend and backend are versioned
        // together — but the fixtures are hand-written JSON and the mock is hand-written JS, so a
        // missing field must mean the common case rather than a decode failure.
        let decoded: Conflict =
            serde_json::from_str(r#"{"original":"a.txt","sidecar":"a.proton-cloud.txt"}"#).unwrap();
        assert_eq!(decoded.kind, ConflictKind::Content);
    }

    #[test]
    fn keep_mine_deletes_only_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs");
        let conflict = Conflict {
            original: "notes.txt".into(),
            sidecar: "notes.proton-cloud.txt".into(),
            kind: ConflictKind::Content,
        };
        apply_resolution(root, &conflict, Resolution::KeepMine).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "mine"
        );
        assert!(!root.join("notes.proton-cloud.txt").exists());
    }

    #[test]
    fn use_proton_replaces_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs");
        let conflict = Conflict {
            original: "notes.txt".into(),
            sidecar: "notes.proton-cloud.txt".into(),
            kind: ConflictKind::Content,
        };
        apply_resolution(root, &conflict, Resolution::UseProton).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "theirs"
        );
        assert!(!root.join("notes.proton-cloud.txt").exists());
    }

    #[test]
    fn keep_both_preserves_local_as_dot_local_and_installs_remote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs");
        let conflict = Conflict {
            original: "notes.txt".into(),
            sidecar: "notes.proton-cloud.txt".into(),
            kind: ConflictKind::Content,
        };
        apply_resolution(root, &conflict, Resolution::KeepBoth).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("notes.local.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "theirs"
        );
        assert!(!root.join("notes.proton-cloud.txt").exists());
    }

    #[test]
    fn local_sibling_naming_matches_engine_extension_semantics() {
        assert_eq!(
            local_sibling_path(Path::new("d/notes.txt")),
            Path::new("d/notes.local.txt")
        );
        assert_eq!(
            local_sibling_path(Path::new("d/README")),
            Path::new("d/README.local")
        );
        assert_eq!(
            local_sibling_path(Path::new("d/.env")),
            Path::new("d/.env.local")
        );
        assert_eq!(
            local_sibling_path(Path::new("d/archive.tar.gz")),
            Path::new("d/archive.tar.local.gz")
        );
    }

    #[test]
    fn decide_later_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs");
        let conflict = Conflict {
            original: "notes.txt".into(),
            sidecar: "notes.proton-cloud.txt".into(),
            kind: ConflictKind::Content,
        };
        apply_resolution(root, &conflict, Resolution::DecideLater).unwrap();
        assert!(root.join("notes.txt").exists());
        assert!(root.join("notes.proton-cloud.txt").exists());
    }
}

#[cfg(test)]
mod pair_tests {
    use super::*;

    fn conflict() -> Conflict {
        Conflict {
            original: "notes.txt".into(),
            sidecar: "notes.proton-cloud.txt".into(),
            kind: ConflictKind::Content,
        }
    }

    #[test]
    fn reads_text_on_both_sides() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), "mine\nline2\n").unwrap();
        std::fs::write(root.join("notes.proton-cloud.txt"), "theirs\nline2\n").unwrap();
        let pair = read_conflict_pair(root, &conflict(), None).unwrap();
        assert_eq!(pair.original.text.as_deref(), Some("mine\nline2\n"));
        assert_eq!(pair.sidecar.text.as_deref(), Some("theirs\nline2\n"));
        assert!(!pair.original.binary_or_large && pair.original.exists);
    }

    #[test]
    fn the_two_verbs_are_computed_against_the_agreed_version() {
        // #217/#347. THE POINT OF THE WHOLE FEATURE: the sentence is a claim against the version
        // both sides last agreed on, and against the other live copy alone the very same edit reads
        // as a removal. These are `3a Conflict diff`'s own drawn lines with the one ancestor that
        // reconciles the two frames.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("notes.txt"),
            "# Todo\n- buy milk\n- call Alice\n- ship v1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("notes.proton-cloud.txt"),
            "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n",
        )
        .unwrap();
        let ancestor =
            LineSummary::of("# Todo\n- buy oat milk\n- call Alice\n- ship v1\n").unwrap();

        let pair = read_conflict_pair(root, &conflict(), Some(&ancestor)).unwrap();
        let happened = pair.happened.expect("both sides compare to the ancestor");
        assert_eq!(
            happened.mine,
            SideChange {
                added: 0,
                changed: 1,
                removed: 0
            }
        );
        assert_eq!(
            happened.theirs,
            SideChange {
                added: 1,
                changed: 0,
                removed: 0
            }
        );
    }

    #[test]
    fn no_ancestor_and_an_unreadable_side_both_say_less_rather_than_guessing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), "a\nb\n").unwrap();
        std::fs::write(root.join("notes.proton-cloud.txt"), "a\nc\n").unwrap();

        // No ancestor at all — a file this daemon never summarised, or one whose summary aged out.
        assert!(
            read_conflict_pair(root, &conflict(), None)
                .unwrap()
                .happened
                .is_none()
        );

        // An ancestor, but one side is binary: BOTH or NEITHER, because one side's verb beside the
        // other's silence reads as "and Proton did nothing", which is a claim nothing checked.
        std::fs::write(root.join("notes.proton-cloud.txt"), [0u8, 1, 2, 3, 0]).unwrap();
        let ancestor = LineSummary::of("a\nb\n").unwrap();
        assert!(
            read_conflict_pair(root, &conflict(), Some(&ancestor))
                .unwrap()
                .happened
                .is_none()
        );
    }

    #[test]
    fn a_sidecar_that_is_a_copy_of_the_local_file_attributes_nothing_to_proton() {
        // FOUND BY ADVERSARIAL REVIEW. `sync.rs` writes the sidecar as a byte copy of the surviving
        // local file when the remote node is confirmed gone (#46), and the disk walk cannot tell
        // that from a downloaded one — so both sides compared against the ancestor reported the
        // user's own edit twice, once attributed to Proton. Proton's copy was DELETED; the card
        // said it had been edited, which is an affirmative claim about the other side that nothing
        // checked.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let edited = "# Todo\n- buy milk\n- call Alice\n";
        std::fs::write(root.join("notes.txt"), edited).unwrap();
        std::fs::write(root.join("notes.proton-cloud.txt"), edited).unwrap();
        let ancestor = LineSummary::of("# Todo\n- buy oat milk\n- call Alice\n").unwrap();

        assert!(
            read_conflict_pair(root, &conflict(), Some(&ancestor))
                .unwrap()
                .happened
                .is_none(),
            "identical sides say nothing about what the other side did"
        );
    }

    #[test]
    fn binary_side_returns_metadata_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), b"ok").unwrap();
        std::fs::write(root.join("notes.proton-cloud.txt"), [0u8, 1, 2, 3, 0]).unwrap(); // NUL bytes
        let pair = read_conflict_pair(root, &conflict(), None).unwrap();
        assert_eq!(pair.sidecar.text, None);
        assert!(pair.sidecar.binary_or_large && pair.sidecar.exists);
        assert_eq!(pair.sidecar.size, 5);
    }

    #[test]
    fn large_side_returns_metadata_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), "small").unwrap();
        std::fs::write(
            root.join("notes.proton-cloud.txt"),
            vec![b'a'; (MAX_DIFF_BYTES + 1) as usize],
        )
        .unwrap();
        let pair = read_conflict_pair(root, &conflict(), None).unwrap();
        assert!(pair.sidecar.binary_or_large && pair.sidecar.text.is_none());
    }

    #[test]
    fn missing_side_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), "mine").unwrap();
        let pair = read_conflict_pair(root, &conflict(), None).unwrap();
        assert!(pair.original.exists);
        assert!(!pair.sidecar.exists && pair.sidecar.text.is_none());
    }

    #[test]
    fn path_traversal_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let evil = Conflict {
            original: "../../etc/passwd".into(),
            sidecar: "notes.proton-cloud.txt".into(),
            kind: ConflictKind::Content,
        };
        assert!(read_conflict_pair(dir.path(), &evil, None).is_err());
    }
}
