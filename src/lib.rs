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

/// The form of a path as it travels on **any** wire this engine publishes — the control
/// protocol's JSON (#61) and the dry-run report (#300) alike — and therefore the form an
/// `approve`/`deny` selector must be matched against: `to_string_lossy`. A client only ever sees
/// this rendering (a non-UTF-8 path cannot survive JSON), so the daemon compares selectors here
/// rather than against the real `PathBuf` it keeps internally — otherwise exactly the paths that
/// motivated the lossy wire would be unreachable through the control plane.
///
/// The rule this fixes in place: a wire path is a **rendering, never an authoritative selector**.
/// Two paths that differ only in invalid bytes collapse to one string, so a command that turns a
/// selector into a destructive side effect must refuse an ambiguous one rather than resolve it
/// arbitrarily (`daemon::apply_approval_command`), and nothing may promote a rendering back into a
/// real path. That covers the plan surface too: a `PlannedAction`'s path is display data, and any
/// future command that authorises a plan row by path (the GUI's typed-DELETE gate, #227) must
/// match in this form and refuse ambiguity rather than invent a second rule.
///
/// Returns the `Cow` unchanged rather than an owned `String`: these fields ride on every status
/// reply and the selector match walks every pending deletion, and `to_string_lossy` borrows for a
/// UTF-8 path — so only a path that actually needs replacing pays for an allocation.
pub fn wire_path(path: &std::path::Path) -> std::borrow::Cow<'_, str> {
    path.to_string_lossy()
}

/// Serde for a `PathBuf` field on the wire: **lossy** out, verbatim back.
///
/// `impl Serialize for Path` *errors* on a non-UTF-8 path, and the engine deliberately supports
/// such paths (`index::index_key` is a BLOB for exactly that). `serde_json` is all-or-nothing, so
/// one such path used to fail the *whole* document: every `ControlResponse` — for `status`,
/// `pause`, `pending` and `approve` alike, a control-plane lockout that could not be cleared
/// through the control plane (#61) — and every `--dry-run` report, which exits 1 having printed
/// nothing and blanks the GUI's plan screen (#300). The daemon keeps the real `PathBuf`; only the
/// JSON is lossy, and byte-identical for a UTF-8 path so no consumer sees a format change.
pub(crate) mod lossy_path {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::{Path, PathBuf};

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::wire_path(path))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Ok(PathBuf::from(String::deserialize(deserializer)?))
    }

    /// The same for an `Option<PathBuf>` field; `None` stays `null`.
    pub mod optional {
        use serde::{Deserialize, Deserializer, Serializer};
        use std::path::PathBuf;

        pub fn serialize<S: Serializer>(
            path: &Option<PathBuf>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match path {
                Some(path) => serializer.serialize_some(&*crate::wire_path(path)),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<PathBuf>, D::Error> {
            Ok(Option::<String>::deserialize(deserializer)?.map(PathBuf::from))
        }
    }
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
