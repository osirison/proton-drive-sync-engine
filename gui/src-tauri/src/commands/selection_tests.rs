//! The selection, the reply that names no pair, the capability gate and the class-W rule (#102
//! phase 5a-2, ADR 0005 §4).
//!
//! Like `pair_tests`, every test here drives the real command against the recording fake daemon
//! (`gui_core::testing`) over a real socket, with the config in a temp directory: a recording of
//! what was SENT is the standard, not a description of what should be.

use super::pair_tests::{harness, refuse_to_launch, two_pair_daemon, Harness, TWO_PAIR_FILE};
use super::*;
use gui_core::state::DaemonState;
use gui_core::testing::{FakeDaemon, FakePair};
use serde_json::Value;
use tauri::Listener;

macro_rules! run {
    ($future:expr) => {
        tauri::async_runtime::block_on($future)
    };
}

fn prefs_of(h: &Harness) -> std::path::PathBuf {
    h._dir.path().join("gui.toml")
}

/// The selector of every request the daemon has seen, in order. `None` is an unaddressed request.
fn selectors(daemon: &FakeDaemon) -> Vec<Option<String>> {
    daemon
        .parsed_requests()
        .iter()
        .map(|request| request.pair.clone())
        .collect()
}

fn some(name: &str) -> Option<String> {
    Some(name.to_owned())
}

/// A status-shaped command's refusal, if it carries one.
fn error_of(payload: StatusPayload) -> Result<(), String> {
    match payload.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

// ---- the selection ------------------------------------------------------------------------------

/// Acceptance 3. Choosing `photos` makes the next class-R request carry `"pair":"photos"`; choosing
/// the default pair makes it stop, because the default is addressed by omission.
///
/// And the first request is unaddressed whatever is chosen: nothing has shown yet that this daemon
/// reads a selector, and a selector sent to one that does not is not refused — it is acted on, on
/// its one pair. The reply that shows it does read one makes the same read repeat, addressed, so
/// the first thing drawn is already the selected pair.
#[test]
fn selecting_a_pair_makes_the_next_read_carry_it_and_choosing_the_default_makes_it_stop() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();

    assert_eq!(
        run!(select_pair(handle(), "photos".to_owned())).as_deref(),
        Ok("photos")
    );
    let first = run!(get_status(handle(), None));
    assert_eq!(
        selectors(&h.daemon),
        [None, some("photos")],
        "unaddressed until the daemon has shown it reads a selector, then again, addressed"
    );
    assert_eq!(first.selected.as_deref(), Some("photos"));
    assert_eq!(
        first.response.as_ref().and_then(|r| r.pair.as_deref()),
        Some("photos"),
        "and what is drawn is about the selected pair, not the default pair's reply"
    );
    assert!(first.pair_unknown.is_none(), "learning is not a fall-back");

    h.daemon.clear_requests();
    let second = run!(get_status(handle(), None));
    assert_eq!(
        selectors(&h.daemon),
        [some("photos")],
        "one request, from now on"
    );
    assert_eq!(second.selected.as_deref(), Some("photos"));
    assert_eq!(
        second.pairs.len(),
        2,
        "every pair rides on the payload, not just the one described"
    );

    run!(select_pair(handle(), "docs".to_owned())).unwrap();
    h.daemon.clear_requests();
    run!(get_status(handle(), None));
    assert_eq!(selectors(&h.daemon), [None]);
}

/// Acceptance 3, the legacy half: a daemon that predates the selector never sees one, and the
/// selection is the default pair whatever is remembered. The file here DOES declare `photos`, and
/// the remembered choice IS `photos` — the case where trusting the memory would be wrong.
#[test]
fn a_legacy_daemon_is_never_sent_a_selector_whatever_is_remembered() {
    let h = harness(
        FakeDaemon::legacy(FakePair::new("default")).start(),
        Some(TWO_PAIR_FILE),
    );
    let handle = || h.app.handle().clone();
    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    assert_eq!(
        gui_core::gui_prefs::load_selected_pair(&prefs_of(&h)).as_deref(),
        Some("photos"),
        "the premise: it is remembered"
    );

    let first = run!(get_status(handle(), None));
    let second = run!(get_status(handle(), None));
    assert_eq!(selectors(&h.daemon), [None, None]);
    assert_eq!(first.selected.as_deref(), Some("default"));
    assert_eq!(second.selected.as_deref(), Some("default"));
    assert!(second.pair_unknown.is_none());
    // And the memory survives: this is not the daemon's to erase.
    assert_eq!(
        gui_core::gui_prefs::load_selected_pair(&prefs_of(&h)).as_deref(),
        Some("photos")
    );
}

/// `an_unresolved_selector_is_not_unreachable`. The daemon restarted onto fewer pairs, so the pair
/// this app remembered does not exist; it answers "no such pair" and acts on nothing. That reply
/// describes no pair that was asked about and is DROPPED: not drawn, and above all not drawn as an
/// outage — the daemon is up. The selection is re-validated against the list that same reply
/// carried, which is the default pair, and the read is repeated once.
#[test]
fn an_unresolved_selector_is_not_unreachable() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    run!(get_status(handle(), None));
    let before = run!(get_status(handle(), None));
    assert_eq!(before.selected.as_deref(), Some("photos"), "the premise");

    h.daemon.set_pairs(vec![FakePair::new("docs")]);
    h.daemon.clear_requests();
    let payload = run!(get_status(handle(), None));

    assert_ne!(
        payload.state,
        DaemonState::Unreachable,
        "the daemon answered"
    );
    assert_eq!(payload.state, DaemonState::Idle);
    assert!(payload.error.is_none(), "{:?}", payload.error);
    assert_eq!(payload.pair_unknown.as_deref(), Some("photos"));
    assert_eq!(payload.selected.as_deref(), Some("docs"));
    assert_eq!(
        payload.response.as_ref().and_then(|r| r.pair.as_deref()),
        Some("docs"),
        "what is drawn is the default pair's reply, never the unresolved one's top level"
    );
    assert_eq!(
        selectors(&h.daemon),
        [some("photos"), None],
        "asked for photos, was told there is none, asked again unaddressed"
    );

    // The preference is a preference, not a fact the daemon can overrule by being momentarily
    // smaller: it is still on disk, and applies again the moment `photos` is back.
    assert_eq!(
        gui_core::gui_prefs::load_selected_pair(&prefs_of(&h)).as_deref(),
        Some("photos")
    );
    h.daemon
        .set_pairs(vec![FakePair::new("docs"), FakePair::new("photos")]);
    let back = run!(get_status(handle(), None));
    assert_eq!(back.selected.as_deref(), Some("photos"));
    assert!(back.pair_unknown.is_none());
}

/// The same reply to a WRITE is refused and never redirected: retrying `pause` for `photos` at the
/// default pair would pause the wrong folder, which is the whole of what the pair argument is for.
#[test]
fn a_write_for_a_pair_that_has_gone_is_refused_and_nothing_else_is_asked() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    run!(get_status(handle(), None)); // the app learns both pairs
    h.daemon.set_pairs(vec![FakePair::new("docs")]); // …and then the daemon loses one
    h.daemon.clear_requests();

    let payload = run!(pause(handle(), "photos".to_owned()));

    assert_eq!(payload.pair_unknown.as_deref(), Some("photos"));
    let error = payload.error.expect("a refusal carries its reason");
    assert!(error.contains("photos"), "{error}");
    assert_eq!(
        selectors(&h.daemon),
        [some("photos")],
        "one request, to the pair that was meant, and no second one anywhere else"
    );
    assert!(
        !h.daemon.is_paused("docs"),
        "the default pair was not paused instead"
    );
}

/// A name this app can place is the whole of "known": a `get_status` that NAMES a pair is the one
/// way a caller overrides the selection, and it too is refused (not redirected) when the pair is not
/// there.
#[test]
fn naming_a_pair_overrides_the_selection_for_a_read_and_an_unknown_name_is_refused() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    run!(get_status(handle(), None));
    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    h.daemon.clear_requests();

    let docs = run!(get_status(handle(), Some("docs".to_owned())));
    assert_eq!(selectors(&h.daemon), [None], "docs is the default: omitted");
    assert_eq!(docs.response.unwrap().pair.as_deref(), Some("docs"));
    assert_eq!(
        docs.selected.as_deref(),
        Some("photos"),
        "`selected` is the app's choice, not the pair this read was about"
    );

    h.daemon.clear_requests();
    let nope = run!(get_status(handle(), Some("nope".to_owned())));
    assert_eq!(nope.pair_unknown.as_deref(), Some("nope"));
    assert!(h.daemon.requests().is_empty(), "nothing leaves for it");
}

// ---- the writes never read the selection ----------------------------------------------------------

/// `a_class_w_command_never_reads_the_selection`. Every command that writes, deletes or starts a
/// multi-step flow takes the pair its CALLER captured, so the selection moving between a click and
/// the command's execution (two webviews, a banner action, the post-restart auto-select) can make a
/// screen stale and never make a deletion land elsewhere.
///
/// Driven both ways round, so the default pair cannot hide a command that reads the selection by
/// agreeing with it: the selection is the OTHER pair each time, and every request a command sends —
/// including each poll — names the pair it was given.
#[test]
fn a_class_w_command_never_reads_the_selection() {
    for (selected, target) in [("photos", "docs"), ("docs", "photos")] {
        let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
        let handle = || h.app.handle().clone();
        let named = || target.to_owned();
        run!(get_status(handle(), None)); // the daemon is heard from, so a selection can apply
        run!(select_pair(handle(), selected.to_owned())).unwrap();
        // The premise: a class-R read DOES follow the selection.
        h.daemon.clear_requests();
        let read = run!(get_status(handle(), None));
        assert_eq!(read.selected.as_deref(), Some(selected));

        let wanted: Option<String> = (target != "docs").then(|| target.to_owned());
        let mut wrong: Vec<String> = Vec::new();
        let mut check = |what: &str| {
            for request in h.daemon.parsed_requests() {
                // The gate's own fresh `status` is unaddressed by design (it asks what the daemon
                // is, not about a pair).
                if request.command == ControlCommand::Status && request.pair.is_none() {
                    continue;
                }
                if request.pair != wanted {
                    wrong.push(format!(
                        "{what}: {:?} addressed {:?}, expected {wanted:?} (selected {selected})",
                        request.command, request.pair
                    ));
                }
            }
            h.daemon.clear_requests();
        };

        h.daemon.clear_requests();
        run!(pause(handle(), named()));
        check("pause");
        assert!(h.daemon.is_paused(target) && !h.daemon.is_paused(selected));
        run!(resume(handle(), named()));
        check("resume");
        run!(sync_now(handle(), named()));
        check("sync_now");
        run!(resync(handle(), named()));
        check("resync");
        run!(approve(handle(), "a.txt".into(), true, None, named()));
        check("approve");
        run!(deny(handle(), "a.txt".into(), true, named()));
        check("deny");
        run!(keep(handle(), "a.txt".into(), true, named()));
        check("keep");
        run!(run_dry_run_with(&h.state(), named(), refuse_to_launch)).unwrap();
        check("run_dry_run");
        run!(apply_plan(handle(), "tok".into(), false, named())).unwrap();
        check("apply_plan");

        assert!(wrong.is_empty(), "{wrong:#?}");
    }
}

/// The same rule for the one class-W command that touches the disk and not the socket: it renames a
/// file in the root of the pair it is GIVEN, not the selected one.
#[test]
fn resolve_conflict_acts_on_the_pair_it_is_given_not_the_selected_one() {
    let docs = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    for (name, root) in [("docs", docs.path()), ("photos", photos.path())] {
        std::fs::write(root.join("note.txt"), format!("pair={name} mine")).unwrap();
        std::fs::write(
            root.join("note.proton-cloud.txt"),
            format!("pair={name} theirs"),
        )
        .unwrap();
    }
    let config = format!(
        "[[pair]]\nname = \"docs\"\nlocal_root = {:?}\nremote_root = \"/Drive/docs\"\n\
         [[pair]]\nname = \"photos\"\nlocal_root = {:?}\nremote_root = \"/Drive/photos\"\n",
        docs.path().display().to_string(),
        photos.path().display().to_string(),
    );
    let h = harness(two_pair_daemon(), Some(&config));
    let handle = || h.app.handle().clone();
    run!(get_status(handle(), None));
    run!(select_pair(handle(), "photos".to_owned())).unwrap();

    resolve_conflict(
        h.state(),
        Conflict {
            original: "note.txt".into(),
            sidecar: "note.proton-cloud.txt".into(),
            kind: conflicts::ConflictKind::Content,
        },
        Resolution::UseProton,
        "docs".to_owned(),
    )
    .expect("applies");

    let text = |root: &std::path::Path| std::fs::read_to_string(root.join("note.txt")).unwrap();
    assert!(text(docs.path()).contains("theirs"), "docs was resolved");
    assert!(
        text(photos.path()).contains("mine"),
        "and photos, the selected pair, was not touched"
    );
}

/// The class-R half of the same table: with the daemon known to read a selector, the reads that
/// default to "the selected pair" really do follow it. (`ROUTED` asserts the default pair with
/// nothing selected; this is the row that makes "selected" and "default" different.)
#[test]
fn class_r_reads_follow_the_selection_when_they_name_no_pair() {
    let r = super::pair_tests::routing_for_selection_tests();
    let handle = || r.h.app.handle().clone();
    run!(get_status(handle(), None));
    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    for (name, observe) in super::pair_tests::class_r_observers() {
        assert_eq!(
            observe(&r, None),
            "photos",
            "{name} with no pair named must read the selected pair, not the default"
        );
    }
}

// ---- the capability gate --------------------------------------------------------------------------

/// A pair of daemons at one socket path: a current one, then (after the first is dropped) a legacy
/// one — "a daemon downgraded in the meantime", the case the gate is for.
///
/// **Kept whole, never destructured.** The fields drop in the order they are declared, harness
/// first: a fake daemon's `Drop` connects to its own socket to wake its accept loop, so the
/// directory holding that socket must outlive it, and two locals bound by a pattern drop in the
/// opposite order and hang the test for ever.
struct Downgrade {
    h: Harness,
    #[allow(dead_code)] // held for its `Drop`: the socket lives in it
    socket_dir: tempfile::TempDir,
}

fn downgraded_after_being_heard_from() -> Downgrade {
    let socket_dir = tempfile::Builder::new().prefix("pds-").tempdir().unwrap();
    let current = FakeDaemon::multi_pair(vec![FakePair::new("docs"), FakePair::new("photos")])
        .in_dir(socket_dir.path())
        .start();
    let h = harness(current, Some(TWO_PAIR_FILE));
    // Heard from, so the app KNOWS both pairs and believes the daemon reads a selector.
    run!(get_status(h.app.handle().clone(), None));
    assert_eq!(
        h.state().lock().unwrap().daemon.capability,
        PairCapability::MultiPair,
        "the premise"
    );
    // Then the daemon is replaced by one that predates the field, at the same socket.
    let Harness { app, daemon, _dir } = h;
    drop(daemon);

    let legacy = FakeDaemon::legacy(FakePair::new("docs"))
        .in_dir(socket_dir.path())
        .start();
    Downgrade {
        h: Harness {
            app,
            daemon: legacy,
            _dir,
        },
        socket_dir,
    }
}

/// `a_destructive_verb_rereads_status_first`, and acceptance 7. A daemon that cannot read `pair`
/// drops it and executes the verb against its one pair, with no signal — so the verbs that destroy
/// data read `status` NOW, and a reply that does not show a selector-reading daemon stops the verb
/// before it is sent. What the recording must show is that the destructive request never arrived.
///
/// A fresh downgrade for every verb: the first refusal corrects what the app believes about the
/// daemon (it is now `Legacy`, and `photos` is no longer a pair it knows), which is right, and means
/// a second verb against the same daemon would be refused for a different reason and never reach
/// the gate this test is about.
#[test]
fn a_destructive_verb_rereads_status_first() {
    type Verb = fn(&Harness) -> Result<(), String>;
    let verbs: [(&str, Verb); 5] = [
        ("approve", |h| {
            error_of(run!(approve(
                h.app.handle().clone(),
                "a".into(),
                true,
                None,
                "photos".to_owned()
            )))
        }),
        ("deny", |h| {
            error_of(run!(deny(
                h.app.handle().clone(),
                "a".into(),
                true,
                "photos".to_owned()
            )))
        }),
        ("keep", |h| {
            error_of(run!(keep(
                h.app.handle().clone(),
                "a".into(),
                true,
                "photos".to_owned()
            )))
        }),
        ("resync", |h| {
            error_of(run!(resync(h.app.handle().clone(), "photos".to_owned())))
        }),
        ("apply_plan", |h| {
            run!(apply_plan(
                h.app.handle().clone(),
                "tok".into(),
                false,
                "photos".to_owned()
            ))
            .map(|_| ())
        }),
    ];
    for (name, verb) in verbs {
        let downgrade = downgraded_after_being_heard_from();
        let h = &downgrade.h;
        assert_eq!(
            verb(h).unwrap_err(),
            NOT_MULTI_PAIR,
            "{name} must be refused with the ADR's sentence"
        );
        let sent: Vec<ControlCommand> = h
            .daemon
            .parsed_requests()
            .into_iter()
            .map(|request| request.command)
            .collect();
        assert_eq!(
            sent,
            [ControlCommand::Status],
            "{name}: one fresh status and then nothing — a destructive request reached a daemon \
             that would have run it on its one pair"
        );
        // And what it learned is kept: the next read knows this daemon is old.
        assert_eq!(
            h.state().lock().unwrap().daemon.capability,
            PairCapability::Legacy,
            "{name}"
        );
    }
}

/// A daemon downgraded to one that predates the selector, WHILE a non-default pair is selected: the
/// next read of the selection is addressed (the app still believes the daemon reads a selector), the
/// legacy daemon drops the field and answers about its one pair, and that reply is not the answer to
/// anything that was asked. It must not be drawn as an outage — the daemon is up — and it must not
/// cost a poll: the reply shows the daemon cannot read a selector, so the read is repeated
/// unaddressed, once, exactly as an unresolved selector is (`an_unresolved_selector_is_not_unreachable`).
#[test]
fn a_selected_read_after_a_downgrade_is_retried_unaddressed_not_reported_unreachable() {
    let downgrade = downgraded_after_being_heard_from();
    let h = &downgrade.h;
    let handle = || h.app.handle().clone();
    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    assert_eq!(
        h.state().lock().unwrap().daemon.capability,
        PairCapability::MultiPair,
        "the premise: the app still believes the daemon reads a selector"
    );
    h.daemon.clear_requests();

    let first = run!(get_status(handle(), None));

    assert_ne!(
        first.state,
        DaemonState::Unreachable,
        "the daemon answered: {:?}",
        first.error
    );
    assert!(first.error.is_none(), "{:?}", first.error);
    assert_eq!(first.state, DaemonState::Idle);
    assert_eq!(
        selectors(&h.daemon),
        [some("photos"), None],
        "asked for photos, was not understood, asked again unaddressed"
    );
    assert_eq!(first.selected.as_deref(), Some("default"));
    assert!(
        first.pair_unknown.is_none(),
        "a downgrade is not a missing pair"
    );
    assert_eq!(
        h.state().lock().unwrap().daemon.capability,
        PairCapability::Legacy
    );
    // And the memory survives: the preference is not the daemon's to erase.
    assert_eq!(
        gui_core::gui_prefs::load_selected_pair(&prefs_of(h)).as_deref(),
        Some("photos")
    );
}

/// The gate asks about a daemon's ability and not about the verb's name only: a verb for the DEFAULT
/// pair is addressed by omission, which every daemon of any age acts on correctly, so nothing is
/// asked and nothing is refused.
#[test]
fn a_destructive_verb_for_the_default_pair_asks_nothing_first() {
    let downgrade = downgraded_after_being_heard_from();
    let h = &downgrade.h;
    let handle = || h.app.handle().clone();
    h.daemon.clear_requests();
    let payload = run!(keep(handle(), "a".into(), true, "docs".to_owned()));
    assert!(payload.error.is_none(), "{:?}", payload.error);
    let sent = h.daemon.parsed_requests();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].command, ControlCommand::Keep);
    assert_eq!(sent[0].pair, None);
}

/// Pause, resume and sync-now are not behind the gate (brief section 2.4): the worst a daemon
/// downgraded in the meantime can do with one is act on its one pair, which is recoverable. They
/// are still not silent about it — the reply is not about the pair that was meant, and says so.
#[test]
fn a_recoverable_verb_is_sent_but_its_reply_is_not_taken_for_the_pair_that_was_meant() {
    let downgrade = downgraded_after_being_heard_from();
    let h = &downgrade.h;
    let payload = run!(pause(h.app.handle().clone(), "photos".to_owned()));
    assert_eq!(payload.error.as_deref(), Some(NOT_MULTI_PAIR));
    assert!(
        payload.response.is_none(),
        "a legacy daemon's pair is not photos"
    );
}

/// The gate's rule on its own, over the three kinds of fresh `status` there are.
#[test]
fn the_gate_wants_a_daemon_that_reads_a_selector_and_runs_that_pair() {
    let multi =
        FakeDaemon::multi_pair(vec![FakePair::new("docs"), FakePair::new("photos")]).start();
    let legacy = FakeDaemon::legacy(FakePair::new("docs")).start();
    let reply = |daemon: &FakeDaemon| {
        gui_core::ipc::command(
            daemon.socket_path(),
            Target::DEFAULT,
            ControlCommand::Status,
            gui_core::ipc::DEFAULT_TIMEOUT,
        )
        .unwrap()
    };
    assert!(gate_decision(&reply(&multi), "photos").is_ok());
    let missing = gate_decision(&reply(&multi), "videos").err().unwrap();
    assert_eq!(missing.unknown_pair.as_deref(), Some("videos"));
    assert!(missing.message.contains("\"docs\""), "{}", missing.message);
    let old = gate_decision(&reply(&legacy), "photos").err().unwrap();
    assert_eq!(old.message, NOT_MULTI_PAIR);
    assert_eq!(old.unknown_pair, None);
}

/// `classify` is how every status-shaped reply is read against the selector it answers.
#[test]
fn a_reply_is_read_against_the_selector_it_answers_by_shape() {
    let multi =
        FakeDaemon::multi_pair(vec![FakePair::new("docs"), FakePair::new("photos")]).start();
    let legacy = FakeDaemon::legacy(FakePair::new("docs")).start();
    let ask = |daemon: &FakeDaemon, target: Target<'_>| {
        gui_core::ipc::command(
            daemon.socket_path(),
            target,
            ControlCommand::Status,
            gui_core::ipc::DEFAULT_TIMEOUT,
        )
        .unwrap()
    };
    let kind = |selector: Option<&str>, reply: ControlResponse| match classify(selector, reply) {
        Ok(Answer::About(_)) => "about",
        Ok(Answer::Unresolved(_)) => "unresolved",
        Ok(Answer::NotUnderstood(_)) => "not understood",
        Err(_) => "malformed",
    };
    // Unaddressed: about the default pair, from any daemon.
    assert_eq!(kind(None, ask(&multi, Target::DEFAULT)), "about");
    assert_eq!(kind(None, ask(&legacy, Target::DEFAULT)), "about");
    // Addressed to a pair that exists: about it.
    assert_eq!(
        kind(Some("photos"), ask(&multi, Target::named("photos"))),
        "about"
    );
    // Addressed to one that does not: unresolved. To a daemon that cannot read it: not understood.
    assert_eq!(
        kind(Some("videos"), ask(&multi, Target::named("videos"))),
        "unresolved"
    );
    assert_eq!(
        kind(Some("photos"), ask(&legacy, Target::named("photos"))),
        "not understood"
    );
    // A reply about a DIFFERENT pair than the one asked, or one that names no pair for an unaddressed
    // request: the daemon is broken, and neither is guessed at.
    assert_eq!(
        kind(Some("docs"), ask(&multi, Target::named("photos"))),
        "malformed"
    );
    let mut nameless = ask(&multi, Target::DEFAULT);
    nameless.pair = None;
    assert_eq!(kind(None, nameless), "malformed");
}

// ---- the payload ----------------------------------------------------------------------------------

/// A daemon that predates the selector receives the object it always did: none of the new pair
/// fields appear, and `selected` is the only addition.
#[test]
fn a_legacy_payload_carries_no_pair_fields_but_the_selection() {
    let h = harness(FakeDaemon::legacy(FakePair::new("default")).start(), None);
    let payload = run!(get_status(h.app.handle().clone(), None));
    let json = serde_json::to_value(&payload).unwrap();
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["response", "selected", "state"], "{json}");
}

/// The state of every pair rides on the payload, and only the pair the reply is about can be
/// first-run: the rest are derived from summaries (F-B).
#[test]
fn every_pairs_state_rides_on_the_payload_and_only_the_described_one_can_be_first_run() {
    let daemon = FakeDaemon::multi_pair(vec![
        FakePair::new("docs").never_synced(),
        FakePair::new("photos").never_synced(),
        FakePair::new("drive").failing("the sync folder /mnt/usb is not available"),
    ])
    .start();
    let file = "[[pair]]\nname = \"docs\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                [[pair]]\nname = \"photos\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
                [[pair]]\nname = \"drive\"\nlocal_root = \"/c\"\nremote_root = \"/Drive/c\"\n";
    let h = harness(daemon, Some(file));
    let handle = || h.app.handle().clone();

    let states = |payload: &StatusPayload| -> Vec<(String, DaemonState)> {
        payload
            .pair_states
            .iter()
            .map(|s| (s.name.clone(), s.state))
            .collect()
    };
    let about_docs = run!(get_status(handle(), None));
    assert_eq!(
        states(&about_docs),
        [
            ("docs".to_owned(), DaemonState::FirstRun),
            ("photos".to_owned(), DaemonState::Idle),
            ("drive".to_owned(), DaemonState::Failed),
        ],
        "docs is the pair the reply describes; photos has never synced as far as its summary says, \
         and a summary cannot say first run"
    );
    // The headline state is the described pair's, exactly the one `pair_states` gives it.
    assert_eq!(about_docs.state, DaemonState::FirstRun);

    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    let about_photos = run!(get_status(handle(), None));
    assert_eq!(
        states(&about_photos),
        [
            ("docs".to_owned(), DaemonState::Idle),
            ("photos".to_owned(), DaemonState::FirstRun),
            ("drive".to_owned(), DaemonState::Failed),
        ]
    );
}

// ---- select_pair itself ---------------------------------------------------------------------------

/// It validates against what exists, writes `gui.toml` first, and announces the choice so the other
/// webview polls at once. A name nobody runs is refused and leaves nothing behind.
#[test]
fn select_pair_validates_persists_and_announces() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    let heard = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let heard = heard.clone();
        h.app.listen("pair-selected", move |event| {
            heard.lock().unwrap().push(event.payload().to_owned());
        });
    }

    let refused = run!(select_pair(handle(), "videos".to_owned()));
    assert!(refused.unwrap_err().contains("videos"));
    assert!(
        !prefs_of(&h).exists(),
        "a refused choice writes nothing: {:?}",
        std::fs::read_to_string(prefs_of(&h))
    );
    assert_eq!(h.state().lock().unwrap().selected, None);
    assert!(heard.lock().unwrap().is_empty());

    assert_eq!(
        run!(select_pair(handle(), "photos".to_owned())).as_deref(),
        Ok("photos")
    );
    assert_eq!(
        gui_core::gui_prefs::load_selected_pair(&prefs_of(&h)).as_deref(),
        Some("photos")
    );
    assert_eq!(
        h.state().lock().unwrap().selected.as_deref(),
        Some("photos")
    );
    assert_eq!(
        *heard.lock().unwrap(),
        ["\"photos\""],
        "the other webview is told, with the name"
    );
}

/// A save re-resolves the paths (it can change a root), and the choice has to outlive it: it is read
/// back from `gui.toml`, the one place it lives, rather than carried across by hand.
#[test]
fn a_save_keeps_the_selection() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    run!(select_pair(h.app.handle().clone(), "photos".to_owned())).unwrap();
    write_config(
        h.state(),
        ConfigUpdate {
            log_level: Some("debug".to_owned()),
            ..Default::default()
        },
    )
    .expect("a daemon-wide edit saves");
    assert_eq!(
        h.state().lock().unwrap().selected.as_deref(),
        Some("photos")
    );
}

/// The selection is read from `gui.toml` beside the config the session names — never from the
/// environment's — when the paths are resolved.
#[test]
fn the_selection_is_loaded_from_the_gui_prefs_beside_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("proton-sync.toml");
    std::fs::write(&config, TWO_PAIR_FILE).unwrap();
    std::fs::write(dir.path().join("gui.toml"), "selected_pair = \"photos\"\n").unwrap();
    assert_eq!(
        RuntimePaths::resolve_at(&config).selected.as_deref(),
        Some("photos")
    );
}

// ---- read_config.pairs -----------------------------------------------------------------------------

fn config_payload(text: Option<&str>) -> ConfigPayload {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proton-sync.toml");
    if let Some(text) = text {
        std::fs::write(&path, text).unwrap();
    }
    let paths = RuntimePaths::resolve_at(&path);
    let app = tauri::test::mock_builder()
        .manage(Mutex::new(paths))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app should build");
    read_config(app.state::<Mutex<RuntimePaths>>()).expect("the file reads")
}

/// `a_pair_file_with_the_daemon_down_is_not_a_fresh_machine`, the Rust half (F-J). In a `[[pair]]`
/// file every root sits inside a table, so the flat top-level roots read `None` — which the first-run
/// check took for "nobody has chosen a folder", with the daemon stopped. The pairs list is what says
/// otherwise.
#[test]
fn read_config_lists_the_pairs_a_pair_file_declares_though_its_flat_roots_are_empty() {
    let payload = config_payload(Some(TWO_PAIR_FILE));
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(
        json["local_root"],
        Value::Null,
        "the premise: nothing at the top level"
    );
    assert_eq!(json["remote_root"], Value::Null);
    let pairs = json["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0]["name"], "docs");
    assert_eq!(pairs[0]["local_root"], "/fake/docs/local");
    assert_eq!(pairs[1]["remote_root"], "/Drive/photos");
}

#[test]
fn read_config_lists_the_one_implicit_pair_of_a_file_with_top_level_roots_and_of_no_file() {
    let json = serde_json::to_value(config_payload(Some(
        "local_root = \"/l\"\nremote_root = \"/Drive/r\"\n",
    )))
    .unwrap();
    assert_eq!(json["pairs"][0]["name"], "default");
    assert_eq!(json["pairs"][0]["local_root"], "/l");
    assert_eq!(json["local_root"], "/l", "the flat reading is unchanged");

    // No file: the implicit pair is there and places nothing, so the check still says "fresh".
    let json = serde_json::to_value(config_payload(None)).unwrap();
    assert_eq!(json["pairs"].as_array().unwrap().len(), 1);
    assert_eq!(json["pairs"][0]["local_root"], Value::Null);
}

/// A file the engine refuses as a config lists no pairs and keeps every flat value, as before.
#[test]
fn read_config_of_a_file_the_engine_refuses_lists_no_pairs_and_keeps_the_flat_values() {
    let json = serde_json::to_value(config_payload(Some(
        "no_such_key = 1\nlocal_root = \"/l\"\nremote_root = \"/Drive/r\"\n",
    )))
    .unwrap();
    assert_eq!(json["pairs"].as_array().unwrap().len(), 0);
    assert_eq!(json["local_root"], "/l");
}
