<div align="center">

<img src="assets/icon.svg" alt="Proton Drive Sync" width="96" height="96">

# Proton Drive Sync

**Two-way file sync between a local folder and Proton Drive — for Linux.**

A fast Rust daemon, a scriptable control CLI, and a desktop app that shows you
every plan before it runs.

📖 **[Read the full documentation →](https://osirison.github.io/proton-drive-sync-engine/)**

</div>

---

> [!CAUTION]
> **This is an early prototype — test with disposable folders and backups first.**
> Sync is bidirectional and deletions propagate: removing a synced file on one side
> removes it from the other. A file or folder deleted **on Proton Drive** is deleted from
> your local disk **permanently** — not moved to your OS trash — and folder deletions
> remove the whole subtree. Always preview with `--dry-run` and check `destructive_actions`
> before running unattended. (The very first sync of an existing remote is a
> non-destructive merge — nothing is deleted.) See **[Safety](#safety-read-this)** below.

## What it is

Proton Drive Sync keeps a local folder and a Proton Drive folder in sync, both ways. It
ships as three pieces backed by one Rust engine:

| Piece | Binary | What it is |
| --- | --- | --- |
| **Daemon** | `proton-syncd` | A background service that watches a local folder and reconciles it with Proton Drive. Also hosts the one-shot `--dry-run` planner. |
| **Control CLI** | `proton-sync` | A thin client over a Unix socket — `status`, `pause`, `resume`, `syncnow`, `history`, and delete approvals. |
| **Desktop app** | `proton-sync-gui` | A [Tauri](https://tauri.app/) app: live status, activity, conflict resolution, dry-run review, settings, onboarding, and a tray icon. |

It uses Proton's own `proton-drive` CLI for every remote **file** operation (list, upload,
download, delete), and reads Proton's volume-event stream directly over HTTPS —
authenticated by **reusing the CLI's login session** rather than any credentials of its
own. Either way, `proton-drive` must be installed, authenticated, and on your `PATH` first.

### How it decides what to do

Every reconcile compares **three sources of truth** — the local files, the remote files,
and the last-synced baseline in a local SQLite index — then plans and executes actions
(upload, download, move, conflict, delete, …). Comparing all three, not just the two live
sides, is what lets it tell a *new* file from a *deleted* one and an *edit* from a *move*.
The index is committed only **after** every action in a pass succeeds, so a failure
mid-plan never leaves half-recorded state. → [How sync works](https://osirison.github.io/proton-drive-sync-engine/concepts/how-sync-works/)

## Quick start

Requires a Rust toolchain (edition 2024 / Rust ≥ 1.85) and an authenticated `proton-drive`
CLI on your `PATH`. Confirm the CLI works first:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

Build the binaries:

```bash
cargo build --release --bins      # → target/release/proton-syncd, proton-sync
```

This only writes the binaries to `target/release/` — it does not put them on your `PATH`.
Add that directory to `PATH` for this shell session (or run `cargo install --path .` once to
install both permanently to `$CARGO_HOME/bin` (defaults to `~/.cargo/bin`)):

```bash
export PATH="$PWD/target/release:$PATH"
```

The commands below are examples — replace `/tmp/demo` with the local folder you want to
sync and `/Drive/RemoteFolder` with the Proton Drive path you want to sync it to. The local
folder must exist before a dry run (create it if needed: `mkdir -p /tmp/demo`); the remote
folder does **not** need to exist yet — it's created for you on the first real sync.

**1. Preview the plan first** — this touches nothing; it prints exactly what would happen:

```bash
proton-syncd --local-root /tmp/demo --remote-root /Drive/RemoteFolder --dry-run
```

Check the `destructive_actions` count in the output before going further.

**2. Start the daemon:**

```bash
proton-syncd --local-root /tmp/demo --remote-root /Drive/RemoteFolder
```

**3. Drive it from another terminal:**

```bash
proton-sync status      # is it running? what did it just do?
proton-sync syncnow     # reconcile now
proton-sync pause       # / resume
proton-sync history     # recent sync summaries
```

State (the SQLite index and its sidecars) lives in `<local-root>/.sync/`, which is always
ignored by sync. The control socket is at `$XDG_RUNTIME_DIR/proton-sync.sock`.

→ [Quick start guide](https://osirison.github.io/proton-drive-sync-engine/start/quick-start/) ·
[Daemon reference](https://osirison.github.io/proton-drive-sync-engine/daemon/reference/) ·
[CLI reference](https://osirison.github.io/proton-drive-sync-engine/cli/reference/)

## The desktop app

Prefer not to touch a terminal? Build and run the desktop app:

```bash
cargo build --release -p proton-sync-gui
```

It's a **thin client** over the same daemon — it owns no sync logic and reads the same
socket and files as the CLI, so the two never disagree. It gives you:

- An **onboarding wizard** — check the CLI, choose the folder pair, review the first
  dry-run, and start the service.
- **Live status** with an activity ledger and per-state actions.
- **Side-by-side conflict resolution** — compare your version against Proton's and pick
  Keep mine / Use Proton's / Keep both, staged until you Apply.
- A **plan preview** that gates a destructive apply behind a typed `DELETE` confirmation.
- A **Deletions** queue for approving withheld deletes, and **Settings** that edit your
  config file safely.
- A **system-tray indicator** with five states; closing the window hides it to the tray
  and syncing keeps running.

→ [Desktop app docs](https://osirison.github.io/proton-drive-sync-engine/desktop/overview/) ·
[Native packages (RPM / deb / AUR)](https://osirison.github.io/proton-drive-sync-engine/distribution/packages/)

## Safety (read this)

Two-way sync means deletions and overwrites really happen. The engine is built to make
that safe and predictable.

- **Deletions propagate — asymmetrically.** Deleting a file locally moves the Proton Drive
  copy to Proton's **Trash** (recoverable). Deleting a file or folder **on Proton Drive**
  removes it from your local disk **permanently** — a direct filesystem delete, recursive
  for folders. There is no local-side undo.
- **The delete/edit safeguard.** A deletion only propagates if the *other* side hasn't
  changed since the last sync. If you delete a file locally but it was edited on Proton
  Drive, the edit is restored instead — the surviving edit always beats the delete.
- **Delete approval is on by default.** A directional guard withholds destructive deletes
  (per direction, per item) until you approve them via `proton-sync pending` / `approve`
  or the app's Deletions screen. Relax it per folder with a `.proton-sync.toml`, or
  globally with `--no-delete-approval`.
- **Conflicts keep both sides.** When both sides change differently, the engine keeps your
  local file and writes the remote version to a `.proton-cloud` sidecar — nothing is lost.
  Resolve by deleting the sidecar (keep yours) or moving it over the original (adopt
  Proton's).
- **Preview anything with `--dry-run`** and watch `destructive_actions`.

→ [Deletions & the safeguard](https://osirison.github.io/proton-drive-sync-engine/safety/deletions/) ·
[Delete approval](https://osirison.github.io/proton-drive-sync-engine/safety/delete-approval/) ·
[Conflicts](https://osirison.github.io/proton-drive-sync-engine/safety/conflicts/)

## More features

- **Selective sync** — include/exclude globs applied to local files, remote files, *and*
  the index, so excluded paths are never treated as deleted.
  → [Selective sync](https://osirison.github.io/proton-drive-sync-engine/daemon/selective-sync/)
- **Fast incremental reconcile** — Proton's volume-event stream gives `O(changes)` change
  detection (on by default), with a periodic full scan as a safety net.
  → [Change detection](https://osirison.github.io/proton-drive-sync-engine/concepts/change-detection/)
- **Config files** with `--config`, plus hierarchical per-directory `.proton-sync.toml`.
  → [Configuration](https://osirison.github.io/proton-drive-sync-engine/daemon/configuration/)
- **Run as a systemd user service**, with sample units and an install helper.
  → [Running as a service](https://osirison.github.io/proton-drive-sync-engine/daemon/running-as-a-service/)
- **Rename/move detection** for files in either direction and for remote-side directories;
  **structured `tracing` logs**; **file-manager emblems** for Nautilus and Nemo.

Unix-only today (control-plane IPC uses Unix domain sockets); Linux is the primary target.

## Documentation

Full docs — installation, concepts, safety, complete daemon/CLI/UI reference, packaging,
and troubleshooting — live at:

**➡️ https://osirison.github.io/proton-drive-sync-engine/**

The site is built from [`website/`](website/) with [Astro](https://astro.build/) +
[Starlight](https://starlight.astro.build/) and deploys to GitHub Pages on every push to
`main`. Architecture and design records are in [`docs/`](docs/) (see the
[ADRs](docs/adr/)).

## Project layout

```text
src/            The engine (library crate) + the two binaries
  bin/          proton-sync.rs (CLI) · proton-syncd.rs (daemon)
  sync.rs       Pure planner: the reconcile decision matrix + conflict naming
  daemon.rs     Runtime loop, watcher, IPC, reconcile execution + commit
  events.rs     Remote change detection via Proton's volume-event stream
  reconstruct.rs  base ⊕ delta remote-map reconstruction (event-driven)
  index.rs      SQLite schema, scanning, hashing, cursor, delete approvals
  proton.rs     proton-drive CLI wrapper + remote JSON parser
  config.rs     Layered config resolution · dirconfig.rs  per-directory settings
gui/            The desktop app (Tauri)
  gui-core/     Pure-Rust data layer (the typed boundary to the daemon)
  src/          Vanilla-JS webview frontend · src-tauri/  the Tauri shell
website/        This documentation site (Astro + Starlight)
packaging/      RPM · deb · AUR · freedesktop · file-manager emblems
examples/       Sample config, systemd unit, release-archive helper
docs/           Design docs + ADRs
```

## Development

Run the validation suite before opening a PR — CI enforces it:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The planner (`src/sync.rs`) is pure and tested directly; the daemon is generic over its
Proton client so reconciliation is tested with injected fakes. Add regression tests next to
the logic you change. → [Development guide](https://osirison.github.io/proton-drive-sync-engine/reference/development/)

## License

Licensed under the terms in [LICENSE](LICENSE) (Apache-2.0).
