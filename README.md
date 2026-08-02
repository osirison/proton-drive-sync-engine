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
Each index change is committed only **after** its side effect succeeds, in incremental
checkpoints as work completes, so a failure mid-plan keeps completed work durable and never
leaves half-recorded state. → [How sync works](https://osirison.github.io/proton-drive-sync-engine/concepts/how-sync-works/)

## Quick setup

One command installs the engine, writes the config a background service reads, and starts
syncing. You need a **Rust toolchain** (edition 2024 / Rust ≥ 1.85) and an **authenticated
`proton-drive` CLI** on your `PATH` first — the engine drives it for every remote operation:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder   # confirm the CLI works & is logged in
git clone https://github.com/osirison/proton-drive-sync-engine.git
cd proton-drive-sync-engine
```

**Terminal — pass the folders on the command line:**

```bash
./setup.sh --local-root ~/ProtonDrive --remote-root /Drive/RemoteFolder
```

This builds and installs `proton-syncd` + `proton-sync`, writes
`~/.config/proton-sync/proton-sync.toml`, installs a **systemd user service** that runs
`proton-syncd --config <that file>`, previews a dry-run (showing the `destructive_actions`
count as a safety check), and — once you confirm — enables and starts the service.

**Desktop — pick the folders in a wizard, no terminal after launch:**

```bash
./setup.sh --gui
```

This also builds the desktop app and opens it straight into the **onboarding wizard**: check
the CLI, choose the folder pair, review the first dry-run, and press Start. The wizard writes
the *same* config the service reads, so the two never disagree.

Once it's running, drive it however you like:

```bash
proton-sync status                        # what is the daemon doing?
systemctl --user status proton-syncd      # the service itself
```

`./setup.sh --help` lists every flag (`--no-start`, `--force-config`, `--proton-cli`,
`--no-build`, …). The next section explains what the script does, step by step.

## Upgrade and uninstall

Two companion scripts sit beside `setup.sh` and reuse its installer logic, so the systemd unit
and desktop launcher they write never drift from what a fresh install would produce.

**Upgrade an existing install** — rebuild the binaries from this checkout, refresh the unit and
(if installed) the desktop launcher, then restart the running service. Your config, folder
pairing, and sync state are left untouched:

```bash
git pull                 # or: ./upgrade.sh --pull  (fast-forwards for you)
./upgrade.sh             # rebuilds whatever is installed, then restarts the service
```

It auto-detects whether the desktop app is installed and upgrades it too; scope it with
`--engine-only` / `--gui`, skip the rebuild with `--no-build`, or leave the service stopped with
`--no-restart`. It refuses to run on a machine with nothing installed (use `setup.sh` for a first
install).

**Uninstall** — remove the service, binaries, config, desktop app files, and all engine state,
leaving the machine clean. Preview first; it prompts once before deleting:

```bash
./uninstall.sh --dry-run    # print exactly what would be removed, change nothing
./uninstall.sh              # remove everything (or -y to skip the prompt)
```

It **never** touches the files you were syncing (only the `.sync` state dir inside your local
root), nor the separate `proton-drive` CLI and its login. Pass `--keep-config` to preserve the
config and folder pairing for a later reinstall. `./uninstall.sh --help` lists every flag.

## Manual setup — what the script does

Prefer to run each step yourself, or skip systemd and just run the daemon in the foreground?
`setup.sh` automates essentially these steps (it also resolves the exact binary path for the
unit's `ExecStart`, previews a dry-run as a safety gate, and — with `--gui` — hands folder
selection to the desktop onboarding wizard):

**1. Confirm the `proton-drive` CLI works** — the daemon fails the same way if it doesn't:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

**2. Build and install the binaries** to `$CARGO_HOME/bin` (defaults to `~/.cargo/bin`):

```bash
cargo install --path .            # installs proton-syncd + proton-sync
```

(Or `cargo build --release --bins` and add `target/release` to your `PATH` for a throwaway run.)

**3. Preview the plan** — this touches nothing; it prints exactly what would happen. Replace
the paths with yours; the local folder must exist first (`mkdir -p ~/ProtonDrive`), the remote
folder is created on the first real sync. Check `destructive_actions` before going further:

```bash
proton-syncd --local-root ~/ProtonDrive --remote-root /Drive/RemoteFolder --dry-run
```

**4a. Run it as a systemd user service** (this is what the script automates) — write a config,
install the unit, and enable it. The heredoc is **unquoted** so `$HOME` expands to a real
absolute path (systemd/TOML don't expand `~`):

```bash
mkdir -p ~/.config/proton-sync ~/ProtonDrive
cat > ~/.config/proton-sync/proton-sync.toml <<TOML
local_root  = "$HOME/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
TOML
# The sample unit's ExecStart is %h/.cargo/bin/proton-syncd — correct if step 2 installed to the
# default ~/.cargo/bin; edit it if your CARGO_HOME/CARGO_INSTALL_ROOT differs.
install -Dm644 examples/systemd/proton-syncd.service \
  ~/.config/systemd/user/proton-syncd.service
systemctl --user daemon-reload
systemctl --user enable --now proton-syncd
```

**4b. …or just run the daemon in the foreground** (no systemd):

```bash
proton-syncd --local-root ~/ProtonDrive --remote-root /Drive/RemoteFolder
```

**5. Drive it from another terminal:**

```bash
proton-sync status      # is it running? what did it just do?
proton-sync syncnow     # reconcile now
proton-sync pause       # / resume
proton-sync history     # recent sync summaries
```

State (the SQLite index and its sidecars) lives in `<local-root>/.sync/`, which is always
ignored by sync. The control socket is at `$XDG_RUNTIME_DIR/proton-sync.sock`.

→ [Quick start guide](https://osirison.github.io/proton-drive-sync-engine/start/quick-start/) ·
[Running as a service](https://osirison.github.io/proton-drive-sync-engine/daemon/running-as-a-service/) ·
[Daemon reference](https://osirison.github.io/proton-drive-sync-engine/daemon/reference/) ·
[CLI reference](https://osirison.github.io/proton-drive-sync-engine/cli/reference/)

## The desktop app

Prefer not to touch a terminal? The quickest path is `./setup.sh --gui` (see
[Quick setup](#quick-setup)), which builds the app, installs a launcher, and opens the
onboarding wizard for you. To build and run it by hand from a checkout instead:

**1. Install system dependencies** (Linux/Fedora; adjust for your distro):

```bash
sudo dnf install webkit2gtk4.1-devel libsoup3-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel
```

**2. Start the daemon** (from the project root):

```bash
cargo run --bin proton-syncd -- --local-root /tmp/demo --remote-root /Drive/RemoteFolder
```

**3. In another terminal, run the GUI**:

```bash
cargo run -p proton-sync-gui
```

The desktop app is a **thin client** over the same daemon — it owns no sync logic and reads the same
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
  detection (on by default), with automatic full-scan fallback when the stream can't resolve a
  change; a periodic full scan is an opt-in safety net (off by default).
  → [Change detection](https://osirison.github.io/proton-drive-sync-engine/concepts/change-detection/)
- **Config files** with `--config`, plus hierarchical per-directory `.proton-sync.toml`.
  → [Configuration](https://osirison.github.io/proton-drive-sync-engine/daemon/configuration/)
- **Run as a systemd user service** — set up in one command by [`setup.sh`](#quick-setup), or
  by hand with the sample units and install helper.
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
setup.sh        One-command installer (terminal quick setup, or --gui onboarding)
upgrade.sh      In-place upgrade of an existing install (rebuild + refresh unit/launcher + restart)
uninstall.sh    Remove everything cleanly (keeps your synced files and the proton-drive CLI)
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
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

`--workspace` is required on `clippy`/`test`: the workspace root is itself a package, so
Cargo's default member selection is just that root crate and the `gui/` members would go
unlinted and untested. (`cargo fmt --all` already covers every member.)

The planner (`src/sync.rs`) is pure and tested directly; the daemon is generic over its
Proton client so reconciliation is tested with injected fakes. Add regression tests next to
the logic you change. → [Development guide](https://osirison.github.io/proton-drive-sync-engine/reference/development/)

## License

Licensed under the terms in [LICENSE](LICENSE) (Apache-2.0).
