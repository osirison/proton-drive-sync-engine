#!/usr/bin/env bash
# The publish job's gate, before any credential is written (openspec change
# add-linux-release-pipeline, design D11/D12).
#
#   packaging/release/tag-checkout.sh <tag>
#
# Run in a checkout with the whole history (fetch-depth: 0), after route.sh has validated the tag.
# A dispatch runs whatever branch it was started from, so this step does two things:
#
#   1. checks out the tag's commit, so every later step runs the scripts and the packaging of the
#      tag and not of the dispatched branch;
#   2. runs gate.sh on it: the tag is v plus the Cargo.toml version, the commit is an ancestor of
#      origin/main (fetched here by name; the gate fails closed if it cannot be read), and
#      version.sh check passes.
#
# A tag made by hand on a commit that is not on main therefore fails here, before the secrets are
# written. When GH_TOKEN is set it is given to git for the two fetches through the environment, not
# the command line. The body is a function and the script ends in `exit`, so the checkout replacing
# this very file cannot change what runs.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

main() {
  [[ $# -eq 1 ]] || die "usage: tag-checkout.sh <tag>"
  local tag="$1" commit
  tag_upstream "$tag" >/dev/null || exit 1
  if [[ -n "${GH_TOKEN-}" ]]; then
    export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader
    GIT_CONFIG_VALUE_0="AUTHORIZATION: basic $(printf 'x-access-token:%s' "$GH_TOKEN" | base64 -w0)"
    export GIT_CONFIG_VALUE_0
  fi
  export GIT_TERMINAL_PROMPT=0
  git -C "$publish_root" fetch --no-tags --quiet origin +refs/heads/main:refs/remotes/origin/main ||
    die "cannot fetch main"
  git -C "$publish_root" fetch --no-tags --quiet origin "+refs/tags/${tag}:refs/tags/${tag}" ||
    die "cannot fetch the tag $tag"
  commit="$(git -C "$publish_root" rev-parse --verify --quiet "refs/tags/${tag}^{commit}")" ||
    die "the tag $tag does not name a commit"
  git -C "$publish_root" checkout --quiet --detach "$commit" || die "cannot check out $commit"
  echo "tag-checkout.sh: checked out $tag ($commit); running its gate" >&2
  exec "$publish_root/packaging/release/gate.sh" "$tag"
}

main "$@"
exit 1
