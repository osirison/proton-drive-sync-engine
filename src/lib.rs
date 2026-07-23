#[cfg(not(unix))]
compile_error!(
    "proton-drive-sync-engine only supports Unix targets: the control-plane IPC \
     (src/ipc.rs), daemon socket handling, and path-safety helpers rely on Unix-only \
     APIs (Unix domain sockets, std::os::unix::fs::FileTypeExt, \
     std::os::unix::ffi::OsStrExt) and have not been validated on other platforms."
);

pub mod config;
pub mod daemon;
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
    Some(normalized)
}
