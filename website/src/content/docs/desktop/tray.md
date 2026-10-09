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

Interactions: **left-clicking the icon opens a small floating panel** — the same hexagon,
sentence and menu rows as the window, small enough to read at a glance. Usually it opens
right next to the icon; on GNOME's Wayland session the desktop places it itself, because a
Wayland app there cannot ask to be put anywhere in particular, so it may not appear beside
the tray. Clicking the icon a second time closes it, and so does picking any row on it, and
so does Esc. On most desktops it also goes away the moment you click away from it. Where the
desktop lets an app anchor a panel beside the tray — KDE Plasma on Wayland does, GNOME does
not — clicking away from it does not close it: an anchored panel is not told that your click
landed on something else, so it stays up until you close it yourself. It is a separate
window from the main one; **Open Drive Sync** on the menu is what raises the full app window
instead. Sync now / Pause / Resume / Try again now all run on a background thread so the
blocking socket never freezes the UI, and **Start the sync service** asks systemd first
(`systemctl --user start proton-syncd`) and, when there is no unit to ask, falls back to
launching `proton-syncd` directly against the saved config.

## Two or more folders

With one folder the tray is exactly what is described above. When the daemon runs two or more
[folder pairs](/daemon/folder-pairs/), it grows in four ways, and **every folder has its own
pause** — there is no *Pause all*:

- **The icon shows the worst state any folder is in**, from the same five forms: a daemon that
  cannot be reached, then a signed-out session, then a folder whose last sync failed, then one that
  has never synced, then syncing, then paused, then up to date. One folder paused and the other up
  to date shows the paused icon; one paused and the other syncing shows the syncing icon; one failed
  and anything else shows the struck icon — which then means *that folder's last sync failed*, not
  that Proton is out of reach.
- **The tooltip says which folder is which**, worst first, naming at most three and then
  *+n more*: *documents paused, photos up to date*.
- **The menu has one row per folder**: *Pause documents* — or *Resume documents* while that folder is
  paused — and *Pause photos*, between two rules. **Sync now** is one row that asks every folder you
  have not paused to sync; it is there while any of them is idle. **Close window · keeps syncing**
  is there while any folder is unpaused. A menu you opened before a folder came or went acts on the
  folder its label named, or on nothing if that folder is gone.
- **The panel is the worst folder's own**: its hexagon and sentence, with the folder's name in a
  line above. A paused folder's sentence names it — *Nothing in documents will move until you
  resume* — because another folder may be syncing. It lists pause rows for five folders, worst
  first, then one row — *2 more folders* — that opens the window; the right-click menu lists every
  folder.
- **A deletion waiting in any folder is counted on the panel**, not only in the folder it shows: an
  up-to-date folder with a deletion waiting is shown ahead of a paused one, and *Review them* opens
  the window with that folder selected. (Conflicts are counted for the one folder the tray scans.)

An expired session, a folder that has never synced and a stopped daemon keep their short menus:
there is nothing to pause, or nobody to send it to. When a pause cannot be saved to the folder's
index the daemon says so on the command line (`proton-sync pause` prints *Not saved*). The tray
panel is dismissed by every row before the daemon answers, so it has no place to show that: it writes
a line about it to the app's log, and the app's window keeps it for that folder and says it under the
folder's hero (*Paused, but not saved*) when that folder is the one it shows — even if another folder
was on screen when the row was pressed. The sentence goes when the folder's pause state changes
under it (a resume from the tray or `proton-sync resume`, for example) and when the daemon stops
answering.

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
