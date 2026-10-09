//! A set-aside across two **real** filesystems (#102 phase 5b-2, review round).
//!
//! The unit tests make `EXDEV` appear by injecting a `rename` that fails with it. This is the one
//! test that lets the kernel do it: the history goes from the temp directory to `/dev/shm`, which are
//! two mounts on Linux, so `rename` really fails with `EXDEV` and the copy → `fsync` → compare →
//! remove path runs against a real filesystem (an `fsync` on a directory is a call some filesystems
//! refuse, and a unit test with a stub cannot say whether this one does). Where the two are the same
//! filesystem, or `/dev/shm` is not there, it says so and stops rather than pass.

use proton_sync_gui_core::config_io::pair_views;
use proton_sync_gui_core::set_aside::{self, Context, How, Planned};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_move_across_two_real_filesystems_copies_syncs_verifies_and_removes() {
    let Ok(shm) = tempfile::tempdir_in("/dev/shm") else {
        eprintln!("SKIPPED: no writable /dev/shm");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    if fs::metadata(dir.path()).unwrap().dev() == fs::metadata(shm.path()).unwrap().dev() {
        eprintln!("SKIPPED: the temp directory and /dev/shm are one filesystem");
        return;
    }
    let (a, b) = (dir.path().join("A"), dir.path().join("B"));
    fs::create_dir_all(&b).unwrap();
    write(&a.join(".sync/sync_index.db"), &"x".repeat(300_000));
    write(&a.join(".sync/sync_index.status.json"), "[]");
    write(&a.join(".sync/nested/deeper/file"), "deep");
    fs::set_permissions(
        a.join(".sync/sync_index.db"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    write(&a.join("mine.txt"), "mine");
    let config = format!(
        "[[pair]]\nname = \"a\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/a\"\n\n\
         [[pair]]\nname = \"b\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/b\"\n",
        a.display(),
        b.display()
    );
    let views = pair_views(&config).unwrap();
    let Planned::Move(plan) = set_aside::plan(&views[0]) else {
        panic!("there is history to move")
    };
    let configured = vec![views[1].clone()];
    let done = set_aside::execute(
        &plan,
        &Context {
            state_dir: shm.path(),
            configured: &configured,
            now: SystemTime::now(),
        },
    )
    .expect("moved");

    assert_eq!(
        done.moved[0].how,
        How::Copy,
        "the kernel refused the rename"
    );
    assert!(!a.join(".sync").exists(), "the original is gone");
    assert_eq!(fs::read_to_string(a.join("mine.txt")).unwrap(), "mine");
    let to = &done.moved[0].to;
    assert_eq!(
        fs::read_to_string(to.join("sync_index.db")).unwrap().len(),
        300_000
    );
    assert_eq!(
        fs::read_to_string(to.join("nested/deeper/file")).unwrap(),
        "deep"
    );
    assert_eq!(
        fs::metadata(to.join("sync_index.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "permissions survive the copy"
    );
    assert!(done.to.join(set_aside::MANIFEST_NAME).exists());
}
