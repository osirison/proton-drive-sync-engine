---
title: Daemon command reference
description: Every proton-syncd flag, its default, and what it does.
sidebar:
  order: 1
---

`proton-syncd` is the long-running background daemon. It gives every folder pair a first
reconcile at startup, one after another, then re-checks each pair on a periodic timer, on the
faster events-poll interval while Proton's change stream is live, and on demand via the control
socket. Filesystem events do not start a pass by themselves: the watcher queues the changed
paths, and the next pass for that pair picks them up. It also hosts the one-shot `--dry-run`
planner. One daemon syncs [several folder pairs](/daemon/folder-pairs/) if the config declares
them.

```bash
proton-syncd \
  --config proton-sync.toml \
  --local-root /path/to/local/folder \
  --remote-root /Drive/RemoteFolder \
  --scan-interval-secs 300 \
  --include 'Documents/**' \
  --exclude '**/*.tmp'
```

Stop it with `Ctrl+C` or `SIGTERM`; it removes its Unix socket on shutdown.

## Flags

| Flag | Default | Description |
| --- | --- | --- |
| `--config <PATH>` | none | TOML file with daemon settings. See [Configuration](/daemon/configuration/). |
| `--local-root <PATH>` | required¹ | Local directory to watch and reconcile. |
| `--remote-root <PATH>` | required¹ | Proton Drive folder used as the remote sync root (an absolute path inside your Drive, e.g. `/Drive/RemoteFolder`). |
| `--db-path <PATH>` | `<local-root>/.sync/sync_index.db` | SQLite index path; relative values are stored under the local root. |
| `--download-batch-size <N>` | `25` | How many planned downloads to bundle into one `proton-drive` invocation (grouped by destination folder, checkpoint-committed per chunk; each invocation's timeout budget is the Proton command timeout × the chunk's file count). `1` downloads one file per invocation. Must be greater than zero. |
| `--socket-path <PATH>` | `$XDG_RUNTIME_DIR/proton-sync.sock` | Unix socket the control CLI connects to. |
| `--lockfile-path <PATH>` | `<local-root>/.sync/…lock` | Advisory lockfile that prevents duplicate daemon instances on this root. |
| `--scan-interval-secs <N>` | `300` | Periodic reconciliation interval, in seconds (clamped to a 1-second minimum). |
| `--include <GLOB>` | none | Limit sync to paths matching one or more relative globs. Repeatable. |
| `--exclude <GLOB>` | none | Exclude paths matching one or more relative globs. Repeatable. Exclude wins over include. |
| `--proton-cli <PATH>` | `proton-drive` | Path to the Proton Drive CLI executable. |
| `--proton-timeout-secs <N>` | `60` | Max time to wait for each `proton-drive` CLI command. |
| `--proton-list-attempts <N>` | `2` | Attempts for read-only remote listings. Uploads, downloads, and deletes are **never** retried. |
| `--dry-run` | `false` | Print the current plan as JSON and exit without changing files or the index. See [Dry-run](/safety/dry-run/). |
| `--no-dry-run` | — | Override `dry_run = true` from a config file. Conflicts with `--dry-run`. |
| `--pair <NAME>` | the default (first) pair | With `--dry-run`, preview this folder pair instead of the default one. Matched exactly; an unknown name is an error that lists the configured pairs; without a dry run it is an error, because the daemon runs every pair. See [Multiple folder pairs](/daemon/folder-pairs/). |
| `--events-driven` | on² | Detect remote changes from Proton's volume-event stream. On by default; the flag exists for explicitness / to override a config `events_driven = false`. |
| `--no-events-driven` | — | Opt out of event-driven detection; use full-tree-walk-only remote change detection. Conflicts with `--events-driven`. |
| `--events-full-scan-every <N>` | `0` | Force a full-tree reconvergence every *N* incremental passes (event-driven mode only). **`0` (the default) disables the periodic resync** — after the one-time startup snapshot the daemon stays purely event-driven; set a positive *N* to reinstate a self-healing full walk. |
| `--no-delete-approval` | — | Disable the [delete-approval guard](/safety/delete-approval/) globally, in both directions. By default deletions are withheld pending approval. |

¹ `--local-root` and `--remote-root` are required **unless** supplied by a `--config` file.
² Event-driven mode is on by default; when the reused CLI session/keyring is unavailable at
runtime, the daemon degrades to snapshot scans automatically.

**With more than one folder pair**, the flags that describe a folder (`--local-root`,
`--remote-root`, `--db-path`, `--lockfile-path`, `--scan-interval-secs`, `--download-batch-size`,
`--include`, `--exclude`, `--events-driven`, `--no-events-driven`, `--events-full-scan-every`,
`--warm-start`, `--no-warm-start`, `--warm-start-full-walk-every`,
`--warm-start-max-cursor-age-secs`, `--no-delete-approval`, `--deletion-policy`,
`--local-delete-mode`, `--conflict-suffix`) are refused at startup, naming the flag: a flag cannot
say which pair it amends. Set them in the `[[pair]]` table instead. The daemon-wide flags
(`--config`, `--socket-path`, `--proton-cli`, `--proton-timeout-secs`, `--proton-list-attempts`,
`--log-level`) and the mode flags (`--dry-run`, `--no-dry-run`, `--full-walk`, `--pair`) work
beside any number of pairs. With one pair every flag amends it, as always.

## Notes on individual flags

### `--remote-root`

This is an absolute path inside your Proton Drive — the same path you'd pass to
`proton-drive filesystem list`. **If the folder doesn't exist, the daemon creates it and
uploads your local content into it**, so a typo silently starts populating a brand-new
folder rather than failing. Confirm the path first:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

### `--local-root`

The daemon creates the folder when it **starts**, if it doesn't exist. If it can't (a parent
that is a file, a read-only filesystem), or the folder is gone later — an unplugged drive, a
deleted tree — the daemon **keeps running** instead of exiting: that folder pair shows the
reason as its error (`proton-sync status`, and the desktop app's error state rather than its
first-run wizard), any other pair keeps syncing, and the pair is tried again on its normal
cadence. A folder that disappears *while the daemon runs* is never created again: put it back
(re-mount the drive, restore the folder) and the next attempt picks it up.

**If the folder lives on a drive, mount the drive — don't restart the daemon.** A restart
creates a missing folder, and a mount point that is missing only because its drive isn't
mounted would be created as an empty folder on the wrong disk and then treated as your sync
folder ([#426](https://github.com/osirison/proton-drive-sync-engine/issues/426)). Restart the
daemon only when the folder is truly gone and should be created empty.

Two cases are held on purpose, so nothing is deleted on a guess:

- A folder **replaced by an empty one** while files are recorded as synced for it (an
  unmounted drive's empty mount point, or a folder deleted and made again) leaves the pair
  unavailable, saying so in its status, until the folder holds something again. Nothing is
  deleted, created or downloaded in the meantime, whether the pair's state lives in the
  default `.sync` directory or elsewhere. To start the folder over from Proton instead, run
  `proton-sync reset-index --yes`: it downloads everything and deletes nothing. Restoring only
  part of the folder ends the hold, and the first pass after that withholds every deletion for
  approval whatever your delete-approval setting says, so the files not yet restored wait as
  pending deletions until you approve them, restore them, or start over — a restart of the
  daemon does not spend that; it remembers that they wait. The hold itself lives in the running
  daemon only: a restart while the pair is held does not keep it.
- A folder deleted and made again **with the pair's state inside it** (the default `.sync`
  directory holds the index and the lock) and **with files in it** is prepared again from
  scratch on the next attempt: the index is new, so the pair adopts what is there, downloads
  what is missing and deletes nothing. (If the new folder is empty, the case above applies —
  also when it is emptied *after* the daemon noticed and *before* it could prepare the pair
  again, whether it was replaced or emptied in place: the daemon remembers how many items the
  old folder had recorded, and if that count could not be read it assumes there were items. A
  folder that was never synced, or whose baseline recorded nothing, is not held.)

A folder replaced while the daemon runs is told from the old one by the directory's identity
(device, inode and birth time). On a filesystem that reports a different identity each time it
is asked — some FUSE mounts — the daemon cannot tell a replaced folder from the same one, says so
once when it starts (`the filesystem names the folder differently on each look`), and checks
only that the folder is there: for such a folder nothing is held or withheld for a replacement,
and your delete-approval setting applies as it is. Keep delete approval on for a folder on such
a filesystem. (A folder like that whose state went with it, and that holds nothing, is still
held: that judgement needs no identity.) A plan (`proton-sync plan`, the desktop app's *Check
again*) on a replaced folder is refused with the same message a sync gives, so it never offers
the replacement's deletions for approval.

While a sync pass runs, the daemon checks that the folder is still the one the pass started on
before each action, and again immediately before an upload or a remote move, which can follow
a second-long call to Proton. A folder swapped in the middle ends the pass at once with
nothing further done to it. A check that fails for another reason (an I/O error on a network
mount) is treated the same way, because the daemon cannot tell what folder it would be working
on: the pass ends and is tried again, and the adoptions it had already worked out are kept. A
swap that lands during one transfer's own call to Proton cannot be caught by any check; the
pass ends at the next one with its place in Proton's change stream held, and the next sync
judges the replacement.

### `--include` / `--exclude`

Both are repeatable and match paths relative to the roots. Passing `--include` on the
command line **replaces** the config file's entire include list (and likewise for
`--exclude`), so repeat every pattern you still want. The two lists are independent. See
[Selective sync](/daemon/selective-sync/).

### `--proton-list-attempts`

Only read-only listings retry, because a failed *read* is safe to repeat. Mutating
operations (uploads, downloads, deletes) are never auto-retried, to avoid duplicate or
surprising side effects.

## Validation

Before starting, the daemon validates its resolved configuration and fails fast on: empty
roots, invalid include/exclude globs, a zero Proton command timeout, zero Proton list
attempts, and a zero download batch size. `scan_interval_secs` is clamped to a minimum of 1 second. Unknown config-file
keys are rejected so typos fail immediately.

## Exit and signals

`SIGTERM` or `Ctrl+C` triggers a graceful shutdown: the daemon removes its socket, and a
shutdown signal can interrupt an in-flight `proton-drive` call directly (well before its
timeout would elapse). A cancelled or timed-out CLI call has its **entire process group**
terminated, so forked helpers can't linger.
