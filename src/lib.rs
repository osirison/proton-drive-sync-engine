pub mod config;
pub mod daemon;
pub mod index;
pub mod ipc;
pub mod paths;
pub mod proton;
pub mod sync;

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
