#!/usr/bin/env bash
# One verification job's body (openspec change add-linux-release-pipeline, task 6.6, design D12):
# runs a channel's check in a clean container of the distribution being checked.
#
#   packaging/release/verify-channel.sh <copr|aur|apt> <container image>
#
# Runs after route.sh and tag-checkout.sh, which left PUBLISH_TAG and PUBLISH_CHANNELS in the
# environment and the TAG's commit checked out, so the checks that run are the tag's own (the same
# old-tag rule as the publish job). The checkout is mounted read-only at /repo in the container, and
# nothing else is: no secret exists in this job, and the container reads only public endpoints.
# A pre-release is routed to beta only, so only beta is checked; a final release checks both.
#
# Passed through to the container when set: COPR_OWNER, VERIFY_EXPECT_VERIFIER, VERIFY_APT_WAIT_SECS,
# VERIFY_APT_POLL_SECS, VERIFY_COPR_WAIT_SECS, VERIFY_COPR_POLL_SECS.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

[[ $# -eq 2 ]] || die "usage: verify-channel.sh <copr|aur|apt> <container image>"
family="$1"
image="$2"
case "$family" in copr | aur | apt) ;; *) die "'$family' is not a family (copr, aur or apt)" ;; esac
[[ -n "${PUBLISH_TAG-}" ]] || die "PUBLISH_TAG is not set: route.sh sets it first"
require_channels
upstream="$(tag_upstream "$PUBLISH_TAG")" || exit 1

environment=()
for name in COPR_OWNER VERIFY_EXPECT_VERIFIER VERIFY_APT_WAIT_SECS VERIFY_APT_POLL_SECS VERIFY_COPR_WAIT_SECS VERIFY_COPR_POLL_SECS; do
  if [[ -n "${!name-}" ]]; then environment+=(-e "$name=${!name}"); fi
done

echo "verify-channel.sh: $family, $PUBLISH_TAG (version $upstream), channels: ${channels[*]}, image $image" >&2
exec docker run --rm -v "$publish_root:/repo:ro" "${environment[@]}" "$image" \
  bash "/repo/packaging/release/verify-$family.sh" "$upstream" "${channels[@]}"
