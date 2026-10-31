#!/usr/bin/env bash
# The AUR check, run INSIDE an archlinux:base-devel container as root (openspec change
# add-linux-release-pipeline, task 6.6, design D8/D12). verify-channel.sh starts it.
#
#   packaging/release/verify-aur.sh <upstream version> <channel>...
#
# For each routed channel it does what an AUR user does: clones the package from the AUR, builds it
# with makepkg as an ordinary user (makepkg refuses root), installs the result and requires pacman to
# report exactly the release's version (version.sh spell arch, with any pkgrel). The PKGBUILD's
# pkgver is compared first, so a package the AUR has not moved to this release fails in seconds
# and not after a build of ten minutes or more. Each package is removed before the next channel (the
# beta package conflicts with the stable one).
#
# It reads only public endpoints (aur.archlinux.org, the release's tarball on GitHub, Arch's mirrors).
# AUR_CLONE_BASE (default https://aur.archlinux.org) replaces the part before <package>.git; the
# tests use a local directory. VERIFY_AUR_HOME (default /home/builder) and VERIFY_AUR_SUDOERS
# (default /etc/sudoers.d/builder) are likewise for the tests, which run this on the host with stub
# pacman, useradd, su, chown and makepkg.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

[[ $# -ge 2 ]] || die "usage: verify-aur.sh <upstream version> <channel>..."
upstream="$1"
shift
arch_version="$("$version_sh" spell arch "$upstream")" || exit 1
base="${AUR_CLONE_BASE:-https://aur.archlinux.org}"
# Where the build user's work happens and where its sudoers entry goes. The defaults are the
# container's; the tests name directories of their own so that a run never reaches /etc.
build_home="${VERIFY_AUR_HOME:-/home/builder}"
sudoers_file="${VERIFY_AUR_SUDOERS:-/etc/sudoers.d/builder}"

echo "verify-aur.sh: preparing the container (git, sudo, a build user)" >&2
pacman -Syu --noconfirm --needed git sudo >/dev/null
useradd -m builder
echo 'builder ALL=(ALL) NOPASSWD: ALL' >"$sudoers_file"

for channel in "$@"; do
  package="$(channel_name "$channel")"
  clone="$build_home/$package"
  echo "verify-aur.sh: == $channel: AUR package $package, version $arch_version ==" >&2
  git clone --quiet "$base/$package.git" "$clone" || die "cannot clone the AUR package $package"
  [[ -f "$clone/PKGBUILD" ]] || die "the AUR package $package has no PKGBUILD"
  pkgver="$(sed -n 's/^pkgver=//p' "$clone/PKGBUILD" | head -n1)"
  [[ "$pkgver" == "$arch_version" ]] ||
    die "the AUR package $package is at pkgver '${pkgver:-<none>}', not $arch_version: it is missing from the channel"
  chown -R builder:builder "$clone"
  su builder -s /bin/bash -c "cd '$clone' && makepkg -s --noconfirm" || die "makepkg failed for $package"
  built=("$clone"/*.pkg.tar.zst)
  [[ ${#built[@]} -eq 1 && -f "${built[0]}" ]] || die "makepkg made ${#built[@]} package files for $package, expected one"
  pacman -U --noconfirm "${built[0]}"
  got="$(pacman -Q "$package" | awk '{print $2}')"
  # Compared as text: the version's dots are not a pattern.
  [[ "$got" == "${arch_version}-"* && "${got#"${arch_version}-"}" =~ ^[0-9]+$ ]] || die "pacman reports $package $got installed, expected $arch_version-<pkgrel>"
  echo "verify-aur.sh: $package: installed $got" >&2
  pacman -Rns --noconfirm "$package"
done
echo "verify-aur.sh: ok: $* at $arch_version" >&2
