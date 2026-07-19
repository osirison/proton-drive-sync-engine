use clap::Parser;
use proton_drive_sync_engine::daemon::{Daemon, DaemonConfig, preview_plan};
use proton_drive_sync_engine::index::ScanOptions;
use proton_drive_sync_engine::paths::{
    default_lockfile_path, default_socket_path, default_state_db_path,
};
use proton_drive_sync_engine::sync::DryRunReport;
use proton_drive_sync_engine::{AppResult, boxed_error};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "proton-syncd",
    about = "Bidirectional Proton Drive background sync daemon"
)]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    local_root: Option<PathBuf>,
    #[arg(long)]
    remote_root: Option<PathBuf>,
    #[arg(long)]
    db_path: Option<PathBuf>,
    #[arg(long)]
    socket_path: Option<PathBuf>,
    #[arg(long)]
    lockfile_path: Option<PathBuf>,
    #[arg(long)]
    scan_interval_secs: Option<u64>,
    #[arg(long)]
    proton_cli: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long = "no-dry-run", conflicts_with = "dry_run")]
    no_dry_run: bool,
    #[arg(long = "include", value_name = "GLOB")]
    include_patterns: Vec<String>,
    #[arg(long = "exclude", value_name = "GLOB")]
    exclude_patterns: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
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
    #[serde(default, alias = "include")]
    include_patterns: Option<Vec<String>>,
    #[serde(default, alias = "exclude")]
    exclude_patterns: Option<Vec<String>>,
    #[serde(default, alias = "dry-run")]
    dry_run: Option<bool>,
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let (config, dry_run) = match resolve_runtime_config(cli) {
        Ok(config) => config,
        Err(error) => {
            error!(%error, "failed to resolve daemon configuration");
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    if dry_run {
        info!("running dry-run sync plan");
        return match preview_plan(&config) {
            Ok(plan) => match serde_json::to_string_pretty(&DryRunReport::new(plan)) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    error!(%error, "failed to serialize dry-run report");
                    eprintln!("failed to serialize dry-run report: {error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                error!(%error, "dry-run sync plan failed");
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    match Daemon::new(config) {
        Ok(daemon) => match daemon.run().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error!(%error, "daemon exited with error");
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            error!(%error, "failed to initialize daemon");
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_runtime_config(cli: Cli) -> AppResult<(DaemonConfig, bool)> {
    let file_config = load_file_config(cli.config.as_ref())?;
    let dry_run = if cli.no_dry_run {
        false
    } else if cli.dry_run {
        true
    } else {
        file_config.dry_run.unwrap_or(false)
    };
    let local_root = cli
        .local_root
        .or(file_config.local_root)
        .ok_or_else(|| boxed_error("missing required --local-root or config local_root"))?;
    let remote_root = cli
        .remote_root
        .or(file_config.remote_root)
        .ok_or_else(|| boxed_error("missing required --remote-root or config remote_root"))?;
    let db_path = cli
        .db_path
        .or(file_config.db_path)
        .map(|path| resolve_path(&local_root, path))
        .unwrap_or_else(|| default_state_db_path(&local_root));

    let config = DaemonConfig {
        local_root,
        remote_root,
        db_path,
        socket_path: cli
            .socket_path
            .or(file_config.socket_path)
            .unwrap_or_else(default_socket_path),
        lockfile_path: cli
            .lockfile_path
            .or(file_config.lockfile_path)
            .unwrap_or_else(default_lockfile_path),
        scan_interval: Duration::from_secs(
            cli.scan_interval_secs
                .or(file_config.scan_interval_secs)
                .unwrap_or(300)
                .max(1),
        ),
        proton_cli: cli
            .proton_cli
            .or(file_config.proton_cli)
            .unwrap_or_else(|| PathBuf::from("proton-drive")),
        include_patterns: merge_patterns(cli.include_patterns, file_config.include_patterns),
        exclude_patterns: merge_patterns(cli.exclude_patterns, file_config.exclude_patterns),
    };
    validate_runtime_config(&config)?;

    Ok((config, dry_run))
}

fn resolve_path(local_root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        local_root.join(path)
    }
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

fn load_file_config(path: Option<&PathBuf>) -> AppResult<FileConfig> {
    let Some(path) = path else {
        return Ok(FileConfig::default());
    };
    let config = fs::read_to_string(path).map_err(|error| {
        boxed_error(format!("failed to read config {}: {error}", path.display()))
    })?;
    toml::from_str(&config).map_err(|error| {
        boxed_error(format!(
            "failed to parse config {}: {error}",
            path.display()
        ))
    })
}

fn merge_patterns(cli_patterns: Vec<String>, config_patterns: Option<Vec<String>>) -> Vec<String> {
    if cli_patterns.is_empty() {
        config_patterns.unwrap_or_default()
    } else {
        cli_patterns
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
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
include = ["Documents/**"]
exclude = ["**/*.tmp"]
dry_run = true
"#,
        )
        .expect("write config");

        let (config, dry_run) = resolve_runtime_config(Cli::parse_from([
            "proton-syncd",
            "--config",
            config_path.to_str().expect("config path"),
        ]))
        .expect("runtime config");

        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("sync-root"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/RemoteFolder"));
        assert_eq!(config.db_path, PathBuf::from("sync-root/state/index.db"));
        assert_eq!(config.scan_interval, Duration::from_secs(42));
        assert_eq!(config.proton_cli, PathBuf::from("fake-proton-drive"));
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
include = ["config/**"]
"#,
        )
        .expect("write config");

        let (config, dry_run) = resolve_runtime_config(Cli::parse_from([
            "proton-syncd",
            "--config",
            config_path.to_str().expect("config path"),
            "--local-root",
            "cli-root",
            "--remote-root",
            "/Drive/Cli",
            "--include",
            "cli/**",
            "--dry-run",
        ]))
        .expect("runtime config");

        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("cli-root"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/Cli"));
        assert_eq!(config.include_patterns, vec!["cli/**"]);
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

        let (_, dry_run) = resolve_runtime_config(Cli::parse_from([
            "proton-syncd",
            "--config",
            config_path.to_str().expect("config path"),
            "--no-dry-run",
        ]))
        .expect("runtime config");

        assert!(!dry_run);
    }

    #[test]
    fn invalid_include_glob_returns_targeted_config_error() {
        let error = resolve_runtime_config(Cli::parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--include",
            "[",
        ]))
        .expect_err("invalid include glob should fail");

        assert!(
            error
                .to_string()
                .contains("invalid scan filter configuration"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn empty_local_root_returns_targeted_config_error() {
        let error = resolve_runtime_config(cli_with_roots(
            PathBuf::new(),
            PathBuf::from("/Drive/Config"),
        ))
        .expect_err("empty local root should fail");

        assert_eq!(error.to_string(), "local_root must not be empty");
    }

    #[test]
    fn empty_remote_root_returns_targeted_config_error() {
        let error =
            resolve_runtime_config(cli_with_roots(PathBuf::from("sync-root"), PathBuf::new()))
                .expect_err("empty remote root should fail");

        assert_eq!(error.to_string(), "remote_root must not be empty");
    }

    #[test]
    fn conflicting_dry_run_flags_are_rejected_by_cli_parser() {
        let result = Cli::try_parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--dry-run",
            "--no-dry-run",
        ]);

        assert!(result.is_err(), "conflicting dry-run flags must fail");
    }

    fn cli_with_roots(local_root: PathBuf, remote_root: PathBuf) -> Cli {
        Cli {
            config: None,
            local_root: Some(local_root),
            remote_root: Some(remote_root),
            db_path: None,
            socket_path: None,
            lockfile_path: None,
            scan_interval_secs: None,
            proton_cli: None,
            dry_run: false,
            no_dry_run: false,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
        }
    }
}
