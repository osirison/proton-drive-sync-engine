//! The **fixed** Tauri command surface. Every command is a thin wrapper over `gui-core`; screens
//! are frontend-only and never add to this list, `generate_handler!`, or `Cargo.toml`. That is what
//! keeps the parallel screen tasks (S1–S11) collision-free.

use crate::config_path::RuntimePaths;
use gui_core::conflicts::{self, Conflict, Resolution};
use gui_core::state::derive_state;
use gui_core::wire::{ControlCommand, ControlResponse, DryRunReport, PendingDeletion};
use gui_core::{config_io, index_read, ipc, plan};
use std::process::Command;
use std::sync::Mutex;
use tauri::State;
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

#[tauri::command]
pub fn get_status(state: Paths) -> StatusPayload {
    let reply = ipc::command(
        &socket_path(&state),
        ControlCommand::Status,
        ipc::DEFAULT_TIMEOUT,
    );
    status_payload_remembering(&state, reply)
}

#[tauri::command]
pub fn pause(state: Paths) -> StatusPayload {
    let reply = ipc::command(
        &socket_path(&state),
        ControlCommand::Pause,
        ipc::DEFAULT_TIMEOUT,
    );
    status_payload_remembering(&state, reply)
}

#[tauri::command]
pub fn resume(state: Paths) -> StatusPayload {
    let reply = ipc::command(
        &socket_path(&state),
        ControlCommand::Resume,
        ipc::DEFAULT_TIMEOUT,
    );
    status_payload_remembering(&state, reply)
}

#[tauri::command]
pub fn sync_now(state: Paths) -> StatusPayload {
    let reply = ipc::command(
        &socket_path(&state),
        ControlCommand::Syncnow,
        ipc::DEFAULT_TIMEOUT,
    );
    status_payload_remembering(&state, reply)
}

#[tauri::command]
pub fn approve(state: Paths, target: String, literal_path: bool) -> StatusPayload {
    let reply = ipc::command_with_argument(
        &socket_path(&state),
        ControlCommand::Approve,
        target,
        literal_path,
        ipc::DEFAULT_TIMEOUT,
    );
    status_payload_remembering(&state, reply)
}

#[tauri::command]
pub fn deny(state: Paths, target: String, literal_path: bool) -> StatusPayload {
    let reply = ipc::command_with_argument(
        &socket_path(&state),
        ControlCommand::Deny,
        target,
        literal_path,
        ipc::DEFAULT_TIMEOUT,
    );
    status_payload_remembering(&state, reply)
}

#[tauri::command]
pub fn list_pending_deletions(state: Paths) -> Result<Vec<PendingDeletion>, String> {
    ipc::command(
        &socket_path(&state),
        ControlCommand::Status,
        ipc::DEFAULT_TIMEOUT,
    )
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
/// seconds against a large remote — never blocks the GTK main loop. Run synchronously on the
/// webview's URI-scheme handler thread it hangs (then aborts) the whole process; here the blocking
/// work runs on a runtime blocking thread instead. (See `restart_service` for the same pattern.)
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

#[cfg(test)]
mod tests {
    use super::strip_ansi;

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
