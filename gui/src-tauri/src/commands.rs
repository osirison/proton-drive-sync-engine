//! The Tauri command surface. Every command is a thin wrapper over `gui-core`; **screens** are
//! frontend-only and never add to this list, `generate_handler!`, or `Cargo.toml`. That is what
//! keeps the parallel screen tasks (S1–S11) collision-free.
//!
//! The **capability** tasks (C1–C6) are the sanctioned exception, and are how the list grows: a
//! screen that needs something the daemon does not expose files a C-item, and the C-item adds the
//! command before the screen is built. `free_space`, `check_cli` and `skip_rule_usage` arrived that
//! way. The rule that matters is the one about parallel screen work, not a freeze.
//!
//! **S6 added two, and they are recorded here rather than smuggled in.** `resync` and
//! `choose_folder` are the two controls on `8a Settings` that no existing command answers —
//! `Sweep now` is a full-tree walk (`Syncnow` is not one) and `Choose…` is a folder picker. Neither
//! was worth a C-item by the time S6 found them: both are five lines over machinery that already
//! exists (`ControlCommand::Resync` has shipped in the daemon since #160), the alternative was two
//! more dead buttons of the kind #224/#227 already record, and S6 is the last screen in flight, so
//! the collision the rule protects against cannot happen. A screen that needs *data* still files a
//! C-item — that is the case the rule is really about, and `skip_rule_usage` is why.
//!
//! **The four openers (#220/#231) arrived as a C-item, after every screen.** `open_paths`,
//! `open_folder`, `open_remote` and `open_system_log` are the one capability behind four buttons
//! that three screens drew and left inert — the rule above is what kept them inert rather than
//! smuggled in, and this is the sanctioned way they land: the command first, the screens wired to
//! it in the same change. See "the openers" below for why none of them takes a URL.
//!
//! **A command that touches the filesystem, a subprocess or a socket must be `async` and do its
//! work in `spawn_blocking`.** A synchronous one runs on the GTK main loop, and WebKitGTK aborts
//! the whole process when that loop stalls (#142/#143). `read_config`, `write_config`,
//! `resolve_conflict`, `read_conflict_pair` and `path_sync_status` predate the rule and are still
//! synchronous — `path_sync_status` in particular can hold the loop for its full 3s index busy
//! timeout. They are bounded enough to have survived; anything unbounded is not, and none of the
//! commands added since is synchronous. S9's two `notify_policy` commands were, for one commit, and
//! the review that caught them is the reason this sentence is checkable at all.

use crate::config_path::RuntimePaths;
use gui_core::conflicts::{self, Conflict, Resolution};
use gui_core::state::derive_state;
use gui_core::wire::{
    ApplyOutcome, ControlCommand, ControlRequest, ControlResponse, DeleteDirection, DryRunReport,
    LocalDisposal, PendingDeletion, PlanOutcome, PLAN_ACTIONS_MAX_LIMIT,
};
use gui_core::{config_io, index_read, ipc, plan};
use std::process::Command;
use std::sync::Mutex;
use tauri::{Manager, State};

type Paths<'a> = State<'a, Mutex<RuntimePaths>>;

/// A status round trip, with the derived UI state folded in so the frontend never re-derives it.
/// On a socket failure the `state` is `unreachable`/etc. and `error` is set — never zeroed counters.
#[derive(serde::Serialize)]
pub struct StatusPayload {
    state: gui_core::DaemonState,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<ControlResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn status_payload(result: Result<ControlResponse, ipc::IpcError>) -> StatusPayload {
    match result {
        Ok(response) => StatusPayload {
            state: derive_state(Ok(&response)),
            response: Some(response),
            error: None,
        },
        Err(error) => StatusPayload {
            state: derive_state(Err(&error)),
            response: None,
            error: Some(error.to_string()),
        },
    }
}

/// `Err` when the control socket cannot be located at all (#277) — a state as unreachable as a
/// refused connection, and reported with its own reason rather than a guessed path's ENOENT.
fn socket_path(state: &Paths) -> Result<std::path::PathBuf, String> {
    state.lock().unwrap().socket_path.clone()
}

/// [`socket_path`] as an `ipc` result, so an unresolvable socket folds into the same
/// `StatusPayload` error shape as an unreachable daemon.
fn socket_path_for_ipc(state: &Paths) -> Result<std::path::PathBuf, ipc::IpcError> {
    socket_path(state).map_err(ipc::IpcError::Unreachable)
}

/// Drop ANSI escape sequences (`ESC [ … <letter>`) from subprocess stderr. The daemon's tracing
/// output is coloured for terminals; rendered raw in the webview it turns error cards into
/// `[2m…[0m` soup.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Fold a status round trip into a payload AND cache the daemon-reported live config, so later
/// commands (conflict scan, emblems, dry run) can act on the roots the daemon is really syncing
/// even when the GUI-owned config file doesn't exist.
fn status_payload_remembering(
    state: &Paths,
    result: Result<ControlResponse, ipc::IpcError>,
) -> StatusPayload {
    if let Ok(response) = &result {
        if let Some(info) = &response.config {
            state.lock().unwrap().remember_daemon_config(info);
        }
    }
    status_payload(result)
}

/// Run a blocking control-socket round trip off the GTK main loop. Every socket command is async +
/// `spawn_blocking` so a slow-but-alive daemon (up to `DEFAULT_TIMEOUT`) never stalls the event
/// loop. A task-join failure (the blocking task panicked, or was cancelled on runtime shutdown) is
/// folded into an `Unreachable` error so callers keep the "socket failure → error state, never
/// zeroed counters" invariant.
async fn spawn_blocking_ipc<F>(f: F) -> Result<ControlResponse, ipc::IpcError>
where
    F: FnOnce() -> Result<ControlResponse, ipc::IpcError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_error) => Err(ipc::IpcError::Unreachable(format!(
            "control-socket task failed: {join_error}"
        ))),
    }
}

// The socket commands take an owned `AppHandle` rather than `State<'_>`: an async command with a
// *reference* input (`State<'_>`) is forced by Tauri to return a `Result`, but these commands never
// fail — a socket error is folded into the payload, never surfaced as a rejected promise. Owning
// the handle also lets us re-borrow the managed paths *after* the `.await` (to remember the daemon's
// live config from the reply), which a `State<'_>` guard cannot cross.

/// The shared body of the no-argument status commands (`get_status`/`pause`/`resume`/`sync_now`):
/// one control-socket round trip run off the main loop, folded into a `StatusPayload`. Generic over
/// the runtime so a `tauri::test` mock app can drive it headlessly (see tests).
async fn status_round_trip<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    command: ControlCommand,
) -> StatusPayload {
    let socket = match socket_path_for_ipc(&app.state()) {
        Ok(socket) => socket,
        Err(error) => return status_payload(Err(error)),
    };
    let reply =
        spawn_blocking_ipc(move || ipc::command(&socket, command, ipc::DEFAULT_TIMEOUT)).await;
    status_payload_remembering(&app.state(), reply)
}

#[tauri::command]
pub async fn get_status(app: tauri::AppHandle) -> StatusPayload {
    status_round_trip(app, ControlCommand::Status).await
}

#[tauri::command]
pub async fn pause(app: tauri::AppHandle) -> StatusPayload {
    status_round_trip(app, ControlCommand::Pause).await
}

#[tauri::command]
pub async fn resume(app: tauri::AppHandle) -> StatusPayload {
    status_round_trip(app, ControlCommand::Resume).await
}

#[tauri::command]
pub async fn sync_now(app: tauri::AppHandle) -> StatusPayload {
    status_round_trip(app, ControlCommand::Syncnow).await
}

/// Settings › *Sweep now* — the full-tree comparison, not an ordinary pass.
///
/// `Resync` and `Syncnow` both schedule a reconcile; the difference is what that reconcile IS. A
/// `Syncnow` under the default config is an incremental, event-driven pass, which is precisely the
/// thing `Compare everything, top to bottom` is offering an alternative to. `Resync` latches the
/// next pass to a full-tree walk (`ControlShared.pair.force_full_walk`, consumed once), so this is the
/// only command in the surface that answers the button.
///
/// An older daemon that predates the variant rejects it as an unknown command — the reply carries
/// the error and the button reports it, rather than silently doing an ordinary sync.
#[tauri::command]
pub async fn resync(app: tauri::AppHandle) -> StatusPayload {
    status_round_trip(app, ControlCommand::Resync).await
}

/// The shared body of `approve`/`deny`: a path-argument round trip folded into a `StatusPayload`.
/// Generic over the runtime so a `tauri::test` mock app can drive it headlessly (see tests).
async fn approval_round_trip<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    command: ControlCommand,
    target: String,
    literal_path: bool,
    direction: Option<DeleteDirection>,
) -> StatusPayload {
    let socket = match socket_path_for_ipc(&app.state()) {
        Ok(socket) => socket,
        Err(error) => return status_payload(Err(error)),
    };
    let reply = spawn_blocking_ipc(move || {
        ipc::command_with_argument(
            &socket,
            command,
            target,
            literal_path,
            direction,
            ipc::DEFAULT_TIMEOUT,
        )
    })
    .await;
    status_payload_remembering(&app.state(), reply)
}

/// `direction` is read by the daemon ONLY when nothing pending matches `target` — the Plan screen
/// approving its own plan's deletion before any pass has withheld it (#227). A pending item's own
/// direction wins over it, and an approval with neither authorises nothing.
#[tauri::command]
pub async fn approve(
    app: tauri::AppHandle,
    target: String,
    literal_path: bool,
    direction: Option<DeleteDirection>,
) -> StatusPayload {
    approval_round_trip(
        app,
        ControlCommand::Approve,
        target,
        literal_path,
        direction,
    )
    .await
}

#[tauri::command]
pub async fn deny(app: tauri::AppHandle, target: String, literal_path: bool) -> StatusPayload {
    approval_round_trip(app, ControlCommand::Deny, target, literal_path, None).await
}

/// `Keep it` — refuse a withheld deletion (#224). The daemon purges the baseline record for the
/// target and schedules the pass that puts the surviving copy back on the other side, so unlike
/// `deny` (which only revokes an approval) the row does not come back.
///
/// An older daemon that predates the variant rejects it as an unknown command, and the reply
/// carries that error rather than the screen recording a decision nothing acted on.
#[tauri::command]
pub async fn keep(app: tauri::AppHandle, target: String, literal_path: bool) -> StatusPayload {
    approval_round_trip(app, ControlCommand::Keep, target, literal_path, None).await
}

#[tauri::command]
pub async fn list_pending_deletions(app: tauri::AppHandle) -> Result<Vec<PendingDeletion>, String> {
    let socket = socket_path(&app.state())?;
    spawn_blocking_ipc(move || ipc::command(&socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT))
        .await
        .map(|response| response.pending_deletions)
        .map_err(|e| e.to_string())
}

/// A read of the GUI-owned config file, exposing both the raw TOML and the known settings.
#[derive(serde::Serialize)]
pub struct ConfigPayload {
    path: String,
    exists: bool,
    toml: String,
    local_root: Option<String>,
    remote_root: Option<String>,
    scan_interval_secs: Option<i64>,
    events_driven: Option<bool>,
    include: Vec<String>,
    exclude: Vec<String>,
    proton_cli: Option<String>,
    proton_timeout_secs: Option<i64>,
    proton_list_attempts: Option<i64>,
    /// The Advanced tab's remaining file keys (G23/#237). Each is the **file's literal value**, not
    /// the effective one: an absent key means the daemon default applies, and reporting the
    /// resolved value here would let the next save bake a default — or a value that came from a
    /// command-line flag — into the file as if the user had typed it.
    socket_path: Option<String>,
    log_level: Option<String>,
    conflict_suffix: Option<String>,
    delete_approval_remote: Option<bool>,
    delete_approval_local: Option<bool>,
    /// The same two booleans as the Settings → Deletions radio (C1, #174).
    ///
    /// Carried **alongside** them, not instead of them: the raw pair is what the Advanced view and
    /// the config text show, while the policy is what a radio group can bind to. Deriving it in the
    /// frontend instead would be a second place that has to know an absent key means `true` — and
    /// that defaulting is precisely what stops an empty config drawing as `Never ask` on a machine
    /// that is in fact asking about everything.
    deletion_policy: config_io::DeletionPolicy,
    /// What a local deletion does to the entity. A DIFFERENT setting from `deletion_policy`, which
    /// decides only whether a deletion waits for a person — this one decides what happens once one
    /// goes ahead, and is what the Deletions tab's second section is bound to.
    local_delete_mode: config_io::LocalDeleteMode,
}

#[tauri::command]
pub fn read_config(state: Paths) -> Result<ConfigPayload, String> {
    let path = state.lock().unwrap().config_path.clone();
    let exists = path.exists();
    let doc = config_io::ConfigDoc::load(&path).map_err(|e| e.to_string())?;
    Ok(ConfigPayload {
        path: path.display().to_string(),
        exists,
        toml: doc.to_toml_string(),
        local_root: doc.get_str("local_root"),
        remote_root: doc.get_str("remote_root"),
        scan_interval_secs: doc.get_int("scan_interval_secs"),
        events_driven: doc.get_bool("events_driven"),
        include: doc.get_string_array("include"),
        exclude: doc.get_string_array("exclude"),
        proton_cli: doc.get_str("proton_cli"),
        proton_timeout_secs: doc.get_int("proton_timeout_secs"),
        proton_list_attempts: doc.get_int("proton_list_attempts"),
        socket_path: doc.get_str("socket_path"),
        log_level: doc.get_str("log_level"),
        conflict_suffix: doc.get_str("conflict_suffix"),
        delete_approval_remote: doc.get_delete_approval("remote"),
        delete_approval_local: doc.get_delete_approval("local"),
        deletion_policy: doc.get_deletion_policy(),
        local_delete_mode: doc.get_local_delete_mode(),
    })
}

/// Partial config update: only `Some` fields are written; everything else (comments, daemon-only
/// keys) is preserved by the edit-in-place writer. Rejected if the daemon parser would refuse it.
#[derive(Default, serde::Deserialize)]
pub struct ConfigUpdate {
    local_root: Option<String>,
    remote_root: Option<String>,
    scan_interval_secs: Option<i64>,
    events_driven: Option<bool>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    proton_cli: Option<String>,
    proton_timeout_secs: Option<i64>,
    proton_list_attempts: Option<i64>,
    socket_path: Option<String>,
    log_level: Option<String>,
    conflict_suffix: Option<String>,
    delete_approval_remote: Option<bool>,
    delete_approval_local: Option<bool>,
    /// Applied AFTER the two raw booleans, so a screen that sends both cannot end up half-written:
    /// the policy always sets both directions, which is what makes a radio selection unambiguous.
    /// `set_deletion_policy` writes it back in whichever spelling the file already uses.
    deletion_policy: Option<config_io::DeletionPolicy>,
    /// Independent of `deletion_policy` above, and applied independently: a screen may send either
    /// without disturbing the other, which is what makes the tab's two sections two settings.
    local_delete_mode: Option<config_io::LocalDeleteMode>,
}

#[tauri::command]
pub fn write_config(state: Paths, update: ConfigUpdate) -> Result<(), String> {
    let path = state.lock().unwrap().config_path.clone();
    let mut doc = config_io::ConfigDoc::load(&path).map_err(|e| e.to_string())?;
    if let Some(v) = &update.local_root {
        doc.set_str("local_root", v);
    }
    if let Some(v) = &update.remote_root {
        doc.set_str("remote_root", v);
    }
    if let Some(v) = update.scan_interval_secs {
        doc.set_int("scan_interval_secs", v);
    }
    if let Some(v) = update.events_driven {
        doc.set_bool("events_driven", v);
    }
    if let Some(v) = &update.include {
        doc.set_string_array("include", v);
    }
    if let Some(v) = &update.exclude {
        doc.set_string_array("exclude", v);
    }
    if let Some(v) = &update.proton_cli {
        doc.set_str("proton_cli", v);
    }
    if let Some(v) = update.proton_timeout_secs {
        doc.set_int("proton_timeout_secs", v);
    }
    if let Some(v) = update.proton_list_attempts {
        doc.set_int("proton_list_attempts", v);
    }
    // An EMPTY string clears the key rather than writing `key = ""` — for all three of these an
    // empty value is either rejected outright (`conflict_suffix`) or means something the user did
    // not ask for, while an absent key is exactly "use the daemon default".
    for (key, value) in [
        ("socket_path", &update.socket_path),
        ("log_level", &update.log_level),
        ("conflict_suffix", &update.conflict_suffix),
    ] {
        match value.as_deref().map(str::trim) {
            Some("") => doc.remove(key),
            Some(v) => doc.set_str(key, v),
            None => {}
        }
    }
    if let Some(v) = update.delete_approval_remote {
        doc.set_delete_approval("remote", v);
    }
    if let Some(v) = update.delete_approval_local {
        doc.set_delete_approval("local", v);
    }
    if let Some(v) = update.deletion_policy {
        doc.set_deletion_policy(v);
    }
    if let Some(v) = update.local_delete_mode {
        doc.set_local_delete_mode(v);
    }
    doc.save(&path).map_err(|e| e.to_string())?;
    // Re-resolve in case local_root / db changed, but keep the daemon-reported live config — the
    // daemon is still running with it until restarted. (Saving still requires a daemon restart to
    // take effect — the frontend prompts for that.)
    //
    // A DIALABLE `socket_path` is the one field this must NOT move to the new value (#336). The
    // save's caller — `saveSettings` — restarts the daemon right after this returns, and that
    // restart has to dial the OLD socket: the daemon is still bound there until its own probe
    // confirms it has gone quiet. Overwriting it here points that dial at an address nothing is
    // listening on yet, so the probe reads `NotRunning`, the save's `only_if_running` gate leaves
    // the still-live old daemon untouched, and the app is left pointed at a socket that will never
    // be bound to it — the exact "reads as unreachable while a daemon is running" #336 describes,
    // arrived at one step earlier than the issue's own account. `restart_service` re-resolves it
    // once it no longer needs this value, gated on the restart's outcome actually confirming the
    // old address is settled.
    //
    // `is_ok()` gates it, not "always preserve": an `Err` (#74's fail-closed state) names no
    // address a restart could dial, so there is no live daemon this could lose track of, and
    // preserving it would strand the session on `Err` for ever — `restart_service`'s own
    // re-resolve is UNREACHABLE while `socket_path` is `Err`, since its first line's `?` returns
    // before reaching it. Letting the fresh value through here is what lets typing a working
    // `socket_path` over a failed-closed one actually take, the moment it is saved.
    let mut paths = state.lock().unwrap();
    let mut resolved = RuntimePaths::resolve();
    if paths.socket_path.is_ok() {
        resolved.socket_path = paths.socket_path.clone();
    }
    resolved.daemon_local_root = paths.daemon_local_root.take();
    resolved.daemon_remote_root = paths.daemon_remote_root.take();
    resolved.daemon_db_path = paths.daemon_db_path.take();
    *paths = resolved;
    Ok(())
}

/// Settings › Folders' `Choose…` — the native folder picker, behind the same facade as everything
/// else, so `api.js` stays the frontend's only backend surface and no capability JSON grants the
/// webview a file dialog of its own.
///
/// `start` seeds the dialog at the value currently in the field; a path that no longer exists is
/// passed anyway and the picker falls back to its own default rather than failing.
///
/// `Ok(None)` IS A DISMISSED PICKER AND `Err` IS A BROKEN ONE, and the two must not be the same
/// answer. The first version returned `Option<String>` and folded a join/panic error into `None`
/// with `unwrap_or(None)` — so a picker that could not open at all was indistinguishable from one
/// somebody closed, and the button reported nothing either way. That is the silence S6's review
/// found on `Sweep now`, one file over.
///
/// BLOCKING, ON A BLOCKING THREAD. The plugin marshals the dialog onto the GTK main thread itself
/// and `blocking_pick_folder` waits for it — waiting on the main loop from the main loop is the
/// WebKitGTK abort of #142/#143, and an async command's body runs on the async runtime, not the
/// main loop. `spawn_blocking` states that rather than relying on it.
#[tauri::command]
pub async fn choose_folder(
    app: tauri::AppHandle,
    start: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let mut builder = app.dialog().file();
        if let Some(dir) = start.filter(|s| !s.is_empty()) {
            builder = builder.set_directory(dir);
        }
        // `into_path()` FAILS INTO `Err`, NOT INTO `None`. It errors on a `FilePath` that is a URI
        // rather than a path — a portal backend, not the `gtk3` one this build links — and folding
        // that into `None` would have made an unusable selection read as a cancellation, which is
        // the contract this function's own doc comment had just finished promising it does not do.
        match builder.blocking_pick_folder() {
            None => Ok(None),
            Some(folder) => folder
                .into_path()
                .map(|path| Some(path.display().to_string()))
                .map_err(|e| format!("that folder is not a path this app can use: {e}")),
        }
    })
    .await
    .unwrap_or_else(|join_error| Err(format!("folder picker task failed: {join_error}")))
}

/// The dry-run plan plus the derived safety facts the Plan-preview screen needs.
#[derive(serde::Serialize)]
pub struct DryRunPayload {
    report: DryRunReport,
    /// Whether the typed-DELETE gate must arm (a `remote_delete`/`local_delete` is present).
    requires_delete_gate: bool,
    /// User-data files a destructive apply would remove (names the gate copy should show).
    files_at_risk: Vec<String>,
    /// The plan's token (#100), when the daemon computed it — what `apply_plan` authorises. `None`
    /// from the child `--dry-run` path: nothing holds that plan, so nothing can be applied by
    /// naming it, and the screen falls back to the approve-then-syncnow route.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    /// What a `local_delete` in this plan would DO — the daemon's `ReviewedPlan.local_disposal`.
    ///
    /// The Plan screen's typed-`DELETE` gate is the last sentence read before authorising a
    /// deletion, and `PLAN.destructiveLocal` asserts "nothing will bring it back" — false under
    /// `local_delete_mode = "trash"`. Carried on the payload rather than on `DryRunReport` because
    /// `sync.rs` is the pure planner and disposal is an execution-time decision; making the planner
    /// depend on `ipc` to say it would be the wrong layering for one string.
    ///
    /// `Permanent` from the **child** `--dry-run` path, which has no daemon to ask: unknown must
    /// over-warn, never under-warn.
    local_disposal: LocalDisposal,
}

/// How long a plan poll waits between `plan_result` requests. The same cadence
/// `proton-sync plan` polls at.
const PLAN_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);
/// Consecutive transport failures that end a plan wait — `watch_syncnow`'s bail-out, at the same
/// count the CLI uses, because it answers the same question: a daemon that has not replied five
/// polls running is not a pass that is still working.
const PLAN_POLL_ERROR_LIMIT: u32 = 5;

/// Async so the full-tree dry run — which is a remote walk that can take many seconds against a
/// large remote — never blocks the GTK main loop. Running it synchronously would stall the
/// webview's URI-scheme handler thread (the GTK main loop) until WebKit aborts the whole process;
/// here the blocking work runs on a runtime blocking thread instead. (See `restart_service` for the
/// same pattern.)
#[tauri::command]
pub async fn run_dry_run(state: Paths<'_>) -> Result<DryRunPayload, String> {
    let (
        socket,
        config_path,
        file_local,
        file_remote,
        file_db,
        daemon_local,
        daemon_remote,
        daemon_db,
    ) = {
        let paths = state.lock().unwrap();
        (
            paths.socket_path.clone(),
            paths.config_path.clone(),
            paths.local_root.clone(),
            paths.remote_root.clone(),
            paths.db_path.clone(),
            paths.daemon_local_root.clone(),
            paths.daemon_remote_root.clone(),
            paths.daemon_db_path.clone(),
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        run_dry_run_impl(
            socket,
            config_path,
            file_local,
            file_remote,
            file_db,
            daemon_local,
            daemon_remote,
            daemon_db,
        )
    })
    .await
    .map_err(|error| format!("dry-run task failed: {error}"))?
}

/// Whether the daemon's `plan` verb answers the question this screen is asking.
///
/// **Two conditions, and the second is the subtle one.** The daemon plans against *its own*
/// configured roots; the GUI's config file may already name a different pair that the running
/// daemon has not been restarted onto. Asking the daemon then would preview the wrong folders
/// silently — where the child, which is handed the file's pair, previews the right ones. So the
/// verb is used only when the file agrees with the daemon (or says nothing, in which case the
/// daemon's pair *is* the effective one).
fn daemon_plans_the_same_roots(
    file_local: Option<&std::path::Path>,
    file_remote: Option<&std::path::Path>,
    daemon_local: Option<&std::path::Path>,
    daemon_remote: Option<&std::path::Path>,
) -> bool {
    let (Some(daemon_local), Some(daemon_remote)) = (daemon_local, daemon_remote) else {
        // Nothing has answered the socket yet, so there is no daemon to ask.
        return false;
    };
    file_local.is_none_or(|local| local == daemon_local)
        && file_remote.is_none_or(|remote| remote == daemon_remote)
}

/// Why a plan could not be taken from the daemon.
enum DaemonPlanFailure {
    /// The daemon is not there (or is too old to know the verb). The caller falls back to the
    /// child `--dry-run`, which is what onboarding runs before any daemon exists.
    Unavailable,
    /// The daemon answered, and this is its answer. **Never fall back to the child on this**: a
    /// running daemon plus a second `proton-syncd --dry-run` is two `proton-drive` clients against
    /// the CLI's shared, not-concurrency-safe SQLite store (#23/#317), which is the hazard this
    /// whole verb exists to retire.
    Reported(String),
}

/// The blocking half of `run_dry_run`.
///
/// Prefers the daemon's `plan` verb (#100/#209/#317) and falls back to spawning
/// `proton-syncd --dry-run` only when there is no daemon to ask — which is exactly onboarding,
/// where the child path is the only one that can work. Kept as a free function so the mutex guard
/// from `run_dry_run` never crosses the `.await`.
#[allow(clippy::too_many_arguments)]
fn run_dry_run_impl(
    socket: Result<std::path::PathBuf, String>,
    config_path: std::path::PathBuf,
    file_local: Option<std::path::PathBuf>,
    file_remote: Option<std::path::PathBuf>,
    file_db: Option<std::path::PathBuf>,
    daemon_local: Option<std::path::PathBuf>,
    daemon_remote: Option<std::path::PathBuf>,
    daemon_db: Option<std::path::PathBuf>,
) -> Result<DryRunPayload, String> {
    let ask_the_daemon = daemon_plans_the_same_roots(
        file_local.as_deref(),
        file_remote.as_deref(),
        daemon_local.as_deref(),
        daemon_remote.as_deref(),
    );
    if let Ok(socket) = &socket {
        if ask_the_daemon {
            match plan_through_daemon(socket) {
                Ok(payload) => return Ok(payload),
                Err(DaemonPlanFailure::Reported(message)) => return Err(message),
                Err(DaemonPlanFailure::Unavailable) => {}
            }
        }
    }
    let mut command = Command::new("proton-syncd");
    command.arg("--dry-run");
    if config_path.exists() {
        command.arg("--config").arg(&config_path);
        // The config file wins wherever it speaks; a live daemon's reported values fill only the
        // gaps the file leaves (explicit CLI flags beat file values in the daemon's own
        // precedence, so only pass a flag when the file has no value of its own).
        if file_local.is_none() {
            if let Some(local) = &daemon_local {
                command.arg("--local-root").arg(local);
            }
        }
        if file_remote.is_none() {
            if let Some(remote) = &daemon_remote {
                command.arg("--remote-root").arg(remote);
            }
        }
        if file_db.is_none() {
            if let Some(db) = &daemon_db {
                command.arg("--db-path").arg(db);
            }
        }
    } else if let (Some(local), Some(remote)) = (&daemon_local, &daemon_remote) {
        // No GUI config file, but a live daemon told us its real roots — preview against those
        // instead of failing on a config path that was never written.
        command.arg("--local-root").arg(local);
        command.arg("--remote-root").arg(remote);
        if let Some(db) = &daemon_db {
            command.arg("--db-path").arg(db);
        }
    } else {
        return Err(format!(
            "no config file at {} and no running daemon to take the folder pair from — set the \
             folders in Settings first",
            config_path.display()
        ));
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to launch proton-syncd: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "proton-syncd --dry-run failed: {}",
            strip_ansi(String::from_utf8_lossy(&output.stderr).trim())
        ));
    }
    let report = plan::parse_dry_run(&String::from_utf8_lossy(&output.stdout))?;
    // No token: nothing holds this plan, so nothing can apply it by name. The screen falls back to
    // the approve-then-syncnow route, which is what it did before #100.
    // No daemon answered, so nothing said which mode a local delete would run under. `Permanent`
    // is the over-warning answer and the only safe default under a typed-`DELETE` gate.
    Ok(payload_from_report(report, None, LocalDisposal::Permanent))
}

fn payload_from_report(
    report: DryRunReport,
    token: Option<String>,
    local_disposal: LocalDisposal,
) -> DryRunPayload {
    let files_at_risk = plan::files_at_risk(&report)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    DryRunPayload {
        requires_delete_gate: plan::requires_delete_gate(&report),
        files_at_risk,
        token,
        local_disposal,
        report,
    }
}

/// Asks the daemon for a plan and waits for it: `plan` acks, then `plan_result` is polled until the
/// pass that answers *this* request has sealed.
///
/// **Not bounded by a wall clock.** A plan is an O(folders) remote walk, which on a large account
/// takes minutes; the child path it replaces was equally unbounded. What it *is* bounded by is the
/// daemon's own sealing discipline — every plan pass seals its request, success or failure — plus
/// the transport bail-out below, so the only way to wait for ever is a socket that keeps answering
/// while the daemon never runs the pass, which cannot happen.
///
/// **And deliberately no paused bail-out.** `proton-sync`'s `watch_syncnow` bails on
/// `paused && !syncing`, which is right *there* because a paused `syncnow` is never sealed —
/// `reconcile_if_needed` early-returns without bumping `reconcile_seq`, so only the client can end
/// that wait. A plan request is always sealed, so the same rule here is a *false positive*:
/// `ControlCommand::Plan` refuses a paused daemon at the ack (handled above), and a pause landing
/// after that ack does **not** cancel the booked pass —
/// `LoopCommand::PlanNow` goes straight to `Daemon::plan_now`, which has no pause check, so the
/// inert rehearsal still runs and still seals (daemon guard:
/// `a_plan_pass_booked_before_a_pause_still_runs_and_seals`). Between that ack and `plan_now`'s
/// `syncing.store(true)` a poller observes `paused && !syncing` for a pass that is about to run and
/// will answer, so such a bail-out would report "paused before the pass ran" for a plan that then
/// completes normally. An overall timeout is no better: a legitimate walk can run 30 minutes, so
/// any bound large enough to be safe is too large to help, and `PLAN_POLL_ERROR_LIMIT` below
/// already covers the real failure — a daemon that stopped answering at all.
fn plan_through_daemon(socket: &std::path::Path) -> Result<DryRunPayload, DaemonPlanFailure> {
    let ack = match ipc::command(socket, ControlCommand::Plan, ipc::DEFAULT_TIMEOUT) {
        Ok(ack) => ack,
        // The request did not complete. That is EITHER no daemon (onboarding, the case the child
        // exists for) OR a daemon too old to parse the verb — which drops the connection without
        // replying, so at this layer the two look identical. Ask `status`, which every daemon that
        // has ever existed answers, and let the answer decide.
        Err(error) => return Err(classify_unreachable_plan(socket, error)),
    };
    let target = match ack.plan {
        Some(PlanOutcome::Scheduled { plan_seq }) => plan_seq,
        Some(PlanOutcome::Paused) => {
            return Err(DaemonPlanFailure::Reported(
                "Syncing is paused, so nothing was worked out. Resume syncing and check again."
                    .to_owned(),
            ));
        }
        // A daemon ANSWERED and did not schedule a plan — an outcome this build does not know, or
        // none at all. Reported, never fallen back from: spawning `proton-syncd --dry-run` beside a
        // live daemon is two `proton-drive` clients against the CLI's shared, not-concurrency-safe
        // store (#23/#317), which is the hazard this verb exists to retire.
        other => {
            return Err(DaemonPlanFailure::Reported(
                unexpected_plan_ack(other.as_ref()).to_owned(),
            ));
        }
    };
    let mut consecutive_errors = 0u32;
    loop {
        std::thread::sleep(PLAN_POLL_INTERVAL);
        // Ask for the daemon's own maximum rather than its default 500: the screen draws a row per
        // action, and a window is a rendering the user decides from. Destructive rows are never
        // truncated whatever the cap (`StoredPlan::outcome`), so what a bigger cap buys is the
        // ordinary rows of an unusually large plan, not safety.
        let request = ControlRequest {
            limit: Some(PLAN_ACTIONS_MAX_LIMIT),
            ..ControlRequest::new(ControlCommand::PlanResult)
        };
        let response = match ipc::send_request(socket, &request, ipc::DEFAULT_TIMEOUT) {
            Ok(response) => {
                consecutive_errors = 0;
                response
            }
            Err(error) => {
                consecutive_errors += 1;
                if consecutive_errors >= PLAN_POLL_ERROR_LIMIT {
                    // The daemon stopped answering mid-plan. Reported, not fallen back from: a
                    // daemon that was alive a second ago may still be running its walk, and
                    // spawning a child beside it is #317.
                    return Err(DaemonPlanFailure::Reported(format!(
                        "lost contact with the daemon while it worked out the plan: {error}"
                    )));
                }
                continue;
            }
        };
        match response.plan {
            Some(PlanOutcome::Computed(plan)) if plan.plan_seq >= target => {
                let token = plan.token.clone();
                // `summary` describes the WHOLE plan; `actions` may be a window (see
                // `PLAN_ACTIONS_*`). The screen counts from the summary for exactly that reason,
                // so a windowed reply cannot make it claim a shorter plan than the one the token
                // applies.
                let local_disposal = plan.local_disposal;
                return Ok(payload_from_report(
                    DryRunReport {
                        summary: plan.summary,
                        plan: plan.actions,
                        cannot_sync: plan.cannot_sync,
                    },
                    Some(token),
                    local_disposal,
                ));
            }
            Some(PlanOutcome::Failed { plan_seq, error }) if plan_seq >= target => {
                return Err(DaemonPlanFailure::Reported(error));
            }
            // Still computing, or an older answer than the one asked for.
            _ => {}
        }
    }
}

/// Which failure a `plan` request that never completed really was.
///
/// `Unavailable` — and therefore the child `--dry-run` — **only** when nothing answers the socket at
/// all. A daemon that answers `status` but not `plan` is one this app is newer than, and the honest
/// answer there is to say so: falling back would put a second `proton-drive` client beside a live
/// daemon (#23/#317), which is exactly what this verb removed.
///
/// So the follow-up `status` is read by **variant, not by `is_err()`**: [`ipc::IpcError::Protocol`]
/// means the daemon *replied* and the reply could not be decoded, which is evidence a daemon is
/// there — the opposite of what a fallback needs. Only [`ipc::IpcError::Unreachable`] (no socket,
/// refused, closed early, timed out) is evidence of absence. Guard:
/// `an_undecodable_status_reply_is_a_live_daemon_not_an_absent_one`.
fn classify_unreachable_plan(socket: &std::path::Path, error: ipc::IpcError) -> DaemonPlanFailure {
    match daemon_presence(socket) {
        DaemonPresence::Answering => DaemonPlanFailure::Reported(format!(
            "the sync daemon is running but could not work out a plan ({error}). It is \
             probably older than this app; restart it from Settings and try again."
        )),
        DaemonPresence::Undecodable(reply) => DaemonPlanFailure::Reported(format!(
            "the sync daemon answered with something this app could not read ({reply}). \
             Restart it from Settings and try again."
        )),
        DaemonPresence::Absent => DaemonPlanFailure::Unavailable,
    }
}

/// Whether a daemon is there at all — asked of `status`, which every daemon that has ever existed
/// answers, and therefore the one question worth asking when a *newer* verb did not complete.
///
/// One definition, two callers ([`classify_unreachable_plan`] and [`probe_folder`]'s remote side),
/// because both are deciding the same thing: whether it is safe to spawn a `proton-drive` child of
/// our own. Each still writes its **own** sentences — the advice differs, and one sentence with the
/// feature interpolated would fit neither.
enum DaemonPresence {
    /// `status` was answered and decoded. A daemon is running.
    Answering,
    /// It replied and the reply could not be decoded — which is still a daemon.
    Undecodable(ipc::IpcError),
    /// Nothing answered the socket: no socket, refused, closed early, timed out.
    Absent,
}

fn daemon_presence(socket: &std::path::Path) -> DaemonPresence {
    match ipc::command(socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT) {
        Ok(_) => DaemonPresence::Answering,
        Err(reply) if !status_error_proves_no_daemon(&reply) => DaemonPresence::Undecodable(reply),
        Err(_) => DaemonPresence::Absent,
    }
}

/// The one question [`classify_unreachable_plan`] asks of a failed `status`, split out so it can be
/// tested without a socket: **is this evidence there is no daemon?** — asked in order to decide
/// whether it is safe to spawn a `proton-drive` child of our own (#23/#317).
///
/// A **policy over [`probe_from_error`]**, not a second reading of `IpcError`. Two exhaustive
/// matches on one enum agreeing on two variants and differing on the third is drift waiting to
/// happen; this way the classification has one definition and only the *answer for `Unknown`*
/// differs, which is the whole of the disagreement and is visible at the arm that makes it.
///
/// **The `Unknown` arm is a known-wrong answer, deliberately deferred — not a defensible one.**
/// Since #335 that state means "the exchange did not finish and this says nothing about whether a
/// daemon exists", so answering "yes, there is no daemon" is asserting exactly what was not
/// observed, and the cost of being wrong is a second CLI client beside a live one — the hazard the
/// plan verb exists to retire. It is left as it was because changing it changes which sentence a
/// timed-out `plan` shows and when the child `--dry-run` is taken, which is #317's decision to
/// make with its own copy and its own tests. It is not this issue's to slip in.
fn status_error_proves_no_daemon(error: &ipc::IpcError) -> bool {
    match probe_from_error(error) {
        DaemonProbe::NotRunning => true,
        // It answered. Undecodably, but it answered.
        DaemonProbe::Running => false,
        DaemonProbe::Unknown => true,
    }
}

/// What to tell the user when the daemon answered `plan` with something other than an ack.
///
/// One sentence per outcome rather than one sentence with the outcome interpolated, because the
/// **advice** differs: "restart it, it is older than this app" is right for a reply carrying no plan
/// field or one this build cannot read, and wrong for a daemon that tried and failed. Exhaustive by
/// variant, no `_`: a new outcome has to be given its own sentence rather than inherit an arm's
/// guess. Never used to *decide* anything — that is what the typed outcome is for (#103).
fn unexpected_plan_ack(outcome: Option<&PlanOutcome>) -> &'static str {
    match outcome {
        // Both mean this app is newer than the daemon: the field is missing, or its value comes
        // from a vocabulary this build predates.
        None | Some(PlanOutcome::Unknown) => {
            "the sync daemon did not answer with a plan. It is probably older than this app — \
             restart it from Settings and try again."
        }
        Some(PlanOutcome::Failed { .. }) => {
            "the sync daemon could not work out a plan. Check again in a moment."
        }
        // Both answer a different question than the one just asked, so either here means the
        // daemon did not schedule the pass.
        Some(PlanOutcome::Absent | PlanOutcome::Computing { .. }) => {
            "the sync daemon did not start working out a plan. Check again in a moment."
        }
        // Handled by the arms above; named so the match stays exhaustive by variant.
        Some(PlanOutcome::Scheduled { .. } | PlanOutcome::Computed(_) | PlanOutcome::Paused) => {
            "the sync daemon answered unexpectedly. Check again in a moment."
        }
    }
}

/// `apply <token>` (#100), and with `skip_destructive` the Plan screen's
/// `Run it without the deletion` (#192).
///
/// Returns the typed [`ApplyOutcome`] rather than a `StatusPayload`, because the caller's next act
/// depends on *which* answer it got and a client must never tell those apart by matching a sentence
/// (#103). `Err` only when the daemon could not be reached at all.
///
/// The wait below needs no paused bail-out either, for a different reason than the plan loop's: a
/// pause *does* cancel a booked apply, and cancelling it is itself a seal —
/// `daemon::discard_queued_apply_for_pause` takes the queued request and seals it `Failed`, which
/// this loop exits on. `schedule_apply` always sends a `LoopCommand::SyncNow`, so the pass that
/// reaches that discard is always scheduled (daemon guard:
/// `a_pause_cancels_an_apply_it_overtook_rather_than_latching_it`).
#[tauri::command]
pub async fn apply_plan(
    app: tauri::AppHandle,
    token: String,
    skip_destructive: bool,
) -> Result<ApplyOutcome, String> {
    let socket = socket_path(&app.state())?;
    let ack = tauri::async_runtime::spawn_blocking(move || {
        let ack = ipc::apply_plan(&socket, token, skip_destructive, ipc::DEFAULT_TIMEOUT)?;
        let target = match ack.apply {
            Some(ApplyOutcome::Scheduled { apply_seq }) => apply_seq,
            // Refused, or a daemon too old to know the verb — either way there is nothing to wait
            // for, and the outcome says which it was.
            other => return Ok(other.unwrap_or(ApplyOutcome::Unknown)),
        };
        let mut consecutive_errors = 0u32;
        loop {
            std::thread::sleep(PLAN_POLL_INTERVAL);
            // The smallest window the daemon will build, because this loop reads `apply` and
            // nothing else — unlike the CLI's wait, which renders the fresh plan out of this same
            // reply when an apply diverges. Without it every 300ms poll ships up to
            // `PLAN_ACTIONS_DEFAULT_LIMIT` rows nobody looks at, for as long as the apply runs.
            // `1`, not `0`: the daemon clamps the limit to at least one row, and a literal that
            // does not survive the clamp reads as a stronger claim than it is. Destructive rows are
            // never truncated whatever the limit, so this bounds the ordinary case only (#321).
            let request = ControlRequest {
                limit: Some(1),
                ..ControlRequest::new(ControlCommand::PlanResult)
            };
            let response = match ipc::send_request(&socket, &request, ipc::DEFAULT_TIMEOUT) {
                Ok(response) => {
                    consecutive_errors = 0;
                    response
                }
                Err(error) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= PLAN_POLL_ERROR_LIMIT {
                        return Err(error);
                    }
                    continue;
                }
            };
            match response.apply {
                Some(
                    outcome @ (ApplyOutcome::Applied { apply_seq, .. }
                    | ApplyOutcome::Diverged { apply_seq }
                    | ApplyOutcome::Failed { apply_seq, .. }),
                ) if apply_seq >= target => return Ok(outcome),
                _ => {}
            }
        }
    })
    .await
    .map_err(|join_error| format!("apply task failed: {join_error}"))?;
    ack.map_err(|error| error.to_string())
}

// `list_remote` WAS HERE, AND IT IS GONE RATHER THAN REWRITTEN (#311).
//
// It shelled `proton-drive filesystem list --json` from the GUI process and handed the raw stdout
// to the webview. That put a second `proton-drive` client beside the daemon's — the CLI's shared
// SQLite store is not concurrency-safe (#23), which is the whole reason `proton::CliGate` and the
// user-global lock in `paths.rs` exist — and it did so for **no caller**: nothing in `gui/src/js`
// ever invoked it, and the one surface that would (`Browse Proton Drive…` on the folders card)
// is unbuilt in Phase 1 and recorded as such in DEVIATIONS §79e.
//
// So this is a deletion and not a port. The daemon already answers the same question over the
// socket (`ControlCommand::List`, #99), typed and gated; writing a socket-backed `list_remote`
// with no caller would move "a verb nothing calls" one layer up instead of removing it. When a
// picker is built, it calls `list` through `gui_core::ipc` like every other verb — the child
// process is not the shape to reach for, on this or any other remote question (`run_dry_run`'s
// `classify_unreachable_plan` is the precedent: the child only when *nothing* answers the socket).

/// Async so a full local-tree conflict scan (the sidecar walk) never blocks the GTK main loop on a
/// large folder.
#[tauri::command]
pub async fn scan_conflicts(state: Paths<'_>) -> Result<Vec<Conflict>, String> {
    // Root and naming come out of the SAME guard: the scanner must look for the suffix this config
    // makes the daemon write, not the compiled-in default (`conflict_suffix`, G23/#237).
    let (local_root, naming) = {
        let paths = state.lock().unwrap();
        let local_root = paths
            .effective_local_root()
            .ok_or_else(|| "local_root is not configured".to_string())?;
        (local_root, paths.conflict_naming.clone())
    };
    tauri::async_runtime::spawn_blocking(move || {
        conflicts::scan_conflicts(&local_root, &naming).map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| format!("conflict-scan task failed: {error}"))?
}

#[tauri::command]
pub fn resolve_conflict(
    state: Paths,
    conflict: Conflict,
    choice: Resolution,
) -> Result<(), String> {
    let local_root = state
        .lock()
        .unwrap()
        .effective_local_root()
        .ok_or_else(|| "local_root is not configured".to_string())?;
    conflicts::apply_resolution(&local_root, &conflict, choice).map_err(|e| e.to_string())
}

/// Read both sides of a conflict (the local file + its sidecar) for the compare
/// view. Path-safe and size-bounded — see `gui_core::conflicts::read_conflict_pair`.
#[tauri::command]
pub fn read_conflict_pair(
    state: Paths,
    conflict: Conflict,
) -> Result<conflicts::ConflictPair, String> {
    let local_root = state
        .lock()
        .unwrap()
        .effective_local_root()
        .ok_or_else(|| "local_root is not configured".to_string())?;
    conflicts::read_conflict_pair(&local_root, &conflict)
}

/// When this engine last moved a path's bytes, and which way (#233).
///
/// **A fact about a transfer this daemon performed, never a local file time.** `direction` is
/// `up` (this computer -> Proton Drive) or `down`, taken from the action's own
/// `SyncAction::transfer_direction`, and only `up` means Proton Drive *received* anything: a `down`
/// row says when this computer received bytes, and a conflict-sidecar fetch is a `down` row filed
/// under the file's own path. A renderer that labels a `down` time as a remote event is telling the
/// user the opposite of what happened, on the one screen whose job is saying where a file stands.
#[derive(serde::Serialize)]
pub struct LastTransfer {
    epoch_secs: u64,
    direction: String,
}

/// Per-path index status for the file-manager emblems (S10). Built from the engine's `FileRecord`
/// (which is not itself `Serialize`). `sync_status` is one of `synced` / `modified` / `conflict`;
/// "syncing" / "paused" / "excluded" are derived by the frontend from live state + globs.
#[derive(serde::Serialize)]
pub struct EmblemStatus {
    tracked: bool,
    sync_status: Option<String>,
    entity_kind: Option<String>,
    file_size: Option<u64>,
    /// The LOCAL modification time, from the record. Not a remote event — see `last_transfer`.
    mtime: Option<i64>,
    proton_id: Option<String>,
    /// The last transfer this engine performed for the path (#233), or `None` when it has no
    /// record of one. Four honest causes: nothing ever transferred; the last transfer aged out of
    /// `HistoryRetention` (20k rows / 90 days); the file was adopted rather than transferred
    /// (`AutoLink` moves no bytes); or it has been moved since. A consumer omits the clause —
    /// there is deliberately no fallback to `mtime`, which would render a local time as a remote
    /// one.
    last_transfer: Option<LastTransfer>,
}

impl EmblemStatus {
    fn untracked() -> Self {
        Self {
            tracked: false,
            sync_status: None,
            entity_kind: None,
            file_size: None,
            mtime: None,
            proton_id: None,
            last_transfer: None,
        }
    }

    /// A record plus the history log's answer for the same path. Not a `From` impl any more,
    /// because the second half is a query: an `EmblemStatus` built from a record alone would
    /// silently carry `last_transfer: None` for every file, which is indistinguishable from the
    /// real answer and so could never be noticed.
    fn new(
        record: gui_core::wire::FileRecord,
        last_transfer: Option<gui_core::wire::FileEvent>,
    ) -> Self {
        Self {
            tracked: true,
            sync_status: Some(record.sync_status.as_str().to_string()),
            entity_kind: Some(record.entity_kind.as_str().to_string()),
            file_size: Some(record.file_size),
            mtime: Some(record.mtime),
            proton_id: record.proton_id,
            last_transfer: last_transfer.and_then(|event| {
                // `None` here is unreachable — `index::last_transfer` returns only rows whose
                // action HAS a direction — and is mapped rather than defaulted so it can never
                // invent one.
                event
                    .action
                    .transfer_direction()
                    .map(|direction| LastTransfer {
                        epoch_secs: event.epoch_secs,
                        direction: match direction {
                            gui_core::wire::TransferDirection::Up => "up".to_string(),
                            gui_core::wire::TransferDirection::Down => "down".to_string(),
                        },
                    })
            }),
        }
    }
}

#[tauri::command]
pub fn path_sync_status(state: Paths, relative_path: String) -> Result<EmblemStatus, String> {
    let db_path = state
        .lock()
        .unwrap()
        .effective_db_path()
        .ok_or_else(|| "no index database configured or reported by the daemon".to_string())?;
    let connection = index_read::open_readonly(&db_path, index_read::DEFAULT_BUSY_TIMEOUT)?;
    let path = std::path::Path::new(&relative_path);
    let Some(record) = index_read::record_for_path(&connection, path)? else {
        return Ok(EmblemStatus::untracked());
    };
    let last_transfer = index_read::last_transfer(&connection, path)?;
    Ok(EmblemStatus::new(record, last_transfer))
}

/// One file the search found: the path it is stored under, and that record's status.
#[derive(serde::Serialize)]
pub struct FileMatch {
    /// Relative to the local root, exactly as the index stores it.
    path: String,
    status: EmblemStatus,
}

/// What the search found. `total` counts every match; `matches` is capped at the asked-for limit,
/// so a screen can say what it is not showing rather than implying the list is all of it.
#[derive(serde::Serialize)]
pub struct FileSearch {
    matches: Vec<FileMatch>,
    total: usize,
    query: String,
}

/// Default cap. Above ~50 rows the answer is "narrow the query", not a longer list.
const SEARCH_LIMIT: usize = 50;

/// Find files by name or by path (S5's lookup field).
///
/// WHY THIS EXISTS ALONGSIDE `path_sync_status`: that one opens the index AT the path it is given,
/// so `spec.md` answers "not tracked" for a file that is really at `docs/spec.md` — the gap the
/// lookup field shipped with (G21). This one matches a bare name, a trailing path, or any fragment.
///
/// The query is taken as a user types it: a leading `~` expands, and an absolute path under the
/// sync folder is reduced to the relative one the index stores. Neither is a guess — both are
/// exactly what someone pastes out of a file manager.
///
/// Async + `spawn_blocking`: a name search is a full table scan behind a 3s busy timeout, and the
/// module header's rule is what keeps WebKitGTK from aborting on a stalled main loop.
#[tauri::command]
pub async fn search_files(
    app: tauri::AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<FileSearch, String> {
    let (db_path, local_root) = {
        let paths = app.state::<Mutex<RuntimePaths>>();
        let paths = paths.lock().unwrap();
        (paths.effective_db_path(), paths.effective_local_root())
    };
    let db_path = db_path
        .ok_or_else(|| "no index database configured or reported by the daemon".to_string())?;
    let limit = limit.unwrap_or(SEARCH_LIMIT).clamp(1, 500);
    tauri::async_runtime::spawn_blocking(move || {
        let query = relative_query(&query, local_root.as_deref());
        let connection = index_read::open_readonly(&db_path, index_read::DEFAULT_BUSY_TIMEOUT)?;
        let (found, total) = index_read::search_records(&connection, &query, limit)?;
        // One indexed point query per row, capped by `limit` (<= 500) and already off the UI
        // thread. The lookup card is fed by THIS command, not `path_sync_status` (G21), so a
        // `last_transfer` only the point-query command carried would never reach the screen.
        let matches = found
            .into_iter()
            .map(|m| {
                let last_transfer = index_read::last_transfer(&connection, &m.path)?;
                Ok(FileMatch {
                    path: m.path.to_string_lossy().into_owned(),
                    status: EmblemStatus::new(m.record, last_transfer),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FileSearch {
            matches,
            total,
            query,
        })
    })
    .await
    .map_err(|error| format!("search task failed: {error}"))?
}

/// A typed query as the index would store it: `~` expanded, the sync folder's own prefix removed
/// when the path is under it, and never a leading separator.
///
/// A path that is NOT under the sync folder keeps its components and loses only that separator, so
/// `/etc/hosts` is asked for as `etc/hosts` — it cannot match as a whole path (nothing in the index
/// is stored that way) but it still matches as a fragment, which is a better answer than none. The
/// separator goes for every query alike because the index stores relative paths and nothing in it
/// begins with one.
fn relative_query(query: &str, local_root: Option<&std::path::Path>) -> String {
    let query = query.trim();
    let expanded = match query.strip_prefix('~') {
        // `~user` is somebody else's home and is not expanded here either — same rule as the
        // daemon's `expand_tilde`.
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var_os("HOME") {
            Some(home) => format!("{}{rest}", home.to_string_lossy()),
            None => query.to_string(),
        },
        _ => query.to_string(),
    };
    // `Path::strip_prefix`, NOT the string one: a textual prefix is not a path prefix, so
    // `/home/me/ProtonDrive-Other/x.md` under the root `/home/me/ProtonDrive` came out as the
    // fragment `-Other/x.md` — a path from outside the sync folder, mangled into one that could
    // match inside it. (Copilot, PR #266.)
    let stripped = local_root
        .and_then(|root| std::path::Path::new(&expanded).strip_prefix(root).ok())
        .map(|rest| rest.to_string_lossy().into_owned())
        .unwrap_or(expanded);
    stripped.trim_start_matches('/').to_string()
}

/// Start the sync daemon: prefer the user's systemd unit; when there is none, fall back to
/// spawning `proton-syncd` directly against the GUI config file. Shared by the window button and
/// the tray menu item so both paths behave identically.
pub(crate) fn start_service_impl(config_path: &std::path::Path) -> Result<String, String> {
    let systemctl = Command::new("systemctl")
        .args(["--user", "start", "proton-syncd"])
        .output();
    if let Ok(output) = &systemctl {
        if output.status.success() {
            return Ok("asked systemd to start proton-syncd".to_string());
        }
    }
    // No unit (or no systemd): launch the daemon directly. Only sensible with a config file —
    // without one the daemon has no folder pair and exits immediately.
    if !config_path.exists() {
        let detail = match &systemctl {
            Ok(output) => strip_ansi(String::from_utf8_lossy(&output.stderr).trim()),
            Err(e) => e.to_string(),
        };
        return Err(format!(
            "couldn't start via systemd ({detail}) and there is no config file at {} to launch \
             proton-syncd with — set the folders in Settings first",
            config_path.display()
        ));
    }
    Command::new("proton-syncd")
        .arg("--config")
        .arg(config_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch proton-syncd: {e}"))?;
    Ok("no systemd unit found — started proton-syncd directly".to_string())
}

/// Async so the `systemctl --user start` round trip (it blocks until the unit reports started)
/// never runs on the GTK main loop. Mirrors `restart_service`.
#[tauri::command]
pub async fn start_service(state: Paths<'_>) -> Result<String, String> {
    let config_path = state.lock().unwrap().config_path.clone();
    tauri::async_runtime::spawn_blocking(move || start_service_impl(&config_path))
        .await
        .map_err(|error| format!("start-service task failed: {error}"))?
}

/// What a restart request did — **the ending, typed, on the `Ok` payload** (#320/#335).
///
/// `restart_service_impl` has five distinguishable endings and this used to type two of them
/// (`{ restarted, detail }`), so three collapsed into one `Err(String)` and the screen drew the
/// same sentence over all of them: *"It is still running the old settings."* That is true of
/// exactly one — [`Self::NeverStopped`] — and the **opposite** of the truth for
/// [`Self::NotStarted`], where the stop succeeded and nothing is running at all. For a sync tool
/// that is false in the dangerous direction.
///
/// **On the `Ok` payload rather than the `Err`, deliberately.** A Tauri command's `Err` crosses the
/// bridge as a bare string, so an ending carried there is prose the webview would have to match on
/// — the bug #103 removes everywhere on the daemon's own wire. This repo already settled that
/// direction once: the delete-approval work found that Tauri commands resolve rather than reject
/// against a dead socket, so a caller must *validate the response* rather than catch the rejection.
/// The residual `Err` is now infrastructure only (an unresolvable socket path, a join failure) —
/// never an ending.
///
/// Internally tagged with `ending` and terminated by [`Self::Unknown`], the pattern
/// `ipc::ListingOutcome`/`PlanOutcome`/`ApplyOutcome` follow, so a client one version behind
/// degrades to "something happened that this build cannot name" instead of failing the parse. The
/// webview is the only consumer, so its fall-through arm is where that degradation actually
/// happens; `Deserialize` is derived so the rule is testable here too.
///
/// The variants are named for **what is running now**, not for the sequence that got there: two
/// sequences reach [`Self::NotStarted`] (a stop that worked followed by a start that did not, and a
/// start that failed against a daemon that was already down) and they are one ending, because the
/// fact they leave behind — nothing is running, and the config on disk is ahead of it — is one fact
/// with one sentence and one way out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "ending", rename_all = "snake_case")]
pub enum RestartOutcome {
    /// A start was requested for the settings that are on disk, and it **succeeded**: the service
    /// was stopped (or already was) and started again. The only ending that may report a success.
    ///
    /// **What that proves, exactly**, because the sentence the screen draws is the strongest one
    /// here: `systemctl --user start` returns once the unit reports started, and the direct-spawn
    /// fallback returns once `proton-syncd` was spawned. Neither re-probes, so a process that dies
    /// a moment later — a bad config the daemon's own parser refuses at startup, a missing
    /// dependency — is inside this ending. Not re-probed on purpose: a fresh daemon has not bound
    /// its socket yet, so an immediate probe would report absence for a healthy start and turn the
    /// one reliable ending into a race. The screen is not the only account either — the main screen
    /// and the tray poll the socket every two seconds and draw a dead daemon as unreachable within
    /// one tick.
    Restarted { detail: String },
    /// It was not running and **nothing was started** — the #320 decision: a save is not a request
    /// to begin syncing. Reachable only with `only_if_running`.
    NotRunning,
    /// **Nothing is running**: the start failed. The config on disk is ahead of a service that is
    /// not up, and the way out is to fix the reason and start it.
    NotStarted { reason: String },
    /// It never stopped: the shutdown was asked for and the socket **kept answering** past
    /// [`STOP_TIMEOUT`], so the **old** process is still up on the **old** settings. The one ending
    /// the pre-#335 sentence was true of.
    ///
    /// **Only reachable from an observation of life.** A drain that timed out having never seen the
    /// socket answer is [`Self::Undetermined`], not this: "it is still running the old settings" is
    /// a positive claim about a live process, and making it from evidence of nothing is the shape
    /// #335 exists to remove. See [`stop_timeout_outcome`].
    NeverStopped { reason: String },
    /// Whether it was running could not be determined, so **nothing was done**. Not folded into
    /// [`Self::NotRunning`]: that one asserts an absence, and asserting one we did not observe is
    /// how a live daemon on stale settings gets drawn as "it will use these when it starts".
    Undetermined { reason: String },
    /// An ending added by a newer build. Never constructed here; it exists so a parse degrades.
    #[serde(other)]
    Unknown,
}

/// Whether the daemon was there when we asked — three states, and **`Unknown` is evidence of
/// neither** (#335).
///
/// The probe used to be `ipc::command(…Status…).is_ok()`, which reads two different things as
/// absence: [`ipc::IpcError::Protocol`] means the daemon *replied* and the reply would not decode,
/// which is positive evidence of life, and `Unreachable` folded a timeout in with a missing socket.
/// Either one drew "the sync service is not running" over a live daemon left on the old settings,
/// with no latch and no retry.
///
/// The shape is the engine's own: `daemon::probe_daemon_lock` answers a three-state `GlobalLockProbe`
/// whose `Unknown` is evidence of neither, for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonProbe {
    /// It answered — decodably or not. Something is bound to that socket and talking.
    Running,
    /// Nothing is listening on the socket. Authoritative absence.
    NotRunning,
    /// **Evidence of neither.** The exchange did not finish and the failure says nothing about
    /// whether a daemon exists: a connect that succeeded and then timed out or closed early, and
    /// also a connect that failed for a reason about the *path* rather than about a listener —
    /// `NotADirectory` (a socket path under a regular file), `InvalidInput` (longer than `sun_len`),
    /// `PermissionDenied`. None of those is absence: the daemon may be up and listening on the
    /// socket it was actually given, which is #336's whole subject.
    Unknown,
}

/// The classification, split from the round trip so it is testable with no socket at all.
///
/// **The one classifier of a `Status` error in this file.** [`status_error_proves_no_daemon`] is a
/// *policy* over this answer rather than a second reading of the same three variants — two
/// exhaustive matches on one enum are how they drift apart.
fn probe_from(reply: &Result<ControlResponse, ipc::IpcError>) -> DaemonProbe {
    match reply {
        Ok(_) => DaemonProbe::Running,
        Err(error) => probe_from_error(error),
    }
}

fn probe_from_error(error: &ipc::IpcError) -> DaemonProbe {
    match error {
        // It answered. Undecodably, but it answered — and `Shutdown` is a request we send, not a
        // reply we read, so a daemon whose *replies* this build cannot parse still stops on ask.
        ipc::IpcError::Protocol(_) => DaemonProbe::Running,
        ipc::IpcError::NotListening(_) => DaemonProbe::NotRunning,
        ipc::IpcError::Unreachable(_) => DaemonProbe::Unknown,
    }
}

/// What a restart request does about the probe it got — **the whole decision, with no I/O in it**.
///
/// A pure predicate beside the branch, because the branch decides whether a subprocess runs and a
/// test of it may not be a test that runs one: on any machine this project is developed on
/// `systemctl --user start proton-syncd` is a live unit, so a poison check that reached
/// [`start_service_impl`] would start the developer's daemon as a side effect. See
/// `docs/agent-notes/gui-tests-that-shell-systemctl.md`.
///
/// The asymmetry between the two callers is the whole of `only_if_running`:
///
/// * A **save** (`only_if_running`) declines both "it was not running" and "we could not tell".
///   Starting a daemon nobody asked for would make a save mean "and begin syncing", and *asserting*
///   an absence we did not observe is the misreport #335 was filed for.
/// * The **retry** (`Restart it now`) is an explicit request, so an unknown probe is attempted:
///   [`RestartPlan::StopThenStart`] starts only after the stop has been *confirmed* by an
///   authoritatively absent socket, so an attempt against a daemon that was not there cannot report
///   a success that did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestartPlan {
    /// Ask it to exit, wait for the socket to go quiet, then start it.
    StopThenStart,
    /// Nothing is there: start it without a shutdown.
    StartOnly,
    /// Leave it alone and answer [`RestartOutcome::NotRunning`].
    LeaveStopped,
    /// Leave it alone and answer [`RestartOutcome::Undetermined`].
    LeaveUndetermined,
}

fn restart_plan(only_if_running: bool, probe: DaemonProbe) -> RestartPlan {
    match (probe, only_if_running) {
        (DaemonProbe::Running, _) => RestartPlan::StopThenStart,
        (DaemonProbe::NotRunning, true) => RestartPlan::LeaveStopped,
        (DaemonProbe::NotRunning, false) => RestartPlan::StartOnly,
        (DaemonProbe::Unknown, true) => RestartPlan::LeaveUndetermined,
        // An explicit retry against a socket that answers ambiguously: treat it as running, because
        // the start below happens only after a confirmed absence either way.
        (DaemonProbe::Unknown, false) => RestartPlan::StopThenStart,
    }
}

/// How long a confirmed shutdown may take before the daemon is reported as never having stopped.
const STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// What a drain that ran out of time may claim, from the last thing it actually observed.
///
/// **A timeout is not by itself evidence that anything is running.** The drain breaks only on
/// [`DaemonProbe::NotRunning`], so every other answer runs it to [`STOP_TIMEOUT`] — including
/// [`DaemonProbe::Unknown`], which is what a socket path under a regular file (`NotADirectory`),
/// one longer than `sun_len` (`InvalidInput`) or an unreadable one (`PermissionDenied`) produces
/// on **every** iteration. Reporting [`RestartOutcome::NeverStopped`] there would tell the user
/// *"It is still running the old settings"* — a positive claim about a live process, made from an
/// observation of nothing, which is the exact shape #335 §2 exists to remove. It is reachable from
/// the shipped app, because `socket_path` is a config key.
///
/// So the ending is the last observation, not the timeout: only a socket that was still **answering**
/// earns the claim that the old process is still up.
///
/// Pure, and separate from the loop, for the [`restart_plan`] reason — the loop's next statement is
/// a `systemctl` spawn, so a poison check of this decision may not be a test that runs one.
fn stop_timeout_outcome(last: DaemonProbe) -> RestartOutcome {
    match last {
        DaemonProbe::Running => RestartOutcome::NeverStopped {
            reason: format!(
                "the sync service was asked to stop and was still answering {}s later — restart it \
                 manually with: systemctl --user restart proton-syncd",
                STOP_TIMEOUT.as_secs()
            ),
        },
        // Never observed at all. Says nothing about what is running, and must not.
        DaemonProbe::Unknown => RestartOutcome::Undetermined {
            reason: format!(
                "the control socket could not be reached for {}s, so the app could not tell \
                 whether the sync service stopped or was ever running",
                STOP_TIMEOUT.as_secs()
            ),
        },
        // Unreachable in practice: the drain breaks on this rather than timing out. Answered anyway,
        // and answered as the *fact* — a socket that is authoritatively absent is a service that is
        // not running, whatever brought us here.
        DaemonProbe::NotRunning => RestartOutcome::NotRunning,
    }
}

/// Restart the sync daemon so a saved config change takes effect. Works no matter how the daemon
/// was launched: ask it to exit gracefully over IPC (its `shutdown` control command), wait for
/// the control socket to go quiet, then start it again through the shared start logic (systemd
/// unit first, direct spawn against the GUI config as fallback). The unit ships
/// `Restart=on-failure`, so systemd does not race us by respawning the clean exit itself.
///
/// `only_if_running` is the save path's (#320). A settings save restarts the daemon so the Plan
/// screen can never preview one folder pair while `Run` executes another — but a daemon that is
/// NOT running has nothing to interrupt and nothing stale to correct: it reads the file on its next
/// start, whenever the user asks for one. Starting it here would make a save mean "and start
/// syncing", which is a decision the Settings screen was never asked to take. The probe is made
/// here rather than in the webview because it has to be the same instant as the shutdown: the GUI's
/// own status is up to a poll old, and a daemon that came up in that window would be left running
/// the old settings — exactly the state this change exists to remove.
///
/// **Every ending is an `Ok`** ([`RestartOutcome`], #335). The two failures that used to be one
/// `Err(String)` are opposites — after [`RestartOutcome::NotStarted`] nothing is running, after
/// [`RestartOutcome::NeverStopped`] the old process still is — and a screen cannot tell them apart
/// from a sentence.
pub(crate) fn restart_service_impl(
    config_path: &std::path::Path,
    socket_path: &std::path::Path,
    only_if_running: bool,
) -> Result<RestartOutcome, String> {
    use std::time::{Duration, Instant};

    let probe = probe_from(&ipc::command(
        socket_path,
        ControlCommand::Status,
        ipc::DEFAULT_TIMEOUT,
    ));
    let started = |detail: String| RestartOutcome::Restarted { detail };
    match restart_plan(only_if_running, probe) {
        RestartPlan::LeaveStopped => return Ok(RestartOutcome::NotRunning),
        RestartPlan::LeaveUndetermined => {
            return Ok(RestartOutcome::Undetermined {
                reason: "the sync service did not answer, so the app could not tell whether it \
                         is running"
                    .to_string(),
            });
        }
        // Nothing to stop. Reported by the fact it leaves behind, not by the sequence: what the
        // screen has to say is whether the service is running the file on disk.
        RestartPlan::StartOnly => {
            return Ok(match start_service_impl(config_path) {
                Ok(detail) => started(format!(
                    "the service was not running; started it ({detail})"
                )),
                Err(reason) => RestartOutcome::NotStarted { reason },
            });
        }
        RestartPlan::StopThenStart => {}
    }

    // Best-effort: if the shutdown call itself errors the daemon may already be exiting;
    // the socket probe below is the authoritative "has it stopped" signal.
    let _ = ipc::command(socket_path, ControlCommand::Shutdown, ipc::DEFAULT_TIMEOUT);
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        // BY THE PROBE, NOT `is_err()` (#335). An undecodable reply mid-drain is a daemon that is
        // still up, and an unreachable socket says nothing either way — breaking on those would
        // start a second process beside a live one and then report a restart that did not happen.
        let last = probe_from(&ipc::command(
            socket_path,
            ControlCommand::Status,
            Duration::from_secs(1),
        ));
        if last == DaemonProbe::NotRunning {
            break;
        }
        // The ending is what was last OBSERVED, not the fact of having timed out.
        if Instant::now() >= deadline {
            return Ok(stop_timeout_outcome(last));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // The stop is CONFIRMED here — the socket is authoritatively absent — so a failed start leaves
    // nothing running, which is the ending the old code discarded along with the successful stop.
    Ok(match start_service_impl(config_path) {
        Ok(detail) => started(format!("the service restarted ({detail})")),
        Err(reason) => RestartOutcome::NotStarted { reason },
    })
}

/// Whether `outcome` confirms the OLD `socket_path` no longer needs dialling — the gate for #336's
/// re-resolve, beside the branch it guards for the same reason `restart_plan` and
/// `stop_timeout_outcome` are: a poison test of this decision must not be a test that starts a
/// service or waits out `STOP_TIMEOUT`.
///
/// [`RestartOutcome::Restarted`], [`RestartOutcome::NotRunning`] and [`RestartOutcome::NotStarted`]
/// all confirm it: a fresh process bound somewhere else, nothing was ever there, or the stop was
/// *confirmed* and only the start after it failed. [`RestartOutcome::NeverStopped`] is the opposite
/// on purpose — reachable, per its own doc, **only from an observation of life**, so the old daemon
/// is still answering at the old address, and moving `socket_path` off it here would draw
/// "unreachable" over a daemon that is not — the very shape #336 exists to remove, self-inflicted by
/// this fix. [`RestartOutcome::Undetermined`] and [`RestartOutcome::Unknown`] are evidence of
/// neither, and the conservative answer matches `NeverStopped`'s: keep dialling the one address
/// there is a positive history with rather than move to one there is none for.
fn old_socket_is_settled(outcome: &RestartOutcome) -> bool {
    matches!(
        outcome,
        RestartOutcome::Restarted { .. }
            | RestartOutcome::NotRunning
            | RestartOutcome::NotStarted { .. }
    )
}

/// Async so the up-to-~10s stop/start sequence never runs on the UI thread; the blocking work
/// itself happens on a runtime blocking thread.
#[tauri::command]
pub async fn restart_service(
    state: Paths<'_>,
    only_if_running: bool,
) -> Result<RestartOutcome, String> {
    let (config_path, socket_path) = {
        let paths = state.lock().unwrap();
        (paths.config_path.clone(), paths.socket_path.clone()?)
    };
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        restart_service_impl(&config_path, &socket_path, only_if_running)
    })
    .await
    .map_err(|error| format!("restart task failed: {error}"))??;
    // NOW it is safe to move `socket_path` to whatever the file on disk says (#336): the shutdown
    // and probe above already ran against the value captured before this line, and the write that
    // could have changed it — `write_config`, which deliberately left `socket_path` untouched —
    // always runs before this command, never concurrently with it. Gated on `old_socket_is_settled`
    // rather than unconditional: `NeverStopped`/`Undetermined` must keep dialling the address they
    // have positive (or at least prior) history with, not one nothing has confirmed yet.
    if old_socket_is_settled(&outcome) {
        state.lock().unwrap().socket_path = RuntimePaths::resolve().socket_path;
    }
    Ok(outcome)
}

// ------------------------------------------------------------------ notifications (S9) ----

/// Show one banner, replacing whichever of ours is still on screen.
///
/// **Linux only, and silent everywhere else.** The whole app is Linux-only (the engine's IPC is
/// Unix-socket) but the notification path is the one that would panic rather than degrade, so the
/// off-Linux arm answers `Ok` with nothing shown: a notification is an addition to the window, and
/// no build target should fail to compile over one.
#[tauri::command]
pub async fn send_notification(
    app: tauri::AppHandle,
    payload: crate::notify::NotifyPayload,
) -> Result<(), String> {
    crate::notify::send(app, payload).await
}

/// Take our banner down. Used when the thing it was about resolved itself.
#[tauri::command]
pub async fn close_notification(app: tauri::AppHandle) -> Result<(), String> {
    crate::notify::close(app).await
}

/// The GUI-local `notify_policy` (C6). Never sent to the daemon — see `gui_core::gui_prefs`.
///
/// `AppHandle` rather than `Paths<'_>`, and `spawn_blocking` rather than a read on the spot: both
/// are this module's own rules. A synchronous command runs on the GTK main loop and WebKitGTK aborts
/// the process when that loop stalls (#142/#143), and an async command taking a borrowed `State<'_>`
/// is forced to return `Result` — so the handle is owned and the state is taken inside.
#[tauri::command]
pub async fn read_notify_policy(app: tauri::AppHandle) -> Result<String, String> {
    let config_path = {
        let state = app.state::<Mutex<RuntimePaths>>();
        let paths = state.lock().unwrap();
        paths.config_path.clone()
    };
    tauri::async_runtime::spawn_blocking(move || {
        gui_core::gui_prefs::load_notify_policy(&gui_core::gui_prefs::gui_prefs_path(&config_path))
            .as_str()
            .to_string()
    })
    .await
    .map_err(|error| format!("read_notify_policy did not complete: {error}"))
}

/// Refuses an unknown token rather than defaulting: a write is a person choosing, and silently
/// storing something else would be the screen answering a question it was not asked.
#[tauri::command]
pub async fn write_notify_policy(app: tauri::AppHandle, policy: String) -> Result<(), String> {
    let parsed = gui_core::gui_prefs::NotifyPolicy::parse(&policy)
        .ok_or_else(|| format!("unknown notify_policy \"{policy}\""))?;
    let config_path = {
        let state = app.state::<Mutex<RuntimePaths>>();
        let paths = state.lock().unwrap();
        paths.config_path.clone()
    };
    tauri::async_runtime::spawn_blocking(move || {
        gui_core::gui_prefs::store_notify_policy(
            &gui_core::gui_prefs::gui_prefs_path(&config_path),
            parsed,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| format!("write_notify_policy did not complete: {error}"))?
}

// ------------------------------------------------------------ the openers (#220, #231) ----
//
// G18: four drawn buttons with nothing behind them — `Open both in an editor` (S2's conflict diff),
// `Open folder` and `Open on Proton Drive` (S5's lookup and pending dialog) and `Open the system
// log` (S5's passes footer and details dialog). This is a C-item, not a screen task: it adds the
// commands, and the screens that were shipped short of them are wired to it in the same change.
//
// NO NEW DEPENDENCY. `tauri-plugin-opener` would mean a capability grant, a dependabot surface and a
// Debian build rule for what `std::process::Command` already does — the same reasoning that has this
// module shelling `systemctl` and `proton-drive` rather than wrapping them.
//
// PATHS ARE GUARDED AT THIS BOUNDARY. Every relative path here comes from the webview, so it goes
// through `gui_core::opener`, which rejects absolute/`..`/prefix components AND a symlink that
// leaves the sync folder. Nothing builds a shell string: `Command` takes an argv, so a path
// containing `;` or `$(…)` is one argument and not a command.

/// The Drive web app.
///
/// **Not a deep link, and the button is honest about opening the app rather than the file.** A
/// per-file URL needs a share id and a link id (`/u/0/<shareId>/folder/<linkId>`); the GUI holds
/// neither. `proton_id` is the engine's composed `volumeId~nodeId`, which is an API identity and not
/// a route the web app resolves, and no reply on the wire carries a share. Constructing a plausible
/// URL out of it would ship a 404 behind a button that promises a file. Hardcoded here so the
/// webview passes no URL at all — the command takes no argument and there is nothing to inject.
const PROTON_DRIVE_URL: &str = "https://drive.proton.me/";

/// The desktop's opener. Not configurable: a user-supplied program name would be the one injection
/// surface this module otherwise does not have.
const OPENER: &str = "xdg-open";

/// How long to wait for the opener to fail before calling it launched.
///
/// `xdg-open` exits 3 (no handler) or 4 (the action failed) within milliseconds. A handler that DID
/// launch may keep the process alive for as long as the editor is open — the generic fallback execs
/// the application in the foreground — so a child still running past this point is a success, not a
/// hang, and waiting for it would leave the button spinning until the user closed their editor.
const OPEN_SETTLE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Hand one target to the opener and report whether it took it.
///
/// `program` is a parameter for the tests alone (`/bin/true`, `/bin/false`, a missing binary) —
/// every caller passes `OPENER`.
///
/// SPAWN, NOT `output()`: the opener may outlive the click by hours. The child is polled to the
/// deadline so a non-zero exit is still an error the user is told about, then reaped off-thread so
/// a long-lived editor does not become a zombie.
fn open_target(program: &str, target: &std::ffi::OsStr) -> Result<(), String> {
    use std::time::Instant;

    let mut child = Command::new(program)
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;

    let deadline = Instant::now() + OPEN_SETTLE;
    let exited = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => break None,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(e) => return Err(format!("couldn't wait for {program}: {e}")),
        }
    };
    match exited {
        None => {
            // Still running — the handler took over. Reap it in Tauri's bounded blocking pool.
            std::mem::drop(tauri::async_runtime::spawn_blocking(move || {
                let _ = child.wait();
            }));
            Ok(())
        }
        Some(status) if status.success() => Ok(()),
        // 3 is `xdg-open`'s own "a required tool could not be found", which in practice is the
        // "nothing is registered to open this" case — the one a silent no-op would hide.
        Some(status) if status.code() == Some(3) => Err(format!(
            "nothing on this computer is set up to open {} — choose a default application for this \
             kind of file and try again",
            target.to_string_lossy()
        )),
        Some(status) => Err(format!(
            "{program} couldn't open {} ({status})",
            target.to_string_lossy()
        )),
    }
}

/// Open one or both sides of a conflict in whatever the desktop opens that kind of file with
/// (#220 — S2's `Open both in an editor`).
///
/// Both paths are `Conflict`'s own `original` and `sidecar`, and both are re-validated here anyway:
/// the boundary rule is about where a path enters a join, not where it came from.
///
/// Every failure is reported, not the first: `Open both` opening one of two and saying nothing about
/// the other is the same silence the button had before. Every failure that is ABOUT a path, that is
/// — see the missing-root check below.
#[tauri::command]
pub async fn open_paths(state: Paths<'_>, relative: Vec<String>) -> Result<(), String> {
    let local_root = state.lock().unwrap().effective_local_root();
    tauri::async_runtime::spawn_blocking(move || {
        if relative.is_empty() {
            return Err("nothing to open".to_string());
        }
        // ONE ROOT, ONE ANSWER. The root is read once, before the loop, so "there is no sync folder"
        // is a fact about the app and not one fact per path — resolving each side of a conflict
        // against `None` pushed the identical sentence twice and joined it to itself. Every OTHER
        // refusal names the path it is about, so only this one can duplicate. (Copilot, PR #283.)
        if local_root.is_none() {
            return Err(gui_core::opener::OpenRefusal::NoLocalRoot.to_string());
        }
        let mut failures = Vec::new();
        for path in &relative {
            let resolved = match gui_core::opener::resolve_under_root(local_root.as_deref(), path) {
                Ok(resolved) => resolved,
                Err(refusal) => {
                    failures.push(refusal.to_string());
                    continue;
                }
            };
            if let Err(e) = open_target(OPENER, resolved.as_os_str()) {
                failures.push(e);
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    })
    .await
    .map_err(|error| format!("open task failed: {error}"))?
}

/// Open the folder a file sits in, in the desktop's file manager (#231 — S5's `Open folder`).
///
/// The parent is computed HERE. Letting the webview send a directory would mean trusting it to have
/// stripped the last component, and a path that is a file or a folder depending on who called is
/// exactly the ambiguity the guard is for.
#[tauri::command]
pub async fn open_folder(state: Paths<'_>, relative: String) -> Result<(), String> {
    let local_root = state.lock().unwrap().effective_local_root();
    tauri::async_runtime::spawn_blocking(move || {
        let folder = gui_core::opener::folder_under_root(local_root.as_deref(), &relative)
            .map_err(|refusal| refusal.to_string())?;
        open_target(OPENER, folder.as_os_str())
    })
    .await
    .map_err(|error| format!("open task failed: {error}"))?
}

/// Open Proton Drive in the browser (#231 — S5's `Open on Proton Drive`). Takes no argument; see
/// `PROTON_DRIVE_URL` for why it cannot land on the file.
#[tauri::command]
pub async fn open_remote() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        open_target(OPENER, std::ffi::OsStr::new(PROTON_DRIVE_URL))
    })
    .await
    .map_err(|error| format!("open task failed: {error}"))?
}

/// Where the log snapshot is written. One stable name, so clicking twice does not litter.
fn log_snapshot_path() -> std::path::PathBuf {
    log_snapshot_path_in(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

/// A RELATIVE BASE DIRECTORY IS INVALID AND IS IGNORED (`config_path::absolute_dir`, #286).
///
/// Honouring one would drop the snapshot under the GUI's own working directory — whatever the
/// desktop launcher happened to leave it as — and hand `xdg-open` a path that resolves somewhere
/// else again. `HOME` carries the same requirement: a relative one puts the fallback in exactly
/// the unpredictable place the rule exists to avoid, and `temp_dir` behind it is always absolute.
///
/// Taking the two values as arguments rather than reading them is what makes the rule testable:
/// setting a process environment variable in a test races every other test in the binary (and is
/// `unsafe` since edition 2024). `config.rs`'s `expand_tilde_with_home` splits itself the same way.
fn log_snapshot_path_in(
    cache_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> std::path::PathBuf {
    let base = crate::config_path::absolute_dir(cache_home)
        .or_else(|| crate::config_path::absolute_dir(home).map(|home| home.join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("proton-sync").join("proton-syncd-log.txt")
}

#[cfg(unix)]
fn write_private_log_snapshot(
    destination: &std::path::Path,
    contents: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "log path has no parent")
        })?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.file_type().is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "log snapshot parent is not a directory: {}",
                parent.display()
            ),
        ));
    }
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;

    let base = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("proton-syncd-log");
    let (mut file, temporary) = loop {
        let candidate = destination.with_file_name(format!(
            ".{base}.{}.{}.tmp",
            std::process::id(),
            unique_log_snapshot_suffix()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(file) => break (file, candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    let write = (|| {
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()
    })();
    if let Err(error) = write.and_then(|_| std::fs::rename(&temporary, destination)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn unique_log_snapshot_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

#[cfg(not(unix))]
fn write_private_log_snapshot(
    destination: &std::path::Path,
    contents: &[u8],
) -> std::io::Result<()> {
    if let Some(parent) = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(destination, contents)
}

/// `journalctl --user -u proton-syncd`, as a file the desktop can open (#231 — S5's
/// `Open the system log`).
///
/// THERE IS NOTHING ELSE TO OPEN. The daemon logs through `tracing` to stderr, and the shipped user
/// unit captures that in the journal — which is a binary store with no path and no registered
/// handler, so `xdg-open` has nothing to point at. A terminal emulator is not a thing a Linux
/// desktop guarantees either. So the journal is read into a `.txt` beside the GUI's other cached
/// state and that is opened; the header line names the command, so anyone who wants it live has it.
///
/// A DAEMON STARTED OUTSIDE SYSTEMD HAS NO LOG AT ALL — `start_service`'s fallback path nulls the
/// child's stderr. Then `journalctl` answers with no entries and this returns the command as an
/// error rather than opening an empty file, which would read as "there is nothing wrong".
fn write_log_snapshot(journalctl: &str, destination: &std::path::Path) -> Result<(), String> {
    let output = Command::new(journalctl)
        .args([
            "--user",
            "-u",
            "proton-syncd",
            "-n",
            "1000",
            "--no-pager",
            "--output",
            "short-iso",
        ])
        .output()
        .map_err(|e| {
            format!(
                "couldn't read the system log ({e}) — this machine may not use systemd. The daemon's \
                 log, when there is one, is: journalctl --user -u proton-syncd"
            )
        })?;
    let body = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    // `-- No entries --` is journalctl's own answer for a unit it has never seen. Treated as empty:
    // the file would otherwise open on one line that looks like a log.
    let empty = body
        .lines()
        .all(|line| line.trim().is_empty() || line.trim().starts_with("-- No entries"));
    if !output.status.success() || empty {
        let detail = strip_ansi(String::from_utf8_lossy(&output.stderr).trim());
        let detail = if detail.is_empty() {
            "the journal has no entries for proton-syncd".to_string()
        } else {
            detail
        };
        return Err(format!(
            "no system log to open: {detail}. The daemon only logs to the journal when it runs as a \
             systemd user service — start it with `systemctl --user start proton-syncd`, or read it \
             with `journalctl --user -u proton-syncd`"
        ));
    }
    let header =
        "# proton-syncd — the last 1000 journal entries, copied at the moment you clicked.\n\
                  # Live: journalctl --user -u proton-syncd -f\n\n";
    write_private_log_snapshot(destination, format!("{header}{body}").as_bytes())
        .map_err(|e| format!("couldn't write {}: {e}", destination.display()))
}

#[tauri::command]
pub async fn open_system_log() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let destination = log_snapshot_path();
        write_log_snapshot("journalctl", &destination)?;
        open_target(OPENER, destination.as_os_str())
    })
    .await
    .map_err(|error| format!("open task failed: {error}"))?
}

// ------------------------------------------------------- the Phase-1 capability commands ----

/// Room on the filesystem the sync folder lives on (C4, #177).
///
/// `path` prices a folder the config has not been written for yet — onboarding step 1 offers a pair
/// before anything is saved, so the screen must be able to ask about a proposal. Omitted, it uses
/// the configured local root.
///
/// **Only half of `9a Review`'s sentence.** `Needs 38.4 GB free` cannot be computed: no level of
/// the dry-run surface carries a file size (G6, #206). See `gui_core::free_space` and DEVIATIONS.
///
/// Async because a hung network or FUSE mount makes `statvfs` block indefinitely, and the sync
/// folder being on such a mount is exactly the case where the number matters.
#[tauri::command]
pub async fn free_space(
    app: tauri::AppHandle,
    path: Option<String>,
) -> Result<gui_core::free_space::FreeSpace, String> {
    let target = match path {
        // A path typed into the folder picker gets the same `~` expansion the config values get —
        // the picker is a text field, and `~/ProtonDrive` is what someone types.
        Some(path) => config_io::expand_config_path(path, "path"),
        None => app
            .state::<Mutex<RuntimePaths>>()
            .lock()
            .unwrap()
            .effective_local_root()
            .ok_or_else(|| "local_root is not configured".to_string())?,
    };
    tauri::async_runtime::spawn_blocking(move || gui_core::free_space::for_path(&target))
        .await
        .map_err(|error| format!("free-space task failed: {error}"))?
}

/// Whether the `proton-drive` CLI is installed, and which distribution this is (C5, #178).
///
/// Two facts in one reply because they are drawn on one dialog and answered at one moment. The
/// screen only ever appears when `installed` is false — the CLI check is otherwise a silent
/// precondition, which is what let onboarding drop from four steps to two.
///
/// **Never cached.** This backs both the silent precondition and the `Check again` button, and the
/// user is expected to install the tool between two calls.
///
/// `installed` is *presence*, not health: a CLI that is installed but logged out is `true`. Sending
/// an authenticated-but-failing user to an install screen would answer a question they do not have.
/// The probe uses the **configured** `proton_cli`, which may be an absolute path (#158's boot-PATH
/// fix), so a tool that is installed and merely off the launcher's PATH is not reported missing.
#[derive(serde::Serialize)]
pub struct CliPresence {
    installed: bool,
    distro: Option<gui_core::distro::Distro>,
}

/// How long the CLI probe waits before giving up on it. Generous for a `--version`, short enough
/// that onboarding does not appear to have died.
const CLI_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tauri::command]
pub async fn check_cli(app: tauri::AppHandle) -> CliPresence {
    let proton_cli = app
        .state::<Mutex<RuntimePaths>>()
        .lock()
        .unwrap()
        .proton_cli
        .clone();
    tauri::async_runtime::spawn_blocking(move || CliPresence {
        installed: probe_cli(&proton_cli),
        distro: gui_core::distro::detect_here(),
    })
    .await
    // A failed join is not "no CLI and no distribution" — but this command cannot reject (the
    // screen must render), so it degrades to the state that shows the install dialog, which is the
    // safe direction: it tells the user to check something rather than assuming it is fine.
    .unwrap_or(CliPresence {
        installed: false,
        distro: None,
    })
}

/// Is the CLI there? Bounded by [`CLI_PROBE_TIMEOUT`], because nothing else in this command is.
///
/// **A probe that never returns is the failure mode this screen cannot have.** `check_cli` is the
/// silent precondition *and* the `Check again` button, so a `proton_cli` that hangs — a stale
/// keyring prompt, a wedged network mount holding the binary, an absolute path pointing at a FIFO —
/// would park a `spawn_blocking` thread forever, leave the JS promise unresolved, and let every
/// press of `Check again` park another. `Command::status()` on its own has no timeout, which is why
/// this polls instead.
///
/// **A successful spawn IS the answer**, and the exit status is deliberately ignored. `installed`
/// means the executable is there, and a `proton-drive` that exits non-zero on `--version` — a
/// wrapper script, a missing shared library, a broken install — is present and broken, not absent.
/// Routing that user to an install screen would answer a question they do not have, which is the
/// same mistake as doing it for a CLI that is merely logged out. Only a failed spawn (`ENOENT`, not
/// executable) is "isn't installed".
///
/// So the wait exists purely to **reap**: an unwaited child stays a zombie for the life of the GUI,
/// and `Check again` can be pressed repeatedly.
fn probe_cli(proton_cli: &str) -> bool {
    let mut child = match Command::new(proton_cli)
        .arg("--version")
        // **stdin is nulled, not inherited.** `Command::status()`/`spawn()` inherit stdin by
        // default, while every other subprocess here uses `.output()` (which pipes it) — so this
        // was the one place a CLI that reads stdin before exiting could block on the GUI's own
        // terminal, forever, with no keyboard attached to answer it. Null makes that an immediate
        // EOF.
        .stdin(std::process::Stdio::null())
        // Null rather than inherited, so a chatty CLI can neither fill a pipe nobody drains nor
        // print into the GUI's own stdout.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    let deadline = std::time::Instant::now() + CLI_PROBE_TIMEOUT;
    loop {
        // `Err` here means the child is unwaitable — still not a statement about presence.
        if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// What each skip rule is hiding right now (C2, #175).
///
/// `patterns` is the rule set **currently on screen**, not the saved config: `8a Skip rules` is
/// drawn mid-edit with a pending removal, and the Add row prices a pattern before it is saved.
///
/// **Pass `include` whenever the config has any**, from the same `read_config` reply the tab is
/// already showing. Omitting it does not fail — it silently widens every count to files the include
/// list already keeps out of the sync, so a rule is credited with hiding something it is not, and
/// `will start syncing` promises files that would not. The Advanced tab owns those globs; this
/// command only needs to know they exist.
///
/// Async and unbounded — this walks the whole local tree (metadata only, no hashing). Running it on
/// the GTK main loop would freeze the window for the length of the walk.
#[tauri::command]
pub async fn skip_rule_usage(
    app: tauri::AppHandle,
    patterns: Vec<String>,
    include: Option<Vec<String>>,
) -> Result<gui_core::skip_rules::SkipRuleReport, String> {
    let (local_root, db_path, naming) = {
        let paths = app.state::<Mutex<RuntimePaths>>();
        let paths = paths.lock().unwrap();
        (
            paths.effective_local_root(),
            paths.effective_db_path(),
            // The baseline `measure` builds is the denominator — "would the daemon sync this
            // file" — and a conflict sidecar is one of the things it answers no to, so it has to
            // ask under the daemon's configured suffix rather than the compiled-in one.
            paths.conflict_naming.clone(),
        )
    };
    // Two different "no" answers, and only one of them makes a rule safe to remove.
    //
    // `is not configured` is the easy one. The dangerous one is a root that IS configured and is
    // not there — an unmounted external drive, a literal `~/ProtonDrive` because `HOME` was unset,
    // a folder the user moved. `measure` would happily walk nothing, and every rule would come back
    // `files: 0` with `folder_exists: Some(false)`, which the tab draws as **`Matching nothing · no
    // such folder here any more — safe to remove`** on every rule at once. Removing them would then
    // start syncing everything they were hiding, the moment the drive came back.
    let local_root = local_root.ok_or_else(|| "local_root is not configured".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        if !local_root.is_dir() {
            return Err(format!(
                "the sync folder {} is not there right now",
                local_root.display()
            ));
        }
        gui_core::skip_rules::measure(
            &local_root,
            &patterns,
            &include.unwrap_or_default(),
            &daemon_ignored_paths(db_path.as_ref()),
            &naming,
        )
    })
    .await
    .map_err(|error| format!("skip-rule scan failed: {error}"))?
}

/// Prices a **candidate** folder before the pair is configured (G25, #240): `9a Folders` shows what
/// is under each side so a wrong folder is caught by its size rather than by its name.
///
/// `side` selects which walk runs, because the two are not the same operation and only one of them
/// answers both numbers:
///
/// - `"local"` — a metadata walk; `files` and `bytes` are both real.
/// - `"remote"` — a bounded listing walk; `files` is real (a lower bound once `truncated`), and
///   **`bytes` is always `null`**, because a remote listing exposes no usable size. Render that as
///   unknown, never as `0 bytes`. [`gui_core::folder_probe`] documents why `totalStorageSize`
///   cannot stand in for it.
///
/// Takes the candidate `path` outright rather than reading the configured roots — this runs during
/// onboarding, before either root exists.
///
/// The remote side **asks the daemon** (#323): one `list` per directory, so every `proton-drive`
/// child is spawned by the daemon behind its one `CliGate` rather than by this process beside it
/// (#23). It falls back to spawning children here only when nothing answers the socket at all,
/// which is `run_dry_run`'s rule and is exactly onboarding, where no daemon exists yet.
///
/// Async: the local side walks a tree and the remote side does up to 64 socket round trips (or, on
/// the fallback, spawns subprocesses), and either on the GTK main loop would freeze the window.
#[tauri::command]
pub async fn probe_folder(
    app: tauri::AppHandle,
    side: String,
    path: String,
) -> Result<gui_core::folder_probe::FolderProbe, String> {
    let (proton_cli, socket) = {
        let paths = app.state::<Mutex<RuntimePaths>>();
        let paths = paths.lock().unwrap();
        (paths.proton_cli.clone(), paths.socket_path.clone())
    };
    tauri::async_runtime::spawn_blocking(move || match side.as_str() {
        "local" => gui_core::folder_probe::probe_local(std::path::Path::new(&path)),
        "remote" => probe_remote_folder(socket, std::path::Path::new(&path), &proton_cli),
        other => Err(format!("unknown side: {other}")),
    })
    .await
    .map_err(|error| format!("folder probe failed: {error}"))?
}

/// The remote probe's daemon-first, child-only-if-nothing-answers rule (#323), split out so it is
/// testable without a Tauri handle.
///
/// The fallback is taken on **one** condition: the very first `list` never reached the socket. Every
/// other answer — a refusal, an outcome this build cannot read, a socket that died mid-walk — is a
/// daemon, and reported. Falling back on any of those would put a second `proton-drive` client
/// beside a live one, which is the whole hazard.
///
/// An unresolvable socket path (#74's fail-closed default) counts as "no daemon": there is nothing
/// to ask, and the child is the only thing that can answer at all.
fn probe_remote_folder(
    socket: Result<std::path::PathBuf, String>,
    candidate: &std::path::Path,
    proton_cli: &str,
) -> Result<gui_core::folder_probe::FolderProbe, String> {
    use gui_core::folder_probe::{describe_probe_failure, probe_remote_via_cli, ProbeListingError};

    let fall_back = || probe_remote_via_cli(proton_cli, candidate);
    let Ok(socket) = socket else {
        return fall_back();
    };
    match gui_core::folder_probe::probe_remote_via_daemon(&socket, candidate) {
        Ok(probe) => Ok(probe),
        Err(ProbeListingError::Unreachable(error)) => match daemon_presence(&socket) {
            // It answered `status` but not `list`. **Two causes, and neither may be named as the
            // one** (Copilot review): a daemon predating the absolute selector drops the connection
            // unparsed, and a current one busy with a long transfer can outrun
            // `PROBE_LISTING_TIMEOUT` — `IpcError::Unreachable` covers a timeout and an early close
            // alike, so this arm cannot tell them apart and must not pretend to. What it can say is
            // what to try, in the order that costs least. Either way: report, never walk beside it.
            DaemonPresence::Answering => Err(format!(
                "the sync daemon is running but did not measure that folder ({error}). It may be \
                 busy with a transfer, or older than this app — try again in a moment, and restart \
                 it from Settings if it keeps happening."
            )),
            DaemonPresence::Undecodable(reply) => Err(format!(
                "the sync daemon answered with something this app could not read ({reply}). \
                 Restart it from Settings and try again."
            )),
            DaemonPresence::Absent => fall_back(),
        },
        // Exhaustive by variant, no `_`: both of these are the daemon's own answer and are
        // reported, and a variant added later must be *given* an arm rather than inherit this one.
        Err(error @ (ProbeListingError::Busy | ProbeListingError::Failed(_))) => {
            Err(describe_probe_failure(error))
        }
    }
}

/// The state files the daemon keeps out of its own scan, so a relocated index inside the sync root
/// is not reported as user data some rule is hiding.
///
/// Mirrors `scan_options_from_config`: the index plus its two JSON sidecars. (`ScanOptions::new`
/// expands SQLite's own `-journal`/`-wal`/`-shm` siblings, and a top-level `.sync/` and the download
/// scratch directory are handled by `should_ignore_path`, so this is the remainder.)
fn daemon_ignored_paths(db_path: Option<&std::path::PathBuf>) -> Vec<std::path::PathBuf> {
    let Some(db_path) = db_path else {
        return Vec::new();
    };
    // `with_extension`, exactly as `status_history_path`/`metrics_path` in the daemon do it — so the
    // sidecars REPLACE the `.db` (`sync_index.status.json`) rather than extending it. Appending
    // would name two files that never exist, leaving the two that do inside the walk, where they
    // would be counted as user data some rule is hiding.
    vec![
        db_path.clone(),
        db_path.with_extension("status.json"),
        db_path.with_extension("metrics.json"),
    ]
}

/// `Ctrl W` — hide the window to the tray. Deliberately the SAME path as the tray's
/// `Close window (keeps syncing in the tray)` item and as the window-manager close button
/// (`on_window_event` in lib.rs prevents the close and hides): three ways to do one thing, and a
/// keyboard shortcut that did something subtly different would be the worst of the three.
///
/// Synchronous on purpose. The rule that GUI commands must be `async` + `spawn_blocking` is about
/// commands that shell out, walk the filesystem, or do a socket round trip — one of those blocking
/// the WebKitGTK main loop is what aborted the process in #142. `hide()` does none of that.
#[tauri::command]
pub fn close_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "no main window".to_string())?;
    window.hide().map_err(|e| e.to_string())
}

/// `Ctrl Q`, and the tray's `Quit` — **stop the daemon, then end the GUI process.**
///
/// # This resolves DEVIATIONS §45, which F4 left open and assigned here
///
/// `10-tray.md` and `14-behaviour-and-state.md` agree with each other and always did: `Ctrl W`
/// closes the window *keeps syncing*, `Ctrl Q` quits *stops syncing*, and the tray must carry both
/// as sub-labels because — 10-tray.md's words — "this is the single worst misunderstanding a tray
/// app can cause". The shipped build did something else: `app.exit(0)`, ending the GUI while the
/// daemon carried on, "a separate process and unaffected by either".
///
/// F4 declined to settle it from a keyboard shortcut and said why: stopping a sync daemon is a
/// lifecycle decision with data consequences, and guessing the more destructive of two readings is
/// the wrong way to reach it. S8 owns the tray, so S8 answers — and the answer is forced, because
/// S8 is the task that puts `Quit` and `stops syncing` on screen together. Once that sub-label is
/// drawn there are only two possibilities: the daemon stops, or the app lies about what a button
/// does in the one place the design says it must not.
///
/// So it stops. `Close window · keeps syncing` sits directly above it and is the path for someone
/// who wants the tray gone and the syncing left alone; both labels are now true.
///
/// The shutdown is the daemon's own graceful path — the same `Shutdown` command `proton-sync stop`
/// and the GUI's restart flow already use, which exits through SIGTERM's route and cancels any
/// in-flight `proton-drive` invocation. A failure to reach it is deliberately NOT fatal to the quit:
/// a daemon that is already gone, or wedged, must not leave a user unable to close the app.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    quit_stopping_the_daemon(app);
}

/// The body of `quit_app`, callable from the tray's menu handler (which is not a command).
pub fn quit_stopping_the_daemon(app: tauri::AppHandle) {
    let socket = {
        let state = app.state::<Mutex<RuntimePaths>>();
        let guard = state.lock().unwrap();
        guard.socket_path.clone()
    };
    // An unlocatable socket (#277) must not block the quit — it is the same "already gone" case
    // the failed-shutdown path below already treats as non-fatal.
    let socket = match socket {
        Ok(socket) => socket,
        Err(reason) => {
            eprintln!("quit: could not locate the daemon's control socket ({reason}); exiting");
            app.exit(0);
            return;
        }
    };
    // On a worker, with the app's exit behind it: the control socket blocks up to DEFAULT_TIMEOUT,
    // and doing that on the UI thread is the WebKitGTK freeze this crate has already shipped once
    // (PR #142). The exit follows the attempt either way.
    std::thread::spawn(move || {
        if let Err(error) = ipc::command(&socket, ControlCommand::Shutdown, ipc::DEFAULT_TIMEOUT) {
            eprintln!("quit: could not stop the daemon ({error}); exiting anyway");
        }
        app.exit(0);
    });
}

/// What a tray row does, independent of which indicator drew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayRow {
    Open,
    SyncNow,
    Pause,
    Resume,
    /// The one row that is not a control command: there is no daemon to send one to.
    Start,
    CloseWindow,
    Quit,
}

/// Every id either indicator may send, in one table.
///
/// THIS EXISTS BECAUSE THE COMMENT THAT USED TO SIT HERE WAS FALSE. It claimed "one id space for
/// both indicators", and three of the seven ids disagreed: the panel sent `syncNow`/`tryAgain`/
/// `closeWindow` and the fallback menu built `sync_now`/`try_again`/`close_window`, each dispatched
/// by its own `match` in its own file. Nothing was broken — each handler understood its own menu —
/// which is exactly what made it worth fixing: two vocabularies that happen to work are a trap for
/// whoever edits one of them, and the comment promised they were one thing.
///
/// So they are one thing now. `ui/compact.js`'s `TRAY_MENU` is the source of the id strings, both
/// native menus build their rows from `tray_menu::rows_for` — which carries the same ids — and all
/// three dispatch through here. An id this does not know returns `None` and the caller reports it
/// rather than silently doing nothing.
///
/// (The comment this replaces pointed at a `FALLBACK_IDS` table "below" that was never written: the
/// fallback menu spelled its rows out by hand. #252 gave it the table the comment described.)
pub fn tray_row(id: &str) -> Option<TrayRow> {
    Some(match id {
        // `Review them` is the panel's own decision button rather than a menu row, and it goes where
        // `Open Drive Sync` goes — see the note in `tray_action`.
        "open" | "review" => TrayRow::Open,
        // `Try again now` IS a sync, and the state it is offered in is `Failed`: the daemon is
        // answering and its last pass was not. It USED to be offered for `Unreachable` as well, on
        // the reasoning that "the thing to retry is reaching it" — but the retry is a `Syncnow` sent
        // down the same socket that just refused the connection, so there was nothing there to
        // reach. That state gets `start` instead.
        "syncNow" | "tryAgain" => TrayRow::SyncNow,
        "pause" => TrayRow::Pause,
        "resume" => TrayRow::Resume,
        "start" => TrayRow::Start,
        "closeWindow" => TrayRow::CloseWindow,
        "quit" => TrayRow::Quit,
        _ => return None,
    })
}

/// The tray panel's rows, dispatched by the id `ui/compact.js`'s `TRAY_MENU` gives them.
#[tauri::command]
pub async fn tray_action(app: tauri::AppHandle, id: String) -> StatusPayload {
    // Every row dismisses the panel. It is a popover: leaving it up over the window it just opened
    // is the "lingering after blur" failure by another route.
    crate::panel::hide(&app);
    // `Review them` and `Open Drive Sync` both land in the window. They differ in where they should
    // land — the deletions queue against the main screen — and S8 does not split them: the panel is
    // dismissed by then, and a `tray-navigate` to a screen the window may be mid-onboarding on is a
    // second routing question this task does not own. Recorded as DEVIATIONS §82l.
    let command = match tray_row(&id) {
        Some(TrayRow::Open) => {
            // `tray::show_window`, not a second copy of it. This WAS the second copy, and it is how
            // the raise fix landed on one of the two paths: the native menus' row raised the window
            // and the panel's own row — this one — left it exactly where it was. §92b.
            crate::tray::show_window(&app);
            ControlCommand::Status
        }
        Some(TrayRow::SyncNow) => ControlCommand::Syncnow,
        Some(TrayRow::Pause) => ControlCommand::Pause,
        Some(TrayRow::Resume) => ControlCommand::Resume,
        Some(TrayRow::Start) => {
            // `spawn_blocking`, like every other subprocess on this surface: `systemctl --user
            // start` blocks until the unit reports started, and a stalled GTK main loop aborts
            // WebKitGTK outright (#142/#143). The guard is cloned out and dropped before the await —
            // a `MutexGuard` is not `Send` and cannot cross one.
            let config_path = {
                let paths: Paths = app.state();
                let path = paths.lock().unwrap().config_path.clone();
                path
            };
            match tauri::async_runtime::spawn_blocking(move || start_service_impl(&config_path))
                .await
            {
                Ok(Ok(detail)) => eprintln!("tray: {detail}"),
                // STDERR IS THE WHOLE REPORT HERE, and that is a limit of the surface rather than a
                // choice: the panel is hidden by the time this runs (every row dismisses it), so
                // there is nothing left on screen to render a reason into. The window's own button
                // quotes the same message — `mainProps`' `startError` — which is why the failure
                // path people can act on is the one in `app.js` and not this one.
                Ok(Err(error)) => eprintln!("tray: could not start the daemon: {error}"),
                Err(join_error) => eprintln!("tray: start-service task failed: {join_error}"),
            }
            // The reply to a `Status` sent THIS instant will usually still say unreachable — the
            // daemon binds its socket after we return. Correct, and not worth special-casing: the
            // tray polls on its own ~2s cadence and the next tick tells the truth.
            ControlCommand::Status
        }
        Some(TrayRow::CloseWindow) => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            ControlCommand::Status
        }
        Some(TrayRow::Quit) => {
            quit_stopping_the_daemon(app.clone());
            ControlCommand::Status
        }
        // An unknown id is a frontend that has grown a row this build does not implement. Answer
        // with the status rather than nothing, so the panel still repaints and the row does not read
        // as a hang — but say so, because silence here is a menu row that does nothing.
        None => {
            eprintln!("tray: no action for row id {id:?}");
            ControlCommand::Status
        }
    };
    status_round_trip(app, command).await
}

/// The panel measures itself once it knows its state and asks the window to match. The states differ
/// by 120px between the tallest and the shortest, and Phase 1 omits lines the frames draw, so a
/// fixed height is either clipped content or a band of empty panel below the menu.
#[tauri::command]
pub fn resize_tray_panel(app: tauri::AppHandle, height: f64) {
    crate::panel::resize(&app, height);
}

/// Esc, and a click on a row. The blur path is handled in `lib.rs`, on the window event.
#[tauri::command]
pub fn hide_tray_panel(app: tauri::AppHandle) {
    crate::panel::hide(&app);
}

#[cfg(test)]
mod tests {
    use super::{
        daemon_ignored_paths, daemon_plans_the_same_roots, probe_cli, relative_query,
        status_error_proves_no_daemon, strip_ansi,
    };
    use gui_core::ipc::IpcError;
    use std::path::Path;

    /// Every user-facing sentence this module builds, rendered rather than merely constructed.
    ///
    /// A `\`-newline continuation in a Rust literal is silently eaten when a patch script writes it
    /// from a non-raw Python string, baking the next line's indentation into the value — invisible
    /// to `cargo fmt`, to clippy, and to any test that compares a constant with itself. So this
    /// asserts what a user would see: no run of spaces inside a sentence.
    /// (`docs/agent-notes/python-patch-scripts-and-rust-string-continuations.md`.)
    #[test]
    fn no_message_carries_a_swallowed_line_continuation() {
        use gui_core::wire::PlanOutcome;

        for outcome in [
            None,
            Some(PlanOutcome::Unknown),
            Some(PlanOutcome::Absent),
            Some(PlanOutcome::Failed {
                plan_seq: 1,
                error: "x".to_owned(),
            }),
            Some(PlanOutcome::Computing { plan_seq: 1 }),
            Some(PlanOutcome::Paused),
        ] {
            let message = super::unexpected_plan_ack(outcome.as_ref());
            assert!(!message.contains("  "), "{message:?}");
        }
    }

    /// The fallback to `proton-syncd --dry-run` may only be taken as evidence of **absence**, and
    /// an undecodable reply is the opposite: something is bound to that socket and answering.
    ///
    /// `classify_unreachable_plan` reads the follow-up `status` by variant rather than `is_err()`
    /// for exactly this. A `Protocol` error means a daemon replied and this app could not read it —
    /// a version skew — and spawning a child there puts a second `proton-drive` client beside a
    /// live daemon (#23/#317), the hazard the whole verb exists to retire. Only `Unreachable` (no
    /// socket, refused, closed early, timed out) says nothing is there.
    ///
    /// #335 split `NotListening` off `Unreachable`; **both** still answer `true` here, and that is a
    /// preserved reading rather than an oversight — see the predicate's own note. What this test
    /// pins is the arm that must never move: `Protocol` is a daemon.
    #[test]
    fn an_undecodable_status_reply_is_a_live_daemon_not_an_absent_one() {
        assert!(status_error_proves_no_daemon(&IpcError::NotListening(
            "no such file".to_owned()
        )));
        assert!(status_error_proves_no_daemon(&IpcError::Unreachable(
            "read: timed out".to_owned()
        )));
        assert!(!status_error_proves_no_daemon(&IpcError::Protocol(
            "expected value at line 1".to_owned()
        )));
    }

    /// The fork behind the plan verb (#100/#209/#317), and the second condition is the subtle one:
    /// the daemon plans against ITS OWN roots, so a config file that already names a different pair
    /// than the running daemon must go to the child — which is handed the file's pair — rather than
    /// silently preview folders nobody asked about.
    #[test]
    fn the_daemon_is_asked_for_a_plan_only_when_it_is_syncing_the_same_folders() {
        let local = Path::new("/home/me/ProtonDrive");
        let remote = Path::new("/Drive/RemoteFolder");
        let other = Path::new("/home/me/Elsewhere");

        // Nothing has answered the socket: there is no daemon to ask.
        assert!(!daemon_plans_the_same_roots(
            Some(local),
            Some(remote),
            None,
            None
        ));
        assert!(!daemon_plans_the_same_roots(
            Some(local),
            Some(remote),
            Some(local),
            None
        ));
        // The file says nothing, so the daemon's pair IS the effective one.
        assert!(daemon_plans_the_same_roots(
            None,
            None,
            Some(local),
            Some(remote)
        ));
        // The file agrees.
        assert!(daemon_plans_the_same_roots(
            Some(local),
            Some(remote),
            Some(local),
            Some(remote)
        ));
        // The file has moved on and the daemon has not been restarted onto it: ask the child.
        assert!(!daemon_plans_the_same_roots(
            Some(other),
            Some(remote),
            Some(local),
            Some(remote)
        ));
        assert!(!daemon_plans_the_same_roots(
            Some(local),
            Some(Path::new("/Drive/Other")),
            Some(local),
            Some(remote)
        ));
    }

    #[test]
    fn a_pasted_absolute_path_is_reduced_to_the_one_the_index_stores() {
        let root = Path::new("/home/me/ProtonDrive");
        assert_eq!(
            relative_query("/home/me/ProtonDrive/docs/spec.md", Some(root)),
            "docs/spec.md"
        );
        // A trailing separator on the root is the same root.
        assert_eq!(
            relative_query(
                "/home/me/ProtonDrive/docs/spec.md",
                Some(Path::new("/home/me/ProtonDrive/"))
            ),
            "docs/spec.md"
        );
    }

    #[test]
    fn a_path_outside_the_sync_folder_keeps_its_components_and_loses_its_leading_slash() {
        // Better a fragment match than "no such file": the user may well be looking for the name.
        // NOT "left alone" — the separator goes, because no stored path begins with one.
        assert_eq!(
            relative_query("/etc/hosts", Some(Path::new("/home/me/ProtonDrive"))),
            "etc/hosts"
        );
    }

    #[test]
    fn a_root_that_is_only_a_textual_prefix_strips_nothing() {
        // The string version turned this into `-Other/x.md` — a path from outside the sync folder,
        // mangled into one that could match inside it.
        assert_eq!(
            relative_query(
                "/home/me/ProtonDrive-Other/x.md",
                Some(Path::new("/home/me/ProtonDrive"))
            ),
            "home/me/ProtonDrive-Other/x.md"
        );
    }

    #[test]
    fn a_bare_name_survives_untouched() {
        assert_eq!(
            relative_query("  spec.md  ", Some(Path::new("/home/me/ProtonDrive"))),
            "spec.md"
        );
        assert_eq!(relative_query("docs/spec.md", None), "docs/spec.md");
    }

    #[test]
    fn a_leading_tilde_expands_the_way_the_daemon_expands_it() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let root = Path::new(&home).join("ProtonDrive");
        assert_eq!(
            relative_query("~/ProtonDrive/docs/spec.md", Some(&root)),
            "docs/spec.md"
        );
        // `~user` is somebody else's home: not expanded, and not a path this index stores.
        assert_eq!(
            relative_query("~someone/file.md", Some(&root)),
            "~someone/file.md"
        );
    }

    #[test]
    fn a_missing_binary_is_the_only_thing_that_reads_as_not_installed() {
        assert!(
            !probe_cli("proton-drive-that-is-definitely-not-installed-xyzzy"),
            "a failed spawn is the real 'isn't installed'"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_present_binary_that_exits_non_zero_is_still_present() {
        // `installed` is presence, not health. A wrapper script, a missing shared library, or a
        // `--version` this tool does not implement all exit non-zero — and routing that user to an
        // install screen answers a question they do not have. `/bin/false` stands in for all three.
        assert!(probe_cli("/bin/false"));
        assert!(probe_cli("/bin/true"));
    }

    #[test]
    fn the_ignored_set_matches_the_daemons_when_there_is_an_index_and_is_empty_when_there_is_not() {
        assert!(daemon_ignored_paths(None).is_empty());
        let db = std::path::PathBuf::from("/x/.sync/sync_index.db");
        let ignored = daemon_ignored_paths(Some(&db));
        // The sidecars REPLACE the `.db`, because `status_history_path`/`metrics_path` in the
        // daemon are `with_extension`. Appending would name two files that never exist and leave
        // the two that do inside the walk, counted as user data some rule is hiding.
        assert_eq!(
            ignored,
            vec![
                db.clone(),
                std::path::PathBuf::from("/x/.sync/sync_index.status.json"),
                std::path::PathBuf::from("/x/.sync/sync_index.metrics.json"),
            ]
        );
    }

    // The openers (#220/#231). `open_target` takes its program as a parameter precisely so these
    // run without a desktop: `/bin/true` is a handler that took the file, `/bin/false` is one that
    // refused it, and a missing binary is a machine with no `xdg-open` at all. No real `xdg-open`
    // is ever spawned by the suite.

    #[test]
    #[cfg(unix)]
    fn an_opener_that_takes_the_file_reports_success() {
        assert!(super::open_target("/bin/true", std::ffi::OsStr::new("/tmp/x")).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn an_opener_that_refuses_the_file_is_an_error_the_user_is_told_about() {
        let error = super::open_target("/bin/false", std::ffi::OsStr::new("/tmp/x"))
            .expect_err("a non-zero exit must not read as opened");
        // The path is IN the message: "it didn't open" without saying what is the silence this
        // whole change exists to remove.
        assert!(error.contains("/tmp/x"), "got {error}");
    }

    #[test]
    fn a_missing_opener_is_an_error_rather_than_a_silent_no_op() {
        let error = super::open_target(
            "xdg-open-that-is-not-installed-xyzzy",
            std::ffi::OsStr::new("/tmp/x"),
        )
        .expect_err("a failed spawn must not read as opened");
        assert!(
            error.contains("xdg-open-that-is-not-installed-xyzzy"),
            "got {error}"
        );
    }

    #[test]
    fn a_child_that_outlives_the_deadline_is_a_launched_handler_and_not_a_hang() {
        // `sleep 30` stands in for an editor left open. The call must return well inside the 30s,
        // because a button that waits for the editor to close is a button that looks broken.
        if !Path::new("/bin/sleep").exists() {
            return;
        }
        let started = std::time::Instant::now();
        assert!(super::open_target("/bin/sleep", std::ffi::OsStr::new("30")).is_ok());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "returned after {:?} — it waited for the handler",
            started.elapsed()
        );
    }

    #[test]
    fn a_journal_with_no_entries_is_refused_rather_than_opened_empty() {
        // `/bin/true` is a journalctl that succeeds and prints nothing — the shape of a daemon
        // started outside systemd, whose stderr `start_service`'s fallback nulls.
        if !Path::new("/bin/true").exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("log.txt");
        let error = super::write_log_snapshot("/bin/true", &destination)
            .expect_err("an empty journal must not become an empty file that reads as 'all clear'");
        assert!(
            error.contains("journalctl --user -u proton-syncd"),
            "got {error}"
        );
        assert!(!destination.exists(), "nothing should have been written");
    }

    #[test]
    fn a_missing_journalctl_names_the_command_it_could_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let error = super::write_log_snapshot(
            "journalctl-that-is-not-installed-xyzzy",
            &dir.path().join("l.txt"),
        )
        .expect_err("no journalctl is not a log");
        assert!(
            error.contains("journalctl --user -u proton-syncd"),
            "got {error}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_journal_with_entries_is_written_with_the_live_command_in_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("nested").join("log.txt");
        // `/bin/echo` ignores journalctl's flags and prints them — enough of a non-empty journal.
        if !Path::new("/bin/echo").exists() {
            return;
        }
        super::write_log_snapshot("/bin/echo", &destination).expect("a non-empty journal writes");
        let written = std::fs::read_to_string(&destination).unwrap();
        assert!(
            written.contains("journalctl --user -u proton-syncd -f"),
            "got {written}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_journal_snapshot_and_its_cache_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;

        if !Path::new("/bin/echo").exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let destination = dir
            .path()
            .join("cache")
            .join("proton-sync")
            .join("proton-syncd-log.txt");
        super::write_log_snapshot("/bin/echo", &destination)
            .expect("a non-empty journal writes a private snapshot");

        let file_mode = std::fs::metadata(&destination)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600, "snapshot must be private");
        let parent_mode = std::fs::metadata(destination.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o700, "cache directory must be private");
    }

    #[test]
    fn the_log_snapshot_lives_under_a_cache_directory_and_ends_in_txt() {
        // `.txt` is load-bearing: `xdg-open` picks a handler by type, and an extensionless file has
        // none on most desktops.
        let path = super::log_snapshot_path();
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("txt"));
        // Unconditionally absolute now: every branch of `log_snapshot_path_in` ends in an absolute
        // base, `temp_dir` included.
        assert!(path.is_absolute(), "got {}", path.display());
    }

    #[test]
    fn a_relative_xdg_cache_home_is_ignored_rather_than_resolved_against_the_cwd() {
        use std::ffi::OsString;
        // The XDG spec calls a relative value invalid and says to ignore it. Honoured, the snapshot
        // would land under whatever cwd the desktop launcher left the GUI in — and `~/.cache`, the
        // literal a shell-less process never expands (#135), is not absolute either.
        for relative in ["cache", ".cache", "~/.cache", "", "sub/dir"] {
            let path = super::log_snapshot_path_in(
                Some(OsString::from(relative)),
                Some(OsString::from("/home/me")),
            );
            assert_eq!(
                path,
                std::path::PathBuf::from("/home/me/.cache/proton-sync/proton-syncd-log.txt"),
                "XDG_CACHE_HOME={relative:?} was honoured"
            );
        }
    }

    #[test]
    fn an_absolute_xdg_cache_home_is_honoured() {
        use std::ffi::OsString;
        let path = super::log_snapshot_path_in(
            Some(OsString::from("/var/tmp/cache")),
            Some(OsString::from("/home/me")),
        );
        assert_eq!(
            path,
            std::path::PathBuf::from("/var/tmp/cache/proton-sync/proton-syncd-log.txt")
        );
    }

    #[test]
    fn a_relative_home_falls_through_to_the_temp_dir_rather_than_the_cwd() {
        use std::ffi::OsString;
        // Same rule, second variable — and the last fallback is absolute by construction.
        for home in [None, Some(OsString::from("me")), Some(OsString::from(""))] {
            let path = super::log_snapshot_path_in(None, home.clone());
            assert!(path.is_absolute(), "HOME={home:?} gave {}", path.display());
            assert!(path.starts_with(std::env::temp_dir()), "HOME={home:?}");
        }
    }

    #[test]
    fn the_proton_drive_url_is_the_web_app_and_is_not_built_from_anything() {
        // The command takes no argument; this pins the constant so a later "improvement" that
        // interpolates an id has to change the test that says why it cannot.
        assert_eq!(super::PROTON_DRIVE_URL, "https://drive.proton.me/");
    }

    #[test]
    fn strip_ansi_removes_color_sequences_and_keeps_text() {
        let colored = "\u{1b}[2m2026-07-27\u{1b}[0m \u{1b}[33m WARN\u{1b}[0m list failed";
        assert_eq!(strip_ansi(colored), "2026-07-27  WARN list failed");
        assert_eq!(strip_ansi("plain text"), "plain text");
        // A dangling escape at end-of-input must not panic or loop.
        assert_eq!(strip_ansi("tail\u{1b}"), "tail");
        assert_eq!(strip_ansi("tail\u{1b}["), "tail");
    }
}

// The socket-command tests need Unix domain sockets, so they are gated `unix` (mirroring
// `gui-core/src/ipc.rs`); the portable `strip_ansi` test above stays under plain `#[cfg(test)]`.
#[cfg(all(test, unix))]
mod socket_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;
    use std::thread;

    /// A canned status reply matching `ControlResponse` (mirrors gui-core's ipc tests).
    const CANNED_REPLY: &str = r#"{"status":"running","paused":false,"pending_changes":3,"message":"sync completed","last_sync_epoch_secs":1750000000,"last_error":null,"last_plan_summary":null,"last_successful_sync_summary":null,"status_history":[],"pending_deletions":[]}"#;

    /// A one-shot fake daemon: bind a Unix socket, read one request line, write back `reply`.
    fn spawn_one_shot_daemon(reply: &'static str) -> (std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
            }
        });
        (path, dir)
    }

    /// A fake daemon that keeps answering, one canned reply per request, in order — and repeats the
    /// last one for ever after. Needed for the probe, which is a *walk*: one `list` request per
    /// directory, and a `status` follow-up when one of them does not complete.
    /// An **empty** reply means "close the connection without answering" — which is exactly what a
    /// daemon does with a command it cannot parse, and therefore the only way to model a daemon
    /// older than a verb.
    fn spawn_repeating_daemon(replies: Vec<String>) -> (std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let mut index = 0usize;
            while let Ok((stream, _)) = listener.accept() {
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                let reply = replies[index.min(replies.len() - 1)].clone();
                index += 1;
                if !reply.is_empty() {
                    let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
                }
            }
        });
        (path, dir)
    }

    /// The canned status reply with a `listing` field spliced in.
    fn listing_reply(listing: serde_json::Value) -> String {
        let mut reply: serde_json::Value = serde_json::from_str(CANNED_REPLY).expect("canned");
        reply["listing"] = listing;
        serde_json::to_string(&reply).expect("serialize")
    }

    /// A `proton_cli` name nothing can execute, so taking the child fallback is *observable*: the
    /// walk's first listing fails with this binary's name in it, which no daemon-side sentence
    /// contains.
    const UNRUNNABLE_CLI: &str = "proton-drive-that-does-not-exist-323";

    /// #323's fallback rule, which is the one thing this change must not get wrong: the GUI may
    /// spawn its own `proton-drive` children **only** when nothing answers the socket at all.
    #[test]
    fn the_remote_probe_falls_back_to_a_child_only_when_no_daemon_answers() {
        let candidate = std::path::Path::new("/Drive/Candidate");

        // 1. No socket path could even be resolved (#74 fails closed). There is nothing to ask.
        let error = probe_remote_folder(
            Err("cannot resolve the control socket path".to_owned()),
            candidate,
            UNRUNNABLE_CLI,
        )
        .expect_err("the bogus CLI cannot answer either");
        assert!(
            error.contains(UNRUNNABLE_CLI),
            "an unresolvable socket must take the child, which is the only thing left: {error}"
        );

        // 2. A socket path that nothing is listening on — onboarding, the case the child exists for.
        let dir = tempfile::tempdir().unwrap();
        let error = probe_remote_folder(
            Ok(dir.path().join("nothing-here.sock")),
            candidate,
            UNRUNNABLE_CLI,
        )
        .expect_err("the bogus CLI cannot answer either");
        assert!(error.contains(UNRUNNABLE_CLI), "{error}");

        // 3. A daemon that answers `status` but never a listing — one older than this app, which
        // knows `list` but refuses an absolute selector. It ANSWERED, so the child must not run.
        let (socket, _dir) = spawn_repeating_daemon(vec![CANNED_REPLY.to_owned()]);
        let error = probe_remote_folder(Ok(socket), candidate, UNRUNNABLE_CLI)
            .expect_err("a daemon that listed nothing is not a probe result");
        assert!(
            !error.contains(UNRUNNABLE_CLI),
            "a live daemon must never send a second proton-drive client out (#23/#317): {error}"
        );
        assert!(error.contains("sync daemon"), "{error}");

        // 4. THE ARM THAT MATTERS. A daemon that drops the `list` connection without replying —
        // which is precisely what one predating the verb does with a command it cannot parse — and
        // then answers `status` normally. At the transport that first failure is `Unreachable`,
        // indistinguishable from a missing socket, so the fallback rule cannot be read off it: the
        // follow-up `status` is what decides, and it says a daemon is there.
        let (socket, _dir) = spawn_repeating_daemon(vec![String::new(), CANNED_REPLY.to_owned()]);
        let error = probe_remote_folder(Ok(socket), candidate, UNRUNNABLE_CLI)
            .expect_err("a daemon too old for the verb is reported, not walked beside");
        assert!(
            !error.contains(UNRUNNABLE_CLI),
            "a dropped connection from a LIVE daemon must not be read as absence: {error}"
        );
        // And it must name BOTH causes it cannot tell apart: this same arm is reached by a current
        // daemon whose listing outran `PROBE_LISTING_TIMEOUT`, since `IpcError::Unreachable` covers
        // a timeout and an early close alike. Blaming the version alone sends a user to restart a
        // daemon that was only busy (Copilot review).
        assert!(error.contains("older than this app"), "{error}");
        assert!(error.contains("busy"), "{error}");

        // 5. A daemon that answers the listing: the walk succeeds and nothing is spawned here.
        let (socket, _dir) = spawn_repeating_daemon(vec![listing_reply(serde_json::json!({
            "state": "listed",
            "path": "/Drive/Candidate",
            "entries": [{
                "path": "/Drive/Candidate/a.txt",
                "name": "a.txt",
                "entity_kind": "file",
                "sha1": null,
                "downloadable": true
            }],
            "total": 1,
            "truncated": false
        }))]);
        let probe = probe_remote_folder(Ok(socket), candidate, UNRUNNABLE_CLI)
            .expect("the daemon answered the walk");
        assert_eq!(probe.files, 1);
        assert_eq!(
            probe.bytes, None,
            "a remote listing exposes no usable size, however it was obtained"
        );
    }

    /// A headless mock app (no webview/display) managing `RuntimePaths` pointed at `socket`.
    fn mock_app(socket: std::path::PathBuf) -> tauri::App<tauri::test::MockRuntime> {
        let mut paths = RuntimePaths::resolve();
        paths.socket_path = Ok(socket);
        tauri::test::mock_builder()
            .manage(Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build")
    }

    #[test]
    fn spawn_blocking_ipc_runs_the_round_trip_off_thread_and_parses_the_reply() {
        let (path, _dir) = spawn_one_shot_daemon(CANNED_REPLY);
        let reply = tauri::async_runtime::block_on(spawn_blocking_ipc(move || {
            ipc::command(&path, ControlCommand::Status, ipc::DEFAULT_TIMEOUT)
        }));
        let response = reply.expect("round trip should succeed");
        assert_eq!(response.status, "running");
        assert_eq!(response.pending_changes, 3);
    }

    #[test]
    fn spawn_blocking_ipc_maps_a_missing_socket_to_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.sock");
        let reply = tauri::async_runtime::block_on(spawn_blocking_ipc(move || {
            ipc::command(&missing, ControlCommand::Status, ipc::DEFAULT_TIMEOUT)
        }));
        // `NotListening` since #335 — a missing socket is the one transport failure that proves
        // nothing is there, and it is drawn as the unreachable state all the same (`derive_state`).
        assert!(
            matches!(reply, Err(ipc::IpcError::NotListening(_))),
            "got {reply:?}"
        );
    }

    // The two tests below drive the real command helpers through a mock `AppHandle`, exercising the
    // `app.state::<Mutex<RuntimePaths>>()` runtime lookup and the reply → `StatusPayload` fold that
    // compile/clippy cannot check — a payload regression would surface here as `Unreachable`.

    #[test]
    fn status_round_trip_resolves_app_state_and_folds_a_live_reply() {
        let (socket, _dir) = spawn_one_shot_daemon(CANNED_REPLY);
        let app = mock_app(socket);
        let payload = tauri::async_runtime::block_on(status_round_trip(
            app.handle().clone(),
            ControlCommand::Status,
        ));
        assert!(
            payload.error.is_none(),
            "unexpected error: {:?}",
            payload.error
        );
        assert!(payload.response.is_some(), "expected a decoded response");
        assert_ne!(payload.state, gui_core::DaemonState::Unreachable);
    }

    /// #277: the socket path can now fail to resolve at all (the engine's fallback fails closed).
    /// That is the unreachable state WITH ITS REASON — not a panic, and not a guessed path whose
    /// ENOENT would name a file the daemon was never asked to bind.
    #[test]
    fn an_unresolvable_socket_path_folds_into_the_unreachable_state_with_its_reason() {
        let (socket, _dir) = spawn_one_shot_daemon(CANNED_REPLY);
        let app = mock_app(socket);
        app.state::<Mutex<RuntimePaths>>()
            .lock()
            .unwrap()
            .socket_path = Err("fallback runtime directory is owned by uid 1234".to_owned());

        let payload = tauri::async_runtime::block_on(status_round_trip(
            app.handle().clone(),
            ControlCommand::Status,
        ));

        assert_eq!(payload.state, gui_core::DaemonState::Unreachable);
        assert!(payload.response.is_none());
        let error = payload.error.expect("the reason must reach the UI");
        assert!(
            error.contains("owned by uid 1234"),
            "the engine's own reason must survive: {error}"
        );
    }

    /// #320. A settings save restarts the daemon so the Plan screen can never preview one folder
    /// pair while `Run` executes another — but it must not START one that was not running: that
    /// would make a save mean "and begin syncing", which is not what the button says. The truth
    /// table is pinned here rather than through `restart_service_impl`, because the branch decides
    /// whether `systemctl --user start proton-syncd` runs and a test of it may not be a test that
    /// runs one. (`docs/agent-notes/gui-tests-that-shell-systemctl.md`.)
    #[test]
    fn a_save_never_starts_a_service_that_was_not_running() {
        // The save path, against a daemon that is not there: the one combination that does nothing.
        assert_eq!(
            restart_plan(true, DaemonProbe::NotRunning),
            RestartPlan::LeaveStopped
        );
        // The save path against a live daemon: the whole point — its roots are about to be stale.
        assert_eq!(
            restart_plan(true, DaemonProbe::Running),
            RestartPlan::StopThenStart
        );
        // The explicit retry, which must start a daemon a failed restart left stopped. Folding this
        // into the arm above would leave the one state #320 exists to remove with no way out of it.
        assert_eq!(
            restart_plan(false, DaemonProbe::NotRunning),
            RestartPlan::StartOnly
        );
        assert_eq!(
            restart_plan(false, DaemonProbe::Running),
            RestartPlan::StopThenStart
        );
    }

    /// #335. An unknown probe is not an absence, and on the SAVE path it may not be treated as one:
    /// "it was not running, so it will use these settings when it starts" is an assertion about a
    /// daemon that may be live on the old settings this second. Doing nothing and saying so is the
    /// only honest answer there.
    ///
    /// The retry is the other way round, and for a reason that is not symmetry: the user asked for a
    /// restart, and `StopThenStart` cannot report a success that did not happen — it starts only
    /// after the socket has gone *authoritatively* absent.
    #[test]
    fn an_undetermined_probe_stops_a_save_and_is_attempted_by_an_explicit_retry() {
        assert_eq!(
            restart_plan(true, DaemonProbe::Unknown),
            RestartPlan::LeaveUndetermined
        );
        assert_eq!(
            restart_plan(false, DaemonProbe::Unknown),
            RestartPlan::StopThenStart
        );
    }

    /// A drain that timed out having never seen the socket answer may not claim a live process.
    ///
    /// The review of #338 found this: the loop breaks only on `NotRunning`, so a `DaemonProbe`
    /// stuck on `Unknown` — which is what `NotADirectory` (a socket path under a regular file),
    /// `InvalidInput` (longer than `sun_len`) and `PermissionDenied` all produce, on every
    /// iteration — ran the full 8s and answered `NeverStopped`, whose sentence is *"It is still
    /// running the old settings."* A positive claim about a live process from an observation of
    /// nothing, on the one arm #335 §2 reasoned about explicitly.
    #[test]
    fn a_drain_that_observed_nothing_may_not_claim_the_old_process_is_still_up() {
        assert!(matches!(
            stop_timeout_outcome(DaemonProbe::Unknown),
            RestartOutcome::Undetermined { .. }
        ));
        // The claim is earned only by a socket that was still ANSWERING at the deadline.
        assert!(matches!(
            stop_timeout_outcome(DaemonProbe::Running),
            RestartOutcome::NeverStopped { .. }
        ));
        // And the sentence each carries, since that is what the whole split is for.
        let RestartOutcome::Undetermined { reason } = stop_timeout_outcome(DaemonProbe::Unknown)
        else {
            panic!("checked above");
        };
        assert!(
            !reason.contains("still answering"),
            "an undetermined drain must claim nothing about a live process: {reason}"
        );
    }

    /// The wiring for the arm above, end to end and with no spawn in it: a socket path whose parent
    /// is a regular file cannot name a listener, and `connect` fails `NotADirectory` — which is
    /// evidence of neither, so the SAVE path leaves everything alone and says so.
    ///
    /// Green-path by construction: `only_if_running` + a probe that is not `Running` returns before
    /// any `Command`, so this can never start the developer's daemon.
    #[test]
    fn a_socket_path_that_cannot_name_a_listener_is_undetermined_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_directory = dir.path().join("notes.txt");
        std::fs::write(&not_a_directory, b"a regular file").unwrap();
        let socket = not_a_directory.join("proton-sync.sock");

        let reply = ipc::command(&socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT);
        assert_eq!(
            probe_from(&reply),
            DaemonProbe::Unknown,
            "a path that cannot name a socket says nothing about whether a daemon is running \
             somewhere else — which is #336's subject, not an absence"
        );

        let outcome =
            restart_service_impl(&dir.path().join("nothing-here.toml"), &socket, true).unwrap();
        assert!(
            matches!(outcome, RestartOutcome::Undetermined { .. }),
            "got {outcome:?}"
        );
    }

    /// #335's other half: which reply is evidence of what. A reply that would not decode is a daemon
    /// ANSWERING — the pre-#335 `is_ok()` read it as absence, skipped the shutdown, started a unit
    /// that was already active (`systemctl start` succeeds for one), and reported "restarted, and is
    /// running your new settings" about the old process. No ending may report a success that did not
    /// happen, so this classification is the thing that has to be right.
    #[test]
    fn the_probe_reads_an_undecodable_reply_as_life_and_a_timeout_as_neither() {
        assert_eq!(
            probe_from(&Err(ipc::IpcError::Protocol("expected value".into()))),
            DaemonProbe::Running,
            "it replied — undecodably, but it replied"
        );
        assert_eq!(
            probe_from(&Err(ipc::IpcError::NotListening("no such file".into()))),
            DaemonProbe::NotRunning,
            "nothing was bound: the only authoritative absence"
        );
        assert_eq!(
            probe_from(&Err(ipc::IpcError::Unreachable("read: timed out".into()))),
            DaemonProbe::Unknown,
            "the connect succeeded, so this is evidence of neither"
        );
    }

    /// The wiring: a socket nothing is listening on, asked the save path's way, answers the typed
    /// `NotRunning` ending rather than starting anything. Green-path only — with `only_if_running`
    /// and an absent socket the decision returns before any spawn, so this test can never reach
    /// `start_service_impl` and start the developer's real daemon.
    #[test]
    fn a_save_restart_against_a_dead_socket_reports_that_nothing_was_restarted() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = restart_service_impl(
            &dir.path().join("nothing-here.toml"),
            &dir.path().join("unused.sock"),
            true,
        )
        .expect("a service that is not running is not a failed restart");
        assert_eq!(outcome, RestartOutcome::NotRunning);
    }

    /// The endings cross the bridge as data, not prose (#335/#103): each is tagged, and a tag this
    /// build does not know degrades to `Unknown` instead of failing the parse. The webview is the
    /// real consumer and has its own fall-through arm; this pins the wire shape it reads.
    #[test]
    fn every_restart_ending_is_tagged_and_an_unknown_tag_degrades() {
        let json = |outcome: &RestartOutcome| serde_json::to_string(outcome).expect("serialize");
        assert!(json(&RestartOutcome::NotRunning).contains(r#""ending":"not_running""#));
        assert!(json(&RestartOutcome::NotStarted {
            reason: "ENOENT".into()
        })
        .contains(r#""ending":"not_started""#));
        assert!(json(&RestartOutcome::NeverStopped {
            reason: "8s".into()
        })
        .contains(r#""ending":"never_stopped""#));
        assert!(json(&RestartOutcome::Undetermined {
            reason: "no answer".into()
        })
        .contains(r#""ending":"undetermined""#));
        assert!(json(&RestartOutcome::Restarted {
            detail: "systemd".into()
        })
        .contains(r#""ending":"restarted""#));
        let newer: RestartOutcome =
            serde_json::from_str(r#"{"ending":"reloaded_in_place","detail":"x"}"#)
                .expect("an unknown ending must parse, not fail the whole reply");
        assert_eq!(newer, RestartOutcome::Unknown);
    }

    /// #336's own gate, exhaustively: pure, so poisoning it can never start the developer's daemon
    /// or spend `STOP_TIMEOUT`'s 8s, the same reason `a_save_never_starts_a_service_that_was_not_
    /// running` tests `restart_plan` rather than `restart_service_impl`.
    #[test]
    fn old_socket_is_settled_only_when_the_old_process_cannot_still_be_running_there() {
        // Confirmed no longer bound to the old address: a fresh process bound elsewhere, nothing
        // was ever there, or the stop was CONFIRMED and only the start after it failed.
        assert!(old_socket_is_settled(&RestartOutcome::Restarted {
            detail: "systemd".into()
        }));
        assert!(old_socket_is_settled(&RestartOutcome::NotRunning));
        assert!(old_socket_is_settled(&RestartOutcome::NotStarted {
            reason: "ENOENT".into()
        }));
        // Reachable ONLY from an observation of life (`stop_timeout_outcome`'s own doc): the old
        // daemon is still answering there, so moving off it would draw "unreachable" over a daemon
        // that plainly is not — #336's own symptom, self-inflicted by this fix.
        assert!(!old_socket_is_settled(&RestartOutcome::NeverStopped {
            reason: "still answering".into()
        }));
        // Evidence of neither. The conservative answer matches `NeverStopped`'s: keep dialling the
        // address there is a positive history with rather than one there is none for.
        assert!(!old_socket_is_settled(&RestartOutcome::Undetermined {
            reason: "no answer".into()
        }));
        assert!(!old_socket_is_settled(&RestartOutcome::Unknown));
    }

    /// A `ConfigUpdate` naming nothing but `socket_path` — every other field `None`, matching what a
    /// screen that only touched the Advanced tab's socket field would send.
    fn socket_path_update(value: &std::path::Path) -> ConfigUpdate {
        ConfigUpdate {
            socket_path: Some(value.display().to_string()),
            ..Default::default()
        }
    }

    /// #336. `write_config` must leave `socket_path` exactly where it was, however different the
    /// value it just wrote to disk is: `restart_service` runs next (see `saveSettings` in app.js)
    /// and has to dial THIS value to shut the still-live old daemon down. Moving it here points that
    /// dial at an address nothing has bound yet.
    ///
    /// `config_path` is redirected to a tempfile — never the real GUI config `RuntimePaths::resolve`
    /// would otherwise read and write on whatever machine runs this test.
    #[test]
    fn a_save_leaves_socket_path_alone_for_the_restart_that_reads_it_next() {
        let dir = tempfile::tempdir().unwrap();
        let old_socket = dir.path().join("old.sock");
        let new_socket = dir.path().join("new.sock");

        let mut paths = RuntimePaths::resolve();
        paths.config_path = dir.path().join("proton-sync.toml");
        paths.socket_path = Ok(old_socket.clone());
        let app = tauri::test::mock_builder()
            .manage(Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build");

        write_config(
            app.state::<Mutex<RuntimePaths>>(),
            socket_path_update(&new_socket),
        )
        .expect("an absolute socket_path saves");

        let after = app.state::<Mutex<RuntimePaths>>();
        let after = after.lock().unwrap();
        assert_eq!(
            after.socket_path.as_deref().ok(),
            Some(old_socket.as_path()),
            "write_config moved socket_path off the value the restart still has to dial (#336)"
        );
    }

    /// The corner the preservation above must not create: an `Err` `socket_path` (#74's fail-closed
    /// state) names no address a restart could dial, so there is nothing to preserve, and NOT
    /// re-resolving here would strand the session on `Err` for ever — `restart_service`'s own
    /// re-resolve is unreachable while `socket_path` is `Err`, since its first line's `?` returns
    /// before reaching it. Typing a working `socket_path` over a failed-closed one must take on
    /// this save, not on the next relaunch.
    #[test]
    fn a_save_recovers_a_failed_closed_socket_path_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let new_socket = dir.path().join("new.sock");

        let mut paths = RuntimePaths::resolve();
        paths.config_path = dir.path().join("proton-sync.toml");
        paths.socket_path = Err("fallback runtime directory is owned by uid 1234".to_owned());
        let app = tauri::test::mock_builder()
            .manage(Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build");

        write_config(
            app.state::<Mutex<RuntimePaths>>(),
            socket_path_update(&new_socket),
        )
        .expect("an absolute socket_path saves");

        // Equality with a FRESH resolve, not `.is_ok()`: on a machine where the engine's own
        // fallback also fails closed, the honest post-save value is another `Err`, and the test
        // must not read that as a regression — it must read the SAME value a resolve computes now,
        // whichever it is (the same env-robust pattern the sequence test below uses).
        let after = app.state::<Mutex<RuntimePaths>>();
        let after_socket = after.lock().unwrap().socket_path.clone();
        assert_eq!(
            after_socket.as_deref().ok(),
            RuntimePaths::resolve().socket_path.ok().as_deref(),
            "a save must not leave a previously-Err socket_path stuck at Err (#336)"
        );
    }

    /// #336, end to end: the SEQUENCE `saveSettings` drives — `write_config` then `restart_service`
    /// — not either function alone. Proves both orderings at once: reverting the `write_config` hunk
    /// above fails this at the first assertion below (before `restart_service` even runs); reverting
    /// `restart_service`'s re-resolve fails the second.
    ///
    /// SAFE BY CONSTRUCTION: nothing listens on `old_socket`, so the probe reads `NotRunning` and
    /// `only_if_running`'s `RestartPlan::LeaveStopped` returns before any `Command` runs — the same
    /// green path `a_save_restart_against_a_dead_socket_reports_that_nothing_was_restarted` uses, so
    /// this can never start the developer's real daemon
    /// (`docs/agent-notes/gui-tests-that-shell-systemctl.md`).
    #[test]
    fn restart_moves_socket_path_only_after_it_has_dialled_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let old_socket = dir.path().join("old.sock");
        let new_socket = dir.path().join("new.sock");

        let mut paths = RuntimePaths::resolve();
        paths.config_path = dir.path().join("proton-sync.toml");
        paths.socket_path = Ok(old_socket.clone());
        let app = tauri::test::mock_builder()
            .manage(Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build");

        // The save. Must not move `socket_path` — pinned directly above, re-checked here because
        // it is the premise the rest of this test depends on.
        write_config(
            app.state::<Mutex<RuntimePaths>>(),
            socket_path_update(&new_socket),
        )
        .expect("an absolute socket_path saves");
        assert_eq!(
            app.state::<Mutex<RuntimePaths>>()
                .lock()
                .unwrap()
                .socket_path
                .as_deref()
                .ok(),
            Some(old_socket.as_path()),
            "premise: the save must still be pointed at the old socket"
        );

        // The restart. Nothing listens on `old_socket`, so this is the green `NotRunning` path.
        let outcome = tauri::async_runtime::block_on(restart_service(
            app.state::<Mutex<RuntimePaths>>(),
            true,
        ))
        .expect("a dead socket is not a failed restart");
        assert_eq!(outcome, RestartOutcome::NotRunning);

        // `NotRunning` is one of `old_socket_is_settled`'s outcomes, so the re-resolve must have
        // fired and moved `socket_path` OFF `old_socket` — otherwise every later request (status,
        // activity, the tray) keeps dialling an address the save already abandoned.
        let after = app.state::<Mutex<RuntimePaths>>();
        let after_socket = after.lock().unwrap().socket_path.clone();
        assert_ne!(
            after_socket.as_deref().ok(),
            Some(old_socket.as_path()),
            "a settled outcome must move the app off the address it just confirmed is unneeded"
        );
        // And it must move to exactly what a fresh resolve computes NOW — not the literal string
        // this test wrote (`RuntimePaths::resolve`'s own `config_path` is the fixed, env-resolved
        // one — see `config_path.rs` — not `state`'s, so it cannot see `new_socket` either) and not
        // a guess: a later request reads this same field, so proving it equals a fresh resolve IS
        // proving a later request reaches wherever that resolve currently points.
        assert_eq!(
            after_socket.as_deref().ok(),
            RuntimePaths::resolve().socket_path.ok().as_deref()
        );
    }

    /// A mock app whose managed paths name `root` as the sync folder. The socket is deliberately a
    /// path nothing is listening on: the opener commands never touch it.
    fn mock_app_rooted(root: &std::path::Path) -> tauri::App<tauri::test::MockRuntime> {
        let mut paths = RuntimePaths::resolve();
        paths.socket_path = Ok(root.join("unused.sock"));
        paths.local_root = Some(root.to_path_buf());
        tauri::test::mock_builder()
            .manage(Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build")
    }

    /// A mock app that knows of no sync folder at all — BOTH sources cleared. `RuntimePaths::resolve`
    /// reads the developer's real GUI config, so leaving `local_root` alone would make this test
    /// pass or fail depending on whose machine it runs on.
    fn mock_app_rootless(dir: &std::path::Path) -> tauri::App<tauri::test::MockRuntime> {
        let mut paths = RuntimePaths::resolve();
        paths.socket_path = Ok(dir.join("unused.sock"));
        paths.local_root = None;
        paths.daemon_local_root = None;
        tauri::test::mock_builder()
            .manage(Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build")
    }

    // The two below drive the REAL commands through a mock app, which is what proves the guard runs
    // before the spawn: each returns its refusal, and no `xdg-open` is ever reached. A test that
    // called the resolver directly would prove the resolver and not the command.

    #[test]
    fn open_folder_refuses_an_escaping_path_before_it_spawns_anything() {
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app_rooted(dir.path());
        for hostile in ["../../etc", "/etc", "docs/../../etc"] {
            let error = tauri::async_runtime::block_on(open_folder(
                app.state::<Mutex<RuntimePaths>>(),
                hostile.to_string(),
            ))
            .expect_err("an escaping path must never reach the opener");
            assert!(
                error.contains("inside the sync folder"),
                "{hostile} gave: {error}"
            );
        }
    }

    #[test]
    fn open_paths_reports_every_refusal_and_not_just_the_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), b"x").unwrap();
        let app = mock_app_rooted(dir.path());
        let error = tauri::async_runtime::block_on(open_paths(
            app.state::<Mutex<RuntimePaths>>(),
            vec!["/etc/passwd".to_string(), "gone.txt".to_string()],
        ))
        .expect_err("both sides are unopenable");
        assert!(error.contains("/etc/passwd"), "got {error}");
        assert!(error.contains("gone.txt"), "got {error}");
    }

    #[test]
    fn a_missing_sync_folder_is_said_once_however_many_paths_were_asked_for() {
        // The root is ONE fact about the app. Resolved per path it was pushed per path, so
        // `Open both in an editor` — which always sends two — printed the identical sentence twice,
        // joined to itself with `; `. (Copilot, PR #283.)
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app_rootless(dir.path());
        let error = tauri::async_runtime::block_on(open_paths(
            app.state::<Mutex<RuntimePaths>>(),
            vec![
                "notes/todo.txt".to_string(),
                "notes/todo.proton-cloud.txt".to_string(),
            ],
        ))
        .expect_err("with no sync folder there is nothing to open");
        assert_eq!(
            error,
            gui_core::opener::OpenRefusal::NoLocalRoot.to_string()
        );
        assert!(
            !error.contains(';'),
            "one refusal, not a joined list: {error}"
        );
    }

    #[test]
    fn open_paths_with_nothing_to_open_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let app = mock_app_rooted(dir.path());
        let error =
            tauri::async_runtime::block_on(open_paths(app.state::<Mutex<RuntimePaths>>(), vec![]))
                .expect_err("an empty list is not a successful open");
        assert_eq!(error, "nothing to open");
    }

    #[test]
    fn approval_round_trip_reaches_the_daemon_and_folds_a_live_reply() {
        let (socket, _dir) = spawn_one_shot_daemon(CANNED_REPLY);
        let app = mock_app(socket);
        let payload = tauri::async_runtime::block_on(approval_round_trip(
            app.handle().clone(),
            ControlCommand::Approve,
            "some/file.txt".to_string(),
            true,
            None,
        ));
        assert!(
            payload.error.is_none(),
            "unexpected error: {:?}",
            payload.error
        );
        assert_ne!(payload.state, gui_core::DaemonState::Unreachable);
    }
}
