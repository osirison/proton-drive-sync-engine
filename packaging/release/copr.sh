#!/usr/bin/env bash
# The publish job's COPR step (openspec change add-linux-release-pipeline, task 6.5, design D7).
#
#   packaging/release/copr.sh <tag>
#
# Needs copr-cli, gh, jq, GITHUB_REPOSITORY, COPR_OWNER (the COPR namespace; publish.yml sets it, in
# one place), COPR_CONFIG_FILE, PUBLISH_CHANNELS and PUBLISH_DRY_RUN (route.sh sets the last two).
#
# For each routed channel it works on the COPR project <COPR_OWNER>/<channel's name>:
#
#   1. reads the project's builds (`list-packages --with-all-builds --output-format json`) and
#      decides, for THIS version (the RPM spelling version.sh gives, e.g. 0.1.0~beta.1; the release
#      part of COPR's version, such as -1.fc43, is ignored):
#        a build that SUCCEEDED   skip the project: nothing is submitted;
#        a build still running    do not submit a second one: wait for that one;
#        otherwise                submit `copr-cli build --nowait` of the release's source RPM URL;
#   2. when every project is submitted, `copr-cli watch-build` on each project's build IDs. That
#      exits non-zero if a chroot failed or was cancelled, and the project is then failed.
#
# THE CHROOTS. `build` is given no -r, which the CLI documents as "all the project's chroots". The
# chroots are the project's, set when the maintainer creates it (task 5.1: one per non-EOL Fedora
# release), so this script keeps no list that could go stale. `copr-cli get` has no JSON output to
# read them from. Removing a release that reached end of life is done on the project.
#
# THE SKIP RULE reads only the builds `list-packages` returns (the newest ten per package, plus the
# latest and latest succeeded), so a succeeded build of an old version that is no longer among them
# is not seen and the build is submitted again. That costs a build, never a wrong skip.
# A read that fails, or is not a JSON array, fails the project: it is never read as "no builds".
#
# A dry run does every read (the release, the project's builds) and logs what it would do; it never
# runs `build` or `watch-build`.
#
# Each project that fails is named at the end; the others still run. The step is recorded as `copr`.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

[[ $# -eq 1 ]] || die "usage: copr.sh <tag>"
tag="$1"
record_on_exit copr
require_dry_run
require_channels
[[ -n "${COPR_OWNER-}" ]] || die "COPR_OWNER is not set: publish.yml's env: names the COPR namespace"
[[ -n "${COPR_CONFIG_FILE-}" && -s "$COPR_CONFIG_FILE" ]] || die "COPR_CONFIG_FILE is not a readable file: the secret COPR_CONFIG is missing"
upstream="$(tag_upstream "$tag")" || exit 1
rpm_version="$("$version_sh" spell rpm "$upstream")"

copr() { copr-cli --config "$COPR_CONFIG_FILE" "$@"; }

# The release's source RPM: exactly one asset, named for this version, with an https URL. Its name
# carries a dist tag (.fc43) that changes over time, so it is read from the release.
json="$(release_json "$tag")" || die "cannot read the release for $tag from the API"
srpm_url="$(jq -r --arg prefix "${PACKAGE_BASE}-${upstream}-" '
  [.assets[]? | select((.name | endswith(".src.rpm")) and (.name | startswith($prefix))) | .browser_download_url] as $urls
  | if ($urls | length) == 1 then $urls[0] else "" end' <<<"$json")"
[[ "$srpm_url" == https://* ]] ||
  die "the release $tag must hold exactly one source RPM named ${PACKAGE_BASE}-${upstream}-*.src.rpm, with an https download URL"

failed=()
declare -A build_ids=()

for channel in "${channels[@]}"; do
  project="${COPR_OWNER}/$(channel_name "$channel")"
  echo "copr.sh: $channel -> COPR project $project (version $rpm_version)" >&2
  if ! packages="$(copr list-packages "$project" --with-all-builds --with-latest-succeeded-build --with-latest-build --output-format json)"; then
    echo "copr.sh: cannot read the builds of $project" >&2
    failed+=("$project")
    continue
  fi
  if ! jq -e 'type == "array"' >/dev/null 2>&1 <<<"$packages"; then
    echo "copr.sh: the builds of $project are not a JSON list" >&2
    failed+=("$project")
    continue
  fi
  # Every build of this package at this version: "<id> <state>" per line.
  builds="$(jq -r --arg name "$PACKAGE_BASE" --arg version "$rpm_version" '
    [ .[] | select(.name == $name)
      | (.builds[]?, .latest_build?, .latest_succeeded_build?)
      | select(. != null)
      | select((.source_package.version // "") | sub("-[^-]*$"; "") == $version)
      | "\(.id) \(.state)" ] | unique | .[]' <<<"$packages")"
  if grep -q ' succeeded$' <<<"$builds"; then
    echo "copr.sh: $project already has a succeeded build of $rpm_version: nothing to submit" >&2
    continue
  fi
  # States that are over and are not a success; anything else is a build still under way.
  ids="$(awk '$2 != "failed" && $2 != "canceled" && $2 != "skipped" && $2 != "forked" { print $1 }' <<<"$builds" | tr '\n' ' ')"
  if [[ -n "${ids// /}" ]]; then
    echo "copr.sh: $project already has an unfinished build of $rpm_version (${ids% }): waiting for it, not submitting another" >&2
    build_ids[$project]="${ids% }"
    continue
  fi
  if [[ "$PUBLISH_DRY_RUN" == true ]]; then
    echo "copr.sh: DRY RUN: would run 'copr-cli build --nowait $project $srpm_url' (all of the project's chroots) and watch it" >&2
    continue
  fi
  if ! output="$(copr build --nowait "$project" "$srpm_url" 2>&1)"; then
    echo "copr.sh: 'copr-cli build' failed for $project: $output" >&2
    failed+=("$project")
    continue
  fi
  echo "$output" >&2
  submitted="$(sed -n 's/^Created builds\?: *\([0-9][0-9 ]*\)$/\1/p' <<<"$output" | tail -n1)"
  submitted="${submitted%"${submitted##*[![:space:]]}"}"
  if [[ -z "$submitted" ]]; then
    echo "copr.sh: 'copr-cli build' printed no build ID for $project, so nothing can be waited on" >&2
    failed+=("$project")
    continue
  fi
  build_ids[$project]="$submitted"
done

# All the builds are in the queue; now wait for each, one project after the other.
for project in "${!build_ids[@]}"; do
  if [[ "$PUBLISH_DRY_RUN" == true ]]; then
    echo "copr.sh: DRY RUN: would watch builds ${build_ids[$project]} of $project" >&2
    continue
  fi
  echo "copr.sh: waiting for $project builds ${build_ids[$project]}" >&2
  # shellcheck disable=SC2086  # the IDs are digits separated by spaces
  if ! copr watch-build ${build_ids[$project]}; then
    echo "copr.sh: a build of $project failed" >&2
    failed+=("$project")
  fi
done

if ((${#failed[@]} > 0)); then
  die "COPR failed for: ${failed[*]}"
fi
finish_recording
echo "copr.sh: done." >&2
