---
title: FAQ
description: Short answers to common questions about how the engine behaves.
sidebar:
  order: 2
---

## Is it safe to use on important files?

It's an **early prototype**. Sync is bidirectional and deletions propagate — a remote delete
removes the local file **permanently**. Use disposable folders and keep backups until you
trust it, always [dry-run](/safety/dry-run/) first, and leave the
[delete-approval guard](/safety/delete-approval/) on. Read [Deletions](/safety/deletions/)
before your first real run.

## Does it talk to Proton Drive directly?

Mostly not. Every remote **file** operation (list, upload, download, delete) shells out to
Proton's own **`proton-drive` CLI**. Change detection does read Proton's volume-event stream
directly over HTTPS, but authenticated by **reusing the CLI's logged-in session** (from your
keyring) rather than any credentials of its own. Either way, `proton-drive` must be
installed, on `PATH`, and authenticated.

## What happens on the very first sync?

The first pass is a **non-destructive merge**: local-only files upload, remote-only files
download, matching files are linked, and files that differ are kept as **both** (a
`.proton-cloud` conflict sidecar). Nothing is deleted on the first pass.

## What does a remote delete do to my local files?

It deletes them **permanently** from disk — a direct filesystem removal, not a move to your
OS trash — and folder deletions are recursive. A *local* delete, by contrast, moves the
Proton Drive copy to Proton's **Trash**, where it's recoverable. See
[Deletions](/safety/deletions/).

## Why was my deletion "withheld"?

The [delete-approval guard](/safety/delete-approval/) is on by default and holds destructive
deletions until you approve them. Use `proton-sync pending` / `approve`, the app's
**Deletions** screen, or relax the guard per folder.

## Can I sync only part of a folder?

Yes — use include/exclude globs. See [Selective sync](/daemon/selective-sync/). Excluded
paths are taken out of sync entirely and are never treated as "deleted".

## Does it detect renames and moves?

Yes for files in either direction, and for directories renamed/moved on the **remote** side.
A directory renamed **locally** is currently handled as a delete-and-recreate, not a move.

## Are symlinks synced?

No. Symlinks under the local root are skipped in both directions, and the daemon refuses to
write through a symlinked directory that would escape the sync root.

## Can I run more than one daemon?

No — **one daemon per user**. Every daemon shells the same `proton-drive` CLI, whose shared
SQLite cache isn't concurrency-safe, so a user-global lock enforces a single instance.

## Does closing the desktop app stop syncing?

No. Closing the window **hides it to the tray**; the app process and the daemon keep running.
Syncing is the daemon's job, independent of the window. See [Tray](/desktop/tray/).

## Why isn't there a transfer percentage?

The daemon reports in-flight progress as `[i/N]` file counts on stderr only — it never
crosses the control socket — so the app shows file counts and an indeterminate bar rather
than a fabricated percentage.

## Does it work on Windows or macOS?

The control plane uses **Unix domain sockets**, so it's Unix-only, and **Linux is the only
supported target** — keyring session reuse (`secret-tool`/libsecret), the desktop app, and
the native packaging are all Linux-specific. Windows is out of scope.

## Where does it keep its state?

In `<local-root>/.sync/` (the SQLite index and its sidecars, plus the per-root lockfile),
which the engine always ignores. The control socket is in `$XDG_RUNTIME_DIR`. See the
[architecture](/concepts/architecture/#state-on-disk).
