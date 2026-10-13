#!/usr/bin/env bash
# Renames built packages so that each file name carries the upstream version.
#
#   packaging/name-assets.sh <upstream version> <package file>...
#
# The package tools put the PACKAGE version in a file name: 0.1.0~beta.1 in an .rpm and a .deb,
# 0.1.0beta1 in a .pkg.tar.zst. A release asset is named by the upstream version instead
# (0.1.0-beta.1, which has no "~"), so the name matches the tag and the tarball. The package
# version is read from the package itself (rpm, dpkg-deb or bsdtar, whichever the file needs), so
# this script holds no copy of the mapping in packaging/version.sh: it replaces what the package
# says its version is with the upstream version, in the file name only. The package's contents
# are not touched. A final release (0.1.0) renames nothing for the .rpm and .pkg.tar.zst.
set -euo pipefail

die() {
  echo "name-assets.sh: $*" >&2
  exit 1
}

[[ $# -ge 2 ]] || die "usage: packaging/name-assets.sh <upstream version> <package file>..."
upstream="$1"
shift

# The version string the package format writes into the file name.
name_version() {
  case "$1" in
    *.rpm) rpm -qp --qf '%{VERSION}' "$1" ;;
    # The Version field is <upstream part>-<revision>, and the file name keeps the revision, so
    # only the part before the last "-" is replaced.
    *.deb) dpkg-deb -f "$1" Version | sed 's/-[^-]*$//' ;;
    # .PKGINFO says "pkgver = <pkgver>-<pkgrel>"; the file name has both too, and the revision
    # stays where it is.
    *.pkg.tar.zst) bsdtar -xOf "$1" .PKGINFO | sed -n 's/^pkgver = \(.*\)-[0-9.]*$/\1/p' ;;
    *) die "$1: not an .rpm, .deb or .pkg.tar.zst" ;;
  esac
}

for file in "$@"; do
  [[ -f "$file" ]] || die "$file is not a file"
  old="$(name_version "$file")"
  [[ -n "$old" ]] || die "$file: could not read the package version"
  dir="$(dirname "$file")"
  base="$(basename "$file")"
  [[ "$base" == *"$old"* ]] || die "$base: the file name does not contain the package version '$old'"
  new="${base/"$old"/"$upstream"}"
  [[ "$new" != *"~"* ]] || die "$new: still contains '~'"
  if [[ "$new" != "$base" ]]; then
    mv -- "$file" "$dir/$new"
  fi
  echo "$new"
done
