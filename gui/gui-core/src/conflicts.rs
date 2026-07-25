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

/// One detected, unresolved conflict. Both paths are **relative to the local root**.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Conflict {
    /// The user's local file the sidecar sits beside (relative to the local root).
    pub original: PathBuf,
    /// The `*.proton-cloud[.ext]` sidecar the engine wrote (relative to the local root).
    pub sidecar: PathBuf,
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
                out.push(Conflict {
                    original: relative(root, &original_abs),
                    sidecar,
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

/// `notes.txt` → `notes.local.txt`; `README` → `README.local`; `.env` → `.env.local`. Byte-safe
/// (no UTF-8 assumption), mirroring the engine's own extension semantics for dotfiles.
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
    fn keep_mine_deletes_only_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("notes.txt"), "mine");
        write(&root.join("notes.proton-cloud.txt"), "theirs");
        let conflict = Conflict {
            original: "notes.txt".into(),
            sidecar: "notes.proton-cloud.txt".into(),
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
        };
        apply_resolution(root, &conflict, Resolution::DecideLater).unwrap();
        assert!(root.join("notes.txt").exists());
        assert!(root.join("notes.proton-cloud.txt").exists());
    }
}
