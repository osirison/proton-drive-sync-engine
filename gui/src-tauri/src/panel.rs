//! The tray panel window (S8) — the 360px compact panel, floating over the desktop.
//!
//! `10-tray.md` replaces the tray's text menu with the panel from the main screen: same hexagon,
//! same seam, same sentence, with the menu rows below it. libayatana-appindicator cannot draw any of
//! that, so the panel is a real webview window — borderless, above everything, and gone the moment
//! you look away.
//!
//! # "Must not steal focus" and "must not linger after blur" are the same sentence twice
//!
//! `IMPLEMENTATION-PLAN.md` §6 lists both as sub-risks, and taken literally they contradict: a
//! window that never takes focus never receives a blur, so a panel that refuses focus can only be
//! dismissed by clicking it, which is exactly the lingering the other half forbids. Every desktop
//! popover resolves this the same way and so does this one:
//!
//!   * the panel takes focus WHEN THE USER CLICKS THE INDICATOR. That is not stealing — it is the
//!     click asking for it. The prohibition is about a panel that raises itself over someone's work
//!     because a sync finished, which nothing here does: it opens on `Activate` and nothing else.
//!   * losing focus hides it. So does Esc, and so does clicking the indicator again.
//!
//! # Position
//!
//! `Activate(x, y)` carries the click in screen coordinates — verified on a live session, where a
//! click on the indicator arrived as `Activate(3192, 2112)`. So the spec's `top:40px; right:16px`
//! fallback is only that: a fallback, for a host that sends `(0, 0)` because it does not track the
//! pointer. The panel is placed against the click and then clamped into the work area, which is what
//! makes it open UPWARD on a bottom panel — the ordinary case on KDE, and the one a fixed
//! top-right rule gets wrong on every Plasma desktop.

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "tray-panel";

/// 362, not 360. The panel does not opt into `border-box` and `base.css` opts the app in globally,
/// so a panel written at its nominal width comes out 2px narrower than every frame it is compared
/// against. DEVIATIONS §19/§48 — F6 writes the drawn number and so does the window around it.
const WIDTH: f64 = 362.0;

/// The tallest state the frames draw (`10a In situ`'s needs-you panel with the menu, 441.5). The
/// window opens at this and the webview corrects it on first paint via `resize`, because Phase 1
/// omits lines the frames draw — the offline panel has no `retrying in 40s` — and a window sized to
/// the drawing would carry that much empty space below the menu.
const HEIGHT: f64 = 442.0;

/// The spec's fallback corner, for a host that sends no usable coordinates.
const FALLBACK_INSET: (f64, f64) = (16.0, 40.0);

/// Show the panel at a click, or hide it if it is already up.
///
/// Called from the D-Bus task, which is not the GTK main loop. Tauri's window operations must run
/// there, so everything is hopped explicitly — a window call from the wrong thread on GTK is
/// undefined behaviour that usually looks like nothing happening.
pub fn toggle(app: &AppHandle, at: Option<(i32, i32)>) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let Some(window) = app.get_webview_window(LABEL) {
            // Visible means the click was a dismiss. Checking `is_visible` rather than tracking our
            // own flag: the window also hides itself on blur and on Esc, and a second source of
            // truth about whether it is up would disagree with the compositor sooner or later.
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
                return;
            }
            place(&window, at);
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }
        match build(&app) {
            Ok(window) => {
                place(&window, at);
                let _ = window.show();
                let _ = window.set_focus();
            }
            Err(error) => eprintln!("tray: cannot open the panel window: {error}"),
        }
    });
}

/// Hide it, from anywhere.
pub fn hide(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.hide();
        }
    });
}

/// The webview sizes itself once it knows what state it is in; this is that report.
pub fn resize(app: &AppHandle, height: f64) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.set_size(LogicalSize::new(WIDTH, height.max(1.0)));
        }
    });
}

fn build(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    // `?surface=tray` rather than a second HTML file. `index.html`'s own comment warns that its
    // stylesheet chain is easy to forget one link of, and a second copy of it is a blank panel with
    // no error the day someone adds a stylesheet to one and not the other. app.js reads the
    // parameter and mounts the panel instead of the shell — the same gate the frame preview uses,
    // which also means `?surface=tray` opens the panel in a browser.
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?surface=tray".into()))
        .title("Proton Drive Sync")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        // Out of the taskbar and the window switcher: it is a popover, and one that answers Alt-Tab
        // is a window the user has to dismiss twice.
        .skip_taskbar(true)
        .shadow(false)
        // Built hidden and shown by the caller once it is positioned. Building it visible paints it
        // at the default position first, so it visibly jumps to the indicator.
        .visible(false)
        .build()
}

/// Put the panel under the click, inside the screen.
///
/// The clamp is what makes this right on a bottom panel: a click at y=2112 on a 2160-tall screen has
/// no room for a 442px panel below it, so the panel goes above the click instead. That is the
/// ordinary KDE case, and the spec's fixed `top:40px` would have put it at the wrong end of the
/// screen on every one of them.
fn place(window: &tauri::WebviewWindow, at: Option<(i32, i32)>) {
    let Ok(monitor) = window.current_monitor() else {
        return;
    };
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let area = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);
    let size = window
        .outer_size()
        .map(|s| s.to_logical::<f64>(scale))
        .unwrap_or(LogicalSize::new(WIDTH, HEIGHT));
    let margin = 8.0;

    // `(0, 0)` is how a host says it does not know where the pointer was — GNOME's extension is the
    // documented case. It is also a legitimate corner click, and treating a real corner click as
    // "unknown" costs nothing (the fallback corner is a few pixels away); treating "unknown" as a
    // real corner click puts the panel in the opposite corner of the screen.
    let click = at.filter(|&(x, y)| x != 0 || y != 0);
    let (x, y) = match click {
        Some((cx, cy)) => {
            let cx = cx as f64;
            let cy = cy as f64;
            // Centred on the click horizontally, and above or below it depending on which half of
            // the screen the indicator is in — which is the same thing as asking whether the panel
            // is at the top or the bottom of the desktop, without having to know that.
            let below = cy - origin.y < area.height / 2.0;
            (
                cx - size.width / 2.0,
                if below { cy + margin } else { cy - size.height - margin },
            )
        }
        None => (
            origin.x + area.width - size.width - FALLBACK_INSET.0,
            origin.y + FALLBACK_INSET.1,
        ),
    };

    let x = x.clamp(origin.x + margin, origin.x + area.width - size.width - margin);
    let y = y.clamp(origin.y + margin, origin.y + area.height - size.height - margin);
    let _ = window.set_position(LogicalPosition::new(x, y));
}
