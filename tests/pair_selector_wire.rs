//! Pins the pair-selector wiring in `proton-sync`'s printed hints (#409, on top of 99ef0c4).
//!
//! 99ef0c4 introduced `cli_hint(pair, tail)` and rewired every "run this next" line through it,
//! but the only tests it added exercise the helper itself (`cli_hint_echoes_the_selector_the_command_was_run_with`)
//! and the handful of unit-level render functions with a `pair` argument passed straight in. None
//! of that proves the CALL SITES still route through `cli_hint` rather than a bare string literal
//! — reverting `print_pending`'s and `print_status`'s `cli_hint(...)` calls back to hardcoded
//! `proton-sync approve <path>` text leaves every pre-existing `proton-sync` bin test green.
//! This file drives the real compiled binary against a scripted fake daemon and asserts on the
//! rendered stdout/stderr, so it catches exactly that regression.
#![cfg(unix)]

use proton_drive_sync_engine::index::EntityKind;
use proton_drive_sync_engine::ipc::{
    ApplyOutcome, ControlRequest, ControlResponse, ListingOutcome, LocalDisposal, PairSummary,
    PendingDeletion, PlanOutcome, ReviewedPlan,
};
use proton_drive_sync_engine::sync::{DeleteDirection, PlanSummary};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A legacy-shaped reply upgraded to a resolved multi-pair one (`pair: Some("work")`, two
/// configured pairs) — the capability gate's own probe reply, reused as the base for every
/// scripted response below.
fn blank() -> ControlResponse {
    let legacy = r#"{"status":"running","paused":false,"pending_changes":0,
        "message":"daemon status","last_sync_epoch_secs":null,"last_error":null,
        "last_plan_summary":null,"last_successful_sync_summary":null,"status_history":[]}"#;
    let mut response: ControlResponse = serde_json::from_str(legacy).expect("legacy reply parses");
    response.reconcile_seq = 1;
    response.pair = Some("work".to_owned());
    response.pairs = vec![summary("default"), summary("work")];
    response
}

fn summary(name: &str) -> PairSummary {
    PairSummary {
        name: name.to_owned(),
        local_root: PathBuf::from("/local"),
        remote_root: PathBuf::from("/Drive/Remote"),
        db_path: PathBuf::from("/local/.sync/index.db"),
        paused: false,
        syncing: false,
        reconcile_seq: 1,
        last_sync_epoch_secs: None,
        last_error: None,
        pending_changes: 0,
        pending_deletions: 0,
    }
}

fn pending_deletion(path: &str) -> PendingDeletion {
    PendingDeletion {
        path: PathBuf::from(path),
        direction: DeleteDirection::Local,
        entity_kind: EntityKind::File,
        fingerprint: "fp".to_owned(),
        detected_epoch_secs: 1,
        first_seen_epoch_secs: 0,
        subtree_files: None,
        subtree_bytes: None,
        disposal: LocalDisposal::Permanent,
    }
}

fn reviewed(token: &str, total: usize) -> PlanOutcome {
    PlanOutcome::Computed(Box::new(ReviewedPlan {
        plan_seq: 3,
        token: token.to_owned(),
        computed_epoch_secs: 100,
        summary: PlanSummary::default(),
        actions: Vec::new(),
        total,
        truncated: false,
        cannot_sync: Vec::new(),
        local_disposal: LocalDisposal::Permanent,
    }))
}

/// One `proton-sync` invocation against a scripted fake daemon: every request it sent, decoded,
/// plus its stdout/stderr/exit status.
struct Run {
    requests: Vec<ControlRequest>,
    stdout: String,
    stderr: String,
    success: bool,
}

/// `listener.accept()` with a bounded wait instead of blocking forever.
///
/// A test here scripts exactly as many replies as the current code makes round trips for. If a
/// future change reduces that count (a poll loop that resolves in fewer trips, say), an unbounded
/// `accept()` would then wait for a connection the already-exited CLI process will never make —
/// `server.join()` hangs forever and the whole suite (and CI) hangs with it, silently. Bounding it
/// turns that into a failing assertion instead: the server thread gives up, `drive` returns with
/// fewer recorded requests than the test expects, and the length assertion fails loudly.
fn accept_bounded(listener: &UnixListener, timeout: Duration) -> Option<UnixStream> {
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).expect("set stream blocking");
                return Some(stream);
            }
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
}

/// Runs the real `proton-sync` binary against `args`, serving `replies` in order, one per
/// connection (the capability gate's own probe is always the first). Every decoded request is
/// recorded in order, so a test can assert "every poll after the ack still carried `--pair work`".
fn drive(args: &[&str], replies: Vec<ControlResponse>) -> Run {
    let directory = tempfile::tempdir().expect("tempdir");
    let socket_path = directory.path().join("fake.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind fake daemon socket");
    let seen: Arc<Mutex<Vec<ControlRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_server = Arc::clone(&seen);
    let server = thread::spawn(move || {
        for reply in replies {
            let Some(stream) = accept_bounded(&listener, Duration::from_secs(10)) else {
                break;
            };
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("read request line");
            seen_server
                .lock()
                .expect("seen lock")
                .push(serde_json::from_str(line.trim_end()).expect("decode request"));
            let mut body = serde_json::to_vec(&reply).expect("serialize reply");
            body.push(b'\n');
            (&stream).write_all(&body).expect("write reply");
            (&stream).flush().expect("flush reply");
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
        .arg("--socket-path")
        .arg(&socket_path)
        .args(args)
        .output()
        .expect("run proton-sync");
    server.join().expect("fake daemon thread panicked");
    let requests = seen.lock().expect("seen lock").clone();
    Run {
        requests,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    }
}

fn pairs_of(run: &Run) -> Vec<Option<String>> {
    run.requests
        .iter()
        .map(|request| request.pair.clone())
        .collect()
}

/// `print_pending`'s `approve`/`keep` lines and `print_status`'s deletions row are two of the
/// call sites 99ef0c4 rewired to `cli_hint(...)` — see this file's own doc for why nothing else
/// pins them against reverting to a bare literal.
#[test]
fn pending_and_status_hints_carry_the_pair_selector() {
    let mut with_pending = blank();
    with_pending.pending_deletions = vec![pending_deletion("docs/a.txt")];
    let run = drive(
        &["--pair", "work", "pending"],
        vec![blank(), with_pending.clone()],
    );
    assert_eq!(pairs_of(&run), vec![None, Some("work".to_owned())]);
    assert!(
        run.stdout
            .contains("proton-sync --pair work approve <path>"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("proton-sync --pair work keep <path>"),
        "stdout: {}",
        run.stdout
    );

    let run = drive(&["--pair", "work", "status"], vec![blank(), with_pending]);
    assert_eq!(pairs_of(&run), vec![None, Some("work".to_owned())]);
    assert!(
        run.stdout
            .contains("review with `proton-sync --pair work pending`"),
        "stdout: {}",
        run.stdout
    );
}

/// The `Pause` arm in `run_for_pair` prints its own line directly rather than through a shared
/// renderer another test here already covers, so it needs its own pin.
#[test]
fn pause_hint_carries_the_pair_selector() {
    let mut ack = blank();
    ack.message = "sync paused".to_owned();
    let run = drive(&["--pair", "work", "pause"], vec![blank(), ack]);
    assert!(
        run.stdout
            .contains("resume with `proton-sync --pair work resume`"),
        "stdout: {}",
        run.stdout
    );
}

/// `watch_syncnow` issues its own poll requests independently of the initial ack. A selector
/// dropped there sends every poll to the default pair, which then reports the wrong pass's
/// progress (or "done") while the pair actually asked for never gets watched at all.
#[test]
fn syncnow_polls_carry_the_pair_selector() {
    let mut ack = blank();
    ack.message = "sync scheduled".to_owned();
    ack.reconcile_seq = 1;
    let mut still = blank();
    still.reconcile_seq = 1;
    still.syncing = true;
    let mut done = blank();
    done.reconcile_seq = 2;
    let run = drive(
        &["--pair", "work", "syncnow"],
        vec![blank(), ack, still, done],
    );
    assert_eq!(run.requests.len(), 4, "requests: {:?}", run.requests);
    for request in &run.requests[1..] {
        assert_eq!(
            request.pair.as_deref(),
            Some("work"),
            "failing: {request:?}\nall: {:?}\nstdout: {}\nstderr: {}",
            run.requests,
            run.stdout,
            run.stderr
        );
    }
}

/// Same rule as `syncnow`, for `watch_plan`'s polls and `run_it_line`'s "apply this" hint — and
/// the paused-ack arm, which is a different code path entirely (`report_plan`'s early return,
/// never reaching `watch_plan`'s poll loop at all).
#[test]
fn plan_polls_and_hints_carry_the_pair_selector() {
    let mut ack = blank();
    ack.plan = Some(PlanOutcome::Scheduled { plan_seq: 3 });
    let mut computing = blank();
    computing.plan = Some(PlanOutcome::Computing { plan_seq: 2 });
    let mut computed = blank();
    computed.plan = Some(reviewed("3:tok", 1));
    let run = drive(
        &["--pair", "work", "plan"],
        vec![blank(), ack, computing, computed],
    );
    assert_eq!(run.requests.len(), 4, "requests: {:?}", run.requests);
    for request in &run.requests[1..] {
        assert_eq!(
            request.pair.as_deref(),
            Some("work"),
            "failing: {request:?}\nall: {:?}\nstdout: {}\nstderr: {}",
            run.requests,
            run.stdout,
            run.stderr
        );
    }
    assert!(
        run.stdout
            .contains("Run it with: proton-sync --pair work apply 3:tok"),
        "stdout: {}",
        run.stdout
    );

    let mut paused = blank();
    paused.plan = Some(PlanOutcome::Paused);
    let run = drive(&["--pair", "work", "plan"], vec![blank(), paused]);
    assert!(
        run.stderr
            .contains("Resume with `proton-sync --pair work resume`"),
        "stderr: {}",
        run.stderr
    );
}

/// The apply family, including the one bug 99ef0c4's commit message calls out by name: the
/// divergence refetch used to build its second `plan_result` request with `ControlRequest::new`,
/// so a diverged `--pair work apply` printed the DEFAULT pair's replacement plan under `work`'s
/// verdict. Covers all three ways an apply ends: diverged-with-a-fresh-plan, stale, and
/// diverged-with-a-failed-refetch (the last replies vector is one short on purpose — see
/// `refetch_plan_if_diverged`'s own doc on degrading rather than failing).
#[test]
fn apply_polls_refetch_and_hints_carry_the_pair_selector() {
    let mut ack = blank();
    ack.apply = Some(ApplyOutcome::Scheduled { apply_seq: 7 });
    let mut diverged = blank();
    diverged.apply = Some(ApplyOutcome::Diverged { apply_seq: 7 });
    let mut refetched = blank();
    refetched.plan = Some(reviewed("4:new", 2));
    let run = drive(
        &["--pair", "work", "apply", "3:tok"],
        vec![blank(), ack, diverged, refetched],
    );
    assert_eq!(run.requests.len(), 4, "requests: {:?}", run.requests);
    for request in &run.requests[1..] {
        assert_eq!(
            request.pair.as_deref(),
            Some("work"),
            "failing: {request:?}\nall: {:?}\nstdout: {}\nstderr: {}",
            run.requests,
            run.stdout,
            run.stderr
        );
    }
    assert!(
        run.stdout
            .contains("Run it with: proton-sync --pair work apply 4:new"),
        "stdout: {}",
        run.stdout
    );

    let mut stale = blank();
    stale.apply = Some(ApplyOutcome::Stale);
    let run = drive(&["--pair", "work", "apply", "3:tok"], vec![blank(), stale]);
    assert!(
        run.stderr
            .contains("Run `proton-sync --pair work plan` again"),
        "stderr: {}",
        run.stderr
    );

    // Deliberately one reply short: the refetch's own connection is never served, so it degrades
    // to the divergence without the fresh plan rather than failing the command outright.
    let mut ack = blank();
    ack.apply = Some(ApplyOutcome::Scheduled { apply_seq: 7 });
    let mut diverged = blank();
    diverged.apply = Some(ApplyOutcome::Diverged { apply_seq: 7 });
    let run = drive(
        &["--pair", "work", "apply", "3:tok"],
        vec![blank(), ack, diverged],
    );
    assert!(
        run.stderr
            .contains("Run `proton-sync --pair work plan` to see the new one"),
        "stderr: {}",
        run.stderr
    );
}

/// `approve`/`deny`/`keep` print `response.message` verbatim — the CLI calls no `cli_hint` of its
/// own here, the daemon composes the whole line (#409's daemon-side half,
/// `daemon::tests::apply_approval_command_hint_echoes_the_pair_selector` and its siblings pin
/// THAT). A fake daemon cannot prove the daemon composes the string correctly; what it proves is
/// the two things the CLI itself owns: the wire carries `pair: Some("work")`, and the reply's
/// `message` — `--pair work` and all — reaches stdout unchanged.
#[test]
fn approve_passes_through_the_daemons_own_pair_carrying_message() {
    let mut reply = blank();
    reply.message =
        "approved 1 pending deletion(s); run `proton-sync --pair work syncnow` to apply now"
            .to_owned();
    let run = drive(
        &["--pair", "work", "approve", "docs/a.txt"],
        vec![blank(), reply],
    );
    assert_eq!(pairs_of(&run), vec![None, Some("work".to_owned())]);
    assert!(
        run.stdout
            .contains("run `proton-sync --pair work syncnow` to apply now"),
        "stdout: {}",
        run.stdout
    );
}

/// `--all-pairs` fans a command out once per configured pair (`print_pair_reply`), keyed on each
/// pair's own name rather than the selector the process was invoked with (there was none) — a
/// fan-out that echoed a single stored selector instead would print the SAME pair on every line.
#[test]
fn all_pairs_fanout_carries_each_pairs_own_name() {
    let mut default_reply = blank();
    default_reply.pair = Some("default".to_owned());
    default_reply.pending_deletions = vec![pending_deletion("x")];
    let mut work_reply = blank();
    work_reply.pair = Some("work".to_owned());
    work_reply.pending_deletions = vec![pending_deletion("y")];
    let run = drive(
        &["--all-pairs", "pending"],
        vec![blank(), default_reply, work_reply],
    );
    assert_eq!(
        pairs_of(&run),
        vec![None, Some("default".to_owned()), Some("work".to_owned())]
    );
    assert!(
        run.stdout
            .contains("proton-sync --pair default approve <path>"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("proton-sync --pair work approve <path>"),
        "stdout: {}",
        run.stdout
    );

    let mut default_reply = blank();
    default_reply.pair = Some("default".to_owned());
    default_reply.plan = Some(PlanOutcome::Paused);
    let mut work_reply = blank();
    work_reply.pair = Some("work".to_owned());
    work_reply.plan = Some(reviewed("9:z", 1));
    let run = drive(
        &["--all-pairs", "plan"],
        vec![blank(), default_reply, work_reply],
    );
    assert!(
        run.stderr
            .contains("Resume with `proton-sync --pair default resume`"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stdout
            .contains("Run it with: proton-sync --pair work apply 9:z"),
        "stdout: {}",
        run.stdout
    );
}

/// An explicit `--pair` that did not resolve must fail because `response.pair` came back `None`
/// — never by pattern-matching `response.message`, which is free-form daemon text the CLI must
/// still print but must not parse. The scripted message below is deliberately NOT worded like a
/// "no such pair" error, so a regression to string-matching would read this as success.
#[test]
fn an_unresolved_selector_fails_by_the_response_shape_not_by_matching_message_text() {
    let mut refused = blank();
    refused.pair = None;
    refused.message = "arbitrary daemon text unrelated to pairs".to_owned();
    let run = drive(&["--pair", "wrok", "status"], vec![blank(), refused]);
    assert!(
        !run.success,
        "an unresolved selector must fail regardless of how the message is worded"
    );
    assert!(
        run.stderr
            .contains("arbitrary daemon text unrelated to pairs"),
        "the daemon's own message must still reach the user verbatim: {}",
        run.stderr
    );
}

/// `--all-pairs --json`'s JSON twin of the bug `9a8d304` fixed for the human branch (#409's
/// Copilot finding, validated): before this, the JSON branch folded only the resolved-selector
/// case, never the verb's own outcome, so a busy `list` printed its payload on stdout and still
/// exited 0. Drives the real binary so the assertion is on the process's own exit status.
#[test]
fn all_pairs_json_list_exits_nonzero_when_the_listing_is_busy() {
    let mut probe = blank();
    probe.pairs = vec![summary("default")];
    let mut busy = blank();
    busy.listing = Some(ListingOutcome::Busy);
    let run = drive(&["--all-pairs", "--json", "list"], vec![probe, busy]);
    assert!(
        !run.success,
        "a busy listing under --all-pairs --json must exit non-zero: stdout={} stderr={}",
        run.stdout, run.stderr
    );
}
