#!/usr/bin/env bash
# Writes the AUR tree for one channel from the repository's PKGBUILD (the only PKGBUILD there is).
#
#   packaging/arch/aur.sh <stable|beta> <assets-dir> [out-dir]
#
# <assets-dir> holds the release tarball, proton-drive-sync-engine-<upstream>.tar.gz. The tree
# is written to [out-dir] (default ./aur-<package name>, which must not exist or must be empty):
#
#   PKGBUILD                         the repository's, with the changes below
#   proton-drive-sync-engine.install the install script the PKGBUILD names
#   .SRCINFO                         `makepkg --printsrcinfo` of that PKGBUILD
#
# The changes to the PKGBUILD are these and nothing else:
#   - sha256sums is the tarball's real SHA-256 instead of 'SKIP'. The repository copy cannot hold
#     it because the PKGBUILD is inside the tarball. If <assets-dir> has a SHA256SUMS, the sum is
#     read from it and must equal the tarball's own; otherwise it is computed.
#   - for `beta`, pkgname is proton-drive-sync-engine-beta, with provides and conflicts naming
#     proton-drive-sync-engine, so the beta package replaces the stable one and anything that
#     depends on the stable name stays satisfied.
# The build steps are the repository PKGBUILD's, untouched. makepkg refuses to run as root, so run
# this as an ordinary user.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

die() {
  echo "aur.sh: $*" >&2
  exit 1
}

usage() {
  echo "usage: packaging/arch/aur.sh <stable|beta> <assets-dir> [out-dir]" >&2
}

[[ $# -ge 2 && $# -le 3 ]] || { usage; exit 2; }
channel="$1"
assets="$2"
case "$channel" in
  stable | beta) ;;
  *) usage; die "channel '$channel' is not stable or beta" ;;
esac
[[ -d "$assets" ]] || die "assets directory '$assets' does not exist"

pkgbuild="$here/PKGBUILD"
install_script="$here/proton-drive-sync-engine.install"
[[ -f "$pkgbuild" ]] || die "$pkgbuild is missing"
[[ -f "$install_script" ]] || die "$install_script is missing"

# A variable the PKGBUILD sets on its own line. Read with sed rather than by sourcing the file,
# which would run its code.
pkgbuild_value() {
  local value
  value="$(sed -n "s/^$1=//p" "$pkgbuild")"
  [[ -n "$value" && "$value" != *$'\n'* ]] || die "$pkgbuild: expected exactly one '$1=' line"
  echo "$value"
}

pkgbase="$(pkgbuild_value _pkgbase)"
upstream="$(pkgbuild_value _upstream_ver)"
if [[ "$channel" == beta ]]; then
  name="${pkgbase}-beta"
else
  name="$pkgbase"
fi

tarball_name="${pkgbase}-${upstream}.tar.gz"
tarball="$assets/$tarball_name"
[[ -f "$tarball" ]] || die "$tarball is missing: the PKGBUILD's source is $tarball_name"

actual="$(sha256sum "$tarball" | awk '{print $1}')"
if [[ -f "$assets/SHA256SUMS" ]]; then
  listed="$(awk -v name="$tarball_name" '$2 == name || $2 == "*" name { print $1 }' "$assets/SHA256SUMS")"
  [[ -n "$listed" ]] || die "$assets/SHA256SUMS does not list $tarball_name"
  [[ "$listed" == "$actual" ]] ||
    die "$assets/SHA256SUMS lists $listed for $tarball_name, but the file's SHA-256 is $actual"
fi

# The two lines this script rewrites must each be there exactly once, so a PKGBUILD edit that
# moves them fails here instead of publishing a package that still says 'SKIP'.
require_once() {
  local count
  count="$(grep -cFx -- "$1" "$pkgbuild" || true)"
  [[ "$count" == 1 ]] || die "$pkgbuild: expected exactly one line '$1', found $count"
}
require_once 'pkgname="${_pkgbase}"'
require_once "sha256sums=('SKIP')"

command -v makepkg >/dev/null || die "makepkg is not installed (run this in an Arch container)"

out="${3:-aur-$name}"
if [[ -e "$out" ]]; then
  [[ -d "$out" && -z "$(ls -A "$out")" ]] || die "$out exists and is not an empty directory"
fi
mkdir -p "$out"

if [[ "$channel" == beta ]]; then
  pkgname_lines="pkgname=${name}\nprovides=(${pkgbase})\nconflicts=(${pkgbase})"
else
  pkgname_lines="pkgname=${name}"
fi

sed \
  -e "s|^pkgname=\"\${_pkgbase}\"\$|${pkgname_lines}|" \
  -e "s|^sha256sums=('SKIP')\$|sha256sums=('${actual}')|" \
  "$pkgbuild" >"$out/PKGBUILD"
cp "$install_script" "$out/"

(cd "$out" && makepkg --printsrcinfo >.SRCINFO) ||
  die "makepkg --printsrcinfo failed on the generated PKGBUILD in $out"

echo "aur.sh: wrote $out ($name $upstream, sha256 $actual)" >&2
