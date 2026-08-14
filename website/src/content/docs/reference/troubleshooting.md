---
title: Troubleshooting
description: Fixes for the common failure modes — can't reach the daemon, lockfile errors, remote failures, and unexpected sync behavior.
sidebar:
  order: 1
---

## `proton-sync` can't connect to the daemon

The control CLI (and the app) reach the daemon over a Unix socket. If a command reports it
can't connect:

- Confirm the daemon is actually running (`systemctl --user status proton-syncd`, or check
  your foreground terminal).
- Confirm both sides use the **same socket path**. With no `--socket-path`, both default to
  `$XDG_RUNTIME_DIR/proton-sync.sock` (with an OS-temp fallback). If you started the daemon
  with a custom `--socket-path`, pass the same one to `proton-sync`.

In the desktop app this surfaces as the **Unreachable** state, which offers "Start
proton-syncd" and "View journal".

## The daemon exits with a lockfile error

Only **one daemon per user** may run, and only one per root. If startup fails with a lock
error, another live daemon already holds a lock — because every daemon shells the same
`proton-drive` CLI, whose shared cache isn't concurrency-safe. Stop the running daemon.
Note that `--lockfile-path` only isolates the **per-root** lock; the **per-user**
single-instance lock can't be bypassed by any flag (for a fully isolated test, run under a
separate `$XDG_STATE_HOME`). The daemon fails to start with a clear error naming the locked
lockfile, rather than failing silently.

A lockfile left on disk after the daemon stops is **normal** and does not mean a daemon is
running: the lock is an advisory `flock` on the file, released when the process exits, and
the (empty) file is deliberately kept so every start contends on the same one. Do not delete
it to "unstick" a start — if startup fails, a daemon really is holding the lock.

## Remote operations fail

If uploads, downloads, or listings fail, reproduce the underlying `proton-drive` call
directly to isolate the problem:

```bash
proton-drive filesystem list --json /Drive/RemoteFolder
```

If that fails, fix authentication, permissions, or the remote folder name first — the daemon
can only be as healthy as the CLI it shells. Check the daemon's logs for the structured
fields (operation, attempt, exit status, stderr, timeout); see [Logging](/daemon/logging/).

## After a reboot the daemon fails every sync until I restart it

Symptom: right after boot, `journalctl --user -u proton-syncd` shows every pass failing —
often `could not run the proton-drive CLI … No such file or directory (os error 2)` — and a
manual `systemctl --user restart proton-syncd` a few minutes later fixes it.

This is a **boot-ordering race**. A systemd *user* service can start before your desktop
session has (a) imported your login `PATH` into the user manager and (b) unlocked the desktop
keyring. So the daemon can't find the `proton-drive` CLI on `PATH`, and can't read its keyring
session. `PATH` is captured once at process start, so retries in the *same* process keep failing
the same way — only a fresh process (a restart) picks up the corrected environment.

Fixes:

- **Pin an absolute `proton_cli`** in `~/.config/proton-sync/proton-sync.toml`, e.g.
  `proton_cli = "~/.local/bin/proton-drive"`. This bypasses `PATH` and is the most reliable fix.
- The shipped unit already sets a `PATH` covering `~/.local/bin`, `~/.cargo/bin`, and `~/bin`, and
  is ordered `After=graphical-session.target`. If you hand-wrote your unit, re-copy the sample (or
  re-run `setup.sh`) and `systemctl --user daemon-reload`.
- The keyring side self-heals: the daemon re-checks the keyring every pass and resumes
  event-driven detection once it's unlocked — no restart needed. Make sure the keyring is actually
  unlocked at login (auto-login without a password can leave it locked).

## Event-driven passes are being skipped

Event-driven detection **reuses the `proton-drive` CLI's keyring session**. If the CLI is
idle and its token expires, a pass fails auth and is skipped until the CLI refreshes — and
the daemon **degrades to snapshot scans** meanwhile, so sync keeps working, just less
cheaply. Keep `proton-drive` logged in, and keep the desktop keyring unlocked (with
`DBUS_SESSION_BUS_ADDRESS` set) for a service. You can also opt out with `--no-events-driven`
to force snapshot-only detection.

## A deletion didn't apply

That's usually the [delete-approval guard](/safety/delete-approval/) doing its job:
deletions are withheld until approved. Check what's pending:

```bash
proton-sync pending
proton-sync approve <path>   # or --all
proton-sync syncnow
```

If a deletion *still* doesn't apply after approval, the entity may have changed since you
approved it — approvals are pinned to the exact content/id you saw, so a stale approval is
inert. Re-run `pending` to see the current queue.

## Sync behavior looks wrong — inspect safely

Pause the daemon and look at the local folder and the index before resuming. The index lives
at `<local-root>/.sync/sync_index.db` (the `.sync` directory is ignored by scanning, so
it's never uploaded):

```bash
proton-sync pause
sqlite3 <local-root>/.sync/sync_index.db 'select * from file_index order by file_path;'
```

Read the index **read-only** while the daemon might run — it has no WAL, so a writer and
reader can race. Prefer the `.status.json` / `.metrics.json` sidecars (or
`proton-sync status`) for live state.

## A new remote folder appeared and started filling up

If you point `--remote-root` at a path that **doesn't exist**, the daemon **creates it** and
uploads your local content into it. A typo therefore silently populates a brand-new folder
rather than failing. Always confirm the remote root with
`proton-drive filesystem list --json <remote-root>` first, and preview with `--dry-run`.

## Something native document won't sync

Proton **Docs and Sheets** are reported as `skip_unsupported` and left untouched on both
sides — the `proton-drive` CLI can't download those native types as files. Likewise,
**symlinks** under the local root are skipped in both directions. Neither is an error.
