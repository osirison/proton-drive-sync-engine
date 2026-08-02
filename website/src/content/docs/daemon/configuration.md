---
title: Configuration
description: The TOML config file, precedence rules, per-directory .proton-sync.toml overrides, and every available key.
sidebar:
  order: 2
---

Passing every path on the command line gets tedious, and service managers want settings out
of the unit file. Load a TOML config with `--config <PATH>`.

## The config file

```toml
local_root = "/home/me/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
scan_interval_secs = 300
proton_cli = "proton-drive"
proton_timeout_secs = 60
proton_list_attempts = 2

# How many planned downloads to bundle into one proton-drive invocation
# (chunked per destination folder, checkpoint-committed per chunk; 1 = one
# subprocess per file)
download_batch_size = 25

# Selective sync (paths relative to the roots)
include = ["Documents/**", "Projects/**/*.md"]
exclude = ["**/*.tmp", "**/.DS_Store"]

# Print a plan and exit instead of running
dry_run = false

# Delete-approval guard (daemon-wide default; both default to true)
[delete_approval]
remote = true
local = true
```

A sample lives at
[`examples/proton-sync.toml`](https://github.com/osirison/proton-drive-sync-engine/blob/main/examples/proton-sync.toml).

Every daemon flag has a matching config key. In addition to the keys above, you can set
`db_path`, `socket_path`, and `lockfile_path`. When omitted, `db_path` and `lockfile_path`
default to the per-root `<local_root>/.sync/` state directory, while only `socket_path`
defaults to `$XDG_RUNTIME_DIR` (with an OS temp-directory fallback) — see [state on
disk](/concepts/architecture/#state-on-disk). Keys also accept **hyphenated aliases** (e.g.
`local-root`), and **unknown keys are rejected** so typos fail fast.

Local filesystem paths (`local_root`, `db_path`, `socket_path`, `lockfile_path`,
`proton_cli`) expand a leading `~` to your home directory, so `local_root =
"~/ProtonDrive"` works the way it would in a shell. The `~user` form is rejected with an
error. `remote_root` is a Drive-side path, so `~` has no meaning there and is not
expanded.

:::note[Config-path convention]
The daemon itself has no default config path — you always pass `--config`. The **desktop
app** and the packaged systemd unit adopt the convention
`~/.config/proton-sync/proton-sync.toml`, and the app reads and writes exactly that file.
:::

## Precedence

Settings resolve in this order, highest first:

1. **An explicit CLI flag** (e.g. `--local-root`).
2. **A value in the `--config` TOML file.**
3. **The XDG default.**

So you can keep normal settings in `proton-sync.toml` and run a one-off filtered preview
with `--config proton-sync.toml --include 'Documents/**' --dry-run`. Use `--no-dry-run`
when the file sets `dry_run = true` but you want to start normally.

Two things to remember:

- `--include`/`--exclude` on the CLI **replace** the corresponding config list entirely
  (each independently) — they don't add to it.
- Passing only `--include` leaves the file's `exclude` in effect, and vice versa.

## Per-directory settings (`.proton-sync.toml`)

Beyond the daemon-wide config, you can drop a `.proton-sync.toml` in **any directory under
the local root** to override settings for that directory and everything beneath it. It's a
`.gitignore`-style layer: the **nearest** file wins, unset options inherit from the parent,
and the daemon-wide config is the bottom of the chain.

Today this layer carries the [delete-approval](/safety/delete-approval/) toggles:

```toml
# <local_root>/Downloads/.proton-sync.toml
[delete_approval]
remote = false
local = false
```

These files are **machine-local**: the engine ignores `.proton-sync.toml` at any depth, so
they are never synced to Proton Drive. A malformed one is ignored and contributes no
override, so the guard is left in whatever state it **inherits** — on by default, but *off*
if an ancestor file or the daemon-wide default (`--no-delete-approval`) disabled it. So if you
use a nested `.proton-sync.toml` to *re-enable* approval for a subtree, a typo there silently
falls back to the inherited (disabled) state — keep it valid. Edits take effect on the next
reconcile.

The config layer is built to extend — future per-directory settings slot into the same
inheritance model. See
[ADR 0002](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/adr/0002-delete-approval-and-directory-config.md).

## Validation

The daemon validates the resolved configuration before starting and rejects empty roots,
invalid globs, a zero Proton timeout, and zero list attempts. `scan_interval_secs` is
clamped to a 1-second minimum. See the [reference](/daemon/reference/#validation).
