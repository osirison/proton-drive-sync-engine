---
title: What it is
description: An overview of the Proton Drive Sync engine, CLI, and desktop app, and how the pieces fit together.
sidebar:
  order: 1
---

**Proton Drive Sync** keeps a local folder and a Proton Drive folder in sync, both
ways. It is a focused sync core written in Rust, split into small, testable modules,
plus a desktop app that puts a face on it.

It ships as three things:

| Piece | Binary | What it is |
| --- | --- | --- |
| Daemon | `proton-syncd` | A long-running background service that watches a local folder and reconciles it with Proton Drive. Also hosts the one-shot `--dry-run` planner. |
| Control CLI | `proton-sync` | A thin client that talks to the daemon over a Unix socket — `status`, `pause`, `resume`, `syncnow`, `history`, and delete-approval commands. |
| Desktop app | `proton-sync-gui` | A [Tauri](https://tauri.app/) app for Linux: live status, activity, conflict resolution, dry-run review, settings, onboarding, and a tray indicator. |

The daemon and CLI are the engine; the app is a **thin client** over them. It owns no
sync logic and no index of its own — it reads the daemon's control socket and on-disk
state and shows it. That means the CLI, the app, and a script all see the same truth.

## How it decides what to do

The engine reconciles **three sources of truth** into a plan, then executes the plan:

1. **Local files** under your local root, scanned and SHA-1 hashed.
2. **Remote files** from Proton Drive, listed through the `proton-drive` CLI.
3. **The last-synced baseline**, stored in a local SQLite index.

Comparing all three — not just the two live sides — is what lets the engine tell a
*new* file from a *deleted* one, and an *edit* from a *move*. The planner is a pure
function with no I/O: given the three maps, it emits a list of actions
(upload, download, move, conflict, delete, …). A separate step carries them out and,
only after they all succeed, commits the new baseline.

Read [How sync works](/concepts/how-sync-works/) for the full model.

## What it does today

- Watches one local root mapped to one Proton Drive remote root, and syncs **files and
  directories** in both directions.
- Detects changes by **SHA-1 content hash** where Proton Drive exposes a digest, and
  handles files with no usable digest conservatively (never a destructive guess).
- Detects **renames and moves** for files in either direction, and for directories
  renamed or moved on the remote side.
- Uses Proton's **volume-event stream** for fast, `O(changes)` incremental reconciles,
  with an optional periodic full-tree scan (off by default) as a safety net. See
  [Change detection](/concepts/change-detection/).
- Preserves both versions on a real **conflict** via a `.proton-cloud` sidecar.
- Withholds destructive deletes behind a directional, per-item **delete-approval** guard
  that is on by default.
- Supports **selective sync** with include/exclude globs, applied to local files, remote
  files, and the index alike.
- Previews any run with **`--dry-run`**, printing the exact plan as JSON.

## What isn't included yet

- **Unix-only.** Control-plane IPC uses Unix domain sockets, so Windows is out of scope
  for now. Linux is the primary target (Fedora, Ubuntu, Arch).
- **One folder pair** per daemon. The config and UI are shaped for many, but today it's
  one local root ↔ one remote root.
- **No symlink sync.** Symlinks under the local root are skipped in both directions.
- **No local-side directory rename detection.** A directory renamed *locally* is planned
  as a delete-and-recreate rather than a move (remote-side directory renames are detected).
- **No Proton Docs/Sheets sync.** Native Proton document types are reported as unsupported
  skips and left untouched, because the `proton-drive` CLI cannot download them as files.

## A note on the Proton Drive CLI

The engine leans on Proton's own separate `proton-drive` command-line tool. Every remote
**file** operation (list, upload, download, delete) shells out to it, and change detection
reads Proton's volume-event stream directly over HTTPS — authenticated by **reusing that
tool's logged-in session** from your desktop keyring, not credentials of its own. Either
way, `proton-drive` must be installed, authenticated, and on your `PATH` before the daemon
can do anything. [Installation](/start/installation/) covers this.

:::caution
This is an early prototype. Sync is bidirectional and deletions propagate across sides.
Always preview with `--dry-run` and keep backups until you trust it. Start with the
[Safety](/safety/deletions/) section.
:::
