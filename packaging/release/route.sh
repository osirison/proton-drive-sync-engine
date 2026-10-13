#!/usr/bin/env bash
# The publish workflow's first step (openspec change add-linux-release-pipeline, design D9, D12).
#
#   packaging/release/route.sh <tag>
#   packaging/release/route.sh --prerelease <true|false>     (the function alone; used by the tests)
#
# Prints the channels the release goes to, one per line: a pre-release goes to `beta`, a final
# release to `beta` and `stable`. With a tag it needs gh, jq and GITHUB_REPOSITORY, and reads
# DRY_RUN (empty, true or false).
#
# The flag comes from the API, not from the tag text. The script FAILS CLOSED, printing nothing:
#   - the tag is not a release tag (version.sh);
#   - the API does not answer, or answers with no published release for the tag (a draft is not
#     one: a dispatch for a tag that is still a draft publishes nothing);
#   - `prerelease` is anything but the JSON true or false;
#   - the flag disagrees with the tag (version.sh is-prerelease): a beta published as a final
#     release would otherwise reach the stable channels, and the fix is the release's flag.
#
# It also clears the results directory, and, when GITHUB_ENV is set, hands the later steps the tag,
# the dry-run flag and the channels as PUBLISH_TAG, PUBLISH_DRY_RUN and PUBLISH_CHANNELS. Those are
# validated here, so no later step reads an event field.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

# The one mapping from the flag to the channels.
channels_for_prerelease() {
  case "$1" in
    true) echo beta ;;
    false) printf 'beta\nstable\n' ;;
    *) return 1 ;;
  esac
}

if [[ "${1-}" == --prerelease ]]; then
  [[ $# -eq 2 ]] || die "usage: route.sh --prerelease <true|false>"
  channels_for_prerelease "$2" || die "the pre-release flag is '$2', not true or false: publishing to nothing"
  exit 0
fi

[[ $# -eq 1 ]] || die "usage: route.sh <tag>"
tag="$1"
dry_run="$(normalise_dry_run "${DRY_RUN-}")"
upstream="$(tag_upstream "$tag")" || exit 1

json="$(release_json "$tag")" || die "cannot read the release for $tag from the API: publishing to nothing"
jq -e --arg tag "$tag" '.tag_name == $tag' >/dev/null <<<"$json" ||
  die "the API answered with a release that is not $tag: publishing to nothing"
jq -e '.draft == false' >/dev/null <<<"$json" ||
  die "the release $tag is a draft: a person publishes it first; publishing to nothing"
flag="$(jq -r 'if (.prerelease | type) == "boolean" then .prerelease | tostring else "missing" end' <<<"$json")"
[[ "$flag" == true || "$flag" == false ]] ||
  die "the release $tag has no usable prerelease flag ('$flag'): publishing to nothing"
expected="$("$version_sh" is-prerelease "$upstream")"
[[ "$flag" == "$expected" ]] ||
  die "the release $tag is marked prerelease=$flag but version $upstream says $expected: fix the flag on the release; publishing to nothing"

routed="$(channels_for_prerelease "$flag")" || die "no channels for prerelease=$flag"
list="$(tr '\n' ' ' <<<"$routed")"
list="${list% }"

rm -rf -- "$(results_dir)"
mkdir -p "$(results_dir)"

if [[ "$flag" == true ]]; then kind="a pre-release"; else kind="a final release"; fi
{
  echo "route.sh: $tag (version $upstream) is $kind: channels: $list"
  if [[ "$dry_run" == true ]]; then
    echo "route.sh: DRY RUN: the targets below would be published to; nothing is pushed, built or uploaded"
    for channel in $list; do
      echo "route.sh:   $channel -> COPR project $(channel_name "$channel"), AUR package $(channel_name "$channel")"
    done
  fi
} >&2

if [[ -n "${GITHUB_ENV-}" ]]; then
  {
    echo "PUBLISH_TAG=$tag"
    echo "PUBLISH_DRY_RUN=$dry_run"
    echo "PUBLISH_CHANNELS=$list"
  } >>"$GITHUB_ENV"
fi
echo "$routed"
