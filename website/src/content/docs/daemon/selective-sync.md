---
title: Selective sync
description: Limit which paths sync with include/exclude globs, applied consistently to local files, remote files, and the index.
sidebar:
  order: 3
---

Use `--include` and `--exclude` (or the `include`/`exclude` config keys) to limit which
relative paths are eligible for sync. Patterns use glob syntax and match paths relative to
both `local-root` and `remote-root`.

```bash
proton-syncd \
  --local-root /tmp/proton-sync-demo \
  --remote-root /Drive/RemoteFolder \
  --include 'Documents/**' \
  --include 'Projects/**/*.md' \
  --exclude '**/*.tmp' \
  --exclude '**/.DS_Store' \
  --dry-run
```

## The rules

Filters are applied **consistently to local files, remote files, and existing index
records** before planning:

- With **no include** pattern, every non-ignored path is eligible.
- With **one or more include** patterns, only matching files are eligible.
- **Exclude always wins** over include.

Because the filters apply to the index too, an excluded record is **never purged as a
"missing" file** — excluding a path takes it out of sync entirely, in both directions,
without the engine deciding it was deleted. This is an important safety property.

## Always ignored

Some paths are ignored regardless of your filters, so operational files never become sync
data:

- **Conflict sidecars** (`.proton-cloud`) — so a kept-both file isn't re-synced.
- **The sync state directory** `<local-root>/.sync/` — the index DB, its `.status.json` and
  `.metrics.json` sidecars, and the lockfile. Only a *top-level* `.sync` is treated as
  state.
- **A custom `--db-path` under the local root** — so the index never becomes sync data.
- **Download staging directories** — any path component starting with
  `.proton-sync-download-`, so a partial download left by a crash is never uploaded.
- **`.proton-sync.toml`** at any depth — the machine-local per-directory settings files.

## Confirm before enabling

Selective-sync mistakes are easy to make and easy to check. **Run a dry-run after changing
filters** to confirm the eligible action set before you let the daemon sync for real:

```bash
proton-syncd --config proton-sync.toml --dry-run
```

## In the desktop app

The app's **Settings → Selective sync** section edits the raw `include`/`exclude` glob
lists directly (add and remove patterns), and states the two rules that trip people up:
exclude beats include, and `.sync/` is always ignored. A checkbox-driven folder-tree view
is planned but not yet available — it needs a directory-listing capability the daemon
doesn't expose today.
