---
title: Desktop app overview
description: What the Proton Drive Sync desktop app is, how it maps onto the daemon, and the six daemon states it shows.
sidebar:
  order: 1
---

**Proton Drive Sync** is a desktop app for Linux, built with [Tauri](https://tauri.app/).
It gives the daemon a face: live status, an activity ledger, side-by-side conflict
resolution, a dry-run review before destructive passes, settings, an onboarding wizard, and
a system-tray indicator.

![The desktop app at rest: a hexagon with a check mark, the headline "Everything is up to date", the sub-line "last synced 2 minutes ago", Sync now and Pause buttons, and a footer naming the folder pair.](../../../assets/screenshots/main-settled.png)

*The app at rest. The design carries no colour at all when there is nothing to report, so
anything coloured on screen is something that wants you.*

## It's a thin client

The app owns **no sync logic and no index of its own**. Everything it shows comes from the
daemon's real surface:

| Source | Supplies |
| --- | --- |
| Control socket (`$XDG_RUNTIME_DIR/proton-sync.sock`) | Live status, history, pending deletions, and the `pause` / `resume` / `syncnow` / `approve` / `deny` / `shutdown` commands. |
| Config file (`~/.config/proton-sync/proton-sync.toml`) | Everything in **Settings**. The app edits this file in place. |
| `.proton-cloud` sidecars on disk | Conflict resolution — plain file operations, no new IPC verb needed. |
| `proton-syncd --dry-run` | The **Plan preview** and onboarding review. |
| The SQLite index (read-only) | File-manager emblems. |

Because the app reads the same socket and files as the CLI, the two never disagree. Closing
the app's window **never stops the daemon** — the window hides to the tray and syncing
continues.

## How it stays current

The app polls the daemon's `status` on a timer — **every 2 seconds when focused, every 10
seconds when in the background** — plus a periodic conflict re-scan and a refresh of pending
deletions. A socket error is its own explicit state; **counters never render as a
misleading zero** when the daemon is unreachable — they show an em-dash instead.

## The six daemon states

Both the window and the tray derive one shared **daemon state** from the status reply (or
its absence), so they always agree. Each state drives the headline, the available actions,
the tray icon, and whether counters are shown:

| State | Meaning | Primary actions offered |
| --- | --- | --- |
| **Running** | Reachable, not paused, changes pending — actively syncing. | Sync now, Pause |
| **Idle** | Reachable, not paused, nothing pending — up to date. | Sync now, Pause |
| **Paused** | Sync is paused. | Resume |
| **Auth expired** | Proton sign-in looks expired (heuristic from the last error). | Re-authenticate (`proton-drive login`), Pause |
| **Unreachable** | The control socket can't be reached, or the reply couldn't be trusted. | Start `proton-syncd`, View journal |
| **First run** | Nothing has synced yet. | Preview plan, Choose folders (→ onboarding) |

In the **Unreachable** and **First run** states, the app deliberately shows **no counters**
(no fake "0 pending") — unknown is not zero.

## Actions the app shows vs. runs

The app runs sync commands for you over the socket (`sync now`, `pause`, `resume`, approve/
deny deletions, resolve conflicts, edit the config). A few system-level actions it
**shows you the command to run** rather than running silently — starting the service
(`systemctl --user start proton-syncd`), viewing the journal
(`journalctl --user -u proton-syncd`), and re-authenticating (`proton-drive login`) — so
nothing surprising happens on your behalf.

## Honest about the engine's limits

The app is careful not to imply capabilities the daemon doesn't have. A few examples you'll
notice:

- **Download progress is indeterminate.** The daemon surfaces live progress over the socket
  (the phase, `[i/N]` action counts, and byte counts when known), but a remote listing carries
  no file size — so a *download's* total bytes are unknown and the app shows a moving bar and
  file counts rather than a fabricated percentage. (Uploads, whose size is known, show a
  percentage.)
- **Applying a reviewed plan is a fresh pass.** Dry-run and the real reconcile are separate
  invocations, so the applied plan can differ from the reviewed one — the app says so.
- **One folder pair.** The daemon syncs one local↔remote pair; the folder selector is shaped
  for more, but one is the reality today.

Continue to [Screens](/desktop/screens/) for a tour of each view, or
[Tray & notifications](/desktop/tray/) for the background indicator.
