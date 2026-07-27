---
title: Conflicts
description: What a conflict is, the .proton-cloud sidecar convention, and how to resolve conflicts from the CLI or the desktop app.
sidebar:
  order: 3
---

A **conflict** happens when *both* sides of a file changed to **different** content since
the last sync. The engine's rule is simple: **never lose an edit.** Instead of picking a
winner, it keeps both versions and asks you to choose.

(If both sides changed to the *same* content, there's no conflict — the engine just links
them with no sidecar.)

## The `.proton-cloud` sidecar

When it detects a conflict, the daemon keeps your local file as-is and downloads the remote
version alongside it, into a **sidecar** path using the `.proton-cloud` naming convention:

```text
notes.txt   →   notes.proton-cloud.txt
archive     →   archive.proton-cloud
```

The sidecar (the remote version) sits next to your original (the local version). Conflict
sidecars are **ignored by local scans**, so they don't create new sync records or get
re-uploaded, and the conflicting path is left untouched on both sides until you resolve it —
so no edit is lost in the meantime.

## Resolving from the CLI / filesystem

Conflict resolution is just file operations on the sidecar — there's no special command:

- **Keep your local version:** delete the `.proton-cloud` sidecar. Removing it marks your
  original as modified, and the next reconcile uploads your local version over the remote.
- **Adopt the remote version:** replace your local file with the sidecar's contents (e.g.
  move the sidecar over the original), then delete any leftover sidecar. The daemon treats
  the updated file as your new local version and syncs it.

## Resolving in the desktop app

The app's **Conflicts** screen makes this visual and safe. It shows a rail of conflicted
files and a **side-by-side compare** — your version (local) on one side, Proton's version
(the sidecar) on the other, with a line-level diff for text files and size/timestamp
metadata for binary or large files (it never fabricates a preview).

You pick one of **four choices** per file. Everything is **staged** — nothing is written to
disk until you press **Apply**:

| Choice | What it does on disk |
| --- | --- |
| **Keep mine** | Delete the sidecar; your local file uploads next pass. |
| **Use Proton's** | Replace your file with Proton's copy (move the sidecar over the original). |
| **Keep both** | Rename your file to `name.local.ext` and keep Proton's copy too. |
| **Decide later** | Nothing is written; the file stays outstanding. |

Choosing auto-advances to the next unresolved file, and the footer counts how many file
operations are staged. Applying runs them and re-scans. The outstanding-conflict count is
computed once and shown consistently everywhere it appears — the sidebar badge, the tab
header, the Overview "needs you" card, the stat tile, and the activity ledger.

See [Screens](/desktop/screens/) for the rest of the app.

## Type conflicts

A related case is a **type conflict**: a path is a *file* on one side and a *directory* on
the other. The engine can't merge those, so it reports a `type_conflict` action. Most type
conflicts are left untouched on both sides for you to sort out manually; the one exception
is a **local directory clashing with a same-named remote file**, where the engine preserves
the remote file's content into a `.proton-cloud` sidecar rather than discarding it.
