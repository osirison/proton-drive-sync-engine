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
`proton-drive` is **not** in Debian/Ubuntu repositories and is not (and cannot be) a
package `Depends:` — it's called out in the `proton-drive-sync-engine` package
`Description` as a user-provided binary that must be on `PATH`. `libsecret-tools`
(provides `secret-tool`) is listed as a `Recommends:`, since it *is* packaged and real.

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
`dh_installsystemduser --name=proton-syncd`, which looks for the source file named
`debian/<package>.<name>.user.service` and installs it as
`/usr/lib/systemd/user/proton-syncd.service`. Users enable it per-session with:

```sh
systemctl --user enable --now proton-syncd
```

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

### Build-Depends (must be installed first)

- `debhelper-compat (= 13)` (pulls in a suitable `debhelper`)
- `cargo (>= 1.85~)`, `rustc (>= 1.85~)` — the root crate is `edition = "2024"`, which
  needs a fairly recent toolchain (edition 2024 stabilized in Rust 1.85). Older
  Debian/Ubuntu-packaged `rustc` (e.g. Debian 12 bookworm's) is too old; use
  `rustup`/backports/a recent Ubuntu release if the archive `rustc` doesn't qualify.
- `pkg-config`
- `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `libgtk-3-dev`, `libglib2.0-dev`,
  `libgtk-layer-shell-dev` (Tauri GUI; the last one is #351/#370's layer-surface tray panel)

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
dev headers, a Rust >= 1.85 toolchain confirmed to build edition 2024, `lintian`) could
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
  `>= 1.85` (this sandbox's `rustc`/`cargo` happen to be new enough at the time of
  writing, but the *system* packages this depends on for the GUI are not installed) and
  network egress to crates.io. Not attempted; see the exact command above.

  That list is the record of what the attempt would have needed then, and it is no longer
  the whole one: `libgtk-layer-shell-dev` was added afterwards (#351/#370 — the tray panel
  is a layer surface). The Build-Depends bullet above carries the current list.

### A known, accepted lintian note

`packaging/debian/source/format` is `3.0 (native)` but the changelog version is
`0.1.0-1` (a dashed Debian revision). Native-format packages conventionally use an
undashed version (`0.1.0`); pairing native format with a dashed version is expected to
surface lintian's `native-package-with-dash-version` (informational) tag. This is
intentional: keeping the `-1` revision leaves room for packaging-only bumps
(`0.1.0-2`, ...) without touching the upstream version, and native format avoids
standing up separate `orig.tar` tooling for a monorepo where this packaging tree and
the application source live together. No `lintian-overrides` file is included for this
— `lintian` wasn't available here to confirm the exact tag name/output, and an
unverified override is worse than an honestly documented, informational note.
