---
title: Troubleshooting
description: Fixes for the common failure modes — can't reach the daemon, lockfile errors, an unavailable folder pair, remote failures, and unexpected sync behavior.
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
lockfile — and, for a per-root lock, the folder pair — rather than failing silently.

A lockfile left on disk after the daemon stops is **normal** and does not mean a daemon is
running: the lock is an advisory `flock` on the file, released when the process exits, and
the (empty) file is deliberately kept so every start contends on the same one. Do not delete
it to "unstick" a start: deleting it lets a second daemon lock a fresh inode and run
alongside the first. Only a start that fails with the lock error above is being refused by
the lock — startup failures with any other message have another cause.

## A folder pair shows an error and isn't syncing

A folder pair whose local folder can't be used does **not** stop the daemon: the pair shows
the reason (`proton-sync status`; the desktop app shows the error rather than the first-run
wizard), every other pair keeps syncing, and the daemon tries the pair again on its normal
cadence. The reason says which of these it is:

- **The folder doesn't exist** — it couldn't be created when the daemon started, or it went
  away since (an unplugged or unmounted drive, a deleted folder). Put it back and the next
  attempt picks it up; if the folder is on a drive, mount the drive. The daemon creates the
  folder only when it *starts*, never while it runs. **Don't restart the daemon to fix a
  missing folder on a drive**: a restart creates a missing folder, and a mount point whose
  drive isn't mounted would be created as an empty folder on the wrong disk and then
  treated as your sync folder
  ([#426](https://github.com/osirison/proton-drive-sync-engine/issues/426)). Restart only
  when the folder is truly gone and should be created empty.
- **Replaced by an empty folder** — the folder is there, but it is a different, empty
  directory (or it was emptied together with its `.sync` state directory) while files are
  recorded as synced for it. Typically that is an unmounted drive's empty mount point, or a
  folder deleted and made again. Reconciled as it stands, that reads as "delete everything",
  and starting over in it would download everything into it, so the daemon holds the pair
  unavailable instead: nothing is deleted, created or downloaded. That holds for every state
  layout, the default `.sync` directory inside the folder included. There are two ways out:
  - **Mount the drive, or put the folder back.** The next attempt picks it up once the folder
    holds something again. If you restore only part of it (or the folder holds only a stray
    entry such as a file manager's `.directory` or an empty `lost+found`), the daemon takes it
    from there, and the first pass **withholds every deletion for approval** whatever your
    delete-approval setting says: the files you have not restored yet show up as pending
    deletions, and nothing is deleted until you approve them, restore the files, or start
    over — restarting the daemon does not change that, it remembers they are waiting. (With
    the default `.sync` layout the index went with the folder, so that pass only downloads and
    adopts and has nothing to delete.)
  - **Start this folder over from Proton** with `proton-sync reset-index --yes`. It downloads
    everything into the folder and deletes nothing. Don't restart the daemon to accept an
    empty folder: the hold lives in the running daemon only, and a restart over an empty mount
    point either deletes everything remote (delete approval off, index kept outside the
    folder) or downloads everything into the mount point (default layout)
    ([#426](https://github.com/osirison/proton-drive-sync-engine/issues/426)).
- **Its state was removed along with the folder** — the folder was deleted and made again
  with files in it, and the index and lock inside it went with it. The daemon prepares the
  pair again from the new folder on its next attempt; you don't need to do anything. (If the
  new folder is empty, it is the case above — also if it is emptied before that next attempt,
  replaced or emptied in place: the daemon remembers how many items the old folder had
  recorded, assumes there were items if it could not read that, and holds the folder rather
  than starting over in it. The message then says the number "could not be counted"; the two
  ways out are the same.)
- **The filesystem names the folder differently on each look** (logged once at start) — the
  filesystem, typically a FUSE mount, reports a different identity for the folder each time it
  is asked, so a folder replaced while the daemon runs cannot be told from the same one. For
  that folder the daemon only checks that it is there: nothing is held or withheld for a
  replacement, and your delete-approval setting applies as it is. Keep delete approval on for
  such a folder, or move it to a filesystem with stable inode numbers.
- **The folder is not available: an I/O error** — a check of the folder failed with
  something other than "not found" (an `Input/output error` or a stale handle on a network
  mount). The daemon cannot tell what folder it would be working on, so the pass ends and is
  tried again at the pair's next turn; nothing further is done to the folder, and the
  adoptions the pass had already worked out are kept, so a large first sync does not start
  over each time. It clears by itself once the mount answers.
- **Locked by another process** — something else holds the folder's lock (a second
  `proton-syncd`, or another account's daemon syncing the same folder). A daemon that is
  *starting* refuses to start, naming the pair; one that is already running leaves that pair
  unavailable until the lock is free.
- **It overlaps another folder pair** — the reason names both pairs and, for every path, what
  the config says and where it really points. For two folders it explains that "Two pairs may not
  share a folder or nest" and why, in a sentence that carries on after those words; the other
  forms say that one pair's index or lockfile is inside the other's folder, or that two pairs
  share a state file. Two pairs' folders are, on disk, the same folder or one inside the other —
  usually a symlink, or a folder moved into another pair's. The daemon refuses to *start* on it;
  a pair whose folder appears later stays unavailable; and when two **running** pairs come to
  overlap, **both** stop before their next pass, because the outer one would otherwise upload the
  inner one's `.sync` directory (its lockfile and status files) as ordinary files. A running pair
  also stops when a pair that is **already unavailable** (for any other reason) has its folder
  inside it, but a pair counts only while its folder exists, whatever state it is in: an unplugged
  or unmounted drive stops nothing.
  Nothing is deleted. Undo the move or the link, and both resume at their next attempt.
- **Its state could not be opened, or its metrics file could not be written** — the
  `.sync` directory (or the index path you configured) isn't usable. Fix the permissions or
  free the space.

If the daemon can't *watch* a folder (usually the inotify watch limit, `ENOSPC`), the pair is
not unavailable: it keeps syncing, but scans the local folder on every pass instead of
waiting for change events, and registers the watch again before each pass. Raise
`fs.inotify.max_user_watches` to get the cheap path back.

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
