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
const PRINT_ENVIRONMENT: &str =
    r#"printf '%s\n%s\n%s\n%s\n' "$HOME" "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_DATA_HOME""#;

#[test]
fn a_sandboxed_command_resolves_every_process_global_default_into_its_directory() {
    // Run on a machine that has all four set (a developer's session does), the child must still
    // see the test's directory: `Command::env` is applied over whatever the parent inherited.
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed("sh", directory.path());
    command.arg("-c").arg(PRINT_ENVIRONMENT);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    let seen = String::from_utf8(output.stdout).expect("utf-8");
    let seen: Vec<&str> = seen.lines().collect();
    let expected = directory.path().to_str().expect("utf-8 path");
    assert_eq!(
        seen, [expected; 4],
        "HOME, XDG_RUNTIME_DIR, XDG_STATE_HOME and XDG_DATA_HOME must all name the test's own \
         directory"
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
        !refused(Command::new(&program).args(["--proton-cli", "/fake"])),
        "a flag names one"
    );
    let named = directory.path().join("named.toml");
    fs::write(&named, "# a comment\nproton_cli = \"/fake\"\n").expect("config");
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

/// The rule, enforced: a test file starts this crate's binaries through `common::sandboxed`, and
/// a long-running one through `common::spawn_logging`.
///
/// A hand-built `Command` for `proton-syncd` is the shape of the incident — it works, it is bounded
/// by nothing, and it reaches whatever the machine's environment names. The needle is assembled so
/// that this file does not match itself.
#[test]
fn no_test_file_starts_a_binary_of_this_crate_without_the_sandbox() {
    let needle = format!("{}{}", "Command::new(env!(\"CARGO_BIN_", "EXE_");
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut offenders = Vec::new();
    for entry in fs::read_dir(&tests_dir).expect("tests directory") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("test source");
        for (index, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let name = path.file_name().expect("file name").to_string_lossy();
            // `.spawn()` is `common::spawn_logging`'s alone: it is where the fake-CLI refusal and
            // the stderr capture are applied. (This file's own mentions are in strings below.)
            let spawn = format!("{}{}", ".spa", "wn()");
            if code.contains(&needle) || (code.contains(&spawn) && name != "test_isolation.rs") {
                offenders.push(format!("{name}:{}", index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "start proton-syncd / proton-sync through common::sandboxed (HOME and the XDG dirs in the \
         test's own directory), run to completion with common::run_bounded or start a daemon with \
         common::spawn_logging; hand-built at: {offenders:?}"
    );
}
