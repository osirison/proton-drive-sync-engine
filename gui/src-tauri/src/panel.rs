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
//! `Activate(x, y)` carries the click in screen coordinates — verified on a live session, where a
//! click on the indicator arrived as `Activate(3192, 2112)`. So the spec's `top:40px; right:16px`
//! fallback is only that: a fallback, for a host that sends `(0, 0)` because it does not track the
//! pointer. The panel is placed against the click and then clamped into the work area, which is what
//! makes it open UPWARD on a bottom panel — the ordinary case on KDE, and the one a fixed
//! top-right rule gets wrong on every Plasma desktop.
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
///
/// On the layer path the height correction does not land while the panel is up — the measurement is
/// on `resize_layer_surface` — so there is currently no gap to close there: the re-place still runs
/// and, the height being unchanged, recomputes the same margins it last set. What margins are
/// measured to do is position the surface AT OPEN; whether changing one moves an already-mapped
/// surface was not part of that measurement.
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
/// **On the layer path the correction does not land while the panel is up** — measured, see
/// `resize_layer_surface` — so the panel is this tall for as long as it is visible and the
/// corrected height only appears after a close and a reopen. On X11, and on a Wayland compositor
/// without `zwlr_layer_shell_v1`, the correction works as written.
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
    place(window, at);
    let _ = window.show();
    place(window, at);
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
            let _ = window.set_size(LogicalSize::new(WIDTH, height));
            // A LAYER SURFACE DOES NOT RESIZE FROM `set_size` — see `resize_layer_surface`, which
            // also carries the measurement showing that NEITHER call resizes a panel that is
            // already mapped. So on the layer path both size calls — the `set_size` above and the
            // `resize_layer_surface` below — are currently inert: the panel stays at `HEIGHT` until
            // it is closed and reopened. The `place` at the end of this function still runs, and
            // with the height unchanged it recomputes the same margins it last set, so nothing is
            // riding on the separate question of whether a margin change moves an ALREADY-MAPPED
            // surface — measured is that margins position it at open, and the resize probe did not
            // cover the other. On X11, and on Wayland without `zwlr_layer_shell_v1`, the `set_size`
            // is the call that works and this branch is not taken. Kept as the path the size has to
            // travel if the mapped-resize gap is closed.
            #[cfg(target_os = "linux")]
            if let Ok(gtk_window) = window.gtk_window() {
                if gtk_layer_shell::LayerShell::is_layer_window(&gtk_window) {
                    resize_layer_surface(&gtk_window, height);
                }
            }
            // The window shrank around a fixed top-left; put it back against the click it opened
            // from, or it hangs in space above the tray.
            place(&window, anchor());
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
/// **MEASURED: NEITHER CALL RESIZES A PANEL THAT IS ALREADY MAPPED.** Plasma 6 Wayland, a clean
/// probe replicating `resize` exactly — tao's `set_size` (GTK `resize`), then this
/// `set_size_request`:
///
/// ```text
/// RZ2 issued resize -> 362x250
/// RZ2 after alloc=362x442        <- GTK unchanged
/// KWin  geom=400,300 362x442     <- compositor unchanged
/// ```
///
/// `queue_resize()` does not help either. So on the layer path the panel opens at `HEIGHT` and the
/// webview's first-paint correction never lands while the panel is visible; the corrected height
/// arrives only after a close and a reopen. Nothing is worked around here, because nothing measured
/// says which call would land — and it is why the paragraph above no longer calls this the size that
/// gets the panel on screen at all.
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

    // A LAYER SURFACE IS POSITIONED BY MARGIN, NOT BY `set_position` (#370). Same arithmetic above,
    // two conversions on the way out, and both are the kind that look like a positioning bug:
    //
    //   * MARGINS ARE PER-OUTPUT. `x`/`y` are global coordinates across all monitors; the anchor is
    //     this surface's own output, so the monitor origin comes back off. On a single display that
    //     is a no-op, which is exactly why it would ship broken and only fail on a second monitor.
    //   * MARGINS ARE LOGICAL PIXELS. Everything above is physical on purpose — the click arrives
    //     physical — so this divides by the scale factor. The same conversion, in the same
    //     direction, that the comment above records getting wrong the first time.
    #[cfg(target_os = "linux")]
    if window
        .gtk_window()
        .is_ok_and(|w| gtk_layer_shell::LayerShell::is_layer_window(&w))
    {
        // Asking the window rather than remembering a flag: `promote_to_layer_surface` can decline
        // (X11, or Mutter), and a second source of truth about which kind of surface this is would
        // disagree with the compositor on exactly the desktops the decline exists for.
        if let Ok(gtk_window) = window.gtk_window() {
            let left = ((x - origin.x) / scale).round() as i32;
            let top = ((y - origin.y) / scale).round() as i32;
            // `set_layer_shell_margin`, not `set_margin`: the latter is GTK's own widget margin,
            // which the crate renames around precisely so this call cannot be written by accident.
            use gtk_layer_shell::{Edge, LayerShell};
            gtk_window.set_layer_shell_margin(Edge::Left, left);
            gtk_window.set_layer_shell_margin(Edge::Top, top);
            return;
        }
    }

    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}
