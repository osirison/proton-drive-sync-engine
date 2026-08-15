---
title: Dry-run preview
description: Preview the exact sync plan as JSON without touching a byte — the output shape, the summary counters, and destructive_actions.
sidebar:
  order: 4
---

`--dry-run` is your seatbelt. It computes and prints the **exact plan the daemon would
execute**, as JSON, and then exits — without uploading, downloading, deleting, or writing
the index. Use it before the first real sync of a new folder, after changing roots, or
whenever behavior surprises you.

```bash
proton-syncd \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --dry-run
```

Dry-run still contacts Proton Drive through the `proton-drive` CLI (to list the remote), so
authentication and remote permissions must already work. It does **not** bind the IPC
socket, take the daemon lock, or run any side effect.

## The output shape

```json
{
  "summary": {
    "total": 1,
    "uploads": 0,
    "downloads": 1,
    "remote_directories_created": 0,
    "local_directories_created": 0,
    "local_moves": 0,
    "remote_moves": 0,
    "auto_links": 0,
    "conflicts": 0,
    "type_conflicts": 0,
    "remote_deletes": 0,
    "local_deletes": 0,
    "purges": 0,
    "skipped_unsupported": 0,
    "destructive_actions": 0
  },
  "plan": [
    {
      "path": "notes.txt",
      "destination_path": null,
      "action": "download",
      "entity_kind": "file",
      "conflict_path": null,
      "remote_id": "remote-file-id"
    }
  ]
}
```

Every field is **always present**, so scripts can rely on a fixed shape. Unused summary
counters are `0`, and the optional plan fields (`destination_path`, `conflict_path`,
`remote_id`) are `null` when they don't apply. One field is the exception:
`skip_reason` appears **only** on a `skip_unsupported` row.

:::note[Paths that are not valid UTF-8]
A filename is bytes on Linux and does not have to be valid UTF-8, but JSON strings do.
Such a path is written with `U+FFFD` (`�`) in place of the offending bytes, exactly as
`proton-sync` and the control socket report it. That rendering is for **display**: it is
not a round-trippable name, so two paths differing only in invalid bytes look identical.
Never feed one back to a command as a selector.
:::

### Reading a plan row

- `action` — one of `upload`, `download`, `create_remote_directory`,
  `create_local_directory`, `move_local`, `move_remote`, `auto_link`, `conflict`,
  `type_conflict`, `remote_delete`, `local_delete`, `purge`, `skip_unsupported`. See [what
  each means](/concepts/how-sync-works/#the-planned-actions).
- `entity_kind` — `"file"` or `"directory"`.
- `destination_path` — populated only for the two move actions.
- `conflict_path` — populated only for a `conflict`, naming the `.proton-cloud` sidecar.
- `remote_id` — the Proton Drive node id, when known.
- `skip_reason` — present only on `skip_unsupported`: `remote_not_downloadable` (a Proton
  Docs/Sheets document, or a node the remote listing could not fully decode) or
  `unrepresentable_path` (a name that is not valid UTF-8). Render an unfamiliar token
  verbatim; more may be added. `proton-sync status` lists the same items under
  `can't sync`.

## The number that matters: `destructive_actions`

`destructive_actions` is the sum of `remote_deletes`, `local_deletes`, and `purges` — the
count worth a second look before you run for real. On a first sync of a fresh pair it
should be `0`, because the first pass is a non-destructive merge.

:::caution
A non-zero `destructive_actions` means the run would delete data somewhere. Confirm the
`remote_delete` / `local_delete` rows are what you intend before starting the daemon
unattended. Remember that a `local_delete` is a **permanent** local removal — see
[Deletions](/safety/deletions/).
:::

Note the two "destructive" senses the tooling distinguishes:

- **Display-destructive** — `remote_delete`, `local_delete`, **and** `purge`. These are
  tinted red and sorted first in the app's plan view, and counted in `destructive_actions`.
- **Gated** — only `remote_delete` and `local_delete`. A `purge` is index-only cleanup, so
  a purge-only plan is never gated by the [delete-approval guard](/safety/delete-approval/)
  or by the app's typed-`DELETE` confirmation.

## In the desktop app

The app's **Plan preview** screen runs the same dry-run and renders it as a summary grid
plus one row per action, destructive rows floated to the top. When the plan contains a real
delete, **Apply** is inert until you type `DELETE` to arm it, and the confirmation names the
files at risk. See [Screens](/desktop/screens/).

One honest caveat the app surfaces: the dry-run and the real reconcile are **separate
invocations**, so a plan applied a moment later can differ from the one you reviewed if
something changed in between. Re-run the preview if you want a fresh check.

## Selective sync applies

Dry-run respects your `--include`/`--exclude` filters, so it's the right way to confirm a
[selective-sync](/daemon/selective-sync/) rule set before enabling regular sync.
