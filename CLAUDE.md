# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Rust prototype for bidirectional file sync between a local folder and Proton Drive. Ships two binaries backed by one library crate (`proton_drive_sync_engine`):

- `proton-syncd` (`src/bin/proton-syncd.rs`) — long-running daemon; also hosts the one-shot `--dry-run` path.
- `proton-sync` (`src/bin/proton-sync.rs`) — thin control CLI that talks to the daemon over a Unix socket.

Unix-only today: control-plane IPC uses Unix domain sockets (`#[cfg(unix)]` guards in `src/ipc.rs`). Edition 2024 toolchain required.

## Commands

```bash
cargo build --all-targets

# Full validation suite (run before committing)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features

# Focused tests
cargo test sync::tests          # planner unit tests (src/sync.rs)
cargo test proton::tests        # remote JSON parsing / CLI wrapper
cargo test events::tests        # volume-events delta parsing + EventsClient 401/refresh logic
cargo test reconstruct::tests   # base ⊕ delta remote-map reconstruction (event-driven)
cargo test daemon::tests        # reconciliation with injected fake ProtonClient + fake EventSource
cargo test --test dry_run_cli   # integration test in tests/
cargo test --test ipc_cli
cargo test --test example_assets
cargo test computes_empty_file_sha1   # single test by name substring
```

`clippy` runs with `-D warnings` — warnings are build failures in CI. Keep `cargo fmt` clean.

The live smoke test against a real authenticated Proton Drive account is `#[ignore]` and read-only:

```bash
PROTON_SYNC_LIVE_REMOTE_ROOT=/Drive/RemoteFolder \
  cargo test --test proton_live --all-features -- --ignored
# Set PROTON_SYNC_LIVE_CLI=/path/to/proton-drive if not on PATH
```

There is also a `#[ignore]`, read-only live check for the volume-events **detection** path
(`tests/events_live.rs`) that drives the real `EventsClient` via a temporary reuse-session
harness (reads the logged-in CLI's session from the OS keyring, shells `curl` for HTTP):

```bash
PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
  cargo test --test events_live -- --ignored --nocapture
# volumeId is a key under "drive" in ~/.local/share/proton-drive-cli/events.json
# Needs the desktop keyring unlocked and DBUS_SESSION_BUS_ADDRESS set.
```

**Event-driven reconcile id-identity gate (`tests/events_identity_live.rs`).** A `#[ignore]` HARD
GATE that must pass on a real account **before enabling `events_driven`**: it proves a stored
`proton_id` (composed `volumeId~nodeId`) equals `events::node_uid(volume, LinkID)` for the same
node — the bridge the incremental resolver relies on. A read-only check verifies the composed-id
round trip and volume derivation; an opt-in write round trip (`PROTON_SYNC_LIVE_WRITE=1`) uploads a
probe, matches its live `Created` event, then deletes it.

```bash
PROTON_SYNC_EVENTS_VOLUME=<volumeId> PROTON_SYNC_LIVE_REMOTE_ROOT=/Drive/RemoteFolder \
  cargo test --test events_identity_live -- --ignored --nocapture
```

## Architecture

The engine reconciles **three sources of truth** into a plan, then executes it:

1. Local files under `local-root` (`index::scan_local_files_with_options`, SHA-1 hashed)
2. Remote files from `proton-drive filesystem list --json` (`proton::ProtonClient::list`)
3. The last-synced baseline in SQLite (`index::load_index`)

Data flow: **scan → `sync::plan_sync` → execute actions → commit index**. This is the spine; changes to sync behavior almost always start in `src/sync.rs`.

### Module responsibilities

- `src/sync.rs` — **pure** planner. `plan_sync` produces `Vec<PlannedAction>` with no I/O. Bootstrap (empty index) vs ongoing logic are separate; ongoing decisions come from a `(local_delta, remote_delta)` matrix in `plan_ongoing_action`. Also owns conflict-sidecar naming (`.proton-cloud` convention).
- `src/daemon.rs` — the runtime. `run()` is a `tokio::select!` loop over filesystem-watch events (`notify`), IPC connections, a periodic scan interval, an (opt-in) faster events-poll interval, and shutdown signals. `reconcile_blocking_inner` dispatches between `bootstrap_reconcile` (full-tree snapshot, the default) and `try_incremental_reconcile` (event-driven, O(changes)); both funnel side effects + the single post-success commit through `execute_plan_and_commit`. The event cursor advances **only** inside that commit, and only when `events_driven` is on (default off = byte-identical to the snapshot path). Holds an advisory lockfile (`LockGuard`) to prevent duplicate instances.
- `src/reconstruct.rs` — **pure** `reconstruct_remote(base ⊕ delta)`: overlays a volume-event delta onto the last-known remote view (the baseline `file_index`) to hand the planner a *complete* remote map without a full walk. Removals (delete / `Updated`+`trashed`) resolve from the baseline; created/updated nodes resolve via an injected `RemoteChangeResolver` (a targeted parent list); anything unresolvable returns `Reconstruction::FallbackToSnapshot` so the daemon re-bootstraps rather than plan against an incomplete map. Selective-sync filters apply to the delta.
- `src/proton.rs` — `ProtonClient` trait + `ProtonDriveClient` impl that shells out to the `proton-drive` executable. Parses the tree-shaped remote JSON (nodes may nest under `children`/`entries`/`files`). Extracts SHA-1 from `activeRevision.claimedDigests.sha1` when present. `CommandPolicy` governs per-command timeout and list retry attempts. `list_directory` is the non-recursive, single-directory list used for targeted event resolution (the O(1) sibling of the O(folders) BFS).
- `src/events.rs` — **remote change detection** via Proton's volume event stream (the O(changes) alternative to the O(folders) `proton.rs` full walk). Pure, transport-agnostic: `parse_volume_events` normalizes the cleartext event delta (`RemoteChange`: created/updated/deleted + `trashed` flag — note a *trash* is `Updated`+`trashed`, not `Deleted`), and `EventsClient<T: HttpTransport, S: SessionProvider>` fetches it, refreshing once on `401`. Detection needs only a session (no decryption); the concrete HTTP transport and session provider (an independent forked session) are injected, so the crate ships no networking dependency. See `docs/adr/0001-*`.
- `src/index.rs` — SQLite schema (`file_index` table + `remote_event_cursor`), local directory scanning, SHA-1 hashing, `ScanOptions` (include/exclude globs + ignore rules), and record CRUD. For event-driven reconcile it also persists the per-scope events cursor (`EventCursor` load/store/clear) and resolves a remote node id back to a local path (`path_for_proton_id`, keyed by the stored `proton_id` = composed `volumeId~nodeId`; bridge an event's raw `LinkID` with `events::node_uid`).
- `src/session.rs` — concrete impls of the `events` seams for the **current CLI-session-reuse** approach: `CliKeyringSession` (a `SessionProvider` that reuses the logged-in `proton-drive` CLI's session from the OS keyring via `secret-tool`; `refresh` re-reads the keyring rather than owning a token refresh) and `CurlHttpTransport` (a dependency-free `HttpTransport` shelling `curl`). Independent (browser-forked) auth is deferred by decision — see `docs/adr/0001-*`.
- `src/config.rs` — layered config resolution. Precedence: **explicit CLI flag > TOML file value > XDG default**. `resolve_runtime_config` merges `DaemonConfigInput` (from clap) with `FileConfig` (from `--config`), validates roots/globs, and resolves `dry_run` (`--no-dry-run` beats `--dry-run` beats file `dry_run`).
- `src/ipc.rs` — JSON-line request/response protocol over the Unix socket; binds with mode `0600`.
- `src/paths.rs` — XDG-aware defaults for state DB, socket, and lockfile.
- `src/lib.rs` — `AppResult<T>` alias, `boxed_error`, and `validate_relative_path` (the shared path-safety guard).

### Invariants to preserve

- **Commit-after-side-effects.** `reconcile_blocking_inner` collects `IndexMutation`s while performing uploads/downloads/deletes, then applies them in a single SQLite transaction *after* all actions succeed. If any action fails mid-plan, earlier successes are **not** recorded in the index (they replan next pass). Do not move index writes ahead of their side effect.
- **Path-safety at boundaries.** Every relative path from an external source (remote listing, local scan, conflict target) must pass `validate_relative_path` / `safe_local_path` before being joined onto a root. Absolute, `..`, and prefix components are rejected to prevent root escape. When adding a boundary that accepts external paths, guard it and add a test there.
- **Non-destructive on unknown digests.** When a remote file exists but exposes no usable SHA-1, `remote_file_delta` returns `FileDelta::Unknown` and the planner avoids destructive actions (see the `Unknown` match arms in `plan_ongoing_action`). Preserve this conservatism.
- **Selective-sync filters apply everywhere.** Include/exclude globs (`ScanOptions`) filter local files, remote files, *and* base-index records before planning, so excluded records are never purged as "missing." Conflict sidecars and the index DB itself are always ignored.

### Testing pattern

`Daemon<C: ProtonClient>` is generic over the client. Tests inject fake `ProtonClient` implementations (see the fakes in `src/daemon.rs` tests — including one that fails uploads to exercise the no-partial-commit invariant) so reconciliation is tested without a real Proton Drive. The planner in `src/sync.rs` is pure and tested directly with in-memory maps. Prefer adding regression tests next to the logic you change (planner tests in `sync.rs`, boundary tests where external paths first enter).

## Conventions

- Errors use `AppResult<T>` = `Result<T, Box<dyn Error + Send + Sync>>`; construct ad-hoc errors with `boxed_error(...)`.
- Structured logging via `tracing` to **stderr** (stdout is reserved for machine-readable dry-run JSON). Control verbosity with `RUST_LOG`; default is `info`.
- Uploads/downloads/deletes are never auto-retried (avoids duplicate side effects); only read-only remote listings retry per `proton_list_attempts`.
