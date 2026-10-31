#!/usr/bin/env bash
# The apt check, run INSIDE a clean Debian or Ubuntu container as root (openspec change
# add-linux-release-pipeline, tasks 6.3 and 6.6, design D10/D12). Two callers use it unchanged:
#
#   * the post-publish verification job, against the live repository (verify-channel.sh runs it);
#   * the pull-request job `apt-repo` in packages.yml, against a tree it built and serves locally.
#
#   packaging/release/verify-apt.sh <upstream version> <suite>...
#
# For each suite, in order, it does what a user does, from the text apt-client.sh prints (the one
# definition of it): put the repository's key where apt reads it, write the .sources file, apt
# update. Then it requires that the suite lists a version spelled for <upstream version> (the revision
# is whatever the release has: -1, -2, ...), and for the FIRST suite it also
#   * checks the key the repository serves against the fingerprints recorded for it,
#   * installs exactly that version and requires dpkg to report exactly it,
#   * runs packaging/smoke-test.sh on the install.
# A later suite is only checked for the version: the package is already installed.
#
# It fails (exit 1) when a suite does not list the version, so a job pointed at a channel that is
# missing the version fails; it waits first, because GitHub Pages caches every file for 10 minutes
# (Cache-Control: max-age=600) and a page the publish step just pushed may not be served yet.
# Known gap: during that window apt can see the new InRelease with the old Packages and report a
# hash mismatch; the wait treats a failed `apt update` as "not there yet". By-hash is the fix.
#
# Environment (all optional):
#   VERIFY_APT_URI, VERIFY_APT_KEY_URL   another repository (default: the public one)
#   VERIFY_APT_WAIT_SECS                 how long to wait for the version (default 1500, above 10 minutes)
#   VERIFY_APT_POLL_SECS                 pause between tries (default 30)
#   VERIFY_EXPECT_VERIFIER               gpgv or sqv: require that this is the only signature verifier
#                                        installed before the first update (see below)
#   APT_FINGERPRINT_FILE                 another fingerprint file (the pull-request job's stand-in key)
#   APT_CLIENT_ROOT                      tests only: prefix of the two /etc directories
#
# THE VERIFIER. apt checks signatures with gpgv, or with sqv where the distribution ships that
# instead (Debian 13; checked 2026-10-13: Ubuntu 22.04 and 26.04 use gpgv). The script prints which of the two packages are installed before the first
# `apt update` of the repository, so the log says which one verified; with VERIFY_EXPECT_VERIFIER it
# fails when that is not so. The prerequisites it installs first (sudo, curl, ca-certificates) pull in
# neither; gnupg, which brings gpgv, is installed only AFTER the update, for the fingerprint check.
#
# NOTE: installing curl first means smoke-test.sh's "curl is on PATH" check passes without proving
# the package recommends it; the smoke-deb jobs in packages.yml cover that.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
version_sh="$here/../version.sh"
smoke="$here/../smoke-test.sh"
client="$here/apt-client.sh"
export DEBIAN_FRONTEND=noninteractive

log() { echo "verify-apt.sh: $*" >&2; }
die() { log "$*"; exit 1; }

[[ $# -ge 2 ]] || die "usage: verify-apt.sh <upstream version> <suite>..."
upstream="$1"
shift
suites=("$@")
want_prefix="$("$version_sh" spell rpm "$upstream")" || exit 1

wait_secs="${VERIFY_APT_WAIT_SECS:-1500}"
poll_secs="${VERIFY_APT_POLL_SECS:-30}"
[[ "$wait_secs" =~ ^[0-9]+$ && "$poll_secs" =~ ^[0-9]+$ ]] || die "VERIFY_APT_WAIT_SECS and VERIFY_APT_POLL_SECS must be numbers"

print_args=()
[[ -z "${VERIFY_APT_URI-}" ]] || print_args+=(--uri "$VERIFY_APT_URI")
[[ -z "${VERIFY_APT_KEY_URL-}" ]] || print_args+=(--key-url "$VERIFY_APT_KEY_URL")

# Is a package installed?
installed() { dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q '^install ok installed$'; }

# 0. What the documented text needs and a bare container lacks.
log "preparing the container (sudo, curl and ca-certificates; the documented text uses them)"
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends sudo curl ca-certificates >/dev/null

# 1. Which verifier will check the signatures.
have_gpgv=no
have_sqv=no
if installed gpgv; then have_gpgv=yes; fi
if installed sqv; then have_sqv=yes; fi
log "signature verifiers installed before the first update of the repository: gpgv=$have_gpgv sqv=$have_sqv"
case "${VERIFY_EXPECT_VERIFIER-}" in
  "") ;;
  gpgv) [[ "$have_gpgv" == yes && "$have_sqv" == no ]] || die "expected gpgv alone to verify, found gpgv=$have_gpgv sqv=$have_sqv" ;;
  sqv) [[ "$have_sqv" == yes && "$have_gpgv" == no ]] || die "expected sqv alone to verify, found gpgv=$have_gpgv sqv=$have_sqv" ;;
  *) die "VERIFY_EXPECT_VERIFIER is '${VERIFY_EXPECT_VERIFIER}': gpgv or sqv" ;;
esac

# The versions a suite lists for this release: apt-cache madison prints
# "name | version | origin suite/component arch Packages".
versions_in_suite() {
  local suite="$1"
  apt-cache madison proton-drive-sync-engine 2>/dev/null |
    awk -F' *[|] *' -v prefix="$want_prefix" -v suite="$suite" '
      $1 ~ /^ *proton-drive-sync-engine *$/ && $3 ~ (" " suite "/main ") && index($2, prefix "-") == 1 { print $2 }' |
    LC_ALL=C sort -uV
}

# wait_for_version SUITE: sets `found` to the newest version of this release in SUITE, or fails once
# the wait is over.
wait_for_version() {
  local suite="$1" deadline=$((SECONDS + wait_secs)) list
  while :; do
    if apt-get update -qq >/dev/null 2>&1; then
      list="$(versions_in_suite "$suite")"
      if [[ -n "$list" ]]; then
        found="$(tail -n1 <<<"$list")"
        return 0
      fi
      log "suite $suite does not list a $want_prefix-<revision> package yet"
    else
      log "apt update failed (a page still being served from the cache, or the repository is wrong)"
    fi
    ((SECONDS < deadline)) || break
    sleep "$poll_secs"
  done
  return 1
}

first=true
for suite in "${suites[@]}"; do
  log "== suite $suite =="
  # 2. The documented commands, verbatim, but for the last line (we install one exact version).
  bash -euo pipefail <("$client" print "$suite" "${print_args[@]}" --no-install)
  found=""
  wait_for_version "$suite" ||
    die "the suite $suite of the repository lists no version $want_prefix-<revision> of proton-drive-sync-engine after waiting ${wait_secs}s: it is missing from the channel"
  log "suite $suite lists proton-drive-sync-engine $found"
  if [[ "$first" == true ]]; then
    # 3. The key the repository serves is the key whose fingerprint is recorded for it.
    log "installing gnupg to read the key's fingerprint"
    apt-get install -y -qq --no-install-recommends gnupg >/dev/null
    expected="$("$client" fingerprints)" || exit 1
    actual="$(bash -euo pipefail <("$client" fingerprint-command) | LC_ALL=C sort -u)"
    [[ "$actual" == "$expected" ]] ||
      die "the key the repository serves has the fingerprint(s) '${actual//$'\n'/ }' but '${expected//$'\n'/ }' is recorded"
    log "the key's fingerprint is the recorded one: ${expected//$'\n'/ }"
    # 4. Install exactly that version.
    # With recommends, as a user installs it: that is where secret-tool comes from.
    apt-get install -y "proton-drive-sync-engine=$found" >/dev/null ||
      die "apt could not install proton-drive-sync-engine=$found"
    got="$(dpkg-query -W -f='${Version}' proton-drive-sync-engine)"
    [[ "$got" == "$found" ]] || die "dpkg reports proton-drive-sync-engine $got installed, expected $found"
    log "installed proton-drive-sync-engine $got"
    # 5. The smoke test (design D13) on the installed package.
    bash "$smoke" "$upstream" deb || die "the smoke test failed"
    first=false
  fi
done
log "ok: ${suites[*]} list $want_prefix-<revision>, installed $got"
