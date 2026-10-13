#!/usr/bin/env bash
# The COPR API token lasts 180 days (design D11). The repository variable COPR_TOKEN_EXPIRES holds
# the day it stops working. This is the one place that reads it.
#
#   COPR_TOKEN_EXPIRES=2027-04-10 packaging/release/token-expiry.sh check
#   COPR_TOKEN_EXPIRES=2027-04-10 packaging/release/token-expiry.sh warn
#
# FORMAT: YYYY-MM-DD, a real calendar date, read as UTC. It is the LAST day the token works: on that
# day it is still good, on the next it is expired. Anything else (empty, 2027-4-10, 2027-02-30, a
# time of day) is unparseable.
#
#   check   the publish workflow's preflight. Exits 1 when the variable is empty or unparseable, or
#           the day has passed. The message names COPR_TOKEN_EXPIRES and the renewal steps. A date
#           within WARN_DAYS only prints a notice.
#           A date more than 200 days ahead is refused too (a token lasts 180, so it is a typo that
#           would switch the check off; `warn` mentions it).
#   warn    release.yml. NEVER exits non-zero: it prints a GitHub warning annotation when the
#           variable is empty, unparseable, passed, or within WARN_DAYS (14) days, and nothing
#           otherwise. The variable does not exist until the maintainer creates it (task 5.1), and a
#           tag run must not fail on that.
#
# TODAY (YYYY-MM-DD) replaces the clock; the tests use it.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

WARN_DAYS=14
MAX_DAYS=200
RENEW="renew it: sign in at https://copr.fedorainfracloud.org/api/ with the Fedora account, copy the new ~/.config/copr text into the environment secret COPR_CONFIG (environment 'release'), then set the repository variable COPR_TOKEN_EXPIRES to the new token's last day (YYYY-MM-DD, 180 days from today); docs/releasing.md has the steps"

mode="${1-}"
[[ $# -eq 1 && ( "$mode" == check || "$mode" == warn ) ]] || die "usage: token-expiry.sh <check|warn>"

# Prints "<expiry> <days left>" and returns 0, or prints why not and returns 1.
read_expiry() {
  local value="${COPR_TOKEN_EXPIRES-}" today="${TODAY:-}" expiry_s today_s
  if [[ -z "$value" ]]; then
    echo "the repository variable COPR_TOKEN_EXPIRES is empty or not set"
    return 1
  fi
  if [[ ! "$value" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || [[ "$(date -u -d "$value" +%F 2>/dev/null)" != "$value" ]]; then
    echo "the repository variable COPR_TOKEN_EXPIRES is '$value', which is not a date in the form YYYY-MM-DD"
    return 1
  fi
  [[ -n "$today" ]] || today="$(date -u +%F)"
  expiry_s="$(date -u -d "$value" +%s)"
  today_s="$(date -u -d "$today" +%s)"
  echo "$value $(((expiry_s - today_s) / 86400))"
}

if ! answer="$(read_expiry)"; then
  if [[ "$mode" == check ]]; then
    die "COPR: $answer, so the token's age is unknown. Set it, then $RENEW"
  fi
  echo "::warning title=COPR_TOKEN_EXPIRES::$answer, so nobody is warned before the COPR token expires. Set it to the token's last day (YYYY-MM-DD)."
  exit 0
fi

expiry="${answer%% *}"
left="${answer##* }"
if ((left < 0)); then
  if [[ "$mode" == check ]]; then
    die "COPR: the token expired on $expiry ($((-left)) days ago): $RENEW"
  fi
  echo "::warning title=COPR token expired::The COPR token expired on $expiry. $RENEW"
  exit 0
fi
# A token lasts 180 days, so a date further ahead is a typo (9999-12-31 would switch the check off).
if ((left > MAX_DAYS)); then
  if [[ "$mode" == check ]]; then
    die "COPR: COPR_TOKEN_EXPIRES is $expiry, $left days away, but a token lasts 180 days (the limit here is $MAX_DAYS): fix the repository variable COPR_TOKEN_EXPIRES. $RENEW"
  fi
  echo "::warning title=COPR_TOKEN_EXPIRES::COPR_TOKEN_EXPIRES is $expiry, $left days away, but a COPR token lasts 180 days: check the repository variable."
  exit 0
fi
if ((left <= WARN_DAYS)); then
  echo "::warning title=COPR token expires soon::The COPR token expires on $expiry ($left days left). $RENEW"
fi
if [[ "$mode" == check ]]; then
  echo "token-expiry.sh: the COPR token is good until $expiry ($left days left)."
fi
