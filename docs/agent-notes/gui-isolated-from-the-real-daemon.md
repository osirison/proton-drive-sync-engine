---
trigger: XDG_RUNTIME_DIR, XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME, DBUS_SESSION_BUS_ADDRESS, WAYLAND_DISPLAY, Gtk.init_check, daemon unreachable, StatusNotifierItem.Activate, sun_path, WAYLAND_DEBUG, isolated GUI, real Proton Drive account
depends_on: gui/src-tauri/src/panel.rs, gui/src-tauri/src/lib.rs, src/paths.rs, src/ipc.rs
recorded: 2026-09-16
---

# Running the GUI against an isolated environment — not the real daemon, not the real account

Preconditions for launching `target/debug/proton-sync-gui` (or the release binary) to measure it —
panel geometry, tray behaviour, unreachable-daemon states — without reaching the developer's real
`proton-syncd` or the real Proton Drive account it syncs. All measured on KDE Plasma 6 / KWin 6.7.5,
Wayland.

## Why isolation is needed at all

A real `proton-syncd` normally already runs on this machine against the real account (installed as a
systemd user unit by `setup.sh`). A GUI launched plainly resolves the same default paths the daemon
does (`default_socket_path` in `src/paths.rs`) and connects to it over the real control socket.
Measuring the GUI must not reach either — not the daemon, and through it not the account.

## The isolation recipe

- **A scratch `XDG_RUNTIME_DIR`, mode 0700, with no `proton-sync.sock` inside it.** The GUI resolves
  its socket path under this var, so an empty scratch directory means it finds nothing to dial and
  reports the daemon unreachable — a real, short panel state worth measuring in its own right, not
  just a side effect of isolating the session.
- **`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` pointed at scratch directories**, so nothing
  the GUI writes lands in the developer's real config or state.
- **`DBUS_SESSION_BUS_ADDRESS` set explicitly to the real session bus**, so the tray still registers
  after `XDG_RUNTIME_DIR` moves. Read the real value before overriding anything else
  (`echo $DBUS_SESSION_BUS_ADDRESS`) and carry it through unchanged.

## Two traps that cost real time

**GTK will not initialise if the Wayland socket is reached through a symlink inside the scratch
`XDG_RUNTIME_DIR`.** MEASURED: `Gtk.init_check` returns false — for a plain Python GTK program too,
so this is not a fault in this app's stack. Point `WAYLAND_DISPLAY` at the **absolute real socket
path** instead of relocating it alongside the other scratch directories.

**A session-scratchpad-length path exceeds `sun_path`.** MEASURED: a socket cannot be bound under
one. The control socket binds under `XDG_RUNTIME_DIR`, and `sun_path`'s budget is a small, fixed OS
constant (`sun_path_capacity` in `src/ipc.rs` computes it off `libc::sockaddr_un` rather than
hardcoding it — 108 bytes on Linux). Use a short path, such as `/tmp/p385run`, not the session
scratchpad directory.

## Opening the tray panel without clicking anything

```bash
gdbus call --session --dest org.kde.StatusNotifierItem-<pid>-1 \
  --object-path /StatusNotifierItem --method org.kde.StatusNotifierItem.Activate <x> <y>
```

**The coordinate space matters, and it is a property of the CALLER rather than of the protocol.**
MEASURED: a real tray host sends `Activate`'s `x`,`y` in **logical** pixels; the hand-made call above
sends whatever integers you type, and the measurement that established this typed **physical** ones.
`gdbus` has no space of its own to default to — you are choosing one whether or not you notice. That
is what issue #394 was about, and it is the general trap this recipe sits inside: a probe supplying
its own coordinates answers a different question from the one the real caller asks.

## Reading the panel back

KWin's scripting API — never `spectacle`. Cross-reference `measuring-a-gtk-layer-shell-surface.md`
section 2 for why (`spectacle` captures no layer surface at all). When filtering a KWin window dump
for the panel specifically, that note's section 1 carries the `resourceClass` trap and the reason for
it — not restated here.

## `WAYLAND_DEBUG=1` is the ground truth

It is the only record of what the client actually asked the compositor for, independent of what
KWin's own scripting API reports a moment later. MEASURED: its timestamps are **UTC**, while the
desktop's own clock — and anything else timestamped locally — is not; an hour apart on this machine.
Correlate the two logs on the sub-second part of the timestamp, not the hour.
