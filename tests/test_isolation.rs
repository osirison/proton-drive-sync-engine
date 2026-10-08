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

/// The rule, enforced: **no test file other than `tests/common/mod.rs` contains the name of one of
/// this crate's binaries** (`CARGO_BIN_` + `EXE_`, which is also what `env!`, `option_env!` and
/// `std::env::var` would be handed). A test starts `proton-syncd` and `proton-sync` through
/// `common::syncd` / `common::sync_cli` (sandboxed), runs one to completion with
/// `common::run_bounded` or starts a daemon with `common::spawn_logging`; and `.spawn()` is
/// `spawn_logging`'s alone.
///
/// What this does and does not catch, exactly. It is a text scan of `tests/**/*.rs`, comment lines
/// skipped. It catches the literal name in any form (`Command::new(env!("..."))`, the name bound to
/// a variable first, `option_env!`), and a `.spawn()` call. It does **not** catch a binary reached
/// without that name: a path built from `CARGO_MANIFEST_DIR` and `target/`, the name split across
/// `concat!` pieces, or a copy of the binary run from elsewhere. It does not follow data flow, and
/// the needle is assembled so that this file does not match itself.
#[test]
fn no_test_file_starts_a_binary_of_this_crate_without_the_sandbox() {
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let offenders = unsandboxed_spawns(&tests_dir);
    assert!(
        offenders.is_empty(),
        "start proton-syncd / proton-sync through common::syncd / common::sync_cli (HOME and the XDG \
         dirs in the test's own directory, no desktop session bus, stub keyring tools), run to \
         completion with common::run_bounded or start a daemon with common::spawn_logging; \
         a bare binary name or `.spawn()` at: {offenders:?}"
    );
}

/// `file:line` for every line under `tests_dir` (recursively) that names one of the crate's
/// binaries or calls `.spawn()`, outside the two files that are allowed to: `common/mod.rs`, and
/// this one (whose mentions are assembled at run time).
fn unsandboxed_spawns(tests_dir: &Path) -> Vec<String> {
    let name = format!("{}{}", "CARGO_BIN_", "EXE_");
    let spawn = format!("{}{}", ".spa", "wn()");
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
            for (index, line) in text.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                if code.contains(&name) || code.contains(&spawn) {
                    offenders.push(format!("{relative}:{}", index + 1));
                }
            }
        }
    }
    offenders.sort();
    offenders
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
            "bypass.rs:4",
            "nested/deep.rs:1",
            "spawns.rs:1"
        ]
    );
}
