---
title: Change detection
description: How the engine spots local and remote changes — content hashing, the quick-check, and Proton's volume-event stream.
sidebar:
  order: 2
---

Reconciliation is only as good as its ability to notice what changed. The engine detects
local and remote changes differently.

## Local changes: SHA-1, with a quick-check

Local files are compared by **SHA-1 content hash**. Re-hashing an entire tree on every
periodic scan would be wasteful, so the scanner uses a **quick-check**: if a file's size
**and** whole-second modification time both match its last-synced record, the file is
assumed unchanged and its stored hash is reused.

The trade-off: a change that somehow preserves *both* the byte size and the whole-second
mtime of a file isn't noticed until the next edit that changes size or mtime. This is the
same default `rsync` makes, and it's rare in practice.

The daemon also watches the folder with the OS filesystem-notification API, so most local
edits trigger a reconcile promptly rather than waiting for the periodic scan.

## Remote changes: two strategies

Detecting remote changes is harder, because Proton Drive gives no "this subtree changed"
flag — a nested change leaves every ancestor byte-identical. There are two ways to find
out what moved.

### Full-tree walk (the baseline)

The straightforward method lists every remote directory with
`proton-drive filesystem list --json`, one call per folder. That's **`O(folders)`** network
round trips every scan. It's correct and simple, but slow on large trees, so it's used as
the **bootstrap** and as a periodic **reconvergence** safety net — not the hot path.

### Event-driven (the default)

Proton's own clients avoid full walks with a **volume-level event stream**: fetch a cursor,
then poll the deltas since that cursor. That's **`O(changes)`**, not `O(folders)`. The
engine uses this by default.

Concretely, the engine:

1. Reads the last stored per-volume **event cursor** from its index.
2. Fetches the delta of created / updated / deleted nodes since that cursor.
3. Overlays that delta onto the last-known remote view to reconstruct a *complete* remote
   map — without walking the whole tree — and hands it to the planner.
4. Advances the cursor **only** in the final commit of a fully-successful pass, so a
   failed or withheld pass re-derives from the same cursor next time.

A **full-tree scan still runs periodically** (every *N* incremental passes, configurable
with `--events-full-scan-every`) as a reconvergence backstop, and the **first reconcile
after the daemon starts is always a full scan** — a fresh process has no pending-change
history, so it can't safely trust an incremental pass alone.

Event-driven mode is **on by default**. Opt out with `--no-events-driven` (or
`events_driven = false`), which restores the byte-identical snapshot-only path.

#### A subtlety worth knowing

In Proton's event stream, moving a file to the **Trash** arrives as an *update* with a
`trashed` flag — **not** as a delete event. The engine normalizes that correctly, so a
trash is treated as a removal. (This is the kind of detail the engine's tests pin down.)

## How the engine reaches the event stream

Reading the event delta needs a Proton **session** but **no decryption** — the events
carry node IDs, parent IDs, type, and trashed/shared flags, all in cleartext. The engine
gets that session by **reusing the logged-in `proton-drive` CLI's session** from your
desktop keyring (read-only), and fetches the deltas over plain HTTPS. It ships no Proton
networking or crypto of its own; the HTTP transport and session provider are injected
seams.

Two consequences:

- The daemon is only as fresh as the CLI keeps the session. If the CLI is idle and its
  token expires, a pass fails auth and is skipped until the CLI refreshes — and the daemon
  **degrades to snapshot scans** meanwhile, so sync keeps working, just less cheaply.
- Created **and** updated (non-trashed) remote nodes are resolved with a **targeted
  single-directory list** rather than a full walk. That targeted list is also where a
  newly-created node's *name* gets decrypted (unlike its ids, the name is encrypted in the
  event) — the one lookup unique to creations.

An independent (browser-forked) session that doesn't depend on the CLI is a deferred design
decision. The rationale for all of this is recorded in
[ADR 0001](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/adr/0001-remote-change-detection-via-volume-events.md).

## Tuning

| Control | Effect |
| --- | --- |
| `--events-driven` / `--no-events-driven` | Turn event-driven detection on (default) or off. |
| `events_driven = true/false` | The config-file equivalent. |
| `--events-full-scan-every N` | Force a full-tree reconvergence every *N* incremental passes. |
| `--scan-interval-secs` | How often the periodic reconcile fires. |

See the [Daemon reference](/daemon/reference/) for all flags.
