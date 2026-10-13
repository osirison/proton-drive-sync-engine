#!/usr/bin/env bash
# The publish job's AUR step (openspec change add-linux-release-pipeline, task 6.4, design D8).
#
#   packaging/release/aur-publish.sh <tag>
#
# Needs gh, jq, git, GITHUB_REPOSITORY, PUBLISH_CHANNELS and PUBLISH_DRY_RUN (route.sh sets the
# last two), AUR_SSH_KEY_FILE, and AUR_GIT_NAME and AUR_GIT_EMAIL (the identity of the commits; the
# workflow sets them, this file holds no name or address).
#
# What it does:
#   1. downloads the release's tarball and SHA256SUMS and unpacks the tarball. The AUR tree is made
#      by the packaging/arch/aur.sh INSIDE that tarball, from its PKGBUILD: the tagged bytes, not
#      whatever the workflow was checked out at (a dispatch from main for an older tag would
#      otherwise publish main's PKGBUILD);
#   2. runs that aur.sh for each routed channel. aur.sh needs makepkg, which the runner lacks, so by
#      default it runs in an archlinux:base-devel container (aur-container.sh) that is given the
#      work directory and nothing else. AUR_GENERATOR=host runs it directly (the tests do);
#   3. for each channel clones ssh://aur@aur.archlinux.org/<package>.git (AUR_REMOTE_BASE replaces
#      the part before the package name), replaces the clone's files with the three generated ones
#      (PKGBUILD, .SRCINFO, the install script) and commits and pushes to master ONLY if git sees a
#      difference. An empty clone is a package that does not exist yet: the first push creates it.
#
# A re-run for the same tag therefore makes no commit. A dry run does everything up to the push
# (the download, the tree, the clone) and logs whether it would commit; it pushes nothing.
# The step is recorded as `aur`; a channel that fails is named at the end and the others still run.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

[[ $# -eq 1 ]] || die "usage: aur-publish.sh <tag>"
tag="$1"
record_on_exit aur
require_dry_run
require_channels
[[ -n "${AUR_SSH_KEY_FILE-}" && -s "$AUR_SSH_KEY_FILE" ]] || die "AUR_SSH_KEY_FILE is not a readable file: the secret AUR_SSH_KEY is missing"
[[ -n "${AUR_GIT_NAME-}" && -n "${AUR_GIT_EMAIL-}" ]] || die "AUR_GIT_NAME and AUR_GIT_EMAIL are not set: the commits need an identity"
upstream="$(tag_upstream "$tag")" || exit 1
base="${AUR_REMOTE_BASE:-ssh://aur@aur.archlinux.org}"
generator="${AUR_GENERATOR:-docker}"
case "$generator" in docker | host) ;; *) die "AUR_GENERATOR is '$generator': docker or host" ;; esac

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
tarball="${PACKAGE_BASE}-${upstream}.tar.gz"

# 1. The release's own tarball, checked against the release's own SHA256SUMS.
mkdir -p "$work/assets" "$work/src"
gh release download "$tag" --repo "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}" --dir "$work/assets" \
  --pattern "$tarball" --pattern SHA256SUMS ||
  die "cannot download $tarball and SHA256SUMS from the release $tag"
[[ -s "$work/assets/$tarball" && -s "$work/assets/SHA256SUMS" ]] || die "the release $tag lacks $tarball or SHA256SUMS"
listed="$(awk -v name="$tarball" '$2 == name || $2 == "*" name { print $1 }' "$work/assets/SHA256SUMS")"
actual="$(sha256sum "$work/assets/$tarball" | awk '{print $1}')"
[[ -n "$listed" && "$listed" == "$actual" ]] ||
  die "SHA256SUMS lists '${listed}' for $tarball but the file's SHA-256 is $actual"
tar -xzf "$work/assets/$tarball" -C "$work/src" --strip-components=1
[[ -f "$work/src/packaging/arch/aur.sh" ]] || die "$tarball holds no packaging/arch/aur.sh"

# 2. The trees.
if [[ "$generator" == docker ]]; then
  mkdir -p "$work/trees"
  docker run --rm --network none -e "HOST_UID=$(id -u)" \
    -v "$work:/work" -v "$publish_here/aur-container.sh:/aur-container.sh:ro" \
    archlinux:base-devel bash /aur-container.sh "${channels[@]}"
else
  mkdir -p "$work/trees"
  for channel in "${channels[@]}"; do
    bash "$work/src/packaging/arch/aur.sh" "$channel" "$work/assets" "$work/trees/$channel"
  done
fi

# 3. Compare with the AUR head, and push when they differ.
export GIT_SSH_COMMAND
GIT_SSH_COMMAND="$(ssh_command_for "$AUR_SSH_KEY_FILE")"
export GIT_TERMINAL_PROMPT=0
failed=()

publish_channel() {
  local channel="$1" tree="$work/trees/$1" expected package clone
  expected="$(channel_name "$channel")"
  [[ -f "$tree/PKGBUILD" && -f "$tree/.SRCINFO" && -f "$tree/${PACKAGE_BASE}.install" ]] ||
    { echo "aur-publish.sh: the $channel tree lacks PKGBUILD, .SRCINFO or the install script" >&2; return 1; }
  package="$(sed -n 's/^pkgbase = //p' "$tree/.SRCINFO" | head -n1)"
  [[ "$package" == "$expected" ]] ||
    { echo "aur-publish.sh: the $channel tree is for the package '$package', not '$expected'" >&2; return 1; }
  clone="$work/clone-$channel"
  echo "aur-publish.sh: $channel -> AUR package $package ($base/$package.git)" >&2
  git clone --quiet "$base/$package.git" "$clone" || { echo "aur-publish.sh: cannot clone $base/$package.git" >&2; return 1; }
  # The AUR takes master only, whatever HEAD of the clone says. A package that exists is continued
  # from its master; an empty clone has no branch yet and takes the name here.
  if git -C "$clone" rev-parse --verify --quiet refs/remotes/origin/master >/dev/null; then
    git -C "$clone" checkout -q -B master origin/master || return 1
  else
    git -C "$clone" symbolic-ref HEAD refs/heads/master || return 1
  fi
  # The clone holds exactly the generated files: what was there is removed, so a file the new tree
  # no longer has goes away. (set -e does not reach into a function run under `if`, so every step
  # that can fail says `|| return 1`.)
  git -C "$clone" ls-files -z | xargs -0 -r git -C "$clone" rm -q -- || return 1
  cp -p -- "$tree/PKGBUILD" "$tree/.SRCINFO" "$tree/${PACKAGE_BASE}.install" "$clone/" || return 1
  git -C "$clone" add -A || return 1
  if [[ -z "$(git -C "$clone" status --porcelain)" ]]; then
    echo "aur-publish.sh: $package: the generated tree equals the AUR head: no commit" >&2
    return 0
  fi
  if [[ "$PUBLISH_DRY_RUN" == true ]]; then
    echo "aur-publish.sh: DRY RUN: $package differs from the AUR head: would commit and push to master" >&2
    git -C "$clone" status --short >&2
    return 0
  fi
  git -C "$clone" -c "user.name=$AUR_GIT_NAME" -c "user.email=$AUR_GIT_EMAIL" -c commit.gpgsign=false \
    commit -q -m "Update to ${upstream}" || return 1
  git -C "$clone" push --quiet origin HEAD:refs/heads/master || return 1
  echo "aur-publish.sh: $package: pushed ${upstream}" >&2
}

for channel in "${channels[@]}"; do
  if ! (publish_channel "$channel"); then
    failed+=("$channel")
  fi
done

if ((${#failed[@]} > 0)); then
  die "the AUR step failed for: ${failed[*]}"
fi
finish_recording
