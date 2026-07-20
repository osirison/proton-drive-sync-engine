#[cfg(unix)]
mod unix_tests {
    use proton_drive_sync_engine::index::load_existing_index;
    use serde_json::Value;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::mpsc;
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
        assert!(status["last_plan_summary"].is_null());
        assert!(status["last_successful_sync_summary"].is_null());

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
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["message"], "sync completed");
        assert_eq!(
            history[0]["successful_sync_summary"]["total"].as_u64(),
            Some(0)
        );
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
    // `block_in_place`, so a SIGINT that arrives while a proton-drive call is stuck
    // is only *observed* once that call returns control to the loop - it is not
    // acted on the instant the signal is delivered. This test proves the daemon
    // still reaches a clean, bounded shutdown once its own command timeout kills
    // the stuck CLI process, and that the interruption leaves no partial index
    // state and releases the lockfile.
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

        let (result_tx, result_rx) = mpsc::channel();
        let syncnow_socket_path = socket_path.clone();
        thread::spawn(move || {
            let _ = result_tx.send(run_control(&syncnow_socket_path, "syncnow"));
        });

        let started_marker = PathBuf::from(format!("{}.started", fake_proton_drive.display()));
        wait_for_marker(&started_marker, &mut daemon.child);

        let status = Command::new("kill")
            .arg("-INT")
            .arg(pid.to_string())
            .status()
            .expect("send SIGINT to daemon");
        assert!(status.success(), "kill -INT should succeed");

        let synced = result_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("syncnow should still receive a response once the blocked CLI call times out");
        assert!(
            synced["message"]
                .as_str()
                .unwrap_or_default()
                .contains("sync failed"),
            "an interrupted upload should be reported as a failed sync: {synced}"
        );

        let exit_status = wait_for_exit(&mut daemon.child, Duration::from_secs(5))
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
    printf 'upload:%s:%s\n' "$3" "$4" >> "$0.args"
    if [ "$(basename "$3")" = "second.txt" ]; then
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
