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
| `history` | Show the recent sync history, newest first. |
| `pause` | Pause automatic **and** manual sync until resumed. |
| `resume` | Resume sync work. |
| `syncnow` | Trigger a sync and watch it finish (`--no-wait` to just schedule it). |
| `stop` | Ask the running daemon to exit gracefully. |
| `pending` | List deletions currently withheld by the [delete-approval guard](/safety/delete-approval/). |
| `approve <path>` \| `approve --all` | Approve a withheld deletion (or all) so it applies next sync. |
| `deny <path>` \| `deny --all` | Revoke a prior approval before it applies. |

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
  just now  ✓ sync completed — 2 uploads, 1 download
   15m ago  ✗ sync failed — proton-drive: request failed …
```

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
prints the full response object (below), `history --json` just the history array, and
`pending --json` the pending-deletions array. `--json` output is always the daemon's data
verbatim — so `history --json` keeps the stored **oldest-first** order, while the
human-readable `history` renders newest first. Scripts that consumed the old JSON output
should add the flag.

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
- `status_history` — the recent history array (also available via `history`).
- `pending_deletions` — the items awaiting delete approval (also via `pending`).
- `activity` — what the daemon is doing right now (`null` when idle): `{ phase, detail,
  folders_listed, files_scanned, action_index, action_total, transfer, since_epoch_secs }`,
  where `phase` is one of `scanning-local`, `listing-remote`, `fetching-events`,
  `executing`, `committing`, and `transfer` (when a file is moving) carries `direction`,
  `path`, `bytes_done` (downloads: sampled live from the staging directory), `bytes_total`
  (uploads only — the remote listing exposes no size), and `started_epoch_secs`. Every
  field is display-only and best-effort; new phases may appear, so render unknown tokens
  rather than failing.

## Status history

Each `status_history` entry has `epoch_secs`, `message`, `last_error`, `plan_summary`, and
`successful_sync_summary`. The daemon keeps the most recent **20** entries in a JSON file
next to the index and reloads them on restart, so recent failures and successes stay visible
through `status` and `history`. For older history, read the journal:

```bash
journalctl --user -u proton-syncd.service
```

## Delete approval

The `pending`, `approve`, and `deny` commands drive the [delete-approval
guard](/safety/delete-approval/). `pending` labels each item with its direction — a
**LOCAL DELETE** (removed on Proton Drive; approving removes your local copy) or a
**REMOTE DELETE** (removed locally; approving removes it on Proton Drive) — so you always
know which way a deletion runs before approving.

```bash
proton-sync pending
proton-sync approve notes/old.txt   # one item
proton-sync approve --all           # everything pending
proton-sync deny notes/old.txt      # revoke before it applies
proton-sync syncnow                 # apply now
```

`approve`/`deny` require either a `<path>` **or** `--all`, never both and never neither — a
bare `approve` won't silently approve everything.

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
