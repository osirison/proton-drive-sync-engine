//! Path resolution for the GUI's open/launch actions (#220 `Open both in an editor`, #231
//! `Open folder`).
//!
//! PURE — no subprocess. The Tauri layer owns the spawn, the way it owns `systemctl`; what lives
//! here is the boundary guard. A path arriving from the webview is external input, so it is checked
//! here before anything joins it onto the sync root, per the engine's path-safety-at-boundaries
//! invariant.

use std::path::{Path, PathBuf};

/// Why a webview-supplied path was refused. Every variant is a sentence the UI shows verbatim —
/// a refused open must say what it refused, not fall silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRefusal {
    /// Neither the GUI config nor the daemon reported a sync folder.
    NoLocalRoot,
    /// Absolute, `..`, or a prefix component — `validate_relative_path` said no.
    NotRelative(String),
    /// Resolved outside the sync folder. Only reachable through a symlink: the textual guard above
    /// cannot see one.
    Escapes(String),
    /// Nothing on disk at the resolved path.
    Missing(String),
}

impl std::fmt::Display for OpenRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLocalRoot => write!(
                f,
                "no sync folder is configured yet — set one in Settings first"
            ),
            Self::NotRelative(path) => write!(
                f,
                "refusing to open {path}: only paths inside the sync folder can be opened"
            ),
            Self::Escapes(path) => write!(
                f,
                "refusing to open {path}: it leads outside the sync folder"
            ),
            Self::Missing(path) => write!(f, "{path} is not there any more"),
        }
    }
}

impl std::error::Error for OpenRefusal {}

/// Resolve a relative path from the webview to a real absolute path under `root`.
///
/// Refuses: no root, absolute/`..`/prefix components, a symlink that leaves the root, and anything
/// that is not on disk. The returned path is canonical, so the caller hands the opener a path that
/// is provably inside the sync folder.
pub fn resolve_under_root(root: Option<&Path>, relative: &str) -> Result<PathBuf, OpenRefusal> {
    let root = root.ok_or(OpenRefusal::NoLocalRoot)?;
    let safe = proton_drive_sync_engine::validate_relative_path(Path::new(relative))
        .ok_or_else(|| OpenRefusal::NotRelative(relative.to_string()))?;
    // Canonicalise BOTH sides. `validate_relative_path` is textual — it rejects a `..` written in
    // the string and cannot see a link inside the folder pointing out of it, which is the same
    // escape by another route.
    let real_root = root
        .canonicalize()
        .map_err(|_| OpenRefusal::Missing(root.display().to_string()))?;
    let joined = real_root.join(&safe);
    let real = joined
        .canonicalize()
        .map_err(|_| OpenRefusal::Missing(joined.display().to_string()))?;
    if !real.starts_with(&real_root) {
        return Err(OpenRefusal::Escapes(relative.to_string()));
    }
    Ok(real)
}

/// The folder to open for a path: the file's own directory, or the path itself when it is one.
///
/// The parent of a canonical path under the root is still under the root, so no second check is
/// needed — except for the root itself, whose parent is not, and which `resolve_under_root` returns
/// for an empty relative path.
pub fn folder_under_root(root: Option<&Path>, relative: &str) -> Result<PathBuf, OpenRefusal> {
    let resolved = resolve_under_root(root, relative)?;
    if resolved.is_dir() {
        return Ok(resolved);
    }
    Ok(resolved.parent().map(Path::to_path_buf).unwrap_or(resolved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/spec.md"), b"x").unwrap();
        dir
    }

    #[test]
    fn a_relative_path_inside_the_root_resolves() {
        let dir = root();
        let resolved = resolve_under_root(Some(dir.path()), "docs/spec.md").unwrap();
        assert!(resolved.ends_with("docs/spec.md"));
        assert!(resolved.is_absolute());
    }

    #[test]
    fn no_configured_root_is_refused_before_anything_is_joined() {
        assert_eq!(
            resolve_under_root(None, "docs/spec.md"),
            Err(OpenRefusal::NoLocalRoot)
        );
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let dir = root();
        assert_eq!(
            resolve_under_root(Some(dir.path()), "/etc/passwd"),
            Err(OpenRefusal::NotRelative("/etc/passwd".into()))
        );
    }

    #[test]
    fn a_parent_component_is_refused_even_when_it_would_land_inside() {
        let dir = root();
        // `docs/../docs/spec.md` resolves to a file that exists under the root, and is still
        // refused: the guard is on the shape of the path, not on where this one happens to land.
        assert_eq!(
            resolve_under_root(Some(dir.path()), "docs/../docs/spec.md"),
            Err(OpenRefusal::NotRelative("docs/../docs/spec.md".into()))
        );
        assert_eq!(
            resolve_under_root(Some(dir.path()), "../../etc/passwd"),
            Err(OpenRefusal::NotRelative("../../etc/passwd".into()))
        );
    }

    #[test]
    fn a_missing_file_is_refused_rather_than_handed_to_the_opener() {
        let dir = root();
        assert!(matches!(
            resolve_under_root(Some(dir.path()), "docs/gone.md"),
            Err(OpenRefusal::Missing(_))
        ));
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_out_of_the_root_is_refused() {
        // The hole the textual guard cannot see: no `..` anywhere in the string.
        let dir = root();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("escape"))
            .unwrap();
        assert_eq!(
            resolve_under_root(Some(dir.path()), "escape"),
            Err(OpenRefusal::Escapes("escape".into()))
        );
    }

    #[test]
    fn the_folder_for_a_file_is_its_directory_and_for_a_directory_is_itself() {
        let dir = root();
        let real_root = dir.path().canonicalize().unwrap();
        assert_eq!(
            folder_under_root(Some(dir.path()), "docs/spec.md").unwrap(),
            real_root.join("docs")
        );
        assert_eq!(
            folder_under_root(Some(dir.path()), "docs").unwrap(),
            real_root.join("docs")
        );
        // An empty path is the sync folder itself — the lookup screen's "no path yet" case.
        assert_eq!(folder_under_root(Some(dir.path()), "").unwrap(), real_root);
    }

    #[test]
    fn every_refusal_says_what_it_refused() {
        assert!(OpenRefusal::NoLocalRoot.to_string().contains("Settings"));
        assert!(
            OpenRefusal::NotRelative("/etc/passwd".into())
                .to_string()
                .contains("/etc/passwd")
        );
        assert!(
            OpenRefusal::Escapes("escape".into())
                .to_string()
                .contains("escape")
        );
        assert!(
            OpenRefusal::Missing("/x/gone".into())
                .to_string()
                .contains("/x/gone")
        );
    }
}
