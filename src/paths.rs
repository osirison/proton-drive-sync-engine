use crate::{AppResult, boxed_error};
use std::env;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

const APP_STATE_DIR: &str = "proton-drive-sync";
/// Mode the fallback runtime directory must end up with: owner-only, no setuid/setgid/sticky.
const RUNTIME_DIR_MODE: u32 = 0o700;
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
///
/// Fallible: with `XDG_RUNTIME_DIR` unset this resolves through [`fallback_runtime_dir`], which
/// fails closed rather than hand back a path in attacker-controlled space (#74).
pub fn default_socket_path() -> AppResult<PathBuf> {
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
///
/// Fallible for the same reason as [`default_socket_path`]: with both `XDG_STATE_HOME` and `HOME`
/// unset it resolves through [`fallback_runtime_dir`], which fails closed (#74).
pub fn default_global_lock_path() -> AppResult<PathBuf> {
    Ok(global_lock_path_in(user_state_dir()?))
}

/// The global lock's layout under a given state directory. Split out so it can be tested without
/// mutating process-wide environment variables (which races other tests).
fn global_lock_path_in(state_dir: PathBuf) -> PathBuf {
    state_dir.join(APP_STATE_DIR).join(GLOBAL_LOCK_NAME)
}

/// `$XDG_STATE_HOME`, falling back to `~/.local/state`, then — if even `HOME` is unset (unusual) —
/// to the per-uid temp directory so the path stays user-private rather than shared.
fn user_state_dir() -> AppResult<PathBuf> {
    if let Some(dir) = env_path("XDG_STATE_HOME") {
        Ok(dir)
    } else if let Some(home) = env_path("HOME") {
        Ok(home.join(".local").join("state"))
    } else {
        fallback_runtime_dir()
    }
}

fn default_runtime_path(file_name: &str, runtime_dir: Option<PathBuf>) -> AppResult<PathBuf> {
    match runtime_dir {
        // A session-manager-provided runtime dir is trusted (and never created by us); only the
        // shared-/tmp fallback below is attacker-plantable, so only it is validated.
        Some(dir) => Ok(dir.join(file_name)),
        None => Ok(fallback_runtime_dir()?.join(file_name)),
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
/// **Fails closed** (#74). The path is predictable and its parent is world-writable, so a local
/// attacker can pre-create it — as a symlink into their own tree, or as a directory they own —
/// before the daemon ever runs. Returning it anyway would bind the control socket in attacker
/// space, where the socket can be unlinked and replaced by an impostor listener that fakes
/// `status`/`approve` acknowledgements. Creation failure, a non-directory, a foreign owner, or a
/// mode that cannot be tightened to 0700 are therefore errors, not warnings.
fn fallback_runtime_dir() -> AppResult<PathBuf> {
    // Suffix the shared-temp fallback with the effective uid so two local users never
    // contend for the same directory: the first user to create a single shared
    // `proton-drive-sync` at mode 0700 would otherwise lock everyone else out of their own
    // fallback runtime path.
    let uid = effective_uid();
    ensure_private_runtime_dir(env::temp_dir().join(format!("{APP_STATE_DIR}-{uid}")), uid)
}

/// Create (or adopt) `dir` and prove it is a real directory private to `owner_uid`.
/// `owner_uid` is a parameter so the refusal paths are testable without a second user account.
fn ensure_private_runtime_dir(dir: PathBuf, owner_uid: u32) -> AppResult<PathBuf> {
    fs::create_dir_all(&dir).map_err(|error| {
        boxed_error(format!(
            "failed to create fallback runtime directory {}: {error}",
            dir.display()
        ))
    })?;
    // Check BEFORE chmod: `set_permissions` follows symlinks, so tightening first would re-mode
    // an attacker's target instead of this path.
    require_private_dir(&dir, owner_uid, None)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(RUNTIME_DIR_MODE)).map_err(|error| {
        boxed_error(format!(
            "failed to restrict fallback runtime directory {} to mode {RUNTIME_DIR_MODE:o}: {error}",
            dir.display()
        ))
    })?;
    // Re-verify after the chmod: closes the swap window between the check above and the chmod,
    // and is the only place the mode itself is asserted.
    require_private_dir(&dir, owner_uid, Some(RUNTIME_DIR_MODE))?;
    Ok(dir)
}

/// Fail-closed gate for a directory the engine is about to put private runtime state in.
/// `expected_mode` is compared against the full permission bits (including setuid/setgid/sticky)
/// and is only meaningful after this process has set them.
fn require_private_dir(dir: &Path, owner_uid: u32, expected_mode: Option<u32>) -> AppResult<()> {
    let metadata = fs::symlink_metadata(dir).map_err(|error| {
        boxed_error(format!(
            "failed to inspect fallback runtime directory {}: {error}",
            dir.display()
        ))
    })?;
    if !metadata.file_type().is_dir() {
        return Err(boxed_error(format!(
            "refusing to use {}: it is not a directory (a symlink or other object is planted at \
             this predictable path), so private runtime state would land in space this user does \
             not control",
            dir.display()
        )));
    }
    if metadata.uid() != owner_uid {
        return Err(boxed_error(format!(
            "refusing to use {}: it is owned by uid {}, not {owner_uid}",
            dir.display(),
            metadata.uid()
        )));
    }
    if let Some(expected) = expected_mode {
        let mode = metadata.permissions().mode() & 0o7777;
        if mode != expected {
            return Err(boxed_error(format!(
                "refusing to use {}: mode is {mode:o}, expected {expected:o} (owner-only)",
                dir.display()
            )));
        }
    }
    Ok(())
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
        let path = default_runtime_path("proton-sync.sock", Some(PathBuf::from("/run/user/1000")))
            .expect("a provided runtime dir needs no validation");

        assert_eq!(path, PathBuf::from("/run/user/1000/proton-sync.sock"));
    }

    #[test]
    fn runtime_defaults_fall_back_to_a_namespaced_temp_subdirectory_when_unset() {
        let path = default_runtime_path("proton-sync.sock", None).expect("fallback resolves");

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
        let path = default_runtime_path("proton-sync.sock", Some(PathBuf::from("/run/user/1000")))
            .expect("a provided runtime dir needs no validation");
        assert_eq!(path, PathBuf::from("/run/user/1000/proton-sync.sock"));
    }

    // #74: the fallback path is predictable and its parent is world-writable, so a local attacker
    // can plant it before the daemon runs. Every planted shape below must be REFUSED — the old
    // code warned and returned the path, which put the control socket in attacker space.

    #[test]
    fn a_symlink_planted_at_the_fallback_runtime_dir_is_refused() {
        let directory = tempfile::tempdir().expect("tempdir");
        let attacker_dir = directory.path().join("attacker");
        fs::create_dir(&attacker_dir).expect("attacker directory");
        fs::set_permissions(&attacker_dir, fs::Permissions::from_mode(0o777))
            .expect("attacker mode");
        let planted = directory.path().join("proton-drive-sync-1000");
        std::os::unix::fs::symlink(&attacker_dir, &planted).expect("plant symlink");

        let error = ensure_private_runtime_dir(planted, effective_uid())
            .expect_err("a symlink at the fallback path must be refused, not warned about");

        assert!(
            error.to_string().contains("not a directory"),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::symlink_metadata(&attacker_dir)
                .expect("attacker metadata")
                .permissions()
                .mode()
                & 0o777,
            0o777,
            "the symlink target must not have been chmod'd through the link"
        );
    }

    #[test]
    fn a_regular_file_planted_at_the_fallback_runtime_dir_is_refused() {
        let directory = tempfile::tempdir().expect("tempdir");
        let planted = directory.path().join("proton-drive-sync-1000");
        fs::write(&planted, b"").expect("plant regular file");

        // Refused at `create_dir_all` (EEXIST on a non-directory) rather than at the type check,
        // but refused either way — the point is that no path is handed back.
        let error = ensure_private_runtime_dir(planted.clone(), effective_uid())
            .expect_err("a non-directory at the fallback path must be refused");

        assert!(
            error.to_string().contains(&planted.display().to_string()),
            "the error must name the refused path: {error}"
        );
        assert!(
            fs::symlink_metadata(&planted)
                .expect("planted metadata")
                .file_type()
                .is_file(),
            "the planted file must be left alone"
        );
    }

    #[test]
    fn a_foreign_owned_fallback_runtime_dir_is_refused() {
        // A directory owned by another local user (the ownership check is parameterised so this
        // needs no second account: the real uid is compared against a different expected owner).
        let directory = tempfile::tempdir().expect("tempdir");
        let planted = directory.path().join("proton-drive-sync-1000");
        fs::create_dir(&planted).expect("planted directory");

        let error = ensure_private_runtime_dir(planted, effective_uid().wrapping_add(1))
            .expect_err("a foreign-owned fallback directory must be refused");

        assert!(
            error.to_string().contains("owned by uid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_fallback_runtime_dir_left_group_or_world_accessible_is_refused() {
        // The post-chmod gate: if the mode is not owner-only when we re-read it, refuse rather
        // than proceed. (Reached in production when a swap wins the race with our chmod.)
        let directory = tempfile::tempdir().expect("tempdir");
        let dir = directory.path().join("proton-drive-sync-1000");
        fs::create_dir(&dir).expect("directory");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).expect("hostile mode");

        let error = require_private_dir(&dir, effective_uid(), Some(RUNTIME_DIR_MODE))
            .expect_err("a group/world-accessible runtime directory must be refused");

        assert!(
            error.to_string().contains("mode is 777"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_owned_fallback_runtime_dir_is_adopted_and_tightened() {
        // The accepted case: a directory this user owns is kept, with a loose mode tightened to
        // 0700 rather than refused (an upgrade from an earlier version must still start).
        let directory = tempfile::tempdir().expect("tempdir");
        let dir = directory.path().join("proton-drive-sync-1000");
        fs::create_dir(&dir).expect("directory");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("loose mode");

        let resolved = ensure_private_runtime_dir(dir.clone(), effective_uid())
            .expect("own directory adopted");

        assert_eq!(resolved, dir);
        assert_eq!(
            fs::symlink_metadata(&dir)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o7777,
            RUNTIME_DIR_MODE
        );
    }
}
