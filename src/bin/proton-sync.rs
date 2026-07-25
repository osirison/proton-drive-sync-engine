use clap::{Parser, Subcommand};
use proton_drive_sync_engine::ipc::{
    ControlCommand, ControlRequest, PendingDeletion, send_request,
};
use proton_drive_sync_engine::paths::default_socket_path;
use proton_drive_sync_engine::sync::DeleteDirection;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "proton-sync",
    about = "Frontend controller for the Proton Drive sync daemon"
)]
struct Cli {
    #[arg(long)]
    socket_path: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Status,
    History,
    Pause,
    Resume,
    Syncnow,
    /// List deletions currently withheld by the delete-approval guard, awaiting approval.
    Pending,
    /// Approve withheld deletions so they apply on the next sync.
    Approve {
        /// Relative path of the pending deletion to approve (as shown by `pending`).
        path: Option<PathBuf>,
        /// Approve every currently-pending deletion.
        #[arg(long)]
        all: bool,
    },
    /// Revoke a prior approval before it has applied.
    Deny {
        /// Relative path of the approval to revoke.
        path: Option<PathBuf>,
        /// Revoke approval for every currently-pending deletion.
        #[arg(long)]
        all: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let socket_path = cli.socket_path.clone().unwrap_or_else(default_socket_path);

    let request = match build_request(&cli.command) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match send_request(&socket_path, request).await {
        Ok(response) => {
            match &cli.command {
                Commands::History => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&response.status_history)
                            .expect("serialize status history")
                    );
                }
                Commands::Pending => print_pending(&response.pending_deletions),
                Commands::Approve { .. } | Commands::Deny { .. } => {
                    println!("{}", response.message)
                }
                _ => println!(
                    "{}",
                    serde_json::to_string_pretty(&response).expect("serialize response")
                ),
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Maps a subcommand to the control request to send, validating the `approve`/`deny` selector.
fn build_request(command: &Commands) -> Result<ControlRequest, String> {
    let (control_command, argument) = match command {
        Commands::Status | Commands::History | Commands::Pending => (ControlCommand::Status, None),
        Commands::Pause => (ControlCommand::Pause, None),
        Commands::Resume => (ControlCommand::Resume, None),
        Commands::Syncnow => (ControlCommand::Syncnow, None),
        Commands::Approve { path, all } => {
            (ControlCommand::Approve, approval_selector(path, *all)?)
        }
        Commands::Deny { path, all } => (ControlCommand::Deny, approval_selector(path, *all)?),
    };
    Ok(ControlRequest {
        command: control_command,
        argument,
    })
}

/// Turns the `<PATH> | --all` selector into the request argument, rejecting the ambiguous or empty
/// cases so a bare `approve` never silently approves everything.
fn approval_selector(path: &Option<PathBuf>, all: bool) -> Result<Option<String>, String> {
    match (path, all) {
        (Some(_), true) => Err("specify either a PATH or --all, not both".to_owned()),
        (Some(path), false) => Ok(Some(path.to_string_lossy().into_owned())),
        (None, true) => Ok(Some("all".to_owned())),
        (None, false) => {
            Err("specify a PATH, or --all to act on every pending deletion".to_owned())
        }
    }
}

fn print_pending(pending: &[PendingDeletion]) {
    if pending.is_empty() {
        println!("No deletions are pending approval.");
        return;
    }
    println!("{} deletion(s) awaiting approval:", pending.len());
    for item in pending {
        let (label, effect) = match item.direction {
            DeleteDirection::Local => (
                "LOCAL DELETE ",
                "was deleted on Proton Drive; approving removes your local copy",
            ),
            DeleteDirection::Remote => (
                "REMOTE DELETE",
                "was deleted locally; approving removes it on Proton Drive",
            ),
        };
        println!("  {label}  {}  ({effect})", item.path.display());
    }
    println!("Approve with: proton-sync approve <path>   (or --all)");
}
