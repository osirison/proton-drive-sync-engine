//! Tauri shell entrypoint. The shell owns no sync logic; it wires the fixed command surface
//! (thin wrappers over `gui-core`) to the webview and manages the resolved runtime paths.

mod commands;
mod config_path;
// Not Linux-gated even though its whole body is: every caller wants "raise this window" and none of
// them wants to know which display server is under it.
mod focus;
mod notify;
mod panel;
mod tray;
// The rows every tray menu draws, in one table. Not Linux-gated: the fallback tray is built on every
// platform, and the table is what stops the three surfaces drawing three menus.
mod tray_menu;

// The tray item, its menu and its glyph theme are the D-Bus half of S8 and exist on Linux alone —
// the only platform this app targets today (the engine's IPC is Unix-socket only), but the split
// keeps the `cfg` at the module boundary instead of scattered through `tray.rs`.
#[cfg(target_os = "linux")]
mod dbusmenu;
#[cfg(target_os = "linux")]
mod icons;
#[cfg(target_os = "linux")]
mod sni;

use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            commands::keep,
            commands::list_pending_deletions,
            commands::read_config,
            commands::write_config,
            commands::choose_folder,
            commands::run_dry_run,
            commands::apply_plan,
            commands::list_remote,
            commands::scan_conflicts,
            commands::resolve_conflict,
            commands::read_conflict_pair,
            commands::path_sync_status,
            commands::search_files,
            commands::start_service,
            commands::restart_service,
            commands::send_notification,
            commands::close_notification,
            commands::read_notify_policy,
            commands::write_notify_policy,
            // The Phase-1 capability commands (C2/C4/C5) — data the design assumes and the daemon
            // does not expose. Added by the C-tasks, not by the screens that consume them.
            commands::free_space,
            commands::check_cli,
            commands::skip_rule_usage,
            commands::probe_folder,
            // The openers (#220/#231): the one capability behind `Open both in an editor`,
            // `Open folder`, `Open on Proton Drive` and `Open the system log`. No plugin and no
            // capability grant — these are app-defined commands shelling `xdg-open`, so the webview
            // gets exactly these four doors out and no general "open anything" permission.
            commands::open_paths,
            commands::open_folder,
            commands::open_remote,
            commands::open_system_log,
            // F4's keyboard map: Ctrl W and Ctrl Q. Same two paths the tray menu already offers.
            commands::close_window,
            commands::quit_app,
            // The tray panel (S8): its rows, and the two things only the webview knows — how tall it
            // came out, and when Esc was pressed.
            commands::tray_action,
            commands::resize_tray_panel,
            commands::hide_tray_panel,
        ])
        .setup(|app| {
            // Both are Linux-only types, and `#[cfg]` binds to ONE statement — the notifier's line
            // was outside the attribute above it and would have failed to compile off Linux.
            #[cfg(target_os = "linux")]
            {
                app.manage::<sni::SniState>(std::sync::Arc::new(tokio::sync::Mutex::new(None)));
                // The notification connection is made on first send (S9), so this starts empty.
                app.manage::<notify::NotifierState>(std::sync::Arc::new(tokio::sync::Mutex::new(
                    None,
                )));
            }
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // Closing the MAIN window hides it to the tray rather than exiting, so the indicator
                // survives while syncing continues. A real exit is the tray's `Quit`, which since S8
                // also stops the daemon — that is what its `stops syncing` sub-label promises, and
                // `Close window · keeps syncing` is this path.
                tauri::WindowEvent::CloseRequested { api, .. }
                    if window.label() != panel::LABEL =>
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // THE PANEL GOES AWAY WHEN YOU LOOK AWAY, and this is the half of that which cannot
                // live in the webview: a click on another window never reaches our DOM.
                //
                // It is also why the panel TAKES focus when the indicator is clicked. The plan's two
                // sub-risks — "must not steal focus" and "must not linger after blur" — are the same
                // sentence twice if read literally: a window that never focuses never blurs, so it
                // could only be dismissed by clicking it, which is the lingering the other half
                // forbids. Taking focus from an explicit click is not stealing it; the thing being
                // ruled out is a panel that raises itself over someone's work unbidden, and nothing
                // here opens except on `Activate`.
                //
                // A BLUR ONLY MEANS SOMETHING IF THE WINDOW EVER HAD FOCUS, and the first version
                // did not check: driving `Activate` over the bus showed the panel and hid it again
                // in the same breath, because the compositor's focus-stealing prevention handed
                // focus straight back to whatever had it and the resulting `Focused(false)` was
                // indistinguishable from the user clicking away. The panel existed, unmapped,
                // looking exactly like a panel that had never opened.
                //
                // So `hide` waits for a blur that follows a focus. When the compositor never grants
                // focus at all the panel stays up, which is the right way round to be wrong: Esc,
                // any menu row, and a second click on the indicator all still dismiss it.
                tauri::WindowEvent::Focused(focused) if window.label() == panel::LABEL => {
                    if *focused {
                        panel::mark_focused();
                    } else if panel::take_focused() {
                        let _ = window.hide();
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running proton-sync-gui");
}
