---
title: Running as a service
description: Install and run proton-syncd as a systemd user service, with the sample unit and install helper.
sidebar:
  order: 5
---

For day-to-day use on a systemd-based Linux workstation, run the daemon as a **systemd user
service** so it starts with your session and restarts on crash.

:::tip[One command]
From a source checkout, [`./setup.sh`](https://github.com/osirison/proton-drive-sync-engine/blob/main/setup.sh)
does everything on this page in one step — it builds and installs the binaries, writes the
config, installs and reloads the unit, previews a dry-run, and (after you confirm) enables and
starts the service. Add `--gui` to choose the folders in the desktop onboarding wizard instead
of on the command line. The sections below are the manual equivalent, and how to install from a
native package.
:::

## From a source install

Install the binaries and the sample assets:

```bash
cargo install --path .
install -Dm600 examples/proton-sync.toml ~/.config/proton-sync/proton-sync.toml
install -Dm644 examples/systemd/proton-syncd.service ~/.config/systemd/user/proton-syncd.service
```

Or use the helper script, which installs the config at mode `0600`, the unit at `0644`,
reloads the user manager, and won't clobber an existing config unless you pass
`--force-config` (add `--enable`/`--start` to enable or start it too):

```bash
examples/systemd/install-user-service.sh
```

Edit `~/.config/proton-sync/proton-sync.toml` for your roots **before** enabling the
service. The sample unit runs `proton-syncd --config %h/.config/proton-sync/proton-sync.toml`
and relies on the daemon's XDG defaults for the socket, lockfile, and index.

## From a native package

The RPM, deb, and AUR packages ship their own unit at
`/usr/lib/systemd/user/proton-syncd.service` (with
`ExecStart=/usr/bin/proton-syncd --config %h/.config/proton-sync/proton-sync.toml`). You
still create your config file first. The sample config's installed location varies by
package — Fedora ships it at `/usr/share/doc/proton-drive-sync-engine/proton-sync.toml`,
Arch at `/usr/share/doc/proton-drive-sync-engine/examples/proton-sync.toml`, and the Debian
package doesn't ship it (use `examples/proton-sync.toml` from the repo). Copy whichever
applies, then edit it:

```bash
install -Dm600 <sample-proton-sync.toml> ~/.config/proton-sync/proton-sync.toml
$EDITOR ~/.config/proton-sync/proton-sync.toml
```

See [Native packages](/distribution/packages/).

## Enable and inspect

```bash
systemctl --user daemon-reload
systemctl --user enable --now proton-syncd.service
systemctl --user status proton-syncd.service
journalctl --user -u proton-syncd.service -f
```

Control it as usual — the CLI finds the default runtime socket automatically:

```bash
proton-sync status
```

:::caution[Dry-run after changing roots]
Run a `--dry-run` from the shell whenever you change `--local-root`, `--remote-root`, or
`--proton-cli`, before letting the service sync unattended. See [Dry-run](/safety/dry-run/).
:::

## Prerequisites for a service

- `proton-drive` must be **on `PATH` and logged in** for the user the service runs as — the
  daemon shells it for remote file operations and reuses its keyring session for change
  detection.
- The desktop keyring must be unlocked for event-driven detection to reuse the CLI session;
  otherwise the daemon degrades to snapshot scans.
- Only **one daemon per user** may run (a user-global lock enforces this) because the shared
  `proton-drive` CLI cache isn't concurrency-safe.

## Building a release archive

To bundle the binaries plus the service assets into a distributable archive:

```bash
examples/packaging/build-release-archive.sh
```

By default it writes under `target/dist`; use `--archive-path <PATH>` to choose another
location. The manifest at `examples/packaging/release-assets.toml` lists what a user-service
distribution is expected to contain.
