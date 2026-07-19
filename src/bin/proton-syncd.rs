use clap::Parser;
use proton_drive_sync_engine::daemon::{Daemon, DaemonConfig};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

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
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
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
    };

    match Daemon::new(config) {
        Ok(daemon) => match daemon.run().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
