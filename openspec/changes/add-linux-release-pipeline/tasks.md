## 1. Version reporting and the version script

- [x] 1.1 Add clap `version` to the `#[command(...)]` of `src/bin/proton-syncd.rs` and
      `src/bin/proton-sync.rs`. Add a test in `tests/` that starts both binaries through the
      `tests/common` helpers (`syncd`/`sync_cli`, `run_bounded`). `refuse_the_real_cli` refuses a
      `proton-syncd` that names no fake CLI, so either pass the sandbox's fake CLI or add a
      `--version`-only exemption pinned by its own case in `tests/test_isolation.rs`. Verify: the
      test asserts `proton-syncd <CARGO_PKG_VERSION>` and `proton-sync <CARGO_PKG_VERSION>` with
      exit 0, and that no socket, lockfile or state directory appears in the sandbox
- [x] 1.2 Write `packaging/version.sh check`. It reads the root `Cargo.toml` version, applies the
      design D3 mapping, and compares all 18 values. That is the 16 places `0.1.0` is written
      today plus the two upstream-version variables added in 2.1 and 2.2:
      - 3 manifests;
      - 3 `Cargo.lock` workspace entries;
      - `tauri.conf.json`;
      - `gui/package.json`;
      - 2 `gui/package-lock.json` root entries;
      - the spec's `upstream_version`, `Version` and newest `%changelog`;
      - the PKGBUILD's `_upstream_ver` and `pkgver`;
      - the newest `debian/changelog` entry;
      - the newest metainfo `<release>`;
      - `release-assets.toml`.

      It skips `website/package.json`. Verify: it passes on the tree as it is, and on a copy with
      `pkgver` edited it fails naming the file, the expected value and the value found
- [x] 1.3 Add `packaging/version.sh set <upstream>`:
      - it refuses any shape outside `X.Y.Z` / `X.Y.Z-(alpha|beta|rc).N`;
      - it writes every site;
      - it regenerates `Cargo.lock` with cargo and `gui/package-lock.json` with
        `npm version --no-git-tag-version`;
      - it adds dated entries to the spec `%changelog`, `debian/changelog` and the metainfo
        `<releases>` (`type="development"` for a pre-release).

      Verify on a scratch copy: `set 0.1.0-beta.1` then `check` passes, and
      `cargo metadata --locked --offline` succeeds; `set 0.1.0-beta` and `set 0.1.0+b.1` exit
      non-zero and change no file
- [x] 1.4 Add two `tests/scripts.rs` cases, both run through `common::run_other_tool`:
      - `version.sh check` runs on the repository;
      - `version.sh set 0.1.0-beta.1` runs on a temporary copy, and the case asserts the literal
        results `0.1.0~beta.1`, `0.1.0~beta.1-1` and `0.1.0beta1`.

      Add `packaging/version.sh check` as a step in `ci.yml`'s `rust` job. Verify:
      `cargo test --test scripts` and `cargo test --test test_isolation` pass, and poisoning one
      site makes both the test and the CI step fail
- [x] 1.5 Document the script in `CLAUDE.md`'s command list and in each packaging README's version
      section, stating that versions are set only through it. Verify: the documented commands run
      as written

## 2. Packaging tree fixes

- [x] 2.1 RPM spec changes (design D4):
      - add `%global upstream_version`;
      - point `Source0` at the release asset URL;
      - change `%autosetup -n %{name}-%{upstream_version}`;
      - drop the `rpm_tag` global.

      Update the "Source0 / release tag" section of `packaging/rpm/README.md`. Verify:
      `rpmspec -P` expands `Source0` to
      `…/releases/download/v0.1.0/proton-drive-sync-engine-0.1.0.tar.gz` while the version is
      still `0.1.0`
- [x] 2.2 PKGBUILD changes (design D4):
      - add `_pkgbase` and `_upstream_ver`;
      - make the tarball `source=` with `sha256sums=('SKIP')` and a comment saying why;
      - make every `cd` use `${_pkgbase}-${_upstream_ver}`;
      - set `install="${_pkgbase}.install"`;
      - add `'curl'` to `depends`.

      Update `packaging/arch/README.md` (source, checksums, the AUR variant). Verify:
      `bash -n PKGBUILD` passes, and `makepkg --printsrcinfo` in an Arch container prints the new
      source
- [x] 2.3 Deb changes:
      - in `debian/control`, give `cargo` and `rustc` the floor `(>= 1.90~)` and the profile
        `<!pkg.proton-drive-sync-engine.rustup>`, and add `curl` to `Recommends`;
      - in `debian/rules`, run `dh_installsystemduser --name=proton-syncd --no-enable`;
      - in `packaging/debian/README.md`, document the binary-only build command with
        `--build-profiles=`, and replace the "accepted lintian note" with the verified
        `dpkg-source` refusal and why no source package is built.

      Verify: `dpkg-checkbuilddeps -P pkg.proton-drive-sync-engine.rustup` (it has no long option) in an
      Ubuntu 22.04 container lists no Rust entry
- [x] 2.4 Change the Rust floor from 1.85 to 1.90 in five files: `README.md`, `setup.sh`'s
      `require_command` message, `packaging/debian/README.md`,
      `website/src/content/docs/start/installation.md` and
      `website/src/content/docs/distribution/packages.md`. Verify: `git grep -n '1\.85'` shows only
      dated history notes, and `(cd website && npm ci && npm run build)` succeeds
- [x] 2.5 Give the metainfo `<release>` entry the shape `version.sh` writes. Verify:
      `appstreamcli validate --no-net` on the metainfo reports no error (run it in a container if
      the host shell refuses it)
- [x] 2.6 Rewrite the CLI text in the three trees (design D15):
      - the RPM `%description`, the `debian/control` `Description` of the main package, and the
        PKGBUILD `pkgdesc` state that Proton's `proton-drive` CLI must be installed and signed in
        first; the two long descriptions give `https://proton.me/drive/download` and
        `proton-drive auth login`;
      - the PKGBUILD header comment and the RPM `%description` say Proton publishes the CLI only
        as a download from proton.me and that it is in no distribution's official repositories;
        drop "not in … the AUR", "not available in Fedora/COPR repositories" and "proprietary";
      - name no third-party build and add no `optdepends` for the CLI.

      Verify: `git grep -nE -e 'proton-drive-cli(-git|-bin)?([^/]|$)' -e 'proton-drive login'
      -- packaging/` prints nothing, and on the built packages `rpm -qip`, `dpkg-deb -I` and `pacman -Qip` show the
      prerequisite
- [x] 2.7 In `packaging/arch/proton-drive-sync-engine.install`, make `post_install` print a short
      note naming `https://proton.me/drive/download` and `proton-drive auth login`, then call
      `post_upgrade` as today. `post_upgrade` prints nothing new. The RPM scriptlets and the deb
      `postinst` stay silent (design D15). Verify: `bash -n` passes on the script; in an Arch
      container, `pacman -U` of the built package prints the note, and running the same
      `pacman -U` again (a reinstall, which runs `post_upgrade`) does not print it

## 3. Package build workflow and the pull-request gate

- [x] 3.1 Add `workflow_call` to `ci.yml`, leaving its `push`/`pull_request` triggers unchanged.
      Verify: an ordinary pull request still runs `rust`, `frontend` and `fidelity`
- [x] 3.2 Create `.github/workflows/packages.yml` with its tarball job (design D5): `git archive`
      with `--prefix=proton-drive-sync-engine-<upstream>/`, piped through `gzip -n`, uploaded as
      an artifact. Verify in the job: the tarball's file list equals
      `git ls-tree -r --name-only HEAD` under that one prefix
- [x] 3.3 Add the RPM job in `fedora:<oldest non-EOL>`:
      - set the tag in one workflow variable;
      - run `dnf builddep` on the spec, then `rpmbuild -ba` from the tarball;
      - rename the outputs to upstream-version names.

      Verify: the job emits the source RPM plus the main, `-nautilus` and `-nemo` RPMs, and
      `rpm -qp --requires` on the main RPM lists `curl`, `libsecret` and `hicolor-icon-theme`
- [x] 3.4 Add the deb job in `ubuntu:22.04`:
      - install rustup (minimal, `stable`);
      - run `apt-get build-dep` with the profile;
      - run `dpkg-buildpackage -b -us -uc` with the profile, using the tree unpacked from the
        tarball with `packaging/debian` linked as `./debian`;
      - rename the outputs.

      Verify: three `.deb` files are produced, and a one-off run with `libgtk-layer-shell-dev`
      removed fails before compiling, naming it
- [x] 3.5 Add the Arch job in `archlinux:base-devel`. It runs `makepkg -s` as an unprivileged user,
      with the tarball beside the PKGBUILD. Verify: one `.pkg.tar.zst` is produced, and
      `namcap` on it reports no error-level finding
- [x] 3.6 Write `packaging/arch/aur.sh <stable|beta> <assets-dir>` (design D8) and run it in the Arch
      job for `beta`. Verify:
      - the generated `PKGBUILD` has `pkgname=proton-drive-sync-engine-beta`, `provides` and
        `conflicts`, and the tarball's real SHA-256;
      - `makepkg --printsrcinfo` output equals the generated `.SRCINFO`;
      - `makepkg` of the generated tree verifies the checksum against the local tarball
- [x] 3.7 Write `packaging/smoke-test.sh` (design D13). Add the install matrix:
      - every non-EOL Fedora, using the RPM;
      - Ubuntu 22.04, 24.04 and 26.04, and Debian 12 and 13, using `apt-get install ./…deb`;
      - `archlinux:latest`, using `pacman -U`.

      The Arch cell keeps the output of `pacman -U` and checks it for the install note (2.7).
      The doctor check is not in this task; it is 3.10. Verify: every cell passes, and a one-off
      run with the unit enabled (the old `debian/rules`) fails the deb cells on the
      `default.target.wants` check
- [x] 3.8 Add the pull-request trigger and a weekly `schedule`. Add a change-detection job over the
      `linux-packages` filter, which build and smoke jobs check with `if:`. Add the aggregator job
      `packages-ok` (design D2), and make only it a required check. Verify on two pull requests:
      - one touching only `website/` shows `packages-ok` passed with every build job skipped,
        and the pull request is mergeable;
      - one touching `packaging/debian/control` runs every build and smoke job, and
        `packages-ok` fails when one of them is made to fail

      Done: the triggers, the `changes` job, the build jobs' gate and `packages-ok` are in
      `packages.yml`, and `packages-ok` passes on a pull request that builds (PR #473). The three
      smoke jobs are in `needs` (PR 2b), and `tests/scripts.rs` runs the aggregator's script over
      every shape `needs` can take (all green, builds skipped, each job failed or cancelled, a
      smoke job skipped while builds were needed, no answer from `changes`).

      Verified on 2026-10-13 with two throwaway draft pull requests against `main` at `0723cc7`,
      both closed unmerged with their branches deleted:
      - PR #475 changed only `website/README.md`. Packages run 38417756086: `changes` passed, and
        `tarball`, `rpm`, `deb`, `arch`, `smoke-rpm`, `smoke-deb` and `smoke-arch` were all skipped
        (no build or smoke job ran); `packages-ok` passed. The pull request read `mergeable:
        MERGEABLE`, `mergeStateStatus: CLEAN` (it was a draft and GitHub still said CLEAN).
      - PR #476 added a `Depends` on a package that does not exist to `packaging/debian/control`.
        Packages run 38417794552: `changes`, `tarball`, `rpm`, `deb`, `arch`, `smoke-rpm` (43, 44)
        and `smoke-arch` all ran and passed; the five `smoke-deb` cells (ubuntu 22.04, 24.04, 26.04,
        debian 12, 13) ran and failed at the install; `packages-ok` failed. The pull request read
        `mergeable: MERGEABLE`, `mergeStateStatus: BLOCKED` (the required check failed).

      `packages-ok` was added to `main`'s required status checks on 2026-10-13
      (`gh api repos/osirison/proton-drive-sync-engine/branches/main/protection/required_status_checks
      -q .contexts` lists it with `rust`, `cargo-audit`, `analyze (rust)` and `frontend`), so nothing
      is left for this task.
- [x] 3.9 Fix whatever the first real builds of the deb and Arch trees (never built before) and the
      RPM tree turn up, in the trees themselves. Record each fix in that tree's README's validation
      section. Verify: all build and smoke jobs are green on the pull request

      Done: the Arch tree needed `!lto` and `!debug`, `dbus` in `depends` with `python` as an
      `optdepends` (namcap), and lost `git` from `makedepends`; the deb and RPM trees built as written. Each is in that tree's README. The first
      smoke runs found no defect in the trees (the only failures were the validators on Ubuntu 22.04
      and Debian 12, which are old tools, not package defects: see the deb README). All build and
      smoke jobs are green on PR #474.
- [x] 3.10 After the separate change `add-dependency-check` is merged, add the doctor check to
      `packaging/smoke-test.sh` (design D13): with no `proton-drive` on `PATH` and no daemon
      running, `proton-sync doctor` must name `proton-drive` as missing, print
      `https://proton.me/drive/download` and exit non-zero, and no `proton-syncd` process or
      control socket may appear afterwards. Do not check what it says about the keyring. Verify:
      every smoke cell passes on a pull request, and a one-off run with the expected URL edited
      fails every cell on this check

      Done: every smoke cell passed on PR #474. The one-off was run locally in podman on all 8
      cells (`fedora:43`, `fedora:44`, `ubuntu:22.04`, `24.04`, `26.04`, `debian:12`, `debian:13`,
      `archlinux:latest`), each with the package from the PR 2a artifacts installed and a copy of
      `smoke-test.sh` whose line `cli_url="https://proton.me/drive/download"` read
      `.../drive/downloads`. All 8 exited 1. Seven printed `FAIL: proton-sync doctor did not print
      https://proton.me/drive/downloads`; the Arch cell printed that line and also `FAIL: pacman -U
      did not print https://proton.me/drive/downloads` (it checks the install note against the same
      URL). This was not run as a pull request, so it is not in CI.

## 4. Tag workflow and draft release

- [x] 4.1 Create `.github/workflows/release.yml`, triggered on pushed `v*` tags. Its gate job checks
      the tag regex, that the tag equals `v` plus the `Cargo.toml` version, that the commit is an
      ancestor of `origin/main`, and `version.sh check`. Verify with throwaway test tags on this
      repository (the maintainer's decision of 2026-10-13: a user cannot fork their own repository,
      so the checks use test tags and test draft releases here, deleted afterwards):
      - `v0.1.0-beta.2` on a tree whose `Cargo.toml` says another version fails naming both
        versions;
      - a matching tag on a branch commit fails;
      - neither creates a release

      Done: `packaging/release/gate.sh` holds the checks and `release.yml` calls it. The tag shape
      and the tree's version come from new read-only `version.sh` subcommands (`tag-version`,
      `current`), so the workflow keeps no pattern of its own. The ancestry check fails closed
      (only `git merge-base --is-ancestor` exit 0 passes; a missing `origin/main`, exit 128 and
      any other failure stop the release), and the workflow fetches the whole history and `main`
      by name first. `tests/scripts.rs` runs the gate in a temporary repository: a match on `main`
      and a commit behind its tip pass; a branch commit, a missing `origin/main`, a `git` that
      answers 128, a version mismatch (the message names both versions), a tag of the wrong shape
      and a drifted version site all fail. Live runs, 2026-10-13, on `main` at `9532681` (each
      tag deleted afterwards, no release created by any of them, checked in the API listing):
      - `v0.1` (run 38444673055): `gate` failed with "'v0.1' is not a release tag ...", `ci`,
        `packages` and `draft` skipped;
      - `v0.1.0-beta.2` (run 38444761099): `gate` failed with "the tag v0.1.0-beta.2 names
        version 0.1.0-beta.2, but Cargo.toml at this commit says 0.1.0", later jobs skipped;
      - `v0.1.0` on a throwaway commit `5403b6b` on top of `main`, never pushed as a branch (run
        38444919037): `gate` failed with "commit 5403b6b... is not an ancestor of
        refs/remotes/origin/main (9532681...)", later jobs skipped.
- [ ] 4.2 Add the `ci` and `packages` jobs as calls to the two reusable workflows. Add the `draft`
      job, the only one with `contents: write`:
      - write `SHA256SUMS`;
      - run `gh release create --draft --verify-tag --generate-notes`, adding `--prerelease` when
        the version has a pre-release part;
      - use `--clobber` and remove stale assets when a draft exists;
      - fail when the release is already published.

      Verify with test tags and test draft releases on this repository (see 4.1):
      - a beta tag gives a draft pre-release with exactly the listed assets;
      - `sha256sum -c SHA256SUMS` passes on the downloaded assets;
      - no asset name contains `~`;
      - a re-run replaces the assets;
      - a re-run after publishing fails and leaves the release unchanged

      Done: `ci` and `packages` call `ci.yml` and `packages.yml` after `gate` (each caller job
      grants `contents: read`, which is all the two files ask for), and `draft` needs all three.
      `packaging/release/draft.sh` does the rest. It finds the tag's releases through the API list
      (a lookup by tag may not return a draft), refuses a published release and more than one draft
      before it writes anything, requires exactly the expected assets (the tarball, the source RPM,
      three RPMs, three debs, the Arch package; named by the upstream version; no `~`), writes
      `SHA256SUMS` and checks it, then creates the draft (`--draft --verify-tag --generate-notes
      --notes-file`, plus `--prerelease` as `version.sh is-prerelease` says) or replaces the one
      draft (`gh release upload --clobber`, then deletes the draft's assets this run did not
      produce). `tests/scripts.rs` runs it with a stub `gh` over every branch: no release, one draft
      with a stale asset, a draft on a later page of the listing, two drafts, a published release
      (with and without a draft beside it), a failed listing, bad assets, and the beta and final
      `--prerelease` decisions; on the refusals the stub fails on any write. Other tests hold
      `release.yml` to the rules above (each `permissions:` block exactly, `needs`, the artifacts
      by name, no `${{ }}` in any `run:` step) and check that no other workflow runs on a tag push.
      The asset set is also checked on every package build: `packages.yml` has a `release-assets`
      job (in `packages-ok`'s `needs`, skipped with the builds) that runs `draft.sh --check-assets`,
      the draft job's own check, so a new RPM subpackage or a debuginfo package fails the pull
      request and not the first release. Replacing a draft touches its assets only: for one tag the
      pre-release flag and the notes header cannot change, so only the notes GitHub generated at
      creation can go stale, and no refresh is built. Left, after this merges:
      Live runs, 2026-10-13 (`v0.1.0` at `9532681`, run 38445051252, re-run as attempt 2; the
      draft and tag were deleted afterwards with `gh release delete --cleanup-tag`):
      - every job succeeded (`gate`, `ci`, `packages` with `release-assets` and `packages-ok`,
        `draft`); exactly one release for the tag, a draft and not a pre-release, with the ten
        assets (tarball, source RPM, three RPMs, three debs, Arch package, `SHA256SUMS`) and no
        `~` in any name;
      - `gh release download` worked on the draft; `sha256sum -c SHA256SUMS` passed on the nine
        files, and `SHA256SUMS` listed exactly those nine;
      - a junk asset was uploaded (eleven assets) and the run re-run: it succeeded, still one
        draft and not a pre-release, ten assets, the junk file gone, every asset id changed (for
        example the tarball 637899865 to 637945487), and `sha256sum -c` passed again.

      4.2 stays unticked because two of its Verify lines cannot be shown yet (below).

      Left, after this merges:
      - "a beta tag gives a draft pre-release" cannot be shown live before the first release: `main`
        says `0.1.0`, so only `v0.1.0` passes the gate until 7.1. The stub test covers the
        `--prerelease` decision; the live check is 7.3;
      - "a re-run after publishing fails and leaves the release unchanged" needs a published
        release, which a test must not make; the stub test covers it, and it is shown live at the
        first release
- [x] 4.3 Write `packaging/release-notes.md`, the draft's notes header:
      - a link to the docs packages page (not a copy of its commands);
      - the `proton-drive` prerequisite: install it from `https://proton.me/drive/download` and
        run `proton-drive auth login` before installing this app, with no other link for the CLI
        (design D15);
      - a pre-release warning.

      Verify: the test draft shows it above the generated notes

      Done: `packaging/release-notes.md` has the three parts. The warning sits between two marker
      lines that `draft.sh` removes, with the warning, for a final version (and keeps, without the
      markers, for a pre-release), so a `v0.1.0` draft carries no pre-release warning.
      `tests/scripts.rs` checks both versions' notes as `gh` receives them. `gh release create
      --help` says notes given with `--notes` are prepended to generated ones; it does not say the
      same of `--notes-file`. Live, on the `v0.1.0` test draft (2026-10-13): `gh release view
      --json body` shows the header first (the `proton-drive` prerequisite with
      `https://proton.me/drive/download` and `proton-drive auth login`, then the packages page
      link, then the `SHA256SUMS` line) and GitHub's generated "What's Changed" list after it,
      so `--notes-file` lands above `--generate-notes`. There is no pre-release warning, as
      expected for a final version.
- [x] 4.4 Add a tag ruleset limiting who may create `v*` tags. Verify:
      `gh api repos/osirison/proton-drive-sync-engine/rulesets` lists it

      Done: the listing shows ruleset 25018911, "Release tags v*", active, target tag, on
      `refs/tags/v*`, with the rules creation, update and deletion and a bypass for the repository
      admin role. The coordinator added it on 2026-10-13. The live runs in 4.1 showed that an
      admin push and an admin delete of a `v*` tag go through (the server printed "Bypassed rule
      violations").

## 5. Accounts, keys and the release environment (maintainer, outside the repo)

- [ ] 5.1 Create the COPR project `proton-drive-sync-engine-beta`:
      - `--enable-net on`;
      - one chroot per non-EOL Fedora release;
      - store the `~/.config/copr` contents as environment secret `COPR_CONFIG`;
      - store the expiry date as repository variable `COPR_TOKEN_EXPIRES`.

      Confirm the COPR namespace (design Open Questions). Verify: `copr-cli whoami` succeeds in
      the preflight steps (6.2)
- [ ] 5.2 Register an AUR account and an SSH key used only by CI, and store the private key as
      `AUR_SSH_KEY`. Verify: `ssh -T aur@aur.archlinux.org` in the preflight steps authenticates
- [ ] 5.3 Generate a dedicated Ed25519 signing key with no expiry (design D11). Store the armored
      private key as `APT_SIGNING_KEY`, and record its fingerprint in
      `packaging/release/apt-key-fingerprints.txt` in a pull request merged before the first tag
      (6.7 has the commands; until it holds a fingerprint `apt-publish.sh` signs nothing). Verify:
      a test `InRelease` signed in CI verifies with `gpgv` in Ubuntu 22.04 and Ubuntu 26.04 and with
      apt's `sqv` in Debian 13 (checked 2026-10-13: Ubuntu 26.04, apt 3.2.0, uses `gpgv`; Debian 13,
      apt 3.0.3, is the `sqv` cell)
- [ ] 5.4 Create the `osirison/proton-drive-sync-engine-apt` repository:
      - Pages served from `gh-pages`, with `.nojekyll`;
      - a write deploy key stored as `APT_DEPLOY_KEY`.

      Verify: `https://osirison.github.io/proton-drive-sync-engine-apt/` serves a placeholder
      file
- [ ] 5.5 Create environment `release` with the maintainer as required reviewer, holding the four
      secrets. Set the environment's deployment branches and tags to `main` and tags matching
      `v*` only (Settings, Environments, release, Deployment branches and tags): without that, a
      run dispatched from any branch with an edited `publish.yml` reaches the secrets after one
      click, and for a solo maintainer that click is a self-approval, not a code review. Verify: a
      `workflow_dispatch` dry run of the publish workflow waits for approval before any job reads
      a secret, and asks for exactly one approval; a dispatch with `--ref` some other branch is
      refused before approval

## 6. Publish workflow

- [ ] 6.1 Create `.github/workflows/publish.yml` (design D12):
      - triggered on `release: [published]`, and on `workflow_dispatch` with inputs `tag` and
        `dry_run`;
      - one `publish` job in environment `release` (the only job that reads secrets);
      - routing: read the release's `prerelease` flag, and send a pre-release to beta only and a
        final release to beta and stable.

      Verify: a dry run for a pre-release tag logs beta targets only, one for a final tag logs
      both, and each run asks for one approval

      Done (PR 4a, 2026-10-13; not ticked, the Verify needs section 5): `publish.yml` has the two
      triggers (`dispatch.dry_run` defaults to true), one `publish` job in environment `release`
      (the only place `secrets.` is read, in one step that writes them to mode-600 files),
      `contents: read`, a per-tag `concurrency` group that is never cancelled, and
      `persist-credentials: false`. `packaging/release/route.sh` reads the release through the API
      (a draft or missing release is an error), accepts `prerelease` only as the JSON true or
      false, refuses a flag that disagrees with `version.sh is-prerelease` (a beta marked final
      would otherwise reach stable), and fails closed, printing nothing. The dry run logs each
      routed channel with its COPR project and AUR package. The apt step and the verification jobs
      (6.3, 6.6) were added later, in PR 4b. Tests: `tests/scripts.rs` (routing, the
      workflow's pinned expression lines, permissions, step order and gating, the namespace in one
      place). Left: a dry run for a beta tag and for a final tag asking for one approval needs the
      environment (5.5) and a published release.
      GitHub creates an environment with no protection the first time a workflow names it. The
      maintainer must create `release` with its reviewer (5.5) BEFORE any release is published.
      Known consequence of checking out the tag (`tag-checkout.sh`): every step after the gate runs
      the TAG's own scripts. So a tag cut before this change merges cannot be published by this
      workflow; a later fix to a publish script does not reach an existing tag without re-tagging;
      and a later change that renames an `env:` name the scripts read (the step-level names in
      `publish.yml`) breaks publishing of older tags. All of these fail closed. Rule: keep those
      names stable, or change them only together with a re-tag. The runbook (6.7) repeats this.
- [ ] 6.2 Add the preflight steps at the top of the `publish` job:
      - `copr-cli whoami`, and fail when `COPR_TOKEN_EXPIRES` has passed;
      - `ssh -T` to the AUR;
      - load and test-sign with the signing key;
      - `git ls-remote` the apt repository with the deploy key;
      - each failure names the credential and its renewal step.

      Also warn in `release.yml` when `COPR_TOKEN_EXPIRES` is within 14 days. Verify: a dispatch
      with a deliberately broken `COPR_CONFIG` fails in preflight, and no channel step runs

      Done (PR 4a; not ticked, it needs the secrets): `preflight.sh` tries all four credentials
      and lists every failure with the credential and its renewal step; the exit is 1 if any failed
      and the channel steps are gated on `steps.preflight.outcome == 'success'`. Findings:
      - `copr-cli whoami` reads the user name from the config file and asks the server nothing
        (checked in copr-cli 2.6 and 2.7: a fake token answers with its user name). It proves the
        file is there and parses, not that the token works. The token's age is therefore carried by
        `COPR_TOKEN_EXPIRES`, and a token the server refuses shows at the build as "Login
        invalid/expired" (the COPR step fails then).
      - The date format is `YYYY-MM-DD`, read as UTC, the last day the token works. Empty,
        unparseable or past fails the preflight and names the variable (`token-expiry.sh check`).
      - `ssh -T aur@aur.archlinux.org` exits 1 on success. The check is the greeting aurweb
        prints, `Welcome to AUR, <user>! Interactive shell is disabled.` (read from aurweb's
        `git/serve.py`), with exit 255 (ssh's own failure) always a failure. Both ssh hosts are
        pinned in `packaging/release/known_hosts`; the fingerprints equal the ones the AUR and
        GitHub publish and a test holds them.
      - The signing key is imported into a throwaway keyring, used to sign a test string over
        SHA-512 and verified; a key with an expiry only warns (D11 wants none).
      The `release.yml` warning is `token-expiry.sh warn` in the gate job: it never fails, and
      it warns also when the variable is empty (it does not exist until 5.1) or unparseable.
      Left: the Verify (a dispatch with a broken `COPR_CONFIG`) and the first real use of the AUR
      greeting match and the pinned host keys, which need 5.1 and 5.2.
- [ ] 6.3 Add the apt step (design D10). Each channel step runs even if an earlier channel step
      failed:
      - download the `.deb` assets of the newest three releases per suite;
      - generate the tree with `apt-ftparchive`;
      - sign `InRelease` and `Release.gpg`;
      - publish the key as `.gpg` and `.asc`;
      - force-push a single-commit `gh-pages`.

      Verify: a re-run gives identical `Packages` files, and every pooled `.deb`'s SHA-256 matches
      its release's `SHA256SUMS`

      Done (PR 4b; not ticked, it needs the signing key, the apt repository and a published
      release): `apt-publish.sh` rebuilds the whole tree from the published releases (design D10,
      "Decisions made when writing the apt step": beta is the newest 3 of all by version, stable the
      newest 3 finals, ordered by `version.sh sort-key`, `APT_WITHDRAWN` withdraws a tag, the push
      is skipped when only `Date:` differs). Every deb is checked against its release's
      `SHA256SUMS` (a missing or different sum stops the step). `apt-client.sh` is the one text of
      the users' commands and of the recorded fingerprints (`apt-key-fingerprints.txt`, empty until
      5.3). `--local` makes the same tree from a directory, and the pull-request job `apt-repo` runs
      it on every package build with throwaway keys: byte-identical `Packages` twice, the install
      on Ubuntu 22.04, Debian 13 and Ubuntu 26.04, and the key rotation (`apt-rotation-check.sh`).
      Proven by hand and in tests: the real `apt-ftparchive` and `gpgv`, publish mode against stub
      `gh` and a local bare repository (the first run one commit, the same run again none, a
      change exactly one), the dry run, the withdrawal, the fingerprint refusals. Bugs that testing
      found in publish mode and that are fixed: an unbound `APT_WITHDRAWN` on every real run, and
      an unquoted `v*` that could withdraw a real tag; a dry run after a real publish that always
      said "would force-push" (the key files were written only when signing); `apt-local-fixture.sh`
      with `/tmp` hardcoded; the `apt-repo` job splitting `$(cat suites.txt)` unquoted.
      Left: the Verify against the real repository, which needs 5.3, 5.4 and a published release;
      `apt-publish.sh` in publish mode has not run against GitHub's real API or Pages, and
      `apt-rotation-check.sh` has run only in containers.
- [ ] 6.4 Add the AUR step. It runs `aur.sh` for each routed channel and pushes `PKGBUILD`,
      `.SRCINFO` and the install script to `ssh://aur@aur.archlinux.org/<pkgname>.git`, creating
      the package on its first push. It does nothing when the generated tree equals the AUR head.
      Verify: a re-run makes no new AUR commit

      Done (PR 4a; not ticked, it needs the AUR account): `aur-publish.sh` downloads the release's
      tarball and `SHA256SUMS`, checks one against the other, and runs the `packaging/arch/aur.sh`
      INSIDE that tarball (the tagged bytes, not the PKGBUILD of whatever the workflow was checked
      out at) in an `archlinux:base-devel` container that is given the work directory and nothing
      else (`aur-container.sh`; the runner has no makepkg). It then clones the AUR package, replaces
      its files with the three generated ones and commits and pushes `HEAD:refs/heads/master` only
      if git sees a difference. An empty clone (a package the AUR has not got yet) takes the name
      `master` and the push creates the package. The commit identity is `AUR_GIT_NAME` and
      `AUR_GIT_EMAIL` from the workflow's `env:` (no personal address in the repository). The test
      uses local bare repositories: the first run makes one commit, the same run again none, a
      changed tarball exactly one; a dry run clones and compares but pushes nothing. What
      was run by hand (2026-10-13), in the first PR the container script alone, from a directory
      made by hand: that missed that `aur-publish.sh` makes its work directory with `mktemp -d`
      (mode 0700, owned by the runner), which the container's `builder` user cannot enter
      (`cd /work: Permission denied`; found in review). Fixed (`chmod 755` on the work directory,
      and in `aur-container.sh`), and then run by hand end to end: the real `aur-publish.sh`,
      default generator (the container), a `docker` shim that calls podman (adds `:z` to the two
      mounts, the full image name, and `HOST_UID=0`, because under rootless podman the container's
      root is the host user, as the runner's uid is under rootful docker), a stub `gh` serving a
      `git archive` tarball and `SHA256SUMS`, local bare repositories as the AUR, a `makepkg`
      that is the real one in `archlinux:base-devel`. Both packages were pushed (one commit each,
      three files). The same run with the two `chmod` lines deleted fails with `cd: /work:
      Permission denied`. Under rootless podman the container is not a real uid split on the
      runner; a run on GitHub's runner is still to be seen. Left: the live run against
      the AUR (5.2), and a re-run showing no new commit there.
- [ ] 6.5 Add the COPR step:
      - run `copr-cli build --nowait` on the release's source RPM URL, in every non-EOL chroot of
        the routed projects;
      - then `copr-cli watch-build` on the returned IDs, failing if any chroot fails;
      - skip a project that already has a succeeded build of that version.

      Add a final step that fails the job if any channel step failed, naming which. Verify: a
      re-run for the same tag starts no new build, and the job ends only after the COPR builds
      finish

      Done (PR 4a; not ticked, it needs the COPR project and token): `copr.sh` builds the release's
      source RPM (its URL read from the release's assets; exactly one `*.src.rpm` named for the
      version) in each routed project with `copr-cli build --nowait` and NO `-r`, which the CLI
      documents as all the project's chroots: the chroots are the project's (5.1 creates one per
      non-EOL Fedora release), so no list is kept here; `copr-cli get` has no JSON to read them
      from. It submits every routed project first, then runs `watch-build` on each project's IDs,
      which exits non-zero when a chroot fails; every failed project is named and the others are
      still watched. The skip rule reads `list-packages --with-all-builds --output-format json`
      (the only call that gives each build's state and version): a succeeded build whose version
      before the last dash equals `version.sh spell rpm` skips the project; a build still running is
      waited for and not submitted again; a failed or cancelled one is built again; a read that
      fails or is not a JSON list fails the project (never read as "no builds"). The per-package
      `builds` list is limited by the server (10 for most packages of @copr/copr, 18 for one, newest
      first there; copr-cli sends no limit or order), so `latest_build` and `latest_succeeded_build`,
      which the server computes, are read as well: a limited list can hide only an older build of the
      version, which costs a build, never a wrong skip. A just-submitted build with no version yet
      counts as this version when it is not over and its source URL ends in the release's source RPM
      file name. The final step is `aggregate.sh preflight aur copr`: each step records `ok` or
      `failed` and a missing record counts as failed. `copr-cli` is installed from
      `packaging/release/copr-cli-requirements.txt`, every dependency pinned with SHA-256
      (`pip install --require-hashes --no-deps`), made with `pip-compile --generate-hashes
      --allow-unsafe --strip-extras` in `python:3.12-slim` from the one line `copr-cli==2.6`
      (it pulls `copr` 2.7). Left: the live run (5.1), the re-run Verify, and
      whether COPR stops building in a chroot once its Fedora release is end of life (not checked;
      5.1 creates the chroots).
- [ ] 6.6 Add one verification job per channel. Each runs after `publish`, outside the environment,
      with no secrets:
      - `dnf copr enable` plus `dnf install` per Fedora chroot;
      - `makepkg` of the AUR package in `archlinux:base-devel`;
      - the documented apt commands on Ubuntu 22.04, Debian 13 and Ubuntu 26.04, followed by
        `smoke-test.sh`.

      Verify: the jobs fail when pointed at a channel missing the version

      Done (PR 4b; not ticked, it needs a real release): `verify-copr`, `verify-aur` and
      `verify-apt` in `publish.yml` (design D12, "Decisions made when writing the verification
      jobs"): `needs: publish`, skipped on a dry run, no secret, no environment; each repeats
      `route.sh` and `tag-checkout.sh` and then runs `verify-channel.sh <family> <image>`, which runs
      the tag's `verify-<family>.sh` in a container with the checkout read-only. COPR installs the
      exact version on Fedora 43 and 44, the AUR job builds the package with `makepkg` and compares
      `pkgver` first, apt runs the documented commands on Ubuntu 22.04, Debian 13 and Ubuntu 26.04,
      checks the served key against `apt-key-fingerprints.txt`, installs the exact version and runs
      `smoke-test.sh`. Tests run each script on the host with stubs, including "a channel missing
      the version fails" for all three; apt was also run in `ubuntu:22.04`, `debian:13` and
      `ubuntu:26.04` against a local tree. Bugs found: `verify-aur.sh` used the version as a regex
      (`0x1x0beta1-1` passed for `0.1.0beta1`) and could not run under stubs (it now has test
      seams). Left: every job on a real release, `verify-aur.sh` with its default paths (never run),
      and the COPR cells beyond a local `dnf` check of `dnf-command(copr)` and an exact-version
      install.
- [ ] 6.7 Write `docs/releasing.md`, the maintainer runbook:
      - bump with `version.sh`, tag, review the draft, publish, approve the environment;
      - re-run a channel with `workflow_dispatch`;
      - renew the COPR token;
      - rotate the signing key;
      - withdraw a version from each channel;
      - the rule that a release is never published from a workflow (`GITHUB_TOKEN` would fire
        nothing).

      Add a `docs/agent-notes/` note keyed to `copr-cli` about the 180-day expiry. Verify: a dry
      run of the publish workflow follows the runbook step by step without a missing step

      Done (PR 4b; not ticked, the Verify needs a published release and section 5): `docs/releasing.md`
      covers the channels, every secret and variable by name, the one-time setup (5.1 to 5.5) with
      commands and checks, cutting a release, the dry run, what each verification job proves, re-runs,
      withdrawal from apt (`APT_WITHDRAWN`), the Pages cache window, key rotation and the rule that a
      release is never published from a workflow. The `copr-cli` note is extended with the 180-day
      expiry. A test in `tests/scripts.rs` holds the names the workflows read, and the scripts the
      page names, against the page. Left to the maintainer: section 5 itself, the COPR namespace,
      and following the page against a real release (a dry run needs a published release, so the
      first one cannot be dry-run first).

## 7. First release: 0.1.0-beta.1

- [ ] 7.1 Run `packaging/version.sh set 0.1.0-beta.1` in a pull request. Verify: `version.sh check`,
      the full Rust and frontend suites, and every package and smoke job are green
- [ ] 7.2 Gate: do not push `v0.1.0-beta.1` until the separate change `add-dependency-check` is
      merged and 3.10 is done. Its code edits (`proton-sync doctor`, the first-run screen, the
      `setup.sh` check, the daemon's boot log, the `proton-drive auth login` fixes) are planned
      there, not here. Verify: `origin/main` contains the merge of `add-dependency-check`, and the
      latest smoke run on `main` includes the doctor check and is green
- [ ] 7.3 After 7.1 and 7.2, push tag `v0.1.0-beta.1`. Verify the draft: marked pre-release; holds
      the tarball, the source RPM, 3 RPMs, 3 debs, 1 Arch package and `SHA256SUMS`;
      `sha256sum -c` passes; the notes header is present
- [ ] 7.4 Publish the draft and approve the environment. Verify: the three verification jobs are
      green, and the stable COPR project, stable AUR package and apt `stable` suite do not exist
      or are empty

## 8. User documentation of the channels (after 7.4, because the docs list only channels that hold a release)

- [ ] 8.1 Rewrite `website/src/content/docs/distribution/packages.md`:
      - make step 1, above the three family sections, "install Proton's `proton-drive` CLI and run
        `proton-drive auth login`" as design D15 words it, linking only Proton's pages;
      - replace the "it isn't in any distro repo" note with: Proton publishes it only as a
        download from proton.me, and it is in no distribution's official repositories;
      - move the "make sure `proton-drive` is installed … logged in first" line of "After
        installing" into step 1;
      - rewrite the three family sections to cover the beta channels:
        `dnf copr enable osirison/proton-drive-sync-engine-beta`; AUR
        `proton-drive-sync-engine-beta`; the apt key, `.sources` file and fingerprint from design
        D10, as full `https://` URLs; the GitHub release assets with `sha256sum -c`; a note that
        COPR and the AUR rebuild from the tarball;
      - remove the "No v0.1.0 tag" note.

      Verify: following step 1 and then each section in a clean Fedora, Ubuntu 24.04, Debian 12
      and Arch container installs `0.1.0-beta.1`
- [ ] 8.2 In `website/src/content/docs/start/installation.md`:
      - rewrite "Prerequisite: the proton-drive CLI" as step 1 (design D15): the download page,
        the `linux/x64-baseline` build for CPUs without AVX2, `proton-drive auth login`,
        `proton-drive version`, Proton's two docs pages, and keeping the default credential store;
      - reword its note that the CLI is not in distribution repositories as in 8.1;
      - update the "Native packages" section to point at the channels;
      - keep the `proton-sync doctor` step that `add-dependency-check` added to this page.

      Verify: `(cd website && npm run build)` succeeds; the page still tells the user to run
      `proton-sync doctor` after installing; every apt link in the built HTML starts
      with `https://osirison.github.io/proton-drive-sync-engine-apt/`; the CLI step comes before
      every package and source-build command
- [ ] 8.3 Add to `README.md`, ahead of the source build, step 1 (install Proton's CLI and run
      `proton-drive auth login`, linking only Proton's pages) and then an "Install from a
      package" path with the same commands as the packages page. Keep the `proton-sync doctor`
      step that `add-dependency-check` added. Verify: the commands match the packages page
      character for character, the CLI step comes before every package and source-build command,
      and the `proton-sync doctor` step is still there
- [ ] 8.4 Check the CLI text of everything this change wrote. Verify:
      `git grep -nE -e 'proton-drive-cli(-git|-bin)?([^/]|$)' -e 'proton-drive login' --
      README.md packaging/ website/src/content/docs/start/installation.md
      website/src/content/docs/distribution/packages.md` prints nothing (the desktop docs pages
      are `add-dependency-check`'s), and every CLI link in those files is one of the four Proton
      URLs in the `linux-packages` spec

## 9. Integration check

- [ ] 9.1 Walk every scenario in the three specs against the first release:
      - tag gate failures (4.1);
      - draft contents (7.3);
      - channel routing (7.4);
      - preflight (6.2);
      - idempotent re-runs (6.3–6.5);
      - smoke tests (3.7), including the doctor check (3.10);
      - the package descriptions and the Arch install note (2.6, 2.7);
      - the docs commands and the CLI step first (8.1–8.4).

      Record any scenario not exercised and why. Verify:
      `openspec validate add-linux-release-pipeline --strict` passes

## Workflow follow-up

- Follow the repository's PR workflow for each merge (commit, push, pull request, review, green CI).
- Write a journal entry under `~/fedora/journal/proton-drive-sync-engine/` once the first release is
  published (context, problems met in the first real deb/Arch builds, decisions).
- Archive the change after the first release is published and its verification jobs are green.
