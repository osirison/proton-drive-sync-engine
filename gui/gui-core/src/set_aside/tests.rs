//! Setting a removed pair's history aside (D8). Every test works in a temporary directory and nothing
//! outside it: there is no real sync folder, daemon or state directory in reach.

use super::*;
use std::cell::Cell;
use std::time::Duration;

/// 2026-10-09 14:25:30 UTC.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_791_555_930)
}

fn write(path: &Path, bytes: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// A pair whose folder is `<base>/<name>` with the default layout: its `.sync` directory holds the
/// index, a WAL, both sidecars and the lockfile, and the folder holds two files of the person's.
fn default_layout(base: &Path, name: &str) -> PairView {
    let root = base.join(name);
    write(&root.join("docs/report.txt"), "the person's file");
    write(&root.join("photo.jpg"), "another one");
    let state = root.join(".sync");
    write(&state.join("sync_index.db"), "index");
    write(&state.join("sync_index.db-wal"), "wal");
    write(&state.join("sync_index.status.json"), "[]");
    write(&state.join("sync_index.metrics.json"), "{}");
    write(&state.join("proton-sync.lock"), "");
    view(
        name,
        &root,
        state.join("sync_index.db"),
        state.join("proton-sync.lock"),
    )
}

fn view(name: &str, root: &Path, db: PathBuf, lock: PathBuf) -> PairView {
    PairView {
        name: name.to_owned(),
        local_root: Some(root.to_owned()),
        remote_root: Some(PathBuf::from(format!("/Drive/{name}"))),
        db_path: Some(db),
        lockfile_path: Some(lock),
        conflict_suffix: None,
    }
}

fn context<'a>(state_dir: &'a Path, configured: &'a [PairView]) -> Context<'a> {
    Context {
        state_dir,
        configured,
        now: now(),
    }
}

/// The plan for a pair that has state to move; anything else is a failed test.
fn plan(view: &PairView) -> Plan {
    match super::plan(view) {
        Planned::Move(plan) => plan,
        other => panic!("expected something to move: {other:?}"),
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Whether this user is stopped by file permissions at all. Root reads everything, so a permission
/// test run as root proves nothing: it says so and stops instead of passing.
fn permissions_are_enforced(base: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let probe = base.join("permission-probe");
    fs::create_dir_all(&probe).unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    let enforced = fs::read_dir(&probe).is_err();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_dir(&probe).unwrap();
    if !enforced {
        eprintln!("SKIPPED: this user is not stopped by file permissions (root?)");
    }
    enforced
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn the_stamp_is_the_utc_time_it_names() {
    assert_eq!(utc_stamp(UNIX_EPOCH), "19700101T000000Z");
    assert_eq!(
        utc_stamp(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        "20231114T221320Z"
    );
    assert_eq!(utc_stamp(now()), "20261009T142530Z");
    // A leap day, and the last second of a year.
    assert_eq!(
        utc_stamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)),
        "20240229T000000Z"
    );
    assert_eq!(
        utc_stamp(UNIX_EPOCH + Duration::from_secs(1_735_689_599)),
        "20241231T235959Z"
    );
}

#[test]
fn the_plan_is_the_state_directory_and_never_a_file_of_the_persons() {
    let base = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let plan = plan(&view);
    assert_eq!(plan.items, [base.path().join("docs/.sync")]);
    assert_eq!(plan.lockfile, view.lockfile_path);
    // A folder with no state has nothing to set aside.
    let empty = base.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let nothing = super::plan(&PairView {
        db_path: Some(empty.join(".sync/sync_index.db")),
        lockfile_path: Some(empty.join(".sync/proton-sync.lock")),
        ..view_for(&empty)
    });
    assert_eq!(nothing, Planned::Nothing { notes: Vec::new() });
}

fn view_for(root: &Path) -> PairView {
    view(
        "x",
        root,
        root.join(".sync/sync_index.db"),
        root.join(".sync/proton-sync.lock"),
    )
}

#[test]
fn state_files_placed_outside_the_state_directory_are_named_one_by_one() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join("a.txt"), "mine");
    let elsewhere = base.path().join("state-elsewhere");
    for name in [
        "idx.db",
        "idx.db-wal",
        "idx.db-shm",
        "idx.status.json",
        "idx.metrics.json",
        "pair.lock",
        "unrelated.txt",
    ] {
        write(&elsewhere.join(name), name);
    }
    let view = view(
        "docs",
        &root,
        elsewhere.join("idx.db"),
        elsewhere.join("pair.lock"),
    );
    let plan = plan(&view);
    let names: Vec<String> = plan
        .items
        .iter()
        .map(|item| item.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "idx.db",
            "idx.db-wal",
            "idx.db-shm",
            "idx.status.json",
            "idx.metrics.json",
            "pair.lock"
        ],
        "exactly the files the pair names, and not `unrelated.txt` beside them"
    );
    // No state directory in the folder: it is not invented.
    assert!(!plan.items.contains(&root.join(".sync")));
}

#[test]
fn a_state_directory_that_is_a_link_is_set_aside_as_the_link_and_never_followed() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    let real_state = base.path().join("real-state");
    write(&real_state.join("sync_index.db"), "index");
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&real_state, root.join(".sync")).unwrap();
    let view = view_for(&root);
    let plan = plan(&view);
    assert_eq!(plan.items, [root.join(".sync")]);
    let state = tempfile::tempdir().unwrap();
    let done = execute(&plan, &context(state.path(), &[])).expect("the link moves");
    assert!(!exists(&root.join(".sync")));
    assert!(
        real_state.join("sync_index.db").exists(),
        "the link's target was not touched"
    );
    assert!(
        fs::symlink_metadata(&done.moved[0].to)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_rename_moves_the_state_directory_whole_and_leaves_every_file_of_the_persons() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let done = execute(&plan(&view), &context(state_dir.path(), &[])).expect("moves");

    assert!(
        !base.path().join("docs/.sync").exists(),
        "the history left the folder"
    );
    assert_eq!(
        fs::read_to_string(base.path().join("docs/docs/report.txt")).unwrap(),
        "the person's file"
    );
    assert_eq!(
        fs::read_to_string(base.path().join("docs/photo.jpg")).unwrap(),
        "another one"
    );

    assert_eq!(done.moved.len(), 1);
    assert_eq!(done.moved[0].how, How::Rename);
    let stored = &done.moved[0].to;
    assert_eq!(
        fs::read_to_string(stored.join("sync_index.db")).unwrap(),
        "index"
    );
    assert_eq!(
        fs::read_to_string(stored.join("sync_index.db-wal")).unwrap(),
        "wal"
    );
    // Under the removed-pairs directory of the state directory, named for the pair and the time.
    assert!(
        done.to
            .starts_with(state_dir.path().join(REMOVED_PAIRS_DIR)),
        "{:?}",
        done.to
    );
    assert_eq!(done.to.file_name().unwrap(), "docs-20261009T142530Z");

    // The manifest says where each piece came from.
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(done.to.join(MANIFEST_NAME)).unwrap()).unwrap();
    assert_eq!(manifest["pair"], "docs");
    assert_eq!(
        manifest["items"][0]["from"],
        base.path().join("docs/.sync").to_str().unwrap()
    );

    // Owner-only: an index lists every file name the person synced.
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(&done.to).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn a_state_file_placed_elsewhere_moves_on_its_own_and_its_neighbours_stay() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join("a.txt"), "mine");
    let elsewhere = base.path().join("elsewhere");
    write(&elsewhere.join("idx.db"), "index");
    write(&elsewhere.join("keep-me.txt"), "not this pair's");
    let view = view(
        "docs",
        &root,
        elsewhere.join("idx.db"),
        elsewhere.join("pair.lock"),
    );
    let done = execute(&plan(&view), &context(state_dir.path(), &[])).expect("moves");
    assert_eq!(done.moved.len(), 1);
    assert!(!elsewhere.join("idx.db").exists());
    assert!(
        elsewhere.join("keep-me.txt").exists(),
        "a neighbour in the same directory was not moved"
    );
    assert!(root.join("a.txt").exists());
}

#[test]
fn across_a_filesystem_boundary_it_copies_compares_and_only_then_removes() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let original = base.path().join("docs/.sync");
    // A link inside the state directory, to prove a copy keeps links as links.
    std::os::unix::fs::symlink("sync_index.db", original.join("alias")).unwrap();

    let exdev = |_: &Path, _: &Path| Err(io::Error::from_raw_os_error(18));
    let seen_while_comparing = Cell::new(false);
    let compare = |from: &Path, to: &Path| {
        // THE ORDER IS THE SAFETY: when the copy is compared, the original is still all there.
        seen_while_comparing
            .set(from.join("sync_index.db").exists() && to.join("sync_index.db").exists());
        same_tree(from, to)
    };
    let done = execute_with(
        &plan(&view),
        &context(state_dir.path(), &[]),
        &Mover {
            rename: &exdev,
            same: &compare,
            sync: &sync_path,
        },
    )
    .expect("copies");
    assert!(
        seen_while_comparing.get(),
        "the original must exist while its copy is being compared"
    );
    assert_eq!(done.moved[0].how, How::Copy);
    assert!(
        !original.exists(),
        "the original is removed once the copy is verified"
    );
    let stored = &done.moved[0].to;
    assert_eq!(
        fs::read_to_string(stored.join("sync_index.db")).unwrap(),
        "index"
    );
    assert_eq!(
        fs::read_link(stored.join("alias")).unwrap(),
        Path::new("sync_index.db")
    );
}

#[test]
fn a_copy_that_does_not_match_leaves_the_original_exactly_as_it_was() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let original = base.path().join("docs/.sync");
    let exdev = |_: &Path, _: &Path| Err(io::Error::from_raw_os_error(18));
    for (name, outcome) in [
        ("a mismatch", Ok(false)),
        (
            "a comparison that errors",
            Err(io::ErrorKind::PermissionDenied),
        ),
    ] {
        let compare = |_: &Path, _: &Path| outcome.map_err(io::Error::from);
        let failed = execute_with(
            &plan(&view),
            &context(state_dir.path(), &[]),
            &Mover {
                rename: &exdev,
                same: &compare,
                sync: &sync_path,
            },
        )
        .expect_err(name);
        assert!(failed.moved.is_empty(), "{name}");
        assert_eq!(failed.remaining, std::slice::from_ref(&original), "{name}");
        assert_eq!(
            fs::read_to_string(original.join("sync_index.db")).unwrap(),
            "index",
            "{name}"
        );
        assert!(
            original.join("proton-sync.lock").exists(),
            "{name}: the original lost a file"
        );
        // The partial copy is gone, and so is the directory made for it.
        let leftovers: Vec<_> = fs::read_dir(state_dir.path().join(REMOVED_PAIRS_DIR))
            .unwrap()
            .collect();
        assert!(leftovers.is_empty(), "{name}: {leftovers:?}");
    }
}

#[test]
fn the_comparison_is_by_bytes_not_by_name_or_size() {
    let base = tempfile::tempdir().unwrap();
    let (a, b) = (base.path().join("a"), base.path().join("b"));
    write(&a.join("f"), "abcd");
    write(&b.join("f"), "abce");
    assert!(
        !same_tree(&a, &b).unwrap(),
        "same name and size, different byte"
    );
    write(&b.join("f"), "abcd");
    assert!(same_tree(&a, &b).unwrap());
    write(&b.join("extra"), "");
    assert!(!same_tree(&a, &b).unwrap(), "an extra entry");
    // Bigger than one comparison block.
    let big = "x".repeat(200_000);
    write(&a.join("big"), &big);
    fs::remove_file(b.join("extra")).unwrap();
    write(&b.join("big"), &format!("{}y", &big[..199_999]));
    assert!(
        !same_tree(&a, &b).unwrap(),
        "a difference in the last byte of a large file"
    );
}

#[test]
fn a_held_lockfile_blocks_the_move_and_nothing_is_touched() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    // What a running daemon does: holds the pair's own lockfile exclusively.
    let daemon = File::open(base.path().join("docs/.sync/proton-sync.lock")).unwrap();
    rustix::fs::flock(
        &daemon,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .unwrap();

    let failed = execute(&plan(&view), &context(state_dir.path(), &[])).expect_err("held");
    assert!(failed.reason.contains("still holds"), "{}", failed.reason);
    assert!(failed.moved.is_empty());
    assert!(base.path().join("docs/.sync/sync_index.db").exists());
    assert!(
        !state_dir.path().join(REMOVED_PAIRS_DIR).exists(),
        "no destination was made"
    );

    // The daemon lets go (it was restarted off the pair), and the same call now succeeds.
    drop(daemon);
    execute(&plan(&view), &context(state_dir.path(), &[])).expect("free now");
}

#[test]
fn a_lockfile_nobody_can_be_asked_about_is_not_read_as_free() {
    let base = tempfile::tempdir().unwrap();
    // A path whose parent is a file: opening it fails with something other than "not found".
    write(&base.path().join("plain"), "x");
    assert!(matches!(
        acquire(Some(&base.path().join("plain/lock"))),
        LockProbe::Unknown(_)
    ));
    assert!(matches!(
        acquire(Some(&base.path().join("missing.lock"))),
        LockProbe::Free(_)
    ));
    assert!(matches!(acquire(None), LockProbe::Free(_)));
}

#[test]
fn history_is_never_set_aside_inside_a_sync_folder() {
    let base = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let other = default_layout(base.path(), "photos");
    // The app's state directory sits INSIDE another configured pair's folder.
    let inside_other = base.path().join("photos/app-state");
    let failed = execute(
        &plan(&view),
        &context(&inside_other, std::slice::from_ref(&other)),
    )
    .expect_err("refused");
    assert!(
        failed.reason.contains("uploaded as ordinary files"),
        "{}",
        failed.reason
    );
    assert!(base.path().join("docs/.sync/sync_index.db").exists());
    // Through a link, too: the real path decides.
    let alias = base.path().join("alias-to-photos");
    std::os::unix::fs::symlink(base.path().join("photos"), &alias).unwrap();
    let failed = execute(
        &plan(&view),
        &context(&alias.join("state"), std::slice::from_ref(&other)),
    )
    .expect_err("refused through a symlink");
    assert!(
        failed.reason.contains("uploaded as ordinary files"),
        "{}",
        failed.reason
    );
    // And inside the removed pair's own folder: the next add of it would upload that history.
    let failed =
        execute(&plan(&view), &context(&base.path().join("docs/state"), &[])).expect_err("refused");
    assert!(
        failed.reason.contains("uploaded as ordinary files"),
        "{}",
        failed.reason
    );
}

#[test]
fn state_that_a_configured_pair_owns_is_not_history() {
    // The folder was added back before the set-aside ran: its `.sync` is that pair's index now.
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let removed = default_layout(base.path(), "docs");
    let readded = removed.clone();
    let failed = execute(
        &plan(&removed),
        &context(state_dir.path(), std::slice::from_ref(&readded)),
    )
    .expect_err("refused");
    assert!(failed.reason.contains("is configured"), "{}", failed.reason);
    assert!(base.path().join("docs/.sync/sync_index.db").exists());
}

#[test]
fn a_failure_part_way_says_what_moved_and_what_is_left() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join("a.txt"), "mine");
    let elsewhere = base.path().join("elsewhere");
    write(&elsewhere.join("idx.db"), "index");
    write(&elsewhere.join("idx.db-wal"), "wal");
    let view = view(
        "docs",
        &root,
        elsewhere.join("idx.db"),
        elsewhere.join("pair.lock"),
    );
    let calls = Cell::new(0);
    let second_fails = |from: &Path, to: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            fs::rename(from, to)
        }
    };
    let failed = execute_with(
        &plan(&view),
        &context(state_dir.path(), &[]),
        &Mover {
            rename: &second_fails,
            same: &|a, b| same_tree(a, b),
            sync: &sync_path,
        },
    )
    .expect_err("the second fails");
    assert_eq!(failed.moved.len(), 1);
    assert_eq!(failed.remaining, [elsewhere.join("idx.db-wal")]);
    assert!(
        elsewhere.join("idx.db-wal").exists(),
        "the one that failed is where it was"
    );
    assert!(failed.moved[0].to.exists());
}

#[test]
fn two_removals_in_one_second_get_two_directories() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let first = default_layout(base.path(), "docs");
    let one = execute(&plan(&first), &context(state_dir.path(), &[])).unwrap();
    // The folder is added back, grows new history, and is removed again within the second.
    let again = default_layout(base.path(), "docs");
    let two = execute(&plan(&again), &context(state_dir.path(), &[])).unwrap();
    assert_ne!(one.to, two.to);
    assert!(
        two.to
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-2")
    );
    assert!(
        one.moved[0].to.join("sync_index.db").exists(),
        "the first set-aside was not overwritten"
    );
}

#[test]
fn a_surviving_index_is_a_file_at_the_pairs_db_path() {
    let base = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    assert_eq!(surviving_index(&view), view.db_path);
    fs::remove_file(view.db_path.as_ref().unwrap()).unwrap();
    assert_eq!(surviving_index(&view), None);
    // A directory at that path is not an index.
    fs::create_dir_all(view.db_path.as_ref().unwrap()).unwrap();
    assert_eq!(surviving_index(&view), None);
}

// ---- pending ---------------------------------------------------------------------------------------

#[test]
fn a_pending_set_aside_is_recorded_and_settles_once_nothing_holds_it() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let plan = plan(&view);
    let record =
        record_pending(state_dir.path(), &plan, "the daemon had not stopped", now()).unwrap();
    assert!(record.starts_with(state_dir.path().join(PENDING_DIR)));
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&record).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let listed = pending(state_dir.path());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].1.plan, plan);
    assert_eq!(listed[0].1.reason, "the daemon had not stopped");

    // Still held: stays pending, with the reason, and nothing moves.
    let daemon = File::open(base.path().join("docs/.sync/proton-sync.lock")).unwrap();
    rustix::fs::flock(
        &daemon,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .unwrap();
    let results = settle_pending(&context(state_dir.path(), &[]));
    assert!(
        matches!(&results[..], [Settled::StillPending { pair, .. }] if pair == "docs"),
        "{results:?}"
    );
    assert_eq!(pending(state_dir.path()).len(), 1, "the record stays");
    assert!(base.path().join("docs/.sync/sync_index.db").exists());

    // Released: it settles, and the record goes.
    drop(daemon);
    let results = settle_pending(&context(state_dir.path(), &[]));
    assert!(
        matches!(&results[..], [Settled::Moved { .. }]),
        "{results:?}"
    );
    assert!(pending(state_dir.path()).is_empty());
    assert!(!base.path().join("docs/.sync").exists());
}

#[test]
fn a_pending_set_aside_for_a_folder_that_was_added_back_is_dropped_without_moving_anything() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    record_pending(state_dir.path(), &plan(&view), "held", now()).unwrap();
    let readded = view.clone();
    let results = settle_pending(&context(state_dir.path(), std::slice::from_ref(&readded)));
    assert!(
        matches!(&results[..], [Settled::Superseded { pair }] if pair == "docs"),
        "{results:?}"
    );
    assert!(
        pending(state_dir.path()).is_empty(),
        "the record is dropped"
    );
    assert!(
        base.path().join("docs/.sync/sync_index.db").exists(),
        "and the re-added pair keeps the history it is running on"
    );
}

#[test]
fn a_folder_added_back_by_another_spelling_is_still_added_back() {
    // The re-add check is by real path: the folder named through a link, or with a `..` in it, is the
    // same folder. Compared as written, the record would not be dropped and the move would be left
    // to a second refusal further down, which reports the folder as merely "still pending".
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let alias = base.path().join("alias-to-docs");
    std::os::unix::fs::symlink(base.path().join("docs"), &alias).unwrap();
    let dotted = base.path().join("elsewhere/../docs");
    fs::create_dir_all(base.path().join("elsewhere")).unwrap();
    for (label, root) in [("a link", alias), ("a `..`", dotted)] {
        record_pending(state_dir.path(), &plan(&view), "held", now()).unwrap();
        let readded = PairView {
            local_root: Some(root.clone()),
            db_path: Some(root.join(".sync/sync_index.db")),
            lockfile_path: Some(root.join(".sync/proton-sync.lock")),
            ..view.clone()
        };
        let results = settle_pending(&context(state_dir.path(), std::slice::from_ref(&readded)));
        assert!(
            matches!(&results[..], [Settled::Superseded { pair }] if pair == "docs"),
            "{label}: {results:?}"
        );
        assert!(
            pending(state_dir.path()).is_empty(),
            "{label}: the record is dropped"
        );
        assert!(
            base.path().join("docs/.sync/sync_index.db").exists(),
            "{label}: the history stays"
        );
    }
}

#[test]
fn a_record_with_no_folder_is_not_taken_for_a_folder_that_was_added_back() {
    // A note with no folder in it (hand-made, or from a config that placed none) compares equal to a
    // configured pair whose folder is empty too. That is no folder at all, not a re-add.
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let nameless = PairView {
        name: "nameless".to_owned(),
        local_root: None,
        remote_root: None,
        db_path: None,
        lockfile_path: None,
        conflict_suffix: None,
    };
    record_pending(
        state_dir.path(),
        &Plan::of_view(&nameless, Vec::new(), Vec::new()),
        "no folder",
        now(),
    )
    .unwrap();
    let configured = PairView {
        local_root: Some(PathBuf::new()),
        ..view_for(base.path())
    };
    let results = settle_pending(&context(
        state_dir.path(),
        std::slice::from_ref(&configured),
    ));
    assert!(
        matches!(&results[..], [Settled::StillPending { .. }]),
        "{results:?}"
    );
    assert_eq!(
        pending(state_dir.path()).len(),
        1,
        "the note is not dropped"
    );
}

#[test]
fn a_relative_index_path_is_not_looked_up_from_here() {
    // `Cargo.toml` exists in the directory the tests run from. A view that names it relatively must
    // not be reported as an index that is there.
    let view = PairView {
        db_path: Some(PathBuf::from("Cargo.toml")),
        ..view_for(Path::new("/nowhere"))
    };
    assert!(Path::new("Cargo.toml").exists(), "the premise of the test");
    assert_eq!(surviving_index(&view), None);
}

// ---- looking is not the same as finding nothing (U1) ----------------------------------------------

#[test]
fn a_folder_that_cannot_be_read_is_not_reported_as_having_no_history() {
    let base = tempfile::tempdir().unwrap();
    if !permissions_are_enforced(base.path()) {
        return;
    }
    let view = default_layout(base.path(), "docs");
    let root = base.path().join("docs");
    // No permission at all: nothing under it can be looked at.
    set_mode(&root, 0o000);
    let planned = super::plan(&view);
    set_mode(&root, 0o755);
    let Planned::Undetermined(undetermined) = planned else {
        panic!("the history is right there, and the app was told there is none: {planned:?}")
    };
    assert!(undetermined.retry, "a permission can be fixed");
    assert!(
        undetermined.reason.contains("cannot be read"),
        "{}",
        undetermined.reason
    );
    assert!(root.join(".sync/sync_index.db").exists());

    // Searchable but not listable (`--x`): a path under it can be stat'ed, but the folder cannot be
    // read, and "no `.sync`" under a folder that cannot be read is not an answer.
    let bare = base.path().join("bare");
    fs::create_dir_all(&bare).unwrap();
    set_mode(&bare, 0o100);
    let planned = super::plan(&view_for(&bare));
    set_mode(&bare, 0o755);
    assert!(
        matches!(planned, Planned::Undetermined(_)),
        "an unlistable folder with no `.sync` is not a folder without history: {planned:?}"
    );

    // Listable but not searchable (`r--`): the folder reads, the stat of `.sync` fails with something
    // that is not "not found" — also not an absence.
    let listable = base.path().join("listable");
    write(&listable.join(".sync/sync_index.db"), "index");
    set_mode(&listable, 0o400);
    let planned = super::plan(&view_for(&listable));
    set_mode(&listable, 0o755);
    let Planned::Undetermined(undetermined) = planned else {
        panic!("{planned:?}")
    };
    assert!(
        undetermined.reason.contains("could not look at"),
        "{}",
        undetermined.reason
    );
}

#[test]
fn a_missing_folder_is_not_reported_as_having_no_history() {
    // An unplugged or unmounted drive looks exactly like this.
    let base = tempfile::tempdir().unwrap();
    let gone = base.path().join("drive-that-is-not-plugged-in/docs");
    let Planned::Undetermined(undetermined) = super::plan(&view_for(&gone)) else {
        panic!("a folder that is not there cannot be said to hold no history")
    };
    assert!(undetermined.retry, "the drive may come back");
    assert!(
        undetermined.reason.contains(gone.to_str().unwrap()),
        "{}",
        undetermined.reason
    );
    // A file where the folder should be is no better.
    let file = base.path().join("a-file");
    write(&file, "x");
    assert!(matches!(
        super::plan(&view_for(&file)),
        Planned::Undetermined(_)
    ));
}

#[test]
fn a_state_file_elsewhere_that_cannot_be_looked_at_is_not_absent() {
    let base = tempfile::tempdir().unwrap();
    if !permissions_are_enforced(base.path()) {
        return;
    }
    let root = base.path().join("docs");
    write(&root.join("a.txt"), "mine");
    let elsewhere = base.path().join("elsewhere");
    write(&elsewhere.join("idx.db"), "index");
    let view = view(
        "docs",
        &root,
        elsewhere.join("idx.db"),
        elsewhere.join("pair.lock"),
    );
    set_mode(&elsewhere, 0o000);
    let planned = super::plan(&view);
    set_mode(&elsewhere, 0o755);
    let Planned::Undetermined(undetermined) = planned else {
        panic!("{planned:?}")
    };
    assert!(
        undetermined.reason.contains("idx.db"),
        "{}",
        undetermined.reason
    );
}

#[test]
fn a_state_file_whose_folder_is_missing_is_not_absent() {
    // An index on a drive that is not plugged in: the folder holds no history the app can see, and the
    // state file has no parent to list.
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join("a.txt"), "mine");
    let gone = base.path().join("usb-not-plugged-in");
    let view = view("docs", &root, gone.join("idx.db"), gone.join("pair.lock"));
    let Planned::Undetermined(undetermined) = super::plan(&view) else {
        panic!("a state file on a missing drive was reported as nothing to move")
    };
    assert!(undetermined.retry, "the drive may come back");
    assert!(
        undetermined.reason.contains(gone.to_str().unwrap()),
        "{}",
        undetermined.reason
    );
    assert!(
        !undetermined.reason.contains("  "),
        "the reason text should not contain double spaces: {}",
        undetermined.reason
    );
    assert!(
        undetermined.reason.contains("cannot tell whether"),
        "the reason text should contain 'cannot tell whether': {}",
        undetermined.reason
    );
    // With the parent present and listable, the same absence is an absence.
    fs::create_dir_all(&gone).unwrap();
    assert!(matches!(super::plan(&view), Planned::Nothing { .. }));
}

#[test]
fn a_move_that_could_not_look_is_recorded_and_happens_when_the_drive_is_back() {
    // The whole flow of an unplugged drive: removal finds nothing it can look at, a record of the pair
    // is kept (no list of items: there was none to make), and when the folder is readable again the
    // record plans from the disk as it is and the history moves.
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("usb/docs");
    let view = view_for(&root);
    let Planned::Undetermined(undetermined) = super::plan(&view) else {
        panic!("not mounted")
    };
    assert!(undetermined.retry);
    let identity = Plan::of_view(&view, Vec::new(), Vec::new());
    record_pending(state_dir.path(), &identity, &undetermined.reason, now()).unwrap();

    let results = settle_pending(&context(state_dir.path(), &[]));
    assert!(
        matches!(&results[..], [Settled::StillPending { reason, .. }] if reason.contains("cannot be read")),
        "{results:?}"
    );
    assert_eq!(pending(state_dir.path()).len(), 1);

    // The drive comes back, with the history on it.
    write(&root.join(".sync/sync_index.db"), "index");
    let results = settle_pending(&context(state_dir.path(), &[]));
    assert!(
        matches!(&results[..], [Settled::Moved { .. }]),
        "{results:?}"
    );
    assert!(!root.join(".sync").exists());
    assert!(pending(state_dir.path()).is_empty());

    // And a record whose folder reads fine and holds nothing any more is dropped, not kept for ever.
    record_pending(state_dir.path(), &identity, "again", now()).unwrap();
    let results = settle_pending(&context(state_dir.path(), &[]));
    assert!(
        matches!(&results[..], [Settled::NothingLeft { pair, had_items: false, .. }] if pair == "x"),
        "{results:?}"
    );
    assert!(pending(state_dir.path()).is_empty());
}

// ---- a relative path is never planned (U2) --------------------------------------------------------

#[test]
fn a_relative_path_is_never_planned_whatever_is_at_it_from_here() {
    let base = tempfile::tempdir().unwrap();
    let absolute = default_layout(base.path(), "docs");
    for (key, view) in [
        (
            "local_root",
            PairView {
                local_root: Some(PathBuf::from("Sync")),
                ..view_for(Path::new("Sync"))
            },
        ),
        (
            "db_path",
            PairView {
                db_path: Some(PathBuf::from("sync_index.db")),
                ..absolute.clone()
            },
        ),
        (
            "lockfile_path",
            PairView {
                lockfile_path: Some(PathBuf::from("proton-sync.lock")),
                ..absolute.clone()
            },
        ),
    ] {
        let Planned::Undetermined(undetermined) = super::plan(&view) else {
            panic!("{key}: a relative path was planned")
        };
        assert!(
            !undetermined.retry,
            "{key}: waiting does not make it absolute"
        );
        assert!(
            undetermined.reason.contains(key) && undetermined.reason.contains("relative path"),
            "{key}: {}",
            undetermined.reason
        );
    }
    // A pair whose folder the config does not place at all is not guessed at either.
    let no_root = PairView {
        local_root: None,
        ..absolute
    };
    assert!(matches!(
        super::plan(&no_root),
        Planned::Undetermined(Undetermined { retry: false, .. })
    ));
    assert!(base.path().join("docs/.sync/sync_index.db").exists());
}

// ---- a record is not the plan (U4) ---------------------------------------------------------------

#[test]
fn a_record_that_names_anything_but_the_pairs_state_is_refused_and_moves_nothing() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let thesis = base.path().join("Documents/thesis.txt");
    write(&thesis, "the thesis");
    let photo = base.path().join("docs/photo.jpg");
    for (label, stranger) in [
        ("a file elsewhere", thesis.clone()),
        ("a file in the pair's own folder", photo.clone()),
        (
            "a path that only looks like its state",
            base.path().join("docs/.sync/../photo.jpg"),
        ),
    ] {
        // The pair's identity is real; the item list is where a planted record lies.
        let mut planted = plan(&view);
        planted.items.push(stranger);
        record_pending(state_dir.path(), &planted, "planted", now()).unwrap();

        let results = settle_pending(&context(state_dir.path(), &[]));
        let [Settled::StillPending { reason, .. }] = &results[..] else {
            panic!("{label}: {results:?}")
        };
        assert!(
            reason.contains("not part of the sync state"),
            "{label}: {reason}"
        );
        assert_eq!(
            fs::read_to_string(&thesis).unwrap(),
            "the thesis",
            "{label}"
        );
        assert!(photo.exists(), "{label}");
        assert!(
            base.path().join("docs/.sync/sync_index.db").exists(),
            "{label}: a refused record moves nothing at all, not even the real state"
        );
        assert_eq!(pending(state_dir.path()).len(), 1, "{label}: it stays");
        for (record, _) in pending(state_dir.path()) {
            fs::remove_file(record).unwrap();
        }
    }
}

#[test]
fn a_record_is_re_planned_from_the_disk_as_it_is_now() {
    // Recorded while one file existed; by the time it is retried, there is more (and the `.sync`
    // that was named has gone). What moves is what the pair's identity finds now.
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join("a.txt"), "mine");
    let elsewhere = base.path().join("elsewhere");
    let view = view(
        "docs",
        &root,
        elsewhere.join("idx.db"),
        elsewhere.join("pair.lock"),
    );
    write(&elsewhere.join("idx.db"), "index");
    let recorded = plan(&view);
    assert_eq!(recorded.items, [elsewhere.join("idx.db")]);
    record_pending(state_dir.path(), &recorded, "held", now()).unwrap();
    // It grows: a WAL appears beside the index.
    write(&elsewhere.join("idx.db-wal"), "wal");
    let results = settle_pending(&context(state_dir.path(), &[]));
    let [Settled::Moved { done, .. }] = &results[..] else {
        panic!("{results:?}")
    };
    assert_eq!(done.moved.len(), 2, "the new file went too");
    assert!(!elsewhere.join("idx.db-wal").exists());
}

// ---- a path that is not UTF-8 (U5) --------------------------------------------------------------

#[test]
fn a_folder_whose_name_is_not_utf8_is_recorded_moved_and_written_to_the_manifest_exactly() {
    use std::os::unix::ffi::OsStringExt;
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base
        .path()
        .join(OsString::from_vec(b"Sync-\xff\xfe".to_vec()));
    write(&root.join(".sync/sync_index.db"), "index");
    let view = view_for(&root);
    let planned = plan(&view);

    // The record is written, and reads back as the very same paths.
    let record = record_pending(state_dir.path(), &planned, "held", now())
        .expect("a non-UTF-8 folder can be recorded");
    let listed = pending(state_dir.path());
    assert_eq!(listed.len(), 1, "the record reads back");
    assert_eq!(listed[0].1.plan, planned);
    assert_eq!(listed[0].1.plan.local_root, root);
    assert_eq!(listed[0].1.plan.items, [root.join(".sync")]);
    let raw = fs::read_to_string(&record).unwrap();
    assert!(raw.contains("\"hex\""), "{raw}");
    assert!(
        raw.contains("Sync-\u{fffd}"),
        "a readable rendering sits beside the exact one: {raw}"
    );

    // Acting on the record moves the folder that is on disk, and the manifest says where from.
    let results = settle_pending(&context(state_dir.path(), &[]));
    let [Settled::Moved { done, .. }] = &results[..] else {
        panic!("{results:?}")
    };
    assert!(!root.join(".sync").exists());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(done.to.join(MANIFEST_NAME)).expect("manifest written"))
            .unwrap();
    let items: Vec<MovedItem> = serde_json::from_value(manifest["items"].clone()).unwrap();
    assert_eq!(items, done.moved, "the manifest names exactly what moved");
    assert_eq!(items[0].from, root.join(".sync"));
}

#[test]
fn a_utf8_path_stays_a_plain_string_in_the_manifest_and_the_record() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let record = record_pending(state_dir.path(), &plan(&view), "held", now()).unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(record).unwrap()).unwrap();
    assert_eq!(
        raw["plan"]["local_root"],
        base.path().join("docs").to_str().unwrap()
    );
    assert_eq!(
        raw["plan"]["items"][0],
        base.path().join("docs/.sync").to_str().unwrap()
    );
}

// ---- reporting what actually happened (U6) -----------------------------------------------------------

#[test]
fn a_state_directory_that_is_a_link_says_the_history_stayed_where_the_link_points() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    let real_state = base.path().join("fast-disk/state-of-docs");
    write(&real_state.join("sync_index.db"), "HISTORY");
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&real_state, root.join(".sync")).unwrap();
    let done = execute(&plan(&view_for(&root)), &context(state_dir.path(), &[])).unwrap();
    assert_eq!(
        done.moved[0].link_target.as_deref(),
        Some(real_state.as_path())
    );
    assert_eq!(
        fs::read_to_string(real_state.join("sync_index.db")).unwrap(),
        "HISTORY",
        "what the link pointed at was not touched"
    );
    // The manifest says so too.
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(done.to.join(MANIFEST_NAME)).unwrap()).unwrap();
    assert_eq!(
        manifest["items"][0]["link_target"],
        real_state.to_str().unwrap()
    );
    // A real directory has no link target.
    let other = default_layout(base.path(), "photos");
    let done = execute(&plan(&other), &context(state_dir.path(), &[])).unwrap();
    assert_eq!(done.moved[0].link_target, None);
}

#[test]
fn a_file_named_dot_sync_is_not_the_engines_state_and_is_left_alone_with_a_note() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join(".sync"), "MY NOTES");
    // Nothing else of the pair's: nothing to move, and the note says why.
    let Planned::Nothing { notes } = super::plan(&view_for(&root)) else {
        panic!("a file is not the engine's state directory")
    };
    assert!(
        notes
            .iter()
            .any(|note| note.contains(".sync") && note.contains("is a file")),
        "{notes:?}"
    );
    assert_eq!(fs::read_to_string(root.join(".sync")).unwrap(), "MY NOTES");

    // With state elsewhere that does exist, that moves — and the lock probe does not trip over the
    // file where its directory should be (the lockfile cannot exist there, so nobody holds it).
    let elsewhere = base.path().join("elsewhere");
    write(&elsewhere.join("idx.db"), "index");
    let view = PairView {
        db_path: Some(elsewhere.join("idx.db")),
        ..view_for(&root)
    };
    let planned = plan(&view);
    assert_eq!(planned.items, [elsewhere.join("idx.db")]);
    assert_eq!(planned.lockfile, None);
    let done = execute(&planned, &context(state_dir.path(), &[])).expect("moves what is state");
    assert_eq!(done.moved.len(), 1);
    assert_eq!(fs::read_to_string(root.join(".sync")).unwrap(), "MY NOTES");
    assert!(done.notes.iter().any(|note| note.contains("is a file")));
}

#[test]
fn a_state_file_that_names_a_folder_is_never_moved() {
    // The engine accepts a `db_path` that names an existing directory. It is the person's folder, not
    // an index, and it is not this module's to move.
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let documents = base.path().join("Documents");
    write(&documents.join("thesis.txt"), "the thesis");
    let root = base.path().join("docs");
    write(&root.join(".sync/sync_index.db"), "index");
    let view = PairView {
        db_path: Some(documents.clone()),
        ..view_for(&root)
    };
    let planned = plan(&view);
    assert_eq!(
        planned.items,
        [root.join(".sync")],
        "only the state directory"
    );
    assert!(
        planned
            .notes
            .iter()
            .any(|note| note.contains("is a folder")),
        "{:?}",
        planned.notes
    );
    execute(&planned, &context(state_dir.path(), &[])).expect("moves");
    assert_eq!(
        fs::read_to_string(documents.join("thesis.txt")).unwrap(),
        "the thesis"
    );

    // With nothing else to move, the folder is the whole finding.
    let bare = base.path().join("bare");
    fs::create_dir_all(&bare).unwrap();
    let Planned::Nothing { notes } = super::plan(&PairView {
        db_path: Some(documents.clone()),
        ..view_for(&bare)
    }) else {
        panic!("a folder is not state")
    };
    assert!(
        notes.iter().any(|note| note.contains("is a folder")),
        "{notes:?}"
    );
    assert!(documents.join("thesis.txt").exists());
}

#[test]
fn a_copy_is_made_durable_before_it_is_compared_and_the_original_removed() {
    use std::cell::RefCell;
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let original = base.path().join("docs/.sync");
    write(&original.join("nested/inner.bin"), "inner");

    let exdev = |_: &Path, _: &Path| Err(io::Error::from_raw_os_error(18));
    let synced = RefCell::new(Vec::<PathBuf>::new());
    let sync = |path: &Path| {
        synced.borrow_mut().push(path.to_owned());
        Ok(())
    };
    let synced_by_compare = RefCell::new(Vec::<PathBuf>::new());
    let compare = |from: &Path, to: &Path| {
        // THE ORDER IS THE SAFETY: what had been made durable by the time the copy is compared, with
        // the original still whole.
        assert!(from.join("sync_index.db").exists());
        *synced_by_compare.borrow_mut() = synced.borrow().clone();
        same_tree(from, to)
    };
    let done = execute_with(
        &plan(&view),
        &context(state_dir.path(), &[]),
        &Mover {
            rename: &exdev,
            same: &compare,
            sync: &sync,
        },
    )
    .expect("copies");
    assert!(!original.exists());

    let stored = &done.moved[0].to;
    let mut expected = vec![done.to.clone(), stored.clone(), stored.join("nested")];
    for file in [
        "sync_index.db",
        "sync_index.db-wal",
        "sync_index.status.json",
        "sync_index.metrics.json",
        "proton-sync.lock",
        "nested/inner.bin",
    ] {
        expected.push(stored.join(file));
    }
    let synced = synced_by_compare.into_inner();
    for path in &expected {
        assert!(
            synced.contains(path),
            "{} was not made durable before the original was removed; synced: {synced:?}",
            path.display()
        );
    }
}

#[test]
fn a_copy_that_cannot_be_made_durable_leaves_the_original_exactly_as_it_was() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let view = default_layout(base.path(), "docs");
    let original = base.path().join("docs/.sync");
    let exdev = |_: &Path, _: &Path| Err(io::Error::from_raw_os_error(18));
    let broken_disk = |_: &Path| Err(io::Error::other("input/output error"));
    let failed = execute_with(
        &plan(&view),
        &context(state_dir.path(), &[]),
        &Mover {
            rename: &exdev,
            same: &|a, b| same_tree(a, b),
            sync: &broken_disk,
        },
    )
    .expect_err("an fsync that fails is a copy that is not safe");
    assert!(failed.moved.is_empty());
    assert_eq!(
        fs::read_to_string(original.join("sync_index.db")).unwrap(),
        "index"
    );
    assert!(
        fs::read_dir(state_dir.path().join(REMOVED_PAIRS_DIR))
            .unwrap()
            .next()
            .is_none(),
        "no partial copy is left behind"
    );
}

#[test]
fn a_failure_part_way_names_the_directory_what_did_move_went_to() {
    let base = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let root = base.path().join("docs");
    write(&root.join(".sync/sync_index.db"), "index");
    let elsewhere = base.path().join("elsewhere");
    write(&elsewhere.join("idx.db"), "index");
    let view = PairView {
        db_path: Some(elsewhere.join("idx.db")),
        ..view_for(&root)
    };
    let calls = Cell::new(0);
    let second_fails = |from: &Path, to: &Path| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            fs::rename(from, to)
        }
    };
    let failed = execute_with(
        &plan(&view),
        &context(state_dir.path(), &[]),
        &Mover {
            rename: &second_fails,
            same: &|a, b| same_tree(a, b),
            sync: &sync_path,
        },
    )
    .expect_err("the second fails");
    assert_eq!(failed.moved.len(), 1);
    let to = failed.to.as_ref().expect("where the first one went");
    assert!(failed.moved[0].to.starts_with(to));
    assert!(to.starts_with(state_dir.path().join(REMOVED_PAIRS_DIR)));
    // Nothing moved, nothing to name.
    let nothing = execute(
        &plan(&default_layout(base.path(), "photos")),
        &context(&base.path().join("photos/app-state"), &[]),
    )
    .expect_err("refused");
    assert_eq!(nothing.to, None);
}
