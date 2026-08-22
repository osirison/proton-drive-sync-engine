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

![The conflicts screen showing one file, notes/todo.txt, with your version on the left and Proton's version on the right, each summarised in plain language, and four choices below: Keep mine, Keep both, Use Proton's, Decide later.](../../../assets/screenshots/conflicts.png)

*The desktop app's view of the same thing: both versions side by side, and no choice made
until you make it.*

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

The app's **Conflicts** screen makes this visual and safe. It shows the conflicts
one file at a time, behind a `1 of N` pager, with a **side-by-side compare** — your version
(local) on one side, Proton's version (the sidecar) on the other. Each side shows size, line
count and edit time; **See the exact differences** opens a line-level diff for text files.
A binary or oversized file shows metadata only — no preview is invented for it.

You pick one of **four choices** per file, and **each one writes immediately**. There is no
staging step and no Apply button:

| Choice | What it does on disk |
| --- | --- |
| **Keep mine** | Delete the sidecar; your local file uploads next pass. |
| **Use Proton's** | Replace your file with Proton's copy (move the sidecar over the original). |
| **Keep both** | Rename your file to `name.local.ext` and keep Proton's copy too. |
| **Decide later** | Nothing is written; the file stays outstanding. |

Choosing auto-advances to the next unresolved file and re-scans. The outstanding-conflict
count is computed once and shown identically wherever it appears — the status chip, the
attention band on the main screen, and notifications.

See [Screens](/desktop/screens/) for the rest of the app.

## Type conflicts

A related case is a **type conflict**: a path is a *file* on one side and a *directory* on
the other. The engine can't merge those, so it reports a `type_conflict` action. Most type
conflicts are left untouched on both sides for you to sort out manually; the one exception
is a **local directory clashing with a same-named remote file**, where the engine preserves
the remote file's content into a `.proton-cloud` sidecar rather than discarding it.
