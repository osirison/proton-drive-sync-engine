use clap::{Parser, Subcommand};
use proton_drive_sync_engine::ipc::{ControlCommand, send_command};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "proton-sync",
    about = "Frontend controller for the Proton Drive sync daemon"
)]
struct Cli {
    #[arg(long, default_value = "/tmp/proton-sync.sock")]
    socket_path: PathBuf,
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
    let command = match cli.command {
        Commands::Status => ControlCommand::Status,
        Commands::Pause => ControlCommand::Pause,
        Commands::Resume => ControlCommand::Resume,
        Commands::Syncnow => ControlCommand::Syncnow,
    };

    match send_command(&cli.socket_path, command).await {
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
