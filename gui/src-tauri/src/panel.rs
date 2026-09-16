//! The tray panel window (S8) — the 360px compact panel, floating over the desktop.
//!
//! `10-tray.md` replaces the tray's text menu with the panel from the main screen: same hexagon,
//! same seam, same sentence, with the menu rows below it. libayatana-appindicator cannot draw any of
//! that, so the panel is a real webview window — borderless, and INTENDED to be above everything
//! and gone the moment you look away. Intended, not everywhere delivered: that sentence is
//! `10-tray.md`'s design, and the third of its three clauses is dead on the layer path, where GTK is
//! delivered no focus event at all and so nothing hides the panel on blur. Which platform gets which
//! of the three is the subject of the paragraphs and bullets below; read this line as the goal they
//! are measured against, never as a description of shipped behaviour.
//!
//! **"Above everything" and "under the click" were both X11-only, and are not any more** (#351 and
//! #370). `always_on_top` and `place` reach GTK as `gtk_window_set_keep_above` and
//! `gtk_window_move`, neither of which xdg-shell has an equivalent for — so on Wayland the panel
//! used to be a compositor-placed toplevel with a taskbar button, and the two calls were measured
//! doing nothing.
//!
//! The panel is now promoted to a **layer surface** where the compositor supports it
//! (`promote_to_layer_surface`), which sits on a layer the compositor stacks above ordinary
//! windows, takes no taskbar button, and is positioned by anchor + margin, which a client is
//! allowed to set. Measured on Plasma 6 Wayland through KWin's scripting API — an ordinary toplevel
//! asking for `400,300` landed at `839,311`; the layer surface with the same margins reported:
//!
//! ```text
//! class=layerprobe cap='' geom=400,300 362x442 skipTaskbar=true skipPager=true
//! ```
//!
//! `geom` is the requested logical position, read straight off KWin's `frameGeometry`.
//!
//! **A LAYER SURFACE IS IN `workspace.windowList()`** — managed, with geometry, and carrying an
//! EMPTY caption because wlr-layer-shell has no title request. This paragraph used to say the
//! surface did not appear in the window list at all: that was a bug in the probe, which filtered
//! its dump on caption and so never matched a surface that has none. The same bug is why the
//! position was first checked by capturing the screen and scanning it for a colour, a method that
//! was never needed and cannot work anyway (see `promote_to_layer_surface`). What survives about
//! the taskbar is the OUTCOME plus one reading — `skipTaskbar=true` on a managed window — and WHAT
//! SETS IT WAS NOT VERIFIED; `promote_to_layer_surface` says what would settle that.
//!
//! **Three desktops, three behaviours, and only the first is fixed.** On Wayland with
//! `zwlr_layer_shell_v1` (KDE, wlroots) both bugs are gone. On Wayland WITHOUT it — **GNOME, whose
//! Mutter does not implement the protocol** — `is_supported()` is false, the panel stays an
//! ordinary toplevel, and both bugs remain exactly as before. On X11 the two hints work and the
//! layer path is never taken. The dismissal contract is unchanged on the two toplevel paths and is
//! not intact on the layer one: blur-to-hide is dead there (measured, in both directions), Esc is
//! unestablished either way, and a second click on the indicator has no reason to be affected —
//! `toggle` DECIDES off `is_visible`, consulting no focus state to do it (it still clears the flag
//! afterwards) — but that is a structural argument and not a measurement, and nothing here has
//! clicked one twice. The bullets below say which is which, and grade each.
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
//!     (On the layer path nothing in this process can observe that happening: no focus event is
//!     delivered for a layer surface at all, see the bullet below.)
//!   * clicking the indicator again hides it. That is `toggle` reading `is_visible` off a D-Bus
//!     `Activate`, so it is the one dismissal of the three that rests on no focus delivery at all.
//!     (Which is not the same as its having been measured on a layer surface. Nothing here has.)
//!   * losing focus hides it — **on the two toplevel paths only.** No `focus-in-event` and no
//!     `focus-out-event` is delivered to GTK for a layer surface (measured; the finding is written
//!     out in full at `KeyboardMode::OnDemand` in `promote_to_layer_surface`), so `mark_focused`
//!     never runs, the blur arm in `lib.rs` never fires, and the layer-shell panel does not go away
//!     when you look away.
//!   * Esc hides it — the webview's own handler, `app.js` calling `hide_tray_panel` — which needs
//!     the keypress to reach the webview, so on the layer path it depends on the surface holding
//!     the keyboard. Whether it does is UNKNOWN and the same note says why the measurement above
//!     does not answer it.
//!
//! # Position
//!
//! `Activate(x, y)` carries the click in screen coordinates, but not consistently in which SPACE
//! (#394). A hand-made `gdbus ... Activate 3192 2112` — a caller that chose its own units, not the
//! real path — arrived PHYSICAL, on a 3840×2160 output at scale 2. A real click on the tray icon,
//! measured on the same output, arrived `Activate(1540, 1060)` — LOGICAL. Treating the second as
//! physical is the bug: every derived value halves, and the top/bottom split (below) compares the
//! unscaled click against half the PHYSICAL screen height, so a bottom-corner click reads as the
//! top half and the panel opens downward instead of up. [`place`] cannot trust either reading and
//! disambiguates by BOUNDS instead (`resolve_click`): a pair some monitor's LOGICAL rectangle holds
//! — origin included, every output considered, not one output's size — is logical and gets promoted
//! by that output's own scale; a pair no logical rectangle holds was already physical. At scale 1
//! the two spaces coincide and the promotion is a multiply by one. Inside the UNION of those
//! rectangles the two readings are indistinguishable from the pair alone, and the tie goes to
//! logical because that is what the measured real host sends — so a physical caller is honoured only
//! OUTSIDE that union, and inside it can be moved to the wrong scale or, when the rectangle belongs
//! to a neighbour, the wrong output. `resolve_click` carries both shapes of that cost, why inverting
//! the tie is worse, and why nothing available on Wayland can break it. So the spec's `top:40px;
//! right:16px` fallback is only that: a fallback, for a host that sends `(0, 0)` because it does not
//! track the pointer — and it is in LOGICAL pixels, scaled where it is used like every other length
//! the spec gives. The panel is placed against the (now-physical) click and then
//! clamped into the work area, which is what makes it open UPWARD on a bottom panel — the ordinary
//! case on KDE, and the one a fixed top-right rule gets wrong on every Plasma desktop.
//!
//! **[`place`]'s last step is X11's, and the arithmetic before it is not.** A Wayland client
//! cannot position its own toplevel: there is no xdg-shell request for it, so `gtk_window_move` is
//! discarded and KWin places the panel by its own policy. Measured, same window under both
//! backends, asking for `400,300`: X11 landed at `400,300`, Wayland at `840,443` (and elsewhere on
//! other runs — it is not a fixed offset, it is KWin's placement). What #370's fix changed is where
//! the computed answer is delivered, not the computation: [`place`] converts the same clamped
//! result into anchor margins and RETURNS BEFORE `set_position` on the layer path, where KWin then
//! reports the panel at the position that was asked for. So the anchoring and clamping below is
//! computed everywhere, consumed as margins on the layer path and as `set_position` on X11, and
//! discarded only on a Wayland compositor without `zwlr_layer_shell_v1`. See [`place`].

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

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
///
/// On the layer path the height correction lands while the panel is up too — MEASURED (#385,
/// refutation measured 2026-09-16, corrects this static's earlier claim that it did not):
/// `set_size` reaches the wire ~250ms after the panel maps and again on every later state change,
/// and KWin honours it. So the same gap this static exists to close is real on that path as well,
/// and closing it used to be one status poll late regardless of path (#401: the re-place read the
/// height back from `outer_size()`, a cache the compositor had not caught up on yet). Fixed by
/// threading the intended height straight into `place` instead of reading it back — see
/// `INTENDED_HEIGHT`.
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

/// The height this process last asked the panel to be, in LOGICAL pixels.
///
/// The panel's size is a value THIS PROCESS sets and nothing else changes — fixed width, never
/// resizable, height driven only by the webview's own report (`resize` is its one writer at
/// runtime). So this is the one place the panel's intended height lives: reading it back from the
/// window (`outer_size()`) is a second copy of a number the caller already holds, and the two
/// disagree for exactly as long as it takes the compositor to catch up (#401) — see the comment on
/// the size line in `place`. Stored as `f64` bits rather than truncated to `i32` like `ANCHOR`,
/// because a reported height is not always a whole number (the frames draw heights like 441.5).
/// Seeded to `HEIGHT`, the height the panel opens at before any `resize` has run. Written only from
/// the event loop, the same idiom as `ANCHOR`.
static INTENDED_HEIGHT: AtomicU64 = AtomicU64::new(HEIGHT.to_bits());

fn remember_height(height: f64) {
    INTENDED_HEIGHT.store(height.to_bits(), Ordering::Relaxed);
}

fn intended_height() -> f64 {
    f64::from_bits(INTENDED_HEIGHT.load(Ordering::Relaxed))
}

/// The panel took focus. Called from the `Focused(true)` event — which a layer surface never
/// receives, so on that path this never runs (see `KeyboardMode::OnDemand` in
/// `promote_to_layer_surface`).
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
/// window opens at this and the webview reports a corrected height on first paint via `resize`,
/// because Phase 1 omits lines the frames draw — the offline panel has no `retrying in 40s` — and a
/// window sized to the drawing would carry that much empty space below the menu.
///
/// **The correction lands on the layer path too** — MEASURED, see `resize_layer_surface` (this
/// doc used to claim the opposite; refuted by #385, refutation measured 2026-09-16). The wire
/// shows `set_size` reaching the surface ~250ms after it maps, KWin answering it, and the same
/// happening again on every later state change. So the panel is this tall only until the webview's
/// first paint, on every path, not for as long as it is visible.
const HEIGHT: f64 = 442.0;

/// The spec's fallback corner, for a host that sends no usable coordinates: `right:16px; top:40px`.
///
/// LOGICAL pixels, like every other length the spec gives, and therefore multiplied by the monitor's
/// scale where it is used — `panel_origin`'s `None` arm, the only reader. It was added straight onto
/// physical coordinates once, beside a `margin` that was already scaled, which put the panel half
/// the intended distance from the corner on a scale-2 output and a third of it on a scale-3 one.
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
            // own flag: the window is also hidden from elsewhere — the blur arm in `lib.rs`, and
            // the Esc the webview handles — and a second source of truth about whether it is up
            // would disagree with the compositor sooner or later. On the layer path both of those
            // are in doubt (no focus event is delivered, so the blur arm is dead; Esc needs a click
            // to grant the keyboard, which is unmeasured — `promote_to_layer_surface` has both),
            // which leaves this branch as the one dismissal OF THE THREE the module doc's contract
            // names — this click, blur, Esc — that rests on no focus delivery at all, and one more
            // reason to ask the window rather than trust a flag. The scope is the point: the panel
            // is also hidden from OUTSIDE that contract, by `commands::tray_action` on every menu
            // row and by `dbusmenu::about_to_show` before a host draws the native menu — the latter
            // documented there as the way out of a panel no blur will hide. But NOT by
            // `commands::hide_tray_panel`: its only caller is the webview's Esc handler, so that is
            // the contract's Esc leg wearing a command name, and counting it as a fourth exit would
            // read as evidence that Esc survives on the layer path, which is the thing nothing here
            // has measured. Dropping the scope turns a true sentence about the contract into a
            // false one about the file.
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
            Ok(window) => {
                // BETWEEN `build` AND THE FIRST `show`, and only here: promoting the window to a
                // layer surface swaps its GdkWindow's surface type, which is a thing that can only
                // be done before it is realized. `build` returns it hidden precisely so this seam
                // exists (see `promote_to_layer_surface`). It is a no-op on X11 and on compositors
                // without `zwlr_layer_shell_v1`, where the panel stays the toplevel it was.
                #[cfg(target_os = "linux")]
                let _ = promote_to_layer_surface(&window);
                show_at(&window, at)
            }
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
    // BOTH attempts have to fail before this is worth saying. The first one runs against an unmapped
    // window, which is the case `resolve_monitor_and_click` exists for but is also the one where
    // `current_monitor()` cannot help it; the second runs mapped. Reporting per attempt said it
    // twice for one open — #395's reported symptom — and said it at all for an open that the second
    // attempt went on to place.
    let height = intended_height();
    let placed_unmapped = place(window, at, height);
    let _ = window.show();
    let placed_mapped = place(window, at, height);
    if !placed_unmapped && !placed_mapped {
        report_unplaced();
    }
    // AND THE FOCUS HAS TO BE ASKED FOR WITH A TIMESTAMP, or KWin refuses it and the panel becomes
    // the thing IMPLEMENTATION-PLAN §6 forbids: never focused, so never blurred, so never hidden by
    // looking away. `focus::present` has the measurement. That defends the X11 path: on the layer
    // path the panel is in exactly that state anyway, because no focus event is delivered to it at
    // all (`promote_to_layer_surface`), and `focus::present`'s stamped path is X11-only regardless.
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
            let height = height.max(1.0);
            remember_height(height);
            let _ = window.set_size(LogicalSize::new(WIDTH, height));
            // A MAPPED LAYER SURFACE DOES RESIZE, BUT NOT FROM THE CALL ABOVE — two facts that a
            // name collision makes easy to run together, and the first version of this comment ran
            // them together and was wrong. `WebviewWindow::set_size` is the TOPLEVEL call, and it
            // still reaches no layer surface: measured, `gtk_window_resize` on its own puts nothing
            // on the wire. The request that lands is the PROTOCOL's `zwlr_layer_surface_v1.set_size`,
            // which gtk-layer-shell derives from the GTK size REQUEST — `resize_layer_surface`
            // below. Same two words, different calls.
            //
            // What is refuted (#385, refutation measured 2026-09-16) is only that a mapped surface
            // ignores the size request: it reaches the wire ~250ms after the panel maps and again on
            // every later state change, and KWin honours it. See `resize_layer_surface`.
            //
            // The call above stays because it is the one that works everywhere else — X11, and
            // Wayland without `zwlr_layer_shell_v1` — where `resize_layer_surface` is not reached.
            #[cfg(target_os = "linux")]
            if let Ok(gtk_window) = window.gtk_window() {
                if gtk_layer_shell::LayerShell::is_layer_window(&gtk_window) {
                    resize_layer_surface(&gtk_window, height);
                }
            }
            // The window shrank or grew around a fixed top-left; put it back against the click it
            // opened from, or it hangs in space above the tray. `height` is the one just requested
            // above, not a value read back from the window (#401) — one attempt, on a window that is
            // up by definition, so its failure is reported directly.
            if !place(&window, anchor(), height) {
                report_unplaced();
            }
        }
    });
}

/// Promote the panel to a **layer surface**, which is what actually fixes #351 and #370.
///
/// A layer surface is not an xdg toplevel. It takes no taskbar button (`skip_taskbar`'s job, which
/// the hint itself does not do on Wayland), it sits on a layer the compositor stacks above normal
/// windows (`always_on_top`'s job likewise), and it is positioned by ANCHOR + MARGIN, which a client
/// may set — the one thing xdg-shell deliberately withholds.
///
/// **THE TASKBAR OUTCOME IS MEASURED; THE MECHANISM IS NOT.** This doc used to say the surface TYPE
/// did that job "rather than a hint the compositor discards". That reading came from a probe
/// reporting the surface absent from `workspace.windowList()`, and the absence was the probe's own
/// bug: it filtered the dump on caption, and a layer surface has no title, so nothing ever matched.
/// Unfiltered, on Plasma 6 Wayland, the surface is there — managed, with geometry:
///
/// ```text
/// class=layerprobe cap='' geom=400,300 362x442 skipTaskbar=true skipPager=true
/// ```
///
/// So what is observed is `skipTaskbar=true` on a managed window, and `geom=400,300` is the
/// requested logical position off KWin's `frameGeometry` — which is the #370 fix, read directly and
/// with no screen capture involved. The capture-and-scan-for-a-colour method was only ever adopted
/// because of the filter bug above, and it could not have answered this anyway: the
/// `set_size_request` comment in the body carries the `spectacle` measurement. WHAT SETS
/// `skipTaskbar` IS UNKNOWN — this measurement cannot tell KWin's own policy for layer surfaces
/// from anything gtk-layer-shell, GTK or the builder's `skip_taskbar` asks for. Settling it needs a
/// probe that varies one of those across two otherwise identical layer surfaces and reads the
/// property back. Until then this doc records the outcome and names no cause.
///
/// **This is not universal, and the fallback is the whole reason it is a function rather than a
/// line.** `zwlr_layer_shell_v1` is a wlroots protocol that KDE adopted; **Mutter does not implement
/// it**, so on GNOME `is_supported()` is false and the panel stays an ordinary toplevel with both
/// bugs — the taskbar entry and the compositor-chosen position. That is not a regression (it is
/// exactly today's behaviour) but it is not a fix there either, and a comment claiming otherwise is
/// the defect #351 was filed about in the first place.
///
/// `is_supported()` is also false on X11, where the two hints work — so the X11 path is untouched
/// and this returns `false` without doing anything.
///
/// # Ordering
///
/// `gtk_layer_init_for_window` must run **before the window is realized**, because it swaps the
/// GdkWindow's surface type. That seam exists because the panel is built `.visible(false)` and
/// shown by the caller: `build` returns an unrealized window and `show_at` is the first thing to
/// map it. Calling this after a `show` is not a soft failure — the surface is already an xdg
/// toplevel by then.
#[cfg(target_os = "linux")]
fn promote_to_layer_surface(window: &tauri::WebviewWindow) -> bool {
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

    if !gtk_layer_shell::is_supported() {
        return false;
    }
    let Ok(gtk_window) = window.gtk_window() else {
        eprintln!("tray: no GTK window to promote to a layer surface; leaving it a toplevel");
        return false;
    };
    gtk_window.init_layer_shell();
    // `Overlay`, not `Top`: `Top` is below fullscreen windows, and a tray popover the user just
    // asked for should not be hidden behind one.
    gtk_window.set_layer(Layer::Overlay);
    // Anchored to the top-left corner so the two margins `place` sets are absolute offsets into the
    // output, which is the same arithmetic `place` already does. Anchoring to ONE corner is what
    // makes that true — anchor two opposite edges and the surface stretches between them instead.
    gtk_window.set_anchor(Edge::Top, true);
    gtk_window.set_anchor(Edge::Left, true);
    // WITHOUT THIS THE PANEL RECEIVES NO KEYBOARD INPUT AT ALL. A layer surface gets none by
    // default, and the panel's dismissal contract was written around focus: Esc closes it (the
    // webview's own handler, `app.js` calling `hide_tray_panel`), and §6's "gone the moment you look
    // away" is the blur arm in `lib.rs`. `OnDemand` is documented to take focus when the surface is
    // clicked and give it back, which is the popover behaviour; `Exclusive` would hold the keyboard
    // hostage.
    //
    // THE FOCUS MEASUREMENT LIVES HERE IN FULL, AND EVERY OTHER MENTION IN THIS FILE POINTS AT IT.
    // On Plasma 6 Wayland a probe logged NO `focus-in-event` AND NO `focus-out-event` for the layer
    // surface — the two GTK signals tao converts into `WindowEvent::Focused` — not one, not even at
    // map. The two halves of the contract come out of that differently:
    //
    //   * THE BLUR HALF IS SETTLED, AND IT IS DEAD. `panel::mark_focused` never runs, so
    //     `take_focused` never returns `true`, so the blur arm in `lib.rs` hides nothing. It is dead
    //     in both directions, and the panel does not dismiss by being looked away from.
    //   * THE Esc HALF IS NOT SETTLED BY THAT SAME MEASUREMENT, and reading it as settled is the
    //     mistake to avoid here. `OnDemand` is meant to grant the keyboard ON A CLICK INTO THE
    //     SURFACE; the probe never clicked; and observing no focus events is exactly what that mode
    //     WITHOUT A CLICK is expected to produce. So whether a click grants focus — and hence
    //     whether Esc still reaches the webview and dismisses the panel — is UNKNOWN. Do not write
    //     "Esc works" here and do not write "Esc is broken" either. A probe that clicks into the
    //     mapped surface and reads the keyboard focus back would settle it.
    //
    // THE DOCS PAGE STATES Esc PLAINLY, AND THAT IS NOT A CONTRADICTION OF THE PARAGRAPH ABOVE.
    // `website/.../desktop/tray.md` describes what the app is meant to do, for a reader who cannot
    // observe whether a surface holds the keyboard and could not act on it if they could. The two
    // halves are documented differently BECAUSE THE EVIDENCE FOR THEM DIFFERS, not by oversight:
    // click-away is MEASURED not to arrive here, so a page promising it would be saying something
    // known to be false, and that page carries the exception; Esc is merely UNMEASURED on this one
    // path, so the page says what the design says. If the probe above comes back negative, that is
    // a bug to fix in this file, not a sentence to qualify over there.
    //
    // Neither finding is a reason to change this line: `OnDemand` is what a popover wants either
    // way, and dropping it could only remove keyboard input the panel may already have.
    gtk_window.set_keyboard_mode(KeyboardMode::OnDemand);
    // THE SIZE REQUEST IS THE CALL THAT REACHES THE SURFACE AND THE TOPLEVEL ONES DO NOT:
    // gtk-layer-shell turns the GTK size REQUEST into `zwlr_layer_surface_v1.set_size`, while
    // `gtk_window_set_default_size`, Tauri's `inner_size` and `WebviewWindow::set_size` are
    // toplevel calls that never get there. That much is unchallenged. WHAT IS NOT ESTABLISHED IS
    // THAT THE SURFACE NEEDS THE CALL AT ALL: this line used to assert, as a fact, that a layer
    // surface anchored to ONE CORNER derives no size from the compositor and so the client has to
    // send one. That half was never re-measured; it predicts precisely the blank screen the
    // paragraph below FAILED to reproduce; and how that re-measurement's probe got a size with no
    // explicit request was never determined. So do not repeat it as a fact. The other anchoring IS
    // measured: two OPPOSITE edges and the compositor sizes it — a probe anchored to all four was
    // configured 1920x1034, the screen minus the panel's exclusive zone.
    //
    // WHY THIS PARTICULAR CALL IS HERE IS NO LONGER ESTABLISHED, AND IT STAYS ANYWAY. It was written
    // against a reading of "without the request the surface reports `visible=true mapped=true
    // alloc=362x442` and nothing is on screen" — i.e. the panel is invisible without it. A
    // re-measurement against tao's exact construction path DID NOT REPRODUCE that. Unreproduced is
    // not disproved, so this comment asserts neither: the invisibility is an open question, and the
    // call is kept because it is harmless and because a reproduction that failed is not evidence
    // that removing it would be safe.
    //
    // WHAT WOULD SETTLE IT IS ONE EXPERIMENT IN TWO HALVES, AND THE SECOND IS WORTHLESS WITHOUT THE
    // FIRST. (1) Run the two constructions side by side — the one the original reading came from,
    // and tao's exact path, where the invisibility did not reproduce — and isolate what differs.
    // (2) In whichever construction reproduces the invisibility, build the window that way (same
    // widget hierarchy, same realize order) and toggle only this call. The same plan, in the same
    // words, is in section 5 of `docs/agent-notes/measuring-a-gtk-layer-shell-surface.md`; the two
    // used to prescribe one half each, which reads as two different next steps for one question.
    //
    // AND THE HARD PART IS READING THE ANSWER, WHICH NOTHING HERE HAS SOLVED. Both halves need a way
    // to tell a drawn surface from an undrawn one, and this change validated none. `spectacle` does
    // not capture layer surfaces at all, measured with KWin reporting the surface's geometry at the
    // same moment: "nothing is on screen" is what a capture says about a layer surface whether or
    // not it is being drawn, which is a hazard for any re-measurement of this line and for the #370
    // position check that first used the same method. KWin is NOT the established substitute — the
    // undrawn state never reproduced, so no dump was ever taken against a surface known not to be
    // drawn, and what such an entry would read is therefore unknown. That cuts both ways: nothing
    // here says KWin CANNOT answer it either. Finding a read-out method is part of the experiment,
    // not a step that precedes it. Geometry is a different question and KWin does answer that one
    // (#370).
    resize_layer_surface(&gtk_window, HEIGHT);
    true
}

/// Push a height at the layer surface, which is a different call from `set_size`.
///
/// The panel's height is not fixed — `resize` is called from the webview once it knows which state
/// it is in — and on a layer surface the toplevel `set_size` does not reach the compositor. Kept
/// beside the promotion so the two cannot drift: the size the surface is built with and the size
/// that changes later have to travel the same path.
///
/// **THIS DOC USED TO SAY NEITHER CALL RESIZES A PANEL THAT IS ALREADY MAPPED, AND THAT IS
/// REFUTED** (#385, refutation measured 2026-09-16, against the shipped binary). On the layer path
/// the correction lands on the wire ~250-500ms after the panel appears, on the FIRST open with no
/// reopen needed, and again on every later state change — KWin honours it every time.
///
/// The probe that produced the old reading (`RZ2`) is kept below because it did reproduce, on a
/// *different* construction, and the three readings of it are worth keeping apart rather than
/// resolved into one:
///
/// ```text
/// RZ2 issued resize -> 362x250
/// RZ2 after alloc=362x442        <- GTK unchanged
/// KWin  geom=400,300 362x442     <- compositor unchanged
/// ```
///
///   * **MEASURED** — a widget UNDER the window carrying its own size request ≥ the old height
///     reproduces both lines above exactly. That this was RZ2's actual construction is
///     **UNDETERMINED**: that probe's own source no longer exists to check.
///   * **INFERRED** — RZ2 may instead have blocked the GTK main loop after its two calls, so GTK
///     never processed the resize and nothing reached the wire. Not measured either way.
///   * **Reading `outer_size()`/GTK's allocation too early is not sufficient on its own.** It
///     explains RZ2's GTK line — every construction reproduces that synchronously, GTK re-allocates
///     a few ms later regardless — but not its KWin line, since a KWin dump costs about 0.6s and
///     every dump taken since has read the new size.
///
/// This measurement cannot distinguish "the old reading was wrong when it was written" from "the
/// stack changed since" — versions, so a future divergence can be dated: `main` @ `04d615f`,
/// Plasma 6 / KWin 6.7.5, tao 0.35.3, gtk-layer-shell 0.8.2, wry 0.55.1, tauri-runtime-wry 2.11.4.
/// `docs/agent-notes/measuring-a-gtk-layer-shell-surface.md` §3 has the full measurement.
///
/// `queue_resize()` does not help either construction. Nothing here is worked around: THIS call is
/// what resizes a mapped layer surface, and the toplevel `set_size` in `resize` still is not —
/// measured, `gtk_window_resize` on its own puts nothing on the wire. An earlier version of this
/// paragraph said both calls take effect, which contradicted this doc's own second paragraph; the
/// two requests share the name `set_size` and nothing else, and that collision is the trap.
#[cfg(target_os = "linux")]
fn resize_layer_surface(gtk_window: &gtk::ApplicationWindow, height: f64) {
    use gtk::prelude::WidgetExt;
    gtk_window.set_size_request(WIDTH.round() as i32, height.round() as i32);
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
    // ON A WAYLAND TOPLEVEL, THREE OF THE NEXT FOUR OPTIONS DO NOTHING, AND A COMMENT THAT DOES NOT
    // SAY SO IS WORSE THAN NO COMMENT (#351): a reader takes the panel's behaviour as designed
    // rather than as whatever the compositor happens to do. They are kept because each is correct
    // where it applies and costs nothing where it is not.
    //
    // THE SCOPE IN THAT SENTENCE IS LOAD-BEARING AND USED TO BE MISSING, which made it a claim about
    // all three paths at once. On X11 two of the three work. On the layer path what sets
    // `skipTaskbar` is NOT established, so nothing here says this builder's `skip_taskbar` does
    // nothing there — see the option's own comment below, and `promote_to_layer_surface`.
    //
    // AND THE TABLE BELOW DESCRIBES THE FALLBACK PATH ONLY. It is a record of a TOPLEVEL probe under
    // two backends, which is exactly what the panel still is on X11 and on a Wayland compositor
    // without `zwlr_layer_shell_v1` — accurate there, and it stays. It says nothing about a layer
    // surface: on that path `toggle` promotes the window before its first `show`, and the surface
    // takes its stacking from `Layer::Overlay` and its position from anchor margins, so none of the
    // three hints below is what decides the behaviour even where one of them would have worked. The
    // fourth option, `.visible(false)`, is the one that matters MORE there — it is what leaves the
    // window unrealized so the promotion can happen at all. The only layer-surface reading of any of
    // these properties is the `skipTaskbar=true` in `promote_to_layer_surface`, and that one has no
    // established cause.
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
    // runtime setter this never reaches. xdg-shell has no stacking request, so on a Wayland
    // toplevel this call does nothing; on the layer path it is not what stacks the panel either,
    // `Layer::Overlay` is. It works on X11 and nowhere else.
    //
    // **AND IT IS NOT THIS OPTION THAT KEPT THE WAYLAND TOPLEVEL USABLE.** Measured on Plasma 6
    // BEFORE the layer-shell fix, when the panel was still a toplevel there: KWin activated it
    // because it had just been mapped, which is a compositor default and not something the app asks
    // for. In particular it was NOT `focus::present`, whose stamped path downcasts to `X11Window`
    // and returns on Wayland, leaving a bare `set_focus` that moves nothing. Losing that focus is
    // what `lib.rs` hides the panel on, so the popover contract survived on that path — resting on
    // a default rather than on any line here. Note where that was measured: the Wayland toplevel is
    // now reached only on a compositor without `zwlr_layer_shell_v1`, i.e. Mutter, where nothing
    // here has measured activation or blur at all. On the layer path the question does not arise at
    // all, because no focus event is delivered there to begin with (`promote_to_layer_surface`).
    .always_on_top(true)
    // `skip_taskbar` → `gtk_window_set_skip_taskbar_hint` PLUS `gtk_window_set_skip_pager_hint`
    // (tao `WindowRequest::SetSkipTaskbar`), both X11-only. So a Wayland TOPLEVEL takes a taskbar
    // button, which is wrong for a window that closes itself on a second click and is meant to close
    // itself on blur and on Esc — an entry offered for something there is nothing to switch to. That
    // is now the fallback path only: a layer surface is measured with `skipTaskbar=true`, though
    // what sets it there is not established and is not claimed to be this call
    // (`promote_to_layer_surface`).
    //
    // TWO CORRECTIONS TO WHAT THIS COMMENT USED TO CLAIM. It said "out of the taskbar AND the
    // window switcher". `skipSwitcher` measures `false` on BOTH backends, and the X11 probe was
    // confirmed reachable by the Walk-Through-Windows shortcut under stock `kwinrc`, so the Alt-Tab
    // half is not something this call delivers anywhere — the justification it was given ("one that
    // answers Alt-Tab is a window the user has to dismiss twice") never described shipped
    // behaviour. Staying out of the switcher needs the separate property, which is what
    // `xwaylandvideobridge` sets alongside this one. The taskbar half is delivered on X11 only.
    //
    // AND THE PARAGRAPH THAT USED TO FOLLOW IS SPENT. It said neither real fix was absorbable and
    // that #351 therefore stayed open rather than being closed here, counting gtk-layer-shell as a
    // new system build dependency across three packaging trees that must also run before the window
    // is realized. Both costs were paid instead: the dependency is in the packaging trees, and
    // `toggle` promotes the window between `build` and the first `show` for the realize order — so
    // this is the change that closes #351, and #370 with it. The other candidate,
    // `org_kde_plasma_surface.set_skip_taskbar`, is still exactly the right knob and still KDE-only
    // needing raw Wayland FFI for one call; not taken, and not needed while the layer path holds.
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

/// `(0, 0)` is how a host says it does not know where the pointer was — GNOME's extension is the
/// documented case. It is also a legitimate corner click, and treating a real corner click as
/// "unknown" costs nothing (the fallback corner is a few pixels away); treating "unknown" as a real
/// corner click puts the panel in the opposite corner of the screen. Applied ONCE, in [`place`],
/// before either half of the placement runs: `resolve_monitor_and_click` needs to know there is no
/// click to resolve a monitor from, `panel_origin` needs to know there is no click to place
/// against, and two readers deciding separately what counts as "none" is exactly the
/// two-places-computing-the-same-thing shape this codebase has been bitten by before.
fn real_click(at: Option<(i32, i32)>) -> Option<(i32, i32)> {
    at.filter(|&(x, y)| x != 0 || y != 0)
}

/// Which coordinate space a click turned out to be in, once [`resolve_click`] has decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickSpace {
    Logical,
    Physical,
}

/// One output as the placement arithmetic needs it — and the reason that arithmetic is exact rather
/// than an approximation. tao 0.35.3's `Monitor::position()` and `size()` take GDK's LOGICAL
/// geometry rect and `.to_physical(scale_factor())` it, each monitor with its own scale
/// (`platform_impl/linux/monitor.rs`). So `origin`/`size` here are physical, `origin / scale` and
/// `size / scale` hand GDK's own logical rect back exactly, and a global LOGICAL point becomes
/// global PHYSICAL by multiplying by the scale of the output it lands on — no per-monitor offset
/// arithmetic, because the physical origin was itself built by scaling the logical one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MonitorGeometry {
    origin: (f64, f64),
    size: (f64, f64),
    scale: f64,
}

impl MonitorGeometry {
    fn logical_rect(&self) -> ((f64, f64), (f64, f64)) {
        (
            (self.origin.0 / self.scale, self.origin.1 / self.scale),
            (self.size.0 / self.scale, self.size.1 / self.scale),
        )
    }

    fn physical_rect(&self) -> ((f64, f64), (f64, f64)) {
        (self.origin, self.size)
    }
}

/// Half-open on the far edges, so side-by-side outputs PARTITION the desktop instead of both
/// claiming the seam: a click at logical x=1920 belongs to the output starting there, not to the
/// 1920-wide one ending there. Inclusive bounds would give that seam to whichever output is listed
/// first — and they are what the single-monitor rule this replaces used, where the same `<=` read a
/// pair exactly at the logical width as inside a rectangle whose last column is one pixel earlier.
/// What half-open gives up in exchange is the desktop's outermost edge, and no host can produce a
/// pair there: a logical coordinate on a 1920-wide output runs 0..=1919.
fn rect_contains(rect: ((f64, f64), (f64, f64)), x: f64, y: f64) -> bool {
    let ((origin_x, origin_y), (width, height)) = rect;
    x >= origin_x && x < origin_x + width && y >= origin_y && y < origin_y + height
}

/// Decide which space a click is in and which output it is on, against the WHOLE monitor layout,
/// and return it promoted to PHYSICAL (#394).
///
/// MEASURED on Plasma 6 Wayland, one 3840×2160 output at scale 2: a real tray-icon click delivers
/// `Activate(1540, 1060)` — the icon's position in LOGICAL pixels — while a hand-made
/// `gdbus ... Activate 3192 2112` delivers PHYSICAL. So the space is a property of the CALLER, not
/// of the protocol, and cannot be assumed either way.
///
/// The rule is bounds, not a flag, and the bounds are the LAYOUT's, not one screen's: a click some
/// monitor's logical rectangle holds is logical and is promoted by THAT monitor's scale; a click no
/// logical rectangle holds but some physical one does was already physical; a click neither holds is
/// left alone for the caller's ladder. Click coordinates are global, so a bound that reads one
/// monitor's size without its origin answers for the first output only — a logical click at x=2500
/// on a second output to the right of a 1920-logical-wide first one exceeds that first width, reads
/// as physical, and is then placed against the wrong screen at half its intended coordinates. Not a
/// mixed-DPI edge case: two IDENTICAL outputs at scale 2, side by side, reach it.
///
/// **THE LOGICAL PASS RUNS FIRST, AND THE TIE IS A DECISION, NOT AN OVERSIGHT.** Inside the UNION of
/// every logical rectangle both readings are internally consistent — the pair carries nothing that
/// separates them — so a physical click there is read as logical. Which states the preserved region
/// exactly, and it is the complement rather than anything vaguer: a physical caller is honoured IFF
/// no logical rectangle holds its click. The measured `gdbus ... Activate 3192 2112` is outside the
/// single output's `[0, 1920) × [0, 1080)`, which is why that path works — not because physical
/// callers are preserved as such.
///
/// The cost has TWO shapes, and the second is not the first restated. On one output it changes the
/// SCALE: physical `(200, 200)` on the 3840×2160 scale-2 output above comes back `(400, 400)`, which
/// for a top-left tray places the panel 100 logical pixels from the click. Across outputs it changes
/// the OUTPUT, because the rectangle a click falls inside can belong to a NEIGHBOUR: with a
/// 5760×3240 scale-3 output at the origin and a 3840×2160 scale-2 one at physical x=3840 (logical
/// x=1920), a physical click at `(2000, 500)` is inside the first output's PHYSICAL rectangle and
/// the second's LOGICAL one, so it reads as logical on the second and is promoted to `(4000, 1000)`
/// — a panel on the wrong screen. Two identical scale-2 outputs side by side do the same to physical
/// `(2500, 500)`.
///
/// **Inverting the tie is worse, which is why it is not taken.** In that identical-outputs layout a
/// real LOGICAL click on the second output at `(2500, 500)` and a PHYSICAL click on the first at
/// `(2500, 500)` are the same pair of integers with two true stories. Preferring physical whenever
/// the two readings name different outputs would misread EVERY real tray click on a second output in
/// order to rescue the hand-made `gdbus` path: the measured real caller traded away for a debug one.
/// The tie goes to logical because the measured real host sends logical.
///
/// Breaking it at all needs a SECOND signal, and the platform all of this is measured on cannot
/// supply one: Wayland delivers pointer events per surface (`wl_pointer` enter/motion), so a client
/// cannot ask where the pointer is over another process's tray icon. X11 can answer it, which is
/// worse rather than better — a disambiguation that holds on one backend only is a second definition
/// of the same answer, and the two would disagree on exactly the clicks this function exists for.
///
/// The order settles an asymmetry too: logical rectangles are GDK's own layout, so outputs showing
/// DIFFERENT content do not overlap in them — a MIRRORED pair does, reporting one rectangle twice,
/// and the rule below covers it. Two PHYSICAL rectangles overlap for outputs showing different
/// content whenever both have scale > 1 and those scales differ: a 1920×1080-logical output at
/// scale 3 beside a 1920×1080-logical one at scale 2 placed at logical x=1920 puts physical
/// `(4000, 1500)` inside both physical rectangles and inside neither logical one. Either pass
/// therefore takes the FIRST containing output, deterministically, rather than pretending there is
/// a right answer to pick.
///
/// At scale 1 the two rectangles coincide, every on-screen click satisfies the logical bound, and
/// the promotion is a multiply by one — which is what makes all of this harmless on every desktop
/// that was already correct.
fn resolve_click(
    cx: f64,
    cy: f64,
    monitors: &[MonitorGeometry],
) -> (Option<usize>, ClickSpace, f64, f64) {
    if let Some(index) = monitors
        .iter()
        .position(|monitor| rect_contains(monitor.logical_rect(), cx, cy))
    {
        let scale = monitors[index].scale;
        return (Some(index), ClickSpace::Logical, cx * scale, cy * scale);
    }
    if let Some(index) = monitors
        .iter()
        .position(|monitor| rect_contains(monitor.physical_rect(), cx, cy))
    {
        return (Some(index), ClickSpace::Physical, cx, cy);
    }
    (None, ClickSpace::Physical, cx, cy)
}

/// Is there room for the panel BELOW the click — the same question as "is the click in the top half
/// of this monitor", and the same question again as "is the tray at the top of the desktop" without
/// having to know where the tray is. A click low on the screen has no room beneath it, so the panel
/// opens upward instead. The name states the RESULT, not the click: it is true for a click in the
/// TOP half, and a name saying the reverse reads as correct at the call site (`let below = …`) while
/// being backwards everywhere else.
fn panel_opens_below(cy: f64, monitor_origin_y: f64, monitor_height: f64) -> bool {
    cy - monitor_origin_y < monitor_height / 2.0
}

/// The panel's clamped, PHYSICAL top-left corner for a click — the pure arithmetic `place` applies
/// once it has a monitor to place against.
///
/// `click` arrives PHYSICAL and already sentinel-filtered. [`place`] drops the `(0, 0)` "unknown
/// pointer" pair with `real_click`, and `resolve_monitor_and_click` promotes what survives, both
/// before this runs; neither is re-derived here. That is the fix for what this doc used to claim:
/// disambiguating in both places meant two answers computed from different inputs — the whole
/// layout there, this one monitor's size here — which agree only while there is a single output.
/// `monitor_origin`/`monitor_area` are the chosen monitor's `position()`/`size()` and `scale` its
/// `scale_factor()`, all three physical by construction (tao's own `Monitor` converts a logical GDK
/// rect to physical before Tauri ever sees it).
///
/// `height` is the panel's intended LOGICAL height — [`place`]'s own `height` parameter, ultimately
/// `INTENDED_HEIGHT` — and THIS FUNCTION converts it to the physical size the arithmetic below needs,
/// from `WIDTH` (a compile-time constant, always positive) and `height` alone. That is deliberate,
/// not incidental: this function is pure and holds no window, so it structurally cannot read a size
/// back from one. Putting the conversion in [`place`] instead leaves a window in scope there for the
/// read to move back into; putting it here removes the window from that call entirely, which is what
/// makes the fix structural rather than merely current.
///
/// The disagreement a window read would reintroduce is MEASURED twice, and the two measurements
/// compound (#395, #401). `outer_size()` is not a size until the compositor has configured the
/// surface AT ALL: tao 0.35.3 keeps it in an atomic pair whose only writer is inside
/// `connect_configure_event`, seeded — in the line
/// `let o_size = window.window().map(|w| w.root_origin()).unwrap_or(w_pos);` — from the window's
/// ORIGIN. A position, not a size, with a position for its own fallback. On an unrealized window
/// that seed is `(0, 0)`, and MEASURED once #395's fix made this reachable: a click promoted to
/// physical `(3080, 2120)` placed the panel at logical `1540,1052` — left edge exactly the click,
/// top 8px above it, which is this arithmetic with both panel dimensions zero. It is ALSO not a size
/// after a RESIZE, until the next configure or allocation (#401) — which is exactly the moment
/// `place` runs from `resize`, so a stale-but-nonzero reading was just as reachable as a zero one,
/// one status poll after every mid-life height change.
fn panel_origin(
    click: Option<(f64, f64)>,
    monitor_origin: (f64, f64),
    monitor_area: (f64, f64),
    scale: f64,
    height: f64,
) -> (f64, f64) {
    let panel_size = {
        let physical = LogicalSize::new(WIDTH, height).to_physical::<f64>(scale);
        (physical.width, physical.height)
    };
    let margin = 8.0 * scale;
    let (x, y) = match click {
        Some((cx, cy)) => {
            // Centred on the click horizontally, and above or below it depending on which half of
            // the screen the indicator is in.
            let below = panel_opens_below(cy, monitor_origin.1, monitor_area.1);
            (
                cx - panel_size.0 / 2.0,
                if below {
                    cy + margin
                } else {
                    cy - panel_size.1 - margin
                },
            )
        }
        None => (
            monitor_origin.0 + monitor_area.0 - panel_size.0 - FALLBACK_INSET.0 * scale,
            monitor_origin.1 + FALLBACK_INSET.1 * scale,
        ),
    };

    // `max` before `clamp`: on a display shorter than the panel the upper bound goes below the
    // lower one and `clamp` panics. Not hypothetical at the tallest state on a 768px laptop screen
    // once the panel is 442 physical at scale 2.
    let max_x =
        (monitor_origin.0 + monitor_area.0 - panel_size.0 - margin).max(monitor_origin.0 + margin);
    let max_y =
        (monitor_origin.1 + monitor_area.1 - panel_size.1 - margin).max(monitor_origin.1 + margin);
    (
        x.clamp(monitor_origin.0 + margin, max_x),
        y.clamp(monitor_origin.1 + margin, max_y),
    )
}

/// The layout shape of a monitor Tauri reported. See [`MonitorGeometry`] for why those three fields
/// are the whole of it.
fn monitor_geometry(monitor: &tauri::Monitor) -> MonitorGeometry {
    MonitorGeometry {
        origin: (monitor.position().x as f64, monitor.position().y as f64),
        size: (monitor.size().width as f64, monitor.size().height as f64),
        scale: monitor.scale_factor(),
    }
}

/// Find the monitor to place the panel against on the FIRST open of a process, before the window has
/// ever been mapped (#395), and hand back the click promoted against it (#394). One call, because
/// they are one question: the promotion needs a scale, and the scale is the chosen output's.
///
/// MEASURED: `place`'s old ladder — `current_monitor()`, else `primary_monitor()` — logged "no
/// monitor to place the panel against" TWICE on every first open, because it was never really a
/// ladder. Reading tao 0.35.3's `platform_impl/linux/window.rs` shows `current_monitor()` ALREADY
/// falls back to `display.primary_monitor()` internally when the window has no mapped `GdkWindow`,
/// so the two-arm match collapsed to one call — and what that one call reaches,
/// `gdk_display_get_primary_monitor`, has no meaning on Wayland and returns `None` there. Nothing
/// could answer until the webview's first paint called `resize` → `place` a third time, by which
/// point the window was mapped and `current_monitor()` could finally see it.
///
/// `available_monitors()` does not have that problem: it asks the display, not the window, so it
/// answers before anything is realized — and it needs no click either, which is the rung the first
/// version of this fix left out. A first open whose host sends no pointer, or the `(0, 0)` sentinel,
/// skipped the display entirely and fell to the same two window calls, so #395's symptom survived on
/// precisely the path that has no click to resolve from.
///
///   1. With a click: [`resolve_click`] over every reported monitor. It names the output and the
///      space in one pass, which is why `monitor_from_point` is gone rather than kept as a rung: it
///      wants a LOGICAL point, and for a click some rectangle holds the layout arithmetic has
///      already named the output, while for a click no rectangle holds there is no scale to build
///      that point with — the dependency loop the old ladder was ordered around.
///   2. Otherwise `current_monitor()`, then `primary_monitor()`: authoritative once the window IS
///      mapped, which is every open after the first.
///   3. Then `available_monitors().first()` — a guess about WHICH output, but a real monitor with a
///      real scale, which is what a no-click first open needs to be placed at all instead of left at
///      `0,0` until the first paint.
///
/// The ladder's monitor re-enters [`resolve_click`] as a one-element layout instead of promoting the
/// click by hand, so the promotion keeps exactly one definition.
fn resolve_monitor_and_click<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    click: Option<(i32, i32)>,
) -> Option<(tauri::Monitor, Option<(f64, f64)>)> {
    // NOT `unwrap_or_default()`: that spells a display that could not be QUERIED and a desktop that
    // genuinely reports no outputs the same way, and the only line downstream of both is `place`'s
    // "no monitor to place the panel against" — which names neither. The empty list is still the
    // fallthrough, since the ladder below can answer without it.
    let monitors = match window.available_monitors() {
        Ok(monitors) => monitors,
        Err(err) => {
            eprintln!("tray: could not list the monitors ({err}); trying the window's own");
            Vec::new()
        }
    };
    if let Some((cx, cy)) = click {
        let geometries: Vec<MonitorGeometry> = monitors.iter().map(monitor_geometry).collect();
        let (index, _, px, py) = resolve_click(cx as f64, cy as f64, &geometries);
        if let Some(index) = index {
            return Some((monitors[index].clone(), Some((px, py))));
        }
    }
    let fallback = match window.current_monitor() {
        Ok(Some(monitor)) => Some(monitor),
        _ => window.primary_monitor().ok().flatten(),
    }
    .or_else(|| monitors.first().cloned())?;
    let promoted = click.map(|(cx, cy)| {
        let (_, _, px, py) = resolve_click(cx as f64, cy as f64, &[monitor_geometry(&fallback)]);
        (px, py)
    });
    Some((fallback, promoted))
}

/// Put the panel under the click, inside the screen — **by `set_position` on X11, by anchor margins
/// on a layer surface, and by neither on a Wayland compositor that has no layer shell.**
///
/// The function's FINAL line is `set_position`, which reaches GTK as `gtk_window_move` (tao 0.35
/// `WindowRequest::Position`). A Wayland client cannot position its own toplevel — xdg-shell has no
/// request for it — so the value is discarded and KWin places the panel by its own policy.
/// Measured, the same window under both backends asking for `400,300`: X11 landed at `400,300`,
/// Wayland at `840,443`, and elsewhere again on other runs. Not a fixed offset to correct for; the
/// absence of client positioning. That was #370, and it is fixed by the layer surface that also
/// closes #351.
///
/// **THE LAYER-SHELL BRANCH BELOW RETURNS BEFORE THAT LINE IS REACHED**, which is the whole fix: the
/// same clamped result leaves as anchor margins instead, and KWin then reports the surface where it
/// was asked to be — `geom=400,300` off `frameGeometry` for a `400,300` request, read out of
/// `workspace.windowList()` (`promote_to_layer_surface` has the full dump, and the probe bug that
/// once made this look unreadable).
///
/// So the arithmetic below is computed everywhere and consumed on two paths of the three: as margins
/// on the layer path, as `set_position` on X11. The one place it still goes nowhere is a Wayland
/// compositor without `zwlr_layer_shell_v1` (Mutter), where the last line is reached and discarded.
/// Left that way rather than worked around, because guessing an offset for a placement policy is how
/// a popover ends up wrong on every desktop instead of one.
///
/// The clamp is what makes this right on a bottom panel: a click at y=2112 on a 2160-tall screen has
/// no room for a 442px panel below it, so the panel goes above the click instead. That is the
/// ordinary KDE case, and the spec's fixed `top:40px` would have put it at the wrong end of the
/// screen on every one of them.
///
/// **Two conversions happen before any of that, and both were measured wrong once.** Which monitor
/// to place against cannot be answered by the window on the FIRST open of a process (#395), and the
/// click handed to this function is not reliably in either coordinate space (#394). One call
/// answers both — `resolve_monitor_and_click`, over `resolve_click` — because the promotion needs
/// the scale of the output the click landed on, so naming the output and naming the space is one
/// question asked once. Everything past that point stays physical, as it always was.
///
/// **Returns whether it placed the panel, and says nothing itself when it did not.** `show_at` calls
/// this twice on purpose — once before `show()`, once after, because only the second lands on a
/// compositor that ignores an unmapped window's position — so a message emitted here is emitted
/// twice for one open, which is #395's reported symptom and not a second fault. The caller owns the
/// report (`report_unplaced`), and that buys more than quiet: an open whose FIRST attempt fails and
/// whose second succeeds — the window is mapped by then, so `current_monitor()` can finally answer —
/// now says nothing at all, where a message from in here reported a failure for an open that ended
/// placed.
#[must_use]
fn place<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    at: Option<(i32, i32)>,
    height: f64,
) -> bool {
    // The panel is built hidden on purpose (showing it first would paint it at the default position
    // and then jump), so resolving the monitor without a mapped window is the ordinary path rather
    // than an edge case: it is what happens every time the panel opens for the first time in a
    // session. See `resolve_monitor_and_click` for why the old `current_monitor()`/
    // `primary_monitor()` ladder answered nothing here (#395).
    let Some((monitor, click)) = resolve_monitor_and_click(window, real_click(at)) else {
        return false;
    };
    // EVERYTHING FROM HERE ON IS PHYSICAL PIXELS: `monitor.position()` and `monitor.size()` are
    // already (tao's own `Monitor` converts a logical GDK rect to physical before Tauri ever sees
    // it), and so is the click — `resolve_monitor_and_click` promoted it against this very monitor
    // on the way out (#394; see [`resolve_click`]).
    let area = monitor.size();
    let origin = monitor.position();
    let scale = monitor.scale_factor();
    let (area, origin) = (
        tauri::PhysicalSize::new(area.width as f64, area.height as f64),
        tauri::PhysicalPosition::new(origin.x as f64, origin.y as f64),
    );

    // `height` PASSES STRAIGHT THROUGH, and `place` computes no size of its own — see
    // [`panel_origin`]'s doc for why the conversion lives there rather than here: it is the caller's
    // intended LOGICAL height (what `resize` just requested, or `show_at`'s `INTENDED_HEIGHT`),
    // never a value read back from the window (#401).
    let (x, y) = panel_origin(
        click,
        (origin.x, origin.y),
        (area.width, area.height),
        scale,
        height,
    );

    // A LAYER SURFACE IS POSITIONED BY MARGIN, NOT BY `set_position` (#370). Same arithmetic above,
    // two conversions on the way out, and both are the kind that look like a positioning bug:
    //
    //   * MARGINS ARE PER-OUTPUT. `x`/`y` are global coordinates across all monitors; the anchor is
    //     this surface's own output, so the monitor origin comes back off. On a single display that
    //     is a no-op, which is exactly why it would ship broken and only fail on a second monitor.
    //   * MARGINS ARE LOGICAL PIXELS. Everything above is physical on purpose — `x`/`y` are
    //     `panel_origin`'s result, and the click it started from has already been promoted to
    //     physical by then regardless of which space it arrived in (#394, `resolve_click`) —
    //     so this divides by the scale factor. The same conversion, in the same direction, that the
    //     module doc's "Position" section records getting wrong the first time.
    #[cfg(target_os = "linux")]
    if let Ok(gtk_window) = window.gtk_window() {
        // Asking the window rather than remembering a flag: `promote_to_layer_surface` can decline
        // (X11, or Mutter), and a second source of truth about which kind of surface this is would
        // disagree with the compositor on exactly the desktops the decline exists for.
        if gtk_layer_shell::LayerShell::is_layer_window(&gtk_window) {
            let left = ((x - origin.x) / scale).round() as i32;
            let top = ((y - origin.y) / scale).round() as i32;
            // `set_layer_shell_margin`, not `set_margin`: the latter is GTK's own widget margin,
            // which the crate renames around precisely so this call cannot be written by accident.
            use gtk_layer_shell::{Edge, LayerShell};
            gtk_window.set_layer_shell_margin(Edge::Left, left);
            gtk_window.set_layer_shell_margin(Edge::Top, top);
            return true;
        }
    }

    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    true
}

/// The one definition of what "nothing could place the panel" says, so the two callers cannot word
/// it differently — and so it is said once per OPEN rather than once per [`place`].
fn report_unplaced() {
    eprintln!("tray: no monitor to place the panel against; leaving it where it is");
}

#[cfg(test)]
mod tests {
    use super::*;

    // The one monitor every #394 measurement below is against: Plasma 6 Wayland, one 3840×2160
    // physical output at scale 2 (1920×1080 logical), positioned at the desktop origin.
    const MONITOR_ORIGIN: (f64, f64) = (0.0, 0.0);
    const MONITOR_AREA: (f64, f64) = (3840.0, 2160.0);
    const MONITOR_SCALE: f64 = 2.0;
    // The panel mid-resize: 317 logical, the height `panel_origin` now takes directly. WIDTH is
    // the module const (362) it derives the other physical dimension from — 362×317 logical is
    // 724×634 physical at scale 2, the numbers these tests used to pass as a size pair.
    const PANEL_HEIGHT: f64 = 317.0;

    fn measured_output() -> MonitorGeometry {
        MonitorGeometry {
            origin: MONITOR_ORIGIN,
            size: MONITOR_AREA,
            scale: MONITOR_SCALE,
        }
    }

    /// The measured output with a second IDENTICAL one to its right: physical origin 3840, logical
    /// origin 1920, same scale. Same DPI, same size — the layout a bound that reads a size without
    /// an origin gets wrong anyway, because click coordinates are global and carry that offset.
    fn two_identical_outputs() -> Vec<MonitorGeometry> {
        vec![
            measured_output(),
            MonitorGeometry {
                origin: (3840.0, 0.0),
                size: MONITOR_AREA,
                scale: MONITOR_SCALE,
            },
        ]
    }

    #[test]
    fn a_real_tray_click_is_recognised_as_logical_and_promoted_to_physical() {
        let (index, space, x, y) = resolve_click(1540.0, 1060.0, &[measured_output()]);
        assert_eq!((index, space), (Some(0), ClickSpace::Logical));
        assert_eq!((x, y), (3080.0, 2120.0));

        // The promoted click is in the BOTTOM half of a 2160-tall screen, not the top — the second
        // half of #394: an unpromoted click compares against half the physical height and gets this
        // backwards, so the panel opens downward off the bottom of the screen.
        assert!(!panel_opens_below(y, MONITOR_ORIGIN.1, MONITOR_AREA.1));

        let origin = panel_origin(
            Some((x, y)),
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            PANEL_HEIGHT,
        );
        assert_eq!(origin, (2718.0, 1470.0));
        // = logical (1359, 735).
        assert_eq!(
            (origin.0 / MONITOR_SCALE, origin.1 / MONITOR_SCALE),
            (1359.0, 735.0)
        );
    }

    #[test]
    fn a_hand_made_physical_click_is_recognised_as_physical_and_not_rescaled() {
        // `gdbus ... Activate 3192 2112` — a caller that already sends physical units. Unchanged
        // for THIS click, and the reason is the click rather than the caller: `(3192, 2112)` is
        // outside the one output's logical rectangle `[0, 1920) × [0, 1080)`, which is the whole of
        // what the physical branch preserves. A physical click inside that rectangle is read as
        // logical instead — see the two deliberate-tie tests below.
        let (index, space, x, y) = resolve_click(3192.0, 2112.0, &[measured_output()]);
        assert_eq!((index, space), (Some(0), ClickSpace::Physical));
        assert_eq!((x, y), (3192.0, 2112.0));

        let origin = panel_origin(
            Some((x, y)),
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            PANEL_HEIGHT,
        );
        assert_eq!(origin, (2830.0, 1462.0));
        // = logical (1415, 731).
        assert_eq!(
            (origin.0 / MONITOR_SCALE, origin.1 / MONITOR_SCALE),
            (1415.0, 731.0)
        );
    }

    #[test]
    fn at_scale_one_the_disambiguation_is_a_no_op() {
        let output = MonitorGeometry {
            origin: (0.0, 0.0),
            size: (1920.0, 1080.0),
            scale: 1.0,
        };

        // Inside the screen either way: classified logical, but at scale 1 "promoted to physical"
        // multiplies by 1 — the same numbers the physical branch would have produced unscaled.
        let (index, space, x, y) = resolve_click(1540.0, 1060.0, &[output]);
        assert_eq!((index, space), (Some(0), ClickSpace::Logical));
        assert_eq!((x, y), (1540.0, 1060.0));

        // Outside the screen either way: no output holds it, and at scale 1 leaving it alone is
        // also a no-op — covering both branches is the point, not just the one a typical click
        // takes.
        let (index, space, x, y) = resolve_click(2500.0, 500.0, &[output]);
        assert_eq!((index, space), (None, ClickSpace::Physical));
        assert_eq!((x, y), (2500.0, 500.0));
    }

    #[test]
    fn a_logical_click_on_a_second_identical_output_is_promoted_by_that_output_not_the_first() {
        // Two 3840×2160 outputs at scale 2 side by side. A logical click at x=2500 is on the
        // SECOND one, 580 logical pixels in. Bounding it by the first output's size alone reads
        // 2500 > 1920 as "already physical", leaves it unpromoted, and places the panel against the
        // first screen at half the intended coordinates — with no mixed DPI anywhere in sight.
        let outputs = two_identical_outputs();
        let (index, space, x, y) = resolve_click(2500.0, 500.0, &outputs);
        assert_eq!((index, space), (Some(1), ClickSpace::Logical));
        assert_eq!((x, y), (5000.0, 1000.0));

        let second = outputs[1];
        let origin = panel_origin(
            Some((x, y)),
            second.origin,
            second.size,
            second.scale,
            PANEL_HEIGHT,
        );
        assert_eq!(origin, (4638.0, 1016.0));
        // The whole point: the panel lands on the second output, not the first.
        assert!(origin.0 >= second.origin.0);
    }

    #[test]
    fn a_logical_click_on_a_second_output_with_its_own_scale_is_promoted_by_that_scale() {
        // A scale-2 output at the origin with a scale-1 one to its right (logical x=1920, so
        // physical x=1920 for an output tao scales by 1). The promotion is the SECOND output's
        // multiply, not the first's — one shared scale would double a click that needs no
        // promotion at all.
        let outputs = [
            measured_output(),
            MonitorGeometry {
                origin: (1920.0, 0.0),
                size: (1920.0, 1080.0),
                scale: 1.0,
            },
        ];
        let (index, space, x, y) = resolve_click(2500.0, 500.0, &outputs);
        assert_eq!((index, space), (Some(1), ClickSpace::Logical));
        assert_eq!((x, y), (2500.0, 500.0));
        assert_ne!((x, y), (5000.0, 1000.0));
    }

    #[test]
    fn a_physical_click_only_the_second_outputs_physical_rectangle_holds_names_that_output() {
        // x=5000 is past both logical rectangles (which end at 3840) and inside the second output's
        // physical one, which starts there.
        let (index, space, x, y) = resolve_click(5000.0, 1500.0, &two_identical_outputs());
        assert_eq!((index, space), (Some(1), ClickSpace::Physical));
        assert_eq!((x, y), (5000.0, 1500.0));
    }

    #[test]
    fn a_physical_click_inside_a_logical_rectangle_is_read_as_logical_by_a_deliberate_tie() {
        // The residual #394 cannot remove: inside a logical rectangle both readings are internally
        // consistent, so a physical caller whose click lands there is promoted anyway — physical
        // (200, 200) on the scale-2 output comes back (400, 400). The tie goes to logical because
        // the measured real host sends logical; breaking it needs the pointer's own position, which
        // Wayland delivers per surface and never for another process's tray icon. Pinned so the
        // cost is a recorded decision rather than a passing bug.
        let (index, space, x, y) = resolve_click(200.0, 200.0, &[measured_output()]);
        assert_eq!((index, space), (Some(0), ClickSpace::Logical));
        assert_eq!((x, y), (400.0, 400.0));
    }

    #[test]
    fn a_physical_click_inside_a_neighbours_logical_rectangle_moves_output_by_the_same_tie() {
        // The tie above, one output further out, and it costs more there: the rectangle a physical
        // click falls inside can belong to a DIFFERENT output, so the misread changes which screen
        // the panel opens on and not just its scale.
        //
        // A 5760×3240 scale-3 output at the origin (logical 1920×1080) with a 3840×2160 scale-2 one
        // at physical x=3840 (logical x=1920). A physical click at (2000, 500) is inside the FIRST
        // output's physical rectangle — a real place for a physical caller to click — and inside
        // the SECOND's logical rectangle [1920, 3840). Logical wins, so it is promoted by the
        // second output's scale and placed there.
        let outputs = [
            MonitorGeometry {
                origin: (0.0, 0.0),
                size: (5760.0, 3240.0),
                scale: 3.0,
            },
            MonitorGeometry {
                origin: (3840.0, 0.0),
                size: (3840.0, 2160.0),
                scale: 2.0,
            },
        ];
        assert!(rect_contains(outputs[0].physical_rect(), 2000.0, 500.0));
        assert!(rect_contains(outputs[1].logical_rect(), 2000.0, 500.0));

        let (index, space, x, y) = resolve_click(2000.0, 500.0, &outputs);
        assert_eq!((index, space), (Some(1), ClickSpace::Logical));
        assert_eq!((x, y), (4000.0, 1000.0));
    }

    #[test]
    fn two_true_stories_for_one_pair_of_integers_is_why_the_tie_is_not_inverted() {
        // The same call as `a_logical_click_on_a_second_identical_output_…`, asserting the other
        // half of it: that one pins the fix, this one pins what the fix cannot know. Two identical
        // scale-2 outputs side by side, and the SAME pair reaches this function from two callers
        // with two correct readings — a real LOGICAL click on the second output, and a PHYSICAL
        // click on the first. Nothing in the pair separates them.
        let outputs = two_identical_outputs();
        assert!(rect_contains(outputs[1].logical_rect(), 2500.0, 500.0));
        assert!(rect_contains(outputs[0].physical_rect(), 2500.0, 500.0));

        let (index, space, x, y) = resolve_click(2500.0, 500.0, &outputs);
        assert_eq!((index, space), (Some(1), ClickSpace::Logical));
        assert_eq!((x, y), (5000.0, 1000.0));

        // Inverting the tie would answer the physical caller and misread every real tray click on a
        // second output — the measured real caller traded away for the hand-made `gdbus` one. So
        // this assertion is the decision, not the bug: the logical reading wins, and what it costs
        // the physical sender is written at `resolve_click`.
    }

    #[test]
    fn a_click_on_the_seam_between_two_outputs_belongs_to_the_one_starting_there() {
        // Half-open far edges: logical x=1920 is the second output's first column, not the first
        // output's last. Inclusive bounds hand every seam click to whichever output is listed
        // first, which is the wrong screen half the time.
        let (index, space, x, y) = resolve_click(1920.0, 500.0, &two_identical_outputs());
        assert_eq!((index, space), (Some(1), ClickSpace::Logical));
        assert_eq!((x, y), (3840.0, 1000.0));
    }

    #[test]
    fn a_click_no_output_holds_names_no_monitor_and_is_left_unpromoted() {
        // Neither rectangle holds it, so there is no scale to promote by and no output to name.
        // The caller's ladder picks a monitor and the clamp does the rest.
        let (index, space, x, y) = resolve_click(9000.0, 4000.0, &two_identical_outputs());
        assert_eq!((index, space), (None, ClickSpace::Physical));
        assert_eq!((x, y), (9000.0, 4000.0));
    }

    #[test]
    fn overlapping_physical_rectangles_take_the_first_containing_output() {
        // Physical rectangles do not tile: a 1920×1080-logical output at scale 3 beside a
        // 1920×1080-logical one at scale 2 (logical x=1920, so physical x=3840) both hold physical
        // (4000, 1500), and neither logical rectangle does. First containing output wins, which is
        // a deterministic answer rather than a right one — the reason the logical pass runs first.
        let outputs = [
            MonitorGeometry {
                origin: (0.0, 0.0),
                size: (5760.0, 3240.0),
                scale: 3.0,
            },
            MonitorGeometry {
                origin: (3840.0, 0.0),
                size: (3840.0, 2160.0),
                scale: 2.0,
            },
        ];
        assert!(rect_contains(outputs[0].physical_rect(), 4000.0, 1500.0));
        assert!(rect_contains(outputs[1].physical_rect(), 4000.0, 1500.0));
        let (index, space, x, y) = resolve_click(4000.0, 1500.0, &outputs);
        assert_eq!((index, space), (Some(0), ClickSpace::Physical));
        assert_eq!((x, y), (4000.0, 1500.0));
    }

    #[test]
    fn mirrored_outputs_report_one_logical_rectangle_twice_and_the_first_one_wins() {
        // The exception to "outputs showing different content do not overlap in logical space".
        // A mirrored pair occupies the same logical rectangle from both outputs, so containment
        // names two — and with different scales they promote differently, which is what makes
        // "first containing" a decision here rather than a tie between identical answers.
        let outputs = [
            measured_output(),
            MonitorGeometry {
                origin: (0.0, 0.0),
                size: (1920.0, 1080.0),
                scale: 1.0,
            },
        ];
        let (index, space, x, y) = resolve_click(1540.0, 1060.0, &outputs);
        assert_eq!((index, space), (Some(0), ClickSpace::Logical));
        assert_eq!((x, y), (3080.0, 2120.0));
        // The second output would have left it alone. Same rectangle, different answer.
        assert_ne!((x, y), (1540.0, 1060.0));
    }

    #[test]
    fn the_panel_opens_below_a_click_in_the_top_half_and_above_one_in_the_bottom_half() {
        // The predicate answers about the PANEL, not the click. A click 100px down has room
        // beneath it; one 100px from the bottom does not.
        assert!(panel_opens_below(100.0, MONITOR_ORIGIN.1, MONITOR_AREA.1));
        assert!(!panel_opens_below(2060.0, MONITOR_ORIGIN.1, MONITOR_AREA.1));
        // And it is measured from the output's own origin, so a second output stacked below the
        // first answers about its own halves rather than the desktop's.
        assert!(panel_opens_below(2260.0, 2160.0, MONITOR_AREA.1));
    }

    /// The one arm of the ladder a headless test can reach: `tauri::test`'s mock window answers
    /// `Ok(None)` to `current_monitor`/`primary_monitor` and `Ok(vec![])` to `available_monitors`,
    /// so it is the desktop where nothing can be resolved. What it pins is that the function ANSWERS
    /// there — `None`, and [`place`] `false` rather than a message of its own — rather than indexing
    /// an empty layout or unwrapping a missing monitor. The rungs that need a real output (which
    /// monitor, and the `available_monitors().first()` fallback #395's no-click path turns on)
    /// cannot be driven by this mock, and are covered by `resolve_click`'s own tests plus the
    /// measurements in the docs.
    ///
    /// Nor can the other half of the report-once change: `show_at` reports only when BOTH of its
    /// `place` calls fail, and driving that needs `show()` on a real window. What is pinned here is
    /// the value that decision reads — `place` answering `false` without saying anything — and the
    /// composition above it is three lines in one caller.
    #[test]
    fn a_display_that_reports_no_monitors_at_all_answers_none_instead_of_panicking() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build");
        let window = tauri::WebviewWindowBuilder::new(
            &app,
            "monitorless",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .build()
        .expect("mock webview window should build");

        assert!(resolve_monitor_and_click(&window, Some((1540, 1060))).is_none());
        assert!(resolve_monitor_and_click(&window, None).is_none());

        // And `place` reports that as a return value, which is what lets `show_at` say it once per
        // open instead of once per attempt.
        assert!(!place(&window, Some((1540, 1060)), HEIGHT));
        assert!(!place(&window, None, HEIGHT));
    }

    #[test]
    fn regression_395_a_zero_height_would_place_the_panel_on_the_click_instead_of_above_it() {
        // MEASURED with the guard absent, on the build before #401's fix: a logical click at
        // (1540, 1060) promoted to physical (3080, 2120) placed the panel at logical 1540,1052 —
        // its left edge exactly the click and its top 8px above it, the arithmetic with a
        // zero-sized panel (`usable_panel_size` read `outer_size()`'s `(0, 0)` and had a WIDTH of
        // zero to work with, same as the height).
        //
        // `panel_origin` cannot reproduce the WIDTH half of that any more, structurally: it derives
        // the physical width from `WIDTH`, a compile-time constant that is never zero, so the only
        // degenerate input left to construct is a zero HEIGHT — passed directly now, not smuggled
        // in as a size pair. That is what this test constructs, and why its `x` no longer matches
        // the click the way the original bug's did: `x` does not depend on `height` at all any
        // more, so it is identical whether the panel is well-formed or zero-height. Only `y` still
        // exhibits the old bug's shape, which is what stays worth pinning as what the design now
        // structurally excludes.
        let (_, _, cx, cy) = resolve_click(1540.0, 1060.0, &[measured_output()]);
        let zeroed = panel_origin(
            Some((cx, cy)),
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            0.0,
        );
        let placed = panel_origin(
            Some((cx, cy)),
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            HEIGHT,
        );
        assert_ne!(zeroed, placed);
        // `x` is identical either way — the structural guarantee the move buys.
        assert_eq!(zeroed.0, placed.0);
        // The zero-height answer is the measured bug's surviving half, in logical pixels:
        // 2718/2 = 1359 (not the click's 1540 any more), 2104/2 = 1052 (still 8px above the click).
        assert_eq!(zeroed, (2718.0, 2104.0));
        // The real height leaves the panel's full height above the click.
        let real_height_physical = HEIGHT * MONITOR_SCALE;
        assert!(placed.1 + real_height_physical <= cy);
    }

    #[test]
    fn regression_401_panel_origin_derives_its_size_from_the_height_it_is_given() {
        // The move that fixes #401: `panel_origin` computes the panel's physical size from the
        // `height` it is given, not from a size handed in by a caller that might be reading one
        // back from a window — so the SAME click, placed for two different heights, lands at two
        // different origins, and `place` (which holds the window) has no size expression left to
        // put a stale read back into.
        //
        // MEASURED, Plasma 6 / KWin 6.7.5, `main` @ 04d615f (#401): the panel opened at HEIGHT
        // (442), corrected to 302 on first paint, then the webview reported two more states while
        // the panel stayed open — 365, then 317. `place` used to read the panel's height back from
        // `outer_size()`, a cache the compositor had not caught up on yet, so each of those two
        // mid-life changes placed the panel for the height it had just LEFT rather than the one it
        // had just SET — 63px off, then 48px off, matching the issue's measured deltas exactly.
        //
        // The panel keeps its BOTTOM edge fixed against the click when it opens upward
        // (`y = cy - panel_size.1 - margin`, and `panel_size.1` now comes from `height` alone), so
        // placing for height `h` instead of the correct `h'` moves the top by exactly `h' - h` —
        // which is why the two offsets equal the two height deltas exactly, not approximately.
        let click = Some((1415.0, 1200.0)); // opens upward: in the bottom half of the monitor.
        let y_for = |height: f64| {
            panel_origin(
                click,
                MONITOR_ORIGIN,
                MONITOR_AREA,
                1.0, // scale 1: these deltas are LOGICAL, matching the issue's `frameGeometry` rows.
                height,
            )
            .1
        };

        // The structural property the move buys: the same click placed for two different heights
        // lands at two different origins, because `panel_origin` derives the size itself.
        assert_ne!(y_for(302.0), y_for(365.0));
        assert_ne!(y_for(365.0), y_for(317.0));

        // 302 -> 365: placing for the height just left (302) instead of the height just set (365)
        // — the #401 bug — offsets the panel by 63px, the issue's first measured delta.
        assert_eq!((y_for(302.0) - y_for(365.0)).abs(), 63.0);
        // 365 -> 317: the second mid-life change, offset by 48px, the issue's second measured delta.
        assert_eq!((y_for(365.0) - y_for(317.0)).abs(), 48.0);
    }

    #[test]
    fn the_intended_height_round_trips_and_is_seeded_to_height() {
        // `INTENDED_HEIGHT` is the one place the panel's intended height lives — `show_at` reads it
        // for both of its `place` calls, and `resize` is its only writer. Seeded to `HEIGHT`, the
        // height the panel opens at before any `resize` has run.
        assert_eq!(intended_height(), HEIGHT);
        remember_height(302.0);
        assert_eq!(intended_height(), 302.0);
        remember_height(441.5);
        assert_eq!(intended_height(), 441.5);
        // Restore the seed: this static is process-global, and other tests assume it.
        remember_height(HEIGHT);
        assert_eq!(intended_height(), HEIGHT);
    }

    #[test]
    fn an_unknown_pointer_takes_the_specs_fallback_corner_in_logical_pixels() {
        // The sentinel is filtered once, in `place`, before anything resolves or places — so the
        // composition is what has to answer here, `panel_origin` no longer filtering for itself.
        assert_eq!(real_click(Some((0, 0))), None);
        let with_sentinel = panel_origin(
            real_click(Some((0, 0))).map(|(x, y)| (x as f64, y as f64)),
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            PANEL_HEIGHT,
        );
        let with_none = panel_origin(
            None,
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            PANEL_HEIGHT,
        );
        assert_eq!(with_sentinel, with_none);
        // A real corner click is not the sentinel and is placed against, not fallen back from.
        assert_eq!(real_click(Some((0, 1))), Some((0, 1)));
        // The spec's corner is `right:16px; top:40px` in LOGICAL pixels, so on this scale-2 output
        // it is physical (3084, 80) — and the inset scales with everything else rather than being
        // added raw to a physical coordinate, which put the panel 8 logical pixels from the right
        // edge instead of 16.
        assert_eq!(with_none, (3084.0, 80.0));
        assert_eq!(
            with_none,
            (
                MONITOR_AREA.0 - WIDTH * MONITOR_SCALE - FALLBACK_INSET.0 * MONITOR_SCALE,
                FALLBACK_INSET.1 * MONITOR_SCALE
            )
        );
        // Read back in logical pixels it is the spec, and the panel's right edge is 16 logical
        // pixels in from the 1920-wide logical screen.
        let logical = (with_none.0 / MONITOR_SCALE, with_none.1 / MONITOR_SCALE);
        assert_eq!(logical, (1542.0, 40.0));
        let logical_screen_width = MONITOR_AREA.0 / MONITOR_SCALE;
        let panel_right_edge = logical.0 + WIDTH;
        assert_eq!(logical_screen_width - panel_right_edge, FALLBACK_INSET.0);
        assert_eq!(logical.1, FALLBACK_INSET.1);
    }

    #[test]
    fn regression_394_a_bottom_corner_click_places_the_panel_just_above_it_not_below() {
        // What the user actually reported: the panel landed mid-screen instead of beside the tray,
        // and opened downward instead of up. Under the OLD "always physical" reading this same click
        // (1540, 1060) would compare against half the 2160-tall screen UNSCALED — 1060 < 1080 reads
        // as the top half — and the panel would open BELOW the click, at physical y = 1060 + 16 =
        // 1076 (logical ~538, ~mid-screen). The fix must place it ABOVE the click instead, hugging
        // its bottom edge.
        let (_, _, cx, cy) = resolve_click(1540.0, 1060.0, &[measured_output()]);
        let (x, y) = panel_origin(
            Some((cx, cy)),
            MONITOR_ORIGIN,
            MONITOR_AREA,
            MONITOR_SCALE,
            PANEL_HEIGHT,
        );
        let margin = 8.0 * MONITOR_SCALE;

        // The panel's bottom edge sits `margin` above the (promoted) click, not ~1000px below it.
        assert_eq!(y + PANEL_HEIGHT * MONITOR_SCALE, cy - margin);
        assert!(y + PANEL_HEIGHT * MONITOR_SCALE < cy);
        assert_eq!((x, y), (2718.0, 1470.0));
    }
}
