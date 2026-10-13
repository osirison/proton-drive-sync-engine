#!/usr/bin/env bash
# The release workflow's first job (openspec change add-linux-release-pipeline, design D3/D12).
#
#   packaging/release/gate.sh <tag>
#
# Run in a checkout of the tagged commit. It refuses, before anything is built:
#
#   1. a tag that is not v plus a version of the accepted shape (version.sh tag-version);
#   2. a tag that is not v plus the root Cargo.toml version (the error names both versions);
#   3. a commit that is not an ancestor of origin/main;
#   4. a tree whose version sites disagree (version.sh check).
#
# The shape and the version are version.sh's: this file keeps no pattern of its own.
#
# The ancestry check FAILS CLOSED. A checkout of a tag push is shallow and has no origin/main until
# the workflow fetches it, and `git merge-base --is-ancestor` answers 0 (is), 1 (is not) and 128
# (a ref it cannot read). Only 0 passes. A missing or unreadable ref, a shallow history that hides
# the answer and any other failure all stop the release; none is read as "fine".
#
# GATE_MAIN_REF names the branch tip to compare with (default origin/main); the tests use it.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
version_sh="$root/packaging/version.sh"
main_ref="${GATE_MAIN_REF:-origin/main}"

die() {
  echo "gate.sh: $*" >&2
  exit 1
}

[[ $# -eq 1 ]] || die "usage: packaging/release/gate.sh <tag>"
tag="$1"

# 1. The tag's shape, and the version it names.
tag_version="$("$version_sh" tag-version "$tag")" || exit 1

# 2. The version the tagged tree states.
tree_version="$("$version_sh" current)" || exit 1
if [[ "$tag_version" != "$tree_version" ]]; then
  die "the tag $tag names version $tag_version, but Cargo.toml at this commit says $tree_version"
fi

# 3. The commit is on main. `^{commit}` resolves a ref (or an annotated tag) to its commit.
commit="$(git -C "$root" rev-parse --verify --quiet "HEAD^{commit}")" ||
  die "cannot resolve the commit to check (HEAD)"
tip="$(git -C "$root" rev-parse --verify --quiet "$main_ref^{commit}")" ||
  die "cannot read $main_ref, so cannot tell whether $commit is on main (fetch it first)"
status=0
git -C "$root" merge-base --is-ancestor "$commit" "$tip" || status=$?
case "$status" in
  0) ;;
  1) die "commit $commit is not an ancestor of $main_ref ($tip): a release is tagged on main" ;;
  *) die "git could not tell whether $commit is an ancestor of $main_ref (exit $status)" ;;
esac

# 4. Every version site agrees with Cargo.toml.
"$version_sh" check

echo "gate.sh: $tag is version $tag_version and commit $commit is on $main_ref."
