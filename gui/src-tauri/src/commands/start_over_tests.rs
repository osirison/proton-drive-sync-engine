//! `check_proton_folder_change` and `change_proton_folder` (#453, the Settings half).
//!
//! What is held down here is the ORDER and what each failure leaves behind, because that is the design:
//! the daemon is stopped before the folder's index is started over, the index is started over before the
//! file names the new Proton folder, and the daemon is started again only after the file is written — and
//! a failure at any step leaves the folder either as it was or fresh, never resumable against the wrong
//! Proton folder. The facts the sequence stands on (the engine's rule for "the same folder", the
//! transaction that starts an index over, the lockfile as the proof) are tested where they live
//! (`config.rs`, `gui_core::start_over`); the engine's side of "a folder started over is not held" is
//! `daemon.rs`'s `an_index_started_over_while_no_daemon_ran_is_not_held_for_a_changed_remote_root`.
//!
//! **Nothing here reaches the machine.** The config, the folders and every index are in one temp
//! directory. The sync service is a fake (`FakeService`) that logs what it was asked, so no test
//! starts a daemon, a process or `systemctl` (`docs/agent-notes/gui-tests-that-shell-systemctl.md`); the
//! tests that go through the real command dial a socket nobody listens on, where the sequence neither
//! stops nor starts anything.

use super::start_over::{change_proton_folder_file, Service, LOCK_ASK_EVERY, LOCK_WAIT};
use super::*;
use gui_core::set_aside::{self, LockGuard, LockProbe};
use gui_core::start_over::testing::{facts, give_history, started_over, view_of, with_history};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

macro_rules! run {
    ($future:expr) => {
        tauri::async_runtime::block_on($future)
    };
}

// ---- a session with folders that have history ---------------------------------------------------

struct Session {
    app: tauri::App<tauri::test::MockRuntime>,
    dir: tempfile::TempDir,
}

fn dead_socket_in(dir: &Path) -> PathBuf {
    dir.join("nobody-listens.sock")
}

impl Session {
    fn state(&self) -> State<'_, Mutex<RuntimePaths>> {
        self.app.state::<Mutex<RuntimePaths>>()
    }
    fn config_path(&self) -> PathBuf {
        self.dir.path().join("proton-sync.toml")
    }
    fn config(&self) -> String {
        std::fs::read_to_string(self.config_path()).unwrap()
    }
    fn view(&self, name: &str) -> gui_core::config_io::PairView {
        gui_core::config_io::pair_views(&self.config())
            .unwrap()
            .into_iter()
            .find(|view| view.name == name)
            .unwrap_or_else(|| panic!("the config has no folder {name}"))
    }
}

/// Folders `docs` and `photos` under the temp directory, each with the history of a pair that synced
/// against `/Drive/<name>`. `docs` is paused when asked.
fn two_synced_folders(docs_paused: bool) -> Session {
    session(|dir| {
        let (docs, photos) = (dir.join("folders/docs"), dir.join("folders/photos"));
        for root in [&docs, &photos] {
            std::fs::create_dir_all(root).unwrap();
        }
        give_history(&view_of(&docs), docs_paused);
        give_history(&view_of(&photos), false);
        format!(
            "# my config\nsocket_path = \"{}\"\nlog_level = \"info\"\n\n\
             [[pair]]\nname = \"docs\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n\n\
             [[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\n",
            dead_socket_in(dir).display(),
            docs.display(),
            photos.display()
        )
    })
}

/// The one-folder file this app has always written: the top-level keys ARE the pair, called `default`.
fn one_synced_folder() -> Session {
    session(|dir| {
        let root = dir.join("folders/docs");
        std::fs::create_dir_all(&root).unwrap();
        give_history(&view_of(&root), false);
        format!(
            "# my config\nsocket_path = \"{}\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n",
            dead_socket_in(dir).display(),
            root.display()
        )
    })
}

fn session(config: impl FnOnce(&Path) -> String) -> Session {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("proton-sync.toml");
    std::fs::write(&config_path, config(dir.path())).unwrap();
    let mut paths = RuntimePaths::resolve_at(&config_path);
    // Where nothing listens: the daemon is "not running", and by construction no command here can
    // reach a daemon, `systemctl` or the developer's `~/.local/state`.
    paths.socket_path = Ok(dead_socket_in(dir.path()));
    paths.state_dir = Some(dir.path().join("app-state"));
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");
    Session { app, dir }
}

fn update(remote_root: &str) -> ConfigUpdate {
    ConfigUpdate {
        remote_root: Some(remote_root.to_owned()),
        ..Default::default()
    }
}

// ---- a sync service that logs ------------------------------------------------------------------

#[derive(Clone, Default)]
struct Log(Rc<RefCell<Vec<String>>>);

impl Log {
    fn say(&self, what: impl Into<String>) {
        self.0.borrow_mut().push(what.into());
    }
    fn lines(&self) -> Vec<String> {
        self.0.borrow().clone()
    }
}

/// A sync service that answers, can hold a pair's lockfile the way a running daemon does, and fails on cue.
struct FakeService {
    log: Log,
    probe: DaemonProbe,
    /// What the "running daemon" holds until it has stopped (and, if slow, a few sleeps after).
    held: Option<LockGuard>,
    /// `None`: released the moment `stop` succeeds. `Some(n)`: released at the `n`th `sleep` after it.
    release_at_sleep: Option<usize>,
    stop_result: Result<(), RestartOutcome>,
    start_result: RestartOutcome,
    sleeps: usize,
    on_stop: Option<Box<dyn FnMut()>>,
    on_start: Option<Box<dyn FnMut()>>,
}

impl FakeService {
    fn with(probe: DaemonProbe, log: &Log) -> Self {
        Self {
            log: log.clone(),
            probe,
            held: None,
            release_at_sleep: None,
            stop_result: Ok(()),
            start_result: RestartOutcome::Restarted {
                detail: "the service restarted (fake)".to_owned(),
            },
            sleeps: 0,
            on_stop: None,
            on_start: None,
        }
    }

    /// A daemon that is running and holds the pair's lockfile, as the real one does.
    fn running_and_holding(view: &gui_core::config_io::PairView, log: &Log) -> Self {
        let mut service = Self::with(DaemonProbe::Running, log);
        service.held = Some(hold(view));
        service
    }
}

/// Take the pair's lockfile the way a daemon does.
fn hold(view: &gui_core::config_io::PairView) -> LockGuard {
    match set_aside::acquire(view.lockfile_path.as_deref()) {
        LockProbe::Free(guard) => guard,
        other => panic!("precondition: nobody holds the lock yet, got {other:?}"),
    }
}

impl Service for FakeService {
    fn probe(&mut self) -> DaemonProbe {
        self.log.say("probe");
        self.probe
    }

    fn stop(&mut self) -> Result<(), RestartOutcome> {
        self.log.say("stop");
        if let Some(hook) = self.on_stop.as_mut() {
            hook();
        }
        self.stop_result.clone()?;
        if self.release_at_sleep.is_none() {
            self.held = None;
        }
        Ok(())
    }

    fn start(&mut self) -> RestartOutcome {
        self.log.say("start");
        if let Some(hook) = self.on_start.as_mut() {
            hook();
        }
        self.start_result.clone()
    }

    fn sleep(&mut self, how_long: Duration) {
        assert_eq!(how_long, LOCK_ASK_EVERY);
        self.sleeps += 1;
        if self.release_at_sleep.is_some_and(|n| self.sleeps >= n) {
            self.held = None;
        }
    }
}

fn the_log_is(log: &Log, expected: &[&str]) {
    assert_eq!(log.lines(), expected);
}

// ---- the question: check_proton_folder_change ------------------------------------------------------

fn check(
    session: &Session,
    pair: &str,
    remote_root: &str,
) -> Result<start_over::RemoteRootCheck, String> {
    run!(check_proton_folder_change(
        session.state(),
        pair.to_owned(),
        remote_root.to_owned()
    ))
}

#[test]
fn another_proton_folder_is_a_change_and_names_the_folder_it_changed_from() {
    let session = two_synced_folders(false);

    let asked = check(&session, "docs", "/Drive/elsewhere").expect("a known folder");

    assert!(asked.changed);
    assert_eq!(
        asked.from.as_deref(),
        Some("/Drive/docs"),
        "the file's value, as written"
    );
    assert_eq!(asked.to, "/Drive/elsewhere");
}

#[test]
fn a_respelling_of_the_saved_folder_is_not_a_change_so_it_asks_nothing() {
    // `/Drive/X`, `Drive/X`, `Drive/X/` and `./Drive/X` are one folder to the engine (the same question the
    // hold asks): a prompt for any of them would be a prompt for nothing.
    let session = two_synced_folders(false);
    for spelling in ["/Drive/docs", "Drive/docs", "/Drive/docs/", "./Drive/docs"] {
        let asked = check(&session, "docs", spelling).expect("a known folder");
        assert!(!asked.changed, "{spelling}");
    }
}

#[test]
fn it_compares_against_the_file_and_not_against_what_the_window_last_read() {
    // The field changed from what the SETTINGS FILE holds. A window's copy can be a poll old: here the
    // file has moved on to `/Drive/elsewhere` since, and that is what the answer is relative to.
    let session = two_synced_folders(false);
    std::fs::write(
        session.config_path(),
        session.config().replace("/Drive/docs", "/Drive/elsewhere"),
    )
    .unwrap();

    assert!(!check(&session, "docs", "/Drive/elsewhere").unwrap().changed);
    assert!(check(&session, "docs", "/Drive/docs").unwrap().changed);
}

#[test]
fn asking_changes_nothing_not_the_file_not_an_index_not_a_file_of_the_persons() {
    let session = two_synced_folders(true);
    let (config, docs, photos) = (
        session.config(),
        session.view("docs"),
        session.view("photos"),
    );

    let _ = check(&session, "docs", "/Drive/elsewhere").unwrap();
    let _ = check(&session, "docs", "/Drive/docs").unwrap();

    assert_eq!(session.config(), config);
    assert_eq!(facts(&docs), with_history(true));
    assert_eq!(facts(&photos), with_history(false));
}

#[test]
fn a_folder_the_app_does_not_know_is_not_asked_about() {
    let session = two_synced_folders(false);

    let refused = check(&session, "nope", "/Drive/elsewhere").expect_err("unknown");

    assert!(refused.contains("nope"), "{refused}");
}

#[test]
fn a_folder_with_no_saved_proton_folder_has_nothing_to_start_over_from() {
    let session = session(|dir| {
        let root = dir.join("folders/docs");
        std::fs::create_dir_all(&root).unwrap();
        format!(
            "socket_path = \"{}\"\nlocal_root = \"{}\"\n",
            dead_socket_in(dir).display(),
            root.display()
        )
    });

    let asked = check(&session, "default", "/Drive/new").expect("known");

    assert!(!asked.changed);
    assert_eq!(asked.from, None);
}

// ---- the sequence ---------------------------------------------------------------------------------

fn change(
    session: &Session,
    pair: &str,
    update: &ConfigUpdate,
    service: &mut FakeService,
) -> Result<start_over::StartOverReply, String> {
    change_proton_folder_file(&session.config_path(), pair, update, service)
}

#[test]
fn a_running_daemon_is_stopped_first_the_index_started_over_then_the_file_written_then_the_daemon_started(
) {
    // The order, proved by observation rather than by reading: each hook looks at the disk at the moment
    // the daemon is asked to do the next thing.
    let session = two_synced_folders(false);
    let (docs, config_before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    {
        let (view, path, before, said) = (
            docs.clone(),
            session.config_path(),
            config_before.clone(),
            log.clone(),
        );
        service.on_stop = Some(Box::new(move || {
            // At the stop nothing has been touched: the index still describes the old folder and the file
            // still names it.
            assert_eq!(facts(&view), with_history(false), "the index at the stop");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                before,
                "the file at the stop"
            );
            said.say("(nothing touched at the stop)");
        }));
        let (view, path, said) = (docs.clone(), session.config_path(), log.clone());
        service.on_start = Some(Box::new(move || {
            // At the start both are done: the file names the new folder, the index is fresh, and the lock
            // is free for the daemon to take.
            assert!(std::fs::read_to_string(&path)
                .unwrap()
                .contains("/Drive/elsewhere"));
            assert_eq!(facts(&view), started_over(false), "the index at the start");
            assert!(matches!(
                set_aside::acquire(view.lockfile_path.as_deref()),
                LockProbe::Free(_)
            ));
            said.say("(both done at the start)");
        }));
    }

    let reply =
        change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("started over");

    the_log_is(
        &log,
        &[
            "probe",
            "stop",
            "(nothing touched at the stop)",
            "start",
            "(both done at the start)",
        ],
    );
    assert!(
        matches!(reply.restart, RestartOutcome::Restarted { .. }),
        "{:?}",
        reply.restart
    );
    assert_eq!(
        reply.index.as_deref(),
        docs.db_path.as_deref().and_then(Path::to_str)
    );
}

#[test]
fn the_folder_ends_fresh_the_new_folder_is_in_the_file_and_nothing_else_moved() {
    let session = two_synced_folders(false);
    let (docs, photos) = (session.view("docs"), session.view("photos"));
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);

    change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("started over");

    // This folder: fresh. Nothing recorded for the engine to hold it for.
    assert_eq!(facts(&docs), started_over(false));
    // The other folder: exactly as it was, history and all.
    assert_eq!(facts(&photos), with_history(false));
    // The file: the new Proton folder for `docs`, `photos`' untouched, and the person's comments kept.
    let config = session.config();
    assert!(config.contains("# my config"), "{config}");
    let saved = gui_core::config_io::ConfigDoc::load(&session.config_path()).unwrap();
    assert_eq!(
        saved
            .pair("docs")
            .unwrap()
            .get_str("remote_root")
            .as_deref(),
        Some("/Drive/elsewhere")
    );
    assert_eq!(
        saved
            .pair("photos")
            .unwrap()
            .get_str("remote_root")
            .as_deref(),
        Some("/Drive/photos")
    );
    // The person's own file is where it was.
    let root = docs.local_root.unwrap();
    assert_eq!(
        std::fs::read(root.join("mine.txt")).unwrap(),
        b"the person's file"
    );
}

#[test]
fn a_paused_folder_stays_paused_across_the_start_over() {
    let session = two_synced_folders(true);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);

    change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("started over");

    assert_eq!(facts(&docs), started_over(true));
}

#[test]
fn the_other_settings_staged_beside_the_folder_are_saved_in_the_same_write() {
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let both = ConfigUpdate {
        remote_root: Some("/Drive/elsewhere".to_owned()),
        scan_interval_secs: Some(120),
        exclude: Some(vec!["*.tmp".to_owned()]),
        ..Default::default()
    };

    change(&session, "docs", &both, &mut service).expect("started over");

    let saved = gui_core::config_io::ConfigDoc::load(&session.config_path()).unwrap();
    let docs = saved.pair("docs").unwrap();
    assert_eq!(
        docs.get_str("remote_root").as_deref(),
        Some("/Drive/elsewhere")
    );
    assert_eq!(docs.get_int("scan_interval_secs"), Some(120));
    assert_eq!(docs.get_string_array("exclude"), ["*.tmp"]);
    // And only for `docs`.
    assert_eq!(
        saved.pair("photos").unwrap().get_int("scan_interval_secs"),
        None
    );
}

#[test]
fn moving_the_folder_and_its_proton_folder_together_starts_over_the_history_at_the_new_folder() {
    // The history that would meet the new Proton folder is whatever sits at the folder the file will
    // name AFTER the save, so that is the one started over; the folder the pair leaves is not touched.
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let elsewhere = session.dir.path().join("folders/elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let there = view_of(&elsewhere);
    give_history(&there, true);
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let both = ConfigUpdate {
        local_root: Some(elsewhere.display().to_string()),
        remote_root: Some("/Drive/elsewhere".to_owned()),
        ..Default::default()
    };

    change(&session, "docs", &both, &mut service).expect("started over");

    assert_eq!(
        facts(&there),
        started_over(true),
        "the folder it moves to, still paused"
    );
    assert_eq!(facts(&docs), with_history(false), "the folder it leaves");
    let saved = gui_core::config_io::ConfigDoc::load(&session.config_path()).unwrap();
    assert_eq!(
        saved.pair("docs").unwrap().get_str("local_root"),
        Some(elsewhere.display().to_string())
    );
}

#[test]
fn a_one_folder_file_in_the_top_level_spelling_is_started_over_too() {
    let session = one_synced_folder();
    let view = session.view("default");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&view, &log);

    change(
        &session,
        "default",
        &update("/Drive/elsewhere"),
        &mut service,
    )
    .expect("started over");

    assert_eq!(facts(&view), started_over(false));
    let config = session.config();
    assert!(config.contains("# my config"), "{config}");
    assert!(
        config.contains("remote_root = \"/Drive/elsewhere\""),
        "{config}"
    );
    assert!(
        !config.contains("[[pair]]"),
        "the file kept the spelling it had: {config}"
    );
}

// ---- no daemon running ---------------------------------------------------------------------------

#[test]
fn no_daemon_running_at_save_time_still_starts_the_folder_over_and_starts_nothing() {
    // A save is not a request to begin syncing (#320). The index is started over under the lockfile's
    // proof, the file is written, and when the daemon is next started the folder is fresh — not held.
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::with(DaemonProbe::NotRunning, &log);

    let reply =
        change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("started over");

    the_log_is(&log, &["probe"]);
    assert_eq!(reply.restart, RestartOutcome::NotRunning);
    assert_eq!(facts(&docs), started_over(false));
    assert!(session.config().contains("/Drive/elsewhere"));
}

/// What the app says when it could not tell whether the service is running, and so changed nothing.
const COULD_NOT_TELL: &str =
    "The sync service did not answer, so the app could not tell whether it is \
                              running. Nothing was changed.";

#[test]
fn a_daemon_that_cannot_be_asked_refuses_the_start_over_and_nothing_is_touched() {
    // The review's case: the daemon is alive but the app could not ask it (the socket path is the
    // fail-closed `Err`, or the exchange timed out), and the folder is HELD or UNAVAILABLE in it — which
    // holds neither the lockfile nor the index. A reset under that daemon would go through; the daemon
    // would lift the hold, bootstrap against the Proton folder it still has in memory and record it; the app
    // would not restart it, and the next restart would hold the folder against the new Proton folder. So the
    // answer that did not come is a refusal, as the plain save's restart treats the same answer.
    let session = two_synced_folders(false);
    let (docs, photos, before) = (
        session.view("docs"),
        session.view("photos"),
        session.config(),
    );
    let log = Log::default();
    // Nobody holds the lockfile: the lock proves nothing, and that is the point.
    let mut service = FakeService::with(DaemonProbe::Unknown, &log);

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("it could not be told whether the service runs");

    assert_eq!(refused, COULD_NOT_TELL);
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false), "the index is as it was");
    assert_eq!(facts(&photos), with_history(false));
    the_log_is(&log, &["probe"]);
}

#[test]
fn a_daemon_that_cannot_be_asked_and_holds_the_folder_refuses_before_the_lock_is_asked() {
    // The same refusal when it does hold the lockfile: the answer is decided at the probe, and nothing
    // after it (no stop, no wait for the lock, no reset, no start) is reached.
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    service.probe = DaemonProbe::Unknown;

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("held and unaskable");

    assert_eq!(refused, COULD_NOT_TELL);
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
    the_log_is(&log, &["probe"]);
}

#[test]
fn the_command_whose_socket_cannot_be_resolved_refuses_and_changes_nothing() {
    // The real command with its real service and #74's fail-closed socket: there is no address to dial, so
    // the daemon cannot be asked, and the start-over does not guess. Nothing here can reach `systemctl`:
    // the refusal precedes any stop, and a service that was not found running is never started.
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    session.state().lock().unwrap().socket_path =
        Err("the runtime directory is not owned by this user".to_owned());

    let refused = change_through_the_command(&session, "docs", update("/Drive/elsewhere"))
        .expect_err("it could not be told whether the service runs");

    assert_eq!(refused, COULD_NOT_TELL);
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
}

// ---- a slow daemon -------------------------------------------------------------------------------

#[test]
fn a_daemon_a_moment_slow_to_let_go_of_its_lockfile_is_waited_for() {
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    service.release_at_sleep = Some(3);

    change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("it let go");

    assert_eq!(
        service.sleeps, 3,
        "it asked again at the pace it was told to"
    );
    assert_eq!(facts(&docs), started_over(false));
}

#[test]
fn a_daemon_that_never_lets_go_changes_nothing_and_is_started_again() {
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    service.release_at_sleep = Some(usize::MAX);

    let refused =
        change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect_err("held");

    assert_eq!(
        service.sleeps,
        (LOCK_WAIT.as_millis() / LOCK_ASK_EVERY.as_millis()) as usize,
        "it waited the time it is allowed and no longer"
    );
    assert!(refused.contains("Nothing was saved"), "{refused}");
    assert!(
        refused.contains("The sync service was started again."),
        "{refused}"
    );
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false), "the index is as it was");
    assert_eq!(log.lines().last().map(String::as_str), Some("start"));
}

// ---- failures ------------------------------------------------------------------------------------

#[test]
fn a_daemon_that_does_not_stop_changes_nothing_and_is_not_started() {
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    service.stop_result = Err(RestartOutcome::NeverStopped {
        reason: "it was still answering 8s later".to_owned(),
    });

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("never stopped");

    assert!(refused.contains("did not stop"), "{refused}");
    assert!(
        refused.contains("it was still answering 8s later"),
        "the daemon's own words: {refused}"
    );
    assert!(refused.contains("nothing was changed"), "{refused}");
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
    the_log_is(&log, &["probe", "stop"]);
}

#[test]
fn an_index_that_cannot_be_started_over_changes_nothing_and_the_daemon_is_started_again() {
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let db = docs.db_path.clone().unwrap();
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let wrecked = {
        let db = db.clone();
        move || std::fs::write(&db, b"not a database any more").unwrap()
    };
    service.on_stop = Some(Box::new(wrecked));

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("not an index");

    assert!(refused.contains("could not be opened"), "{refused}");
    assert!(refused.contains("Nothing was saved"), "{refused}");
    assert!(
        refused.contains("The sync service was started again."),
        "{refused}"
    );
    assert_eq!(session.config(), before);
    assert_eq!(std::fs::read(&db).unwrap(), b"not a database any more");
    the_log_is(&log, &["probe", "stop", "start"]);
}

/// Whether this process is held to a directory's mode (root is not).
fn modes_are_enforced(directory: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let probe = directory.join("mode-probe");
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    let enforced = std::fs::write(&probe, b"x").is_err();
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _ = std::fs::remove_file(&probe);
    enforced
}

#[test]
fn a_file_that_cannot_be_written_leaves_the_folder_started_over_under_its_old_proton_folder() {
    // The one step that can fail after the index is gone. The file is as it was, so the daemon is started
    // on the OLD Proton folder with a fresh index: a bootstrap, which deletes nothing. Never the old
    // baseline against the new folder.
    let session = two_synced_folders(false);
    let directory = session.dir.path().to_owned();
    if !modes_are_enforced(&directory) {
        return;
    }
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    {
        let directory = directory.clone();
        service.on_stop = Some(Box::new(move || {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        }));
    }

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("unwritable");

    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(refused.contains("Nothing was saved"), "{refused}");
    assert!(
        refused.contains("start over again under the Proton Drive folder the settings still name"),
        "{refused}"
    );
    assert!(
        refused.contains("The sync service was started again."),
        "{refused}"
    );
    assert_eq!(
        session.config(),
        before,
        "the file still names the old Proton folder"
    );
    assert_eq!(
        facts(&docs),
        started_over(false),
        "and the index has nothing to resume"
    );
    the_log_is(&log, &["probe", "stop", "start"]);
}

#[test]
fn a_daemon_that_will_not_start_is_reported_and_the_work_is_still_done() {
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    service.start_result = RestartOutcome::NotStarted {
        reason: "systemctl said no".to_owned(),
    };

    let reply = change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("saved");

    // Saved, started over, and said plainly that nothing is running: the screen's own `not_started`.
    assert_eq!(
        reply.restart,
        RestartOutcome::NotStarted {
            reason: "systemctl said no".to_owned()
        }
    );
    assert_eq!(facts(&docs), started_over(false));
    assert!(session.config().contains("/Drive/elsewhere"));
}

#[test]
fn a_failure_to_start_again_after_a_refusal_is_said_not_hidden() {
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    service.release_at_sleep = Some(usize::MAX);
    service.start_result = RestartOutcome::NotStarted {
        reason: "systemctl said no".to_owned(),
    };

    let refused =
        change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect_err("held");

    assert!(
        refused.contains("could not be started again: systemctl said no"),
        "{refused}"
    );
}

// ---- refusals that precede everything -------------------------------------------------------------

#[test]
fn a_respelling_is_refused_before_the_daemon_is_asked_anything() {
    // Not a change: the dialog never asks for one, and a command that is sent one anyway starts nothing
    // over and stops nothing — the folder's history is for a different Proton folder only.
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);

    let refused =
        change(&session, "docs", &update("Drive/docs/"), &mut service).expect_err("not a change");

    assert!(refused.contains("nothing to start over"), "{refused}");
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
    the_log_is(&log, &[]);
}

#[test]
fn a_save_that_does_not_name_the_proton_folder_is_refused_before_the_daemon_is_asked_anything() {
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let other = ConfigUpdate {
        scan_interval_secs: Some(120),
        ..Default::default()
    };

    let refused = change(&session, "docs", &other, &mut service).expect_err("no folder in it");

    assert!(
        refused.contains("does not change the Proton Drive folder"),
        "{refused}"
    );
    the_log_is(&log, &[]);
}

#[test]
fn a_proton_folder_the_client_would_refuse_is_refused_while_nothing_has_been_touched() {
    // The index is about to be thrown away: a root the daemon starts on and then fails every pass on
    // (`..` in it) is told now, in the engine's words, and not after.
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);

    let refused =
        change(&session, "docs", &update("/Drive/../escape"), &mut service).expect_err("unsafe");

    assert!(refused.contains("never sync"), "{refused}");
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
    the_log_is(&log, &[]);
}

#[test]
fn an_empty_proton_folder_is_not_a_change_and_is_refused_before_anything_is_touched() {
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);

    // An empty value is not a "change" (the writer's own refusal speaks for it, on an ordinary save).
    let refused = change(&session, "docs", &update("  "), &mut service).expect_err("empty");

    assert!(refused.contains("nothing to start over"), "{refused}");
    assert_eq!(session.config(), before);
    the_log_is(&log, &[]);
}

#[test]
fn a_folder_that_cannot_be_read_is_refused_in_the_start_overs_own_words_before_the_daemon_is_stopped(
) {
    // An unplugged drive: the index that matters is on it, and "no index here" would be a claim about a
    // folder nobody could read. The sentence is the start-over's: Remove's says it "moved nothing", and
    // nothing is moved here.
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let root = docs.local_root.clone().unwrap();
    let moved = session.dir.path().join("folders/docs-away");
    std::fs::rename(&root, &moved).unwrap();
    let log = Log::default();
    let mut service = FakeService::with(DaemonProbe::Running, &log);

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("unreadable");

    assert!(
        refused.starts_with(&format!("The folder {} cannot be read (", root.display())),
        "{refused}"
    );
    assert!(
        refused.ends_with(
            "); an unplugged drive looks the same, so the app cannot start it over. \
             Nothing was changed. Plug in the drive or make the folder readable, then save again."
        ),
        "{refused}"
    );
    assert!(
        !refused.contains("moved"),
        "nothing is moved here: {refused}"
    );
    assert_eq!(session.config(), before);
    assert!(!root.exists(), "the folder was not made");
    the_log_is(&log, &[]);
}

#[test]
fn a_relative_path_is_refused_in_the_start_overs_own_words_without_advice_that_cannot_help() {
    let session = two_synced_folders(false);
    let before = session.config();
    std::fs::write(
        session.config_path(),
        before.replace(
            &format!(
                "local_root = \"{}\"",
                session.view("docs").local_root.unwrap().display()
            ),
            "local_root = \"Sync\"",
        ),
    )
    .unwrap();
    let before = session.config();
    let log = Log::default();
    let mut service = FakeService::with(DaemonProbe::Running, &log);

    let refused = change(&session, "docs", &update("/Drive/elsewhere"), &mut service)
        .expect_err("a relative folder cannot be located");

    assert!(
        refused.contains("local_root `Sync` is a relative path"),
        "{refused}"
    );
    assert!(refused.ends_with("Nothing was changed."), "{refused}");
    assert!(!refused.contains("moved"), "{refused}");
    assert_eq!(session.config(), before);
    the_log_is(&log, &[]);
}

#[test]
fn moving_the_folder_to_one_that_does_not_exist_yet_together_with_the_proton_folder_goes_ahead() {
    // The person typed a new folder and a new Proton folder in one save. The new folder is not there, under a
    // folder that can be listed: an absence, by the rule Remove applies to a state file. There is no history
    // to start over; the daemon makes the folder when it starts, and bootstraps it against the new Proton
    // folder. The folder the pair leaves is untouched.
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let fresh = session.dir.path().join("folders/brand-new");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let both = ConfigUpdate {
        local_root: Some(fresh.display().to_string()),
        remote_root: Some("/Drive/elsewhere".to_owned()),
        ..Default::default()
    };

    let reply = change(&session, "docs", &both, &mut service).expect("nothing to start over");

    assert_eq!(reply.index, None, "there was no index to start over");
    assert!(!fresh.exists(), "the app does not make the folder");
    assert_eq!(facts(&docs), with_history(false), "the folder it leaves");
    let saved = gui_core::config_io::ConfigDoc::load(&session.config_path()).unwrap();
    let docs_entry = saved.pair("docs").unwrap();
    assert_eq!(
        docs_entry.get_str("local_root"),
        Some(fresh.display().to_string())
    );
    assert_eq!(
        docs_entry.get_str("remote_root").as_deref(),
        Some("/Drive/elsewhere")
    );
    the_log_is(&log, &["probe", "stop", "start"]);
}

#[test]
fn moving_the_folder_to_one_on_a_drive_that_is_not_there_is_refused() {
    // `usb-not-plugged-in` is not there, so neither is anything that could be told from an unplugged drive.
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());
    let away = session.dir.path().join("usb-not-plugged-in/docs");
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let both = ConfigUpdate {
        local_root: Some(away.display().to_string()),
        remote_root: Some("/Drive/elsewhere".to_owned()),
        ..Default::default()
    };

    let refused = change(&session, "docs", &both, &mut service).expect_err("it cannot be told");

    assert!(refused.contains("cannot be read"), "{refused}");
    assert!(refused.contains("Plug in the drive"), "{refused}");
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
    assert!(!away.exists());
    the_log_is(&log, &[]);
}

#[test]
fn leaving_the_pairs_folder_unreadable_stays_a_refusal_when_only_the_proton_folder_changes() {
    // The rule that lets a new folder through is for a folder the SAVE points the pair at. The folder a pair
    // already syncs that is not there is a drive that may come back, whatever else is typed.
    let session = two_synced_folders(false);
    let docs = session.view("docs");
    let root = docs.local_root.clone().unwrap();
    std::fs::rename(&root, session.dir.path().join("folders/docs-away")).unwrap();
    let log = Log::default();
    let mut service = FakeService::with(DaemonProbe::Running, &log);
    let respelled = ConfigUpdate {
        // The same folder, spelled with a trailing slash: not a move.
        local_root: Some(format!("{}/", root.display())),
        remote_root: Some("/Drive/elsewhere".to_owned()),
        ..Default::default()
    };

    let refused = change(&session, "docs", &respelled, &mut service).expect_err("unreadable");

    assert!(refused.contains("cannot be read"), "{refused}");
    the_log_is(&log, &[]);
}

// ---- the refusals that must precede everything, pinned one by one (#453 review, finding 1) -------------------

#[test]
fn another_folders_proton_folder_is_refused_while_nothing_has_been_touched() {
    // `photos` syncs with `/Drive/photos`. Pointing `docs` there is refused by the config's own validation
    // (two folders may not share a Proton folder) — and it must be refused HERE, before the daemon is
    // stopped and `docs`' index is thrown away: the save validates too, but after the reset, which would
    // leave the folder started over, the settings unchanged and the service stopped and started for nothing.
    let session = two_synced_folders(true);
    let (docs, photos, before) = (
        session.view("docs"),
        session.view("photos"),
        session.config(),
    );
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);

    let refused = change(&session, "docs", &update("/Drive/photos"), &mut service)
        .expect_err("another folder already syncs with it");

    assert!(
        refused.starts_with("config would be rejected by the daemon: ")
            && refused.contains("remote_root `/Drive/photos` is the same path as pair `docs`'s"),
        "the refusal is the config's own, naming what it collides with: {refused}"
    );
    assert_eq!(session.config(), before, "the file is as it was");
    assert_eq!(facts(&docs), with_history(true), "the index is as it was");
    assert_eq!(facts(&photos), with_history(false));
    the_log_is(&log, &[]);
}

#[test]
fn a_local_folder_that_reaches_another_folders_through_a_link_is_refused_while_nothing_has_been_touched(
) {
    // The half of the daemon's boot check the lexical validation cannot see: two roots that differ as text
    // and reach one folder through a symlink. The save writes it (it validates lexically), the daemon
    // refuses to start on it — after this sequence had stopped the service and thrown the index away.
    let session = two_synced_folders(false);
    let (docs, photos, before) = (
        session.view("docs"),
        session.view("photos"),
        session.config(),
    );
    let link = session.dir.path().join("folders/also-photos");
    std::os::unix::fs::symlink(photos.local_root.as_ref().unwrap(), &link).unwrap();
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    let both = ConfigUpdate {
        local_root: Some(link.display().to_string()),
        remote_root: Some("/Drive/elsewhere".to_owned()),
        ..Default::default()
    };

    let refused = change(&session, "docs", &both, &mut service).expect_err("one folder, two names");

    assert!(
        refused.contains("folder pair 'photos'")
            && refused.contains("is the same folder as folder pair 'docs''s local_root")
            && refused.contains("Symlinks are followed"),
        "the refusal is the real-path check's own, naming the folder it reaches: {refused}"
    );
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
    assert_eq!(facts(&photos), with_history(false));
    the_log_is(&log, &[]);
}

#[test]
fn another_folders_history_is_never_touched_by_a_start_over() {
    let session = two_synced_folders(false);
    let (docs, photos) = (session.view("docs"), session.view("photos"));
    let log = Log::default();
    let mut service = FakeService::running_and_holding(&docs, &log);
    // `photos` is held by "its own daemon" the whole time: starting `docs` over must not need it.
    let _photos_lock = hold(&photos);

    change(&session, "docs", &update("/Drive/elsewhere"), &mut service).expect("started over");

    assert_eq!(facts(&photos), with_history(false));
}

// ---- the command, end to end -----------------------------------------------------------------------

fn change_through_the_command(
    session: &Session,
    pair: &str,
    update: ConfigUpdate,
) -> Result<start_over::StartOverReply, String> {
    run!(change_proton_folder(
        session.state(),
        pair.to_owned(),
        update
    ))
}

#[test]
fn the_command_with_no_daemon_listening_starts_the_folder_over_and_starts_nothing() {
    // The real command with its real service: the socket is one nobody listens on, so the probe finds
    // `NotRunning` and the sequence neither stops nor starts anything — no `systemctl`.
    let session = two_synced_folders(false);
    let docs = session.view("docs");

    let reply = change_through_the_command(&session, "docs", update("/Drive/elsewhere"))
        .expect("started over");

    assert_eq!(reply.restart, RestartOutcome::NotRunning);
    assert_eq!(facts(&docs), started_over(false));
    assert!(session.config().contains("/Drive/elsewhere"));
    // And the session reads the file it wrote, not the environment's.
    let paths = session.state();
    let paths = paths.lock().unwrap();
    assert_eq!(paths.config_path, session.config_path());
}

#[test]
fn the_command_refuses_a_folder_the_app_does_not_know() {
    let session = two_synced_folders(false);
    let before = session.config();

    let refused = change_through_the_command(&session, "nope", update("/Drive/elsewhere"))
        .expect_err("unknown");

    assert!(refused.contains("nope"), "{refused}");
    assert_eq!(session.config(), before);
}

#[test]
fn the_command_refuses_a_respelling_and_changes_nothing() {
    let session = two_synced_folders(false);
    let (docs, before) = (session.view("docs"), session.config());

    let refused = change_through_the_command(&session, "docs", update("Drive/docs"))
        .expect_err("not a change");

    assert!(refused.contains("nothing to start over"), "{refused}");
    assert_eq!(session.config(), before);
    assert_eq!(facts(&docs), with_history(false));
}
