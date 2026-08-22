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

/// Make the window's identity the launcher's, so the desktop can find its icon.
///
/// GTK3 takes the Wayland `app_id` from `g_get_prgname()` (argv[0] — `proton-sync-gui`) and the X11
/// `WM_CLASS` from `(prgname, program class)`. Neither matched
/// `app.protondrivesync.engine.desktop`, so KWin drew `QIcon::fromTheme("wayland")` instead.
///
/// Must precede `Builder::run`: `gtk_init` fills an unset prgname from argv[0], the app_id is
/// stamped at toplevel creation, and the config's windows are built before the `setup` hook.
///
/// `gtk::init` only satisfies gdk-rs's initialized-GDK assert (the C call needs none). It is tao's
/// own first act and idempotent; no display leaves the X11 half unset and tao reports as before.
#[cfg(target_os = "linux")]
fn adopt_launcher_identity(identifier: &str) {
    gtk::glib::set_prgname(Some(identifier));
    if gtk::init().is_ok() {
        gtk::gdk::set_program_class(identifier);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Not a literal: this is `tauri.conf.json`'s `identifier`, and a second spelling here is a
    // second thing to keep in step with the launcher.
    let context = tauri::generate_context!();
    #[cfg(target_os = "linux")]
    adopt_launcher_identity(&context.config().identifier);

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
        .run(context)
        .expect("error while running proton-sync-gui");
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn repo_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn identifier() -> String {
        let config = std::fs::read_to_string(repo_path("gui/src-tauri/tauri.conf.json"))
            .expect("tauri.conf.json");
        let config: serde_json::Value = serde_json::from_str(&config).expect("valid JSON");
        config["identifier"]
            .as_str()
            .expect("identifier is a string")
            .to_owned()
    }

    /// `adopt_launcher_identity` makes the runtime app_id the config's `identifier`, which only
    /// resolves to an icon while a launcher of that exact name declares it. Nothing at build time
    /// links the two, so the drift is silent: the window falls back to breeze's `wayland` glyph and
    /// every gate stays green.
    #[test]
    fn the_launcher_is_named_for_the_identifier_the_window_announces() {
        let identifier = identifier();

        for launcher in [
            repo_path(&format!("packaging/freedesktop/{identifier}.desktop")),
            // setup.sh writes its own copy, with an absolute `Exec`; the identity lines must match.
            repo_path("setup.sh"),
        ] {
            let text = std::fs::read_to_string(&launcher)
                .unwrap_or_else(|e| panic!("{}: {e}", launcher.display()));
            for line in [
                format!("Icon={identifier}"),
                format!("StartupWMClass={identifier}"),
            ] {
                assert!(
                    text.contains(&line),
                    "{} does not carry `{line}`",
                    launcher.display()
                );
            }
        }

        // setup.sh's install path, and uninstall.sh's removal path, name the file itself.
        for script in ["setup.sh", "upgrade.sh", "uninstall.sh"] {
            let text = std::fs::read_to_string(repo_path(script)).expect(script);
            assert!(
                text.contains(&format!("{identifier}.desktop")),
                "{script} does not name `{identifier}.desktop`"
            );
        }
    }

    /// The `desktop-entry` hint is the same fact as the app_id: it tells the notification server
    /// which launcher the banner belongs to.
    #[test]
    fn the_notification_desktop_entry_hint_is_that_same_identifier() {
        let source =
            std::fs::read_to_string(repo_path("gui/src-tauri/src/notify.rs")).expect("notify.rs");
        assert!(
            source.contains(&format!("\"{}\"", identifier())),
            "notify.rs's `desktop-entry` hint has drifted from tauri.conf.json's identifier"
        );
    }
}
