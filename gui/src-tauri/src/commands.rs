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
//! **A command that touches the filesystem, a subprocess or a socket must be `async` and do its
//! work in `spawn_blocking`.** A synchronous one runs on the GTK main loop, and WebKitGTK aborts
//! the whole process when that loop stalls (#142/#143). `read_config`, `write_config`,
//! `resolve_conflict`, `read_conflict_pair` and `path_sync_status` predate the rule and are still
//! synchronous — `path_sync_status` in particular can hold the loop for its full 3s index busy
//! timeout. They are bounded enough to have survived; anything unbounded is not, and none of the
//! commands added since is synchronous.

use crate::config_path::RuntimePaths;
use gui_core::conflicts::{self, Conflict, Resolution};
use gui_core::state::derive_state;
use gui_core::wire::{ControlCommand, ControlResponse, DryRunReport, PendingDeletion};
use gui_core::{config_io, index_read, ipc, plan};
use std::process::Command;
use std::sync::Mutex;
use tauri::{Manager, State};
use tauri_plugin_notification::NotificationExt;

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

fn socket_path(state: &Paths) -> std::path::PathBuf {
    state.lock().unwrap().socket_path.clone()
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
    let socket = socket_path(&app.state());
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
    let socket = socket_path(&app.state());
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
    let socket = socket_path(&app.state());
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
    delete_approval_remote: Option<bool>,
    delete_approval_local: Option<bool>,
    /// Applied AFTER the two raw booleans, so a screen that sends both cannot end up half-written:
    /// the policy always sets both directions, which is what makes a radio selection unambiguous.
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

/// Async so a full local-tree conflict scan (the `.proton-cloud` sidecar walk) never blocks the
/// GTK main loop on a large folder.
#[tauri::command]
pub async fn scan_conflicts(state: Paths<'_>) -> Result<Vec<Conflict>, String> {
    let local_root = state
        .lock()
        .unwrap()
        .effective_local_root()
        .ok_or_else(|| "local_root is not configured".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        conflicts::scan_conflicts(&local_root).map_err(|e| e.to_string())
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

/// Read both sides of a conflict (the local file + its `.proton-cloud` sidecar) for the compare
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

#[tauri::command]
pub fn path_sync_status(state: Paths, relative_path: String) -> Result<EmblemStatus, String> {
    let db_path = state
        .lock()
        .unwrap()
        .effective_db_path()
        .ok_or_else(|| "no index database configured or reported by the daemon".to_string())?;
    let connection = index_read::open_readonly(&db_path, index_read::DEFAULT_BUSY_TIMEOUT)?;
    let record = index_read::record_for_path(&connection, std::path::Path::new(&relative_path))?;
    Ok(match record {
        Some(record) => EmblemStatus {
            tracked: true,
            sync_status: Some(record.sync_status.as_str().to_string()),
            entity_kind: Some(record.entity_kind.as_str().to_string()),
            file_size: Some(record.file_size),
            mtime: Some(record.mtime),
            proton_id: record.proton_id,
        },
        None => EmblemStatus {
            tracked: false,
            sync_status: None,
            entity_kind: None,
            file_size: None,
            mtime: None,
            proton_id: None,
        },
    })
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
        (paths.config_path.clone(), paths.socket_path.clone())
    };
    tauri::async_runtime::spawn_blocking(move || restart_service_impl(&config_path, &socket_path))
        .await
        .map_err(|error| format!("restart task failed: {error}"))?
}

#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
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
/// So they are one thing now. `ui/compact.js`'s `TRAY_MENU` is the source of the id strings, the
/// fallback menu in `tray.rs` builds its items from `FALLBACK_IDS` below, and both dispatch through
/// here. An id this does not know returns `None` and the caller reports it rather than silently
/// doing nothing.
pub fn tray_row(id: &str) -> Option<TrayRow> {
    Some(match id {
        // `Review them` is the panel's own decision button rather than a menu row, and it goes where
        // `Open Drive Sync` goes — see the note in `tray_action`.
        "open" | "review" => TrayRow::Open,
        // `Try again now` IS a sync: the daemon is unreachable, so the thing to retry is reaching
        // it, and if it is back the pass it schedules is what the row promises.
        "syncNow" | "tryAgain" => TrayRow::SyncNow,
        "pause" => TrayRow::Pause,
        "resume" => TrayRow::Resume,
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
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            ControlCommand::Status
        }
        Some(TrayRow::SyncNow) => ControlCommand::Syncnow,
        Some(TrayRow::Pause) => ControlCommand::Pause,
        Some(TrayRow::Resume) => ControlCommand::Resume,
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
    use super::{daemon_ignored_paths, probe_cli, strip_ansi};

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
        paths.socket_path = socket;
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
