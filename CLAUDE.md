# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Rust prototype for bidirectional file sync between a local folder and Proton Drive. Ships two binaries backed by one library crate (`proton_drive_sync_engine`):

- `proton-syncd` (`src/bin/proton-syncd.rs`) — long-running daemon; also hosts the one-shot `--dry-run` path.
- `proton-sync` (`src/bin/proton-sync.rs`) — thin control CLI that talks to the daemon over a Unix socket.

Unix-only today: control-plane IPC uses Unix domain sockets (`#[cfg(unix)]` guards in `src/ipc.rs`). Edition 2024 toolchain required.

## Commands

```bash
cargo build --workspace --all-targets

# Rust validation suite (run before committing) — mirrors the `rust` job in .github/workflows/ci.yml
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features

# Frontend validation suite (the gui/src webview) — mirrors the `frontend` job in the same workflow
(cd gui && npm ci)         # first time, and after any gui/package-lock.json change
(cd gui && npm run check)  # = prettier --check, then eslint

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

`--workspace` matters: this workspace's root is itself a package, so Cargo's default member
selection is *just* the root crate. Without the flag, `gui/gui-core` and `gui/src-tauri` are
neither linted nor tested. (`cargo fmt --all` already spans every member.)

The webview frontend in `gui/src/js` is gated separately by `gui/package.json` (eslint + prettier,
both dev-only — nothing there ships). There is **no bundler**: Tauri serves `gui/src` raw
(`frontendDist: "../src"`), so `import-x/extensions: always` is load-bearing — an import specifier
that loses its `.js` still resolves under Node's resolver and 404s in WebKitGTK, which is a blank
window at runtime rather than a build error. `eslint.config.js` documents the rest of the ruleset.

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
- `src/daemon.rs` — the runtime. `run()` reconciles once on startup, then runs a `tokio::select!` loop over filesystem-watch events (`notify`), control commands forwarded from the IPC task (`syncnow`/`shutdown`), a periodic scan interval, a faster events-poll interval, and shutdown signals. **The control socket is served on its own task** (`serve_control_socket`, one further task per connection) from a continuously-published `ControlShared` snapshot (+`paused`/`syncing` atomics and a `reconcile_seq` pass counter), so status/pause/approve answer instantly even while a reconcile blocks the main task; the IPC task holds a second SQLite connection for approval writes (both connections set a busy timeout). `Syncnow` is an immediate ack — the sync runs on the main loop, and clients (e.g. `proton-sync syncnow`) poll status until `reconcile_seq` advances past the ack's value with `syncing == false`. `Shutdown` (IPC) exits through the same path as SIGTERM, including cancelling an in-flight proton-drive command. `reconcile_blocking_inner` dispatches between `bootstrap_reconcile` (full-tree snapshot) and `try_incremental_reconcile` (event-driven, O(changes)); both funnel side effects + incremental checkpoint commits through `execute_plan_and_commit` (consecutive planned downloads execute as chunked multi-file `download_many` invocations, grouped by destination directory). The event cursor advances **only** in the final commit of a fully-successful pass, and only when `events_driven` is on. `events_driven` is now **on by default** (opt out with `--no-events-driven` / `events_driven = false`, which restores the byte-identical snapshot-only path); when the reused CLI session/keyring is unavailable the daemon degrades to snapshots at runtime anyway. Holds an advisory lockfile (`LockGuard`) to prevent duplicate instances.
- `src/reconstruct.rs` — **pure** `reconstruct_remote(base ⊕ delta)`: overlays a volume-event delta onto the last-known remote view (the baseline `file_index`) to hand the planner a *complete* remote map without a full walk. Removals (delete / `Updated`+`trashed`) resolve from the baseline; created/updated nodes resolve via an injected `RemoteChangeResolver` (a targeted parent list); anything unresolvable returns `Reconstruction::FallbackToSnapshot` so the daemon re-bootstraps rather than plan against an incomplete map. Selective-sync filters apply to the delta.
- `src/proton.rs` — `ProtonClient` trait + `ProtonDriveClient` impl that shells out to the `proton-drive` executable. Parses the tree-shaped remote JSON (nodes may nest under `children`/`entries`/`files`). Extracts SHA-1 from `activeRevision.claimedDigests.sha1` when present. `CommandPolicy` governs per-command timeout and list retry attempts. `download_many` fetches a batch of files in **one** CLI invocation (multiple `path...` args, one shared staging dir, per-file results; the per-invocation timeout scales with batch size, and a failed batch salvages fully-staged files by verifying their claimed SHA-1) — the executor chunks consecutive planned downloads to `download_batch_size` (default 25; `1` restores one-subprocess-per-file). `list_directory` is the non-recursive, single-directory list used for targeted event resolution (the O(1) sibling of the O(folders) BFS).
- `src/events.rs` — **remote change detection** via Proton's volume event stream (the O(changes) alternative to the O(folders) `proton.rs` full walk). Pure, transport-agnostic: `parse_volume_events` normalizes the cleartext event delta (`RemoteChange`: created/updated/deleted + `trashed` flag — note a *trash* is `Updated`+`trashed`, not `Deleted`), and `EventsClient<T: HttpTransport, S: SessionProvider>` fetches it, refreshing once on `401`. Detection needs only a session (no decryption); the concrete HTTP transport and session provider (an independent forked session) are injected, so the crate ships no networking dependency. See `docs/adr/0001-*`.
- `src/index.rs` — SQLite schema (`file_index` table + `remote_event_cursor` + `delete_approvals`), local directory scanning, SHA-1 hashing, `ScanOptions` (include/exclude globs + ignore rules), and record CRUD. For event-driven reconcile it also persists the per-scope events cursor (`EventCursor` load/store/clear) and resolves a remote node id back to a local path (`path_for_proton_id`, keyed by the stored `proton_id` = composed `volumeId~nodeId`; bridge an event's raw `LinkID` with `events::node_uid`). The `delete_approvals` table stores standing per-item delete approvals keyed by `(path, direction)` and pinned to a `fingerprint`. `should_ignore_path` also skips `.proton-sync.toml` at any depth.
- `src/session.rs` — concrete impls of the `events` seams for the **current CLI-session-reuse** approach: `CliKeyringSession` (a `SessionProvider` that reuses the logged-in `proton-drive` CLI's session from the OS keyring via `secret-tool`; `refresh` re-reads the keyring rather than owning a token refresh) and `CurlHttpTransport` (a dependency-free `HttpTransport` shelling `curl`). Independent (browser-forked) auth is deferred by decision — see `docs/adr/0001-*`.
- `src/config.rs` — layered config resolution. Precedence: **explicit CLI flag > TOML file value > XDG default**. `resolve_runtime_config` merges `DaemonConfigInput` (from clap) with `FileConfig` (from `--config`), validates roots/globs, and resolves `dry_run` (`--no-dry-run` beats `--dry-run` beats file `dry_run`). Every local-filesystem path option expands a leading `~` to `$HOME` (`expand_tilde`; `~user` is rejected) — the daemon runs shell-less, and a literal `~` root diverges from the `proton-drive` CLI, which does expand it. Also resolves the daemon-wide **delete-approval** defaults (`[delete_approval] remote/local`, both default `true`; `--no-delete-approval` forces both off) that seed `src/dirconfig.rs`.
- `src/dirconfig.rs` — **hierarchical, per-directory config** (the `.gitignore`-style layer). A `.proton-sync.toml` in any directory applies to it and everything beneath it; `DirectoryConfigResolver::resolve` walks the entity's ancestor directories root-first over the daemon-wide default (nearest wins; unset options inherit). Machine-local: ignored by scan/plan/watch (`index::should_ignore_path`) and never synced. Malformed/unreadable files fail safe (guard stays on). All-`Option` `DirectorySettings` → resolved `EffectiveSettings`; extend by adding an `Option` field + resolved counterpart. See `docs/adr/0002-*`.
- `src/ipc.rs` — JSON-line request/response protocol over the Unix socket; binds with mode `0600`. `ControlResponse` carries `syncing` + `reconcile_seq` + `activity` (all `#[serde(default)]` for wire compat); `ControlCommand::Shutdown` asks the daemon to exit gracefully (used by the GUI's restart-daemon flow and `proton-sync stop`). `SyncActivity` is the live "what is the daemon doing right now" surface (phase + current path + action i/N + in-flight transfer): the daemon core updates it from inside the blocking reconcile (phases, per-action), the concrete client feeds it via `proton::ProgressSink` (per-folder walk progress, download staging dir), and `ControlShared` samples a download's bytes-so-far from the staging directory at status-reply time. Display-only — nothing in it participates in sync decisions. The `proton-sync` CLI prints human-readable, git-style output by default — `--json` restores the raw response for scripts and tests.
- `src/paths.rs` — default state paths. The SQLite index (plus its status/metrics sidecars) and the per-root instance lockfile default to the per-root `<local_root>/.sync` state directory, which the engine ignores everywhere (`index::should_ignore_path` → `is_sync_state_path`; only a *top-level* `.sync` is treated as state). The control socket stays in `$XDG_RUNTIME_DIR` — the XDG home for sockets — since it is a session-scoped runtime endpoint (not persistent state), must stay short for the `sun_path` limit, and must be locatable by the control CLI without knowing the sync root. **Two-tier locking:** the per-root lockfile stops two daemons on the *same* root, while a **user-global** single-instance lock (`default_global_lock_path`, keyed on `$XDG_STATE_HOME`, *not* the per-session runtime dir) stops a second daemon anywhere for this user — required because every daemon shells the same `proton-drive` CLI, whose shared SQLite cache/session store is not concurrency-safe (`SQLITE_BUSY`; #23).
- `src/lib.rs` — `AppResult<T>` alias, `boxed_error`, and `validate_relative_path` (the shared path-safety guard).

### Invariants to preserve

- **Commit-after-side-effects, checkpointed.** An index write never precedes its side effect. Mutations commit in per-action **checkpoint transactions** (`commit_checkpoint` in `src/daemon.rs`): every side-effecting action — and every batched download chunk — commits its own mutations immediately after succeeding, while index-only mutations (`AutoLink`/`Purge`) accumulate into the next checkpoint or the final commit (so an adoption-heavy pass isn't thousands of fsyncs). A mid-plan failure keeps completed work durable; the failed action itself is never recorded and replans next pass along with its unexecuted successors. The **event cursor** advances only in the final commit of a fully-successful pass — never in a checkpoint. Approval consumptions commit in the same checkpoint as their delete's purge. Do not move index writes ahead of their side effect. (See `docs/adr/0003-*`.)
- **Path-safety at boundaries.** Every relative path from an external source (remote listing, local scan, conflict target) must pass `validate_relative_path` / `safe_local_path` before being joined onto a root. Absolute, `..`, and prefix components are rejected to prevent root escape. When adding a boundary that accepts external paths, guard it and add a test there.
- **Non-destructive on unknown digests.** When a remote file exists but exposes no usable SHA-1, `remote_file_delta` returns `FileDelta::Unknown` and the planner avoids destructive actions (see the `Unknown` match arms in `plan_ongoing_action`). Preserve this conservatism.
- **Selective-sync filters apply everywhere.** Include/exclude globs (`ScanOptions`) filter local files, remote files, *and* base-index records before planning, so excluded records are never purged as "missing." Conflict sidecars and the index DB itself are always ignored.
- **Startup snapshots first (event-driven).** The first reconcile after process startup always full-scans, never an incremental pass — `incremental_passes_since_full_scan` is seeded at `effective_full_scan_every(events_full_scan_every)` in the constructor. A fresh process has an empty `pending_changes` (`notify` does not replay pre-existing files), so an incremental pass would idle-skip a file edited while the daemon was down. Do not reset that counter to 0 at construction. The same floor is re-applied by `reacquire_event_source_if_needed` (see below): when a keyring that was locked at boot becomes readable mid-life, installing the event source also reseeds the counter, so the reacquisition pass full-scans (capturing a fresh cursor) rather than going incremental against a stale persisted cursor — making mid-life reacquisition byte-identical to a restart. (Guards: `first_reconcile_after_startup_full_scans_even_with_a_persisted_cursor`, `event_source_is_reacquired_when_the_keyring_becomes_readable`.)
- **Periodic safety resync is opt-in (off by default).** `events_full_scan_every` **defaults to `0`, which disables the periodic full-tree resync entirely**: after the mandatory startup snapshot the daemon stays purely event-driven until it is restarted or the event stream forces a fallback (no cursor / no volume / fetch error / server refresh / unresolvable node). `effective_full_scan_every` maps the `0` sentinel to `u64::MAX` so the startup floor still fires exactly once but the pass counter never reaches it again. Set a positive `N` (flag `--events-full-scan-every N` or file `events_full_scan_every = N`) to reinstate a self-healing full walk every `N` incremental passes. (Guard: `disabled_periodic_resync_never_full_scans_after_the_startup_snapshot`.)
- **Delete-approval guard is execution-time, and holds the cursor when withholding.** The gate lives in `execute_plan_and_commit` (`decide_delete_gate`), **not** the pure planner: it withholds `RemoteDelete`/`LocalDelete` (never `Purge`) whose direction is guarded at that path (per `dirconfig`) unless a matching approval exists. A withheld action skips **both** its side effect and its index mutation (re-plans next pass). If any action is withheld this pass, `cursor_update` is forced to `None` so a withheld `LocalDelete` keeps re-deriving from ground truth (else `reconstruct_remote` would drop it once the cursor passed its event). Approvals are consumed in the same checkpoint transaction as their delete's index purge — an executed delete's approval is consumed even when a later action fails the pass. (Guards: the `guard_*`/`a_withheld_local_delete_holds_the_event_cursor` daemon tests.)

### Testing pattern

`Daemon<C: ProtonClient>` is generic over the client. Tests inject fake `ProtonClient` implementations (see the fakes in `src/daemon.rs` tests — including one that fails uploads to exercise the checkpoint-commit invariant: completed actions stay committed, the failed action is never recorded — see the `reconcile_checkpoints_*` tests) so reconciliation is tested without a real Proton Drive. The planner in `src/sync.rs` is pure and tested directly with in-memory maps. Prefer adding regression tests next to the logic you change (planner tests in `sync.rs`, boundary tests where external paths first enter).

## Conventions

- Errors use `AppResult<T>` = `Result<T, Box<dyn Error + Send + Sync>>`; construct ad-hoc errors with `boxed_error(...)`.
- Structured logging via `tracing` to **stderr** (stdout is reserved for machine-readable dry-run JSON). Control verbosity with `RUST_LOG`; default is `info`.
- Uploads/downloads/deletes are never auto-retried (avoids duplicate side effects); only read-only remote listings retry per `proton_list_attempts`.
