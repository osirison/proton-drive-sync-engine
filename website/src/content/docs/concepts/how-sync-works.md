---
title: How sync works
description: The three-sources-of-truth model, bootstrap vs. ongoing reconciliation, and what every planned action means.
sidebar:
  order: 1
---

The engine's job is to turn the current state of two folders — plus the memory of how
they last agreed — into a safe list of actions, and then carry them out. This page
explains that model.

## Three sources of truth

Every reconcile compares three maps of the sync tree:

1. **Local** — the files under your local root, scanned and SHA-1 hashed.
2. **Remote** — the files in your Proton Drive remote root, listed through `proton-drive`.
3. **Baseline** — the last-synced state, stored in a local SQLite index.

The baseline is what makes two-way sync safe. With only the two live sides you cannot
tell a *newly created* file from a *deleted* one, or an *edit* from a *move*. The baseline
records what both sides looked like at the last successful sync, so the planner can
compute a **delta per side** (unchanged / created / modified / deleted / moved) and decide
from the pair.

## The pipeline

```text
scan local ─┐
list remote ─┼─▶ plan_sync ──▶ execute actions ──▶ commit index
load index ─┘   (pure, no I/O)   (uploads, etc.)     (checkpointed)
```

- **`plan_sync`** is a **pure function**: given the three maps, it returns a
  `Vec<PlannedAction>` and performs no I/O. This is the heart of the engine, and it's
  tested directly with in-memory maps.
- **Execution** performs the uploads, downloads, moves, and deletes.
- **Commit** checkpoints the new baseline into SQLite as actions complete — see the
  invariant below.

### Commit only after side effects succeed

An index write never happens before its side effect has succeeded. Completed work is
committed **incrementally**: after every action that performed a side effect (and after
every batched download chunk), the mutations recorded so far land in a checkpoint
transaction. If an action fails partway through a plan, everything already completed
stays durably recorded, the failed action itself is **never** recorded, and the
remainder replans on the next pass. There is never a half-recorded state that claims
something synced when it didn't — and a failure late in a long pass no longer discards
the work that preceded it. This is a load-bearing invariant of the design.

## Bootstrap vs. ongoing

The very first reconcile against a folder pair (an empty index) uses **bootstrap** logic,
which is **non-destructive** — it never deletes anything on either side:

- Local-only files are **uploaded**.
- Remote-only files are **downloaded**.
- Files that already match on both sides are **linked** in the index (recorded as synced).
- Files that differ create a **conflict** sidecar, keeping both versions.
- Empty directories are **created** to match on the other side.

After bootstrap, every pass uses **ongoing** logic: it computes each side's delta against
the baseline and chooses an action from the `(local_delta, remote_delta)` matrix.

## The planned actions

A plan is a list of typed actions. These are the `action` values you'll see in a
[dry-run](/safety/dry-run/) and in the desktop [Plan preview](/desktop/screens/):

| Action | Meaning |
| --- | --- |
| `upload` | Send a local file to Proton Drive. |
| `download` | Fetch a remote file to local disk. |
| `create_remote_directory` | Create a directory on Proton Drive. |
| `create_local_directory` | Create a directory locally. |
| `move_local` | Rename/move a local file or directory to match a remote move. |
| `move_remote` | Rename/move a remote file to match a local move. |
| `auto_link` | Record that both sides already match — no data moves. |
| `conflict` | Both sides changed differently; write a `.proton-cloud` sidecar and keep both. |
| `type_conflict` | A path is a file on one side and a directory on the other. Usually left untouched; the one exception — a local directory vs. a same-named remote file — preserves the remote file's content into a `.proton-cloud` sidecar. |
| `remote_delete` | Propagate a local deletion by trashing the file on Proton Drive. |
| `local_delete` | Propagate a remote deletion by permanently removing the local file. |
| `purge` | Drop a stale index row when both sides are already gone. No data is lost. |
| `skip_unsupported` | Something that cannot be synced at all: a Proton Docs/Sheets file the CLI can't download as a file, or a local name that is not valid UTF-8 (the remote listing speaks JSON and could never name it back). The row carries a `skip_reason`, and `proton-sync status` lists every such item under `can't sync`. (A symlink, socket, named pipe or device node is dropped by the *scanner* and so never becomes an action at all — but it is still listed under `can't sync`, from a separate report the local walk makes.) |

`move_local`/`move_remote` carry a `destination_path`; `conflict` carries a
`conflict_path`. The two **destructive** directions — `remote_delete` and `local_delete` —
are what the [delete-approval guard](/safety/delete-approval/) gates.

## Deletions and edits

Once a file has been synced, deleting it on one side propagates to the other on the next
pass — **but only if the other side hasn't also changed since the last sync**. If you
delete a file locally while it was edited on Proton Drive, the edit wins and is restored
locally rather than the remote copy being deleted. The surviving edit always beats the
delete. This delete/edit safeguard is separate from, and sits underneath, the
delete-approval guard. See [Deletions & the safeguard](/safety/deletions/).

## Non-destructive on unknown digests

Proton Drive doesn't always expose a usable SHA-1 for a remote file. When a remote file's
digest is unavailable, the planner treats the comparison as **unknown** and avoids any
destructive action for that path — it won't delete or overwrite based on a guess. This
conservatism is deliberate and preserved throughout the planner.

## Path safety at every boundary

Every relative path that enters the engine from an external source — a remote listing, a
local scan, a conflict target — is validated before it is joined onto a root. Absolute
paths, `..` components, and root-prefix escapes are rejected, so a malicious or malformed
remote entry can't cause a write outside the sync root. Local destination writes are also
symlink-aware: the daemon refuses a write that would land outside the root through a
symlinked directory.

## Next

- [Change detection](/concepts/change-detection/) — how the engine finds remote changes fast.
- [Architecture](/concepts/architecture/) — the module map behind this pipeline.
- [Conflicts](/safety/conflicts/) — resolving `.proton-cloud` sidecars.
