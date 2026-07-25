---
title: Proton Drive Sync Engine
description: Developer and user guide for the Rust Proton Drive bidirectional sync daemon and control CLI
---

## Proton Drive Sync Engine

<img src="assets/icon.svg" alt="Proton Drive Sync Engine icon" width="96" height="96">

`proton-drive-sync-engine` is a Rust prototype for bidirectional file synchronization between a local folder and Proton Drive. It is split into a long-running daemon, `proton-syncd`, and a small control CLI, `proton-sync`.

The project is designed as a focused sync core: local filesystem scanning, Proton Drive CLI integration, SQLite-backed sync state, Unix socket IPC, and deterministic sync planning live in separate modules so behavior can be tested and evolved safely.

> [!CAUTION]
> This is an early prototype. Test it with disposable folders and verified backups before pointing it at data you cannot afford to lose. Sync is bidirectional and deletions propagate: removing a previously synced file on one side removes it from the other. A file or folder deleted on Proton Drive is deleted from your local disk **permanently** — it is not moved to your OS trash, and folder deletions remove the whole subtree. Always preview with `--dry-run` and check the `destructive_actions` count before running the daemon unattended. The very first run against an existing remote is a non-destructive two-way merge (nothing is deleted); see [Sync Behavior](#sync-behavior).

## What It Does

* Watches a local directory for file changes
* Lists, uploads, downloads, and deletes remote files through the `proton-drive` CLI
* Tracks sync state in a local SQLite database
* Plans bootstrap and ongoing sync actions from local, remote, and indexed file state
* Creates conflict sidecar files using the `.proton-cloud` naming convention
* Exposes `status`, `pause`, `resume`, and `syncnow` commands over a local Unix socket
* Prints a dry-run sync plan without uploading, downloading, deleting, or updating the index
* Loads daemon settings from a TOML config file with explicit CLI overrides
* Supports include and exclude glob patterns for selective sync
* Emits structured logs through `tracing` for daemon and dry-run diagnostics
* Persists recent daemon status history next to the local sync state
* Writes a local metrics snapshot JSON file next to the sync database
* Prevents concurrent daemon instances with an advisory lockfile

## Current Scope

This repository currently targets Unix-like systems because control-plane IPC uses Unix domain sockets.

Supported today:

* One local root directory mapped to one Proton Drive remote root
* File-level synchronization
* SHA-1 based change detection where Proton Drive exposes a file digest
* Conservative handling for remote files whose digest is unavailable
* Rename and move detection for files in either direction, and for directories renamed or moved on the remote side
* Manual control through the companion CLI
* Include and exclude glob filters over relative sync paths
* Release archive creation for binaries and user-service sample assets
* Local metrics snapshot export for file-based operational inspection

Not yet included:

* Native package-manager packages
* Cross-platform IPC for Windows
* Automated mutating live end-to-end test for the pause/resume scenario
* Rename or move detection for directories renamed or moved on the local side
* Symbolic link synchronization (symlinks under the local root are skipped)
* Distributed tracing or dashboards

## Requirements

* A Rust toolchain with edition 2024 support
* The `proton-drive` CLI installed, authenticated, and available on `PATH`
* A Unix-like operating system with Unix socket support
* A local folder reserved for sync testing
* Optional XDG environment variables for runtime and state paths

This engine does not talk to Proton Drive directly — it shells out to the separate `proton-drive` CLI for every remote operation. Install and authenticate that CLI following its own documentation, then confirm it works before starting the daemon:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

If that command fails, the daemon will fail the same way. Point the daemon at a different executable with `--proton-cli /path/to/proton-drive` when it is not on `PATH`.

## Install and Build

Clone the repository, then build both binaries:

```bash
cargo build --all-targets
```

Run the validation suite used by contributors:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## Quick Start

Create a local test folder:

```bash
mkdir -p /tmp/proton-sync-demo
```

Preview the sync plan first — always do this before the first real sync of a new folder, so you can see (and sanity-check) what the daemon would upload, download, or delete without changing anything:

```bash
cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --dry-run
```

Once the plan looks right, start the daemon in one terminal:

```bash
cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder
```

Use the control CLI from another terminal:

```bash
cargo run --bin proton-sync -- status
cargo run --bin proton-sync -- history
cargo run --bin proton-sync -- syncnow
cargo run --bin proton-sync -- pause
cargo run --bin proton-sync -- resume
```

By default, the daemon keeps all of its persistent state — the SQLite index, its status/metrics sidecars, and the instance lockfile — in a `.sync` directory at the top of your local sync folder (`<local-root>/.sync`), which the engine always ignores when scanning and planning. The control socket stays in `$XDG_RUNTIME_DIR/proton-sync.sock` (a session-scoped runtime endpoint, not persistent state) when that runtime directory is available.

## Daemon Usage

```bash
cargo run --bin proton-syncd -- \
  --config proton-sync.toml \
  --local-root /path/to/local/folder \
  --remote-root /Drive/RemoteFolder \
  --db-path sync_index.db \
  --socket-path /tmp/proton-sync.sock \
  --lockfile-path /tmp/proton-sync.lock \
  --scan-interval-secs 300 \
  --include 'Documents/**' \
  --exclude '**/*.tmp' \
  --proton-cli proton-drive \
  --proton-timeout-secs 60 \
  --proton-list-attempts 2 \
  --dry-run
```

| Flag | Default | Description |
| ------ | ------- | ----------- |
| `--config <PATH>` | None | TOML file with daemon settings |
| `--local-root` | Required unless configured | Local directory to watch and reconcile |
| `--remote-root` | Required unless configured | Proton Drive folder used as the remote sync root |
| `--db-path` | XDG state path | SQLite index path; relative values are stored under `local-root` |
| `--socket-path` | XDG runtime path | Unix socket used by `proton-sync` |
| `--lockfile-path` | XDG runtime path | Advisory lockfile used to prevent duplicate daemon instances |
| `--scan-interval-secs` | `300` | Periodic reconciliation interval in seconds |
| `--include <GLOB>` | None | Limits sync to paths matching one or more relative glob patterns |
| `--exclude <GLOB>` | None | Excludes paths matching one or more relative glob patterns; exclude wins over include |
| `--proton-cli` | `proton-drive` | Path to the Proton Drive CLI executable |
| `--proton-timeout-secs` | `60` | Maximum time to wait for each Proton Drive CLI command |
| `--proton-list-attempts` | `2` | Attempts for read-only remote listings; uploads, downloads, and deletes are not retried |
| `--dry-run` | `false` | Prints the current sync plan as JSON and exits without changing local files, remote files, or the index |
| `--no-dry-run` | None | Overrides `dry_run = true` from a config file |

`--remote-root` is an absolute path inside your Proton Drive (for example `/Drive/RemoteFolder`), the same path you would pass to `proton-drive filesystem list`. If that folder does not exist yet, the daemon creates it and uploads your local content into it — so a mistyped remote root silently starts populating a brand-new folder rather than failing. Confirm the path with `proton-drive filesystem list --json <remote-root>` first.

Stop the daemon with `Ctrl+C` or `SIGTERM`. On shutdown, the daemon removes its Unix socket.

## Config File

Use `--config <PATH>` to load daemon settings from TOML. This is the recommended shape for service managers because it keeps long paths and filter rules out of the unit file.

```toml
local_root = "/home/me/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
scan_interval_secs = 300
proton_cli = "proton-drive"
proton_timeout_secs = 60
proton_list_attempts = 2
include = ["Documents/**"]
exclude = ["**/*.tmp"]
dry_run = false
```

Every daemon flag has a matching config key. In addition to the keys shown above, `db_path`, `socket_path`, and `lockfile_path` may be set in the config file; when omitted they fall back to the XDG defaults. The `[delete_approval]` table (`remote`/`local`, both defaulting to `true`) sets the daemon-wide default for the [delete-approval guard](#delete-approval); the `--no-delete-approval` flag is the CLI equivalent of setting both to `false`. Keys also accept hyphenated aliases (for example `local-root`), and unknown keys are rejected so typos fail fast.

Explicit CLI flags override values from the config file. For example, you can keep normal settings in `proton-sync.toml` and run one filtered preview with `--config proton-sync.toml --include 'Documents/**' --dry-run`. Use `--no-dry-run` when a config file sets `dry_run = true` but you want to start the daemon normally. Note that `--include` and `--exclude` are each all-or-nothing: passing `--include` on the command line replaces the config file's include list entirely (and, independently, `--exclude` replaces the exclude list) rather than adding to it, so repeat every pattern in that list you still want. The two lists are independent — passing only `--include` leaves the config file's `exclude` patterns in effect.

The daemon validates empty roots, invalid include or exclude globs, zero Proton command timeouts, and zero Proton list attempts before starting. `scan_interval_secs` is clamped to a minimum of 1 second. A sample config is available at [examples/proton-sync.toml](examples/proton-sync.toml).

## Dry-Run Planning

Use `--dry-run` before the first real sync, after changing roots, or before investigating unexpected behavior. Dry-run mode scans the local root, lists the remote root through `proton-drive filesystem list --json`, reads the existing SQLite index in read-only mode when it exists, prints a summary plus the planned actions, and exits.

```bash
cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --dry-run
```

Example output:

```json
{
  "summary": {
    "total": 1,
    "uploads": 0,
    "downloads": 1,
    "remote_directories_created": 0,
    "local_directories_created": 0,
    "local_moves": 0,
    "remote_moves": 0,
    "auto_links": 0,
    "conflicts": 0,
    "type_conflicts": 0,
    "remote_deletes": 0,
    "local_deletes": 0,
    "purges": 0,
    "skipped_unsupported": 0,
    "destructive_actions": 0
  },
  "plan": [
    {
      "path": "notes.txt",
      "destination_path": null,
      "action": "download",
      "entity_kind": "file",
      "conflict_path": null,
      "remote_id": "remote-file-id"
    }
  ]
}
```

Every field is always present in the output, so scripts can rely on a fixed shape. Unused summary counters are `0`, and the optional plan fields `destination_path`, `conflict_path`, and `remote_id` are `null` when they do not apply. `entity_kind` is `"file"` or `"directory"`; `destination_path` is populated only for move actions. The possible `action` values are `upload`, `download`, `create_remote_directory`, `create_local_directory`, `move_local`, `move_remote`, `auto_link`, `conflict`, `type_conflict`, `remote_delete`, `local_delete`, `purge`, and `skip_unsupported`. `destructive_actions` is the sum of `remote_deletes`, `local_deletes`, and `purges` — the number worth a second look before running for real.

Dry-run mode respects the configured `--include` and `--exclude` filters. It does not bind the IPC socket, take the daemon lock, execute uploads or downloads, delete files, or update `sync_index.db`. It still contacts Proton Drive through the configured CLI, so authentication and remote permissions must already work.

## Selective Sync

Use `--include` and `--exclude` to limit which relative paths are eligible for sync. Patterns use glob syntax and match paths relative to `local-root` and `remote-root`.

```bash
cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --include 'Documents/**' \
  --include 'Projects/**/*.md' \
  --exclude '**/*.tmp' \
  --exclude '**/.DS_Store' \
  --dry-run
```

Rules are applied consistently to local files, remote files, and existing index records:

* When no include pattern is provided, every non-ignored path is eligible.
* When one or more include patterns are provided, only matching files are eligible.
* Exclude patterns always win over include patterns.
* Conflict sidecars and index files are ignored even when they match an include pattern.
* A custom `--db-path` under `local-root` is ignored so the sync index does not become sync data.
* The persisted status history (`.status.json`) and metrics (`.metrics.json`) files next to the index are ignored for the same reason.
* The daemon's own download staging directories (any path component starting with `.proton-sync-download-`) are ignored, so a partial download left behind by a crash is never uploaded.

Run a dry-run after changing filters to confirm the planned action set before enabling regular sync.

## Logging

The daemon and dry-run path emit structured logs through `tracing`. Logs are written to stderr so dry-run JSON remains machine-readable on stdout.

Set `RUST_LOG` to control verbosity:

```bash
RUST_LOG=debug cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder
```

Useful values include `error`, `warn`, `info`, and `debug`. When `RUST_LOG` is not set, the daemon uses `info` logging. Retry attempts, unsuccessful Proton Drive CLI exits, and command timeouts are logged with structured fields such as operation, attempt, exit status, stderr, and timeout milliseconds.

## Control CLI Usage

```bash
cargo run --bin proton-sync -- [--socket-path /tmp/proton-sync.sock] <command>
```

When `--socket-path` is omitted, the control CLI uses the same default socket path as the daemon: `$XDG_RUNTIME_DIR/proton-sync.sock` or the OS temporary directory fallback.

| Command | Behavior |
| ------- | -------- |
| `status` | Prints daemon status, pause state, pending change count, message, last sync timestamp, last error, recent sync summaries, and status history |
| `history` | Prints only the recent status history array from the daemon status response |
| `pause` | Pauses automatic and manual sync work until resumed |
| `resume` | Resumes sync work |
| `syncnow` | Triggers reconciliation immediately when the daemon is not paused |
| `pending` | Lists deletions currently withheld by the delete-approval guard (see [Delete approval](#delete-approval)) |
| `approve <path> \| --all` | Approves a withheld deletion (or all) so it applies on the next sync |
| `deny <path> \| --all` | Revokes a prior approval before it has applied |

Responses are JSON so scripts can consume them directly. The `status`, `pause`, `resume`, and `syncnow` commands print the full response object (the `pending`, `approve`, and `deny` commands print human-readable text instead):

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

The `history` command prints only the `status_history` array. Each entry has `epoch_secs`, `message`, `last_error`, `plan_summary`, and `successful_sync_summary` fields. The daemon keeps the most recent entries in a JSON file next to the configured SQLite database. A restarted daemon reloads that history, so recent failures and successful summaries remain visible through `proton-sync status` and `proton-sync history`.

The daemon also writes a metrics snapshot next to the configured SQLite database using the `.metrics.json` suffix. For the default `sync_index.db`, the metrics path is `sync_index.metrics.json`. This file is intended for local inspection by scripts or service monitors and is ignored by sync scans.

## Running as a User Service

For local testing on a systemd-based Linux workstation, install the binaries into your Cargo bin directory and run the daemon as a user service. The repository includes sample assets at [examples/proton-sync.toml](examples/proton-sync.toml) and [examples/systemd/proton-syncd.service](examples/systemd/proton-syncd.service).

```bash
cargo install --path .
install -Dm600 examples/proton-sync.toml ~/.config/proton-sync/proton-sync.toml
install -Dm644 examples/systemd/proton-syncd.service ~/.config/systemd/user/proton-syncd.service
```

You can also install the sample assets with the helper script:

```bash
examples/systemd/install-user-service.sh
```

The helper installs the sample config with mode `0600`, installs the service with mode `0644`, reloads the user systemd manager, and keeps an existing config file unless you pass `--force-config`. Use `--enable` and `--start` when you want the helper to enable or start the service after installation.

Edit `~/.config/proton-sync/proton-sync.toml` for your local and remote roots before enabling the service. The sample service calls `proton-syncd --config %h/.config/proton-sync/proton-sync.toml` and relies on the daemon's XDG defaults for the socket, lockfile, and index paths.

Enable and inspect the service:

```bash
systemctl --user daemon-reload
systemctl --user enable --now proton-syncd.service
systemctl --user status proton-syncd.service
journalctl --user -u proton-syncd.service -f
```

Use the default runtime socket when controlling the service:

```bash
proton-sync status
```

Run a dry-run from the shell before enabling the service whenever you change `--local-root`, `--remote-root`, or `--proton-cli`.

The minimal release asset manifest at [examples/packaging/release-assets.toml](examples/packaging/release-assets.toml) lists the binaries, sample config, systemd unit, and install helper expected in a user-service distribution.

Build a release archive containing the binaries and service assets with:

```bash
examples/packaging/build-release-archive.sh
```

By default, the archive is written under `target/dist`. Use `--archive-path <PATH>` to choose another output path.

## Sync Behavior

The sync planner compares three sources of truth:

* Current local files under `local-root`
* Current remote files returned by `proton-drive filesystem list --json <remote-root>`
* The last synced state stored in SQLite

During the first run, the engine bootstraps state. Bootstrap is **non-destructive** — it never deletes anything on either side:

* Local-only files are uploaded
* Remote-only files are downloaded
* Matching local and remote files are linked in the index
* Different local and remote content creates a conflict sidecar, keeping both versions
* Empty directories are created to match on the other side

After bootstrap, the planner compares each side against the last synced state and chooses per path: upload, download, create a local or remote directory, move/rename, auto-link (record that both sides already match), conflict, remote delete, local delete, or purge (drop a stale index row once both sides are already gone). When a remote file exists but Proton Drive does not expose a usable SHA-1 digest, the planner avoids destructive assumptions and uses non-destructive handling.

### Deletions and the delete/edit safeguard

Once a file has been synced, deleting it on one side propagates the deletion to the other on the next reconcile:

* Deleting a local file removes it from Proton Drive by moving it to Proton's **trash**, where it stays recoverable.
* Deleting a file or folder on Proton Drive removes it from your local disk **permanently** — a direct filesystem delete, not a move to your OS trash. Folder deletions are recursive and remove the entire subtree.

A deletion is only propagated when the other side has **not** changed since the last sync. If you delete a file locally but it was edited on Proton Drive since the last sync, the remote edit is restored to your local folder rather than the remote copy being deleted; to make the deletion stick, delete it again once the file is back in sync on both sides. In the mirror case — you edit a file locally that was deleted on Proton Drive — your local edit is preserved rather than erased. Either way the surviving edit wins over the delete.

Preview any run with `--dry-run` and check the `destructive_actions` count before letting the daemon run unattended.

### Delete approval

On top of the delete/edit safeguard above, a **delete-approval guard** withholds destructive actions until you approve them. It is **on by default** and **directional**, so you decide each direction independently:

* `remote` — approval to delete a file **on Proton Drive** because you removed it locally (a `RemoteDelete`).
* `local` — approval to delete a file **on your local disk** because it was removed/trashed on Proton Drive (a `LocalDelete`, the permanent-local-delete direction).

(Index-only cleanup — dropping a stale record once both sides are already gone — destroys no data and is never gated.)

While a deletion is guarded, the daemon withholds it and lists it for you:

```bash
proton-sync pending                 # show what is withheld, and which direction
proton-sync approve notes/old.txt   # approve one; applies on the next sync
proton-sync approve --all           # approve everything currently pending
proton-sync deny notes/old.txt      # revoke an approval before it applies
proton-sync syncnow                 # apply approved deletions now
```

An approval is pinned to exactly what you saw (a file's last-synced content hash, or a folder's remote id): if the file changes before the delete applies, the approval no longer matches and nothing is deleted. Nothing is ever deleted while a deletion sits unapproved.

**Per-directory settings.** The guard's default comes from the daemon config (`[delete_approval]` in the config file, or `--no-delete-approval` to turn both directions off globally). You can override it for any subtree by dropping a `.proton-sync.toml` file in a directory:

```toml
# <local_root>/Downloads/.proton-sync.toml
# Let deletions under Downloads/ flow without approval, in both directions.
[delete_approval]
remote = false
local = false
```

A directory file applies to that directory and everything beneath it, and the **nearest** file wins over shallower ones — the same inheritance model as `.gitignore`. Any option a file leaves unset inherits from its parent. Changes take effect on the next reconcile (within the events poll interval, or immediately with `proton-sync syncnow`).

These files are **machine-local**: they are ignored by sync and never uploaded to Proton Drive, so a file on the remote can never silently weaken your local delete protection. A malformed or unreadable settings file is ignored and the guard stays on (fail-safe).

### Change detection

Changes are detected by SHA-1 content hash where Proton Drive exposes a digest. To avoid re-reading and re-hashing an unchanged tree on every periodic scan, the daemon uses a quick-check: a local file whose size and modification time both match the last synced record is assumed unchanged and its stored hash is reused. As a consequence, a change that somehow preserves both the byte size and the whole-second mtime of a file is not noticed until the next size- or mtime-changing edit — the same trade-off `rsync` makes by default.

### What is and isn't synced

* Regular files and directories under `local-root` are synced. **Symbolic links are skipped** in both directions: the scanner does not follow or upload them, and the daemon refuses to write a remote entry through a symlinked directory that would lead outside the sync root.
* Proton Docs and Sheets are reported as unsupported skip actions and left untouched on both sides, because the Proton Drive CLI cannot download those native document types as files.

## Conflict Files

When both sides changed to different content, the daemon preserves the local file and downloads the remote version into a sidecar path (if both sides changed to the *same* content, it simply links them with no sidecar):

```text
notes.txt -> notes.proton-cloud.txt
archive -> archive.proton-cloud
```

Conflict sidecars are ignored by regular local scans so they do not create new sync records. To resolve a conflict:

* **Keep your local version:** delete the `.proton-cloud` sidecar. Removing it marks the original file as modified, and the next reconcile uploads your local version over the remote.
* **Adopt the remote version:** replace your local file with the sidecar's contents (for example, move the sidecar over the original), then delete any leftover sidecar. The daemon treats the updated file as your new local version and syncs it.

Until you resolve it, the conflicting path is left as-is on both sides so no edit is lost.

## Safety and Security Notes

* Start with a disposable folder until you understand the sync plan and Proton Drive CLI behavior.
* Keep external backups for important files.
* The daemon rejects absolute, parent-directory, and root-escaping remote paths before joining them to the local sync root.
* Local destination writes are also symlink-aware: before writing a downloaded or created entry, the daemon resolves the deepest existing parent and refuses the write if it would land outside the sync root through a symlinked directory. (The scanner likewise skips symlinks, so they are never uploaded.)
* Selective sync filters are applied before planning, so excluded index records are not purged as missing files.
* The delete-approval guard is on by default and withholds deletions (in both directions) until you approve them; it fails safe (stays on) when a per-directory settings file is malformed, and those settings files are never synced. See [Delete approval](#delete-approval).
* The IPC socket is restricted to mode `0600` after binding, and control-connection reads and writes are bounded by a timeout so an idle or stalled client cannot stall the daemon.
* The advisory lockfile prevents two live daemon instances from using the same configuration at once.
* The daemon shells out to the configured `proton-drive` executable. Use a trusted executable path.
* Proton Drive CLI calls are bounded by a timeout so a hung subprocess cannot block reconciliation indefinitely; a shutdown signal can also interrupt a stuck call directly, well before its full timeout would otherwise elapse.
* When a command is cancelled or times out, the daemon terminates the CLI process's entire process group (not just the directly spawned process), so a subprocess that has forked helpers of its own cannot keep running unnoticed after the daemon has already reported the operation as cancelled.
* Read-only remote listings may be retried after a transient CLI failure; uploads, downloads, and deletes are not automatically retried to avoid duplicate or surprising side effects.
* Reconciliation commits SQLite index mutations only after all planned side effects succeed. If a later action fails, earlier successful actions are not marked as synced in the index.
* Live end-to-end testing must follow the safety gates in [docs/live-e2e-test-plan.md](docs/live-e2e-test-plan.md) before enabling mutating Proton Drive scenarios.

## Design Notes

* [docs/live-e2e-test-plan.md](docs/live-e2e-test-plan.md) defines the opt-in safety gates and scenario matrix for live upload, download, rename, move, delete, directory id/trash/rename, and conflict tests, plus the still-pending pause/resume scenario.
* [docs/rename-detection-design.md](docs/rename-detection-design.md) defines the planner and index approach for safe rename and move detection, including the directory-level remote-to-local design and its local-to-remote limitation.

## Project Layout

```text
src/
  bin/
    proton-sync.rs   Control CLI entry point
    proton-syncd.rs  Daemon CLI entry point
  config.rs          Daemon config loading, merge/default rules, and validation
  daemon.rs          Runtime loop, watcher, IPC handling, reconciliation execution
  index.rs           SQLite schema, local scanning, file hashing, index persistence
  ipc.rs             Unix socket request and response protocol
  lib.rs             Shared result and path validation helpers
  paths.rs           XDG-aware default state, socket, and lockfile paths
  proton.rs          Proton Drive CLI wrapper and remote JSON parser
  sync.rs            Sync planning matrix and conflict naming
examples/
  packaging/         Minimal release asset manifest
  proton-sync.toml   Sample daemon config file
  systemd/           Sample systemd user service and install helper
docs/
  live-e2e-test-plan.md       Safety plan for mutating live tests
  rename-detection-design.md  Design for rename and move detection
```

## Development Workflow

Format, lint, and test before opening or updating a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Useful focused commands:

```bash
cargo test sync::tests
cargo test proton::tests
cargo test daemon::tests
cargo test --test dry_run_cli
cargo test --test example_assets
cargo test --test ipc_cli
bash -n examples/packaging/build-release-archive.sh
```

When changing sync behavior, add regression tests near the planner in `src/sync.rs`. When changing local filesystem or Proton JSON safety boundaries, add tests around the boundary that first accepts external paths.

The repository also includes an ignored live smoke test for authenticated Proton Drive CLI installations. It lists a configured remote root but does not upload, download, or delete data:

```bash
PROTON_SYNC_LIVE_REMOTE_ROOT=/Drive/RemoteFolder \
  cargo test --test proton_live --all-features -- --ignored
```

Set `PROTON_SYNC_LIVE_CLI=/path/to/proton-drive` when the executable is not available as `proton-drive` on `PATH`.
Set `PROTON_SYNC_LIVE_TIMEOUT_SECS` and `PROTON_SYNC_LIVE_LIST_ATTEMPTS` to tune the read-only listing policy for slower live environments. Both values must be greater than zero.

## Troubleshooting

If `proton-sync` cannot connect, confirm the daemon is running and the client uses the same `--socket-path`. When no socket is configured, both binaries default to `$XDG_RUNTIME_DIR/proton-sync.sock` and fall back to the OS temporary directory.

If the daemon exits with a lockfile error, another live daemon may already hold the advisory lock. Stop the running daemon or use a different `--lockfile-path` for an isolated test.

If remote operations fail, run the equivalent `proton-drive` command directly to confirm authentication, permissions, and remote folder names.

If sync behavior looks unexpected, pause the daemon and inspect the local folder plus the SQLite index before resuming. The index lives at your `--db-path`, which defaults to `<local-root>/.sync/sync_index.db` (the `.sync` state directory is ignored by scanning, so it is never uploaded):

```bash
cargo run --bin proton-sync -- pause
sqlite3 <local-root>/.sync/sync_index.db 'select * from file_index order by file_path;'
```

## License

This project is licensed under the terms in [LICENSE](LICENSE).
