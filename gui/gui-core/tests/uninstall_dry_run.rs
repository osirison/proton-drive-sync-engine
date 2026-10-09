//! What `uninstall.sh --dry-run` makes of a `[[pair]]` file the APP wrote (#102 phase 5c-2, brief PR 8).
//!
//! `tests/shell_reader.rs` holds the reader's one function (`config_values`) against every state the
//! editor produces; the engine's own `tests/scripts.rs` runs the whole script over hand-written files.
//! This is the join: a file made the way the app makes it — an implicit single-pair file, promoted to
//! `[[pair]]` form by `add_pair` and saved by the atomic writer — handed to the real script, in a cleared
//! environment, in its dry-run mode only. The question is the one a person asks before they uninstall: are
//! the `.sync` directories of BOTH folders in the plan, or would the app's first multi-folder file leave one
//! behind?
//!
//! **Nothing here touches the machine.** `--dry-run` returns before anything is removed; `HOME` and every
//! XDG directory are inside the temp directory, `PATH` leads with stub `systemctl` and `cargo` that report
//! "nothing installed" (the engine's `tests/scripts.rs` sandbox, written out because an integration test of
//! another crate cannot import its `tests/common`), and the session bus is removed.
#![cfg(unix)]

use proton_sync_gui_core::config_io::{ConfigDoc, PairInit};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn uninstall_script() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../uninstall.sh")
}

/// A scratch tree whose every directory the script reads is under it.
fn sandbox(root: &Path) {
    for name in [
        "home",
        "xdg-config",
        "xdg-data",
        "xdg-cache",
        "xdg-state",
        "runtime",
        "tmp",
        "bin",
    ] {
        fs::create_dir(root.join(name)).expect("sandbox directory");
    }
    for (name, body) in [
        ("systemctl", "#!/bin/sh\nexit 1\n"),
        ("cargo", "#!/bin/sh\nexit 0\n"),
    ] {
        let path = root.join("bin").join(name);
        fs::write(&path, body).expect("stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("stub mode");
    }
}

/// A folder with the `.sync` directory the script validates before it lists it.
fn folder_with_state(root: &Path, name: &str) -> std::path::PathBuf {
    let folder = root.join("folders").join(name);
    fs::create_dir_all(folder.join(".sync")).expect("folder with .sync");
    folder
}

fn dry_run(root: &Path, config: &Path) -> String {
    let output = Command::new("bash")
        .env_clear()
        .env("HOME", root.join("home"))
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.join("bin").display()),
        )
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("TMPDIR", root.join("tmp"))
        .arg(uninstall_script())
        .arg("--dry-run")
        .arg("--config")
        .arg(config)
        .output()
        .expect("run uninstall.sh --dry-run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("Dry run"),
        "the run ended at the dry-run return: {text}"
    );
    text
}

#[test]
fn a_file_the_app_wrote_for_two_folders_lists_both_folders_state_in_the_uninstall_plan() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    sandbox(root);
    let docs = folder_with_state(root, "docs");
    let photos = folder_with_state(root, "photos");

    // The file as a first-time setup writes it: no `[[pair]]` anywhere, one implicit pair.
    let config = root.join("proton-sync.toml");
    fs::write(
        &config,
        format!(
            "# my config\nlocal_root = \"{}\"\nremote_root = \"/Drive/docs\"\nexclude = [\"*.tmp\"]\n",
            docs.display()
        ),
    )
    .expect("write config");

    // And then the add dialog's one write: promotion and the new table together, saved the way the app saves.
    let mut document = ConfigDoc::load(&config).expect("load");
    document
        .add_pair(PairInit {
            name: "photos".to_owned(),
            local_root: photos.display().to_string(),
            remote_root: "/Drive/photos".to_owned(),
            exclude: vec!["*.raw".to_owned()],
        })
        .expect("the add");
    document.save(&config).expect("save");
    let written = fs::read_to_string(&config).expect("read back");
    assert_eq!(
        written.matches("[[pair]]").count(),
        2,
        "two tables: {written}"
    );
    assert!(
        written.contains("name = \"default\""),
        "the implicit pair became the first table, called default: {written}"
    );

    let text = dry_run(root, &config);
    for folder in [&docs, &photos] {
        assert!(
            text.contains(&format!("{}", folder.join(".sync").display())),
            "{} is listed for removal: {text}",
            folder.display()
        );
    }
    assert!(
        !text.contains("NOT in the plan"),
        "and no pair is reported as one the script could not read: {text}"
    );
}
