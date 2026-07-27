---
title: Quick start
description: Preview a sync plan, start the daemon, and drive it from the control CLI in a few minutes.
sidebar:
  order: 3
---

This walkthrough takes you from nothing to a running two-way sync, safely. It assumes
you've [installed](/start/installation/) the binaries and that `proton-drive` is logged in.

:::caution[Use a disposable folder first]
Sync is bidirectional and deletions propagate. For your first run, use a throwaway local
folder and a throwaway remote folder you don't mind losing. Read
[Deletions & the safeguard](/safety/deletions/) before pointing this at real data.
:::

## 1. Create a local folder

```bash
mkdir -p /tmp/proton-sync-demo
```

## 2. Preview the plan (`--dry-run`)

**Always do this before a folder's first real sync.** Dry-run scans locally, lists the
remote, reads any existing index read-only, prints the plan as JSON, and exits — without
uploading, downloading, deleting, or writing the index.

```bash
proton-syncd \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --dry-run
```

Look at the `summary` block, and especially at **`destructive_actions`** — the number of
deletes and purges the run would perform. On a first sync of a fresh pair it should be `0`
(the first pass is a non-destructive merge). See [Dry-run preview](/safety/dry-run/) for
how to read the output.

## 3. Start the daemon

Once the plan looks right, run the daemon in the foreground:

```bash
proton-syncd \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder
```

It reconciles once on startup, then watches the folder and reconciles on changes, on a
periodic timer, and on demand. Stop it with `Ctrl+C` or `SIGTERM`; it removes its socket
on shutdown.

By default the daemon keeps all of its state — the SQLite index, its status/metrics
sidecars, and the instance lockfile — in a `<local-root>/.sync/` directory that the engine
always ignores. The control socket lives at `$XDG_RUNTIME_DIR/proton-sync.sock`.

## 4. Drive it from another terminal

The control CLI talks to the running daemon over its Unix socket:

```bash
proton-sync status      # is it running? what did it just do?
proton-sync history     # recent sync summaries and errors
proton-sync syncnow     # reconcile right now
proton-sync pause       # stop automatic + manual sync
proton-sync resume      # resume
```

`status`, `pause`, `resume`, and `syncnow` print the full JSON status object, so scripts
can consume them directly. See the [CLI reference](/cli/reference/) for every command.

## 5. Approve your first deletion

The delete-approval guard is **on by default**, so the first time you delete a synced file
and reconcile, the deletion is *withheld* rather than applied. List and approve it:

```bash
proton-sync pending                 # what's withheld, and in which direction
proton-sync approve notes/old.txt   # approve one item
proton-sync syncnow                 # apply it now
```

This is deliberate — see [Delete approval](/safety/delete-approval/) to configure it
(including turning it off globally or per folder).

## Prefer a config file

Passing long paths on every invocation gets old. Put them in a TOML file and load it with
`--config`:

```toml
# proton-sync.toml
local_root = "/tmp/proton-sync-demo"
remote_root = "/Drive/RemoteFolder"
scan_interval_secs = 300
```

```bash
proton-syncd --config proton-sync.toml
```

Explicit CLI flags still override file values. See [Configuration](/daemon/configuration/).

## Or use the desktop app

If you'd rather not touch a terminal, launch the desktop app. Its **onboarding wizard**
walks you through the same four steps — check the CLI, choose the folder pair, review the
dry-run plan, and start the service — and then gives you live status, conflict resolution,
and a tray icon. See [Desktop app overview](/desktop/overview/).

## Next

- [How sync works](/concepts/how-sync-works/) — what each action means and why.
- [Running as a service](/daemon/running-as-a-service/) — keep it running with systemd.
- [Selective sync](/daemon/selective-sync/) — sync only part of a folder.
