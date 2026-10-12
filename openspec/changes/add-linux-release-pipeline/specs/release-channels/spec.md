## Purpose

Defines how a version tag becomes a draft GitHub release, how a person publishing that draft
delivers it to Fedora COPR, the AUR and the project's apt repository, which channel receives which
release, and what install instructions users are given for each channel.

## ADDED Requirements

### Requirement: A version tag creates a draft release

Pushing a tag that names a valid upstream version SHALL, when every gate passes (the version checks,
the full CI suite, the three package builds and every install smoke test), create a draft GitHub
release for that tag. The draft SHALL be marked pre-release exactly when the version has a
pre-release part. Pushing a tag SHALL NOT change any distribution channel.

#### Scenario: A beta tag produces a draft pre-release

- **WHEN** tag `v0.1.0-beta.1` is pushed and every gate passes
- **THEN** a draft release for `v0.1.0-beta.1` exists, marked pre-release
- **AND** it holds the source tarball, the source RPM, the three `.rpm` files, the three `.deb`
  files, the `.pkg.tar.zst` and `SHA256SUMS`, and nothing else
- **AND** COPR, the AUR and the apt repository are unchanged

#### Scenario: A final tag produces a draft full release

- **WHEN** tag `v0.1.0` is pushed and every gate passes
- **THEN** the draft release for `v0.1.0` is not marked pre-release

#### Scenario: A failed gate creates nothing

- **WHEN** any test or package build fails for a pushed tag
- **THEN** no draft release exists for that tag and no asset is uploaded

### Requirement: Release assets are named by the upstream version and checksummed

Every package and tarball asset filename SHALL contain the upstream version, and no asset filename
SHALL contain `~`. The checksum asset, named `SHA256SUMS`, SHALL list every other asset of the
release by its exact stored name, in the format `sha256sum -c` reads.

#### Scenario: Verifying a download

- **WHEN** a user downloads all assets of a release into one directory and runs
  `sha256sum -c SHA256SUMS`
- **THEN** every line reports `OK` and no asset is missing from the file

#### Scenario: Names carry the upstream version

- **WHEN** the assets of `v0.1.0-beta.1` are listed
- **THEN** every asset except `SHA256SUMS` has `0.1.0-beta.1` in its name
- **AND** no asset name contains `~`

### Requirement: Re-running a tag never alters a published release

Running the tag workflow again for a tag whose release is still a draft SHALL replace that draft's
assets. Running it for a tag whose release is already published SHALL fail without changing the
release or its assets.

#### Scenario: Rebuilding a draft

- **WHEN** the tag workflow is re-run for `v0.1.0-beta.1` while its release is a draft
- **THEN** the draft holds one set of assets, all from the new run, and `SHA256SUMS` matches them

#### Scenario: Rebuilding a published release is refused

- **WHEN** the tag workflow is re-run for a tag whose release is published
- **THEN** the run fails with a message saying the release is already published
- **AND** the release's assets are unchanged

### Requirement: Only publishing a release updates the channels

COPR, the AUR and the apt repository SHALL be updated only by the publish workflow, which runs when
a person publishes a draft release or a maintainer re-runs it for an already-published tag. The
credentials those updates need SHALL be readable only by jobs in a GitHub environment that requires
a named reviewer's approval.

#### Scenario: Publishing the beta draft

- **WHEN** a person publishes the draft release `v0.1.0-beta.1` and the environment approval is
  given
- **THEN** the beta COPR project, the beta AUR package and the apt `beta` suite each offer
  `0.1.0-beta.1` in their own version form

#### Scenario: A draft left unpublished reaches no one

- **WHEN** a draft release exists and nobody publishes it
- **THEN** no channel offers its version

### Requirement: Pre-releases go to beta channels, final releases to both

A pre-release SHALL be published to the beta channels only. A final release SHALL be published to
both the beta and the stable channels, so a beta user is upgraded by a normal update. The beta
channels are the COPR project `proton-drive-sync-engine-beta`, the AUR package
`proton-drive-sync-engine-beta` and the apt suite `beta`; the stable channels drop `-beta` and use
the suite `stable`.

#### Scenario: The first beta creates no stable channel

- **WHEN** `v0.1.0-beta.1` is published
- **THEN** the stable COPR project, the stable AUR package and the apt `stable` suite do not offer it

#### Scenario: A final release reaches beta users

- **WHEN** `v0.1.0` is published after `v0.1.0-beta.1`
- **THEN** a system following any beta channel is offered `0.1.0` as an upgrade
- **AND** the stable channels offer `0.1.0`

#### Scenario: The AUR beta package replaces the stable one

- **WHEN** a user installs `proton-drive-sync-engine-beta` on a system that has
  `proton-drive-sync-engine`
- **THEN** pacman reports the two as conflicting and offers to remove `proton-drive-sync-engine`
- **AND** packages that depend on `proton-drive-sync-engine` stay satisfied, because the beta
  package provides that name

### Requirement: Every credential is checked before any channel changes

Before changing any channel, the publish workflow SHALL check every credential it needs: the COPR
token, the AUR SSH key, the signing key and write access to the apt repository. If any check fails,
it SHALL change no channel and SHALL name the credential that failed and how to renew it.

#### Scenario: An expired COPR token stops the publish early

- **WHEN** the COPR API token has expired and a release is published
- **THEN** the publish workflow fails before touching COPR, the AUR or the apt repository
- **AND** its message names the COPR token and the page where a new one is generated

### Requirement: A publish can be repeated per channel

Running the publish workflow again for the same tag SHALL leave each channel offering that version
once, and SHALL complete any channel that failed in the earlier run.

#### Scenario: Finishing a partial publish

- **WHEN** a publish updated COPR and the apt repository but the AUR push failed, and a maintainer
  re-runs the publish for the same tag
- **THEN** the AUR package offers the version
- **AND** COPR and the apt repository each offer it once, with no duplicate entry

### Requirement: The COPR channel builds every supported Fedora release

The publish SHALL build the release's source RPM in the matching COPR project for every Fedora
release that is not end-of-life, with network access enabled during the build. The source RPM
SHALL contain the release tarball whose checksum is in `SHA256SUMS`.

#### Scenario: Installing the beta on Fedora

- **WHEN** a user on a supported Fedora release runs
  `sudo dnf copr enable osirison/proton-drive-sync-engine-beta` and then
  `sudo dnf install proton-drive-sync-engine`
- **THEN** version `0.1.0~beta.1` is installed

### Requirement: The AUR channel builds from the release tarball

The published AUR package SHALL contain a `PKGBUILD` whose source is the release tarball's URL with
its SHA-256 from `SHA256SUMS`, the `.SRCINFO` generated from that `PKGBUILD`, and the install
script it names. Its build steps SHALL be those of `packaging/arch/PKGBUILD`.

#### Scenario: An AUR build verifies the tarball

- **WHEN** a user builds `proton-drive-sync-engine-beta` from the AUR with `makepkg`
- **THEN** makepkg downloads `proton-drive-sync-engine-0.1.0-beta.1.tar.gz` from the GitHub release
  and its checksum check passes

#### Scenario: Metadata matches the PKGBUILD

- **WHEN** `makepkg --printsrcinfo` runs on the published `PKGBUILD`
- **THEN** its output is identical to the published `.SRCINFO`

### Requirement: The apt channel serves the tested packages, signed

The apt repository at `https://osirison.github.io/proton-drive-sync-engine-apt/` SHALL serve amd64
binary packages in suites `beta` and `stable`, component `main`, with signed `InRelease` and
`Release.gpg`. Each `.deb` it serves SHALL be byte-identical to the release asset of that name.
It SHALL publish its public key at a fixed URL and serve no source packages.

#### Scenario: Adding the repository on Ubuntu

- **WHEN** a user on Ubuntu 24.04 stores the published key in
  `/etc/apt/keyrings/proton-drive-sync-engine.gpg`, adds a source for the `beta` suite with
  `Signed-By` naming that file, and runs `sudo apt update`
- **THEN** apt reports no signature error or warning for the repository
- **AND** `sudo apt install proton-drive-sync-engine` installs `0.1.0~beta.1-1`

#### Scenario: A tampered index is refused

- **WHEN** the repository's `Release` file differs from what the signature covers
- **THEN** `apt update` refuses the repository's index

#### Scenario: Served bytes equal released bytes

- **WHEN** a `.deb` is downloaded from the repository's pool
- **THEN** its SHA-256 equals the line for that file in the release's `SHA256SUMS`

### Requirement: Install instructions match the channels

`README.md` and the docs site's installation and packages pages SHALL give, for each distribution
family, the exact commands for every channel that holds a release, and no commands for a channel
that holds none. Their first install step, above every channel's commands, SHALL be installing
Proton's `proton-drive` CLI from Proton's own pages and running `proton-drive auth login`. They
SHALL link the apt repository and key by full `https://` URLs.

#### Scenario: The CLI comes first on every install page

- **WHEN** a person reads `README.md`, the installation page or the packages page from the top
- **THEN** the first install step says to install the `proton-drive` CLI from
  `https://proton.me/drive/download` and run `proton-drive auth login`
- **AND** that step comes before every `dnf`, `apt`, AUR and source-build command on the page
- **AND** the step links no page other than Proton's own

#### Scenario: Following the Debian instructions works

- **WHEN** a person follows the Debian/Ubuntu instructions on the packages page on a clean
  Debian 12 system after `v0.1.0-beta.1` is published
- **THEN** `proton-drive-sync-engine` `0.1.0~beta.1-1` is installed

#### Scenario: No stable instructions before a stable release

- **WHEN** only pre-releases have been published
- **THEN** the docs show the beta channel commands and no stable channel commands
