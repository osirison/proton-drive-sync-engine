//! `--version` on both binaries: it prints the upstream version and does nothing else.
//!
//! The release pipeline's install test asks an installed package's binaries for their version
//! (`packaging/version.sh` writes the same string into every manifest), on a machine with no
//! config file and no daemon. Answering must not read a config, bind the control socket, take a
//! lock or create a state directory, so the sandbox directory is checked to be empty afterwards.
#![cfg(unix)]

mod common;

use std::fs;
use tempfile::tempdir;

/// The sandbox directory holds nothing: no control socket, lockfile, state directory or config.
fn assert_directory_is_empty(directory: &std::path::Path) {
    let left: Vec<_> = fs::read_dir(directory)
        .expect("read the sandbox directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert!(
        left.is_empty(),
        "--version created something in the sandbox: {left:?}"
    );
}

#[test]
fn the_daemon_prints_its_version_and_creates_nothing() {
    let directory = tempdir().expect("tempdir");
    // `refuse_the_real_cli` still applies to the daemon: name a fake CLI that is never created
    // (clap exits before it is looked at) and turn events off, so a run that did start would be
    // refused by the sandbox rather than reaching the keyring.
    let fake_cli = directory.path().join("fake-proton-drive");
    let mut command = common::syncd(directory.path());
    command
        .arg("--proton-cli")
        .arg(&fake_cli)
        .arg("--no-events-driven")
        .arg("--version");
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        format!("proton-syncd {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_directory_is_empty(directory.path());
}

#[test]
fn the_control_cli_prints_its_version_and_creates_nothing() {
    let directory = tempdir().expect("tempdir");
    let mut command = common::sync_cli(directory.path());
    command.arg("--version");
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8"),
        format!("proton-sync {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_directory_is_empty(directory.path());
}
