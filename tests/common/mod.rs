//! The one place an integration test starts a binary of this crate.
//!
//! Cargo builds every `tests/*.rs` as its own crate, so each declares `mod common;` and uses a
//! subset of this file.
//!
//! **Why it exists.** A test that spawns `proton-syncd` is one wrong flag away from being a real
//! daemon, and a real daemon reaches the machine it runs on: it binds the control socket under
//! `$XDG_RUNTIME_DIR` (replacing the live daemon's, and deleting it again on exit), takes the
//! user-global lock under `$XDG_STATE_HOME`, moves local deletions into `$XDG_DATA_HOME/Trash`,
//! and reads `$HOME`. A check that is meant to *refuse* a run fails exactly by starting one, so
//! the sandbox cannot be the thing a failing test forgets (the incident is in
//! `docs/agent-notes/resolved-configs-carry-the-real-global-lock.md`).
//!
//! `tests/test_isolation.rs` pins the helpers and refuses any other file that spawns a binary of
//! this crate by hand.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How long a short-lived run (a dry-run preview, a refused start, a control-CLI call) may take.
pub const RUN_BOUND: Duration = Duration::from_secs(60);

/// A [`Command`] for `program` with every process-global default a daemon or the control CLI
/// resolves pointed into `directory`:
///
/// * `XDG_RUNTIME_DIR` — where the default control socket lives (a caller still passes its own
///   `--socket-path`; this is for the flag it forgets);
/// * `XDG_STATE_HOME` — the user-global single-instance lock;
/// * `XDG_DATA_HOME` — the FreeDesktop trash a local deletion moves into;
/// * `HOME` — what the three above fall back to, and what a literal `~` expands to.
///
/// The sandbox cuts the daemon off from the machine's files, not from its keyring session, so a
/// daemon that shells the real `proton-drive` CLI could still reach the real account. That is
/// [`refuse_the_real_cli`]'s to stop, and `run_bounded` applies it.
pub fn sandboxed(program: impl AsRef<OsStr>, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env("HOME", directory)
        .env("XDG_RUNTIME_DIR", directory)
        .env("XDG_STATE_HOME", directory)
        .env("XDG_DATA_HOME", directory);
    command
}

/// Refuses a `proton-syncd` command that names no CLI: the default is `proton-drive` on `PATH`,
/// which is the developer's real one, logged in to the real account through the desktop keyring
/// (which no environment variable here redirects). A test names a fake with `--proton-cli`, or a
/// `--config` file with a `proton_cli` key.
///
/// Any other program passes: the control CLI never runs `proton-drive`.
pub fn refuse_the_real_cli(command: &Command) {
    if Path::new(command.get_program()).file_name() != Some(OsStr::new("proton-syncd")) {
        return;
    }
    let args: Vec<&OsStr> = command.get_args().collect();
    let flag = args
        .iter()
        .any(|arg| *arg == "--proton-cli" || arg.to_string_lossy().starts_with("--proton-cli="));
    let file = args.windows(2).any(|pair| {
        pair[0] == "--config"
            && fs::read_to_string(pair[1]).is_ok_and(|text| {
                text.lines()
                    .any(|line| line.trim_start().starts_with("proton_cli"))
            })
    });
    assert!(
        flag || file,
        "{command:?} names no fake CLI (--proton-cli, or a config with proton_cli): it would run \
         the real `proton-drive`, signed in to the real account"
    );
}

/// Starts a long-running `command` (a daemon) with its stderr written to `stderr_path`, which the
/// test's wait helpers quote when they time out. The caller owns the child and its bound: kill it
/// on drop, and wait for it with a deadline. **The one way these tests start a daemon.**
pub fn spawn_logging(command: &mut Command, stderr_path: &Path) -> Child {
    refuse_the_real_cli(command);
    let stderr = File::create(stderr_path).expect("create the daemon's stderr log");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap_or_else(|error| panic!("spawn {command:?}: {error}"))
}

/// Runs `command` to completion and returns what it printed, **killing it after `limit`**.
///
/// A run that should have ended and has not is a run that turned into a daemon, so the overrun is
/// a failure with a message that says so, never a hang. Output goes to unlinked temporary files
/// rather than pipes: a child that prints more than a pipe holds would block on a reader that is
/// busy polling for its exit.
pub fn run_bounded(command: &mut Command, limit: Duration) -> Output {
    refuse_the_real_cli(command);
    let mut stdout = tempfile::tempfile().expect("a temporary file for stdout");
    let mut stderr = tempfile::tempfile().expect("a temporary file for stderr");
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone().expect("clone stdout")))
        .stderr(Stdio::from(stderr.try_clone().expect("clone stderr")))
        .spawn()
        .unwrap_or_else(|error| panic!("spawn {command:?}: {error}"));
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "{command:?} was still running after {limit:?}: a run that should have ended \
                 started a daemon (or hung)\nstderr so far: {}",
                read_back(&mut stderr)
            );
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: read_back_bytes(&mut stdout),
        stderr: read_back_bytes(&mut stderr),
    }
}

fn read_back_bytes(file: &mut File) -> Vec<u8> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0)).expect("rewind the capture");
    file.read_to_end(&mut bytes).expect("read the capture");
    bytes
}

fn read_back(file: &mut File) -> String {
    String::from_utf8_lossy(&read_back_bytes(file)).into_owned()
}
