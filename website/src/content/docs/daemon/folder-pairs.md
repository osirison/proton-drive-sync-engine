---
title: Multiple folder pairs
description: Syncing several local folders to several Proton Drive folders with one daemon — the [[pair]] tables, names, the default pair, and the rules that keep them apart.
sidebar:
  order: 6
---

One `proton-syncd` can sync several folder pairs — say `~/Documents ⇄ /Drive/Docs` and
`~/Pictures ⇄ /Drive/Photos` — each with its own index, its own settings and its own
deletion guard. A config file with no `[[pair]]` table is **one pair named `default`**, exactly as
before, and is never rewritten: nothing here changes a single-folder setup.

## Why one daemon, and what that means

The `proton-drive` CLI keeps one cache and one session store per user, and it is not safe to
run twice at once. So there is **one daemon, one CLI client, and the pairs take turns**: when two
are due at the same moment, one waits for the other's pass to finish. Anything you ask for by
hand (`proton-sync syncnow`) goes ahead of anything on a timer.

## Writing the config

Put the settings that describe the daemon at the top of the file, and one `[[pair]]` table per
folder:

```toml
# The daemon: one value for the whole process.
socket_path = "/run/user/1000/proton-sync.sock"
log_level = "info"
proton_cli = "/usr/bin/proton-drive"
proton_timeout_secs = 60
proton_list_attempts = 2

[[pair]]
name = "documents"             # required; the first table is the default pair
local_root = "~/Documents"     # required beside other pairs
remote_root = "/Drive/Docs"    # required beside other pairs
exclude = ["*.tmp"]
deletion_policy = "ask_every_time"
scan_interval_secs = 300

[[pair]]
name = "photos"
local_root = "~/Pictures"
remote_root = "/Drive/Photos"
events_driven = false
```

**Which keys go where.** `socket_path`, `log_level`, `proton_cli`, `proton_timeout_secs` and
`proton_list_attempts` describe the process and the one shared CLI client, so they live at the top
level and apply to every pair. Everything that describes a folder — the two roots, `db_path`,
`lockfile_path`, `include`/`exclude`, the deletion policy, `local_delete_mode`, `conflict_suffix`,
`scan_interval_secs`, `full_scan_schedule`, `events_driven` and the warm-start keys — belongs
inside the `[[pair]]` table it is about. A per-pair key at the top level of a file that *also*
has `[[pair]]` tables is refused, naming the key: it would be one setting written two ways.

**Names.** `name` is required, 1–64 characters of letters, digits, `.`, `_` and `-`, and unique
without regard to case (`Photos` and `photos` cannot both exist). It cannot be `.` or `..` or start
with `-`, so it is always a safe command-line argument. `default` is reserved for the first
table.

**The default pair** is the first `[[pair]]` table. It is the pair any command that names no
pair addresses — every `proton-sync` invocation written before this feature — so reordering the
tables changes which pair that is.

## Rules the daemon checks before it starts

- **Both roots in every table.** Beside other pairs, each table sets `local_root` and
  `remote_root`: no command-line flag can supply one (see below).
- **No two pairs may share a folder or nest.** A pair inside another pair's folder would have its
  `.sync` state directory — index, lockfile and sidecars — uploaded to Proton Drive as the outer
  pair's ordinary files. The same goes for two pairs over one Proton folder, one `db_path` or
  `lockfile_path` used twice, and a state file placed inside another pair's folder.
- **Symlinks are followed.** The check on the file is by path, and cannot see a symlink. The daemon
  checks again at startup on the folders as they really are, and refuses to start naming both
  pairs and, for every path, what you wrote and where it really points. A pair whose folder was
  missing when the daemon started is checked the same way when the folder appears, and stays
  unavailable (with the same message as its error in `proton-sync status`) instead of syncing into
  another pair's tree.
- **A pair that is already running is checked again.** Before each of its passes the daemon asks
  the same question of every other pair, running or not. If a layout changes under a running
  daemon — a pair's folder moved into another pair's with a link left where it was — **both pairs
  stop** (each shows as unavailable, with the overlap as its reason) rather than the outer pair
  uploading the inner pair's `.sync` files as its own. Stopping only the inner one would not be
  enough: its `.sync` is already inside the outer pair's folder. A pair that is *already
  unavailable*, for any other reason, is asked about too, because its `.sync` is still on disk:
  the running pair stops if that folder is inside it, and the unavailable pair stays as it was. Its
  folder counts only while it exists, whatever state that pair is in, so an unmounted drive stops
  nothing and holds back no pair that is coming back up. Nothing is deleted or
  tidied; undo the move or the link and both pairs resume at their next attempt. (The same layout
  would refuse to start the daemon.)
- **`dry_run = true` inside a `[[pair]]` table is refused** when there is more than one pair. A
  dry run previews one pair and exits, so a key inside one table cannot decide what the whole
  daemon does. The two readers of the file differ only in what a command line can do: **the
  daemon refuses to start on it unless `--dry-run` or `--no-dry-run` is given** (the flag then
  decides what the run does, and the key decides nothing), while the desktop app's save path has no
  command line and **always refuses to write it**. `dry_run = false` is accepted. Preview with
  `--dry-run` on the command line instead.

## Command-line flags

With one pair, flags amend that pair exactly as before. **With several pairs, every flag that
describes a folder is refused at startup**, naming it — `--local-root`, `--remote-root`,
`--db-path`, `--lockfile-path`, `--scan-interval-secs`, `--download-batch-size`, `--include`,
`--exclude`, `--events-driven`, `--no-events-driven`, `--events-full-scan-every`, `--warm-start`,
`--no-warm-start`, `--warm-start-full-walk-every`, `--warm-start-max-cursor-age-secs`,
`--no-delete-approval`, `--deletion-policy`, `--local-delete-mode` and `--conflict-suffix`. A flag
cannot say *which* pair it amends, and the opt-outs are the dangerous ones: applying
`--no-delete-approval` to one pair and not another would quietly turn a safeguard off for part of
your files. Set the value in the table it belongs to.

These still work beside any number of pairs: `--config`, `--socket-path`, `--proton-cli`,
`--proton-timeout-secs`, `--proton-list-attempts`, `--log-level`, and the mode flags `--dry-run`,
`--no-dry-run`, `--full-walk` (every pair's first pass) and `--pair`.

## Previewing one pair

`proton-syncd --config proton-sync.toml --dry-run` previews the **default pair**.
`--pair <NAME>` previews another:

```bash
proton-syncd --config proton-sync.toml --dry-run --pair photos
```

A preview rehearses one folder, so it is one pair per invocation. `--pair` names are matched
exactly; an unknown name is an error that lists the configured pairs, and `--pair` without a dry
run is an error too, because the daemon runs every pair. See [Dry-run](/safety/dry-run/).

## Controlling the pairs

`proton-sync` addresses one pair per command: `proton-sync --pair photos pause`,
`proton-sync --pair documents syncnow`. `--all-pairs` runs the command once per pair. Pausing a pair
pauses only that pair; the others keep syncing on their own schedules. A pair's pause is
remembered in that pair's own index, so it survives a daemon restart: the pair stays paused until you
resume it, and `proton-sync reset-index` does not clear it. One case is not covered: a pause made while the pair is unavailable (its folder or index cannot be reached) is kept in memory only, and a restart loses it. A paused unavailable pair is not retried, so to make the pause stick, resume the pair once its folder is back and pause it again. `proton-sync pause` says "Not saved". See the
[CLI reference](/cli/reference/#folder-pairs).

## Known limits

- **A request that names no pair acts on the default pair only.** That is every `proton-sync`
  command without `--pair`, and any client that predates folder pairs. The daemon logs a warning
  about this once, at startup, when it finds more than one pair configured. Use
  `proton-sync --pair NAME` to address the others. The desktop app addresses each folder itself: its
  **window** follows the folder you choose from the pill in its header, its **notifications** name
  the folder they are about and act on it, and its **tray** lists every pair, pauses and resumes each
  one separately, and shows the worst folder's state (see
  [the tray](/desktop/tray/#two-or-more-folders)). The app can create a `[[pair]]` file: its
  [Add folder dialog](/desktop/screens/#adding-and-removing-a-folder) turns a single-pair file into
  `[[pair]]` tables (the first one named `default`, with every per-pair key moved into it) and
  appends the new folder as one table, and its Settings screen then edits the chosen folder's own
  table for a per-pair setting and the top level for a daemon-wide one. It does not edit pairs
  written as an inline array (`pair = [{ ... }]`). Removing a folder takes its table out of the file
  and moves its sync history out of the folder (to `removed-pairs` in the app's state directory), so
  adding the same folder back later starts fresh.
- **Two pairs on one Proton volume can make each other do full scans.** Proton reports changes per
  volume, not per folder. When an event names something under neither pair's indexed folders, a
  pair falls back to a full walk of its remote tree — safe, but it is the cost event-driven
  detection exists to avoid. Even one pair pays this when you change something elsewhere in your
  Drive; with several pairs on the usual single volume, one pair's busy folder can trigger the
  others. **This has not been measured on a real account.** To measure it, run the daemon at
  `RUST_LOG=info` with two pairs on one volume, change files in one pair's folder only, and count
  the `event-driven pass fell back to a full-tree snapshot` lines per pair — every line carries its
  `pair{name=…}` prefix — against a run with one pair. If the idle pair's count follows the busy
  pair's activity, it is worth fixing; see
  [ADR 0005](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/adr/0005-multiple-folder-pairs.md)
  §8a.
- **One pair that cannot start does not stop the others** — a missing folder or a lock held by
  another process leaves that pair unavailable with the reason as its error. The exception is a
  lock held by another process *at startup*, which refuses to start and names the pair.
- **Uninstalling** removes every pair's `.sync` state directory, not only the first. It reads the
  roots from `[[pair]]` tables; pairs written as an inline array (`pair = [{ ... }]`) cannot be read
  by the script, which says so and leaves those `.sync` directories in place for you to remove.
- **Re-running `setup.sh` previews the default pair only.** It keeps an existing config, so with
  several pairs its start-up preview is of the first one while the service it starts syncs all of
  them. It says so; preview another with `proton-syncd --config proton-sync.toml --dry-run --pair NAME`.
