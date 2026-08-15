use clap::{Parser, Subcommand};
use proton_drive_sync_engine::config::resolve_control_socket_path;
use proton_drive_sync_engine::index::PassRecord;
use proton_drive_sync_engine::ipc::{
    ControlCommand, ControlRequest, ControlResponse, PendingDeletion, SyncActivity, send_request,
    wire_path,
};
use proton_drive_sync_engine::sync::{DeleteDirection, PlanSummary, SyncAction, UnsyncableItem};
use std::collections::BTreeMap;
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
    /// Daemon config file to read the control socket's location from, so a file-configured
    /// `socket_path` does not have to be repeated as `--socket-path` on every invocation (#63).
    /// Only `socket_path` is read here; everything else in the file belongs to the daemon.
    #[arg(long)]
    config: Option<PathBuf>,
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
    /// Show the recorded sync passes, newest first: how long each took, which strategy it ran,
    /// and how it ended. Idle passes are not recorded, so these are passes that did something.
    History,
    /// Show what has moved recently, newest first — or one path's own history when PATH is given.
    Activity {
        /// Relative path (as shown by `status`) to show the history of. Omit for the global feed.
        path: Option<PathBuf>,
        /// How far back to look. Omit for everything the daemon still keeps.
        #[arg(long)]
        days: Option<u64>,
        /// Cap on the rows shown.
        #[arg(long)]
        limit: Option<usize>,
    },
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
    /// Force a full remote re-scan on the daemon's next pass (instead of the fast warm start),
    /// e.g. to self-heal suspected drift. Returns immediately; watch with `proton-sync status`.
    Resync,
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
    // Same precedence and same validation as the daemon (`--socket-path` > config file >
    // XDG default), through the engine's own resolver so the two cannot drift (#63). `main`
    // returns ExitCode, so the error is reported here rather than propagated.
    let socket_path =
        match resolve_control_socket_path(cli.socket_path.clone(), cli.config.as_deref()) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("cannot resolve the control socket path: {error}");
                return ExitCode::FAILURE;
            }
        };
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
                    serde_json::to_string_pretty(&response.history)
                        .expect("serialize pass history")
                );
            } else {
                print_history(&response, &style);
            }
            ExitCode::SUCCESS
        }
        Commands::Activity { .. } => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response.file_history)
                        .expect("serialize file history")
                );
            } else {
                print_activity(&response, &style);
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
        Commands::Resync => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!("{}", response.message);
            }
            ExitCode::SUCCESS
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
        // `history` reads the pass log, which rides on every status reply — no second verb.
        Commands::Status | Commands::History | Commands::Pending => (ControlCommand::Status, None),
        Commands::Activity { path, .. } => (
            ControlCommand::Activity,
            path.as_ref().map(|path| wire_path(path).into_owned()),
        ),
        Commands::Pause => (ControlCommand::Pause, None),
        Commands::Resume => (ControlCommand::Resume, None),
        Commands::Syncnow { .. } => (ControlCommand::Syncnow, None),
        Commands::Resync => (ControlCommand::Resync, None),
        Commands::Stop => (ControlCommand::Shutdown, None),
        Commands::Approve { path, all } => {
            (ControlCommand::Approve, approval_selector(path, *all)?)
        }
        Commands::Deny { path, all } => (ControlCommand::Deny, approval_selector(path, *all)?),
    };
    // A `<PATH>` selector is always a literal path on the wire: `proton-sync approve all`
    // targets a pending deletion literally named `all` instead of silently becoming the
    // every-item form (which requires the explicit `--all`, exactly as documented).
    let (window_secs, limit) = match command {
        // Days, not seconds, at the CLI: the reply's window is in seconds because a GUI slider is
        // not a calendar.
        Commands::Activity { days, limit, .. } => {
            (days.map(|days| days.saturating_mul(86_400)), *limit)
        }
        _ => (None, None),
    };
    let literal_path = matches!(
        command,
        Commands::Approve {
            path: Some(_),
            all: false
        } | Commands::Deny {
            path: Some(_),
            all: false
        }
    );
    Ok(ControlRequest {
        argument,
        literal_path,
        window_secs,
        limit,
        ..ControlRequest::new(control_command)
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
        // `wire_path`, not an ad-hoc `to_string_lossy`: the daemon matches selectors in exactly
        // this form, so the two sides must name the same function (#61).
        (Some(path), false) => Ok(Some(wire_path(path).into_owned())),
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
    if response.syncing
        && let Some(activity) = &response.activity
    {
        rows.push(("activity", describe_activity(activity)));
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
    if response.failed_item_count > 0 {
        rows.push((
            "failed",
            style.red(&format!(
                "{} item(s) did not sync; everything else did",
                response.failed_item_count
            )),
        ));
    }
    if !response.unsyncable.is_empty() {
        rows.push((
            "can't sync",
            format!("{} item(s) — listed below", response.unsyncable.len()),
        ));
    }
    if let Some(error) = &response.last_error {
        rows.push(("error", style.red(error)));
    }

    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, value) in rows {
        println!("  {} {value}", style.dim(&format!("{label:<width$}")));
    }
    // The per-item detail behind the `failed` row (#136). Bounded by the daemon, so a pass that
    // failed thousands of items still prints a handful plus the count above.
    for item in &response.failed_items {
        println!(
            "    {} {} — {}",
            style.red("✗"),
            item.path.display(),
            item.error
        );
    }
    print_unsyncable(&response.unsyncable, style);
}

/// The entities the daemon cannot sync, by name and cause. A count alone is unactionable: the
/// account behind #295 carried one such file for five months, and `skipped_unsupported: 1` was the
/// only trace of it. Grouped by cause so a Docs-heavy account reads as one fact, not N.
fn print_unsyncable(items: &[UnsyncableItem], style: &Style) {
    if items.is_empty() {
        return;
    }
    println!();
    println!(
        "{}",
        style.bold(&format!("{} item(s) cannot be synced:", items.len()))
    );
    // `reason` is a wire token an older client may not know, so group on the token itself and
    // render an unfamiliar one verbatim rather than dropping the row.
    let mut groups: BTreeMap<&str, Vec<&UnsyncableItem>> = BTreeMap::new();
    for item in items {
        groups.entry(item.reason.as_str()).or_default().push(item);
    }
    for (token, group) in groups {
        println!(
            "  {}",
            style.dim(&format!("{token} — {}", group[0].reason.describe()))
        );
        for item in group {
            println!(
                "    {}  {}",
                item.path.display(),
                style.dim(&format!(
                    "(since {})",
                    relative_time(item.first_seen_epoch_secs)
                ))
            );
        }
    }
    println!(
        "{}",
        style.dim(
            "These are skipped on every pass; nothing transfers in either direction until the \
             cause changes."
        )
    );
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
    // Before the error arm, and that placement is the whole point (#136/#246): a partial pass
    // sets `last_error` too, so an `error`-first order would report a pass that synced everything
    // but three files as a sync that did not happen.
    if response.failed_item_count > 0 {
        return (
            style.yellow("●"),
            "partial",
            format!(
                "the last sync completed with {} failed item(s); details below",
                response.failed_item_count
            ),
        );
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
///
/// Reads the durable pass log, NOT `status_history`: with event-driven detection on by default the
/// daemon runs a pass every 30s and records every one of them in that rolling 20-entry trail, so
/// it holds about ten minutes and is almost entirely idle polls. The pass log records only passes
/// that did something (plus every full sweep, and every pass that did not end clean).
fn print_history(response: &ControlResponse, style: &Style) {
    let Some(history) = &response.history else {
        println!("No sync history yet.");
        return;
    };
    if let Some(sweep) = &history.last_full_sweep {
        println!(
            "Last full sweep {} — {}.",
            relative_time(sweep.started_epoch_secs),
            if sweep.changed == 0 {
                "nothing was out of step".to_owned()
            } else {
                format!("{} change(s)", sweep.changed)
            }
        );
    }
    if history.today.uploaded_bytes > 0 || history.today.downloaded_bytes > 0 {
        println!(
            "Today: {} sent · {} received.",
            human_bytes(history.today.uploaded_bytes),
            human_bytes(history.today.downloaded_bytes)
        );
    }
    if history.recent.is_empty() {
        println!("No passes recorded yet (an idle pass records nothing).");
        return;
    }
    println!();
    for pass in &history.recent {
        let when = style.dim(&format!("{:>8}", relative_time(pass.started_epoch_secs)));
        let took = style.dim(&format!("{:>7}", human_duration(pass.duration_ms)));
        let kind = style.dim(&format!("{:>11}", pass.kind));
        // Four outcomes, four arms, and an explicit `other` for a token a newer daemon knows and
        // this build does not — never a trailing arm that quietly renders one state as another.
        let (mark, detail) = match pass.outcome.as_str() {
            "clean" => (style.green("✓"), describe_pass_work(pass)),
            "partial" => (
                style.yellow("!"),
                format!(
                    "{} — {} item(s) failed",
                    describe_pass_work(pass),
                    pass.failed
                ),
            ),
            "failed" => (
                style.red("✗"),
                pass.error.clone().unwrap_or_else(|| "failed".to_owned()),
            ),
            "interrupted" => (
                style.yellow("!"),
                "interrupted — the daemon stopped mid-pass".to_owned(),
            ),
            other => (style.dim("?"), other.to_owned()),
        };
        println!("{when} {took} {kind}  {mark} {detail}");
    }
}

/// "3 changes, 1.2 MB sent" — what one recorded pass actually moved.
fn describe_pass_work(pass: &PassRecord) -> String {
    if pass.changed == 0 {
        return "nothing to do".to_owned();
    }
    let mut parts = vec![format!("{} change(s)", pass.changed)];
    if pass.bytes_uploaded > 0 {
        parts.push(format!("{} sent", human_bytes(pass.bytes_uploaded)));
    }
    if pass.bytes_downloaded > 0 {
        parts.push(format!("{} received", human_bytes(pass.bytes_downloaded)));
    }
    parts.join(", ")
}

/// `proton-sync activity [PATH]` — the per-file feed, newest first.
fn print_activity(response: &ControlResponse, style: &Style) {
    let Some(history) = &response.file_history else {
        println!(
            "{}",
            if response.last_error.is_some() {
                response.message.clone()
            } else {
                "This daemon does not report per-file activity.".to_owned()
            }
        );
        return;
    };
    if history.events.is_empty() {
        println!("Nothing has moved in that window.");
        return;
    }
    for event in &history.events {
        let when = style.dim(&format!("{:>8}", relative_time(event.epoch_secs)));
        let size = match event.bytes {
            Some(bytes) => style.dim(&format!(" ({})", human_bytes(bytes))),
            None => String::new(),
        };
        let path = match &event.source_path {
            Some(from) => format!("{} ← {}", wire_path(&event.path), wire_path(from)),
            None => wire_path(&event.path).into_owned(),
        };
        println!("{when}  {:<24} {path}{size}", describe_action(event.action));
    }
    println!();
    // The byte clause is present only for the unfiltered feed: a path-filtered reply carries no
    // totals, because the window-wide number would read as that one file's.
    let traffic = match &history.totals {
        Some(totals) => format!(
            " · {} sent · {} received",
            human_bytes(totals.uploaded_bytes),
            human_bytes(totals.downloaded_bytes)
        ),
        None => String::new(),
    };
    println!(
        "{}",
        style.dim(&format!(
            "{} file(s), {} event(s) in this window{traffic}",
            history.files, history.total
        ))
    );
}

/// The user-facing sentence for one recorded action. Every variant, no fall-through.
fn describe_action(action: SyncAction) -> &'static str {
    match action {
        SyncAction::Upload => "sent to Proton Drive",
        SyncAction::Download => "brought to this computer",
        SyncAction::CreateRemoteDirectory => "folder made on Proton",
        SyncAction::CreateLocalDirectory => "folder made here",
        SyncAction::MoveLocal => "moved here",
        SyncAction::MoveRemote => "moved on Proton Drive",
        SyncAction::Conflict => "both sides changed",
        SyncAction::TypeConflict => "file/folder clash",
        SyncAction::RemoteDelete => "removed from Proton",
        SyncAction::LocalDelete => "removed here",
        // Never recorded (no side effect), listed so a future change to that rule shows up here.
        SyncAction::AutoLink => "linked",
        SyncAction::Purge => "forgotten",
        SyncAction::SkipUnsupported => "skipped",
    }
}

/// "1.4s" / "830ms" — a pass duration at the precision it deserves.
fn human_duration(millis: u64) -> String {
    if millis < 1000 {
        format!("{millis}ms")
    } else if millis < 60_000 {
        format!("{:.1}s", millis as f64 / 1000.0)
    } else {
        format!("{}m{:02}s", millis / 60_000, (millis % 60_000) / 1000)
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

/// One live line for the daemon's current activity (`status`, and the `syncnow` spinner).
///
/// ```text
/// listing remote folders — 214 listed · in Companies/Acme
/// scanning local files — 1,204 seen · at Photos/2024/IMG_1834.jpg
/// downloading Companies/takeout.tgz — 1.4 GiB so far · 3m12s [step 812/6377]
/// uploading docs/report.pdf — 4.2 MiB · 12s [step 5/6377]
/// ```
fn describe_activity(activity: &SyncActivity) -> String {
    let step = match (activity.action_index, activity.action_total) {
        (Some(index), Some(total)) => format!(" [step {index}/{total}]"),
        _ => String::new(),
    };
    // Elapsed time in the current phase — the walk and scan can each run for many minutes,
    // and the growing clock is what shows they are alive even between per-item updates.
    let phase_elapsed = activity
        .since_epoch_secs
        .map(elapsed_label)
        .unwrap_or_default();
    match activity.phase.as_str() {
        "scanning-local" => {
            let seen = activity
                .files_scanned
                .map(|count| format!(" — {count} seen"))
                .unwrap_or_default();
            let at = activity
                .detail
                .as_ref()
                .map(|path| format!(" · at {path}"))
                .unwrap_or_default();
            format!("scanning local files{seen}{at}{phase_elapsed}")
        }
        "listing-remote" => {
            let listed = activity
                .folders_listed
                .map(|count| format!(" — {count} listed"))
                .unwrap_or_default();
            let along = activity
                .detail
                .as_ref()
                .map(|path| format!(" · in {path}"))
                .unwrap_or_default();
            format!("listing remote folders{listed}{along}{phase_elapsed}")
        }
        "fetching-events" => format!("checking the remote change feed{phase_elapsed}"),
        "committing" => format!("committing the sync index{phase_elapsed}"),
        "executing" => {
            if let Some(transfer) = &activity.transfer {
                let verb = if transfer.direction == "upload" {
                    "uploading"
                } else {
                    "downloading"
                };
                let progress = match (transfer.bytes_done, transfer.bytes_total) {
                    (Some(done), Some(total)) if total > 0 => format!(
                        " — {} / {} ({}%)",
                        human_bytes(done),
                        human_bytes(total),
                        (done.min(total)) * 100 / total
                    ),
                    (Some(done), _) => format!(" — {} so far", human_bytes(done)),
                    (None, Some(total)) => format!(" — {}", human_bytes(total)),
                    (None, None) => String::new(),
                };
                let elapsed = elapsed_label(transfer.started_epoch_secs);
                format!(
                    "{verb} {}{progress}{elapsed}{step}",
                    transfer.path.display()
                )
            } else {
                // Non-transfer actions (directory creation, moves, deletes) can still take
                // noticeable time — keep the elapsed clock ticking for them too.
                let what = activity
                    .detail
                    .clone()
                    .unwrap_or_else(|| "applying planned actions".to_owned());
                format!("{what}{phase_elapsed}{step}")
            }
        }
        // Unknown phase from a newer daemon: show the raw token (plus detail) rather than hide it.
        other => match &activity.detail {
            Some(detail) => format!("{other} · {detail}"),
            None => other.to_owned(),
        },
    }
}

/// ` · 3m12s` since `epoch_secs`, empty within the first couple of seconds.
fn elapsed_label(epoch_secs: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let delta = now.saturating_sub(epoch_secs);
    if delta < 3 {
        String::new()
    } else if delta < 60 {
        format!(" · {delta}s")
    } else if delta < 3600 {
        format!(" · {}m{:02}s", delta / 60, delta % 60)
    } else {
        format!(" · {}h{:02}m", delta / 3600, (delta % 3600) / 60)
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
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
        let request = ControlRequest::new(ControlCommand::Status);
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
                // A partial pass sets `last_error` too, but say so explicitly rather than rely on
                // that: "some items failed" is its own outcome, and it is not a success.
                return if status.last_error.is_none() && status.failed_item_count == 0 {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                };
            }
            // Three outcomes, three arms (#136). A partial pass exits non-zero — something did
            // not sync — but says which state it is in, and names the items.
            if status.failed_item_count > 0 {
                println!(
                    "{} sync completed with {} failed item(s):",
                    style.yellow("!"),
                    status.failed_item_count
                );
                for item in &status.failed_items {
                    println!(
                        "  {} {} — {}",
                        style.red("✗"),
                        item.path.display(),
                        item.error
                    );
                }
                return ExitCode::FAILURE;
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
        // Prefer the live activity ("downloading X — 1.4 GiB so far") over the static plan
        // summary, so a long transfer visibly makes progress instead of freezing the line.
        let detail = if status.syncing {
            status
                .activity
                .as_ref()
                .map(|activity| format!(" — {}", describe_activity(activity)))
                .or_else(|| {
                    status
                        .last_plan_summary
                        .as_ref()
                        .map(|summary| format!(" ({})", summarize_plan(summary)))
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use proton_drive_sync_engine::ipc::TransferActivity;

    fn blank_activity(phase: &str) -> SyncActivity {
        SyncActivity {
            phase: phase.to_owned(),
            detail: None,
            folders_listed: None,
            files_scanned: None,
            action_index: None,
            action_total: None,
            transfer: None,
            since_epoch_secs: None,
            pass: None,
        }
    }

    #[test]
    fn describe_activity_renders_each_phase() {
        let mut walk = blank_activity("listing-remote");
        walk.folders_listed = Some(214);
        walk.detail = Some("Companies/Acme".to_owned());
        assert_eq!(
            describe_activity(&walk),
            "listing remote folders — 214 listed · in Companies/Acme"
        );

        let mut scan = blank_activity("scanning-local");
        scan.files_scanned = Some(1204);
        scan.detail = Some("Photos/IMG_1834.jpg".to_owned());
        assert_eq!(
            describe_activity(&scan),
            "scanning local files — 1204 seen · at Photos/IMG_1834.jpg"
        );

        let mut transfer = blank_activity("executing");
        transfer.action_index = Some(812);
        transfer.action_total = Some(6377);
        transfer.transfer = Some(TransferActivity {
            direction: "download".to_owned(),
            path: PathBuf::from("Companies/takeout.tgz"),
            bytes_total: None,
            bytes_done: Some(1_500_000_000),
            // Far future → zero elapsed → no elapsed fragment, keeping the assertion stable.
            started_epoch_secs: u64::MAX,
        });
        assert_eq!(
            describe_activity(&transfer),
            "downloading Companies/takeout.tgz — 1.4 GiB so far [step 812/6377]"
        );

        let mut plain = blank_activity("executing");
        plain.detail = Some("creating local folder a/b".to_owned());
        plain.action_index = Some(5);
        plain.action_total = Some(10);
        assert_eq!(
            describe_activity(&plain),
            "creating local folder a/b [step 5/10]"
        );

        // An unknown phase from a newer daemon renders its raw token instead of vanishing.
        let mut unknown = blank_activity("defragmenting-flux");
        unknown.detail = Some("x".to_owned());
        assert_eq!(describe_activity(&unknown), "defragmenting-flux · x");
    }

    fn blank_response() -> ControlResponse {
        ControlResponse {
            status: "running".to_owned(),
            paused: false,
            syncing: false,
            reconcile_seq: 1,
            pending_changes: 0,
            message: "daemon status".to_owned(),
            last_sync_epoch_secs: None,
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: Vec::new(),
            pending_deletions: Vec::new(),
            failed_items: Vec::new(),
            failed_item_count: 0,
            config: None,
            activity: None,
            unsyncable: Vec::new(),
            history: None,
            file_history: None,
        }
    }

    #[test]
    fn a_partial_pass_headlines_as_itself_not_as_a_failed_one() {
        // #136: the daemon sets `last_error` on a partial pass so older clients still see a
        // problem, which means this client must check the item count FIRST or it would report a
        // pass that synced everything but three files as a sync that did not happen (#246).
        let style = Style { enabled: false };
        let mut partial = blank_response();
        partial.failed_item_count = 3;
        partial.last_error = Some("3 item(s) failed to sync (first: a.txt)".to_owned());
        let (_, state, detail) = headline(&partial, &style);
        assert_eq!(state, "partial");
        assert!(detail.contains("3 failed item(s)"), "unexpected: {detail}");

        // A wholly-failed pass is still its own state...
        let mut failed = blank_response();
        failed.last_error = Some("list failed".to_owned());
        assert_eq!(headline(&failed, &style).1, "error");
        // ...and a clean one is not dragged into either.
        assert_eq!(headline(&blank_response(), &style).1, "idle");
    }

    #[test]
    fn human_bytes_scales_through_the_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_bytes(1_500_000_000), "1.4 GiB");
    }
}
