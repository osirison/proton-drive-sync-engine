#!/usr/bin/env bash
# The ONE definition of what a user of the apt channel types (openspec change
# add-linux-release-pipeline, design D10, tasks 6.3 and 6.6), and of the key fingerprints that go with it.
#
#   packaging/release/apt-client.sh print <beta|stable> [--uri URL] [--key-url URL] [--no-sudo] [--no-install]
#   packaging/release/apt-client.sh fingerprint-command
#   packaging/release/apt-client.sh fingerprints
#
# `print` writes the commands the docs show: put the repository's key where apt reads it, write the
# `.sources` file, update, install. With no option it prints them exactly as a user types them
# (sudo, the public URLs). The options exist for the jobs that run the same text somewhere else:
#
#   --uri URL / --key-url URL   another repository (the pull-request job serves its own tree over HTTP)
#   --no-sudo                   the lines without `sudo ` (a container runs as root; install `sudo`
#                               instead to run the text verbatim)
#   --no-install                everything but the last line, `apt update && apt install ...`, so the
#                               caller can install one exact version
#
# `fingerprint-command` prints the command that shows the fingerprint of the key a user downloaded.
# `fingerprints` prints the fingerprints recorded for this repository, one per line, from
# packaging/release/apt-key-fingerprints.txt (APT_FINGERPRINT_FILE replaces the file; tests only).
# It fails while none is recorded, which is the case until the signing key exists (task 5.3).
#
# Three things read these and must not keep a copy: the pull-request job (`apt-repo` in packages.yml),
# the post-publish verification (verify-apt.sh) and, in task 8, the docs page, which is checked
# against this output. Keep this file the only place the text is written.
#
# APT_CLIENT_ROOT (tests only) is put in front of the two directories under /etc, so a test can run
# the text without touching the machine's apt configuration.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

readonly SITE_URL="https://osirison.github.io/proton-drive-sync-engine-apt/"
readonly KEY_NAME="proton-drive-sync-engine.gpg"
readonly KEYRING_DIR="/etc/apt/keyrings"
readonly SOURCES_DIR="/etc/apt/sources.list.d"
readonly SOURCES_NAME="proton-drive-sync-engine.sources"

die() {
  echo "apt-client.sh: $*" >&2
  exit 1
}

usage() {
  echo "usage: apt-client.sh print <beta|stable> [--uri URL] [--key-url URL] [--no-sudo] [--no-install]" >&2
  echo "       apt-client.sh fingerprint-command" >&2
  echo "       apt-client.sh fingerprints" >&2
  exit 64
}

cmd_print() {
  [[ $# -ge 1 ]] || usage
  local suite="$1" uri="$SITE_URL" key_url="" sudo="sudo " install=true
  shift
  case "$suite" in beta | stable) ;; *) die "'$suite' is not a suite (beta or stable)" ;; esac
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --uri) [[ $# -ge 2 ]] || usage; uri="$2"; shift 2 ;;
      --key-url) [[ $# -ge 2 ]] || usage; key_url="$2"; shift 2 ;;
      --no-sudo) sudo=""; shift ;;
      --no-install) install=false; shift ;;
      *) usage ;;
    esac
  done
  [[ -n "$key_url" ]] || key_url="${uri%/}/$KEY_NAME"
  local keyring="${APT_CLIENT_ROOT-}$KEYRING_DIR/$KEY_NAME"
  # The text. Each line is one command; the key is fetched with curl, written with tee, and the
  # .sources file is a quoted here-document so nothing in it is expanded.
  printf '%sinstall -d -m 0755 %s\n' "$sudo" "${APT_CLIENT_ROOT-}$KEYRING_DIR"
  printf 'curl -fsSL %s \\\n' "$key_url"
  printf '  | %stee %s >/dev/null\n' "$sudo" "$keyring"
  printf "%stee %s >/dev/null <<'EOF'\n" "$sudo" "${APT_CLIENT_ROOT-}$SOURCES_DIR/$SOURCES_NAME"
  printf 'Types: deb\n'
  printf 'URIs: %s\n' "$uri"
  printf 'Suites: %s\n' "$suite"
  printf 'Components: main\n'
  printf 'Architectures: amd64\n'
  printf 'Signed-By: %s\n' "$keyring"
  printf 'EOF\n'
  if [[ "$install" == true ]]; then
    printf '%sapt update && %sapt install proton-drive-sync-engine\n' "$sudo" "$sudo"
  fi
}

cmd_fingerprint_command() {
  [[ $# -eq 0 ]] || usage
  # The first `fpr` line after a `pub` line is the primary key's; a subkey's follows its `sub` line.
  printf "gpg --show-keys --with-colons %s | awk -F: '\$1 == \"pub\" { want = 1; next } \$1 == \"fpr\" && want { print \$10; want = 0 }'\n" \
    "${APT_CLIENT_ROOT-}$KEYRING_DIR/$KEY_NAME"
}

# The recorded fingerprints: 40 upper-case hex digits per line, no spaces; `#` starts a comment.
cmd_fingerprints() {
  [[ $# -eq 0 ]] || usage
  local file="${APT_FINGERPRINT_FILE:-$here/apt-key-fingerprints.txt}" line found=()
  [[ -f "$file" ]] || die "$file does not exist"
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%%#*}"
    line="${line//[[:space:]]/}"
    [[ -n "$line" ]] || continue
    [[ "$line" =~ ^[0-9A-F]{40}$ ]] ||
      die "$file: '$line' is not a fingerprint (40 upper-case hex digits, no spaces)"
    found+=("$line")
  done <"$file"
  ((${#found[@]} > 0)) ||
    die "no signing-key fingerprint is recorded in $file yet: create the key (task 5.3), then record its fingerprint there (docs/releasing.md)"
  printf '%s\n' "${found[@]}" | LC_ALL=C sort -u
}

case "${1-}" in
  print) shift; cmd_print "$@" ;;
  fingerprint-command) shift; cmd_fingerprint_command "$@" ;;
  fingerprints) shift; cmd_fingerprints "$@" ;;
  *) usage ;;
esac
