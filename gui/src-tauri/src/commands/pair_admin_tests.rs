//! `add_pair` and `remove_pair` (#102 phase 5b-2, ADR 0005 §7, maintainer decision D8).
//!
//! The document machinery (promotion, the table's shape, the refusals) is tested where it lives
//! (`gui_core::config_io`, with the install scripts' reader as a second oracle), and the move itself
//! in `gui_core::set_aside`. What is held down here is what the COMMANDS add: that a refusal leaves
//! the file byte-identical, in which order the refusals speak, that a removal moves the history only
//! after the daemon has been shown to let go, and that the two ends of D8 meet — remove a folder, add
//! it back, and nothing is left to resume.
//!
//! **Nothing here reaches the machine.** The config, the folders and the app's state directory are
//! all in one temp directory. No test starts a daemon or a process: `remove_pair` is driven with a
//! socket nobody listens on (`restart_service_impl(only_if_running = true)` then returns before it
//! would run `systemctl`), and the restart-and-ask sequence is driven through `finish_removal` with the
//! daemon replaced by closures.

use super::pair_admin::{
    finish_removal, AddPairRequest, RemovedPair, SetAsideReport, DAEMON_ANSWER_WAIT,
    DAEMON_ASK_EVERY,
};
use super::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

macro_rules! run {
    ($future:expr) => {
        tauri::async_runtime::block_on($future)
    };
}

/// A mock app whose config, folders and state directory live in one temp directory.
struct Session {
    app: tauri::App<tauri::test::MockRuntime>,
    dir: tempfile::TempDir,
}

/// Where nothing listens: the daemon is "not running".
fn dead_socket_in(dir: &Path) -> PathBuf {
    dir.join("nobody-listens.sock")
}

/// A folder under `dir`, with a file of the person's in it and, if asked, the history a daemon would
/// have left: an index, a WAL, both sidecars and the lockfile.
fn make_folder(dir: &Path, name: &str, history: bool) -> PathBuf {
    let root = dir.join("folders").join(name);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("mine.txt"), format!("{name}'s file")).unwrap();
    if history {
        let state = root.join(".sync");
        std::fs::create_dir_all(&state).unwrap();
        for (file, body) in [
            ("sync_index.db", "the index"),
            ("sync_index.db-wal", "wal"),
            ("sync_index.status.json", "[]"),
            ("sync_index.metrics.json", "{}"),
            ("proton-sync.lock", ""),
        ] {
            std::fs::write(state.join(file), body).unwrap();
        }
    }
    root
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
    fn state_dir(&self) -> PathBuf {
        self.dir.path().join("app-state")
    }
    fn folder(&self, name: &str, history: bool) -> PathBuf {
        make_folder(self.dir.path(), name, history)
    }
}

/// A session whose config is `config(<its directory>)`. The control socket is one nobody listens
/// on and the state directory is inside the temp directory, so by construction no command here can
/// reach a daemon, `systemctl` or the developer's `~/.local/state`.
fn session(config: impl FnOnce(&Path) -> String) -> Session {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("proton-sync.toml");
    std::fs::write(&config_path, config(dir.path())).unwrap();
    let mut paths = RuntimePaths::resolve_at(&config_path);
    paths.socket_path = Ok(dead_socket_in(dir.path()));
    paths.state_dir = Some(dir.path().join("app-state"));
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");
    Session { app, dir }
}

fn pair_text(name: &str, root: &Path) -> String {
    format!(
        "[[pair]]\nname = \"{name}\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/{name}\"\n\n",
        root.display()
    )
}

/// Two `[[pair]]` tables, `docs` first, whose folders exist and each hold history.
fn two_pairs() -> Session {
    session(|dir| {
        let (docs, photos) = (
            make_folder(dir, "docs", true),
            make_folder(dir, "photos", true),
        );
        format!(
            "socket_path = \"{}\"\nlog_level = \"info\"\n\n{}{}",
            dead_socket_in(dir).display(),
            pair_text("docs", &docs),
            pair_text("photos", &photos)
        )
    })
}

/// A single-pair (implicit) file, as every config this app has ever written.
fn one_pair() -> Session {
    session(|dir| {
        let docs = make_folder(dir, "docs", false);
        format!(
            "# my config\nsocket_path = \"{}\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n",
            dead_socket_in(dir).display(),
            docs.display()
        )
    })
}

fn add(
    session: &Session,
    name: &str,
    local_root: &Path,
    remote_root: &str,
) -> Result<pair_admin::AddPairReply, String> {
    add_with(
        session,
        name,
        local_root.to_str().unwrap(),
        remote_root,
        &[],
    )
}

fn add_with(
    session: &Session,
    name: &str,
    local_root: &str,
    remote_root: &str,
    exclude: &[&str],
) -> Result<pair_admin::AddPairReply, String> {
    run!(add_pair(
        session.state(),
        name.to_owned(),
        AddPairRequest {
            local_root: local_root.to_owned(),
            remote_root: remote_root.to_owned(),
            exclude: exclude.iter().map(|rule| (*rule).to_owned()).collect(),
        }
    ))
}

fn remove(session: &Session, name: &str) -> Result<pair_admin::RemovePairReply, String> {
    run!(remove_pair(session.state(), name.to_owned()))
}

// ---- add_pair ------------------------------------------------------------------------------------

#[test]
fn add_pair_promotes_a_single_pair_file_writes_one_table_and_says_a_restart_is_needed() {
    let session = one_pair();
    let photos = session.folder("photos", false);
    let reply = add_with(
        &session,
        "photos",
        photos.to_str().unwrap(),
        "/Drive/photos",
        &["**/*.raw"],
    )
    .expect("adds");
    assert_eq!(reply.pair, "photos");
    assert!(reply.restart_needed);
    assert!(reply.surviving_index.is_none());

    let text = session.config();
    assert!(
        text.starts_with("# my config\nsocket_path = "),
        "the daemon-wide key stays on top: {text}"
    );
    assert!(
        text.contains("[[pair]]\nname = \"default\"\nlocal_root = "),
        "{text}"
    );
    assert!(
        text.ends_with(&format!(
            "\n[[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\nexclude = [\"**/*.raw\"]\n",
            photos.display()
        )),
        "{text}"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(session.config_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the atomic save keeps the file owner-only");

    // The session re-read the file: the new pair is on the roster, the state directory survived.
    let paths = session.state();
    let paths = paths.lock().unwrap();
    assert_eq!(
        paths
            .pairs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["default", "photos"]
    );
    assert_eq!(
        paths.state_dir,
        Some(session.state_dir()),
        "a write keeps the state directory"
    );
}

#[test]
fn names_are_validated_by_the_engines_rule() {
    let session = one_pair();
    let before = session.config();
    let folder = session.folder("music", false);
    for (name, existing) in [
        ("a b", vec!["default"]),
        ("..", vec!["default"]),
        ("-x", vec!["default"]),
        ("DEFAULT", vec!["default"]),
        ("", vec!["default"]),
    ] {
        let error = add(&session, name, &folder, "/Drive/music").unwrap_err();
        let engine = gui_core::config_io::validate_pair_name_among(name, &existing, existing.len())
            .unwrap_err();
        assert_eq!(
            error, engine,
            "{name:?}: the sentence is the engine's, verbatim"
        );
    }
    assert_eq!(session.config(), before, "a refused name writes nothing");
}

#[test]
fn a_relative_local_root_is_refused_before_it_can_reach_the_daemon() {
    // #431: the engine accepts a relative root and the daemon then resolves it against its own
    // working directory and watches the wrong folder.
    let session = one_pair();
    let before = session.config();
    for relative in ["Photos", "./Photos", "../Photos", "sub/dir"] {
        let error = add_with(&session, "photos", relative, "/Drive/photos", &[]).unwrap_err();
        assert!(error.contains("not a full path"), "{relative}: {error}");
    }
    assert_eq!(session.config(), before);
}

#[test]
fn a_root_that_is_missing_or_not_a_folder_is_refused_and_never_created() {
    let session = one_pair();
    let before = session.config();
    let missing = session.dir.path().join("folders/not-made-yet");
    let error = add(&session, "photos", &missing, "/Drive/photos").unwrap_err();
    assert!(error.contains("does not exist"), "{error}");
    assert!(!missing.exists(), "the app never makes the folder");

    let file = session.dir.path().join("folders/just-a-file.txt");
    std::fs::write(&file, "x").unwrap();
    let error = add(&session, "photos", &file, "/Drive/photos").unwrap_err();
    assert!(error.contains("is not a folder"), "{error}");

    // And a root left empty is the engine's to refuse, in its words.
    let error = add_with(&session, "photos", "   ", "/Drive/photos", &[]).unwrap_err();
    assert!(error.contains("local_root"), "{error}");
    assert_eq!(session.config(), before);
}

#[test]
fn a_symlinked_alias_is_refused_before_the_save() {
    // Lexically the two folders are apart; really one is inside the other. The lexical validator
    // passes it and the daemon refuses to start on it, so the command asks the disk.
    let session = one_pair();
    let before = session.config();
    let docs = session.dir.path().join("folders/docs");
    std::fs::create_dir_all(docs.join("inner")).unwrap();
    let alias = session.dir.path().join("alias-to-inner");
    std::os::unix::fs::symlink(docs.join("inner"), &alias).unwrap();

    let error = add(&session, "photos", &alias, "/Drive/photos").unwrap_err();
    assert!(
        error.starts_with("folder pair 'photos': its local_root")
            && error.contains("is inside folder pair 'default''s"),
        "{error}"
    );
    assert!(
        error.contains("(really `"),
        "the engine's sentence names the real path: {error}"
    );
    assert_eq!(session.config(), before);
}

#[test]
fn the_engines_validation_speaks_before_the_real_path_check_and_the_sentences_are_pinned() {
    // A request the engine refuses for its OWN reason and that ALSO overlaps through a symlink. The
    // engine's validation is the first answer: a file that is simply invalid says so in the
    // validator's words before anything is asked of the filesystem.
    let session = one_pair();
    let docs = session.dir.path().join("folders/docs");
    std::fs::create_dir_all(docs.join("inner")).unwrap();
    let alias = session.dir.path().join("alias-to-inner");
    std::os::unix::fs::symlink(docs.join("inner"), &alias).unwrap();

    // Invalid name + a real overlap: the name.
    let error = add(&session, "a b", &alias, "/Drive/photos").unwrap_err();
    assert!(error.contains("may use only"), "{error}");
    assert!(
        !error.contains("really"),
        "the real-path check must not have spoken: {error}"
    );

    // A lexical overlap (which is also a real one): the VALIDATOR's sentence, not the real-path one.
    let nested = docs.join("inner");
    let error = add(&session, "photos", &nested, "/Drive/photos").unwrap_err();
    assert!(
        error.starts_with("config would be rejected by the daemon: "),
        "{error}"
    );
    assert!(error.contains("is inside pair `default`"), "{error}");
    assert!(
        !error.contains("folder pair 'photos': its local_root"),
        "{error}"
    );

    // And only a request the validator passes reaches the real-path sentence, whose shape is pinned.
    let error = add(&session, "photos", &alias, "/Drive/photos").unwrap_err();
    assert!(
        error.starts_with("folder pair 'photos': its local_root"),
        "{error}"
    );
}

#[test]
fn a_failed_add_leaves_the_file_untouched() {
    let session = one_pair();
    let before = session.config();
    let before_modified = std::fs::metadata(session.config_path())
        .unwrap()
        .modified()
        .unwrap();
    let good = session.folder("photos", false);
    for (name, local_root, remote_root) in [
        ("photos", "relative", "/Drive/photos"),
        ("a b", good.to_str().unwrap(), "/Drive/photos"),
        ("photos", good.to_str().unwrap(), "/Drive/docs"),
        ("photos", good.to_str().unwrap(), ""),
        ("photos", "/no/such/folder/anywhere", "/Drive/photos"),
        ("photos", "/home/x/\"quoted\"", "/Drive/photos"),
    ] {
        let _ = add_with(&session, name, local_root, remote_root, &[]).expect_err(name);
        assert_eq!(
            session.config(),
            before,
            "{name} {local_root} {remote_root}"
        );
    }
    assert_eq!(
        std::fs::metadata(session.config_path())
            .unwrap()
            .modified()
            .unwrap(),
        before_modified,
        "the file was not even rewritten with the same bytes"
    );
    assert!(
        std::fs::read_dir(session.dir.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
        "no temp file is left behind"
    );
}

#[test]
fn a_single_pair_file_that_says_dry_run_true_is_refused_when_a_second_pair_is_added() {
    let session = session(|dir| {
        let docs = make_folder(dir, "docs", false);
        format!(
            "dry_run = true\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n",
            docs.display()
        )
    });
    let photos = session.folder("photos", false);
    let before = session.config();
    let error = add(&session, "photos", &photos, "/Drive/photos").unwrap_err();
    // The engine's M3 sentence, naming the key; the same words `validate_file_config_text` gives.
    assert!(error.contains("`default` sets `dry_run = true`"), "{error}");
    assert!(
        error.contains("cannot be used with 2 folder pairs"),
        "{error}"
    );
    assert_eq!(session.config(), before);
}

#[test]
fn a_surviving_index_is_reported() {
    let session = one_pair();
    // A folder that still holds the history of an earlier run.
    let with_history = session.folder("photos", true);
    let reply = add(&session, "photos", &with_history, "/Drive/photos").unwrap();
    let survivor = reply.surviving_index.expect("the old index is named");
    assert_eq!(
        survivor.path,
        with_history.join(".sync/sync_index.db").to_str().unwrap()
    );
    assert!(!survivor.set_aside_pending);
    assert!(
        survivor.message.contains("reset-index"),
        "{}",
        survivor.message
    );
    assert!(
        survivor.message.contains("read as a deletion"),
        "{}",
        survivor.message
    );

    // A clean folder reports nothing.
    let clean = session.folder("music", false);
    assert!(add(&session, "music", &clean, "/Drive/music")
        .unwrap()
        .surviving_index
        .is_none());
}

#[test]
fn an_inline_pair_array_is_never_edited_by_add_or_remove() {
    let session = session(|dir| {
        let (docs, photos) = (
            make_folder(dir, "docs", false),
            make_folder(dir, "photos", false),
        );
        format!(
            "pair = [{{ name = \"docs\", local_root = \"{}\", remote_root = \"/Drive/docs\" }}, \
             {{ name = \"photos\", local_root = \"{}\", remote_root = \"/Drive/photos\" }}]\n",
            docs.display(),
            photos.display()
        )
    });
    let text = session.config();
    let music = session.folder("music", false);
    assert!(add(&session, "music", &music, "/Drive/music")
        .unwrap_err()
        .contains("inline array"));
    assert!(remove(&session, "docs")
        .unwrap_err()
        .contains("inline array"));
    assert_eq!(session.config(), text);
}

// ---- remove_pair ---------------------------------------------------------------------------------

#[test]
fn removing_a_pair_with_no_daemon_running_moves_its_history_out_and_leaves_the_persons_files() {
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    let docs = session.dir.path().join("folders/docs");
    let reply = remove(&session, "photos").expect("removes");

    // The config.
    let text = session.config();
    assert!(!text.contains("photos"), "{text}");
    assert!(text.contains("name = \"docs\""), "{text}");
    assert!(
        text.starts_with("socket_path = "),
        "the daemon-wide keys stay: {text}"
    );

    // The restart: nobody was running, so nothing to restart.
    assert_eq!(reply.restart, RestartOutcome::NotRunning);
    assert!(!reply.restart_needed);
    assert_eq!(reply.new_default, None, "the first pair is still the first");

    // The history: out of the folder, outside every sync root, and the reply says where.
    let SetAsideReport::Moved {
        to, items, message, ..
    } = &reply.set_aside
    else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(
        !photos.join(".sync").exists(),
        "the history left the folder"
    );
    assert_eq!(
        std::fs::read_to_string(photos.join("mine.txt")).unwrap(),
        "photos's file"
    );
    assert!(
        std::path::Path::new(to).starts_with(session.state_dir().join("removed-pairs")),
        "{to}"
    );
    assert!(
        !std::path::Path::new(to).starts_with(&photos)
            && !std::path::Path::new(to).starts_with(&docs)
    );
    assert_eq!(
        std::fs::read_to_string(items[0].to.clone() + "/sync_index.db").unwrap(),
        "the index"
    );
    assert!(
        message.contains(to.as_str()),
        "the reply tells the user where it went: {message}"
    );
    // The other pair's history was not touched.
    assert!(docs.join(".sync/sync_index.db").exists());
}

#[test]
fn a_folder_added_back_after_its_removal_starts_fresh() {
    // THE TWO ENDS OF D8: nothing left in the folder for the new pair to resume.
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    remove(&session, "photos").expect("removes");
    let reply = add(&session, "photos", &photos, "/Drive/photos").expect("adds it back");
    assert!(
        reply.surviving_index.is_none(),
        "{:?}",
        reply.surviving_index
    );
    assert!(!photos.join(".sync/sync_index.db").exists());
}

#[test]
fn removing_the_first_pair_says_which_one_is_the_default_now() {
    let session = two_pairs();
    let reply = remove(&session, "docs").expect("removes");
    assert_eq!(reply.new_default.as_deref(), Some("photos"));
}

#[test]
fn removing_the_last_pair_is_refused_and_touches_nothing() {
    let session = two_pairs();
    remove(&session, "photos").unwrap();
    let before = session.config();
    let history = session.dir.path().join("folders/docs/.sync/sync_index.db");
    let error = remove(&session, "docs").unwrap_err();
    assert!(error.contains("only folder pair"), "{error}");
    assert_eq!(session.config(), before);
    assert!(history.exists(), "a refused removal moves nothing");
}

#[test]
fn removing_a_pair_nobody_knows_is_refused_without_reading_the_file() {
    let session = two_pairs();
    let before = session.config();
    let error = remove(&session, "videos").unwrap_err();
    assert!(error.contains("no folder pair named"), "{error}");
    assert_eq!(session.config(), before);
}

#[test]
fn history_is_never_set_aside_inside_a_folder_another_pair_syncs() {
    // The app's state directory sits inside `docs`' folder. Moving `photos`' history there would upload
    // it as `docs`' ordinary files, so the removal refuses the destination and keeps the history.
    let session = two_pairs();
    let inside_docs = session.dir.path().join("folders/docs/app-state");
    session.state().lock().unwrap().state_dir = Some(inside_docs);
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    let SetAsideReport::Pending { reason, .. } = &reply.set_aside else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(reason.contains("uploaded as ordinary files"), "{reason}");
    assert!(session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db")
        .exists());
}

#[test]
fn a_daemon_that_holds_the_history_leaves_it_in_place_and_the_reply_says_so() {
    // The daemon is "not running" by its socket but holds the pair's lockfile (a daemon started by
    // hand with another socket): the lock is the proof that does not depend on the socket.
    let session = two_pairs();
    let lock = session
        .dir
        .path()
        .join("folders/photos/.sync/proton-sync.lock");
    let held = gui_core::testing::hold_lockfile(&lock);

    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    let SetAsideReport::Pending {
        reason,
        record,
        message,
        ..
    } = &reply.set_aside
    else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(reason.contains("still holds"), "{reason}");
    assert!(session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db")
        .exists());
    let record = record.as_ref().expect("a note of the pending move is kept");
    assert!(
        std::path::Path::new(record).starts_with(session.state_dir().join("pending-set-asides"))
    );
    assert!(message.contains("NOT moved"), "{message}");
    drop(held);
}

#[test]
fn a_session_with_no_state_directory_records_nothing_and_says_how_to_finish() {
    let session = two_pairs();
    session.state().lock().unwrap().state_dir = None;
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    let SetAsideReport::Pending {
        reason,
        record,
        message,
        ..
    } = &reply.set_aside
    else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(reason.contains("no state directory"), "{reason}");
    assert!(record.is_none());
    assert!(
        message.contains("Move it out of the folder yourself"),
        "{message}"
    );
    assert!(session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db")
        .exists());
}

#[test]
fn a_pending_move_is_retried_on_the_next_add_or_remove() {
    let session = two_pairs();
    let lock = session
        .dir
        .path()
        .join("folders/photos/.sync/proton-sync.lock");
    let held = gui_core::testing::hold_lockfile(&lock);
    let SetAsideReport::Pending { .. } = remove(&session, "photos").unwrap().set_aside else {
        panic!()
    };
    // The daemon lets go. Adding ANOTHER folder is enough to finish the earlier move.
    drop(held);
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    assert!(
        matches!(&reply.settled_earlier[..], [SetAsideReport::Moved { .. }]),
        "{:?}",
        reply.settled_earlier
    );
    assert!(!session.dir.path().join("folders/photos/.sync").exists());
}

// ---- the sequence, with the daemon replaced by closures ---------------------------------------------

fn removed_with_history(session: &Session) -> RemovedPair {
    let root = session.dir.path().join("folders/photos");
    let view = gui_core::config_io::PairView {
        name: "photos".to_owned(),
        local_root: Some(root.clone()),
        remote_root: Some(PathBuf::from("/Drive/photos")),
        db_path: Some(root.join(".sync/sync_index.db")),
        lockfile_path: Some(root.join(".sync/proton-sync.lock")),
        conflict_suffix: None,
    };
    RemovedPair {
        name: "photos".to_owned(),
        view,
        remaining: Vec::new(),
        new_default: None,
    }
}

fn finish(
    session: &Session,
    restart: impl FnOnce() -> Result<RestartOutcome, String>,
    ask: impl FnMut() -> Option<Vec<String>>,
    sleep: impl FnMut(Duration),
) -> pair_admin::RemovePairReply {
    finish_removal(
        &removed_with_history(session),
        &session.config_path(),
        Some(&session.state_dir()),
        restart,
        ask,
        sleep,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_555_930),
    )
}

#[test]
fn the_history_moves_only_after_the_restarted_daemon_has_stopped_listing_the_pair() {
    let session = two_pairs();
    let history = session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db");
    let events = RefCell::new(Vec::<String>::new());
    // The daemon is still coming up for two asks, then lists only `docs`.
    let answers = RefCell::new(vec![None, None, Some(vec!["docs".to_owned()])]);
    let reply = finish(
        &session,
        || {
            events
                .borrow_mut()
                .push(format!("restart (history present: {})", history.exists()));
            Ok(RestartOutcome::Restarted {
                detail: "restarted".to_owned(),
            })
        },
        || {
            events
                .borrow_mut()
                .push(format!("ask (history present: {})", history.exists()));
            answers.borrow_mut().remove(0)
        },
        |_| {},
    );
    // Every moment before the answer, the history was where it was.
    assert_eq!(
        events.into_inner(),
        [
            "restart (history present: true)",
            "ask (history present: true)",
            "ask (history present: true)",
            "ask (history present: true)",
        ]
    );
    assert!(
        matches!(reply.set_aside, SetAsideReport::Moved { .. }),
        "{:?}",
        reply.set_aside
    );
    assert!(!history.exists());
    assert!(!reply.restart_needed);
}

#[test]
fn a_daemon_that_still_lists_the_pair_keeps_the_history() {
    let session = two_pairs();
    let reply = finish(
        &session,
        || {
            Ok(RestartOutcome::Restarted {
                detail: String::new(),
            })
        },
        || Some(vec!["docs".to_owned(), "photos".to_owned()]),
        |_| {},
    );
    let SetAsideReport::Pending { reason, .. } = &reply.set_aside else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(reason.contains("still lists the pair"), "{reason}");
    assert!(session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db")
        .exists());
}

#[test]
fn a_daemon_that_never_answers_is_waited_for_a_bounded_time_and_the_history_stays() {
    let session = two_pairs();
    let waited = Cell::new(Duration::ZERO);
    let reply = finish(
        &session,
        || {
            Ok(RestartOutcome::Restarted {
                detail: String::new(),
            })
        },
        || None,
        |pause| waited.set(waited.get() + pause),
    );
    assert_eq!(
        waited.get(),
        DAEMON_ANSWER_WAIT,
        "bounded, and at the cadence it was asked"
    );
    assert!(DAEMON_ASK_EVERY < DAEMON_ANSWER_WAIT);
    let SetAsideReport::Pending { reason, .. } = &reply.set_aside else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(reason.contains("did not say which pairs"), "{reason}");
    assert!(session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db")
        .exists());
}

#[test]
fn only_an_ending_that_proves_nothing_holds_the_pair_lets_the_history_move() {
    // Exhaustive over the restart's endings: the two that prove it, and every other.
    for (outcome, moves) in [
        (Ok(RestartOutcome::NotRunning), true),
        (
            Ok(RestartOutcome::NotStarted {
                reason: "no unit".to_owned(),
            }),
            true,
        ),
        (
            Ok(RestartOutcome::NeverStopped {
                reason: "still answering".to_owned(),
            }),
            false,
        ),
        (
            Ok(RestartOutcome::Undetermined {
                reason: "could not tell".to_owned(),
            }),
            false,
        ),
        (Ok(RestartOutcome::Unknown), false),
        (Err("no socket path".to_owned()), false),
    ] {
        let session = two_pairs();
        let label = format!("{outcome:?}");
        let reply = finish(
            &session,
            || outcome,
            || panic!("asked a daemon that was not restarted"),
            |_| {},
        );
        let moved = !session.dir.path().join("folders/photos/.sync").exists();
        assert_eq!(moved, moves, "{label}: {:?}", reply.set_aside);
        assert_eq!(
            matches!(reply.set_aside, SetAsideReport::Moved { .. }),
            moves,
            "{label}"
        );
    }
}

#[test]
fn restart_needed_is_false_only_when_the_daemon_is_known_to_run_the_new_file() {
    for (outcome, needed) in [
        (
            Ok(RestartOutcome::Restarted {
                detail: String::new(),
            }),
            false,
        ),
        (Ok(RestartOutcome::NotRunning), false),
        (
            Ok(RestartOutcome::NotStarted {
                reason: String::new(),
            }),
            true,
        ),
        (
            Ok(RestartOutcome::NeverStopped {
                reason: String::new(),
            }),
            true,
        ),
        (
            Ok(RestartOutcome::Undetermined {
                reason: String::new(),
            }),
            true,
        ),
        (Err("x".to_owned()), true),
    ] {
        let session = two_pairs();
        let label = format!("{outcome:?}");
        let reply = finish(
            &session,
            || outcome,
            || Some(vec!["docs".to_owned()]),
            |_| {},
        );
        assert_eq!(reply.restart_needed, needed, "{label}");
    }
}

#[test]
fn a_pair_with_no_history_on_disk_has_nothing_to_move_and_asks_the_daemon_nothing() {
    let session = session(|_| String::new());
    let root = session.folder("bare", false);
    let removed = RemovedPair {
        name: "bare".to_owned(),
        view: gui_core::config_io::PairView {
            name: "bare".to_owned(),
            local_root: Some(root.clone()),
            remote_root: Some(PathBuf::from("/Drive/bare")),
            db_path: Some(root.join(".sync/sync_index.db")),
            lockfile_path: Some(root.join(".sync/proton-sync.lock")),
            conflict_suffix: None,
        },
        remaining: Vec::new(),
        new_default: Some("other".to_owned()),
    };
    let reply = finish_removal(
        &removed,
        &session.config_path(),
        Some(&session.state_dir()),
        || Ok(RestartOutcome::NotRunning),
        || None,
        |_| {},
        SystemTime::UNIX_EPOCH,
    );
    assert!(
        matches!(reply.set_aside, SetAsideReport::NothingToMove { .. }),
        "{:?}",
        reply.set_aside
    );
    assert_eq!(reply.new_default.as_deref(), Some("other"));
    assert!(
        !session.state_dir().join("removed-pairs").exists(),
        "no destination was made for nothing"
    );
}

// ---- carried from PR 4's review ----------------------------------------------------------------------

#[test]
fn a_local_root_refusal_inside_apply_update_cannot_be_swallowed() {
    // A per-pair key sent to a file whose pairs are an inline array: the editor refuses. If
    // `apply_update` swallowed that refusal, `write_config` would save the file unchanged and report
    // SUCCESS for an edit that never happened.
    let inline = "pair = [{ name = \"docs\", local_root = \"/a\", remote_root = \"/Drive/a\" }]\n";
    let mut doc = config_io::ConfigDoc::from_toml_str(inline).unwrap();
    let update = ConfigUpdate {
        local_root: Some("/elsewhere".to_owned()),
        ..Default::default()
    };
    let error = apply_update(&mut doc, "docs", &update).expect_err("the refusal is returned");
    assert!(error.to_string().contains("inline array"), "{error}");

    // And through the command: the save fails, with the file byte-identical.
    let session = session(|dir| {
        let folder = make_folder(dir, "docs", false);
        format!(
            "pair = [{{ name = \"docs\", local_root = \"{}\", remote_root = \"/Drive/a\" }}]\n",
            folder.display()
        )
    });
    let text = session.config();
    let error = run!(write_config(session.state(), "docs".to_owned(), update)).unwrap_err();
    assert!(error.contains("inline array"), "{error}");
    assert_eq!(session.config(), text, "a refused edit changes nothing");

    // The same on a file of `[[pair]]` tables when the pair named is not in it: a refusal is an
    // error, not a skipped key.
    let tables = two_pairs();
    let before = tables.config();
    let error = run!(write_config(
        tables.state(),
        "docs".to_owned(),
        ConfigUpdate {
            local_root: Some("/x".to_owned()),
            ..Default::default()
        }
    ));
    // `docs` exists, so this one is an ordinary edit and succeeds; the point is the unknown pair:
    assert!(error.is_ok());
    let error = run!(write_config(
        tables.state(),
        "ghost".to_owned(),
        ConfigUpdate {
            local_root: Some("/y".to_owned()),
            ..Default::default()
        }
    ))
    .unwrap_err();
    assert!(error.contains("ghost"), "{error}");
    assert_ne!(tables.config(), before, "the first edit landed");
    assert!(!tables.config().contains("/y"), "the refused one did not");
}

#[test]
fn write_config_runs_the_engines_validation_before_the_real_path_check() {
    // An edit the validator refuses for its own reason AND the real-path rule would refuse for
    // another. The validator's sentence is the one the person gets, and each sentence is pinned.
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    std::fs::create_dir_all(photos.join("inner")).unwrap();
    let before = session.config();

    // docs moved INSIDE photos, written lexically: both rules refuse it.
    let nested = photos.join("inner");
    let error = run!(write_config(
        session.state(),
        "docs".to_owned(),
        ConfigUpdate {
            local_root: Some(nested.display().to_string()),
            ..Default::default()
        }
    ))
    .unwrap_err();
    assert!(
        error.starts_with("config would be rejected by the daemon: "),
        "{error}"
    );
    assert!(
        !error.starts_with("folder pair "),
        "the real-path sentence must not be the first answer: {error}"
    );

    // docs moved to an alias of a folder inside photos: lexically fine, so only the real-path rule
    // refuses, in its own shape.
    let alias = session.dir.path().join("alias");
    std::os::unix::fs::symlink(photos.join("inner"), &alias).unwrap();
    let error = run!(write_config(
        session.state(),
        "docs".to_owned(),
        ConfigUpdate {
            local_root: Some(alias.display().to_string()),
            ..Default::default()
        }
    ))
    .unwrap_err();
    assert!(
        error.starts_with("folder pair 'photos': its local_root"),
        "{error}"
    );
    assert!(error.contains("(really `"), "{error}");
    assert_eq!(session.config(), before);
}

#[test]
fn a_save_keeps_the_state_directory_a_re_resolve_cannot_know() {
    let session = two_pairs();
    let expected = session.state().lock().unwrap().state_dir.clone();
    assert!(expected.is_some());
    run!(write_config(
        session.state(),
        "docs".to_owned(),
        ConfigUpdate {
            scan_interval_secs: Some(90),
            ..Default::default()
        }
    ))
    .unwrap();
    assert_eq!(session.state().lock().unwrap().state_dir, expected);
}
