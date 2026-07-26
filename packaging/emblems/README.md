# File-manager emblems (S10, #91)

Per-file sync emblems (synced / syncing / conflict) for Linux file managers, by reading the sync
engine's SQLite index **read-only**. These ship as **separate per-distro packages** (installed by
P3/P4/P5), **not** inside the GUI app and **not** in a Flatpak — so emblems are an *enhancement*,
never the only signal of sync state.

## Contents & install destinations

| File | Installs to |
| --- | --- |
| `nautilus/proton-sync-nautilus.py` | `/usr/share/nautilus-python/extensions/` |
| `nemo/proton-sync-nemo.py` | `/usr/share/nemo-python/extensions/` |
| `icons/hicolor/scalable/emblems/emblem-proton-sync-*.svg` | `/usr/share/icons/hicolor/scalable/emblems/` |

After installing icons, refresh the cache: `gtk-update-icon-cache -f /usr/share/icons/hicolor`.

**Runtime deps:** `nautilus-python` (Fedora: `nautilus-python`, Debian: `python3-nautilus`) and/or
`nemo-python`; Python 3. Nautilus must be restarted (`nautilus -q`) to load the extension.

## How it works
For a file, the extension walks **up** its ancestors to the sync root that holds `.sync/sync_index.db`
(no hardcoded config path — the daemon may run with a different `--config`/`--db-path`, and there may
be several roots), opens that index `mode=ro` with a busy timeout (the index has no WAL), and queries
the file's path relative to the root. `file_path` is matched as both text and raw bytes so non-UTF-8
names resolve. Maps `sync_status` → emblem: `synced`→synced, `modified`→syncing, `conflict`→conflict.

## Scope / limitations (v1)
- **Three states only.** `excluded` (needs selective-sync glob evaluation) and `paused` (needs live
  daemon state) aren't in the index, so those two emblems ship as icons but aren't applied yet.
- **Dolphin (KDE) not included** — it needs a C++ `KOverlayIconPlugin` (KIO/ECM build), a different
  ecosystem; deferred to a follow-up issue.
- The Nautilus↔Nemo index logic is intentionally duplicated (each extension loads standalone from its
  own file-manager's dir, no shared `sys.path` import).

## Verification done here
`python3 -m py_compile` on both extensions (syntax) and `xmllint` on the SVGs. The live behaviour
(emblem rendering, the exact `add_emblem` name↔icon mapping per Nautilus 4 / Nemo version) needs a
real desktop session with `nautilus-python`/`nemo-python` installed — not available in this
environment.
