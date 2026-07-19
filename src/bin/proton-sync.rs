use clap::{Parser, Subcommand};
use proton_drive_sync_engine::ipc::{ControlCommand, send_command};
use proton_drive_sync_engine::paths::default_socket_path;
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
    Pause,
    Resume,
    Syncnow,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let socket_path = cli.socket_path.unwrap_or_else(default_socket_path);
    let command = match cli.command {
        Commands::Status => ControlCommand::Status,
        Commands::Pause => ControlCommand::Pause,
        Commands::Resume => ControlCommand::Resume,
        Commands::Syncnow => ControlCommand::Syncnow,
    };

    match send_command(&socket_path, command).await {
        Ok(response) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&response).expect("serialize response")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
