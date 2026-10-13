#!/usr/bin/env bash
# Shared by the publish workflow's scripts (openspec change add-linux-release-pipeline, design D9,
# D11, D12). Sourced, never run.
#
# What lives here and nowhere else:
#   - the channel names and what each channel is called in COPR and on the AUR;
#   - the reading of the dry-run flag, the channel list and the tag;
#   - the one place a result is recorded, so the final step can name a channel that did not finish;
#   - the ssh options that pin the two hosts the workflow connects to.
#
# The COPR namespace is NOT here. It is `COPR_OWNER`, set once in publish.yml's `env:`; a script that
# needs it fails when it is unset, so there is no default to drift from the workflow's.
# shellcheck shell=bash

publish_here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
publish_root="$(cd "$publish_here/../.." && pwd)"
version_sh="$publish_root/packaging/version.sh"
script_name="$(basename "${0:-publish}")"

# The package every channel carries; a channel adds a suffix.
PACKAGE_BASE=proton-drive-sync-engine

die() {
  echo "$script_name: $*" >&2
  exit 1
}

# The COPR project and the AUR package of a channel (design D9): the stable channel is the package
# name and the beta channel adds -beta.
channel_name() {
  case "$1" in
    stable) echo "$PACKAGE_BASE" ;;
    beta) echo "${PACKAGE_BASE}-beta" ;;
    *) die "'$1' is not a channel (beta or stable)" ;;
  esac
}

# DRY_RUN as the workflow gives it: empty (a release event has no input) is false. Anything that is
# not exactly true or false stops the run, so a typo is never read as "publish for real".
normalise_dry_run() {
  case "$1" in
    "" | false) echo false ;;
    true) echo true ;;
    *) die "dry_run must be true or false, not '$1'" ;;
  esac
}

# PUBLISH_DRY_RUN, written by route.sh. A script that finds it missing stops instead of guessing.
require_dry_run() {
  case "${PUBLISH_DRY_RUN-}" in
    true | false) ;;
    *) die "PUBLISH_DRY_RUN is '${PUBLISH_DRY_RUN-}': route.sh sets it to true or false first" ;;
  esac
}

# PUBLISH_CHANNELS, written by route.sh: a space-separated list that must be beta, or beta stable.
# Sets the array `channels`. It is called directly, never in $(...) or <(...): a `die` in a subshell
# would end only the subshell and the step would go on with an empty list.
require_channels() {
  local channel seen_beta=false
  channels=()
  [[ -n "${PUBLISH_CHANNELS-}" ]] || die "PUBLISH_CHANNELS is empty: route.sh sets it first"
  for channel in $PUBLISH_CHANNELS; do
    channel_name "$channel" >/dev/null
    [[ "$channel" != beta ]] || seen_beta=true
    channels+=("$channel")
  done
  [[ "$seen_beta" == true ]] || die "PUBLISH_CHANNELS '$PUBLISH_CHANNELS' does not include beta"
}

# The tag, validated by version.sh (the one definition of its shape). Prints the upstream version.
tag_upstream() {
  [[ -n "${1-}" ]] || die "no tag given"
  "$version_sh" tag-version "$1"
}

# The published release for a tag, as the API gives it. A draft or a missing release is an error:
# the lookup by tag answers published releases only.
release_json() {
  [[ -n "${GITHUB_REPOSITORY-}" ]] || die "GITHUB_REPOSITORY is not set"
  gh api "repos/${GITHUB_REPOSITORY}/releases/tags/$1"
}

# Where each step records how it ended. The final step reads these, so a step that was killed, or
# never ran, shows as missing and not as fine.
results_dir() {
  local dir="${PUBLISH_RESULTS_DIR:-}"
  if [[ -z "$dir" && -n "${RUNNER_TEMP-}" ]]; then
    dir="$RUNNER_TEMP/publish-results"
  fi
  [[ -n "$dir" ]] || die "PUBLISH_RESULTS_DIR (or RUNNER_TEMP) is not set"
  echo "$dir"
}

record_result() {
  local dir
  dir="$(results_dir)"
  mkdir -p "$dir"
  printf '%s\n' "$2" >"$dir/$1"
}

# record_on_exit NAME: the step is recorded `failed` unless it reaches `finish_recording`. Any exit,
# including `die`, leaves `failed`.
record_on_exit() {
  recording_name="$1"
  record_result "$recording_name" failed
}

finish_recording() {
  record_result "$recording_name" ok
}

# The ssh command line for a key file, with the two hosts pinned in known_hosts beside this file.
# Host-key checking is on and the user's own files are not read.
ssh_command_for() {
  printf 'ssh -i %q -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=%q -o GlobalKnownHostsFile=/dev/null' \
    "$1" "$publish_here/known_hosts"
}

# ssh with that key and those pins: run_ssh KEY ssh-arguments...
run_ssh() {
  local key="$1"
  shift
  ssh -i "$key" -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes \
    -o "UserKnownHostsFile=$publish_here/known_hosts" -o GlobalKnownHostsFile=/dev/null "$@"
}

# The apt repository (task 5.4) lives beside this one, under the same owner.
apt_repository_url() {
  if [[ -n "${APT_REMOTE_URL-}" ]]; then
    echo "$APT_REMOTE_URL"
    return
  fi
  [[ -n "${GITHUB_REPOSITORY_OWNER-}" ]] || die "GITHUB_REPOSITORY_OWNER is not set"
  echo "git@github.com:${GITHUB_REPOSITORY_OWNER}/${PACKAGE_BASE}-apt.git"
}
