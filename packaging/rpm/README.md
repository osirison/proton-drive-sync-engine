# RPM packaging (P3, #95)

Fedora/COPR `.spec` for the whole desktop app: `proton-syncd`, `proton-sync`, and the Tauri GUI
`proton-sync-gui`, plus a packaged systemd `--user` unit, the P2 freedesktop launcher/icon/
AppStream assets, and the S10 file-manager emblem assets as separate optional subpackages.

## Contents

| File | Purpose |
| --- | --- |
| `proton-drive-sync-engine.spec` | The RPM spec (main package + `-nautilus` + `-nemo` subpackages) |
| `proton-syncd.service` | Packaged copy of `examples/systemd/proton-syncd.service` with `ExecStart=/usr/bin/proton-syncd` (the example uses a `cargo install` path) |

## Package layout

- **`proton-drive-sync-engine`** (main): `proton-syncd`, `proton-sync`, `proton-sync-gui` in
  `/usr/bin/`; the systemd user unit at `/usr/lib/systemd/user/proton-syncd.service`; the P2
  desktop launcher, AppStream metainfo, and app icons (`packaging/freedesktop/`); the S10 emblem
  **icons** (`packaging/emblems/icons/`). `Requires: curl libsecret hicolor-icon-theme` plus
  systemd's own `Requires(post/preun/postun)`, plus a soft `Recommends: libappindicator-gtk3`. GTK3/
  WebKitGTK/libsoup3/dbus/cairo/glib runtime deps are **not** hand-listed — RPM's automatic
  ELF/soname dependency generator picks those up from the actual linked binaries. This was verified
  against a real build in this environment (see "Validation done in this environment" below):
  `rpm -qp --requires` on the built RPM lists exactly the expected sonames, auto-generated, no
  manual entries. `libappindicator-gtk3` (Fedora's package for `libappindicator3.so.1`) is
  deliberately a `Recommends`, not a build or hard runtime `Requires`: `tray-icon` (pulled in via
  Tauri's `tray-icon` feature) loads it with `dlopen` at runtime via the `libloading` crate
  (`libappindicator-sys` has no `build.rs`, no header/pkg-config dependency at all) — confirmed by
  inspecting the vendored crate source during this build and by the fact the built binary has no
  `libappindicator3` in its ELF `NEEDED` list. An earlier draft of this spec had
  `libappindicator-gtk3-devel` in `BuildRequires`; it was removed after the real build proved it
  unnecessary.
- **`proton-drive-sync-engine-nautilus`**: `packaging/emblems/nautilus/proton-sync-nautilus.py`,
  requires `nautilus-python`.
- **`proton-drive-sync-engine-nemo`**: `packaging/emblems/nemo/proton-sync-nemo.py`, requires
  `nemo-python`.

Both emblem subpackages are split out from the main package (per the issue) so a headless/server
install of the daemon doesn't pull in the GObject/Python stack.

## The `proton-drive` external dependency

`proton-syncd` shells out to the official `proton-drive` CLI and reads its keyring session via
`secret-tool`. `proton-drive` is published by Proton only as a download from proton.me and is in no
distribution's official repositories, so it **cannot** be an RPM `Requires` — it is documented in
`%description` as a binary the user must install and sign in to before using this package.
`libsecret` (provides `secret-tool`) **is** a real `Requires`.

## Source0 / release tag

`Source0` is the source tarball attached to the GitHub release:
```
https://github.com/osirison/proton-drive-sync-engine/releases/download/v%{upstream_version}/proton-drive-sync-engine-%{upstream_version}.tar.gz
```
`upstream_version` is the version as the tag and the asset name write it (`0.1.0-beta.1`), while
`Version` is the RPM form (`0.1.0~beta.1`, which sorts below `0.1.0`). For a final release the two
are equal. Both are written by `packaging/version.sh`, which is the only way to change them (see
"Version" below).

The tarball is made with `git archive --prefix=proton-drive-sync-engine-<upstream_version>/`, so its
one top-level directory is `%{name}-%{upstream_version}`, and `%prep` unpacks that name. It is not
GitHub's own tag archive: that one names its top-level directory after the tag (`…-v0.1.0`) and
changes with GitHub's archive code, while the release asset is made once and fixed.

## Version

`Version`, `%global upstream_version` and the newest `%changelog` entry are set together by
`packaging/version.sh set <upstream>` and compared by `packaging/version.sh check`. Do not edit them
by hand; the script is the only place the mapping from the upstream version to the RPM version is
written.

`set` also drops the changelog and AppStream entries of every version equal to or higher than the
one it sets (it prints each), and refuses if a tag `v<that version>` exists. It reads **local tags
only**, so run `git fetch --tags` first; when it cannot look (git missing, or the tree is not the
top level of a git repository) it says so on stderr and goes on. `SOURCE_DATE_EPOCH` fixes the date
it writes.

## Build strategy: plain `cargo build`, not cargo2rpm

This workspace pulls in the Tauri/webkit2gtk-rs binding stack (`gui/src-tauri`), which is not
individually packaged in Fedora's crate ecosystem. Following the strict Fedora Rust packaging
guidelines (one subpackage per crate, `%cargo_generate_buildrequires`) would require hundreds of
crate subpackages that mostly don't exist upstream — infeasible for a COPR-targeted package. This
spec instead does a plain, network-fetching `cargo build --release --locked` against crates.io in
`%build`, which is the common pattern for large Rust GUI apps distributed via COPR.

**This means the build needs network access**, which mock/COPR disable by default for the actual
build step (as opposed to the dependency-resolution step, which always has network):

- **COPR**: in the project's Settings → your build chroot → check **"Enable internet access
  during builds"** (or pass `--enable-net on` via `copr-cli buildscm`/`copr-cli build` if scripting
  the trigger).
- **Local `mock`**: add `--enable-network` to the build invocation, e.g.:
  ```sh
  mock -r fedora-42-x86_64 --enable-network \
      --spec packaging/rpm/proton-drive-sync-engine.spec \
      --sources packaging/rpm/ \
      --resultdir /tmp/mock-result
  ```
  (`Source0` still needs to actually resolve — either publish the release first, or generate a
  local source tarball named `proton-drive-sync-engine-0.1.0.tar.gz` whose top-level directory is
  `proton-drive-sync-engine-0.1.0` and drop it in the sources dir mock is pointed at, e.g. via
  `git archive --format=tar --prefix=proton-drive-sync-engine-0.1.0/ HEAD | gzip -n > packaging/rpm/proton-drive-sync-engine-0.1.0.tar.gz`.)

## No debuginfo subpackage

`%global debug_package %{nil}` is set. The workspace's `[profile.release]` doesn't turn on Rust
debug info (Cargo's default), so `find-debuginfo` finds nothing to split out and rpmbuild hard-
fails on an empty `debugsourcefiles.list`. Producing real Rust debuginfo (`RUSTFLAGS=-Cdebuginfo=2`
plus a working debugsource layout) is a reasonable follow-up but out of scope here.

## Validation

### Built in CI's container (2026-10-13)

The spec was built from the `git archive` tarball in a `fedora:43` container (the oldest release
that is not end of life that day: Fedora 43 until 2026-12-09, 44 until 2027-06-02, from
`https://endoflife.date/api/fedora.json`), with Fedora's own `rust` and `cargo` packages and
`dnf builddep` on the spec. These are the steps of the `rpm` job of
`.github/workflows/packages.yml`. **No defect was found in the spec**: it built as written and
produced the source RPM, the main RPM and the `-nautilus` and `-nemo` RPMs.

On the main RPM, `rpm -qp --requires` lists `curl`, `libsecret` and `hicolor-icon-theme` beside the
generated sonames (`libgtk-layer-shell.so.0` among them), no `proton-drive`, and `rpm -qip` shows
the prerequisite text with `https://proton.me/drive/download` and `proton-drive auth login`.
`rpm -qlp` shows the three binaries, the user unit, the desktop entry, the metainfo, the icons and
the three licence texts; the scriptlets enable nothing themselves (`systemd-update-helper
install-user-units` applies the distribution's presets).

The `%{?dist}` is part of the file names: `proton-drive-sync-engine-0.1.0-1.fc43.x86_64.rpm`. The
workflow renames outputs with `packaging/name-assets.sh`, which replaces only the version
(`0.1.0~beta.1` becomes `0.1.0-beta.1`) and leaves the release and `.fc43` alone.

### Earlier, in the development sandbox

This sandbox turned out to already have every devel package the spec required **at the time**
installed (`webkit2gtk4.1-devel`, `libsoup3-devel`, `gtk3-devel`, `glib2-devel`,
`libappindicator-gtk3-devel`, `dbus-devel`) plus a working `cargo`/`rustc` 1.97.1 toolchain
(via rustup, not the `rust`/`cargo` RPMs) and network access, so beyond spec-syntax checks a real
end-to-end build was run against a local source tarball built from this branch (working around the
not-yet-existing `v0.1.0` git tag) — see the PR description for the exact pass/fail results,
including `rpmlint`.

That parenthesis is the record of what this sandbox had then, and it is no longer the whole
`BuildRequires` list: `gtk-layer-shell-devel` was added afterwards (#351/#370 — the tray
panel becomes a layer surface where the compositor supports one), so repeating the build
below on a fresh box needs that package as well.

Commands used:
```sh
# Spec sanity (macro expansion, package/subpackage listing):
rpmspec -P --define "_sourcedir $PWD/packaging/rpm" packaging/rpm/*.spec
rpmspec -q --qf '%{name} %{version} %{release}\n' \
    --define "_sourcedir $PWD/packaging/rpm" packaging/rpm/*.spec

# rpmlint (pip-installed in this sandbox; not preinstalled):
rpmlint packaging/rpm/*.spec

# Full build against a local source tarball (see "Source0 / release tag" above for why):
rpmbuild -bb --define "_topdir /tmp/rpmbuild" \
    --define "_sourcedir /tmp/rpmbuild/SOURCES" \
    packaging/rpm/proton-drive-sync-engine.spec
```

What a **real** COPR/mock build additionally needs that a from-scratch sandbox typically won't
have: the release tarball published (or a local copy in the sources directory), and network enabled for the build chroot
as described above.
