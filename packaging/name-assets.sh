#!/usr/bin/env bash
# Renames built packages so that each file name carries the upstream version.
#
#   packaging/name-assets.sh <upstream version> <package file>...
#
# The package tools put the PACKAGE version in a file name: 0.1.0~beta.1 in an .rpm and a .deb,
# 0.1.0beta1 in a .pkg.tar.zst. A release asset is named by the upstream version instead
# (0.1.0-beta.1, which has no "~"), so the name matches the tag and the tarball.
#
# The script renames only a package that really is that version. It asks `version.sh spell` what
# the package's own version must be for the given upstream version (the mapping stays in
# version.sh), reads the version from the package itself (rpm, dpkg-deb or bsdtar, whichever the
# file needs), and refuses when the two differ, or when the upstream version is not one version.sh
# accepts. Only the file name changes; the package's contents are not touched. A final release
# (0.1.0) renames nothing for the .rpm and .pkg.tar.zst.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

die() {
  echo "name-assets.sh: $*" >&2
  exit 1
}

[[ $# -ge 2 ]] || die "usage: packaging/name-assets.sh <upstream version> <package file>..."
upstream="$1"
shift

# The package format of a file, by its name.
format_of() {
  case "$1" in
    *.rpm) echo rpm ;;
    *.deb) echo deb ;;
    *.pkg.tar.zst) echo arch ;;
    *) die "$1: not an .rpm, .deb or .pkg.tar.zst" ;;
  esac
}

# The version string a package format writes into the file name, read from the package.
name_version() {
  case "$(format_of "$1")" in
    rpm) rpm -qp --qf '%{VERSION}' "$1" ;;
    # The Version field is <upstream part>-<revision>, and the file name keeps the revision, so
    # only the part before the last "-" is compared and replaced.
    deb) dpkg-deb -f "$1" Version | sed 's/-[^-]*$//' ;;
    # .PKGINFO says "pkgver = <pkgver>-<pkgrel>"; the file name has both too, and the revision
    # stays where it is.
    arch) bsdtar -xOf "$1" .PKGINFO | sed -n 's/^pkgver = \(.*\)-[0-9.]*$/\1/p' ;;
  esac
}

# What the package's own version must be for this upstream version. A Debian revision is not part
# of it: a packaging-only rebuild raises the revision and keeps the upstream part.
expected_version() {
  local spelled
  spelled="$("$here/version.sh" spell "$1" "$upstream")" || exit 1
  if [[ "$1" == deb ]]; then
    spelled="${spelled%-*}"
  fi
  printf '%s' "$spelled"
}

# One check up front, so a bad upstream version fails before any file is looked at.
"$here/version.sh" spell rpm "$upstream" >/dev/null || exit 1

for file in "$@"; do
  [[ -f "$file" ]] || die "$file is not a file"
  format="$(format_of "$file")"
  old="$(name_version "$file")"
  [[ -n "$old" ]] || die "$file: could not read the package version"
  expected="$(expected_version "$format")"
  [[ "$old" == "$expected" ]] ||
    die "$file: the package says version '$old', but $upstream is '$expected' in an .$format"
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
