use clap::Parser;
use proton_drive_sync_engine::config::{DaemonConfigInput, resolve_runtime_config};
use proton_drive_sync_engine::daemon::{Daemon, preview_plan};
use proton_drive_sync_engine::sync::DryRunReport;
use std::path::PathBuf;
use std::process::ExitCode;
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
    proton_timeout_secs: Option<u64>,
    #[arg(long)]
    proton_list_attempts: Option<usize>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long = "no-dry-run", conflicts_with = "dry_run")]
    no_dry_run: bool,
    #[arg(long = "include", value_name = "GLOB")]
    include_patterns: Vec<String>,
    #[arg(long = "exclude", value_name = "GLOB")]
    exclude_patterns: Vec<String>,
    /// Detect remote changes from Proton's volume event stream instead of a full-tree walk.
    /// This is the default; the flag is kept for explicitness and to override a config-file
    /// `events_driven = false`.
    #[arg(long = "events-driven")]
    events_driven: bool,
    /// Opt out of event-driven detection and use full-tree-walk-only remote change detection.
    #[arg(long = "no-events-driven", conflicts_with = "events_driven")]
    no_events_driven: bool,
    /// Force a full-tree resync every N incremental passes (event-driven mode only).
    #[arg(long = "events-full-scan-every", value_name = "N")]
    events_full_scan_every: Option<u64>,
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let (config, dry_run) = match resolve_runtime_config(cli.into()) {
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

impl From<Cli> for DaemonConfigInput {
    fn from(cli: Cli) -> Self {
        Self {
            config: cli.config,
            local_root: cli.local_root,
            remote_root: cli.remote_root,
            db_path: cli.db_path,
            socket_path: cli.socket_path,
            lockfile_path: cli.lockfile_path,
            scan_interval_secs: cli.scan_interval_secs,
            proton_cli: cli.proton_cli,
            proton_timeout_secs: cli.proton_timeout_secs,
            proton_list_attempts: cli.proton_list_attempts,
            dry_run: cli.dry_run,
            no_dry_run: cli.no_dry_run,
            include_patterns: cli.include_patterns,
            exclude_patterns: cli.exclude_patterns,
            events_driven: cli.events_driven,
            no_events_driven: cli.no_events_driven,
            events_full_scan_every: cli.events_full_scan_every,
        }
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

    #[test]
    fn proton_policy_flags_parse_into_config_input() {
        let cli = Cli::parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--proton-timeout-secs",
            "12",
            "--proton-list-attempts",
            "5",
        ]);

        let input = DaemonConfigInput::from(cli);

        assert_eq!(input.proton_timeout_secs, Some(12));
        assert_eq!(input.proton_list_attempts, Some(5));
    }
}
