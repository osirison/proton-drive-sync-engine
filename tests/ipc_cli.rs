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
        wait_for_socket(&socket_path, &mut daemon);

        // The control socket answers during the startup reconcile now, so wait for that first
        // pass to complete before asserting on its results.
        let status = wait_for_reconcile_seq(&socket_path, &mut daemon, 1);
        assert_eq!(status["status"], "running");
        assert_eq!(status["paused"], false);
        assert!(status["last_error"].is_null());
        // The startup reconcile ran against empty local and remote roots, so an empty plan and a
        // matching successful-sync summary are already present before any manual syncnow.
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

        // `syncnow` acks immediately and the CLI watches status until the scheduled pass
        // finishes, so the final `--json` payload is the post-sync status.
        let synced = run_control(&socket_path, "syncnow");
        assert_eq!(synced["status"], "running");
        assert_eq!(synced["syncing"], false);
        assert!(synced["reconcile_seq"].as_u64().unwrap_or(0) >= 2);
        assert!(synced["last_sync_epoch_secs"].as_u64().is_some());
        assert!(synced["last_error"].is_null());
        assert_eq!(synced["last_plan_summary"]["total"].as_u64(), Some(0));
        assert_eq!(
            synced["last_successful_sync_summary"]["total"].as_u64(),
            Some(0)
        );

        // `history --json` is the durable pass log, newest first — not the rolling
        // `status_history` trail (which records every pass, idle ones included, and holds about
        // ten minutes at the events poll cadence).
        let history = run_control(&socket_path, "history");
        let recent = history["recent"].as_array().expect("recent passes");
        // Two passes: the startup reconcile, then the manual syncnow after resume. (The syncnow
        // issued while paused is skipped without scheduling, so it runs no pass at all.) Both are
        // full-tree walks against an empty tree — recorded despite changing nothing, because
        // "when did the last full sweep run, and was anything out of step" is exactly what a full
        // sweep's row exists to answer (#238).
        assert_eq!(recent.len(), 2);
        for pass in recent {
            assert_eq!(pass["kind"], "full-sweep");
            assert_eq!(pass["outcome"], "clean");
            assert_eq!(pass["changed"].as_u64(), Some(0));
        }
        // Newest first.
        assert!(
            recent[0]["started_epoch_secs"].as_u64().unwrap()
                >= recent[1]["started_epoch_secs"].as_u64().unwrap()
        );
        assert_eq!(
            history["last_full_sweep"]["id"].as_i64(),
            recent[0]["id"].as_i64(),
            "the last full sweep is the most recent pass here"
        );
        // Nothing moved, so today's totals are zero rather than absent.
        assert_eq!(history["today"]["uploaded_bytes"].as_u64(), Some(0));
        assert_eq!(history["today"]["downloaded_bytes"].as_u64(), Some(0));

        // The per-file feed is a separate verb, and this run moved no files.
        let activity = run_control(&socket_path, "activity");
        assert_eq!(activity["total"].as_u64(), Some(0));
        assert_eq!(activity["files"].as_u64(), Some(0));
        assert!(activity["events"].as_array().expect("events").is_empty());
    }

    /// #63: a `socket_path` set in the daemon's config file used to be invisible to the control
    /// CLI, so every invocation had to repeat `--socket-path`. Both ends now read the same file.
    #[test]
    fn the_control_cli_finds_a_file_configured_socket_without_repeating_the_flag() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let fake_proton_drive = write_fake_proton_drive(directory.path(), "/Drive/RemoteFolder");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            format!(
                "local_root = \"{}\"\nremote_root = \"/Drive/RemoteFolder\"\n\
                 socket_path = \"{}\"\nlockfile_path = \"{}\"\ndb_path = \"{}\"\n\
                 proton_cli = \"{}\"\nscan_interval_secs = 60\nevents_driven = false\n",
                local_root.display(),
                socket_path.display(),
                directory.path().join("daemon.lock").display(),
                directory.path().join("sync_index.db").display(),
                fake_proton_drive.display(),
            ),
        )
        .expect("write config");

        // This daemon is configured entirely from the file, so it cannot go through
        // `DaemonProcess::spawn_with_args` — but it captures its stderr the same way, so a
        // timeout here can still say what the daemon was complaining about.
        let stderr_path = directory.path().join("daemon.stderr");
        let stderr_file = fs::File::create(&stderr_path).expect("create daemon stderr log");
        let child = Command::new(env!("CARGO_BIN_EXE_proton-syncd"))
            .arg("--config")
            .arg(&config_path)
            .env("XDG_STATE_HOME", directory.path())
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .expect("spawn proton-syncd");
        let mut daemon = DaemonProcess { child, stderr_path };
        wait_for_socket(&socket_path, &mut daemon);

        // An empty XDG_RUNTIME_DIR, so the default socket path resolves somewhere the daemon is
        // NOT listening: only the config file can produce a successful round trip here.
        let empty_runtime_dir = directory.path().join("runtime");
        fs::create_dir(&empty_runtime_dir).expect("runtime dir");
        let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--config")
            .arg(&config_path)
            .arg("--json")
            .arg("status")
            .env("XDG_RUNTIME_DIR", &empty_runtime_dir)
            .output()
            .expect("run proton-sync");

        assert!(
            output.status.success(),
            "proton-sync --config status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status: Value = serde_json::from_slice(&output.stdout).expect("control response JSON");
        // What this test is about is WHICH daemon the reply came from, so it asserts that — the
        // resolved local root, which only this daemon can report. It deliberately does not pin
        // `status`: the socket is bound before the startup reconcile finishes, so `syncing` is a
        // correct answer here and pinning `running` made the test race its own daemon.
        assert_eq!(
            status["config"]["local_root"],
            Value::String(local_root.display().to_string()),
            "the reply came from the daemon this config names"
        );
        assert!(
            ["running", "syncing"].contains(&status["status"].as_str().unwrap_or_default()),
            "an unpaused daemon reports one of the two live states: {}",
            status["status"]
        );

        // Without --config the same invocation looks in $XDG_RUNTIME_DIR and finds nothing, which
        // is what made the flag necessary.
        let without_config = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--json")
            .arg("status")
            .env("XDG_RUNTIME_DIR", &empty_runtime_dir)
            .output()
            .expect("run proton-sync");
        assert!(
            !without_config.status.success(),
            "the default socket path must not reach this daemon, or the test proves nothing"
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
        wait_for_socket(&socket_path, &mut daemon);

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

        // Wait past the startup reconcile so the asserted status is the settled "running", not a
        // transient "syncing".
        let status = wait_for_reconcile_seq(&socket_path, &mut daemon, 1);
        assert_eq!(status["status"], "running");
    }

    #[test]
    fn failed_upload_syncnow_commits_only_the_completed_uploads() {
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
        wait_for_socket(&socket_path, &mut daemon);

        // The watched pass ends with a failed item, so `syncnow --json` exits non-zero while
        // still printing the final status payload.
        let synced = run_control_any_exit(&socket_path, "syncnow");

        assert_eq!(synced["status"], "running");
        assert_eq!(synced["syncing"], false);
        // #136: the pass is reported as partial — a summary in `last_error` (so every older
        // client still sees a problem) and the per-item detail in `failed_items`.
        assert_eq!(synced["failed_item_count"], 1, "{synced}");
        assert_eq!(synced["failed_items"][0]["path"], "second.txt", "{synced}");
        assert_eq!(synced["failed_items"][0]["action"], "upload", "{synced}");
        assert!(
            synced["failed_items"][0]["error"]
                .as_str()
                .unwrap_or_default()
                .contains("proton-drive upload failed"),
            "daemon should expose the upload failure per item: {synced}"
        );
        assert!(
            synced["last_error"]
                .as_str()
                .unwrap_or_default()
                .contains("1 item(s) failed to sync"),
            "daemon should summarise the partial pass in last_error: {synced}"
        );
        let index = load_existing_index(&db_path).expect("load index after failed upload");
        assert!(
            index.contains_key(std::path::Path::new("first.txt")),
            "the upload that completed before the failure must be checkpoint-committed: {index:?}"
        );
        assert!(
            !index.contains_key(std::path::Path::new("second.txt")),
            "the failed upload must never be recorded: {index:?}"
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
        wait_for_socket(&socket_path, &mut daemon);
        let pid = daemon.child.id();

        // The startup reconcile's upload is now the blocked call; its marker proves we are wedged
        // inside the (interruptible) reconcile before the signal is sent.
        let started_marker = PathBuf::from(format!("{}.started", fake_proton_drive.display()));
        wait_for_marker(&started_marker, &mut daemon);

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

        // The lockfile is deliberately left behind (#13 — unlinking it re-opens the
        // flock-over-unlink race); what a clean shutdown must guarantee is that the flock is
        // RELEASED, i.e. the next start can lock the very same inode.
        assert!(
            lockfile_path.exists(),
            "lockfile must persist after shutdown so the next start contends on the same inode"
        );
        let lockfile = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lockfile_path)
            .expect("reopen lockfile");
        fs2::FileExt::try_lock_exclusive(&lockfile)
            .expect("a clean shutdown must release the flock on the leftover lockfile");
    }

    // The core responsiveness guarantee behind the concurrent control-socket task: a status
    // request issued while a reconcile is blocked mid-transfer must be answered immediately
    // (reporting `syncing`), not queue behind the reconcile. Before the IPC task existed, this
    // exact scenario froze every CLI/GUI status call for the duration of the sync.
    #[test]
    fn status_is_answered_while_a_sync_is_blocked_mid_transfer() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("blocking.txt"), b"content").expect("write fixture");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive =
            write_blocking_upload_proton_drive(directory.path(), "/Drive/RemoteFolder");

        let mut daemon = DaemonProcess::spawn_with_proton_timeout(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
            30,
        );
        wait_for_socket(&socket_path, &mut daemon);

        // The startup reconcile is now wedged inside the fake's endless upload.
        let started_marker = PathBuf::from(format!("{}.started", fake_proton_drive.display()));
        wait_for_marker(&started_marker, &mut daemon);

        let asked = Instant::now();
        let status = run_control(&socket_path, "status");
        let elapsed = asked.elapsed();

        assert!(
            elapsed < Duration::from_secs(5),
            "status must be answered while a sync is in flight; took {elapsed:?}"
        );
        assert_eq!(status["status"], "syncing");
        assert_eq!(status["syncing"], true);
        assert_eq!(status["paused"], false);
        // The in-flight pass's plan is already published: one upload.
        assert_eq!(status["last_plan_summary"]["uploads"].as_u64(), Some(1));
        // And the live activity names the wedged transfer itself — the whole point of the
        // field is that a blocked pass still reports what it is doing right now.
        assert_eq!(status["activity"]["phase"], "executing");
        assert_eq!(status["activity"]["transfer"]["direction"], "upload");
        assert_eq!(status["activity"]["transfer"]["path"], "blocking.txt");
        assert_eq!(
            status["activity"]["transfer"]["bytes_total"].as_u64(),
            Some(b"content".len() as u64),
            "an upload's total is its local file size"
        );

        // A pause is accepted mid-sync too, and takes effect for the *next* pass.
        let paused = run_control(&socket_path, "pause");
        assert_eq!(paused["paused"], true);

        // Release the blocked upload so the daemon can wind down cleanly.
        fs::write(format!("{}.release", fake_proton_drive.display()), b"go").expect("release");
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
        wait_for_socket(&socket_path, &mut daemon);

        // The startup reconcile downloads keep.txt and records a synced baseline. Wait for the
        // BASELINE, not the file: without the record there is nothing to plan a RemoteDelete
        // against and the pass below plans a plain Download instead (#327).
        let local_file = local_root.join("keep.txt");
        let settled =
            wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        let seq_before = settled["reconcile_seq"].as_u64().expect("reconcile_seq");

        // Remove the local copy: the next reconcile plans a RemoteDelete (local gone, remote still
        // present, baseline unchanged). The guard (on by default) must withhold it.
        fs::remove_file(&local_file).expect("remove local file");
        let trash_marker = PathBuf::from(format!("{}.trash", fake_proton_drive.display()));

        // No pass was in flight at `seq_before` (the wait above returns only when idle), so any
        // pass that reaches `seq_before + 1` scanned the tree after this removal — the ordering is
        // by construction rather than by the client's own `+ 1` / `+ 2` arithmetic, which is taken
        // against whatever instant the ack happened to land in.
        run_control_args(&socket_path, &["--json", "syncnow", "--no-wait"]);
        let withheld = wait_for_reconcile_seq(&socket_path, &mut daemon, seq_before + 1);
        assert!(
            withheld["last_error"].is_null(),
            "pass should succeed: {withheld}"
        );
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
        assert!(
            applied["last_error"].is_null(),
            "pass should succeed: {applied}"
        );
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

    #[test]
    fn keeping_a_withheld_deletion_restores_the_other_side_over_ipc() {
        // #224. `deny` only revokes an approval, so refusing a deletion used to be nothing at all:
        // the planner re-derived the same withheld action every pass and the row came back at the
        // next launch. `keep` purges the baseline record, and the surviving remote copy is adopted
        // back onto this computer by the pass the command schedules.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_delete_approval_proton_drive(directory.path());
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon);

        // The startup reconcile downloads keep.txt and records a synced baseline; removing the
        // local copy makes the next pass plan a RemoteDelete, which the guard withholds. Waiting
        // for the baseline rather than the file is what makes that first clause true (#327).
        let local_file = local_root.join("keep.txt");
        let settled =
            wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        let seq_before = settled["reconcile_seq"].as_u64().expect("reconcile_seq");
        fs::remove_file(&local_file).expect("remove local file");
        let trash_marker = PathBuf::from(format!("{}.trash", fake_proton_drive.display()));

        // Ordered by construction: nothing was in flight at `seq_before`, so the pass that reaches
        // `seq_before + 1` scanned the tree after the removal.
        run_control_args(&socket_path, &["--json", "syncnow", "--no-wait"]);
        let withheld = wait_for_reconcile_seq(&socket_path, &mut daemon, seq_before + 1);
        let pending = withheld["pending_deletions"]
            .as_array()
            .expect("pending_deletions array");
        assert_eq!(
            pending.len(),
            1,
            "the remote delete is withheld: {withheld}"
        );
        // The age is the deletion's own, carried across passes (#225), and a real epoch rather
        // than the zero an older daemon would leave.
        assert!(
            pending[0]["first_seen_epoch_secs"].as_u64().unwrap_or(0) > 0,
            "a withheld deletion reports when it was first seen: {withheld}"
        );

        // Keep it: the local copy comes back and the remote is never trashed. The re-adoption is
        // a fresh download plus a fresh baseline row, so wait for both — the file alone would let
        // the assertions below read the pass that is still landing it.
        let kept = run_control_raw(&socket_path, &["keep", "keep.txt"]);
        assert!(kept.contains("kept 1"), "keep should confirm: {kept}");
        let restored =
            wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        let seq_restored = restored["reconcile_seq"].as_u64().expect("reconcile_seq");
        assert!(
            !trash_marker.exists(),
            "keeping must never delete the surviving copy"
        );

        // And it is durable: nothing is pending any more, because the planner no longer derives
        // the deletion at all.
        run_control_args(&socket_path, &["--json", "syncnow", "--no-wait"]);
        let after = wait_for_reconcile_seq(&socket_path, &mut daemon, seq_restored + 1);
        assert!(
            after["pending_deletions"]
                .as_array()
                .expect("pending array")
                .is_empty(),
            "a kept deletion does not come back on the next pass: {after}"
        );
        assert!(
            load_existing_index(&db_path)
                .expect("load index")
                .contains_key(Path::new("keep.txt")),
            "the restored file is tracked again, as a fresh copy"
        );
    }

    /// #327, deterministically: the startup pass lands `keep.txt` on disk and then **fails** the
    /// action, so the file is there with no baseline row behind it. Removing the local copy at
    /// that point plans a fresh `Download`, not the `RemoteDelete` the delete-approval tests are
    /// about — which is how the racing CI run read `pending_deletions: []`.
    ///
    /// This is the guard for the wait: a test that mutates the tree must wait for the *baseline*,
    /// not for the file.
    #[test]
    fn a_partial_startup_pass_still_withholds_the_delete_it_should() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_landing_then_failing_download_proton_drive(directory.path());
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon);

        let local_file = local_root.join("keep.txt");
        let settled =
            wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        let seq_before = settled["reconcile_seq"].as_u64().expect("reconcile_seq");

        // The forcing really happened: the startup pass is on the record as `partial`. Without
        // this the fake could quietly become an ordinary one and the test would still pass, having
        // stopped exercising the state it exists for.
        let history = run_control(&socket_path, "history");
        let recent = history["recent"].as_array().expect("recent passes");
        assert!(
            recent
                .iter()
                .any(|pass| pass["outcome"] == "partial" && pass["failed"].as_u64() == Some(1)),
            "the startup pass must have failed its download: {history}"
        );
        // …and the bytes on disk are the ones that failed pass's own: the CLI was asked to
        // download exactly once, so nothing re-fetched them. That is the state the file cannot
        // distinguish and the baseline row can — the second pass adopted what was already there.
        let downloads = fs::read_to_string(format!("{}.downloads", fake_proton_drive.display()))
            .unwrap_or_else(|error| {
                panic!("the fake must have recorded its download attempts: {error}")
            });
        assert_eq!(
            downloads.lines().count(),
            1,
            "exactly one download was attempted, and it failed: {downloads}"
        );
        assert!(local_file.exists(), "the bytes are still on disk");

        fs::remove_file(&local_file).expect("remove local file");
        let trash_marker = PathBuf::from(format!("{}.trash", fake_proton_drive.display()));

        run_control_args(&socket_path, &["--json", "syncnow", "--no-wait"]);
        let withheld = wait_for_reconcile_seq(&socket_path, &mut daemon, seq_before + 1);
        let pending = withheld["pending_deletions"]
            .as_array()
            .expect("pending_deletions array");
        assert_eq!(
            pending.len(),
            1,
            "the remote delete is withheld: {withheld}"
        );
        assert_eq!(pending[0]["direction"], "remote");
        assert_eq!(pending[0]["path"], "keep.txt");
        assert!(
            !trash_marker.exists(),
            "no remote trash may happen before approval"
        );
    }

    /// #99: the read-only `list` verb, end to end through the real daemon, the real socket and
    /// the real `proton-sync` client — the layer where "the GUI shells the CLI itself" is actually
    /// replaced.
    #[test]
    fn the_list_verb_browses_the_remote_through_the_daemon() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_browsable_proton_drive(directory.path());
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon);
        wait_for_reconcile_seq(&socket_path, &mut daemon, 1);

        // No path argument: the remote root, which is the legitimate empty-selector case.
        let root = run_control_args(&socket_path, &["--json", "list"]);
        assert_eq!(root["state"], "listed");
        assert_eq!(root["path"], "");
        assert_eq!(root["total"].as_u64(), Some(2));
        assert_eq!(root["truncated"], false);
        let names: Vec<&str> = root["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["name"].as_str().expect("name"))
            .collect();
        // Directories first, then by name — and the root itself is not inside itself.
        assert_eq!(names, vec!["photos", "notes.txt"]);
        assert_eq!(root["entries"][0]["entity_kind"], "directory");
        assert_eq!(root["entries"][1]["entity_kind"], "file");
        assert_eq!(root["entries"][1]["downloadable"], true);

        // A subfolder lists its own contents.
        let photos = run_control_args(&socket_path, &["--json", "list", "photos"]);
        assert_eq!(photos["state"], "listed");
        assert_eq!(photos["path"], "photos");
        assert_eq!(photos["entries"][0]["name"], "beach.jpg");
        assert_eq!(photos["entries"][0]["path"], "photos/beach.jpg");

        // The daemon reached Proton, so `status` reports the session as usable — evidence, not a
        // default (#103).
        let status = run_control(&socket_path, "status");
        assert_eq!(status["auth"], "signed-in");
        // …and every other verb omits the listing rather than implying an empty folder.
        assert!(status["listing"].is_null());

        // A selector that escapes the root is refused before it is joined, and the CLI exits
        // non-zero so a script never mistakes a refusal for an empty folder.
        let escape = run_control_args_any_exit(&socket_path, &["--json", "list", "../etc"]);
        assert_eq!(escape.0["state"], "failed");
        assert!(
            escape.0["error"]
                .as_str()
                .expect("error")
                .contains("unsafe remote path"),
            "{escape:?}"
        );
        assert!(!escape.1, "a refused listing must exit non-zero");
    }

    /// #103: an auth failure is classified by the engine and published as an explicit state, so a
    /// UI never has to pattern-match the error string. Also the other half of the pair — the
    /// classification is what a *listing* reports when it is refused.
    #[test]
    fn an_expired_session_is_classified_and_published_as_an_auth_state() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_signed_out_proton_drive(directory.path());
        let mut daemon = DaemonProcess::spawn(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
        );
        wait_for_socket(&socket_path, &mut daemon);
        wait_for_reconcile_seq(&socket_path, &mut daemon, 1);

        let status = run_control(&socket_path, "status");
        assert_eq!(
            status["auth"], "signed-out",
            "the daemon classifies the CLI's own refusal rather than leaving it to the client"
        );
        // The error text is still there for humans; it is simply no longer the thing a client has
        // to parse to know what happened.
        assert!(
            status["last_error"]
                .as_str()
                .expect("last_error")
                .contains("401"),
            "{status}"
        );

        // The human output names it too, with the action that fixes it.
        let human = run_control_raw(&socket_path, &["status"]);
        assert!(human.contains("proton-drive login"), "{human}");

        // A listing refused for the same reason reports itself as failed, not as empty.
        let (listing, success) = run_control_args_any_exit(&socket_path, &["--json", "list"]);
        assert_eq!(listing["state"], "failed");
        assert!(!success);
    }

    /// A fake `proton-drive` with a browsable two-level tree: `notes.txt` and `photos/beach.jpg`,
    /// both downloadable so the daemon's own bootstrap pass completes cleanly.
    fn write_browsable_proton_drive(directory: &Path) -> PathBuf {
        // SHA-1 of the literal bytes "hello" (what `download` writes below).
        let hello_sha1 = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        write_script(
            directory,
            "fake-browsable-proton-drive",
            &format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
  case "$4" in
    */RemoteFolder/photos)
      printf '{{"id":"remote-photos","name":"photos","path":"/Drive/RemoteFolder/photos","type":"folder","entries":[{{"id":"remote-beach","name":"beach.jpg","path":"/Drive/RemoteFolder/photos/beach.jpg","activeRevision":{{"claimedDigests":{{"sha1":"{hello_sha1}"}}}}}}]}}\n'
      ;;
    */RemoteFolder)
      printf '{{"entries":[{{"id":"remote-notes","name":"notes.txt","path":"/Drive/RemoteFolder/notes.txt","activeRevision":{{"claimedDigests":{{"sha1":"{hello_sha1}"}}}}}},{{"id":"remote-photos","name":"photos","path":"/Drive/RemoteFolder/photos","type":"folder","entries":[]}}]}}\n'
      ;;
    *)
      echo "unexpected list target: $4" >&2
      exit 64
      ;;
  esac
  exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "download" ]; then
  shift 2
  for argument in "$@"; do scratch="$argument"; done
  for argument in "$@"; do
    if [ "$argument" != "$scratch" ]; then
      printf 'hello' > "$scratch/$(basename "$argument")"
    fi
  done
  exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
    }

    /// #100/#192/#209, end to end through the real daemon, the real socket and the real
    /// `proton-sync` client — the layer where "the GUI shells `proton-syncd --dry-run` itself" is
    /// actually replaced (#317's on-demand instance).
    #[test]
    fn the_plan_verb_reviews_and_the_token_applies_that_exact_plan() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_delete_approval_proton_drive(directory.path());
        // The guard off, so the plan's deletion is a row the apply really executes — with it on,
        // an ordinary apply and a filtered one would both leave the file alone and the test would
        // prove nothing about `--skip-destructive`.
        let mut daemon = DaemonProcess::spawn_with_args(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
            &["--no-delete-approval"],
        );
        wait_for_socket(&socket_path, &mut daemon);
        // The startup reconcile downloads keep.txt and records a synced baseline. The baseline is
        // the precondition — without the row the plan below is a Download, not a RemoteDelete
        // (#327) — and a pass that has ended is what makes the sequence assertion below mean
        // anything.
        let local_file = local_root.join("keep.txt");
        let settled =
            wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        let seq_before = settled["reconcile_seq"].as_u64().expect("reconcile_seq");

        // Remove the local copy: the plan is now one RemoteDelete.
        fs::remove_file(&local_file).expect("remove local file");
        let trash_marker = PathBuf::from(format!("{}.trash", fake_proton_drive.display()));

        let plan = run_control_args(&socket_path, &["--json", "plan"]);
        assert_eq!(plan["state"], "computed");
        assert_eq!(plan["total"].as_u64(), Some(1), "{plan}");
        assert_eq!(plan["actions"][0]["action"], "remote_delete");
        assert_eq!(plan["actions"][0]["path"], "keep.txt");
        assert_eq!(plan["summary"]["destructive_actions"].as_u64(), Some(1));
        let token = plan["token"].as_str().expect("token").to_owned();
        assert!(token.starts_with("1:"), "{token}");

        // A rehearsal changes nothing: it did not trash anything, and it did not move the
        // sequence a `syncnow` watcher polls.
        assert!(
            !trash_marker.exists(),
            "a rehearsal must perform no side effect"
        );
        let status = run_control(&socket_path, "status");
        // Against the sequence the startup wait ended on, not a literal `1`: how many passes it
        // took to reach a synced baseline is the daemon's business, and this assertion is about
        // the rehearsal adding none of them.
        assert_eq!(
            status["reconcile_seq"].as_u64(),
            Some(seq_before),
            "a plan pass must not bump the reconcile sequence a syncnow watcher polls: {status}"
        );

        // A token that is not the current plan's authorises nothing, and schedules nothing.
        let (stale, ok) =
            run_control_args_any_exit(&socket_path, &["--json", "apply", "1:not-a-real-plan"]);
        assert_eq!(stale["state"], "stale");
        assert!(!ok, "a refused apply must exit non-zero");
        assert!(!trash_marker.exists(), "a stale token must run nothing");

        // The real token applies that exact plan.
        let applied = run_control_args(&socket_path, &["--json", "apply", &token]);
        assert_eq!(applied["state"], "applied", "{applied}");
        assert_eq!(applied["executed"].as_u64(), Some(1));
        assert_eq!(applied["skipped_destructive"].as_u64(), Some(0));
        assert_eq!(applied["failed"].as_u64(), Some(0));
        assert!(
            trash_marker.exists(),
            "the reviewed deletion must have reached the remote"
        );
        // And the token is spent: the plan it named is no longer the current one.
        let (stale, ok) = run_control_args_any_exit(&socket_path, &["--json", "plan"]);
        assert_eq!(stale["state"], "computed");
        assert_ne!(stale["token"].as_str(), Some(token.as_str()));
        assert!(ok);
    }

    /// #192: `Run it without the deletion`. The plan holds a deletion, the apply is asked to skip
    /// it, and both copies survive.
    #[test]
    fn a_filtered_apply_runs_the_plan_without_its_deletions() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_delete_approval_proton_drive(directory.path());
        let mut daemon = DaemonProcess::spawn_with_args(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
            &["--no-delete-approval"],
        );
        wait_for_socket(&socket_path, &mut daemon);
        let local_file = local_root.join("keep.txt");
        wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        fs::remove_file(&local_file).expect("remove local file");
        let trash_marker = PathBuf::from(format!("{}.trash", fake_proton_drive.display()));

        let plan = run_control_args(&socket_path, &["--json", "plan"]);
        assert_eq!(
            plan["summary"]["destructive_actions"].as_u64(),
            Some(1),
            "precondition: the plan holds exactly one destructive row: {plan}"
        );
        let token = plan["token"].as_str().expect("token").to_owned();

        let applied = run_control_args(
            &socket_path,
            &["--json", "apply", &token, "--skip-destructive"],
        );
        assert_eq!(applied["state"], "applied", "{applied}");
        assert_eq!(applied["skipped_destructive"].as_u64(), Some(1));
        assert_eq!(applied["executed"].as_u64(), Some(0));
        assert!(
            !trash_marker.exists(),
            "a filtered apply must issue no remote delete, guard or no guard"
        );
        // The deletion re-plans: nothing about it was consumed.
        let again = run_control_args(&socket_path, &["--json", "plan"]);
        assert_eq!(again["summary"]["destructive_actions"].as_u64(), Some(1));
    }

    /// A fake `proton-drive` whose every command fails the way an expired session does.
    fn write_signed_out_proton_drive(directory: &Path) -> PathBuf {
        write_script(
            directory,
            "fake-signed-out-proton-drive",
            "#!/bin/sh\necho 'Error: request failed: 401 Unauthorized' >&2\nexit 1\n",
        )
    }

    fn write_script(directory: &Path, name: &str, content: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, content).expect("write fake proton-drive");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }

    /// `proton-sync <args...> --json`, parsed. Unlike `run_control` this takes the whole argument
    /// vector, so a subcommand with its own positional argument (`list photos`) can be driven.
    fn run_control_args(socket_path: &Path, args: &[&str]) -> Value {
        let (value, success) = run_control_args_any_exit(socket_path, args);
        assert!(success, "proton-sync {args:?} exited non-zero: {value}");
        value
    }

    /// As `run_control_args`, but reports the exit status instead of asserting on it: `list`
    /// deliberately exits non-zero when nothing was listed, so a script can branch on the code
    /// rather than on the payload.
    fn run_control_args_any_exit(socket_path: &Path, args: &[&str]) -> (Value, bool) {
        let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--socket-path")
            .arg(socket_path)
            .args(args)
            .output()
            .expect("run proton-sync");
        let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "proton-sync {args:?} did not print JSON ({error}); stdout: {}; stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (value, output.status.success())
    }

    struct DaemonProcess {
        child: Child,
        /// Where this daemon's stderr is captured. It used to be `Stdio::null()`, which is why
        /// the CI run behind #327 kept no evidence of *why* its startup pass failed its download
        /// — the one line that would have explained it. Every wait helper's timeout panic tails
        /// this file, and `RUST_LOG` is `warn` rather than `error` because a failed action is
        /// reported at `warn` (`PassFailures::record` in `src/daemon.rs`).
        stderr_path: PathBuf,
    }

    impl DaemonProcess {
        fn spawn(
            local_root: &Path,
            socket_path: &Path,
            lockfile_path: &Path,
            db_path: &Path,
            proton_cli: &Path,
        ) -> Self {
            Self::spawn_with_args(
                local_root,
                socket_path,
                lockfile_path,
                db_path,
                proton_cli,
                &[],
            )
        }

        fn spawn_with_proton_timeout(
            local_root: &Path,
            socket_path: &Path,
            lockfile_path: &Path,
            db_path: &Path,
            proton_cli: &Path,
            proton_timeout_secs: u64,
        ) -> Self {
            let proton_timeout_secs = proton_timeout_secs.to_string();
            Self::spawn_with_args(
                local_root,
                socket_path,
                lockfile_path,
                db_path,
                proton_cli,
                &["--proton-timeout-secs", &proton_timeout_secs],
            )
        }

        /// The one spawn body: every daemon these tests start differs only by extra flags, so
        /// three near-identical copies of the argument list meant three places to keep the
        /// isolation environment and the log capture in step.
        fn spawn_with_args(
            local_root: &Path,
            socket_path: &Path,
            lockfile_path: &Path,
            db_path: &Path,
            proton_cli: &Path,
            extra_args: &[&str],
        ) -> Self {
            let stderr_path = db_path
                .parent()
                .expect("db path has a parent dir")
                .join("daemon.stderr");
            let stderr_file = fs::File::create(&stderr_path).expect("create daemon stderr log");
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
                .args(extra_args)
                // Isolate the user-global single-instance lock per test: `default_global_lock_path`
                // keys on `$XDG_STATE_HOME`, so pointing it at this test's tempdir stops parallel
                // ipc_cli daemons contending on one machine-global lock (they would else exit 1,
                // and a real proton-syncd on this machine would win — #77).
                .env(
                    "XDG_STATE_HOME",
                    lockfile_path.parent().expect("lockfile has a parent dir"),
                )
                .env("RUST_LOG", "warn")
                .stdout(Stdio::null())
                .stderr(Stdio::from(stderr_file))
                .spawn()
                .expect("spawn proton-syncd");
            Self { child, stderr_path }
        }

        /// The tail of this daemon's log, for a wait helper that is about to panic. The tempdir is
        /// removed as the test unwinds, so a helper that does not quote the log leaves nothing
        /// behind to read.
        fn stderr_tail(&self) -> String {
            match fs::read_to_string(&self.stderr_path) {
                Ok(log) if log.trim().is_empty() => "daemon stderr: <empty>".to_owned(),
                Ok(log) => {
                    let mut lines: Vec<&str> = log.lines().rev().take(20).collect();
                    lines.reverse();
                    format!(
                        "daemon stderr (last {} lines):\n{}",
                        lines.len(),
                        lines.join("\n")
                    )
                }
                Err(error) => format!(
                    "daemon stderr unreadable at {}: {error}",
                    self.stderr_path.display()
                ),
            }
        }
    }

    impl Drop for DaemonProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn wait_for_socket(socket_path: &Path, daemon: &mut DaemonProcess) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if socket_path.exists() {
                return;
            }
            if let Some(status) = daemon.child.try_wait().expect("daemon status") {
                panic!(
                    "proton-syncd exited before binding socket: {status}\n{}",
                    daemon.stderr_tail()
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "timed out waiting for daemon socket at {}\n{}",
            socket_path.display(),
            daemon.stderr_tail()
        );
    }

    /// Waits for a file to appear, and for nothing else.
    ///
    /// Only for the fake CLI's `.started` marker, where the daemon is deliberately **wedged**
    /// mid-transfer for the rest of the test: `syncing` never goes false there and no baseline is
    /// ever written, so any condition about the pass would hang. A test that goes on to mutate the
    /// local tree wants `wait_for_synced_baseline` instead — see its doc for what waiting on the
    /// file alone cost (#327).
    fn wait_for_marker(marker_path: &Path, daemon: &mut DaemonProcess) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if marker_path.exists() {
                return;
            }
            if let Some(status) = daemon.child.try_wait().expect("daemon status") {
                panic!(
                    "proton-syncd exited before reaching the expected marker: {status}\n{}",
                    daemon.stderr_tail()
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "timed out waiting for marker file at {}\n{}",
            marker_path.display(),
            daemon.stderr_tail()
        );
    }

    /// Waits until the daemon has landed `<local_root>/<relative>` on disk **and** recorded a
    /// baseline row for it, with no pass in flight — the precondition every test that then mutates
    /// the tree actually depends on. Returns that status reply, whose `reconcile_seq` is a count
    /// no pass was in flight at.
    ///
    /// #327: waiting for the file is not waiting for the pass, twice over. A download lands by
    /// `fs::rename` out of its staging directory *before* the checkpoint that records it, so the
    /// file can appear mid-pass; and a pass that lands the bytes and then fails the action leaves
    /// the file on disk with **no** baseline row at all and `is_first_reconcile` still set. Remove
    /// the local copy in that state and the next pass plans a fresh `Download` — there is no
    /// baseline to derive a `RemoteDelete` from — so a test asserting on a withheld deletion reads
    /// `pending_deletions: []` and blames the delete gate. Waiting for the baseline is what makes
    /// the precondition true rather than likely.
    ///
    /// It **asks** for a pass rather than waiting one out: nothing here reschedules on its own —
    /// filesystem-watch events only accumulate `pending_changes` (see the `select!` loop in
    /// `src/daemon.rs`), and `--scan-interval-secs 60` outlives the test — so a startup pass that
    /// failed its download would otherwise leave the baseline missing for ever.
    fn wait_for_synced_baseline(
        socket_path: &Path,
        daemon: &mut DaemonProcess,
        db_path: &Path,
        local_root: &Path,
        relative: &str,
    ) -> Value {
        let marker_path = local_root.join(relative);
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut last_nudge: Option<Instant> = None;
        let mut last_status = Value::Null;
        while Instant::now() < deadline {
            if let Some(status) = daemon.child.try_wait().expect("daemon status") {
                panic!(
                    "proton-syncd exited before recording a baseline for {relative}: {status}\n{}",
                    daemon.stderr_tail()
                );
            }
            let status = run_control(socket_path, "status");
            let idle = !status["syncing"].as_bool().unwrap_or(false);
            // An unreadable index is "not yet", never a failure: the daemon holds the same
            // database open, so a poll can land on one of its write transactions.
            let recorded = load_existing_index(db_path)
                .map(|index| index.contains_key(Path::new(relative)))
                .unwrap_or(false);
            last_status = status;
            if idle && recorded && marker_path.exists() {
                return last_status;
            }
            if idle
                && !recorded
                && last_nudge.is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
            {
                // `--no-wait`: this asks for a pass, it does not watch one. Watching would lean on
                // the client's `reconcile_seq + 1` / `+ 2` arithmetic, which is the other half of
                // the same race.
                run_control_args(socket_path, &["--json", "syncnow", "--no-wait"]);
                last_nudge = Some(Instant::now());
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "timed out waiting for a synced baseline for {relative} (the file is present: {}); \
             last status: {last_status}\n{}",
            marker_path.exists(),
            daemon.stderr_tail()
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

    /// Runs the control CLI with `--json` and parses the response. The human-readable output is
    /// the CLI's default now; these process-level tests assert on the machine-readable form.
    fn run_control(socket_path: &Path, command: &str) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--socket-path")
            .arg(socket_path)
            .arg("--json")
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

    /// As `run_control`, but tolerates a non-zero exit — `syncnow --json` exits 1 when the pass
    /// it watched failed, and some tests exercise exactly that.
    fn run_control_any_exit(socket_path: &Path, command: &str) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_proton-sync"))
            .arg("--socket-path")
            .arg(socket_path)
            .arg("--json")
            .arg(command)
            .output()
            .expect("run proton-sync");
        serde_json::from_slice(&output.stdout).expect("control response JSON")
    }

    /// Polls `status` until the daemon has completed at least `passes` reconcile attempts. The
    /// control socket now answers while a reconcile is in flight, so tests that assert on
    /// last-sync state must explicitly wait for the pass to finish instead of relying on the
    /// old accept-queue blocking.
    fn wait_for_reconcile_seq(
        socket_path: &Path,
        daemon: &mut DaemonProcess,
        passes: u64,
    ) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(status) = daemon.child.try_wait().expect("daemon status") {
                panic!(
                    "proton-syncd exited while waiting for a reconcile: {status}\n{}",
                    daemon.stderr_tail()
                );
            }
            let status = run_control(socket_path, "status");
            let seq = status["reconcile_seq"].as_u64().unwrap_or(0);
            let syncing = status["syncing"].as_bool().unwrap_or(false);
            if seq >= passes && !syncing {
                return status;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "timed out waiting for reconcile pass {passes}\n{}",
            daemon.stderr_tail()
        );
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

    /// `write_delete_approval_proton_drive`, except that the **first** download lands its bytes
    /// straight at their destination and then exits non-zero (#327).
    ///
    /// That is the exact state the racing CI run hit: `proton.rs` stages a download in a scratch
    /// directory *inside* the destination folder and moves it into place, so a CLI that writes to
    /// the destination itself and fails leaves the file on disk with the action failed — the pass
    /// ends `partial`, no baseline row is written, and `is_first_reconcile` stays set. A test that
    /// waits for the FILE then removes it therefore makes the next pass plan a fresh `Download`
    /// (there is no baseline to derive a `RemoteDelete` from) instead of the withheld deletion it
    /// is asserting on.
    ///
    /// The failure text is deliberately bland: `node not found` would type the error
    /// [`proton_drive_sync_engine::proton::NodeNotFound`] and make the executor *skip* the action,
    /// and any of the auth vocabulary would type it `AuthFailure` — either way the pass would not
    /// be the partial one this fake exists to force.
    fn write_landing_then_failing_download_proton_drive(directory: &Path) -> PathBuf {
        let path = directory.join("fake-landing-then-failing-proton-drive");
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
  # $3 = remote path, $4 = the scratch directory the daemon stages into, which lives inside the
  # destination folder — so "$(dirname "$4")" is the local root.
  printf 'download:%s\n' "$3" >> "$0.downloads"
  if [ ! -f "$0.first-download" ]; then
    : > "$0.first-download"
    printf 'hello' > "$(dirname "$4")/$(basename "$3")"
    echo "simulated transfer failure after the bytes landed" >&2
    exit 1
  fi
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
