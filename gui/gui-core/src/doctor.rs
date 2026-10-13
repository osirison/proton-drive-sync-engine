//! The dependency check, as the desktop app asks it (`add-dependency-check`, D10).
//!
//! The check itself is the engine's (`proton_drive_sync_engine::doctor`), and `proton-sync doctor`
//! runs the same code, so the screen and the command cannot disagree about what is wrong. This
//! module only maps what the app knows (its configured `proton_cli`, its config path) to the
//! engine's [`Inputs`] and decides what `ready` means.
//!
//! **No `SystemEnvironment` here.** [`check`] takes the environment as an argument, and the one
//! caller that passes the real one is the Tauri command, which swaps in an inert one in a test
//! build. `isolation_scan` in `src-tauri` holds both crates to that: the real environment runs
//! `secret-tool lookup` on the desktop keyring and `systemctl --user show` on the live session, and
//! a test that reached it would do both on the developer's machine.

use std::path::Path;

pub use proton_drive_sync_engine::doctor::{
    CheckId, CheckResult, DaemonLock, Environment, Link, Outcome, Report, Severity, Verdict, links,
};
use proton_drive_sync_engine::doctor::{CliSource, ConfigFile, Inputs};

/// The reply of the `check_cli` command, without the distribution (the command adds it).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Checked {
    /// False exactly when the verdict is `failed`: a required check is missing or failed. A
    /// required check that could not be checked, and every recommended result, leaves it true so a
    /// user whose setup the check cannot see is not locked out.
    pub ready: bool,
    pub report: Report,
}

/// The real environment: runs `secret-tool` and `systemctl` and searches the real `PATH`. Named
/// once, here, and called once, by the Tauri command in a non-test build.
pub fn system_environment() -> impl Environment {
    proton_drive_sync_engine::doctor::SystemEnvironment
}

/// An environment that finds nothing and runs nothing, for a build that must not reach the machine.
pub fn inert_environment() -> impl Environment {
    proton_drive_sync_engine::doctor::InertEnvironment
}

/// The engine's inputs for the app: its configured CLI (an absolute path stays one, a bare name is
/// searched on `PATH`), its config file (absent is fine: first run has none), and the lock probe.
pub fn inputs(proton_cli: &str, config_path: &Path, daemon_lock: DaemonLock) -> Inputs {
    let mut inputs = Inputs::new(ConfigFile::Default(config_path.to_path_buf()));
    inputs.cli = CliSource::Explicit(proton_cli.into());
    inputs.daemon_lock = daemon_lock;
    inputs
}

/// Run every check. Blocking: up to the engine's per-tool limits, so call it off the UI thread.
pub fn check(
    proton_cli: &str,
    config_path: &Path,
    daemon_lock: DaemonLock,
    env: &dyn Environment,
) -> Checked {
    let report =
        proton_drive_sync_engine::doctor::run(&inputs(proton_cli, config_path, daemon_lock), env);
    Checked {
        ready: report.verdict != Verdict::Failed,
        report,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;
    use std::time::Duration;

    use proton_drive_sync_engine::daemon::GlobalLockProbe;
    use proton_drive_sync_engine::doctor::{Ran, Stdout};

    use super::*;

    /// A table of answers. Nothing here spawns.
    struct Fake {
        version: Ran,
        keyring: Ran,
        programs: Vec<&'static str>,
        asked: RefCell<Vec<String>>,
    }

    impl Fake {
        fn healthy() -> Self {
            Self {
                version: Ran::Exited {
                    code: 0,
                    stdout_head: String::new(),
                    stderr_head: String::new(),
                },
                keyring: Ran::Exited {
                    code: 0,
                    stdout_head: String::new(),
                    stderr_head: String::new(),
                },
                programs: vec!["secret-tool", "curl", "systemctl"],
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl Environment for Fake {
        fn find_program(&self, name: &str) -> Option<PathBuf> {
            self.programs
                .contains(&name)
                .then(|| PathBuf::from("/fake").join(name))
        }

        fn var(&self, name: &str) -> Option<OsString> {
            match name {
                "DBUS_SESSION_BUS_ADDRESS" => Some("unix:path=/fake/bus".into()),
                _ => None,
            }
        }

        fn read_file(&self, _path: &Path) -> io::Result<String> {
            Err(io::ErrorKind::NotFound.into())
        }

        fn probe_lock(&self, _path: &Path) -> GlobalLockProbe {
            GlobalLockProbe::Unknown("fake".to_owned())
        }

        fn run(&self, program: &Path, _args: &[&str], _limit: Duration, _out: Stdout) -> Ran {
            let name = program.file_name().unwrap().to_string_lossy().into_owned();
            self.asked.borrow_mut().push(name.clone());
            if name == "secret-tool" {
                self.keyring.clone()
            } else if name == "systemctl" {
                Ran::NotFound
            } else {
                self.version.clone()
            }
        }
    }

    fn run(fake: &Fake) -> Checked {
        check(
            "/opt/proton/proton-drive",
            Path::new("/fake/proton-sync.toml"),
            DaemonLock::Skip,
            fake,
        )
    }

    #[test]
    fn ready_is_false_only_for_a_failed_verdict() {
        // Everything required is in place: ok, ready.
        let ok = run(&Fake::healthy());
        assert_eq!(ok.report.verdict, Verdict::Ok);
        assert!(ok.ready);

        // The CLI is not there: a required problem, so failed, so not ready.
        let missing = run(&Fake {
            version: Ran::NotFound,
            ..Fake::healthy()
        });
        assert_eq!(missing.report.verdict, Verdict::Failed);
        assert!(!missing.ready);

        // A CLI that crashes is a required problem too.
        let crashed = run(&Fake {
            version: Ran::Exited {
                code: 1,
                stdout_head: String::new(),
                stderr_head: "boom".to_owned(),
            },
            ..Fake::healthy()
        });
        assert_eq!(crashed.report.verdict, Verdict::Failed);
        assert!(!crashed.ready);

        // The sign-in lookup could not be made (the tool is not there): could-not-check, and a
        // recommended `secret-tool` problem. Neither blocks first run.
        let unknown = run(&Fake {
            programs: vec!["curl", "systemctl"],
            ..Fake::healthy()
        });
        assert_ne!(unknown.report.verdict, Verdict::Failed);
        assert!(unknown.ready, "{:?}", unknown.report);
    }

    #[test]
    fn the_app_checks_its_own_configured_cli_not_the_path() {
        let fake = Fake::healthy();
        run(&fake);
        // The `version` run named the configured binary.
        assert!(fake.asked.borrow().contains(&"proton-drive".to_owned()));
    }
}
