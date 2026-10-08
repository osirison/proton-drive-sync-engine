//! The one place an integration test starts a binary of this crate.
//!
//! Cargo builds every `tests/*.rs` as its own crate, so each declares `mod common;` and uses a
//! subset of this file.
//!
//! **Why it exists.** A test that spawns `proton-syncd` is one wrong flag away from being a real
//! daemon, and a real daemon reaches the machine it runs on: it binds the control socket under
//! `$XDG_RUNTIME_DIR` (replacing the live daemon's, and deleting it again on exit), takes the
//! user-global lock under `$XDG_STATE_HOME`, moves local deletions into `$XDG_DATA_HOME/Trash`,
//! reads `$HOME`, and — with `events_driven` on, which is the default — runs `secret-tool` to read
//! the signed-in Proton session out of the desktop keyring and then calls the events API with it.
//! A check that is meant to *refuse* a run fails exactly by starting one, so the sandbox cannot be
//! the thing a failing test forgets (the incident is in
//! `docs/agent-notes/resolved-configs-carry-the-real-global-lock.md`).
//!
//! `tests/test_isolation.rs` pins the helpers and refuses any other file that names a binary of
//! this crate (`CARGO_BIN_EXE_`).
#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// How long a short-lived run (a dry-run preview, a refused start, a control-CLI call) may take.
pub const RUN_BOUND: Duration = Duration::from_secs(60);

/// The environment variable a sandboxed command's stub tools append to when they are run. It names
/// a file in the sandbox's own directory, created only by a stub being reached.
pub const STUB_LOG_ENV: &str = "PROTON_SYNC_TEST_STUB_LOG";

/// Set on a command by [`opt_in_to_events_driven`]; read by [`refuse_the_real_cli`].
const EVENTS_OPT_IN_ENV: &str = "PROTON_SYNC_TEST_EVENTS_OPT_IN";

/// The tools by which a daemon with `events_driven` on reaches the real Proton session: `secret-tool`
/// reads it out of the desktop keyring, `curl` calls the events API with it.
const STUBBED_TOOLS: [&str; 2] = ["secret-tool", "curl"];

/// A [`Command`] for `program`, sandboxed by [`sandbox`].
pub fn sandboxed(program: impl AsRef<OsStr>, directory: &Path) -> Command {
    let mut command = Command::new(program);
    sandbox(&mut command, directory);
    command
}

/// Cuts `command` off from the machine it runs on:
///
/// * `XDG_RUNTIME_DIR` — where the default control socket lives (a caller still passes its own
///   `--socket-path`; this is for the flag it forgets);
/// * `XDG_STATE_HOME` — the user-global single-instance lock;
/// * `XDG_DATA_HOME` — the FreeDesktop trash a local deletion moves into;
/// * `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` — nothing in the daemon reads them, but the install
///   scripts and the desktop app do, and a test that runs one must not find the real config;
/// * `HOME` — what the above fall back to, and what a literal `~` expands to.
///
/// And from the user's **signed-in Proton session**, which a daemon with `events_driven` on reads
/// from the desktop keyring over D-Bus and then uses against the real events API. Defence in depth,
/// because each layer is the one the previous layer's mistake needs:
///
/// * `DBUS_SESSION_BUS_ADDRESS` and `DBUS_SYSTEM_BUS_ADDRESS` are removed, so there is no bus to
///   ask (and `XDG_RUNTIME_DIR`, where the default session bus socket is, names the sandbox);
/// * a directory holding stub `secret-tool` and `curl` is **first on `PATH`**: each exits non-zero
///   and appends to a file in `directory` named by [`STUB_LOG_ENV`], and [`run_bounded`] fails a
///   test whose run reached one, so an incident is loud instead of a degraded-but-passing run;
/// * [`refuse_the_real_cli`] refuses a daemon whose configuration leaves `events_driven` on.
///
/// A caller that clears the environment afterwards (the install-script sandbox does) is on its own
/// for the rest and keeps the stubs by building `PATH` with [`path_with_stubs`].
pub fn sandbox(command: &mut Command, directory: &Path) {
    command
        .env("HOME", directory)
        .env("XDG_RUNTIME_DIR", directory)
        .env("XDG_STATE_HOME", directory)
        .env("XDG_DATA_HOME", directory)
        .env("XDG_CONFIG_HOME", directory)
        .env("XDG_CACHE_HOME", directory)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("DBUS_SYSTEM_BUS_ADDRESS")
        .env(
            "PATH",
            path_with_stubs(&std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())),
        )
        .env(STUB_LOG_ENV, directory.join(".sandbox-stub-hits"));
}

/// `rest` (a `PATH` value) behind the directory of stub tools.
pub fn path_with_stubs(rest: &str) -> OsString {
    let mut path = OsString::from(stub_directory());
    path.push(":");
    path.push(rest);
    path
}

/// A sandboxed `proton-syncd` (see [`sandbox`]). **The only place a test names the binary**: the
/// guard in `tests/test_isolation.rs` fails any other file that contains its `CARGO_BIN_EXE_` name.
pub fn syncd(directory: &Path) -> Command {
    sandboxed(env!("CARGO_BIN_EXE_proton-syncd"), directory)
}

/// A sandboxed `proton-sync`, the control CLI (see [`sandbox`]).
pub fn sync_cli(directory: &Path) -> Command {
    sandboxed(env!("CARGO_BIN_EXE_proton-sync"), directory)
}

/// The directory of stub tools, written once per test binary. It lives under cargo's own scratch
/// space (`CARGO_TARGET_TMPDIR`, emptied by `cargo clean`), not in a test's directory, which can be
/// the very folder a daemon syncs. Written beside and renamed into place, so two test binaries
/// starting together never exec a half-written file.
fn stub_directory() -> &'static Path {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    DIRECTORY.get_or_init(|| {
        let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sandbox-bin");
        fs::create_dir_all(&directory).expect("create the stub tools' directory");
        for tool in STUBBED_TOOLS {
            let body = format!(
                "#!/bin/sh\n\
                 # Stub of `{tool}` for a test sandbox (tests/common/mod.rs). Reaching it means a\n\
                 # test started something that would have used the real Proton session.\n\
                 echo \"{tool} $*\" >> \"${{{STUB_LOG_ENV}:-/dev/null}}\"\n\
                 exit 97\n"
            );
            let target = directory.join(tool);
            if fs::read_to_string(&target).is_ok_and(|existing| existing == body) {
                continue;
            }
            let staging = directory.join(format!(".{tool}.{}", std::process::id()));
            fs::write(&staging, body).expect("write a stub tool");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
                    .expect("make a stub tool executable");
            }
            fs::rename(&staging, &target).expect("put a stub tool in place");
        }
        directory
    })
}

/// What the stubs recorded for `command` (empty when none was reached): one line per call. Read
/// from the sandbox directory the command was given, so a command that was not sandboxed — or whose
/// environment was cleared since — reports nothing.
pub fn stub_hits(command: &Command) -> String {
    command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new(STUB_LOG_ENV))
        .and_then(|(_, value)| value)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Lets `command` (a `proton-syncd`) leave `events_driven` on. **No test needs this today**: a
/// daemon with events on reads the signed-in Proton session from the desktop keyring, and the
/// stubs in [`sandbox`] make that fail rather than succeed, so the only reason to opt in is a test
/// *about* that failure — which should say so by calling this.
pub fn opt_in_to_events_driven(command: &mut Command) -> &mut Command {
    command.env(EVENTS_OPT_IN_ENV, "1")
}

/// Refuses a `proton-syncd` command that would reach the developer's real account:
///
/// * **it names no CLI.** The default is `proton-drive` on `PATH`, the developer's real one, signed
///   in through the desktop keyring (which no environment variable here redirects). A test names a
///   fake with `--proton-cli`, or a `--config` file with a `proton_cli` key;
/// * **it leaves `events_driven` on** (the default) in any pair: the daemon would shell
///   `secret-tool` for the real session and call the events API with it. A test passes
///   `--no-events-driven`, or sets `events_driven = false` in the config file (in *every* `[[pair]]`
///   table, when it has them), or calls [`opt_in_to_events_driven`].
///
/// Any other program passes: the control CLI never runs `proton-drive` or reads the keyring.
pub fn refuse_the_real_cli(command: &Command) {
    if Path::new(command.get_program()).file_name() != Some(OsStr::new("proton-syncd")) {
        return;
    }
    let args: Vec<&OsStr> = command.get_args().collect();
    let flag = args
        .iter()
        .any(|arg| *arg == "--proton-cli" || arg.to_string_lossy().starts_with("--proton-cli="));
    let config_texts: Vec<String> = args
        .windows(2)
        .filter(|pair| pair[0] == "--config")
        .map(|pair| fs::read_to_string(pair[1]).unwrap_or_default())
        .collect();
    let file = config_texts.iter().any(|text| {
        text.lines()
            .any(|line| line.trim_start().starts_with("proton_cli"))
    });
    assert!(
        flag || file,
        "{command:?} names no fake CLI (--proton-cli, or a config with proton_cli): it would run \
         the real `proton-drive`, signed in to the real account"
    );
    let opted_in = command
        .get_envs()
        .any(|(name, value)| name == OsStr::new(EVENTS_OPT_IN_ENV) && value.is_some());
    let events_off = args.iter().any(|arg| *arg == "--no-events-driven")
        || config_texts
            .iter()
            .any(|text| config_turns_events_off(text));
    assert!(
        opted_in || events_off,
        "{command:?} leaves events_driven on (the default): the daemon would read the signed-in \
         Proton session out of the desktop keyring with `secret-tool` and call the events API with \
         it. Pass --no-events-driven, set `events_driven = false` in the config (in every \
         [[pair]] table, if it has them), or call common::opt_in_to_events_driven"
    );
}

/// Whether the config file `text` has events off for **every** pair it declares: the top-level key
/// when it has no `[[pair]]` tables, else each table's own (a table that is silent is on, the
/// default). A file that does not parse is not off: the daemon would refuse it, but a guard that
/// is wrong should be wrong loudly.
fn config_turns_events_off(text: &str) -> bool {
    let Ok(document) = text.parse::<toml::Table>() else {
        return false;
    };
    let off = |table: &toml::Table| {
        table.get("events_driven").and_then(toml::Value::as_bool) == Some(false)
    };
    match document.get("pair").and_then(toml::Value::as_array) {
        Some(tables) if !tables.is_empty() => {
            tables.iter().all(|table| table.as_table().is_some_and(off))
        }
        _ => off(&document),
    }
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
///
/// **A run that reached a stub tool fails too** ([`stub_hits`]): it tried to read the real Proton
/// session or call the network, and the stub made that harmlessly fail, so without this the test
/// would have gone on to pass on a degraded run.
pub fn run_bounded(command: &mut Command, limit: Duration) -> Output {
    let (output, hits) = run_bounded_reporting_stub_hits(command, limit);
    assert!(
        hits.is_empty(),
        "{command:?} reached the real keyring or network (a stub tool in the sandbox answered):\n{hits}"
    );
    output
}

/// [`run_bounded`] that returns the stub tools' record instead of failing on it: for the one test
/// that proves the stubs answer.
pub fn run_bounded_reporting_stub_hits(command: &mut Command, limit: Duration) -> (Output, String) {
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
                 started a daemon (or hung)\nstderr so far: {}\nstub tools reached: {}",
                read_back(&mut stderr),
                stub_hits(command)
            );
        }
        thread::sleep(Duration::from_millis(20));
    };
    let output = Output {
        status,
        stdout: read_back_bytes(&mut stdout),
        stderr: read_back_bytes(&mut stderr),
    };
    (output, stub_hits(command))
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
