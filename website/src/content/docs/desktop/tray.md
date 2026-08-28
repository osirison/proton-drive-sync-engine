---
title: Tray & notifications
description: The system-tray indicator — its five icon states, the state-dependent menu, and how closing the window keeps syncing.
sidebar:
  order: 3
---

The desktop app lives in your **system tray**. It polls the daemon every 2 seconds and drives
the tray icon, tooltip, and menu — even while the window is hidden — so the tray is a
reliable at-a-glance status even when you're not looking at the app.

## Closing the window keeps syncing — quitting stops it

Closing the app window (the ✕ button) **hides it to the tray** rather than quitting — the
tray and the app process keep running, and the daemon is untouched. The tray menu carries
both exits, each with a sub-label that is literally true: **Close window · keeps syncing**
does the same hide-to-tray, while **Quit · stops syncing** stops the daemon first — its own
graceful shutdown, the same one `proton-sync stop` uses — and then ends the app process, tray
included.

## The five icon states

The tray icon is a symbolic, monochrome-safe glyph mapped from the shared [daemon
state](/desktop/overview/#the-seven-daemon-states):

Every glyph is the same hexagon outline from the main window's hero mark, in a different
treatment — there is no separate iconography for the tray:

| Icon | State | Tooltip |
| --- | --- | --- |
| **Syncing** — a faint hexagon with one solid arc drawn partway around it | Running — reachable, not paused, changes pending | "syncing (N changes)" |
| **Up to date** — a plain hollow hexagon | Idle — reachable, not paused, nothing pending | "up to date" |
| **Paused** — a dashed hexagon outline | Paused | "paused" |
| **Attention** — a hexagon with a solid dot at its centre | First run — nothing has ever synced | "nothing synced yet" |
| **Struck** — a hexagon crossed by a diagonal line | Auth expired, sync failed, **or** the control socket itself unreachable or untrusted — three different causes sharing one glyph | "sign-in expired" / "last sync failed" / "daemon unreachable" |

The icon groups by **form**: Auth expired, Failed and Unreachable all draw the same struck
mark, because from the glyph alone "nothing is syncing and something is wrong" is the only
claim the three actually share. The menu below parts them again by **cause** — a session you
have to fix in the window is not a daemon you can retry, and neither is a stopped service.

## The menu adapts to state

Six row sets, one per cause. Every set carries
**Open Drive Sync** and **Quit · stops syncing**; **Close window · keeps syncing** appears
only in Idle and Running — the two states where the service is healthily up and will keep
syncing on its own. Anywhere else (paused, or an outright problem) that sub-label would be
a lie:

- **Idle** — Open Drive Sync, Sync now, Pause syncing, Close window, Quit.
- **Running** — Open Drive Sync, Pause syncing, Close window, Quit. No Sync now — it would do
  nothing mid-sync.
- **Paused** — Resume syncing, Open Drive Sync, Quit.
- **Failed** — Try again now, Open Drive Sync, Quit. The daemon answered, so a retry reaches
  it.
- **Unreachable** — Start the sync service, Open Drive Sync, Quit. The control socket is what
  did not answer, so nothing here retries a sync; the one row that fixes it starts the
  service.
- **Auth expired** and **First run** — Open Drive Sync, Quit. Both are fixed in the window,
  not by retrying a sync: nothing in the app can sign you in, and first-run's takeover is one
  row away.

There is no Settings row and no conflict-resolution row on this menu — **Open Drive Sync** is
the only way out of the tray, and Settings and conflicts are reached from inside the window
it opens.

Interactions: **left-clicking the icon opens a small floating panel** next to it — the same
hexagon, sentence and menu rows as the window, small enough to read at a glance, and
dismissed the moment you click away or press Esc. It is a separate window from the main
one; **Open Drive Sync** on the menu is what raises the full app window instead. Sync now /
Pause / Resume / Try again now all run on a background thread so the blocking socket never
freezes the UI, and **Start the sync service** asks systemd first (`systemctl --user start
proton-syncd`) and, when there is no unit to ask, falls back to launching `proton-syncd`
directly against the saved config.

## Desktop notifications

The app can fire native desktop notifications through the OS notification service, so
noteworthy events can surface even when the window is hidden.

## Requirements

The tray speaks `org.kde.StatusNotifierItem` directly over D-Bus — the same protocol
`libappindicator` speaks, hand-rolled rather than loaded, because the interaction the design
calls for (left click opens the panel) needs an `Activate` method libappindicator's own items
never publish. On any desktop with a status-notifier host (KDE Plasma, GNOME with the
AppIndicator extension, and most other Linux desktops with a tray at all) that is the whole
story: nothing here loads `libappindicator`.

`libappindicator` only comes into it as a **fallback**, built solely when the SNI item fails
to register — a session with no status-notifier host at all. That fallback is a plain text
menu with no click interaction of its own (libappindicator's items publish no `Activate`
either, which is the same limitation this page's SNI item exists to route around). The native
packages still declare the dependency for that path (a hard dependency on Arch via
`libayatana-appindicator`; a soft `Recommends` on Fedora, where it would be loaded with
`dlopen`). If your desktop environment has no tray support of any kind, the window still
works — you just won't see a tray icon.
