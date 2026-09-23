use clap::{Parser, Subcommand};
use proton_drive_sync_engine::config::resolve_control_socket_path;
use proton_drive_sync_engine::index::{EntityKind, PassRecord};
use proton_drive_sync_engine::ipc::{
    ApplyOutcome, AuthState, ControlCommand, ControlRequest, ControlResponse, ListingOutcome,
    LocalDisposal, PendingDeletion, PlanOutcome, ReviewedPlan, SyncActivity, send_request,
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
    /// Address one folder pair by name (#102 phase 3), rather than the default pair every
    /// pre-multi-pair invocation addresses. Requires a daemon that supports multiple folder
    /// pairs; an older one is refused with a message to upgrade `proton-syncd` rather than
    /// silently running against its one pair under the wrong name.
    #[arg(long, global = true, conflicts_with = "all_pairs")]
    pair: Option<String>,
    /// Run the command once per configured folder pair, in the order `status` lists them
    /// (`--pair` names one pair; this is every pair). Not named `--all`: `approve`/`deny`/`keep`
    /// already use that flag for "every pending deletion" on the one pair they address, and one
    /// flag cannot mean two different fan-outs. Same capability gate as `--pair`.
    #[arg(long, global = true, conflicts_with = "pair")]
    all_pairs: bool,
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
    /// Discard everything the daemon has learned — the baseline index, the event cursors and the
    /// standing delete approvals — and rebuild it from a full scan of both sides. Your files are
    /// not touched: the rebuild re-adopts every local/remote pair that already agrees. Requires
    /// `--yes` because a mismatched pair is re-decided from scratch, conflict sidecars included.
    ResetIndex {
        /// Confirm the reset. Without it nothing is sent.
        #[arg(long)]
        yes: bool,
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
        /// Which deletion at PATH to authorize when the daemon has *planned* it but not yet
        /// withheld it (nothing is pending yet, e.g. straight after a dry run). `local` deletes
        /// the copy on this computer, `remote` the copy on Proton Drive. Ignored when PATH is
        /// already pending — that item's own direction wins.
        #[arg(long, value_parser = ["local", "remote"])]
        direction: Option<String>,
    },
    /// Revoke a prior approval before it has applied.
    Deny {
        /// Relative path of the approval to revoke.
        path: Option<PathBuf>,
        /// Revoke approval for every currently-pending deletion.
        #[arg(long)]
        all: bool,
    },
    /// List one folder on Proton Drive, as the daemon sees it right now. Read-only: it shows what
    /// is on the remote, not what would sync — selective-sync rules are not applied.
    List {
        /// Folder to list, relative to the daemon's configured remote root — or an absolute
        /// Proton Drive path (`/Drive/Other`) to look outside it. Omit for the root.
        path: Option<PathBuf>,
        /// Cap on the entries shown.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Work out what a sync would change, without changing anything. The daemon computes it on its
    /// own proton-drive client, so it never races the sync in progress.
    Plan {
        /// Cap on the plan rows shown. Destructive rows are always shown, whatever the cap.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Run a plan you reviewed with `plan`, named by its token.
    ///
    /// The daemon re-plans, compares, and applies only if nothing has changed since — so an apply
    /// can never quietly do more than what was reviewed. If the plan moved, nothing runs and the
    /// new plan is printed instead.
    Apply {
        /// The token printed by `proton-sync plan`.
        token: String,
        /// Run the plan without its destructive actions (deletions). Standing approvals do not
        /// override this: the deletions are skipped for this run and re-plan next pass.
        #[arg(long)]
        skip_destructive: bool,
        /// Schedule the apply and return immediately instead of waiting for it to finish.
        #[arg(long)]
        no_wait: bool,
    },
    /// Refuse a withheld deletion and put the two sides back in step: the surviving copy is sent
    /// back to the side it was deleted from on the next sync, and the item leaves the queue for
    /// good (unlike `deny`, which only revokes an approval and leaves it waiting).
    Keep {
        /// Relative path of the pending deletion to keep (as shown by `pending`).
        path: Option<PathBuf>,
        /// Keep every currently-pending deletion.
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

    // The client capability gate (#102 phase 3, ADR 0005 §4): an explicit `--pair`/`--all-pairs`
    // first reads `status` and refuses if the reply carries neither `pair` nor `pairs` — a daemon
    // that predates multi-pair. No `--pair`, no gate: the omitted selector means the default pair,
    // and an old daemon's only pair *is* the default pair, so it is correct without asking. The
    // residual race (the daemon is downgraded between this check and the real request) is accepted
    // rather than solved — see the ADR.
    if cli.all_pairs {
        return run_all_pairs(&cli, &socket_path, &style).await;
    }
    if cli.pair.is_some()
        && let Err(code) = capability_gate(&socket_path).await
    {
        return code;
    }
    run_for_pair(&cli, &socket_path, &style, cli.pair.as_deref()).await
}

/// Refuses with a non-zero exit when `socket_path` answers a `status` request with neither `pair`
/// nor `pairs` — the shape only a daemon that predates multi-pair produces (every real reply
/// carries both; see `ipc::PairSummary`'s doc for why `pairs` is never empty on one that
/// understands the field at all).
async fn capability_gate(socket_path: &Path) -> Result<(), ExitCode> {
    let probe = ControlRequest::new(ControlCommand::Status);
    let response = match request_with_timeout(socket_path, probe).await {
        Ok(response) => response,
        Err(message) => {
            eprintln!("{message}");
            return Err(ExitCode::FAILURE);
        }
    };
    if response.pair.is_none() && response.pairs.is_empty() {
        eprintln!(
            "this daemon does not support multiple folder pairs; upgrade proton-syncd to use \
             --pair/--all-pairs"
        );
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

/// One command against one pair (or the default, when `pair` is `None`) — everything `main` did
/// before #102 phase 3, unchanged for that case. `--all-pairs` calls this once per configured
/// pair with a header line ahead of each (see `run_all_pairs`); `--pair NAME` calls it once.
async fn run_for_pair(
    cli: &Cli,
    socket_path: &Path,
    style: &Style,
    pair: Option<&str>,
) -> ExitCode {
    let request = match build_request(&cli.command, pair) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let response = match request_with_timeout(socket_path, request).await {
        Ok(response) => response,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    // An explicit selector that did not resolve authorises nothing (ADR 0005 §4): the reply's
    // `pair` is structurally `None`, never inferred from `message`, and every verb reports it the
    // same way `list`/`plan`/`apply` already report a non-success outcome — exit non-zero rather
    // than rendering the verb's ordinary success path over a request that did nothing.
    if pair.is_some() && response.pair.is_none() {
        if cli.json {
            print_pretty_json(&response);
        } else {
            eprintln!("{}", response.message);
        }
        return ExitCode::FAILURE;
    }

    match &cli.command {
        Commands::Status => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                print_status(&response, style, pair);
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
                print_history(&response, style);
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
                print_activity(&response, style);
            }
            ExitCode::SUCCESS
        }
        Commands::Pause => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!(
                    "Sync paused. Edits are still tracked; resume with `{}`.",
                    cli_hint(pair, "resume")
                );
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
            watch_syncnow(socket_path, response, *no_wait, cli.json, style, pair).await
        }
        Commands::Resync | Commands::ResetIndex { .. } => {
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
                print_pending(&response.pending_deletions, pair);
            }
            ExitCode::SUCCESS
        }
        Commands::Approve { .. } | Commands::Deny { .. } | Commands::Keep { .. } => {
            if cli.json {
                print_pretty_json(&response);
            } else {
                println!("{}", response.message);
            }
            ExitCode::SUCCESS
        }
        Commands::Plan { limit } => {
            watch_plan(socket_path, response, *limit, cli.json, style, pair).await
        }
        Commands::Apply { no_wait, .. } => {
            watch_apply(socket_path, response, *no_wait, cli.json, style, pair).await
        }
        Commands::List { .. } => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response.listing)
                        .expect("serialize the remote listing")
                );
            }
            // A listing that did not happen exits non-zero even under `--json`, so a script can
            // branch on the exit code rather than parsing `state` out of the payload — and so
            // `busy` is never mistaken for an empty folder.
            print_listing(&response, cli.json, style)
        }
    }
}

/// `--all-pairs`: the capability gate, then every configured pair in the order `status` lists
/// them — except `stop`, which is daemon-wide and runs once, not per pair (see below).
///
/// **Deliberately does not run `run_for_pair`'s wait loops (`syncnow`/`apply`/`plan`) per pair.**
/// A wait loop's spinner and its final report are written for a single command watching a single
/// pass; N of them interleaved on one terminal would be unreadable, and a JSON array of "the
/// final response" per pair cannot also carry N independent progress streams. So under
/// `--all-pairs` every command answers with its **immediate reply** (the same ack a `--no-wait`
/// `syncnow`/`apply` gets, or the plain reply for anything else) — scheduled, not watched. This is
/// a scope decision, recorded in the ADR 0005 phase-3 note: `--pair NAME` is how a script or a
/// person watches one pair's pass to completion; `--all-pairs` is how they fire the same command
/// at every pair and see what each one said.
async fn run_all_pairs(cli: &Cli, socket_path: &Path, style: &Style) -> ExitCode {
    let probe = ControlRequest::new(ControlCommand::Status);
    let gate = match request_with_timeout(socket_path, probe).await {
        Ok(response) => response,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    if gate.pair.is_none() && gate.pairs.is_empty() {
        eprintln!(
            "this daemon does not support multiple folder pairs; upgrade proton-syncd to use \
             --pair/--all-pairs"
        );
        return ExitCode::FAILURE;
    }

    // `shutdown` is daemon-wide and ignores the selector (ADR 0005 §4's verb table), so it is not
    // a per-pair loop: with N pairs the loop below would fire N shutdowns, the first killing the
    // daemon and every later iteration failing to connect — a successful stop reported as a
    // failure. One request, same print path as an unqualified `stop`.
    if matches!(cli.command, Commands::Stop) {
        return run_for_pair(cli, socket_path, style, None).await;
    }

    let mut worst = ExitCode::SUCCESS;
    let mut json_results = Vec::new();
    for pair in &gate.pairs {
        let request = match build_request(&cli.command, Some(pair.name.as_str())) {
            Ok(request) => request,
            Err(message) => {
                eprintln!("{}: {message}", pair.name);
                worst = ExitCode::FAILURE;
                continue;
            }
        };
        let response = match request_with_timeout(socket_path, request).await {
            Ok(response) => response,
            Err(message) => {
                eprintln!("{}: {message}", pair.name);
                worst = ExitCode::FAILURE;
                continue;
            }
        };
        if response.pair.is_none() {
            worst = ExitCode::FAILURE;
        }
        // The one fold both branches share (`reply_exit_code`'s doc) — a busy `list`, a paused
        // `plan`, a failed `apply` must fail `--all-pairs` whether it is rendered or reported as
        // JSON.
        if reply_exit_code(&cli.command, &response) != ExitCode::SUCCESS {
            worst = ExitCode::FAILURE;
        }
        if cli.json {
            json_results.push(serde_json::json!({
                "pair": pair.name,
                "result": json_result_value(&cli.command, &response),
            }));
        } else {
            println!("{}", style.dim(&format!("== {} ==", pair.name)));
            print_pair_reply(&cli.command, &response, style, &pair.name);
        }
    }
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_results).expect("serialize --all-pairs results")
        );
    }
    worst
}

/// The exit code a `--all-pairs` reply for `command` earns — the **one** decision both of
/// `run_all_pairs`'s branches fold into `worst`. Before this, the JSON branch folded only
/// `response.pair` and never the verb's own outcome, so a busy `list`, a paused `plan`, or a
/// failed `apply` under `--all-pairs --json` printed the payload and exited 0 (the JSON twin of
/// the human-branch bug 9a8d304 fixed). `list_exit_code`/`plan_exit_code`/`apply_exit_code` are
/// the same functions `print_listing`/`report_plan`/`report_apply` compute their return from, so
/// the two branches cannot answer differently for the same reply.
fn reply_exit_code(command: &Commands, response: &ControlResponse) -> ExitCode {
    match command {
        Commands::List { .. } => list_exit_code(response.listing.as_ref()),
        Commands::Plan { .. } => plan_exit_code(response.plan.as_ref()),
        Commands::Apply { .. } => apply_exit_code(response.apply.as_ref()),
        Commands::Status
        | Commands::History
        | Commands::Activity { .. }
        | Commands::Pending
        | Commands::Pause
        | Commands::Resume
        | Commands::Syncnow { .. }
        | Commands::Resync
        | Commands::ResetIndex { .. }
        | Commands::Stop
        | Commands::Approve { .. }
        | Commands::Deny { .. }
        | Commands::Keep { .. } => ExitCode::SUCCESS,
    }
}

/// The human-readable rendering of one pair's immediate reply under `--all-pairs` — the same
/// per-verb rule `run_for_pair`'s non-json branches follow (`response.message` for a plain ack,
/// the dedicated printer for `status`/`history`/`activity`/`pending`, a plan/apply summary for
/// their *ack* rather than their watched outcome, since `--all-pairs` does not wait). Print-only:
/// `run_all_pairs` gets this reply's exit code from `reply_exit_code`, not from here.
fn print_pair_reply(
    command: &Commands,
    response: &ControlResponse,
    style: &Style,
    pair_name: &str,
) {
    let pair = Some(pair_name);
    match command {
        Commands::Status => print_status(response, style, pair),
        Commands::History => print_history(response, style),
        Commands::Activity { .. } => print_activity(response, style),
        Commands::Pending => print_pending(&response.pending_deletions, pair),
        Commands::List { .. } => {
            print_listing(response, false, style);
        }
        Commands::Plan { .. } => {
            report_plan(response, false, style, pair);
        }
        Commands::Apply { .. } => {
            report_apply(response, false, style, pair);
        }
        Commands::Pause
        | Commands::Resume
        | Commands::Syncnow { .. }
        | Commands::Resync
        | Commands::ResetIndex { .. }
        | Commands::Stop
        | Commands::Approve { .. }
        | Commands::Deny { .. }
        | Commands::Keep { .. } => println!("{}", response.message),
    }
}

/// The JSON value a single-pair `--json` invocation of `command` would print for `response` — what
/// `--all-pairs --json`'s per-pair `result` field must match exactly (decision #10, ADR 0005 §4
/// departure 5: "the same value a single-pair `--json` invocation prints for that verb," nested
/// under `result` rather than spread, uniformly across verbs). `history`/`activity`/`pending`/
/// `list`/`plan`/`apply` each print one projected field in `run_for_pair`, never the whole
/// envelope; every other verb prints the whole `ControlResponse` there too, so this matches that.
fn json_result_value(command: &Commands, response: &ControlResponse) -> serde_json::Value {
    match command {
        Commands::History => {
            serde_json::to_value(&response.history).expect("serialize pass history")
        }
        Commands::Activity { .. } => {
            serde_json::to_value(&response.file_history).expect("serialize file history")
        }
        Commands::Pending => {
            serde_json::to_value(&response.pending_deletions).expect("serialize pending deletions")
        }
        Commands::List { .. } => {
            serde_json::to_value(&response.listing).expect("serialize the remote listing")
        }
        Commands::Plan { .. } => serde_json::to_value(&response.plan).expect("serialize the plan"),
        Commands::Apply { .. } => {
            serde_json::to_value(&response.apply).expect("serialize the apply outcome")
        }
        Commands::Status
        | Commands::Pause
        | Commands::Resume
        | Commands::Syncnow { .. }
        | Commands::Resync
        | Commands::ResetIndex { .. }
        | Commands::Stop
        | Commands::Approve { .. }
        | Commands::Deny { .. }
        | Commands::Keep { .. } => serde_json::to_value(response).expect("serialize response"),
    }
}

/// Maps a subcommand to the control request to send, validating the `approve`/`deny` selector.
fn build_request(command: &Commands, pair: Option<&str>) -> Result<ControlRequest, String> {
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
        // The gate is here rather than on the daemon: a control command carries no confirmation
        // field, so a client that skipped the prompt would be indistinguishable from one that did
        // not. Refusing before the socket is even opened keeps "nothing was sent" literally true.
        Commands::ResetIndex { yes } => {
            if !yes {
                return Err(
                    "reset-index discards the sync baseline and every standing delete approval; \
                     re-run with --yes to confirm"
                        .to_owned(),
                );
            }
            (ControlCommand::ResetIndex, None)
        }
        Commands::Stop => (ControlCommand::Shutdown, None),
        Commands::Approve { path, all, .. } => {
            (ControlCommand::Approve, approval_selector(path, *all)?)
        }
        Commands::Deny { path, all } => (ControlCommand::Deny, approval_selector(path, *all)?),
        Commands::Keep { path, all } => (ControlCommand::Keep, approval_selector(path, *all)?),
        // No `literal_path` and no `--all`: a listing has no reserved word, so a folder named
        // `all` lists like any other (see `ControlCommand::List`).
        Commands::List { path, .. } => (
            ControlCommand::List,
            path.as_ref().map(|path| wire_path(path).into_owned()),
        ),
        // The ack only; `watch_plan` then polls `plan_result` for the answer. Same two-verb shape
        // as `syncnow`, which acks and then polls `status`.
        Commands::Plan { .. } => (ControlCommand::Plan, None),
        // The token IS the argument, and it is not a path — no `literal_path`, no validation as
        // one. An apply with no token is refused by the daemon as stale, which is why clap makes
        // it required here rather than optional-and-meaning-"whatever you have".
        Commands::Apply { token, .. } => (ControlCommand::Apply, Some(token.clone())),
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
        Commands::List { limit, .. } => (None, *limit),
        Commands::Plan { limit } => (None, *limit),
        _ => (None, None),
    };
    let literal_path = matches!(
        command,
        Commands::Approve {
            path: Some(_),
            all: false,
            ..
        } | Commands::Deny {
            path: Some(_),
            all: false
        } | Commands::Keep {
            path: Some(_),
            all: false
        }
    );
    // Only `approve` carries one, and only its pre-approval branch reads it (#227). Parsed here
    // rather than passed as a string so an unknown word is a CLI error, not a silent no-op.
    let direction = match command {
        Commands::Approve {
            direction: Some(value),
            ..
        } => Some(
            value
                .parse::<DeleteDirection>()
                .map_err(|error| error.to_string())?,
        ),
        _ => None,
    };
    // Read by `apply` alone (#192): every other command ignores it.
    let skip_destructive = matches!(
        command,
        Commands::Apply {
            skip_destructive: true,
            ..
        }
    );
    Ok(ControlRequest {
        argument,
        literal_path,
        window_secs,
        limit,
        direction,
        skip_destructive,
        pair: pair.map(str::to_owned),
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
fn print_status(response: &ControlResponse, style: &Style, pair: Option<&str>) {
    println!("{}", status_headline_line(response, style));

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
        if let Some(queue) = describe_transfer_queue(activity) {
            rows.push(("queued", queue));
        }
        if let Some(moved) = describe_pass_transfers(activity) {
            rows.push(("moved", moved));
        }
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
                "{} awaiting approval — review with `{}`",
                response.pending_deletions.len(),
                cli_hint(pair, "pending")
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
    // Only when there is something to say. `Unknown` is not a problem to report — it is the
    // daemon having learned nothing yet (or an older daemon that classifies nothing), and a
    // `sign-in unknown` row on every healthy status would train the reader to skip the block.
    if response.auth == AuthState::SignedOut {
        rows.push((
            "sign-in",
            style.red("Proton refused the session — run `proton-drive login`"),
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

/// The exit code a `list` reply's outcome earns, computed with no printing so `reply_exit_code`
/// (the `--all-pairs --json` fold) and [`print_listing`] can never disagree about the same reply.
/// Only [`ListingOutcome::Listed`] succeeds: `busy`, `failed`, and an outcome this build does not
/// know are three different things to a script, but none of them is "the folder is empty".
fn list_exit_code(listing: Option<&ListingOutcome>) -> ExitCode {
    match listing {
        Some(ListingOutcome::Listed { .. }) => ExitCode::SUCCESS,
        Some(ListingOutcome::Busy) => ExitCode::FAILURE,
        Some(ListingOutcome::Failed { .. }) => ExitCode::FAILURE,
        Some(ListingOutcome::Unknown) | None => ExitCode::FAILURE,
    }
}

/// Renders a `list` reply. Prints only; the exit code is [`list_exit_code`]'s alone.
fn print_listing(response: &ControlResponse, json: bool, style: &Style) -> ExitCode {
    match &response.listing {
        Some(ListingOutcome::Listed {
            path,
            entries,
            total,
            truncated,
        }) => {
            if !json {
                let where_ = if path.as_os_str().is_empty() {
                    "the remote root".to_owned()
                } else {
                    path.display().to_string()
                };
                if entries.is_empty() {
                    println!("{where_} is empty.");
                } else {
                    println!("{}", style.dim(&where_));
                    for entry in entries {
                        let is_directory = entry.entity_kind == EntityKind::Directory;
                        // A trailing slash, as `ls -p` does it: the kind is the first thing a
                        // browser needs and the cheapest thing to render.
                        let name = if is_directory {
                            format!("{}/", entry.name)
                        } else {
                            entry.name.clone()
                        };
                        // A node the engine cannot fetch is named as such rather than listed as an
                        // ordinary file that simply never arrives (#295's lesson, one layer up).
                        // Only for files: `downloadable` is meaningless on a directory and says
                        // nothing about what is inside it.
                        let note = if !is_directory && !entry.downloadable {
                            format!("  {}", style.dim("(can't be downloaded)"))
                        } else {
                            String::new()
                        };
                        println!("  {name}{note}");
                    }
                    if *truncated {
                        println!(
                            "  {}",
                            style.dim(&format!(
                                "… {} of {total} shown; raise --limit for more",
                                entries.len()
                            ))
                        );
                    }
                }
            }
        }
        Some(ListingOutcome::Busy) => {
            eprintln!(
                "The proton-drive CLI is busy with a sync operation; nothing was listed. Try again."
            );
        }
        Some(ListingOutcome::Failed { error }) => {
            eprintln!("Could not list that folder: {error}");
        }
        // A state this client does not know, and the `None` an older daemon sends. Both mean the
        // same thing to a user — no listing — and neither is an empty folder.
        Some(ListingOutcome::Unknown) | None => {
            eprintln!("The daemon did not return a listing for that folder.");
        }
    }
    list_exit_code(response.listing.as_ref())
}

/// The entities the daemon cannot sync, by name and cause. A count alone is unactionable: the
/// account behind #295 carried one such file for five months, and `skipped_unsupported: 1` was the
/// only trace of it. Grouped by cause so a Docs-heavy account reads as one fact, not N.
fn print_unsyncable(items: &[UnsyncableItem], style: &Style) {
    for line in unsyncable_lines(items, style) {
        println!("{line}");
    }
}

/// The lines [`print_unsyncable`] prints, built rather than printed so the grouping can be tested.
/// A stdout-only renderer is one nothing checks, and this one has to keep working as reasons are
/// added — #232 added five at once.
fn unsyncable_lines(items: &[UnsyncableItem], style: &Style) -> Vec<String> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        String::new(),
        style.bold(&format!("{} item(s) cannot be synced:", items.len())),
    ];
    // `reason` is a wire token an older client may not know, so group on the token itself and
    // render an unfamiliar one verbatim rather than dropping the row.
    let mut groups: BTreeMap<&str, Vec<&UnsyncableItem>> = BTreeMap::new();
    for item in items {
        groups.entry(item.reason.as_str()).or_default().push(item);
    }
    for (token, group) in groups {
        lines.push(format!(
            "  {}",
            style.dim(&format!("{token} — {}", group[0].reason.describe()))
        ));
        for item in group {
            lines.push(format!(
                "    {}  {}",
                item.path.display(),
                style.dim(&format!(
                    "(since {})",
                    relative_time(item.first_seen_epoch_secs)
                ))
            ));
        }
    }
    lines.push(style.dim(
        "These are skipped on every pass; nothing transfers in either direction until the \
         cause changes.",
    ));
    lines
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

/// The headline's printed line (decision #14, ADR 0005 §4): the pair is named only when more than
/// one is configured, so a single-pair daemon's headline is byte-identical to before #102 phase 3 —
/// "everything is up to date" silently meaning one of three folders is #246's lie read the other
/// way round. Separate from [`headline`] itself because its `state` word is asserted on verbatim
/// by tests (`"idle"`, `"error"`, …) and must stay a bare machine-comparable token, not a
/// pair-prefixed string.
fn status_headline_line(response: &ControlResponse, style: &Style) -> String {
    let (dot, state, detail) = headline(response, style);
    match (response.pairs.len() > 1, &response.pair) {
        (true, Some(name)) => format!(
            "{dot} {} {} — {detail}",
            style.bold(name),
            style.bold(state)
        ),
        _ => format!("{dot} {} — {detail}", style.bold(state)),
    }
}

/// `proton-sync history` — one line per recorded pass, newest first.
///
/// Reads the durable pass log, NOT `status_history`: with event-driven detection on by default the
/// daemon runs a pass every 30s and records every one of them in that rolling 20-entry trail, so
/// it holds about ten minutes and is almost entirely idle polls. The pass log records only passes
/// that did something (plus every full sweep, and every pass that did not end clean).
fn print_history(response: &ControlResponse, style: &Style) {
    // `None` is NOT "no history": the daemon publishes this block before its first pass, so an
    // absent one means the daemon is older than the field or its history read failed (the daemon
    // logs that). Saying "no sync history yet" here would answer a question this reply cannot
    // answer, with a confident wrong number — the `unknown is not zero` rule.
    let Some(history) = &response.history else {
        println!(
            "This daemon does not report pass history. Upgrade it, or check its log for a \
             history read failure."
        );
        return;
    };
    if let Some(sweep) = &history.last_full_sweep {
        println!(
            "Last full sweep {} — {}.",
            relative_time(sweep.started_epoch_secs),
            describe_sweep(sweep)
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
                pass.error
                    .as_deref()
                    .map(one_line)
                    .unwrap_or_else(|| "failed".to_owned()),
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

/// Collapses an error onto one line for an inline clause.
///
/// A daemon error carries the CLI child's stderr verbatim, newlines included, and these two
/// renderings put it mid-sentence and mid-row — where a raw newline breaks the sentence in half
/// and strands its full stop on a line of its own.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the last full sweep found, as a clause for `Last full sweep 2 days ago — …`.
///
/// Branches on **outcome first**, and only then on the count. `changed == 0` alone does not mean
/// "nothing was out of step": a sweep that failed, or was killed mid-pass, also moved nothing, and
/// reading the count without the outcome prints a confident all-clear over a pass that never
/// finished — the exact shape of #246, and exactly what #238 warns about ("answering a different
/// question with a confident number"). Every outcome is enumerated, including an unknown token
/// from a newer daemon; there is no trailing arm that could absorb a state added later.
fn describe_sweep(sweep: &PassRecord) -> String {
    match sweep.outcome.as_str() {
        "clean" if sweep.changed == 0 => "nothing was out of step".to_owned(),
        "clean" => format!("{} change(s)", sweep.changed),
        "partial" => format!(
            "{} change(s), {} item(s) failed",
            sweep.changed, sweep.failed
        ),
        "failed" => match &sweep.error {
            Some(error) => format!("it failed: {}", one_line(error)),
            None => "it failed".to_owned(),
        },
        "interrupted" => "it did not finish".to_owned(),
        other => format!("it ended `{other}`"),
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

/// Why a reply carries no `file_history`. The daemon's `message` is the generic reply label, so a
/// `last_error` dropped here leaves the one debuggable fact on the floor (a read failure, a bad
/// window) and the user with "this daemon does not do that", which is a different claim.
fn missing_history_message(message: &str, last_error: Option<&str>) -> String {
    match last_error {
        Some(error) => format!("{message}: {error}"),
        None => "This daemon does not report per-file activity.".to_owned(),
    }
}

/// What an empty page means. `FileHistory::total` is a `COUNT(*)` over the same predicate, but
/// `events` additionally drops rows whose action token this build cannot decode
/// (`index::read_file_event` → `None`) — which is exactly what a **newer** daemon's rows look like
/// to an older client. So an empty page over a non-zero count is not an empty window, and reporting
/// "nothing has moved" would be a confident false all-clear: the shape that once rendered
/// "Everything is up to date" over a failed pass (#246).
fn empty_activity_message(total: usize) -> String {
    if total > 0 {
        format!(
            "{total} event(s) in that window, none readable by this client — it is older than the \
             daemon that wrote them. Upgrade proton-sync to read them."
        )
    } else {
        "Nothing has moved in that window.".to_owned()
    }
}

/// `proton-sync activity [PATH]` — the per-file feed, newest first.
fn print_activity(response: &ControlResponse, style: &Style) {
    let Some(history) = &response.file_history else {
        println!(
            "{}",
            missing_history_message(&response.message, response.last_error.as_deref())
        );
        return;
    };
    if history.events.is_empty() {
        println!("{}", empty_activity_message(history.total));
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
            "{} file(s), {}{} event(s) in this window{traffic}",
            history.files,
            shown_prefix(history.events.len(), history.total),
            history.total
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

/// `"showing 2 of "` when the printed list is shorter than the count beside it, `""` otherwise.
///
/// Two things shorten the list — the `--limit` cap, and a row whose action token this build does
/// not know (written by a newer daemon; `index::read_file_event` skips it while the SQL still
/// counts it) — and in both cases a bare total under a shorter list is a count that contradicts
/// what the reader can see.
fn shown_prefix(shown: usize, total: usize) -> String {
    if shown < total {
        format!("showing {shown} of ")
    } else {
        String::new()
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
            if let Some(transfer) = activity.active_transfer() {
                let verb = if transfer.direction == "upload" {
                    "uploading"
                } else {
                    "downloading"
                };
                // A batched download is one row over a whole chunk, so it names the folder and how
                // many files are landing in it — "downloading 25 files in photos/2024", not a
                // folder rendered as if it were the file.
                let what = match transfer.files {
                    Some(files) => format!("{files} files in {}", transfer.path.display()),
                    None => transfer.path.display().to_string(),
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
                let elapsed = transfer
                    .started_epoch_secs
                    .map(elapsed_label)
                    .unwrap_or_default();
                format!("{verb} {what}{progress}{elapsed}{step}")
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

/// What is waiting behind the transfer in flight (#211): the next few paths, then a count for the
/// tail the window does not name.
///
/// ```text
/// queued     notes/scratch.md, reports/q3-summary.pdf and 115 more
/// ```
///
/// `None` when nothing is queued — an omitted row, never `0 queued`.
fn describe_transfer_queue(activity: &SyncActivity) -> Option<String> {
    let named: Vec<String> = activity
        .queued_transfers()
        .map(|transfer| transfer.path.display().to_string())
        .collect();
    let more = activity.transfers_past_the_window();
    if named.is_empty() && more == 0 {
        return None;
    }
    let tail = match more {
        0 => String::new(),
        1 => " and 1 more".to_owned(),
        n => format!(" and {n} more"),
    };
    if named.is_empty() {
        // Everything left is past the window (a chunk wide enough to fill it on its own).
        return Some(format!("{more} more"));
    }
    Some(format!("{}{tail}", named.join(", ")))
}

/// This pass's per-direction progress (#243 counts, #98 bytes), from the pass block.
///
/// ```text
/// moved      44 sent · 115 received — 386.0 MB up, 1.1 GB down
/// ```
///
/// `None` before anything has landed: a pass that has moved nothing says nothing, rather than
/// printing a row of zeroes for every folder-creation pass.
fn describe_pass_transfers(activity: &SyncActivity) -> Option<String> {
    let pass = activity.pass.as_ref()?;
    if pass.uploaded_files == 0 && pass.downloaded_files == 0 {
        return None;
    }
    Some(format!(
        "{} sent · {} received — {} up, {} down",
        pass.uploaded_files,
        pass.downloaded_files,
        human_bytes(pass.uploaded_bytes),
        human_bytes(pass.downloaded_bytes),
    ))
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
    pair: Option<&str>,
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
            pair: pair.map(str::to_owned),
            ..ControlRequest::new(ControlCommand::Status)
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

// ---- plan / apply ------------------------------------------------------------------------------

/// How many consecutive transport failures end a wait. Same bail-out `watch_syncnow` uses: a
/// daemon that stopped answering is not a pass that is still running.
const WAIT_ERROR_LIMIT: u32 = 5;

/// The daemon acks `plan` immediately and computes it on its main loop, so the CLI watches until
/// the pass that answers *this* request finishes. It waits on the plan plane's own counter, not on
/// `reconcile_seq`: a plan pass deliberately does not bump that, because doing so would satisfy a
/// concurrent `syncnow` watcher's target and make it report a sync that never ran.
async fn watch_plan(
    socket_path: &Path,
    ack: ControlResponse,
    limit: Option<usize>,
    json: bool,
    style: &Style,
    pair: Option<&str>,
) -> ExitCode {
    let target = match &ack.plan {
        Some(PlanOutcome::Scheduled { plan_seq }) => *plan_seq,
        // Paused, shutting down, or a daemon too old to know the verb: nothing to watch. Reported
        // from the typed outcome, never by matching the message (#103).
        _ => return report_plan(&ack, json, style, pair),
    };
    let outcome = poll_until(
        socket_path,
        || ControlRequest {
            limit,
            pair: pair.map(str::to_owned),
            ..ControlRequest::new(ControlCommand::PlanResult)
        },
        // The generation, not merely "no longer computing": a second client's `plan` while ours is
        // in flight books a newer request, and reading its answer as ours would show a plan whose
        // token we never asked for.
        |response| plan_generation(response.plan.as_ref()).is_some_and(|seq| seq >= target),
    )
    .await;
    match outcome {
        Ok(response) => report_plan(&response, json, style, pair),
        Err(message) => {
            eprintln!("lost contact with the daemon while waiting: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The request generation a plan outcome answers, or `None` while nothing has been answered.
/// [`PlanOutcome::Computing`] carries the *last answered* generation, so it never satisfies a wait
/// for a newer one.
fn plan_generation(outcome: Option<&PlanOutcome>) -> Option<u64> {
    match outcome? {
        PlanOutcome::Computed(plan) => Some(plan.plan_seq),
        PlanOutcome::Failed { plan_seq, .. } => Some(*plan_seq),
        // Not an answer to anything: still working, refused, or a state this build does not know.
        PlanOutcome::Computing { .. }
        | PlanOutcome::Scheduled { .. }
        | PlanOutcome::Paused
        | PlanOutcome::Absent
        | PlanOutcome::Unknown => None,
    }
}

/// The request generation an apply verdict answers. Same rule as [`plan_generation`].
fn apply_generation(outcome: Option<&ApplyOutcome>) -> Option<u64> {
    match outcome? {
        ApplyOutcome::Applied { apply_seq, .. }
        | ApplyOutcome::Diverged { apply_seq }
        | ApplyOutcome::Failed { apply_seq, .. } => Some(*apply_seq),
        ApplyOutcome::Scheduled { .. }
        | ApplyOutcome::Stale
        | ApplyOutcome::Paused
        | ApplyOutcome::Unknown => None,
    }
}

/// `apply` is a normal pass, so this waits on the apply plane's counter and then reports the
/// verdict. A divergence is not an error the user made — the same reply carries the new plan, so it
/// is printed for review.
async fn watch_apply(
    socket_path: &Path,
    ack: ControlResponse,
    no_wait: bool,
    json: bool,
    style: &Style,
    pair: Option<&str>,
) -> ExitCode {
    let target = match &ack.apply {
        Some(ApplyOutcome::Scheduled { apply_seq }) => *apply_seq,
        _ => return report_apply(&ack, json, style, pair),
    };
    if no_wait {
        return report_apply(&ack, json, style, pair);
    }
    let outcome = poll_until(
        socket_path,
        // The smallest window the daemon will build, because this wait reads `apply` and nothing
        // else. Without it every 300 ms poll rebuilds and ships up to `PLAN_ACTIONS_DEFAULT_LIMIT`
        // rows nobody looks at, for as long as the apply runs — minutes, on a large plan (#321).
        // `Some(1)`, not `Some(0)`: `StoredPlan::outcome` clamps the limit to at least one row, so
        // a literal that does not survive the clamp would read as a stronger claim than it is. It
        // bounds the ordinary rows only — destructive rows are never truncated at any limit.
        || ControlRequest {
            limit: Some(1),
            pair: pair.map(str::to_owned),
            ..ControlRequest::new(ControlCommand::PlanResult)
        },
        |response| apply_generation(response.apply.as_ref()).is_some_and(|seq| seq >= target),
    )
    .await;
    match outcome {
        Ok(response) => {
            let response = refetch_plan_if_diverged(socket_path, response, target, pair).await;
            report_apply(&response, json, style, pair)
        }
        Err(message) => {
            eprintln!("lost contact with the daemon while waiting: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The other half of the minimal poll window: a **divergence** is the one verdict whose reply is
/// read for its plan as well, so fetch that plan once, at the display limit, when the wait ends on
/// one (#321).
///
/// Only the `plan` field is taken from the second reply. The verdict stays the one this wait was
/// watching for: `plan_result` re-reads the published slot, so a newer apply landing between the two
/// requests would otherwise have this command report *its* outcome under our token.
///
/// A failed re-request degrades to the divergence **without** the plan rather than failing the
/// command: nothing was applied either way, and that is the fact the user needs. `report_apply`
/// prints `proton-sync plan` as the way to see the new one.
///
/// `pair` must be the same selector `watch_apply` was called with — this refetch was missed
/// once, and it addressed the default pair no matter which pair the apply was for.
async fn refetch_plan_if_diverged(
    socket_path: &Path,
    waited: ControlResponse,
    target: u64,
    pair: Option<&str>,
) -> ControlResponse {
    if !matches!(waited.apply, Some(ApplyOutcome::Diverged { apply_seq }) if apply_seq >= target) {
        return waited;
    }
    // The daemon's default window (`PLAN_ACTIONS_DEFAULT_LIMIT`), which is what this wait shipped on
    // every poll before #321 — `apply` has no `--limit` of its own to raise it with.
    let request = ControlRequest {
        pair: pair.map(str::to_owned),
        ..ControlRequest::new(ControlCommand::PlanResult)
    };
    match request_with_timeout(socket_path, request).await {
        Ok(fresh) => ControlResponse {
            plan: fresh.plan,
            ..waited
        },
        Err(message) => {
            eprintln!("could not fetch the new plan: {message}");
            ControlResponse {
                plan: None,
                ..waited
            }
        }
    }
}

/// The shared wait: poll `plan_result` on the `syncnow` cadence until `ready` says the answer has
/// landed, bailing out only on [`WAIT_ERROR_LIMIT`] consecutive transport failures.
///
/// **Deliberately no paused bail-out, unlike [`watch_syncnow`].** That is not an inconsistency —
/// the two waits watch counters with opposite guarantees. A `syncnow` skipped by a pause is never
/// sealed: `reconcile_if_needed` early-returns without bumping `reconcile_seq`, so its target is
/// unreachable and only the client can end the wait. Every `plan`/`apply` request, by contrast, is
/// *always* sealed — a plan booked before a pause still runs (`plan_now` has no pause check), an
/// apply overtaken by one is sealed `Failed` by `discard_queued_apply_for_pause`, and a daemon that
/// died stops answering and trips `WAIT_ERROR_LIMIT`. So a pause here proves nothing about this
/// request, and bailing on it was a **false positive**: between the `plan` ack and `plan_now`'s
/// `syncing.store(true)` a poller reads `paused && !syncing` for a pass that is about to run, and
/// would report "paused before the pass ran" for a plan that then completes and seals normally. For
/// `apply` it raced the daemon's own typed verdict and answered with a sentence this client made up
/// instead — exactly the #103 shape. Guard:
/// `a_pause_mid_wait_does_not_end_a_plan_or_apply_wait`.
async fn poll_until(
    socket_path: &Path,
    request: impl Fn() -> ControlRequest,
    ready: impl Fn(&ControlResponse) -> bool,
) -> Result<ControlResponse, String> {
    let spinner = Spinner::for_stderr();
    let started = Instant::now();
    let mut consecutive_errors = 0u32;
    let result = loop {
        tokio::time::sleep(WAIT_POLL_INTERVAL).await;
        match request_with_timeout(socket_path, request()).await {
            Ok(response) => {
                consecutive_errors = 0;
                if ready(&response) {
                    break Ok(response);
                }
                spinner.tick(&started, &response);
            }
            Err(message) => {
                consecutive_errors += 1;
                if consecutive_errors >= WAIT_ERROR_LIMIT {
                    break Err(message);
                }
            }
        }
    };
    spinner.clear();
    result
}

/// A copy-pasteable `proton-sync` invocation, carrying the same `--pair` selector the command
/// that is printing the hint was run with (ADR 0005 §4: a wire selector is never inferred, only
/// echoed byte-exact). `tail` is everything after the binary name, e.g. `"plan"` or `"resume"`.
///
/// Without this, a hint printed after `proton-sync --pair work ...` reads `proton-sync plan` —
/// which a user pastes verbatim, and which then addresses the *default* pair (`--pair` omitted
/// resolves to index 0, `ipc.rs` §"pair selector"), not `work`. Some of those omissions fail
/// loud (`apply`'s token no longer matches the default pair's stored plan, so it answers `Stale`
/// rather than running); others do not — `plan`/`resume`/`pending`/`approve`/`keep` all accept an
/// unrelated pair's selector and act on it with no signal that it was the wrong one.
fn cli_hint(pair: Option<&str>, tail: &str) -> String {
    match pair {
        Some(name) => format!("proton-sync --pair {name} {tail}"),
        None => format!("proton-sync {tail}"),
    }
}

/// The `Run it with:` footer, or `None` when there is nothing to run.
///
/// The token is the whole point of printing it: `apply` names it, and nothing else can authorise
/// this exact plan. But an **empty** plan has nothing to authorise — applying one is a provable
/// no-op (the daemon re-plans, compares, and executes nothing) — so offering the command under
/// "Nothing would change" both contradicts that sentence and invites a pointless pass. Keyed on
/// `total`, the untruncated plan length, never on `actions`, which is a window over it.
fn run_it_line(total: usize, token: &str, pair: Option<&str>, style: &Style) -> Option<String> {
    (total > 0).then(|| {
        format!(
            "Run it with: {}",
            style.dim(&cli_hint(pair, &format!("apply {token}")))
        )
    })
}

/// The exit code a `plan` reply's outcome earns — [`list_exit_code`]'s sibling, read by both
/// [`report_plan`] and `reply_exit_code`. [`PlanOutcome::Scheduled`] is the `--all-pairs` immediate
/// ack (`report_plan`'s doc on that arm) and succeeds like [`PlanOutcome::Computed`]; a plan that
/// did not happen — paused, failed, still computing, absent, or an outcome this build does not
/// know — fails, so a script never mistakes it for "nothing to do".
fn plan_exit_code(plan: Option<&PlanOutcome>) -> ExitCode {
    match plan {
        Some(PlanOutcome::Computed(_)) => ExitCode::SUCCESS,
        Some(PlanOutcome::Scheduled { .. }) => ExitCode::SUCCESS,
        Some(PlanOutcome::Paused) => ExitCode::FAILURE,
        Some(PlanOutcome::Failed { .. }) => ExitCode::FAILURE,
        Some(PlanOutcome::Computing { .. }) => ExitCode::FAILURE,
        Some(PlanOutcome::Absent) => ExitCode::FAILURE,
        Some(PlanOutcome::Unknown) | None => ExitCode::FAILURE,
    }
}

/// Renders a plan reply. Prints only; the exit code is [`plan_exit_code`]'s alone.
///
/// `pair` is the selector this command was run with (not `response.pair`, which is `Some(name)`
/// on every successful reply and would print `--pair <name>` even for a single-pair user who
/// never typed `--pair` at all) — threaded through so every hint this prints echoes it (`cli_hint`).
fn report_plan(
    response: &ControlResponse,
    json: bool,
    style: &Style,
    pair: Option<&str>,
) -> ExitCode {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response.plan).expect("serialize the plan")
        );
    }
    match &response.plan {
        Some(PlanOutcome::Computed(plan)) => {
            let ReviewedPlan {
                token,
                summary,
                actions,
                total,
                truncated,
                cannot_sync,
                ..
            } = plan.as_ref();
            if !json {
                if *total == 0 {
                    println!("Nothing would change — both sides already match.");
                } else {
                    println!("{}", style.dim(&summarize_plan(summary)));
                    for action in actions {
                        let mark = if action.action.is_destructive() {
                            style.red("-")
                        } else {
                            style.dim("·")
                        };
                        let path = match &action.destination_path {
                            Some(destination) => {
                                format!("{} → {}", action.path.display(), destination.display())
                            }
                            None => action.path.display().to_string(),
                        };
                        println!("  {mark} {:<24} {path}", describe_action(action.action));
                    }
                    if *truncated {
                        println!(
                            "  {}",
                            style.dim(&format!(
                                "… {} of {total} shown; raise --limit for more",
                                actions.len()
                            ))
                        );
                    }
                }
                if !cannot_sync.is_empty() {
                    println!("\n{}", style.dim("can't sync"));
                    for entry in cannot_sync {
                        println!(
                            "  {} {}",
                            entry.relative_path.display(),
                            style.dim(entry.reason.describe())
                        );
                    }
                }
                if let Some(line) = run_it_line(*total, token, pair, style) {
                    println!("\n{line}");
                }
            }
        }
        Some(PlanOutcome::Paused) => {
            eprintln!(
                "Syncing is paused, so nothing was planned. Resume with `{}`.",
                cli_hint(pair, "resume")
            );
        }
        Some(PlanOutcome::Failed { error, .. }) => {
            eprintln!("Could not work out a plan: {error}");
        }
        Some(PlanOutcome::Computing { .. }) => {
            eprintln!("The daemon is still working out the plan.");
        }
        Some(PlanOutcome::Absent) => {
            eprintln!("The daemon has not worked out a plan yet.");
        }
        // The immediate ack `--all-pairs` reports instead of waiting (it never calls `watch_plan`,
        // so this is the only caller that can see this arm): scheduled, not computed, and that is
        // success, the same reading `ApplyOutcome::Scheduled` already gets below.
        Some(PlanOutcome::Scheduled { .. }) => {
            if !json {
                println!(
                    "Plan scheduled; watch it with `{}`.",
                    cli_hint(pair, "status")
                );
            }
        }
        // A state this client does not know, and the `None` an older daemon sends. Neither is an
        // empty plan.
        Some(PlanOutcome::Unknown) | None => {
            eprintln!("The daemon did not return a plan.");
        }
    }
    plan_exit_code(response.plan.as_ref())
}

/// The exit code an `apply` reply's outcome earns — [`list_exit_code`]'s sibling, read by both
/// [`report_apply`] and `reply_exit_code`. [`ApplyOutcome::Applied`] fails only when it landed
/// items that themselves failed (#136's partial outcome); [`ApplyOutcome::Scheduled`] is the
/// `--all-pairs` immediate ack and succeeds like `Computed`/`Scheduled` do for a plan; a
/// divergence, a stale token, a pause, an outright failure, and an outcome this build does not
/// know all fail — nothing ran, or nothing is known to have.
fn apply_exit_code(apply: Option<&ApplyOutcome>) -> ExitCode {
    match apply {
        Some(ApplyOutcome::Applied { failed, .. }) => {
            if *failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Some(ApplyOutcome::Scheduled { .. }) => ExitCode::SUCCESS,
        Some(ApplyOutcome::Diverged { .. }) => ExitCode::FAILURE,
        Some(ApplyOutcome::Stale) => ExitCode::FAILURE,
        Some(ApplyOutcome::Paused) => ExitCode::FAILURE,
        Some(ApplyOutcome::Failed { .. }) => ExitCode::FAILURE,
        Some(ApplyOutcome::Unknown) | None => ExitCode::FAILURE,
    }
}

/// Renders an apply reply. Prints only; the exit code is [`apply_exit_code`]'s alone. A divergence
/// prints the new plan: nothing ran, and the user has something to review.
///
/// `pair` is the selector this command was run with — see [`report_plan`]'s doc for why it is not
/// re-derived from `response.pair`.
fn report_apply(
    response: &ControlResponse,
    json: bool,
    style: &Style,
    pair: Option<&str>,
) -> ExitCode {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&response.apply).expect("serialize the apply outcome")
        );
    }
    match &response.apply {
        Some(ApplyOutcome::Applied {
            executed,
            skipped_destructive,
            failed,
            ..
        }) => {
            if !json {
                let mut clauses = vec![format!("{executed} action(s) applied")];
                if *skipped_destructive > 0 {
                    clauses.push(format!("{skipped_destructive} deletion(s) skipped"));
                }
                if *failed > 0 {
                    clauses.push(format!("{failed} failed"));
                }
                let line = clauses.join(" · ");
                if *failed > 0 {
                    println!("{} {line}", style.yellow("!"));
                } else {
                    println!("{} {line}", style.green("✓"));
                }
            }
        }
        Some(ApplyOutcome::Diverged { .. }) => {
            if !json {
                // Two sentences, because the plan is no longer guaranteed to be here. The wait
                // polls with a minimal window and fetches the real one once, on this verdict alone
                // (#321) — and that fetch can fail, which is not a reason to fail the command:
                // nothing was applied either way.
                if response.plan.is_some() {
                    eprintln!(
                        "The plan changed since you reviewed it, so nothing was applied. The new \
                         plan:"
                    );
                    // Not a second rendering: the reply carries the fresh plan under the same field
                    // `plan` prints from. Same `pair` this apply was run with, so `run_it_line`
                    // inside it echoes it too.
                    report_plan(response, false, style, pair);
                } else {
                    eprintln!(
                        "The plan changed since you reviewed it, so nothing was applied. Run \
                         `{}` to see the new one.",
                        cli_hint(pair, "plan")
                    );
                }
            }
        }
        Some(ApplyOutcome::Stale) => {
            eprintln!(
                "That plan is no longer the current one. Run `{}` again and apply the token it \
                 prints.",
                cli_hint(pair, "plan")
            );
        }
        Some(ApplyOutcome::Paused) => {
            eprintln!(
                "Syncing is paused, so nothing was applied. Resume with `{}`.",
                cli_hint(pair, "resume")
            );
        }
        Some(ApplyOutcome::Failed { error, .. }) => {
            eprintln!("The apply failed: {error}");
        }
        Some(ApplyOutcome::Scheduled { .. }) => {
            if !json {
                println!(
                    "Apply scheduled; watch it with `{}`.",
                    cli_hint(pair, "status")
                );
            }
        }
        Some(ApplyOutcome::Unknown) | None => {
            eprintln!("The daemon did not say what happened to the apply.");
        }
    }
    apply_exit_code(response.apply.as_ref())
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

/// How long a withheld deletion has been waiting, or `None` when the age cannot be stated: the
/// daemon did not say (an older daemon sends `0`), or the stamp is in the future. The latter is a
/// skewed clock, and `saturating_sub` would render it `0s` — asserting the item was first seen just
/// now, which is the one answer a stale-age display must never give.
fn relative_age(first_seen_epoch_secs: u64) -> Option<String> {
    if first_seen_epoch_secs == 0 {
        return None;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let seconds = now.checked_sub(first_seen_epoch_secs)?;
    Some(match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    })
}

fn print_pending(pending: &[PendingDeletion], pair: Option<&str>) {
    if pending.is_empty() {
        println!("No deletions are pending approval.");
        return;
    }
    println!("{} deletion(s) awaiting approval:", pending.len());
    for item in pending {
        // THE EFFECT IS THE DISPOSAL'S, NOT THE DIRECTION'S. `local` stopped meaning "gone for
        // good" when `local_delete_mode` arrived, and this line is the whole reason a person reads
        // the command — so it is asked of the daemon's own `disposal`, which says what THIS daemon
        // will do, rather than re-derived from the side the deletion applies to.
        let (label, effect) = match (item.direction, item.disposal) {
            (DeleteDirection::Local, LocalDisposal::Recoverable) => (
                "LOCAL DELETE ",
                "was deleted on Proton Drive; approving moves your copy to this computer's trash",
            ),
            (DeleteDirection::Local, LocalDisposal::Permanent) => (
                "LOCAL DELETE ",
                "was deleted on Proton Drive; approving removes your local copy for good",
            ),
            (DeleteDirection::Remote, _) => (
                "REMOTE DELETE",
                "was deleted locally; approving removes it on Proton Drive",
            ),
        };
        println!("  {label}  {}  ({effect})", item.path.display());
        // `first_seen_epoch_secs`, NEVER `detected_epoch_secs`: the second is the age of the pass
        // that re-derived this item and refreshes every ~30s (#225). A zero means an older daemon
        // cannot say, so the line is omitted rather than aged from 1970. The subtree line is what a
        // folder would actually cost (#208); a file's own size is not repeated here.
        let waiting = relative_age(item.first_seen_epoch_secs);
        let subtree = item.subtree_files.map(|files| {
            match item.subtree_bytes {
                // `human_bytes`, the same formatter the activity line uses — a raw byte count is
                // the one number on this line nobody reads. A missing total is dropped rather than
                // printed as `0 B`, which is a real answer for an empty folder.
                Some(bytes) => format!("{files} file(s), {}", human_bytes(bytes)),
                None => format!("{files} file(s)"),
            }
        });
        match (waiting, subtree) {
            (Some(waiting), Some(subtree)) => {
                println!("                 waiting {waiting} · holds {subtree}")
            }
            (Some(waiting), None) => println!("                 waiting {waiting}"),
            (None, Some(subtree)) => println!("                 holds {subtree}"),
            (None, None) => {}
        }
    }
    println!(
        "Approve with: {}   (or --all)",
        cli_hint(pair, "approve <path>")
    );
    println!(
        "Keep it instead: {}   (or --all)",
        cli_hint(pair, "keep <path>")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use proton_drive_sync_engine::ipc::{PairSummary, TransferActivity};
    use proton_drive_sync_engine::sync::UnsyncableReason;
    use std::sync::{Arc, Mutex};

    fn blank_activity(phase: &str) -> SyncActivity {
        SyncActivity {
            phase: phase.to_owned(),
            detail: None,
            folders_listed: None,
            files_scanned: None,
            action_index: None,
            action_total: None,
            transfers: Vec::new(),
            transfers_remaining: None,
            transfer: None,
            since_epoch_secs: None,
            pass: None,
        }
    }

    /// A plain `Style`, so the assertions below read the words and not the escape codes.
    fn plain_style() -> Style {
        Style { enabled: false }
    }

    fn unsyncable(path: &str, reason: UnsyncableReason) -> UnsyncableItem {
        UnsyncableItem {
            path: PathBuf::from(path),
            entity_kind: EntityKind::File,
            reason,
            first_seen_epoch_secs: 0,
        }
    }

    #[test]
    fn cant_sync_groups_by_cause_and_names_each_local_kind() {
        // #232 added five reasons at once, and the whole value of this section is that it says
        // WHAT and WHY rather than counting. A group whose cause is blank is a count with extra
        // steps, which is the state #295 spent five months in.
        let items = vec![
            unsyncable(".cache/session.sock", UnsyncableReason::LocalSocket),
            unsyncable("projects/current", UnsyncableReason::LocalSymlink),
            unsyncable("run/queue", UnsyncableReason::LocalFifo),
            unsyncable("dev/loop0", UnsyncableReason::LocalDevice),
            unsyncable("odd", UnsyncableReason::LocalSpecialFile),
            unsyncable("Unsorted/Networth", UnsyncableReason::RemoteNotDownloadable),
        ];
        let out = unsyncable_lines(&items, &plain_style()).join("\n");

        assert!(out.contains("6 item(s) cannot be synced:"), "{out}");
        for reason in [
            UnsyncableReason::LocalSocket,
            UnsyncableReason::LocalSymlink,
            UnsyncableReason::LocalFifo,
            UnsyncableReason::LocalDevice,
            UnsyncableReason::LocalSpecialFile,
            UnsyncableReason::RemoteNotDownloadable,
        ] {
            let heading = format!("{} — {}", reason.as_str(), reason.describe());
            assert!(
                out.contains(&heading),
                "every cause gets its own heading, and the heading says what to change: {out}"
            );
        }
        for path in [
            ".cache/session.sock",
            "projects/current",
            "run/queue",
            "dev/loop0",
            "odd",
            "Unsorted/Networth",
        ] {
            assert!(out.contains(path), "{path} must be named: {out}");
        }
    }

    #[test]
    fn a_cause_this_build_does_not_know_is_still_printed_with_its_paths() {
        // The rule the hand-written serde impl exists for, at the surface that has to honour it:
        // a newer daemon's token groups and renders verbatim rather than dropping the rows under
        // it. Dropping them would make a file that cannot sync invisible to the one command that
        // exists to name it.
        let items = vec![
            unsyncable(
                "odd-one",
                UnsyncableReason::Other("local_wormhole".to_owned()),
            ),
            unsyncable(".cache/session.sock", UnsyncableReason::LocalSocket),
        ];
        let out = unsyncable_lines(&items, &plain_style()).join("\n");

        assert!(out.contains("local_wormhole"), "{out}");
        assert!(out.contains("odd-one"), "{out}");
        assert!(out.contains("local_socket"), "{out}");
    }

    #[test]
    fn nothing_unsyncable_prints_nothing_at_all() {
        // Not even the heading: `0 item(s) cannot be synced` is a section about an empty set, and
        // the reassurance under it would be reassuring nobody about nothing.
        assert!(unsyncable_lines(&[], &plain_style()).is_empty());
    }

    #[test]
    fn reset_index_refuses_to_send_anything_without_the_typed_confirmation() {
        // The gate is client-side because the wire carries no confirmation field: a daemon cannot
        // tell a confirmed request from an unconfirmed one. Refusing before the socket is opened is
        // what makes "nothing was sent" literally true.
        let error = build_request(&Commands::ResetIndex { yes: false }, None)
            .expect_err("an unconfirmed reset must not be sent");
        assert!(
            error.contains("--yes"),
            "the refusal must say how to confirm: {error}"
        );

        let request = build_request(&Commands::ResetIndex { yes: true }, None).expect("confirmed");
        assert_eq!(request.command, ControlCommand::ResetIndex);
        assert_eq!(request.argument, None);
        assert!(
            !request.literal_path,
            "reset-index carries no path selector"
        );
    }

    #[test]
    fn report_plan_treats_the_all_pairs_scheduled_ack_as_success() {
        // `--all-pairs` never waits (`watch_plan` alone extracts `plan_seq` and polls), so
        // `report_plan` is the only caller that can see `PlanOutcome::Scheduled` — and before this
        // it fell into the catch-all "the daemon did not return a plan" arm and exited non-zero on
        // every successful `proton-sync --all-pairs plan`.
        let mut response = blank_response();
        response.plan = Some(PlanOutcome::Scheduled { plan_seq: 3 });
        assert_eq!(
            report_plan(&response, false, &plain_style(), Some("work")),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn json_result_value_matches_the_single_pair_projection_for_every_projected_verb() {
        // Decision #10 / ADR 0005 §4 departure (5): `--all-pairs --json`'s per-pair `result` must
        // be the SAME value a single-pair `--json` invocation prints for that verb, not the whole
        // envelope. `history`/`activity`/`pending`/`list`/`plan`/`apply` each print one projected
        // field in `run_for_pair`; before this fix `run_all_pairs` always nested the whole
        // `ControlResponse`, so these six carried a second, redundant `pairs` array under `result`.
        let mut response = blank_response();
        response.history = None;
        assert_eq!(
            json_result_value(&Commands::History, &response),
            serde_json::to_value(&response.history).unwrap()
        );

        response.pending_deletions = vec![];
        assert_eq!(
            json_result_value(&Commands::Pending, &response),
            serde_json::to_value(&response.pending_deletions).unwrap()
        );

        response.plan = Some(PlanOutcome::Scheduled { plan_seq: 7 });
        let projected = json_result_value(&Commands::Plan { limit: None }, &response);
        assert_eq!(projected, serde_json::to_value(&response.plan).unwrap());
        // The whole-response shape would also carry `pairs` — the bug this pins against.
        assert!(
            projected.get("pairs").is_none(),
            "plan's projection must not carry the envelope: {projected}"
        );

        response.apply = Some(ApplyOutcome::Stale);
        assert_eq!(
            json_result_value(
                &Commands::Apply {
                    token: "tok".to_owned(),
                    skip_destructive: false,
                    no_wait: true,
                },
                &response
            ),
            serde_json::to_value(&response.apply).unwrap()
        );

        response.listing = None;
        assert_eq!(
            json_result_value(
                &Commands::List {
                    path: None,
                    limit: None
                },
                &response
            ),
            serde_json::to_value(&response.listing).unwrap()
        );

        // Everything else prints the whole envelope in `run_for_pair` too, so the projection stays
        // the whole response — `status` is the representative case.
        assert_eq!(
            json_result_value(&Commands::Status, &response),
            serde_json::to_value(&response).unwrap()
        );
    }

    #[test]
    fn reply_exit_code_matches_the_verbs_own_outcome() {
        // `run_all_pairs`'s human branch used to discard `print_pair_reply`'s return value
        // entirely (`{ ...; }` blocks), so a busy `list`, a paused `plan` or a failed `apply`
        // under `--all-pairs` printed an error line but the process still exited 0. Both branches
        // now fold `reply_exit_code` instead — see `all_pairs_json_list_exits_by_its_own_outcome`
        // and its plan/apply siblings for the JSON twin of that bug (#409's Copilot finding).
        let mut failing_list = blank_response();
        failing_list.listing = Some(ListingOutcome::Busy);
        assert_eq!(
            reply_exit_code(
                &Commands::List {
                    path: None,
                    limit: None
                },
                &failing_list
            ),
            ExitCode::FAILURE
        );

        let mut failing_plan = blank_response();
        failing_plan.plan = Some(PlanOutcome::Paused);
        assert_eq!(
            reply_exit_code(&Commands::Plan { limit: None }, &failing_plan),
            ExitCode::FAILURE
        );

        let ok = blank_response();
        assert_eq!(reply_exit_code(&Commands::Status, &ok), ExitCode::SUCCESS);
    }

    #[test]
    fn list_exit_code_covers_every_listing_outcome() {
        assert_eq!(
            list_exit_code(Some(&ListingOutcome::Listed {
                path: PathBuf::new(),
                entries: Vec::new(),
                total: 0,
                truncated: false,
            })),
            ExitCode::SUCCESS
        );
        assert_eq!(
            list_exit_code(Some(&ListingOutcome::Busy)),
            ExitCode::FAILURE
        );
        assert_eq!(
            list_exit_code(Some(&ListingOutcome::Failed {
                error: "x".to_owned()
            })),
            ExitCode::FAILURE
        );
        assert_eq!(
            list_exit_code(Some(&ListingOutcome::Unknown)),
            ExitCode::FAILURE
        );
        assert_eq!(list_exit_code(None), ExitCode::FAILURE);
    }

    #[test]
    fn plan_exit_code_covers_every_plan_outcome() {
        assert_eq!(plan_exit_code(Some(&reviewed("tok"))), ExitCode::SUCCESS);
        // The `--all-pairs` immediate ack — decision #9a8d304 pinned for `report_plan`, restated
        // here because `plan_exit_code` is now the one place that decision lives.
        assert_eq!(
            plan_exit_code(Some(&PlanOutcome::Scheduled { plan_seq: 1 })),
            ExitCode::SUCCESS
        );
        assert_eq!(
            plan_exit_code(Some(&PlanOutcome::Paused)),
            ExitCode::FAILURE
        );
        assert_eq!(
            plan_exit_code(Some(&PlanOutcome::Failed {
                plan_seq: 1,
                error: "x".to_owned()
            })),
            ExitCode::FAILURE
        );
        assert_eq!(
            plan_exit_code(Some(&PlanOutcome::Computing { plan_seq: 1 })),
            ExitCode::FAILURE
        );
        assert_eq!(
            plan_exit_code(Some(&PlanOutcome::Absent)),
            ExitCode::FAILURE
        );
        assert_eq!(
            plan_exit_code(Some(&PlanOutcome::Unknown)),
            ExitCode::FAILURE
        );
        assert_eq!(plan_exit_code(None), ExitCode::FAILURE);
    }

    #[test]
    fn apply_exit_code_covers_every_apply_outcome() {
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Applied {
                apply_seq: 1,
                executed: 1,
                skipped_destructive: 0,
                failed: 0,
            })),
            ExitCode::SUCCESS
        );
        // The data-dependent arm: `Applied` only fails when it landed items that themselves
        // failed (#136). This is the case a `matches!` one-liner over the variant alone would
        // get wrong.
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Applied {
                apply_seq: 1,
                executed: 1,
                skipped_destructive: 0,
                failed: 1,
            })),
            ExitCode::FAILURE
        );
        // The `--all-pairs` immediate ack, success like `PlanOutcome::Scheduled`.
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Scheduled { apply_seq: 1 })),
            ExitCode::SUCCESS
        );
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Diverged { apply_seq: 1 })),
            ExitCode::FAILURE
        );
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Stale)),
            ExitCode::FAILURE
        );
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Paused)),
            ExitCode::FAILURE
        );
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Failed {
                apply_seq: 1,
                error: "x".to_owned()
            })),
            ExitCode::FAILURE
        );
        assert_eq!(
            apply_exit_code(Some(&ApplyOutcome::Unknown)),
            ExitCode::FAILURE
        );
        assert_eq!(apply_exit_code(None), ExitCode::FAILURE);
    }

    /// One `--all-pairs` run against a single configured pair: the capability probe, then this
    /// one verb reply. Returns the process's own exit code, exactly what `main` would return.
    fn run_all_pairs_with(json: bool, command: Commands, verb_reply: ControlResponse) -> ExitCode {
        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("control.sock");
        let probe_reply = ControlResponse {
            pairs: vec![pair_summary("default")],
            ..blank_response()
        };
        let cli = Cli {
            config: None,
            socket_path: None,
            json,
            pair: None,
            all_pairs: true,
            command,
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let server = tokio::spawn(serve_scripted(
                    socket_path.clone(),
                    vec![probe_reply, verb_reply],
                ));
                tokio::task::yield_now().await;
                let code = run_all_pairs(&cli, &socket_path, &Style { enabled: false }).await;
                server.abort();
                code
            })
    }

    /// The JSON twin of the bug 9a8d304 fixed for the human branch: `--all-pairs --json`'s fold
    /// used to check only `response.pair`, never the verb's own outcome, so a busy `list` printed
    /// its payload and exited 0.
    #[test]
    fn all_pairs_json_list_exits_by_its_own_outcome() {
        let busy = ControlResponse {
            listing: Some(ListingOutcome::Busy),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(
                true,
                Commands::List {
                    path: None,
                    limit: None
                },
                busy
            ),
            ExitCode::FAILURE,
            "a busy listing must fail --all-pairs --json"
        );

        let listed = ControlResponse {
            listing: Some(ListingOutcome::Listed {
                path: PathBuf::new(),
                entries: Vec::new(),
                total: 0,
                truncated: false,
            }),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(
                true,
                Commands::List {
                    path: None,
                    limit: None
                },
                listed
            ),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn all_pairs_json_plan_exits_by_its_own_outcome() {
        let paused = ControlResponse {
            plan: Some(PlanOutcome::Paused),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(true, Commands::Plan { limit: None }, paused),
            ExitCode::FAILURE,
            "a paused plan must fail --all-pairs --json"
        );

        // `Scheduled` is the immediate ack every `--all-pairs plan` reply actually carries (it
        // never watches), and it is a success — the case #409's finding calls out by name.
        let scheduled = ControlResponse {
            plan: Some(PlanOutcome::Scheduled { plan_seq: 3 }),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(true, Commands::Plan { limit: None }, scheduled),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn all_pairs_json_apply_exits_by_its_own_outcome() {
        fn apply_command() -> Commands {
            Commands::Apply {
                token: "tok".to_owned(),
                skip_destructive: false,
                no_wait: true,
            }
        }

        let diverged = ControlResponse {
            apply: Some(ApplyOutcome::Diverged { apply_seq: 1 }),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(true, apply_command(), diverged),
            ExitCode::FAILURE,
            "a diverged apply must fail --all-pairs --json"
        );

        // The data-dependent arm: applied, but with a failed item (#136's partial outcome).
        let partial = ControlResponse {
            apply: Some(ApplyOutcome::Applied {
                apply_seq: 1,
                executed: 1,
                skipped_destructive: 0,
                failed: 1,
            }),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(true, apply_command(), partial),
            ExitCode::FAILURE,
            "a partial apply must fail --all-pairs --json"
        );

        let scheduled = ControlResponse {
            apply: Some(ApplyOutcome::Scheduled { apply_seq: 7 }),
            ..blank_response()
        };
        assert_eq!(
            run_all_pairs_with(true, apply_command(), scheduled),
            ExitCode::SUCCESS
        );
    }

    /// The property that stops the two branches drifting again: for the same reply, the JSON
    /// branch and the human branch must exit with the same code. Each is driven through the real
    /// `run_all_pairs` (not the two `xxx_exit_code` functions directly), so a regression that
    /// reintroduces a second, divergent decision inside either branch is caught here even if it
    /// leaves both `xxx_exit_code` functions themselves untouched.
    #[test]
    fn all_pairs_json_and_human_branches_exit_alike() {
        let busy_list = ControlResponse {
            listing: Some(ListingOutcome::Busy),
            ..blank_response()
        };
        let json_code = run_all_pairs_with(
            true,
            Commands::List {
                path: None,
                limit: None,
            },
            busy_list.clone(),
        );
        let human_code = run_all_pairs_with(
            false,
            Commands::List {
                path: None,
                limit: None,
            },
            busy_list,
        );
        assert_eq!(json_code, human_code);
        assert_eq!(json_code, ExitCode::FAILURE);

        let scheduled_plan = ControlResponse {
            plan: Some(PlanOutcome::Scheduled { plan_seq: 3 }),
            ..blank_response()
        };
        let json_code =
            run_all_pairs_with(true, Commands::Plan { limit: None }, scheduled_plan.clone());
        let human_code = run_all_pairs_with(false, Commands::Plan { limit: None }, scheduled_plan);
        assert_eq!(json_code, human_code);
        assert_eq!(json_code, ExitCode::SUCCESS);

        let partial_apply = ControlResponse {
            apply: Some(ApplyOutcome::Applied {
                apply_seq: 1,
                executed: 1,
                skipped_destructive: 0,
                failed: 1,
            }),
            ..blank_response()
        };
        let apply_command = || Commands::Apply {
            token: "tok".to_owned(),
            skip_destructive: false,
            no_wait: true,
        };
        let json_code = run_all_pairs_with(true, apply_command(), partial_apply.clone());
        let human_code = run_all_pairs_with(false, apply_command(), partial_apply);
        assert_eq!(json_code, human_code);
        assert_eq!(json_code, ExitCode::FAILURE);
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
        transfer.transfers = vec![TransferActivity {
            bytes_done: Some(1_500_000_000),
            // Far future → zero elapsed → no elapsed fragment, keeping the assertion stable.
            ..TransferActivity::active(
                "download",
                PathBuf::from("Companies/takeout.tgz"),
                None,
                u64::MAX,
            )
        }];
        assert_eq!(
            describe_activity(&transfer),
            "downloading Companies/takeout.tgz — 1.4 GiB so far [step 812/6377]"
        );

        // A batched download is one row over a chunk: it names the folder and the file count, so
        // the folder is not rendered as if it were the file.
        let mut batch = blank_activity("executing");
        batch.action_index = Some(40);
        batch.action_total = Some(100);
        batch.transfers = vec![TransferActivity {
            files: Some(25),
            ..TransferActivity::active("download", PathBuf::from("photos/2024"), None, u64::MAX)
        }];
        assert_eq!(
            describe_activity(&batch),
            "downloading 25 files in photos/2024 [step 40/100]"
        );

        // A daemon predating #211 sends only the singular mirror, and the line is unchanged.
        let mut legacy = blank_activity("executing");
        legacy.action_index = Some(1);
        legacy.action_total = Some(2);
        legacy.transfer = Some(TransferActivity {
            bytes_total: Some(4_400_000),
            ..TransferActivity::active("upload", PathBuf::from("docs/report.pdf"), None, u64::MAX)
        });
        assert_eq!(
            describe_activity(&legacy),
            "uploading docs/report.pdf — 4.2 MiB [step 1/2]"
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
            index_totals: None,
            listing: None,
            plan: None,
            apply: None,
            auth: AuthState::Unknown,
            pair: Some("default".to_owned()),
            pairs: Vec::new(),
        }
    }

    fn sweep(outcome: &str, changed: usize, failed: usize) -> PassRecord {
        PassRecord {
            id: 1,
            started_epoch_secs: 100,
            duration_ms: 1000,
            kind: "full-sweep".to_owned(),
            outcome: outcome.to_owned(),
            changed,
            failed,
            bytes_uploaded: 0,
            bytes_downloaded: 0,
            error: Some("proton-drive: request\n  failed".to_owned()),
        }
    }

    #[test]
    fn an_error_with_newlines_stays_on_one_line() {
        // A daemon error carries the CLI child's stderr verbatim; inline it would otherwise break
        // the sentence in half and leave the full stop alone on the next line.
        assert_eq!(one_line("boom\n"), "boom");
        assert_eq!(
            one_line("list failed:\n  boom\n\n  twice"),
            "list failed: boom twice"
        );
        assert_eq!(one_line("already tidy"), "already tidy");
        assert!(!describe_sweep(&sweep("failed", 0, 0)).contains('\n'));
    }

    #[test]
    fn a_full_sweep_that_did_not_finish_is_never_an_all_clear() {
        // #246's shape, in the one sentence #238 asks for. A failed or interrupted sweep also has
        // `changed == 0`, so keying the clause off the count alone printed "nothing was out of
        // step" over a pass that never got far enough to know.
        assert_eq!(
            describe_sweep(&sweep("clean", 0, 0)),
            "nothing was out of step"
        );
        assert_eq!(describe_sweep(&sweep("clean", 3, 0)), "3 change(s)");
        assert_eq!(
            describe_sweep(&sweep("partial", 2, 1)),
            "2 change(s), 1 item(s) failed"
        );
        assert_eq!(
            describe_sweep(&sweep("failed", 0, 0)),
            "it failed: proton-drive: request failed",
            "the embedded newline is collapsed, not printed"
        );
        assert_eq!(
            describe_sweep(&sweep("interrupted", 0, 0)),
            "it did not finish"
        );
        // A token this build does not know is rendered, never silently treated as clean.
        assert_eq!(
            describe_sweep(&sweep("quiesced", 0, 0)),
            "it ended `quiesced`"
        );
        for outcome in ["failed", "interrupted", "quiesced"] {
            assert!(
                !describe_sweep(&sweep(outcome, 0, 0)).contains("nothing was out of step"),
                "{outcome} must not read as an all-clear"
            );
        }
    }

    #[test]
    fn a_shorter_list_than_its_own_count_says_so() {
        // "a count is a claim": `total` is a SQL count over the window, and the list is shortened
        // by `--limit` and by any row whose action token this build does not know. Drives the
        // function `print_activity` actually calls, so deleting the branch fails this.
        assert_eq!(shown_prefix(2, 6), "showing 2 of ");
        assert_eq!(shown_prefix(6, 6), "");
        // Never "showing 6 of 2": a count below the rows would mean the page and the counts
        // described different sets, which the shared SQL predicate makes impossible.
        assert_eq!(shown_prefix(6, 2), "");
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

    fn pair_summary(name: &str) -> PairSummary {
        PairSummary {
            name: name.to_owned(),
            local_root: PathBuf::from("/local"),
            remote_root: PathBuf::from("/Drive/Remote"),
            db_path: PathBuf::from("/local/.sync/index.db"),
            paused: false,
            syncing: false,
            reconcile_seq: 1,
            last_sync_epoch_secs: None,
            last_error: None,
            pending_changes: 0,
            pending_deletions: 0,
        }
    }

    #[test]
    fn the_headline_names_the_pair_only_when_more_than_one_is_configured() {
        // Decision #14 (ADR 0005 §4): "everything is up to date" silently meaning one of three
        // folders is #246's lie read the other way round — but a one-pair daemon (today's only
        // shape) must print exactly what it prints before #102 phase 3.
        let style = Style { enabled: false };
        let single = blank_response();
        let line = status_headline_line(&single, &style);
        assert!(
            !line.contains("default"),
            "single configured pair must not be named: {line}"
        );

        let mut multi = blank_response();
        multi.pairs = vec![pair_summary("default"), pair_summary("second")];
        let line = status_headline_line(&multi, &style);
        assert!(
            line.contains("default"),
            "multi-pair headline must name the selected pair: {line}"
        );

        // An unresolved selector (`pair: None`) names nothing — there is no pair to name.
        let mut unresolved = multi;
        unresolved.pair = None;
        let line = status_headline_line(&unresolved, &style);
        assert!(
            !line.contains("default") && !line.contains("second"),
            "an unresolved selector must not name a pair: {line}"
        );
    }

    #[test]
    fn human_bytes_scales_through_the_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_bytes(1_500_000_000), "1.4 GiB");
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_secs()
    }

    #[test]
    fn relative_age_scales_and_refuses_what_it_cannot_state() {
        let now = now_secs();
        assert_eq!(relative_age(now - 5).as_deref(), Some("5s"));
        assert_eq!(relative_age(now - 600).as_deref(), Some("10m"));
        assert_eq!(relative_age(now - 7200).as_deref(), Some("2h"));
        assert_eq!(relative_age(now - 3 * 86_400).as_deref(), Some("3d"));

        // An older daemon omits the field, which deserializes to 0.
        assert_eq!(relative_age(0), None);

        // A stamp in the future is a skewed clock. `saturating_sub` would render it "0s" —
        // "first seen just now" is the one answer this display must never invent, since the whole
        // point of the field (#225) is that the age was previously re-stamped to now every pass.
        assert_eq!(relative_age(now + 3600), None);
    }

    #[test]
    fn an_unreadable_page_is_never_reported_as_an_empty_window() {
        // Rows this build cannot decode are dropped from `events` but still counted by `total`
        // (`file_events` pages through `read_file_event`, counts with `COUNT(*)`). An older client
        // against a newer daemon therefore sees `events: [], total: n` — and "Nothing has moved"
        // would be a false all-clear about the user's data, not a cosmetic wording slip.
        assert_eq!(
            empty_activity_message(0),
            "Nothing has moved in that window."
        );
        for total in [1, 42] {
            let rendered = empty_activity_message(total);
            assert!(
                rendered.contains(&total.to_string()) && rendered.contains("Upgrade"),
                "an unreadable page must say how many and what to do: {rendered}"
            );
            assert!(
                !rendered.contains("Nothing has moved"),
                "must not claim an empty window: {rendered}"
            );
        }
    }

    #[test]
    fn a_refused_history_reports_the_daemon_s_reason() {
        // `message` is the generic reply label; the reason lives in `last_error`. Printing only the
        // label turns "I failed, here is why" into "I do not support that", a different claim.
        assert_eq!(
            missing_history_message("daemon status", Some("database is locked")),
            "daemon status: database is locked"
        );
        assert_eq!(
            missing_history_message("daemon status", None),
            "This daemon does not report per-file activity."
        );
    }

    /// An empty plan offers no `apply` command: there is nothing to authorise, and printing one
    /// under "Nothing would change — both sides already match" contradicts the line above it.
    #[test]
    fn an_empty_plan_offers_nothing_to_run() {
        let style = plain_style();
        assert_eq!(run_it_line(0, "1:abc", None, &style), None);
        assert_eq!(
            run_it_line(1, "1:abc", None, &style),
            Some("Run it with: proton-sync apply 1:abc".to_owned())
        );
    }

    /// The bug class the `refetch_plan_if_diverged` fix belongs to, in text rather than in a
    /// request: a hint printed after `--pair work ...` must echo `--pair work`, or a user who
    /// pastes it verbatim is silently sent to the default pair (ADR 0005 §4's selector, omitted,
    /// resolves to index 0 — never the pair the hint was printed for).
    #[test]
    fn cli_hint_echoes_the_selector_the_command_was_run_with() {
        assert_eq!(cli_hint(None, "plan"), "proton-sync plan");
        assert_eq!(
            cli_hint(Some("work"), "plan"),
            "proton-sync --pair work plan"
        );
    }

    /// `run_it_line` is the primary paste target of the plan/apply pair (#321's own hint), so it
    /// carries the selector too — even though a wrong-pair `apply` usually fails loud (`Stale`,
    /// since a token is a content hash of the *other* pair's rows and only coincidentally matches
    /// this pair's stored plan), the wording is still wrong and still worth fixing.
    #[test]
    fn run_it_line_carries_the_pair_selector() {
        let style = plain_style();
        assert_eq!(
            run_it_line(1, "1:abc", Some("work"), &style),
            Some("Run it with: proton-sync --pair work apply 1:abc".to_owned())
        );
    }

    /// `Computing` carries the last generation **answered** (`slot.completed`), never the one in
    /// flight — so a wait must never resolve on it, even when that number has caught up with its
    /// own target. It can: our pass completes (`completed == our plan_seq`) and a second client
    /// books the next one, which is `requested > completed` again. Comparing the number there would
    /// end our wait on someone else's in-flight request and report a plan we never asked for.
    #[test]
    fn a_computing_reply_answers_no_generation_however_high_its_number() {
        for plan_seq in [0, 1, 2, 7] {
            assert_eq!(
                plan_generation(Some(&PlanOutcome::Computing { plan_seq })),
                None,
                "Computing must never satisfy a wait (plan_seq {plan_seq})"
            );
        }
        // The ack is not an answer either, for the same reason: it precedes the pass.
        assert_eq!(
            plan_generation(Some(&PlanOutcome::Scheduled { plan_seq: 2 })),
            None
        );
        // Only these two answer anything.
        assert_eq!(
            plan_generation(Some(&PlanOutcome::Failed {
                plan_seq: 2,
                error: "x".to_owned()
            })),
            Some(2)
        );
    }

    /// Serves one canned [`ControlResponse`] per connection, in order, on a real control socket —
    /// the smallest fake daemon [`poll_until`] cannot tell from the real one.
    async fn serve_scripted(socket_path: PathBuf, replies: Vec<ControlResponse>) {
        serve_recording(socket_path, replies, Arc::new(Mutex::new(Vec::new()))).await
    }

    /// [`serve_scripted`], keeping every request it was sent — the only way to assert the *shape*
    /// of a poll (its `limit`), which is what #321 is about.
    async fn serve_recording(
        socket_path: PathBuf,
        replies: Vec<ControlResponse>,
        seen: Arc<Mutex<Vec<ControlRequest>>>,
    ) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind");
        for reply in replies {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            seen.lock().expect("seen lock").push(
                serde_json::from_str::<ControlRequest>(line.trim_end()).expect("decode request"),
            );
            let mut stream = reader.into_inner();
            let mut body = serde_json::to_vec(&reply).expect("serialize");
            body.push(b'\n');
            stream.write_all(&body).await.expect("write response");
            stream.flush().await.expect("flush");
        }
    }

    /// `--all-pairs stop` used to loop like every other verb, sending one shutdown request per
    /// configured pair (ADR 0005 §4's verb table settles `shutdown` as daemon-wide, selector
    /// ignored). With N pairs the first shutdown kills the daemon and every later iteration fails
    /// to connect, reporting a clean stop as a failure. A third reply is scripted so a regression
    /// that still loops is caught by the request COUNT, not by a starved connection that would
    /// otherwise only show up as a wrong exit code.
    #[test]
    fn all_pairs_stop_sends_exactly_one_shutdown_request() {
        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("control.sock");
        let seen = Arc::new(Mutex::new(Vec::new()));

        let probe_reply = ControlResponse {
            pairs: vec![pair_summary("default"), pair_summary("second")],
            ..blank_response()
        };
        let shutdown_reply = ControlResponse {
            message: "shutting down".to_owned(),
            ..blank_response()
        };

        let cli = Cli {
            config: None,
            socket_path: None,
            json: false,
            pair: None,
            all_pairs: true,
            command: Commands::Stop,
        };

        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let server = tokio::spawn(serve_recording(
                    socket_path.clone(),
                    vec![probe_reply, shutdown_reply.clone(), shutdown_reply],
                    Arc::clone(&seen),
                ));
                // `run_all_pairs`'s first request has no retry (unlike `poll_until`'s callers),
                // so the listener must exist before it connects — one yield is enough for the
                // spawned task to reach its own first pending await (`accept`).
                tokio::task::yield_now().await;
                let code = run_all_pairs(&cli, &socket_path, &Style { enabled: false }).await;
                server.abort();
                code
            });

        assert_eq!(code, ExitCode::SUCCESS);
        let requests = seen.lock().expect("seen lock").clone();
        assert_eq!(
            requests.len(),
            2,
            "one capability probe, then exactly one shutdown — never one per pair: {requests:?}"
        );
        assert_eq!(requests[0].command, ControlCommand::Status);
        assert_eq!(requests[1].command, ControlCommand::Shutdown);
    }

    /// A pause **does not** end a `plan`/`apply` wait, and must not be "made consistent" with
    /// [`watch_syncnow`], which does bail on one.
    ///
    /// The two watch counters with opposite guarantees: a paused `syncnow` is never sealed (no
    /// `reconcile_seq` bump), so only the client can end that wait, while every plan/apply request
    /// is always sealed. Bailing here was a false positive — a plan booked before a pause still
    /// runs (`plan_now` has no pause check), so the poller must keep asking until the seal lands
    /// rather than report a pause for a plan that completes. The scripted daemon below is paused
    /// with nothing running on the first poll and answers on the second, which is exactly the
    /// ack→`syncing.store(true)` window.
    #[test]
    fn a_pause_mid_wait_does_not_end_a_plan_or_apply_wait() {
        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("control.sock");

        let paused_and_idle = ControlResponse {
            paused: true,
            syncing: false,
            plan: None,
            ..blank_response()
        };
        // The seal. `Failed` is the smallest one that still counts as an answer to generation 1
        // (see `plan_generation`), so the test needs no fixture plan to assert the rule.
        let sealed = ControlResponse {
            paused: true,
            syncing: false,
            plan: Some(PlanOutcome::Failed {
                plan_seq: 1,
                error: "the walk failed".to_owned(),
            }),
            ..blank_response()
        };

        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let server = tokio::spawn(serve_scripted(
                    socket_path.clone(),
                    vec![paused_and_idle, sealed],
                ));
                // Bounded, and the server is aborted rather than awaited: a regression here must
                // FAIL, not hang. Two polls at `WAIT_POLL_INTERVAL` sit far inside this, while a
                // wait that ended early leaves the scripted daemon holding a reply nobody asks for.
                let outcome = tokio::time::timeout(
                    Duration::from_secs(10),
                    poll_until(
                        &socket_path,
                        || ControlRequest::new(ControlCommand::PlanResult),
                        |response| {
                            plan_generation(response.plan.as_ref()).is_some_and(|seq| seq >= 1)
                        },
                    ),
                )
                .await;
                server.abort();
                outcome
            });

        let response = outcome
            .expect("the wait must finish well inside the timeout")
            .expect("a pause mid-wait must not end the wait");
        assert_eq!(
            plan_generation(response.plan.as_ref()),
            Some(1),
            "the wait must return the daemon's own sealed verdict, not a pause the client inferred"
        );
    }

    fn reviewed(token: &str) -> PlanOutcome {
        PlanOutcome::Computed(Box::new(ReviewedPlan {
            plan_seq: 3,
            token: token.to_owned(),
            computed_epoch_secs: 100,
            summary: PlanSummary::default(),
            actions: Vec::new(),
            total: 0,
            truncated: false,
            cannot_sync: Vec::new(),
            local_disposal: LocalDisposal::Permanent,
        }))
    }

    /// #321: the apply wait reads `response.apply` and nothing else, so it must not ask for a plan
    /// window on every 300 ms poll — and a **divergence** is the one verdict that does need the
    /// plan, fetched once, afterwards, at the display limit.
    #[test]
    fn an_apply_wait_polls_a_minimal_window_and_fetches_the_plan_only_on_a_divergence() {
        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("control.sock");
        let seen = Arc::new(Mutex::new(Vec::new()));

        let ack = ControlResponse {
            apply: Some(ApplyOutcome::Scheduled { apply_seq: 7 }),
            ..blank_response()
        };
        // The verdict the wait ends on. Its own reply carries no plan, because the minimal window
        // is what the poll asked for.
        let diverged = ControlResponse {
            apply: Some(ApplyOutcome::Diverged { apply_seq: 7 }),
            plan: None,
            ..blank_response()
        };
        // The follow-up. Its `apply` is deliberately a DIFFERENT, newer verdict: only `plan` may be
        // taken from it, or a second client's apply landing between the two requests would be
        // reported under this command's token.
        let refetched = ControlResponse {
            apply: Some(ApplyOutcome::Applied {
                apply_seq: 9,
                executed: 4,
                skipped_destructive: 0,
                failed: 0,
            }),
            plan: Some(reviewed("tok-new")),
            ..blank_response()
        };

        let code = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let server = tokio::spawn(serve_recording(
                    socket_path.clone(),
                    vec![diverged, refetched],
                    Arc::clone(&seen),
                ));
                let code = tokio::time::timeout(
                    Duration::from_secs(10),
                    watch_apply(
                        &socket_path,
                        ack,
                        false,
                        false,
                        &Style { enabled: false },
                        Some("work"),
                    ),
                )
                .await
                .expect("the wait must finish well inside the timeout");
                server.abort();
                code
            });

        assert_eq!(
            code,
            ExitCode::FAILURE,
            "a divergence applied nothing, so it is not a success"
        );
        let requests = seen.lock().expect("seen lock").clone();
        assert_eq!(
            requests.len(),
            2,
            "one poll, then one plan fetch: {requests:?}"
        );
        assert_eq!(
            requests[0].limit,
            Some(1),
            "the poll must ask for the smallest window the daemon will build, not the default 500"
        );
        assert_eq!(
            requests[1].limit, None,
            "and the divergence fetch must ask at the display limit, which is the daemon's default"
        );
        assert!(
            requests
                .iter()
                .all(|request| request.command == ControlCommand::PlanResult),
            "both round trips are plan_result reads: {requests:?}"
        );
        assert!(
            requests
                .iter()
                .all(|request| request.pair.as_deref() == Some("work")),
            "the divergence refetch used to address the default pair no matter which pair the \
             apply was for: {requests:?}"
        );
    }

    /// The other half of the same rule, tested at the seam because the merge is invisible from
    /// outside `watch_apply`: a failed re-fetch reports the divergence **without** a plan rather
    /// than failing the command, and the verdict reported is always the one the wait waited for.
    #[test]
    fn a_failed_divergence_refetch_keeps_the_verdict_and_drops_the_plan() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing_socket = directory.path().join("nothing-here.sock");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let diverged = ControlResponse {
            apply: Some(ApplyOutcome::Diverged { apply_seq: 7 }),
            plan: Some(reviewed("tok-stale")),
            ..blank_response()
        };
        let answered = runtime.block_on(refetch_plan_if_diverged(
            &missing_socket,
            diverged.clone(),
            7,
            None,
        ));
        assert_eq!(
            answered.apply,
            Some(ApplyOutcome::Diverged { apply_seq: 7 }),
            "nothing was applied, and that is the fact the user needs even with no plan to show"
        );
        assert_eq!(
            answered.plan, None,
            "the stale window must not be printed as the new plan"
        );

        // Anything that is not this wait's divergence is passed straight through — a re-fetch on an
        // `Applied` verdict would be a round trip for a field nothing reads.
        let applied = ControlResponse {
            apply: Some(ApplyOutcome::Applied {
                apply_seq: 7,
                executed: 1,
                skipped_destructive: 0,
                failed: 0,
            }),
            plan: Some(reviewed("tok-kept")),
            ..blank_response()
        };
        let answered =
            runtime.block_on(refetch_plan_if_diverged(&missing_socket, applied, 7, None));
        assert!(
            matches!(answered.plan, Some(PlanOutcome::Computed(_))),
            "a non-divergent verdict must not be touched at all"
        );
    }
}
