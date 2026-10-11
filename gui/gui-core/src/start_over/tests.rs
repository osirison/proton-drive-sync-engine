//! Starting a folder over when its Proton folder changes (#453, the Settings half). Every test works in a
//! temporary directory with a real SQLite index written by the engine's own functions, so what is
//! asserted about the index is what the daemon would read: nothing here reaches a daemon, a socket or
//! the machine's state.

use super::testing::{facts, give_history, started_over, view_of, with_history};
use super::*;
use proton_drive_sync_engine::index::load_existing_recorded_remote_root;
use std::fs;
use std::path::{Path, PathBuf};

/// A folder the daemon has synced against `/Drive/old`, with its history (`testing::give_history`).
fn synced_folder(base: &Path, paused: bool) -> PairView {
    let view = view_of(&base.join("docs"));
    give_history(&view, paused);
    view
}

// ---- the question --------------------------------------------------------------------------------

#[test]
fn a_respelling_of_the_saved_folder_is_not_a_change() {
    // The engine's own rule decides (`config::same_remote_folder`): none of these is another folder,
    // so none may ask the person anything.
    for staged in [
        "/Drive/old",
        "Drive/old",
        "/Drive/old/",
        "./Drive/old",
        "Drive/./old",
    ] {
        assert!(
            !remote_folder_changed(Some("/Drive/old"), staged),
            "{staged} is the saved folder"
        );
    }
    assert!(!remote_folder_changed(Some("Drive/old/"), "/Drive/old"));
}

#[test]
fn another_folder_is_a_change_however_close() {
    for staged in [
        "/Drive/new",
        "/Drive/old/sub",
        "/Drive",
        "/Drive/Old",
        "/Drive/old2",
        "/Drive/old ",
        "/",
    ] {
        assert!(
            remote_folder_changed(Some("/Drive/old"), staged),
            "{staged} is not the saved folder"
        );
    }
}

#[test]
fn nothing_saved_or_nothing_staged_is_not_a_change_to_start_over_from() {
    // A pair with no saved Proton folder has no baseline to resume, and an empty staged value is a
    // save the config writer refuses in the engine's words: neither is this module's to prompt for.
    assert!(!remote_folder_changed(None, "/Drive/new"));
    assert!(!remote_folder_changed(Some(""), "/Drive/new"));
    assert!(!remote_folder_changed(Some("   "), "/Drive/new"));
    assert!(!remote_folder_changed(Some("/Drive/old"), ""));
    assert!(!remote_folder_changed(Some("/Drive/old"), "  "));
}

// ---- starting over -------------------------------------------------------------------------------

#[test]
fn starting_over_empties_everything_the_daemon_decides_from_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), false);
    let root = view.local_root.clone().unwrap();
    assert_eq!(facts(&view), with_history(false), "precondition");

    let done = start_over(&view).expect("nothing holds the folder");

    assert_eq!(done.index, view.db_path, "it says which index it reset");
    // The baseline, the folder it described (the engine would hold the pair for it otherwise), the
    // cursor and the warm-start count are gone; the history of what happened is not learned state and
    // stays, and so do the person's files, byte for byte.
    assert_eq!(facts(&view), started_over(false));
    assert_eq!(
        fs::read(root.join("mine.txt")).unwrap(),
        b"the person's file"
    );
}

#[test]
fn a_paused_folder_is_still_paused_after_it_starts_over() {
    // The engine keeps the pause through a reset on purpose (`load_pair_paused`): a start-over that
    // quietly resumed the folder would sync one the person had told it to leave alone.
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), true);

    start_over(&view).expect("started over");

    assert_eq!(facts(&view), started_over(true));
}

#[test]
fn what_the_engine_would_hold_a_folder_for_is_gone() {
    // The hold reads exactly this: `load_existing_recorded_remote_root` on the index file, read-only,
    // as a held pair's retry and `--dry-run` do. After the start-over it must say nothing is recorded.
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), false);
    let db = view.db_path.clone().unwrap();
    assert_eq!(
        load_existing_recorded_remote_root(&db).unwrap(),
        Some(PathBuf::from("Drive/old")),
        "precondition: the engine would hold this folder for another Proton folder"
    );

    start_over(&view).expect("started over");

    assert_eq!(load_existing_recorded_remote_root(&db).unwrap(), None);
}

#[test]
fn a_folder_with_no_index_has_nothing_to_start_over_and_gets_nothing_made() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("mine.txt"), "x").unwrap();
    let view = view_of(&root);

    let done = start_over(&view).expect("nothing to reset is not a failure");

    assert_eq!(done.index, None);
    assert!(
        !root.join(".sync").exists(),
        "no state directory, no index and no lock were made in the person's folder"
    );
}

#[test]
fn a_daemon_that_holds_the_folder_refuses_the_start_over_and_the_index_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), false);
    let before = fs::read(view.db_path.as_ref().unwrap()).unwrap();
    // What a running daemon holds: the pair's own lockfile, exclusively.
    let held = match set_aside::acquire(view.lockfile_path.as_deref()) {
        set_aside::LockProbe::Free(guard) => guard,
        other => panic!("precondition: nobody holds the lock yet, got {other:?}"),
    };

    let refused = start_over(&view).expect_err("a held folder is not reset");

    assert!(matches!(refused, Refusal::InUse(_)), "{refused:?}");
    assert_eq!(fs::read(view.db_path.as_ref().unwrap()).unwrap(), before);
    drop(held);
    start_over(&view).expect("and once the holder lets go it is reset");
}

#[test]
fn a_folder_that_cannot_be_looked_at_is_not_reset_and_not_made() {
    // An unplugged drive's mount point is gone: "no index there" would be a claim about a folder nobody
    // could read, and the index that matters is on the drive.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("not-mounted");
    let view = view_of(&root);

    let refused = start_over(&view).expect_err("nothing could be looked at");

    assert!(matches!(refused, Refusal::CannotLocate(_)), "{refused:?}");
    assert!(!root.exists(), "the folder was not made");
}

#[test]
fn a_relative_path_is_not_followed_into_the_apps_own_working_directory() {
    // The daemon resolves a relative path against ITS working directory, not this app's; looking
    // relative to ours would reset some other folder's index.
    let dir = tempfile::tempdir().unwrap();
    let mut view = synced_folder(dir.path(), false);
    view.db_path = Some(PathBuf::from("sync_index.db"));

    let refused = start_over(&view).expect_err("a relative index path cannot be located");

    assert!(matches!(refused, Refusal::CannotLocate(_)), "{refused:?}");
}

#[test]
fn an_index_that_is_not_a_database_is_left_exactly_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(root.join(".sync")).unwrap();
    let view = view_of(&root);
    let db = view.db_path.clone().unwrap();
    fs::write(
        &db,
        b"this is not a sqlite file, and it is not ours to rewrite",
    )
    .unwrap();
    let before = fs::read(&db).unwrap();

    let refused = start_over(&view).expect_err("a file that is not an index is not reset");

    assert!(matches!(refused, Refusal::Failed(_)), "{refused:?}");
    assert_eq!(fs::read(&db).unwrap(), before);
}

#[test]
fn an_index_path_that_is_a_directory_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(root.join(".sync/sync_index.db")).unwrap();
    let view = view_of(&root);

    let refused = start_over(&view).expect_err("a directory is not an index");

    assert!(matches!(refused, Refusal::Failed(_)), "{refused:?}");
}

#[test]
fn the_check_that_precedes_a_stop_touches_nothing() {
    // `locate` is what the sequence asks BEFORE it stops anything: it must be able to say "this cannot
    // be done" without having done any of it.
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), false);
    let before = fs::read(view.db_path.as_ref().unwrap()).unwrap();

    locate(&view).expect("this folder can be looked at");

    assert_eq!(fs::read(view.db_path.as_ref().unwrap()).unwrap(), before);
    let missing = view_of(&dir.path().join("elsewhere"));
    assert!(locate(&missing).is_err());
}
