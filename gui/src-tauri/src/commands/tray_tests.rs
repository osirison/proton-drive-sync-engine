//! The tray's rows against the recording fake daemon (#102 phase 5d, ADR 0005 §4 and the brief's
//! section 4.4): a folder row acts on its own folder, the panel is answered its own status, `Sync now`
//! reaches every unpaused folder, and at one folder every request is the one this app always sent.
//!
//! Like `selection_tests`, each test drives the real function over a real socket with the config in a
//! temp directory. The rows' wire is `tray_control_row` — one body that the panel's `tray_action` and
//! both native menus' `handle_menu_event` run — so what is asserted here is what all three do.

use super::pair_tests::{harness, two_pair_daemon, Harness, TWO_PAIR_FILE};
use super::*;
use gui_core::testing::{FakeDaemon, FakePair};
use gui_core::wire::ControlCommand as Verb;

macro_rules! run {
    ($future:expr) => {
        tauri::async_runtime::block_on($future)
    };
}

/// `(command, selector)` of every request the daemon has seen, in order.
fn sent(daemon: &FakeDaemon) -> Vec<(Verb, Option<String>)> {
    daemon
        .parsed_requests()
        .iter()
        .map(|request| (request.command.clone(), request.pair.clone()))
        .collect()
}

fn some(name: &str) -> Option<String> {
    Some(name.to_owned())
}

/// The app has heard the daemon's folder list, which is what every tray click is judged against
/// (`roster_has_many`) — the native poll and the panel's own poll both do this every two seconds.
fn learn_the_folders(h: &Harness) {
    run!(tray_status(h.app.handle().clone()));
    h.daemon.clear_requests();
}

fn three_folders(paused: &[&str]) -> FakeDaemon {
    let pair = |name: &str| {
        let mut pair = FakePair::new(name);
        pair.paused = paused.contains(&name);
        pair
    };
    FakeDaemon::multi_pair(vec![pair("docs"), pair("photos"), pair("music")]).start()
}

// ---- the panel's poll -------------------------------------------------------------------------------

/// Carried item (a) of the PR: the panel's very first poll, before it has heard a list of folders from
/// which to name one, used to name none — which Rust reads as the pair the WINDOW has selected. With
/// `photos` selected the panel therefore drew `photos` for one tick, beside rows that act on `docs`.
/// `tray_status` asks for the default pair by construction, so there is no first poll that can differ.
///
/// Revert: answer `tray_status` with `Ask::Selected` — the first call then goes out unaddressed and
/// comes back about `photos`.
#[test]
fn the_tray_polls_the_default_pair_even_on_its_first_call_with_another_pair_selected() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    let handle = || h.app.handle().clone();
    run!(select_pair(handle(), "photos".to_owned())).unwrap();
    h.daemon.clear_requests();

    let first = run!(tray_status(handle()));

    assert_eq!(
        sent(&h.daemon),
        [(Verb::Status, None)],
        "one request, about the default pair, and no second one addressed at the selection"
    );
    assert_eq!(
        first.response.as_ref().and_then(|r| r.pair.as_deref()),
        Some("docs"),
        "what the panel is handed describes the default pair, not the window's selection"
    );
    assert_eq!(
        first.pairs.len(),
        2,
        "and lists every folder, which is all its rows need"
    );
    assert_eq!(first.pair_states.len(), 2);
    assert!(first
        .pair_states
        .iter()
        .all(|s| s.rank == gui_core::state::severity(s.state)));

    // The window's own read is still about the selection: the two surfaces are not the same question.
    h.daemon.clear_requests();
    let window = run!(get_status(handle(), None));
    assert_eq!(
        window.response.as_ref().and_then(|r| r.pair.as_deref()),
        Some("photos")
    );
}

// ---- a folder's row -----------------------------------------------------------------------------------

/// Acceptance 3: a click on a row drawn for `photos` pauses `photos` — addressed — and nothing else.
/// And acceptance 7 / `a_pair_row_returns_the_panels_own_status`: what the panel is handed back is
/// the DEFAULT folder's status, never the reply to the pause (which describes `photos` and would be
/// published into a panel that is showing something else).
///
/// Revert (the second half): return the addressed command's reply.
#[test]
fn a_folder_row_pauses_its_own_folder_and_the_panel_is_handed_its_own_status() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    learn_the_folders(&h);

    let payload = run!(tray_control_row(
        h.app.handle().clone(),
        &TrayRow::PausePair("photos".to_owned())
    ));

    assert_eq!(
        sent(&h.daemon),
        [(Verb::Pause, some("photos")), (Verb::Status, None),],
        "the pause is addressed to photos, then the panel's own unaddressed status is read"
    );
    assert!(h.daemon.is_paused("photos"));
    assert!(
        !h.daemon.is_paused("docs"),
        "the default folder is not touched"
    );
    assert_eq!(
        payload.response.as_ref().and_then(|r| r.pair.as_deref()),
        Some("docs"),
        "the panel is handed the default folder's reply, not photos'"
    );
    let photos = payload.pairs.iter().find(|p| p.name == "photos").unwrap();
    assert!(
        photos.paused,
        "and that reply already lists photos as paused"
    );
}

/// The default folder is addressed by omission on the wire, as everywhere else — a row for it names
/// it, and the request does not.
#[test]
fn a_row_for_the_default_folder_is_addressed_by_omission() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    learn_the_folders(&h);

    run!(tray_control_row(
        h.app.handle().clone(),
        &TrayRow::PausePair("docs".to_owned())
    ));

    assert_eq!(sent(&h.daemon)[0], (Verb::Pause, None));
    assert!(h.daemon.is_paused("docs"));
    assert!(!h.daemon.is_paused("photos"));
}

/// Acceptance 3, the stale-menu half. A menu drawn when the folders were `docs` and `photos` still
/// carries `pause@photos` after the daemon restarted onto `music` and `photos` — and the click goes to
/// `photos`, addressed (it is no longer the default), because the row carries its folder and not a
/// position.
#[test]
fn a_click_on_an_id_issued_for_photos_reaches_photos_even_after_the_list_changed() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    learn_the_folders(&h);
    let row = crate::commands::tray_row("pause@photos").expect("the id was issued for photos");

    h.daemon
        .set_pairs(vec![FakePair::new("music"), FakePair::new("photos")]);
    learn_the_folders(&h); // the next poll: the list the clicks are judged against has changed

    run!(tray_control_row(h.app.handle().clone(), &row));

    assert_eq!(sent(&h.daemon)[0], (Verb::Pause, some("photos")));
    assert!(h.daemon.is_paused("photos"));
    assert!(
        !h.daemon.is_paused("music"),
        "and not whatever stands first now"
    );
}

/// An id naming a folder that is gone acts on nothing, whether the app has heard yet or not. Not heard:
/// the request goes out and the DAEMON's byte-exact selector rule refuses it (`pair: None`, nothing
/// done). Heard: the app refuses it before it leaves.
#[test]
fn a_row_for_a_folder_that_went_acts_on_nothing() {
    for heard in [false, true] {
        let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
        learn_the_folders(&h);
        h.daemon.set_pairs(vec![FakePair::new("docs")]);
        if heard {
            learn_the_folders(&h);
        }

        run!(tray_control_row(
            h.app.handle().clone(),
            &TrayRow::PausePair("photos".to_owned())
        ));

        assert!(
            !h.daemon.is_paused("docs"),
            "heard={heard}: a refused row paused the folder that remained"
        );
        let pauses: Vec<_> = sent(&h.daemon)
            .into_iter()
            .filter(|(verb, _)| *verb == Verb::Pause)
            .collect();
        if heard {
            assert!(
                pauses.is_empty(),
                "the app already knows photos is gone: {pauses:?}"
            );
        } else {
            // The one request that left was addressed at photos, and the daemon did nothing with it.
            assert_eq!(pauses, [(Verb::Pause, some("photos"))]);
        }
    }
}

/// Resume is the same row the other way round.
#[test]
fn a_resume_row_resumes_its_folder() {
    let mut photos = FakePair::new("photos");
    photos.paused = true;
    let h = harness(
        FakeDaemon::multi_pair(vec![FakePair::new("docs"), photos]).start(),
        Some(TWO_PAIR_FILE),
    );
    learn_the_folders(&h);
    assert!(h.daemon.is_paused("photos"), "the premise");

    run!(tray_control_row(
        h.app.handle().clone(),
        &TrayRow::ResumePair("photos".to_owned())
    ));

    assert_eq!(sent(&h.daemon)[0], (Verb::Resume, some("photos")));
    assert!(!h.daemon.is_paused("photos"));
}

// ---- Sync now -------------------------------------------------------------------------------------------

/// D5: one `Sync now` row reaches every UNPAUSED folder, each addressed, a paused one left alone.
#[test]
fn sync_now_reaches_every_unpaused_folder_and_leaves_a_paused_one_alone() {
    let h = harness(three_folders(&["photos"]), None);
    learn_the_folders(&h);

    run!(tray_control_row(h.app.handle().clone(), &TrayRow::SyncNow));

    let verbs = sent(&h.daemon);
    let syncs: Vec<_> = verbs
        .iter()
        .filter(|(verb, _)| *verb == Verb::Syncnow)
        .cloned()
        .collect();
    assert_eq!(
        syncs,
        [(Verb::Syncnow, None), (Verb::Syncnow, some("music"))],
        "docs (the default, by omission) and music — not the paused photos: {verbs:?}"
    );
}

/// D2 at the command: with one folder `Sync now` is the single unaddressed request it always was —
/// no status read first, nothing else.
#[test]
fn sync_now_at_one_folder_is_todays_single_request() {
    for daemon in [
        FakeDaemon::multi_pair(vec![FakePair::new("docs")]).start(),
        FakeDaemon::legacy(FakePair::new("default")).start(),
    ] {
        let h = harness(daemon, None);
        learn_the_folders(&h);

        run!(tray_control_row(h.app.handle().clone(), &TrayRow::SyncNow));

        assert_eq!(sent(&h.daemon), [(Verb::Syncnow, None)]);
    }
}

// ---- the rows of a menu drawn before the second folder arrived --------------------------------------------

/// A stale `Pause syncing` (the row of a menu drawn at one folder) acts on nothing once the daemon
/// lists two: its label said "syncing", which at one folder meant the only folder.
#[test]
fn a_stale_unaddressed_pause_acts_on_nothing_once_there_are_two_folders() {
    let h = harness(two_pair_daemon(), Some(TWO_PAIR_FILE));
    learn_the_folders(&h);

    for row in [TrayRow::Pause, TrayRow::Resume] {
        let payload = run!(tray_control_row(h.app.handle().clone(), &row));
        assert!(payload.error.is_none(), "{:?}", payload.error);
    }

    let verbs = sent(&h.daemon);
    assert!(
        verbs.iter().all(|(verb, _)| *verb == Verb::Status),
        "only the panel's own status may leave: {verbs:?}"
    );
    assert!(!h.daemon.is_paused("docs"));
}

/// …and at one folder, or against a daemon that lists none, they are what they always were.
#[test]
fn pause_at_one_folder_is_todays_unaddressed_request() {
    for daemon in [
        FakeDaemon::multi_pair(vec![FakePair::new("docs")]).start(),
        FakeDaemon::legacy(FakePair::new("default")).start(),
    ] {
        let h = harness(daemon, None);
        learn_the_folders(&h);
        run!(tray_control_row(h.app.handle().clone(), &TrayRow::Pause));
        assert_eq!(sent(&h.daemon), [(Verb::Pause, None)]);
    }
}

/// A legacy daemon whose config FILE declares two tables still gets today's `Pause syncing`: only the
/// DAEMON's own list decides, because the rows the tray draws are decided by it too.
#[test]
fn a_file_with_two_tables_does_not_refuse_a_legacy_daemons_pause() {
    let h = harness(
        FakeDaemon::legacy(FakePair::new("docs")).start(),
        Some(TWO_PAIR_FILE),
    );
    learn_the_folders(&h);
    run!(tray_control_row(h.app.handle().clone(), &TrayRow::Pause));
    assert_eq!(sent(&h.daemon), [(Verb::Pause, None)]);
}
