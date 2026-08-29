//! The tray panel window (S8) — the 360px compact panel, floating over the desktop.
//!
//! `10-tray.md` replaces the tray's text menu with the panel from the main screen: same hexagon,
//! same seam, same sentence, with the menu rows below it. libayatana-appindicator cannot draw any of
//! that, so the panel is a real webview window — borderless, meant to be above everything, and gone
//! the moment you look away.
//!
//! **"Above everything" and "under the click" are both X11-only, measured** (#351 and #370).
//! `always_on_top` and `place` reach GTK as `gtk_window_set_keep_above` and `gtk_window_move`,
//! neither of which xdg-shell has an equivalent for. What the panel gets on Wayland is a compositor-placed toplevel that KWin activates because
//! it was just mapped. The blur-to-hide contract still holds — that is what makes the panel usable
//! there at all — but every other sentence in this file about stacking or position describes X11.
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
//!
//! **All of which happens on X11 only.** A Wayland client cannot position its own toplevel: there
//! is no xdg-shell request for it, so `gtk_window_move` is discarded and KWin places the panel by
//! its own policy. Measured, same window under both backends, asking for `400,300`: X11 landed at
//! `400,300`, Wayland at `840,443` (and elsewhere on other runs — it is not a fixed offset, it is
//! KWin's placement). Everything below about anchoring and clamping is therefore correct and
//! inert on the desktop this was written against. See [`place`].

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use tauri::{AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "tray-panel";

/// Has the panel held focus since it was last shown?
///
/// See the `Focused` arm in `lib.rs` for what this is defending against: a compositor that refuses
/// the focus we asked for emits a blur the moment we show, and hiding on it makes the panel look
/// like it never opened. An `AtomicBool` rather than window state because both writers are on the
/// event loop and the read has to be cheap enough to sit in a window-event match arm.
static HAD_FOCUS: AtomicBool = AtomicBool::new(false);

/// The click the panel is currently anchored to, so a resize can re-place against it.
///
/// The panel opens at the tallest state's height and the webview corrects it on first paint — and
/// on a BOTTOM panel the correction moves the window: it is positioned by its top-left, so losing
/// 125px of height leaves the same top edge and a 125px gap between the panel and the tray it is
/// supposed to be hanging from. Re-placing against the original click closes it. Two `i32`s rather
/// than a mutex because both writers are the event loop.
static ANCHOR: (AtomicI32, AtomicI32) = (AtomicI32::new(0), AtomicI32::new(0));

fn remember(at: Option<(i32, i32)>) {
    let (x, y) = at.unwrap_or((0, 0));
    ANCHOR.0.store(x, Ordering::Relaxed);
    ANCHOR.1.store(y, Ordering::Relaxed);
}

fn anchor() -> Option<(i32, i32)> {
    let at = (
        ANCHOR.0.load(Ordering::Relaxed),
        ANCHOR.1.load(Ordering::Relaxed),
    );
    // `(0, 0)` is the same "no usable coordinates" sentinel `place` reads it as.
    (at != (0, 0)).then_some(at)
}

/// The panel took focus. Called from the `Focused(true)` event.
pub fn mark_focused() {
    HAD_FOCUS.store(true, Ordering::Relaxed);
}

/// Did it have focus, and clear the flag. `true` means this blur is a real "the user looked away".
pub fn take_focused() -> bool {
    HAD_FOCUS.swap(false, Ordering::Relaxed)
}

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
                take_focused();
                let _ = window.hide();
                return;
            }
            take_focused();
            show_at(&window, at);
            return;
        }
        match build(&app) {
            Ok(window) => show_at(&window, at),
            Err(error) => eprintln!("tray: cannot open the panel window: {error}"),
        }
    });
}

/// Position, show, and position again.
///
/// THE SECOND `place` IS THE ONE THAT WORKS, and the first is not redundant. On X11 a position set
/// on an unmapped window is advisory: the window manager places the window when it maps it and is
/// free to ignore what was asked for, which KWin does — the panel came up dead centre at exactly
/// `(screen - width) / 2`, the giveaway that nothing had positioned it at all.
///
/// So the position is applied again once the window is mapped, which is the call that lands. The
/// first one stays because a compositor that DOES honour it never shows the panel at the wrong
/// place even for a frame, and on the ones that do not it costs a no-op.
fn show_at(window: &tauri::WebviewWindow, at: Option<(i32, i32)>) {
    remember(at);
    place(window, at);
    let _ = window.show();
    place(window, at);
    // AND THE FOCUS HAS TO BE ASKED FOR WITH A TIMESTAMP, or KWin refuses it and the panel becomes
    // the thing IMPLEMENTATION-PLAN §6 forbids: never focused, so never blurred, so never hidden by
    // looking away. `focus::present` has the measurement.
    crate::focus::present(window);
}

/// Hide it, from anywhere.
pub fn hide(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        take_focused();
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
            // The window shrank around a fixed top-left; put it back against the click it opened
            // from, or it hangs in space above the tray.
            place(&window, anchor());
        }
    });
}

fn build(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    // `?surface=tray` rather than a second HTML file. `index.html`'s own comment warns that its
    // stylesheet chain is easy to forget one link of, and a second copy of it is a blank panel with
    // no error the day someone adds a stylesheet to one and not the other. app.js reads the
    // parameter and mounts the panel instead of the shell — the same gate the frame preview uses,
    // which also means `?surface=tray` opens the panel in a browser.
    WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?surface=tray".into()),
    )
    .title("Proton Drive Sync")
    .inner_size(WIDTH, HEIGHT)
    .resizable(false)
    .decorations(false)
    // THREE OF THE NEXT FOUR OPTIONS DO NOTHING HERE, AND A COMMENT THAT DOES NOT SAY SO IS WORSE
    // THAN NO COMMENT (#351): a reader takes the panel's behaviour as designed rather than as
    // whatever the compositor happens to do. They are kept because each is correct where it applies
    // and costs nothing where it is not.
    //
    // Measured on Plasma 6, two plain GTK3 windows differing only in `GDK_BACKEND`, issuing the
    // exact calls this builder issues, read back through KWin's scripting API. The X11 row is the
    // control and is the point: without it the Wayland row is equally consistent with the probe
    // never making the call. `docs/agent-notes/kwin-read-a-window-app-id.md` has the method.
    //
    //   GDK_BACKEND=x11      skipTaskbar=true   skipPager=true   keepAbove=true   skipSwitcher=false
    //   GDK_BACKEND=wayland  skipTaskbar=false  skipPager=false  keepAbove=false  skipSwitcher=false
    //
    // Read through tao 0.35's Linux backend; the CALLS are GTK/Wayland facts and will not rot, the
    // source citations will (`lib.rs`'s titlebar fix carries its version bound for the same reason).
    //
    // `always_on_top` → `gtk_window_set_keep_above`, from tao's BUILDER (`platform_impl/linux/
    // window.rs`, `attributes.always_on_top`) — not `WindowRequest::AlwaysOnTop`, which is the
    // runtime setter this never reaches. xdg-shell has no stacking request, so on Wayland the panel
    // is an ordinary toplevel. **It is not this option that keeps it usable there**: KWin activates
    // it because it was just mapped, which is a compositor default and not something the app asks
    // for. In particular it is NOT `focus::present`, whose stamped path downcasts to `X11Window` and
    // returns on Wayland, leaving a bare `set_focus` that moves nothing. Losing that focus is what
    // `lib.rs` hides the panel on, so the popover contract survives — resting on a default rather
    // than on any line here.
    .always_on_top(true)
    // `skip_taskbar` → `gtk_window_set_skip_taskbar_hint` PLUS `gtk_window_set_skip_pager_hint`
    // (tao `WindowRequest::SetSkipTaskbar`), both X11-only. So on Wayland the popover takes a
    // taskbar button, which is wrong for a window that hides on blur, on Esc, and on a second
    // click — an entry offered for something there is nothing to switch to.
    //
    // TWO CORRECTIONS TO WHAT THIS COMMENT USED TO CLAIM. It said "out of the taskbar AND the
    // window switcher". `skipSwitcher` measures `false` on BOTH backends, and the X11 probe was
    // confirmed reachable by the Walk-Through-Windows shortcut under stock `kwinrc`, so the Alt-Tab
    // half is not something this call delivers anywhere — the justification it was given ("one that
    // answers Alt-Tab is a window the user has to dismiss twice") never described shipped
    // behaviour. Staying out of the switcher needs the separate property, which is what
    // `xwaylandvideobridge` sets alongside this one. The taskbar half is delivered on X11 only.
    //
    // Neither real fix is absorbable, which is why #351 stays open rather than being closed here:
    // gtk-layer-shell makes the panel a layer surface that never reaches the taskbar AND can place
    // itself (so it would close #370, `place`'s positioning gap, in the same change), but it is a
    // new system build
    // dependency across three packaging trees and must run before the window is realized;
    // `org_kde_plasma_surface.set_skip_taskbar` is exactly the right knob and is KDE-only, needing
    // raw Wayland FFI for one call. Both are decisions, not cleanups.
    .skip_taskbar(true)
    // And this one is inert on BOTH backends, which is a different fault to the two above and the
    // reason the measurement above needs its X11 control: `tauri-runtime-wry`'s `shadow` has arms
    // for Windows and macOS only, and tao's Linux window has no shadow concept at all. A call that
    // compiles to nothing is not a platform gap, it is a line that never did anything.
    .shadow(false)
    // Built hidden and shown by the caller once it is positioned. Building it visible paints it
    // at the default position first, so it visibly jumps to the indicator.
    .visible(false)
    .build()
}

/// Put the panel under the click, inside the screen — **on X11. On Wayland this function computes
/// a correct position and the compositor ignores it.**
///
/// The last line is `set_position`, which reaches GTK as `gtk_window_move` (tao 0.35
/// `WindowRequest::Position`). A Wayland client cannot position its own toplevel — xdg-shell has no
/// request for it — so the value is discarded and KWin places the panel by its own policy.
/// Measured, the same window under both backends asking for `400,300`: X11 landed at `400,300`,
/// Wayland at `840,443`, and elsewhere again on other runs. Not a fixed offset to correct for; the
/// absence of client positioning. Filed as #370, whose fix (a layer surface) would also close #351.
///
/// Everything below is therefore right and inert on the desktop it was written against. It is left
/// intact deliberately: it is exactly what a layer surface would need (gtk-layer-shell can anchor
/// and margin, which is the same arithmetic), and it is what runs on X11 today. Filed rather than
/// worked around, because guessing an offset for a placement policy is how a popover ends up wrong
/// on every desktop instead of one.
///
/// The clamp is what makes this right on a bottom panel: a click at y=2112 on a 2160-tall screen has
/// no room for a 442px panel below it, so the panel goes above the click instead. That is the
/// ordinary KDE case, and the spec's fixed `top:40px` would have put it at the wrong end of the
/// screen on every one of them.
fn place(window: &tauri::WebviewWindow, at: Option<(i32, i32)>) {
    // A WINDOW THAT HAS NEVER BEEN SHOWN HAS NO CURRENT MONITOR, and the first version returned
    // early on that — silently, so the panel kept whatever position the window manager had given it.
    // On this desktop that was dead centre, at exactly `(3840 − 724) / 2`, which is the tell: a
    // centred popover is not a positioning bug, it is the absence of positioning.
    //
    // The panel is built hidden on purpose (showing it first would paint it at the default position
    // and then jump), so this is the ordinary path rather than an edge case: it is what happens
    // every time the panel opens for the first time in a session.
    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => Some(monitor),
        _ => window.primary_monitor().ok().flatten(),
    };
    let Some(monitor) = monitor else {
        eprintln!("tray: no monitor to place the panel against; leaving it where it is");
        return;
    };
    // EVERYTHING HERE IS IN PHYSICAL PIXELS, because that is what the host sends.
    //
    // Measured, and it is not what the first version assumed. On this 3840×2160 display at scale 2,
    // a click on the indicator arrived as `Activate(3192, 2112)` — physical coordinates, not the
    // 1920×1080 logical space Tauri's `LogicalPosition` works in. Treating them as logical put the
    // panel in the middle of the screen: the arithmetic was right and the units were not, which is
    // the failure that looks like a positioning bug and is a conversion bug.
    //
    // `monitor.position()`, `monitor.size()` and `outer_size()` are all physical already, so working
    // in that space means the click needs no conversion at all — and a conversion that is not
    // performed cannot be performed in the wrong direction.
    let area = monitor.size();
    let origin = monitor.position();
    let scale = monitor.scale_factor();
    let fallback = LogicalSize::new(WIDTH, HEIGHT).to_physical::<f64>(scale);
    let size = window
        .outer_size()
        .map(|s| tauri::PhysicalSize::new(s.width as f64, s.height as f64))
        .unwrap_or(fallback);
    let (area, origin) = (
        tauri::PhysicalSize::new(area.width as f64, area.height as f64),
        tauri::PhysicalPosition::new(origin.x as f64, origin.y as f64),
    );
    let margin = 8.0 * scale;

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
                if below {
                    cy + margin
                } else {
                    cy - size.height - margin
                },
            )
        }
        None => (
            origin.x + area.width - size.width - FALLBACK_INSET.0,
            origin.y + FALLBACK_INSET.1,
        ),
    };

    // `max` before `clamp`: on a display shorter than the panel the upper bound goes below the
    // lower one and `clamp` panics. Not hypothetical at the tallest state on a 768px laptop screen
    // once the panel is 442 physical at scale 2.
    let max_x = (origin.x + area.width - size.width - margin).max(origin.x + margin);
    let max_y = (origin.y + area.height - size.height - margin).max(origin.y + margin);
    let x = x.clamp(origin.x + margin, max_x);
    let y = y.clamp(origin.y + margin, max_y);
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}
