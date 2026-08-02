---
title: Installation
description: Requirements, the proton-drive CLI prerequisite, building from source, and installing native packages.
sidebar:
  order: 2
---

## Requirements

- A **Linux** desktop (Fedora, Ubuntu, Arch). The engine is Unix-only — the control plane
  uses Unix domain sockets — but Linux is the only supported and tested platform: keyring
  session reuse (`secret-tool`/libsecret), the desktop app, and the native packaging are all
  Linux-specific.
- A **Rust toolchain with edition 2024 support** (Rust 1.85 or newer) to build from source.
- The **`proton-drive` CLI**, installed, authenticated, and on your `PATH` (see below).
- For the desktop app: the usual **Tauri Linux build deps** — `webkit2gtk-4.1`,
  `libsoup-3.0`, `gtk3`, and `glib2` development packages, plus `libappindicator`
  for the system tray.

## Prerequisite: the proton-drive CLI

The engine relies on Proton's own `proton-drive` CLI: every remote **file** operation
(list, upload, download, delete) shells out to it, and change detection reads Proton's
volume-event stream directly over HTTPS, authenticated by **reusing that tool's keyring
session**. **Install and authenticate it first**, following its own documentation, then
confirm it works:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

If that command fails, the daemon will fail the same way. If `proton-drive` is not on
your `PATH`, point the daemon at it explicitly with `--proton-cli /path/to/proton-drive`.

:::note
`proton-drive` is not available in Linux distribution repositories, so it can't be a
package dependency. Install it separately and keep it logged in — it is, for now, the
owner of your Proton session. If the CLI's session expires, the daemon falls back to
snapshot scans until the CLI refreshes.
:::

## Build from source

Clone the repository and build both engine binaries:

```bash
git clone https://github.com/osirison/proton-drive-sync-engine.git
cd proton-drive-sync-engine
cargo build --release --bins
```

This produces `target/release/proton-syncd` and `target/release/proton-sync`. To install
them into your Cargo bin directory:

```bash
cargo install --path .
```

### Build the desktop app

The desktop app is a Tauri workspace member. With the Tauri Linux build deps installed:

```bash
cargo build --release -p proton-sync-gui
```

The resulting `proton-sync-gui` binary is the desktop client. See the
[Desktop app overview](/desktop/overview/) for what it does.

### Validate a source checkout

The Rust validation suite:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

`clippy` runs with `-D warnings`, so warnings fail the build in CI.

The desktop app's webview frontend is gated separately and needs Node 20 or newer (CI uses
Node 22):

```bash
cd gui
npm ci        # first time, and after any gui/package-lock.json change
npm run check # prettier --check, then eslint
```

Both suites are enforced by CI. See [Development](/reference/development/) for focused test
commands and the frontend tooling notes.

## Native packages

Packaging trees live under [`packaging/`](https://github.com/osirison/proton-drive-sync-engine/tree/main/packaging)
for RPM (Fedora/COPR), deb (Debian/Ubuntu), and an Arch `PKGBUILD` (AUR). Each installs
the three binaries, a systemd **user** unit at `/usr/lib/systemd/user/proton-syncd.service`,
the freedesktop launcher/icon/AppStream metadata, and the Nautilus/Nemo file-manager emblem
extensions (split into optional subpackages on RPM and deb; bundled in the main package on
Arch, with the Python bindings as optional dependencies).

See [Native packages](/distribution/packages/) for the per-distro build and install steps.

:::note
The native packages deliberately do **not** declare `proton-drive` as a dependency (it
isn't in any distro repo). Install and log into it separately after installing the package.
:::

## Next

- [Quick start](/start/quick-start/) — preview a plan and run your first sync.
- [Running as a service](/daemon/running-as-a-service/) — install the systemd user unit.
