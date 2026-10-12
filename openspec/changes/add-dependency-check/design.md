## Context

See proposal.md for why. This section records what the code does today, verified by reading it
on 2026-10-12, and the constraints that shape the design.

**The checks that exist today.**

- **GUI `check_cli`** (`gui/src-tauri/src/commands.rs`, about line 3523) calls `probe_cli`,
  which spawns `<proton_cli> --version`. It counts any successful spawn as installed and ignores
  the exit status on purpose: its comment and the tests
  `a_missing_binary_is_the_only_thing_that_reads_as_not_installed` and
  `a_present_binary_that_exits_non_zero_is_still_present` record "presence, not health"
  (DEVIATIONS §72). It already bounds the wait (10 s), nulls stdin, stdout and stderr, and reaps
  the child. Its reply is `{ installed, distro }`.
- **Readers of that reply:**
  - `api.js` (the command, the fixture lookup and the default payload);
  - `app.js` (`cliPresence?.installed === false` picks the `cliMissing` dialog, and
    `ensureCliCheck` drives "Check again");
  - `routes.js`;
  - `fixtures/onboarding.js` (`9a CLI missing`) and `fixtures/fids.js`;
  - `gui/test/routes.test.js`;
  - `tools/fidelity/check-n1-identity.mjs` and `check-pair-routing.mjs`;
  - the command-classification table in `commands/pair_tests.rs`.
- **The `cliMissing` screen** (`screens/onboarding.js`, `renderCliMissing`) draws a title, one
  body sentence with a `Detected <distro>` clause, and "Check again". It does not draw the
  "Installation help" button that frame `9a CLI missing` shows. The copy (`copy.js`, about line
  1543) explains the missing link: nothing named an install page for the CLI (DEVIATIONS §102).
  Proton's page is now known, so that reason no longer holds.
- **`setup.sh`** warns in `check_common_prereqs`, before the build, when `proton-drive` is not on
  `PATH`, and carries on. The terminal flow is: build, `write_config`, `install_unit`, then
  `preview_and_start`, which runs the dry run and stops if it fails. `upgrade.sh` and
  `uninstall.sh` source `setup.sh`. `main` is guarded and `upgrade.sh` calls
  `install_engine_binaries`, so nothing new may go into a shared function.
- **The daemon** reads the keyring at boot when any pair streams events. When it cannot, it
  reports the cause once through `note_degraded_session_if_needed`, which follows the
  one-reason-per-cause invariant. A pass refused for the session sets the `auth` state, with the
  reason `AUTH_DECLINE_REASON`.

**The control CLI's shape.** `proton-sync` resolves the socket path in `main` before it
dispatches, and every verb is a socket round trip. When no runtime directory is set, the
default-socket fallback (`paths::default_socket_path` → `ensure_private_runtime_dir`) calls
`create_dir_all`. The `--pair` path dials the socket (`capability_gate`) before the verb runs.

**Keyring coordinates.** `src/session.rs` holds them as private constants: service
`ch.proton.drive/drive-sdk-cli`, account `auth-session`. `read_keyring_session` captures stdout,
which is the secret.

**Test isolation.**

- `tests/common::sandbox` removes both D-Bus addresses and puts refusing `secret-tool` and `curl`
  stubs first on `PATH`.
- `run_bounded` fails any run that reached one of those stubs.
- `systemctl` is not stubbed. `tests/scripts.rs` puts its own `systemctl` stub after the refusing
  stubs.
- `refuse_the_real_cli` covers `proton-syncd` only.

**Docs site.** Astro with Starlight and the Sätteri markdown processor, `site` defaulting to
`https://osirison.github.io` and `base` to `/proton-drive-sync-engine`. An existing in-page link
(`/desktop/screens/#changing-a-folders-proton-drive-folder`) shows that heading ids drop
apostrophes, as github-slugger does. The sibling change rewrites `start/installation.md`'s
prerequisite section after the first release (its task 8.2).

**The wrong sign-in command.** A scan of every tracked file, matching across line breaks, finds
13 occurrences of `proton-drive login`. Nine are live:

- `src/daemon.rs:10448` (`AUTH_DECLINE_REASON`)
- `src/bin/proton-sync.rs:837`
- `tests/ipc_cli.rs:944`
- `setup.sh:501`
- `gui/src/js/screens/main.js:523` (comment)
- `gui/src/js/ui/copy.js:1767` (comment)
- `website/src/content/docs/cli/reference.md:325`, wrapped across lines 325–326, which
  `git grep` misses
- `website/src/content/docs/desktop/overview.md:70`
- `website/src/content/docs/desktop/screens.md:49`

Four are dated records:

- `docs/agent-notes/python-patch-scripts-and-rust-string-continuations.md:13`
- `docs/design-v1-old/Proton Sync UI.dc.html:1691` and `:1752`
- `docs/design-v2/DEVIATIONS.md:1700`

**Dependencies already in the tree.** `wait-timeout` (bounded child waits) and `libc` (signal
numbers) are already dependencies. `regex` is not.

## Goals / Non-Goals

**Goals:**

- One definition of each check, one set of links and one wording, shared by the four callers.
- No check can hang a caller, read the session secret, or start a second CLI client.
- Every branch can be tested without a real `proton-drive`, keyring or service manager.

**Non-Goals:**

- Checking that Proton still accepts the stored session. That takes a network call, and the
  daemon's `auth` state already answers it after the first pass.
- Checking the environment of the systemd user manager from a terminal. The daemon's start-up
  log covers the service's own environment.
- Putting check results on the daemon's status reply.
- Running the check in package scriptlets. They run as root, with no user session or keyring.
- Any change to `upgrade.sh` or `uninstall.sh`.
- Installing anything. The check reports problems and links to the fix; it never fixes them.

## Decisions

### D1. One module, `src/doctor.rs`, in the engine library

The check lives in the engine library because all four callers can reach it there. The control
CLI and the daemon are in the crate, and the desktop app reaches the engine through `gui-core`,
which already depends on it and re-exports engine types (`gui_core::wire`). The module shares its
name with the verb, so one thing has one name. "Preflight" is already the name of a step in the
sibling change's release workflow.

The module exposes:

- `CheckId`: `proton-drive`, `signed-in`, `config`, `secret-tool`, `curl`, `session-bus`,
  `service`.
- `Severity`: `required` or `recommended`, fixed per id.
- `Outcome`: `ok`, `missing`, `failed` or `unknown`.
- `CheckResult`: `{ id, severity, outcome, statement, fix, links }`.
- `Report`: `{ checks, verdict }`, where the verdict is `ok`, `failed` or `unknown`.
- `run(&Inputs, &dyn Environment) -> Report`, where `Inputs` carries the CLI path, the config
  path, which checks to run, and how the CLI path was chosen.

All of these derive `Serialize`, so `--json` and the desktop app use one shape.

Alternatives rejected:

- **In `gui-core`.** The daemon and the control CLI cannot depend on it.
- **In the `proton-sync` binary.** The desktop app and the daemon cannot reach it.
- **A shell copy in `setup.sh`.** That would be a second definition, which is the bug shape this
  repo keeps finding.

### D2. Every outside fact comes through one injected `Environment`

`Environment` has four methods:

- `find_program(name) -> Option<PathBuf>`: a `PATH` search that executes nothing.
- `var(name) -> Option<OsString>`.
- `read_file(path) -> io::Result<String>`, for the config.
- `run(program, args, limit, stdout: Keep | Discard) -> Ran`.

The answer from `run` is one of:

- `Exited { code, stdout_head, stderr_head }`;
- `Signalled(signal)`;
- `NotFound`, `NotExecutable` or `ExecFormat`;
- `SpawnError(kind)`;
- `TimedOut`.

The real implementation, `SystemEnvironment`, works as follows:

- stdin is `/dev/null`.
- stdout is `/dev/null` when `Discard`, and piped otherwise.
- stderr is always piped.
- Only the first line of each pipe is kept, cut to 200 characters.
- The child is waited on with `wait-timeout`, and killed and reaped on timeout.

A test environment is a table of answers. No unit test spawns anything.

Presence of `secret-tool`, `curl` and `systemctl` is a `PATH` search, not a run. That keeps the
sandbox's refusing stubs from being reached, and it is how the shell's `command -v` behaves.

### D3. `proton-drive`: run `version`, judge how it ended

The program is resolved first:

- a value containing `/` is used as it is;
- a bare name is searched on `PATH`;
- not found means `missing`, without spawning anything.

The resolved absolute path is then run with the single argument `version`, with a limit of 10 s
(the GUI probe's existing bound). `version` is the subcommand Proton's README documents. The GUI's
`--version` was never checked against the real CLI, and this change stops using it.

| How it ended | Outcome | Statement / fix |
| --- | --- | --- |
| exit 0 | ok | first line of stdout (the version) and the absolute path |
| `NotFound` (on `PATH` or at the path) | missing | install from the download page, then `proton-drive auth login`; if it is installed off `PATH`, set `proton_cli` to its full path |
| `NotExecutable` (`EACCES`) | failed | "found at P but not executable": reinstall from the download page |
| `ExecFormat` (`ENOEXEC`) | failed | "not a program this system can run": download the Linux x64 build |
| `Signalled(SIGILL)` | failed | "crashed with an illegal instruction": the CPU probably lacks AVX2, so use the `linux/x64-baseline` build |
| other signal | failed | "crashed (signal N)": reinstall from the download page |
| exit ≠ 0 | failed | "`version` exited with status N: <first stderr line>": reinstall or update from the download page |
| `TimedOut` | unknown | "did not answer within 10 s" |
| other `SpawnError` | unknown | the error kind |

SIGILL is detected with `ExitStatusExt::signal() == Some(libc::SIGILL)`, never from stderr text.

**This reverses DEVIATIONS §72's "presence, not health".** That rule existed so that a CLI that
was installed but broken would not be sent to an install screen. A separate "found but does not
run" result with its own fix now answers that concern, so judging the exit status no longer brings
the problem back. The two GUI tests that pin the old rule are deleted. Their cases move to the
engine's unit tests with the second one inverted: a non-zero exit is `failed`, and still not
`missing`.

### D4. `signed-in`: a status-only keyring lookup that never reads the secret

The steps run in order, and the first that applies decides the result:

1. `PROTON_DRIVE_CREDENTIALS_STORE` is set to `pass` or `unsafe_file` → `unknown`. The sign-in is
   kept where neither this check nor the daemon's change detection reads it.
2. `secret-tool` is not on `PATH` → `unknown` ("secret-tool is not installed").
3. `DBUS_SESSION_BUS_ADDRESS` is unset or empty → `unknown` ("no session bus in this shell"). The
   lookup is not run, which also keeps sandboxed tests from reaching the stub.
4. Otherwise run `secret-tool lookup service ch.proton.drive/drive-sdk-cli account auth-session`
   with stdout to `/dev/null`, stderr captured, and a 60 s limit:
   - exit 0 → `ok`, "a sign-in is stored";
   - non-zero with empty stderr → `missing`;
   - non-zero with stderr → `unknown`, quoting the first line;
   - `TimedOut` → `unknown`, "the keyring did not answer within 60 s (an unlock prompt may be
     waiting)".

**stdout goes to `/dev/null`, so the secret is never in this process.** It is not read, kept,
printed or logged.

The coordinates and the argument list are written once in `session.rs`, as a `pub(crate)`
function that builds the lookup arguments. `read_keyring_session` and this check both call it.

The `missing` fix reads: "run `proton-drive auth login`. If you were asked to unlock the keyring
and dismissed it, check again and unlock it." That wording covers the case in the open question
below.

The lookup can raise the desktop keyring's unlock prompt. The 60 s limit gives a person time to
answer it.

### D5. Severities

- **Required:** `proton-drive`, `signed-in` and `config`. Without any one of them no pass can
  work, or the daemon does not start at all.
- **Recommended: `secret-tool`, `curl` and `session-bus`.** Without them the daemon cannot follow
  Proton's change feed, so every pass walks the whole Proton folder (`session.rs`,
  `note_degraded_session_if_needed`). That is correct but slow, and the wording says exactly this.
  `session-bus` is not required because doctor's shell is not the service's environment. An SSH
  shell has no bus while the user service does, and blocking `setup.sh` over SSH would refuse a
  setup that works.
- **Recommended: `service`.** The daemon runs without systemd, but the desktop app's Start button
  needs the unit. The check runs `systemctl --user show proton-syncd.service -p LoadState --value`
  with a 10 s limit, the same as the CLI probe:
  - `loaded` → ok;
  - `not-found` → missing;
  - `masked` → failed;
  - non-zero exit, or `systemctl` not on `PATH` → unknown.

  It reads a structured value, so it matches no message text. It is read-only, and never starts
  or enables anything.

### D6. The config check is in, and the default config path moves into the library

Without a config, a `doctor` run by hand would check `proton-drive` on `PATH` while the service
runs the `proton_cli` that the app wrote. An invalid config is also a daemon that will not start.
The check is cheap: `config::validate_file_config_text` already exists. Its doc comment says it
touches nothing on the filesystem, and reading its body (`config.rs`, about line 2381) agrees: it
parses, resolves the pairs and checks values, and it reads only `$HOME` (for `~` expansion). It
does not use the real-path check `real_path_conflicts`.

Which file is read:

- `--config` if given;
- otherwise the default `$XDG_CONFIG_HOME/proton-sync/proton-sync.toml`, else
  `~/.config/proton-sync/proton-sync.toml`.

That default is defined today in `gui/src-tauri/src/config_path.rs` (`gui_config_path`), in
`setup.sh` (`default_config_path`) and in the packaged units (`%h/.config/...`). The Rust
definition moves to `paths::default_config_path`, and the GUI delegates to it, as #277 did for
the socket. The shell copy stays: a script cannot call Rust, and `setup.sh` always passes
`--config`.

The results:

| Case | Result |
| --- | --- |
| default file absent | ok, "no config file yet" (normal before first run) |
| named file absent | failed |
| file present and invalid | failed, quoting the validator's error |

**Which CLI is checked:** `--proton-cli`, else the file's `proton_cli` when the file validated,
else `proton-drive`. When the file did not validate, the `proton-drive` statement says it checked
`proton-drive` on `PATH` because the config could not be read.

Alternatives rejected:

- **Leave config out.** `doctor` would ignore the CLI path the service uses.
- **Require `--config`.** It is the same problem for anyone who omits it.
- **Validate with `resolve_runtime_configs`.** That also checks the roots and the lock paths,
  which belong to the daemon, and it would reject a file in which only flags are missing.

### D7. Three results, and exit codes 0, 1 and 3

Wire tokens are `ok`, `missing`, `failed` and `unknown`. `missing` and `failed` are the two kinds
of problem, and the spec states the three classes. The verdict is `failed` if any required check
is `missing` or `failed`, else `unknown` if any required check is `unknown`, else `ok`. Exit codes
are 0, 1 and 3, mapped from the verdict.

**Not 2:** clap exits 2 on a usage error. An older `proton-sync` that has no `doctor` (for example
under `setup.sh --no-build`) exits 2 with "unrecognized subcommand", and `setup.sh` must not read
that as "could not check".

Alternatives rejected:

- **0 and 1 only.** A `pass` user, or `setup.sh` over SSH, would be blocked for ever.
- **0, 1 and 2.** It collides with clap's usage error.

### D8. `proton-sync doctor` is dispatched before the socket exists

`main` matches `Commands::Doctor` first, before `resolve_control_socket_path` and before the
`--pair` and `--all-pairs` branches. So `doctor`:

- never resolves the socket path, and so never runs the fallback's `create_dir_all`;
- never dials the socket;
- never starts `proton-syncd`.

This is the first verb that does not talk to the daemon, and the CLI reference says so.

It refuses `--socket-path`, `--pair` and `--all-pairs` with exit 2 and the sentence "doctor
checks this machine, not the daemon; it takes no socket or folder pair". A flag that had no effect
would suggest a per-pair answer that does not exist.

`doctor` accepts:

- `--proton-cli PATH`, a subcommand option with the daemon's flag name;
- `--config`, which stays a top-level option given before the verb, as for every verb today;
- `--json`, which is already global.

`--config` is not made global. That would add a parse path to every verb for the sake of one.

The full form is `proton-sync [--config FILE] [--json] doctor [--proton-cli PATH]`. A user who
types `proton-sync doctor --config F` gets clap's usage error (exit 2), which names the flag.

Human output on stdout, one line per check:

```
proton-sync doctor: what this machine needs before the first sync

  ok       proton-drive   proton-drive 1.4.2 at /home/me/.local/bin/proton-drive
  missing  signed in      no proton-drive sign-in is stored in the keyring
           fix:  run `proton-drive auth login`, then run `proton-sync doctor` again
           help: https://proton.me/support/drive-cli
           docs: https://osirison.github.io/proton-drive-sync-engine/reference/troubleshooting/#no-proton-drive-sign-in-is-stored
  ok       config         /home/me/.config/proton-sync/proton-sync.toml
  ...
1 required problem. Fix it and run `proton-sync doctor` again.
```

Every URL stands alone, followed by a newline. The `ok` line prints the CLI's absolute path,
because the service may resolve a bare `proton_cli` against a different `PATH` (troubleshooting,
"After a reboot …").

Colour follows the existing `Style::for_stdout()` rule.

### D9. One set of links, and a test that every anchor exists

`doctor::links` holds the following.

- **Proton's pages** (the four URLs in the sibling's `linux-packages` spec):
  - download: `https://proton.me/drive/download`
  - direct download: `https://proton.me/download/drive/cli/index.html`
  - support page: `https://proton.me/support/drive-cli`
  - README: `https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md`
- **The sign-in command:** `proton-drive auth login`.
- **`DOCS_SITE`:** `https://osirison.github.io/proton-drive-sync-engine`.
- **Each check's anchor** on `/reference/troubleshooting/`.

Each `CheckResult.links` lists `{ id, url }` pairs drawn only from this set.

**Where the sections go.** A new "Dependency check" section in `reference/troubleshooting.md`,
with one `###` subsection per check. It does not go in `installation.md`, because the sibling
rewrites that page's prerequisite section and anchors there would move. Proposed headings and
their slugs:

| Check | Heading | Anchor |
| --- | --- | --- |
| `proton-drive` | `proton-drive is missing or does not run` | `proton-drive-is-missing-or-does-not-run` |
| `signed-in` | `No proton-drive sign-in is stored` | `no-proton-drive-sign-in-is-stored` |
| `config` | `The config file cannot be read` | `the-config-file-cannot-be-read` |
| `secret-tool` | `secret-tool is missing` | `secret-tool-is-missing` |
| `curl` | `curl is missing` | `curl-is-missing` |
| `session-bus` | `No session bus` | `no-session-bus` |
| `service` | `The proton-syncd service is not installed` | `the-proton-syncd-service-is-not-installed` |

**The slug rule.** A heading a check links to may contain only ASCII letters, digits, spaces and
hyphens. Its slug is the heading lower-cased, with each space replaced by a hyphen. On that
subset github-slugger and any common slugger agree, so the rule does not depend on what Sätteri
does with punctuation.

**The test** is `tests/doctor_links.rs`. It spawns nothing. For every anchor the module can
emit, it asserts all of the following:

- the page exists under `website/src/content/docs/`;
- one heading has exactly that slug;
- that heading uses only the subset above;
- the defaults in `website/astro.config.mjs` (`site` and `base`) concatenate to `DOCS_SITE`.

A one-time check during implementation confirms the ids in the built HTML (task 6.3).

### D10. Desktop app: same command name, new reply, links by id

`check_cli` keeps its name, so the readers listed in Context, including the `pair_tests.rs`
classification, change only where they read the reply. The new reply is
`{ ready, report, distro }`, where `ready` is false exactly when the verdict is `failed`. The
command builds `Inputs` from `RuntimePaths` (its configured `proton_cli` and config path) and
runs the full check on `spawn_blocking`. `probe_cli` and its constant are deleted. `gui-core`
gains a thin `doctor` module that maps `RuntimePaths` values to `Inputs` and re-exports the
types.

The dialog appears when `ready === false`, which replaces `installed === false`. A required
`unknown` and every recommended result do not block first run, because a `pass` user must not be
locked out.

The screen lists every result that is not ok:

- required problems first, then recommended problems, then could-not-check results;
- each row has the statement, its fix, a "How to fix" link to the troubleshooting anchor, and for
  `proton-drive` and `signed-in` rows also Proton's page.

The title depends on the first required problem:

- "Proton Drive's command line tool isn't installed" (missing CLI);
- "Proton Drive's command line tool doesn't run" (failed CLI);
- "Sign in to Proton Drive's command line tool" (`signed-in`);
- "Your settings file can't be read" (`config`).

The `Detected <distro>` clause stays as its own line, because nothing in it is false.

"Installation help" is drawn. It opens `https://proton.me/drive/download`. "Check again" re-runs
the whole check.

**Links are opened by id.** A new command, `open_help_link { check, link }`, looks the URL up in
the engine's table and hands it to the same `open_target(OPENER, …)` path that `open_remote`
uses. The page never passes a URL, which keeps `open_remote`'s property that the webview has
nothing to inject.

**"Auth expired" hero** (`main.js`): the sub-line gains "Run `proton-drive auth login` in a
terminal, then try again." A second control, "Sign-in help", opens `proton-cli-docs` through
`open_help_link`. "Try again now" stays. This hero is not a drawn frame (`copy.js` says
`authExpired` is undrawn), so it changes no fidelity row. It needs a `main-screen.test.js` case.

**Fidelity bookkeeping.**

- A new DEVIATIONS section:
  - reverses §102's "no URL" (Proton's page is now known);
  - reverses §72's "presence, not health" (D3);
  - records the checklist that replaces the frame's command box.
- Updated `known-deviations.mjs` rows for `9a CLI missing`.
- `copy-gate.mjs` `DRAWN` rows for strings the frame draws and the build now renders
  ("Installation help").
- The `9a CLI missing` fixture changes to the new reply shape.
- New non-frame test cases in `gui/test/onboarding.test.js`: crashed, signed out, could not
  check.
- No published screenshot shows `9a` or the auth-expired hero (`render-screenshots.mjs`
  `PUBLISHED_SHOTS`), so `npm run screenshots` is not needed.

### D11. `setup.sh`: one call, outside every shared function

A new function, `run_dependency_check MODE`:

- resolves `proton-sync` with `resolve_installed_bin proton-sync`;
- runs `"$bin" --config <file> doctor` in terminal mode, with `--proton-cli <value>` added after
  `doctor` when setup's flag was given. `--config` comes before the verb, because it is not a
  global flag (D8);
- in `--gui` mode passes no `--config`, because no config is written yet. `doctor` then reads the
  default path, the one the wizard writes to, and an absent file reads as "no config file yet"
  rather than as a named file that is missing;
- shows the output;
- branches on the status:

| Status | Terminal mode | `--gui` mode |
| --- | --- | --- |
| 0 | continue | continue |
| 3 | warn, continue | warn, continue |
| 1 | stop, exit non-zero (continue with `--ignore-checks`) | note "the app runs the same check", continue |
| anything else (2: an older `proton-sync` without `doctor`) | stop: "this proton-sync cannot run the check; rebuild without --no-build" (continue with `--ignore-checks`) | note, continue |

**Where it is called:**

- terminal mode: in `run_terminal_setup`, after `write_config` and before `install_unit`, so
  doctor reads the exact file the service will read;
- `--gui` mode: in `run_gui_setup`, after `install_engine_binaries` and before the slow GUI build.

It is not called from `check_common_prereqs`, `install_engine_binaries` or anything else that
`upgrade.sh` reaches. The shell `proton-drive` warning in `check_common_prereqs` is deleted, and
`preview_and_start`'s error says `proton-drive auth login`.

**`--ignore-checks`** runs and prints the check but does not stop on it. A false positive, such
as a future CLI whose `version` exits non-zero, must not make setup unusable, and the dry-run gate
still follows. `--help` lists the flag.

`--gui` mode does not stop because the app runs the same check, blocks first run with links and
"Check again", and stopping here would mean a rebuild after the fix.

Alternatives rejected:

- **Keep a shell pre-check before the build.** It is a second definition of a check.
- **Run an older installed `doctor` before the build.** Its result would come from a different
  version of the check.

The cost is that a user without the CLI learns about it only after the build (minutes).

### D12. Daemon: the start-up check covers only what nothing else reports

In `Daemon::run`, after the control-socket task is spawned (so status answers at once) and before
the first step, the daemon runs `doctor::run` on a blocking thread, through the `Environment` it
holds. How it gets one:

- The field's default is an inert environment: it finds nothing, runs nothing and reports the
  checks as not run.
- Only `Daemon::new`, the production door, installs `SystemEnvironment`.
- `from_pairs` keeps its signature. It has 24 uses in `src/daemon.rs`, and a new parameter would
  change every one of them.
- A test that exercises the start-up check sets a fake through a `#[cfg(test)]` setter. No lib
  test can reach a real tool by accident.

It runs two checks:

- `proton-drive`, always, with the resolved `proton_cli`. The service's environment (the systemd
  user manager's `PATH`) is exactly what doctor in a terminal cannot see.
- `curl`, only when `any_pair_streams`.

Each result that is not ok is logged once at WARN, with `check`, `fix` and `docs` fields. An ok
CLI gets one INFO line with its version and path. The daemon keeps running (M1).

It does not run `signed-in`, `secret-tool` or `session-bus`. The degraded-session report
(`note_degraded_session_if_needed`, one reason per cause) and the `auth` state already speak for
those causes, and a second line for the same cause breaks that invariant. Instead:

- the no-session reason gains the docs link for `signed-in`;
- `AUTH_DECLINE_REASON` becomes "the proton-drive CLI session is signed out or expired; run
  `proton-drive auth login` (the daemon reuses that CLI's session)" plus the docs link.

It skips `config` (already validated, and fatal) and `service` (the daemon may be the service).

**Not on the status reply.**

- The desktop app runs the check in its own process.
- The service's view differs only in its environment, and that difference already shows as this
  log line and as the first pass's `last_error`.
- A status field would be a second published copy of the answer.

This can be added later, additively and with `#[serde(default)]`, if the app needs the daemon's
view.

**The binary tests.** `version` is not taken through `CliGate`. At that moment no pass is running.
The only possible overlap is a `list` request on the IPC task, which is accepted under assumption
A2.

Binary-level tests that start `proton-syncd` with a fake `proton_cli` see one extra `version` call
at boot. Tests that count or log fake-CLI calls are audited (task 4.3).

### D13. Fixing the sign-in command, and the guard

The nine live sites in Context become `proton-drive auth login`. The `tests/ipc_cli.rs` assertion
checks the new text. The dated records under `docs/` stay as written, and the new DEVIATIONS
entry records the correction.

**The guard** is `tests/sign_in_command.rs`. It spawns nothing and adds no dependency.

- **What it reads:** every file under `src/`, `tests/`, `gui/src/`, `gui/src-tauri/src/`,
  `gui/gui-core/src/`, `gui/tools/`, `website/src/content/`, `packaging/` and `examples/`, plus
  `README.md`, `setup.sh`, `upgrade.sh` and `uninstall.sh`.
- **What is exempt:** `docs/` and `openspec/`, which are records and plans that quote the wrong
  command on purpose.
- **How it normalises each file:**
  - it joins lines;
  - it drops leading comment and quote leaders (`//`, `#`, `*`, `>`) from continuation lines;
  - it drops backticks and quotes;
  - it treats a trailing `\` line continuation as whitespace;
  - it collapses runs of whitespace.
- **What fails it:** `proton-drive login` with no `auth` between the two words. The failure names
  the file and the line.
- **How it avoids matching itself:** the needle is built from parts.

### D14. Test isolation for `doctor`

**Unit tests.** Every outcome row in D3, D4, D5 and D6 is a unit test against a fake
`Environment`, including the reversed §72 case and the rule that nothing is read from a lookup's
stdout. For that rule, the fake records the stdout mode it was asked for, and the test asserts
`Discard`.

**Binary tests** go through a new helper in `tests/common`, `DoctorTools`:

- It writes answering stubs for `proton-drive`, `secret-tool` and `systemctl` into the test's
  directory, each with a chosen exit status, stderr or signal.
- Its `attach` sets `PATH` to `<answering stubs>:<refusing stubs>` with **no system directory**.
  A tool a test forgot is then reported missing and cannot be the real one.
- Because `PATH` holds no system directory, a stub may use only shell builtins and absolute
  paths:
  - the timeout stub runs `exec /bin/sleep 30`, not `sleep 30`, which would fail inside the stub
    with "not found" and pass the test for the wrong reason;
  - the signal stub runs `kill -s ILL $$` (a builtin).

  The `#!/bin/sh` line is already absolute.
- It sets a marker variable.

`refuse_the_real_cli` is extended to refuse a `proton-sync doctor` command without that marker.

The sandbox removes `DBUS_SESSION_BUS_ADDRESS`, so the keyring lookup runs only in a test that
sets a fake address on purpose. Such a test also installs an answering `secret-tool`, and
`run_bounded` still fails a run that reaches a refusing stub.

`tests/test_isolation.rs` pins the helper the way it pins `ScriptedEvents`.

**The release smoke test** runs `doctor` in a throwaway container, outside `tests/`, and is the
sibling change's.

## Risks / Trade-offs

- **[`proton-drive version` touches shared state]** (assumption A2). → If verification shows it
  opens the CLI's cache or `events.lock`, `doctor` first probes the user-global lock the way
  `--dry-run` does (`probe_daemon_lock`: shared, non-blocking, released at once). When a daemon
  holds that lock, `doctor` reports `proton-drive` as `unknown` ("the daemon is running; its
  start-up log checks the CLI") instead of running `version`. Task 1.1 settles this before any
  code is written.
- **[A dismissed unlock prompt reads as "no sign-in stored"]** (assumption A1). → The `missing`
  fix names that case. "Check again" costs one click.
- **[Doctor's environment is not the service's]** → The `ok` line prints the absolute path. The
  daemon's start-up check logs the service's own view. The troubleshooting section explains
  setting an absolute `proton_cli`.
- **[First run now blocks on sign-in]** → It is the maintainer's request. "Check again" releases
  it, and `unknown` never blocks.
- **[The check raises a keyring prompt at first run or in `doctor`]** → That is the same prompt
  the daemon raises at its first start. A 60 s limit applies, and the wording says a prompt may be
  waiting.
- **[A wording change breaks the sibling's smoke test]** → That test compares only the URL, the
  exit status and the name `proton-drive`. All three are fixed by the spec.
- **[A future CLI changes `version`]** → `setup.sh --ignore-checks` covers it, and the desktop app
  shows the exit status in the fix line.
- **[The slugger differs from the rule]** → Headings are restricted to a subset every slugger
  agrees on, and the built HTML is checked once (task 6.3).
- **[The user learns about a missing CLI only after the build]** → Accepted (D11). The README and
  the installation page put the CLI first.

## Migration Plan

1. Merge this change before the first release tag, which the sibling's task 7.2 gates on it. No
   data migration: no index, config or wire field changes. The GUI reply shape changes, but the
   GUI and its Tauri commands ship together.
2. The sibling then adds the `doctor` check to `packaging/smoke-test.sh` (its task 3.10).
3. **Coordination with `add-linux-release-pipeline`.**
   - Its tasks 8.2 and 8.3 rewrite `installation.md` and `README.md` after the release. They must
     keep the `proton-sync doctor` line this change adds there.
   - Its 8.4 grep leaves the desktop pages to this change, and D13 fixes them.

**Rollback:** revert the merge. Nothing persistent depends on it.

## Open Questions

- **A1.** Does `secret-tool lookup` exit non-zero with empty stderr for "no such item" and print
  the error for every other failure? The design depends on that split. Is a dismissed unlock
  prompt reported as an error or as "no item"? Settle it by reading libsecret's
  `tool/secret-tool.c` (task 1.2). If errors turn out to be silent too, D4's `missing` wording
  already names the other case, and no spec scenario changes.
- **A2.** Does `proton-drive version`:
  - exit 0 offline, and without a session;
  - open no file under the CLI's data directory (`~/.local/share/proton-drive-cli/`, its SQLite
    cache, `events.lock`);
  - make no network connection?

  The maintainer settles this on a machine where running the real CLI is safe, or from the CLI's
  source (task 1.1). D12 and the first risk above give the fallback.
- **A3.** Settled on 2026-10-12 (systemd on Fedora 44): `systemctl --user show
  no-such-unit-xyz.service -p LoadState --value` printed `not-found` and exited 0. Task 1.3 re-runs
  it on the CI image.
- Should recommended problems and could-not-check results also appear somewhere in the desktop
  app after first run? Today they appear only on the blocking screen and in `doctor`. Adding a
  notice later changes nothing here.
