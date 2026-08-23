//! `trash::dispose` against the **real** FreeDesktop trash, in its own process.
//!
//! WHY THIS FILE EXISTS SEPARATELY FROM THE UNIT TESTS. The `trash` crate resolves the home trash by
//! reading `XDG_DATA_HOME` on every call, so pointing it somewhere harmless means
//! `std::env::set_var` — `unsafe` under edition 2024, because `setenv` races *any* concurrent
//! `getenv` and not merely one on the same key. In the lib test binary that race is real: hundreds
//! of tests run across threads and several of them read `HOME` and the other XDG variables. Cargo
//! gives each integration test file its own process, so here the only tests in the binary are these,
//! and [`trash_home`] performs the single write through a `OnceLock` whose initialisation is ordered
//! before every trash-crate read that follows it.
//!
//! WHAT IT BUYS. The daemon tests dispose through a thread-local hook (`trash::test_hook`) so a
//! `cargo test` run never writes into the developer's real `~/.local/share/Trash`. That hook proves
//! the engine calls the seam correctly and proves nothing about the seam itself. This file is the
//! other half: it proves that the crate behind `LocalDeleteMode::Trash` really relocates the entity
//! and really writes the `.trashinfo` record a file manager restores from.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{fs, io};

use proton_drive_sync_engine::index::EntityKind;
use proton_drive_sync_engine::trash::{LocalDeleteMode, dispose};

/// The redirected `XDG_DATA_HOME` for this whole process, created and installed exactly once.
///
/// Kept for the process lifetime on purpose (leaked rather than held in a `TempDir`): the trash must
/// outlive every test that inspects it, and a `TempDir` dropped by the first finishing test would
/// delete the trash out from under the others.
///
/// SAME FILESYSTEM AS THE ENTITIES BEING TRASHED, which is not incidental. When the home trash and
/// the file are on different mounts the spec sends the file to a `.Trash-$uid` at *its own* mount
/// point instead, and the assertions below would be looking in the wrong place. Both live under one
/// temp root, so the home-trash path is the one taken.
fn trash_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("proton-sync-trash-test-{}", std::process::id()));
        let data_home = root.join("data-home");
        fs::create_dir_all(&data_home).expect("create the redirected XDG_DATA_HOME");
        // SAFETY: `OnceLock::get_or_init` runs this closure exactly once and blocks every other
        // caller until it returns, and every test in this binary calls `trash_home()` before it
        // performs any disposal. The write is therefore ordered before every `getenv` the `trash`
        // crate makes in this process, and nothing else in this binary reads the environment.
        unsafe { std::env::set_var("XDG_DATA_HOME", &data_home) };
        root
    })
    .as_path()
}

/// A fresh directory under the shared temp root for one test's entities.
fn workspace(name: &str) -> PathBuf {
    let dir = trash_home().join("work").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create the test workspace");
    dir
}

/// The `files` half of the redirected home trash — where the entity itself lands.
fn trashed_files_dir() -> PathBuf {
    trash_home().join("data-home/Trash/files")
}

/// The `info` half — where the `.trashinfo` record that makes a restore possible lands.
fn trashed_info_dir() -> PathBuf {
    trash_home().join("data-home/Trash/info")
}

/// Every entry in a directory, or an empty list when it does not exist yet.
fn entries(dir: &Path) -> Vec<String> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The one `.trashinfo` body naming `original`, or `None` when nothing in the trash points there.
///
/// Matched on the recorded path rather than on the file name because the spec lets the trash rename
/// a colliding entry, so `doomed.txt` may land as `doomed.txt.2` — and this whole file is about the
/// record being right rather than the name being unchanged.
fn trashinfo_for(original: &Path) -> Option<String> {
    let needle = format!("Path={}", original.display());
    entries(&trashed_info_dir())
        .into_iter()
        .filter_map(|name| fs::read_to_string(trashed_info_dir().join(name)).ok())
        .find(|body| body.lines().any(|line| line == needle))
}

#[test]
fn a_trashed_file_leaves_its_path_and_lands_in_the_real_trash() {
    let work = workspace("file");
    let file = work.join("doomed.txt");
    fs::write(&file, b"the bytes that must survive").expect("write");

    dispose(LocalDeleteMode::Trash, &file, EntityKind::File).expect("the trash move must succeed");

    assert!(!file.exists(), "the entity must leave its original path");

    // The bytes are still somewhere a person can reach: the trash holds a file of this size.
    let landed: Vec<_> = entries(&trashed_files_dir())
        .into_iter()
        .filter(|name| name.starts_with("doomed.txt"))
        .collect();
    assert_eq!(landed.len(), 1, "expected one trashed copy, got {landed:?}");
    assert_eq!(
        fs::read(trashed_files_dir().join(&landed[0])).expect("read the trashed copy"),
        b"the bytes that must survive",
        "trashing must move the entity, not truncate or replace it"
    );
}

#[test]
fn the_trash_records_the_original_path_and_the_deletion_time() {
    // The spec requirement a file manager's Restore depends on — and the reason the `chrono`
    // feature is taken back after `default-features = false`: without it the crate writes no
    // `DeletionDate` and the entry lands with no timestamp to age or sort by.
    let work = workspace("record");
    let file = work.join("recorded.txt");
    fs::write(&file, b"bytes").expect("write");

    dispose(LocalDeleteMode::Trash, &file, EntityKind::File).expect("trash");

    let info = trashinfo_for(&file).unwrap_or_else(|| {
        panic!(
            "no .trashinfo names {}; info dir holds {:?}",
            file.display(),
            entries(&trashed_info_dir())
        )
    });
    assert!(
        info.starts_with("[Trash Info]"),
        "the record must be a FreeDesktop trashinfo file, got:\n{info}"
    );
    let deletion_date = info
        .lines()
        .find_map(|line| line.strip_prefix("DeletionDate="))
        .unwrap_or_else(|| panic!("no DeletionDate in:\n{info}"));
    // `%Y-%m-%dT%H:%M:%S` — a shape, not a value, since the clock is the machine's.
    assert_eq!(deletion_date.len(), 19, "unexpected DeletionDate {deletion_date:?}");
    assert!(
        deletion_date.contains('T') && deletion_date.starts_with("20"),
        "unexpected DeletionDate {deletion_date:?}"
    );
}

#[test]
fn a_trashed_directory_moves_whole_with_its_contents_inside_it() {
    let work = workspace("directory");
    let dir = work.join("photos");
    fs::create_dir_all(dir.join("2019")).expect("mkdir");
    fs::write(dir.join("2019/one.jpg"), b"one").expect("write");
    fs::write(dir.join("top.txt"), b"top").expect("write");

    dispose(LocalDeleteMode::Trash, &dir, EntityKind::Directory).expect("trash the directory");

    assert!(!dir.exists(), "the folder must leave its original path");
    let landed: Vec<_> = entries(&trashed_files_dir())
        .into_iter()
        .filter(|name| name.starts_with("photos"))
        .collect();
    assert_eq!(landed.len(), 1, "expected one trashed folder, got {landed:?}");
    let trashed = trashed_files_dir().join(&landed[0]);
    assert_eq!(
        fs::read(trashed.join("2019/one.jpg")).expect("read a nested file"),
        b"one",
        "the subtree must travel inside the folder, not be flattened or dropped"
    );
    assert_eq!(fs::read(trashed.join("top.txt")).expect("read"), b"top");
}

#[test]
fn permanent_mode_puts_nothing_in_the_trash() {
    // The pre-change behaviour, asserted against the real trash rather than assumed: a permanent
    // deletion must not quietly become recoverable.
    let work = workspace("permanent");
    let file = work.join("gone-for-good.txt");
    fs::write(&file, b"bytes").expect("write");

    dispose(LocalDeleteMode::Permanent, &file, EntityKind::File).expect("permanent dispose");

    assert!(!file.exists());
    assert!(
        trashinfo_for(&file).is_none(),
        "permanent mode must leave no trash record"
    );
    assert!(
        !entries(&trashed_files_dir())
            .iter()
            .any(|name| name.starts_with("gone-for-good")),
        "permanent mode must leave nothing in the trash"
    );
}

#[test]
fn disposing_of_a_missing_entity_is_an_error_in_both_modes() {
    // The caller's `exists()` check is what makes a missing path a no-op; the seam itself does not
    // invent that, and this pins which side owns the decision.
    let work = workspace("missing");
    let absent = work.join("never-existed.txt");

    let permanent = dispose(LocalDeleteMode::Permanent, &absent, EntityKind::File)
        .expect_err("permanent must surface the missing path");
    assert_eq!(
        permanent
            .downcast_ref::<io::Error>()
            .map(io::Error::kind)
            .expect("permanent mode passes the io::Error through"),
        io::ErrorKind::NotFound
    );

    let trashed = dispose(LocalDeleteMode::Trash, &absent, EntityKind::File)
        .expect_err("trash must surface the missing path");
    assert!(
        trashed.to_string().contains("never-existed.txt"),
        "the error must name the path: {trashed}"
    );
}
