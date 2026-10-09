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
pub(super) struct Harness {
    pub(super) app: tauri::App<tauri::test::MockRuntime>,
    pub(super) daemon: FakeDaemon,
    // Held for its `Drop`: the config file lives in it.
    pub(super) _dir: tempfile::TempDir,
}

pub(super) fn harness(daemon: FakeDaemon, config: Option<&str>) -> Harness {
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
    pub(super) fn state(&self) -> State<'_, Mutex<RuntimePaths>> {
        self.app.state::<Mutex<RuntimePaths>>()
    }
}

/// The one-pair config whose roots are the fake's own, so a dry run is answered by the daemon and
/// never by a child.
const ONE_PAIR_FILE: &str =
    "local_root = \"/fake/default/local\"\nremote_root = \"/Drive/default\"\n";

/// Two pairs, `docs` first (so it is the default pair) and `photos` second.
pub(super) const TWO_PAIR_FILE: &str = "\
[[pair]]
name = \"docs\"
local_root = \"/fake/docs/local\"
remote_root = \"/Drive/docs\"

[[pair]]
name = \"photos\"
local_root = \"/fake/photos/local\"
remote_root = \"/Drive/photos\"
";

pub(super) fn two_pair_daemon() -> FakeDaemon {
    FakeDaemon::multi_pair(vec![FakePair::new("docs"), FakePair::new("photos")]).start()
}

/// The `run_dry_run` child must never start in these tests; reaching it is a failure of the test and
/// not a side effect on the machine.
pub(super) fn refuse_to_launch(_: &[OsString]) -> std::io::Result<std::process::Output> {
    Err(std::io::Error::other(
        "the child proton-syncd must not be launched by a test that expects the daemon to answer",
    ))
}

/// Drive every command the goldens cover, with `pair` as the frontend would pass it, and return what
/// each sent. Order matters in exactly one place: `get_status` first, so the roots the daemon
/// reports are cached before the dry run decides who plans.
///
/// A class-R command is handed `pair` as it is (naming none means the selection). A class-W command
/// REQUIRES a name, so it gets `pair` or, when the caller names none, `default` — the default pair's
/// own name, which the wire still omits.
fn drive(h: &Harness, pair: Option<&str>, default: &str) -> BTreeMap<String, Vec<String>> {
    let handle = || h.app.handle().clone();
    let named = || pair.unwrap_or(default).to_owned();
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
    case!("pause", run!(pause(handle(), named())));
    case!("resume", run!(resume(handle(), named())));
    case!("sync_now", run!(sync_now(handle(), named())));
    case!("resync", run!(resync(handle(), named())));
    case!(
        "approve_path_literal",
        run!(approve(
            handle(),
            "docs/a.txt".to_owned(),
            true,
            None,
            named()
        ))
    );
    case!(
        "approve_all_with_direction",
        run!(approve(
            handle(),
            "all".to_owned(),
            false,
            Some(DeleteDirection::Remote),
            named()
        ))
    );
    case!(
        "deny_path_literal",
        run!(deny(handle(), "docs/a.txt".to_owned(), true, named()))
    );
    case!(
        "keep_path_literal",
        run!(keep(handle(), "docs/a.txt".to_owned(), true, named()))
    );
    assert!(case!(
        "list_pending_deletions",
        run!(list_pending_deletions(handle(), pair()))
    )
    .is_ok());
    let dry_run = case!(
        "run_dry_run_through_the_daemon",
        run!(run_dry_run_with(&h.state(), named(), refuse_to_launch))
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
            run!(apply_plan(handle(), "tok".to_owned(), skip, named()))
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

    // (what is running, what the config file says, which pair the caller names, the default pair's name)
    let setups: Vec<(&str, Harness, Option<&str>, &str)> = vec![
        (
            "a legacy daemon and no config file at all",
            harness(FakeDaemon::legacy(FakePair::new("default")).start(), None),
            None,
            "default",
        ),
        (
            "a legacy daemon and a one-pair file",
            harness(
                FakeDaemon::legacy(FakePair::new("default")).start(),
                Some(ONE_PAIR_FILE),
            ),
            None,
            "default",
        ),
        (
            "a legacy daemon, the default pair named explicitly",
            harness(
                FakeDaemon::legacy(FakePair::new("default")).start(),
                Some(ONE_PAIR_FILE),
            ),
            Some("default"),
            "default",
        ),
        (
            "a current daemon running one pair",
            harness(
                FakeDaemon::multi_pair(vec![FakePair::new("default")]).start(),
                Some(ONE_PAIR_FILE),
            ),
            None,
            "default",
        ),
        (
            "two pairs, none named: the default pair",
            harness(two_pair_daemon(), Some(TWO_PAIR_FILE)),
            None,
            "docs",
        ),
        (
            "two pairs, the default pair named: omitted all the same",
            harness(two_pair_daemon(), Some(TWO_PAIR_FILE)),
            Some("docs"),
            "docs",
        ),
    ];
    for (what, h, pair, default) in &setups {
        let sent = drive(h, *pair, default);
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

/// The goldens that are a destructive verb's: the ones the capability gate stands in front of when
/// they are addressed to a pair other than the default.
const GATED: [&str; 7] = [
    "approve_path_literal",
    "approve_all_with_direction",
    "deny_path_literal",
    "keep_path_literal",
    "resync",
    "apply_plan",
    "apply_plan_skipping_destructive",
];

/// Acceptance 2, the other half: a NON-default pair is named on the wire, and **nothing else about
/// the request changes** — the same fields, the same order of requests, the same polling.
///
/// The one addition is the gate's: a destructive verb for a non-default pair is preceded by a fresh,
/// UNADDRESSED `status` (the line `get_status` sends for the default pair), and by nothing else.
#[test]
fn a_non_default_pair_is_named_on_the_wire_and_nothing_else_changes() {
    let golden = goldens();
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let sent = drive(&h, Some("photos"), "docs");
    let fresh_status = golden["get_status"][0].clone();
    for (command, lines) in &golden {
        let mut expected: Vec<Value> = Vec::new();
        if GATED.contains(&command.as_str()) {
            expected.push(serde_json::from_str(&fresh_status).unwrap());
        }
        expected.extend(lines.iter().map(|line| {
            let mut request: Value = serde_json::from_str(line).unwrap();
            request["pair"] = Value::String("photos".to_owned());
            request
        }));
        let actual: Vec<Value> = sent[command]
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            actual, expected,
            "`{command}` for `photos` must be its recorded request with `pair` set, and nothing else \
             (but for the gate's fresh status before a destructive verb)"
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
    // A class-W command takes its pair as a required name, and a name nobody runs is refused all the same.
    let unknown_name = || "not-a-pair".to_owned();
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
    refused_status(run!(pause(handle(), unknown_name())), "pause");
    refused_status(run!(resume(handle(), unknown_name())), "resume");
    refused_status(run!(sync_now(handle(), unknown_name())), "sync_now");
    refused_status(run!(resync(handle(), unknown_name())), "resync");
    refused_status(
        run!(approve(handle(), "a".into(), true, None, unknown_name())),
        "approve",
    );
    refused_status(
        run!(deny(handle(), "a".into(), true, unknown_name())),
        "deny",
    );
    refused_status(
        run!(keep(handle(), "a".into(), true, unknown_name())),
        "keep",
    );
    assert!(run!(list_pending_deletions(handle(), unknown())).is_err());
    assert!(run!(apply_plan(handle(), "t".into(), false, unknown_name())).is_err());
    assert!(run!(run_dry_run_with(
        &h.state(),
        unknown_name(),
        refuse_to_launch
    ))
    .is_err());
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
    assert!(resolve_conflict(
        h.state(),
        conflict.clone(),
        Resolution::KeepMine,
        unknown_name()
    )
    .is_err());
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

// ---- the pair a command ACTS on ---------------------------------------------------------------------

/// Two pairs that share nothing: `docs` (the default) and `photos`, each with a folder, an index and
/// a conflict of its own. **The same relative names in both folders and different bytes behind
/// them**, so a command that joins a path onto the wrong root does not fail — it succeeds, on the
/// wrong pair's files, and says so only in what it touched.
///
/// And every OTHER thing a command reads about its pair is made to differ too, because a routing
/// test that observes only the folder lets a command read the right folder with the wrong index or
/// the wrong sidecar spelling (the review of PR #436 found `read_conflict_pair`'s index and
/// `skip_rule_usage`'s index and naming unobserved, and a poison forcing them to the default pair left
/// the suite green):
///
/// - **the index** is relocated INSIDE each root as `state.db` (the default `.sync/` location is
///   always ignored, which would hide a wrong path from `skip_rule_usage`), and holds an agreed
///   version of `note.txt` of a different length per pair;
/// - **the sidecar spelling** differs (`docs-cloud` / `photos-cloud`), and each folder holds a file
///   spelled both ways.
pub(super) struct Routing {
    pub(super) h: Harness,
    docs: tempfile::TempDir,
    photos: tempfile::TempDir,
}

/// The agreed version each pair's index holds for `note.txt`: two lines for `docs`, four for
/// `photos`. The local file is one line, so the number of lines the card says were `removed` is the
/// length minus one — 1 or 3 — and names the index that was read.
const AGREED_DOCS: &str = "a\nb\n";
const AGREED_PHOTOS: &str = "a\nb\nc\nd\n";

impl Routing {
    fn new() -> Self {
        let docs = tempfile::tempdir().unwrap();
        let photos = tempfile::tempdir().unwrap();
        for (name, root, logs, agreed) in [
            ("docs", docs.path(), 2, AGREED_DOCS),
            ("photos", photos.path(), 3, AGREED_PHOTOS),
        ] {
            let write = |relative: &str, text: &str| {
                let path = root.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            };
            // One conflict, spelled the same in both folders; each side says whose it is.
            write("note.txt", &format!("pair={name} mine"));
            write("note.proton-cloud.txt", &format!("pair={name} theirs"));
            write("sub/inner.txt", "x");
            // `logs` files a `*.log` rule hides: a count that differs by pair.
            for n in 0..logs {
                write(&format!("{n}.log"), "log");
            }
            // One file spelled each pair's way. Under a pair's own suffix its spelling is a conflict
            // sidecar (not a file a skip rule could hide) and the other one is an ordinary file.
            write("probe.docs-cloud.dat", "x");
            write("probe.photos-cloud.dat", "x");
            // An index of its own, inside the root so the skip-rule walk would meet it.
            let db = root.join("state.db");
            gui_core::testing::write_index(&db, &[&format!("{name}-only.txt")]);
            gui_core::testing::write_agreed_summary(&db, "note.txt", agreed);
        }
        let config = format!(
            "[[pair]]\nname = \"docs\"\nlocal_root = {docs:?}\nremote_root = \"/Drive/docs\"\n\
             db_path = {docs_db:?}\nconflict_suffix = \"docs-cloud\"\n\
             [[pair]]\nname = \"photos\"\nlocal_root = {photos:?}\nremote_root = \"/Drive/photos\"\n\
             db_path = {photos_db:?}\nconflict_suffix = \"photos-cloud\"\n",
            docs = docs.path().display().to_string(),
            photos = photos.path().display().to_string(),
            docs_db = docs.path().join("state.db").display().to_string(),
            photos_db = photos.path().join("state.db").display().to_string(),
        );
        Self {
            h: harness(two_pair_daemon(), Some(&config)),
            docs,
            photos,
        }
    }

    fn root(&self, pair: &str) -> &std::path::Path {
        match pair {
            "docs" => self.docs.path(),
            "photos" => self.photos.path(),
            other => panic!("no such pair in this fixture: {other}"),
        }
    }

    /// The pair a class-R command means when it is told none: the one the app has selected.
    fn selected(&self) -> String {
        self.h.state().lock().unwrap().selected_pair().name
    }

    /// The pair whose folder `path` is under, as the fixture's two roots (canonical, as a resolved
    /// target is) tell it. `"none"` or `"both"` when it is not exactly one.
    fn pair_of(&self, path: &std::path::Path) -> &'static str {
        let under = |pair: &str| path.starts_with(self.root(pair).canonicalize().unwrap());
        match (under("docs"), under("photos")) {
            (true, false) => "docs",
            (false, true) => "photos",
            (true, true) => "both",
            (false, false) => "none",
        }
    }

    /// Which pairs a handed-to-the-opener target landed under. Recorded globally (see
    /// `opener_record`), so this reads only what is under THIS fixture's two roots.
    fn opened_under(&self) -> String {
        let mut pairs: Vec<&str> = opener_record::handed()
            .iter()
            .map(|target| self.pair_of(std::path::Path::new(target)))
            .filter(|pair| *pair != "none")
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        match pairs.as_slice() {
            [] => "nothing".to_owned(),
            pairs => pairs.join("+"),
        }
    }
}

/// How one command reports which pair it acted on: `docs`, `photos`, or what went wrong. The
/// observation comes from the EFFECT (the file it changed, the index it opened, the folder it
/// handed on), never from a name the command was given back.
pub(super) type Observe = fn(&Routing, Option<&str>) -> String;

/// The pair a text names: every marker file says `pair=<name> …`.
fn marked_pair(text: &str) -> String {
    text.strip_prefix("pair=")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("unmarked")
        .to_owned()
}

fn conflict_of_note() -> Conflict {
    Conflict {
        original: "note.txt".into(),
        sidecar: "note.proton-cloud.txt".into(),
        kind: conflicts::ConflictKind::Content,
    }
}

fn observe_resolve_conflict(r: &Routing, pair: Option<&str>) -> String {
    // Class W: it renames the sidecar over the original. The pair it acted on is the one whose
    // `note.txt` is now Proton's — and the other must be exactly as it was.
    resolve_conflict(
        r.h.state(),
        conflict_of_note(),
        Resolution::UseProton,
        // A class-W command names its pair. Naming none, in this table, means the default pair's own
        // name — which is what the third row below checks reaches the default folder.
        pair.unwrap_or("docs").to_owned(),
    )
    .expect("the resolution applies");
    let changed: Vec<&str> = ["docs", "photos"]
        .into_iter()
        .filter(|name| {
            std::fs::read_to_string(r.root(name).join("note.txt"))
                .is_ok_and(|text| text.contains("theirs"))
        })
        .collect();
    match changed.as_slice() {
        [] => "neither".to_owned(),
        changed => changed.join("+"),
    }
}

fn read_note(r: &Routing, pair: Option<&str>) -> gui_core::conflicts::ConflictPair {
    read_conflict_pair(r.h.state(), conflict_of_note(), pair.map(str::to_owned))
        .expect("both sides read")
}

fn observe_read_conflict_pair_folder(r: &Routing, pair: Option<&str>) -> String {
    marked_pair(
        &read_note(r, pair)
            .original
            .text
            .expect("the original is text"),
    )
}

/// The INDEX `read_conflict_pair` opened: each pair's holds an agreed version of `note.txt` of a
/// different length, and the card says how many of its lines the local file `removed`. That is the
/// length minus one — 1 for `docs`, 3 for `photos` — so a command reading the right folder with the
/// other pair's index says the other number. (No ancestor at all is its own answer: an index that
/// was not found says less rather than failing, and a test that took "nothing" for "docs" would
/// pass on a command that opened no index.)
fn observe_read_conflict_pair_index(r: &Routing, pair: Option<&str>) -> String {
    match read_note(r, pair)
        .happened
        .map(|happened| happened.mine.removed)
    {
        Some(1) => "docs".to_owned(),
        Some(3) => "photos".to_owned(),
        Some(other) => format!("an ancestor of {other} removed lines"),
        None => "no ancestor".to_owned(),
    }
}

fn observe_search_files(r: &Routing, pair: Option<&str>) -> String {
    // A path pasted out of a file manager: under the addressed pair's folder, naming the file only
    // that pair's index holds. It is found only by a command that opened THAT pair's index AND
    // reduced the path against THAT pair's folder; either half wrong and nothing matches.
    let addressed = pair.map_or_else(|| r.selected(), str::to_owned);
    let query = format!("{}/{addressed}-only.txt", r.root(&addressed).display());
    let found = tauri::async_runtime::block_on(search_files(
        r.h.app.handle().clone(),
        query,
        None,
        pair.map(str::to_owned),
    ))
    .expect("the search runs");
    match found.matches.as_slice() {
        [] => "nothing".to_owned(),
        [only] => only.path.trim_end_matches("-only.txt").to_owned(),
        _ => "several".to_owned(),
    }
}

fn observe_open_paths(r: &Routing, pair: Option<&str>) -> String {
    tauri::async_runtime::block_on(open_paths(
        r.h.state(),
        vec!["note.txt".into()],
        pair.map(str::to_owned),
    ))
    .expect("a file in the folder opens");
    r.opened_under()
}

fn observe_open_folder(r: &Routing, pair: Option<&str>) -> String {
    tauri::async_runtime::block_on(open_folder(
        r.h.state(),
        "sub/inner.txt".into(),
        pair.map(str::to_owned),
    ))
    .expect("the folder of a file in it opens");
    r.opened_under()
}

fn observe_free_space(r: &Routing, pair: Option<&str>) -> String {
    let space = tauri::async_runtime::block_on(free_space(
        r.h.app.handle().clone(),
        None,
        pair.map(str::to_owned),
    ))
    .expect("the folder exists, so it is measured");
    r.pair_of(&space.measured_at.canonicalize().unwrap())
        .to_owned()
}

/// One `skip_rule_usage` call with three rules, each of which answers a different question about
/// WHICH PAIR the command read: `*.log` the folder, `*.db` the index, `*.dat` the sidecar spelling.
fn skip_report(r: &Routing, pair: Option<&str>) -> gui_core::skip_rules::SkipRuleReport {
    tauri::async_runtime::block_on(skip_rule_usage(
        r.h.app.handle().clone(),
        vec!["*.log".into(), "*.db".into(), "*.dat".into()],
        None,
        pair.map(str::to_owned),
    ))
    .expect("the folder is there, so it is walked")
}

/// The FOLDER walked: docs holds two `.log` files and photos three.
fn observe_skip_rule_usage_folder(r: &Routing, pair: Option<&str>) -> String {
    folder_walked(&skip_report(r, pair))
}

fn folder_walked(report: &gui_core::skip_rules::SkipRuleReport) -> String {
    match report.rules[0].files {
        2 => "docs".to_owned(),
        3 => "photos".to_owned(),
        other => format!("{other} files"),
    }
}

/// The INDEX excluded from the walk. Each folder holds its own pair's relocated index, and the
/// command leaves a pair's index out of the count by the path it is told is the daemon's state file
/// (`daemon_ignored_paths`). Told the right pair's, the `*.db` rule matches nothing; told the other
/// pair's — a path in another folder — the folder's own index is counted as a file some rule is
/// hiding, which is exactly the false claim this command exists to avoid.
fn observe_skip_rule_usage_index(r: &Routing, pair: Option<&str>) -> String {
    let report = skip_report(r, pair);
    let folder = folder_walked(&report);
    match report.rules[1].files {
        0 => folder,
        counted => format!("{counted} index file(s) counted as user data in {folder}'s folder"),
    }
}

/// The SIDECAR SPELLING. Each folder holds `probe.docs-cloud.dat` and `probe.photos-cloud.dat`. A
/// pair's own spelling is a conflict sidecar under its own suffix — not a file at all as far as a
/// skip rule goes — so the one `*.dat` file left to hide is the OTHER spelling. Which one that is
/// names the suffix the command asked under.
fn observe_skip_rule_usage_naming(r: &Routing, pair: Option<&str>) -> String {
    let report = skip_report(r, pair);
    match report.rules[2].samples.as_slice() {
        [only] if only.path.contains("photos-cloud") => "docs".to_owned(),
        [only] if only.path.contains("docs-cloud") => "photos".to_owned(),
        other => format!("samples {other:?}"),
    }
}

/// What each routed command observes about the pair it acted on. A command may appear more than
/// once, once per thing it reads about its pair — and `skip_rule_usage` and `read_conflict_pair`
/// each read three (folder, index, spelling) and a test of one let a command read the other two from
/// the wrong pair without a test noticing.
///
/// `resolve_conflict` is the one class-W row and is not driven with no pair: see
/// `a_class_w_command_never_reads_the_selection`.
const ROUTED: [(&str, &str, Observe); 10] = [
    ("resolve_conflict", "folder", observe_resolve_conflict),
    (
        "read_conflict_pair",
        "folder",
        observe_read_conflict_pair_folder,
    ),
    (
        "read_conflict_pair",
        "index",
        observe_read_conflict_pair_index,
    ),
    ("search_files", "folder and index", observe_search_files),
    ("open_paths", "folder", observe_open_paths),
    ("open_folder", "folder", observe_open_folder),
    ("free_space", "folder", observe_free_space),
    ("skip_rule_usage", "folder", observe_skip_rule_usage_folder),
    ("skip_rule_usage", "index", observe_skip_rule_usage_index),
    (
        "skip_rule_usage",
        "sidecar spelling",
        observe_skip_rule_usage_naming,
    ),
];

/// The fixture, for the tests of the selection (`selection_tests`), which need the same two
/// unlike pairs.
pub(super) fn routing_for_selection_tests() -> Routing {
    Routing::new()
}

/// The routed commands that are class R — those for which "told nothing" means the SELECTED pair.
pub(super) fn class_r_observers() -> Vec<(&'static str, Observe)> {
    ROUTED
        .iter()
        .filter(|(command, _, _)| *command != "resolve_conflict")
        .map(|(command, _, observe)| (*command, *observe))
        .collect()
}

/// Each routed command acts on the pair it was told to, and on the default pair when told nothing —
/// asked of a fresh fixture each time, because three of them change the disk. Every command is
/// checked before the test fails, so a break names every command it reaches and not the first.
#[test]
fn the_routed_commands_act_on_the_addressed_pair_in_everything_they_read() {
    let mut wrong = Vec::new();
    for (command, what, observe) in ROUTED {
        for (asked, expected) in [
            (Some("photos"), "photos"),
            (Some("docs"), "docs"),
            (None, "docs"),
        ] {
            let observed = observe(&Routing::new(), asked);
            if observed != expected {
                wrong.push(format!(
                    "{command}({asked:?}) read {what} from {observed}, expected {expected}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The table above is only as good as its completeness. A command that reads a pair's folder, index
/// or sidecar spelling must be driven by one of the two routing tests; a new one that is not fails
/// here, instead of surviving a review the way those seven did.
#[test]
fn every_command_that_reads_a_pair_slot_has_its_routing_driven() {
    let source = include_str!("../commands.rs");
    let readers = [
        "effective_local_root(",
        "effective_db_path(",
        "conflict_naming(",
    ];
    let mut reading: Vec<String> = Vec::new();
    // Top-level functions only: they start at column 0 and end at the first column-0 `}`, so a test
    // module's own reads below them are never part of one.
    let mut current: Option<(String, bool)> = None;
    for line in source.lines() {
        let starts = [
            "fn ",
            "pub fn ",
            "pub async fn ",
            "async fn ",
            "pub(crate) fn ",
        ]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix));
        if let Some(rest) = starts {
            let name = rest.split(['(', '<']).next().unwrap().trim().to_owned();
            current = Some((name, false));
        }
        if let Some((name, reads)) = current.as_mut() {
            *reads |= readers.iter().any(|reader| line.contains(reader));
            if line == "}" {
                if *reads {
                    reading.push(name.clone());
                }
                current = None;
            }
        }
    }
    reading.sort_unstable();
    let mut driven: Vec<String> = ROUTED
        .iter()
        .map(|(name, _, _)| (*name).to_owned())
        .collect();
    driven.extend(["scan_conflicts".to_owned(), "path_sync_status".to_owned()]);
    driven.sort_unstable();
    driven.dedup();
    assert_eq!(
        reading, driven,
        "a function reads a pair's folder, index or sidecar spelling without a routing test (left), \
         or a routing test names a command that reads none (right)"
    );
}

/// A config file the engine refuses gives the app no pairs (the daemon will not start on it either),
/// and every command that needs a folder must then say THAT, in the engine's words — not "local_root
/// is not configured", which is false of a file that sets one beside a key it rejects.
#[test]
fn a_refused_config_file_is_the_reason_every_folder_command_gives() {
    let h = harness(
        two_pair_daemon(),
        Some("no_such_key = 1\nlocal_root = \"/fake/default/local\"\n"),
    );
    let handle = || h.app.handle().clone();
    macro_rules! run {
        ($future:expr) => {
            tauri::async_runtime::block_on($future)
        };
    }
    let conflict = Conflict {
        original: "a".into(),
        sidecar: "a.x".into(),
        kind: conflicts::ConflictKind::Content,
    };
    let answers: Vec<(&str, Option<String>)> = vec![
        (
            "scan_conflicts",
            run!(scan_conflicts(h.state(), None)).err(),
        ),
        (
            "resolve_conflict",
            resolve_conflict(
                h.state(),
                conflict.clone(),
                Resolution::KeepMine,
                "default".to_owned(),
            )
            .err(),
        ),
        (
            "read_conflict_pair",
            read_conflict_pair(h.state(), conflict, None).err(),
        ),
        ("free_space", run!(free_space(handle(), None, None)).err()),
        (
            "skip_rule_usage",
            run!(skip_rule_usage(handle(), vec![], None, None)).err(),
        ),
        (
            "path_sync_status",
            run!(path_sync_status(h.state(), "a".into(), None)).err(),
        ),
        (
            "search_files",
            run!(search_files(handle(), "a".into(), None, None)).err(),
        ),
        (
            "open_paths",
            run!(open_paths(h.state(), vec!["a".into()], None)).err(),
        ),
        (
            "open_folder",
            run!(open_folder(h.state(), "a".into(), None)).err(),
        ),
    ];
    // Every command is checked, so one failure names them all rather than the first.
    let wrong: Vec<String> = answers
        .into_iter()
        .filter_map(|(command, answer)| match answer {
            None => Some(format!("{command} succeeded on a refused file")),
            Some(answer)
                if answer.starts_with("the config file has an error: ")
                    && answer.contains("no_such_key") =>
            {
                None
            }
            Some(answer) => Some(format!("{command} blamed something else: {answer}")),
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The other half, so the new wording cannot leak: a file that is fine but places nothing keeps
/// the plain answers it always gave.
#[test]
fn a_fine_file_that_places_nothing_keeps_the_plain_answers() {
    let h = harness(two_pair_daemon(), Some("# nothing set\n"));
    assert_eq!(
        tauri::async_runtime::block_on(scan_conflicts(h.state(), None))
            .err()
            .as_deref(),
        Some("local_root is not configured")
    );
    assert_eq!(
        tauri::async_runtime::block_on(path_sync_status(h.state(), "a".into(), None))
            .err()
            .as_deref(),
        Some("no index database configured or reported by the daemon")
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
        // The file calls the pair what it is selected by, unless a test says otherwise.
        file_pair: Some(name.to_owned()),
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

/// The child reads the FILE, so the pair it is told to preview is the file's name for it. The app
/// selects a pair by the daemon's name once the daemon has answered, and a daemon that calls its
/// one pair `default` while the file's only table is `photos` used to get `--pair default`, which
/// the engine refuses with "names no configured folder pair" (reproduced against the built
/// `proton-syncd` by the review of PR #436).
#[test]
fn the_child_is_told_the_files_name_for_the_pair_not_the_daemons() {
    let reported = (Some("/l/d"), Some("/Drive/d"), Some("/l/d.db"));
    let mut selected = inputs("default", true, (None, None, None), reported);
    selected.file_pair = Some("photos".to_owned());
    assert_eq!(
        strings(&dry_run_args(&selected, true).unwrap()),
        [
            "--dry-run",
            "--config",
            "/cfg/proton-sync.toml",
            "--pair",
            "photos"
        ]
    );

    // A file with no name for it leaves the selected name, and the engine then says which pairs it
    // does have — better than guessing a neighbour.
    selected.file_pair = None;
    assert_eq!(
        strings(&dry_run_args(&selected, true).unwrap())[4],
        "default"
    );
}

/// The same, end to end through the command: a legacy daemon that reports `default`, a file whose
/// only table is `photos`, and nothing answering the plan verb — so the child is launched, with the
/// file's name. The launcher is the test's own; nothing is spawned.
#[test]
fn a_daemon_calling_its_pair_default_does_not_rename_the_files_only_table() {
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

    let h = harness(
        FakeDaemon::legacy(FakePair::new("default")).start(),
        Some("[[pair]]\nname = \"photos\"\nlocal_root = \"/fake/photos\"\nremote_root = \"/Drive/photos\"\n"),
    );
    let Harness {
        app,
        daemon,
        _dir: dir,
    } = h;
    // The daemon answers once, so its pair is cached under the name IT gives it; then it goes.
    tauri::async_runtime::block_on(get_status(app.handle().clone(), None));
    assert_eq!(
        app.state::<Mutex<RuntimePaths>>()
            .lock()
            .unwrap()
            .known_pair_names(),
        ["default"],
        "the premise: the selection is validated against the daemon's name"
    );
    drop(daemon);

    let payload = tauri::async_runtime::block_on(run_dry_run_with(
        &app.state::<Mutex<RuntimePaths>>(),
        "default".to_owned(),
        record,
    ))
    .expect("the canned report parses");
    assert!(payload.token.is_none(), "a child's plan has no token");

    let config = dir.path().join("proton-sync.toml").display().to_string();
    let launched = LAUNCHED.lock().unwrap();
    let mine: Vec<_> = launched
        .iter()
        .filter(|args| args.contains(&config))
        .collect();
    assert_eq!(mine.len(), 1, "one child, for one preview: {launched:?}");
    assert_eq!(
        mine[0],
        &["--dry-run", "--config", config.as_str(), "--pair", "photos"]
    );
}

/// The file's roots for a renamed pair are the file's roots, whatever the daemon calls it: the daemon
/// plans the preview only when the file agrees with it about where the pair is, and otherwise the
/// child previews the file's folders. (Reading the file's values by the daemon's name found none, so
/// a file pointing somewhere else was never noticed and the daemon previewed the wrong folders.)
#[test]
fn a_renamed_pair_is_previewed_by_the_daemon_only_when_the_files_roots_agree() {
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
    // The fake's pair `default` is at /fake/default/local and /Drive/default.
    let file_for = |local: &str, remote: &str| {
        format!("[[pair]]\nname = \"photos\"\nlocal_root = {local:?}\nremote_root = {remote:?}\n")
    };
    let sent_plan = |h: &Harness| {
        h.daemon
            .requests()
            .iter()
            .any(|line| line.contains("\"command\":\"plan\""))
    };

    // The file agrees with where the daemon's pair is: the daemon plans, no child.
    let agree = harness(
        FakeDaemon::legacy(FakePair::new("default")).start(),
        Some(&file_for("/fake/default/local", "/Drive/default")),
    );
    tauri::async_runtime::block_on(get_status(agree.app.handle().clone(), None));
    agree.daemon.clear_requests();
    let planned = tauri::async_runtime::block_on(run_dry_run_with(
        &agree.state(),
        "default".to_owned(),
        refuse_to_launch,
    ));
    assert!(planned.is_ok(), "the daemon answers: {:?}", planned.err());
    assert!(sent_plan(&agree), "{:?}", agree.daemon.requests());

    // The file puts the pair somewhere else: the daemon is not asked about the wrong folders.
    let differ = harness(
        FakeDaemon::legacy(FakePair::new("default")).start(),
        Some(&file_for("/fake/elsewhere", "/Drive/elsewhere")),
    );
    tauri::async_runtime::block_on(get_status(differ.app.handle().clone(), None));
    differ.daemon.clear_requests();
    let previewed = tauri::async_runtime::block_on(run_dry_run_with(
        &differ.state(),
        "default".to_owned(),
        record,
    ));
    assert!(previewed.is_ok(), "{:?}", previewed.err());
    assert!(
        !sent_plan(&differ),
        "the daemon planned folders the file does not name: {:?}",
        differ.daemon.requests()
    );
    let config = differ
        ._dir
        .path()
        .join("proton-sync.toml")
        .display()
        .to_string();
    let launched = LAUNCHED.lock().unwrap();
    let mine: Vec<_> = launched
        .iter()
        .filter(|args| args.contains(&config))
        .collect();
    assert_eq!(mine.len(), 1, "{launched:?}");
    assert_eq!(
        mine[0],
        &["--dry-run", "--config", config.as_str(), "--pair", "photos"]
    );
}

/// And the lookup under it: the table of the file that stands where the daemon's pair stands, only
/// when neither side knows the other's name — never a neighbour that is some OTHER pair of the
/// daemon's.
#[test]
fn the_files_name_for_a_pair_is_by_name_then_by_position_and_never_a_neighbour() {
    use crate::config_path::PairReported;
    let table = |name: &str| {
        format!(
            "[[pair]]\nname = {name:?}\nlocal_root = \"/l/{name}\"\nremote_root = \"/Drive/{name}\"\n"
        )
    };
    let reported = |names: &[&str]| -> Vec<PairReported> {
        names
            .iter()
            .map(|name| PairReported {
                name: (*name).to_owned(),
                local_root: format!("/r/{name}").into(),
                remote_root: format!("/Drive/r/{name}").into(),
                db_path: format!("/r/{name}.db").into(),
            })
            .collect()
    };
    let dir = tempfile::tempdir().unwrap();
    let paths_for = |tables: &[&str], daemon: &[&str]| {
        let path = dir.path().join("proton-sync.toml");
        std::fs::write(&path, tables.iter().map(|t| table(t)).collect::<String>()).unwrap();
        let mut paths = RuntimePaths::resolve_at(&path);
        paths.daemon.pairs = reported(daemon);
        paths
    };

    // Before the daemon has said anything the known names ARE the file's.
    let before = paths_for(&["a", "b"], &[]);
    assert_eq!(before.file_pair_name("b"), Some("b"));
    assert_eq!(before.file_pair_name("c"), None);

    // The daemon's one pair is `default`, the file's one table is `photos`: the same slot.
    assert_eq!(
        paths_for(&["photos"], &["default"]).file_pair_name("default"),
        Some("photos")
    );
    // A name the file has wins over wherever it stands — the file may have been reordered.
    assert_eq!(
        paths_for(&["b", "a"], &["a", "b"]).file_pair_name("a"),
        Some("a")
    );
    // `x` stands first among the daemon's, where the file has `y` — but `y` is a pair of the
    // daemon's too, elsewhere, so it is not `x`'s name. Better no answer than that one.
    assert_eq!(
        paths_for(&["y", "z"], &["x", "y"]).file_pair_name("x"),
        None
    );
    // Nothing stands there in the file at all.
    assert_eq!(
        paths_for(&["photos"], &["default", "extra"]).file_pair_name("extra"),
        None
    );
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
        "photos".to_owned(),
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
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offending = Vec::new();
    for root in [manifest.join("src"), manifest.join("../gui-core/src")] {
        rust_sources(&root, &mut offending);
    }
    assert!(
        offending.is_empty(),
        "a function named for the remote root exists, which is what a merge of the configured and \
         the reported one is called: {offending:?}"
    );
}

/// Appends `file:line: name` for every non-comment line under `dir` that declares a function with
/// `remote_root` in its name — `effective_remote_root`, `remote_root_for`, `merged_remote_root`.
///
/// **By name, and the limit is the name.** A getter that merges the two and is called something else
/// passes; what the scan removes is the shapes the previous literal missed (it matched one spelling,
/// `effective…_remote_root`, in two files). This file is skipped: it names the test that holds the
/// line.
fn rust_sources(dir: &std::path::Path, offending: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, offending);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_some_and(|name| name != "pair_tests.rs")
        {
            let source = std::fs::read_to_string(&path).unwrap();
            for (number, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if let Some(name) = declared_remote_root_function(line) {
                    offending.push(format!("{}:{}: {name}", path.display(), number + 1));
                }
            }
        }
    }
}

/// The name of a function declared on `line` that has `remote_root` in it, if there is one.
fn declared_remote_root_function(line: &str) -> Option<String> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut rest = line;
    while let Some(at) = rest.find("fn ") {
        let preceded_by_ident = rest[..at].chars().next_back().is_some_and(is_ident);
        let name: String = rest[at + 3..]
            .chars()
            .take_while(|c| is_ident(*c))
            .collect();
        if !preceded_by_ident && name.contains("remote_root") {
            return Some(name);
        }
        rest = &rest[at + 3..];
    }
    None
}

/// The scan above has to be able to find what it is looking for.
#[test]
fn the_remote_root_scan_sees_every_spelling_of_a_third_resolver() {
    for line in [
        "    pub fn effective_remote_root(&self, pair: &str) -> Option<PathBuf> {",
        "fn remote_root_for(pair: &str) -> PathBuf {",
        "    pub(crate) fn merged_remote_root(&self) -> Option<PathBuf> {",
        "    async fn the_remote_root(&self) {}",
    ] {
        assert!(declared_remote_root_function(line).is_some(), "{line}");
    }
    for line in [
        "    pub fn effective_local_root(&self, pair: &str) -> Option<PathBuf> {",
        "    let remote_root = pair.remote_root.clone();",
        "    file_remote: configured.and_then(|c| c.remote_root.clone()),",
        "    let defn = something_fn remote_root;",
    ] {
        assert_eq!(declared_remote_root_function(line), None, "{line}");
    }
}

// ---- classification ------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A single-shot read: a wrong pair costs a wrong screen. Takes `pair: Option<String>`, and
    /// naming none means the pair the app has selected.
    R,
    /// A write, a deletion, or the start of a multi-step flow: a wrong pair costs data. Takes
    /// `pair: String` — **required** (#102 phase 5a-2), the value the caller captured when the screen
    /// or row began, and never the selection: a missing or unknown name is refused, not defaulted.
    W,
    /// The one writer of the selection. Names its pair as `name`, because it is not a request
    /// addressed to a pair: it changes what later class-R reads mean.
    Selection,
    /// Addressed by the ROW that was pressed: its id carries the pair (`pause@photos`), so the pair
    /// is whatever the row was drawn for and never the selection (#102 phase 5d).
    Row,
    /// Asks about a pair that does not exist yet (#102 phase 5c-2): the add dialog's check names the
    /// candidate as `name`, reads the config file as a whole, and writes nothing. Not addressed to a
    /// pair, so it takes no `pair` argument.
    NewPair,
    /// Always the default pair, by design and not by omission: the tray panel's poll, whose rows
    /// were built around that pair's full reply. Takes no `pair` argument because it has no choice
    /// to make — and a read that named none would mean the SELECTED pair (`Ask::Selected`).
    DefaultPair,
    /// About the process, the daemon as a whole, or the window — no folder pair is involved.
    Independent,
}

/// Every `#[tauri::command]` and what it is. Adding a command without adding it here fails
/// `every_pair_slot_command_names_its_class`, which is the point: a command that reads a pair slot
/// must say how it takes the pair.
const COMMAND_CLASSES: [(&str, Class); 43] = [
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
    ("select_pair", Class::Selection),
    ("read_config", Class::R),
    ("write_config", Class::W),
    // Class W with one difference, stated at the command: `pair` is the NEW pair's name, so the engine
    // refuses one that already exists where every other W command refuses one that does not.
    ("add_pair", Class::W),
    // Asks whether `add_pair` would go ahead, for a pair that does not exist yet: it names it as `name`,
    // reads the file, and changes nothing.
    ("check_add_pair", Class::NewPair),
    ("remove_pair", Class::W),
    ("tray_action", Class::Row),
    ("tray_status", Class::DefaultPair),
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
        let optional_pair = signature.contains("pair: Option<String>");
        let required_pair = signature.contains("pair: String");
        match class {
            Class::R => assert!(
                optional_pair && !required_pair,
                "`{name}` is class R and must take `pair: Option<String>` (naming none = the selection)"
            ),
            // REQUIRED, and the type is the guard: a `String` cannot be absent, so no frontend slip
            // turns a write into "whatever pair is selected" or "the default".
            Class::W => assert!(
                required_pair && !optional_pair,
                "`{name}` is class W and must take `pair: String`"
            ),
            Class::Selection
            | Class::NewPair
            | Class::Row
            | Class::DefaultPair
            | Class::Independent => assert!(
                !optional_pair && !required_pair,
                "`{name}` is {class:?} and must not take a `pair` argument"
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
    tauri::async_runtime::block_on(write_config(
        app.state::<Mutex<RuntimePaths>>(),
        "default".to_owned(),
        ConfigUpdate {
            proton_cli: Some("/fake/cli".to_owned()),
            ..Default::default()
        },
    ))
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
