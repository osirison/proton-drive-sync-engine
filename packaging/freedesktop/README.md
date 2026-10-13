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

## Version

The newest `<release>` of the metainfo carries the version (`0.1.0~beta.1` for `0.1.0-beta.1`, with
`type="development"`; a final release has `type="stable"`) and the date. They are written by
`packaging/version.sh set <upstream version>` and compared by `packaging/version.sh check`.
Versions are set only through that script; do not add a `<release>` by hand.

`set` also drops the changelog and AppStream entries of every version equal to or higher than the
one it sets (it prints each), and refuses if a tag `v<that version>` exists. It reads **local tags
only**, so run `git fetch --tags` first; when it cannot look (git missing, or the tree is not the
top level of a git repository) it says so on stderr and goes on. `SOURCE_DATE_EPOCH` fixes the date
it writes.
