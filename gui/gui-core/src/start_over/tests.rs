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

    let done = start_over(&view, LocalFolder::Unchanged).expect("nothing holds the folder");

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

    start_over(&view, LocalFolder::Unchanged).expect("started over");

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

    start_over(&view, LocalFolder::Unchanged).expect("started over");

    assert_eq!(load_existing_recorded_remote_root(&db).unwrap(), None);
}

#[test]
fn a_folder_with_no_index_has_nothing_to_start_over_and_gets_nothing_made() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("mine.txt"), "x").unwrap();
    let view = view_of(&root);

    let done =
        start_over(&view, LocalFolder::Unchanged).expect("nothing to reset is not a failure");

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

    let refused =
        start_over(&view, LocalFolder::Unchanged).expect_err("a held folder is not reset");

    assert!(matches!(refused, Refusal::InUse(_)), "{refused:?}");
    assert_eq!(fs::read(view.db_path.as_ref().unwrap()).unwrap(), before);
    drop(held);
    start_over(&view, LocalFolder::Unchanged).expect("and once the holder lets go it is reset");
}

#[test]
fn a_folder_that_cannot_be_looked_at_is_not_reset_and_not_made() {
    // An unplugged drive's mount point is gone: "no index there" would be a claim about a folder nobody
    // could read, and the index that matters is on the drive.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("not-mounted");
    let view = view_of(&root);

    let refused =
        start_over(&view, LocalFolder::Unchanged).expect_err("nothing could be looked at");

    assert!(
        matches!(refused, Refusal::FolderUnreadable(_)),
        "{refused:?}"
    );
    assert!(!root.exists(), "the folder was not made");
}

#[test]
fn a_relative_path_is_not_followed_into_the_apps_own_working_directory() {
    // The daemon resolves a relative path against ITS working directory, not this app's; looking
    // relative to ours would reset some other folder's index.
    let dir = tempfile::tempdir().unwrap();
    let mut view = synced_folder(dir.path(), false);
    view.db_path = Some(PathBuf::from("sync_index.db"));

    let refused = start_over(&view, LocalFolder::Unchanged)
        .expect_err("a relative index path cannot be located");

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

    let refused = start_over(&view, LocalFolder::Unchanged)
        .expect_err("a file that is not an index is not reset");

    assert!(matches!(refused, Refusal::Failed(_)), "{refused:?}");
    assert_eq!(fs::read(&db).unwrap(), before);
}

#[test]
fn an_index_path_that_is_a_directory_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(root.join(".sync/sync_index.db")).unwrap();
    let view = view_of(&root);

    let refused =
        start_over(&view, LocalFolder::Unchanged).expect_err("a directory is not an index");

    assert!(matches!(refused, Refusal::Failed(_)), "{refused:?}");
}

#[test]
fn the_check_that_precedes_a_stop_touches_nothing() {
    // `locate` is what the sequence asks BEFORE it stops anything: it must be able to say "this cannot
    // be done" without having done any of it.
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), false);
    let before = fs::read(view.db_path.as_ref().unwrap()).unwrap();

    locate(&view, LocalFolder::Unchanged).expect("this folder can be looked at");

    assert_eq!(fs::read(view.db_path.as_ref().unwrap()).unwrap(), before);
    let missing = view_of(&dir.path().join("elsewhere"));
    assert!(locate(&missing, LocalFolder::Unchanged).is_err());
}

// ---- the words are the start-over's own (#453 review, finding 3) ----------------------------------------

#[test]
fn a_folder_that_cannot_be_read_is_refused_in_the_start_overs_own_words() {
    // Remove's sentence for this says it "moved nothing". Nothing is moved here: the sentence says the folder
    // cannot be read, that an unplugged drive looks the same, and that the app cannot start it over; what the
    // person can do about it is `advice`.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("not-mounted");
    let view = view_of(&root);

    let refused = locate(&view, LocalFolder::Unchanged).expect_err("nobody could read it");

    assert!(
        matches!(refused, Refusal::FolderUnreadable(_)),
        "{refused:?}"
    );
    let said = refused.reason();
    assert!(
        said.contains(root.to_str().unwrap()) && said.contains("cannot be read"),
        "{said}"
    );
    assert!(
        said.contains("an unplugged drive looks the same")
            && said.contains("the app cannot start it over"),
        "{said}"
    );
    assert!(
        !said.contains("moved") && !said.contains("nothing was moved"),
        "nothing is moved by a start-over: {said}"
    );
    assert_eq!(
        refused.advice(),
        Some("Plug in the drive or make the folder readable, then save again.")
    );
}

#[test]
fn no_refusal_of_the_start_over_says_it_moved_anything() {
    // Every way the disk can fail to say, worded by the start-over and not by a removal.
    let dir = tempfile::tempdir().unwrap();
    let absolute = synced_folder(dir.path(), false);
    let mut refusals = Vec::new();
    // A folder that is not there.
    refusals.push(locate(
        &view_of(&dir.path().join("gone")),
        LocalFolder::Unchanged,
    ));
    // A relative path, for each of the three the config places.
    for view in [
        PairView {
            local_root: Some(PathBuf::from("Sync")),
            ..absolute.clone()
        },
        PairView {
            db_path: Some(PathBuf::from("sync_index.db")),
            ..absolute.clone()
        },
        PairView {
            lockfile_path: Some(PathBuf::from("proton-sync.lock")),
            ..absolute.clone()
        },
    ] {
        refusals.push(locate(&view, LocalFolder::Unchanged));
    }
    // No folder at all.
    refusals.push(locate(
        &PairView {
            local_root: None,
            ..absolute.clone()
        },
        LocalFolder::Unchanged,
    ));
    // A state file on a drive that is not there.
    let away = dir.path().join("usb-not-plugged-in");
    refusals.push(locate(
        &PairView {
            db_path: Some(away.join("idx.db")),
            lockfile_path: Some(away.join("pair.lock")),
            ..absolute
        },
        LocalFolder::Unchanged,
    ));

    assert_eq!(refusals.len(), 6);
    for refusal in refusals {
        let refusal = refusal.expect_err("none of these can be looked at");
        assert!(
            !refusal.reason().contains("moved") && !refusal.reason().contains("nothing was moved"),
            "{}",
            refusal.reason()
        );
    }
}

#[test]
fn a_relative_path_and_a_state_file_on_a_missing_drive_are_worded_by_what_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let absolute = synced_folder(dir.path(), false);

    let relative = locate(
        &PairView {
            db_path: Some(PathBuf::from("sync_index.db")),
            ..absolute.clone()
        },
        LocalFolder::Unchanged,
    )
    .expect_err("relative");
    assert!(matches!(relative, Refusal::CannotLocate(_)), "{relative:?}");
    assert!(
        relative.reason().contains("db_path") && relative.reason().contains("relative path"),
        "{}",
        relative.reason()
    );
    assert_eq!(relative.advice(), None, "waiting does not make it absolute");

    let away = dir.path().join("usb-not-plugged-in");
    let drive = locate(
        &PairView {
            db_path: Some(away.join("idx.db")),
            lockfile_path: Some(away.join("pair.lock")),
            ..absolute
        },
        LocalFolder::Unchanged,
    )
    .expect_err("the index is on a drive that is not there");
    assert!(matches!(drive, Refusal::FolderUnreadable(_)), "{drive:?}");
    assert!(
        drive.reason().contains(away.to_str().unwrap()),
        "{}",
        drive.reason()
    );
}

// ---- a folder the save moves the pair to (#453 review, finding 3) -----------------------------------------

#[test]
fn a_folder_that_does_not_exist_yet_has_no_history_to_start_over_when_the_save_moves_the_pair_to_it()
 {
    // The person typed a new folder together with a new Proton folder. It is not there, under a folder that
    // can be listed: that is an absence, the rule Remove applies to a state file. The daemon makes the folder
    // when it starts; there is nothing to reset, and nothing is made here.
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("brand-new");
    let view = view_of(&fresh);

    assert_eq!(locate(&view, LocalFolder::Moved), Ok(None));
    let done = start_over(&view, LocalFolder::Moved).expect("nothing to start over");

    assert_eq!(done.index, None);
    assert!(!fresh.exists(), "the folder was not made");
    // The same folder, for a pair that already syncs it, is a drive that may come back.
    assert!(matches!(
        locate(&view, LocalFolder::Unchanged),
        Err(Refusal::FolderUnreadable(_))
    ));
}

#[test]
fn a_folder_under_a_folder_that_is_not_there_is_not_known_to_be_absent_even_when_the_save_moves_to_it()
 {
    // `/run/media/me/Backup/docs` with `Backup` gone: an unplugged drive, not a folder that is new.
    let dir = tempfile::tempdir().unwrap();
    let under_nothing = dir.path().join("usb-not-plugged-in/docs");
    let view = view_of(&under_nothing);

    let refused = locate(&view, LocalFolder::Moved).expect_err("it cannot be told");

    assert!(
        matches!(refused, Refusal::FolderUnreadable(_)),
        "{refused:?}"
    );
    assert!(!dir.path().join("usb-not-plugged-in").exists());
}

#[test]
fn a_moved_to_folder_that_is_there_but_cannot_be_read_is_not_an_absence() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let enforced = fs::read_dir(&locked).is_err();
    let refused = locate(&view_of(&locked), LocalFolder::Moved);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if !enforced {
        eprintln!("SKIPPED: this user is not stopped by file permissions (root?)");
        return;
    }

    let refused = refused.expect_err("the history may be right there");
    assert!(
        matches!(refused, Refusal::FolderUnreadable(_)),
        "{refused:?}"
    );
}

#[test]
fn a_moved_to_folder_that_exists_is_started_over_like_any_other() {
    // The save points the pair at a folder that already holds an index from an earlier pair: that index
    // describes another Proton folder, and it is the one that meets the new Proton folder.
    let dir = tempfile::tempdir().unwrap();
    let view = synced_folder(dir.path(), true);

    start_over(&view, LocalFolder::Moved).expect("started over");

    assert_eq!(facts(&view), started_over(true));
}

#[test]
fn an_index_kept_outside_a_folder_that_is_not_there_yet_is_still_history_to_start_over() {
    // The config places the index somewhere else. Moving the pair to a folder that does not exist leaves
    // that index behind with the old baseline: it is what would meet the new Proton folder.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(&root).unwrap();
    let state = dir.path().join("state");
    let view = PairView {
        db_path: Some(state.join("docs.db")),
        lockfile_path: Some(state.join("docs.lock")),
        ..view_of(&root)
    };
    give_history(&view, false);
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(facts(&view), with_history(false), "precondition");

    // The folder is not there and the pair syncs it: a drive that may come back.
    assert!(matches!(
        start_over(&view, LocalFolder::Unchanged),
        Err(Refusal::FolderUnreadable(_))
    ));
    // The save moves the pair to a folder that does not exist yet; the index outside it is reset.
    let done = start_over(&view, LocalFolder::Moved).expect("started over");

    assert_eq!(done.index, view.db_path);
    assert_eq!(facts(&view), started_over(false));
    assert!(!root.exists(), "no folder was made");
}

#[test]
fn an_index_kept_on_a_drive_that_is_not_there_is_not_an_absence_for_a_moved_folder_either() {
    let dir = tempfile::tempdir().unwrap();
    let away = dir.path().join("usb-not-plugged-in");
    let view = PairView {
        db_path: Some(away.join("docs.db")),
        lockfile_path: Some(away.join("docs.lock")),
        ..view_of(&dir.path().join("brand-new"))
    };

    let refused = locate(&view, LocalFolder::Moved).expect_err("the index may be on the drive");

    assert!(
        matches!(refused, Refusal::FolderUnreadable(_)),
        "{refused:?}"
    );
}

#[test]
fn a_folder_is_moved_only_when_it_is_another_folder_not_another_spelling() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    let link = dir.path().join("link-to-docs");
    std::os::unix::fs::symlink(&docs, &link).unwrap();
    let before = view_of(&docs);
    let at = |root: &Path| PairView {
        local_root: Some(root.to_owned()),
        ..before.clone()
    };

    for same in [
        docs.clone(),
        PathBuf::from(format!("{}/", docs.display())),
        dir.path().join("./docs"),
        dir.path().join("other/../docs"),
        link.clone(),
    ] {
        assert_eq!(
            local_folder_moved(&before, &at(&same)),
            LocalFolder::Unchanged,
            "{}",
            same.display()
        );
    }
    // The folder of a drive that is not plugged in does not become a new folder by being typed again.
    let away = dir.path().join("usb/docs");
    assert_eq!(
        local_folder_moved(&at(&away), &at(&dir.path().join("usb/./docs"))),
        LocalFolder::Unchanged
    );
    for other in [
        dir.path().join("docs2"),
        dir.path().join("Docs"),
        dir.path().join("docs/sub"),
        dir.path().to_owned(),
    ] {
        assert_eq!(
            local_folder_moved(&before, &at(&other)),
            LocalFolder::Moved,
            "{}",
            other.display()
        );
    }
    let none = PairView {
        local_root: None,
        ..before.clone()
    };
    assert_eq!(local_folder_moved(&none, &none), LocalFolder::Unchanged);
    assert_eq!(local_folder_moved(&none, &before), LocalFolder::Moved);
    assert_eq!(local_folder_moved(&before, &none), LocalFolder::Moved);
}
