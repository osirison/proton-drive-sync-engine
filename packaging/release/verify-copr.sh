#!/usr/bin/env bash
# The COPR check, run INSIDE a clean Fedora container as root (openspec change
# add-linux-release-pipeline, task 6.6, design D12). verify-channel.sh starts it; the container
# image is the chroot being checked (one job per supported Fedora release).
#
#   packaging/release/verify-copr.sh <upstream version> <channel>...
#
# For each routed channel it does what a user does: `dnf copr enable <owner>/<project>`, then
# installs the package at the release's exact version, requires rpm to report exactly that version,
# and removes both again before the next channel. The version is pinned in the install request
# (name-version), so a project that lacks it fails the job instead of installing a different one, and
# a later release in the same project cannot make a good release look wrong. Any release number
# (-1.fc43, -2.fc43) is accepted: only the upstream spelling (version.sh spell rpm) must match.
#
# It reads only public endpoints (COPR's repository, Fedora's mirrors). Needs COPR_OWNER (publish.yml's
# env: names the namespace). COPR publishes the repository when the build finishes, which the publish
# step has already waited for; the install is still tried again for VERIFY_COPR_WAIT_SECS (default
# 600) every VERIFY_COPR_POLL_SECS (default 30), because dnf caches repository metadata.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

[[ $# -ge 2 ]] || die "usage: verify-copr.sh <upstream version> <channel>..."
upstream="$1"
shift
[[ -n "${COPR_OWNER-}" ]] || die "COPR_OWNER is not set: publish.yml's env: names the COPR namespace"
rpm_version="$("$version_sh" spell rpm "$upstream")" || exit 1
wait_secs="${VERIFY_COPR_WAIT_SECS:-600}"
poll_secs="${VERIFY_COPR_POLL_SECS:-30}"
[[ "$wait_secs" =~ ^[0-9]+$ && "$poll_secs" =~ ^[0-9]+$ ]] || die "VERIFY_COPR_WAIT_SECS and VERIFY_COPR_POLL_SECS must be numbers"

echo "verify-copr.sh: installing the dnf copr command" >&2
dnf -y install 'dnf-command(copr)' >/dev/null

for channel in "$@"; do
  project="${COPR_OWNER}/$(channel_name "$channel")"
  echo "verify-copr.sh: == $channel: COPR project $project, version $rpm_version ==" >&2
  dnf -y copr enable "$project" || die "cannot enable the COPR project $project"
  deadline=$((SECONDS + wait_secs))
  until dnf -y --refresh install "${PACKAGE_BASE}-${rpm_version}"; do
    ((SECONDS < deadline)) ||
      die "COPR project $project has no ${PACKAGE_BASE}-${rpm_version} for this Fedora release after ${wait_secs}s: it is missing from the channel"
    echo "verify-copr.sh: not installable yet; trying again in ${poll_secs}s" >&2
    sleep "$poll_secs"
  done
  got="$(rpm -q --qf '%{VERSION}' "$PACKAGE_BASE")"
  [[ "$got" == "$rpm_version" ]] || die "rpm reports ${PACKAGE_BASE} $got installed from $project, expected $rpm_version"
  echo "verify-copr.sh: $project: installed ${PACKAGE_BASE} $got" >&2
  dnf -y remove "$PACKAGE_BASE"
  dnf -y copr disable "$project"
done
echo "verify-copr.sh: ok: $* at $rpm_version" >&2
