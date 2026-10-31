# Releasing

The runbook for one maintainer who does the release steps by hand. The private keys never pass
through an agent or a script on your machine: you make them, you paste them into GitHub, and the
publish workflow reads them from there.

Everything here is what `.github/workflows/release.yml`, `publish.yml` and `packages.yml` and the
scripts in `packaging/release/` really do. Where the scripts do not settle something, this page says
it is your choice. `tests/scripts.rs` checks that every secret and variable `publish.yml` reads is
named here, that each is of the kind and set where the workflows read it, that the environment is
created before the first `--env release` command, and that every script named here exists.

## What a release is

A release goes to four places:

| Channel | Beta | Stable |
| --- | --- | --- |
| GitHub Release | the release itself | the release itself |
| COPR | project `<COPR_OWNER>/proton-drive-sync-engine-beta` | project `<COPR_OWNER>/proton-drive-sync-engine` |
| AUR | package `proton-drive-sync-engine-beta` | package `proton-drive-sync-engine` |
| apt | suite `beta` | suite `stable` |

Two steps, two people-clicks:

1. You push a tag `v<version>`. `release.yml` checks it, runs the tests and the package builds, and
   makes a **draft** GitHub release with the packages and `SHA256SUMS`. Nothing is published to a
   channel.
2. You publish the draft. That fires `publish.yml`, which waits for your approval of the `release`
   environment, then publishes to apt, the AUR and COPR, and then three kinds of verification job
   install the new version from the live channels.

A pre-release (`0.1.0-beta.1`) goes to the beta channels. A final release (`0.1.0`) goes to beta and
stable. The choice comes from the release's `prerelease` flag, never from the tag text (see
"The pre-release rule").

Never publish a release from a workflow. A release published with `GITHUB_TOKEN` fires no other
workflow, so `publish.yml` would never run. Publish from the web page, signed in as yourself.

## One-time setup

Do these once, in this order: the `release` environment first, then the four secrets. Nothing here
is done by the repository. The runbook's order is not the order of the change's tasks: `tasks.md`
lists the environment last (task 5.5) and keeps that numbering, but `gh secret set --env release`
needs the environment to exist, so setting a secret before creating it fails. The sections below
carry their task numbers: 5.5, then 5.1 to 5.4.

**The first real use of each credential is the first real publish.** A dry run needs a published
release (`route.sh` refuses a draft), so none can run before the first release exists. Preflight is
the only check that tries all four credentials, and it runs inside the publish run itself, before any
channel step. What you can check earlier is local and is named in each section: the COPR file
parses, the AUR accepts the SSH key, the apt key makes a signature on your machine, and the
environment waits for your approval.

### Every secret and variable

| Name | Kind | Where it is set | Required | Read by |
| --- | --- | --- | --- | --- |
| `COPR_CONFIG` | secret | environment `release` | yes | preflight, `copr.sh` |
| `AUR_SSH_KEY` | secret | environment `release` | yes | preflight, `aur-publish.sh` |
| `APT_SIGNING_KEY` | secret | environment `release` | yes | preflight, `apt-publish.sh` |
| `APT_DEPLOY_KEY` | secret | environment `release` | yes | preflight, `apt-publish.sh` |
| `COPR_TOKEN_EXPIRES` | variable | the repository (Settings, Secrets and variables, Actions, Variables) | yes | `release.yml` (warns), preflight (fails) |
| `APT_WITHDRAWN` | variable | the repository | no | `apt-publish.sh` |

- All four secrets are required for every run, a pre-release and a dry run too: preflight tries all
  four before any channel step.
- `COPR_TOKEN_EXPIRES` is a repository variable, not an environment one, because the gate job in
  `release.yml` has no environment and could not read it.
- `APT_WITHDRAWN` unset or empty withdraws nothing.

Values that are not settings, but are written in `publish.yml`'s `env:` and changed by editing the
file: `COPR_OWNER` (the COPR namespace, currently `osirison`; **not decided**, your choice),
`AUR_GIT_NAME`, `AUR_GIT_EMAIL`, `APT_GIT_NAME` and `APT_GIT_EMAIL` (the identity of the commits
pushed to the AUR and to the apt repository; your choice, no personal address is in the repository).
`GH_TOKEN` is the workflow's own read-only token. The scripts stop when one of these is missing.
Per the file's own rule, do not rename any of these names without re-tagging: the scripts of an
older tag read the old names.

### Before the first secret: a scratch directory

Every private key below is made in one scratch directory outside the repository, with `umask 077`,
and deleted as soon as it is stored. Work in one terminal for the whole setup. In a new shell, run
`umask 077`, set `repo="$(git rev-parse --show-toplevel)"`, and set `scratch` to the SAME directory
that `echo "$scratch"` printed (for example `scratch=/tmp/pdse-setup.AbC123`). Do not run `mktemp`
again: it makes a new directory and the keys already in the old one are lost.

```sh
repo="$(git rev-parse --show-toplevel)"      # run inside your checkout; the pinned known_hosts is in it
umask 077
scratch="$(mktemp -d /tmp/pdse-setup.XXXXXX)"   # mode 700
echo "$scratch"
```

Nothing in this runbook writes a key under `$repo`. `.gitignore` also lists `aur-ssh-key`,
`apt-signing-key.asc` and `apt-deploy-key` as a second guard, but do not rely on it: when you commit,
name the file (`git add packaging/release/apt-key-fingerprints.txt`), never `git add -A`.

### The `release` environment (task 5.5)

Settings, Environments, New environment, named `release`. Create it first and put **no secret in it
yet**: the secret commands below use `--env release`, which needs the environment to exist.

1. Required reviewers: yourself. Leave "Prevent self-review" off, or you cannot approve your own
   run.
2. Deployment branches and tags: "Selected branches and tags", with `main` (branch) and `v*` (tag).
   This stops a dispatch from any other branch: without it, anyone who can push a branch can run
   `publish.yml` as edited on that branch, and your approval click is only a click, not a review of
   that code. It does **not** stop a `v*` tag. Whoever can push one can run `gh workflow run
   publish.yml --ref <that tag>`, which runs `publish.yml` as edited at that tag, and an edited copy
   can drop the gate. Tag creation is restricted by the repository ruleset "Release tags v*" (id
   25018911: active on `refs/tags/v*`, with creation, update and deletion rules), and the repository
   admin role bypasses it always. So the reviewer's approval is what stands between such a tag and
   the secrets: before you approve a run, check that its ref is `main` or a tag you made.
3. GitHub creates an environment with no protection the first time a workflow names it. Creating
   this one first, and before any release is published, removes that case.

Check: `gh workflow run publish.yml --ref main -f tag=v0.0.0-test -f dry_run=true` makes a run that
waits for your approval before its first step, so before any job reads a secret; it asks once.
Reject it. A dispatch with `--ref` another branch is refused before the approval. (The tag value is
not looked at until the run is approved.) This reads no secret and works before any release exists.

### COPR project and token (task 5.1)

1. Sign in at https://copr.fedorainfracloud.org/api/ with the Fedora account. The page shows the
   config text and the login. Save it as `~/.config/copr` and run `chmod 600 ~/.config/copr`.
2. Create the beta project, with network on (the build downloads crates) and one chroot per
   Fedora release that is not end of life. `packages.yml` lists them: today 43 and 44, and the
   packages are built for x86_64:

   ```sh
   copr-cli create --chroot fedora-43-x86_64 --chroot fedora-44-x86_64 --enable-net on \
     proton-drive-sync-engine-beta
   ```

   `copr-cli create` with no owner makes the project under the account in `~/.config/copr`. The
   scripts name the project `<COPR_OWNER>/<name>`, and `publish.yml` has `COPR_OWNER: osirison`. So
   `COPR_OWNER` must equal what `copr-cli --config ~/.config/copr whoami` prints. For a project under
   a group, `COPR_OWNER` is `@group` and the create command needs the owner prefix
   (`@group/proton-drive-sync-engine-beta`); the command shown creates it under the account, and I
   have not run the group form. If they differ, change `COPR_OWNER` in `publish.yml`'s top-level `env:` (and the
   tag you cut must contain the change), or create the project under the owner `publish.yml` names.

   `copr.sh` builds in every chroot the project has and keeps no list of its own. When a Fedora
   release reaches end of life, the lists in `packages.yml` (`FEDORA_OLDEST`, the `smoke-rpm`
   matrix) and `publish.yml` (`verify-copr`) change together, and so do the project's chroots.
3. Create the stable project the same way, named `proton-drive-sync-engine`, **before the first
   final release**. The scripts do not create it: `copr.sh` reads the project first, and a project
   that does not exist fails that channel.
4. Store the token and its last day:

   ```sh
   gh secret set COPR_CONFIG --env release --repo osirison/proton-drive-sync-engine < ~/.config/copr
   gh variable set COPR_TOKEN_EXPIRES --repo osirison/proton-drive-sync-engine \
     --body "$(date -u -d '+180 days' +%F)"
   ```

   The token lasts 180 days. `COPR_TOKEN_EXPIRES` is the **last day it works**, as `YYYY-MM-DD`,
   read as UTC. `token-expiry.sh` refuses a date more than 200 days ahead, so a typo such as
   `9999-12-31` cannot switch the check off. `+180 days` from the day you made the token is the
   rule `token-expiry.sh` states; if the COPR page shows an exact date, using that is your choice.
5. Check. `copr-cli --config ~/.config/copr whoami` prints the user name. That proves only that the
   file parses: `whoami` asks the server nothing, so it succeeds with a made-up token
   (`docs/agent-notes/copr-cli-whoami-does-not-check-the-token.md`). No script in this repository
   tries the token against the server. A refused token first shows at the first `copr-cli build`, as
   "Login invalid/expired", so the date is the only thing that stands for its age. The first real
   use of the token is the first real publish.

What warns before it lapses: the gate job of `release.yml` prints a warning, 14 days ahead and also
when the variable is empty, unparseable or already past. It never fails a tag run. Preflight in
`publish.yml` does fail: on an empty, unparseable, past or more-than-200-days-ahead value, naming
the variable and the renewal steps. Nothing warns unless a tag is pushed or a publish runs.

To renew: do steps 1, 4 and 5 again (new token into `COPR_CONFIG`, new date into
`COPR_TOKEN_EXPIRES`).

### AUR account and SSH key (task 5.2)

1. Register an account at https://aur.archlinux.org/. Use a key for CI and nothing else:

   ```sh
   ssh-keygen -t ed25519 -N '' -C 'proton-drive-sync-engine CI' -f "$scratch/aur-ssh-key"
   ```

2. Paste `$scratch/aur-ssh-key.pub` into the account page (https://aur.archlinux.org/account/, field
   "SSH Public Key").
3. Store the private key and check it:

   ```sh
   gh secret set AUR_SSH_KEY --env release --repo osirison/proton-drive-sync-engine < "$scratch/aur-ssh-key"
   ssh -i "$scratch/aur-ssh-key" -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes \
     -o UserKnownHostsFile="$repo/packaging/release/known_hosts" -T aur@aur.archlinux.org; echo "exit $?"
   ```

   The AUR has no shell, so the right answer is `Welcome to AUR, <user>! Interactive shell is
   disabled.` and **exit 1**. Exit 255 is a failure. This also tests the pinned host key. It is a
   check against the real service that you can make before the first publish.
4. Delete the key now: `rm -f "$scratch/aur-ssh-key" "$scratch/aur-ssh-key.pub"`. You make no
   package by hand: the first push of `aur-publish.sh` creates `proton-drive-sync-engine-beta`, and
   the first final release creates `proton-drive-sync-engine`.

### The apt signing key (task 5.3)

Make it on your own machine, with nothing from a pipeline involved. Ed25519, **no expiry** and
**no passphrase**: the scripts sign with an empty passphrase, and an expiring key would stop apt for
every user until each downloaded the key again. The name in the key is your choice. The key lives in
`$scratch` (set above, `umask 077` in force, so the exported file is mode 600):

```sh
export GNUPGHOME="$scratch/gnupg"; mkdir -m 700 "$GNUPGHOME"
gpg --batch --pinentry-mode loopback --passphrase '' \
  --quick-gen-key 'Proton Drive sync engine apt <you@example.org>' ed25519 sign never
gpg --armor --export-secret-keys "$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "pub" { want = 1; next } $1 == "fpr" && want { print $10; want = 0 }')" > "$scratch/apt-signing-key.asc"
```

1. Store the key and **keep an offline copy** of `$scratch/apt-signing-key.asc` (encrypted, off this
   machine). A lost key means a rotation with no overlap, and every user must fetch the new key.
   GitHub secrets cannot be read back, so this copy is the only one.

   ```sh
   gh secret set APT_SIGNING_KEY --env release --repo osirison/proton-drive-sync-engine < "$scratch/apt-signing-key.asc"
   ```

2. Read its fingerprint (40 hex digits, the **primary** key's):

   ```sh
   gpg --show-keys --with-colons "$scratch/apt-signing-key.asc" | awk -F: '$1 == "sec" { want = 1; next } $1 == "fpr" && want { print $10; want = 0 }'
   ```

3. In a pull request, put that fingerprint in `packaging/release/apt-key-fingerprints.txt`: one
   fingerprint per line, 40 upper-case hex digits, no spaces; a line starting with `#` is a note.
   Merge it **before the first tag**. The file ships in the tag, and every step after the gate runs
   the tag's files. Commit that one file by name.
4. What the file does, so you know what a failure means:
   - While it holds no fingerprint (it does not at first), `apt-publish.sh` refuses to run, **a dry
     run too**, and `verify-apt.sh` refuses too.
   - `apt-publish.sh` signs only when the fingerprints in `APT_SIGNING_KEY` are exactly the ones in
     the file. A different key in the secret, or an extra one, publishes nothing.
   - `verify-apt.sh` compares the key the live site serves with the file. A key swapped on the site
     fails the verification job.
5. Check. Step 2 shows the key parses. The first real use of the key in CI is the first real
   publish: preflight signs and verifies a test string with it (and warns, without failing, if the
   key has an expiry), then the `apt` step signs the repository. A dry run (below) also makes one
   test signature with `gpgv`, but it needs a published release, so it comes after the first
   release. Task 5.3 also wants proof that apt on Ubuntu 22.04 (`gpgv`), Debian 13 (`sqv`) and Ubuntu
   26.04 (`gpgv`) accept the key: the `apt-repo` job of `packages.yml` runs that with throwaway keys on
   every pull request, and `verify-apt` runs it against the real key at every release.
6. When the offline copy exists and the pull request is open, delete the key and its keyring:

   ```sh
   gpgconf --kill gpg-agent; unset GNUPGHOME
   rm -rf "$scratch/gnupg" "$scratch/apt-signing-key.asc"
   ```

### The apt repository and GitHub Pages (task 5.4)

1. Create a **public** repository `osirison/proton-drive-sync-engine-apt` (users download from it
   without signing in). `apt-publish.sh` takes its URL as
   `git@github.com:<owner of this repository>/proton-drive-sync-engine-apt.git`, and the users' text
   in `apt-client.sh` names `https://osirison.github.io/proton-drive-sync-engine-apt/`: a different
   owner or name changes both files.
2. Give it a `gh-pages` branch holding `.nojekyll` and a placeholder file (the first publish
   replaces the branch, whatever it holds; the placeholder text is your choice). Clone it in the
   scratch directory, not inside this repository:

   ```sh
   git clone git@github.com:osirison/proton-drive-sync-engine-apt.git "$scratch/apt-repo" && cd "$scratch/apt-repo"
   git switch --orphan gh-pages
   touch .nojekyll && echo placeholder > index.html
   git add -A && git commit -m 'Placeholder' && git push origin gh-pages
   cd "$repo"
   ```

3. Settings, Pages: deploy from the branch `gh-pages`, folder `/`.
4. A write deploy key, only for this repository:

   ```sh
   ssh-keygen -t ed25519 -N '' -C 'apt deploy key' -f "$scratch/apt-deploy-key"
   ```

   Add `$scratch/apt-deploy-key.pub` under the apt repository's Settings, Deploy keys, with **Allow
   write access** ticked. Then:

   ```sh
   gh secret set APT_DEPLOY_KEY --env release --repo osirison/proton-drive-sync-engine < "$scratch/apt-deploy-key"
   ```

5. Check: `https://osirison.github.io/proton-drive-sync-engine-apt/` serves the placeholder.
   Preflight runs `git ls-remote` of the repository with the key, which proves the key can **read**,
   in the first publish run: that is the key's first real use. It cannot see whether write access is
   ticked; a key without it fails at the push, in the `apt` step.
6. All four secrets are stored. Delete everything private that is left:
   `rm -rf "$scratch"; unset scratch`. The only copy of the apt signing key is now the offline one.

### Immutable releases (a repository setting; your decision)

**Status when this was written: off.** Nothing in the repository turns it on, and no script reads it.
Why you may want it: the apt step trusts a release's `.deb` assets and its `SHA256SUMS`, and both are
assets of the same release. Without immutability, anyone who can edit the release (a maintainer
account, or a token with `contents: write`) can replace a `.deb` **and** `SHA256SUMS` after it is
published, and the next run of the `apt` step signs the replacement into the repository.
`apt-publish.sh` now reads each deb's own control file and refuses one that does not say what its name
says, but it cannot tell an honest package from a tampered one with the same labels. Immutability is
what stops the replacement.

What I checked, so that turning it on cannot break the pipeline: no job in this repository changes a
**published** release.

- Only the `draft` job of `release.yml` has `contents: write`, and it runs `draft.sh`. `draft.sh`
  lists the releases, and **refuses before its first write** when the tag already has a published
  release ("refusing to change it. Nothing was changed."). It uploads, replaces (`--clobber`) and
  deletes assets of a **draft** only. A stub test pins that the refusals write nothing.
- `name-assets.sh` renames files on the runner's disk before they are uploaded. It never touches a
  release.
- `publish.yml` and the verification jobs have `contents: read`. The scripts they run only list
  releases and download assets (`apt-publish.sh`, `aur-publish.sh`, `route.sh`).
- Re-running `release.yml` for a tag that is already published fails in `draft.sh` and changes
  nothing; task 4.2's check "a re-run after publishing fails and leaves the release unchanged"
  therefore holds with or without the setting. Re-running `publish.yml` never writes a release.

What it does, as GitHub documents it (I did not turn it on, so none of this was observed here): a
release published **after** the setting is on cannot have assets added, replaced or removed, and its
tag cannot be moved or deleted. A draft stays editable until you publish it. Releases published
before it was on are not covered.

Downsides, all consequences of that:

- A published release with a wrong asset cannot be repaired. You fix forward with the next version
  (the same rule as for COPR and the AUR).
- A tag that was published is gone for good: deleting the release does not free the tag name. "Start
  over" (below) works only **before** you publish the draft. Test with drafts
  and delete the draft, as the 4.2 live runs did; do not publish a throwaway release.
- You cannot attach anything (a signature, a fixed `SHA256SUMS`) to a published release.
- `APT_WITHDRAWN` is unaffected: it needs no change to the release.

To turn it on: the repository's Settings, General, the Releases section, the option for immutable
releases (GitHub's wording), **before** you publish the first release. A later release's page then
shows it as immutable.

## Cutting a release

1. **Bump the version.** In a branch:

   ```sh
   packaging/version.sh set 0.2.0      # needs cargo and npm; the only way to change the version
   packaging/version.sh check          # every one of the 19 sites agrees with Cargo.toml
   ```

   Open a pull request. Merge it when CI and the package jobs are green.
2. **Tag the merge commit on `main`.** The tag is `v` plus the `Cargo.toml` version exactly.

   ```sh
   git switch main && git pull --ff-only
   git tag v0.2.0 && git push origin v0.2.0
   ```

   `release.yml` runs: `gate.sh` (tag shape, tag equals `Cargo.toml`, the commit is on `origin/main`,
   `version.sh check`), the full CI suite, the package builds and install smoke tests, and then
   `draft.sh`. A failed job leaves **no** draft. A fix to the scripts does not reach an existing tag:
   the tag's own files run.

   **Start over (only while no release of that tag is published).** The draft exists from this step
   on, and a re-run of `release.yml` reuses it: `draft.sh` replaces its assets only, so the notes
   GitHub generated and the target it recorded stay as the first run left them. So delete the draft
   as well as the tag. `gh release view` and `gh release delete` may not find a draft by its tag, so
   find the ids and delete by id. The loop deletes every draft of the tag: one, several, or none (a
   failed job leaves no draft):

   ```sh
   for id in $(gh api --paginate repos/osirison/proton-drive-sync-engine/releases \
     --jq '.[] | select(.tag_name == "v0.2.0" and .draft) | .id'); do
     gh api -X DELETE "repos/osirison/proton-drive-sync-engine/releases/$id"
   done
   git push origin :refs/tags/v0.2.0 && git tag -d v0.2.0
   ```

   Deleting a release does not delete its tag. The ruleset "Release tags v*" has a deletion rule for
   `v*` tags, and only the repository admin role bypasses it, so the tag push works signed in as the
   admin (the live runs of task 4.1 showed an admin delete go through). Then fix, and tag again.
3. **Check the draft** (Releases, the draft, or through the API: `gh release view` may not find a
   draft by tag):

   ```sh
   gh api repos/osirison/proton-drive-sync-engine/releases \
     --jq '.[] | select(.tag_name == "v0.2.0") | {draft, prerelease, assets: [.assets[].name]}'
   ```

   - `draft` is true; `prerelease` is true exactly when the version has a pre-release part.
   - Ten files: the source tarball, the source RPM, 3 RPMs, 3 debs, the Arch package, and
     `SHA256SUMS` (`draft.sh` refuses any other set).
   - Download them from the draft's page and run `sha256sum -c SHA256SUMS`.
   - The notes start with the header from `packaging/release-notes.md` (a pre-release keeps its
     warning, a final release does not). Below it are the notes GitHub generated. On the **first**
     release there is no earlier tag, so they list every pull request since #1: trim them if you
     want, in the draft's editor. Re-running `release.yml` for the same tag replaces the draft's
     assets and leaves the notes you edited alone.
4. **Publish the draft** on its page ("Publish release"). Leave the pre-release box as it is.
5. **Approve the environment.** `publish.yml` starts and waits at `release`. Open the run, Review
   deployments, approve. One run asks for one approval, whatever the number of channels. The
   approval is the last point to stop: rejecting it ends the run before any step.
6. **Watch.** The `publish` job runs route, the tag's gate, the credential files, preflight, then the
   `apt`, `AUR` and `COPR` steps, then a final step that names every step that did not succeed.
   COPR waits for its builds, so the job takes tens of minutes. The verification jobs start after it.

There is no way to dry-run a release that is not published yet, because `route.sh` refuses a draft.

### A dry run

For a release that is already published (`dry_run` defaults to true):

```sh
gh workflow run publish.yml --ref main -f tag=v0.2.0 -f dry_run=true
```

It still waits for the environment approval and runs the preflight with the real credentials. Then:

- route logs the channels and the COPR project and AUR package of each;
- apt downloads the releases, builds the tree, uses the signing key for one test signature (verified
  and discarded), clones `gh-pages` and says whether it would force-push or finds it equal;
- AUR downloads the tarball, builds the tree in a container, clones the package and says whether it
  would commit;
- COPR reads the project's builds and says whether it would build;
- nothing is pushed, committed, built or uploaded, and the verification jobs are skipped.

A dispatch runs `main`'s `publish.yml` but checks out the tag and runs the tag's scripts.
`--ref <tag>` runs the tag's own workflow file.

### A tag must contain the channel scripts

A release event runs the workflow file **at the tag**, so what a tag publishes is fixed by what it
contains. A tag cut before the publish workflow (#480) has no `publish.yml`: publishing its release
starts nothing. A tag cut after #480 and before the pull request that added this page has a
`publish.yml` with the COPR and AUR steps only: a normal publish runs that file, goes green with
those two channels, skips apt without saying so, and verifies nothing. Only a `workflow_dispatch`
from `main` runs the newer file, and that fails at the `apt` step, because `apt-publish.sh` does not
exist in such a tag. Re-tag with a version that contains the channel scripts. No tag exists yet.

### The pre-release rule

The channels come from the release's `prerelease` flag as the GitHub API gives it, never from the
tag text. `draft.sh` sets the flag from the version, and `route.sh` checks it against the version
and refuses a disagreement: a beta marked final would reach the stable channels. If the flag is
wrong, fix it on the release and dispatch again. `route.sh` also refuses a draft and anything that is
not a published release of that tag.

## What the verification jobs check

They run after `publish`, outside the environment, with no secret. Each runs `route.sh` and
`tag-checkout.sh`, so the checks are the **tag's** scripts, and then `verify-channel.sh <family>
<image>` runs the check in a clean container with the checkout mounted read-only. A pre-release
checks the beta channel; a final release checks beta and stable. A failed `publish` job skips all of
them; a dry run skips them too.

| Job | Containers | What it does | A pass proves | A failure means |
| --- | --- | --- | --- | --- |
| `verify-copr` | `fedora:43`, `fedora:44` | `dnf copr enable`, then installs `proton-drive-sync-engine-<version>` exactly; `rpm` must report that version. Retries up to 600 s (30 s apart), `VERIFY_COPR_WAIT_SECS`, `VERIFY_COPR_POLL_SECS` | A Fedora user can enable the project and install this version | The project has no build of this version for that Fedora release: check the build in COPR, and that the project has that chroot |
| `verify-aur` | `archlinux:base-devel` | Clones the package from the AUR, requires its `pkgver` to match, builds it with `makepkg`, installs it; `pacman` must report the version | The AUR head builds from the release tarball and installs | `pkgver` mismatch: the AUR step did not push (read its log). A build failure: the tarball, or an Arch update between releases |
| `verify-apt` | `ubuntu:22.04` (gpgv), `debian:13` (sqv), `ubuntu:26.04` (gpgv) | Runs the commands `apt-client.sh` prints, which are the users' commands. Waits up to 1500 s for each suite to list the version (30 s apart), checks the served key against `apt-key-fingerprints.txt`, installs exactly that version, then runs `smoke-test.sh` | A user on each distribution can install from the live site with the documented commands, and the key is the recorded one | Version missing after the wait: the push did not happen or Pages still serves old files. Fingerprint mismatch: the site serves another key. Verifier mismatch: an image's apt changed (the matrix names the verifier, and a run asserts it) |

Neither COPR nor the AUR job runs the smoke test; the install smoke tests that ran on the draft
cover the package. The COPR and AUR packages are built again from source, so their bytes are not the
tested bytes; only the GitHub assets and the apt packages are.

## When something goes wrong

**Run a channel again.** `gh workflow run publish.yml --ref main -f tag=<tag> -f dry_run=false`. It
asks for an approval again. It always runs all three channels; each is safe to repeat: apt pushes
only if the files differ, the AUR commits only if the generated tree differs from its head, COPR
skips a project that has a succeeded build of the version, waits for one that is running, and builds
again after a failed or cancelled one. The verification jobs run after it. The final step of a
failed run names the steps that failed.

**Older tags overwrite newer state.** The AUR step compares with the AUR head only, so a dispatch for
an older tag pushes that tag's `PKGBUILD` over a newer one. That is how you roll the AUR back, and a
mistake if you meant otherwise. Dispatch the newest published tag for a plain re-run.

**A failed channel.** Read the step's log; `aggregate.sh` lists what failed. Preflight failures name
the credential and its renewal step. A COPR failure names the project.

**The apt step refuses a deb.** Before anything is signed it reads every `.deb` of the releases it
uses with `dpkg-deb -f` (control file only, no maintainer script runs) and stops, naming the file and
what the control file says, unless all of these hold:

- `Package` equals the package in the file's name, and is one of the packages `packaging/debian/control`
  declares (`proton-drive-sync-engine`, `proton-sync-nautilus`, `proton-sync-nemo`). `apt-ftparchive`
  writes `Packages` from the control file, so a deb named for this project whose control file says
  `Package: sudo` would otherwise be offered to apt users as `sudo`.
- `Version` equals the release's deb version (`version.sh spell rpm <version>` plus the revision in
  the name: `0.1.0-beta.1` is `0.1.0~beta.1-1` in the control file) and `Architecture` equals the
  name's (`amd64` or `all`; nothing else is published).
- A field that is missing, or a file that is not a deb, is a refusal too.

The remedy is a release whose debs are right: nothing from the release is published to apt. A release
that is already published and wrong is withdrawn (below) and fixed forward with the next version. The
check needs `dpkg-deb` (part of `dpkg`, on the runner and in every Debian and Ubuntu image); the step
stops at once if it is missing.

**The tag is not in the listing.** The apt step lists the published releases and stops with "`<tag>`
is not among the published releases GitHub lists" when the tag it was started for is not one of them:
a tree without it would be pushed and recorded as `ok`, and `verify-apt` would then fail only after
its 1500 s wait. The usual cause is the listing lagging a release that was just published. Wait a
minute and run the step again (`gh workflow run publish.yml --ref main -f tag=<tag> -f dry_run=false`).
If it still fails, the release is a draft, was deleted, or its tag is not the one you dispatched; look
at the release page. Two cases pass on purpose: a tag in `APT_WITHDRAWN` (the step logs "this is a
deliberate rebuild without it"; see "Withdraw a version from apt" for what that does),
and a published tag older than the newest three, which is in the listing and is simply outside the
window.

**Withdraw a version from apt.** The apt tree is rebuilt from the published releases on every run.

1. Set the repository variable `APT_WITHDRAWN` to the tags to leave out (space or comma separated,
   each a release tag). A name that is not a release tag stops the run.
2. Dispatch a real run for **the newest tag that is not in `APT_WITHDRAWN`**:

   ```sh
   gh variable set APT_WITHDRAWN --repo osirison/proton-drive-sync-engine --body 'v0.2.0-beta.2'
   gh workflow run publish.yml --ref main -f tag=v0.2.0-beta.3 -f dry_run=false
   ```

   The `apt` step rebuilds the tree without the withdrawn tags and pushes it, and `verify-apt` then
   finds the dispatched version and passes.

Dispatching the withdrawn tag itself also works for apt: the `apt` step lets it through on purpose (it
logs "this is a deliberate rebuild without it") and rebuilds the tree without it. But `verify-apt`
then waits its full 1500 s for a version the tree no longer holds, and the run ends red.

What a dispatch does to the other channels: the AUR step compares with the AUR head only, and the
COPR step skips a project that already has a succeeded build of the version. A dispatch touches only
the channels its own tag routes to. So when the withdrawn version is not the newest, dispatching the
newest tag changes neither the AUR nor COPR. When it **is** the newest release, you choose:

- **The next older tag (the default above).** The run is green, but it also pushes that older tag's
  `PKGBUILD` to the AUR, which rolls the AUR package back to that version. Only the channels that tag
  routes to are touched: if the withdrawn release is a final and the next older one is a pre-release,
  only the beta AUR package changes, and the stable AUR package keeps the withdrawn final. COPR is
  skipped (the older version already has its build), so COPR keeps the withdrawn build until you
  remove it by hand.
- **The withdrawn tag itself.** apt only: the AUR head already equals that tag's `PKGBUILD` and COPR
  already has its build, so neither changes, and `verify-aur` and `verify-copr` pass. The run ends
  red at `verify-apt`, after 1500 s. That is your choice to make, and the red run is the cost.

Each suite keeps the newest three, so withdrawing one lets the next older release back into the
window. The variable works whether or not releases are immutable (see "Immutable releases"): it
changes what the tree is built from, not the release. The release and its assets stay on GitHub;
removing them is your choice. Users who installed it keep it.

**Withdraw a version from COPR or the AUR.** No script does it. Remove the build in COPR by hand, or
push the previous `PKGBUILD` to the AUR (a dispatch for the previous tag does that, see above), and
fix forward with the next pre-release number: lowering a version later needs an `epoch`.

**The Pages cache window.** GitHub Pages serves every file with `Cache-Control: max-age=600`. For up
to ten minutes after a push a client can get the new `InRelease` with the old `Packages` and see a
hash mismatch from `apt update`. `verify-apt.sh` treats a failed `apt update` as "not there yet" and
polls through it, which is why its wait is 1500 seconds. There is no `by-hash` directory; that is a
known gap. A user who hits the mismatch tries again later.

**Two tags at once.** The concurrency group is per tag, so two different tags published at the same
moment can race on the force-push to `gh-pages`. Dispatch either tag again; the tree is rebuilt from
the published releases. (Within one tag, GitHub keeps one waiting run: a third run replaces the
waiting second.)

**What decides "changed" on apt.** The tree is a pure function of the published releases: a final
release enters both suites, a pre-release only `beta`. Releases are ordered by version
(`packaging/version.sh sort-key`), never by date; `beta` holds the newest three of all of them and
`stable` the newest three finals, and `stable` does not exist until a final release is published.
`gh-pages` is pushed only when the pool, the `Packages` files, the keys or `.nojekyll` differ, a
`Release` file differs by more than its `Date:` line, or a signature already on `gh-pages` does not
verify (`gpgv`, against the keys of the new tree). Signatures differ on every run, so their bytes
decide nothing, but a damaged or foreign `InRelease` or `Release.gpg` is repaired by the next run
for the same releases, which logs "the signatures on gh-pages do not verify".

**A killed step.** The `apt` step and preflight import the signing key into a throwaway GnuPG home
`/tmp/pdse-gpg.*` and remove it when they exit. A step killed outright (a timeout, `SIGKILL`) cannot,
so `credentials.sh remove`, which runs under `if: always()`, also deletes `pdse-gpg.*` homes and logs
in `/tmp` that belong to the runner's user. Do not run `credentials.sh remove` on a machine where
another signing run is in progress.

## Rotate the signing key

A user holds the key as a file. Dropping the old key at once breaks every user who has not fetched
the new one, so the old key stays for one release. The sequence, each step before the next:

1. Make the new key as above, offline. Keep the old key.
2. In one pull request, list **both** fingerprints in `packaging/release/apt-key-fingerprints.txt`.
   Merge it. The tag you cut next carries it.
3. Set `APT_SIGNING_KEY` to the old key's secret block followed by the new one's (the two armored
   files joined, `cat old.asc new.asc`). Between steps 2 and 3, and whenever the secret and the file
   disagree, `apt-publish.sh` signs nothing. Do 2 and 3 close together, before the tag.
4. Cut a release. Its `InRelease` and `Release.gpg` carry both signatures, and the published key file
   holds both keys. `verify-apt` accepts it only if the served key file has exactly the two recorded
   fingerprints.
5. Tell users to fetch the key again (the `curl ... | sudo tee` line of the apt instructions). apt
   does not fetch a new key by itself. How and when to tell them is your choice.
6. In a later pull request, remove the old fingerprint from the file, set `APT_SIGNING_KEY` to the new
   key only, and cut the next release. Users who still hold only the old key then fail at
   `apt update` until they fetch the new one.

`apt-rotation-check.sh` proves steps 4 to 6 on the client side with the distribution's own apt: with a
dual-signed repository the old key alone and the new key alone both work, with a new-key repository
the new key works and the old key alone fails. It runs only in a container, as root: the `apt-repo`
job of `packages.yml` runs it on every package build, with throwaway keys made by
`apt-local-fixture.sh`. Do not run it on your machine.

If the signing key is lost or leaked, the overlap is not available: make a new key, do steps 2, 3 and
6 in one change, and every user must fetch the new key.

## Scripts this page relies on

`packaging/version.sh`, `packaging/release/gate.sh`, `packaging/release/draft.sh`,
`packaging/release/route.sh`, `packaging/release/tag-checkout.sh`, `packaging/release/credentials.sh`,
`packaging/release/preflight.sh`, `packaging/release/token-expiry.sh`,
`packaging/release/apt-publish.sh`, `packaging/release/apt-client.sh`,
`packaging/release/apt-key-fingerprints.txt`, `packaging/release/apt-local-fixture.sh`,
`packaging/release/apt-rotation-check.sh`, `packaging/release/aur-publish.sh`,
`packaging/release/copr.sh`, `packaging/release/aggregate.sh`, `packaging/release/verify-channel.sh`,
`packaging/release/verify-apt.sh`, `packaging/release/verify-copr.sh`,
`packaging/release/verify-aur.sh`, `packaging/release/known_hosts`,
`packaging/release-notes.md`.
