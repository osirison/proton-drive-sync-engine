# Debian/Ubuntu packaging (P4, #96)

Debian packaging tree for the three shipped binaries (`proton-syncd`, `proton-sync`,
`proton-sync-gui`), the packaged systemd **user** unit, the P2 freedesktop integration
assets, and the S10 Nautilus/Nemo emblem extensions.

## Layout / binary-package split

- **`proton-drive-sync-engine`** (Architecture: any) — `proton-syncd`, `proton-sync`,
  `proton-sync-gui`, the desktop entry + AppStream metainfo + hicolor app icons
  (`packaging/freedesktop/`), the emblem *icons* (`packaging/emblems/icons/`), and the
  packaged systemd user unit at `/usr/lib/systemd/user/proton-syncd.service`.
- **`proton-sync-nautilus`** (Architecture: all) — the Nautilus sync-status emblem
  extension (`packaging/emblems/nautilus/`). `Depends: python3-nautilus`.
- **`proton-sync-nemo`** (Architecture: all) — the Nemo sync-status emblem extension
  (`packaging/emblems/nemo/`). `Depends: nemo-python | python3-nemo` (package name
  varies by distro/release).

The emblem extensions are split into their own optional packages (per #96's scope) so a
headless/server install of the main package doesn't pull in `nautilus-python`/
`nemo-python` and the GObject/python stack. They're wired as `Enhances:` on their own
package and `Suggests:` from the main package — not a hard `Depends:` in either
direction, since the extensions only *read* the daemon's SQLite index file and can be
installed/removed independently.

## The external `proton-drive` CLI dependency

`proton-syncd` shells out to Proton's own `proton-drive` CLI for every remote operation
and reads its cached login session from the desktop keyring via `secret-tool`.
Proton publishes `proton-drive` only as a download from `https://proton.me/drive/download`,
and it is in no distribution's official repositories, so it is not (and cannot be) a package
`Depends:` — the `proton-drive-sync-engine` package `Description` says it must be installed,
on `PATH` and signed in (`proton-drive auth login`) before the package is used. `libsecret-tools`
(provides `secret-tool`) and `curl` (the daemon runs it for the events API) are listed as
`Recommends:`, since both are packaged and real. The package prints nothing at install.

## systemd user unit

The example unit shipped in the repo (`examples/systemd/proton-syncd.service`) uses
`ExecStart=%h/.cargo/bin/proton-syncd`, which assumes a `cargo install` layout — wrong
for a distro package. This tree ships its own copy with the path patched to
`ExecStart=/usr/bin/proton-syncd`:
`packaging/debian/proton-drive-sync-engine.proton-syncd.user.service`.

It's installed via `dh_installsystemduser` (debhelper >= 12, included by default in the
`dh` sequence at `debhelper-compat (= 13)` — no extra `--with` needed). Because the
packaged unit is named `proton-syncd.service` rather than the package-name default
(`proton-drive-sync-engine.service`), `debian/rules` overrides the step with
`dh_installsystemduser --name=proton-syncd --no-enable`, which looks for the source file named
`debian/<package>.<name>.user.service` and installs it as
`/usr/lib/systemd/user/proton-syncd.service`. `--no-enable` is there because without it the
generated `postinst` enables the unit for every user at install, and every user's next login then
starts the daemon against a config that may not exist, which systemd restarts every 10 s. Each user
enables it after writing a config:

```sh
systemctl --user enable --now proton-syncd
```

## Version

The version lives only in `debian/changelog`. `packaging/version.sh set <upstream version>` adds
the dated entry (`0.1.0~beta.1-1` for `0.1.0-beta.1`) and `packaging/version.sh check` compares the
newest entry against the root `Cargo.toml`. Versions are set only through that script; do not add
the entry by hand.

`set` also drops the changelog and AppStream entries of every version equal to or higher than the
one it sets (it prints each), and refuses if a tag `v<that version>` exists. It reads **local tags
only**, so run `git fetch --tags` first; when it cannot look (git missing, or the tree is not the
top level of a git repository) it says so on stderr and goes on. Setting the same version again
keeps a raised revision (`-2`). `SOURCE_DATE_EPOCH` fixes the date it writes.

## Building a real `.deb`

`dpkg-buildpackage` hard-codes `./debian` relative to the invocation directory — it has
no flag to point at a control tree living elsewhere. Since this tree deliberately lives
under `packaging/debian/` (so it can sit alongside the freedesktop/emblem asset dirs
without claiming the repo-root `debian/` namespace for a *branch* that isn't the
canonical Debian packaging location yet), symlink it into place before building, from
the **repository root**:

```sh
ln -s packaging/debian debian
dpkg-buildpackage -us -uc -b
rm debian   # remove the symlink again afterward; do not commit it
```

With a Rust toolchain installed by `rustup` instead of the distribution's `cargo` and `rustc`
packages (the release build on Ubuntu 22.04, which packages an older Rust), name the build profile
that drops those two entries from the build-dependency check. Every other entry is still checked:

```sh
dpkg-buildpackage -us -uc -b --build-profiles=pkg.proton-drive-sync-engine.rustup
```

The same profile is `-P pkg.proton-drive-sync-engine.rustup` to `dpkg-checkbuilddeps` (it has no
long form) and `apt-get build-dep` reads it from `DEB_BUILD_PROFILES`.

Only binary packages are built (`-b`). No source package is built or published: the source format
is `3.0 (native)` with the revision `-1` in the version, and `dpkg-source -b` from dpkg 1.22
(Debian 13) refuses that pair (`native package version may not have a revision`). dpkg 1.21 (Ubuntu
22.04) only warns. A binary-only build does not run `dpkg-source`, so the pair is accepted there on
every release, and the `-1` is kept so that a packaging-only rebuild can be `-2`.

### Build-Depends (must be installed first)

- `debhelper-compat (= 13)` (pulls in a suitable `debhelper`)
- `cargo (>= 1.90~)`, `rustc (>= 1.90~)` — the root crate is `edition = "2024"`, and the locked
  `tauri-runtime`, `tauri-utils`, `tauri-runtime-wry`, `tauri-plugin-fs` and `tray-icon` crates
  declare `rust-version = 1.90`, which is the real floor. Debian 12's `rustc` is too old; use
  `rustup` with the profile above, or a distribution release that packages 1.90 or newer.
- `pkg-config`
- `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `libgtk-3-dev`, `libglib2.0-dev`,
  `libgtk-layer-shell-dev` (Tauri GUI; the last one is #351/#370's tray panel, promoted to a
  layer surface where the compositor supports one)

`debian/rules` drives `cargo build` directly (`override_dh_auto_build`) rather than
relying on debhelper's Rust/cargo buildsystem auto-detection, since a `dh-cargo`-style
buildsystem class isn't assumed to be present. It also skips `dh_auto_test`
deliberately — the workspace's non-unit tests include `#[ignore]`'d gates that need a
real authenticated Proton Drive session/keyring (see the repo's top-level `CLAUDE.md`);
those aren't appropriate to run at package-build time.

Network access to crates.io is required during the build (dependencies are not
vendored/pre-fetched into the source package) — this tree does not attempt Debian
archive-quality reproducibility/offline builds; it targets a locally-built,
locally-installable `.deb`.

## Validation actually run in this sandbox

This is a Fedora sandbox with no `apt`/root, so none of the build-deps above (webkit2gtk
dev headers, a Rust >= 1.90 toolchain, `lintian`) could
be installed. What *was* run, from the repo root:

- `dpkg-parsechangelog -l packaging/debian/changelog` — **parses cleanly**, no errors or
  warnings.
- `dpkg-checkbuilddeps` (via the symlink trick above, against `packaging/debian/control`)
  — **parses the `control` file's `Build-Depends`/field syntax without error**; it (as
  expected) reports the webkit2gtk/gtk3/glib -dev packages as not installed on this box
  — that's a missing-build-deps report, not a syntax problem.
- `debian/rules` was reviewed by hand for `make` syntax; a real `dpkg-buildpackage -b`
  was **not** attempted — it needs the Tauri build-deps and a Rust toolchain new enough
  for edition 2024, neither available here, and per the task instructions a fake/partial
  run was explicitly avoided rather than claimed as passing.

Not run / not available, and why:

- **`lintian`** — not installed, and there is no `apt`/root in this sandbox to install
  it. Not run.
- **Full `dpkg-buildpackage -us -uc -b`** — would need `libwebkit2gtk-4.1-dev` +
  `libsoup-3.0-dev` + `libgtk-3-dev` + `libglib2.0-dev` + a Rust toolchain confirmed
  `>= 1.90` (this sandbox's `rustc`/`cargo` happen to be new enough at the time of
  writing, but the *system* packages this depends on for the GUI are not installed) and
  network egress to crates.io. Not attempted; see the exact command above.

  That list is the record of what the attempt would have needed then, and it is no longer
  the whole one: `libgtk-layer-shell-dev` was added afterwards (#351/#370 — the tray panel
  becomes a layer surface where the compositor supports one). The Build-Depends bullet
  above carries the current list.

### Why no source package is built

`packaging/debian/source/format` is `3.0 (native)` but the changelog version has a dashed Debian
revision (`0.1.0-1`, or `0.1.0~beta.1-1` for a pre-release). Native format conventionally has no
revision. Run on this tree, with `packaging/debian` linked as `./debian`:

| `dpkg-source -b` from | Result |
| --- | --- |
| dpkg 1.22.22 (Debian 13) | error: `can't build with source format '3.0 (native)': native package version may not have a revision` |
| dpkg 1.21.1 (Ubuntu 22.04) | warning `native package version may not have a revision`, and the source package is built |

So a source package cannot be built on current dpkg, and none is published. A binary-only
`dpkg-buildpackage -b` does not run `dpkg-source` and accepts the version on both. The `-1` is
kept on purpose: it is what allows a packaging-only rebuild (`-2`) without touching the upstream
version. Switching to `3.0 (quilt)` with an `.orig.tar.gz` named after the Debian version would
allow source packages, and is only worth doing if a PPA is ever added.
