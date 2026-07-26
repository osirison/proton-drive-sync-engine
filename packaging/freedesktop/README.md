# Freedesktop integration assets (P2, #94)

Shared assets the native packages (RPM/deb/PKGBUILD — P3/P4/P5) install so the GUI shows up in the
app menu, software centres, and with a proper icon. App id: **`app.protondrivesync.engine`**.

## Contents & install destinations

| File | Installs to (system prefix) |
| --- | --- |
| `app.protondrivesync.engine.desktop` | `/usr/share/applications/` |
| `app.protondrivesync.engine.metainfo.xml` | `/usr/share/metainfo/` |
| `icons/hicolor/<size>x<size>/apps/app.protondrivesync.engine.png` (16–512) | `/usr/share/icons/hicolor/<size>x<size>/apps/` |
| `icons/hicolor/scalable/apps/app.protondrivesync.engine.svg` | `/usr/share/icons/hicolor/scalable/apps/` |

The `.desktop` launches `proton-sync-gui` (installed to `/usr/bin` by the native packages) and its
`StartupWMClass` matches the Tauri app id so the launcher associates with the running window.

This set covers the **application** icon + launcher + AppStream. The other icon sets the design
mentions live in their own components, not here: the **tray** glyphs are embedded in the GUI binary
(S7, `gui/src-tauri/icons/tray/`) and the **file-manager emblem** icons ship with the emblem
packages (S10, `packaging/emblems/`).

## Validation (run in this directory)

```sh
desktop-file-validate app.protondrivesync.engine.desktop
appstreamcli validate app.protondrivesync.engine.metainfo.xml
```

Both pass here. Icons are rendered from the repo's `assets/icon.svg` (ImageMagick).

> Note: the AppStream `<release>` date and `0.1.0` version should be bumped at release time.
