#!/usr/bin/env bash
# Runs INSIDE the archlinux:base-devel container that aur-publish.sh starts (openspec change
# add-linux-release-pipeline, task 6.4). It holds no secret: the only thing mounted is the work
# directory, at /work, with the unpacked release tarball in src/ and the downloaded assets in
# assets/.
#
#   aur-container.sh <channel>...        (HOST_UID: the owner the files get back)
#
# makepkg (which aur.sh runs for .SRCINFO) refuses root, so aur.sh runs as a user made here, which
# is not the owner of /work (the runner's uid on the host). So /work is made traversable for it
# (mode 755: without that `cd /work` is refused for the new user), the trees are written to
# /work/trees/<channel>, and the whole directory is handed to HOST_UID at exit so the runner can
# read and delete it, also when aur.sh failed.
#
# AUR_WORK_DIR replaces /work; the tests use it.
set -euo pipefail

work="${AUR_WORK_DIR:-/work}"
[[ $# -ge 1 ]] || { echo "aur-container.sh: no channel given" >&2; exit 2; }
[[ "${HOST_UID-}" =~ ^[0-9]+$ ]] || { echo "aur-container.sh: HOST_UID is not a number" >&2; exit 2; }
for channel in "$@"; do
  case "$channel" in
    beta | stable) ;;
    *) echo "aur-container.sh: '$channel' is not a channel" >&2; exit 2 ;;
  esac
done

useradd -m builder
chmod 755 "$work"
mkdir "$work/trees"
chown builder "$work/trees"
chown -R builder "$work/src" "$work/assets"
trap 'chown -R "$HOST_UID" "$work"' EXIT
for channel in "$@"; do
  su builder -s /bin/bash -c "cd '$work' && '$work/src/packaging/arch/aur.sh' $channel '$work/assets' '$work/trees/$channel'"
done
