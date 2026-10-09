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
fn a_drive_path_with_a_dotdot_is_refused_in_the_engines_words_and_writes_nothing() {
    // MEASURED, not inferred (review of #450, F4): a daemon started on `remote_root = "/Drive/../x"`
    // runs, hands the path to the CLI as written and — when the CLI does not find it — plans the root
    // for creation, which fails with `unsafe remote root path: /Drive/../x` on every pass. The engine
    // accepts such a file at startup (and must go on doing so: it starts today), so the add is where
    // it stops, in the sentence the daemon itself fails with.
    let session = one_pair();
    let before = session.config();
    let folder = session.folder("photos", false);
    for remote in ["/Drive/../x", "Drive/a/../b", "/../x", "/Drive/photos/.."] {
        let engine = format!("unsafe remote root path: {remote}");
        let error = add(&session, "photos", &folder, remote).unwrap_err();
        assert!(error.starts_with(&engine), "{remote}: {error}");
        assert_eq!(
            session.config(),
            before,
            "{remote}: a refusal writes nothing"
        );
        // The dialog's check says the same thing, before anything is added.
        let reported = check(&session, "photos", folder.to_str().unwrap(), remote);
        assert_eq!(
            reported.refusal.as_deref(),
            Some(error.as_str()),
            "{remote}: the check and the add must say the same thing"
        );
    }
    // A `.` is cleaned by the engine, not refused: it is not what was measured.
    add(&session, "photos", &folder, "/Drive/./photos").expect("a '.' component is cleaned");
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
    // The maintainer's sentence (decision D8), naming the pair: `--pair` is a global flag of
    // `proton-sync`, and `reset-index --yes` alone resets the DEFAULT pair, which is not the one added.
    assert!(
        survivor
            .message
            .contains("already holds sync history from an earlier setup"),
        "{}",
        survivor.message
    );
    assert!(
        survivor.message.contains("deletions to approve"),
        "{}",
        survivor.message
    );
    assert!(
        survivor
            .message
            .contains("run proton-sync reset-index --yes --pair photos after adding"),
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

// ---- check_add_pair -------------------------------------------------------------------------------

fn check(
    session: &Session,
    name: &str,
    local_root: &str,
    remote_root: &str,
) -> pair_admin::AddPairCheck {
    run!(check_add_pair(
        session.state(),
        name.to_owned(),
        AddPairRequest {
            local_root: local_root.to_owned(),
            remote_root: remote_root.to_owned(),
            exclude: Vec::new(),
        }
    ))
    .expect("a check does not fail, it reports")
}

#[test]
fn a_check_says_what_add_pair_would_say_and_writes_nothing() {
    // The dialog asks first and the command asks again; the two must never disagree, so the check is
    // held to the command, case by case: the refusal it reports IS the error the add returns.
    let session = one_pair();
    let before = session.config();
    let real = session.folder("music", false);
    let real = real.to_str().unwrap();
    let missing = session.dir.path().join("folders/not-there");
    let missing = missing.to_str().unwrap();
    for (name, local, remote) in [
        ("music", real, "/Drive/music"),
        ("music", "Music", "/Drive/music"),
        ("music", "./Music", "/Drive/music"),
        ("music", missing, "/Drive/music"),
        ("music", real, ""),
        ("music", real, "/Drive/../escape"),
    ] {
        let reported = check(&session, name, local, remote);
        let added = add_with(&session, name, local, remote, &[]);
        match (reported.refusal, reported.name_error, added) {
            (None, None, Ok(_)) => {}
            (Some(refusal), None, Err(error)) => assert_eq!(
                refusal, error,
                "{name} {local:?} {remote:?}: the check and the add must say the same thing"
            ),
            // A half-typed form (a root still empty) is asked about its name and nothing else, so the
            // add may refuse where the check stays quiet.
            (None, None, Err(_)) if remote.is_empty() => {}
            other => {
                panic!("{name} {local:?} {remote:?}: the check and the add disagree: {other:?}")
            }
        }
        // Whatever the add did, undo it so every case starts from the same file.
        std::fs::write(session.config_path(), &before).unwrap();
    }
}

#[test]
fn a_check_alone_writes_nothing_and_settles_nothing() {
    let session = one_pair();
    let before = session.config();
    let music = session.folder("music", true);
    for name in ["music", "a b", ""] {
        check(&session, name, music.to_str().unwrap(), "/Drive/music");
        check(&session, name, "", "");
    }
    assert_eq!(session.config(), before, "a check writes nothing");
    assert!(
        !session.state_dir().exists(),
        "and settles nothing: it must not create the app's state directory"
    );
    assert!(
        music.join(".sync/sync_index.db").exists(),
        "and moves nothing of the folder it looked at"
    );
}

#[test]
fn a_check_gives_the_engines_sentence_for_a_bad_name_on_its_own_while_the_rest_is_unwritten() {
    let session = one_pair();
    for (name, existing) in [
        ("a b", vec!["default"]),
        ("-x", vec!["default"]),
        ("DEFAULT", vec!["default"]),
        ("", vec!["default"]),
    ] {
        // Both roots empty: the form is half-typed, and only the name is asked about.
        let reported = check(&session, name, "", "");
        let engine = gui_core::config_io::validate_pair_name_among(name, &existing, existing.len())
            .unwrap_err();
        assert_eq!(
            reported.name_error.as_deref(),
            Some(engine.as_str()),
            "{name:?}"
        );
        assert!(
            reported.refusal.is_none(),
            "{name:?}: the name speaks alone"
        );
    }
    // A good name with the roots still empty is quiet.
    let quiet = check(&session, "photos", "", "");
    assert!(quiet.name_error.is_none() && quiet.refusal.is_none());
}

#[test]
fn a_check_suggests_a_name_from_the_folder_that_the_engine_accepts() {
    let session = one_pair();
    let reported = check(&session, "", "~/My Photos", "");
    assert_eq!(reported.suggested_name, "my-photos");
    // The folder `docs` is the file's own pair `default`; a folder called "default" is not offered
    // that name, which already belongs to the first table.
    let reported = check(&session, "", "~/Default", "");
    assert_eq!(reported.suggested_name, "default-2");
    gui_core::config_io::validate_pair_name_among(&reported.suggested_name, &["default"], 1)
        .expect("a suggestion is a name the engine takes");
}

#[test]
fn a_check_names_a_surviving_index_before_anything_is_saved() {
    let session = one_pair();
    let with_history = session.folder("photos", true);
    let before = session.config();
    let reported = check(
        &session,
        "photos",
        with_history.to_str().unwrap(),
        "/Drive/photos",
    );
    assert!(reported.refusal.is_none(), "{:?}", reported.refusal);
    let survivor = reported
        .surviving_index
        .expect("the old index is named in the check, not only after the add");
    assert!(!survivor.set_aside_pending);
    assert!(
        survivor
            .message
            .contains("run proton-sync reset-index --yes --pair photos after adding"),
        "{}",
        survivor.message
    );
    assert_eq!(session.config(), before, "naming it saved nothing");

    // A clean folder names nothing, and so does a refused add (there is nothing to resume yet).
    let clean = session.folder("music", false);
    assert!(
        check(&session, "music", clean.to_str().unwrap(), "/Drive/music")
            .surviving_index
            .is_none()
    );
    assert!(
        check(&session, "photos", "relative/Photos", "/Drive/photos")
            .surviving_index
            .is_none()
    );
}

#[test]
fn a_check_reports_a_real_path_overlap_in_the_words_the_add_uses() {
    // Not lexical: only a symlink makes the second folder the first's, and the check must see it.
    let session = one_pair();
    let docs = session.dir.path().join("folders/docs");
    let alias = session.dir.path().join("folders/alias-of-docs");
    std::os::unix::fs::symlink(&docs, &alias).unwrap();
    let reported = check(&session, "alias", alias.to_str().unwrap(), "/Drive/alias");
    let refused = add_with(
        &session,
        "alias",
        alias.to_str().unwrap(),
        "/Drive/alias",
        &[],
    )
    .expect_err("the add refuses an alias of another pair's folder");
    assert_eq!(reported.refusal.as_deref(), Some(refused.as_str()));
}

// ---- add, restart, list, remove -------------------------------------------------------------------

/// A session whose control socket is a LIVE fake daemon. **Nothing here may restart anything**: a
/// `restart_service_impl` against a socket that answers goes on to `systemctl`, so the restart is a
/// closure these tests supply (`finish_removal`) and `add_pair` — which starts nothing — is the only
/// command run.
fn session_with_daemon(
    daemon: &gui_core::testing::FakeDaemon,
    config: impl FnOnce(&Path) -> String,
) -> Session {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("proton-sync.toml");
    std::fs::write(&config_path, config(dir.path())).unwrap();
    let mut paths = RuntimePaths::resolve_at(&config_path);
    paths.socket_path = Ok(daemon.socket_path().to_owned());
    paths.state_dir = Some(dir.path().join("app-state"));
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");
    Session { app, dir }
}

#[test]
fn a_folder_is_added_to_an_implicit_file_listed_by_the_restarted_daemon_and_removed_again() {
    use gui_core::testing::{FakeDaemon, FakePair};
    // The daemon runs the file's one implicit pair, `default`.
    let daemon = FakeDaemon::multi_pair(vec![FakePair::new("default")]).start();
    let session = session_with_daemon(&daemon, |dir| {
        let docs = make_folder(dir, "docs", false);
        format!(
            "# my config\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n",
            docs.display()
        )
    });
    let photos = session.folder("photos", true);
    let socket = daemon.socket_path().to_owned();
    assert_eq!(
        pairs_the_daemon_runs(&Ok(socket.clone())),
        Some(vec!["default".to_owned()])
    );

    // 1. The check, then the one add.
    let reported = check(
        &session,
        "photos",
        photos.to_str().unwrap(),
        "/Drive/photos",
    );
    assert!(reported.refusal.is_none() && reported.name_error.is_none());
    assert_eq!(reported.suggested_name, "photos");
    let reply = add(&session, "photos", &photos, "/Drive/photos").expect("adds");
    assert!(reply.restart_needed);

    // 2. The file is now two `[[pair]]` tables, `default` first.
    let text = session.config();
    let names: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .map(|rest| rest.trim_end_matches('"'))
        .collect();
    assert_eq!(names, ["default", "photos"], "{text}");
    assert_eq!(text.matches("[[pair]]").count(), 2, "{text}");

    // 3. The daemon has not picked it up until it restarts; the restarted one lists both.
    assert_eq!(
        pairs_the_daemon_runs(&Ok(socket.clone())),
        Some(vec!["default".to_owned()]),
        "a daemon that has not restarted still runs one"
    );
    daemon.set_pairs(vec![FakePair::new("default"), FakePair::new("photos")]);
    assert_eq!(
        pairs_the_daemon_runs(&Ok(socket.clone())),
        Some(vec!["default".to_owned(), "photos".to_owned()])
    );

    // 4. Removing it: the pair leaves the file, the daemon restarts without it, and only then does its
    //    history move (it had some, because the folder was added with a `.sync` in it).
    let removed = pair_admin::remove_pair_file(&session.config_path(), "photos").expect("removes");
    assert_eq!(removed.new_default, None, "photos was not the first");
    let reply = finish_removal(
        &removed,
        &session.config_path(),
        Some(&session.state_dir()),
        || {
            daemon.set_pairs(vec![FakePair::new("default")]);
            Ok(RestartOutcome::Restarted {
                detail: "restarted".to_owned(),
            })
        },
        || pairs_the_daemon_runs(&Ok(socket.clone())),
        |_| {},
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_555_930),
    );
    assert!(
        matches!(reply.set_aside, SetAsideReport::Moved { .. }),
        "{:?}",
        reply.set_aside
    );
    assert!(
        !photos.join(".sync").exists(),
        "the history is out of the folder"
    );
    assert!(
        photos.join("mine.txt").exists(),
        "and the person's file is where it was"
    );
    assert!(!reply.restart_needed);
    let after = session.config();
    assert_eq!(after.matches("[[pair]]").count(), 1, "{after}");
    assert!(after.contains("name = \"default\""), "{after}");
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

/// What the daemon last said it runs: these names, and nothing about where they are.
fn the_daemon_runs(session: &Session, names: &[&str]) {
    session.state().lock().unwrap().daemon.pairs = names
        .iter()
        .map(|name| crate::config_path::PairReported {
            name: (*name).to_owned(),
            local_root: format!("/r/{name}").into(),
            remote_root: format!("/Drive/r/{name}").into(),
            db_path: format!("/r/{name}.db").into(),
        })
        .collect();
}

#[test]
fn a_folder_the_file_lists_and_the_daemon_does_not_run_is_removed() {
    // The state a failed restart after an add leaves: the file has `photos`, the daemon answered and runs
    // `docs` alone, and the Settings list draws `photos` with `Remove`. The name is the FILE's to resolve.
    let session = two_pairs();
    the_daemon_runs(&session, &["docs"]);
    let reply = remove(&session, "photos").expect("a folder only the file knows can be removed");
    assert_eq!(reply.pair, "photos");
    let text = session.config();
    assert!(!text.contains("photos"), "{text}");
    assert!(text.contains("name = \"docs\""), "{text}");
}

#[test]
fn a_name_only_the_daemon_knows_is_refused_against_the_file_and_touches_nothing() {
    // The daemon lists `videos` (an older file it was started on); the file does not have it. The refusal
    // names the file's folders, not the daemon's, and the file is byte-identical afterwards.
    let session = two_pairs();
    the_daemon_runs(&session, &["docs", "photos", "videos"]);
    let before = session.config();
    for name in ["videos", "Photos", "photos ", ""] {
        let error = remove(&session, name).unwrap_err();
        assert!(
            error.contains(&format!("no folder pair named {name:?}")),
            "{name:?}: {error}"
        );
        assert!(
            error.ends_with("(it has \"docs\", \"photos\")"),
            "{name:?}: the refusal lists the file's folders: {error}"
        );
        assert_eq!(
            session.config(),
            before,
            "{name:?}: a refusal writes nothing"
        );
    }
    assert!(session
        .dir
        .path()
        .join("folders/photos/.sync/sync_index.db")
        .exists());
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

// ---- the review round: what a reply may say ----------------------------------------------------------

/// Whether this user is stopped by file permissions at all. Root reads everything, so a permission
/// test run as root proves nothing: it says so and stops instead of passing.
fn permissions_are_enforced(base: &Path) -> bool {
    let probe = base.join("permission-probe");
    std::fs::create_dir_all(&probe).unwrap();
    set_mode(&probe, 0o000);
    let enforced = std::fs::read_dir(&probe).is_err();
    set_mode(&probe, 0o755);
    std::fs::remove_dir(&probe).unwrap();
    if !enforced {
        eprintln!("SKIPPED: this user is not stopped by file permissions (root?)");
    }
    enforced
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The notes of moves that have not happened yet, in the session's state directory.
fn pending_records(session: &Session) -> usize {
    std::fs::read_dir(session.state_dir().join("pending-set-asides"))
        .map_or(0, |entries| entries.filter_map(Result::ok).count())
}

#[test]
fn a_folder_that_cannot_be_read_is_pending_and_never_nothing_to_move() {
    let session = two_pairs();
    if !permissions_are_enforced(session.dir.path()) {
        return;
    }
    let photos = session.dir.path().join("folders/photos");
    set_mode(&photos, 0o000);
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    set_mode(&photos, 0o755);

    let SetAsideReport::Pending {
        reason,
        record,
        still_at,
        message,
        ..
    } = &reply.set_aside
    else {
        panic!(
            "the history is still there and the reply said otherwise: {:?}",
            reply.set_aside
        )
    };
    assert!(reason.contains("cannot be read"), "{reason}");
    assert!(record.is_some(), "a note is kept so it can be tried again");
    assert!(still_at.is_empty(), "the app does not know what is there");
    assert!(message.contains("could not look"), "{message}");
    assert!(photos.join(".sync/sync_index.db").exists());
    assert!(!session.config().contains("name = \"photos\""));

    // Readable again: the next add or remove finishes it.
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    assert!(
        matches!(&reply.settled_earlier[..], [SetAsideReport::Moved { .. }]),
        "{:?}",
        reply.settled_earlier
    );
    assert!(!photos.join(".sync").exists());
    assert_eq!(pending_records(&session), 0);
}

#[test]
fn a_state_file_on_a_missing_drive_is_pending_and_keeps_a_note() {
    let session = session(|dir| {
        let docs = make_folder(dir, "docs", true);
        let photos = make_folder(dir, "photos", false);
        let usb = dir.join("usb-not-plugged-in");
        format!(
            "{}[[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\n\
             db_path = \"{}\"\nlockfile_path = \"{}\"\n",
            pair_text("docs", &docs),
            photos.display(),
            usb.join("idx.db").display(),
            usb.join("pair.lock").display()
        )
    });
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    let SetAsideReport::Pending { record, reason, .. } = &reply.set_aside else {
        panic!(
            "the index may be on the missing drive: {:?}",
            reply.set_aside
        )
    };
    assert!(reason.contains("usb-not-plugged-in"), "{reason}");
    assert!(record.is_some(), "a note is kept so it can be tried again");
    assert_eq!(pending_records(&session), 1);
}

#[test]
fn a_note_filed_for_an_unreadable_folder_does_not_claim_it_had_history_to_move() {
    let session = session(|dir| {
        let docs = make_folder(dir, "docs", true);
        let photos = make_folder(dir, "photos", false);
        format!(
            "{}{}",
            pair_text("docs", &docs),
            pair_text("photos", &photos)
        )
    });
    if !permissions_are_enforced(session.dir.path()) {
        return;
    }
    let photos = session.dir.path().join("folders/photos");
    set_mode(&photos, 0o000);
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    set_mode(&photos, 0o755);
    assert!(
        matches!(reply.set_aside, SetAsideReport::Pending { .. }),
        "{:?}",
        reply.set_aside
    );
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    let [SetAsideReport::NothingToMove { message, .. }] = &reply.settled_earlier[..] else {
        panic!("{:?}", reply.settled_earlier)
    };
    assert!(
        message.contains("can now be read and holds no history; nothing was moved"),
        "{message}"
    );
    assert!(!message.contains("had history to move"), "{message}");
    assert_eq!(pending_records(&session), 0);
}

#[test]
fn a_folder_that_is_not_there_is_pending_not_nothing_to_move() {
    // An unplugged drive. Its history is on it, and the app cannot see that.
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    let unplugged = session.dir.path().join("unplugged");
    std::fs::rename(&photos, &unplugged).unwrap();
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    let SetAsideReport::Pending { reason, record, .. } = &reply.set_aside else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(
        reason.contains(photos.to_str().unwrap()) && reason.contains("cannot be read"),
        "{reason}"
    );
    assert!(record.is_some());

    // The drive comes back; adding another folder finishes the move.
    std::fs::rename(&unplugged, &photos).unwrap();
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    assert!(
        matches!(&reply.settled_earlier[..], [SetAsideReport::Moved { .. }]),
        "{:?}",
        reply.settled_earlier
    );
    assert!(!photos.join(".sync").exists());
}

#[test]
fn a_relative_local_root_is_never_planned_and_the_removal_still_goes_ahead() {
    // Hand-written; `remove` has no #431 refusal. The daemon resolves it against its own directory and
    // this app against another, so the app cannot say which folder is meant.
    let session = session(|dir| {
        let docs = make_folder(dir, "docs", true);
        format!(
            "{}[[pair]]\nname = \"rel\"\nlocal_root = \"relative-sync-folder\"\n\
             remote_root = \"/Drive/rel\"\n",
            pair_text("docs", &docs)
        )
    });
    let reply = remove(&session, "rel").expect("the config removal goes ahead");
    assert!(!session.config().contains("relative-sync-folder"));
    let SetAsideReport::Pending {
        reason,
        record,
        message,
        ..
    } = &reply.set_aside
    else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(reason.contains("relative path"), "{reason}");
    assert!(reason.contains("local_root"), "{reason}");
    assert!(record.is_none(), "waiting does not make it absolute");
    assert!(
        message.contains("Look in that folder yourself"),
        "{message}"
    );
    assert_eq!(pending_records(&session), 0);
    assert!(
        !session.state_dir().join("removed-pairs").exists(),
        "nothing was moved"
    );
}

/// `docs` and `photos`, `photos` with its index in `ro-state/idx.db` — a directory the test can make
/// read-only to stop the second item of the move.
fn photos_with_its_index_elsewhere() -> (Session, PathBuf) {
    let mut ro_state = PathBuf::new();
    let session = session(|dir| {
        let docs = make_folder(dir, "docs", true);
        let photos = make_folder(dir, "photos", true);
        ro_state = dir.join("ro-state");
        std::fs::create_dir_all(&ro_state).unwrap();
        std::fs::write(ro_state.join("idx.db"), "the index").unwrap();
        format!(
            "{}[[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\n\
             db_path = \"{}\"\n",
            pair_text("docs", &docs),
            photos.display(),
            ro_state.join("idx.db").display()
        )
    });
    (session, ro_state)
}

#[test]
fn a_move_that_stops_part_way_names_what_moved_and_what_is_left() {
    let (session, ro_state) = photos_with_its_index_elsewhere();
    if !permissions_are_enforced(session.dir.path()) {
        return;
    }
    let photos = session.dir.path().join("folders/photos");
    set_mode(&ro_state, 0o555);
    let reply = remove(&session, "photos").expect("the removal itself succeeds");
    set_mode(&ro_state, 0o755);

    let SetAsideReport::Pending {
        moved,
        moved_to,
        still_at,
        record,
        message,
        ..
    } = &reply.set_aside
    else {
        panic!("{:?}", reply.set_aside)
    };
    // `.sync` did move, and the reply says where.
    assert!(!photos.join(".sync").exists());
    let moved_to = moved_to.as_ref().expect("where the first item went");
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(moved[0].from, photos.join(".sync").to_str().unwrap());
    assert!(moved[0].to.starts_with(moved_to.as_str()), "{moved:?}");
    assert!(
        std::path::Path::new(&moved[0].to)
            .join("sync_index.db")
            .exists(),
        "what the reply names is there"
    );
    // The index is what is left, and the reply says so.
    assert_eq!(still_at, &[ro_state.join("idx.db").to_str().unwrap()]);
    assert!(ro_state.join("idx.db").exists());
    assert!(record.is_some());
    assert!(message.contains("WAS moved"), "{message}");
    assert!(message.contains(moved_to.as_str()), "{message}");
    assert!(message.contains("idx.db"), "{message}");

    // Later, the rest goes the same way.
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    assert!(
        matches!(&reply.settled_earlier[..], [SetAsideReport::Moved { .. }]),
        "{:?}",
        reply.settled_earlier
    );
    assert!(!ro_state.join("idx.db").exists());
}

#[test]
fn a_state_directory_that_is_a_link_is_reported_as_a_link_that_moved() {
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    let real_state = session.dir.path().join("fast-disk/photos-state");
    std::fs::create_dir_all(real_state.parent().unwrap()).unwrap();
    std::fs::rename(photos.join(".sync"), &real_state).unwrap();
    std::os::unix::fs::symlink(&real_state, photos.join(".sync")).unwrap();

    let reply = remove(&session, "photos").expect("removes");
    let SetAsideReport::LinkMoved {
        left_behind,
        message,
        to,
        ..
    } = &reply.set_aside
    else {
        panic!(
            "only a link moved, and the reply said otherwise: {:?}",
            reply.set_aside
        )
    };
    assert_eq!(left_behind.len(), 1);
    assert_eq!(left_behind[0].target, real_state.to_str().unwrap());
    assert!(message.contains("LINK"), "{message}");
    assert!(message.contains(real_state.to_str().unwrap()), "{message}");
    assert!(message.contains(to.as_str()), "{message}");
    assert!(
        !message.contains("history of 'photos' was moved"),
        "{message}"
    );
    // The history is exactly where it was.
    assert_eq!(
        std::fs::read_to_string(real_state.join("sync_index.db")).unwrap(),
        "the index"
    );
    // On the wire it is its own outcome, so a screen cannot read it as `moved`.
    let wire = serde_json::to_value(&reply.set_aside).unwrap();
    assert_eq!(wire["outcome"], "link_moved");
}

#[test]
fn a_link_among_real_state_is_moved_as_a_link_and_the_reply_says_so() {
    // `.sync` is a real directory and moves whole; the pair's index is reached through a link, and
    // only the link moves. The outcome is `moved` (something real did), with the link named.
    let (mut link, mut real_db) = (PathBuf::new(), PathBuf::new());
    let session = session(|dir| {
        let (docs, photos) = (
            make_folder(dir, "docs", true),
            make_folder(dir, "photos", true),
        );
        real_db = dir.join("real-db/idx.db");
        std::fs::create_dir_all(real_db.parent().unwrap()).unwrap();
        std::fs::write(&real_db, "the real index").unwrap();
        link = dir.join("db-links/idx.db");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real_db, &link).unwrap();
        format!(
            "{}[[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\n\
             db_path = \"{}\"\n",
            pair_text("docs", &docs),
            photos.display(),
            link.display()
        )
    });
    let reply = remove(&session, "photos").expect("removes");
    let SetAsideReport::Moved {
        items,
        left_behind,
        message,
        ..
    } = &reply.set_aside
    else {
        panic!("{:?}", reply.set_aside)
    };
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(left_behind.len(), 1, "{left_behind:?}");
    assert_eq!(left_behind[0].link, link.to_str().unwrap());
    assert_eq!(left_behind[0].target, real_db.to_str().unwrap());
    assert!(message.contains("Part of it was a link"), "{message}");
    assert_eq!(
        std::fs::read_to_string(&real_db).unwrap(),
        "the real index",
        "what the link pointed at was not touched"
    );
    assert!(!session.dir.path().join("folders/photos/.sync").exists());
}

#[test]
fn a_note_whose_history_has_gone_is_dropped_and_said_so() {
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    let unplugged = session.dir.path().join("unplugged");
    std::fs::rename(&photos, &unplugged).unwrap();
    assert!(matches!(
        remove(&session, "photos").unwrap().set_aside,
        SetAsideReport::Pending { .. }
    ));
    assert_eq!(pending_records(&session), 1);

    // The drive comes back with the history gone (deleted by hand).
    std::fs::rename(&unplugged, &photos).unwrap();
    std::fs::remove_dir_all(photos.join(".sync")).unwrap();
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    let [SetAsideReport::NothingToMove { message, .. }] = &reply.settled_earlier[..] else {
        panic!("{:?}", reply.settled_earlier)
    };
    // The note was filed because the folder could not be read, so it listed nothing to move.
    assert!(
        message.contains("can now be read and holds no history; nothing was moved"),
        "{message}"
    );
    assert_eq!(pending_records(&session), 0);
}

#[test]
fn a_file_named_dot_sync_is_nothing_to_move_with_a_note_and_is_left_alone() {
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    std::fs::remove_dir_all(photos.join(".sync")).unwrap();
    std::fs::write(photos.join(".sync"), "MY NOTES").unwrap();

    let reply = remove(&session, "photos").expect("removes");
    let SetAsideReport::NothingToMove { notes, message, .. } = &reply.set_aside else {
        panic!("{:?}", reply.set_aside)
    };
    assert!(
        notes.iter().any(|note| note.contains("is a file")),
        "{notes:?}"
    );
    assert!(message.contains("is a file"), "{message}");
    assert!(
        message.contains(photos.to_str().unwrap()) && message.contains("could be read"),
        "\"nothing\" names the folder it was said of: {message}"
    );
    assert_eq!(
        std::fs::read_to_string(photos.join(".sync")).unwrap(),
        "MY NOTES"
    );
    assert_eq!(
        pending_records(&session),
        0,
        "nothing to wait for: it would never move"
    );
}

#[test]
fn a_note_that_names_a_file_of_the_persons_is_refused_when_it_is_retried() {
    let session = two_pairs();
    let thesis = session.dir.path().join("Documents/thesis.txt");
    std::fs::create_dir_all(thesis.parent().unwrap()).unwrap();
    std::fs::write(&thesis, "the thesis").unwrap();
    // A note for a pair whose identity is real and whose item list names the thesis.
    let ghost = session.folder("ghost", true);
    let view = gui_core::config_io::PairView {
        name: "ghost".to_owned(),
        local_root: Some(ghost.clone()),
        remote_root: None,
        db_path: Some(ghost.join(".sync/sync_index.db")),
        lockfile_path: Some(ghost.join(".sync/proton-sync.lock")),
        conflict_suffix: None,
    };
    let planted = gui_core::set_aside::Plan::of_view(&view, vec![thesis.clone()], Vec::new());
    gui_core::set_aside::record_pending(
        &session.state_dir(),
        &planted,
        "planted",
        SystemTime::now(),
    )
    .unwrap();

    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    let [SetAsideReport::Pending { reason, .. }] = &reply.settled_earlier[..] else {
        panic!("{:?}", reply.settled_earlier)
    };
    assert!(reason.contains("not part of the sync state"), "{reason}");
    assert_eq!(std::fs::read_to_string(&thesis).unwrap(), "the thesis");
    assert!(
        ghost.join(".sync/sync_index.db").exists(),
        "nothing moved at all"
    );
    assert_eq!(pending_records(&session), 1, "the note stays");
}

#[test]
fn a_refused_add_still_reports_the_earlier_removal_it_settled_first() {
    // The earlier removal has to be settled BEFORE the add is looked at (see the next test), so a
    // refused add can have moved a history. The refusal says so.
    let session = two_pairs();
    let lock = session
        .dir
        .path()
        .join("folders/photos/.sync/proton-sync.lock");
    let held = gui_core::testing::hold_lockfile(&lock);
    assert!(matches!(
        remove(&session, "photos").unwrap().set_aside,
        SetAsideReport::Pending { .. }
    ));
    drop(held);

    let music = session.folder("music", false);
    let failure = pair_admin::add_pair_file(
        &session.config_path(),
        Some(&session.state_dir()),
        "-h",
        &AddPairRequest {
            local_root: music.display().to_string(),
            remote_root: "/Drive/music".to_owned(),
            exclude: Vec::new(),
        },
        SystemTime::now(),
    )
    .expect_err("a name that starts with `-` is refused");
    assert!(
        matches!(&failure.settled_earlier[..], [SetAsideReport::Moved { .. }]),
        "{:?}",
        failure.settled_earlier
    );
    assert!(!session.dir.path().join("folders/photos/.sync").exists());

    // Through the command, the person is told in the one string they get.
    let session = two_pairs();
    let lock = session
        .dir
        .path()
        .join("folders/photos/.sync/proton-sync.lock");
    let held = gui_core::testing::hold_lockfile(&lock);
    remove(&session, "photos").unwrap();
    drop(held);
    let music = session.folder("music", false);
    let error = add(&session, "-h", &music, "/Drive/music").unwrap_err();
    assert!(error.contains("moved to"), "{error}");
    assert!(error.contains("earlier removal"), "{error}");
}

#[test]
fn a_folder_removed_while_held_and_added_back_starts_fresh_once_the_daemon_lets_go() {
    // THE D8 FLOW THAT IS EASY TO GET WRONG: the removal could not move the history (the daemon had
    // it), the daemon lets go, and the SAME folder is added back. The pending move must run before the
    // add — against a config that does not hold the folder — or it would find the folder configured
    // again and be dropped, and the new pair would resume the old index.
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    let lock = photos.join(".sync/proton-sync.lock");
    let held = gui_core::testing::hold_lockfile(&lock);
    let SetAsideReport::Pending { .. } = remove(&session, "photos").unwrap().set_aside else {
        panic!("held")
    };
    assert!(photos.join(".sync/sync_index.db").exists());
    drop(held);

    let reply = add(&session, "photos", &photos, "/Drive/photos").expect("adds it back");
    assert!(
        matches!(&reply.settled_earlier[..], [SetAsideReport::Moved { .. }]),
        "the earlier removal finished first: {:?}",
        reply.settled_earlier
    );
    assert!(
        reply.surviving_index.is_none(),
        "nothing is left for the new pair to resume: {:?}",
        reply.surviving_index
    );
    assert!(!photos.join(".sync/sync_index.db").exists());
    assert_eq!(pending_records(&session), 0);
    assert_eq!(
        std::fs::read_to_string(photos.join("mine.txt")).unwrap(),
        "photos's file"
    );
}

#[test]
fn a_folder_removed_while_held_and_added_back_while_still_held_is_told_it_will_resume_and_the_move_dropped(
) {
    let session = two_pairs();
    let photos = session.dir.path().join("folders/photos");
    let lock = photos.join(".sync/proton-sync.lock");
    let held = gui_core::testing::hold_lockfile(&lock);
    remove(&session, "photos").unwrap();

    // Still held at the add: the earlier move cannot run, and the add says what the new pair will do.
    let reply = add(&session, "photos", &photos, "/Drive/photos").expect("adds it back");
    assert!(
        matches!(&reply.settled_earlier[..], [SetAsideReport::Pending { .. }]),
        "{:?}",
        reply.settled_earlier
    );
    let survivor = reply.surviving_index.expect("the old index is named");
    assert!(
        survivor.set_aside_pending,
        "it is there because a removal has not finished: {survivor:?}"
    );
    assert!(
        survivor.message.contains("dropped rather than run"),
        "{}",
        survivor.message
    );
    assert_eq!(pending_records(&session), 1);

    // The next call finds the folder configured and drops the note without moving anything.
    drop(held);
    let music = session.folder("music", false);
    let reply = add(&session, "music", &music, "/Drive/music").unwrap();
    assert!(
        matches!(
            &reply.settled_earlier[..],
            [SetAsideReport::Superseded { .. }]
        ),
        "{:?}",
        reply.settled_earlier
    );
    assert_eq!(pending_records(&session), 0);
    assert!(photos.join(".sync/sync_index.db").exists());
}

#[test]
fn an_add_whose_folder_holds_the_state_directory_warns_and_still_adds() {
    // The app's set-aside histories live under its state directory. A sync folder that contains that
    // directory would upload them, so the add says so; it is not refused, because a folder that holds
    // the whole home directory is a legitimate choice.
    let session = one_pair();
    let broad = session.folder("broad", false);
    session.state().lock().unwrap().state_dir = Some(broad.join("state-inside"));
    let reply = add(&session, "broad", &broad, "/Drive/broad").expect("added");
    assert_eq!(reply.warnings.len(), 1, "{:?}", reply.warnings);
    assert!(
        reply.warnings[0].contains("removed-pairs"),
        "{}",
        reply.warnings[0]
    );
    assert!(
        reply.warnings[0].contains("upload"),
        "{}",
        reply.warnings[0]
    );
    assert!(session.config().contains("name = \"broad\""));

    // A folder that does not contain it says nothing.
    let session = one_pair();
    let other = session.folder("other", false);
    let reply = add(&session, "other", &other, "/Drive/other").unwrap();
    assert!(reply.warnings.is_empty(), "{:?}", reply.warnings);
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
