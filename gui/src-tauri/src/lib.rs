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
        // Settings › Folders' `Choose…`. Registered for the Rust side only: `commands::choose_folder`
        // calls it, and an app-defined command needs no capability grant — so the webview never gets
        // a file dialog it could open on its own.
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(config_path::RuntimePaths::resolve()))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::pause,
            commands::resume,
            commands::sync_now,
            commands::resync,
            commands::approve,
            commands::deny,
            commands::list_pending_deletions,
            commands::read_config,
            commands::write_config,
            commands::choose_folder,
            commands::run_dry_run,
            commands::list_remote,
            commands::scan_conflicts,
            commands::resolve_conflict,
            commands::read_conflict_pair,
            commands::path_sync_status,
            commands::start_service,
            commands::restart_service,
            commands::notify,
            // The Phase-1 capability commands (C2/C4/C5) — data the design assumes and the daemon
            // does not expose. Added by the C-tasks, not by the screens that consume them.
            commands::free_space,
            commands::check_cli,
            commands::skip_rule_usage,
            // F4's keyboard map: Ctrl W and Ctrl Q. Same two paths the tray menu already offers.
            commands::close_window,
            commands::quit_app,
        ])
        .setup(|app| {
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it to the tray rather than exiting, so the indicator
            // survives while syncing continues. A real exit lives in the tray menu ("Quit Proton
            // Drive Sync"); the daemon is a separate process and is unaffected either way.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running proton-sync-gui");
}
