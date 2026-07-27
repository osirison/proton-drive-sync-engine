---
title: Control CLI reference
description: Every proton-sync command — status, history, pause, resume, syncnow, and the delete-approval commands.
sidebar:
  order: 1
---

`proton-sync` is a thin control client. It sends one request to the running daemon over its
Unix socket and prints the reply. It runs no sync logic of its own.

```bash
proton-sync [--socket-path <PATH>] <command>
```

When `--socket-path` is omitted, the CLI uses the same default as the daemon —
`$XDG_RUNTIME_DIR/proton-sync.sock`, with an OS-temp-directory fallback — so on a normal
setup you don't need to pass it.

## Commands

| Command | What it does |
| --- | --- |
| `status` | Print the full daemon status object (see below). |
| `history` | Print just the recent status-history array. |
| `pause` | Pause automatic **and** manual sync until resumed. |
| `resume` | Resume sync work. |
| `syncnow` | Trigger a reconcile immediately (no-op while paused). |
| `pending` | List deletions currently withheld by the [delete-approval guard](/safety/delete-approval/). |
| `approve <path>` \| `approve --all` | Approve a withheld deletion (or all) so it applies next sync. |
| `deny <path>` \| `deny --all` | Revoke a prior approval before it applies. |

`status`, `pause`, `resume`, and `syncnow` print the full response object as pretty JSON, so
scripts can consume them directly. `history` prints only the history array. `pending`,
`approve`, and `deny` print human-readable text.

## The status object

```json
{
  "status": "running",
  "paused": false,
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

- `status` / `paused` / `pending_changes` — the live daemon state and the number of changes
  waiting to reconcile.
- `last_sync_epoch_secs` — when the last sync ran (`null` before the first).
- `last_error` — the most recent error, if any.
- `last_plan_summary` / `last_successful_sync_summary` — nullable [plan
  summaries](/safety/dry-run/#the-output-shape) (upload/download/conflict/… counts).
- `status_history` — the recent history array (also available via `history`).
- `pending_deletions` — the items awaiting delete approval (also via `pending`).

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

## Exit codes

The CLI exits `0` on a successful round-trip and non-zero on failure (for example, if it
can't reach the daemon). If it can't connect, confirm the daemon is running and that both
sides use the same socket path — see [Troubleshooting](/reference/troubleshooting/).
