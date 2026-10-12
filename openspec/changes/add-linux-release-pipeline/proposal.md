## Why

The only way to install the app today is to build it from source with `setup.sh` (`cargo install`
into `~/.cargo/bin`). The repo has packaging trees for RPM, deb and Arch, but no workflow builds
them, the deb and Arch trees have never been built at all, and no tag or release exists. The first
release, `0.1.0-beta.1`, needs a repeatable path from a version tag to packages that Fedora,
Debian/Ubuntu and Arch users can install and upgrade with their own package manager.

## What Changes

- Set the version to `0.1.0-beta.1` everywhere it is written, with one mapping from that version
  to each package format (RPM `0.1.0~beta.1`, deb `0.1.0~beta.1-1`, Arch `0.1.0beta1`), one script
  that writes every site, and one check that fails when any site disagrees.
- Build the `.rpm`, `.deb` and Arch package from the existing trees in `packaging/` (rpmbuild,
  dpkg-buildpackage, makepkg), each in its own distro container, from the same source tarball.
  Install each package in clean containers of every supported release and check it.
- Build all three packages on any pull request that changes packaging, the lockfile, or a build
  dependency, so the trees cannot go stale between releases.
- A pushed `v*` tag runs the full CI suite and the package builds and, only if all pass, creates
  a **draft** GitHub release (marked pre-release for a pre-release version) holding the packages,
  the source tarball and `SHA256SUMS`.
- Publishing that draft (a person's action) pushes the release to Fedora COPR, the AUR, and a
  signed apt repository for Debian/Ubuntu hosted on GitHub Pages in a separate repository.
- Pre-releases go to beta channels only; final releases go to both beta and stable channels.
- Packaging tree fixes needed for the above: `Source0`/`source=` point at the release tarball,
  each tree separates its package version from the upstream version, the deb Rust build-deps
  move behind a build profile, the stated Rust floor moves from 1.85 to 1.90 (what the locked
  Tauri crates declare), and both binaries answer `--version`.
- **Behaviour change (deb only):** installing the `.deb` no longer enables the user unit for
  every user; like the RPM and Arch packages, each user enables it after writing a config. The deb
  and Arch packages also declare `curl`, which the daemon runs and only the RPM declared.
- README and the two install pages on the docs site describe the channels this pipeline
  produces: repo URLs, `dnf copr enable`, AUR package names, the apt source line and key. Their
  first step, above every channel's commands, is "install Proton's `proton-drive` CLI and run
  `proton-drive auth login`".
- Everything this change writes about the `proton-drive` CLI (the three package descriptions, the
  PKGBUILD comment, the release notes header, README and the two install pages) links only
  Proton's own pages and names no third-party build. Proton publishes the CLI only as a download
  from proton.me, and it is in no distribution's official repositories (community repackages
  exist; this project does not point to them). The Arch package prints a
  short note at install naming Proton's download page and `proton-drive auth login`; the RPM and
  deb print nothing and rely on their descriptions and the docs. No package names the CLI in any
  dependency field, `optdepends` included.
- **Depends on `add-dependency-check`.** That separate change adds `proton-sync doctor`. The first
  tag, `v0.1.0-beta.1`, is not pushed until it is merged, and the install smoke test then checks
  that `doctor` in a clean container without `proton-drive` reports it missing, prints Proton's
  download URL and exits non-zero.

## Capabilities

### New Capabilities

- `release-versioning`: the single upstream version, how it maps to each package format and to
  the git tag, and the checks that keep every version site in agreement.
- `linux-packages`: which native packages are built, from which trees, for which distro
  releases, what each installs, and the checks that prove a package installs and runs.
- `release-channels`: how a tag becomes a draft release, how publishing it reaches COPR, the AUR
  and the apt repository, which channel gets which release, and what users are told to run.

### Modified Capabilities

<!-- None. The only existing capability is local-deletion-disposal; nothing here changes how the
     engine disposes of local files. -->

## Impact

- **New workflows**: a reusable package-build workflow, a tag workflow that creates the draft
  release, and a publish workflow. `ci.yml` gains a `workflow_call` trigger so the tag workflow
  runs the same suite, not a copy.
- **Packaging trees**: `packaging/rpm/proton-drive-sync-engine.spec`, `packaging/arch/PKGBUILD`,
  `packaging/arch/proton-drive-sync-engine.install` (the install note),
  `packaging/debian/{control,changelog,rules}`, their READMEs, and the AppStream metainfo
  `<release>` entry. The spec's `%description`, the control file's `Description` and the
  PKGBUILD's `pkgdesc` and comment state the CLI prerequisite. New: `packaging/release-notes.md`.
- **Version sites**: `Cargo.toml`, `gui/src-tauri/Cargo.toml`, `gui/gui-core/Cargo.toml`,
  `Cargo.lock`, `gui/src-tauri/tauri.conf.json`, `gui/package.json`, `gui/package-lock.json`,
  the spec, the PKGBUILD, `debian/changelog`, the metainfo, and
  `examples/packaging/release-assets.toml`.
- **Binaries**: `proton-syncd` and `proton-sync` gain a `--version` flag. No engine behaviour
  changes.
- **New external accounts and secrets**: a COPR project and API token, an AUR account and SSH
  key, a GPG signing key, and a second GitHub repository for the apt site with a deploy key. All
  secrets live in one GitHub environment that requires a reviewer.
- **Docs**: `README.md`, `website/src/content/docs/start/installation.md`,
  `website/src/content/docs/distribution/packages.md`, and a maintainer runbook for releasing
  and renewing credentials. Pages written here say `proton-drive auth login`, never
  `proton-drive login`.
- **Owned by `add-dependency-check`, not planned here**: the `proton-sync doctor` command, the same
  check in the desktop app's first-run screen and in `setup.sh`, the daemon's boot-time log of a
  missing dependency, Proton's link on the app's "CLI missing" screen, and the fix of the wrong
  `proton-drive login` command in the daemon, the control CLI, the desktop app, `setup.sh` and the
  two desktop docs pages.
- **Not affected**: `setup.sh`, `upgrade.sh` and `uninstall.sh` keep working as the source-build
  path; this change edits only the Rust version in one `setup.sh` error message. The Tauri
  bundler stays off.
