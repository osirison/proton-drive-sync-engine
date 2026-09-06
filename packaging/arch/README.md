# Arch / AUR packaging (P5, #97)

`PKGBUILD` for `proton-drive-sync-engine`, targeting the AUR. Builds and installs:

- `proton-syncd`, `proton-sync` — root crate (`cargo build --release --bins`).
- `proton-sync-gui` — the Tauri desktop GUI, workspace member `gui/src-tauri`
  (`cargo build --release -p proton-sync-gui`).
- A packaged systemd **user** unit (`proton-syncd.service`, `ExecStart=/usr/bin/proton-syncd`,
  installed to `/usr/lib/systemd/user/`) — do not confuse with the `cargo install`-oriented
  example at `examples/systemd/proton-syncd.service`, which is left untouched.
- The P2 freedesktop launcher/icon/AppStream assets from `packaging/freedesktop/`.
- The S10 Nautilus/Nemo sync-status emblem extensions + icons from `packaging/emblems/`.

## Contents

| File | Purpose |
| --- | --- |
| `PKGBUILD` | Package build recipe |
| `proton-drive-sync-engine.install` | post_install/post_upgrade/post_remove scriptlets (icon cache, desktop database, AppStream refresh) |
| `proton-syncd.service` | Packaged systemd user unit (`ExecStart=/usr/bin/proton-syncd`) |

## Design choices / deviations worth flagging

- **Single package, not split.** Emblem extensions install in the main package; the Python
  bindings that actually load them are `optdepends` (`nautilus-python`, `nemo-python`) rather
  than a separate `-nautilus`/`-nemo` subpackage. Simpler, and idiomatic for AUR-sized projects.
- **`nautilus-python` / `nemo-python`, not `python-nautilus` / `python-nemo`.** These are the
  current Arch `extra` package names for the GNOME Files / Nemo Python bindings
  (`python-nautilus` is the stale pre-rename name). Verified against
  `https://archlinux.org/packages/extra/x86_64/nautilus-python/` and
  `https://archlinux.org/packages/extra/x86_64/nemo-python/`.
- **`libayatana-appindicator`** is in both `depends` and `makedepends`, beyond the originally
  scoped dependency list. The GUI enables Tauri's `tray-icon` feature for the S7 system tray;
  on Linux that feature links against libappindicator via pkg-config
  (`libappindicator-sys` in `Cargo.lock`) at build time and needs the same library at
  runtime for the tray icon to render.
- **`proton-drive` CLI is deliberately not a `depends`.** The daemon shells out to it and reads
  its keyring session via `secret-tool` (from `libsecret`, which *is* a real dependency). At the
  time of writing `proton-drive` is not in the official Arch repos or the AUR — install it
  separately and make sure it's on `$PATH` before starting `proton-syncd`.
- **Source is a git tag, not a release tarball**, since no GitHub release/tag exists yet:
  `source=("$pkgname::git+$url.git#tag=v$pkgver")` with `sha256sums=('SKIP')` (standard for a
  VCS-fetched source). **Prerequisite:** tag `v0.1.0` must be pushed on a commit that already
  contains `packaging/freedesktop/`, `packaging/emblems/`, and this `packaging/arch/` directory
  — `package()` reads all three at their in-repo paths after the clone.
- **`prepare()` runs `cargo fetch --locked`** so `build()` can use `--frozen` (fully offline)
  inside makepkg's chroot, per the Arch Rust packaging guidelines pattern.

## Building

```sh
cd packaging/arch
makepkg -si            # build + install, resolving deps via pacman/AUR helper
```

`makepkg` needs, beyond this package's own `depends`/`makedepends`: a configured Rust
edition-2024-capable toolchain (Arch's `rust` package tracks current stable, which qualifies)
and network access to clone the tagged source.

Before submitting to the AUR, regenerate the metadata file from this directory:

```sh
makepkg --printsrcinfo > .SRCINFO
```

Once a real `v0.1.0` tag exists upstream, replace the placeholder checksum:

```sh
updpkgsums            # from pacman-contrib; rewrites sha256sums=('SKIP') with the real hash
```

## Validation performed here

- `bash -n PKGBUILD` and `bash -n proton-drive-sync-engine.install` — **passed** (syntax only).
- `namcap PKGBUILD` — **could not run**: `namcap` is an Arch-only static analysis tool and this
  sandbox is Fedora-based; it isn't installed and can't reasonably be added here. Run it after
  a real `makepkg` build on Arch (or in an `archlinux` container):
  ```sh
  namcap PKGBUILD
  namcap *.pkg.tar.zst    # after a successful makepkg build
  ```
- A full `makepkg -si` build — **could not run**: it needs the Tauri build dependencies
  (webkit2gtk-4.1 + libsoup3 + gtk3 + glib2 dev headers via pkg-config), a Rust edition-2024
  toolchain, and network access to clone the (not-yet-existing) `v0.1.0` tag — none of which are
  available/appropriate in this non-Arch sandbox. Not faked; see "Building" above for the real
  command and prerequisites.

  That parenthesis is the record of what the attempt needed then, and it is no longer the
  whole list: `gtk-layer-shell` was added to `depends` and `makedepends` afterwards
  (#351/#370 — the tray panel is a layer surface), so a real `makepkg -si` on Arch needs it
  too. Arch ships the headers in the same package, so there is no separate `-devel` to add
  alongside it.

## Post-install

```sh
systemctl --user enable --now proton-syncd
```

Edit `~/.config/proton-sync/proton-sync.toml` first (see the shipped example at
`/usr/share/doc/proton-drive-sync-engine/examples/proton-sync.toml`) and make sure the
`proton-drive` CLI is on `$PATH` and logged in before starting the service.
