//! Tauri shell entrypoint. The shell owns no sync logic; it wires the fixed command surface
//! (thin wrappers over `gui-core`) to the webview and manages the resolved runtime paths.

mod commands;
mod config_path;
mod tray;

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
            commands::read_conflict_pair,
            commands::path_sync_status,
            commands::notify,
        ])
        .setup(|app| {
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it to the tray rather than exiting — the tray's "Quit" is the
            // real exit. (The daemon is a separate process and is unaffected either way.)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running proton-sync-gui");
}
