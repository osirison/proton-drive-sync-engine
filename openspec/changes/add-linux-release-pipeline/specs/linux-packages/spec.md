## Purpose

Defines the native Linux packages built from the repository's own packaging trees: what each one
installs, which distribution releases it supports, and the checks that prove it builds, installs
and runs before anyone can publish it.

## ADDED Requirements

### Requirement: Packages are built from the existing packaging trees

The RPM SHALL be built by `rpmbuild` from `packaging/rpm/proton-drive-sync-engine.spec`, the deb by
`dpkg-buildpackage` from `packaging/debian`, and the Arch package by `makepkg` from
`packaging/arch/PKGBUILD`, each inside a container of its own distribution. No other packaging
definition (a generated spec, a second control file, a bundler config) SHALL produce a published
package.

#### Scenario: Each package carries its own tree's dependencies

- **WHEN** the three packages are built for a release
- **THEN** the declared dependencies of the `.rpm`, `.deb` and `.pkg.tar.zst` are those written in
  the spec, `debian/control` and `PKGBUILD` respectively, plus the shared-library dependencies the
  distribution's tools generate from the built binaries

#### Scenario: No bundler output is released

- **WHEN** a release's assets are listed
- **THEN** none of them is an AppImage, a Tauri bundle, or a package made by a tool other than the
  three named above

### Requirement: Every package is built from one source tarball

Each release SHALL produce one source tarball, `proton-drive-sync-engine-<upstream>.tar.gz`, whose
only top-level directory is `proton-drive-sync-engine-<upstream>/` and whose files are the tracked
files of the tagged commit. The RPM, deb and Arch package SHALL each be built from that tarball,
not from a git checkout.

#### Scenario: The tarball matches the tag

- **WHEN** the release tarball for `v0.1.0-beta.1` is unpacked
- **THEN** it holds one directory, `proton-drive-sync-engine-0.1.0-beta.1/`
- **AND** its file list equals `git ls-tree -r --name-only v0.1.0-beta.1`

### Requirement: What a package installs

The main package of each family SHALL install `proton-syncd`, `proton-sync` and `proton-sync-gui`
to `/usr/bin`, the user unit `/usr/lib/systemd/user/proton-syncd.service` running
`/usr/bin/proton-syncd`, the desktop entry, the AppStream metainfo, the hicolor icons, and the
Apache-2.0 and two OFL-1.1 licence texts. Installing a package SHALL NOT enable or start the service
for any user; each user enables it after writing a config.

#### Scenario: The package names in each family

- **WHEN** a release is built
- **THEN** the RPM family is `proton-drive-sync-engine`, `proton-drive-sync-engine-nautilus` and
  `proton-drive-sync-engine-nemo`
- **AND** the deb family is `proton-drive-sync-engine`, `proton-sync-nautilus` and
  `proton-sync-nemo`
- **AND** Arch has one package, `proton-drive-sync-engine`, with `nautilus-python` and `nemo-python`
  as optional dependencies

#### Scenario: Installing does not enable or start the service

- **WHEN** the main package of any family is installed
- **THEN** `/etc/systemd/user/default.target.wants/proton-syncd.service` does not exist
- **AND** no `proton-syncd` process is running afterwards

### Requirement: The proton-drive CLI is never a package dependency

No package SHALL depend on, recommend, suggest or bundle the `proton-drive` CLI. Proton publishes
it only as a download from proton.me, and it is in no distribution's official repositories. Each
main package SHALL declare `curl` and the package that provides `secret-tool` as a dependency or a
recommendation, because the daemon runs both.

#### Scenario: Installing without proton-drive

- **WHEN** a main package is installed in a clean container that has no `proton-drive` binary
- **THEN** the install succeeds

#### Scenario: Helper tools arrive with the package

- **WHEN** a main package is installed with its recommended packages in a clean container
- **THEN** `curl` and `secret-tool` are on `PATH`

#### Scenario: No package relation names the CLI

- **WHEN** the dependency fields of the three main packages are read (RPM `Requires`,
  `Recommends` and `Suggests`; deb `Depends`, `Recommends` and `Suggests`; PKGBUILD `depends`,
  `optdepends` and `makedepends`)
- **THEN** none of them names a `proton-drive` package

### Requirement: Every mention of the CLI points only to Proton

Every mention of the `proton-drive` CLI in package metadata, the Arch install message, the release
notes header, `README.md` and the docs site's install pages SHALL link only Proton's official
pages and SHALL say that the CLI must be installed and signed in before this app is used. Where it
gives the sign-in command, it SHALL give `proton-drive auth login`. None SHALL name or link a
third-party build of the CLI.

#### Scenario: Each package description states the prerequisite

- **WHEN** the installed main package's description is shown with `rpm -qi`, `apt show` or
  `pacman -Qi`
- **THEN** it says that Proton's `proton-drive` CLI must be installed and signed in first

#### Scenario: The Arch install message names Proton's page

- **WHEN** the Arch package is installed with `pacman -U` in a clean container
- **THEN** pacman's output contains `https://proton.me/drive/download` and `proton-drive auth login`

#### Scenario: No third-party build is named

- **WHEN** the RPM spec, `packaging/debian/control`, the PKGBUILD, the Arch install script, the
  release notes header, `README.md` and the two install pages are searched
- **THEN** none names the AUR packages `proton-drive-cli`, `proton-drive-cli-git` or
  `proton-drive-cli-bin` (the CLI's own data path `~/.local/share/proton-drive-cli/` is not a
  package name)
- **AND** none contains `proton-drive login`
- **AND** every link about the CLI is one of `https://proton.me/drive/download`,
  `https://proton.me/download/drive/cli/index.html`, `https://proton.me/support/drive-cli` and
  `https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md`

### Requirement: Supported distribution releases

A release SHALL support x86_64 on: every Fedora release that is not end-of-life on the day the
release is published; Ubuntu 22.04, 24.04 and 26.04 and Debian 12 and 13, all served by one `.deb`;
and current Arch Linux. A distribution release SHALL be listed as supported only while the install
smoke test passes on it.

#### Scenario: One deb installs on every supported Debian and Ubuntu release

- **WHEN** the release `.deb` is installed with `apt-get install ./<file>.deb` in a clean
  container of Ubuntu 22.04, 24.04, 26.04, Debian 12 and Debian 13
- **THEN** the install succeeds on all five

#### Scenario: A supported release that stops installing blocks the release

- **WHEN** the `.deb` fails to install in the Debian 13 container
- **THEN** the release workflow fails and creates no draft release

### Requirement: Each package is install-tested before release

For every supported distribution release, the package SHALL be installed with that distribution's
package manager in a clean container, and the test SHALL check that both CLIs print the release
version, that the user unit runs `/usr/bin/proton-syncd` and is not enabled, that every shared
library `proton-sync-gui` links resolves, and that `proton-sync doctor` reports a missing
`proton-drive` CLI. The test SHALL NOT start the daemon or the GUI.

#### Scenario: Smoke test of an installed RPM

- **WHEN** the release `.rpm` is installed with `dnf install` in a clean Fedora container
- **THEN** `proton-syncd --version` and `proton-sync --version` print `0.1.0-beta.1`
- **AND** `/usr/lib/systemd/user/proton-syncd.service` exists and its `ExecStart` begins with
  `/usr/bin/proton-syncd`
- **AND** `ldd /usr/bin/proton-sync-gui` reports no library as not found

#### Scenario: The dependency check reports a missing CLI without the daemon

- **WHEN** `proton-sync doctor` runs in a clean container where the package is installed, no
  `proton-drive` binary is on `PATH` and no daemon is running
- **THEN** its output says the `proton-drive` CLI is missing and contains
  `https://proton.me/drive/download`
- **AND** it exits non-zero
- **AND** no `proton-syncd` process is started and no control socket is created

#### Scenario: The smoke test never runs the daemon

- **WHEN** any install smoke test runs
- **THEN** no `proton-syncd` process is started
- **AND** the binaries are run only with `--version`, except `proton-sync doctor`

### Requirement: Packaging changes are built before they merge

A pull request that changes anything under `packaging/`, `Cargo.lock`, any `Cargo.toml`,
`gui/src-tauri/tauri.conf.json` or the package-build workflow SHALL build all three packages and
run their install smoke tests. A pull request that changes none of these SHALL report the same
check as passed without building, so the check can be required.

#### Scenario: A control-file edit is built

- **WHEN** a pull request edits `packaging/debian/control`
- **THEN** the RPM, deb and Arch packages are built and smoke-tested on that pull request

#### Scenario: A docs-only change skips the build

- **WHEN** a pull request changes only files under `website/`
- **THEN** the package check reports success and no package is built

### Requirement: The deb build checks its build dependencies

The deb build SHALL check every `Build-Depends` entry in `packaging/debian/control` and fail on any
that is missing, except the Rust toolchain entries, which the build MAY supply itself when the
build container's distribution packages a `rustc` older than the floor the control file states.

#### Scenario: A missing library is caught by the package tools

- **WHEN** `libgtk-layer-shell-dev` is absent from the deb build container
- **THEN** the build fails before compiling, naming `libgtk-layer-shell-dev` as unmet

#### Scenario: An old distribution rustc does not block the build

- **WHEN** the build container's packaged `rustc` is older than the stated floor
- **THEN** the build uses a toolchain it installed itself and still checks every other entry
