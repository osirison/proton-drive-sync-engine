#!/usr/bin/env bash
# The publish job's first checks (openspec change add-linux-release-pipeline, task 6.2): every
# credential is tried BEFORE any channel step, so an expired or broken one fails the run at the
# top with its renewal step, not half way through a publish.
#
#   packaging/release/preflight.sh
#
# Reads the file paths credentials.sh writes (COPR_CONFIG_FILE, AUR_SSH_KEY_FILE,
# APT_SIGNING_KEY_FILE, APT_DEPLOY_KEY_FILE) and COPR_TOKEN_EXPIRES. All four credentials are tried,
# whatever the first one did, and every failure is listed with the credential and what to do about
# it; the exit status is 1 if there was any. All four are checked for a pre-release too (the apt
# channel is not built in this change, but the secrets are shared), and for a dry run, because
# every check here only reads.
#
#   COPR  `copr-cli whoami` and the token's last day (token-expiry.sh). NOTE: `whoami` reads the
#         user name from the config file and asks the server nothing, so it proves the file is
#         there and parses, not that the token works. The expiry date is what stands for the
#         token; a token the server refuses shows at the build as "Login invalid/expired".
#   AUR   `ssh -T aur@aur.archlinux.org` with the key. The AUR has no shell, so the exit status is
#         1 on SUCCESS (and 255 is ssh's own failure): success is the greeting aurweb prints,
#         "Welcome to AUR, <user>! Interactive shell is disabled." (aurweb/git/serve.py).
#   apt signing key  imported into a throwaway keyring, then used to sign and verify a test string.
#   apt deploy key   `git ls-remote` of the apt repository over ssh with the key.
#
# Both ssh hosts are pinned (packaging/release/known_hosts).
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

record_on_exit preflight
failures=()
fail() {
  failures+=("$1")
  echo "preflight.sh: FAILED: $1" >&2
}

# A credential file that must be there and not empty.
have_file() { [[ -n "${1-}" && -s "$1" ]]; }

check_copr() {
  local name=COPR_CONFIG
  local renew="renew the token (it lasts 180 days): https://copr.fedorainfracloud.org/api/ , then update the environment secret COPR_CONFIG and the repository variable COPR_TOKEN_EXPIRES; docs/releasing.md"
  if ! have_file "${COPR_CONFIG_FILE-}"; then
    fail "$name: the secret is empty or not set, so there is no COPR config. Create it: $renew"
    return
  fi
  local who
  if ! who="$(copr-cli --config "$COPR_CONFIG_FILE" whoami 2>&1)"; then
    fail "$name: 'copr-cli whoami' failed ($(tr '\n' ' ' <<<"$who")). $renew"
    return
  fi
  echo "preflight.sh: COPR: whoami says '$who'" >&2
  local expiry
  if ! expiry="$("$publish_here/token-expiry.sh" check 2>&1)"; then
    fail "COPR_TOKEN_EXPIRES / $name: $(tr '\n' ' ' <<<"$expiry")"
    return
  fi
  echo "$expiry" >&2
}

check_aur() {
  local name=AUR_SSH_KEY
  local renew="the key is registered on the AUR account (https://aur.archlinux.org/account/ , SSH Public Key): register a new key, update the environment secret AUR_SSH_KEY; docs/releasing.md"
  if ! have_file "${AUR_SSH_KEY_FILE-}"; then
    fail "$name: the secret is empty or not set. $renew"
    return
  fi
  local output status=0
  output="$(run_ssh "$AUR_SSH_KEY_FILE" -T aur@aur.archlinux.org 2>&1)" || status=$?
  if [[ "$status" == 255 ]]; then
    fail "$name: ssh could not connect or authenticate to aur.archlinux.org ($(tr '\n' ' ' <<<"$output")). $renew"
    return
  fi
  if [[ "$output" != *"Welcome to AUR, "* ]]; then
    fail "$name: ssh -T aur@aur.archlinux.org did not greet us ($(tr '\n' ' ' <<<"$output")). $renew"
    return
  fi
  echo "preflight.sh: AUR: $(head -n1 <<<"$output")" >&2
}

check_signing() {
  local name=APT_SIGNING_KEY
  local renew="generate the key again as docs/releasing.md says (Ed25519, no expiry) and update the environment secret APT_SIGNING_KEY; users must be told to fetch the new key"
  if ! have_file "${APT_SIGNING_KEY_FILE-}"; then
    fail "$name: the secret is empty or not set. $renew"
    return
  fi
  # The directory is short because gpg-agent's socket path has a length limit.
  local home result=0
  home="$(mktemp -d /tmp/pdse-gpg.XXXXXX)"
  chmod 700 "$home"
  (
    export GNUPGHOME="$home"
    trap 'gpgconf --kill gpg-agent >/dev/null 2>&1 || true; rm -rf -- "$GNUPGHOME"' EXIT
    gpg --batch --quiet --import "$APT_SIGNING_KEY_FILE" 2>&1 || { echo "gpg could not import the key"; exit 1; }
    fingerprint="$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" { print $10; exit }')"
    [[ -n "$fingerprint" ]] || { echo "the file holds no secret key"; exit 1; }
    printf 'preflight test %s\n' "$fingerprint" >"$GNUPGHOME/data"
    gpg --batch --yes --quiet --pinentry-mode loopback --passphrase '' --digest-algo SHA512 \
      --local-user "$fingerprint" --armor --detach-sign -o "$GNUPGHOME/data.asc" "$GNUPGHOME/data" 2>&1 ||
      { echo "the key cannot sign (a passphrase, or an expired or revoked key)"; exit 1; }
    gpg --batch --quiet --verify "$GNUPGHOME/data.asc" "$GNUPGHOME/data" 2>&1 || { echo "the test signature does not verify"; exit 1; }
    expires="$(gpg --batch --with-colons --list-keys "$fingerprint" | awk -F: '$1 == "pub" { print $7; exit }')"
    if [[ -n "$expires" ]]; then
      echo "::warning title=APT_SIGNING_KEY has an expiry::the apt signing key expires, but design D11 says it must not (users would have to fetch a new key)"
    fi
    echo "preflight.sh: apt signing key: signed and verified a test string with $fingerprint"
  ) >"$home.log" 2>&1 || result=$?
  if [[ "$result" != 0 ]]; then
    fail "$name: $(tr '\n' ' ' <"$home.log"). $renew"
  else
    sed 's/^/preflight.sh: /' "$home.log" >&2
  fi
  rm -f -- "$home.log"
}

check_apt_deploy() {
  local name=APT_DEPLOY_KEY
  local renew="the key is a deploy key with write access on the apt repository (its Settings, Deploy keys): add a new one and update the environment secret APT_DEPLOY_KEY; docs/releasing.md"
  if ! have_file "${APT_DEPLOY_KEY_FILE-}"; then
    fail "$name: the secret is empty or not set. $renew"
    return
  fi
  local url output
  url="$(apt_repository_url)" || { fail "$name: cannot work out the apt repository"; return; }
  if ! output="$(GIT_SSH_COMMAND="$(ssh_command_for "$APT_DEPLOY_KEY_FILE")" GIT_TERMINAL_PROMPT=0 git ls-remote "$url" 2>&1)"; then
    fail "$name: 'git ls-remote $url' failed ($(tr '\n' ' ' <<<"$output")). $renew"
    return
  fi
  echo "preflight.sh: apt deploy key: $url answers" >&2
}

check_copr
check_aur
check_signing
check_apt_deploy

if ((${#failures[@]} > 0)); then
  echo "preflight.sh: ${#failures[@]} credential(s) failed; no channel step will run:" >&2
  printf 'preflight.sh:   - %s\n' "${failures[@]}" >&2
  exit 1
fi
finish_recording
echo "preflight.sh: all four credentials work." >&2
