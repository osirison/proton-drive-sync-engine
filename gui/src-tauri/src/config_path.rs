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
    pub socket_path: PathBuf,
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

fn default_socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(std::env::temp_dir)
        .join("proton-sync.sock")
}

impl RuntimePaths {
    /// Resolve from the GUI config path, applying engine defaults where a key is unset.
    pub fn resolve() -> Self {
        let config_path = gui_config_path();
        let doc = gui_core::config_io::ConfigDoc::load(&config_path).ok();
        let get = |key: &str| doc.as_ref().and_then(|d| d.get_str(key));

        let local_root = get("local_root").map(PathBuf::from);
        let remote_root = get("remote_root").map(PathBuf::from);
        let socket_path = get("socket_path")
            .map(PathBuf::from)
            .unwrap_or_else(default_socket_path);
        let db_path = get("db_path").map(PathBuf::from).or_else(|| {
            local_root
                .as_ref()
                .map(|root| root.join(".sync").join("sync_index.db"))
        });
        let proton_cli = get("proton_cli").unwrap_or_else(|| "proton-drive".to_string());

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
