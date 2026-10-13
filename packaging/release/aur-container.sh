#!/usr/bin/env bash
# Runs INSIDE the archlinux:base-devel container that aur-publish.sh starts (openspec change
# add-linux-release-pipeline, task 6.4). It holds no secret: the only thing mounted is the work
# directory, at /work, with the unpacked release tarball in src/ and the downloaded assets in
# assets/.
#
#   aur-container.sh <channel>...        (HOST_UID: the owner the files get back)
#
# makepkg (which aur.sh runs for .SRCINFO) refuses root, so aur.sh runs as a user made here. The
# trees are written to /work/trees/<channel>, then handed to HOST_UID so the runner can read and
# delete them.
set -euo pipefail

[[ $# -ge 1 ]] || { echo "aur-container.sh: no channel given" >&2; exit 2; }
[[ "${HOST_UID-}" =~ ^[0-9]+$ ]] || { echo "aur-container.sh: HOST_UID is not a number" >&2; exit 2; }

useradd -m builder
install -d -o builder -g builder /work/trees
chown -R builder /work/src /work/assets
trap 'chown -R "$HOST_UID" /work' EXIT
for channel in "$@"; do
  case "$channel" in
    beta | stable) ;;
    *) echo "aur-container.sh: '$channel' is not a channel" >&2; exit 2 ;;
  esac
  su builder -s /bin/bash -c "cd /work && /work/src/packaging/arch/aur.sh $channel /work/assets /work/trees/$channel"
done
