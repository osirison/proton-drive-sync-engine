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
| `aur.sh` | Writes the AUR tree (`PKGBUILD` with the real checksum, install script, `.SRCINFO`) for the `stable` or `beta` package; see "Building" |

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
- **`options=('!lto' '!debug')`.** Found by the first real build. makepkg's default LTO puts
  `-flto` in the C flags, so the SQLite that `libsqlite3-sys` compiles is built as GCC LTO objects
  that rustc's linker (`ld.lld`) cannot read: linking `proton-syncd` failed with
  `undefined symbol: sqlite3_*`. Without `!debug`, makepkg also produced a second,
  `-debug` package of dangling symlinks (the release profile has no debug info), whose symlinks
  `namcap` reports as errors; the build must produce one package.
- **`python` is an `optdepends`, and `dbus` is in `depends`.** `namcap` reports the Nautilus/Nemo
  extension scripts as needing `python` (`import sqlite3`): as an error when it is in neither list,
  as `W: Dependency python detected but optional` when it is an optdepend. The extensions run only
  inside the two file managers, whose bindings depend on python, so a hard dependency would put it
  on every headless install. Measured on the built package with its `.PKGINFO` edited (first
  build, then `namcap`): python in `depends` gives no error; python in neither list gives
  `E: Dependency python detected and not included` and, because python had been supplying it,
  `E: Dependency dbus detected and not included (libraries ['usr/lib/libdbus-1.so.3'] needed in
  files ['usr/bin/proton-sync-gui'])`; `dbus` in `depends` plus python as an optdepend gives the
  `W` line above and no error. `python-nautilus` is not an Arch package (`pacman -Si` says so);
  `nautilus-python` and `nemo-python` are.
- **No `git` in `makedepends`.** The source is a tarball and nothing in the build runs git; a
  build in a clean `archlinux:base-devel` container (which has no git) passes without it.
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

To make the AUR tree, run `aur.sh` on a directory that holds the release tarball (and, for a
release, its `SHA256SUMS`), as an ordinary user on Arch:

```sh
packaging/arch/aur.sh beta  ./assets ./aur-beta      # proton-drive-sync-engine-beta
packaging/arch/aur.sh stable ./assets ./aur-stable   # proton-drive-sync-engine
```

It writes `PKGBUILD`, the install script and `.SRCINFO` (from `makepkg --printsrcinfo`) to the
third argument. The tarball's real SHA-256 replaces `'SKIP'`; if `SHA256SUMS` is there it must
list the tarball with the same sum, or the script stops. `beta` also sets
`pkgname=proton-drive-sync-engine-beta` with `provides` and `conflicts` naming
`proton-drive-sync-engine`. Nothing else differs from this PKGBUILD. The `arch` job of
`.github/workflows/packages.yml` runs it for `beta` on every build and checks the three results:
the fields above, `.SRCINFO` equal to a fresh `makepkg --printsrcinfo`, and `makepkg
--verifysource` accepting the checksum against the tarball.

## Version

`_upstream_ver`, `pkgver` and `pkgrel` are set by `packaging/version.sh set <upstream version>` and
compared against the root `Cargo.toml` by `packaging/version.sh check`. Versions are set only
through that script; do not edit these lines by hand.

`set` also drops the changelog and AppStream entries of every version equal to or higher than the
one it sets (it prints each), and refuses if a tag `v<that version>` exists. It reads **local tags
only**, so run `git fetch --tags` first; when it cannot look (git missing, or the tree is not the
top level of a git repository) it says so on stderr and goes on. `pkgrel` is reset to 1 only when
the upstream version changes. `SOURCE_DATE_EPOCH` fixes the date it writes.

## Validation

### First real build (2026-10-13)

The PKGBUILD was built for the first time in `archlinux:base-devel` (`rust` from the
repository), as an unprivileged user with `makepkg -s --noconfirm`, from the `git archive`
tarball beside the PKGBUILD. The same steps are the `arch` job of
`.github/workflows/packages.yml`. The first build **failed** and three things were fixed in the
PKGBUILD, each described under "Design choices" above:

| Found by | Symptom | Fix |
| --- | --- | --- |
| `makepkg` | link of `proton-syncd` failed: `ld.lld: error: undefined symbol: sqlite3_*` | `options=('!lto')` |
| `makepkg` | two packages: the main one and `proton-drive-sync-engine-debug` | `options=('!debug')` |
| `namcap` on the package | `E: Dependency python detected and not included` (Nautilus/Nemo scripts) | `python` as an `optdepends` and `dbus` in `depends` (first fix was `python` in `depends`) |

A fourth item was cleaned up, not a failure: `git` was in `makedepends` and nothing needs it.

After the fixes: one `proton-drive-sync-engine-0.1.0-1-x86_64.pkg.tar.zst`; `namcap PKGBUILD` and
`namcap` on the package report warnings only (make-dependencies already in `depends`, the
Python-binding modules the optdepends provide, the install script running tools that hooks
provide, unused `ld-linux`); `pacman -Qip` shows the `proton-drive` prerequisite in the description
and `curl`, `libsecret` and `hicolor-icon-theme` among `Depends On`.

`aur.sh beta` and `aur.sh stable` were run on the same tarball: the generated `PKGBUILD` has the
fields above, `makepkg --printsrcinfo` equals the generated `.SRCINFO`, and `makepkg
--verifysource` passes against the tarball and fails (`FAILED`) after the checksum is changed.

### Install smoke test (2026-10-13)

In a clean `archlinux:latest` container (`pacman -Syu`, no other package added), `pacman -U` of the
built package printed the install note (`==> Proton's proton-drive command-line client must be
installed and signed in before you use Proton Drive Sync. Download it from
https://proton.me/drive/download, then run: proton-drive auth login`), and the same `pacman -U`
again (a reinstall, which runs `post_upgrade`) printed no note. `packaging/smoke-test.sh arch`
then passed: both CLIs print the version, the files are where the spec says, the unit runs
`/usr/bin/proton-syncd` and is not enabled, `ldd proton-sync-gui` resolves every library, and
`proton-sync doctor` reports `proton-drive` missing with exit 1. This is the `smoke-arch` job of
`.github/workflows/packages.yml`.

### Earlier, before the first real build

- `bash -n PKGBUILD` and `bash -n proton-drive-sync-engine.install` passed (syntax only). The
  sandbox was Fedora-based, so `namcap` and `makepkg` could not run there; the first real build
  above superseded this.

## Post-install

```sh
systemctl --user enable --now proton-syncd
```

Edit `~/.config/proton-sync/proton-sync.toml` first (see the shipped example at
`/usr/share/doc/proton-drive-sync-engine/examples/proton-sync.toml`) and make sure the
`proton-drive` CLI is on `$PATH` and signed in (`proton-drive auth login`) before starting the
service. The package does not enable or start the service for any user.
