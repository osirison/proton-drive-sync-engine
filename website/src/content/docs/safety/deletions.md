---
title: Deletions & the safeguard
description: How deletions propagate across sides, why a remote delete removes your local file permanently, and the delete/edit safeguard.
sidebar:
  order: 1
---

This is the most important page in the docs. Read it before you point the daemon at data
you care about.

## Deletions propagate

Once a file has been synced, **deleting it on one side propagates the deletion to the
other** on the next reconcile. That is the point of two-way sync — but it means a deletion
is a real action the engine will carry out for you, in both directions:

- **Delete a file locally** → the engine removes it from Proton Drive by moving it to
  Proton's **Trash**, where it stays recoverable until you empty the trash.
- **Delete a file (or folder) on Proton Drive** → the engine removes it from your local
  disk **permanently**. This is a direct filesystem delete, **not** a move to your OS
  trash. Folder deletions are **recursive** and remove the entire subtree.

:::danger[A remote delete is a permanent local delete]
Deleting a folder on Proton Drive deletes the matching local folder and everything inside
it, straight from disk, with no undo. There is no OS-trash safety net on the local side.
Always [preview with `--dry-run`](/safety/dry-run/) and check `destructive_actions` before
running unattended.
:::

The two directions are asymmetric on purpose: Proton Drive has a trash and your local
filesystem, in general, does not.

## The delete/edit safeguard

A deletion is only propagated when **the other side has not changed since the last sync**.
The surviving edit always wins over the delete:

- You **delete a file locally**, but it was **edited on Proton Drive** since the last sync
  → the remote edit is **restored to your local folder** rather than the remote copy being
  deleted. To make the deletion stick, delete it again once both sides are back in sync.
- You **edit a file locally**, but it was **deleted on Proton Drive** → your local edit is
  **preserved** rather than erased.

This safeguard lives in the pure planner and applies regardless of the delete-approval
guard below.

## The delete-approval guard

On top of the safeguard, a **delete-approval guard** withholds destructive actions until
you approve them. It is **on by default** and **directional**, so you approve each
direction independently. This is a full topic of its own — see
[Delete approval](/safety/delete-approval/).

In short: the first time a synced file's deletion would propagate, the daemon *withholds*
it and lists it under `proton-sync pending` (or the app's **Deletions** screen). Nothing is
deleted until you approve it.

## What "purge" is (and why it's safe)

You'll see a `purge` action in plans. A purge is **index-only cleanup**: when a file is
already gone from *both* sides, the engine drops its stale baseline row. No user data is
touched, so purge is **never** gated by the approval guard, even though the dry-run counts
it under `destructive_actions` for display.

## Non-destructive on unknown digests

If a remote file exists but Proton Drive exposes no usable SHA-1 for it, the planner treats
the comparison as *unknown* and refuses to make a destructive decision about that path. It
would rather do nothing than delete or overwrite on a guess.

## Practical rules

1. **Test with disposable folders and backups** until you trust the behavior.
2. **Dry-run before every first sync** of a new pair, and after changing roots — check
   `destructive_actions`.
3. **Leave delete-approval on** while you're learning; relax it per folder only where you
   understand the consequences.
4. **Deleting a synced folder on Proton Drive is destructive and recursive locally** — be
   deliberate about it.

## Next

- [Delete approval](/safety/delete-approval/) — configure and use the guard.
- [Dry-run preview](/safety/dry-run/) — read a plan before it runs.
- [Conflicts](/safety/conflicts/) — when both sides change, nothing is lost.
