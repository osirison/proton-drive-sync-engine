#[cfg(unix)]
mod unix_tests {
    use proton_drive_sync_engine::index::load_existing_index;
    use serde_json::Value;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn control_cli_exercises_daemon_ipc_lifecycle() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_fake_proton_drive(directory.path(), "/Drive/RemoteFolder");
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon.child);

        let status = run_control(&socket_path, "status");
        assert_eq!(status["status"], "running");
        assert_eq!(status["paused"], false);
        assert!(status["last_error"].is_null());
        // The daemon reconciles once on startup (remote and local are both empty here), so an
        // empty plan and a matching successful-sync summary are already present before any
        // manual syncnow.
        assert_eq!(status["last_plan_summary"]["total"].as_u64(), Some(0));
        assert_eq!(
            status["last_successful_sync_summary"]["total"].as_u64(),
            Some(0)
        );
        assert!(status["last_sync_epoch_secs"].as_u64().is_some());

        let paused = run_control(&socket_path, "pause");
        assert_eq!(paused["status"], "paused");
        assert_eq!(paused["paused"], true);

        let skipped = run_control(&socket_path, "syncnow");
        assert_eq!(skipped["status"], "paused");
        assert_eq!(skipped["message"], "sync skipped because daemon is paused");

        let resumed = run_control(&socket_path, "resume");
        assert_eq!(resumed["status"], "running");
        assert_eq!(resumed["paused"], false);

        let synced = run_control(&socket_path, "syncnow");
        assert_eq!(synced["status"], "running");
        assert_eq!(synced["message"], "sync completed");
        assert!(synced["last_sync_epoch_secs"].as_u64().is_some());
        assert!(synced["last_error"].is_null());
        assert_eq!(synced["last_plan_summary"]["total"].as_u64(), Some(0));
        assert_eq!(
            synced["last_successful_sync_summary"]["total"].as_u64(),
            Some(0)
        );

        let history = run_control(&socket_path, "history");
        let history = history.as_array().expect("history JSON array");
        // Two entries: the startup reconcile, then the manual syncnow after resume. (The syncnow
        // issued while paused is skipped without reconciling, so it records no history entry.)
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["message"], "sync completed");
        assert_eq!(history[1]["message"], "sync completed");
        assert_eq!(
            history[1]["successful_sync_summary"]["total"].as_u64(),
            Some(0)
        );
    }

    #[test]
    fn malformed_control_request_does_not_crash_the_daemon() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_fake_proton_drive(directory.path(), "/Drive/RemoteFolder");
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon.child);

        // Send an invalid (non-JSON) request line and drop the connection without
        // reading a response. Before the fix, `read_request`'s parse error propagated
        // out of the `run()` select loop via `?` and terminated the whole daemon
        // process; a well-behaved daemon must instead log the error and keep serving
        // subsequent connections.
        let mut malformed = UnixStream::connect(&socket_path).expect("connect malformed");
        malformed
            .write_all(b"not valid json\n")
            .expect("write malformed request");
        drop(malformed);

        // An abrupt disconnect with no data at all (immediate EOF) must be tolerated
        // the same way.
        let abrupt = UnixStream::connect(&socket_path).expect("connect abrupt");
        drop(abrupt);

        assert!(
            daemon.child.try_wait().expect("daemon status").is_none(),
            "daemon must still be running after malformed control requests"
        );

        let status = run_control(&socket_path, "status");
        assert_eq!(status["status"], "running");
    }

    #[test]
    fn failed_upload_syncnow_does_not_commit_partial_index_state() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("first.txt"), b"first").expect("first file");
        fs::write(local_root.join("second.txt"), b"second").expect("second file");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive =
            write_failing_upload_proton_drive(directory.path(), "/Drive/RemoteFolder");
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon.child);

        let synced = run_control(&socket_path, "syncnow");

        assert_eq!(synced["status"], "running");
        assert!(
            synced["message"]
                .as_str()
                .unwrap_or_default()
                .contains("sync failed"),
            "syncnow should report failure: {synced}"
        );
        assert!(
            synced["last_error"]
                .as_str()
                .unwrap_or_default()
                .contains("proton-drive upload failed"),
            "daemon should expose upload failure: {synced}"
        );
        let index = load_existing_index(&db_path).expect("load index after failed upload");
        assert!(
            index.is_empty(),
            "failed upload must not commit any synced rows: {index:?}"
        );
    }

    // Regression test for real SIGINT handling during a blocked sync. The daemon's
    // main loop is a single tokio task that runs its reconcile step via
    // `block_in_place`, so a SIGINT is observed by a separate, always-running task
    // that flips a shared cancel flag the instant the signal arrives; `run_once`'s
    // polling loop (see `src/proton.rs`) then notices that flag within its short
    // poll interval and kills the stuck CLI's whole process group, letting the
    // blocked reconcile call return well before the CLI's own command timeout
    // would otherwise elapse. This test proves the daemon reaches a clean,
    // tightly bounded shutdown - not just eventually, or only once its own
    // command timeout kills the stuck CLI process - and that the interruption
    // leaves no partial index state and releases the lockfile.
    //
    // The daemon reconciles on startup, so the pre-seeded `blocking.txt` drives the blocking
    // upload directly (the startup reconcile *is* the blocked sync); no syncnow is needed. This
    // also exercises the startup path specifically: a SIGINT delivered while the very first
    // reconcile is stuck must still be latched by the loop's shutdown future and exit cleanly.
    #[test]
    fn sigint_during_blocked_upload_exits_cleanly_without_partial_index_state() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("blocking.txt"), b"content").expect("write fixture");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive =
            write_blocking_upload_proton_drive(directory.path(), "/Drive/RemoteFolder");

        // Keep the CLI's own timeout short: it bounds how long the daemon's
        // reconcile call can stay blocked before it forcibly kills the stuck
        // upload and re-observes the SIGINT it already received.
        let mut daemon = DaemonProcess::spawn_with_proton_timeout(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
            2,
        );
        wait_for_socket(&socket_path, &mut daemon.child);
        let pid = daemon.child.id();

        // The startup reconcile's upload is now the blocked call; its marker proves we are wedged
        // inside the (interruptible) reconcile before the signal is sent.
        let started_marker = PathBuf::from(format!("{}.started", fake_proton_drive.display()));
        wait_for_marker(&started_marker, &mut daemon.child);

        let status = Command::new("kill")
            .arg("-INT")
            .arg(pid.to_string())
            .status()
            .expect("send SIGINT to daemon");
        assert!(status.success(), "kill -INT should succeed");

        let exit_status = wait_for_exit(&mut daemon.child, Duration::from_secs(4))
            .expect("daemon should exit promptly once it re-observes the already-delivered SIGINT");
        assert!(
            exit_status.success(),
            "daemon should shut down cleanly after SIGINT: {exit_status:?}"
        );

        let index = load_existing_index(&db_path).expect("load index after interrupted upload");
        assert!(
            index.is_empty(),
            "an interrupted upload must not leave partial index state: {index:?}"
        );

        assert!(
            !lockfile_path.exists(),
            "lockfile should be removed after a clean SIGINT-triggered shutdown"
        );
    }

    #[test]
    fn delete_approval_withholds_a_remote_delete_until_approved_over_ipc() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        // A fake that serves one downloadable remote file, records trashes, and never lists the
        // file as gone — so once its local copy is removed the daemon plans a RemoteDelete.
        let fake_proton_drive = write_delete_approval_proton_drive(directory.path());
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon.child);

        // The startup reconcile downloads keep.txt and records a synced baseline.
        let local_file = local_root.join("keep.txt");
        wait_for_marker(&local_file, &mut daemon.child);

        // Remove the local copy: the next reconcile plans a RemoteDelete (local gone, remote still
        // present, baseline unchanged). The guard (on by default) must withhold it.
        fs::remove_file(&local_file).expect("remove local file");
        let trash_marker = PathBuf::from(format!("{}.trash", fake_proton_drive.display()));

        let withheld = run_control(&socket_path, "syncnow");
        assert_eq!(withheld["message"], "sync completed");
        let pending = withheld["pending_deletions"]
            .as_array()
            .expect("pending_deletions array");
        assert_eq!(
            pending.len(),
            1,
            "the remote delete must be withheld: {withheld}"
        );
        assert_eq!(pending[0]["direction"], "remote");
        assert_eq!(pending[0]["path"], "keep.txt");
        assert!(
            !trash_marker.exists(),
            "no remote trash may happen before approval"
        );

        // The `pending` control command renders the withheld deletion for the user.
        let listed = run_control_raw(&socket_path, &["pending"]);
        assert!(
            listed.contains("keep.txt") && listed.contains("REMOTE DELETE"),
            "`pending` must show the withheld remote delete: {listed}"
        );

        // Approve exactly that path, then reconcile again: the delete now applies.
        let approved = run_control_raw(&socket_path, &["approve", "keep.txt"]);
        assert!(
            approved.contains("approved 1"),
            "approve should confirm one approval: {approved}"
        );

        let applied = run_control(&socket_path, "syncnow");
        assert_eq!(applied["message"], "sync completed");
        assert!(
            applied["pending_deletions"]
                .as_array()
                .expect("pending array")
                .is_empty(),
            "nothing should remain pending after the approved delete applies: {applied}"
        );
        assert!(
            trash_marker.exists(),
            "the approved remote delete must have trashed the remote file"
        );
        let trashed = fs::read_to_string(&trash_marker).expect("read trash marker");
        assert!(
            trashed.contains("keep.txt"),
            "the trash call must target keep.txt: {trashed}"
        );
    }

    struct DaemonProcess {
        child: Child,
    }

    impl DaemonProcess {
        fn spawn(
            local_root: &Path,
            socket_path: &Path,
            lockfile_path: &Path,
            db_path: &Path,
            proton_cli: &Path,
        ) -> Self {
            let child = Command::new(env!("CARGO_BIN_EXE_proton-syncd"))
                .arg("--local-root")
                .arg(local_root)
                .arg("--remote-root")
                .arg("/Drive/RemoteFolder")
                .arg("--socket-path")
                .arg(socket_path)
                .arg("--lockfile-path")
                .arg(lockfile_path)
                .arg("--db-path")
                .arg(db_path)
                .arg("--proton-cli")
                .arg(proton_cli)
                .arg("--scan-interval-secs")
                .arg("60")
                // Keep these process-level tests on the full-tree snapshot path (the default is
                // now event-driven, which would try to read the CLI keyring session at startup).
                .arg("--no-events-driven")
                // Isolate the user-global single-instance lock per test: `default_global_lock_path`
                // keys on `$XDG_STATE_HOME`, so pointing it at this test's tempdir stops parallel
                // ipc_cli daemons contending on one machine-global lock (they would else exit 1).
                .env(
                    "XDG_STATE_HOME",
                    lockfile_path.parent().expect("lockfile has a parent dir"),
                )
                .env("RUST_LOG", "error")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn proton-syncd");
            Self { child }
        }

        fn spawn_with_proton_timeout(
            local_root: &Path,
            socket_path: &Path,
            lockfile_path: &Path,
            db_path: &Path,
            proton_cli: &Path,
            proton_timeout_secs: u64,
        ) -> Self {
            let child = Command::new(env!("CARGO_BIN_EXE_proton-syncd"))
                .arg("--local-root")
                .arg(local_root)
                .arg("--remote-root")
                .arg("/Drive/RemoteFolder")
                .arg("--socket-path")
                .arg(socket_path)
                .arg("--lockfile-path")
                .arg(lockfile_path)
                .arg("--db-path")
                .arg(db_path)
                .arg("--proton-cli")
                .arg(proton_cli)
                .arg("--scan-interval-secs")
                .arg("60")
                // Keep these process-level tests on the full-tree snapshot path (the default is
                // now event-driven, which would try to read the CLI keyring session at startup).
                .arg("--no-events-driven")
                .arg("--proton-timeout-secs")
                .arg(proton_timeout_secs.to_string())
                .env("RUST_LOG", "error")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn proton-syncd");
            Self { child }
        }
    }

    impl Drop for DaemonProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn wait_for_socket(socket_path: &Path, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if socket_path.exists() {
                return;
            }
            if let Some(status) = child.try_wait().expect("daemon status") {
                panic!("proton-syncd exited before binding socket: {status}");
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "timed out waiting for daemon socket at {}",
            socket_path.display()
        );
    }

    fn wait_for_marker(marker_path: &Path, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if marker_path.exists() {
                return;
            }
            if let Some(status) = child.try_wait().expect("daemon status") {
                panic!("proton-syncd exited before reaching the expected marker: {status}");
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "timed out waiting for marker file at {}",
            marker_path.display()
        );
    }

    fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().expect("daemon status") {
                return Some(status);
            }
            thread::sleep(Duration::from_millis(25));
        }
        None
    }

    fn run_control(socket_path: &Path, command: &str) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--socket-path")
            .arg(socket_path)
            .arg(command)
            .output()
            .expect("run proton-sync");
        assert!(
            output.status.success(),
            "proton-sync {command} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("control response JSON")
    }

    /// Runs the control CLI and returns its raw stdout, for subcommands whose output is
    /// human-readable text rather than JSON (`pending`, `approve`, `deny`).
    fn run_control_raw(socket_path: &Path, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--socket-path")
            .arg(socket_path)
            .args(args)
            .output()
            .expect("run proton-sync");
        assert!(
            output.status.success(),
            "proton-sync {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn write_fake_proton_drive(directory: &Path, remote_root: &str) -> PathBuf {
        let path = directory.join("fake-proton-drive");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "{remote_root}" ]; then
  printf '{{"entries":[]}}\n'
  exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
        .expect("fake proton-drive script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }

    /// A fake `proton-drive` for the delete-approval flow: `list` always reports one downloadable
    /// file `keep.txt` (with the SHA-1 of the bytes `download` writes, so the baseline matches the
    /// remote and a removed local copy plans a *RemoteDelete*), `download` writes those bytes, and
    /// `trash` appends the trashed path to `<script>.trash` for the test to observe.
    fn write_delete_approval_proton_drive(directory: &Path) -> PathBuf {
        let path = directory.join("fake-delete-approval-proton-drive");
        // SHA-1 of the literal bytes "hello" (what `download` writes below).
        let hello_sha1 = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
  printf '{{"entries":[{{"id":"remote-keep","name":"keep.txt","activeRevision":{{"claimedDigests":{{"sha1":"{hello_sha1}"}}}}}}]}}\n'
  exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "download" ]; then
  # $3 = remote path, $4 = scratch directory; name the file after the remote basename.
  printf 'hello' > "$4/$(basename "$3")"
  exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "trash" ]; then
  printf 'trash:%s\n' "$3" >> "$0.trash"
  exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
        .expect("fake proton-drive script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }

    fn write_failing_upload_proton_drive(directory: &Path, remote_root: &str) -> PathBuf {
        let path = directory.join("fake-failing-upload-proton-drive");
        fs::write(
                        &path,
                        format!(
                                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "{remote_root}" ]; then
    printf '{{"entries":[]}}\n'
    exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "upload" ]; then
    printf 'upload:%s:%s\n' "$5" "$6" >> "$0.args"
    if [ "$(basename "$5")" = "second.txt" ]; then
        echo "simulated interrupted upload" >&2
        exit 130
    fi
    exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
                        ),
                )
                .expect("fake proton-drive script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }

    fn write_blocking_upload_proton_drive(directory: &Path, remote_root: &str) -> PathBuf {
        let path = directory.join("fake-blocking-upload-proton-drive");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "{remote_root}" ]; then
    printf '{{"entries":[]}}\n'
    exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "upload" ]; then
    touch "$0.started"
    while [ ! -f "$0.release" ]; do
        sleep 0.05
    done
    exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
        .expect("fake proton-drive script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }
}
