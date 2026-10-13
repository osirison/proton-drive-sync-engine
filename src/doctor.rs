//! The dependency check behind `proton-sync doctor`, the daemon's start-up log, `setup.sh` and the
//! desktop app's first-run screen (`openspec/changes/add-dependency-check`).
//!
//! One definition of each check, run against an injected [`Environment`]: every outside fact (the
//! `PATH`, the environment, a file, the daemon's lock, a child process) comes through it, so every
//! branch is a unit test against a table of answers and no test starts a real tool.

use std::ffi::OsString;
use std::io::{self, BufRead, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Serialize;
use wait_timeout::ChildExt;

use crate::config;
use crate::daemon::{GlobalLockProbe, probe_daemon_lock};
use crate::proton::CliGate;
use crate::session::keyring_lookup_args;

/// Every URL and docs anchor the check may show a person, in one place. A link spelled anywhere
/// else is a second copy that can drift, so callers take it from here.
///
/// `tests/doctor_links.rs` holds the anchors to the troubleshooting page: each one must be the
/// slug of exactly one heading there.
pub mod links {
    /// Proton's download page for the `proton-drive` CLI.
    pub const PROTON_DOWNLOAD: &str = "https://proton.me/drive/download";
    /// Proton's direct download index.
    pub const PROTON_DIRECT_DOWNLOAD: &str = "https://proton.me/download/drive/cli/index.html";
    /// Proton's support page for the CLI.
    pub const PROTON_SUPPORT: &str = "https://proton.me/support/drive-cli";
    /// The CLI's README, which documents its commands.
    pub const PROTON_README: &str =
        "https://github.com/ProtonDriveApps/sdk/blob/main/cli/README.md";

    /// The command that signs the CLI in.
    pub const SIGN_IN_COMMAND: &str = "proton-drive auth login";

    /// The docs site (the default `site` + `base` of `website/astro.config.mjs`).
    pub const DOCS_SITE: &str = "https://osirison.github.io/proton-drive-sync-engine";

    /// The docs page every check links to, relative to [`DOCS_SITE`].
    pub const TROUBLESHOOTING_PAGE: &str = "/reference/troubleshooting/";

    /// One troubleshooting anchor per check id. A heading a check links to uses only ASCII
    /// letters, digits, spaces and hyphens, so its slug is the heading lower-cased with each space
    /// made a hyphen, whichever slugger the site uses.
    pub const ANCHORS: [(&str, &str); 7] = [
        ("proton-drive", "proton-drive-is-missing-or-does-not-run"),
        ("signed-in", "no-proton-drive-sign-in-is-stored"),
        ("config", "the-config-file-cannot-be-read"),
        ("secret-tool", "secret-tool-is-missing"),
        ("curl", "curl-is-missing"),
        ("session-bus", "no-session-bus"),
        ("service", "the-proton-syncd-service-is-not-installed"),
    ];

    /// The full docs URL for a check id, or `None` for an id with no anchor.
    pub fn docs_url(check: &str) -> Option<String> {
        ANCHORS
            .iter()
            .find(|(id, _)| *id == check)
            .map(|(_, anchor)| format!("{DOCS_SITE}{TROUBLESHOOTING_PAGE}#{anchor}"))
    }

    /// The URL behind a link id on a check's result, for a caller that was handed only the two ids
    /// (the desktop app's `open_help_link`). `None` for a check or link this module never emits,
    /// and for a Proton page on a check that carries none.
    pub fn url_for(check: &str, link: &str) -> Option<String> {
        let docs = docs_url(check)?;
        let on_proton_page = matches!(check, "proton-drive" | "signed-in");
        match link {
            "docs" => Some(docs),
            "proton-download" if on_proton_page => Some(PROTON_DOWNLOAD.to_owned()),
            "proton-cli-docs" if on_proton_page => Some(PROTON_SUPPORT.to_owned()),
            _ => None,
        }
    }
}

// ---- the result types -------------------------------------------------------------------------

/// The seven checks, with stable wire ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckId {
    ProtonDrive,
    SignedIn,
    Config,
    SecretTool,
    Curl,
    SessionBus,
    Service,
}

impl CheckId {
    /// In report order.
    pub const ALL: [CheckId; 7] = [
        CheckId::ProtonDrive,
        CheckId::SignedIn,
        CheckId::Config,
        CheckId::SecretTool,
        CheckId::Curl,
        CheckId::SessionBus,
        CheckId::Service,
    ];

    /// The wire id; also the key of [`links::ANCHORS`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProtonDrive => "proton-drive",
            Self::SignedIn => "signed-in",
            Self::Config => "config",
            Self::SecretTool => "secret-tool",
            Self::Curl => "curl",
            Self::SessionBus => "session-bus",
            Self::Service => "service",
        }
    }

    /// The name the human report draws.
    pub fn label(self) -> &'static str {
        match self {
            Self::SignedIn => "signed in",
            other => other.as_str(),
        }
    }

    /// Fixed per id: a required check is one without which no sync can work (or the daemon does
    /// not start); a recommended one is one without which sync still works.
    pub fn severity(self) -> Severity {
        match self {
            Self::ProtonDrive | Self::SignedIn | Self::Config => Severity::Required,
            Self::SecretTool | Self::Curl | Self::SessionBus | Self::Service => {
                Severity::Recommended
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Required,
    Recommended,
}

/// `missing` and `failed` are the two kinds of problem; `unknown` is "could not check" and is
/// neither of them, nor ok.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Missing,
    Failed,
    Unknown,
}

impl Outcome {
    fn is_problem(self) -> bool {
        matches!(self, Self::Missing | Self::Failed)
    }
}

/// A link a result carries: an id the app can open it by, and the URL. URLs come only from
/// [`links`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub id: &'static str,
    pub url: String,
}

impl Link {
    /// This check's section of the troubleshooting page.
    fn docs(check: CheckId) -> Self {
        Self {
            id: "docs",
            url: links::docs_url(check.as_str()).expect("every check has a docs anchor"),
        }
    }

    fn proton_download() -> Self {
        Self {
            id: "proton-download",
            url: links::PROTON_DOWNLOAD.to_owned(),
        }
    }

    fn proton_cli_docs() -> Self {
        Self {
            id: "proton-cli-docs",
            url: links::PROTON_SUPPORT.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    pub id: CheckId,
    pub severity: Severity,
    pub outcome: Outcome,
    pub statement: String,
    /// One line; `None` only for an ok result.
    pub fix: Option<String>,
    /// Empty for an ok result.
    pub links: Vec<Link>,
}

/// The overall answer (D7): `failed` if a required check is missing or failed, else `unknown` if a
/// required check could not be checked, else `ok`. Recommended results never move it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub checks: Vec<CheckResult>,
    pub verdict: Verdict,
}

impl Report {
    pub fn new(checks: Vec<CheckResult>) -> Self {
        let required = || {
            checks
                .iter()
                .filter(|check| check.severity == Severity::Required)
        };
        let verdict = if required().any(|check| check.outcome.is_problem()) {
            Verdict::Failed
        } else if required().any(|check| check.outcome == Outcome::Unknown) {
            Verdict::Unknown
        } else {
            Verdict::Ok
        };
        Self { checks, verdict }
    }

    /// The human report (D8). `color` follows the caller's terminal rule.
    pub fn render_human(&self, color: bool) -> String {
        let paint = |code: &str, text: &str| {
            if color {
                format!("\x1b[{code}m{text}\x1b[0m")
            } else {
                text.to_owned()
            }
        };
        let mut out =
            String::from("proton-sync doctor: what this machine needs before the first sync\n\n");
        for check in &self.checks {
            let (word, code) = match check.outcome {
                Outcome::Ok => ("ok", "32"),
                Outcome::Missing => ("missing", "31"),
                Outcome::Failed => ("failed", "31"),
                Outcome::Unknown => ("unknown", "33"),
            };
            out.push_str(&format!(
                "  {} {:<13} {}\n",
                paint(code, &format!("{word:<8}")),
                check.id.label(),
                check.statement
            ));
            if check.outcome == Outcome::Ok {
                continue;
            }
            if let Some(fix) = &check.fix {
                out.push_str(&format!("           fix:  {fix}\n"));
            }
            for link in &check.links {
                let tag = if link.id == "docs" {
                    "docs: "
                } else {
                    "help: "
                };
                // The URL ends the line: nothing is attached to it.
                out.push_str(&format!("           {tag}{}\n", link.url));
            }
        }
        out.push('\n');
        out.push_str(&self.closing_line());
        out.push('\n');
        out
    }

    fn closing_line(&self) -> String {
        let count = |wanted: fn(Outcome) -> bool, severity| {
            self.checks
                .iter()
                .filter(|check| check.severity == severity && wanted(check.outcome))
                .count()
        };
        let plural = |n: usize, one: &str, many: &str| if n == 1 { one } else { many }.to_owned();
        let required_problems = count(Outcome::is_problem, Severity::Required);
        let required_unknown = count(|o| o == Outcome::Unknown, Severity::Required);
        let recommended = count(Outcome::is_problem, Severity::Recommended);
        match self.verdict {
            Verdict::Failed => format!(
                "{required_problems} required {}. Fix {} and run `proton-sync doctor` again.",
                plural(required_problems, "problem", "problems"),
                plural(required_problems, "it", "them"),
            ),
            Verdict::Unknown => format!(
                "No required problem, but {required_unknown} required {} could not be checked. \
                 Run `proton-sync doctor` again where it can be.",
                plural(required_unknown, "check", "checks"),
            ),
            Verdict::Ok if recommended > 0 => format!(
                "Everything required is in place. {recommended} recommended {} above, which \
                 do not stop sync.",
                plural(recommended, "problem is listed", "problems are listed"),
            ),
            Verdict::Ok => "Everything required is in place.".to_owned(),
        }
    }
}

// ---- the inputs and the environment seam ------------------------------------------------------

/// Which CLI the `proton-drive` check runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliSource {
    /// A path the caller already settled on (`--proton-cli`, or the daemon's resolved
    /// `proton_cli`). The config file is not consulted for it.
    Explicit(PathBuf),
    /// The validated config file's `proton_cli`, else `proton-drive` on `PATH` (D6).
    FromConfig,
}

/// Which config file the `config` check reads. The caller supplies the default path
/// ([`crate::paths::default_config_path`]), so the check reads no environment for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigFile {
    /// The default file: absent is fine ("no config file yet").
    Default(PathBuf),
    /// There is no default file to name (no absolute `XDG_CONFIG_HOME` or `HOME`). Nothing is
    /// read: a file in the shared temp directory could be planted by any local user, and its
    /// `proton_cli` would be run. The check says it could not be made and the CLI is looked up on
    /// `PATH`.
    NoHome,
    /// `--config`: absent is a problem.
    Named(PathBuf),
}

impl ConfigFile {
    /// The default file at `path`, or [`Self::NoHome`] when there is none to name.
    pub fn default_at(path: Option<PathBuf>) -> Self {
        path.map_or(Self::NoHome, Self::Default)
    }
}

/// Whether the `proton-drive` check looks at the user-global lock before it runs `version`.
///
/// `version` is not proven to leave the CLI's shared state alone (design A2), and the desktop app
/// runs this check beside a live daemon far more often than anyone runs `doctor`, so the probe is
/// part of the shared check and not of any one caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonLock {
    /// Probe this lock (shared, non-blocking); a daemon holding it means "do not spawn".
    Probe(PathBuf),
    /// Do not probe. The daemon's own start-up check, which holds the lock itself and so would
    /// always read "held".
    Skip,
}

impl DaemonLock {
    /// The default global lock, or [`Self::Skip`] when there is none to look at. The check creates
    /// nothing, so with no usable `XDG_STATE_HOME` or `HOME` the lock would be under a per-uid
    /// temp directory that only a daemon makes: when it is not there, no daemon of this user is
    /// running and there is nothing to probe. Skipping is the same as the probe answering
    /// `Unknown` or `Free`, which does not stop the check from running.
    pub fn user_default() -> Self {
        crate::paths::existing_global_lock_path().map_or(Self::Skip, Self::Probe)
    }
}

pub struct Inputs {
    pub cli: CliSource,
    pub config: ConfigFile,
    pub checks: Vec<CheckId>,
    pub daemon_lock: DaemonLock,
    /// When set, the `version` spawn is made while holding this gate (the daemon passes its
    /// client's, so the check never overlaps a `list` request's child, #23).
    pub gate: Option<Arc<CliGate>>,
    /// When set and raised, a running `version` is killed and reaped at once (the daemon's cancel
    /// flag), so no child outlives the daemon that started it (#23).
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Inputs {
    /// Every check, the CLI taken from the config, the default lock probed, no gate, not
    /// cancellable.
    pub fn new(config: ConfigFile) -> Self {
        Self {
            cli: CliSource::FromConfig,
            config,
            checks: CheckId::ALL.to_vec(),
            daemon_lock: DaemonLock::user_default(),
            gate: None,
            cancel: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdout {
    Keep,
    /// `/dev/null`: the output never enters this process.
    Discard,
}

/// How a child ended (D2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// First line of each pipe, cut to 200 characters; `stdout_head` is empty under `Discard`.
    Exited {
        code: i32,
        stdout_head: String,
        stderr_head: String,
    },
    Signalled(i32),
    NotFound,
    NotExecutable,
    ExecFormat,
    SpawnError(io::ErrorKind),
    TimedOut,
    /// The caller's cancel flag was raised; the child was killed and reaped.
    Cancelled,
}

pub trait Environment {
    /// A `PATH` search that executes nothing.
    fn find_program(&self, name: &str) -> Option<PathBuf>;
    fn var(&self, name: &str) -> Option<OsString>;
    fn read_file(&self, path: &Path) -> io::Result<String>;
    /// Looks at the user-global lock (see [`probe_daemon_lock`]).
    fn probe_lock(&self, path: &Path) -> GlobalLockProbe;
    /// stdin is always `/dev/null`.
    fn run(&self, program: &Path, args: &[&str], limit: Duration, stdout: Stdout) -> Ran;
    /// [`Self::run`] that stops early, killing and reaping the child, once `cancel` is raised
    /// ([`Ran::Cancelled`]). The default ignores the flag, which is right for an environment that
    /// starts no real child.
    fn run_cancellable(
        &self,
        program: &Path,
        args: &[&str],
        limit: Duration,
        stdout: Stdout,
        _cancel: Option<&AtomicBool>,
    ) -> Ran {
        self.run(program, args, limit, stdout)
    }
    /// Whether `path` is a directory (a stat, nothing executed). The default says no.
    fn is_directory(&self, _path: &Path) -> bool {
        false
    }
    /// An environment that cannot look at anything ([`InertEnvironment`]): a caller that would
    /// report its answers as problems skips the check instead.
    fn is_inert(&self) -> bool {
        false
    }
}

/// The real environment.
pub struct SystemEnvironment;

/// Finds nothing and runs nothing: the daemon's default, so a `Daemon` built by a test cannot
/// reach a real tool. Only `Daemon::new` installs [`SystemEnvironment`].
pub struct InertEnvironment;

impl Environment for InertEnvironment {
    fn find_program(&self, _name: &str) -> Option<PathBuf> {
        None
    }

    fn var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn read_file(&self, _path: &Path) -> io::Result<String> {
        Err(io::ErrorKind::NotFound.into())
    }

    fn probe_lock(&self, _path: &Path) -> GlobalLockProbe {
        GlobalLockProbe::Unknown("inert environment".to_owned())
    }

    fn run(&self, _program: &Path, _args: &[&str], _limit: Duration, _stdout: Stdout) -> Ran {
        Ran::NotFound
    }

    fn is_inert(&self) -> bool {
        true
    }
}

const HEAD_CHARS: usize = 200;
/// How long to wait for a pipe's first line once the child is gone. A grandchild that kept the
/// pipe open must not hold the check up.
const HEAD_WAIT: Duration = Duration::from_millis(500);
/// How often a cancellable wait looks at its flag.
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// What stands in for a control character in text a child printed.
const CONTROL_PLACEHOLDER: char = '\u{FFFD}';

impl Environment for SystemEnvironment {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        // Only absolute entries: an empty or relative one would be searched against whatever
        // directory the caller happens to be in.
        std::env::split_paths(&path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(name))
            .find(|candidate| {
                candidate
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
    }

    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn probe_lock(&self, path: &Path) -> GlobalLockProbe {
        probe_daemon_lock(path)
    }

    fn run(&self, program: &Path, args: &[&str], limit: Duration, stdout: Stdout) -> Ran {
        self.run_cancellable(program, args, limit, stdout, None)
    }

    fn run_cancellable(
        &self,
        program: &Path,
        args: &[&str],
        limit: Duration,
        stdout: Stdout,
        cancel: Option<&AtomicBool>,
    ) -> Ran {
        #[cfg(test)]
        refuse_real_tool(program);
        let spawned = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(match stdout {
                Stdout::Keep => Stdio::piped(),
                Stdout::Discard => Stdio::null(),
            })
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return spawn_failure(&error),
        };
        let out = child.stdout.take().map(first_line);
        let err = child.stderr.take().map(first_line);
        // Waits in short slices only when there is a flag to watch; otherwise one wait.
        let deadline = Instant::now() + limit;
        let waited = loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                break Err(Ran::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let slice = match cancel {
                Some(_) => remaining.min(CANCEL_POLL),
                None => remaining,
            };
            match child.wait_timeout(slice) {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if remaining <= slice => break Err(Ran::TimedOut),
                Ok(None) => {}
                Err(error) => break Err(Ran::SpawnError(error.kind())),
            }
        };
        match waited {
            Ok(status) => match status.signal() {
                Some(signal) => Ran::Signalled(signal),
                None => Ran::Exited {
                    code: status.code().unwrap_or(-1),
                    stdout_head: out.map(head).unwrap_or_default(),
                    stderr_head: err.map(head).unwrap_or_default(),
                },
            },
            Err(ran) => {
                // Timed out, cancelled, or the wait itself failed: stop the child and reap it, so
                // nothing is left running or zombied. Only the direct child is killed.
                let _ = child.kill();
                let _ = child.wait();
                ran
            }
        }
    }

    fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

fn spawn_failure(error: &io::Error) -> Ran {
    if error.raw_os_error() == Some(libc::ENOEXEC) {
        return Ran::ExecFormat;
    }
    match error.kind() {
        io::ErrorKind::NotFound => Ran::NotFound,
        io::ErrorKind::PermissionDenied => Ran::NotExecutable,
        kind => Ran::SpawnError(kind),
    }
}

/// Reads a pipe on its own thread: sends the first line (cut to [`HEAD_CHARS`]) as soon as it has
/// it or the pipe closes, then drains the rest so the child never blocks on a full pipe.
fn first_line<R: Read + Send + 'static>(pipe: R) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = io::BufReader::new(pipe);
        let mut line = Vec::new();
        let _ = Read::take(&mut reader, 4096).read_until(b'\n', &mut line);
        let text = String::from_utf8_lossy(&line);
        // The text is a child's, so untrusted: an escape sequence in a version line would be
        // interpreted by the terminal and the log viewer it reaches. Cleaned here, at the one
        // capture point, before anything else sees it.
        let first: String = text
            .trim_end_matches(['\r', '\n'])
            .chars()
            .take(HEAD_CHARS)
            .map(|c| {
                if c.is_control() {
                    CONTROL_PLACEHOLDER
                } else {
                    c
                }
            })
            .collect();
        let _ = tx.send(first);
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    rx
}

fn head(rx: mpsc::Receiver<String>) -> String {
    rx.recv_timeout(HEAD_WAIT).unwrap_or_default()
}

/// NOT A LINT — the guardrail `trash::dispose` has, for processes. A lib test that reaches the real
/// `SystemEnvironment` with one of the tools the check is about would run the developer's own
/// `proton-drive`, keyring or service manager. Tests use a fake `Environment`; the only real
/// programs they may start are `/bin/sh` and deliberately missing or non-executable paths.
#[cfg(test)]
fn refuse_real_tool(program: &Path) {
    const REAL: [&str; 4] = ["proton-drive", "secret-tool", "systemctl", "curl"];
    if let Some(name) = program.file_name().and_then(|name| name.to_str()) {
        assert!(
            !REAL.contains(&name),
            "a lib test asked SystemEnvironment to run the real tool `{name}`: use a fake \
             Environment"
        );
    }
}

// ---- the checks -------------------------------------------------------------------------------

const CLI_LIMIT: Duration = Duration::from_secs(10);
const SERVICE_LIMIT: Duration = Duration::from_secs(10);
/// Long enough for a person to answer the keyring's unlock prompt.
const LOOKUP_LIMIT: Duration = Duration::from_secs(60);

/// What a recommended problem says still works.
const SLOW_SYNC: &str = "sync still works, but every pass walks the whole Proton Drive folder instead of following \
     changes";

fn ok_result(id: CheckId, statement: String) -> CheckResult {
    CheckResult {
        id,
        severity: id.severity(),
        outcome: Outcome::Ok,
        statement,
        fix: None,
        links: Vec::new(),
    }
}

fn finding(
    id: CheckId,
    outcome: Outcome,
    statement: String,
    fix: String,
    extra: Vec<Link>,
) -> CheckResult {
    let mut links = vec![Link::docs(id)];
    links.extend(extra);
    CheckResult {
        id,
        severity: id.severity(),
        outcome,
        statement,
        fix: Some(fix),
        links,
    }
}

/// Runs the checks `inputs` selects, in report order.
pub fn run(inputs: &Inputs, env: &dyn Environment) -> Report {
    let wants = |id| inputs.checks.contains(&id);
    let needs_config = wants(CheckId::Config) || inputs.cli == CliSource::FromConfig;
    let config = needs_config.then(|| check_config(inputs, env));

    let mut checks = Vec::new();
    for id in CheckId::ALL.into_iter().filter(|id| wants(*id)) {
        checks.push(match id {
            CheckId::ProtonDrive => {
                let (cli, note) = choose_cli(inputs, config.as_ref());
                check_proton_drive(inputs, env, &cli, note)
            }
            CheckId::SignedIn => check_signed_in(env),
            CheckId::Config => config
                .as_ref()
                .expect("config is read whenever it is asked for")
                .result
                .clone(),
            CheckId::SecretTool => check_presence(env, CheckId::SecretTool, "secret-tool"),
            CheckId::Curl => check_presence(env, CheckId::Curl, "curl"),
            CheckId::SessionBus => check_session_bus(env),
            CheckId::Service => check_service(env),
        });
    }
    Report::new(checks)
}

// -- config (D6) --

struct ConfigEval {
    result: CheckResult,
    /// The file's `proton_cli`, when the file validated.
    proton_cli: Option<PathBuf>,
    /// The file was read and validated (or is the absent default).
    usable: bool,
}

fn check_config(inputs: &Inputs, env: &dyn Environment) -> ConfigEval {
    let (path, named) = match &inputs.config {
        ConfigFile::Named(path) => (path, true),
        ConfigFile::Default(path) => (path, false),
        ConfigFile::NoHome => {
            // Nothing is read, and the CLI is looked up on PATH: there is no file whose
            // `proton_cli` could be trusted.
            return ConfigEval {
                result: finding(
                    CheckId::Config,
                    Outcome::Unknown,
                    "No home directory, so the default config file cannot be located".to_owned(),
                    "Set HOME or XDG_CONFIG_HOME to an absolute path, or pass --config, then \
                     check again"
                        .to_owned(),
                    Vec::new(),
                ),
                proton_cli: None,
                usable: true,
            };
        }
    };
    let shown = path.display();
    let failed = |statement: String, fix: &str| ConfigEval {
        result: finding(
            CheckId::Config,
            Outcome::Failed,
            statement,
            fix.to_owned(),
            Vec::new(),
        ),
        proton_cli: None,
        usable: false,
    };
    let text = match env.read_file(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !named => {
            return ConfigEval {
                result: ok_result(
                    CheckId::Config,
                    format!("No config file yet (the default is {shown})"),
                ),
                proton_cli: None,
                usable: true,
            };
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return failed(
                format!("The config file {shown} does not exist"),
                "Check the path given to --config, or leave --config out to use the default file",
            );
        }
        Err(error) => {
            return failed(
                format!("The config file {shown} cannot be read: {error}"),
                "Make the file readable by this user, then check again",
            );
        }
    };
    if let Err(error) = config::validate_file_config_text(&text) {
        return failed(
            format!("The config file {shown} is not valid: {error}"),
            "Correct the setting named above, then check again",
        );
    }
    let proton_cli = config::parse_file_config(&text)
        .ok()
        .and_then(|file| file.proton_cli().map(Path::to_path_buf))
        .and_then(|cli| config::expand_tilde(cli, "proton_cli").ok());
    ConfigEval {
        result: ok_result(CheckId::Config, shown.to_string()),
        proton_cli,
        usable: true,
    }
}

/// `--proton-cli`, else a validated file's `proton_cli`, else `proton-drive`; with a note when the
/// file could not be read, so the statement says what was checked instead.
fn choose_cli(inputs: &Inputs, config: Option<&ConfigEval>) -> (PathBuf, Option<&'static str>) {
    match (&inputs.cli, config) {
        (CliSource::Explicit(path), _) => (path.clone(), None),
        (CliSource::FromConfig, Some(config)) if !config.usable => (
            PathBuf::from("proton-drive"),
            Some("checked proton-drive on PATH because the config file could not be read"),
        ),
        (CliSource::FromConfig, Some(config)) => (
            config
                .proton_cli
                .clone()
                .unwrap_or_else(|| PathBuf::from("proton-drive")),
            None,
        ),
        (CliSource::FromConfig, None) => (PathBuf::from("proton-drive"), None),
    }
}

// -- proton-drive (D3, and the lock probe of D12 / task 3.7) --

fn resolve_program(env: &dyn Environment, cli: &Path) -> Option<PathBuf> {
    if cli.as_os_str().as_bytes().contains(&b'/') {
        Some(std::path::absolute(cli).unwrap_or_else(|_| cli.to_path_buf()))
    } else {
        env.find_program(&cli.to_string_lossy())
    }
}

fn check_proton_drive(
    inputs: &Inputs,
    env: &dyn Environment,
    cli: &Path,
    note: Option<&str>,
) -> CheckResult {
    let id = CheckId::ProtonDrive;
    let say = |statement: String| match note {
        Some(note) => format!("{statement} ({note})"),
        None => statement,
    };
    let install = format!(
        "Install it from {} and then run `{}`. If it is installed off PATH, set `proton_cli` to \
         its full path",
        links::PROTON_DOWNLOAD,
        links::SIGN_IN_COMMAND
    );
    let reinstall = |what: &str| {
        format!(
            "{what} from {} and then check again",
            links::PROTON_DOWNLOAD
        )
    };
    let proton_links = || vec![Link::proton_download(), Link::proton_cli_docs()];

    // A relative path is resolved against the working directory of whoever runs it, and the
    // service's is not this process's: it would read `ok` from one directory and `missing` from
    // another. Refused outright; a bare name is a PATH lookup and stays one.
    if cli.as_os_str().as_bytes().contains(&b'/') && !cli.is_absolute() {
        return finding(
            id,
            Outcome::Failed,
            say(format!(
                "proton_cli is a relative path ({}); the service resolves it against its own \
                 working directory",
                cli.display()
            )),
            "Set proton_cli to an absolute path, then check again".to_owned(),
            Vec::new(),
        );
    }

    // Found by looking, not running: a CLI that is not there is missing whatever a daemon is doing.
    let Some(program) = resolve_program(env, cli) else {
        return finding(
            id,
            Outcome::Missing,
            say(format!("{} was not found on PATH", cli.display())),
            install,
            proton_links(),
        );
    };

    // `version` is not proven to leave the CLI's shared state alone (design A2), so a daemon that
    // holds the user-global lock gets no second client. `Unknown` is evidence of neither state:
    // like the dry run's warning it carries on and runs `version`, because refusing to check
    // would make `doctor` useless on a lockfile that merely cannot be opened.
    if let DaemonLock::Probe(lock) = &inputs.daemon_lock
        && matches!(env.probe_lock(lock), GlobalLockProbe::Held)
    {
        return finding(
            id,
            Outcome::Unknown,
            say("The daemon is running; its start-up log checks the CLI".to_owned()),
            "Read the proton-drive line of the daemon's start-up log, or stop the daemon and check \
             again"
                .to_owned(),
            proton_links(),
        );
    }

    let shown = program.display();
    let ran = {
        let _gate = inputs.gate.as_ref().map(|gate| gate.enter());
        env.run_cancellable(
            &program,
            &["version"],
            CLI_LIMIT,
            Stdout::Keep,
            inputs.cancel.as_deref(),
        )
    };
    match ran {
        Ran::Exited {
            code: 0,
            stdout_head,
            ..
        } => {
            let version = if stdout_head.is_empty() {
                "proton-drive (it printed no version)"
            } else {
                stdout_head.as_str()
            };
            ok_result(id, say(format!("{version} at {shown}")))
        }
        Ran::Exited {
            code, stderr_head, ..
        } => {
            let detail = if stderr_head.is_empty() {
                String::new()
            } else {
                format!(": {stderr_head}")
            };
            finding(
                id,
                Outcome::Failed,
                say(format!(
                    "{shown} was found but did not run correctly: `version` exited with status \
                     {code}{detail}"
                )),
                reinstall("Reinstall or update it"),
                proton_links(),
            )
        }
        Ran::NotFound => finding(
            id,
            Outcome::Missing,
            say(format!("No executable was found at {shown}")),
            install,
            proton_links(),
        ),
        Ran::NotExecutable if env.is_directory(&program) => finding(
            id,
            Outcome::Failed,
            say(format!(
                "{shown} is a directory, not the proton-drive program"
            )),
            "Point proton_cli at the program file, then check again".to_owned(),
            proton_links(),
        ),
        Ran::NotExecutable => finding(
            id,
            Outcome::Failed,
            say(format!("{shown} was found but is not executable")),
            reinstall("Reinstall it"),
            proton_links(),
        ),
        Ran::Cancelled => finding(
            id,
            Outcome::Unknown,
            say(format!(
                "{shown} was not asked to finish: the check was cancelled"
            )),
            "Check again".to_owned(),
            proton_links(),
        ),
        Ran::ExecFormat => finding(
            id,
            Outcome::Failed,
            say(format!("{shown} is not a program this system can run")),
            reinstall("Download the Linux x64 build"),
            proton_links(),
        ),
        Ran::Signalled(signal) if signal == libc::SIGILL => finding(
            id,
            Outcome::Failed,
            say(format!("{shown} crashed with an illegal instruction")),
            format!(
                "The CPU probably lacks AVX2: use the linux/x64-baseline build from {} and then \
                 check again",
                links::PROTON_DOWNLOAD
            ),
            proton_links(),
        ),
        Ran::Signalled(signal) => finding(
            id,
            Outcome::Failed,
            say(format!("{shown} crashed (signal {signal})")),
            reinstall("Reinstall it"),
            proton_links(),
        ),
        Ran::TimedOut => finding(
            id,
            Outcome::Unknown,
            say(format!(
                "{shown} did not answer `version` within {} s",
                CLI_LIMIT.as_secs()
            )),
            "Check again; `version` asks proton.me for the newest version, so it can be slow \
             offline"
                .to_owned(),
            proton_links(),
        ),
        Ran::SpawnError(kind) => finding(
            id,
            Outcome::Unknown,
            say(format!("{shown} could not be started: {kind}")),
            "Check that the file can be run by this user, then check again".to_owned(),
            proton_links(),
        ),
    }
}

// -- signed-in (D4) --

fn check_signed_in(env: &dyn Environment) -> CheckResult {
    let id = CheckId::SignedIn;
    let sign_in = links::SIGN_IN_COMMAND;
    let could_not = |statement: String, fix: String| {
        finding(
            id,
            Outcome::Unknown,
            statement,
            fix,
            vec![Link::proton_cli_docs()],
        )
    };

    if let Some(store) = env.var("PROTON_DRIVE_CREDENTIALS_STORE") {
        let store = store.to_string_lossy();
        if store == "pass" || store == "unsafe_file" {
            return could_not(
                format!(
                    "PROTON_DRIVE_CREDENTIALS_STORE is `{store}`, so the sign-in is not kept in \
                     the keyring and cannot be checked here; the daemon's change detection reads \
                     the sign-in only from the default keychain store"
                ),
                format!(
                    "To let the daemon follow changes, unset it and run `{sign_in}`; otherwise \
                     nothing is needed"
                ),
            );
        }
    }
    let Some(secret_tool) = env.find_program("secret-tool") else {
        return could_not(
            "secret-tool is not installed, so the keyring cannot be asked".to_owned(),
            "Install secret-tool (libsecret-tools on Debian and Ubuntu, libsecret on Fedora) and \
             check again"
                .to_owned(),
        );
    };
    if env
        .var("DBUS_SESSION_BUS_ADDRESS")
        .is_none_or(|address| address.is_empty())
    {
        return could_not(
            "No session bus in this shell, so the keyring cannot be asked".to_owned(),
            "Run this from your desktop session (an SSH shell has no session bus), then check \
             again"
                .to_owned(),
        );
    }
    // stdout is the session secret: it goes to /dev/null and never enters this process.
    let ran = env.run(
        &secret_tool,
        &keyring_lookup_args(),
        LOOKUP_LIMIT,
        Stdout::Discard,
    );
    match ran {
        Ran::Exited { code: 0, .. } => ok_result(id, "A sign-in is stored".to_owned()),
        Ran::Exited { stderr_head, .. } if stderr_head.is_empty() => finding(
            id,
            Outcome::Missing,
            "No proton-drive sign-in is stored in the keyring".to_owned(),
            format!(
                "Run `{sign_in}` and then check again. If you were asked to unlock the keyring \
                 and dismissed it, unlock it and check again"
            ),
            vec![Link::proton_cli_docs()],
        ),
        Ran::Exited { stderr_head, .. } => could_not(
            format!("The keyring could not be asked: {stderr_head}"),
            "Unlock the keyring or start the Secret Service, then check again".to_owned(),
        ),
        Ran::TimedOut => could_not(
            format!(
                "The keyring did not answer within {} s (an unlock prompt may be waiting)",
                LOOKUP_LIMIT.as_secs()
            ),
            "Answer the unlock prompt, then check again".to_owned(),
        ),
        other => could_not(
            format!(
                "The keyring lookup could not run ({})",
                describe_failure(&other)
            ),
            "Check that secret-tool works for this user, then check again".to_owned(),
        ),
    }
}

fn describe_failure(ran: &Ran) -> String {
    match ran {
        Ran::Signalled(signal) => format!("killed by signal {signal}"),
        Ran::NotFound => "not found".to_owned(),
        Ran::NotExecutable => "not executable".to_owned(),
        Ran::ExecFormat => "not a runnable program".to_owned(),
        Ran::SpawnError(kind) => kind.to_string(),
        Ran::TimedOut => "timed out".to_owned(),
        Ran::Cancelled => "cancelled".to_owned(),
        Ran::Exited { code, .. } => format!("exit status {code}"),
    }
}

// -- secret-tool, curl, session-bus, service (D5) --

fn check_presence(env: &dyn Environment, id: CheckId, name: &str) -> CheckResult {
    match env.find_program(name) {
        Some(path) => ok_result(id, format!("{name} at {}", path.display())),
        None => finding(
            id,
            Outcome::Missing,
            format!("{name} is not installed; {SLOW_SYNC}"),
            match id {
                CheckId::SecretTool => {
                    "Install secret-tool (libsecret-tools on Debian and Ubuntu, libsecret on \
                     Fedora)"
                }
                _ => "Install curl with your package manager",
            }
            .to_owned(),
            Vec::new(),
        ),
    }
}

fn check_session_bus(env: &dyn Environment) -> CheckResult {
    let id = CheckId::SessionBus;
    match env.var("DBUS_SESSION_BUS_ADDRESS") {
        Some(address) if !address.is_empty() => {
            ok_result(id, "A session bus address is set".to_owned())
        }
        _ => finding(
            id,
            Outcome::Missing,
            format!(
                "No session bus in this shell; without one in the daemon's environment, {SLOW_SYNC}"
            ),
            "Run this from your desktop session; a service started by systemd --user has one \
             already, so an SSH shell lacking it does not mean the service does"
                .to_owned(),
            Vec::new(),
        ),
    }
}

fn check_service(env: &dyn Environment) -> CheckResult {
    let id = CheckId::Service;
    let could_not = |statement: String| {
        finding(
            id,
            Outcome::Unknown,
            statement,
            "This only matters for the desktop app's Start button; check again from your desktop \
             session"
                .to_owned(),
            Vec::new(),
        )
    };
    let Some(systemctl) = env.find_program("systemctl") else {
        return could_not(
            "systemctl is not installed, so the service was not looked up".to_owned(),
        );
    };
    let ran = env.run(
        &systemctl,
        &[
            "--user",
            "show",
            "proton-syncd.service",
            "-p",
            "LoadState",
            "--value",
        ],
        SERVICE_LIMIT,
        Stdout::Keep,
    );
    match ran {
        Ran::Exited {
            code: 0,
            stdout_head,
            ..
        } => match stdout_head.trim() {
            "loaded" => ok_result(id, "proton-syncd.service is installed".to_owned()),
            "not-found" => finding(
                id,
                Outcome::Missing,
                "proton-syncd.service is not installed; the daemon can still run without it, but \
                 the desktop app's Start button needs it"
                    .to_owned(),
                "Install the user unit (setup.sh does, and so do the packages)".to_owned(),
                Vec::new(),
            ),
            "masked" => finding(
                id,
                Outcome::Failed,
                "proton-syncd.service is masked, so it cannot be started".to_owned(),
                "Run `systemctl --user unmask proton-syncd.service`".to_owned(),
                Vec::new(),
            ),
            other => could_not(format!(
                "systemctl reported the service as `{other}`, which this check does not know"
            )),
        },
        Ran::Exited { stderr_head, .. } => could_not(format!(
            "systemctl could not look the service up: {stderr_head}"
        )),
        other => could_not(format!(
            "systemctl could not look the service up ({})",
            describe_failure(&other)
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::fs;

    use super::*;

    // ---- 3.1: SystemEnvironment against /bin/sh --------------------------------------------

    fn sh(script: &str, limit: Duration, stdout: Stdout) -> Ran {
        SystemEnvironment.run(Path::new("/bin/sh"), &["-c", script], limit, stdout)
    }

    #[test]
    fn an_exit_status_and_the_first_line_of_each_pipe() {
        let ran = sh(
            "echo out1; echo out2; echo err1 >&2; echo err2 >&2; exit 3",
            Duration::from_secs(10),
            Stdout::Keep,
        );
        assert_eq!(
            ran,
            Ran::Exited {
                code: 3,
                stdout_head: "out1".to_owned(),
                stderr_head: "err1".to_owned(),
            }
        );
    }

    #[test]
    fn a_long_first_line_is_cut_to_200_characters() {
        let Ran::Exited { stdout_head, .. } = sh(
            "printf '%0500d\\n' 7",
            Duration::from_secs(10),
            Stdout::Keep,
        ) else {
            panic!("expected an exit");
        };
        assert_eq!(stdout_head.chars().count(), 200);
    }

    #[test]
    fn a_signal_is_reported_as_a_signal() {
        assert_eq!(
            sh("kill -s ILL $$", Duration::from_secs(10), Stdout::Keep),
            Ran::Signalled(libc::SIGILL)
        );
    }

    #[test]
    fn a_child_past_its_limit_is_killed_and_reaped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("pid");
        let started = std::time::Instant::now();
        let ran = sh(
            &format!("echo $$ > {}; sleep 5", pid_file.display()),
            Duration::from_millis(100),
            Stdout::Keep,
        );
        assert_eq!(ran, Ran::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(4));
        let pid: i32 = fs::read_to_string(&pid_file)
            .expect("the script wrote its pid")
            .trim()
            .parse()
            .expect("a pid");
        // Reaped: a zombie would still answer signal 0.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    fn a_raised_cancel_flag_kills_and_reaps_the_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("pid");
        let flag = Arc::new(AtomicBool::new(false));
        let raiser = {
            let flag = Arc::clone(&flag);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                flag.store(true, Ordering::SeqCst);
            })
        };
        let started = std::time::Instant::now();
        let ran = SystemEnvironment.run_cancellable(
            Path::new("/bin/sh"),
            &[
                "-c",
                &format!("echo $$ > {}; exec sleep 20", pid_file.display()),
            ],
            Duration::from_secs(60),
            Stdout::Keep,
            Some(&flag),
        );
        raiser.join().expect("raiser");
        assert_eq!(ran, Ran::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid: i32 = fs::read_to_string(&pid_file)
            .expect("the script wrote its pid")
            .trim()
            .parse()
            .expect("a pid");
        // Reaped: a zombie would still answer signal 0.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    fn a_flag_never_raised_changes_nothing() {
        let flag = AtomicBool::new(false);
        let ran = SystemEnvironment.run_cancellable(
            Path::new("/bin/sh"),
            &["-c", "echo hi"],
            Duration::from_secs(10),
            Stdout::Keep,
            Some(&flag),
        );
        assert_eq!(
            ran,
            Ran::Exited {
                code: 0,
                stdout_head: "hi".to_owned(),
                stderr_head: String::new(),
            }
        );
    }

    #[test]
    fn control_characters_in_a_childs_output_never_leave_the_capture() {
        // An escape sequence in a version line would reach the terminal, the log and the app.
        let Ran::Exited {
            stdout_head,
            stderr_head,
            ..
        } = sh(
            "printf '\\033[31mv1.2\\033[0m\\001\\177\\n'; printf 'bad\\033]0;x\\007\\n' >&2",
            Duration::from_secs(10),
            Stdout::Keep,
        )
        else {
            panic!("expected an exit");
        };
        for text in [&stdout_head, &stderr_head] {
            assert!(!text.chars().any(char::is_control), "{text:?}");
        }
        assert!(stdout_head.contains("v1.2"), "{stdout_head:?}");
        assert!(stderr_head.starts_with("bad"), "{stderr_head:?}");
    }

    #[test]
    fn discard_leaves_stdout_empty() {
        let Ran::Exited { stdout_head, .. } =
            sh("echo secret", Duration::from_secs(10), Stdout::Discard)
        else {
            panic!("expected an exit");
        };
        assert_eq!(stdout_head, "");
    }

    #[test]
    fn a_missing_program_is_not_found() {
        let ran = SystemEnvironment.run(
            Path::new("/nonexistent-dir-for-doctor-tests/prog"),
            &[],
            Duration::from_secs(1),
            Stdout::Keep,
        );
        assert_eq!(ran, Ran::NotFound);
    }

    #[test]
    fn a_non_executable_file_is_not_executable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("prog");
        fs::write(&file, "#!/bin/sh\n").expect("write");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("chmod");
        let ran = SystemEnvironment.run(&file, &[], Duration::from_secs(1), Stdout::Keep);
        assert_eq!(ran, Ran::NotExecutable);
    }

    #[test]
    fn the_real_environment_refuses_to_run_the_tools_the_check_is_about() {
        for name in ["proton-drive", "secret-tool", "systemctl", "curl"] {
            let outcome = std::panic::catch_unwind(|| {
                SystemEnvironment.run(
                    &Path::new("/nonexistent-dir-for-doctor-tests").join(name),
                    &[],
                    Duration::from_secs(1),
                    Stdout::Keep,
                )
            });
            assert!(outcome.is_err(), "{name} must be refused");
        }
    }

    #[test]
    fn finding_a_program_searches_path_without_running_it() {
        // `sh` is on every PATH the tests run under; the answer is absolute.
        let found = SystemEnvironment.find_program("sh").expect("sh on PATH");
        assert!(found.is_absolute());
        assert_eq!(
            SystemEnvironment.find_program("no-such-program-doctor"),
            None
        );
    }

    // ---- the fake environment --------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct Call {
        program: PathBuf,
        args: Vec<String>,
        stdout: Stdout,
        limit: Duration,
    }

    enum FakeLock {
        Held,
        Free,
        Unknown,
    }

    #[derive(Default)]
    struct Fake {
        programs: HashMap<&'static str, PathBuf>,
        vars: HashMap<&'static str, OsString>,
        files: HashMap<PathBuf, Result<String, io::ErrorKind>>,
        lock: Option<FakeLock>,
        answers: HashMap<&'static str, Ran>,
        calls: RefCell<Vec<Call>>,
        lock_probes: RefCell<Vec<PathBuf>>,
        during_run: Option<Box<dyn Fn()>>,
        directories: Vec<PathBuf>,
        reads: RefCell<Vec<PathBuf>>,
        /// For each `run_cancellable` call: was a flag handed in, and was it raised.
        cancels: RefCell<Vec<Option<bool>>>,
    }

    impl Environment for Fake {
        fn find_program(&self, name: &str) -> Option<PathBuf> {
            self.programs.get(name).cloned()
        }
        fn var(&self, name: &str) -> Option<OsString> {
            self.vars.get(name).cloned()
        }
        fn read_file(&self, path: &Path) -> io::Result<String> {
            self.reads.borrow_mut().push(path.to_path_buf());
            match self.files.get(path) {
                Some(Ok(text)) => Ok(text.clone()),
                Some(Err(kind)) => Err(io::Error::from(*kind)),
                None => Err(io::Error::from(io::ErrorKind::NotFound)),
            }
        }
        fn probe_lock(&self, path: &Path) -> GlobalLockProbe {
            self.lock_probes.borrow_mut().push(path.to_path_buf());
            match self.lock {
                Some(FakeLock::Held) => GlobalLockProbe::Held,
                Some(FakeLock::Unknown) => GlobalLockProbe::Unknown("fake".to_owned()),
                Some(FakeLock::Free) | None => GlobalLockProbe::Free,
            }
        }
        fn is_directory(&self, path: &Path) -> bool {
            self.directories.iter().any(|d| d == path)
        }
        fn run_cancellable(
            &self,
            program: &Path,
            args: &[&str],
            limit: Duration,
            stdout: Stdout,
            cancel: Option<&AtomicBool>,
        ) -> Ran {
            self.cancels
                .borrow_mut()
                .push(cancel.map(|flag| flag.load(Ordering::SeqCst)));
            self.run(program, args, limit, stdout)
        }
        fn run(&self, program: &Path, args: &[&str], limit: Duration, stdout: Stdout) -> Ran {
            self.calls.borrow_mut().push(Call {
                program: program.to_path_buf(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
                stdout,
                limit,
            });
            if let Some(hook) = &self.during_run {
                hook();
            }
            let name = program.file_name().and_then(|n| n.to_str()).expect("name");
            self.answers
                .get(name)
                .unwrap_or_else(|| panic!("the test did not expect `{name}` to be run"))
                .clone()
        }
    }

    fn exited(code: i32, stdout: &str, stderr: &str) -> Ran {
        Ran::Exited {
            code,
            stdout_head: stdout.to_owned(),
            stderr_head: stderr.to_owned(),
        }
    }

    /// Everything present and answering well.
    fn healthy() -> Fake {
        let mut fake = Fake::default();
        for name in ["proton-drive", "secret-tool", "curl", "systemctl"] {
            fake.programs
                .insert(name, PathBuf::from("/usr/bin").join(name));
        }
        fake.vars.insert(
            "DBUS_SESSION_BUS_ADDRESS",
            OsString::from("unix:path=/run/user/1000/bus"),
        );
        fake.answers
            .insert("proton-drive", exited(0, "proton-drive 1.4.2", ""));
        fake.answers.insert("secret-tool", exited(0, "", ""));
        fake.answers.insert("systemctl", exited(0, "loaded", ""));
        fake
    }

    const CONFIG: &str = "/cfg/proton-sync.toml";

    fn inputs() -> Inputs {
        let mut inputs = Inputs::new(ConfigFile::Default(PathBuf::from(CONFIG)));
        inputs.daemon_lock = DaemonLock::Skip;
        inputs
    }

    fn only(id: CheckId, inputs: &mut Inputs) {
        inputs.checks = vec![id];
    }

    fn result(fake: &Fake, mut inputs: Inputs, id: CheckId) -> CheckResult {
        only(id, &mut inputs);
        run(&inputs, fake).checks.remove(0)
    }

    fn calls_to(fake: &Fake, name: &str) -> Vec<Call> {
        fake.calls
            .borrow()
            .iter()
            .filter(|c| c.program.file_name().is_some_and(|n| n == name))
            .cloned()
            .collect()
    }

    // ---- ids and severities ----------------------------------------------------------------

    #[test]
    fn ids_serialize_as_their_wire_tokens_and_each_has_an_anchor() {
        for id in CheckId::ALL {
            assert_eq!(
                serde_json::to_value(id).expect("json"),
                serde_json::Value::String(id.as_str().to_owned())
            );
            assert!(links::docs_url(id.as_str()).is_some(), "{id:?}");
        }
        let required: Vec<_> = CheckId::ALL
            .iter()
            .filter(|id| id.severity() == Severity::Required)
            .map(|id| id.as_str())
            .collect();
        assert_eq!(required, ["proton-drive", "signed-in", "config"]);
    }

    // ---- 3.2: proton-drive -----------------------------------------------------------------

    fn drive(fake: &Fake) -> CheckResult {
        result(fake, inputs(), CheckId::ProtonDrive)
    }

    #[test]
    fn the_cli_runs_only_version_with_a_ten_second_limit() {
        let fake = healthy();
        drive(&fake);
        let calls = calls_to(&fake, "proton-drive");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args, ["version"]);
        assert_eq!(calls[0].limit, Duration::from_secs(10));
        assert_eq!(calls[0].program, PathBuf::from("/usr/bin/proton-drive"));
    }

    #[test]
    fn a_working_cli_is_ok_with_its_version_and_absolute_path() {
        let found = drive(&healthy());
        assert_eq!(found.outcome, Outcome::Ok);
        assert_eq!(
            found.statement,
            "proton-drive 1.4.2 at /usr/bin/proton-drive"
        );
        assert!(found.fix.is_none() && found.links.is_empty());
    }

    #[test]
    fn a_cli_not_on_path_is_missing_and_nothing_is_run() {
        let mut fake = healthy();
        fake.programs.remove("proton-drive");
        let found = drive(&fake);
        assert_eq!(found.outcome, Outcome::Missing);
        assert!(
            found
                .fix
                .as_deref()
                .unwrap()
                .contains(links::PROTON_DOWNLOAD)
        );
        assert!(
            found
                .fix
                .as_deref()
                .unwrap()
                .contains(links::SIGN_IN_COMMAND)
        );
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn a_cli_that_is_not_at_its_path_is_missing() {
        let mut fake = healthy();
        fake.answers.insert("proton-drive", Ran::NotFound);
        assert_eq!(drive(&fake).outcome, Outcome::Missing);
    }

    #[test]
    fn a_non_zero_exit_is_failed_and_not_missing() {
        let mut fake = healthy();
        fake.answers
            .insert("proton-drive", exited(2, "", "boom: bad state"));
        let found = drive(&fake);
        assert_eq!(found.outcome, Outcome::Failed);
        assert!(found.statement.contains("status 2"), "{}", found.statement);
        assert!(found.statement.contains("boom: bad state"));
        assert!(!found.statement.contains("not installed"));
    }

    #[test]
    fn an_illegal_instruction_names_the_baseline_build() {
        let mut fake = healthy();
        fake.answers
            .insert("proton-drive", Ran::Signalled(libc::SIGILL));
        let found = drive(&fake);
        assert_eq!(found.outcome, Outcome::Failed);
        assert!(found.fix.as_deref().unwrap().contains("linux/x64-baseline"));
    }

    #[test]
    fn the_other_proton_drive_endings_each_have_their_own_row() {
        let cases = [
            (Ran::Signalled(libc::SIGSEGV), Outcome::Failed, "signal 11"),
            (Ran::NotExecutable, Outcome::Failed, "not executable"),
            (Ran::ExecFormat, Outcome::Failed, "not a program"),
            (Ran::TimedOut, Outcome::Unknown, "did not answer"),
            (
                Ran::SpawnError(io::ErrorKind::OutOfMemory),
                Outcome::Unknown,
                "could not be started",
            ),
        ];
        for (ran, outcome, words) in cases {
            let mut fake = healthy();
            fake.answers.insert("proton-drive", ran.clone());
            let found = drive(&fake);
            assert_eq!(found.outcome, outcome, "{ran:?}");
            assert!(
                found.statement.contains(words),
                "{ran:?}: {}",
                found.statement
            );
            assert!(found.fix.is_some() && !found.links.is_empty());
        }
    }

    #[test]
    fn a_path_with_a_slash_is_used_as_it_is() {
        let mut fake = healthy();
        fake.programs.remove("proton-drive");
        let mut inputs = inputs();
        inputs.cli = CliSource::Explicit(PathBuf::from("/opt/proton/proton-drive"));
        let found = result(&fake, inputs, CheckId::ProtonDrive);
        assert_eq!(found.outcome, Outcome::Ok);
        assert_eq!(
            calls_to(&fake, "proton-drive")[0].program,
            PathBuf::from("/opt/proton/proton-drive")
        );
    }

    #[test]
    fn a_relative_path_is_failed_whichever_directory_it_is_read_from() {
        // `ok` from one directory and `missing` from another is no answer; the service resolves it
        // against its own directory in any case.
        let fake = healthy();
        let mut explicit = inputs();
        explicit.cli = CliSource::Explicit(PathBuf::from("bin/proton-drive"));
        let found = result(&fake, explicit, CheckId::ProtonDrive);
        assert_eq!(found.outcome, Outcome::Failed);
        assert_eq!(
            found.statement,
            "proton_cli is a relative path (bin/proton-drive); the service resolves it against \
             its own working directory"
        );
        assert_eq!(
            found.fix.as_deref(),
            Some("Set proton_cli to an absolute path, then check again")
        );
        assert!(fake.calls.borrow().is_empty(), "nothing is run");

        // The same from a config file's value, and for a `./` spelling.
        for value in ["../proton-drive", "./proton-drive"] {
            let mut fake = healthy();
            fake.files.insert(
                PathBuf::from(CONFIG),
                Ok(format!("proton_cli = \"{value}\"\n")),
            );
            let found = drive(&fake);
            assert_eq!(found.outcome, Outcome::Failed, "{value}");
            assert!(found.statement.contains("relative path"), "{value}");
            assert!(fake.calls.borrow().is_empty(), "{value}");
        }
    }

    #[test]
    fn a_directory_is_not_called_unexecutable() {
        let mut fake = healthy();
        fake.answers.insert("proton-drive", Ran::NotExecutable);
        fake.directories
            .push(PathBuf::from("/opt/proton/proton-drive"));
        let mut inputs = inputs();
        inputs.cli = CliSource::Explicit(PathBuf::from("/opt/proton/proton-drive"));
        let found = result(&fake, inputs, CheckId::ProtonDrive);
        assert_eq!(found.outcome, Outcome::Failed);
        assert_eq!(
            found.statement,
            "/opt/proton/proton-drive is a directory, not the proton-drive program"
        );
        assert_eq!(
            found.fix.as_deref(),
            Some("Point proton_cli at the program file, then check again")
        );
    }

    #[test]
    fn with_no_home_no_config_is_read_and_the_cli_is_looked_up_on_path() {
        let mut fake = healthy();
        // A file at the temp-directory spot would name another program; it must never be read.
        fake.files.insert(
            PathBuf::from("/tmp/proton-sync/proton-sync.toml"),
            Ok("proton_cli = \"/tmp/planted/proton-drive\"\n".to_owned()),
        );
        let mut inputs = inputs();
        inputs.config = ConfigFile::default_at(None);
        let report = run(&inputs, &fake);
        assert!(fake.reads.borrow().is_empty(), "{:?}", fake.reads.borrow());
        let config = &report.checks[2];
        assert_eq!(config.id, CheckId::Config);
        assert_eq!(config.outcome, Outcome::Unknown);
        assert_eq!(
            config.statement,
            "No home directory, so the default config file cannot be located"
        );
        let cli = &report.checks[0];
        assert_eq!(cli.outcome, Outcome::Ok);
        assert!(
            cli.statement.ends_with("at /usr/bin/proton-drive"),
            "{}",
            cli.statement
        );
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn the_cancel_flag_reaches_the_version_run() {
        let fake = healthy();
        let flag = Arc::new(AtomicBool::new(true));
        let mut with = inputs();
        with.cancel = Some(Arc::clone(&flag));
        result(&fake, with, CheckId::ProtonDrive);
        assert_eq!(*fake.cancels.borrow(), vec![Some(true)]);

        let fake = healthy();
        drive(&fake);
        assert_eq!(*fake.cancels.borrow(), vec![None]);
    }

    #[test]
    fn a_cancelled_run_is_unknown_and_says_so() {
        let mut fake = healthy();
        fake.answers.insert("proton-drive", Ran::Cancelled);
        let found = drive(&fake);
        assert_eq!(found.outcome, Outcome::Unknown);
        assert!(found.statement.contains("cancelled"), "{}", found.statement);
    }

    // ---- 3.7: the lock probe and the gate --------------------------------------------------

    fn with_lock(lock: Option<FakeLock>) -> (Fake, Inputs) {
        let mut fake = healthy();
        fake.lock = lock;
        let mut inputs = inputs();
        inputs.daemon_lock = DaemonLock::Probe(PathBuf::from("/state/single-instance.lock"));
        (fake, inputs)
    }

    #[test]
    fn a_daemon_holding_the_lock_reports_unknown_and_nothing_is_run() {
        let (fake, inputs) = with_lock(Some(FakeLock::Held));
        let found = result(&fake, inputs, CheckId::ProtonDrive);
        assert_eq!(found.outcome, Outcome::Unknown);
        assert!(
            found
                .statement
                .contains("The daemon is running; its start-up log checks the CLI")
        );
        assert!(found.fix.is_some() && !found.links.is_empty());
        assert!(fake.calls.borrow().is_empty(), "nothing may be spawned");
        assert_eq!(
            *fake.lock_probes.borrow(),
            [PathBuf::from("/state/single-instance.lock")]
        );
    }

    #[test]
    fn a_free_lock_runs_version() {
        let (fake, inputs) = with_lock(Some(FakeLock::Free));
        assert_eq!(
            result(&fake, inputs, CheckId::ProtonDrive).outcome,
            Outcome::Ok
        );
        assert_eq!(calls_to(&fake, "proton-drive").len(), 1);
    }

    #[test]
    fn a_lock_that_cannot_be_looked_at_is_evidence_of_neither_so_version_runs() {
        let (fake, inputs) = with_lock(Some(FakeLock::Unknown));
        assert_eq!(
            result(&fake, inputs, CheckId::ProtonDrive).outcome,
            Outcome::Ok
        );
        assert_eq!(calls_to(&fake, "proton-drive").len(), 1);
    }

    #[test]
    fn skipping_the_probe_never_looks_at_the_lock() {
        let (mut fake, mut inputs) = with_lock(Some(FakeLock::Held));
        inputs.daemon_lock = DaemonLock::Skip;
        fake.lock = Some(FakeLock::Held);
        assert_eq!(
            result(&fake, inputs, CheckId::ProtonDrive).outcome,
            Outcome::Ok
        );
        assert!(fake.lock_probes.borrow().is_empty());
    }

    #[test]
    fn a_cli_that_is_not_there_is_missing_even_beside_a_daemon() {
        let (mut fake, inputs) = with_lock(Some(FakeLock::Held));
        fake.programs.remove("proton-drive");
        assert_eq!(
            result(&fake, inputs, CheckId::ProtonDrive).outcome,
            Outcome::Missing
        );
    }

    #[test]
    fn version_runs_while_the_gate_is_held_and_not_after() {
        let gate = Arc::new(CliGate::new());
        let seen_busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (mut fake, mut inputs) = with_lock(None);
        inputs.daemon_lock = DaemonLock::Skip;
        inputs.gate = Some(Arc::clone(&gate));
        {
            let gate = Arc::clone(&gate);
            let seen_busy = Arc::clone(&seen_busy);
            fake.during_run = Some(Box::new(move || {
                let busy = gate.enter_within(Duration::from_millis(1)).is_none();
                seen_busy.store(busy, std::sync::atomic::Ordering::SeqCst);
            }));
        }
        result(&fake, inputs, CheckId::ProtonDrive);
        assert!(seen_busy.load(std::sync::atomic::Ordering::SeqCst));
        assert!(gate.enter_within(Duration::from_millis(1)).is_some());
    }

    // ---- 3.3: signed-in --------------------------------------------------------------------

    fn signed_in(fake: &Fake) -> CheckResult {
        result(fake, inputs(), CheckId::SignedIn)
    }

    #[test]
    fn a_stored_sign_in_is_ok_and_the_lookup_discards_stdout() {
        let fake = healthy();
        let found = signed_in(&fake);
        assert_eq!(found.outcome, Outcome::Ok);
        assert_eq!(found.statement, "A sign-in is stored");
        let calls = calls_to(&fake, "secret-tool");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].stdout, Stdout::Discard);
        assert_eq!(calls[0].args, keyring_lookup_args());
        assert_eq!(calls[0].limit, Duration::from_secs(60));
    }

    #[test]
    fn credentials_kept_outside_the_keyring_are_unknown_without_a_lookup() {
        for store in ["pass", "unsafe_file"] {
            let mut fake = healthy();
            fake.vars
                .insert("PROTON_DRIVE_CREDENTIALS_STORE", OsString::from(store));
            let found = signed_in(&fake);
            assert_eq!(found.outcome, Outcome::Unknown);
            assert!(found.statement.contains("default keychain store"));
            assert!(fake.calls.borrow().is_empty());
        }
    }

    #[test]
    fn another_store_value_still_asks_the_keyring() {
        let mut fake = healthy();
        fake.vars
            .insert("PROTON_DRIVE_CREDENTIALS_STORE", OsString::from("keychain"));
        assert_eq!(signed_in(&fake).outcome, Outcome::Ok);
    }

    #[test]
    fn without_secret_tool_it_is_unknown_and_nothing_is_run() {
        let mut fake = healthy();
        fake.programs.remove("secret-tool");
        let found = signed_in(&fake);
        assert_eq!(found.outcome, Outcome::Unknown);
        assert!(found.statement.contains("secret-tool is not installed"));
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn without_a_session_bus_the_lookup_is_not_run() {
        for address in [None, Some("")] {
            let mut fake = healthy();
            fake.vars.remove("DBUS_SESSION_BUS_ADDRESS");
            if let Some(address) = address {
                fake.vars
                    .insert("DBUS_SESSION_BUS_ADDRESS", OsString::from(address));
            }
            let found = signed_in(&fake);
            assert_eq!(found.outcome, Outcome::Unknown);
            assert!(found.statement.contains("No session bus"));
            assert!(fake.calls.borrow().is_empty());
        }
    }

    #[test]
    fn a_lookup_with_a_stderr_line_is_unknown_quoting_it() {
        let mut fake = healthy();
        fake.answers.insert(
            "secret-tool",
            exited(1, "", "secret-tool: Cannot autolaunch D-Bus"),
        );
        let found = signed_in(&fake);
        assert_eq!(found.outcome, Outcome::Unknown);
        assert!(found.statement.contains("Cannot autolaunch D-Bus"));
    }

    #[test]
    fn a_silent_failure_is_missing() {
        let mut fake = healthy();
        fake.answers.insert("secret-tool", exited(1, "", ""));
        let found = signed_in(&fake);
        assert_eq!(found.outcome, Outcome::Missing);
        assert!(
            found
                .fix
                .as_deref()
                .unwrap()
                .contains(links::SIGN_IN_COMMAND)
        );
    }

    #[test]
    fn a_timeout_mentions_the_unlock_prompt() {
        let mut fake = healthy();
        fake.answers.insert("secret-tool", Ran::TimedOut);
        let found = signed_in(&fake);
        assert_eq!(found.outcome, Outcome::Unknown);
        assert!(found.statement.contains("unlock prompt"));
    }

    // ---- 3.4: secret-tool, curl, session-bus, service --------------------------------------

    #[test]
    fn a_missing_recommended_tool_says_sync_still_works() {
        for (id, name) in [
            (CheckId::SecretTool, "secret-tool"),
            (CheckId::Curl, "curl"),
        ] {
            let present = result(&healthy(), inputs(), id);
            assert_eq!(present.outcome, Outcome::Ok);
            assert_eq!(present.statement, format!("{name} at /usr/bin/{name}"));

            let mut fake = healthy();
            fake.programs.remove(name);
            let found = result(&fake, inputs(), id);
            assert_eq!(found.outcome, Outcome::Missing);
            assert_eq!(found.severity, Severity::Recommended);
            assert!(
                found.statement.contains(
                    "sync still works, but every pass walks the whole Proton Drive folder"
                ),
                "{}",
                found.statement
            );
            assert!(fake.calls.borrow().is_empty(), "presence is a PATH search");
        }
    }

    #[test]
    fn the_session_bus_is_environment_only() {
        let fake = healthy();
        assert_eq!(
            result(&fake, inputs(), CheckId::SessionBus).outcome,
            Outcome::Ok
        );
        for address in [None, Some("")] {
            let mut fake = healthy();
            fake.vars.remove("DBUS_SESSION_BUS_ADDRESS");
            if let Some(address) = address {
                fake.vars
                    .insert("DBUS_SESSION_BUS_ADDRESS", OsString::from(address));
            }
            let found = result(&fake, inputs(), CheckId::SessionBus);
            assert_eq!(found.outcome, Outcome::Missing);
            assert!(found.statement.contains("sync still works"));
        }
    }

    #[test]
    fn the_service_is_judged_by_its_load_state() {
        let cases = [
            ("loaded", Outcome::Ok),
            ("not-found", Outcome::Missing),
            ("masked", Outcome::Failed),
            ("bad-setting", Outcome::Unknown),
        ];
        for (value, outcome) in cases {
            let mut fake = healthy();
            fake.answers.insert("systemctl", exited(0, value, ""));
            assert_eq!(
                result(&fake, inputs(), CheckId::Service).outcome,
                outcome,
                "{value}"
            );
            let calls = calls_to(&fake, "systemctl");
            assert_eq!(
                calls[0].args,
                [
                    "--user",
                    "show",
                    "proton-syncd.service",
                    "-p",
                    "LoadState",
                    "--value"
                ]
            );
            assert_eq!(calls[0].limit, Duration::from_secs(10));
        }
    }

    #[test]
    fn a_service_check_that_cannot_look_is_unknown() {
        let mut fake = healthy();
        fake.programs.remove("systemctl");
        let found = result(&fake, inputs(), CheckId::Service);
        assert_eq!(found.outcome, Outcome::Unknown);
        assert!(fake.calls.borrow().is_empty());

        for ran in [exited(1, "", "Failed to connect to bus"), Ran::TimedOut] {
            let mut fake = healthy();
            fake.answers.insert("systemctl", ran);
            assert_eq!(
                result(&fake, inputs(), CheckId::Service).outcome,
                Outcome::Unknown
            );
        }
    }

    // ---- 3.5: config and the choice of CLI -------------------------------------------------

    const VALID_WITH_CLI: &str = "local_root = \"/tmp/a\"\nremote_root = \"/Drive/a\"\n\
                                  proton_cli = \"/opt/from-file/proton-drive\"\n";

    fn with_file(text: &str) -> Fake {
        let mut fake = healthy();
        fake.programs.remove("proton-drive");
        fake.files
            .insert(PathBuf::from(CONFIG), Ok(text.to_owned()));
        fake
    }

    #[test]
    fn an_absent_default_file_is_ok_and_says_so() {
        let found = result(&healthy(), inputs(), CheckId::Config);
        assert_eq!(found.outcome, Outcome::Ok);
        assert!(found.statement.contains("No config file yet"));
    }

    #[test]
    fn an_absent_named_file_is_failed_naming_the_path() {
        let mut inputs = inputs();
        inputs.config = ConfigFile::Named(PathBuf::from("/named/x.toml"));
        let found = result(&healthy(), inputs, CheckId::Config);
        assert_eq!(found.outcome, Outcome::Failed);
        assert!(found.statement.contains("/named/x.toml"));
    }

    #[test]
    fn an_unreadable_file_is_failed() {
        let mut fake = healthy();
        fake.files
            .insert(PathBuf::from(CONFIG), Err(io::ErrorKind::PermissionDenied));
        let found = result(&fake, inputs(), CheckId::Config);
        assert_eq!(found.outcome, Outcome::Failed);
        assert!(found.statement.contains("cannot be read"));
    }

    #[test]
    fn a_valid_file_is_ok_and_its_proton_cli_is_the_one_run() {
        let fake = with_file(VALID_WITH_CLI);
        let report = run(&inputs(), &fake);
        let config = report
            .checks
            .iter()
            .find(|c| c.id == CheckId::Config)
            .unwrap();
        assert_eq!(config.outcome, Outcome::Ok, "{}", config.statement);
        assert_eq!(
            calls_to(&fake, "proton-drive")[0].program,
            PathBuf::from("/opt/from-file/proton-drive")
        );
    }

    #[test]
    fn the_flag_beats_the_file() {
        let fake = with_file(VALID_WITH_CLI);
        let mut inputs = inputs();
        inputs.cli = CliSource::Explicit(PathBuf::from("/opt/flag/proton-drive"));
        run(&inputs, &fake);
        assert_eq!(
            calls_to(&fake, "proton-drive")[0].program,
            PathBuf::from("/opt/flag/proton-drive")
        );
    }

    #[test]
    fn an_invalid_file_fails_quoting_the_validator_and_the_cli_check_says_what_it_ran() {
        let mut fake = healthy();
        fake.files
            .insert(PathBuf::from(CONFIG), Ok("this is = = not toml".to_owned()));
        let report = run(&inputs(), &fake);
        let config = report
            .checks
            .iter()
            .find(|c| c.id == CheckId::Config)
            .unwrap();
        assert_eq!(config.outcome, Outcome::Failed);
        let validator = config::validate_file_config_text("this is = = not toml")
            .expect_err("invalid")
            .to_string();
        assert!(
            config.statement.contains(&validator),
            "{}",
            config.statement
        );

        let cli = report
            .checks
            .iter()
            .find(|c| c.id == CheckId::ProtonDrive)
            .unwrap();
        assert!(cli.statement.contains("checked proton-drive on PATH"));
        assert_eq!(
            calls_to(&fake, "proton-drive")[0].program,
            PathBuf::from("/usr/bin/proton-drive")
        );
        assert_eq!(report.verdict, Verdict::Failed);
    }

    // ---- 3.6: verdict, rendering, JSON -----------------------------------------------------

    fn result_with(id: CheckId, outcome: Outcome) -> CheckResult {
        CheckResult {
            id,
            severity: id.severity(),
            outcome,
            statement: String::new(),
            fix: None,
            links: Vec::new(),
        }
    }

    #[test]
    fn the_verdict_table() {
        use CheckId::*;
        use Outcome::*;
        let cases = [
            (
                vec![(ProtonDrive, Ok), (SignedIn, Ok), (Config, Ok)],
                Verdict::Ok,
            ),
            (vec![(ProtonDrive, Missing)], Verdict::Failed),
            (vec![(Config, Failed), (SignedIn, Unknown)], Verdict::Failed),
            (
                vec![(SignedIn, Unknown), (ProtonDrive, Ok)],
                Verdict::Unknown,
            ),
            // Recommended results never move it.
            (
                vec![(Curl, Missing), (SecretTool, Failed), (Service, Unknown)],
                Verdict::Ok,
            ),
            (vec![(Curl, Missing), (SignedIn, Unknown)], Verdict::Unknown),
        ];
        for (rows, verdict) in cases {
            let checks = rows.into_iter().map(|(id, o)| result_with(id, o)).collect();
            assert_eq!(Report::new(checks).verdict, verdict);
        }
    }

    /// A report where every check is a problem of some kind.
    fn bad_fake() -> Fake {
        let mut fake = Fake::default();
        fake.files
            .insert(PathBuf::from(CONFIG), Ok("this is = = not toml".to_owned()));
        fake
    }

    fn reports() -> Vec<Report> {
        let mut crashed = healthy();
        crashed
            .answers
            .insert("proton-drive", Ran::Signalled(libc::SIGILL));
        let mut slow = healthy();
        slow.answers.insert("proton-drive", Ran::TimedOut);
        slow.answers.insert("secret-tool", Ran::TimedOut);
        slow.answers.insert("systemctl", exited(0, "not-found", ""));
        let mut failing = healthy();
        failing.answers.insert("proton-drive", exited(7, "", "bad"));
        failing.answers.insert("secret-tool", exited(1, "", ""));
        failing.answers.insert("systemctl", exited(0, "masked", ""));
        let mut held = healthy();
        held.lock = Some(FakeLock::Held);
        let mut held_inputs = inputs();
        held_inputs.daemon_lock = DaemonLock::Probe(PathBuf::from("/l"));
        vec![
            run(&inputs(), &bad_fake()),
            run(&inputs(), &crashed),
            run(&inputs(), &slow),
            run(&inputs(), &failing),
            run(&held_inputs, &held),
            run(&inputs(), &healthy()),
        ]
    }

    #[test]
    fn every_url_in_the_human_output_stands_alone() {
        for report in reports() {
            let text = report.render_human(false);
            for word in text
                .split_whitespace()
                .filter(|w| w.starts_with("https://"))
            {
                let last = word.chars().last().unwrap();
                assert!(
                    last.is_ascii_alphanumeric() || last == '/',
                    "punctuation attached to {word}"
                );
            }
            for line in text.lines() {
                if let Some(at) = line.find("https://") {
                    let url = line[at..].split_whitespace().next().unwrap();
                    assert!(!url.contains('`'), "{line}");
                }
            }
        }
    }

    #[test]
    fn the_human_report_has_the_documented_shape() {
        let text = run(&inputs(), &bad_fake()).render_human(false);
        assert!(text.starts_with("proton-sync doctor: what this machine needs"));
        assert!(text.contains("  missing  proton-drive"));
        assert!(text.contains("           fix:  "));
        assert!(text.contains("           docs: https://osirison.github.io/"));
        assert!(text.contains("           help: https://proton.me/"));
        assert!(text.contains("required problem"));
        assert!(text.ends_with("run `proton-sync doctor` again.\n"));
        let healthy_text = run(&inputs(), &healthy()).render_human(false);
        assert!(healthy_text.contains("Everything required is in place."));
        assert!(!healthy_text.contains("fix:"));
        assert!(!text.contains('\x1b'));
        assert!(
            run(&inputs(), &healthy())
                .render_human(true)
                .contains('\x1b')
        );
    }

    #[test]
    fn json_round_trips_and_carries_every_field() {
        for report in reports() {
            let text = serde_json::to_string(&report).expect("json");
            let value: serde_json::Value = serde_json::from_str(&text).expect("parses");
            assert!(["ok", "failed", "unknown"].contains(&value["verdict"].as_str().unwrap()));
            let checks = value["checks"].as_array().unwrap();
            assert_eq!(checks.len(), 7);
            for check in checks {
                for key in ["id", "severity", "outcome", "statement", "fix", "links"] {
                    assert!(check.get(key).is_some(), "{key} in {check}");
                }
                assert!(
                    ["ok", "missing", "failed", "unknown"]
                        .contains(&check["outcome"].as_str().unwrap())
                );
            }
        }
    }

    #[test]
    fn every_problem_carries_a_fix_and_its_docs_link_and_only_known_links() {
        let known = [
            links::PROTON_DOWNLOAD,
            links::PROTON_DIRECT_DOWNLOAD,
            links::PROTON_SUPPORT,
            links::PROTON_README,
        ];
        for report in reports() {
            for check in &report.checks {
                if check.outcome == Outcome::Ok {
                    continue;
                }
                assert!(check.fix.is_some(), "{:?}", check.id);
                assert_eq!(
                    check.links[0].url,
                    links::docs_url(check.id.as_str()).unwrap()
                );
                for link in &check.links {
                    assert!(
                        known.contains(&link.url.as_str())
                            || link.url.starts_with(links::DOCS_SITE),
                        "{}",
                        link.url
                    );
                }
            }
        }
    }

    #[test]
    fn every_statement_and_fix_starts_with_a_capital_unless_it_starts_with_a_name_or_path() {
        let mut reports = reports();
        // Reports the table above does not reach: the ones that cannot look, and the rest of the
        // proton-drive endings.
        let mut bare = Fake::default();
        bare.vars
            .insert("PROTON_DRIVE_CREDENTIALS_STORE", OsString::from("pass"));
        reports.push(run(&inputs(), &bare));
        reports.push(run(&inputs(), &Fake::default()));
        let mut store = healthy();
        store
            .vars
            .insert("PROTON_DRIVE_CREDENTIALS_STORE", OsString::from("pass"));
        reports.push(run(&inputs(), &store));
        let mut nobus = healthy();
        nobus.vars.clear();
        reports.push(run(&inputs(), &nobus));
        let mut broken = healthy();
        broken
            .answers
            .insert("secret-tool", exited(1, "", "locked"));
        broken.answers.insert("systemctl", exited(1, "", "no bus"));
        reports.push(run(&inputs(), &broken));
        let mut timed = healthy();
        timed.answers.insert("secret-tool", Ran::TimedOut);
        timed.answers.insert("systemctl", exited(0, "odd", ""));
        reports.push(run(&inputs(), &timed));
        for ran in [
            Ran::NotFound,
            Ran::NotExecutable,
            Ran::ExecFormat,
            Ran::Signalled(libc::SIGSEGV),
            Ran::SpawnError(io::ErrorKind::OutOfMemory),
            Ran::Cancelled,
        ] {
            let mut fake = healthy();
            fake.answers.insert("proton-drive", ran);
            reports.push(run(&inputs(), &fake));
        }
        let mut no_home = inputs();
        no_home.config = ConfigFile::NoHome;
        reports.push(run(&no_home, &healthy()));
        let mut relative = inputs();
        relative.cli = CliSource::Explicit(PathBuf::from("bin/proton-drive"));
        reports.push(run(&relative, &healthy()));
        let mut named = inputs();
        named.config = ConfigFile::Named(PathBuf::from("/nope.toml"));
        reports.push(run(&named, &healthy()));
        let mut unreadable = healthy();
        unreadable
            .files
            .insert(PathBuf::from(CONFIG), Err(io::ErrorKind::PermissionDenied));
        reports.push(run(&inputs(), &unreadable));

        // A program, a path, an environment variable or the config key: not a sentence's first word.
        let names = [
            "proton-drive",
            "proton-syncd",
            "proton_cli",
            "secret-tool",
            "curl",
            "systemctl",
            "/",
        ];
        let mut seen = 0;
        for report in reports {
            for check in report.checks {
                for text in [Some(check.statement.as_str()), check.fix.as_deref()]
                    .into_iter()
                    .flatten()
                {
                    seen += 1;
                    let first = text.chars().next().expect("not empty");
                    assert!(
                        first.is_uppercase() || names.iter().any(|name| text.starts_with(name)),
                        "{:?}: {text}",
                        check.id
                    );
                }
            }
        }
        assert!(seen > 60, "only {seen} sentences were looked at");
    }

    #[test]
    fn the_report_lists_the_seven_checks_in_order() {
        let report = run(&inputs(), &healthy());
        let ids: Vec<_> = report.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "proton-drive",
                "signed-in",
                "config",
                "secret-tool",
                "curl",
                "session-bus",
                "service"
            ]
        );
        assert_eq!(report.verdict, Verdict::Ok);
    }

    #[test]
    fn selecting_checks_runs_only_those() {
        let fake = healthy();
        let mut inputs = inputs();
        inputs.cli = CliSource::Explicit(PathBuf::from("proton-drive"));
        inputs.checks = vec![CheckId::ProtonDrive, CheckId::Curl];
        let report = run(&inputs, &fake);
        assert_eq!(report.checks.len(), 2);
        // The config was not needed, so it was not read.
        assert!(fake.lock_probes.borrow().is_empty());
    }

    // ---- links (kept from slice 2) ---------------------------------------------------------

    #[test]
    fn a_docs_url_joins_the_site_the_page_and_the_anchor() {
        assert_eq!(
            links::docs_url("signed-in").as_deref(),
            Some(
                "https://osirison.github.io/proton-drive-sync-engine/reference/troubleshooting/#no-proton-drive-sign-in-is-stored"
            )
        );
        assert_eq!(links::docs_url("no-such-check"), None);
    }

    #[test]
    fn a_link_id_resolves_only_to_a_url_this_module_emits() {
        assert_eq!(
            links::url_for("proton-drive", "proton-download").as_deref(),
            Some(links::PROTON_DOWNLOAD)
        );
        assert_eq!(
            links::url_for("signed-in", "proton-cli-docs").as_deref(),
            Some(links::PROTON_SUPPORT)
        );
        assert_eq!(links::url_for("curl", "docs"), links::docs_url("curl"));
        // A Proton page on a check that never carries one, an unknown link and an unknown check.
        assert_eq!(links::url_for("curl", "proton-download"), None);
        assert_eq!(links::url_for("signed-in", "https://evil.example/"), None);
        assert_eq!(links::url_for("no-such-check", "docs"), None);
    }
}
