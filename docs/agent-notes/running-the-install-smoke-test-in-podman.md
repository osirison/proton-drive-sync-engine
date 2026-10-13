---
trigger: podman run with a mounted package, packaging/smoke-test.sh, "Permission denied" on a mounted file, apt-get install ./x.deb "Unsupported file"
depends_on: packaging/smoke-test.sh, .github/workflows/packages.yml
recorded: 2026-10-13
---

# Running the install smoke test in podman

Install the built package in a clean container of the distribution, then run
`packaging/smoke-test.sh <upstream version> rpm|deb` (or `arch <first pacman -U output> <second>`)
inside it. Mount the package directory and the script read-only; nothing else (never `$HOME`, a
runtime directory, D-Bus or the podman socket). The script is for throwaway containers only.

- **SELinux hosts (Fedora): add `--security-opt label=disable`.** Without it every mounted file is
  unreadable inside the container, and the symptoms point elsewhere: `pacman -U` says
  `permission denied` on the package, `bash: /smoke.sh: Permission denied`, and
  `apt-get install ./x.deb` says `E: Unsupported file ./x.deb given on commandline` for a file that
  exists. `:z`/`:Z` on a mount would relabel files on the host; `label=disable` changes only the
  container.
- `apt-get install` needs `apt-get update` first in every Debian/Ubuntu image, and a path with a
  `/` (`./x.deb` or absolute) so apt reads a file and not a package name.
- The Arch cell runs `pacman -U` twice and keeps both outputs: the first must print the install
  note, the second (a reinstall, `post_upgrade`) must not.
- The smoke script's `proton-sync doctor` check needs a container with no `proton-drive`. Do not
  install one; the package does not depend on it.
- The packages from the last green `packages.yml` run can be downloaded with
  `gh run download <run id> -D <dir>` (artifacts `rpm-packages`, `deb-packages`, `arch-package`,
  `source-tarball`) instead of building them (10 minutes each).
