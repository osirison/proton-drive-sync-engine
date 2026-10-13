## Why

The sync needs Proton's `proton-drive` CLI, installed and signed in, plus a few desktop tools, and
nothing checks for them before the first sync. Today a missing or broken CLI shows up as a failed
pass, the desktop app counts any CLI that starts as "installed" (even one that crashes), nothing
checks sign-in, no screen links Proton's install page, and every place that gives the sign-in
command gives the wrong one (`proton-drive login`; Proton's is `proton-drive auth login`). The
maintainer asked on 2026-10-12 for one check that runs before the first sync, recommends only
Proton's official CLI and points at the right docs. The first release (`add-linux-release-pipeline`)
waits for this change: its install smoke test runs `proton-sync doctor`.

## What Changes

- One dependency check in the engine library, used by four callers. Each check has an id, a
  severity (required or recommended), one of three results (ok, a problem, could not check), a
  fix, and links. The checks:
  - the `proton-drive` CLI runs (`<proton_cli> version`, judged by exit status, with "not found",
    "not executable", "crashed" and "exited non-zero" told apart);
  - a sign-in is stored (a `secret-tool lookup` judged by exit status; the secret itself is never
    read);
  - `secret-tool` and `curl` are installed;
  - a session bus address is set;
  - the `proton-syncd` user service is installed;
  - the config file can be read.
- **New verb `proton-sync doctor`.** It runs the check without the daemon socket. It is the only
  verb that does not talk to the daemon. It prints a human report, or JSON with `--json`. Exit
  status: 0 when every required check passes, 1 when a required check fails, 3 when none fails but
  one could not be checked. It runs only `proton-drive version`: no listing, and no second CLI
  client beside a running daemon.
- **Desktop app first run.** The `check_cli` command runs the same check instead of its own probe.
  The "CLI missing" screen becomes a list of the problems found, each with its fix and link. It
  draws the "Installation help" button, which opens Proton's download page, and keeps
  "Check again". **BREAKING (GUI behaviour):** a `proton-drive` that starts but exits non-zero or
  crashes now blocks first run as "found but does not run". The old rule ("presence, not health",
  DEVIATIONS §72) is reversed, and so is "no URL" (§102). The "Auth expired" hero gains the
  instruction `proton-drive auth login` and a link to Proton's CLI page.
- **`setup.sh`.** After it builds and writes the config, and before it installs and enables the
  service, it runs the built `proton-sync doctor`. It stops when a required check fails, warns and
  carries on when one could not be checked, and `--ignore-checks` overrides the stop. In `--gui`
  mode it prints the report and carries on, because the app runs the same check. The shell's own
  `proton-drive` warning is removed.
- **Daemon.** At boot it runs the parts of the check it alone can see, which are whether the CLI
  runs in the service's own environment and whether `curl` is present. It logs each problem once,
  with its fix and link, and keeps running (maintainer decision M1). The status reply is not
  changed.
- **One set of links.** One constant set in the library holds Proton's download, direct-download
  and docs URLs, the sign-in command, and one anchor per check on the docs site's troubleshooting
  page, with the site's base path. A test checks that every anchor the check emits exists in the
  docs source.
- **The wrong sign-in command is fixed everywhere.** `proton-drive login` is replaced with
  `proton-drive auth login` at 9 sites:
  - the daemon, the control CLI and one test;
  - `setup.sh`;
  - two comments in the desktop app;
  - three docs pages, one of them a line-wrapped occurrence that plain `git grep` misses.

  Four more copies sit in dated design records under `docs/` and stay as written. A test stops
  the old spelling coming back.
- **Docs.** The troubleshooting page gets one section per check. The CLI reference documents
  `doctor` and its exit codes. The README, the installation page and the quick start say to run
  `proton-sync doctor` after installing.

## Capabilities

### New Capabilities

- `dependency-check`: what is checked before the first sync and by whom, the three results and the
  exit status, which links and fixes each problem carries, and what each caller does with the
  answer (the `doctor` verb, the desktop app's first run, `setup.sh` and the daemon's boot log).

### Modified Capabilities

<!-- None. The only existing capability is local-deletion-disposal; nothing here changes how the
     engine disposes of local files. -->

## Impact

- **Engine library**:
  - new module `src/doctor.rs`;
  - `src/session.rs` shares its keyring coordinates and lookup arguments with it;
  - `src/paths.rs` gains the default config path that the desktop app and the packaged unit
    already use;
  - `src/daemon.rs` gains the boot-time check and fixes `AUTH_DECLINE_REASON`.
- **Control CLI** (`src/bin/proton-sync.rs`): the `doctor` verb is dispatched before the socket
  path is resolved, so it never resolves or creates the socket's directory. The `sign-in` status
  row's wording is fixed.
- **Desktop app**:
  - `gui/src-tauri/src/commands.rs`: `check_cli` changes reply shape, `probe_cli` is deleted, and
    a link-opening command takes link ids, never URLs;
  - `gui/gui-core` re-exports the check;
  - `gui/src/js`: the onboarding screen, the main hero, `copy.js`, `api.js`, `app.js`, the
    fixtures and `fids.js`;
  - the fidelity tools (`known-deviations.mjs`, `copy-gate.mjs`, `check-n1-identity.mjs`,
    `check-pair-routing.mjs`) and a new `docs/design-v2/DEVIATIONS.md` entry.
- **Scripts**: `setup.sh` gains the doctor step and the `--ignore-checks` flag. `upgrade.sh` and
  `uninstall.sh` source `setup.sh` and are unchanged.
- **Tests**:
  - unit tests for every result branch, through an injected runner;
  - binary-level `doctor` tests through `tests/common` with stub tools;
  - a `tests/scripts.rs` test of the setup gate;
  - the anchor test, and a guard against the old sign-in command;
  - `tests/ipc_cli.rs` updated, and every test that counts fake-CLI calls checked for the extra
    `version` call at boot.
- **Docs**: `website/src/content/docs/reference/troubleshooting.md`, `cli/reference.md`,
  `start/installation.md`, `start/quick-start.md`, `desktop/overview.md`, `desktop/screens.md`,
  and `README.md`.
- **Release pipeline**: the sibling change's install smoke test (its task 3.10) depends on the
  `doctor` output and exit status this change defines.
