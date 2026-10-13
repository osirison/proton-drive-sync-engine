#!/usr/bin/env bash
# What a client does across a signing-key rotation (docs/releasing.md, "Rotate the signing key"),
# run INSIDE a clean Debian or Ubuntu container as root by the pull-request job `apt-repo`
# (openspec change add-linux-release-pipeline, design D11). It proves the procedure the runbook
# gives, with this container's apt, before any real key is rotated.
#
#   packaging/release/apt-rotation-check.sh <base URL> <suite>
#
# <base URL> serves, each a complete repository made by apt-publish.sh --local:
#   dual/            signed by BOTH the old and the new key, with a key file holding both
#   new/             signed by the new key only
# and the two key files a client may hold:
#   keys/old.gpg     the old key alone (what a user who installed before the rotation has)
#   keys/new.gpg     the new key alone (what a user who fetches the key after it has)
#
# The four cases (apt-client.sh print, the same text as the docs; the key URL is the only change):
#   dual-signed repository, old key only   apt update succeeds   (the one release of overlap)
#   dual-signed repository, new key only   apt update succeeds
#   new-key repository,     new key only   apt update succeeds
#   new-key repository,     old key only   apt update FAILS      (what happens when the old key is
#                                           dropped and a user has not fetched the new key)
# Exit 1 if any case goes the other way.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
client="$here/apt-client.sh"
export DEBIAN_FRONTEND=noninteractive

log() { echo "apt-rotation-check.sh: $*" >&2; }
die() { log "$*"; exit 1; }

[[ $# -eq 2 ]] || die "usage: apt-rotation-check.sh <base URL> <suite>"
base="${1%/}"
suite="$2"

apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends sudo curl ca-certificates >/dev/null

failures=0
# case NAME REPOSITORY KEY EXPECT(ok|fail)
run_case() {
  local name="$1" repository="$2" key="$3" expect="$4" status=0
  bash -euo pipefail <("$client" print "$suite" --uri "$base/$repository/" --key-url "$base/keys/$key.gpg" --no-install)
  # A clean slate: lists from an earlier case must not answer for this one.
  rm -rf /var/lib/apt/lists/*
  apt-get update >/tmp/rotation-update.log 2>&1 || status=$?
  if [[ "$expect" == ok && "$status" == 0 ]]; then
    log "ok: $name: apt update succeeded"
  elif [[ "$expect" == fail && "$status" != 0 ]]; then
    log "ok: $name: apt update failed, as it must ($(grep -m1 -E 'NO_PUBKEY|not signed|signature|GPG error' /tmp/rotation-update.log || echo "exit $status"))"
  else
    log "FAIL: $name: apt update exited $status, expected $expect"
    sed 's/^/    /' /tmp/rotation-update.log >&2
    failures=$((failures + 1))
  fi
}

run_case "dual-signed repository, old key only" dual old ok
run_case "dual-signed repository, new key only" dual new ok
run_case "new-key repository, new key only" new new ok
run_case "new-key repository, old key only" new old fail

((failures == 0)) || die "$failures rotation case(s) went the wrong way"
log "all four rotation cases behave as the runbook says"
