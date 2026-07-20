use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tracing::warn;

const APP_STATE_DIR: &str = "proton-drive-sync";
const DEFAULT_SOCKET_NAME: &str = "proton-sync.sock";
const DEFAULT_LOCKFILE_NAME: &str = "proton-sync.lock";
const DEFAULT_INDEX_NAME: &str = "sync_index.db";

pub fn default_socket_path() -> PathBuf {
    default_runtime_path(DEFAULT_SOCKET_NAME, env_path("XDG_RUNTIME_DIR"))
}

pub fn default_lockfile_path() -> PathBuf {
    default_runtime_path(DEFAULT_LOCKFILE_NAME, env_path("XDG_RUNTIME_DIR"))
}

pub fn default_state_db_path(local_root: &Path) -> PathBuf {
    match default_state_dir(env_path("XDG_STATE_HOME"), env_path("HOME")) {
        Some(state_dir) => state_dir.join(APP_STATE_DIR).join(DEFAULT_INDEX_NAME),
        None => local_root.join(DEFAULT_INDEX_NAME),
    }
}

fn default_runtime_path(file_name: &str, runtime_dir: Option<PathBuf>) -> PathBuf {
    match runtime_dir {
        Some(dir) => dir.join(file_name),
        None => fallback_runtime_dir().join(file_name),
    }
}

/// `XDG_RUNTIME_DIR` is normally set by a login session manager (for example
/// systemd-logind) to a private, per-user, mode-0700 directory. When it is unset
/// (minimal environments, some containers, `su`'d shells) fall back to a
/// namespaced, owner-only-permission subdirectory of the shared temporary
/// directory rather than writing predictable filenames (`proton-sync.sock`)
/// directly into world-writable `/tmp`, where they would be guessable and could
/// collide with another local user's daemon instance.
///
/// Directory creation and permission tightening are best-effort: any failure is
/// logged rather than propagated, since callers still get a concrete path back
/// and will surface a clear I/O error themselves the moment they actually try to
/// create the socket or lockfile there.
fn fallback_runtime_dir() -> PathBuf {
    // Suffix the shared-temp fallback with the effective uid so two local users never
    // contend for the same directory: the first user to create a single shared
    // `proton-drive-sync` at mode 0700 would otherwise lock everyone else out of their own
    // fallback runtime path.
    let dir = env::temp_dir().join(format!("{APP_STATE_DIR}-{}", effective_uid()));
    if let Err(error) = fs::create_dir_all(&dir) {
        warn!(
            path = %dir.display(),
            %error,
            "failed to create fallback runtime directory"
        );
        return dir;
    }
    // `set_permissions` follows symlinks, so a symlink (or any non-directory) swapped in at
    // this predictable path would have its target chmod'd instead. Inspect the link itself
    // and only tighten permissions on a real directory; otherwise leave it alone and warn.
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            if let Err(error) = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)) {
                warn!(
                    path = %dir.display(),
                    %error,
                    "failed to restrict fallback runtime directory permissions"
                );
            }
        }
        Ok(_) => warn!(
            path = %dir.display(),
            "fallback runtime directory path is not a real directory; leaving permissions unchanged"
        ),
        Err(error) => warn!(
            path = %dir.display(),
            %error,
            "failed to inspect fallback runtime directory; leaving permissions unchanged"
        ),
    }
    dir
}

/// The effective user id, used to give each local user a private fallback runtime
/// directory under the shared temporary directory.
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no arguments, never fails, and touches no memory.
    unsafe { libc::geteuid() }
}

fn default_state_dir(xdg_state_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg_state_home.or_else(|| home.map(|home| home.join(".local/state")))
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_defaults_use_xdg_runtime_dir_when_available() {
        let path = default_runtime_path("proton-sync.sock", Some(PathBuf::from("/run/user/1000")));

        assert_eq!(path, PathBuf::from("/run/user/1000/proton-sync.sock"));
    }

    #[test]
    fn runtime_defaults_fall_back_to_a_namespaced_temp_subdirectory_when_unset() {
        let path = default_runtime_path("proton-sync.sock", None);

        let parent = path.parent().expect("parent directory");
        let parent_name = parent
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fallback directory name");
        assert!(
            parent_name.starts_with(APP_STATE_DIR),
            "the fallback socket path must live under an app-namespaced subdirectory, got {parent_name}"
        );
        assert_ne!(
            parent_name, APP_STATE_DIR,
            "the fallback directory must be per-user (uid-suffixed) to avoid cross-user collisions"
        );
        assert_ne!(
            parent,
            env::temp_dir(),
            "the fallback must not place a predictable, unnamespaced file directly \
             in the shared temp directory"
        );

        let metadata = fs::metadata(parent).expect("fallback directory must be created");
        assert!(metadata.is_dir());
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            0o700,
            "fallback runtime directory must only be accessible to its owner"
        );
    }

    #[test]
    fn state_defaults_prefer_xdg_state_home() {
        let state_dir = default_state_dir(
            Some(PathBuf::from("/home/me/.local/state-custom")),
            Some(PathBuf::from("/home/me")),
        );

        assert_eq!(
            state_dir,
            Some(PathBuf::from("/home/me/.local/state-custom"))
        );
    }

    #[test]
    fn state_defaults_fall_back_to_home_local_state() {
        let state_dir = default_state_dir(None, Some(PathBuf::from("/home/me")));

        assert_eq!(state_dir, Some(PathBuf::from("/home/me/.local/state")));
    }
}
