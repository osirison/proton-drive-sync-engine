//! Tauri shell entrypoint. The shell owns no sync logic; it wires the fixed command surface
//! (thin wrappers over `gui-core`) to the webview and manages the resolved runtime paths.

mod commands;
mod config_path;

use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(Mutex::new(config_path::RuntimePaths::resolve()))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::pause,
            commands::resume,
            commands::sync_now,
            commands::approve,
            commands::deny,
            commands::list_pending_deletions,
            commands::read_config,
            commands::write_config,
            commands::run_dry_run,
            commands::list_remote,
            commands::scan_conflicts,
            commands::resolve_conflict,
            commands::path_sync_status,
            commands::notify,
        ])
        .run(tauri::generate_context!())
        .expect("error while running proton-sync-gui");
}
