//! Runtime path resolution.
//!
//! The daemon has **no canonical config path**, so the GUI *owns* the convention:
//! `$XDG_CONFIG_HOME/proton-sync/proton-sync.toml` (default `~/.config/proton-sync/proton-sync.toml`).
//! From that file (via `gui-core`'s reader) we resolve the socket, index DB, and roots the commands
//! need, falling back to the engine's own defaults when a key is unset.

use std::path::PathBuf;

/// Paths the command layer needs, resolved once at startup and re-resolved after a config write.
pub struct RuntimePaths {
    pub config_path: PathBuf,
    pub socket_path: PathBuf,
    pub db_path: PathBuf,
    pub local_root: Option<PathBuf>,
    pub remote_root: Option<PathBuf>,
    pub proton_cli: String,
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
        let db_path = get("db_path")
            .map(PathBuf::from)
            .or_else(|| {
                local_root
                    .as_ref()
                    .map(|root| root.join(".sync").join("sync_index.db"))
            })
            .unwrap_or_else(|| PathBuf::from("sync_index.db"));
        let proton_cli = get("proton_cli").unwrap_or_else(|| "proton-drive".to_string());

        Self {
            config_path,
            socket_path,
            db_path,
            local_root,
            remote_root,
            proton_cli,
        }
    }
}
