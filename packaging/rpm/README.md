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
`secret-tool`. `proton-drive` is not in Fedora/COPR repos and **cannot** be an RPM `Requires` — it
is documented in `%description` as a binary the user must install separately and have on `$PATH`.
`libsecret` (provides `secret-tool`) **is** a real `Requires`.

## Source0 / release tag

`Source0` is a GitHub tag archive:
```
https://github.com/osirison/proton-drive-sync-engine/archive/refs/tags/v%{version}/proton-drive-sync-engine-%{version}.tar.gz
```
GitHub's source archives keep the *tag* (not the requested filename) as the top-level directory
name, so `%prep` unpacks `%{name}-v%{version}`. **No `v0.1.0` tag exists yet** in this repo — push
one at release time, or for COPR, use the "Build from SCM" method (rpkg/Git webhook) instead of a
tagged tarball while the project is pre-tag; that snapshots the working tree directly and doesn't
need `Source0` to resolve.

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
  (`Source0` still needs to actually resolve — either push the release tag first, or generate a
  local source tarball named `proton-drive-sync-engine-0.1.0.tar.gz` whose top-level directory is
  `proton-drive-sync-engine-v0.1.0` and drop it in the sources dir mock is pointed at, e.g. via
  `git archive --format=tar --prefix=proton-drive-sync-engine-v0.1.0/ HEAD | gzip > packaging/rpm/proton-drive-sync-engine-0.1.0.tar.gz`.)

## No debuginfo subpackage

`%global debug_package %{nil}` is set. The workspace's `[profile.release]` doesn't turn on Rust
debug info (Cargo's default), so `find-debuginfo` finds nothing to split out and rpmbuild hard-
fails on an empty `debugsourcefiles.list`. Producing real Rust debuginfo (`RUSTFLAGS=-Cdebuginfo=2`
plus a working debugsource layout) is a reasonable follow-up but out of scope here.

## Validation done in this environment

This sandbox turned out to already have every build-time devel package installed
(`webkit2gtk4.1-devel`, `libsoup3-devel`, `gtk3-devel`, `glib2-devel`,
`libappindicator-gtk3-devel`, `dbus-devel`) plus a working `cargo`/`rustc` 1.97.1 toolchain
(via rustup, not the `rust`/`cargo` RPMs) and network access, so beyond spec-syntax checks a real
end-to-end build was run against a local source tarball built from this branch (working around the
not-yet-existing `v0.1.0` git tag) — see the PR description for the exact pass/fail results,
including `rpmlint`.

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
have: the `v0.1.0` git tag pushed (or SCM-webhook build), and network enabled for the build chroot
as described above.
