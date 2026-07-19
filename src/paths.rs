use std::env;
use std::path::{Path, PathBuf};

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
    runtime_dir.unwrap_or_else(env::temp_dir).join(file_name)
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
