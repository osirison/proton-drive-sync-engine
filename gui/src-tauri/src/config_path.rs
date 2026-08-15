//! Runtime path resolution.
//!
//! The daemon has **no canonical config path**, so the GUI *owns* the convention:
//! `$XDG_CONFIG_HOME/proton-sync/proton-sync.toml` (default `~/.config/proton-sync/proton-sync.toml`).
//! From that file (via `gui-core`'s reader) we resolve the socket, index DB, and roots the commands
//! need, falling back to the engine's own defaults when a key is unset.
//!
//! A daemon may also be running with flags or a different config file entirely. The status reply
//! carries its *live* resolved roots (`RunningConfigInfo`); `get_status` caches them here so every
//! command can fall back to the daemon's ground truth when the GUI config doesn't provide a value.

use std::ffi::OsString;
use std::path::PathBuf;

/// A base-directory environment value, honoured **only when it is absolute** (#286). Every XDG
/// reader in this crate goes through it; the value is taken as an argument (never read here) so
/// the rule is testable without mutating process-wide environment variables, which races every
/// other test in the binary and is `unsafe` since edition 2024.
///
/// The XDG Base Directory specification is explicit: these variables "must be absolute", and an
/// implementation that meets a relative one "should consider the path invalid and ignore it". A
/// relative value is resolved against the *process's* working directory — for a desktop launcher,
/// whatever it happened to leave behind — so the GUI would write its config, dial its socket, and
/// drop its tray glyphs somewhere neither the user nor the daemon chose. The socket is the one
/// with a visible symptom: the daemon binds one resolution and the GUI dials another, which reads
/// as `unreachable` against a healthy daemon.
///
/// `is_absolute` subsumes the emptiness check these readers used to make on their own (an empty
/// path is not absolute) and catches the literal `~` no shell expanded for a GUI process (#135).
pub fn absolute_dir(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Paths the command layer needs, resolved once at startup and re-resolved after a config write.
pub struct RuntimePaths {
    pub config_path: PathBuf,
    /// `Err` only when the GUI config sets no `socket_path` **and** the engine's default fails
    /// closed (#74 — `XDG_RUNTIME_DIR` unset and the shared-/tmp fallback is not a directory this
    /// user owns at 0700). Carried as a `Result` rather than a guessed path so the UI reports the
    /// real reason instead of a wrong "connect: No such file" (#277).
    pub socket_path: Result<PathBuf, String>,
    /// From the GUI config file (explicit `db_path`, or derived from `local_root`). `None` when
    /// the config file provides neither — the daemon-reported path may still fill in.
    pub db_path: Option<PathBuf>,
    pub local_root: Option<PathBuf>,
    pub remote_root: Option<PathBuf>,
    pub proton_cli: String,
    /// Live values reported by the running daemon (cached from the last successful status round
    /// trip). Fallbacks only: an explicit GUI-config value always wins.
    pub daemon_local_root: Option<PathBuf>,
    pub daemon_remote_root: Option<PathBuf>,
    pub daemon_db_path: Option<PathBuf>,
}

/// The GUI's owned config path convention.
pub fn gui_config_path() -> PathBuf {
    gui_config_path_in(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn gui_config_path_in(config_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = absolute_dir(config_home)
        // Same rule for `HOME`: a relative one lands the config in the same per-cwd place.
        .or_else(|| absolute_dir(home).map(|home| home.join(".config")))
        // Last resort. Was a relative `.config`, which is the very shape the rule above rejects —
        // the config would be written under, and re-read from, whatever cwd each launch had.
        .unwrap_or_else(std::env::temp_dir);
    base.join("proton-sync").join("proton-sync.toml")
}

use gui_core::config_io::expand_config_path as expand;

// There is deliberately no `default_socket_path` here any more (#277). This crate had a private
// copy, and a private copy is how the GUI ended up dialling `<temp>/proton-sync.sock` while the
// daemon bound `<temp>/proton-drive-sync-<uid>/proton-sync.sock` — a healthy daemon rendered
// `unreachable`. `RuntimePaths::resolve` delegates to `gui_core::ipc::default_socket_path`, so the
// #286 absolute-XDG rule reaches the socket through the engine's own `paths::default_socket_path`
// rather than through a second implementation of it here. What the deleted
// `a_relative_runtime_dir_does_not_move_the_socket_the_gui_dials` asserted is now asserted where
// the code is: `paths::a_relative_runtime_dir_falls_through_to_the_validated_fallback` for the
// rule, and gui-core's `the_default_socket_path_is_never_the_unnamespaced_temp_one_the_gui_used_
// to_build` for the GUI resolving to exactly the engine's answer.

impl RuntimePaths {
    /// Resolve from the GUI config path, applying engine defaults where a key is unset.
    pub fn resolve() -> Self {
        let config_path = gui_config_path();
        let doc = gui_core::config_io::ConfigDoc::load(&config_path).ok();
        let get = |key: &str| doc.as_ref().and_then(|d| d.get_str(key));

        // `~` is expanded HERE, once, for the same reason the daemon expands it once at config
        // resolution (#135). Onboarding writes `local_root = "~/ProtonDrive"` — a literal the shell
        // never touched — and every GUI feature that joins that value onto the filesystem then
        // operates on a directory named `~` under the process's working directory: the conflict
        // scan finds nothing, the emblem lookup opens no index, and the free-space check reports
        // ENOENT on the one screen whose whole job is to say whether there is room. Expanding at
        // the funnel fixes every consumer at once, including the ones not written yet; expanding
        // per command is how the two halves drift apart again.
        //
        // `local_root` is expanded BEFORE `db_path` derives from it, or the derived index path
        // inherits the unexpanded root.
        let local_root = get("local_root").map(|value| expand(value, "local_root"));
        // A Proton Drive path, not a local one.
        let remote_root = get("remote_root").map(PathBuf::from);

        // The default comes from the engine (`gui_core::ipc::default_socket_path`), never from a
        // private copy here: the copy this replaced pointed at `<temp>/proton-sync.sock` while the
        // daemon bound `<temp>/proton-drive-sync-<uid>/proton-sync.sock` whenever
        // `XDG_RUNTIME_DIR` was unset (#277).
        let socket_path = match get("socket_path") {
            Some(value) => Ok(expand(value, "socket_path")),
            None => gui_core::ipc::default_socket_path(),
        };
        let db_path = get("db_path")
            .map(|value| expand(value, "db_path"))
            .or_else(|| {
                local_root
                    .as_ref()
                    .map(|root| root.join(".sync").join("sync_index.db"))
            });
        let proton_cli = get("proton_cli")
            .map(|value| expand(value, "proton_cli").to_string_lossy().into_owned())
            .unwrap_or_else(|| "proton-drive".to_string());

        Self {
            config_path,
            socket_path,
            db_path,
            local_root,
            remote_root,
            proton_cli,
            daemon_local_root: None,
            daemon_remote_root: None,
            daemon_db_path: None,
        }
    }

    /// Cache the daemon-reported live config from a status reply.
    pub fn remember_daemon_config(&mut self, info: &gui_core::wire::RunningConfigInfo) {
        self.daemon_local_root = Some(info.local_root.clone());
        self.daemon_remote_root = Some(info.remote_root.clone());
        self.daemon_db_path = Some(info.db_path.clone());
    }

    /// The local root commands should act on: GUI config first, daemon-reported second.
    pub fn effective_local_root(&self) -> Option<PathBuf> {
        self.local_root
            .clone()
            .or_else(|| self.daemon_local_root.clone())
    }

    /// The remote root for remote listings: GUI config first, daemon-reported second.
    pub fn effective_remote_root(&self) -> Option<PathBuf> {
        self.remote_root
            .clone()
            .or_else(|| self.daemon_remote_root.clone())
    }

    /// The index DB for read-only lookups: GUI config first, daemon-reported second.
    pub fn effective_db_path(&self) -> Option<PathBuf> {
        self.db_path.clone().or_else(|| self.daemon_db_path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #286. Asserted through the resolved paths, not the predicate: a relative value fails nothing
    // loudly — it resolves against whatever working directory the launcher left, per process, so
    // the symptom is two processes disagreeing about one path rather than an error anywhere.

    #[test]
    fn a_relative_config_home_falls_through_to_the_home_default() {
        let path = gui_config_path_in(Some(OsString::from(".config")), Some("/home/me".into()));

        assert_eq!(
            path,
            PathBuf::from("/home/me/.config/proton-sync/proton-sync.toml")
        );
    }

    #[test]
    fn a_relative_home_leaves_no_relative_config_path_behind() {
        // Both values invalid: the last resort must still be absolute, or the GUI writes its config
        // under one cwd and re-reads nothing under the next.
        let path = gui_config_path_in(Some(OsString::new()), Some(OsString::from("home/me")));

        assert!(path.is_absolute(), "config path: {}", path.display());
        assert_eq!(
            path,
            std::env::temp_dir()
                .join("proton-sync")
                .join("proton-sync.toml")
        );
    }

    #[test]
    fn only_absolute_env_values_are_honoured() {
        assert_eq!(
            absolute_dir(Some(OsString::from("/run/user/1000"))),
            Some(PathBuf::from("/run/user/1000"))
        );
        assert_eq!(absolute_dir(None), None);
        // Empty: what the old emptiness checks caught, still caught.
        assert_eq!(absolute_dir(Some(OsString::new())), None);
        assert_eq!(absolute_dir(Some(OsString::from(".config"))), None);
        // A literal `~` no shell expanded is a relative component, not $HOME (#135).
        assert_eq!(absolute_dir(Some(OsString::from("~/.config"))), None);
    }
}
