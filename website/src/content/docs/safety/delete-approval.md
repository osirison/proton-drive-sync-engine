---
title: Delete approval
description: The directional, per-item delete-approval guard — how to approve, deny, configure defaults, and override per folder.
sidebar:
  order: 2
---

The delete-approval guard withholds destructive deletions until you approve them. It is
**on by default**, and it's what stops a surprising deletion from running unattended.

## The two directions

The guard is **directional** — you control each direction independently:

| Direction | Gates | Meaning |
| --- | --- | --- |
| `remote` | `remote_delete` | Approval to delete a file **on Proton Drive** because you removed it locally. Proton moves it to Trash (recoverable). |
| `local` | `local_delete` | Approval to delete a file **on your local disk** because it was removed/trashed on Proton Drive. This is the **permanent** local delete. |

Index-only cleanup (`purge`, when both sides are already gone) destroys no data and is
**never** gated.

## Approving and denying

While a deletion is guarded, the daemon withholds it — skipping **both** the delete itself
and its index update, so it re-plans and stays pending next pass — and lists it for you.

From the CLI:

```bash
proton-sync pending                 # show what's withheld, and in which direction
proton-sync approve notes/old.txt   # approve one; applies on the next sync
proton-sync approve --all           # approve everything currently pending
proton-sync deny notes/old.txt      # revoke an approval before it applies
proton-sync syncnow                 # apply approved deletions now
```

From the desktop app, the **Deletions** screen shows each pending item with plain-language
copy about exactly what approving will do, and offers per-item **Approve**/**Deny** plus
**Approve all**/**Deny all**. See [Screens](/desktop/screens/).

### Approvals are pinned

An approval is pinned to **exactly what you saw** — a file's last-synced content hash, or a
directory's remote id. If the entity changes before the delete applies, the fingerprint no
longer matches and the stale approval is inert: nothing is deleted. Approvals are consumed
in the same post-success transaction as the delete they authorize.

## Setting the default

The daemon-wide default comes from the `[delete_approval]` table in your config file (both
directions default to `true`):

```toml
[delete_approval]
remote = true   # gate deleting on Proton Drive (from a local delete)
local = true    # gate deleting on local disk (from a remote delete)
```

To turn the guard **off entirely, in both directions**, use the flag:

```bash
proton-syncd --config proton-sync.toml --no-delete-approval
```

`--no-delete-approval` is the CLI equivalent of setting both booleans to `false`.

## Per-folder overrides

You can override the guard for any subtree by dropping a `.proton-sync.toml` file in a
directory. It applies to that directory **and everything beneath it**, with the **nearest**
file winning — the same inheritance model as `.gitignore`. Any option a file leaves unset
inherits from its parent, and ultimately from the daemon-wide default.

```toml
# <local_root>/Downloads/.proton-sync.toml
# Let deletions under Downloads/ flow without approval, in both directions.
[delete_approval]
remote = false
local = false
```

Changes take effect on the next reconcile (within the events-poll interval, or immediately
with `proton-sync syncnow`).

### These files are machine-local and fail safe

- They are **never synced**. The engine ignores `.proton-sync.toml` at any depth, so a file
  on the remote can never silently weaken your local delete protection.
- A **malformed or unreadable** settings file is ignored and **the guard stays on** — it
  fails safe, never open.

## A correctness note for event-driven mode

When a pass withholds any deletion, the daemon does **not** advance its volume-event cursor.
A withheld local delete originates from a remote-delete event; if the cursor moved past that
event, the incremental remote reconstruction would stop re-deriving the deletion and the
pending item would vanish until the next full scan. Holding the cursor keeps every pending
deletion re-derived from ground truth, so an approval applies promptly. The cursor resumes
advancing on the first pass that withholds nothing.

The design rationale is recorded in
[ADR 0002](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/adr/0002-delete-approval-and-directory-config.md).
