//! `read_config(pair)` and `write_config(pair, update)` (#102 phase 5b-1, ADR 0005 §7).
//!
//! The pair-table machinery itself is tested where it lives (`gui_core::config_io`); what is held
//! down here is what the COMMANDS add to it: which pair a call means, that a write names its pair
//! rather than reading the selection, and that a root edit is checked against the real paths.
//!
//! Nothing here reaches the machine. The config lives in a temp directory, the socket is the fake
//! daemon's, and the folders the overlap test builds are temp directories too.

use super::pair_tests::{harness, two_pair_daemon, Harness};
use super::*;
use serde_json::Value;

macro_rules! run {
    ($future:expr) => {
        tauri::async_runtime::block_on($future)
    };
}

/// Two pairs with a value in every table that a mix-up would show: different intervals, skip rules,
/// deletion policies and glob spellings, plus the daemon-wide keys at the top.
const PAIRS: &str = "\
# hand-written
log_level = \"info\"
proton_cli = \"proton-drive\"

[[pair]]
name = \"docs\"
local_root = \"/fake/docs/local\"
remote_root = \"/Drive/docs\"
exclude = [\"*.tmp\"]
events_driven = false

# the photos
[[pair]]
name = \"photos\"
local_root = \"/fake/photos/local\"
remote_root = \"/Drive/photos\"
scan_interval_secs = 600
deletion_policy = \"never\"
include_patterns = [\"a/**\"]
";

fn handle(h: &Harness) -> tauri::AppHandle<tauri::test::MockRuntime> {
    h.app.handle().clone()
}

fn config_text(h: &Harness) -> String {
    std::fs::read_to_string(h._dir.path().join("proton-sync.toml")).unwrap()
}

fn read(h: &Harness, pair: Option<&str>) -> Result<Value, String> {
    run!(read_config(h.state(), pair.map(str::to_owned))).map(|p| serde_json::to_value(p).unwrap())
}

fn update(f: impl FnOnce(&mut ConfigUpdate)) -> ConfigUpdate {
    let mut update = ConfigUpdate::default();
    f(&mut update);
    update
}

/// The text of one pair's table, up to the next `[[pair]]` header.
fn table_of(text: &str, name: &str) -> String {
    text.split("[[pair]]")
        .find(|table| table.contains(&format!("name = \"{name}\"")))
        .unwrap_or_else(|| panic!("no table for {name} in:\n{text}"))
        .to_owned()
}

/// The daemon has answered, so the app knows it reads a selector, and `photos` is chosen.
fn select_photos(h: &Harness) {
    run!(get_status(handle(h), None));
    run!(select_pair(handle(h), "photos".to_owned())).unwrap();
    assert_eq!(h.state().lock().unwrap().selected_pair().name, "photos");
}

// ---- read_config(pair) -------------------------------------------------------------------------------

#[test]
fn read_config_describes_the_pair_it_was_asked_about() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    let photos = read(&h, Some("photos")).unwrap();
    assert_eq!(photos["pair"], "photos");
    assert_eq!(photos["local_root"], "/fake/photos/local");
    assert_eq!(photos["scan_interval_secs"], 600);
    assert_eq!(photos["deletion_policy"], "never");
    // `include_patterns` is a spelling the parser accepts and this app used to read as nothing (F-D).
    assert_eq!(photos["include"], serde_json::json!(["a/**"]));
    assert_eq!(photos["exclude"], serde_json::json!([]));

    let docs = read(&h, Some("docs")).unwrap();
    assert_eq!(docs["pair"], "docs");
    assert_eq!(docs["local_root"], "/fake/docs/local");
    assert_eq!(docs["exclude"], serde_json::json!(["*.tmp"]));
    assert_eq!(docs["events_driven"], false);
    assert_eq!(
        docs["scan_interval_secs"],
        Value::Null,
        "the other pair's interval is not read"
    );
    assert_eq!(docs["deletion_policy"], "ask_every_time");

    // The daemon-wide values are the top level whichever pair is asked, and the roster is the whole file.
    for payload in [&photos, &docs] {
        assert_eq!(payload["log_level"], "info");
        assert_eq!(payload["proton_cli"], "proton-drive");
        assert_eq!(payload["pairs"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn read_config_naming_no_pair_means_the_selected_one_and_says_which() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    // Before the daemon has shown it reads a selector, the selection is the default pair.
    assert_eq!(read(&h, None).unwrap()["pair"], "docs");
    select_photos(&h);
    let selected = read(&h, None).unwrap();
    assert_eq!(
        selected["pair"], "photos",
        "the payload names the pair it describes"
    );
    assert_eq!(selected["scan_interval_secs"], 600);
    // Naming a pair overrides the selection.
    assert_eq!(read(&h, Some("docs")).unwrap()["pair"], "docs");
}

#[test]
fn read_config_refuses_a_pair_nobody_knows_and_a_pair_the_file_does_not_declare() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    let unknown = read(&h, Some("videos")).unwrap_err();
    assert!(unknown.contains("videos"), "{unknown}");

    // The daemon knows a pair the file has since lost: an answer built from another table would be
    // the wrong pair's settings under this one's name.
    run!(get_status(handle(&h), None));
    std::fs::write(
        h._dir.path().join("proton-sync.toml"),
        "[[pair]]\nname = \"docs\"\nlocal_root = \"/fake/docs/local\"\nremote_root = \"/Drive/docs\"\n",
    )
    .unwrap();
    let missing = read(&h, Some("photos")).unwrap_err();
    assert!(
        missing.contains("no folder pair named \"photos\"") && missing.contains("\"docs\""),
        "{missing}"
    );
}

#[test]
fn a_one_pair_file_reads_as_it_always_did() {
    // The N=1 half: the top level is the pair, and every flat value is the one it always was. The
    // daemon has not been asked anything, so the file's own roster is what names the pair.
    let h = harness(
        two_pair_daemon(),
        Some(
            "local_root = \"/l\"\nremote_root = \"/Drive/r\"\nscan_interval_secs = 90\n\
             exclude = [\"*.o\"]\nlog_level = \"debug\"\n",
        ),
    );
    let payload = read(&h, None).unwrap();
    assert_eq!(payload["pair"], "default");
    assert_eq!(payload["local_root"], "/l");
    assert_eq!(payload["scan_interval_secs"], 90);
    assert_eq!(payload["exclude"], serde_json::json!(["*.o"]));
    assert_eq!(payload["log_level"], "debug");
    assert_eq!(read(&h, Some("default")).unwrap(), payload);
    // …and the pair the file does not have is refused rather than answered with the top level.
    assert!(read(&h, Some("docs")).unwrap_err().contains("docs"));
}

// ---- write_config(pair, update) ----------------------------------------------------------------------

#[test]
fn write_config_writes_the_named_pair_and_only_it() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    let before = config_text(&h);
    run!(write_config(
        h.state(),
        "photos".to_owned(),
        update(|u| {
            u.scan_interval_secs = Some(900);
            u.exclude = Some(vec!["*.psd".to_owned()]);
            u.deletion_policy = Some(config_io::DeletionPolicy::AskEveryTime);
            u.log_level = Some("debug".to_owned());
        }),
    ))
    .expect("a valid update saves");
    let after = config_text(&h);

    assert_eq!(
        table_of(&before, "docs"),
        table_of(&after, "docs"),
        "the pair that was not named is the same bytes"
    );
    let photos = table_of(&after, "photos");
    assert!(photos.contains("scan_interval_secs = 900"), "{after}");
    assert!(photos.contains("exclude = [\"*.psd\"]"), "{after}");
    assert!(
        photos.contains("deletion_policy = \"ask_every_time\""),
        "{after}"
    );
    assert!(
        photos.contains("include_patterns = [\"a/**\"]"),
        "an untouched key keeps its spelling and value: {after}"
    );
    // The daemon-wide key stayed at the top level, and was edited there.
    assert!(
        after.starts_with("# hand-written\nlog_level = \"debug\"\n"),
        "{after}"
    );
    assert_eq!(after.matches("log_level").count(), 1, "{after}");
    // And the app's view of the file moved with it.
    let reread = read(&h, Some("photos")).unwrap();
    assert_eq!(reread["scan_interval_secs"], 900);
    assert_eq!(reread["deletion_policy"], "ask_every_time");
}

/// A save of a one-pair file is the same BYTES it was before pairs had tables: a key the file did not
/// have is appended where it is written, so the order `write_config` applies an update in is part of
/// what a save produces. This is the order of the function this replaced, spelled out, applied to an
/// empty file so every key is new.
#[test]
fn a_one_pair_save_appends_new_keys_in_the_order_it_always_did() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(two_pair_daemon(), None);
    run!(write_config(
        h.state(),
        "default".to_owned(),
        update(|u| {
            // Deliberately NOT in the order they are applied: a struct literal's order is not the
            // order of the writes.
            u.local_delete_mode = Some(config_io::LocalDeleteMode::Permanent);
            u.deletion_policy = Some(config_io::DeletionPolicy::Never);
            u.full_scan_schedule = Some("weekly sun 03:00".to_owned());
            u.conflict_suffix = Some("from-cloud".to_owned());
            u.log_level = Some("debug".to_owned());
            u.socket_path = Some(dir.path().join("p.sock").display().to_string());
            u.proton_list_attempts = Some(3);
            u.proton_timeout_secs = Some(90);
            u.proton_cli = Some("/bin/proton-drive".to_owned());
            u.exclude = Some(vec!["*.tmp".to_owned()]);
            u.include = Some(vec!["a/**".to_owned()]);
            u.events_driven = Some(true);
            u.scan_interval_secs = Some(60);
            u.remote_root = Some("/Drive/x".to_owned());
            u.local_root = Some(dir.path().join("x").display().to_string());
        }),
    ))
    .expect("saves");
    assert_eq!(
        config_text(&h),
        format!(
            "local_root = \"{0}/x\"\nremote_root = \"/Drive/x\"\nscan_interval_secs = 60\n\
             events_driven = true\ninclude = [\"a/**\"]\nexclude = [\"*.tmp\"]\n\
             proton_cli = \"/bin/proton-drive\"\nproton_timeout_secs = 90\nproton_list_attempts = 3\n\
             socket_path = \"{0}/p.sock\"\nlog_level = \"debug\"\nconflict_suffix = \"from-cloud\"\n\
             full_scan_schedule = \"weekly sun 03:00\"\ndeletion_policy = \"never\"\n\
             local_delete_mode = \"permanent\"\n",
            dir.path().display()
        )
    );
}

/// THE GUARD THE WHOLE CLASS-W RULE EXISTS FOR (brief 2.3, A3). The selection lives in Rust and moves
/// on a click in the other webview, a banner action or the post-restart auto-select; a write that read
/// it at the moment it ran would land on whichever folder was selected by then. The pair is an
/// argument, captured when the edit began.
#[test]
fn a_save_for_pair_a_never_lands_in_pair_b_when_the_selection_moves() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    select_photos(&h);

    // The edit was staged against `docs`; the selection is `photos` by the time it runs.
    let before = config_text(&h);
    run!(write_config(
        h.state(),
        "docs".to_owned(),
        update(|u| u.scan_interval_secs = Some(45)),
    ))
    .expect("saves");
    let after = config_text(&h);

    assert!(
        table_of(&after, "docs").contains("scan_interval_secs = 45"),
        "{after}"
    );
    assert_eq!(
        table_of(&before, "photos"),
        table_of(&after, "photos"),
        "the selected pair was not the one named, so it was not written"
    );
    assert!(table_of(&after, "photos").contains("scan_interval_secs = 600"));
}

#[test]
fn a_write_refuses_a_pair_the_app_does_not_know_and_changes_nothing() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    let before = config_text(&h);
    for name in ["videos", "Docs", ""] {
        let error = run!(write_config(
            h.state(),
            name.to_owned(),
            update(|u| u.scan_interval_secs = Some(1)),
        ))
        .unwrap_err();
        assert!(error.contains("no folder pair named"), "{name:?}: {error}");
    }
    assert_eq!(config_text(&h), before);
}

#[test]
fn a_write_for_a_pair_the_daemon_knows_and_the_file_lost_is_refused_by_the_file() {
    let h = harness(two_pair_daemon(), Some(PAIRS));
    run!(get_status(handle(&h), None));
    let only_docs =
        "[[pair]]\nname = \"docs\"\nlocal_root = \"/fake/docs/local\"\nremote_root = \"/Drive/docs\"\n";
    std::fs::write(h._dir.path().join("proton-sync.toml"), only_docs).unwrap();
    let error = run!(write_config(
        h.state(),
        "photos".to_owned(),
        update(|u| u.scan_interval_secs = Some(1)),
    ))
    .unwrap_err();
    assert!(error.contains("no folder pair named \"photos\""), "{error}");
    assert_eq!(
        config_text(&h),
        only_docs,
        "nothing was written, least of all into another table"
    );
}

#[test]
fn an_inline_pair_array_is_read_and_a_per_pair_save_is_refused_with_the_plain_sentence() {
    let inline = "log_level = \"info\"\npair = [\
        { name = \"docs\", local_root = \"/fake/docs/local\", remote_root = \"/Drive/docs\", \
          exclude = [\"*.tmp\"] }, \
        { name = \"photos\", local_root = \"/fake/photos/local\", remote_root = \"/Drive/photos\" }]\n";
    let h = harness(two_pair_daemon(), Some(inline));

    // READ: the inline pair's own values, through the same reader as a table.
    let docs = read(&h, Some("docs")).unwrap();
    assert_eq!(docs["local_root"], "/fake/docs/local");
    assert_eq!(docs["exclude"], serde_json::json!(["*.tmp"]));

    // A per-pair save is refused, in a sentence that names the file and says what the app edits.
    let error = run!(write_config(
        h.state(),
        "docs".to_owned(),
        update(|u| u.scan_interval_secs = Some(60)),
    ))
    .unwrap_err();
    assert!(error.contains("proton-sync.toml"), "{error}");
    assert!(error.contains("inline array"), "{error}");
    assert!(error.contains("`[[pair]]` tables"), "{error}");
    assert_eq!(config_text(&h), inline, "refused means untouched");

    // A daemon-wide save is not in the way: the top level is still the daemon's.
    run!(write_config(
        h.state(),
        "docs".to_owned(),
        update(|u| u.log_level = Some("debug".to_owned())),
    ))
    .expect("a daemon-wide edit saves beside an inline array");
    assert_eq!(
        config_text(&h),
        inline.replace("log_level = \"info\"", "log_level = \"debug\"")
    );
}

// ---- the real-path check -----------------------------------------------------------------------------

/// Two pairs whose folders are apart, and a symlink to the inside of the first.
struct Folders {
    _dir: tempfile::TempDir,
    docs: std::path::PathBuf,
    photos: std::path::PathBuf,
    alias: std::path::PathBuf,
}

fn folders() -> Folders {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    let photos = dir.path().join("photos");
    std::fs::create_dir_all(docs.join("inner")).unwrap();
    std::fs::create_dir_all(&photos).unwrap();
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(docs.join("inner"), &alias).unwrap();
    Folders {
        _dir: dir,
        docs,
        photos,
        alias,
    }
}

fn pairs_at(folders: &Folders) -> String {
    format!(
        "[[pair]]\nname = \"docs\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n\n\
         [[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\n",
        folders.docs.display(),
        folders.photos.display()
    )
}

#[test]
fn a_root_edit_that_overlaps_through_a_symlink_is_refused_at_save() {
    let folders = folders();
    let text = pairs_at(&folders);
    let h = harness(two_pair_daemon(), Some(&text));

    // Lexically the folders are apart (`…/alias` and `…/docs`), so the validator passes this.
    let error = run!(write_config(
        h.state(),
        "photos".to_owned(),
        update(|u| u.local_root = Some(folders.alias.display().to_string())),
    ))
    .unwrap_err();
    // The ENGINE's sentence for the same fact the daemon would exit on at boot: both pairs named, and
    // the path as written beside the folder it really is.
    assert!(
        error.starts_with("folder pair 'photos': its local_root"),
        "{error}"
    );
    assert!(
        error.contains("is inside folder pair 'docs''s local_root"),
        "{error}"
    );
    assert!(error.contains("(really `"), "{error}");
    assert_eq!(config_text(&h), text, "refused means untouched");

    // A root that is apart saves, so the check is not simply refusing every root edit.
    let elsewhere = folders._dir.path().join("elsewhere");
    run!(write_config(
        h.state(),
        "photos".to_owned(),
        update(|u| u.local_root = Some(elsewhere.display().to_string())),
    ))
    .expect("an apart folder saves");
    assert!(config_text(&h).contains(&elsewhere.display().to_string()));
}

#[test]
fn the_real_path_check_runs_when_an_update_touches_a_root_and_not_otherwise() {
    // An already-overlapping file is the daemon's boot refusal to report, not every unrelated save's.
    // The check is asked for when an update can MOVE a folder.
    let folders = folders();
    let overlapping = format!(
        "[[pair]]\nname = \"docs\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\n\n\
         [[pair]]\nname = \"photos\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/photos\"\n",
        folders.docs.display(),
        folders.alias.display()
    );
    let h = harness(two_pair_daemon(), Some(&overlapping));
    run!(write_config(
        h.state(),
        "photos".to_owned(),
        update(|u| u.scan_interval_secs = Some(60)),
    ))
    .expect("an update that moves no folder is not held up by a standing overlap");

    let error = run!(write_config(
        h.state(),
        "photos".to_owned(),
        update(|u| u.local_root = Some(folders.alias.display().to_string())),
    ))
    .unwrap_err();
    assert!(error.contains("(really `"), "{error}");
}

#[test]
fn only_an_update_that_names_a_folder_asks_for_the_real_path_check() {
    assert!(!ConfigUpdate::default().touches_state_paths());
    assert!(update(|u| u.local_root = Some("/x".to_owned())).touches_state_paths());
    for quiet in [
        update(|u| u.remote_root = Some("/Drive/x".to_owned())),
        update(|u| u.scan_interval_secs = Some(1)),
        update(|u| u.exclude = Some(vec![])),
        update(|u| u.log_level = Some("debug".to_owned())),
    ] {
        assert!(!quiet.touches_state_paths());
    }
}

#[test]
fn a_one_pair_root_edit_saves_with_nothing_to_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        "local_root = \"{0}/a\"\nremote_root = \"/Drive/a\"\n",
        dir.path().display()
    );
    let h = harness(two_pair_daemon(), Some(&text));
    run!(write_config(
        h.state(),
        "default".to_owned(),
        update(|u| u.local_root = Some(format!("{}/b", dir.path().display()))),
    ))
    .expect("one pair has nothing to overlap with");
    assert!(config_text(&h).contains("/b\""));
}
