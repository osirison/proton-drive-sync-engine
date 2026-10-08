//! What the pair argument may and may not change (#102 phase 5a, ADR 0005).
//!
//! **The standard these hold the commands to is a recording, not a description.** Every request each
//! command sent at `4e277ab` — before any command could name a pair — is stored byte for byte in
//! `testdata/request-goldens.json`, captured by driving that commit's commands against the same
//! recording fake daemon (`gui_core::testing`) these tests use. A command that sends anything else
//! to a one-pair setup, or to the default pair of a many-pair one, has changed behaviour for
//! everyone who never asked for the feature.
//!
//! Nothing here reaches the machine. The config is resolved at a file in a temp directory, the
//! socket is the fake's, and the child `proton-syncd --dry-run` is a function the test supplies
//! (`LaunchChild`) — never the binary on `PATH`, which is signed in to a real Proton account.

use super::*;
use gui_core::testing::{FakeDaemon, FakePair};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;

/// The requests `4e277ab` sent, per command, as the recording fake logged them.
const GOLDENS: &str = include_str!("../../testdata/request-goldens.json");

fn goldens() -> BTreeMap<String, Vec<String>> {
    serde_json::from_str(GOLDENS).expect("the goldens file is a map of command -> request lines")
}

/// A mock app whose config lives in a temp directory and whose socket is `daemon`'s.
struct Harness {
    app: tauri::App<tauri::test::MockRuntime>,
    daemon: FakeDaemon,
    // Held for its `Drop`: the config file lives in it.
    _dir: tempfile::TempDir,
}

fn harness(daemon: FakeDaemon, config: Option<&str>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("proton-sync.toml");
    if let Some(config) = config {
        std::fs::write(&config_path, config).unwrap();
    }
    let mut paths = RuntimePaths::resolve_at(&config_path);
    paths.socket_path = Ok(daemon.socket_path().to_owned());
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");
    Harness {
        app,
        daemon,
        _dir: dir,
    }
}

impl Harness {
    fn state(&self) -> State<'_, Mutex<RuntimePaths>> {
        self.app.state::<Mutex<RuntimePaths>>()
    }
}

/// The one-pair config whose roots are the fake's own, so a dry run is answered by the daemon and
/// never by a child.
const ONE_PAIR_FILE: &str =
    "local_root = \"/fake/default/local\"\nremote_root = \"/Drive/default\"\n";

/// Two pairs, `docs` first (so it is the default pair) and `photos` second.
const TWO_PAIR_FILE: &str = "\
[[pair]]
name = \"docs\"
local_root = \"/fake/docs/local\"
remote_root = \"/Drive/docs\"

[[pair]]
name = \"photos\"
local_root = \"/fake/photos/local\"
remote_root = \"/Drive/photos\"
";

fn two_pair_daemon() -> FakeDaemon {
    FakeDaemon::multi_pair(vec![FakePair::new("docs"), FakePair::new("photos")]).start()
}

/// The `run_dry_run` child must never start in these tests; reaching it is a failure of the test and
/// not a side effect on the machine.
fn refuse_to_launch(_: &[OsString]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::other(
        "the child proton-syncd must not be launched by a test that expects the daemon to answer",
    ))
}

/// Drive every command the goldens cover, with `pair` as the frontend of the future would pass it,
/// and return what each sent. Order matters in exactly one place: `get_status` first, so the roots
/// the daemon reports are cached before the dry run decides who plans.
fn drive(h: &Harness, pair: Option<&str>) -> BTreeMap<String, Vec<String>> {
    let handle = || h.app.handle().clone();
    let pair = || pair.map(str::to_owned);
    let mut sent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    macro_rules! case {
        ($name:expr, $body:expr) => {{
            h.daemon.clear_requests();
            let result = $body;
            sent.insert($name.to_owned(), h.daemon.requests());
            result
        }};
    }
    macro_rules! run {
        ($future:expr) => {
            tauri::async_runtime::block_on($future)
        };
    }

    case!("get_status", run!(get_status(handle(), pair())));
    case!("pause", run!(pause(handle(), pair())));
    case!("resume", run!(resume(handle(), pair())));
    case!("sync_now", run!(sync_now(handle(), pair())));
    case!("resync", run!(resync(handle(), pair())));
    case!(
        "approve_path_literal",
        run!(approve(
            handle(),
            "docs/a.txt".to_owned(),
            true,
            None,
            pair()
        ))
    );
    case!(
        "approve_all_with_direction",
        run!(approve(
            handle(),
            "all".to_owned(),
            false,
            Some(DeleteDirection::Remote),
            pair()
        ))
    );
    case!(
        "deny_path_literal",
        run!(deny(handle(), "docs/a.txt".to_owned(), true, pair()))
    );
    case!(
        "keep_path_literal",
        run!(keep(handle(), "docs/a.txt".to_owned(), true, pair()))
    );
    assert!(case!(
        "list_pending_deletions",
        run!(list_pending_deletions(handle(), pair()))
    )
    .is_ok());
    let dry_run = case!(
        "run_dry_run_through_the_daemon",
        run!(run_dry_run_with(&h.state(), pair(), refuse_to_launch))
    );
    assert!(
        dry_run.is_ok(),
        "the daemon was there to plan, so the child was never needed: {:?}",
        dry_run.err()
    );
    for (name, skip) in [
        ("apply_plan", false),
        ("apply_plan_skipping_destructive", true),
    ] {
        let outcome = case!(
            name,
            run!(apply_plan(handle(), "tok".to_owned(), skip, pair()))
        );
        assert!(
            matches!(outcome, Ok(ApplyOutcome::Applied { .. })),
            "{name}: {outcome:?}"
        );
    }
    sent
}

/// Acceptance 1 and the default half of acceptance 2. The same recorded requests come out of every
/// one-pair setup, every daemon shape, and the default pair of a many-pair one — named or not.
#[test]
fn every_command_sends_the_request_it_sent_at_4e277ab() {
    let golden = goldens();
    assert_eq!(
        golden.len(),
        13,
        "the goldens cover thirteen request shapes"
    );

    // (what is running, what the config file says, which pair the caller names)
    let setups: Vec<(&str, Harness, Option<&str>)> = vec![
        (
            "a legacy daemon and no config file at all",
            harness(FakeDaemon::legacy(FakePair::new("default")).start(), None),
            None,
        ),
        (
            "a legacy daemon and a one-pair file",
            harness(
                FakeDaemon::legacy(FakePair::new("default")).start(),
                Some(ONE_PAIR_FILE),
            ),
            None,
        ),
        (
            "a legacy daemon, the default pair named explicitly",
            harness(
                FakeDaemon::legacy(FakePair::new("default")).start(),
                Some(ONE_PAIR_FILE),
            ),
            Some("default"),
        ),
        (
            "a current daemon running one pair",
            harness(
                FakeDaemon::multi_pair(vec![FakePair::new("default")]).start(),
                Some(ONE_PAIR_FILE),
            ),
            None,
        ),
        (
            "two pairs, none named: the default pair",
            harness(two_pair_daemon(), Some(TWO_PAIR_FILE)),
            None,
        ),
        (
            "two pairs, the default pair named: omitted all the same",
            harness(two_pair_daemon(), Some(TWO_PAIR_FILE)),
            Some("docs"),
        ),
    ];
    for (what, h, pair) in &setups {
        let sent = drive(h, *pair);
        for (command, lines) in &golden {
            assert_eq!(
                sent.get(command),
                Some(lines),
                "`{command}` against {what} must send what it sent at 4e277ab"
            );
        }
        assert_eq!(
            sent.len(),
            golden.len(),
            "{what}: an unrecorded command ran"
        );
    }
}

/// Acceptance 2, the other half: a NON-default pair is named on the wire, and **nothing else about
/// the request changes** — the same fields, the same order of requests, the same polling.
#[test]
fn a_non_default_pair_is_named_on_the_wire_and_nothing_else_changes() {
    let golden = goldens();
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let sent = drive(&h, Some("photos"));
    for (command, lines) in &golden {
        let expected: Vec<Value> = lines
            .iter()
            .map(|line| {
                let mut request: Value = serde_json::from_str(line).unwrap();
                request["pair"] = Value::String("photos".to_owned());
                request
            })
            .collect();
        let actual: Vec<Value> = sent[command]
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            actual, expected,
            "`{command}` for `photos` must be its recorded request with `pair` set, and nothing else"
        );
    }
}

/// The pure chain behind "selection photos": a remembered choice is validated against what exists,
/// and only a non-default pair becomes a selector.
#[test]
fn a_remembered_pair_reaches_the_wire_only_when_it_is_not_the_default() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let paths = h.state();
    let paths = paths.lock().unwrap();
    let known = paths.known_pair_names();
    for (saved, selector) in [
        (Some("photos"), Some("photos")),
        (Some("docs"), None),
        // Gone, or never saved: the default pair, which is addressed by omission.
        (Some("deleted-since"), None),
        (None, None),
    ] {
        let selected = gui_core::pairs::resolve_selection(saved, &known).unwrap();
        let reference = paths.resolve_pair(Some(selected)).unwrap();
        assert_eq!(reference.selector.as_deref(), selector, "saved = {saved:?}");
        assert_eq!(reference.name, selected);
    }
}

/// A pair this app cannot place is refused **before anything is sent**, by every command that takes
/// one: reading it as the default would act on the wrong folder.
#[test]
fn a_pair_this_app_cannot_place_sends_nothing() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    let unknown = || Some("not-a-pair".to_owned());
    macro_rules! run {
        ($future:expr) => {
            tauri::async_runtime::block_on($future)
        };
    }
    let refused_status = |payload: StatusPayload, what: &str| {
        let error = payload
            .error
            .unwrap_or_else(|| panic!("{what} was not refused"));
        assert!(error.contains("not-a-pair"), "{what}: {error}");
    };

    refused_status(run!(get_status(handle(), unknown())), "get_status");
    refused_status(run!(pause(handle(), unknown())), "pause");
    refused_status(run!(resume(handle(), unknown())), "resume");
    refused_status(run!(sync_now(handle(), unknown())), "sync_now");
    refused_status(run!(resync(handle(), unknown())), "resync");
    refused_status(
        run!(approve(handle(), "a".into(), true, None, unknown())),
        "approve",
    );
    refused_status(run!(deny(handle(), "a".into(), true, unknown())), "deny");
    refused_status(run!(keep(handle(), "a".into(), true, unknown())), "keep");
    assert!(run!(list_pending_deletions(handle(), unknown())).is_err());
    assert!(run!(apply_plan(handle(), "t".into(), false, unknown())).is_err());
    assert!(run!(run_dry_run_with(&h.state(), unknown(), refuse_to_launch)).is_err());
    assert!(run!(scan_conflicts(h.state(), unknown())).is_err());
    assert!(run!(open_folder(h.state(), "a".into(), unknown())).is_err());
    assert!(run!(open_paths(h.state(), vec!["a".into()], unknown())).is_err());
    assert!(run!(path_sync_status(h.state(), "a".into(), unknown())).is_err());
    assert!(run!(search_files(handle(), "a".into(), None, unknown())).is_err());
    assert!(run!(free_space(handle(), None, unknown())).is_err());
    assert!(run!(skip_rule_usage(handle(), vec![], None, unknown())).is_err());
    let conflict = Conflict {
        original: "a".into(),
        sidecar: "a.x".into(),
        kind: conflicts::ConflictKind::Content,
    };
    assert!(
        resolve_conflict(h.state(), conflict.clone(), Resolution::KeepMine, unknown()).is_err()
    );
    assert!(read_conflict_pair(h.state(), conflict, unknown()).is_err());

    assert!(
        h.daemon.requests().is_empty(),
        "nothing may leave for a pair that cannot be placed: {:?}",
        h.daemon.requests()
    );
}

/// The class R commands read the addressed pair's folder, index and sidecar spelling — each pair
/// its own — and a command naming nothing reads the default pair's.
#[test]
fn class_r_commands_read_the_addressed_pairs_folder_and_naming() {
    let docs = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    // `docs` writes `.beta` sidecars and `photos` writes `.alpha` ones. Each folder holds a conflict
    // of its own AND one spelled the other pair's way, which is an ordinary file under its own rules.
    let write = |root: &std::path::Path, name: &str| std::fs::write(root.join(name), "x").unwrap();
    write(docs.path(), "report.txt");
    write(docs.path(), "report.beta.txt");
    write(docs.path(), "memo.txt");
    write(docs.path(), "memo.alpha.txt");
    write(photos.path(), "trip.jpg");
    write(photos.path(), "trip.alpha.jpg");
    write(photos.path(), "raw.jpg");
    write(photos.path(), "raw.beta.jpg");
    let config = format!(
        "[[pair]]\nname = \"docs\"\nlocal_root = {docs:?}\nremote_root = \"/Drive/docs\"\n\
         db_path = {docs_db:?}\nconflict_suffix = \"beta\"\n\
         [[pair]]\nname = \"photos\"\nlocal_root = {photos:?}\nremote_root = \"/Drive/photos\"\n\
         db_path = {photos_db:?}\nconflict_suffix = \"alpha\"\n",
        docs = docs.path().display().to_string(),
        photos = photos.path().display().to_string(),
        docs_db = docs.path().join("docs-index.db").display().to_string(),
        photos_db = photos.path().join("photos-index.db").display().to_string(),
    );
    let h = harness(two_pair_daemon(), Some(&config));
    let scan = |pair: Option<&str>| -> Vec<String> {
        tauri::async_runtime::block_on(scan_conflicts(h.state(), pair.map(str::to_owned)))
            .expect("the scan runs")
            .into_iter()
            .map(|c| c.sidecar.display().to_string())
            .collect()
    };
    assert_eq!(scan(Some("docs")), ["report.beta.txt"]);
    assert_eq!(scan(Some("photos")), ["trip.alpha.jpg"]);
    // Naming nothing is the default pair, `docs`.
    assert_eq!(scan(None), ["report.beta.txt"]);

    // The index a lookup opens is the addressed pair's: the failure names ITS file.
    let status_error = |pair: &str| {
        tauri::async_runtime::block_on(path_sync_status(
            h.state(),
            "x".into(),
            Some(pair.to_owned()),
        ))
        .err()
        .expect("no index exists there")
    };
    assert!(
        status_error("docs").contains("docs-index.db"),
        "{}",
        status_error("docs")
    );
    assert!(
        status_error("photos").contains("photos-index.db"),
        "{}",
        status_error("photos")
    );
}

/// A reply about one pair never repaints another's slot through the command layer either: the
/// cache is keyed by the pair the reply is about.
#[test]
fn a_status_reply_for_one_pair_leaves_the_others_cached_roots_alone() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    tauri::async_runtime::block_on(get_status(handle(), Some("photos".to_owned())));
    let paths = h.state();
    let paths = paths.lock().unwrap();
    assert_eq!(
        paths.reported("docs").map(|p| p.local_root.clone()),
        Some(std::path::PathBuf::from("/fake/docs/local"))
    );
    assert_eq!(
        paths.reported("photos").map(|p| p.local_root.clone()),
        Some(std::path::PathBuf::from("/fake/photos/local"))
    );
}

// ---- the child `proton-syncd --dry-run` -----------------------------------------------------------

/// Inputs for a dry run of pair `name`, built by hand. The three file roots and the three reported
/// ones are separate arguments' worth of facts, kept apart exactly as production keeps them.
fn inputs(
    name: &str,
    tables: bool,
    file: (Option<&str>, Option<&str>, Option<&str>),
    daemon: (Option<&str>, Option<&str>, Option<&str>),
) -> DryRunInputs {
    let path = |value: Option<&str>| value.map(std::path::PathBuf::from);
    DryRunInputs {
        socket: Err("no daemon in this test".to_owned()),
        config_path: std::path::PathBuf::from("/cfg/proton-sync.toml"),
        pair: PairRef {
            name: name.to_owned(),
            selector: None,
        },
        pair_tables: tables,
        file_local: path(file.0),
        file_remote: path(file.1),
        file_db: path(file.2),
        daemon_local: path(daemon.0),
        daemon_remote: path(daemon.1),
        daemon_db: path(daemon.2),
    }
}

fn strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Acceptance 4. A `[[pair]]` file is previewed by NAME, and the root flags are never passed: the
/// engine refuses them beside several pairs, which is what `proton-syncd --dry-run failed: …` was.
#[test]
fn a_pair_file_is_previewed_by_name_and_never_with_root_flags() {
    // The daemon has reported roots — the very values that used to be handed to the child as flags.
    let args = dry_run_args(
        &inputs(
            "photos",
            true,
            (None, None, None),
            (
                Some("/l/photos"),
                Some("/Drive/photos"),
                Some("/l/photos.db"),
            ),
        ),
        true,
    )
    .unwrap();
    assert_eq!(
        strings(&args),
        [
            "--dry-run",
            "--config",
            "/cfg/proton-sync.toml",
            "--pair",
            "photos"
        ]
    );
    for flag in ["--local-root", "--remote-root", "--db-path"] {
        assert!(!strings(&args).contains(&flag.to_owned()), "{flag}");
    }
}

/// And the implicit one-pair file keeps the flag-filling it always had — byte for byte, since a
/// one-pair setup is the case nothing may change for.
#[test]
fn an_implicit_pair_file_keeps_filling_the_slots_it_leaves_empty() {
    let reported = (Some("/l/d"), Some("/Drive/d"), Some("/l/d.db"));
    // The file says nothing: all three come from the daemon.
    assert_eq!(
        strings(
            &dry_run_args(
                &inputs("default", false, (None, None, None), reported),
                true
            )
            .unwrap()
        ),
        [
            "--dry-run",
            "--config",
            "/cfg/proton-sync.toml",
            "--local-root",
            "/l/d",
            "--remote-root",
            "/Drive/d",
            "--db-path",
            "/l/d.db"
        ]
    );
    // The file speaks for the roots: only the index is filled in.
    assert_eq!(
        strings(
            &dry_run_args(
                &inputs(
                    "default",
                    false,
                    (Some("/f"), Some("/Drive/f"), None),
                    reported
                ),
                true
            )
            .unwrap()
        ),
        [
            "--dry-run",
            "--config",
            "/cfg/proton-sync.toml",
            "--db-path",
            "/l/d.db"
        ]
    );
    // No file, but a daemon that told us its roots.
    assert_eq!(
        strings(
            &dry_run_args(
                &inputs("default", false, (None, None, None), reported),
                false
            )
            .unwrap()
        ),
        [
            "--dry-run",
            "--local-root",
            "/l/d",
            "--remote-root",
            "/Drive/d",
            "--db-path",
            "/l/d.db"
        ]
    );
    // Neither: nothing to preview, and it says so.
    let error = dry_run_args(
        &inputs("default", false, (None, None, None), (None, None, None)),
        false,
    )
    .unwrap_err();
    assert!(
        error.contains("set the folders in Settings first"),
        "{error}"
    );
}

/// The wiring behind acceptance 4: with no daemon to ask, a many-pair config is previewed through
/// the child with exactly those arguments. The launcher is the test's own; nothing is spawned.
#[test]
fn a_dry_run_with_no_daemon_hands_the_child_the_pair_name() {
    static LAUNCHED: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
    fn record(args: &[OsString]) -> std::io::Result<std::process::Output> {
        LAUNCHED.lock().unwrap().push(strings(args));
        let report = gui_core::wire::DryRunReport {
            summary: Default::default(),
            plan: Vec::new(),
            cannot_sync: Vec::new(),
        };
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: serde_json::to_vec(&report).unwrap(),
            stderr: Vec::new(),
        })
    }

    // The fake daemon exists only to own a socket path; it is stopped, so nothing answers it.
    let daemon = two_pair_daemon();
    let socket = daemon.socket_path().to_owned();
    drop(daemon);
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("proton-sync.toml");
    std::fs::write(&config_path, TWO_PAIR_FILE).unwrap();
    let mut paths = RuntimePaths::resolve_at(&config_path);
    paths.socket_path = Ok(socket);
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");

    let payload = tauri::async_runtime::block_on(run_dry_run_with(
        &app.state::<Mutex<RuntimePaths>>(),
        Some("photos".to_owned()),
        record,
    ))
    .expect("the canned report parses");
    assert!(payload.token.is_none(), "a child's plan has no token");

    let launched = LAUNCHED.lock().unwrap();
    let mine: Vec<_> = launched
        .iter()
        .filter(|args| args.contains(&config_path.display().to_string()))
        .collect();
    assert_eq!(mine.len(), 1, "one child, for one preview: {launched:?}");
    assert_eq!(
        mine[0],
        &[
            "--dry-run".to_owned(),
            "--config".to_owned(),
            config_path.display().to_string(),
            "--pair".to_owned(),
            "photos".to_owned(),
        ]
    );
}

// ---- the two remote roots ----------------------------------------------------------------------

/// Acceptance 6. `daemon_plans_the_same_roots` has to tell "the file says X" from "the daemon says
/// X", which is why there are two getters per pair and **no merged one**. A fixture where they
/// disagree must keep disagreeing all the way to the decision.
#[test]
fn the_two_remote_roots_stay_two() {
    let daemon = FakeDaemon::multi_pair(vec![FakePair::with_roots(
        "default",
        std::path::Path::new("/fake/default/local"),
        std::path::Path::new("/Drive/Reported"),
        std::path::Path::new("/fake/default/local/.sync/sync_index.db"),
    )])
    .start();
    let h = harness(
        daemon,
        Some("local_root = \"/fake/default/local\"\nremote_root = \"/Drive/Configured\"\n"),
    );
    tauri::async_runtime::block_on(get_status(h.app.handle().clone(), None));
    let paths = h.state();
    let paths = paths.lock().unwrap();

    let configured = paths.pair("default").unwrap().remote_root.clone();
    let reported = paths.reported("default").map(|p| p.remote_root.clone());
    assert_eq!(
        configured,
        Some(std::path::PathBuf::from("/Drive/Configured"))
    );
    assert_eq!(reported, Some(std::path::PathBuf::from("/Drive/Reported")));

    // The decision `run_dry_run` makes from them: the file points somewhere the daemon is NOT
    // running, so the daemon must not be asked to plan it.
    let pair = paths.resolve_pair(None).unwrap();
    let inputs = DryRunInputs::read(&paths, pair);
    assert!(!daemon_plans_the_same_roots(
        inputs.file_local.as_deref(),
        inputs.file_remote.as_deref(),
        inputs.daemon_local.as_deref(),
        inputs.daemon_remote.as_deref(),
    ));

    // And no third resolver exists. This is a source scan because absence cannot be compiled: the
    // moment someone writes one, "which of the two answered" has an answer nobody can read.
    let forbidden = ["fn effective", "_remote_root"].concat();
    for (file, source) in [
        ("config_path.rs", include_str!("../config_path.rs")),
        ("commands.rs", include_str!("../commands.rs")),
    ] {
        let offending: Vec<&str> = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains(&forbidden))
            .collect();
        assert!(
            offending.is_empty(),
            "{file} merges the configured and the reported remote root: {offending:?}"
        );
    }
}

// ---- classification ------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A single-shot read: a wrong pair costs a wrong screen. Takes `pair: Option<String>`.
    R,
    /// A write, a deletion, or the start of a multi-step flow: a wrong pair costs data. Takes
    /// `pair: Option<String>` (required from 5a-2, when the frontend can name one).
    W,
    /// Pair-bound, but addressed by something else until the PR named here.
    Deferred(&'static str),
    /// About the process, the daemon as a whole, or the window — no folder pair is involved.
    Independent,
}

/// Every `#[tauri::command]` and what it is. Adding a command without adding it here fails
/// `every_pair_slot_command_names_its_class`, which is the point: a command that reads a pair slot
/// must say how it takes the pair.
const COMMAND_CLASSES: [(&str, Class); 38] = [
    ("get_status", Class::R),
    ("list_pending_deletions", Class::R),
    ("scan_conflicts", Class::R),
    ("read_conflict_pair", Class::R),
    ("path_sync_status", Class::R),
    ("search_files", Class::R),
    ("skip_rule_usage", Class::R),
    ("free_space", Class::R),
    ("open_paths", Class::R),
    ("open_folder", Class::R),
    ("pause", Class::W),
    ("resume", Class::W),
    ("sync_now", Class::W),
    ("resync", Class::W),
    ("approve", Class::W),
    ("deny", Class::W),
    ("keep", Class::W),
    ("apply_plan", Class::W),
    ("run_dry_run", Class::W),
    ("resolve_conflict", Class::W),
    ("read_config", Class::Deferred("5b-1: read_config(pair)")),
    (
        "write_config",
        Class::Deferred("5b-1: write_config(pair, update)"),
    ),
    (
        "tray_action",
        Class::Deferred("5d: pair-carrying tray rows"),
    ),
    ("choose_folder", Class::Independent),
    ("start_service", Class::Independent),
    ("restart_service", Class::Independent),
    ("send_notification", Class::Independent),
    ("close_notification", Class::Independent),
    ("read_notify_policy", Class::Independent),
    ("write_notify_policy", Class::Independent),
    ("check_cli", Class::Independent),
    ("probe_folder", Class::Independent),
    ("open_remote", Class::Independent),
    ("open_system_log", Class::Independent),
    ("close_window", Class::Independent),
    ("quit_app", Class::Independent),
    ("resize_tray_panel", Class::Independent),
    ("hide_tray_panel", Class::Independent),
];

/// The names of every function the source marks `#[tauri::command]`, with its signature text.
fn commands_in(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() != "#[tauri::command]" {
            continue;
        }
        // The signature runs from `fn` to the body's opening brace — the first `{`, since a
        // command's generics and arguments carry none.
        let mut text = String::new();
        for line in lines.by_ref() {
            text.push_str(line);
            text.push('\n');
            if line.contains('{') {
                break;
            }
        }
        let name = text
            .split("fn ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '<']).next())
            .expect("a command is a function")
            .trim()
            .to_owned();
        found.push((name, text));
    }
    found
}

#[test]
fn every_pair_slot_command_names_its_class() {
    let found = commands_in(include_str!("../commands.rs"));
    let mut names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    let mut classified: Vec<&str> = COMMAND_CLASSES.iter().map(|(name, _)| *name).collect();
    classified.sort_unstable();
    assert_eq!(
        names, classified,
        "every #[tauri::command] must be in COMMAND_CLASSES, and every row must be a command"
    );

    // Each class means something about the signature, so the table cannot drift from the code.
    for (name, signature) in &found {
        let class = COMMAND_CLASSES
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, class)| *class)
            .unwrap();
        let takes_pair = signature.contains("pair: Option<String>");
        match class {
            Class::R | Class::W => {
                assert!(
                    takes_pair,
                    "`{name}` is class {class:?} and must take `pair: Option<String>`"
                )
            }
            Class::Deferred(_) | Class::Independent => assert!(
                !takes_pair,
                "`{name}` is {class:?} and must not take a pair argument yet"
            ),
        }
    }

    // And the registry: a command that is in `generate_handler!` is in the table.
    let registered: Vec<&str> = include_str!("../lib.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("commands::"))
        .filter_map(|rest| rest.strip_suffix(','))
        .collect();
    let mut registered = registered;
    registered.sort_unstable();
    assert_eq!(
        registered, classified,
        "lib.rs registers a different set of commands"
    );
}

// ---- the config path a session was given ---------------------------------------------------------

/// Three production sites used to ask the ENVIRONMENT where the config was — `write_config`'s
/// re-resolve, `start_service_and_adopt`'s adoption and `restart_service`'s — and every one of them
/// therefore read the developer's real config in a test, and wrote it on the next save. Each now
/// re-resolves at the path the session holds. The file this test names is the only one that has
/// the values asserted below; the environment's has none of them.
///
/// Safe by construction: `restart_service(true)` against a socket nobody listens on returns before
/// any `systemctl` (`docs/agent-notes/gui-tests-that-shell-systemctl.md`), and the shared adoption
/// is exercised directly rather than through `start_service`, which would start the real unit.
#[test]
fn write_config_restart_and_start_keep_the_config_path_they_were_given() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("session.toml");
    let adopted = dir.path().join("adopted.sock");
    let dead = dir.path().join("dead.sock");
    std::fs::write(
        &config_path,
        format!("socket_path = \"{}\"\n", adopted.display()),
    )
    .unwrap();
    let mut paths = RuntimePaths::resolve_at(&config_path);
    paths.socket_path = Ok(dead.clone());
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");
    let state = app.state::<Mutex<RuntimePaths>>();

    // 1. A save re-resolves at the file it wrote. `proton_cli` is the witness: only this file says
    //    `/fake/cli`, so a re-resolve anywhere else cannot produce it.
    write_config(
        app.state::<Mutex<RuntimePaths>>(),
        ConfigUpdate {
            proton_cli: Some("/fake/cli".to_owned()),
            ..Default::default()
        },
    )
    .expect("a valid update saves");
    {
        let after = state.lock().unwrap();
        assert_eq!(after.config_path, config_path);
        assert_eq!(
            after.proton_cli, "/fake/cli",
            "write_config re-read another file"
        );
        assert_eq!(
            after.socket_path.as_deref().ok(),
            Some(dead.as_path()),
            "and still left the socket for the restart that reads it next (#336)"
        );
    }

    // 2. The adoption `start_service_and_adopt` ends in.
    apply_socket_adoption(&state, true);
    assert_eq!(
        state.lock().unwrap().socket_path.as_deref().ok(),
        Some(adopted.as_path()),
        "the start's adoption read another file"
    );

    // 3. The adoption `restart_service` ends in. Back to the dead address first, so a restart that
    //    adopted nothing would show.
    state.lock().unwrap().socket_path = Ok(dead.clone());
    let outcome =
        tauri::async_runtime::block_on(restart_service(app.state::<Mutex<RuntimePaths>>(), true))
            .expect("a dead socket is not a failed restart");
    assert_eq!(outcome, RestartOutcome::NotRunning);
    assert_eq!(
        state.lock().unwrap().socket_path.as_deref().ok(),
        Some(adopted.as_path()),
        "the restart's adoption read another file"
    );
}
