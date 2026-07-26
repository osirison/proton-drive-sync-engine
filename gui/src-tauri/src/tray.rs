//! System-tray indicator (S7, #90… issue #88). A backend status poll drives the tray so it stays
//! live even when the window is hidden. Five glyph states (design §3.7): up to date, syncing,
//! paused, needs attention, daemon unreachable. When unreachable the menu collapses to
//! Start service / View journal / Settings and shows **no** stale counters.

use crate::config_path::RuntimePaths;
use gui_core::ipc;
use gui_core::state::{derive_state, DaemonState};
use gui_core::wire::{ControlCommand, ControlResponse};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

const TRAY_ID: &str = "proton-sync-tray";
const POLL_INTERVAL: Duration = Duration::from_secs(5);

fn icon_for(state: DaemonState) -> tauri::image::Image<'static> {
    match state {
        DaemonState::Running => tauri::include_image!("icons/tray/syncing.png"),
        DaemonState::Idle => tauri::include_image!("icons/tray/uptodate.png"),
        DaemonState::Paused => tauri::include_image!("icons/tray/paused.png"),
        DaemonState::AuthExpired | DaemonState::FirstRun => {
            tauri::include_image!("icons/tray/attention.png")
        }
        DaemonState::Unreachable => tauri::include_image!("icons/tray/unreachable.png"),
    }
}

fn tooltip_for(state: DaemonState, response: Option<&ControlResponse>) -> String {
    match state {
        DaemonState::Running => {
            let pending = response.map(|r| r.pending_changes).unwrap_or(0);
            format!("Proton Drive Sync — syncing ({pending} pending)")
        }
        DaemonState::Idle => "Proton Drive Sync — up to date".into(),
        DaemonState::Paused => "Proton Drive Sync — paused".into(),
        DaemonState::AuthExpired => "Proton Drive Sync — sign-in expired".into(),
        DaemonState::FirstRun => "Proton Drive Sync — nothing synced yet".into(),
        DaemonState::Unreachable => "Proton Drive Sync — daemon unreachable".into(),
    }
}

/// Build the tray menu for the current state. Reachable states get the full action menu; when the
/// daemon is unreachable it collapses to Start service / View journal / Settings — never counters.
fn build_menu(
    app: &AppHandle,
    state: DaemonState,
    response: Option<&ControlResponse>,
) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    // "Quit" closes the window only and leaves the tray running (design §3.7 / #88): the indicator
    // must survive so the user doesn't lose visibility while syncing continues.
    let quit = MenuItem::with_id(
        app,
        "quit",
        "Close window (keeps syncing in the tray)",
        true,
        None::<&str>,
    )?;

    if state == DaemonState::FirstRun {
        // First run has no meaningful counts yet — show a disabled note instead of "0 pending" /
        // "Resolve 0 conflicts", which would read as real (zero) counters (the em-dash convention).
        let info = MenuItem::with_id(
            app,
            "info_first_run",
            "Nothing synced yet",
            false,
            None::<&str>,
        )?;
        return Menu::with_items(app, &[&show, &info, &settings, &sep, &quit]);
    }

    if state == DaemonState::Unreachable {
        let start = MenuItem::with_id(
            app,
            "start_service",
            "Start proton-syncd",
            true,
            None::<&str>,
        )?;
        let journal = MenuItem::with_id(app, "journal", "View journal", true, None::<&str>)?;
        return Menu::with_items(app, &[&show, &start, &journal, &settings, &sep, &quit]);
    }

    let pending = response.map(|r| r.pending_changes).unwrap_or(0);
    let conflicts = response
        .and_then(|r| r.last_plan_summary.as_ref())
        .map(|s| s.conflicts)
        .unwrap_or(0);
    let paused = response.map(|r| r.paused).unwrap_or(false);

    let sync_now = MenuItem::with_id(
        app,
        "sync_now",
        format!("Sync now ({pending} pending)"),
        !paused,
        None::<&str>,
    )?;
    let pause_resume = if paused {
        MenuItem::with_id(app, "resume", "Resume", true, None::<&str>)?
    } else {
        MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?
    };
    let conflicts_item = MenuItem::with_id(
        app,
        "conflicts",
        format!(
            "Resolve {conflicts} conflict{}",
            if conflicts == 1 { "" } else { "s" }
        ),
        conflicts > 0,
        None::<&str>,
    )?;

    Menu::with_items(
        app,
        &[
            &show,
            &sync_now,
            &pause_resume,
            &conflicts_item,
            &settings,
            &sep,
            &quit,
        ],
    )
}

/// Install the tray icon + menu and start the background poll that keeps it current.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    // Initial state: unknown until the first poll — show the unreachable glyph + collapsed menu.
    let menu = build_menu(app, DaemonState::Unreachable, None)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon_for(DaemonState::Unreachable))
        .tooltip("Proton Drive Sync")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            // Left click toggles the window into view.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    spawn_poll(app.clone());
    Ok(())
}

fn spawn_poll(app: AppHandle) {
    std::thread::spawn(move || loop {
        let socket = {
            let state = app.state::<Mutex<RuntimePaths>>();
            let guard = state.lock().unwrap();
            guard.socket_path.clone()
        };
        let reply = ipc::command(&socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT);
        let daemon_state = derive_state(reply.as_ref());
        let response = reply.ok();
        update_tray(&app, daemon_state, response.as_ref());
        std::thread::sleep(POLL_INTERVAL);
    });
}

fn update_tray(app: &AppHandle, state: DaemonState, response: Option<&ControlResponse>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let _ = tray.set_icon(Some(icon_for(state)));
    let _ = tray.set_tooltip(Some(tooltip_for(state, response)));
    if let Ok(menu) = build_menu(app, state, response) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "show" => show_window(app),
        "settings" => navigate(app, "settings"),
        "conflicts" => navigate(app, "conflicts"),
        "sync_now" => send_command(app, ControlCommand::Syncnow),
        "pause" => send_command(app, ControlCommand::Pause),
        "resume" => send_command(app, ControlCommand::Resume),
        "start_service" => {
            // Best-effort; the user's session manager owns the unit. Failure is surfaced only in
            // the window's own "daemon unreachable" state, which the next poll will refresh.
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "start", "proton-syncd"])
                .spawn();
        }
        "journal" => navigate(app, "history"), // the History screen surfaces the journalctl command
        // "Quit" closes the window only — the tray (and this GUI process) keep running so the
        // indicator survives, and the daemon is a separate process either way (design §3.7 / #88).
        "quit" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        _ => {}
    }
}

/// Show the window and ask the frontend to switch tabs (the shell listens for `tray-navigate`).
fn navigate(app: &AppHandle, tab: &str) {
    show_window(app);
    let _ = app.emit("tray-navigate", tab);
}

fn send_command(app: &AppHandle, command: ControlCommand) {
    // Menu events run on the UI/event-loop thread and the control socket is synchronous (blocking up
    // to DEFAULT_TIMEOUT), so do the round trip on a background thread and refresh the tray after —
    // a slow/unreachable daemon must never freeze tray interaction.
    let app = app.clone();
    std::thread::spawn(move || {
        let socket = {
            let state = app.state::<Mutex<RuntimePaths>>();
            let guard = state.lock().unwrap();
            guard.socket_path.clone()
        };
        let _ = ipc::command(&socket, command, ipc::DEFAULT_TIMEOUT);
        // Refresh the tray promptly rather than waiting for the next poll tick.
        let reply = ipc::command(&socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT);
        let state = derive_state(reply.as_ref());
        update_tray(&app, state, reply.ok().as_ref());
    });
}
