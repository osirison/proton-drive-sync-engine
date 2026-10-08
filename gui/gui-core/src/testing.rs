//! The one fake daemon every GUI test shares (#102 phase 5a). Compiled only with the `test-support`
//! cargo feature, which `gui/src-tauri` names in `[dev-dependencies]` alone.
//!
//! It is a real Unix-socket server in a short temporary directory, so a test drives the same
//! `ipc::send_request` the app does, and it **records every request line** exactly as it arrived.
//! That record is what makes "this command sends the request it always sent" a checkable sentence:
//! the goldens in `src-tauri` compare against it byte for byte.
//!
//! **Replies are built from the engine's own types**, never from hand-written JSON. A reply the GUI
//! cannot decode is drawn as "daemon unreachable" with the cause surfaced nowhere, so a fixture
//! that drifts from the wire would turn every test around it into a quiet lie.
//!
//! It models the three daemons a GUI meets:
//!
//! - [`FakeDaemonBuilder::multi_pair`] — a current daemon. It honours `ControlRequest::pair` the
//!   way the real one does: a resolved selector answers with `pair: Some(name)`, an unknown one
//!   answers `pair: None` with `pairs` still populated and **does nothing**, and `shutdown` ignores
//!   the selector.
//! - [`FakeDaemonBuilder::legacy`] — a daemon that predates the selector. It never reads
//!   `pair`, acts on its one pair whatever a request says, and its replies carry neither `pair` nor
//!   `pairs`. A client that sends it a selector is silently misrouted, which is exactly what a test
//!   of the capability gate needs to be able to see.
//! - [`FakeDaemonBuilder::drop_connection_for`] — a daemon older than a verb. The real one closes
//!   the connection without replying to a command it cannot parse, and at the transport that is
//!   indistinguishable from a daemon that is not there.
//!
//! **Nothing here may reach the real machine.** The socket lives in a directory this module
//! creates (or one the caller hands it) and nothing else is touched; a test that needs a daemon
//! uses this and never `default_socket_path()`.

use crate::wire::{
    ApplyOutcome, ControlCommand, ControlRequest, ControlResponse, LocalDisposal, PairSummary,
    PlanOutcome, PlanSummary, ReviewedPlan, RunningConfigInfo,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// `sun_path` is 108 bytes on Linux; refuse a directory that would not fit the socket name rather
/// than let `bind` fail with an error that names nothing.
const MAX_SOCKET_PATH_BYTES: usize = 100;

/// The epoch every fake reply reports as its last sync, so a reply is never `FirstRun` by accident.
const FAKE_LAST_SYNC: u64 = 1_750_000_000;

/// One folder pair the fake daemon is running.
#[derive(Debug, Clone)]
pub struct FakePair {
    pub name: String,
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
    pub db_path: PathBuf,
    pub paused: bool,
    pub pending_deletions: usize,
    /// When the pair last synced. `None` is a pair that never has — and, since a fake reply's
    /// status history is always empty, the state a full reply derives `FirstRun` from.
    pub last_sync: Option<u64>,
    /// Why the pair's last pass failed, or why it is unavailable (an unplugged drive).
    pub last_error: Option<String>,
    /// Set, a `pause` or `resume` of this pair is answered `pause_unsaved` with this reason — the
    /// daemon applied the change and could not save it (#102 decision D12).
    pub pause_unsaved: Option<String>,
    plan_seq: u64,
    apply_seq: u64,
}

impl FakePair {
    /// A pair with roots derived from its name: `/fake/<name>/local`, `/Drive/<name>` and
    /// `/fake/<name>/local/.sync/sync_index.db`. Nothing is created on disk.
    pub fn new(name: &str) -> Self {
        let local_root = PathBuf::from(format!("/fake/{name}/local"));
        Self::with_roots(
            name,
            &local_root,
            Path::new(&format!("/Drive/{name}")),
            &local_root.join(".sync").join("sync_index.db"),
        )
    }

    pub fn with_roots(name: &str, local_root: &Path, remote_root: &Path, db_path: &Path) -> Self {
        Self {
            name: name.to_owned(),
            local_root: local_root.to_owned(),
            remote_root: remote_root.to_owned(),
            db_path: db_path.to_owned(),
            paused: false,
            pending_deletions: 0,
            last_sync: Some(FAKE_LAST_SYNC),
            last_error: None,
            pause_unsaved: None,
            plan_seq: 0,
            apply_seq: 0,
        }
    }

    /// A pair that has never synced: no last sync, and (as every fake reply's history is empty) the
    /// state a full reply derives `FirstRun` from.
    pub fn never_synced(mut self) -> Self {
        self.last_sync = None;
        self
    }

    /// A pair whose last pass failed with `reason`, or whose folder is unavailable.
    pub fn failing(mut self, reason: &str) -> Self {
        self.last_error = Some(reason.to_owned());
        self
    }

    /// A pair whose pause or resume the daemon applies and cannot save, for `reason`.
    pub fn unable_to_save_a_pause(mut self, reason: &str) -> Self {
        self.pause_unsaved = Some(reason.to_owned());
        self
    }

    fn summary(&self) -> PairSummary {
        PairSummary {
            name: self.name.clone(),
            local_root: self.local_root.clone(),
            remote_root: self.remote_root.clone(),
            db_path: self.db_path.clone(),
            paused: self.paused,
            syncing: false,
            reconcile_seq: 0,
            last_sync_epoch_secs: self.last_sync,
            last_error: self.last_error.clone(),
            pending_changes: 0,
            pending_deletions: self.pending_deletions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Replies carry `pair` and `pairs`, and a request's selector is honoured.
    MultiPair,
    /// Replies carry neither, and a request's selector is never read.
    Legacy,
}

struct Shared {
    shape: Shape,
    /// Commands this daemon is too old to parse: the connection is closed without a reply.
    drops: Vec<ControlCommand>,
    requests: Mutex<Vec<String>>,
    pairs: Mutex<Vec<FakePair>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Builds a [`FakeDaemon`]. Consumed by [`Self::start`], which is where the socket is bound.
pub struct FakeDaemonBuilder {
    shape: Shape,
    pairs: Vec<FakePair>,
    drops: Vec<ControlCommand>,
    dir: Option<PathBuf>,
}

impl FakeDaemonBuilder {
    /// Bind in `dir` (which must already exist and be short) instead of a fresh temporary one.
    pub fn in_dir(mut self, dir: &Path) -> Self {
        self.dir = Some(dir.to_owned());
        self
    }

    /// Model a daemon older than these verbs: it closes the connection on each, without replying.
    pub fn drop_connection_for(mut self, commands: &[ControlCommand]) -> Self {
        self.drops.extend_from_slice(commands);
        self
    }

    /// Bind the socket and start answering.
    pub fn start(self) -> FakeDaemon {
        assert!(
            !self.pairs.is_empty(),
            "a fake daemon runs at least one pair"
        );
        let (dir, owned_dir) = match self.dir {
            Some(dir) => (dir, None),
            // `/tmp` is short on every machine this runs on; the default `TMPDIR` may not be.
            None => {
                let dir = tempfile::Builder::new()
                    .prefix("pds-")
                    .tempdir()
                    .expect("a temporary directory for the fake daemon's socket");
                (dir.path().to_owned(), Some(dir))
            }
        };
        let socket_path = dir.join("f.sock");
        assert!(
            socket_path.as_os_str().len() < MAX_SOCKET_PATH_BYTES,
            "the fake daemon's socket path is too long for sun_path: {}",
            socket_path.display()
        );
        let listener = UnixListener::bind(&socket_path).expect("bind the fake daemon's socket");
        let shared = Arc::new(Shared {
            shape: self.shape,
            drops: self.drops,
            requests: Mutex::new(Vec::new()),
            pairs: Mutex::new(self.pairs),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
            std::thread::spawn(move || serve(&listener, &shared, &stop))
        };
        FakeDaemon {
            socket_path,
            shared,
            stop,
            thread: Some(thread),
            _dir: owned_dir,
        }
    }
}

/// A recording fake `proton-syncd` on a Unix socket. Stops, and removes its socket, when dropped.
pub struct FakeDaemon {
    socket_path: PathBuf,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    // Held for its `Drop`; `None` when the caller supplied the directory.
    _dir: Option<tempfile::TempDir>,
}

impl FakeDaemon {
    /// A current daemon running `pairs`; the first is the default pair.
    pub fn multi_pair(pairs: Vec<FakePair>) -> FakeDaemonBuilder {
        FakeDaemonBuilder {
            shape: Shape::MultiPair,
            pairs,
            drops: Vec::new(),
            dir: None,
        }
    }

    /// A daemon that predates the pair selector, running one pair.
    pub fn legacy(pair: FakePair) -> FakeDaemonBuilder {
        FakeDaemonBuilder {
            shape: Shape::Legacy,
            pairs: vec![pair],
            drops: Vec::new(),
            dir: None,
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Every request line received, in arrival order, exactly as the client wrote it (without the
    /// trailing newline). Includes requests this daemon then refused to answer.
    pub fn requests(&self) -> Vec<String> {
        lock(&self.shared.requests).clone()
    }

    /// [`Self::requests`] parsed as the engine's own request type. A line that does not parse is a
    /// test failure here, not a silently shorter list.
    pub fn parsed_requests(&self) -> Vec<ControlRequest> {
        self.requests()
            .iter()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|error| panic!("not a ControlRequest ({error}): {line}"))
            })
            .collect()
    }

    /// Forget what was recorded, so a test can assert on one command at a time.
    pub fn clear_requests(&self) {
        lock(&self.shared.requests).clear();
    }

    /// Replace the pairs this daemon runs — a restart onto a different config, with the same socket.
    /// A client that remembered the old list now addresses names the daemon no longer knows.
    pub fn set_pairs(&self, pairs: Vec<FakePair>) {
        assert!(!pairs.is_empty(), "a fake daemon runs at least one pair");
        *lock(&self.shared.pairs) = pairs;
    }

    /// Whether the named pair is paused right now — the side effect `pause`/`resume` have.
    pub fn is_paused(&self, name: &str) -> bool {
        lock(&self.shared.pairs)
            .iter()
            .find(|pair| pair.name == name)
            .is_some_and(|pair| pair.paused)
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // `accept` is blocking: one connection wakes it so it can see the flag.
        let _ = std::os::unix::net::UnixStream::connect(&self.socket_path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn serve(listener: &UnixListener, shared: &Shared, stop: &AtomicBool) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        // One request per connection is the protocol; reading to the first newline is enough.
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            continue;
        }
        let line = line.trim_end().to_owned();
        lock(&shared.requests).push(line.clone());
        let Ok(request) = serde_json::from_str::<ControlRequest>(&line) else {
            // What the real daemon does with a line it cannot parse: close, say nothing.
            continue;
        };
        if shared.drops.contains(&request.command) {
            continue;
        }
        let Ok(reply) = serde_json::to_string(&answer(shared, &request)) else {
            continue;
        };
        let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
    }
}

/// What the daemon says to `request`, and what it does about it.
fn answer(shared: &Shared, request: &ControlRequest) -> ControlResponse {
    let mut pairs = lock(&shared.pairs);
    let resolved = match shared.shape {
        // An old daemon has one pair and never reads the selector.
        Shape::Legacy => Some(0),
        Shape::MultiPair => match (&request.command, request.pair.as_deref()) {
            // Daemon-wide: the selector is ignored (`shutdown`), the default pair answers.
            (ControlCommand::Shutdown, _) | (_, None) => Some(0),
            (_, Some(name)) => pairs.iter().position(|pair| pair.name == name),
        },
    };
    let Some(index) = resolved else {
        // An unresolved selector: nothing is done, `pair` is absent, `pairs` says what exists.
        let known: Vec<&str> = pairs.iter().map(|pair| pair.name.as_str()).collect();
        let mut reply = reply_for(shared.shape, &pairs, 0);
        reply.pair = None;
        reply.message = format!(
            "no folder pair named {:?}; the configured pairs are {known:?}",
            request.pair.as_deref().unwrap_or_default()
        );
        return reply;
    };

    let pair = &mut pairs[index];
    let mut plan = None;
    let mut apply = None;
    match request.command {
        ControlCommand::Pause => pair.paused = true,
        ControlCommand::Resume => pair.paused = false,
        ControlCommand::Plan => {
            pair.plan_seq += 1;
            plan = Some(PlanOutcome::Scheduled {
                plan_seq: pair.plan_seq,
            });
        }
        ControlCommand::PlanResult => {
            plan = Some(if pair.plan_seq == 0 {
                PlanOutcome::Absent
            } else {
                PlanOutcome::Computed(Box::new(ReviewedPlan {
                    plan_seq: pair.plan_seq,
                    token: format!("fake-token-{}-{}", pair.name, pair.plan_seq),
                    computed_epoch_secs: FAKE_LAST_SYNC,
                    summary: PlanSummary::default(),
                    actions: Vec::new(),
                    total: 0,
                    truncated: false,
                    cannot_sync: Vec::new(),
                    local_disposal: LocalDisposal::Recoverable,
                }))
            });
            apply = (pair.apply_seq > 0).then_some(ApplyOutcome::Applied {
                apply_seq: pair.apply_seq,
                executed: 0,
                skipped_destructive: 0,
                failed: 0,
            });
        }
        ControlCommand::Apply => {
            pair.apply_seq += 1;
            apply = Some(ApplyOutcome::Scheduled {
                apply_seq: pair.apply_seq,
            });
        }
        _ => {}
    }
    let mut reply = reply_for(shared.shape, &pairs, index);
    reply.plan = plan;
    reply.apply = apply;
    if matches!(
        request.command,
        ControlCommand::Pause | ControlCommand::Resume
    ) {
        reply.pause_unsaved = pairs[index].pause_unsaved.clone();
    }
    reply
}

/// A status-shaped reply describing `pairs[index]`, in the shape this daemon speaks.
fn reply_for(shape: Shape, pairs: &[FakePair], index: usize) -> ControlResponse {
    let pair = &pairs[index];
    ControlResponse {
        status: if pair.paused { "paused" } else { "running" }.to_owned(),
        paused: pair.paused,
        syncing: false,
        reconcile_seq: 0,
        pending_changes: 0,
        message: "fake daemon".to_owned(),
        pause_unsaved: None,
        last_sync_epoch_secs: pair.last_sync,
        last_error: pair.last_error.clone(),
        last_plan_summary: None,
        last_successful_sync_summary: None,
        status_history: Vec::new(),
        pending_deletions: Vec::new(),
        config: Some(RunningConfigInfo {
            local_root: pair.local_root.clone(),
            remote_root: pair.remote_root.clone(),
            db_path: pair.db_path.clone(),
        }),
        activity: None,
        failed_items: Vec::new(),
        failed_item_count: 0,
        unsyncable: Vec::new(),
        history: None,
        file_history: None,
        index_totals: None,
        listing: None,
        plan: None,
        apply: None,
        auth: Default::default(),
        pair: (shape == Shape::MultiPair).then(|| pair.name.clone()),
        pairs: if shape == Shape::MultiPair {
            pairs.iter().map(FakePair::summary).collect()
        } else {
            Vec::new()
        },
    }
}

/// Create the index database at `db_path` (and its directory) holding one synced file per entry of
/// `relative_paths`, so a test can give each pair an index of its own and see which one a command
/// opened. The engine's own writer, not a hand-made schema: what a command reads back is what the
/// daemon would have written.
pub fn write_index(db_path: &Path, relative_paths: &[&str]) {
    use crate::wire::{EntityKind, FileRecord};
    use proton_drive_sync_engine::index::{
        SyncStatus, initialize_schema, open_database, upsert_record,
    };

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).expect("the index directory is created");
    }
    let connection = open_database(db_path).expect("the index opens");
    initialize_schema(&connection).expect("the schema is created");
    for relative in relative_paths {
        upsert_record(
            &connection,
            &FileRecord {
                file_path: PathBuf::from(relative),
                entity_kind: EntityKind::File,
                file_size: 10,
                mtime: 0,
                sha1_hash: Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_owned()),
                proton_id: None,
                sync_status: SyncStatus::Synced,
            },
        )
        .expect("the record is written");
    }
}

/// File `text` as the version both sides last agreed on for `relative_path`, in the index at
/// `db_path` — the row a conflict card reads its first line from (#217/#347).
///
/// The index is opened (and its schema created) if it is not there. A test gives each pair an
/// ancestor of a different SHAPE, so which index a command opened shows in what it reports.
pub fn write_agreed_summary(db_path: &Path, relative_path: &str, text: &str) {
    use proton_drive_sync_engine::ancestor::LineSummary;
    use proton_drive_sync_engine::index::{initialize_schema, open_database, store_agreed_summary};

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).expect("the index directory is created");
    }
    let connection = open_database(db_path).expect("the index opens");
    initialize_schema(&connection).expect("the schema is created");
    let summary = LineSummary::of(text).expect("a summarisable text");
    store_agreed_summary(
        &connection,
        Path::new(relative_path),
        "agreed-digest",
        &summary,
        1,
    )
    .expect("the summary is written");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, DEFAULT_TIMEOUT};
    use crate::pairs::{PairCapability, Target};

    fn status(daemon: &FakeDaemon, target: Target<'_>) -> ControlResponse {
        ipc::command(
            daemon.socket_path(),
            target,
            ControlCommand::Status,
            DEFAULT_TIMEOUT,
        )
        .expect("the fake answers status")
    }

    #[test]
    fn it_records_every_request_line_exactly_as_it_arrived() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::new("a")]).start();
        status(&daemon, Target::DEFAULT);
        status(&daemon, Target::named("a"));
        let lines = daemon.requests();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"command\":\"status\""), "{}", lines[0]);
        assert!(lines[0].contains("\"pair\":null"), "{}", lines[0]);
        assert!(lines[1].contains("\"pair\":\"a\""), "{}", lines[1]);
        assert_eq!(daemon.parsed_requests()[1].pair.as_deref(), Some("a"));
        daemon.clear_requests();
        assert!(daemon.requests().is_empty());
    }

    /// A current daemon's three answers to a selector, as the engine gives them (ADR 0005 §4).
    #[test]
    fn a_multi_pair_daemon_resolves_an_omitted_selector_a_named_one_and_an_unknown_one() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::new("a"), FakePair::new("b")]).start();

        let omitted = status(&daemon, Target::DEFAULT);
        assert_eq!(omitted.pair.as_deref(), Some("a"), "omitted = the first");
        assert_eq!(omitted.pairs.len(), 2);

        let named = status(&daemon, Target::named("b"));
        assert_eq!(named.pair.as_deref(), Some("b"));
        assert_eq!(
            named.config.expect("config").local_root,
            PathBuf::from("/fake/b/local"),
            "the top-level fields describe the selected pair"
        );

        let unknown = status(&daemon, Target::named("B"));
        assert_eq!(unknown.pair, None, "byte-exact: `B` is not `b`");
        assert_eq!(unknown.pairs.len(), 2, "but the list is still there");
        assert_eq!(
            PairCapability::from_reply(&unknown),
            PairCapability::MultiPair
        );
    }

    /// The unresolved selector's other half: it did nothing at all.
    #[test]
    fn an_unresolved_selector_has_no_side_effect() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::new("a"), FakePair::new("b")]).start();
        let reply = ipc::command(
            daemon.socket_path(),
            Target::named("nope"),
            ControlCommand::Pause,
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert_eq!(reply.pair, None);
        assert!(!daemon.is_paused("a") && !daemon.is_paused("b"));
        // And a resolved one pauses exactly its own pair.
        ipc::command(
            daemon.socket_path(),
            Target::named("b"),
            ControlCommand::Pause,
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert!(daemon.is_paused("b") && !daemon.is_paused("a"));
    }

    #[test]
    fn a_legacy_daemon_never_reads_the_selector_and_says_neither_pair_nor_pairs() {
        let daemon = FakeDaemon::legacy(FakePair::new("only")).start();
        let reply = ipc::command(
            daemon.socket_path(),
            Target::named("somewhere-else"),
            ControlCommand::Pause,
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert_eq!(PairCapability::from_reply(&reply), PairCapability::Legacy);
        assert!(reply.pair.is_none() && reply.pairs.is_empty());
        assert!(
            daemon.is_paused("only"),
            "the selector was ignored, so the one pair it has was paused"
        );
        assert_eq!(
            daemon.parsed_requests()[0].pair.as_deref(),
            Some("somewhere-else"),
            "and the record shows what was really sent"
        );
    }

    #[test]
    fn a_daemon_older_than_a_verb_closes_the_connection_without_a_reply() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::new("a")])
            .drop_connection_for(&[ControlCommand::Plan])
            .start();
        let error = ipc::command(
            daemon.socket_path(),
            Target::DEFAULT,
            ControlCommand::Plan,
            DEFAULT_TIMEOUT,
        )
        .expect_err("dropped");
        assert!(matches!(error, ipc::IpcError::Unreachable(_)), "{error:?}");
        // The request was still received, and the other verbs still work.
        assert_eq!(daemon.requests().len(), 1);
        status(&daemon, Target::DEFAULT);
    }

    /// The fake can be made to say the states a GUI test needs, and to change its mind: a restart
    /// onto fewer pairs leaves a client holding a name the daemon has dropped.
    #[test]
    fn a_pair_can_be_never_synced_or_failing_and_the_pairs_can_be_replaced_at_the_same_socket() {
        use crate::state::{DaemonState, derive_state};
        let daemon = FakeDaemon::multi_pair(vec![
            FakePair::new("a"),
            FakePair::new("b").never_synced(),
            FakePair::new("c").failing("the sync folder is not available"),
        ])
        .start();
        let state_of = |name| derive_state(Ok(&status(&daemon, Target::named(name))));
        assert_eq!(state_of("a"), DaemonState::Idle);
        assert_eq!(state_of("b"), DaemonState::FirstRun);
        assert_eq!(state_of("c"), DaemonState::Failed);
        let listed = status(&daemon, Target::DEFAULT).pairs;
        assert_eq!(listed[1].last_sync_epoch_secs, None);
        assert_eq!(
            listed[2].last_error.as_deref(),
            Some("the sync folder is not available")
        );

        daemon.set_pairs(vec![FakePair::new("a")]);
        let after = status(&daemon, Target::named("b"));
        assert_eq!(after.pair, None, "`b` is gone");
        assert_eq!(after.pairs.len(), 1);
    }

    #[test]
    fn plan_and_apply_are_counted_per_pair() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::new("a"), FakePair::new("b")]).start();
        let plan = |target| {
            ipc::command(
                daemon.socket_path(),
                target,
                ControlCommand::Plan,
                DEFAULT_TIMEOUT,
            )
            .unwrap()
            .plan
        };
        assert_eq!(
            plan(Target::named("b")),
            Some(PlanOutcome::Scheduled { plan_seq: 1 })
        );
        assert_eq!(
            plan(Target::named("b")),
            Some(PlanOutcome::Scheduled { plan_seq: 2 })
        );
        assert_eq!(
            plan(Target::DEFAULT),
            Some(PlanOutcome::Scheduled { plan_seq: 1 }),
            "pair `a` has its own counter"
        );
    }

    #[test]
    fn dropping_the_daemon_removes_its_socket() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::new("a")]).start();
        let path = daemon.socket_path().to_owned();
        assert!(path.exists());
        drop(daemon);
        assert!(!path.exists());
    }
}
