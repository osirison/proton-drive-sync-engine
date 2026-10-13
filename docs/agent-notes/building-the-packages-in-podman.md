---
trigger: podman run, makepkg, dpkg-buildpackage, rpmbuild, namcap
depends_on: .github/workflows/packages.yml, packaging/
recorded: 2026-10-13
---

# Building the three packages locally in podman

Run each build in its distribution's container, from the `git archive` tarball, with the
commands of the matching job in `.github/workflows/packages.yml`. A release build compiles the
whole workspace and the Tauri GUI: about 10 minutes on 16 cores for one build, longer with three
at once. Mount only a scratch directory (the tarball, read-only) and the worktree's `packaging/`
(read-only); never `$HOME` or a socket.

- **Arch**: `makepkg` refuses root, so make a user with `NOPASSWD` sudo and run
  `sudo -u builder makepkg -s --noconfirm`. The base-devel image has no `git`. A package you
  built must pass `namcap` with no `E:` line; the default makepkg options (`lto`, `debug`) break
  this package, so the PKGBUILD turns both off.
- **Fedora**: `dnf -y install rpm-build 'dnf-command(builddep)'`, then `dnf -y builddep <spec>`,
  then `rpmbuild -ba` with `--define "_topdir …" --define "_sourcedir …"`; the sources directory
  must hold the tarball and `packaging/rpm/proton-syncd.service`.
- **Ubuntu 22.04**: rustup (`--profile minimal`) first, then
  `apt-get build-dep -o APT::Build-Profiles=pkg.proton-drive-sync-engine.rustup ./` and
  `dpkg-buildpackage -b -us -uc --build-profiles=pkg.proton-drive-sync-engine.rustup`, with
  `packaging/debian` linked as `./debian`.

Test a change to a packaging file by mounting the worktree's `packaging/` and copying from it
instead of from the tarball (the tarball is made from `HEAD`, which lacks uncommitted edits).

A script piped through `set -o pipefail` that ends in `| head` is killed by SIGPIPE and the
container exits non-zero; write the list to a file and `head` that.
