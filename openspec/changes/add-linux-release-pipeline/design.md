## Context

See `proposal.md` for why. This section records the current state the approach depends on.
"Verified" means a command was run on 2026-10-12; "to verify" means it was not.

**Packaging trees.** All three exist and none is built by CI.

- **RPM spec:** built once by hand, per its own header.
  - `Source0` is GitHub's tag archive for `v%{version}`.
  - `%autosetup -n %{name}-v%{version}`.
- **Deb tree:** has never been built (its README says so).
  - `3.0 (native)` format.
  - Changelog version `0.1.0-1`.
  - `cargo (>= 1.85~)` and `rustc (>= 1.85~)` in `Build-Depends`.
  - `dh_installsystemduser` runs without `--no-enable`. Its man page says this enables the user
    unit on install. Neither the RPM (systemd presets) nor the Arch package does this.
- **PKGBUILD:** has never been built.
  - `source=` is `git+…#tag=v${pkgver}` with `SKIP`.
  - `cd "${pkgname}"`.
  - No `.SRCINFO`.

**Versions.** `0.1.0` is written in 16 places (verified with `git grep`):

- `Cargo.toml` ×3
- `Cargo.lock` ×3 workspace entries
- `tauri.conf.json`
- `gui/package.json`
- `gui/package-lock.json` ×2
- the spec's `Version` and `%changelog`
- `PKGBUILD`
- `debian/changelog`
- the metainfo `<release>`
- `release-assets.toml`

`website/package.json` also says `0.1.0`. That is the docs site's own package version, not the
product's.

**Version ordering.** Each line below was verified with the real tool for that format.

| Tool | Result |
| --- | --- |
| RPM (`rpm.labelCompare`) | `0.1.0~beta.1` < `0.1.0`; `~beta.9` < `~beta.10` < `~rc.1` |
| dpkg 1.22.21 | `0.1.0~beta.1-1` < `0.1.0-1`; same order for 9/10/rc |
| pacman 7.1.0 `vercmp` | `0.1.0beta1` < `0.1.0`; `beta9` < `beta10` < `rc1`; `alpha1` < `beta1` |
| pacman 7.1.0 `vercmp` | `0.1.0.beta1`, `0.1.0_beta1` and `0.1.0~beta.1` all sort **above** `0.1.0` |
| AppStream 1.0.5 | `0.1.0~beta.1` << `0.1.0`; `0.1.0-beta.1` >> `0.1.0` |

**Deb source format.** Verified in Debian 13 with dpkg 1.22.22:

- `dpkg-source -b` refuses native format with a revision: *"native package version may not have a
  revision"*.
- A binary-only `dpkg-buildpackage -b` with the same version succeeds.
- `--build-profiles=pkg.X.rustup` makes `dpkg-checkbuilddeps` skip the profiled entries and still
  report the others.
- The short form `-P` followed by a space and then the name did **not** take effect.

**Rust floor.** The real floor is 1.90, not the 1.85 the docs and `debian/control` state.
`cargo metadata` over the locked Linux dependency graph shows `tauri-runtime`, `tauri-utils`,
`tauri-runtime-wry`, `tauri-plugin-fs` and `tray-icon` all declare `rust-version = 1.90`.

Packaged `rustc` per distro (packages.ubuntu.com / packages.debian.org):

| Release | `rustc` |
| --- | --- |
| Ubuntu 22.04 | 1.75 |
| Ubuntu 24.04 | 1.75 |
| Ubuntu 26.04 | 1.93 |
| Debian 12 | 1.63 |
| Debian 13 | 1.85.1 (1.95 in backports) |

**Library floors and availability.** Floors come from the locked sys crates. `wry 0.57` enables
`webkit2gtk` feature `v2_38`.

| Package | Floor | Ubuntu 22.04 | Debian 12 | Ubuntu 24.04 | Debian 13 |
| --- | --- | --- | --- | --- | --- |
| `webkit2gtk-4.1` | ≥ 2.38 | 2.50.4 | 2.50.6 | 2.52.6 | 2.52.6 |
| `gtk-layer-shell-0` | ≥ 0.6 | 0.7.0 | 0.8.0 | 0.8.2 | 0.9.0 |

- `libsoup-3.0` needs ≥ 3.0.
- `gtk+-3.0` needs ≥ 3.22.

**Hosting facts** (verified with `gh api`):

- The repository is public.
- `osirison` is a user account with no `osirison.github.io` site, so there is no custom domain.
- Pages for this repository is deployed by `docs.yml` through `actions/deploy-pages`.
  - URL: `https://osirison.github.io/proton-drive-sync-engine/`.
  - Astro `base` is `/proton-drive-sync-engine`.
  - It deploys only on pushes to `main` that touch `website/**`.
  - Each deploy replaces the whole site.
- `website/astro.config.mjs` adds the base to every root-relative `href="/…"` in Markdown.
- Existing environments are `copilot` and `github-pages`.
- No tags exist.

**Other facts.**

- Neither CLI declares clap `version`, so `--version` does not exist today. `main` in both
  binaries calls `Cli::parse()` before doing anything else.
- `git archive HEAD | gzip -n` is 4.9 MB.
- The COPR API token is valid for 180 days. Its API page and docs say so, and there is no
  `copr-cli` renew command.
- The AUR RPC returns no package named `proton-drive-sync-engine` or
  `proton-drive-sync-engine-beta`, so both names are free.

**The `proton-drive` CLI.** Read on 2026-10-12 from Proton's support page
(https://proton.me/support/drive-cli) and the CLI README in `ProtonDriveApps/sdk`; the AUR
package list from the AUR RPC on the same day.

- Proton publishes it only as a download from proton.me:
  - the CLI section of `https://proton.me/drive/download`;
  - direct: `https://proton.me/download/drive/cli/index.html`;
  - CPUs without AVX2 use the `linux/x64-baseline` build.
- Its documentation is `https://proton.me/support/drive-cli` and
  `https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md`.
- Sign-in is `proton-drive auth login`, which opens a browser. `proton-drive version` prints its
  version.
- By default it keeps credentials in the OS secret store, service `ch.proton.drive/drive-sdk-cli`.
  `PROTON_DRIVE_CREDENTIALS_STORE=pass` or `unsafe_file` stores them elsewhere, where the daemon's
  keyring reuse cannot read them.
- It is in no distribution's official repositories. The AUR has community repackages
  (`proton-drive-cli`, `proton-drive-cli-git`, `proton-drive-cli-bin`). They are not Proton's.

**What the repository says about the CLI today** (verified with `git grep`):

- The PKGBUILD comment says the CLI is not in the AUR. That is out of date.
- The RPM `%description` says it is not in the Fedora or COPR repositories. The deb `Description`
  calls it proprietary. The packages page says it "isn't in any distro repo", and the installation
  page says it is not in Linux distribution repositories. None of them links Proton's pages.
- `proton-drive login`, which is the wrong command, appears in `src/daemon.rs`,
  `src/bin/proton-sync.rs`, `setup.sh`, `gui/src/js/screens/main.js`, `gui/src/js/ui/copy.js`,
  and the docs pages `desktop/overview.md` and `desktop/screens.md`. `tests/ipc_cli.rs` asserts it.
  `add-dependency-check` fixes these (D15). Older design notes under `docs/` also contain it.
- The AppStream metainfo and the desktop entry do not mention the CLI.

## Goals / Non-Goals

**Goals:**

- One build definition per distribution family. The pull-request gate, the tag build and the
  published package all use it.
- Nothing reaches a user without a person publishing the draft.
- Every channel can be rebuilt from the release's own assets.
- Adding a build or runtime dependency touches the same files it touches today. The pipeline
  adds no new dependency list.

**Non-Goals:**

- aarch64 or any architecture other than x86_64.
- Flatpak, Snap or AppImage.
- Debian source packages, a Launchpad PPA, or submission to official distribution repositories.
- Signing the RPMs attached to the GitHub release (COPR signs its own), or signing
  `SHA256SUMS`. Uploading a signature after publishing would also conflict with GitHub's
  immutable releases.
- A `CHANGELOG` file. Release notes are GitHub's generated notes plus a fixed header.
- Shipping the binary tarball from `examples/packaging/build-release-archive.sh` as a release
  asset. Its manifest stays a version site only.
- Reproducible builds, vendored crates and offline builds.
- Any change to what `setup.sh`, `upgrade.sh` or `uninstall.sh` do. The one edit is the Rust
  version in `setup.sh`'s error message (D14).
- The dependency check and the `proton-drive auth login` fixes in code, in `setup.sh` and in the
  desktop docs pages. They belong to the separate change `add-dependency-check` (D15).
- Naming, recommending or packaging any build of the `proton-drive` CLI other than Proton's own
  download.

## Decisions

### D1. Build from the existing trees, each in its own distribution's container

Each package is built by its native tool in its own container:

| Package | Container | Command |
| --- | --- | --- |
| RPM | `fedora:<oldest non-EOL>` | `rpmbuild -ba` on the spec, build deps from `dnf builddep` |
| Deb | `ubuntu:22.04` | `dpkg-buildpackage -b` on `packaging/debian` linked as `./debian`, build deps from `apt-get build-dep` |
| Arch | `archlinux:base-devel` | `makepkg -s` as an unprivileged user, since makepkg refuses root |

Build dependencies are read from the trees by each distribution's own tool. So the CI comment's
count of dependency lists that must agree stays at thirteen.

Alternatives rejected:

- **`cargo-deb`, `cargo-generate-rpm`, `nfpm`:** each carries its own dependency list (a 14th)
  and its own idea of the file layout, beside trees that already define both.
- **The Tauri bundler:** it is deliberately off (`bundle.active: false`). Its deb/rpm/AppImage
  output would be a second definition of the same packages.
- **OBS or Launchpad:** their builders have no network. The spec and `debian/rules` run
  `cargo build` against crates.io, so this would mean vendoring hundreds of crates.
- **Packit:** a GitHub App with write access that still builds from the same spec. It adds an
  account and a config format and removes nothing.

### D2. One reusable package workflow, called by the pull-request gate and the tag workflow

`packages.yml` has three triggers:

- `workflow_call`
- `pull_request`
- a weekly `schedule`, to catch a distribution update that breaks the build between releases

Its jobs:

1. Make the source tarball.
2. Build the three packages from that tarball.
3. Run the install smoke tests (D13) over the supported-release matrix.

The tag workflow calls it. It also calls `ci.yml`, which gains a `workflow_call` trigger, so the
release runs the same suite rather than a copy of it.

**Pull-request filter.** The workflow always runs. A first job compares the pull request's
changed files with the filter in the `linux-packages` spec, and the build and smoke jobs carry an
`if:` on its answer. The filter is: `packaging/`, `Cargo.lock`, any `Cargo.toml`,
`gui/src-tauri/tauri.conf.json`, `packages.yml` itself, and the files outside `packaging/` that the
three trees install or read (`LICENSE`, `README.md`, `examples/proton-sync.toml`,
`gui/src/fonts/OFL-*.txt`; found by reading the spec's `%license`/`%doc`, the PKGBUILD's `install`
lines and the debian `copyright`). It leaves out `src/` and the GUI sources on purpose: the CI
suite builds them, and a package build per source change would cost far more than it finds.

**The required check is one aggregator job, `packages-ok`.** It has `needs:` on every build and
smoke job and `if: always()`. It fails if any of them failed or was cancelled, and passes when
they succeeded or were skipped. Only `packages-ok` is made a required check. Requiring the matrix
jobs directly would not work: a matrix job skipped by `if:` reports under its unexpanded name, so
the per-cell checks a branch rule names would never report. That is the same "waiting for status
to be reported" failure `ci.yml`'s comments describe for path-filtered workflows.

Alternative rejected: separate build scripts for pull requests and tags. Two copies drift, and
the gate would test something other than what ships.

### D3. One mapping, written once, in a version script

`packaging/version.sh` has two modes:

- `set <upstream>` writes every site.
- `check` reads `Cargo.toml` and compares every site.

The mapping exists only in this script.

| Site | Value for `0.1.0-beta.1` |
| --- | --- |
| Tag | `v0.1.0-beta.1` |
| 3× `Cargo.toml`, 3 `Cargo.lock` entries, `tauri.conf.json`, `gui/package.json`, 2 `package-lock.json` entries | `0.1.0-beta.1` |
| Spec `%global upstream_version` / `Version` / newest `%changelog` | `0.1.0-beta.1` / `0.1.0~beta.1` / `0.1.0~beta.1-1` |
| PKGBUILD `_upstream_ver` / `pkgver` / `pkgrel` | `0.1.0-beta.1` / `0.1.0beta1` / `1` |
| `debian/changelog` newest entry | `0.1.0~beta.1-1` |
| metainfo newest `<release>` | `version="0.1.0~beta.1" type="development"` |
| `release-assets.toml` `default_archive` | `…-0.1.0-beta.1.tar.gz` |

**How `set` writes the files.**

- `Cargo.lock` and `gui/package-lock.json` are regenerated by cargo and npm, which own those
  files, not edited by `sed`. Otherwise the next `--locked` build refuses the stale lockfile.
- The changelog entries are added, never rewritten.

**Where `check` runs.**

- As a step in CI.
- From `tests/scripts.rs` through `common::run_other_tool`, so `cargo test` also catches drift.
  That test reads files and starts no crate binary.
- A second `tests/scripts.rs` test runs `set` on a temporary copy of the tree. It compares the
  results against literal examples, which are not a second mapping.

**Tag gate.** The release workflow's first job:

- refuses a tag outside `^v\d+\.\d+\.\d+(-(alpha|beta|rc)\.\d+)?$`;
- refuses a tag that does not equal `v` plus the `Cargo.toml` version;
- refuses a tag whose commit is not an ancestor of `origin/main`;
- runs `check`.

The pattern is written as a regex here; the shell form may differ.

Alternatives rejected:

- **Deriving package versions at build time with `sed` in the workflow:** COPR, the AUR and
  anyone building a tree by hand read the literal version in the file. A version that exists
  only in CI is wrong everywhere else.
- **A Rust test that holds its own copy of the mapping beside the script:** that is two
  definitions of one rule.

### D4. Each tree separates its package version from the upstream version

`~` cannot appear in the tag, and Arch cannot use `-`. Today all three trees compute the tag from
their package version, so all three break at `0.1.0-beta.1`:

- `Source0 … v%{version}` would become `v0.1.0~beta.1`.
- `#tag=v$pkgver` would become `v0.1.0beta1`.

**RPM.**

- `Source0: %{url}/releases/download/v%{upstream_version}/%{name}-%{upstream_version}.tar.gz`.
- `%autosetup -n %{name}-%{upstream_version}`.
- `Version` stays a literal. `check` holds the two together.

**PKGBUILD.**

- `source=("${_pkgbase}-${_upstream_ver}.tar.gz::${url}/releases/download/v${_upstream_ver}/${_pkgbase}-${_upstream_ver}.tar.gz")`.
- Every `cd` uses `${_pkgbase}-${_upstream_ver}`.
- `install=` uses `_pkgbase`, so the beta variant (D8) changes only `pkgname`, `provides` and
  `conflicts`.
- The repository copy keeps `sha256sums=('SKIP')`. The tarball contains the PKGBUILD, so its own
  checksum cannot be written into it. The AUR copy gets the real sum (D8).

**Deb.** The version lives only in `debian/changelog`. The format stays `3.0 (native)` and the
build is binary-only, so there is no orig tarball whose name could break. This is also why no
source package is published.

Alternatives rejected:

- **Switching to `3.0 (quilt)` with the release tarball renamed `…_0.1.0~beta.1.orig.tar.gz`:**
  it is only needed to publish source packages, which is a non-goal. Revisit it with a PPA.
- **Dropping the `-1` revision:** it was decided, and it is what allows a packaging-only rebuild
  (`-2`).

### D5. The source tarball is made by `git archive`, once, and attached

The tarball is made with:

```sh
git archive --format=tar --prefix=proton-drive-sync-engine-<upstream>/ <tag> | gzip -n
```

That makes `proton-drive-sync-engine-<upstream>.tar.gz`. It is built once per run and passed to
the three build jobs as an artifact. It is attached to the release and is the URL `Source0`, the
AUR `source=` and the COPR source RPM all point at.

Alternative rejected: GitHub's automatic tag archives. GitHub chooses their name and top
directory, and their checksum changed once in 2023, which broke every packager that pinned it.

### D6. The deb is built once on Ubuntu 22.04 and serves five releases

Ubuntu 22.04 has the oldest glibc (2.35) of the supported Debian/Ubuntu releases and has every
build dependency at its floor (Context table). So one `.deb` built there can install on:

| Release | glibc |
| --- | --- |
| Ubuntu 22.04 | 2.35 |
| Ubuntu 24.04 | 2.39 |
| Ubuntu 26.04 | to verify |
| Debian 12 | 2.36 |
| Debian 13 | 2.41 |

**Rust toolchain.**

- The `cargo` and `rustc` entries in `debian/control` get the build-profile restriction
  `<!pkg.proton-drive-sync-engine.rustup>`, and their floor rises to `1.90~`.
- The release build installs Rust with rustup (minimal profile, `stable`). It then runs
  `apt-get build-dep` and `dpkg-buildpackage -b -us -uc` with
  `--build-profiles=pkg.proton-drive-sync-engine.rustup`.
- Every other `Build-Depends` entry is still checked.
- Without the profile, the control file still describes a distribution build, which Ubuntu 26.04
  or Debian 13 backports could satisfy.

Alternatives rejected:

- **`dpkg-buildpackage -d`:** it switches off every build-dependency check, not just Rust's.
- **One build per release:** five builds and five suites to serve one set of binaries.
- **Building on `ubuntu-latest`:** its glibc rules out 22.04 and Debian 12.

**The t64 renames: settled by the smoke test (2026-10-13), no second build needed.** Ubuntu 24.04+
and Debian 13 renamed libraries such as `libglib2.0-0` to `libglib2.0-0t64`, and the jammy-built
package depends on the old names. The jammy-built `.deb` has twelve `Depends` entries
(`libc6`, `libcairo2`, `libdbus-1-3`, `libgcc-s1`, `libgdk-pixbuf-2.0-0`, `libglib2.0-0`,
`libgtk-3-0`, `libgtk-layer-shell0`, `libjavascriptcoregtk-4.1-0`, `libsoup-3.0-0`,
`libwebkit2gtk-4.1-0`, `init-system-helpers`). Installed with `apt-get install ./…deb` in clean
containers of Ubuntu 22.04, 24.04 and 26.04 and Debian 12 and 13, it installed on all five, and in
each, checked after the install with `dpkg-query` (installed status of the exact name, else the
installed packages whose `Provides` list it): ten names were installed packages under their own
name on every release, and two, `libglib2.0-0` and `libgtk-3-0`, were satisfied **only through a
versioned `Provides`** on Ubuntu 24.04 and 26.04 and Debian 13 (the installed packages are
`libglib2.0-0t64` and `libgtk-3-0t64`, e.g. `Provides: libglib2.0-0 (= 2.80.0-6ubuntu3.9)` on
24.04). On Ubuntu 22.04 and Debian 12 all twelve are direct. A versioned `Provides` satisfies the
versioned dependency (`libglib2.0-0 (>= 2.65.1)`), which is why `apt` resolved the jammy-built
names to the renamed packages and installed. `ldd /usr/bin/proton-sync-gui` found every library. The fallback below is not needed for this
package; the `smoke-deb` matrix keeps checking it on every run.

**Fallback if they fail.** Add a second build on `ubuntu:24.04` for 24.04, 26.04 and Debian 13,
and give the apt repository one suite per base. That changes the "one `.deb`" scenario in the
`linux-packages` spec, so it would need a spec update.

### D7. RPM: a GitHub asset from the oldest Fedora, and COPR builds of the same source RPM

**The GitHub asset.** The release job runs `rpmbuild -ba` in `fedora:<oldest non-EOL>`. That
gives the source RPM and the three RPMs. The container tag is one workflow variable. On
2026-10-12 it is `43`; Fedora 45 is due 2026-10-20 and 43 reaches end of life about four weeks
later.

**COPR.** The publish job builds the release's source RPM by URL:
`copr-cli build --nowait … <project> <srpm-url>`.

- It then waits with `copr-cli watch-build` on the returned build IDs and fails if any chroot
  fails. A Tauri build takes tens of minutes, so without the wait the verification job would
  run `dnf install` before the build exists.
- It builds in every chroot of a non-EOL Fedora release.
- The project is created once with `--enable-net on`, because `%build` runs
  `cargo build --locked` against crates.io.
- `%check` stays off. The tests already ran in CI.
- COPR builds with each Fedora release's own `rust` package, so a Fedora `rust` older than 1.90
  would fail there. Fedora updates `rust` in stable releases, so this is not expected (to verify
  at the first publish).

Alternatives rejected:

- **COPR SCM or custom-webhook builds:** they need no token, but they build from git, not from
  the source RPM whose tarball is in `SHA256SUMS`.
- **Packit:** see D1.

### D8. Arch: AUR source packages, derived from the one PKGBUILD

**The GitHub asset.** The `.pkg.tar.zst` is built in `archlinux:base-devel` from the release
tarball placed beside the PKGBUILD, so makepkg does not download it. It is attached for
reference and manual install.

**The AUR channel.** Arch is rolling. A binary built today breaks at the next library soname bump
and nothing rebuilds it, so the supported channel is the AUR **source** package, built on the
user's machine.

**`packaging/arch/aur.sh <channel> <assets-dir>`** writes the AUR tree. It:

1. copies the PKGBUILD and the install script;
2. for `beta`, sets `pkgname=proton-drive-sync-engine-beta` and adds
   `provides=(proton-drive-sync-engine)` and `conflicts=(proton-drive-sync-engine)`;
3. writes the tarball's sum from `SHA256SUMS` into `sha256sums`;
4. generates `.SRCINFO` with `makepkg --printsrcinfo` in the Arch container.

The pull-request gate runs it on the built tarball, so the publish path is exercised before any
release.

Alternatives rejected:

- **A `-bin` package:** it goes stale on a rolling distribution, as above.
- **`-git`:** it tracks `main`, not releases.
- **A second, hand-kept PKGBUILD for beta:** two trees.

### D9. Beta and stable channels from the first release

Confirmed by the maintainer on 2026-10-12.

Publishing reads the release's `prerelease` flag:

- A pre-release goes to the beta channels.
- A final release goes to the beta and the stable channels.

| Family | Beta channel | Stable channel |
| --- | --- | --- |
| COPR | `osirison/proton-drive-sync-engine-beta` | `osirison/proton-drive-sync-engine` |
| AUR | `proton-drive-sync-engine-beta` | `proton-drive-sync-engine` |
| Apt | suite `beta` | suite `stable` |

The stable channels are created by the first final release. An AUR package cannot be created
empty. Until that release the docs show only the beta commands.

Alternative rejected: one channel per family that receives everything. That is simpler for
`0.1.0-beta.1`. But once `0.1.0` ships, every later beta reaches every user, and beta testers
would have to move to a new channel at that point. With two channels from the start, nobody
moves: beta users get finals in their channel.

### D10. The apt repository lives on its own repository's GitHub Pages

**Location.** `osirison/proton-drive-sync-engine-apt` is served from its `gh-pages` branch:

- URL: `https://osirison.github.io/proton-drive-sync-engine-apt/`.
- The branch holds a `.nojekyll` file, so Pages serves files as they are and does not run Jekyll
  on them.

**How each publish rebuilds the tree.**

1. Download the `.deb` assets of the newest three releases for each suite from GitHub Releases.
   Releases are the store; the apt repository keeps nothing that cannot be rebuilt.
2. Put them under `pool/main/p/proton-drive-sync-engine/`, keeping the asset names, which carry
   no `~`.
3. Generate `Packages` and `Release` for each suite with `apt-ftparchive` (`apt-utils`). It keeps
   no database between runs, unlike `reprepro` or `aptly`.
4. Sign `InRelease` and `Release.gpg`.
5. Publish the key as `proton-drive-sync-engine.gpg` (binary) and `.asc`.
6. Force-push a single-commit `gh-pages`, so old `.deb`s do not pile up in git history.

**Size.** The Pages limit is 1 GB. The tarball is 4.9 MB. The `.deb` size is to be measured, and
with three releases the total stays far below the limit.

**Docs links.** The docs link the repository and key by full `https://` URLs. A root-relative
link would be rewritten under `/proton-drive-sync-engine`.

**Users install it with** (shown with `beta`):

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://osirison.github.io/proton-drive-sync-engine-apt/proton-drive-sync-engine.gpg \
  | sudo tee /etc/apt/keyrings/proton-drive-sync-engine.gpg >/dev/null
sudo tee /etc/apt/sources.list.d/proton-drive-sync-engine.sources >/dev/null <<'EOF'
Types: deb
URIs: https://osirison.github.io/proton-drive-sync-engine-apt/
Suites: beta
Components: main
Architectures: amd64
Signed-By: /etc/apt/keyrings/proton-drive-sync-engine.gpg
EOF
sudo apt update && sudo apt install proton-drive-sync-engine
```

The docs also print the key's fingerprint.

Alternatives rejected:

- **The apt tree inside the docs Pages artifact.** A repository has one Pages site, and
  `docs.yml` replaces it on every `website/**` push. So every docs deploy would have to fetch and
  re-include the apt tree. A deploy that failed to fetch it would delete the apt repository from
  under every user, and the signed metadata or the key would come within reach of a workflow that
  runs on every docs edit.
- **Cloudsmith or packagecloud:** a third-party account and terms for a 5 MB-scale repository.
- **A Launchpad PPA:** offline builders (D1), and Ubuntu only.
- **Release assets as the pool:** apt needs one base URL with a `dists/` tree, and Pages cannot
  redirect.

### D11. Secrets live in one environment that needs a reviewer

All publish credentials are secrets of a GitHub environment named `release`, whose required
reviewer is the maintainer. That approval is the second human step after publishing the draft.
The approval is a click on a run, not a review of the code that run executes: for a solo maintainer
it is a self-click, so it does not by itself stop a `workflow_dispatch` of a workflow edited on a
branch from reading the keys. What does is the environment's **deployment branches and tags**
setting, limited to `main` and tags matching `v*` (task 5.5); the file that runs is then main's or a
tag's. The publish job also checks out the tag's commit and runs the release gate on it before it
writes a secret (`tag-checkout.sh`), so a dispatch from any ref publishes the tag's own scripts and
packaging, and a tag made by hand on a commit that is not on `main` publishes nothing.

| Secret | Purpose | Lifetime |
| --- | --- | --- |
| `COPR_CONFIG` | `~/.config/copr` contents (API token) | **180 days**; repo variable `COPR_TOKEN_EXPIRES` lets preflight fail with a renewal message, and warn in the tag workflow 14 days ahead |
| `AUR_SSH_KEY` | key registered on the AUR account | no expiry |
| `APT_SIGNING_KEY` | armored private Ed25519 key used only for this repository | no expiry, deliberately (below) |
| `APT_DEPLOY_KEY` | SSH deploy key with write access to the apt repository only | no expiry |

`GITHUB_TOKEN` cannot push to another repository, which is why the apt deploy key exists.

**Key algorithm: Ed25519, signing over SHA-512.**

- Newer apt verifies with `sqv`, which rejects SHA-1 and weak RSA.
- GnuPG 2.2 on Ubuntu 22.04 verifies Ed25519.
- The live-apt verification runs on 22.04 (oldest apt) and 26.04 (newest), so both ends are
  checked.
- RSA 4096 is the fallback if a supported release refuses the key.

**Why the signing key has no expiry.** Users hold the key as a file named by `Signed-By`. If the
key expired, apt would fail for every user until each one downloaded the key again. Rotation is
manual instead: publish a new key, sign with both keys for one release, then drop the old one.
This is documented in the runbook.

### D12. Workflows and how they are triggered

**`release.yml`** runs on a pushed `v*` tag. Its jobs:

1. **gate:** D3.
2. **ci:** calls `ci.yml`.
3. **packages:** calls `packages.yml`.
4. **draft:**
   - writes `SHA256SUMS`;
   - creates the draft with `gh release create --draft --verify-tag --generate-notes`, adding
     `--prerelease` when the version has a pre-release part, and the notes header from
     `packaging/release-notes.md`. The header links to the docs packages page rather than
     repeating the install commands, so there is no third copy of them to drift. It also states
     the CLI prerequisite with Proton's download link (D15);
   - if a draft already exists, uploads with `--clobber` and deletes assets the new run did not
     produce;
   - if the release is already published, fails without changes.

Only the draft job gets `contents: write`.

The gate and draft logic is in `packaging/release/gate.sh` and `draft.sh`, which `tests/scripts.rs`
runs with a stub `gh`, so the workflow only calls them. Decisions made when writing them:

- **One definition of the tag.** The tag shape, the version a tag names, "is this a pre-release" and
  the tree's version are `version.sh` subcommands (`tag-version`, `is-prerelease`, `current`). The
  workflow and the scripts hold no pattern of their own.
- **The ancestry check fails closed.** `git merge-base --is-ancestor` answers 0 (is), 1 (is not) and
  128 (a ref it cannot read), and only 0 passes. The gate job checks out the whole history and then
  fetches `main` by name, because a tag-push checkout has no `origin/main`.
- **Drafts are found through the API list**, filtered on `tag_name`, because a lookup by tag may
  not return a draft and GitHub allows two drafts on one tag. One draft is replaced; two fail naming
  both; a published release fails; all three are decided before anything is written. A per-tag
  `concurrency` group (never cancelled) stops two runs of one tag racing to make two drafts.
- **The draft holds exactly** the tarball, the source RPM, three RPMs, three debs, the Arch package
  and `SHA256SUMS`; `draft.sh` refuses any other set, a name without the upstream version and a name
  with `~`.
- **The pre-release warning is conditional.** It sits between two marker lines in
  `release-notes.md`, and `draft.sh` drops it for a final version. The notes file is passed with
  `--notes-file` beside `--generate-notes`; `gh release create --help` documents the order only for
  `--notes`, so the first real draft is looked at.
- **Verification uses test tags and test draft releases on this repository**, not a fork (the
  maintainer's decision of 2026-10-13: a user cannot fork their own repository).

**`publish.yml`** runs on `release: [published]` and on `workflow_dispatch` with inputs `tag` and
`dry_run`. The event is `published`, not `released`, because `released` does not fire for
pre-releases. It has two kinds of job.

**One `publish` job in environment `release`.** It is the only job that reads secrets, so one run
asks for one approval. An approval is given per job as each job becomes ready, so several
environment jobs started at different times would each wait for their own click. Its steps run
in order:

1. preflight;
2. apt (seconds);
3. AUR (seconds);
4. COPR, which submits and then waits on the build (tens of minutes).

Each channel step is idempotent and runs even if an earlier channel step failed. A final step
fails the job if any channel failed and names which.

**Verification jobs, outside the environment.** They read no secrets and run after `publish`,
one per channel, against the live channel:

- `dnf install` from COPR on each chroot's Fedora;
- `makepkg` of the AUR package;
- `apt install` from the live repository on Ubuntu 22.04, Debian 13 and Ubuntu 26.04.

**Which workflow file runs.** Release events run the workflow file as it is at the tagged commit.
So both workflows must be on `main` before the first tag. A fix for an existing tag reaches it
through `workflow_dispatch` from `main`, which runs `main`'s file.

**Never publish from a workflow.** A release published through `GITHUB_TOKEN` fires no workflow,
so a release published that way would silently skip publishing to the channels.

**Tag protection.** A tag ruleset limits who may create `v*` tags. A tag only makes a draft, so
this is defence in depth, not the main gate.

Decisions made when writing the `publish` job (the scripts are in `packaging/release/`, tested with
stubs like `draft.sh`; the workflow only calls them):

- **Routing reads the API and checks it against the tag.** `route.sh` takes `prerelease` from the
  published release (a draft or a missing release is an error), accepts only the JSON `true` or
  `false`, and refuses a flag that disagrees with `version.sh is-prerelease`: a beta published as a
  final release would otherwise reach the stable channels. Every failure prints no channel.
- **Credentials are files, written in one step.** Only one step reads `secrets.*`; it writes mode-600
  files in a mode-700 directory and passes their paths on. An `if: always()` step removes them.
  Nothing else in the file reads a secret, and no `run:` holds an expression, so the tag, the dry-run
  flag and the secrets reach scripts through `env:` only (the tests pin every expression line).
- **One step per channel, each recorded.** Each channel step is `if: !cancelled() && preflight
  succeeded`, not `always()`, so a person cancelling the run stops the publishing. Every step records
  `ok` or `failed` in a results directory (failed until the script reaches its end), and the final
  step `aggregate.sh` fails naming every step that is not `ok`, a missing record included.
- **The COPR namespace is one `env:` value, `COPR_OWNER`**, in `publish.yml`. The scripts read it and
  fail when it is unset. The project names are `proton-drive-sync-engine` and `-beta` from D9.
- **COPR token date:** `COPR_TOKEN_EXPIRES` is `YYYY-MM-DD` in UTC, the last day the token works.
  `copr-cli whoami` does not ask the server (it prints the user name from the config), so it only
  proves the config parses; the date is what stands for the token's age.
- **COPR chroots:** `copr-cli build` without `-r` builds in every chroot of the project, so the
  project's chroot list (task 5.1) is the list of non-EOL releases, and no list is kept in the
  workflow. `copr-cli get` has no JSON, so the list could not be read from it anyway.
- **COPR skip rule:** `list-packages --with-all-builds --output-format json` is read, and a build
  counts when its state is `succeeded` and its version before the last dash equals the RPM spelling
  of the upstream version (the release part carries a dist tag that changes). A running build is
  waited for, not repeated. The server limits the per-package build list (newest first in
  the project looked at), and `latest_build` and `latest_succeeded_build` are read too, so an old
  version may at worst be built again; a failed read fails the project.
- **AUR tree from the tarball.** The tree is made by the `aur.sh` and PKGBUILD inside the release's
  own tarball, so a dispatch for an older tag from any checkout publishes that tag's packaging. It
  runs in an `archlinux:base-devel` container with only the work directory mounted. The push is
  `HEAD:refs/heads/master` (the AUR takes master only), made only when git sees a difference.
- **The tag is checked out and gated before any secret is written.** `tag-checkout.sh` fetches
  `main` and the tag, checks out the tag's commit and runs `gate.sh` on it (tag = Cargo.toml
  version, commit on `main`, `version.sh check`). It is a `run:` step with the tag taken from
  `PUBLISH_TAG` (validated by `route.sh`), so no `with:` expression is needed and the pinned
  expression rule is unchanged. Every later step then runs the tag's scripts, whichever ref the run
  was dispatched from. A tag from before these scripts cannot be published by this workflow.
- **The tree from the container is untrusted.** The AUR step refuses a tree or an entry that is not a
  plain file or directory, and any name but the three expected, then copies without following links.
- **Pinned host keys.** `packaging/release/known_hosts` holds the Ed25519 keys of
  `aur.archlinux.org` and `github.com`; ssh runs with `StrictHostKeyChecking=yes` and no other
  known-hosts file.
- **A dry run reads and never writes.** It reads the release and each channel's current state
  (the COPR project's builds, the AUR clone) and logs what it would do; it never runs
  `copr-cli build` or `watch-build`, and never commits or pushes. A dispatch's `dry_run` defaults to
  true.
- **Known limits.** GitHub keeps one pending run per concurrency group, so a third run for a tag
  replaces the waiting second; `copr-cli` is installed from a hash-locked requirements file (Python 3.12 markers); the two
  verification jobs and the apt step (6.3, 6.6) are a later change and have marked places in the
  file.

### D13. The install smoke test

`packaging/smoke-test.sh` runs inside each clean container after the distribution's package
manager has installed the package. It checks, and does nothing else:

- `--version` output of both CLIs;
- that the unit file exists and its `ExecStart` begins with `/usr/bin/proton-syncd`;
- that `/etc/systemd/user/default.target.wants/proton-syncd.service` is absent;
- `ldd` of the GUI;
- that `curl` and `secret-tool` are on `PATH`;
- `desktop-file-validate` and `appstreamcli validate --no-net` where the tool exists (the workflow
  installs both after the package, except on Ubuntu 22.04 and Debian 12, whose versions reject
  Desktop Entry 1.5 and newer AppStream tags that the current tools accept);
- that `proton-sync doctor`, with no `proton-drive` on `PATH`, names `proton-drive` as missing,
  prints `https://proton.me/drive/download` and exits non-zero, and that no `proton-syncd`
  process and no control socket appear.

The Arch cell also keeps the output of `pacman -U` and checks it for the install note (D15).

**The doctor check comes from another change.** `proton-sync doctor` is added by
`add-dependency-check` and works without the daemon socket. If the pipeline merges first, its
pull-request gate runs without the doctor check. The check is added to
`smoke-test.sh` once `add-dependency-check` is merged, and the first tag waits for both (Migration
Plan). The test compares only the URL, the exit status and that the output names `proton-drive`,
not the wording. `add-dependency-check` must print that exact URL.

It never starts the daemon or the GUI. It asserts nothing about a keyring: the containers have no
session bus, so what `doctor` reports about one is outside the test's control. It lives outside
`tests/` and runs only in throwaway containers. The `tests/test_isolation.rs` rule is about tests
that start a binary on the developer's machine, and that situation does not arise here.

Supported-release matrix:

| Family | Containers |
| --- | --- |
| Fedora | every non-EOL release |
| Ubuntu | 22.04, 24.04, 26.04 |
| Debian | 12, 13 |
| Arch | `archlinux:latest` |

### D14. Small fixes in the trees that the first real build needs

- **Rust floor 1.85 → 1.90** in `debian/control` and in five more files that state it in prose:
  `README.md`, `setup.sh`'s error message, `packaging/debian/README.md`, and the docs site's
  installation and packages pages. Confirmed by the maintainer on 2026-10-12.
- **`dh_installsystemduser --no-enable`.** Today the deb enables the unit for every user at
  install. Every user's next login then starts the daemon against a config that may not exist,
  and systemd restarts it every 10 s. The RPM (presets) and the Arch package do not do this, and
  the docs tell users to run `systemctl --user enable --now` themselves. Confirmed by the
  maintainer on 2026-10-12.
- **`curl`** added to the deb's `Recommends` beside `libsecret-tools`, and to the PKGBUILD's
  `depends`. The RPM already `Requires` it.
- **clap `version`** on both binaries.
- **The CLI text in all three trees** (D15): the RPM `%description`, the deb `Description`, the
  PKGBUILD `pkgdesc` and header comment, and a note in the Arch `.install`.

### D15. The `proton-drive` CLI: only Proton's pages, and before this app

Decided by the maintainer on 2026-10-12.

**What every mention says.** Everything this change writes about the CLI:

- links only Proton's pages (Context), if it links anything;
- says the CLI must be installed and signed in before this app is used, and gives the sign-in
  command, where it gives one, as `proton-drive auth login`;
- where it says where the CLI comes from, says Proton publishes it only as a download from
  proton.me and that it is in no distribution's official repositories;
- names no third-party build. The PKGBUILD comment may say community repackages exist, without
  naming them.

**Where.**

| Surface | What it says |
| --- | --- |
| RPM `%description`, deb `Description` | the prerequisite, the download page and `proton-drive auth login` |
| PKGBUILD `pkgdesc` | one clause: needs Proton's `proton-drive` CLI, signed in |
| PKGBUILD header comment | the statement above, replacing "not in … the AUR" |
| Arch `.install` | a note at install (below) |
| `packaging/release-notes.md` | the prerequisite with the download link |
| `README.md`, installation page, packages page | step 1, above every channel's commands and the source build |

**Install pages.** Step 1 is: install the CLI from `https://proton.me/drive/download` (the
`linux/x64-baseline` build on a CPU without AVX2), run `proton-drive auth login`, and check it
with `proton-drive version`. Keep the default credential store: with `pass` or `unsafe_file` the
daemon cannot read the session. Docs written by this change say `proton-drive auth login`, never
`proton-drive login`.

**The Arch install note.** `post_install` prints two or three lines naming the download page and
`proton-drive auth login`, then refreshes the caches as it does today. `post_upgrade` does not
print it: a user upgrading has already set the CLI up, and pacman would show the note on every
upgrade. Today `post_install` only calls `post_upgrade`, so the note goes in `post_install` before
that call.

**RPM and deb print nothing at install.** RPM scriptlet output is discouraged, and a dpkg
maintainer script runs as root with no user session, often with nobody watching (a software
centre, unattended upgrades). The deb `postinst` stays silent. Both rely on the package
description and the docs.

**No `optdepends`.** The Arch package names no CLI package in `optdepends`. Decided by the
maintainer on 2026-10-12: only the official download is recommended, and every AUR package of the
CLI is a community repackage.

**The dependency check is another change's.** `add-dependency-check` adds `proton-sync doctor`
(it works without the daemon socket), the same check in the desktop app's first-run screen and
in `setup.sh` before it enables the service, a boot-time log from the daemon that keeps it
running, Proton's link on the app's "CLI missing" screen, and the `proton-drive auth login` fixes
(Context). This change plans none of those edits. It depends on that change in two places: the
smoke test runs `doctor` (D13), and the first tag waits for it (Migration Plan).

Alternatives rejected:

- **`optdepends` on an AUR repackage:** it points users at a third-party build of a tool that
  holds their Proton session.
- **An RPM `%post` or deb `postinst` message:** see above.

## Risks / Trade-offs

- **The deb and Arch trees have never been built, so the first real build will find defects.** →
  The pull-request gate lands before any tag, and it builds and fixes the trees in a normal pull
  request.
- **The t64 renames break the jammy-built `.deb` on 24.04+ or Debian 13.** → The smoke test
  catches it before a draft exists. The fallback is D6's second build base.
- **COPR and the AUR rebuild from source, so their bits are not the bits that were tested.** →
  This is stated in the docs. Only the GitHub assets and the apt repository carry tested bytes.
  Each channel has a post-publish verification job, and a COPR failure (for example a Fedora
  `rust` change) blocks only that channel.
- **The COPR token expires after 180 days.** → Preflight fails first, with the renewal URL, and
  the tag workflow warns 14 days ahead. The runbook covers renewal.
- **A rolling Arch update breaks the build between releases.** → The weekly scheduled run
  catches it.
- **Every Dependabot cargo pull request touches `Cargo.lock`.** Each one therefore runs three
  release builds and the whole smoke matrix. → Accepted. This is how a dependency that raises the
  Rust floor or a system library floor is caught before it reaches a release.
- **A failed half-publish leaves channels out of step.** → Each channel job is idempotent, and
  `workflow_dispatch` re-runs the publish for the tag.
- **A published bad release is hard to withdraw.** Lowering a version later needs an `epoch` in
  RPM and Arch. → Fix forward with the next pre-release number. The runbook says how to remove a
  version from each channel if it must go.
- **Network outages during builds** (crates.io, distribution mirrors). → Re-run the job. Nothing
  is published until a person publishes the draft.
- **The environment approval is a second click** after publishing the draft. → Accepted. It is
  what keeps the keys from every other run.
- **`add-dependency-check` is not merged when the pipeline is ready.** → The first tag waits. The
  pipeline merges and is proven on pull requests without the doctor check in the meantime.
- **The doctor check's output is written by another change.** → The smoke test compares only the
  download URL, the exit status and that `proton-drive` is named, so a change of wording there
  does not break it. A change of URL does, which is intended.

## Migration Plan

1. Merge the pipeline: the workflows, `version.sh`, `aur.sh`, `smoke-test.sh`, the tree fixes, the
   clap `version` flag, and the runbook. The version stays `0.1.0`, and the pull-request gate
   proves all three trees build.
2. Create the external accounts and secrets:
   - the COPR beta project (network on, chroots);
   - the AUR account and SSH key;
   - the signing key;
   - the `proton-drive-sync-engine-apt` repository with Pages from `gh-pages` and a deploy key;
   - environment `release` with a reviewer;
   - the tag ruleset.
3. Merge the bump to `0.1.0-beta.1` (`version.sh set 0.1.0-beta.1`).
4. Wait for the separate change `add-dependency-check` to merge. Then add the doctor check to
   `smoke-test.sh` (D13) and see every smoke cell pass with it on a pull request. The first tag is
   not pushed before this.
5. Push `v0.1.0-beta.1`. Review the draft and its notes. Publish. Approve the environment.
6. Check the three verification jobs. Then merge the docs (README and the two site pages), which
   now describe channels that hold a release.

**Rollback.**

- Before publishing: delete the draft and the tag.
- After publishing: remove the build from COPR, push the previous PKGBUILD to the AUR, and re-run
  the apt job with the release excluded. Then fix forward with `beta.2`.

## Open Questions

- The COPR namespace is assumed to be `osirison`, the Fedora account name. If it differs, only
  the strings in the commands and docs change. In `publish.yml` it is one value, `COPR_OWNER` in the
  workflow's `env:`; a test holds the scripts free of it.
- That the `ubuntu:26.04` container tag exists under that name, and its glibc version.
- Whether to pin the rustup toolchain used for the deb to a fixed version, rather than `stable`,
  for repeatable re-runs.
- The size of the `.deb`, used to confirm the retention of three releases per suite.
