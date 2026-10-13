## Purpose

Defines the one upstream version of the product, how it maps to the git tag and to each package
format's version, and the checks that keep every file that states the version in agreement.

## ADDED Requirements

### Requirement: One upstream version

The product SHALL have one upstream version, written as `X.Y.Z` for a final release or `X.Y.Z-P.N`
for a pre-release, where `P` is `alpha`, `beta` or `rc` and `N` is a positive integer. Every file
the build reads its version from SHALL carry that string unchanged. Any other version shape SHALL be
refused.

#### Scenario: The first beta is set

- **WHEN** the version is set to `0.1.0-beta.1`
- **THEN** `cargo metadata` reports `0.1.0-beta.1` for `proton-drive-sync-engine`, `proton-sync-gui`
  and `proton-sync-gui-core`
- **AND** `gui/src-tauri/tauri.conf.json`, `gui/package.json` and the root entry of
  `gui/package-lock.json` all say `0.1.0-beta.1`

#### Scenario: A version shape the mapping does not cover is refused

- **WHEN** the version-setting script is given `0.1.0-beta` (no number), `0.1.0-preview.1` or
  `0.1.0+build.5`
- **THEN** it exits non-zero, names the accepted shapes, and changes no file

### Requirement: Each package format carries the mapped version

Each package format SHALL carry a version derived from the upstream version by one fixed rule. The
git tag is `v` plus the upstream version. RPM and AppStream replace `-` with `~`. Debian is the RPM
form plus the revision `-1`. Arch removes the `-` and every `.` from the pre-release part. A final
release has no pre-release part, so every form equals the upstream version (plus `-1` for Debian).

#### Scenario: The forms of 0.1.0-beta.1

- **WHEN** the upstream version is `0.1.0-beta.1`
- **THEN** the tag is `v0.1.0-beta.1`
- **AND** the RPM `Version` is `0.1.0~beta.1`
- **AND** the Debian version is `0.1.0~beta.1-1`
- **AND** the Arch `pkgver` is `0.1.0beta1` with `pkgrel=1`
- **AND** the AppStream `<release>` version is `0.1.0~beta.1`

#### Scenario: The forms of a final release

- **WHEN** the upstream version is `0.1.0`
- **THEN** the RPM `Version` and Arch `pkgver` are `0.1.0`, and the Debian version is `0.1.0-1`

### Requirement: A pre-release sorts below its final release

Every mapped pre-release version SHALL sort below the final release of the same `X.Y.Z` in that
format's own version comparison (RPM, dpkg, pacman `vercmp`, AppStream). Pre-releases of one
`X.Y.Z` SHALL sort by `P` (`alpha` < `beta` < `rc`), then by `N` as a number.

#### Scenario: A beta user is offered the final release

- **WHEN** a system has the `0.1.0-beta.1` package installed and its package manager sees the
  `0.1.0` package
- **THEN** the package manager treats `0.1.0` as an upgrade

#### Scenario: Pre-releases sort by number, not by text

- **WHEN** the mapped forms of `0.1.0-beta.9`, `0.1.0-beta.10` and `0.1.0-rc.1` are compared in
  each format
- **THEN** `beta.9` sorts below `beta.10`, and `beta.10` sorts below `rc.1`

### Requirement: Every version site agrees

A check SHALL compare every version site against the mapping of the root `Cargo.toml` version and
fail when any differs, naming the file, the expected value and the value found. It SHALL run on
every pull request and before a release is built. The docs site's own `website/package.json`
version is not a version site and SHALL NOT be checked.

#### Scenario: A stale PKGBUILD fails the pull request

- **WHEN** `Cargo.toml` says `0.1.0-beta.2` and `packaging/arch/PKGBUILD` still says
  `pkgver=0.1.0beta1`
- **THEN** the check fails with a message naming `packaging/arch/PKGBUILD`, the expected
  `0.1.0beta2` and the found `0.1.0beta1`

#### Scenario: The full list of sites is checked

- **WHEN** the check runs
- **THEN** it reads the three crate manifests, the three workspace entries in `Cargo.lock`,
  `tauri.conf.json`, `gui/package.json`, both root entries of `gui/package-lock.json`, the RPM
  spec's `Version`, upstream version and newest `%changelog` entry, the PKGBUILD's `pkgver` and
  upstream version, the newest `debian/changelog` entry, the newest AppStream `<release>`, and the
  archive name in `examples/packaging/release-assets.toml`

#### Scenario: The docs site version is ignored

- **WHEN** `website/package.json` says a version different from `Cargo.toml`
- **THEN** the check passes

### Requirement: Setting the version writes every site

One script SHALL set the version: given an upstream version, it SHALL write every version site,
regenerate the workspace entries in `Cargo.lock` and `gui/package-lock.json`, and add a dated entry
to the RPM `%changelog`, `debian/changelog` and the AppStream releases. Its output SHALL pass the
agreement check.

#### Scenario: A bump leaves a buildable tree

- **WHEN** the script sets the version to `0.1.0-beta.1` on a clean checkout
- **THEN** the agreement check passes
- **AND** `cargo build --workspace --locked` succeeds without changing `Cargo.lock`

### Requirement: A release tag must match the version it builds

The release workflow SHALL refuse, before building anything, a tag whose name is not `v` plus the
`Cargo.toml` version at the tagged commit, a tag whose commit is not on `main`, and a tag that is
not a valid upstream version.

#### Scenario: Tag and manifest disagree

- **WHEN** tag `v0.1.0-beta.2` is pushed on a commit whose `Cargo.toml` says `0.1.0-beta.1`
- **THEN** the release workflow fails with a message naming both versions
- **AND** no draft release and no release assets are created

#### Scenario: Tag on a commit outside main

- **WHEN** a valid, matching tag is pushed on a commit that is not reachable from `main`
- **THEN** the release workflow fails and creates no draft release

### Requirement: The binaries report their version

`proton-syncd --version` and `proton-sync --version` SHALL print the upstream version and exit 0.
Neither SHALL read a config file, open a socket, take a lock or start syncing to answer it.

#### Scenario: Asking an installed daemon its version

- **WHEN** `proton-syncd --version` runs on a system with no config file and no running daemon
- **THEN** it prints `proton-syncd 0.1.0-beta.1` and exits 0
- **AND** no socket, lockfile or state directory is created
