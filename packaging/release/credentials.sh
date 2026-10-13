#!/usr/bin/env bash
# Puts the publish job's four secrets into files, and takes them out again.
#
#   COPR_CONFIG=... AUR_SSH_KEY=... APT_SIGNING_KEY=... APT_DEPLOY_KEY=... \
#     packaging/release/credentials.sh write
#   packaging/release/credentials.sh remove
#
# The secrets arrive in the environment (the workflow's `env:` is the only place an expression reads
# a secret), and each is written to a file in a directory only this user can enter (mode 700, files
# 600, umask 077). A secret that is empty gets NO file and no path: the preflight then names it. The
# paths are handed to the later steps through GITHUB_ENV (or printed, when it is not set) as
# COPR_CONFIG_FILE, AUR_SSH_KEY_FILE, APT_SIGNING_KEY_FILE and APT_DEPLOY_KEY_FILE.
#
# `remove` deletes the whole directory; the workflow runs it in an `if: always()` step.
#
# PUBLISH_CREDENTIALS_DIR names the directory (default $RUNNER_TEMP/publish-credentials).
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

dir="${PUBLISH_CREDENTIALS_DIR:-}"
if [[ -z "$dir" && -n "${RUNNER_TEMP-}" ]]; then
  dir="$RUNNER_TEMP/publish-credentials"
fi
[[ -n "$dir" ]] || die "PUBLISH_CREDENTIALS_DIR (or RUNNER_TEMP) is not set"

case "${1-}" in
  write)
    umask 077
    rm -rf -- "$dir"
    mkdir -p "$dir"
    chmod 700 "$dir"
    # variable, file, variable that names the file
    for entry in COPR_CONFIG:copr-config:COPR_CONFIG_FILE \
      AUR_SSH_KEY:aur-ssh-key:AUR_SSH_KEY_FILE \
      APT_SIGNING_KEY:apt-signing-key:APT_SIGNING_KEY_FILE \
      APT_DEPLOY_KEY:apt-deploy-key:APT_DEPLOY_KEY_FILE; do
      IFS=: read -r variable file path_variable <<<"$entry"
      value="${!variable-}"
      if [[ -z "$value" ]]; then
        echo "credentials.sh: $variable is empty or not set: no file written (the preflight reports it)" >&2
        continue
      fi
      # A secret pasted into the settings page may lack its final newline, which ssh and gpg need.
      printf '%s\n' "$value" >"$dir/$file"
      chmod 600 "$dir/$file"
      if [[ -n "${GITHUB_ENV-}" ]]; then
        echo "$path_variable=$dir/$file" >>"$GITHUB_ENV"
      else
        echo "$path_variable=$dir/$file"
      fi
    done
    ;;
  remove)
    rm -rf -- "$dir"
    ;;
  *)
    die "usage: credentials.sh <write|remove>"
    ;;
esac
