## 1. Settle the three assumptions before any code

- [x] 1.1 A2 (design, Open Questions): find out what `proton-drive version` does. Either the
      maintainer runs it on a machine where running the real CLI is safe, or it is read from the
      CLI's source. The questions:
      - with the network down, and while signed out, does it still exit 0?
      - under `strace -f -e trace=%file,connect`, does it open nothing under
        `~/.local/share/proton-drive-cli/` and make no `connect`?

      Write the answers into design.md's Open Questions, replacing A2. If it touches shared
      state, task 3.7 is in scope; if not, mark 3.7 as not needed. Verify: design.md states the
      result and its source
- [x] 1.2 A1: read the lookup action in libsecret's `tool/secret-tool.c` and record three things,
      with a link to the source:
      - whether "no such item" exits non-zero with empty stderr;
      - whether every other failure prints to stderr;
      - what a dismissed unlock prompt returns.

      Verify: design.md states the result. If errors are silent, D4's `missing` wording already
      names that case, and no scenario changes
- [x] 1.3 A3: run `systemctl --user show no-such-unit-xyz.service -p LoadState --value; echo $?` and
      the same for an installed unit. It is read-only. Verify: design.md records the output (D5
      expects `not-found` with status 0)

## 2. Shared definitions and the docs anchors

- [x] 2.1 Add `paths::default_config_path()` to the engine: `$XDG_CONFIG_HOME` (absolute only,
      the existing `absolute_dir` rule), else `$HOME/.config`, then `proton-sync/proton-sync.toml`.
      Make `gui/src-tauri/src/config_path.rs`'s `gui_config_path` delegate to it (design D6).
      Verify: new engine unit tests cover a relative `XDG_CONFIG_HOME` being ignored, and the
      existing `config_path` tests pass (`cargo test --workspace config_path`)
- [x] 2.2 In `src/session.rs`, add one `pub(crate)` builder for the `secret-tool lookup service …
      account …` arguments and make `read_keyring_session` use it (D4). Behaviour does not
      change. Verify: `cargo test session::tests` passes, and a new unit test pins the exact
      argument list
- [x] 2.3 Add `doctor::links` (D9). It holds:
      - Proton's four URLs;
      - `proton-drive auth login`;
      - `DOCS_SITE`;
      - one troubleshooting anchor per check id.

      Verify: it compiles, and nothing outside the module spells these URLs (`git grep -n
      'proton.me/drive/download' -- src gui/src-tauri gui/gui-core` lists only `src/doctor.rs`)
- [x] 2.4 Add a "Dependency check" section to
      `website/src/content/docs/reference/troubleshooting.md`, with one `###` subsection per check
      using the D9 headings. Each subsection says:
      - what the result means;
      - the fix, linking only Proton's pages for the CLI;
      - for `proton-drive`, the `linux/x64-baseline` build for CPUs without AVX2, and setting an
        absolute `proton_cli` when the service cannot find it;
      - for `signed-in`, `proton-drive auth login`, the unlock-prompt case, and that `pass` or
        `unsafe_file` stores cannot be read;
      - for `secret-tool`, `curl` and `session-bus`, that sync still works but walks the whole
        folder on every pass.

      Link the existing "Remote operations fail" and "After a reboot …" sections to it.
      Verify: `(cd website && npm run build)` succeeds
- [x] 2.5 Add `tests/doctor_links.rs` (D9). It spawns nothing. For every anchor `doctor::links`
      can emit, it asserts:
      - the page exists;
      - exactly one heading has that slug;
      - the heading uses only ASCII letters, digits, spaces and hyphens;
      - `website/astro.config.mjs`'s default `site` and `base` concatenate to `DOCS_SITE`.

      Verify: `cargo test --test doctor_links` passes. Renaming one heading in a scratch edit
      fails it, naming the anchor and the page, and so does changing the `base` default. Revert
      both

## 3. The check module (`src/doctor.rs`)

- [x] 3.1 Add the types (`CheckId`, `Severity`, `Outcome`, `CheckResult`, `Report`, `Inputs`), the
      `Environment` trait and `SystemEnvironment` (D1, D2). `SystemEnvironment`:
      - nulls stdin;
      - keeps or discards stdout;
      - keeps the first line of each pipe, cut to 200 characters;
      - waits with `wait-timeout`, and kills and reaps the child on timeout.

      Verify: unit tests against `/bin/sh` cover all of the following:
      - an exit status;
      - `kill -s ILL $$` giving `Signalled(SIGILL)`;
      - `sleep 5` with a 100 ms limit giving `TimedOut`, with the child reaped;
      - `Discard` leaving `stdout_head` empty;
      - a missing program giving `NotFound`;
      - a non-executable file giving `NotExecutable`.

      Run with `cargo test doctor::tests`
- [x] 3.2 Implement the `proton-drive` check: resolve the program (a `PATH` search, no spawn,
      for a bare name) and run `version` with a 10 s limit (D3). Verify: one unit test per row of
      D3's table, all against a fake `Environment`, including:
      - "non-zero exit is `failed` and not `missing`" (the reversed §72 case);
      - SIGILL naming `linux/x64-baseline`;
      - the `ok` statement carrying the version and the absolute path
- [x] 3.3 Implement the `signed-in` check in D4's order:
      1. the credential store;
      2. `secret-tool` on `PATH`;
      3. the session bus;
      4. the lookup, with stdout discarded and a 60 s limit.

      Verify: unit tests for each step, including:
      - the lookup is not run when the bus address is unset or empty;
      - the fake records `Discard` for the lookup;
      - a stderr line gives `unknown`, and empty stderr gives `missing`;
      - a timeout's wording mentions the unlock prompt
- [x] 3.4 Implement `secret-tool`, `curl` and `session-bus` (presence and environment only), and
      `service` (`systemctl --user show proton-syncd.service -p LoadState --value`, D5). Verify:
      unit tests for every outcome, including `masked` → failed and `systemctl` absent → unknown,
      and the recommended wording that says sync still works but walks the whole folder
- [x] 3.5 Implement the `config` check and the choice of CLI path (D6):
      - `--proton-cli` first;
      - then a validated file's `proton_cli`;
      - then `proton-drive`.

      Validation goes through `config::validate_file_config_text`. Verify: unit tests for each
      case:
      - default file absent → ok, "No config file yet";
      - named file absent → failed;
      - invalid file → failed, quoting the validator, while the CLI check runs `proton-drive`
        and says so;
      - an absolute `proton_cli` in the file is the path run;
      - `--proton-cli` beats the file
- [x] 3.6 Implement the verdict (D7), the human renderer (D8) and `Serialize` for `--json`.
      Verify: unit tests cover:
      - the verdict table, including a recommended problem leaving `ok`;
      - every URL in the human output followed by whitespace or a newline, with no punctuation
        attached;
      - the JSON round-tripping through `serde_json::from_str`;
      - no CLI-related result carrying a link outside `doctor::links`
- [x] 3.7 Only if 1.1 found that `version` touches shared state: before running `version`, probe
      the user-global lock with `daemon::probe_daemon_lock` (shared, non-blocking). When a daemon
      holds it, report `proton-drive` as `unknown` ("the daemon is running; its start-up log
      checks the CLI") without spawning. Verify: a unit test with the lock held asserts that
      nothing was run

## 4. `proton-sync doctor`

- [x] 4.1 Add `Commands::Doctor { proton_cli: Option<PathBuf> }` to `src/bin/proton-sync.rs`, with
      the form `proton-sync [--config FILE] [--json] doctor [--proton-cli PATH]` (`--config`
      stays top-level, D8). Dispatch it at the top of `main`, before
      `resolve_control_socket_path` and the `--pair`/`--all-pairs` branches. It:
      - refuses `--socket-path`, `--pair` and `--all-pairs` with exit 2 and D8's sentence;
      - prints the human report or `--json`;
      - exits 0, 1 or 3.

      `build_request`, `run_for_pair`'s exit-code match and its print match are exhaustive, so
      each needs a `Doctor` arm, an `unreachable!` naming the early dispatch. Verify: through
      4.3's tests
- [x] 4.2 Add `DoctorTools` to `tests/common/mod.rs` (D14). It:
      - writes answering stubs for `proton-drive`, `secret-tool` and `systemctl`, each with a
        chosen status, stderr or signal;
      - sets `PATH` to the answering stubs, then the refusing stubs, with no system directory;
      - sets a marker variable.

      Stubs use only builtins and absolute paths, because `PATH` has no system directory: the
      timeout stub runs `exec /bin/sleep 30`, and the signal stub runs `kill -s ILL $$`.

      Extend `refuse_the_real_cli` to refuse a `doctor` command that has no marker, and pin both
      in `tests/test_isolation.rs`. Verify: `cargo test --test test_isolation` passes, and a
      scratch test running `sync_cli(dir).arg("doctor")` without the helper is refused
- [x] 4.3 Add `tests/doctor_cli.rs`, every run through `sync_cli` + `DoctorTools` + `run_bounded`.
      The cases:
      - **(a) the sibling's smoke scenario.** No `proton-drive`, no config, no bus. Stdout names
        `proton-drive` as missing and contains `https://proton.me/drive/download`, and the exit is
        1. With `XDG_RUNTIME_DIR` unset and `TMPDIR` in the sandbox, no socket and no
        `proton-drive-sync-*` directory appear.
      - **(b) everything ok.** Fake bus address and answering stubs: exit 0.
      - **(c) no bus, the rest ok:** exit 3.
      - **(d)** `--pair x doctor` exits 2.
      - **(e)** `--json` parses.
      - **(f)** a stub `secret-tool` that prints a marker on stdout: the marker appears nowhere in
        stdout or stderr.
      - **(g)** `--proton-cli` beats a config file's `proton_cli`.
      - **(h)** the exact argument lists `setup.sh` builds in each mode (`--config F doctor`,
        `--config F doctor --proton-cli P`, and `doctor` alone) parse, and exit 0 when everything
        is ok.
      - **(i)** the timeout stub gives `unknown` with "did not answer", not `missing`.

      Verify: `cargo test --test doctor_cli`
- [x] 4.4 Document `doctor` in `website/src/content/docs/cli/reference.md`:
      - a Commands-table row;
      - a "Checking this machine: doctor" section saying it does not use the daemon socket and
        listing its flags;
      - the refused flags;
      - the "Exit codes" section, with 0, 1 and 3 for `doctor` and 2 for usage errors.

      Verify: `(cd website && npm run build)` succeeds

## 5. Daemon start-up check

- [x] 5.1 Hold an `Arc<dyn doctor::Environment>` in `Daemon`. It defaults to an inert
      environment that finds and runs nothing. Only `Daemon::new` installs `SystemEnvironment`,
      and a `#[cfg(test)]` setter installs a fake. `from_pairs` keeps its signature (24 uses).
      In `Daemon::run`, after the control-socket task is spawned and before the first step, run
      `proton-drive` (always) and `curl` (when `any_pair_streams`) on a blocking thread. Log:
      - each problem once at WARN, with `check`, `fix` and `docs` fields;
      - an ok CLI once at INFO, with its version and path.

      Verify: new `daemon::tests` cover:
      - a fake missing CLI gives one WARN, and the daemon keeps answering on its socket;
      - the WARN is not repeated by later passes;
      - `curl` is not checked when no pair streams;
      - the fake records every program asked for, and no lib test reaches `SystemEnvironment`
- [x] 5.2 Change `AUTH_DECLINE_REASON` to `proton-drive auth login` plus the `signed-in` docs
      link, and add the same link to the no-session reason (D12). Change `proton-sync status`'s
      `sign-in` row to `proton-drive auth login`. Make `tests/ipc_cli.rs:944` assert the new
      text. Verify: `cargo test --test ipc_cli` passes, and the
      degraded-session-once tests still pass
- [x] 5.3 Audit every binary test that starts `proton-syncd` with a fake `proton_cli`. Make each
      fake answer `version` with exit 0, and update any test that counts or logs CLI calls so
      that it ignores the start-up `version` call. Verify: `cargo test --workspace --all-targets`
      is green, and the PR lists the fake CLI scripts touched
- [x] 5.4 Document the start-up check in `website/src/content/docs/daemon/logging.md`: what it
      checks, that it logs once and keeps running, and where the fix is. Verify: the site builds

## 6. The sign-in command, everywhere, and its guard

- [x] 6.1 Replace `proton-drive login` with `proton-drive auth login` at the remaining live
      sites:
      - `setup.sh:501`;
      - the comments in `gui/src/js/screens/main.js:523` and `gui/src/js/ui/copy.js:1767`;
      - `website/src/content/docs/cli/reference.md:325-326` (wrapped across lines);
      - `website/src/content/docs/desktop/overview.md:70`;
      - `website/src/content/docs/desktop/screens.md:49`.

      The daemon, the control CLI and `tests/ipc_cli.rs` are task 5.2. The dated records under
      `docs/` stay as written. Verify: the line-break-aware scan (design Context) finds no live
      site
- [x] 6.2 Add `tests/sign_in_command.rs` (D13): the roots, the exemptions, the normalisation and
      the needle built from parts. Verify: it passes. Putting the wrapped form back into a docs
      page fails it, naming the file and line. Revert that edit

## 7. Desktop app

- [x] 7.1 Add a `doctor` module to `gui-core` that maps `RuntimePaths`' `proton_cli` and config
      path to `Inputs`, takes an `&dyn Environment`, and re-exports the types. In
      `gui/src-tauri/src/commands.rs`:
      - `check_cli` returns `{ ready, report, distro }`, run on `spawn_blocking` with
        `SystemEnvironment`;
      - delete `probe_cli`, `CLI_PROBE_TIMEOUT` and the tests
        `a_missing_binary_is_the_only_thing_that_reads_as_not_installed` and
        `a_present_binary_that_exits_non_zero_is_still_present` (their cases live in 3.2);
      - add `open_help_link { check, link }`, which looks the URL up in `doctor::links`, refuses
        unknown ids without spawning, and opens through `open_target(OPENER, …)`.

      Register it in `lib.rs` and classify it in `commands/pair_tests.rs`. Verify:
      `cargo test --workspace` covers:
      - a gui-core test that `ready` is false only for a `failed` verdict;
      - an `open_help_link` test that an unknown id spawns nothing;
      - the `pair_tests` classification passing
- [x] 7.2 Frontend (`gui/src/js`):
      - `api.js`: `checkCli`, the fixture lookup, the default payload `{ ready: true, … }`, and
        `openHelpLink`.
      - `app.js`: `cliPresence?.ready === false` picks the dialog.
      - `copy.js`: the titles for the first required problem (D10), "How to fix", "Sign-in help"
        and the hero sub-line. Delete the comment saying no URL exists.
      - `renderCliMissing`: list every non-ok result, required first, each with its fix and
        links, plus "Installation help" and "Check again", keeping the `Detected` clause.
      - `main.js`: the auth-expired hero's sentence and "Sign-in help".

      Verify: `(cd gui && npm run check)` is green, with new cases in `test/onboarding.test.js`
      (missing, crashed, signed out, could-not-check-only not blocking, links passing ids only)
      and `test/main-screen.test.js` (the auth-expired sentence and link)
- [x] 7.3 Fixtures and fidelity:
      - the `9a CLI missing` fixture gets the new reply shape;
      - `fids.js` gets the new nodes;
      - `tools/fidelity/check-n1-identity.mjs` and `check-pair-routing.mjs` get the new mocked
        `check_cli` reply;
      - `known-deviations.mjs` gets the new `9a` rows;
      - `copy-gate.mjs` gets `DRAWN` rows for strings the frame draws and the build now renders.

      Add a `docs/design-v2/DEVIATIONS.md` section that reverses §72 ("presence, not health") and
      §102 ("no URL") and records the checklist that replaces the command box. Verify:
      `(cd gui && npm run check && npm run fidelity)` is green, with the `fidelity:copy` count
      before and after in the PR. Render `npm run screenshots -- --out <scratch> --frame "9a CLI
      missing"` for review only. The published screenshots are not regenerated, because none
      shows these states
- [x] 7.4 Docs:
      - `website/src/content/docs/desktop/overview.md`: the Auth expired row gets "Sign-in help"
        and the paragraph at line 70 gets the command.
      - `website/src/content/docs/desktop/screens.md`: the line 49 paragraph, and the first-run
        paragraph at line 321, describing the checklist, "Installation help" and that a crashing
        CLI is "doesn't run", not "isn't installed".

      Verify: the site builds

## 8. setup.sh

- [x] 8.1 Add `run_dependency_check MODE` (D11). Call it:
      - from `run_terminal_setup`, after `write_config` and before `install_unit`;
      - from `run_gui_setup`, after `install_engine_binaries`.

      Add `--ignore-checks` to `parse_args` and `usage`. Delete the `proton-drive` warning from
      `check_common_prereqs`. Touch nothing that `upgrade.sh` or `uninstall.sh` call. Verify via
      8.2, and `bash -n setup.sh`
- [x] 8.2 Extend `tests/scripts.rs`. Source `setup.sh` in the existing scratch sandbox, with a
      stub `proton-sync` that records its argv and exits 0, 1, 2 or 3, and a `systemctl` stub
      that logs its calls, and assert:
      - the recorded argv is `--config <file> doctor` in terminal mode (plus `--proton-cli P`
        after `doctor` when given) and `doctor` with no `--config` in `--gui` mode. These are the
        lists 4.3 (h) proves the real binary accepts;
      - terminal mode stops on 1 and on 2 without reaching `install_unit`;
      - it warns and continues on 3;
      - `--ignore-checks` continues on 1;
      - `--gui` mode continues on 1;
      - sourcing `setup.sh` from `upgrade.sh`'s path still runs nothing.

      Verify: `cargo test --test scripts`
- [x] 8.3 Docs, saying to run `proton-sync doctor` after installing:
      - `README.md` (Quick setup and Manual setup);
      - `website/src/content/docs/start/installation.md` (one line after the prerequisite
        section, and in "Build from source");
      - `website/src/content/docs/start/quick-start.md` (before the first preview).

      In the README's Quick setup block, replace the `proton-drive filesystem list` "confirm the
      CLI works" line with `proton-drive version`, and add `proton-sync doctor` after the install
      command. Verify: the site builds, and the README's command block runs as written in a
      scratch checkout up to the build

## 9. Integration checks

- [x] 9.1 Run the full gates: `cargo fmt --all -- --check`,
      `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
      `cargo test --workspace --all-targets --all-features`, `(cd gui && npm ci && npm run check)`,
      `(cd gui && npm run fidelity)` and `(cd website && npm ci && npm run build)`. Verify: all
      green, with the output in the PR
- [x] 9.2 In the built site, confirm the ids:
      `grep -o 'id="[^"]*"' website/dist/reference/troubleshooting/index.html` contains every
      anchor in `doctor::links` (D9's one-time check). Verify: the list in the PR matches
      design D9's table
- [x] 9.3 Run the sibling's smoke expectation by hand in a throwaway `fedora:latest` container
      with the built `proton-sync` bind-mounted, and no `proton-drive`, config, bus or daemon.
      `proton-sync doctor` must:
      - name `proton-drive`;
      - print `https://proton.me/drive/download`;
      - exit 1;
      - leave no `proton-syncd` process and no socket.

      Verify: the transcript is in the PR, and the sibling change's task 3.10 can use the same
      commands

## 10. Review fixes

- [x] 10.1 `doctor` creates nothing on any path: the lock probe uses
      `paths::existing_global_lock_path` (D8). Verify: `with_no_home_doctor_creates_nothing_in_the_shared_temp_directory`
- [x] 10.2 No home directory means no default config and no `proton_cli` from the temp directory
      (D6), in `doctor` and in the desktop app. Verify:
      `with_no_home_a_config_planted_in_the_temp_directory_is_neither_read_nor_run`
- [x] 10.3 Record that `list` can answer `busy` for up to 10 s after start-up (Risks)
- [x] 10.4 Shutdown cancels the start-up check's child and joins its thread (D12). Verify:
      `a_shutdown_kills_the_start_up_check_and_run_waits_for_it`
- [x] 10.5 A relative `proton_cli` is `failed` (D3). Verify:
      `a_relative_path_is_failed_whichever_directory_it_is_read_from`
- [x] 10.6 Capitalised sentences, and backtick spans drawn as code in the app (D3). Verify:
      `every_statement_and_fix_starts_with_a_capital_unless_it_starts_with_a_name_or_path`, `codeSpansOf` tests
- [x] 10.7 Control characters in a child's output are replaced at capture (D3). Verify:
      `control_characters_in_a_childs_output_never_leave_the_capture`
- [x] 10.8 A directory as `proton_cli` has its own statement and fix (D3). Verify:
      `a_directory_is_not_called_unexecutable`
- [x] 10.9 `setup.sh --gui --proton-cli P` passes `P` to `doctor` (D11). Verify:
      `gui_setup_passes_the_cli_path_it_was_given_and_still_no_config`
- [x] 10.10 The `check_cli` comment states what a failed join does (the page logs it and carries
      on; nothing depends on the check)

## Workflow follow-up

- Open the PR. Merge after review and green CI, per the repository's workflow.
- Tell the `add-linux-release-pipeline` work that this change is merged:
  - its task 3.10 can be done;
  - its tasks 8.2 and 8.3 must keep the `proton-sync doctor` line this change adds to
    `installation.md` and `README.md`.
- Archive the change once merged, and sync the `dependency-check` spec into `openspec/specs/`.
