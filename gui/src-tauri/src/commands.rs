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
use gui_core::wire::{ControlCommand, ControlResponse, DryRunReport, PendingDeletion};
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
/// next pass to a full-tree walk (`ControlShared.force_full_walk`, consumed once), so this is the
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
) -> StatusPayload {
    let socket = match socket_path_for_ipc(&app.state()) {
        Ok(socket) => socket,
        Err(error) => return status_payload(Err(error)),
    };
    let reply = spawn_blocking_ipc(move || {
        ipc::command_with_argument(&socket, command, target, literal_path, ipc::DEFAULT_TIMEOUT)
    })
    .await;
    status_payload_remembering(&app.state(), reply)
}

#[tauri::command]
pub async fn approve(app: tauri::AppHandle, target: String, literal_path: bool) -> StatusPayload {
    approval_round_trip(app, ControlCommand::Approve, target, literal_path).await
}

#[tauri::command]
pub async fn deny(app: tauri::AppHandle, target: String, literal_path: bool) -> StatusPayload {
    approval_round_trip(app, ControlCommand::Deny, target, literal_path).await
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
    })
}

/// Partial config update: only `Some` fields are written; everything else (comments, daemon-only
/// keys) is preserved by the edit-in-place writer. Rejected if the daemon parser would refuse it.
#[derive(serde::Deserialize)]
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
    doc.save(&path).map_err(|e| e.to_string())?;
    // Re-resolve in case local_root / socket / db changed, but keep the daemon-reported live
    // config — the daemon is still running with it until restarted. (Saving still requires a
    // daemon restart to take effect — the frontend prompts for that.)
    let mut paths = state.lock().unwrap();
    let mut resolved = RuntimePaths::resolve();
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
}

/// Async so the full-tree dry run — a `proton-syncd --dry-run` subprocess that can take many
/// seconds against a large remote — never blocks the GTK main loop. Running it synchronously would
/// stall the webview's URI-scheme handler thread (the GTK main loop) until WebKit aborts the whole
/// process; here the blocking work runs on a runtime blocking thread instead. (See
/// `restart_service` for the same pattern.)
#[tauri::command]
pub async fn run_dry_run(state: Paths<'_>) -> Result<DryRunPayload, String> {
    let (config_path, file_local, file_remote, file_db, daemon_local, daemon_remote, daemon_db) = {
        let paths = state.lock().unwrap();
        (
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

/// The blocking half of `run_dry_run`: build and run `proton-syncd --dry-run`, then parse its
/// stdout. Kept as a free function so the mutex guard from `run_dry_run` never crosses the `.await`.
fn run_dry_run_impl(
    config_path: std::path::PathBuf,
    file_local: Option<std::path::PathBuf>,
    file_remote: Option<std::path::PathBuf>,
    file_db: Option<std::path::PathBuf>,
    daemon_local: Option<std::path::PathBuf>,
    daemon_remote: Option<std::path::PathBuf>,
    daemon_db: Option<std::path::PathBuf>,
) -> Result<DryRunPayload, String> {
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
    let files_at_risk = plan::files_at_risk(&report)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    Ok(DryRunPayload {
        requires_delete_gate: plan::requires_delete_gate(&report),
        files_at_risk,
        report,
    })
}

/// Async so the remote-listing subprocess (`proton-drive filesystem list`, which walks the remote
/// tree and can be slow) never blocks the GTK main loop.
#[tauri::command]
pub async fn list_remote(state: Paths<'_>, path: Option<String>) -> Result<String, String> {
    let (proton_cli, remote_root) = {
        let paths = state.lock().unwrap();
        (paths.proton_cli.clone(), paths.effective_remote_root())
    };
    tauri::async_runtime::spawn_blocking(move || {
        let target = path
            .or_else(|| remote_root.map(|r| r.display().to_string()))
            .ok_or_else(|| "no remote path given and remote_root is not configured".to_string())?;
        let output = Command::new(&proton_cli)
            .arg("filesystem")
            .arg("list")
            .arg("--json")
            .arg(&target)
            .output()
            .map_err(|e| format!("failed to launch {proton_cli}: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "{proton_cli} list failed: {}",
                strip_ansi(String::from_utf8_lossy(&output.stderr).trim())
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    })
    .await
    .map_err(|error| format!("remote-list task failed: {error}"))?
}

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

/// Per-path index status for the file-manager emblems (S10). Built from the engine's `FileRecord`
/// (which is not itself `Serialize`). `sync_status` is one of `synced` / `modified` / `conflict`;
/// "syncing" / "paused" / "excluded" are derived by the frontend from live state + globs.
#[derive(serde::Serialize)]
pub struct EmblemStatus {
    tracked: bool,
    sync_status: Option<String>,
    entity_kind: Option<String>,
    file_size: Option<u64>,
    mtime: Option<i64>,
    proton_id: Option<String>,
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
        }
    }
}

impl From<gui_core::wire::FileRecord> for EmblemStatus {
    fn from(record: gui_core::wire::FileRecord) -> Self {
        Self {
            tracked: true,
            sync_status: Some(record.sync_status.as_str().to_string()),
            entity_kind: Some(record.entity_kind.as_str().to_string()),
            file_size: Some(record.file_size),
            mtime: Some(record.mtime),
            proton_id: record.proton_id,
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
    let record = index_read::record_for_path(&connection, std::path::Path::new(&relative_path))?;
    Ok(record
        .map(EmblemStatus::from)
        .unwrap_or_else(EmblemStatus::untracked))
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
        Ok(FileSearch {
            matches: found
                .into_iter()
                .map(|m| FileMatch {
                    path: m.path.to_string_lossy().into_owned(),
                    status: EmblemStatus::from(m.record),
                })
                .collect(),
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

/// Restart the sync daemon so a saved config change takes effect. Works no matter how the daemon
/// was launched: ask it to exit gracefully over IPC (its `shutdown` control command), wait for
/// the control socket to go quiet, then start it again through the shared start logic (systemd
/// unit first, direct spawn against the GUI config as fallback). The unit ships
/// `Restart=on-failure`, so systemd does not race us by respawning the clean exit itself.
pub(crate) fn restart_service_impl(
    config_path: &std::path::Path,
    socket_path: &std::path::Path,
) -> Result<String, String> {
    use std::time::{Duration, Instant};

    let was_running =
        ipc::command(socket_path, ControlCommand::Status, ipc::DEFAULT_TIMEOUT).is_ok();
    if was_running {
        // Best-effort: if the shutdown call itself errors the daemon may already be exiting;
        // the socket probe below is the authoritative "has it stopped" signal.
        let _ = ipc::command(socket_path, ControlCommand::Shutdown, ipc::DEFAULT_TIMEOUT);
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if ipc::command(socket_path, ControlCommand::Status, Duration::from_secs(1)).is_err() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(
                    "the daemon did not stop within 8s — restart it manually with: \
                     systemctl --user restart proton-syncd"
                        .to_string(),
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    start_service_impl(config_path).map(|detail| {
        if was_running {
            format!("daemon restarted ({detail})")
        } else {
            format!("daemon was not running; started it ({detail})")
        }
    })
}

/// Async so the up-to-~10s stop/start sequence never runs on the UI thread; the blocking work
/// itself happens on a runtime blocking thread.
#[tauri::command]
pub async fn restart_service(state: Paths<'_>) -> Result<String, String> {
    let (config_path, socket_path) = {
        let paths = state.lock().unwrap();
        (paths.config_path.clone(), paths.socket_path.clone()?)
    };
    tauri::async_runtime::spawn_blocking(move || restart_service_impl(&config_path, &socket_path))
        .await
        .map_err(|error| format!("restart task failed: {error}"))?
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
    let (local_root, db_path) = {
        let paths = app.state::<Mutex<RuntimePaths>>();
        let paths = paths.lock().unwrap();
        (paths.effective_local_root(), paths.effective_db_path())
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
/// Async: the local side walks a tree and the remote side spawns subprocesses, and either on the
/// GTK main loop would freeze the window.
#[tauri::command]
pub async fn probe_folder(
    app: tauri::AppHandle,
    side: String,
    path: String,
) -> Result<gui_core::folder_probe::FolderProbe, String> {
    let proton_cli = {
        let paths = app.state::<Mutex<RuntimePaths>>();
        let cli = paths.lock().unwrap().proton_cli.clone();
        cli
    };
    tauri::async_runtime::spawn_blocking(move || match side.as_str() {
        "local" => gui_core::folder_probe::probe_local(std::path::Path::new(&path)),
        "remote" => {
            gui_core::folder_probe::probe_remote_via_cli(&proton_cli, std::path::Path::new(&path))
        }
        other => Err(format!("unknown side: {other}")),
    })
    .await
    .map_err(|error| format!("folder probe failed: {error}"))?
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
    use super::{daemon_ignored_paths, probe_cli, relative_query, strip_ansi};
    use std::path::Path;

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
        assert!(
            matches!(reply, Err(ipc::IpcError::Unreachable(_))),
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
        ));
        assert!(
            payload.error.is_none(),
            "unexpected error: {:?}",
            payload.error
        );
        assert_ne!(payload.state, gui_core::DaemonState::Unreachable);
    }
}
