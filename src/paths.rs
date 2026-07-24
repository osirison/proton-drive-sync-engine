use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tracing::warn;

const APP_STATE_DIR: &str = "proton-drive-sync";
const DEFAULT_SOCKET_NAME: &str = "proton-sync.sock";
const DEFAULT_LOCKFILE_NAME: &str = "proton-sync.lock";
const DEFAULT_INDEX_NAME: &str = "sync_index.db";
/// Filename of the user-global single-instance lock (see [`default_global_lock_path`]).
const GLOBAL_LOCK_NAME: &str = "single-instance.lock";
/// Name of the per-sync-root directory that holds all of the engine's persistent state (the SQLite
/// index and its status/metrics sidecars) plus the instance lockfile. It lives at the top of
/// `local_root` (like `.git`) and is ignored by scanning, planning, the base-index filter, and the
/// filesystem watcher (see `index::should_ignore_path`).
pub const SYNC_STATE_DIR_NAME: &str = ".sync";

/// The `<local_root>/.sync` state directory.
pub fn sync_state_dir(local_root: &Path) -> PathBuf {
    local_root.join(SYNC_STATE_DIR_NAME)
}

/// The control socket stays in `$XDG_RUNTIME_DIR` — the XDG-designated home for sockets/IPC. It is
/// a session-scoped runtime endpoint, not persistent state; it must stay short to respect the
/// `sun_path` length limit; and the control CLI locates it there without needing to know the sync
/// root. So, unlike the persistent state, it does *not* move into `.sync`.
pub fn default_socket_path() -> PathBuf {
    default_runtime_path(DEFAULT_SOCKET_NAME, env_path("XDG_RUNTIME_DIR"))
}

/// The instance lockfile lives in the per-root `.sync` directory, so each sync root locks
/// independently and all of a root's state sits in one place.
pub fn default_lockfile_path(local_root: &Path) -> PathBuf {
    sync_state_dir(local_root).join(DEFAULT_LOCKFILE_NAME)
}

/// The SQLite index — and, alongside it, the status/metrics sidecars derived from this path — lives
/// in the per-root `.sync` directory.
pub fn default_state_db_path(local_root: &Path) -> PathBuf {
    sync_state_dir(local_root).join(DEFAULT_INDEX_NAME)
}

/// The **user-global** single-instance lock: the same path for every `proton-syncd` this user
/// runs, regardless of session, `XDG_RUNTIME_DIR`, sync root, or `--socket-path`. It admits at
/// most one daemon per user account, because every daemon shells the same `proton-drive` CLI —
/// whose SQLite cache **and** session store are shared per user and are not safe for concurrent
/// use (`SQLITE_BUSY`; see the concurrency note on `ProtonDriveClient::run_proton_drive` and #23).
///
/// Deliberately keyed on `$XDG_STATE_HOME` (→ `~/.local/state`) — a *per-user*, always-resolvable
/// location — and **not** on the per-session runtime dir: `$XDG_RUNTIME_DIR` is unset or differs
/// across sessions (SSH without pam_systemd, many containers), so a session-keyed lock would let a
/// second daemon slip through in exactly those cases and still contend on the shared cache. This
/// complements the *per-root* [`default_lockfile_path`], which stops two daemons on the same root.
pub fn default_global_lock_path() -> PathBuf {
    global_lock_path_in(user_state_dir())
}

/// The global lock's layout under a given state directory. Split out so it can be tested without
/// mutating process-wide environment variables (which races other tests).
fn global_lock_path_in(state_dir: PathBuf) -> PathBuf {
    state_dir.join(APP_STATE_DIR).join(GLOBAL_LOCK_NAME)
}

/// `$XDG_STATE_HOME`, falling back to `~/.local/state`, then — if even `HOME` is unset (unusual) —
/// to the per-uid temp directory so the path stays user-private rather than shared.
fn user_state_dir() -> PathBuf {
    if let Some(dir) = env_path("XDG_STATE_HOME") {
        dir
    } else if let Some(home) = env_path("HOME") {
        home.join(".local").join("state")
    } else {
        fallback_runtime_dir()
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
    fn state_db_and_lockfile_live_in_the_sync_state_dir_at_the_root() {
        let local_root = PathBuf::from("/home/me/Proton");

        assert_eq!(
            default_state_db_path(&local_root),
            PathBuf::from("/home/me/Proton/.sync/sync_index.db"),
            "the index DB must live in the per-root .sync state directory"
        );
        assert_eq!(
            default_lockfile_path(&local_root),
            PathBuf::from("/home/me/Proton/.sync/proton-sync.lock"),
            "the lockfile must live in the per-root .sync state directory"
        );
        assert_eq!(
            sync_state_dir(&local_root),
            PathBuf::from("/home/me/Proton/.sync")
        );
    }

    #[test]
    fn global_lock_is_user_scoped_under_the_state_dir() {
        // The user-global single-instance lock lives under the per-user state dir, namespaced by
        // the app dir — the same path for every daemon this user runs, independent of sync root or
        // socket. This is what lets it admit at most one daemon per user (the proton-drive CLI's
        // cache/session store are shared per user; see #23).
        assert_eq!(
            global_lock_path_in(PathBuf::from("/home/me/.local/state")),
            PathBuf::from("/home/me/.local/state/proton-drive-sync/single-instance.lock"),
        );
    }

    #[test]
    fn socket_stays_in_the_runtime_dir_not_the_sync_state_dir() {
        // The socket is a session-scoped IPC endpoint, not persistent state, so it stays in
        // $XDG_RUNTIME_DIR rather than moving into <local_root>/.sync.
        let path = default_runtime_path("proton-sync.sock", Some(PathBuf::from("/run/user/1000")));
        assert_eq!(path, PathBuf::from("/run/user/1000/proton-sync.sock"));
    }
}
