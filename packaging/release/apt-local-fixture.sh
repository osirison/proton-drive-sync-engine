#!/usr/bin/env bash
# What the pull-request job `apt-repo` serves (openspec change add-linux-release-pipeline, tasks 5.3
# and 6.3, design D10/D11): the apt repository the publish step would make, from the PR's own built
# debs, signed with throwaway Ed25519 keys made here (the same kind D11 asks for: no passphrase, no
# expiry). The tree is made by the SAME script the publish step runs, in its --local mode.
#
#   packaging/release/apt-local-fixture.sh <deb dir> <work dir> <upstream version>
#
# Writes under <work dir>:
#   serve/                 the repository, signed by the "old" key (what is published today)
#   serve/dual/            the same, signed by the old AND the new key (the rotation's overlap release)
#   serve/new/             the same, signed by the new key only (after the old key is dropped)
#   serve/keys/old.gpg, serve/keys/new.gpg   the keys a client may hold
#   fingerprint.txt        the old key's fingerprint, in the format of apt-key-fingerprints.txt
#   suites.txt             the suites the tree has, one line
# and checks that making the tree twice gives byte-identical Packages files.
#
# <deb dir> holds the three debs of one build; this script writes the SHA256SUMS of a release beside
# a copy of them (so the checksum check runs, but against sums made here, not by draft.sh).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
publish="$here/apt-publish.sh"
log() { echo "apt-local-fixture.sh: $*" >&2; }

[[ $# -eq 3 ]] || { echo "usage: apt-local-fixture.sh <deb dir> <work dir> <upstream version>" >&2; exit 64; }
debs="$1"
work="$2"
upstream="$3"

rm -rf -- "$work"
mkdir -p "$work/release" "$work/serve"
cp -- "$debs"/*.deb "$work/release/"
(cd "$work/release" && sha256sum -- *.deb >SHA256SUMS)

# Two keys, as the rotation needs. A short GNUPGHOME: gpg-agent's socket path has a length limit.
home="$(mktemp -d "${TMPDIR:-/tmp}/pdse-fixture.XXXXXX")"
chmod 700 "$home"
trap 'GNUPGHOME="$home" gpgconf --kill gpg-agent >/dev/null 2>&1 || true; rm -rf -- "$home"' EXIT
gpg_home() { GNUPGHOME="$home" gpg --batch --quiet --pinentry-mode loopback --passphrase '' "$@"; }
gpg_home --quick-gen-key 'fixture old <old@example.invalid>' ed25519 sign never 2>/dev/null
old="$(GNUPGHOME="$home" gpg --batch --with-colons --list-keys | awk -F: '$1 == "pub" { want = 1; next } $1 == "fpr" && want { print $10; exit }')"
gpg_home --quick-gen-key 'fixture new <new@example.invalid>' ed25519 sign never 2>/dev/null
new="$(GNUPGHOME="$home" gpg --batch --with-colons --list-keys | awk -F: '$1 == "pub" { want = 1; next } $1 == "fpr" && want { print $10; want = 0 }' | grep -vx "$old")"
[[ -n "$old" && -n "$new" ]] || { echo "apt-local-fixture.sh: could not make the keys" >&2; exit 1; }
gpg_home --armor --export-secret-keys "$old" >"$work/old.asc"
gpg_home --armor --export-secret-keys "$new" >"$work/new.asc"
cat "$work/old.asc" "$work/new.asc" >"$work/both.asc"
mkdir -p "$work/serve/keys"
gpg_home --export "$old" >"$work/serve/keys/old.gpg"
gpg_home --export "$new" >"$work/serve/keys/new.gpg"
printf '%s\n' "$old" >"$work/fingerprint.txt"

make_tree() { # key file, output directory
  APT_SIGNING_KEY_FILE="$1" "$publish" --local "$2" "$upstream=$work/release"
}
make_tree "$work/old.asc" "$work/serve"
make_tree "$work/both.asc" "$work/serve/dual"
make_tree "$work/new.asc" "$work/serve/new"

# Determinism: the same debs make the same Packages (only Release's Date and the signatures move).
make_tree "$work/old.asc" "$work/again"
for packages in "$work"/serve/dists/*/main/binary-amd64/Packages; do
  suite="$(basename "$(dirname "$(dirname "$(dirname "$packages")")")")"
  cmp -- "$packages" "$work/again/dists/$suite/main/binary-amd64/Packages" ||
    { echo "apt-local-fixture.sh: the $suite Packages file differs between two runs" >&2; exit 1; }
done
rm -rf -- "$work/again"

find "$work/serve/dists" -mindepth 1 -maxdepth 1 -printf '%f\n' | LC_ALL=C sort | paste -sd ' ' >"$work/suites.txt"
log "tree ready; suites: $(cat "$work/suites.txt"); old key $old, new key $new"
