---
title: Daemon command reference
description: Every proton-syncd flag, its default, and what it does.
sidebar:
  order: 1
---

`proton-syncd` is the long-running background daemon. It reconciles once on startup, then
watches the folder and reconciles on filesystem events, on a periodic timer, on the faster
events-poll interval, and on demand via the control socket. It also hosts the one-shot
`--dry-run` planner.

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
| `--events-driven` | on² | Detect remote changes from Proton's volume-event stream. On by default; the flag exists for explicitness / to override a config `events_driven = false`. |
| `--no-events-driven` | — | Opt out of event-driven detection; use full-tree-walk-only remote change detection. Conflicts with `--events-driven`. |
| `--events-full-scan-every <N>` | `0` | Force a full-tree reconvergence every *N* incremental passes (event-driven mode only). **`0` (the default) disables the periodic resync** — after the one-time startup snapshot the daemon stays purely event-driven; set a positive *N* to reinstate a self-healing full walk. |
| `--no-delete-approval` | — | Disable the [delete-approval guard](/safety/delete-approval/) globally, in both directions. By default deletions are withheld pending approval. |

¹ `--local-root` and `--remote-root` are required **unless** supplied by a `--config` file.
² Event-driven mode is on by default; when the reused CLI session/keyring is unavailable at
runtime, the daemon degrades to snapshot scans automatically.

## Notes on individual flags

### `--remote-root`

This is an absolute path inside your Proton Drive — the same path you'd pass to
`proton-drive filesystem list`. **If the folder doesn't exist, the daemon creates it and
uploads your local content into it**, so a typo silently starts populating a brand-new
folder rather than failing. Confirm the path first:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

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
