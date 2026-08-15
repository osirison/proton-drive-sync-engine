---
title: Control CLI reference
description: Every proton-sync command — status, history, pause, resume, syncnow, stop, and the delete-approval commands.
sidebar:
  order: 1
---

`proton-sync` is a thin control client. It sends requests to the running daemon over its
Unix socket and prints the reply. It runs no sync logic of its own. The daemon answers
control requests from an in-memory snapshot on a dedicated task, so every command below
responds immediately — even while a sync pass is running.

```bash
proton-sync [--config <PATH>] [--socket-path <PATH>] [--json] <command>
```

When `--socket-path` is omitted, the CLI uses the same default as the daemon —
`$XDG_RUNTIME_DIR/proton-sync.sock`, with an OS-temp-directory fallback — so on a normal
setup you don't need to pass it.

If your daemon runs with a `socket_path` set in its [config file](/daemon/configuration/),
point the CLI at the same file with `--config` instead of repeating `--socket-path` on
every command. The precedence matches the daemon's: `--socket-path` beats the config file's
`socket_path`, which beats the default. Only `socket_path` is read from the file; every other
key belongs to the daemon. A relative `socket_path` is rejected on both sides, because it
would resolve against each process's own working directory.

## Commands

| Command | What it does |
| --- | --- |
| `status` | Show what the daemon is doing right now (see below). |
| `history` | Show the recorded sync passes, newest first — duration, kind, outcome. |
| `activity [<path>]` | Show what has moved recently, or one path's own history. |
| `pause` | Pause automatic **and** manual sync until resumed. |
| `resume` | Resume sync work. |
| `syncnow` | Trigger a sync and watch it finish (`--no-wait` to just schedule it). |
| `stop` | Ask the running daemon to exit gracefully. |
| `pending` | List deletions currently withheld by the [delete-approval guard](/safety/delete-approval/). |
| `approve <path>` \| `approve --all` | Approve a withheld deletion (or all) so it applies next sync. |
| `deny <path>` \| `deny --all` | Revoke a prior approval before it applies. |
| `keep <path>` \| `keep --all` | Refuse a withheld deletion and put the surviving copy back on the other side. |

## Human-readable by default, `--json` for scripts

Commands print a concise, git-style summary by default:

```text
$ proton-sync status
● syncing — 2 uploads, 1 download planned
  folders    ~/ProtonDrive ⇄ /Drive/RemoteFolder
  activity   downloading Documents/takeout.tgz — 1.4 GiB so far · 3m12s [step 812/6377]
  last sync  2m ago
  changes    3 queued locally

$ proton-sync history
Last full sweep 2d ago — nothing was out of step.
Today: 386 MB sent · 1.1 GB received.

  just now   1.4s  incremental  ✓ 3 change(s), 2.1 MB sent
   15m ago    12s   full-sweep  ✗ proton-drive: request failed …

$ proton-sync activity
  just now  sent to Proton Drive     notes/today.md (4.2 kB)
   3m ago   brought to this computer photos/trip.jpg (2.1 MB)

2 file(s), 2 event(s) in this window · 4.2 kB sent · 2.1 MB received
```

`activity` takes an optional path (`proton-sync activity notes/today.md`) to show just that
file's history, `--days N` to bound the window, and `--limit N` to cap the rows. A
path-filtered reply carries no byte totals: the totals are per pass and hold no paths, so
the only number available would be the window's whole traffic — which would read as that
one file's.

The headline dot reflects the daemon state: **idle** (everything up to date), **syncing**
(a pass is in flight, with the plan when known), **running** (changes queued for the next
pass), **paused**, or **error** (the last pass failed; the error is shown on its own line).
Extra lines — queued changes, deletions awaiting approval, the last error — appear only
when they apply.

While a pass is in flight, the **activity** line shows what the daemon is doing *right
now*: which remote folder a full-tree walk is listing (with a running count), which local
file the scan is hashing, or which file is transferring — with live bytes-so-far for
downloads, sampled from the staging directory while the transfer runs. The same line
animates in place on the `syncnow` spinner.

Pass `--json` to any command to get the daemon's raw response instead: `status --json`
prints the full response object (below), `history --json` the `history` block, `activity
--json` the `file_history` block, and `pending --json` the pending-deletions array.
`--json` output is always the daemon's data verbatim.

`history --json` **changed shape**: it used to be the raw `status_history` array, stored
oldest-first, and is now the `history` object described under [Pass history](#pass-history).
Scripts reading it need updating; `status --json` still carries `status_history` unchanged
for anything that wants the old attempt trail.

## `syncnow`

`syncnow` schedules a pass and is acknowledged immediately — the daemon never blocks other
control clients while syncing. The CLI then polls status and shows a live spinner until
*your* pass finishes, printing its outcome:

```text
$ proton-sync syncnow
✓ sync completed — 3 uploads
```

The exit code is non-zero if the watched pass failed. Use `--no-wait` to only schedule the
sync and return at once, and `--json` to print the final status object (or, with
`--no-wait`, the acknowledgement). While the daemon is paused, `syncnow` reports the skip
and schedules nothing.

## The status object (`--json`)

```json
{
  "status": "running",
  "paused": false,
  "syncing": false,
  "reconcile_seq": 4,
  "pending_changes": 0,
  "message": "daemon status",
  "last_sync_epoch_secs": null,
  "last_error": null,
  "last_plan_summary": null,
  "last_successful_sync_summary": null,
  "status_history": [],
  "pending_deletions": []
}
```

- `status` / `paused` / `pending_changes` — the live daemon state (`running`, `syncing`, or
  `paused`) and the number of changes waiting to reconcile. The string reports live
  activity: it stays `syncing` while a pass is in flight even if a pause was just accepted
  mid-pass (`paused` carries the standing request; the CLI shows this combination as
  "pausing").
- `syncing` — `true` while a reconcile pass is actually in flight.
- `reconcile_seq` — count of completed passes since the daemon started; clients that
  scheduled a sync poll until it advances to know their pass finished.
- `last_sync_epoch_secs` — when the last sync ran (`null` before the first).
- `last_error` — the most recent error, if any.
- `last_plan_summary` / `last_successful_sync_summary` — nullable [plan
  summaries](/safety/dry-run/#the-output-shape) (upload/download/conflict/… counts).
- `status_history` — the last 20 reconcile **attempts**, idle ones included: a rolling debug
  trail, not the pass log. See [Pass history](#pass-history).
- `history` — the durable pass log: `{ recent, last_full_sweep, today }` (also via
  `history`). See [Pass history](#pass-history).
- `pending_deletions` — the items awaiting delete approval (also via `pending`).
- `activity` — what the daemon is doing right now (`null` when idle): `{ phase, detail,
  folders_listed, files_scanned, action_index, action_total, transfer, since_epoch_secs }`,
  where `phase` is one of `scanning-local`, `listing-remote`, `fetching-events`,
  `executing`, `committing`, and `transfer` (when a file is moving) carries `direction`,
  `path`, `bytes_done` (downloads: sampled live from the staging directory), `bytes_total`
  (uploads only — the remote listing exposes no size), and `started_epoch_secs`. Every
  field is display-only and best-effort; new phases may appear, so render unknown tokens
  rather than failing. `since_epoch_secs` is when the **phase** began and resets on every
  phase change; the `pass` block — `{ started_epoch_secs, changes, kind }` — is the pass as
  a unit and does not, so an elapsed time rendered from it climbs monotonically. `changes`
  is `null` until the plan exists, which means *unknown*, not zero.

## Pass history

Two different records, answering two different questions.

**`status_history`** is the *attempt* trail: every completed reconcile attempt, idle ones
included, with `epoch_secs`, `message`, `last_error`, `plan_summary`,
`successful_sync_summary` and `failed_item_count`. The daemon keeps the most recent **20**
in a JSON file next to the index and reloads them on restart. With event-driven detection on
by default the daemon runs a pass every 30 seconds, so those twenty entries are roughly ten
minutes of wall clock and are mostly idle polls — useful for "is it alive and what did the
last few passes say", useless for anything older.

**`history`** is the durable pass log, stored in the index database:

- `recent` — the last 20 recorded passes, newest first. Each is
  `{ id, started_epoch_secs, duration_ms, kind, outcome, changed, failed, bytes_uploaded,
  bytes_downloaded, error }`. `kind` is `full-sweep`, `warm-start` or `incremental`;
  `outcome` is `clean`, `partial`, `failed`, or `interrupted` (a pass the process died in
  the middle of). Both are open tokens — render an unfamiliar one verbatim.
- `last_full_sweep` — the most recent full-tree walk, however long ago.
- `today` — `{ since_epoch_secs, uploaded_bytes, downloaded_bytes }` since local midnight.

**An idle pass records nothing.** Recording ~2900 "nothing happened" rows a day would evict
every interesting one within minutes, and "the daemon is alive" is already answered by
`reconcile_seq` and `last_sync_epoch_secs`. A pass is recorded when it changed something, or
did not end cleanly, or was a full sweep — a changeless full sweep is recorded precisely
because "when did the last one run, and was anything out of step" is what it answers.

Retention is bounded in both dimensions: passes keep the newest 2000 rows and one year, the
per-file event log the newest 20 000 rows and 90 days. The most recent full sweep is exempt
from both pass bounds, so a chronically failing daemon cannot evict it. For anything older,
read the journal:

```bash
journalctl --user -u proton-syncd.service
```

## Delete approval

The `pending`, `approve`, `deny` and `keep` commands drive the [delete-approval
guard](/safety/delete-approval/). `pending` labels each item with its direction — a
**LOCAL DELETE** (removed on Proton Drive; approving removes your local copy) or a
**REMOTE DELETE** (removed locally; approving removes it on Proton Drive) — so you always
know which way a deletion runs before approving.

```bash
proton-sync pending
proton-sync approve notes/old.txt   # one item
proton-sync approve --all           # everything pending
proton-sync deny notes/old.txt      # revoke an approval before it applies
proton-sync keep notes/old.txt      # refuse it: put the surviving copy back on the other side
proton-sync syncnow                 # apply now
```

`approve`/`deny`/`keep` require either a `<path>` **or** `--all`, never both and never
neither — a bare `approve` won't silently approve everything.

`approve` also takes `--direction local|remote`, which is read **only** when nothing pending
matches the path: that is how a deletion is approved *before* the pass that withholds it (the
desktop app's Plan screen does exactly this when you type `DELETE`). A path alone does not say
which of the two deletions at it you mean, so without the flag nothing is recorded.

## Stopping the daemon

`proton-sync stop` asks the daemon to exit through the same clean path as a `SIGTERM`: an
in-flight transfer is cancelled (actions that fully completed before the stop keep their
checkpoint commits; the interrupted action is never recorded and replans, along with the
rest, on next start), the control socket is removed, and the process exits. Under systemd, start it again
with `systemctl --user start proton-syncd`; the unit's `Restart=on-failure` does not respawn
a clean stop.

## Exit codes

The CLI exits `0` on a successful round-trip and non-zero on failure — it can't reach the
daemon, or the pass watched by `syncnow` failed. If it can't connect, confirm the daemon is
running and that both sides use the same socket path — see
[Troubleshooting](/reference/troubleshooting/).
