use clap::Parser;
use proton_drive_sync_engine::daemon::{Daemon, DaemonConfig, preview_plan};
use std::path::PathBuf;
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
    local_root: PathBuf,
    #[arg(long)]
    remote_root: PathBuf,
    #[arg(long, default_value = "sync_index.db")]
    db_path: PathBuf,
    #[arg(long, default_value = "/tmp/proton-sync.sock")]
    socket_path: PathBuf,
    #[arg(long, default_value = "/tmp/proton-sync.lock")]
    lockfile_path: PathBuf,
    #[arg(long, default_value_t = 300)]
    scan_interval_secs: u64,
    #[arg(long, default_value = "proton-drive")]
    proton_cli: PathBuf,
    #[arg(long)]
    dry_run: bool,
    #[arg(long = "include", value_name = "GLOB")]
    include_patterns: Vec<String>,
    #[arg(long = "exclude", value_name = "GLOB")]
    exclude_patterns: Vec<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let dry_run = cli.dry_run;
    let db_path = if cli.db_path.is_absolute() {
        cli.db_path
    } else {
        cli.local_root.join(cli.db_path)
    };

    let config = DaemonConfig {
        local_root: cli.local_root,
        remote_root: cli.remote_root,
        db_path,
        socket_path: cli.socket_path,
        lockfile_path: cli.lockfile_path,
        scan_interval: Duration::from_secs(cli.scan_interval_secs.max(1)),
        proton_cli: cli.proton_cli,
        include_patterns: cli.include_patterns,
        exclude_patterns: cli.exclude_patterns,
    };

    if dry_run {
        info!("running dry-run sync plan");
        return match preview_plan(&config) {
            Ok(plan) => match serde_json::to_string_pretty(&plan) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    error!(%error, "failed to serialize dry-run plan");
                    eprintln!("failed to serialize dry-run plan: {error}");
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

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
