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
| `proton-drive-sync-engine.install` | post_install/post_upgrade/post_remove scriptlets (icon cache, desktop database, AppStream refresh; `post_install` also prints the `proton-drive` prerequisite note) |
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
- **`proton-drive` CLI is deliberately not a `depends` or an `optdepends`.** The daemon shells
  out to it and reads its keyring session via `secret-tool` (from `libsecret`, which *is* a real
  dependency, as is `curl`, which the daemon also runs). Proton publishes `proton-drive` only as
  a download from `https://proton.me/drive/download`, and it is in no distribution's official
  repositories — install it, put it on `$PATH` and sign in with `proton-drive auth login` before
  using this package. The install script prints that once, at the first install.
- **Source is the release tarball**, not a git clone:
  `source=("${_pkgbase}-${_upstream_ver}.tar.gz::${url}/releases/download/v${_upstream_ver}/${_pkgbase}-${_upstream_ver}.tar.gz")`.
  The tarball is made by `git archive` with one top-level directory,
  `${_pkgbase}-${_upstream_ver}`, which every function `cd`s into. `package()` reads
  `packaging/freedesktop/`, `packaging/emblems/` and this directory from inside it.
- **`_pkgbase` and `_upstream_ver` are separate from `pkgname` and `pkgver`.** Arch has no `-` in
  a version, so a pre-release `0.1.0-beta.1` is `pkgver=0.1.0beta1` (which `vercmp` sorts below
  `0.1.0`), while the tag and the tarball keep `0.1.0-beta.1`. The AUR beta package changes only
  `pkgname`, `provides` and `conflicts`; the install script and the source path use `_pkgbase`.
- **`sha256sums=('SKIP')` in the repository copy.** The PKGBUILD is inside the tarball it would
  name, so the tarball's own checksum cannot be written into it. The copy published to the AUR
  gets the tarball's real SHA-256 in place of `'SKIP'`.
- **`prepare()` runs `cargo fetch --locked`** so `build()` can use `--frozen` (fully offline)
  inside makepkg's chroot, per the Arch Rust packaging guidelines pattern.

## Building

```sh
cd packaging/arch
makepkg -si            # build + install, resolving deps via pacman/AUR helper
```

`makepkg` needs, beyond this package's own `depends`/`makedepends`: a configured Rust
edition-2024-capable toolchain (Arch's `rust` package tracks current stable, which qualifies)
and network access to download the release tarball. To build without the download, put the
tarball beside the PKGBUILD under the name `proton-drive-sync-engine-<upstream version>.tar.gz`;
makepkg uses a file of that name instead of downloading it.

To publish to the AUR, regenerate the metadata file from the directory that holds the tarball's
real checksum:

```sh
makepkg --printsrcinfo > .SRCINFO
```

The checksum is the only difference between this PKGBUILD and the AUR one:

```sh
updpkgsums            # from pacman-contrib; rewrites sha256sums=('SKIP') with the real hash
```

## Version

`_upstream_ver`, `pkgver` and `pkgrel` are set by `packaging/version.sh set <upstream version>` and
compared against the root `Cargo.toml` by `packaging/version.sh check`. Versions are set only
through that script; do not edit these lines by hand.

`set` also drops the changelog and AppStream entries of every version equal to or higher than the
one it sets (it prints each), and refuses if a tag `v<that version>` exists. It reads **local tags
only**, so run `git fetch --tags` first; when it cannot look (git missing, or the tree is not the
top level of a git repository) it says so on stderr and goes on. `pkgrel` is reset to 1 only when
the upstream version changes. `SOURCE_DATE_EPOCH` fixes the date it writes.

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
  (#351/#370 — the tray panel becomes a layer surface where the compositor supports one),
  so a real `makepkg -si` on Arch needs it too. Arch ships the headers in the same package,
  so there is no separate `-devel` to add alongside it.

## Post-install

```sh
systemctl --user enable --now proton-syncd
```

Edit `~/.config/proton-sync/proton-sync.toml` first (see the shipped example at
`/usr/share/doc/proton-drive-sync-engine/examples/proton-sync.toml`) and make sure the
`proton-drive` CLI is on `$PATH` and signed in (`proton-drive auth login`) before starting the
service. The package does not enable or start the service for any user.
