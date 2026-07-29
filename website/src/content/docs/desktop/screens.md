---
title: Screens
description: A tour of every screen in the desktop app — Overview, Activity, Conflicts, Deletions, Plan preview, History, Settings, and Onboarding.
sidebar:
  order: 2
---

The window has a left sidebar with one row per screen (some carry a live count or badge), a
title bar showing the `local ⇄ remote` folder pair and a light/dark toggle, a state pill, a
safety banner on destructive screens, and a status footer. Here's what each screen does.

## Overview

The home dashboard — *is it working, does it need me, what did it just do.*

- A **hexagon status widget** whose center shows the pending-change count; it spins while
  syncing, dims when paused, and drops its arcs when idle or unreachable.
- A **headline and sub-line** from the current [daemon state](/desktop/overview/#the-six-daemon-states)
  (e.g. "Everything is up to date", "last synced 5 min ago").
- **Transfer rows** while syncing — "Uploading N files" / "Downloading N files" from the
  last plan summary, with indeterminate progress bars (the engine reports file counts, not
  a percentage).
- **Four stat tiles**: pending changes, conflicts, destructive actions, skipped-unsupported
  — each showing an em-dash when the daemon is unreachable.
- A **"Needs you" card** that appears only when there are unresolved conflicts, with a
  "Resolve now" button.
- An **activity ledger** of recent history, with filter chips.
- **State-aware action buttons** (Sync now / Pause / Resume / Re-authenticate / …), each
  with a small hint naming the underlying command.

## Activity

The full-height activity ledger. Rows come from the daemon's status history (newest first);
an entry with an error renders as an `error` row, otherwise as a neutral row with its
message. Filter chips — `all / uploads / downloads / moves / conflicts / skipped / errors` —
narrow the view, and their counts derive from the visible rows. Colour never carries meaning
alone; the action name is always present. Read-only.

## Conflicts

Review and resolve [conflicts](/safety/conflicts/) side by side. A left rail lists each
conflicted file; the compare panel shows **your version** (local) against **Proton's
version** (the sidecar), with a line-level diff for text files and size/timestamp metadata
for binary or large files — never a fabricated preview.

Four staged choices per file — **nothing is written until you press Apply**:

| Choice | Effect |
| --- | --- |
| **Keep mine** | Delete the sidecar; your local file uploads next pass. |
| **Use Proton's** | Replace your file with Proton's copy. |
| **Keep both** | Rename yours to `name.local.ext` and keep Proton's too. |
| **Decide later** | Nothing written; still counts as outstanding. |

Choosing auto-advances to the next unresolved file, and the footer counts staged operations.
The outstanding-conflict count is computed once and shown identically in the sidebar badge,
the tab header, the Overview card, the stat tile, and the ledger.

## Deletions

The [delete-approval](/safety/delete-approval/) queue. When the guard withholds a deletion,
it lands here instead of running. Each item shows its path, whether it's a file or directory,
its direction, and when it was detected — with plain-language copy about exactly what
approving will do:

- **Local** — already deleted on Proton Drive; approving **permanently deletes the local
  file, straight from disk, no trash** (directories: "and everything inside it").
- **Remote** — already deleted locally; approving **moves the Proton Drive copy to Proton's
  Trash**, recoverable there until emptied.

Per-item **Approve** / **Deny**, plus **Approve all** / **Deny all**. While any approve/deny
is in flight, all the buttons disable to prevent overlapping destructive operations.

## Plan preview

Run a [dry-run](/safety/dry-run/) and see exactly what the next sync would do. The screen
shows a **summary grid** of every counter and one **row per action** (action, path, entity
kind, remote id), with destructive rows tinted red and sorted first.

Applying is gated:

- If the plan contains a real delete (`remote_delete` / `local_delete`), **Apply is inert
  until you type `DELETE`** to arm it; the red banner names the files at risk.
- A **purge-only** plan is **not** gated — purge is index-only cleanup — and the screen says
  so.

Because the real reconcile is a separate pass, the outcome card reminds you it may differ
from the reviewed plan; re-run the preview for a fresh check.

## History

A reverse-chronological list of sync history: a coloured dot (red on error), the local time,
a label, and a one-line summary like `7↑ 2↓ · 2 conflicts · 1 move · 3 deletes · 1 skipped`.
The screen states the limit honestly — the daemon keeps only the **last 20 entries**,
persisted across restarts — and points you to `journalctl --user -u proton-syncd` for older
history. Read-only.

## Settings

Edit the daemon's TOML config directly. Saving writes **only the fields you changed**,
preserves comments and daemon-only keys, and refuses to write anything the daemon's own
parser would reject (so a save can't brick startup). The file is written atomically at mode
`0600`. Sections:

- **Folders** — `local_root`, `remote_root`. Editing a root warns that it re-bootstraps the
  index; preview the plan first before restarting.
- **Schedule** — `scan_interval_secs`, and the `events_driven` toggle (Proton volume-events
  stream for fast incremental reconcile).
- **Selective sync** — the raw `include` / `exclude` glob lists. Copy states exclude beats
  include and `.sync/` is always ignored. (The checkbox folder-tree view is deferred.)
- **CLI** — `proton_cli` path, `proton_timeout_secs`, `proton_list_attempts`.
- **Delete approval** — the `remote` and `local` toggles, both shown **on (protected)** by
  default so a `null` in the file never misrepresents a security guard as off.
- **Service** — the config file path and a **Restart daemon now** button (the equivalent
  manual command is shown alongside).

The daemon only reads its config at startup, so after a successful save the screen prompts
you to restart it and offers a one-click **Restart daemon now** button. The restart asks the
running daemon to exit gracefully over IPC (this works however it was launched), waits for
the control socket to go quiet, then starts it again — via the systemd unit when installed,
else by spawning `proton-syncd` directly against the saved config. Changing a root
additionally promotes a "Preview plan" button ahead of the restart, since a root change
re-bootstraps the index.

## Onboarding

A first-run wizard that takes over the whole window when nothing has synced yet. Four steps:

1. **Check the CLI** — probe the daemon; if it's unreachable, independently try listing the
   remote (which shells `proton-drive`) to distinguish "not started" from "not logged in"
   (`proton-drive login`).
2. **Choose folders** — set and save `local_root` and `remote_root` (both required).
3. **Review the plan** — run a dry-run, show the summary and rows, warn if the *first* pass
   unexpectedly contains deletions, and require an explicit checkbox acknowledging that
   **deletions propagate in both directions** before continuing.
4. **Start the service** — show `systemctl --user start proton-syncd`, then re-poll; once the
   daemon is up, the app hands off automatically out of onboarding.

The copy is explicit that the **first sync is a non-destructive merge** — local-only files
upload, remote-only files download, matching files link, differing files are kept as both —
and that **nothing is deleted on the first pass**.
