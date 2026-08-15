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

use std::path::PathBuf;

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
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("proton-sync").join("proton-sync.toml")
}

use gui_core::config_io::expand_config_path as expand;

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
