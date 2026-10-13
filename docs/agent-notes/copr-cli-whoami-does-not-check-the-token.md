---
trigger: copr-cli whoami, copr-cli list-packages, copr-cli build
depends_on: packaging/release/preflight.sh, packaging/release/copr.sh
recorded: 2026-10-13
---

# copr-cli: `whoami` is local, and the build list has no version except through list-packages

Checked against copr-cli 2.6 (PyPI) and 2.7 (Fedora 43) in a throwaway `fedora:43` container.

- `copr-cli --config F whoami` prints the `username` of the config file and makes no request. A
  config with a made-up token answers with its user name and exit 0. A missing file exits 6. So it
  proves the file parses, not that the token works; the token's age is `COPR_TOKEN_EXPIRES`. A
  token the server refuses shows only at a request that needs it (`build` prints "Login
  invalid/expired" and exits 1).
- `list-builds --output-format json` has only `id`, `name` and `state`. The version of a build
  (`source_package.version`, `version-release`) is in `list-packages <owner/project>
  --with-all-builds --output-format json`, a list of packages each with a `builds` list. The list is
  limited by the server, not by copr-cli (10 builds for most packages of @copr/copr, 18 for one;
  newest first there); `latest_build` and `latest_succeeded_build` are the server's own and are
  also in the output with `--with-latest-build --with-latest-succeeded-build`. A project that does not exist is exit 1 with "Project ... does not exist".
- `build --nowait` prints a line `Created builds: <id> [<id>...]` and `watch-build <ids>` exits
  non-zero when a build failed or was cancelled.
- `get` (the project) has no JSON output.
- The 180-day token expiry is tracked by the repository variable `COPR_TOKEN_EXPIRES`
  (`YYYY-MM-DD`, UTC, the last good day).
