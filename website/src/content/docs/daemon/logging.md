---
title: Logging
description: Structured tracing logs on stderr, RUST_LOG verbosity, and the status/metrics sidecar files.
sidebar:
  order: 4
---

## Structured logs on stderr

The daemon and the dry-run path emit **structured logs through `tracing`**, written to
**stderr**. Stdout is reserved for machine-readable dry-run JSON, so you can pipe a plan
without log lines polluting it.

Control verbosity with the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug proton-syncd \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder
```

Useful values are `error`, `warn`, `info`, and `debug`. When `RUST_LOG` is unset, the
daemon defaults to `info`.

Retry attempts, unsuccessful `proton-drive` CLI exits, and command timeouts are logged with
structured fields — operation, attempt, exit status, stderr, and timeout milliseconds — so
you can see exactly which remote call misbehaved.

Under systemd, these logs go to the journal:

```bash
journalctl --user -u proton-syncd.service -f
```

## The start-up check

Right after the control socket is up, the daemon checks the two things nothing else would
report. The check runs once, on its own thread, so the socket, a shutdown and the first sync
never wait for it.

| Check | When | What it looks at |
| --- | --- | --- |
| `proton-drive` | always | The CLI the daemon will use (`proton_cli`, else `proton-drive` on the `PATH` **the daemon runs with**) exists and answers `version`. A systemd user service has a shorter `PATH` than your terminal, so this can fail where `proton-sync doctor` passes. |
| `curl` | when a folder follows changes (`events_driven`) | `curl` is installed; the change feed needs it. |

A working CLI is one `INFO` line with its version and path. A problem is one `WARN` line with
three fields: `check` (which check), `fix` (what to do) and `docs` (the section of
[Troubleshooting](/reference/troubleshooting/#dependency-check) that explains it). Each is
logged once per start, and **the daemon keeps running** whatever the result; a missing `curl`
only makes every sync walk the whole folder.

The sign-in and the keyring are not part of this check. The daemon reports them when a sync
needs them, once per cause, and `proton-sync status` shows the sign-in state. To check
everything, including those, run `proton-sync doctor` (see the
[CLI reference](/cli/reference/#checking-this-machine-doctor)).

## Status and metrics sidecars

Alongside logs, the daemon persists two JSON files next to its SQLite index, for local
inspection by scripts, service monitors, and the desktop app. They are written **atomically**
and are always ignored by sync.

| File (for `sync_index.db`) | Contents |
| --- | --- |
| `sync_index.status.json` | Recent status history — the last **20** entries: timestamp, message, last error, plan summary, and successful-sync summary. Reloaded on restart, so recent failures and successes survive a restart. |
| `sync_index.metrics.json` | A metrics snapshot for operational inspection. |

These sidecars — not the live SQLite index — are the safe way to read daemon state, because
the index runs without WAL and a reader would race the writer. The desktop app and the
`proton-sync history` command both surface the status history.

See the [CLI reference](/cli/reference/) for `proton-sync status` and `proton-sync history`,
which expose the same information over the control socket.
