use clap::{Parser, Subcommand};
use proton_drive_sync_engine::ipc::{
    ControlCommand, ControlRequest, ControlResponse, PendingDeletion, send_request,
};
use proton_drive_sync_engine::paths::default_socket_path;
use proton_drive_sync_engine::sync::{DeleteDirection, PlanSummary};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Client-side bound on a single control-socket round trip. The daemon answers every request
/// from an in-memory snapshot, so anything slower than this means it is not actually serving.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Poll cadence while `syncnow` waits for the scheduled pass to finish.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Debug, Parser)]
#[command(
    name = "proton-sync",
    about = "Frontend controller for the Proton Drive sync daemon"
)]
struct Cli {
    #[arg(long)]
    socket_path: Option<PathBuf>,
    /// Print the daemon's raw JSON response instead of the human-readable output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Show what the sync daemon is doing right now.
    Status,
    /// Show the recent sync history, newest first. (--json prints the daemon's raw
    /// status_history array instead, which is stored oldest-first.)
    History,
    /// Pause syncing (edits are still tracked while paused).
    Pause,
    /// Resume syncing.
    Resume,
    /// Trigger a sync and watch it finish.
    Syncnow {
        /// Schedule the sync and return immediately instead of waiting for it to finish.
        #[arg(long)]
        no_wait: bool,
    },
    /// Ask the running daemon to exit gracefully.
    Stop,
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
    let style = Style::for_stdout();

    let request = match build_request(&cli.command) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let response = match request_with_timeout(&socket_path, request).await {
        Ok(response) => response,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match &cli.command {
        Commands::Status => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                print_status(&response, &style);
            }
            ExitCode::SUCCESS
        }
        Commands::History => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response.status_history)
                        .expect("serialize status history")
                );
            } else {
                print_history(&response, &style);
            }
            ExitCode::SUCCESS
        }
        Commands::Pause => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!("Sync paused. Edits are still tracked; resume with `proton-sync resume`.");
            }
            ExitCode::SUCCESS
        }
        Commands::Resume => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!("Sync resumed.");
            }
            ExitCode::SUCCESS
        }
        Commands::Syncnow { no_wait } => {
            watch_syncnow(&socket_path, response, *no_wait, cli.json, &style).await
        }
        Commands::Stop => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!("Shutdown requested; the daemon is exiting.");
            }
            ExitCode::SUCCESS
        }
        Commands::Pending => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response.pending_deletions)
                        .expect("serialize pending deletions")
                );
            } else {
                print_pending(&response.pending_deletions);
            }
            ExitCode::SUCCESS
        }
        Commands::Approve { .. } | Commands::Deny { .. } => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!("{}", response.message);
            }
            ExitCode::SUCCESS
        }
    }
}

/// Maps a subcommand to the control request to send, validating the `approve`/`deny` selector.
fn build_request(command: &Commands) -> Result<ControlRequest, String> {
    let (control_command, argument) = match command {
        Commands::Status | Commands::History | Commands::Pending => (ControlCommand::Status, None),
        Commands::Pause => (ControlCommand::Pause, None),
        Commands::Resume => (ControlCommand::Resume, None),
        Commands::Syncnow { .. } => (ControlCommand::Syncnow, None),
        Commands::Stop => (ControlCommand::Shutdown, None),
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

/// One time-bounded round trip, with connection failures mapped to an actionable message.
async fn request_with_timeout(
    socket_path: &Path,
    request: ControlRequest,
) -> Result<ControlResponse, String> {
    match tokio::time::timeout(REQUEST_TIMEOUT, send_request(socket_path, request)).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(format!(
            "cannot reach the sync daemon at {}: {error}\nIs it running? Start it with: systemctl --user start proton-syncd",
            socket_path.display()
        )),
        Err(_elapsed) => Err(format!(
            "the sync daemon at {} did not answer within {}s",
            socket_path.display(),
            REQUEST_TIMEOUT.as_secs()
        )),
    }
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

// ---- human-readable output --------------------------------------------------------------------

/// Minimal ANSI styling, enabled only when stdout is a terminal (like git's auto color mode).
struct Style {
    enabled: bool,
}

impl Style {
    fn for_stdout() -> Self {
        Self {
            enabled: std::io::stdout().is_terminal(),
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }
    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
    fn red(&self, text: &str) -> String {
        self.paint("31", text)
    }
    fn green(&self, text: &str) -> String {
        self.paint("32", text)
    }
    fn yellow(&self, text: &str) -> String {
        self.paint("33", text)
    }
    fn cyan(&self, text: &str) -> String {
        self.paint("36", text)
    }
}

fn print_pretty_json(response: &ControlResponse) {
    println!(
        "{}",
        serde_json::to_string_pretty(response).expect("serialize response")
    );
}

/// `proton-sync status` — a compact, git-style summary instead of raw JSON.
///
/// ```text
/// ● syncing — 2 uploads, 1 download planned
///   folders    ~/ProtonDrive ⇄ /Drive/RemoteFolder
///   last sync  2m ago
///   changes    3 queued locally
///   deletions  1 awaiting approval — review with `proton-sync pending`
/// ```
fn print_status(response: &ControlResponse, style: &Style) {
    let (dot, state, detail) = headline(response, style);
    println!("{dot} {} — {detail}", style.bold(state));

    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(config) = &response.config {
        rows.push((
            "folders",
            format!(
                "{} ⇄ {}",
                config.local_root.display(),
                config.remote_root.display()
            ),
        ));
    }
    rows.push((
        "last sync",
        match response.last_sync_epoch_secs {
            Some(epoch) => relative_time(epoch),
            None => "never".to_owned(),
        },
    ));
    if response.pending_changes > 0 {
        rows.push((
            "changes",
            format!("{} queued locally", response.pending_changes),
        ));
    }
    if !response.pending_deletions.is_empty() {
        rows.push((
            "deletions",
            format!(
                "{} awaiting approval — review with `proton-sync pending`",
                response.pending_deletions.len()
            ),
        ));
    }
    if let Some(error) = &response.last_error {
        rows.push(("error", style.red(error)));
    }

    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, value) in rows {
        println!("  {} {value}", style.dim(&format!("{label:<width$}")));
    }
}

/// The status headline: a coloured state dot, the state word, and a one-line detail.
fn headline(response: &ControlResponse, style: &Style) -> (String, &'static str, String) {
    if response.paused && response.syncing {
        // A pause accepted mid-pass: the in-flight pass still runs to completion, then the
        // daemon holds. Saying "paused — stopped" here would be untrue while transfers run.
        return (
            style.yellow("●"),
            "pausing",
            "finishing the current pass, then holding".to_owned(),
        );
    }
    if response.paused {
        return (
            style.yellow("●"),
            "paused",
            "syncing is stopped; edits are still tracked".to_owned(),
        );
    }
    if response.syncing {
        let detail = response
            .last_plan_summary
            .as_ref()
            .map(|summary| format!("{} planned", summarize_plan(summary)))
            .unwrap_or_else(|| "reconciling changes".to_owned());
        return (style.cyan("●"), "syncing", detail);
    }
    if response.last_error.is_some() {
        return (
            style.red("●"),
            "error",
            "the last sync failed; details below".to_owned(),
        );
    }
    if response.pending_changes > 0 {
        return (
            style.green("●"),
            "running",
            format!(
                "{} local change(s) waiting for the next pass",
                response.pending_changes
            ),
        );
    }
    (
        style.green("●"),
        "idle",
        "everything is up to date".to_owned(),
    )
}

/// `proton-sync history` — one line per recorded pass, newest first.
fn print_history(response: &ControlResponse, style: &Style) {
    if response.status_history.is_empty() {
        println!("No sync history yet.");
        return;
    }
    for entry in response.status_history.iter().rev() {
        let when = style.dim(&format!("{:>8}", relative_time(entry.epoch_secs)));
        match &entry.last_error {
            Some(error) => {
                println!(
                    "{when}  {} {}",
                    style.red("✗"),
                    format_args!("{} — {error}", entry.message)
                );
            }
            None => {
                let summary = entry
                    .successful_sync_summary
                    .as_ref()
                    .map(|summary| format!(" — {}", summarize_plan(summary)))
                    .unwrap_or_default();
                println!("{when}  {} {}{summary}", style.green("✓"), entry.message);
            }
        }
    }
}

/// "2 uploads, 1 download, 1 conflict" — only the non-zero, user-meaningful counters.
fn summarize_plan(summary: &PlanSummary) -> String {
    fn push(parts: &mut Vec<String>, count: usize, singular: &str, plural: &str) {
        if count > 0 {
            let noun = if count == 1 { singular } else { plural };
            parts.push(format!("{count} {noun}"));
        }
    }
    let mut parts = Vec::new();
    push(&mut parts, summary.uploads, "upload", "uploads");
    push(&mut parts, summary.downloads, "download", "downloads");
    push(
        &mut parts,
        summary.local_moves + summary.remote_moves,
        "move",
        "moves",
    );
    push(
        &mut parts,
        summary.conflicts + summary.type_conflicts,
        "conflict",
        "conflicts",
    );
    push(
        &mut parts,
        summary.remote_deletes + summary.local_deletes,
        "delete",
        "deletes",
    );
    push(
        &mut parts,
        summary.skipped_unsupported,
        "skipped item",
        "skipped items",
    );
    if parts.is_empty() {
        "nothing to transfer".to_owned()
    } else {
        parts.join(", ")
    }
}

fn relative_time(epoch_secs: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let delta = now.saturating_sub(epoch_secs);
    if delta < 5 {
        "just now".to_owned()
    } else if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

// ---- syncnow ----------------------------------------------------------------------------------

/// The daemon acks `syncnow` immediately (the sync runs on its main loop); by default the CLI
/// then watches status until that pass completes, so `proton-sync syncnow` still reads like a
/// synchronous command — without ever freezing the daemon's control socket for other clients.
async fn watch_syncnow(
    socket_path: &Path,
    ack: ControlResponse,
    no_wait: bool,
    json: bool,
    style: &Style,
) -> ExitCode {
    let scheduled = ack.message == "sync scheduled" || ack.message.contains("already in progress");
    if !scheduled || no_wait {
        // Paused, shutting down, --no-wait, or an older (blocking) daemon that already synced:
        // nothing to watch — report the ack itself.
        if json {
            print_pretty_json(&ack);
        } else {
            println!("{}", ack.message);
        }
        return ExitCode::SUCCESS;
    }

    // The ack carries the count of *completed* passes. Our scheduled pass is the next one —
    // or the one after, when a pass was already in flight at request time (that pass predates
    // the request, so it cannot be the one we asked for).
    let target_seq = ack.reconcile_seq
        + if ack.message.contains("already in progress") {
            2
        } else {
            1
        };
    let spinner = Spinner::for_stderr();
    let started = Instant::now();
    let mut consecutive_errors = 0u32;
    let outcome = loop {
        tokio::time::sleep(WAIT_POLL_INTERVAL).await;
        let request = ControlRequest {
            command: ControlCommand::Status,
            argument: None,
        };
        match request_with_timeout(socket_path, request).await {
            Ok(status) => {
                consecutive_errors = 0;
                if status.reconcile_seq >= target_seq && !status.syncing {
                    break Ok(status);
                }
                // Paused with our pass neither started nor finished: it will never run (the
                // daemon skips scheduled syncs while paused), so stop waiting rather than spin.
                if status.paused && !status.syncing {
                    spinner.clear();
                    println!("sync was paused before the scheduled pass ran; resume and retry");
                    return ExitCode::FAILURE;
                }
                spinner.tick(&started, &status);
            }
            Err(message) => {
                consecutive_errors += 1;
                if consecutive_errors >= 5 {
                    break Err(message);
                }
            }
        }
    };
    spinner.clear();

    match outcome {
        Ok(status) => {
            if json {
                print_pretty_json(&status);
                return if status.last_error.is_none() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                };
            }
            match &status.last_error {
                None => {
                    let summary = status
                        .last_successful_sync_summary
                        .as_ref()
                        .map(summarize_plan)
                        .unwrap_or_else(|| "nothing to transfer".to_owned());
                    println!("{} sync completed — {summary}", style.green("✓"));
                    ExitCode::SUCCESS
                }
                Some(error) => {
                    println!("{} sync failed: {error}", style.red("✗"));
                    ExitCode::FAILURE
                }
            }
        }
        Err(message) => {
            eprintln!("lost contact with the daemon while waiting: {message}");
            ExitCode::FAILURE
        }
    }
}

/// A single-line stderr spinner for the `syncnow` wait, shown only on a terminal.
struct Spinner {
    interactive: bool,
    frames: &'static [&'static str],
    frame: std::cell::Cell<usize>,
}

impl Spinner {
    fn for_stderr() -> Self {
        Self {
            interactive: std::io::stderr().is_terminal(),
            frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            frame: std::cell::Cell::new(0),
        }
    }

    fn tick(&self, started: &Instant, status: &ControlResponse) {
        if !self.interactive {
            return;
        }
        let glyph = self.frames[self.frame.get() % self.frames.len()];
        self.frame.set(self.frame.get() + 1);
        let detail = status
            .last_plan_summary
            .as_ref()
            .filter(|_| status.syncing)
            .map(|summary| format!(" ({})", summarize_plan(summary)))
            .unwrap_or_default();
        eprint!(
            "\r\x1b[2K{glyph} syncing… {}s{detail}",
            started.elapsed().as_secs()
        );
        let _ = std::io::stderr().flush();
    }

    fn clear(&self) {
        if self.interactive {
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
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
