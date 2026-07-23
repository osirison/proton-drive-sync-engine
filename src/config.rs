use crate::daemon::DaemonConfig;
use crate::index::ScanOptions;
use crate::paths::{default_lockfile_path, default_socket_path, default_state_db_path};
use crate::proton::CommandPolicy;
use crate::{AppResult, boxed_error};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default number of incremental event-driven passes between forced full-tree resyncs when
/// `events_driven` is on and no explicit value is configured.
const DEFAULT_EVENTS_FULL_SCAN_EVERY: u64 = 20;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DaemonConfigInput {
    pub config: Option<PathBuf>,
    pub local_root: Option<PathBuf>,
    pub remote_root: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
    pub lockfile_path: Option<PathBuf>,
    pub scan_interval_secs: Option<u64>,
    pub proton_cli: Option<PathBuf>,
    pub proton_timeout_secs: Option<u64>,
    pub proton_list_attempts: Option<usize>,
    pub dry_run: bool,
    pub no_dry_run: bool,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub events_driven: bool,
    pub events_full_scan_every: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    #[serde(alias = "local-root")]
    local_root: Option<PathBuf>,
    #[serde(alias = "remote-root")]
    remote_root: Option<PathBuf>,
    #[serde(alias = "db-path")]
    db_path: Option<PathBuf>,
    #[serde(alias = "socket-path")]
    socket_path: Option<PathBuf>,
    #[serde(alias = "lockfile-path")]
    lockfile_path: Option<PathBuf>,
    #[serde(alias = "scan-interval-secs")]
    scan_interval_secs: Option<u64>,
    #[serde(alias = "proton-cli")]
    proton_cli: Option<PathBuf>,
    #[serde(alias = "proton-timeout-secs")]
    proton_timeout_secs: Option<u64>,
    #[serde(alias = "proton-list-attempts")]
    proton_list_attempts: Option<usize>,
    #[serde(default, alias = "include")]
    include_patterns: Option<Vec<String>>,
    #[serde(default, alias = "exclude")]
    exclude_patterns: Option<Vec<String>>,
    #[serde(default, alias = "dry-run")]
    dry_run: Option<bool>,
    #[serde(default, alias = "events-driven")]
    events_driven: Option<bool>,
    #[serde(default, alias = "events-full-scan-every")]
    events_full_scan_every: Option<u64>,
}

pub fn resolve_runtime_config(input: DaemonConfigInput) -> AppResult<(DaemonConfig, bool)> {
    let file_config = load_file_config(input.config.as_ref())?;
    let dry_run = if input.no_dry_run {
        false
    } else if input.dry_run {
        true
    } else {
        file_config.dry_run.unwrap_or(false)
    };
    let local_root = input
        .local_root
        .or(file_config.local_root)
        .ok_or_else(|| boxed_error("missing required --local-root or config local_root"))?;
    let remote_root = input
        .remote_root
        .or(file_config.remote_root)
        .ok_or_else(|| boxed_error("missing required --remote-root or config remote_root"))?;
    let db_path = input
        .db_path
        .or(file_config.db_path)
        .map(|path| resolve_path(&local_root, path))
        .unwrap_or_else(|| default_state_db_path(&local_root));
    let default_command_policy = CommandPolicy::default();

    let config = DaemonConfig {
        local_root,
        remote_root,
        db_path,
        socket_path: input
            .socket_path
            .or(file_config.socket_path)
            .unwrap_or_else(default_socket_path),
        lockfile_path: input
            .lockfile_path
            .or(file_config.lockfile_path)
            .unwrap_or_else(default_lockfile_path),
        scan_interval: Duration::from_secs(
            input
                .scan_interval_secs
                .or(file_config.scan_interval_secs)
                .unwrap_or(300)
                .max(1),
        ),
        proton_cli: input
            .proton_cli
            .or(file_config.proton_cli)
            .unwrap_or_else(|| PathBuf::from("proton-drive")),
        proton_timeout: resolve_positive_duration_secs(
            input.proton_timeout_secs,
            file_config.proton_timeout_secs,
            default_command_policy.timeout.as_secs(),
            "proton_timeout_secs",
        )?,
        proton_list_attempts: resolve_positive_usize(
            input.proton_list_attempts,
            file_config.proton_list_attempts,
            default_command_policy.list_attempts,
            "proton_list_attempts",
        )?,
        include_patterns: merge_patterns(input.include_patterns, file_config.include_patterns),
        exclude_patterns: merge_patterns(input.exclude_patterns, file_config.exclude_patterns),
        events_driven: input.events_driven || file_config.events_driven.unwrap_or(false),
        events_full_scan_every: input
            .events_full_scan_every
            .or(file_config.events_full_scan_every)
            .unwrap_or(DEFAULT_EVENTS_FULL_SCAN_EVERY)
            .max(1),
    };
    validate_runtime_config(&config)?;

    Ok((config, dry_run))
}

pub fn load_file_config(path: Option<&PathBuf>) -> AppResult<FileConfig> {
    let Some(path) = path else {
        return Ok(FileConfig::default());
    };
    let config = fs::read_to_string(path).map_err(|error| {
        boxed_error(format!("failed to read config {}: {error}", path.display()))
    })?;
    parse_file_config(&config).map_err(|error| {
        boxed_error(format!(
            "failed to parse config {}: {error}",
            path.display()
        ))
    })
}

pub fn parse_file_config(config: &str) -> Result<FileConfig, toml::de::Error> {
    toml::from_str(config)
}

fn resolve_path(local_root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        local_root.join(path)
    }
}

fn resolve_positive_duration_secs(
    input_value: Option<u64>,
    file_value: Option<u64>,
    default_value: u64,
    field_name: &str,
) -> AppResult<Duration> {
    let value = input_value.or(file_value).unwrap_or(default_value);
    if value == 0 {
        return Err(boxed_error(format!(
            "{field_name} must be greater than zero"
        )));
    }
    Ok(Duration::from_secs(value))
}

fn resolve_positive_usize(
    input_value: Option<usize>,
    file_value: Option<usize>,
    default_value: usize,
    field_name: &str,
) -> AppResult<usize> {
    let value = input_value.or(file_value).unwrap_or(default_value);
    if value == 0 {
        return Err(boxed_error(format!(
            "{field_name} must be greater than zero"
        )));
    }
    Ok(value)
}

fn validate_runtime_config(config: &DaemonConfig) -> AppResult<()> {
    if config.local_root.as_os_str().is_empty() {
        return Err(boxed_error("local_root must not be empty"));
    }
    if config.remote_root.as_os_str().is_empty() {
        return Err(boxed_error("remote_root must not be empty"));
    }
    ScanOptions::new(
        &config.local_root,
        std::slice::from_ref(&config.db_path),
        &config.include_patterns,
        &config.exclude_patterns,
    )
    .map_err(|error| boxed_error(format!("invalid scan filter configuration: {error}")))?;
    Ok(())
}

fn merge_patterns(cli_patterns: Vec<String>, config_patterns: Option<Vec<String>>) -> Vec<String> {
    if cli_patterns.is_empty() {
        config_patterns.unwrap_or_default()
    } else {
        cli_patterns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_file_supplies_required_daemon_options() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
db_path = "state/index.db"
socket_path = "/tmp/from-config.sock"
lockfile_path = "/tmp/from-config.lock"
scan_interval_secs = 42
proton_cli = "fake-proton-drive"
proton_timeout_secs = 17
proton_list_attempts = 4
include = ["Documents/**"]
exclude = ["**/*.tmp"]
dry_run = true
"#,
        )
        .expect("write config");

        let (config, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("sync-root"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/RemoteFolder"));
        assert_eq!(config.db_path, PathBuf::from("sync-root/state/index.db"));
        assert_eq!(config.scan_interval, Duration::from_secs(42));
        assert_eq!(config.proton_cli, PathBuf::from("fake-proton-drive"));
        assert_eq!(config.proton_timeout, Duration::from_secs(17));
        assert_eq!(config.proton_list_attempts, 4);
        assert_eq!(config.include_patterns, vec!["Documents/**"]);
        assert_eq!(config.exclude_patterns, vec!["**/*.tmp"]);
    }

    #[test]
    fn explicit_cli_values_override_config_file_values() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "config-root"
remote_root = "/Drive/Config"
proton_timeout_secs = 10
proton_list_attempts = 2
include = ["config/**"]
"#,
        )
        .expect("write config");

        let (config, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            local_root: Some(PathBuf::from("cli-root")),
            remote_root: Some(PathBuf::from("/Drive/Cli")),
            proton_timeout_secs: Some(22),
            proton_list_attempts: Some(5),
            dry_run: true,
            include_patterns: vec!["cli/**".to_owned()],
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("cli-root"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/Cli"));
        assert_eq!(config.proton_timeout, Duration::from_secs(22));
        assert_eq!(config.proton_list_attempts, 5);
        assert_eq!(config.include_patterns, vec!["cli/**"]);
    }

    #[test]
    fn events_options_resolve_from_file_and_default_scan_interval() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
events_driven = true
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            config.events_driven,
            "events_driven must come from the file"
        );
        assert_eq!(
            config.events_full_scan_every, DEFAULT_EVENTS_FULL_SCAN_EVERY,
            "an unset periodic-resync interval falls back to the default"
        );
    }

    #[test]
    fn explicit_cli_events_flag_and_interval_override_defaults() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            events_driven: true,
            events_full_scan_every: Some(5),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(config.events_driven);
        assert_eq!(config.events_full_scan_every, 5);
    }

    #[test]
    fn zero_events_full_scan_every_is_clamped_to_one() {
        // The periodic safety resync is mandatory; a configured 0 must never disable it.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            events_driven: true,
            events_full_scan_every: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(config.events_full_scan_every, 1);
    }

    #[test]
    fn explicit_no_dry_run_overrides_config_file_dry_run() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "config-root"
remote_root = "/Drive/Config"
dry_run = true
"#,
        )
        .expect("write config");

        let (_, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            no_dry_run: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(!dry_run);
    }

    #[test]
    fn invalid_include_glob_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            include_patterns: vec!["[".to_owned()],
            ..DaemonConfigInput::default()
        })
        .expect_err("invalid include glob should fail");

        assert!(
            error
                .to_string()
                .contains("invalid scan filter configuration"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn zero_proton_timeout_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_timeout_secs: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect_err("zero Proton timeout should fail");

        assert_eq!(
            error.to_string(),
            "proton_timeout_secs must be greater than zero"
        );
    }

    #[test]
    fn zero_proton_list_attempts_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_list_attempts: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect_err("zero Proton list attempts should fail");

        assert_eq!(
            error.to_string(),
            "proton_list_attempts must be greater than zero"
        );
    }

    #[test]
    fn empty_local_root_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::new()),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty local root should fail");

        assert_eq!(error.to_string(), "local_root must not be empty");
    }

    #[test]
    fn empty_remote_root_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::new()),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty remote root should fail");

        assert_eq!(error.to_string(), "remote_root must not be empty");
    }
}
