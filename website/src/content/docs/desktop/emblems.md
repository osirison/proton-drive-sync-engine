---
title: File-manager emblems
description: Per-file sync-status emblems for Nautilus and Nemo, how they read the index, and which states are wired today.
sidebar:
  order: 4
---

File-manager **emblems** overlay a small sync-status badge on your files inside GNOME Files
(Nautilus) and Nemo, so you can see a file's state without opening the app. They read the
daemon's SQLite index **read-only** and never write anything.

Emblems are an **enhancement, never the only signal** — they ship as separate per-distro
packages, not inside the app, and not in a Flatpak. If they're not installed, you simply
don't get the overlays; nothing else changes.

## What ships

| File manager | Extension | Runtime dependency |
| --- | --- | --- |
| GNOME Files (Nautilus) | `proton-sync-nautilus.py` | `nautilus-python` (Debian: `python3-nautilus`) |
| Nemo | `proton-sync-nemo.py` | `nemo-python` |

Plus the emblem **icons** (`emblem-proton-sync-*.svg`) installed into the hicolor icon
theme. On **RPM and deb** the extensions ship as **separate optional packages**, so a
headless install doesn't pull in the GObject/Python stack; on **Arch/AUR** the icons and
extensions live in the single main package with the Python bindings
(`nautilus-python`/`nemo-python`) as `optdepends`. See
[Native packages](/distribution/packages/).

## States wired today

The extension walks **up** from a file to the sync root that holds `.sync/sync_index.db`,
opens that index read-only (with a busy timeout, since the index has no WAL), and looks up
the file's status. It maps the engine's stored status to an emblem:

| Index status | Emblem |
| --- | --- |
| `synced` | **Synced** |
| `modified` | **Syncing** |
| `conflict` | **Conflict** |

Two further emblem icons ship but are **not applied yet**: **excluded** (would need
selective-sync glob evaluation) and **paused** (would need live daemon state) — neither is
recorded in the index, so those overlays are a follow-up.

Because it finds the root by walking up to the nearest `.sync/sync_index.db`, the extension
works with a non-default `--db-path`/`--config` and with several sync roots at once. File
paths are matched as both text and raw bytes, so non-UTF-8 filenames still resolve.

## Installing and refreshing

After installing the icons, refresh the icon cache and restart the file manager:

```bash
gtk-update-icon-cache -f /usr/share/icons/hicolor
nautilus -q   # restart Nautilus to load the extension
```

## Not included

- **Dolphin (KDE)** — needs a C++ `KOverlayIconPlugin` (a different ecosystem); deferred.
- The extensions only **read** the index, so they can be installed or removed independently
  of the daemon and the app.
