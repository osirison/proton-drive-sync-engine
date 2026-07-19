#[cfg(unix)]
mod unix_tests {
    use serde_json::Value;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
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
}
