---
title: Tray & notifications
description: The system-tray indicator — its five icon states, the state-dependent menu, and how closing the window keeps syncing.
sidebar:
  order: 3
---

The desktop app lives in your **system tray**. A background thread polls the daemon every 5
seconds and drives the tray icon, tooltip, and menu — even while the window is hidden — so
the tray is a reliable at-a-glance status even when you're not looking at the app.

## Closing the window keeps syncing

Closing the app window (the ✕ button) **hides it to the tray** rather than quitting — the
tray and the app process keep running. The tray menu makes both exits explicit: **Close
window (keeps syncing in the tray)** does the same hide-to-tray, while **Quit Proton Drive
Sync** ends the app process (the tray with it). The **daemon is a separate process and is
never affected** by either — to stop *syncing* you pause or stop the daemon, not close the
window.

## The five icon states

The tray icon is a symbolic, monochrome-safe glyph mapped from the shared [daemon
state](/desktop/overview/#the-six-daemon-states):

| Icon | State | Tooltip |
| --- | --- | --- |
| **Syncing** (two arcs) | Running — reachable, not paused, changes pending | "syncing (N pending)" |
| **Up to date** (check) | Idle — reachable, not paused, nothing pending | "up to date" |
| **Paused** (bars) | Paused | "paused" |
| **Attention** (!) | Auth expired **or** First run (they share this icon) | "sign-in expired" / "nothing synced yet" |
| **Unreachable** (✕) | Socket unreachable or reply untrusted; also the initial state before the first poll | "daemon unreachable" |

## The menu adapts to state

The tray menu changes with the state, and never shows misleading counters:

- **First run** — Show window, a disabled "Nothing synced yet" note (no fake "0 pending"),
  Settings, Close window, Quit.
- **Unreachable** — Show window, **Start proton-syncd**, **View journal**, Settings, Close window, Quit.
  No counters at all.
- **Reachable** (running / idle / paused / auth-expired) — Show window,
  **Sync now (N pending)** (disabled while paused), **Pause** / **Resume** (follows state),
  **Resolve K conflict(s)** (from the last plan summary; disabled at zero), Settings, Close window, Quit.

Interactions: left-clicking the icon shows the window; menu items that open a screen
(Settings, Resolve conflicts, View journal → History) bring the window forward and navigate
there; Sync now / Pause / Resume run on a background thread so the blocking socket never
freezes the UI; **Start proton-syncd** does a best-effort `systemctl --user start
proton-syncd`.

## Desktop notifications

The app can fire native desktop notifications through the OS notification service, so
noteworthy events can surface even when the window is hidden.

## Requirements

On Linux the tray needs `libappindicator` present at runtime. The native packages declare it
(a hard dependency on Arch via `libayatana-appindicator`; a soft `Recommends` on Fedora,
where Tauri's tray loads it with `dlopen`). If your desktop environment has no system tray /
app-indicator support, the window still works — you just won't see the tray glyph.
