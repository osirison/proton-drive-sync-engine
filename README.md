---
title: Proton Drive Sync Engine
description: Developer and user guide for the Rust Proton Drive bidirectional sync daemon and control CLI
---

## Proton Drive Sync Engine

`proton-drive-sync-engine` is a Rust prototype for bidirectional file synchronization between a local folder and Proton Drive. It is split into a long-running daemon, `proton-syncd`, and a small control CLI, `proton-sync`.

The project is designed as a focused sync core: local filesystem scanning, Proton Drive CLI integration, SQLite-backed sync state, Unix socket IPC, and deterministic sync planning live in separate modules so behavior can be tested and evolved safely.

> [!CAUTION]
> This is an early prototype. Test it with disposable folders and verified backups before pointing it at data you cannot afford to lose.

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
* Prevents concurrent daemon instances with an advisory lockfile

## Current Scope

This repository currently targets Unix-like systems because control-plane IPC uses Unix domain sockets.

Supported today:

* One local root directory mapped to one Proton Drive remote root
* File-level synchronization
* SHA-1 based change detection where Proton Drive exposes a file digest
* Conservative handling for remote files whose digest is unavailable
* Manual control through the companion CLI
* Include and exclude glob filters over relative sync paths

Not yet included:

* Native package-manager packaging or generated service units
* Cross-platform IPC for Windows
* End-to-end tests against a live Proton Drive account
* Rename detection
* Metrics, distributed tracing, or dashboards

## Requirements

* A Rust toolchain with edition 2024 support
* The `proton-drive` CLI installed, authenticated, and available on `PATH`
* A Unix-like operating system with Unix socket support
* A local folder reserved for sync testing
* Optional XDG environment variables for runtime and state paths

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

Start the daemon in one terminal:

```bash
cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder
```

Preview the sync plan first when testing a new folder:

```bash
cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --dry-run
```

Use the control CLI from another terminal:

```bash
cargo run --bin proton-sync -- status
cargo run --bin proton-sync -- syncnow
cargo run --bin proton-sync -- pause
cargo run --bin proton-sync -- resume
```

By default, the daemon stores its index under `$XDG_STATE_HOME/proton-drive-sync` or `~/.local/state/proton-drive-sync`, and it listens on `$XDG_RUNTIME_DIR/proton-sync.sock` when that runtime directory is available.

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
| `--dry-run` | `false` | Prints the current sync plan as JSON and exits without changing local files, remote files, or the index |
| `--no-dry-run` | None | Overrides `dry_run = true` from a config file |

Stop the daemon with `Ctrl+C` or `SIGTERM`. On shutdown, the daemon removes its Unix socket.

## Config File

Use `--config <PATH>` to load daemon settings from TOML. This is the recommended shape for service managers because it keeps long paths and filter rules out of the unit file.

```toml
local_root = "/home/me/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
scan_interval_secs = 300
proton_cli = "proton-drive"
include = ["Documents/**"]
exclude = ["**/*.tmp"]
dry_run = false
```

Explicit CLI flags override values from the config file. For example, you can keep normal settings in `proton-sync.toml` and run one filtered preview with `--config proton-sync.toml --include 'Documents/**' --dry-run`. Use `--no-dry-run` when a config file sets `dry_run = true` but you want to start the daemon normally.

The daemon validates empty roots and invalid include or exclude globs before starting. A sample config is available at [examples/proton-sync.toml](examples/proton-sync.toml).

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
    "auto_links": 0,
    "conflicts": 0,
    "remote_deletes": 0,
    "local_deletes": 0,
    "purges": 0,
    "destructive_actions": 0
  },
  "plan": [
    {
      "path": "notes.txt",
      "action": "download",
      "conflict_path": null,
      "remote_id": "remote-file-id"
    }
  ]
}
```

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

Run a dry-run after changing filters to confirm the planned action set before enabling regular sync.

## Logging

The daemon and dry-run path emit structured logs through `tracing`. Logs are written to stderr so dry-run JSON remains machine-readable on stdout.

Set `RUST_LOG` to control verbosity:

```bash
RUST_LOG=debug cargo run --bin proton-syncd -- \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder
```

Useful values include `error`, `warn`, `info`, and `debug`. When `RUST_LOG` is not set, the daemon uses `info` logging.

## Control CLI Usage

```bash
cargo run --bin proton-sync -- [--socket-path /tmp/proton-sync.sock] <command>
```

When `--socket-path` is omitted, the control CLI uses the same default socket path as the daemon: `$XDG_RUNTIME_DIR/proton-sync.sock` or the OS temporary directory fallback.

| Command | Behavior |
| ------- | -------- |
| `status` | Prints daemon status, pause state, pending change count, message, and last sync timestamp |
| `pause` | Pauses automatic and manual sync work until resumed |
| `resume` | Resumes sync work |
| `syncnow` | Triggers reconciliation immediately when the daemon is not paused |

Responses are JSON so scripts can consume them directly:

```json
{
  "status": "running",
  "paused": false,
  "pending_changes": 0,
  "message": "daemon status",
  "last_sync_epoch_secs": null
}
```

## Running as a User Service

For local testing on a systemd-based Linux workstation, install the binaries into your Cargo bin directory and run the daemon as a user service. The repository includes sample assets at [examples/proton-sync.toml](examples/proton-sync.toml) and [examples/systemd/proton-syncd.service](examples/systemd/proton-syncd.service).

```bash
cargo install --path .
install -Dm600 examples/proton-sync.toml ~/.config/proton-sync/proton-sync.toml
install -Dm644 examples/systemd/proton-syncd.service ~/.config/systemd/user/proton-syncd.service
```

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

## Sync Behavior

The sync planner compares three sources of truth:

* Current local files under `local-root`
* Current remote files returned by `proton-drive filesystem list --json <remote-root>`
* The last synced state stored in SQLite

During the first run, the engine bootstraps state:

* Local-only files are uploaded
* Remote-only files are downloaded
* Matching local and remote files are linked in the index
* Different local and remote content creates a conflict sidecar

After bootstrap, the planner uses local and remote deltas to choose upload, download, local delete, remote delete, purge, or conflict actions. When a remote file exists but Proton Drive does not expose a usable SHA-1 digest, the planner avoids destructive assumptions and uses non-destructive handling.

## Conflict Files

When both sides changed, the daemon preserves the local file and downloads the remote version into a sidecar path:

```text
notes.txt -> notes.proton-cloud.txt
archive -> archive.proton-cloud
```

Conflict sidecars are ignored by regular local scans so they do not create new sync records. Removing a sidecar marks the original file as modified for a future reconciliation pass.

## Safety and Security Notes

* Start with a disposable folder until you understand the sync plan and Proton Drive CLI behavior.
* Keep external backups for important files.
* The daemon rejects absolute, parent-directory, and root-escaping remote paths before joining them to the local sync root.
* Local destination joins are guarded by the same relative-path validation.
* Selective sync filters are applied before planning, so excluded index records are not purged as missing files.
* The IPC socket is restricted to mode `0600` after binding.
* The advisory lockfile prevents two live daemon instances from using the same configuration at once.
* The daemon shells out to the configured `proton-drive` executable. Use a trusted executable path.

## Project Layout

```text
src/
  bin/
    proton-sync.rs   Control CLI entry point
    proton-syncd.rs  Daemon CLI entry point
  daemon.rs          Runtime loop, watcher, IPC handling, reconciliation execution
  index.rs           SQLite schema, local scanning, file hashing, index persistence
  ipc.rs             Unix socket request and response protocol
  lib.rs             Shared result and path validation helpers
  paths.rs           XDG-aware default state, socket, and lockfile paths
  proton.rs          Proton Drive CLI wrapper and remote JSON parser
  sync.rs            Sync planning matrix and conflict naming
examples/
  proton-sync.toml   Sample daemon config file
  systemd/           Sample systemd user service
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
cargo test --test ipc_cli
```

When changing sync behavior, add regression tests near the planner in `src/sync.rs`. When changing local filesystem or Proton JSON safety boundaries, add tests around the boundary that first accepts external paths.

## Troubleshooting

If `proton-sync` cannot connect, confirm the daemon is running and the client uses the same `--socket-path`. When no socket is configured, both binaries default to `$XDG_RUNTIME_DIR/proton-sync.sock` and fall back to the OS temporary directory.

If the daemon exits with a lockfile error, another live daemon may already hold the advisory lock. Stop the running daemon or use a different `--lockfile-path` for an isolated test.

If remote operations fail, run the equivalent `proton-drive` command directly to confirm authentication, permissions, and remote folder names.

If sync behavior looks unexpected, pause the daemon and inspect the local folder plus the SQLite index before resuming:

```bash
cargo run --bin proton-sync -- pause
sqlite3 /path/to/local/folder/sync_index.db 'select * from file_index order by file_path;'
```

## License

This project is licensed under the terms in [LICENSE](LICENSE).
