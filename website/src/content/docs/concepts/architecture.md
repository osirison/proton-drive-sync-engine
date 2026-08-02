---
title: Architecture
description: The module map — how the daemon, planner, index, remote client, event stream, and desktop app fit together.
sidebar:
  order: 3
---

The codebase is one library crate (`proton_drive_sync_engine`) that backs two binaries,
plus a separate desktop-app workspace. The guiding principle is **pure core, impure
edges**: the planner and the event/reconstruction logic are pure functions with no I/O, so
they're tested directly; the daemon is the runtime that wires them to the filesystem, the
socket, and the `proton-drive` CLI.

## The engine crate

| Module | Responsibility |
| --- | --- |
| `sync.rs` | The **pure planner**. `plan_sync` turns the three maps into a `Vec<PlannedAction>` with no I/O. Owns bootstrap vs. ongoing logic, the `(local_delta, remote_delta)` decision matrix, and `.proton-cloud` conflict-sidecar naming. |
| `daemon.rs` | The **runtime**. Reconciles once on startup, then runs a `select!` loop over filesystem-watch events, IPC connections, a periodic scan, an events-poll interval, and shutdown signals. Dispatches between full-tree bootstrap and event-driven incremental reconcile; funnels every side effect and the incremental checkpoint commits through one path. Holds the delete-approval gate and an advisory lockfile. |
| `reconstruct.rs` | Pure `reconstruct_remote(base ⊕ delta)` — overlays a volume-event delta onto the last-known remote view so the planner gets a complete remote map without a full walk. Falls back to a full snapshot when anything is unresolvable. |
| `proton.rs` | The `ProtonClient` trait and the impl that shells out to `proton-drive`. Parses the tree-shaped remote JSON and extracts SHA-1 digests. Governs per-command timeouts and list-retry policy. |
| `events.rs` | **Remote change detection** via Proton's volume-event stream. Pure and transport-agnostic: normalizes the cleartext event delta and fetches it through injected HTTP-transport and session seams, refreshing once on `401`. Ships no networking dependency. |
| `session.rs` | The concrete seams for today's approach: reuse the logged-in `proton-drive` CLI's keyring session, and a dependency-free HTTP transport that shells `curl`. |
| `index.rs` | The **SQLite layer**: schema, local directory scanning, SHA-1 hashing, selective-sync `ScanOptions`, record CRUD, the per-volume event cursor, and the `delete_approvals` table. |
| `config.rs` | Layered config resolution — precedence is **explicit CLI flag > TOML file value > XDG default**. Validates roots and globs and resolves delete-approval defaults. |
| `dirconfig.rs` | **Hierarchical per-directory config** — a `.gitignore`-style layer where a `.proton-sync.toml` in any directory applies to it and everything beneath it, nearest wins. Machine-local and never synced. |
| `ipc.rs` | The JSON-line request/response protocol over the Unix control socket, bound at mode `0600`. |
| `paths.rs` | Default state paths — the index and per-root lockfile in `<local-root>/.sync/`, the control socket under `$XDG_RUNTIME_DIR`, and the user-global lock keyed on `$XDG_STATE_HOME` — plus the two-tier locking scheme. |
| `lib.rs` | Shared helpers: the `AppResult` alias, `boxed_error`, and the `validate_relative_path` path-safety guard. |

### Two-tier locking

Because every daemon shells the same `proton-drive` CLI — whose shared SQLite cache is not
concurrency-safe — the engine enforces **one daemon per user**. A per-root lockfile stops
two daemons on the same root; a user-global lock stops a second daemon anywhere for the
same user. On contention the daemon fails to start with a clear error naming the locked
lockfile, rather than failing silently.

## The two binaries

- **`proton-syncd`** (`src/bin/proton-syncd.rs`) — the daemon; also hosts the one-shot
  `--dry-run` planner path.
- **`proton-sync`** (`src/bin/proton-sync.rs`) — the thin control CLI over the Unix socket.

## The desktop app

The app is a separate Tauri workspace with two parts:

- **`gui-core`** — a **pure-Rust data layer** with no Tauri/webkit dependency, so it builds
  and unit-tests standalone. It is the single typed boundary between the UI and the daemon,
  reusing the engine's own wire types (a *facade rule*: screens import wire types from
  `gui-core`, never from the engine crate directly). It reads the control socket, the JSON
  state sidecars, the config file, the conflict sidecars, and the dry-run plan — and it
  derives the shared `DaemonState` that both the window and the tray agree on.
- **`src-tauri`** — the Tauri shell: a small, fixed set of `#[command]`s that wrap
  `gui-core`, plus a system-tray thread. The webview frontend is dependency-free vanilla JS.

The app owns **no sync logic and no index of its own**. See [Desktop app
overview](/desktop/overview/).

## State on disk

| What | Where (default) |
| --- | --- |
| SQLite index + status/metrics sidecars + per-root lockfile | `<local-root>/.sync/` (always ignored by sync) |
| Control socket | `$XDG_RUNTIME_DIR/proton-sync.sock` |
| User-global single-instance lock | keyed on `$XDG_STATE_HOME` |
| Daemon config (app convention) | `~/.config/proton-sync/proton-sync.toml` |
| Per-directory settings | `.proton-sync.toml` in any synced directory |

## Design records

The non-obvious decisions are written up as ADRs in the repo:

- [ADR 0001 — Remote change detection via volume events](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/adr/0001-remote-change-detection-via-volume-events.md)
- [ADR 0002 — Delete-approval guard and per-directory config](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/adr/0002-delete-approval-and-directory-config.md)
