//! A hand-written **relative** `local_root` and the set-aside (#102 phase 5b-2, review round).
//!
//! The daemon resolves a relative path against *its own* working directory; this app has another.
//! Planning it relative to the app's meant moving whichever folder of that name was in the app's
//! working directory — someone else's `.sync` — and calling the removal "moved". The planner now
//! refuses a relative path without looking at the disk.
//!
//! **This file holds exactly one test, on purpose:** it changes the process's working directory, which
//! is process-wide, and a test binary of one is the only place that cannot race another test.

use proton_sync_gui_core::config_io::pair_views;
use proton_sync_gui_core::set_aside::{self, Context, Planned};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_relative_root_is_never_resolved_against_the_apps_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    // Two different working directories: the daemon's (where its history really is) and the app's
    // (where another folder of the same name happens to be).
    let daemon_cwd = dir.path().join("home");
    let app_cwd = dir.path().join("project");
    write(
        &daemon_cwd.join("Sync/.sync/sync_index.db"),
        "THE DAEMON'S HISTORY",
    );
    write(
        &app_cwd.join("Sync/.sync/other-tool.state"),
        "somebody else's state",
    );
    write(&app_cwd.join("Sync/.sync/proton-sync.lock"), "");
    fs::create_dir_all(dir.path().join("B")).unwrap();
    let config = format!(
        "[[pair]]\nname = \"a\"\nlocal_root = \"Sync\"\nremote_root = \"/Drive/a\"\n\n\
         [[pair]]\nname = \"b\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/b\"\n",
        dir.path().join("B").display()
    );
    let views = pair_views(&config).unwrap();
    assert!(views[0].local_root.as_ref().unwrap().is_relative());

    std::env::set_current_dir(&app_cwd).unwrap();
    let planned = set_aside::plan(&views[0]);
    let Planned::Undetermined(undetermined) = planned else {
        panic!("a relative root was planned against the app's directory: {planned:?}")
    };
    assert!(!undetermined.retry);
    assert!(
        undetermined.reason.contains("relative path"),
        "{}",
        undetermined.reason
    );

    // And a record that somehow carries it moves nothing either.
    let state = dir.path().join("state");
    let identity = set_aside::Plan::of_view(&views[0], Vec::new(), Vec::new());
    set_aside::record_pending(&state, &identity, "test", SystemTime::now()).unwrap();
    let configured = vec![views[1].clone()];
    let results = set_aside::settle_pending(&Context {
        state_dir: &state,
        configured: &configured,
        now: SystemTime::now(),
    });
    assert!(
        matches!(&results[..], [set_aside::Settled::StillPending { .. }]),
        "{results:?}"
    );
    assert!(
        app_cwd.join("Sync/.sync/other-tool.state").exists(),
        "the folder in the app's directory was not touched"
    );
    assert!(
        daemon_cwd.join("Sync/.sync/sync_index.db").exists(),
        "and neither was the daemon's"
    );
}
