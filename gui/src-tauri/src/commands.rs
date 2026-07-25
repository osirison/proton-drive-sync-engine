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

#[tauri::command]
pub fn get_status(state: Paths) -> StatusPayload {
    status_payload(ipc::command(
        &socket_path(&state),
        ControlCommand::Status,
        ipc::DEFAULT_TIMEOUT,
    ))
}

#[tauri::command]
pub fn pause(state: Paths) -> StatusPayload {
    status_payload(ipc::command(
        &socket_path(&state),
        ControlCommand::Pause,
        ipc::DEFAULT_TIMEOUT,
    ))
}

#[tauri::command]
pub fn resume(state: Paths) -> StatusPayload {
    status_payload(ipc::command(
        &socket_path(&state),
        ControlCommand::Resume,
        ipc::DEFAULT_TIMEOUT,
    ))
}

#[tauri::command]
pub fn sync_now(state: Paths) -> StatusPayload {
    status_payload(ipc::command(
        &socket_path(&state),
        ControlCommand::Syncnow,
        ipc::DEFAULT_TIMEOUT,
    ))
}

#[tauri::command]
pub fn approve(state: Paths, target: String) -> StatusPayload {
    status_payload(ipc::command_with_argument(
        &socket_path(&state),
        ControlCommand::Approve,
        target,
        ipc::DEFAULT_TIMEOUT,
    ))
}

#[tauri::command]
pub fn deny(state: Paths, target: String) -> StatusPayload {
    status_payload(ipc::command_with_argument(
        &socket_path(&state),
        ControlCommand::Deny,
        target,
        ipc::DEFAULT_TIMEOUT,
    ))
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
    // Re-resolve in case local_root / socket / db changed. (Saving still requires a daemon restart
    // to take effect — the frontend prompts for that.)
    *state.lock().unwrap() = RuntimePaths::resolve();
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

#[tauri::command]
pub fn run_dry_run(state: Paths) -> Result<DryRunPayload, String> {
    let config_path = state.lock().unwrap().config_path.clone();
    let output = Command::new("proton-syncd")
        .arg("--dry-run")
        .arg("--config")
        .arg(&config_path)
        .output()
        .map_err(|e| format!("failed to launch proton-syncd: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "proton-syncd --dry-run failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
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

#[tauri::command]
pub fn list_remote(state: Paths, path: Option<String>) -> Result<String, String> {
    let (proton_cli, remote_root) = {
        let paths = state.lock().unwrap();
        (paths.proton_cli.clone(), paths.remote_root.clone())
    };
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
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn scan_conflicts(state: Paths) -> Result<Vec<Conflict>, String> {
    let local_root = state
        .lock()
        .unwrap()
        .local_root
        .clone()
        .ok_or_else(|| "local_root is not configured".to_string())?;
    conflicts::scan_conflicts(&local_root).map_err(|e| e.to_string())
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
        .local_root
        .clone()
        .ok_or_else(|| "local_root is not configured".to_string())?;
    conflicts::apply_resolution(&local_root, &conflict, choice).map_err(|e| e.to_string())
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
    let db_path = state.lock().unwrap().db_path.clone();
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

#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}
