//! `proton-sync doctor` end to end (openspec `add-dependency-check`, tasks 4.1-4.3), against the
//! real control CLI binary and answering stub tools.
//!
//! Every run goes through `common::sync_cli` + `common::DoctorTools` + `common::run_bounded`:
//! `PATH` holds the test's stubs and the sandbox's refusing ones and no system directory, the
//! sandbox has no D-Bus address (a test that wants one sets a fake on purpose), and the lock probe
//! reads a lock file inside the sandbox. A run that reaches a refusing stub fails its test.
#![cfg(unix)]

mod common;

use common::{DoctorTools, Stub};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A `doctor` command in a sandbox at `directory`, running with `tools`.
fn doctor(directory: &Path, tools: &DoctorTools, args: &[&str]) -> Command {
    let mut command = common::sync_cli(directory);
    command.args(args);
    tools.attach(&mut command);
    command
}

fn finish(mut command: Command) -> Run {
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    Run {
        code: output.status.code().expect("exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A fake session bus address: the check only asks whether one is set.
fn with_bus(command: &mut Command, directory: &Path) {
    command.env(
        "DBUS_SESSION_BUS_ADDRESS",
        format!("unix:path={}", directory.join("no-such-bus").display()),
    );
}

/// Every tool present and answering well.
fn answering_tools(directory: &Path) -> DoctorTools {
    let tools = DoctorTools::in_directory(directory);
    tools
        .proton_drive(Stub::exits(0).prints("proton-drive 1.4.2"))
        .secret_tool(Stub::exits(0))
        .systemctl(Stub::exits(0).prints("loaded"));
    tools
}

/// The report line of check `label` (the first column after the outcome word).
fn line_for<'a>(stdout: &'a str, label: &str) -> &'a str {
    stdout
        .lines()
        .find(|line| line.split_whitespace().nth(1) == Some(label))
        .unwrap_or_else(|| panic!("no line for {label} in:\n{stdout}"))
}

#[test]
fn nothing_installed_names_proton_drive_links_the_download_and_exits_1_without_a_socket() {
    // (a) The release pipeline's smoke scenario: a bare machine. No proton-drive, no config, no
    // bus, and no runtime directory, so a control CLI that resolved its socket path would make the
    // fallback directory under TMPDIR.
    let directory = tempdir().expect("tempdir");
    let scratch_tmp = directory.path().join("tmp");
    fs::create_dir(&scratch_tmp).expect("tmp");
    let tools = DoctorTools::in_directory(directory.path());
    let mut command = doctor(directory.path(), &tools, &["doctor"]);
    command
        .env_remove("XDG_RUNTIME_DIR")
        .env("TMPDIR", &scratch_tmp);
    let run = finish(command);

    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    let line = line_for(&run.stdout, "proton-drive");
    assert!(line.contains("missing"), "{line}");
    assert!(
        run.stdout.contains("https://proton.me/drive/download\n"),
        "{}",
        run.stdout
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);

    // Nothing was created for a socket: not in TMPDIR, not anywhere in the sandbox.
    assert_eq!(
        fs::read_dir(&scratch_tmp).expect("tmp").count(),
        0,
        "the runtime fallback directory was made under TMPDIR"
    );
    let mut stack = vec![directory.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read the sandbox") {
            let entry = entry.expect("entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.starts_with("proton-drive-sync-") && !name.ends_with(".sock"),
                "doctor made {}",
                entry.path().display()
            );
            if entry.file_type().expect("type").is_dir() {
                stack.push(entry.path());
            }
        }
    }
}

#[test]
fn everything_in_place_exits_0() {
    // (b)
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    let mut command = doctor(directory.path(), &tools, &["doctor"]);
    with_bus(&mut command, directory.path());
    let run = finish(command);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        line_for(&run.stdout, "proton-drive").contains("proton-drive 1.4.2 at "),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("Everything required is in place."),
        "{}",
        run.stdout
    );
}

#[test]
fn no_session_bus_leaves_the_sign_in_unchecked_and_exits_3() {
    // (c) The sandbox has no bus; the rest answers.
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    let run = finish(doctor(directory.path(), &tools, &["doctor"]));
    assert_eq!(run.code, 3, "{}\n{}", run.stdout, run.stderr);
    assert!(
        line_for(&run.stdout, "signed").contains("unknown"),
        "{}",
        run.stdout
    );
}

#[test]
fn doctor_refuses_the_socket_and_pair_flags_with_exit_2() {
    // (d)
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    let socket = directory.path().join("daemon.sock");
    let socket = socket.to_str().expect("utf-8");
    for args in [
        vec!["--pair", "x", "doctor"],
        vec!["--all-pairs", "doctor"],
        vec!["--socket-path", socket, "doctor"],
    ] {
        let run = finish(doctor(directory.path(), &tools, &args));
        assert_eq!(run.code, 2, "{args:?}: {}\n{}", run.stdout, run.stderr);
        assert!(
            run.stderr.contains(
                "doctor checks this machine, not the daemon; it takes no socket or folder pair"
            ),
            "{args:?}: {}",
            run.stderr
        );
        assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
    }
}

#[test]
fn json_output_parses_and_carries_the_verdict() {
    // (e)
    let directory = tempdir().expect("tempdir");
    let tools = DoctorTools::in_directory(directory.path());
    let run = finish(doctor(directory.path(), &tools, &["--json", "doctor"]));
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    let report: serde_json::Value = serde_json::from_str(&run.stdout).expect("--json is JSON");
    assert_eq!(report["verdict"], "failed");
    let checks = report["checks"].as_array().expect("checks");
    let cli = checks
        .iter()
        .find(|check| check["id"] == "proton-drive")
        .expect("a proton-drive result");
    assert_eq!(cli["outcome"], "missing");
}

#[test]
fn nothing_a_keyring_lookup_prints_reaches_either_stream() {
    // (f) The lookup's stdout is the stored session, so the check sends it to /dev/null.
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    tools.secret_tool(Stub::exits(0).prints("SESSION-MARKER-0451"));
    let mut command = doctor(directory.path(), &tools, &["doctor"]);
    with_bus(&mut command, directory.path());
    let run = finish(command);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        line_for(&run.stdout, "signed").contains("a sign-in is stored"),
        "the lookup ran: {}",
        run.stdout
    );
    for stream in [&run.stdout, &run.stderr] {
        assert!(!stream.contains("SESSION-MARKER-0451"), "{stream}");
    }
}

#[test]
fn proton_cli_flag_beats_the_config_files_proton_cli() {
    // (g)
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    let gone = directory.path().join("gone").join("proton-drive");
    let config = directory.path().join("doctor.toml");
    fs::write(
        &config,
        format!("proton_cli = \"{}\"\n", gone.to_str().expect("utf-8")),
    )
    .expect("config");
    let config = config.to_str().expect("utf-8");

    // The file alone is read: its CLI is the one that is checked, and it is not there.
    let mut command = doctor(directory.path(), &tools, &["--config", config, "doctor"]);
    with_bus(&mut command, directory.path());
    let run = finish(command);
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    let line = line_for(&run.stdout, "proton-drive");
    assert!(line.contains("missing") && line.contains("gone"), "{line}");

    // The flag wins over it.
    let flag = tools.proton_drive_path();
    let flag = flag.to_str().expect("utf-8");
    let mut command = doctor(
        directory.path(),
        &tools,
        &["--config", config, "doctor", "--proton-cli", flag],
    );
    with_bus(&mut command, directory.path());
    let run = finish(command);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        line_for(&run.stdout, "proton-drive").contains(flag),
        "{}",
        run.stdout
    );
}

#[test]
fn the_argument_lists_setup_sh_builds_parse_and_exit_0_when_ready() {
    // (h) `--config F doctor`, `--config F doctor --proton-cli P` and `doctor` alone: the three
    // forms `setup.sh` passes in its terminal, terminal-with-a-CLI and `--gui` modes.
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    let cli = tools.proton_drive_path();
    let cli = cli.to_str().expect("utf-8");
    let config = directory.path().join("setup.toml");
    fs::write(&config, format!("proton_cli = \"{cli}\"\n")).expect("config");
    let config = config.to_str().expect("utf-8");

    for args in [
        vec!["--config", config, "doctor"],
        vec!["--config", config, "doctor", "--proton-cli", cli],
        vec!["doctor"],
    ] {
        let mut command = doctor(directory.path(), &tools, &args);
        with_bus(&mut command, directory.path());
        let run = finish(command);
        assert_eq!(run.code, 0, "{args:?}: {}\n{}", run.stdout, run.stderr);
    }
}

#[test]
fn a_cli_that_does_not_answer_is_unknown_not_missing() {
    // (i) The stub sleeps past `version`'s 10 s limit.
    let directory = tempdir().expect("tempdir");
    let tools = answering_tools(directory.path());
    tools.proton_drive(Stub::never_answers());
    let mut command = doctor(directory.path(), &tools, &["doctor"]);
    with_bus(&mut command, directory.path());
    let run = finish(command);
    assert_eq!(run.code, 3, "{}\n{}", run.stdout, run.stderr);
    let line = line_for(&run.stdout, "proton-drive");
    assert!(
        line.contains("unknown") && line.contains("did not answer"),
        "{line}"
    );
    assert!(!line.contains("missing"), "{line}");
}
