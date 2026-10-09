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
    assert!(nothing.items.is_empty(), "{nothing:?}");
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
    assert!(!exists_without_following(&root.join(".sync")));
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
