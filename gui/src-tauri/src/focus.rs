//! Asking for the keyboard focus in a way the compositor will grant (S8).
//!
//! # `set_focus()` is a request, and on a live KDE session it was being refused
//!
//! Every window this app raises is raised because the user clicked the tray indicator — and that
//! click goes to **plasmashell**, not to us. So at the moment we ask, the timestamp our request
//! carries is whatever GTK last saw an event with in this process, which is older than the click.
//! That is exactly the shape KWin's focus-stealing prevention exists to refuse: measured on Plasma
//! 6.7/X11, the panel came up carrying `_NET_WM_STATE_DEMANDS_ATTENTION` — the WM's marker for "this
//! window asked to be focused and I said no" — while the keyboard focus stayed where it was.
//!
//! Two things then went wrong, and they are the same seam seen from both sides:
//!
//!   * **The panel lingered.** `lib.rs` hides it on a blur that follows a focus, deliberately, so a
//!     compositor that refuses focus cannot make the panel flash and vanish. Refused focus means no
//!     focus, so no blur ever counts: an always-on-top borderless popover stayed over the user's
//!     work until they clicked the indicator again. IMPLEMENTATION-PLAN §6's second sub-risk, met
//!     on the ORDINARY path rather than an edge case.
//!   * **`Open Drive Sync` did nothing.** With the main window already open behind other windows,
//!     the row raised nothing and focused nothing — the stacking order was byte-identical before and
//!     after. The tray's most-used row, silently inert, on the desktop the design names first.
//!
//! # The fix is the timestamp, not the request
//!
//! `gtk_window_present_with_time` carries the timestamp into the `_NET_ACTIVE_WINDOW` message, and a
//! timestamp at least as new as the user's last input is what KWin compares against.
//! `gdk_x11_get_server_time` is the supported way to get one: it appends a zero-length property to a
//! window and reads the time off the `PropertyNotify` that comes back, so the value is the X
//! server's own clock rather than anything this process guessed.
//!
//! **This is not the focus stealing the plan warns about.** The prohibition is on a window that
//! raises itself over someone's work unbidden; nothing here runs except from a click on the
//! indicator or a row in its menu. What the timestamp restores is the click's provenance, which was
//! being lost on the way through the bus.
//!
//! # Why it runs from an idle callback
//!
//! `gdk_x11_get_server_time` needs a window that is realized AND selecting `PropertyNotify`, or it
//! waits for an event that will never arrive — the one failure mode here that would be worse than
//! the bug. A mapped GTK toplevel is both; an unrealized one is neither. And the panel is not mapped
//! yet when the caller asks: `WebviewWindow::show()` posts a message to the event loop rather than
//! calling GTK, so `gtk_window().window()` is still `None` on the line after it (measured — the
//! first version of this file did exactly that and took the not-realized branch every time). An idle
//! callback runs once that queue has drained, which is the earliest point the window exists.
//!
//! So the plain `set_focus()` stays where it was and this is added after it: on any display where
//! the stamped path does not apply — Wayland, an unrealized window, a server that returns no time —
//! the behaviour is exactly what it was before.

use tauri::WebviewWindow;

/// Raise `window` and ask for the focus, with a timestamp the compositor will honour.
///
/// **Main thread only** (every path below is a GTK call, and the idle callback is registered on the
/// thread-default main context).
pub fn present(window: &WebviewWindow) {
    let _ = window.set_focus();
    #[cfg(target_os = "linux")]
    present_stamped(window);
}

#[cfg(target_os = "linux")]
fn present_stamped(window: &WebviewWindow) {
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    gtk::glib::idle_add_local_once(move || {
        stamp_and_present(&gtk_window);
    });
}

/// One attempt, on a window that may or may not be able to answer. Silent when it cannot: the plain
/// `set_focus` above has already been made, so there is nothing here to report or retry.
#[cfg(target_os = "linux")]
fn stamp_and_present(gtk_window: &gtk::ApplicationWindow) {
    use gtk::glib::Cast;
    use gtk::prelude::{GtkWindowExt, WidgetExt};

    let Some(gdk_window) = gtk_window.window() else {
        return;
    };
    // Wayland leaves here: `X11Window` is the X11 subclass of `GdkWindow`, and the downcast is how
    // the backend is asked without linking a second display-server check into the build.
    let Ok(x11) = gdk_window.clone().downcast::<gdkx11::X11Window>() else {
        return;
    };
    // The guard the hang depends on. A mapped toplevel selects `PropertyNotify`; anything else may
    // not, and `gdk_x11_get_server_time` would block the GTK main loop waiting for it.
    if !gdk_window.is_visible() {
        return;
    }
    let time = gdkx11::functions::x11_get_server_time(&x11);
    // `0` is `CurrentTime`, which is precisely the value that means "no timestamp" — the state this
    // function exists to get out of. Presenting with it would be the refused call again.
    if time == 0 {
        return;
    }
    gtk_window.present_with_time(time);
}
