//! Conflict detection and the staged resolution file-operations.
//!
//! When both sides change, the engine keeps the local file and writes the remote copy beside it as
//! a sidecar. Resolution needs **no IPC verb** — it is ordinary file work the GUI performs on
//! disk, after which the daemon reconciles from the resulting on-disk state.
//!
//! The sidecar name has two forms and a correct scanner must match **both**:
//! `{stem}.proton-cloud.{ext}` (files with an extension) and the extensionless `{name}.proton-cloud`
//! (dotfiles / no extension). We reuse the engine's own [`is_conflict_copy`] /
//! [`original_from_conflict_copy`] so detection can never disagree with what the daemon wrote.

use proton_drive_sync_engine::sync::{is_conflict_copy, original_from_conflict_copy};
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
    /// The `*.proton-cloud[.ext]` sidecar the engine wrote (relative to the local root).
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
pub fn scan_conflicts(local_root: &Path) -> std::io::Result<Vec<Conflict>> {
    let mut out = Vec::new();
    walk(local_root, local_root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Conflict>) -> std::io::Result<()> {
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
            walk(root, &path, out)?;
        } else if file_type.is_file() && is_conflict_copy(&path) {
            let sidecar = relative(root, &path);
            if let Some(original_abs) = original_from_conflict_copy(&path) {
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

/// Both sides of a conflict: the user's local `original` and the `sidecar` (Proton's copy).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConflictPair {
    pub original: ConflictSide,
    pub sidecar: ConflictSide,
}

/// Files above this size are shown as metadata only (no diff).
const MAX_DIFF_BYTES: u64 = 512 * 1024;

/// Read both sides of a conflict for the compare view. **Path-safe**: each relative path is passed
/// through the engine's `validate_relative_path` guard (rejecting `..`/absolute/prefix components)
/// before being joined onto `local_root`. Text is returned only for small, valid-UTF-8, NUL-free
/// files; everything else comes back as metadata with `binary_or_large = true`.
pub fn read_conflict_pair(local_root: &Path, conflict: &Conflict) -> Result<ConflictPair, String> {
    Ok(ConflictPair {
        original: read_side(local_root, &conflict.original)?,
        sidecar: read_side(local_root, &conflict.sidecar)?,
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

        let conflicts = scan_conflicts(root).unwrap();
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

        let conflicts = scan_conflicts(root).unwrap();
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

        let conflicts = scan_conflicts(root).unwrap();
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

        let conflicts = scan_conflicts(root).unwrap();
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
        let pair = read_conflict_pair(root, &conflict()).unwrap();
        assert_eq!(pair.original.text.as_deref(), Some("mine\nline2\n"));
        assert_eq!(pair.sidecar.text.as_deref(), Some("theirs\nline2\n"));
        assert!(!pair.original.binary_or_large && pair.original.exists);
    }

    #[test]
    fn binary_side_returns_metadata_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), b"ok").unwrap();
        std::fs::write(root.join("notes.proton-cloud.txt"), [0u8, 1, 2, 3, 0]).unwrap(); // NUL bytes
        let pair = read_conflict_pair(root, &conflict()).unwrap();
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
        let pair = read_conflict_pair(root, &conflict()).unwrap();
        assert!(pair.sidecar.binary_or_large && pair.sidecar.text.is_none());
    }

    #[test]
    fn missing_side_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), "mine").unwrap();
        let pair = read_conflict_pair(root, &conflict()).unwrap();
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
        assert!(read_conflict_pair(dir.path(), &evil).is_err());
    }
}
