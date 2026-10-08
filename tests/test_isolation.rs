//! Pins the sandbox every process-level test runs in (`tests/common/mod.rs`), and refuses a test
//! file that starts a binary of this crate without it.
//!
//! The failure being guarded against is not a failing test: it is a test that *works* and, on the
//! way, starts a real `proton-syncd` that binds the developer's live control socket. Nothing in a
//! passing run shows it, which is why the rule is enforced here by reading the sources.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::tempdir;

/// What a spawned process sees for each process-global default, one per line.
const PRINT_ENVIRONMENT: &str = r#"printf '%s\n%s\n%s\n%s\n%s\n%s\n' "$HOME" "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME""#;

#[test]
fn a_sandboxed_command_resolves_every_process_global_default_into_its_directory() {
    // Run on a machine that has them set (a developer's session does), the child must still see
    // the test's directory: `Command::env` is applied over whatever the parent inherited.
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed("sh", directory.path());
    command.arg("-c").arg(PRINT_ENVIRONMENT);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    let seen = String::from_utf8(output.stdout).expect("utf-8");
    let seen: Vec<&str> = seen.lines().collect();
    let expected = directory.path().to_str().expect("utf-8 path");
    assert_eq!(
        seen, [expected; 6],
        "HOME and the five XDG directories must all name the test's own directory"
    );
}

#[test]
fn a_run_that_outlives_its_bound_is_killed_and_reported_not_waited_on() {
    let directory = tempdir().expect("tempdir");
    let started = Instant::now();
    let outcome = std::panic::catch_unwind(|| {
        let mut command = common::sandboxed("sleep", directory.path());
        command.arg("30");
        common::run_bounded(&mut command, Duration::from_millis(300));
    });
    let message = match outcome {
        Err(payload) => payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
            .unwrap_or_default(),
        Ok(()) => panic!("a 30 s sleep returned inside a 300 ms bound"),
    };
    assert!(
        message.contains("was still running after"),
        "the overrun says what it means: {message}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "and it did not wait for the child: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_run_that_prints_more_than_a_pipe_holds_still_finishes() {
    // 256 KiB is four times a Linux pipe buffer. With a pipe nobody drains while the parent polls
    // `try_wait`, the child blocks in `write` and the bound fires on a healthy process.
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed("sh", directory.path());
    command.arg("-c").arg(
        "head -c 262144 /dev/zero | tr '\\0' 'x'; head -c 262144 /dev/zero | tr '\\0' 'y' >&2",
    );
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(output.stdout.len(), 262_144);
    assert_eq!(output.stderr.len(), 262_144);
}

#[test]
fn a_daemon_command_that_names_no_fake_cli_is_refused_before_it_runs() {
    use std::process::Command;
    let directory = tempdir().expect("tempdir");
    // A program that does not exist, named like the daemon: the refusal must come before the spawn,
    // because a spawn that succeeded would be a daemon over the real `proton-drive`.
    let program = directory.path().join("proton-syncd");
    let refused = |command: &mut Command| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            common::refuse_the_real_cli(command)
        }))
        .is_err()
    };
    assert!(
        refused(Command::new(&program).arg("--dry-run")),
        "no --proton-cli and no config: the real CLI"
    );
    assert!(
        !refused(Command::new(&program).args(["--proton-cli", "/fake", "--no-events-driven"])),
        "a flag names one"
    );
    let named = directory.path().join("named.toml");
    fs::write(
        &named,
        "# a comment\nproton_cli = \"/fake\"\nevents_driven = false\n",
    )
    .expect("config");
    assert!(
        !refused(Command::new(&program).arg("--config").arg(&named)),
        "a config that sets proton_cli names one"
    );
    let silent = directory.path().join("silent.toml");
    fs::write(&silent, "remote_root = \"/Drive/x\"\n").expect("config");
    assert!(
        refused(Command::new(&program).arg("--config").arg(&silent)),
        "a config that does not is the real CLI again"
    );
    assert!(
        !refused(Command::new(directory.path().join("proton-sync")).arg("status")),
        "the control CLI never runs proton-drive"
    );
}

/// The panic message of `run`, or `None` when it returned.
fn panic_message(run: impl FnOnce()) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .err()
        .map(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|text| (*text).to_owned())
                })
                .unwrap_or_default()
        })
}

#[test]
fn a_daemon_that_leaves_events_driven_on_is_refused_before_it_runs() {
    use std::process::Command;
    // PR #434 second review, H2. A daemon with `events_driven` on (the default) runs `secret-tool`
    // to read the signed-in Proton session out of the desktop keyring and then calls the events
    // API with it. Naming a fake `proton-drive` does not stop that, so it is its own refusal.
    let directory = tempdir().expect("tempdir");
    let program = directory.path().join("proton-syncd");
    let refused = |command: &mut Command| panic_message(|| common::refuse_the_real_cli(command));

    let message = refused(Command::new(&program).args(["--proton-cli", "/fake"]))
        .expect("a fake CLI with events left on is refused");
    assert!(message.contains("leaves events_driven on"), "{message}");

    let file = |name: &str, text: &str| {
        let path = directory.path().join(name);
        fs::write(&path, text).expect("config");
        path
    };
    let silent = file("silent.toml", "proton_cli = \"/fake\"\n");
    assert!(
        refused(Command::new(&program).arg("--config").arg(&silent)).is_some(),
        "a config that does not mention events_driven leaves the default"
    );
    let on = file("on.toml", "proton_cli = \"/fake\"\nevents_driven = true\n");
    assert!(
        refused(Command::new(&program).arg("--config").arg(&on)).is_some(),
        "and one that says true"
    );
    let off = file(
        "off.toml",
        "proton_cli = \"/fake\"\nevents_driven = false\n",
    );
    assert!(
        refused(Command::new(&program).arg("--config").arg(&off)).is_none(),
        "events_driven = false in a one-pair file"
    );
    let tables_off = file(
        "tables-off.toml",
        "proton_cli = \"/fake\"\n[[pair]]\nname = \"a\"\nevents_driven = false\n\
         [[pair]]\nname = \"b\"\nevents_driven = false\n",
    );
    assert!(
        refused(Command::new(&program).arg("--config").arg(&tables_off)).is_none(),
        "every [[pair]] table says false"
    );
    let one_table_on = file(
        "one-table-on.toml",
        "proton_cli = \"/fake\"\n[[pair]]\nname = \"a\"\nevents_driven = false\n\
         [[pair]]\nname = \"b\"\n",
    );
    assert!(
        refused(Command::new(&program).arg("--config").arg(&one_table_on)).is_some(),
        "one silent table is a pair that streams: the whole run reads the keyring"
    );
    let unparsable = file(
        "unparsable.toml",
        "proton_cli = \"/fake\"\nevents_driven = false\nthis is not toml\n",
    );
    assert!(
        refused(Command::new(&program).arg("--config").arg(&unparsable)).is_some(),
        "a file that does not parse is not evidence of anything"
    );
    assert!(
        refused(Command::new(&program).args(["--proton-cli", "/fake", "--no-events-driven"]))
            .is_none(),
        "the flag"
    );
    let mut opted_in = Command::new(&program);
    opted_in.args(["--proton-cli", "/fake"]);
    common::opt_in_to_events_driven(&mut opted_in);
    assert!(
        refused(&mut opted_in).is_none(),
        "a test about the session says so by name"
    );
    assert!(
        refused(&mut Command::new(directory.path().join("proton-sync"))).is_none(),
        "the control CLI never reads the keyring"
    );
    // PR #434 third review, L1. `--events-driven` overrides a file's `events_driven = false`
    // (the flag beats the file), so a config that looks safe is not once the flag is beside it.
    let message = refused(
        Command::new(&program)
            .arg("--config")
            .arg(&off)
            .arg("--events-driven"),
    )
    .expect("the flag turns events back on over a file that turned them off");
    assert!(message.contains("leaves events_driven on"), "{message}");
    assert!(
        refused(Command::new(&program).args(["--proton-cli", "/fake", "--events-driven"]))
            .is_some(),
        "and over nothing at all"
    );
    let mut flag_opted_in = Command::new(&program);
    flag_opted_in.args(["--proton-cli", "/fake", "--events-driven"]);
    common::opt_in_to_events_driven(&mut flag_opted_in);
    assert!(
        refused(&mut flag_opted_in).is_none(),
        "a test about the session may pass the flag, having said so"
    );
}

#[test]
fn a_daemon_that_reached_a_stub_tool_fails_its_test_when_the_child_is_dropped() {
    // PR #434 third review, L1. `run_bounded` fails a run that reached a stub; a daemon started
    // with `spawn_logging` is long-running and never goes through it, so a test that waited on a
    // socket and passed would have degraded silently. Dropping the child is the end of the test.
    let directory = tempdir().expect("tempdir");
    let log = directory.path().join("child.stderr");
    let message = panic_message(|| {
        let mut command = common::sandboxed("sh", directory.path());
        command
            .arg("-c")
            .arg("secret-tool lookup service \"$(echo z)\"; exit 0");
        let mut child = common::spawn_logging(&mut command, &log);
        child.wait().expect("the child ends");
    })
    .expect("a child that reached a stub tool fails the test that started it");
    assert!(
        message.contains("reached the real keyring or network")
            && message.contains("secret-tool lookup service z\n"),
        "{message}"
    );

    let clean = directory.path().join("clean");
    fs::create_dir(&clean).expect("a second sandbox");
    let message = panic_message(|| {
        let mut command = common::sandboxed("sh", &clean);
        command.arg("-c").arg("exit 0");
        let mut child = common::spawn_logging(&mut command, &clean.join("child.stderr"));
        child.wait().expect("the child ends");
    });
    assert!(
        message.is_none(),
        "a child that reached nothing passes: {message:?}"
    );
}

#[test]
fn the_tool_runner_refuses_the_binaries_of_this_crate() {
    use std::process::Command;
    let directory = tempdir().expect("tempdir");
    for name in ["proton-syncd", "proton-sync"] {
        let message = panic_message(|| {
            let mut command = Command::new(directory.path().join(name));
            common::run_other_tool(&mut command);
        })
        .expect("a binary of this crate is not an unsandboxed tool");
        assert!(message.contains("binary of this crate"), "{message}");
    }
    let output = common::run_other_tool(Command::new("sh").args(["-c", "printf ok"]));
    assert_eq!(output.stdout, b"ok", "any other program runs");
}

#[test]
fn a_sandboxed_child_cannot_see_the_desktop_session_bus() {
    // The parent's environment is whatever it is (a developer's session has the bus, CI may not), so
    // the address is put on the command first and the sandbox must take it off again.
    let directory = tempdir().expect("tempdir");
    let mut command = std::process::Command::new("sh");
    command
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/0/bus")
        .env(
            "DBUS_SYSTEM_BUS_ADDRESS",
            "unix:path=/run/dbus/system_bus_socket",
        );
    common::sandbox(&mut command, directory.path());
    command.arg("-c").arg(
        "printf '%s\n%s\n' \"${DBUS_SESSION_BUS_ADDRESS-unset}\" \"${DBUS_SYSTEM_BUS_ADDRESS-unset}\"",
    );
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        "unset\nunset\n",
        "no bus address, so nothing to ask for the keyring over"
    );
}

#[test]
fn a_sandboxed_child_that_runs_the_keyring_or_network_tools_reaches_stubs_not_the_real_ones() {
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed("sh", directory.path());
    command.arg("-c").arg(
        "command -v secret-tool; command -v curl; \
         secret-tool lookup service ch.proton.drive/drive-sdk-cli account auth-session; echo \"rc=$?\"; \
         curl --silent https://example.invalid/events; echo \"rc=$?\"",
    );
    let (output, hits) = common::run_bounded_reporting_stub_hits(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{stdout}");
    for tool in &lines[..2] {
        assert!(
            tool.contains("sandbox-bin"),
            "PATH finds the stub first, not the real tool: {tool}"
        );
        assert!(
            !tool.starts_with("/usr/") && !tool.starts_with("/bin/"),
            "{tool}"
        );
    }
    assert_eq!(&lines[2..], ["rc=97", "rc=97"], "each stub fails");
    assert_eq!(
        hits.lines().collect::<Vec<_>>(),
        [
            "secret-tool lookup service ch.proton.drive/drive-sdk-cli account auth-session",
            "curl --silent https://example.invalid/events"
        ],
        "and records what was asked, which is what makes a reach visible"
    );
}

#[test]
fn a_run_that_reaches_a_stub_tool_fails_the_test_instead_of_passing_degraded() {
    let directory = tempdir().expect("tempdir");
    let message = panic_message(|| {
        let mut command = common::sandboxed("sh", directory.path());
        command
            .arg("-c")
            .arg("secret-tool lookup service \"$(echo x)\"; exit 0");
        common::run_bounded(&mut command, common::RUN_BOUND);
    })
    .expect("a clean exit after a stub was reached still fails");
    // The command line is in the message too, so what is asserted is the stub's own record of it.
    assert!(
        message.contains("(a stub tool in the sandbox answered):\nsecret-tool lookup service x\n"),
        "{message}"
    );
}

#[test]
fn a_run_that_overruns_its_bound_says_which_stub_tools_it_reached() {
    let directory = tempdir().expect("tempdir");
    let message = panic_message(|| {
        let mut command = common::sandboxed("sh", directory.path());
        command
            .arg("-c")
            .arg("secret-tool lookup service \"$(echo y)\"; sleep 30");
        common::run_bounded(&mut command, Duration::from_millis(1500));
    })
    .expect("the overrun fails");
    assert!(
        message.contains("was still running after")
            && message.contains("stub tools reached: secret-tool lookup service y\n"),
        "{message}"
    );
}

#[test]
fn a_daemon_start_applies_the_fake_cli_refusal_before_it_spawns() {
    // A program that does not exist, named like the daemon. If the refusal were skipped the failure
    // would be the spawn's ("No such file"); the refusal comes first and says what it is.
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed(directory.path().join("proton-syncd"), directory.path());
    command.arg("--dry-run");
    let log = directory.path().join("daemon.stderr");
    let message = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        common::spawn_logging(&mut command, &log)
    }))
    .err()
    .map(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
            .unwrap_or_default()
    })
    .expect("a daemon with no fake CLI is refused");
    assert!(message.contains("names no fake CLI"), "{message}");
    assert!(!log.exists(), "and nothing was created for it");
}

#[test]
fn a_bounded_run_applies_the_fake_cli_refusal_before_it_spawns() {
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed(directory.path().join("proton-syncd"), directory.path());
    command.arg("--dry-run");
    let message = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        common::run_bounded(&mut command, common::RUN_BOUND)
    }))
    .err()
    .map(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
            .unwrap_or_default()
    })
    .expect("a preview with no fake CLI is refused");
    assert!(message.contains("names no fake CLI"), "{message}");
}

/// The rule, enforced. **No test file other than `tests/common/mod.rs` may**:
///
/// * contain the name of one of this crate's binaries as `CARGO_BIN_` + `EXE_…` (what `env!`,
///   `option_env!` and `std::env::var` are handed), however it is bound or aliased;
/// * contain a string literal that **ends in** `proton-syncd` or `proton-sync` (`"proton-syncd"`,
///   `"/home/me/.cargo/bin/proton-syncd"`): a bare name resolves through `PATH` to the user's
///   installed binary, which on a developer's machine is the **live daemon**. Longer names
///   (`.proton-sync.toml`, `proton-sync-gui`) are not the binaries and are not matched;
/// * run a `Command` at all: `.spawn(`, `.output(`, `.status(` or `.exec(`, and the same called as
///   `Command::spawn(&mut c)`; the name and the parenthesis may be on different lines;
/// * rename the type (`Command as Other`), which would hide it from the form above;
/// * use `common::DAEMON_FILE_NAME` / `common::CONTROL_CLI_FILE_NAME` (the binaries' file names,
///   for a test that compares them as data) in a file that also mentions `Command`.
///
/// A test starts `proton-syncd` and `proton-sync` through `common::syncd` / `common::sync_cli`
/// (sandboxed), runs one to completion with `common::run_bounded`, starts a daemon with
/// `common::spawn_logging`, and runs any *other* program (`kill`, `pgrep`, the live `proton-drive`)
/// with `common::run_other_tool`.
///
/// **What this catches and what it does not, exactly.** It is a text scan of every `tests/**/*.rs`
/// except `tests/common/mod.rs` and **this file** (`tests/test_isolation.rs`, whose probe trees
/// contain every form on purpose), comment lines (those starting with `//`) skipped. It does not
/// understand block comments, so a line inside `/* ... */` that names a form is reported: it fails
/// closed. It does **not** follow data flow, so it misses a binary reached without its name (a
/// path built from `CARGO_MANIFEST_DIR` and `target/`, a name split across `concat!` pieces, a
/// copy of the binary run from elsewhere), a launch through a function that wraps `Command` (it
/// would have to contain one of the forms itself), and `cargo run --bin`. `Command::new` alone is
/// not a launch and is not reported unless it names a binary of this crate.
#[test]
fn no_test_file_starts_a_binary_of_this_crate_without_the_sandbox() {
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let offenders = unsandboxed_spawns(&tests_dir);
    assert!(
        offenders.is_empty(),
        "start proton-syncd / proton-sync through common::syncd / common::sync_cli (HOME and the XDG \
         dirs in the test's own directory, no desktop session bus, stub keyring tools), run to \
         completion with common::run_bounded or start a daemon with common::spawn_logging, and run \
         any other program with common::run_other_tool; a binary name, `Command` launch \
         (spawn/output/status/exec) or `Command as` rename at: {offenders:?}"
    );
}

/// `file:line` for every offending line under `tests_dir` (recursively), outside the two files that
/// are allowed to contain the forms: `common/mod.rs`, and this one.
fn unsandboxed_spawns(tests_dir: &Path) -> Vec<String> {
    let mut offenders = Vec::new();
    let mut directories = vec![tests_dir.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).expect("tests directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let relative = path
                .strip_prefix(tests_dir)
                .expect("under tests")
                .to_string_lossy()
                .into_owned();
            if relative == "common/mod.rs" || relative == "test_isolation.rs" {
                continue;
            }
            let text = fs::read_to_string(&path).expect("test source");
            for line in offending_lines(&text) {
                offenders.push(format!("{relative}:{line}"));
            }
        }
    }
    offenders.sort();
    offenders
}

/// The 1-based lines of `text` that hold one of the forms listed at
/// [`no_test_file_starts_a_binary_of_this_crate_without_the_sandbox`]. Needles are assembled from
/// pieces so that the file this lives in does not match them (it is exempt anyway).
fn offending_lines(text: &str) -> Vec<usize> {
    // Comment lines are blanked, not removed, so line numbers survive.
    let code = text
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("//") {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let line_of = |index: usize| code[..index].matches('\n').count() + 1;
    let mut lines = std::collections::BTreeSet::new();

    // The crate's binary names, as the environment variable cargo provides.
    let env_name = format!("{}{}", "CARGO_BIN_", "EXE_");
    for (index, _) in code.match_indices(&env_name) {
        lines.insert(line_of(index));
    }

    // A string literal ending in a binary's name: the character before it is the opening quote or
    // a path separator, and the one after it (past an optional `d`) is the closing quote.
    let binary = format!("{}{}", "proton-", "sync");
    for (index, _) in code.match_indices(&binary) {
        let rest = &code[index + binary.len()..];
        let rest = rest.strip_prefix('d').unwrap_or(rest);
        if rest.starts_with('"') && (code[..index].ends_with('"') || code[..index].ends_with('/')) {
            lines.insert(line_of(index));
        }
    }

    // A launch: `.spawn(`, or `Command::spawn(`, with whitespace (newlines too) allowed between
    // the pieces. The name must be the whole identifier (`respawn(` has no `.` before it,
    // `spawn_with_config(` no `(` after it), and `thread::spawn(` is not a `Command`.
    for method in ["spawn", "output", "status", "exec"] {
        for (index, _) in code.match_indices(method) {
            let before = code[..index].trim_end();
            let called_as_method = before.ends_with('.');
            let called_as_function = before
                .strip_suffix("::")
                .is_some_and(|path| path.trim_end().ends_with("Command"));
            // `spawn_with_config(` and `output_to(` continue the identifier, so no `(` follows.
            let after = code[index + method.len()..].trim_start();
            if (called_as_method || called_as_function) && after.starts_with('(') {
                lines.insert(line_of(index));
            }
        }
    }

    // The two file-name constants stand for the bare names, so a file that mentions `Command` may
    // not use them: `Command::new(common::DAEMON_FILE_NAME)` is the bare name in a trench coat.
    if code.contains("Command") {
        for constant in ["DAEMON_FILE_NAME", "CONTROL_CLI_FILE_NAME"] {
            for (index, _) in code.match_indices(constant) {
                lines.insert(line_of(index));
            }
        }
    }

    // The type renamed on import hides every `Command::` form above.
    let rename = format!("{} as ", "Command");
    for (index, _) in code.match_indices(&rename) {
        lines.insert(line_of(index));
    }
    lines.into_iter().collect()
}

#[test]
fn the_spawn_guard_catches_the_name_bound_to_a_variable_first() {
    // PR #434 second review, H3. The first version of the guard matched the text
    // `Command::new(env!("...` and was bypassed by binding the name to a variable. A probe tree
    // with the bypass forms proves the scan sees each of them (and not a comment).
    let tests = tempdir().expect("tempdir");
    let name = format!("{}{}", "CARGO_BIN_", "EXE_proton-syncd");
    fs::create_dir(tests.path().join("common")).expect("common");
    fs::write(
        tests.path().join("common").join("mod.rs"),
        format!("pub fn path() -> &'static str {{ env!(\"{name}\") }}\n"),
    )
    .expect("common/mod.rs may name it");
    fs::write(
        tests.path().join("bypass.rs"),
        format!(
            "// {name} in a comment is not a use\n\
             let exe = env!(\"{name}\");\n\
             std::process::Command::new(exe).arg(\"--help\").output();\n\
             let also = option_env!(\"{name}\");\n"
        ),
    )
    .expect("probe");
    fs::create_dir(tests.path().join("nested")).expect("nested");
    fs::write(
        tests.path().join("nested").join("deep.rs"),
        format!("let exe = std::env::var(\"{name}\");\n"),
    )
    .expect("nested probe");
    fs::write(
        tests.path().join("spawns.rs"),
        "let child = command.spawn().unwrap();\n",
    )
    .expect("spawn probe");
    assert_eq!(
        unsandboxed_spawns(tests.path()),
        [
            "bypass.rs:2",
            "bypass.rs:3",
            "bypass.rs:4",
            "nested/deep.rs:1",
            "spawns.rs:1"
        ]
    );
}

/// One probe file per form the third review (L2) found the scan missing, each alone in its tree so
/// that what is reported can only be that form. The right-hand side is the line the form is on.
#[test]
fn the_spawn_guard_catches_every_way_to_reach_a_binary_or_run_a_command() {
    let bare_syncd = format!("let c = Command::new(\"{}{}\");\n", "proton-", "syncd");
    let bare_sync = format!("let c = Command::new(\"{}{}\");\n", "proton-", "sync");
    let path_syncd = format!(
        "let p = \"/home/someone/.cargo/bin/{}{}\";\n",
        "proton-", "syncd"
    );
    let forms: Vec<(&str, String)> = vec![
        // A bare name resolves through PATH to the user's LIVE installed binary.
        ("bare-syncd", bare_syncd),
        ("bare-sync", bare_sync),
        ("path-syncd", path_syncd),
        // `.output()` and `.status()`, not only `.spawn()`.
        ("output", "let _ = tool.arg(\"x\").output();\n".to_owned()),
        ("status", "let _ = tool.arg(\"x\").status();\n".to_owned()),
        // Universal function call syntax.
        (
            "ufcs-spawn",
            "let child = std::process::Command::spawn(&mut command);\n".to_owned(),
        ),
        (
            "ufcs-output",
            "let out = Command::output(&mut command);\n".to_owned(),
        ),
        (
            "ufcs-status",
            "let out = process::Command::status(&mut command);\n".to_owned(),
        ),
        // The method name on a line of its own, and the parenthesis on another.
        (
            "split-spawn",
            "let child = command\n    .spawn\n    ()\n    .unwrap();\n".to_owned(),
        ),
        (
            "split-output",
            "let out = command\n    .output\n    (\n    );\n".to_owned(),
        ),
        // A renamed import hides the type from the `Command::` forms.
        (
            "alias",
            "use std::process::Command as Process;\n".to_owned(),
        ),
        // The file-name constants are data; beside a `Command` they are the bare name.
        (
            "file-name-constant",
            "use std::process::Command;\nlet name = common::DAEMON_FILE_NAME;\n".to_owned(),
        ),
        // A block comment is not understood, so a line inside one fails closed.
        (
            "block-comment",
            format!("/* Command::new(\"{}{}\") */\n", "proton-", "syncd"),
        ),
    ];
    let lines = [
        ("bare-syncd", 1),
        ("bare-sync", 1),
        ("path-syncd", 1),
        ("output", 1),
        ("status", 1),
        ("ufcs-spawn", 1),
        ("ufcs-output", 1),
        ("ufcs-status", 1),
        // The line the method name is on.
        ("split-spawn", 2),
        ("split-output", 2),
        ("alias", 1),
        ("file-name-constant", 2),
        ("block-comment", 1),
    ];
    for ((form, source), (listed, line)) in forms.iter().zip(lines) {
        assert_eq!(*form, listed, "the two lists are in the same order");
        let tests = tempdir().expect("tempdir");
        fs::write(tests.path().join(format!("{form}.rs")), source).expect("probe");
        assert_eq!(
            unsandboxed_spawns(tests.path()),
            [format!("{form}.rs:{line}")],
            "the form `{form}` is caught"
        );
    }
}

#[test]
fn the_spawn_guard_does_not_flag_what_is_not_a_launch() {
    let tests = tempdir().expect("tempdir");
    // Each of these shares a word with a form above and starts nothing.
    fs::write(
        tests.path().join("fine.rs"),
        format!(
            "// Command::new(\"{0}{1}\").spawn() in a comment\n\
             /// and in a doc comment: command.output()\n\
             let child = common::syncd(directory);\n\
             let status = output.status.success();\n\
             let config = \".{0}{2}.toml\";\n\
             let handle = std::thread::spawn(move || ());\n\
             let again = DaemonProcess::spawn_with_config(&config);\n\
             daemon.respawn();\n\
             let label = \"not-{0}{1}\";\n\
             let out = common::run_other_tool(&mut tool);\n\
             let child = common::spawn_logging(&mut command, &log);\n",
            "proton-", "syncd", "sync"
        ),
    )
    .expect("probe");
    // The constants beside no `Command` are a test comparing a manifest.
    fs::write(
        tests.path().join("manifest.rs"),
        "assert_eq!(name, Some(common::DAEMON_FILE_NAME));\n\
         assert!(script.contains(common::CONTROL_CLI_FILE_NAME));\n",
    )
    .expect("probe");
    assert_eq!(unsandboxed_spawns(tests.path()), Vec::<String>::new());
}
