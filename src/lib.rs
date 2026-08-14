#[cfg(not(unix))]
compile_error!(
    "proton-drive-sync-engine only supports Unix targets: the control-plane IPC \
     (src/ipc.rs), daemon socket handling, and path-safety helpers rely on Unix-only \
     APIs (Unix domain sockets, std::os::unix::fs::FileTypeExt, \
     std::os::unix::ffi::OsStrExt) and have not been validated on other platforms."
);

pub mod config;
pub mod daemon;
pub mod dirconfig;
pub mod events;
pub mod index;
pub mod ipc;
pub mod paths;
pub mod proton;
pub mod reconstruct;
pub mod session;
pub mod sync;

/// Filename prefix for the private per-download staging directory that
/// `ProtonDriveClient::download` creates next to each download destination (so the final
/// move onto the destination is an atomic same-filesystem rename). Because that places
/// the directory inside the synced local root, the scanner, base-index filter, and
/// filesystem watcher all skip any path containing a component with this prefix, so a
/// scratch directory orphaned by a hard crash mid-download is never scanned or uploaded
/// to the remote as junk. Kept here as the single source of truth shared by the code
/// that creates the directory (`src/proton.rs`) and the code that ignores it
/// (`src/index.rs`).
pub const DOWNLOAD_SCRATCH_PREFIX: &str = ".proton-sync-download-";

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn boxed_error(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

/// Validate and canonicalize a relative path so that it is safe to join onto a
/// local or remote root directory.
///
/// Returns `None` when `path` contains any component that could cause the
/// resolved path to escape the root:
/// - absolute paths / root-directory components,
/// - parent-directory (`..`) components,
/// - OS-specific prefix components.
///
/// `CurDir` (`.`) components are harmless but create inconsistent keys (e.g.
/// `foo/./bar` ≠ `foo/bar` in a `HashMap`); they are stripped so only
/// `Normal` components remain in the returned path.
pub fn validate_relative_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return None,
        }
    }
    let normalized: std::path::PathBuf = path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();
    // Deliberately `Some("")` for an empty/`.` path: the root itself is a legitimate value here
    // (`proton::collect_node`'s depth-0 wrapper node normalizes to it, and the daemon's
    // `CreateRemoteDirectory` uses it to mean "create the remote root"). Callers that resolve a
    // path into a *side effect* must use `validate_relative_path_non_empty` instead — see #72.
    Some(normalized)
}

/// [`validate_relative_path`], additionally rejecting the empty (root) result.
///
/// Joining an empty relative path onto a root resolves to the root itself, which silently
/// promotes a per-entry action into a whole-root one: a download over the local root, a delete
/// of the entire sync root. Any boundary that turns an externally-sourced relative path into a
/// filesystem or remote operation must reject it (issue #72).
pub fn validate_relative_path_non_empty(path: &std::path::Path) -> Option<std::path::PathBuf> {
    validate_relative_path(path).filter(|normalized| !normalized.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn validate_relative_path_keeps_the_empty_root_path() {
        // Pinned, not incidental: `proton::collect_node` returns early on `None`, so rejecting
        // the empty path here would skip the listing's root wrapper node and its entire
        // subtree, and `CreateRemoteDirectory`'s empty-path arm would stop creating the root.
        assert_eq!(validate_relative_path(Path::new("")), Some(PathBuf::new()));
        assert_eq!(validate_relative_path(Path::new(".")), Some(PathBuf::new()));
        assert_eq!(
            validate_relative_path(Path::new("./")),
            Some(PathBuf::new())
        );
    }

    #[test]
    fn validate_relative_path_non_empty_rejects_the_root_path() {
        assert_eq!(validate_relative_path_non_empty(Path::new("")), None);
        assert_eq!(validate_relative_path_non_empty(Path::new(".")), None);
        assert_eq!(validate_relative_path_non_empty(Path::new("./.")), None);
        assert_eq!(validate_relative_path_non_empty(Path::new("..")), None);
        assert_eq!(validate_relative_path_non_empty(Path::new("/abs")), None);
        assert_eq!(
            validate_relative_path_non_empty(Path::new("./sub/notes.txt")),
            Some(PathBuf::from("sub/notes.txt"))
        );
    }
}
