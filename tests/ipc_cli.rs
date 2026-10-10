mod common;

#[cfg(unix)]
mod unix_tests {
    use crate::common;
    use proton_drive_sync_engine::index::load_existing_index;
    use serde_json::Value;
    use std::ffi::OsStr;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Output};
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
        // Resuming a paused pair queues a pass of its own (#456). It is waited for, so the
        // `syncnow` below is a pass of its own too and does not coalesce into this one while it is
        // still queued.
        wait_for_reconcile_seq(&socket_path, &mut daemon, 2);

        // `syncnow` acks immediately and the CLI watches status until the scheduled pass
        // finishes, so the final `--json` payload is the post-sync status.
        let synced = run_control(&socket_path, "syncnow");
        assert_eq!(synced["status"], "running");
        assert_eq!(synced["syncing"], false);
        assert!(synced["reconcile_seq"].as_u64().unwrap_or(0) >= 3);
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
        // Three passes: the startup reconcile, the one the resume queued, then the manual syncnow
        // after it. (The syncnow issued while paused is skipped without scheduling, so it runs no
        // pass at all.) All are full-tree walks against an empty tree — recorded despite changing
        // nothing, because "when did the last full sweep run, and was anything out of step" is
        // exactly what a full sweep's row exists to answer (#238).
        assert_eq!(recent.len(), 3);
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
        // `DaemonProcess::spawn_with_args` — but it starts through the same `start`, so it has the
        // same sandbox and a timeout here can still say what the daemon was complaining about.
        let mut daemon = DaemonProcess::start(
            directory.path(),
            [OsStr::new("--config"), config_path.as_os_str()],
        );
        wait_for_socket(&socket_path, &mut daemon);

        // An empty XDG_RUNTIME_DIR, so the default socket path resolves somewhere the daemon is
        // NOT listening: only the config file can produce a successful round trip here.
        let empty_runtime_dir = directory.path().join("runtime");
        fs::create_dir(&empty_runtime_dir).expect("runtime dir");
        let output = run_client(
            proton_sync(directory.path())
                .arg("--config")
                .arg(&config_path)
                .arg("--json")
                .arg("status")
                .env("XDG_RUNTIME_DIR", &empty_runtime_dir),
        );

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
        let without_config = run_client(
            proton_sync(directory.path())
                .arg("--json")
                .arg("status")
                .env("XDG_RUNTIME_DIR", &empty_runtime_dir),
        );
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
        let _reaper = KillBlockingUploadGroup {
            script: fake_proton_drive.clone(),
        };

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

        let sent = common::run_other_tool(Command::new("kill").arg("-INT").arg(pid.to_string()));
        assert!(sent.status.success(), "kill -INT should succeed: {sent:?}");

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
        let _reaper = KillBlockingUploadGroup {
            script: fake_proton_drive.clone(),
        };

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

    /// Drives one local deletion end to end through a real daemon in `mode`, and hands back
    /// `(the withheld item's JSON, what `proton-sync pending` printed, the local root, the trash)`.
    ///
    /// A REAL DAEMON AND A REAL TRASH. Everything below the socket is the shipping code path: the
    /// `trash` crate, the FreeDesktop layout, the executor. Only `XDG_DATA_HOME` is redirected, and
    /// that is done for every spawn in `DaemonProcess` rather than here — the hazard belongs to the
    /// default, not to this test.
    fn drive_one_local_deletion(mode: Option<&str>) -> (Value, String, PathBuf, PathBuf) {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake_proton_drive = write_local_delete_proton_drive(directory.path());
        let extra: Vec<&str> = match mode {
            Some(mode) => vec!["--local-delete-mode", mode],
            None => Vec::new(),
        };
        let mut daemon = DaemonProcess::spawn_with_args(
            &local_root,
            &socket_path,
            &lockfile_path,
            &db_path,
            &fake_proton_drive,
            &extra,
        );
        wait_for_socket(&socket_path, &mut daemon);

        // The startup pass downloads keep.txt and records a baseline; without the record there is
        // nothing to derive a LocalDelete from.
        let settled =
            wait_for_synced_baseline(&socket_path, &mut daemon, &db_path, &local_root, "keep.txt");
        let seq_before = settled["reconcile_seq"].as_u64().expect("reconcile_seq");

        // The remote copy disappears → the next pass plans a LocalDelete. The local guard is on by
        // default, so it is withheld and we can read the disposal off the reply before it applies.
        fs::write(format!("{}.gone", fake_proton_drive.display()), b"").expect("gone marker");
        run_control_args(&socket_path, &["--json", "syncnow", "--no-wait"]);
        let withheld = wait_for_reconcile_seq(&socket_path, &mut daemon, seq_before + 1);
        let pending = withheld["pending_deletions"]
            .as_array()
            .expect("pending_deletions array");
        assert_eq!(
            pending.len(),
            1,
            "the local delete must be withheld: {withheld}"
        );
        assert_eq!(pending[0]["direction"], "local");
        assert_eq!(pending[0]["path"], "keep.txt");
        let listed = run_control_raw(&socket_path, &["pending"]);

        let approved = run_control_raw(&socket_path, &["approve", "keep.txt"]);
        assert!(approved.contains("approved 1"), "approve: {approved}");
        let applied = run_control(&socket_path, "syncnow");
        assert!(
            applied["last_error"].is_null(),
            "pass should succeed: {applied}"
        );
        assert!(
            !local_root.join("keep.txt").exists(),
            "the approved local delete must remove the file from the sync root: {applied}"
        );

        // `XDG_DATA_HOME` is the lockfile's parent (see `DaemonProcess::spawn_with_args`).
        let trash = lockfile_path
            .parent()
            .expect("lockfile parent")
            .join("Trash/files");
        // The tempdir is dropped with `directory`, so read what the assertions need first.
        let item = pending[0].clone();
        let trashed: Vec<String> = fs::read_dir(&trash)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        (
            item,
            listed,
            local_root.clone(),
            PathBuf::from(trashed.join(",")),
        )
    }

    #[test]
    fn a_local_deletion_reports_recoverable_and_lands_in_the_real_trash_over_ipc() {
        // THE WHOLE CHANGE, end to end and through the shipping code: a default daemon reports the
        // deletion as recoverable, says so in words, and the bytes really are in the trash
        // afterwards. Nothing here is a fake but the `proton-drive` CLI itself.
        let (item, listed, _root, trashed) = drive_one_local_deletion(None);
        assert_eq!(
            item["disposal"], "recoverable",
            "a default daemon trashes, so the wire must say so: {item}"
        );
        assert!(
            listed.contains("keep.txt") && listed.contains("LOCAL DELETE"),
            "`pending` must show the withheld local delete: {listed}"
        );
        assert!(
            listed.contains("moves your copy to this computer's trash"),
            "and must say what approving would actually do: {listed}"
        );
        assert!(
            trashed.to_string_lossy().contains("keep.txt"),
            "the approved deletion must be recoverable from the trash, found: {trashed:?}"
        );
    }

    #[test]
    fn a_permanent_mode_local_deletion_reports_permanent_and_trashes_nothing_over_ipc() {
        // The opt-out, asserted rather than assumed. `!exists()` cannot tell the modes apart — a
        // trashed file also stops existing — so the discriminating facts are the wire's `permanent`
        // and an empty trash.
        let (item, listed, _root, trashed) = drive_one_local_deletion(Some("permanent"));
        assert_eq!(
            item["disposal"], "permanent",
            "a permanent-mode daemon must not report recoverable: {item}"
        );
        assert!(
            listed.contains("removes your local copy for good"),
            "the warning must come back with the mode: {listed}"
        );
        assert!(
            !trashed.to_string_lossy().contains("keep.txt"),
            "permanent mode must leave nothing in the trash, found: {trashed:?}"
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

    // ---------------------------------------------------------------------------------------
    // #102 phase 3: the `--pair`/`--all-pairs` client flags, driven against the real binaries.
    // ---------------------------------------------------------------------------------------

    #[test]
    fn an_explicit_pair_selector_addresses_the_named_pair() {
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
        let status = wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("default"), 1);
        assert_eq!(status["pair"], "default");
        assert_eq!(status["pairs"].as_array().map(Vec::len), Some(1));
        assert_eq!(status["pairs"][0]["name"], "default");
    }

    #[test]
    fn an_unknown_pair_selector_is_refused_and_names_the_configured_pairs() {
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
        wait_for_reconcile_seq(&socket_path, &mut daemon, 1);

        let (response, success) =
            run_control_args_any_exit(&socket_path, &["--pair", "videos", "--json", "status"]);
        assert!(!success, "an unresolved --pair must exit non-zero");
        assert!(response["pair"].is_null());
        assert!(
            response["message"]
                .as_str()
                .unwrap_or_default()
                .contains("default"),
            "the message must name the configured pairs: {response}"
        );
    }

    /// #102 phase 3, ADR 0005 §4: the client capability gate. No real daemon needed — a bare
    /// socket that answers exactly what a pre-multi-pair daemon's `status` reply looks like
    /// (neither `pair` nor `pairs`) is enough to prove the client refuses rather than silently
    /// addressing that daemon's one pair under the wrong name.
    #[test]
    fn the_capability_gate_refuses_pair_against_a_daemon_that_predates_it() {
        use std::os::unix::net::UnixListener;

        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("old-daemon.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind fake old daemon");
        let legacy_reply = r#"{"status":"running","paused":false,"pending_changes":0,
            "message":"daemon status","last_sync_epoch_secs":null,"last_error":null,
            "last_plan_summary":null,"last_successful_sync_summary":null,"status_history":[]}"#;
        let server = thread::spawn(move || {
            // One connection is enough: the capability gate issues exactly one `status` probe
            // before the client refuses and exits, never reaching the real request.
            if let Ok((mut stream, _)) = listener.accept() {
                let mut line = String::new();
                std::io::BufRead::read_line(&mut std::io::BufReader::new(&stream), &mut line).ok();
                stream
                    .write_all(legacy_reply.replace('\n', "").as_bytes())
                    .ok();
                stream.write_all(b"\n").ok();
            }
        });

        let output = run_client(
            control_command(&socket_path)
                .arg("--pair")
                .arg("photos")
                .arg("status"),
        );
        server.join().expect("fake old daemon thread");

        assert!(
            !output.status.success(),
            "an explicit --pair against a daemon that predates multi-pair must be refused"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("does not support multiple folder pairs"),
            "stderr: {stderr}"
        );
    }

    #[test]
    fn all_pairs_over_one_pair_is_a_loop_of_one_and_its_json_form_is_an_array() {
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
        wait_for_reconcile_seq(&socket_path, &mut daemon, 1);

        let response = run_control_args(&socket_path, &["--all-pairs", "--json", "status"]);
        let array = response
            .as_array()
            .expect("--all-pairs --json is a JSON array");
        assert_eq!(array.len(), 1, "one configured pair, one array element");
        assert_eq!(array[0]["pair"], "default");
        assert_eq!(array[0]["result"]["pair"], "default");
    }

    // ---------------------------------------------------------------------------------------
    // #102 phase 4c: the lift, driven against the real binaries. One daemon, a config of several
    // `[[pair]]` tables, and only daemon-wide flags on the command line (a per-pair flag beside
    // several pairs is refused, which `a_per_pair_flag_with_a_two_pair_config_...` proves).
    // ---------------------------------------------------------------------------------------

    /// One `[[pair]]` table per `(name, remote_root, scan_interval_secs)`, each with its root and
    /// state under `directory/<name>/` and the two keys that keep a test off the real session and
    /// off the clock: `events_driven = false` (the default would read the machine's keyring) and the
    /// scan interval. Returns the config path and each pair's `(local_root, db_path)`, in order.
    fn write_pairs_config(
        directory: &Path,
        extra_top_level: &str,
        pairs: &[(&str, &str, u64)],
    ) -> (PathBuf, Vec<(PathBuf, PathBuf)>) {
        write_pairs_config_with_events(directory, extra_top_level, pairs, false)
    }

    /// [`write_pairs_config`] with `events_driven` chosen. `true` is for the one test that runs the
    /// daemon's real session and events code against a scripted stream (`common::ScriptedEvents`).
    fn write_pairs_config_with_events(
        directory: &Path,
        extra_top_level: &str,
        pairs: &[(&str, &str, u64)],
        events_driven: bool,
    ) -> (PathBuf, Vec<(PathBuf, PathBuf)>) {
        let mut text = String::from(extra_top_level);
        let mut paths = Vec::new();
        for (name, remote_root, scan_interval_secs) in pairs {
            let local_root = directory.join(name).join("local");
            let db_path = directory.join(name).join("state").join("sync_index.db");
            let lockfile_path = directory.join(name).join("state").join("daemon.lock");
            fs::create_dir_all(&local_root).expect("local root");
            text.push_str(&format!(
                "\n[[pair]]\nname = \"{name}\"\nlocal_root = \"{}\"\nremote_root = \"{remote_root}\"\n\
                 db_path = \"{}\"\nlockfile_path = \"{}\"\nevents_driven = {events_driven}\n\
                 scan_interval_secs = {scan_interval_secs}\n",
                local_root.display(),
                db_path.display(),
                lockfile_path.display(),
            ));
            paths.push((local_root, db_path));
        }
        let path = directory.join("pairs.toml");
        fs::write(&path, text).expect("write pairs config");
        (path, paths)
    }

    /// A fake `proton-drive` for several remote roots: `list` answers an empty tree for any root
    /// under `/Drive/`, and `upload` appends `upload:<local>:<remote parent>` to `<script>.uploads`
    /// **before** it does anything else. When `block_upload_for` names a remote root, an upload into
    /// it touches `<script>.started` and then waits, for ever, for a `<script>.release` that no test
    /// writes: a pair wedged mid-transfer, keyed on its root so the others are not.
    fn write_multi_root_proton_drive(directory: &Path, block_upload_for: Option<&str>) -> PathBuf {
        let blocking = match block_upload_for {
            Some(root) => format!(
                "  case \"$6\" in\n    {root}|{root}/)\n      touch \"$0.started\"\n      \
                 while [ ! -f \"$0.release\" ]; do sleep 0.05; done\n      exit 0\n      ;;\n  esac\n"
            ),
            None => String::new(),
        };
        write_script(
            directory,
            "fake-multi-root-proton-drive",
            &format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
  case "$4" in
    /Drive/*) printf '{{"entries":[]}}\n'; exit 0 ;;
  esac
fi
if [ "$1" = "filesystem" ] && [ "$2" = "upload" ]; then
  printf 'upload:%s:%s\n' "$5" "$6" >> "$0.uploads"
{blocking}  exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
    }

    /// What the fake recorded uploading, one `upload:<local>:<remote parent>` per line.
    fn recorded_uploads(fake_proton_drive: &Path) -> Vec<String> {
        fs::read_to_string(format!("{}.uploads", fake_proton_drive.display()))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Two pairs with a file each, a daemon over them, and both boot passes done. The scan
    /// intervals are the tests' to choose (60 keeps a pair quiet; 1 keeps it busy).
    fn two_pair_daemon(
        directory: &Path,
        scan_intervals: [u64; 2],
    ) -> (DaemonProcess, PathBuf, PathBuf, Vec<(PathBuf, PathBuf)>) {
        let (config, paths) = write_pairs_config(
            directory,
            "",
            &[
                ("a", "/Drive/A", scan_intervals[0]),
                ("b", "/Drive/B", scan_intervals[1]),
            ],
        );
        for (local_root, _) in &paths {
            fs::write(local_root.join("f.txt"), b"content").expect("a file to upload");
        }
        let socket_path = directory.join("daemon.sock");
        let fake = write_multi_root_proton_drive(directory, None);
        let mut daemon = DaemonProcess::spawn_with_config(&config, &socket_path, &fake);
        wait_for_socket(&socket_path, &mut daemon);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("a"), 1);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("b"), 1);
        (daemon, socket_path, fake, paths)
    }

    fn pair_seq(socket_path: &Path, pair: &str) -> u64 {
        run_control_pair(socket_path, Some(pair), "status")["reconcile_seq"]
            .as_u64()
            .expect("reconcile_seq")
    }

    #[test]
    fn two_pairs_sync_independently_through_one_daemon() {
        let directory = tempdir().expect("tempdir");
        let (daemon, socket_path, fake, paths) = two_pair_daemon(directory.path(), [60, 60]);

        let a = run_control_pair(&socket_path, Some("a"), "status");
        let b = run_control_pair(&socket_path, Some("b"), "status");
        assert_eq!(
            (a["pair"].as_str(), b["pair"].as_str()),
            (Some("a"), Some("b"))
        );
        assert_eq!(a["pairs"].as_array().map(Vec::len), Some(2));
        for status in [&a, &b] {
            assert!(status["last_error"].is_null(), "{status}");
            assert_eq!(
                status["last_plan_summary"]["total"].as_u64(),
                Some(1),
                "{status}"
            );
        }
        // No selector is the default pair, the first table.
        let default = run_control(&socket_path, "status");
        assert_eq!(default["pair"], "a");

        // Each pair uploaded ITS file into ITS remote root, and nothing crossed.
        let uploads = recorded_uploads(&fake);
        let local_a = paths[0].0.join("f.txt");
        let local_b = paths[1].0.join("f.txt");
        assert_eq!(uploads.len(), 2, "{uploads:?}");
        assert!(
            uploads
                .iter()
                .any(|line| line.starts_with(&format!("upload:{}:/Drive/A", local_a.display()))),
            "{uploads:?}"
        );
        assert!(
            uploads
                .iter()
                .any(|line| line.starts_with(&format!("upload:{}:/Drive/B", local_b.display()))),
            "{uploads:?}"
        );
        // And each baseline is in its own index.
        for (_, db_path) in &paths {
            let index = load_existing_index(db_path).expect("index");
            assert!(index.contains_key(Path::new("f.txt")), "{index:?}");
        }

        // The notice that an unaddressed request means the default pair, once, at startup (maintainer
        // decision M4).
        let log = fs::read_to_string(&daemon.stderr_path).expect("daemon log");
        assert_eq!(
            log.matches("acts on the default pair only").count(),
            1,
            "{log}"
        );
    }

    #[test]
    fn syncnow_on_one_pair_advances_only_that_pairs_reconcile_seq() {
        let directory = tempdir().expect("tempdir");
        let (mut daemon, socket_path, _fake, _paths) = two_pair_daemon(directory.path(), [60, 60]);
        let (a_before, b_before) = (pair_seq(&socket_path, "a"), pair_seq(&socket_path, "b"));

        // `--no-wait`: the wait helper below is bounded, where a watcher for a pass that never
        // comes (the failure this test exists to show) is not.
        run_control_args(
            &socket_path,
            &["--pair", "b", "--json", "syncnow", "--no-wait"],
        );
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("b"), b_before + 1);
        assert_eq!(
            pair_seq(&socket_path, "a"),
            a_before,
            "a request for b ran b's pass, not the default pair's"
        );
        assert_eq!(pair_seq(&socket_path, "b"), b_before + 1);
    }

    /// A fake `proton-drive` for two roots on **one volume** (`vol`): `/Drive/A` and `/Drive/B`
    /// each list one file, `f.txt`, whose SHA-1 is `sha1` (the content the test wrote locally, so
    /// nothing needs transferring), under a root folder that carries its own uid. It appends every
    /// directory it is asked to list to `<script>.lists`, which is what the test reads.
    fn write_shared_volume_proton_drive(directory: &Path, sha1: &str) -> PathBuf {
        let listing = |name: &str, node: &str| {
            format!(
                r#"{{"entries":[{{"name":"{name}","type":"folder","uid":"vol~root-{node}","entries":[{{"name":"f.txt","uid":"vol~f-{node}","activeRevision":{{"claimedDigests":{{"sha1":"{sha1}"}}}}}}]}}]}}"#
            )
        };
        let (a, b) = (listing("A", "a"), listing("B", "b"));
        write_script(
            directory,
            "fake-shared-volume-proton-drive",
            &format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
  printf '%s\n' "$4" >> "$0.lists"
  case "$4" in
    /Drive/A) printf '%s\n' '{a}'; exit 0 ;;
    /Drive/B) printf '%s\n' '{b}'; exit 0 ;;
  esac
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
    }

    /// How many times the fake was asked to list `remote_root`.
    fn lists_of(fake_proton_drive: &Path, remote_root: &str) -> usize {
        fs::read_to_string(format!("{}.lists", fake_proton_drive.display()))
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == remote_root)
            .count()
    }

    /// #456, driven through the real binaries: two pairs on one Proton volume, the daemon's real
    /// session and events code against a scripted stream. A change in B's folder arrives in A's
    /// delta as a created node whose parent A has never heard of, and A used to list its whole tree
    /// for it. Here it lists nothing.
    #[test]
    fn a_second_pairs_change_never_makes_the_first_pair_list_its_tree() {
        let directory = tempdir().expect("tempdir");
        let (config, paths) = write_pairs_config_with_events(
            directory.path(),
            "",
            &[("a", "/Drive/A", 3600), ("b", "/Drive/B", 3600)],
            true,
        );
        for (local_root, _) in &paths {
            fs::write(local_root.join("f.txt"), b"content")
                .expect("a file in step with the remote");
        }
        let fake = write_shared_volume_proton_drive(
            directory.path(),
            &proton_drive_sync_engine::index::sha1_hex(b"content"),
        );
        let events = common::ScriptedEvents::in_directory(directory.path());
        events.latest("e1");
        let socket_path = directory.path().join("daemon.sock");
        let mut daemon = DaemonProcess::spawn_with_config_and_scripted_events(
            &config,
            &socket_path,
            &fake,
            &events,
        );
        wait_for_socket(&socket_path, &mut daemon);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("a"), 1);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("b"), 1);
        let a_before = lists_of(&fake, "/Drive/A");
        assert!(a_before >= 1, "precondition: A's boot pass listed its tree");
        let a_status = run_control_pair(&socket_path, Some("a"), "status");
        assert!(
            a_status["last_error"].is_null(),
            "the boot pass was clean: {a_status}"
        );

        // Somebody adds a file to B's folder. The volume's stream reports it to every pair.
        events.page(
            "e1",
            "e2",
            r#"[{"EventID":"ev-1","EventType":1,"Link":{"LinkID":"new-in-b","ParentLinkID":"root-b","IsShared":false,"IsTrashed":false}}]"#,
        );
        let a_seq = pair_seq(&socket_path, "a");
        run_control_args(&socket_path, &["--pair", "a", "--json", "syncnow"]);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("a"), a_seq + 1);

        assert!(
            events
                .requests()
                .iter()
                .any(|url| url.ends_with("/drive/v2/volumes/vol/events/e1")),
            "A's pass fetched the delta: {:?}",
            events.requests()
        );
        assert_eq!(
            lists_of(&fake, "/Drive/A"),
            a_before,
            "A listed nothing for a node in B's folder\n{}",
            daemon.stderr_tail()
        );
        let a_status = run_control_pair(&socket_path, Some("a"), "status");
        assert!(a_status["last_error"].is_null(), "{a_status}");
    }

    /// #456: a resumed pair runs at once, not at its next timer. Its timer here is ten minutes.
    #[test]
    fn a_resumed_pair_runs_before_its_timer() {
        let directory = tempdir().expect("tempdir");
        let (mut daemon, socket_path, _fake, _paths) =
            two_pair_daemon(directory.path(), [600, 600]);

        let paused = run_control_args(&socket_path, &["--pair", "b", "--json", "pause"]);
        assert_eq!(paused["paused"], true);
        let before = pair_seq(&socket_path, "b");

        let resumed = run_control_args(&socket_path, &["--pair", "b", "--json", "resume"]);
        assert_eq!(resumed["paused"], false);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("b"), before + 1);

        assert_eq!(pair_seq(&socket_path, "a"), 1, "and only `b` ran");
    }

    #[test]
    fn pausing_one_pair_leaves_the_other_syncing() {
        let directory = tempdir().expect("tempdir");
        // `b` is on a one-second cadence, so "b keeps syncing" and "b stopped" are both visible in
        // seconds rather than minutes. `a` stays quiet.
        let (mut daemon, socket_path, _fake, _paths) = two_pair_daemon(directory.path(), [60, 1]);

        // Pause the DEFAULT pair: b must carry on. (A pause read from pair 0 for every pair would
        // stop it.)
        let paused = run_control_args(&socket_path, &["--pair", "a", "--json", "pause"]);
        assert_eq!(paused["paused"], true);
        let before = pair_seq(&socket_path, "b");
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("b"), before + 2);
        assert_eq!(
            run_control_pair(&socket_path, Some("b"), "status")["paused"],
            false
        );
        run_control_args(&socket_path, &["--pair", "a", "--json", "resume"]);

        // Pause the SECOND pair: it stops, and the default pair is not what said so. (A pause read
        // from pair 0 would never see it, and b would run on.)
        run_control_args(&socket_path, &["--pair", "b", "--json", "pause"]);
        thread::sleep(Duration::from_millis(1500)); // any pass already in flight finishes
        let settled = pair_seq(&socket_path, "b");
        thread::sleep(Duration::from_millis(2500));
        assert_eq!(
            pair_seq(&socket_path, "b"),
            settled,
            "a paused pair runs no pass on its own cadence"
        );
        assert_eq!(
            run_control_pair(&socket_path, Some("a"), "status")["paused"],
            false
        );
    }

    /// #102, decision D12: a pair's pause is remembered in its own index, so stopping the daemon and
    /// starting it again over the same config does not resume it. Driven through the real binaries
    /// (sandboxed by `common`), the way the desktop app's restart-after-save does it.
    #[test]
    fn a_paused_pair_stays_paused_when_the_daemon_is_restarted() {
        let directory = tempdir().expect("tempdir");
        let (mut daemon, socket_path, fake, _paths) = two_pair_daemon(directory.path(), [60, 60]);
        let config = directory.path().join("pairs.toml");

        let paused = run_control_args(&socket_path, &["--pair", "b", "--json", "pause"]);
        assert_eq!(paused["paused"], true);
        assert!(
            paused["pause_unsaved"].is_null(),
            "the pause was saved: {paused}"
        );

        run_control_args(&socket_path, &["--json", "stop"]);
        let status = wait_for_exit(&mut daemon.child, Duration::from_secs(10))
            .expect("the daemon exits when asked to");
        assert!(status.success(), "a clean stop: {status:?}");
        drop(daemon);

        let mut daemon = DaemonProcess::spawn_with_config(&config, &socket_path, &fake);
        wait_for_socket(&socket_path, &mut daemon);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("a"), 1);
        // `b`'s job is popped right after `a`'s: give it the chance to run that it must not take.
        thread::sleep(Duration::from_millis(500));

        let b = run_control_pair(&socket_path, Some("b"), "status");
        assert_eq!(b["paused"], true, "`b` was paused before the restart: {b}");
        assert_eq!(b["status"], "paused", "{b}");
        assert_eq!(
            b["reconcile_seq"].as_u64(),
            Some(0),
            "its boot pass was skipped"
        );
        let a = run_control_pair(&socket_path, Some("a"), "status");
        assert_eq!(a["paused"], false, "`a` was not: {a}");
        assert!(a["reconcile_seq"].as_u64().unwrap_or(0) >= 1);

        // And resuming is remembered the same way.
        run_control_args(&socket_path, &["--pair", "b", "--json", "resume"]);
        run_control_args(&socket_path, &["--json", "stop"]);
        wait_for_exit(&mut daemon.child, Duration::from_secs(10)).expect("the daemon exits");
        drop(daemon);
        let mut daemon = DaemonProcess::spawn_with_config(&config, &socket_path, &fake);
        wait_for_socket(&socket_path, &mut daemon);
        wait_for_pair_reconcile_seq(&socket_path, &mut daemon, Some("b"), 1);
        assert_eq!(
            run_control_pair(&socket_path, Some("b"), "status")["paused"],
            false,
            "`b` was resumed before the second restart"
        );
    }

    #[test]
    fn all_pairs_status_over_two_pairs_is_a_two_element_array() {
        let directory = tempdir().expect("tempdir");
        let (_daemon, socket_path, _fake, _paths) = two_pair_daemon(directory.path(), [60, 60]);

        let response = run_control_args(&socket_path, &["--all-pairs", "--json", "status"]);
        let array = response.as_array().expect("--all-pairs --json is an array");
        assert_eq!(array.len(), 2, "one element per configured pair");
        assert_eq!(array[0]["pair"], "a");
        assert_eq!(array[1]["pair"], "b");
        for element in array {
            assert_eq!(element["result"]["pair"], element["pair"]);
            assert_eq!(element["result"]["pairs"].as_array().map(Vec::len), Some(2));
        }
    }

    #[test]
    fn sigint_while_the_second_pair_is_mid_transfer_leaves_the_first_committed_and_the_third_untouched()
     {
        let directory = tempdir().expect("tempdir");
        // A short CLI timeout bounds how long the blocked upload can hold the pass before the
        // daemon kills it and re-observes the signal it already received.
        let (config, paths) = write_pairs_config(
            directory.path(),
            "proton_timeout_secs = 2\n",
            &[
                ("a", "/Drive/A", 60),
                ("b", "/Drive/B", 60),
                ("c", "/Drive/C", 60),
            ],
        );
        for (local_root, _) in &paths {
            fs::write(local_root.join("f.txt"), b"content").expect("a file to upload");
        }
        let socket_path = directory.path().join("daemon.sock");
        let fake = write_multi_root_proton_drive(directory.path(), Some("/Drive/B"));
        let mut daemon = DaemonProcess::spawn_with_config(&config, &socket_path, &fake);
        wait_for_socket(&socket_path, &mut daemon);
        let pid = daemon.child.id();

        // Pair `a` has uploaded and committed, and pair `b`'s upload is the blocked call.
        wait_for_marker(
            &PathBuf::from(format!("{}.started", fake.display())),
            &mut daemon,
        );
        let sent = common::run_other_tool(Command::new("kill").arg("-INT").arg(pid.to_string()));
        assert!(sent.status.success(), "kill -INT should succeed: {sent:?}");
        let exit_status = wait_for_exit(&mut daemon.child, Duration::from_secs(6))
            .expect("the daemon exits once it re-observes the signal");
        assert!(exit_status.success(), "a clean shutdown: {exit_status:?}");

        assert!(
            load_existing_index(&paths[0].1)
                .expect("a's index")
                .contains_key(Path::new("f.txt")),
            "the pair that finished before the signal stays committed"
        );
        assert!(
            load_existing_index(&paths[1].1)
                .map(|index| index.is_empty())
                .unwrap_or(true),
            "the interrupted pair recorded nothing for the upload that never finished"
        );
        let uploads = recorded_uploads(&fake);
        assert!(
            uploads
                .iter()
                .any(|line| line.ends_with(":/Drive/A") || line.ends_with(":/Drive/A/")),
            "a uploaded: {uploads:?}"
        );
        assert!(
            !uploads.iter().any(|line| line.contains("/Drive/C")),
            "no pair starts after shutdown is asked for: {uploads:?}"
        );
        assert!(
            load_existing_index(&paths[2].1)
                .map(|index| index.is_empty())
                .unwrap_or(true),
            "the third pair recorded nothing"
        );
        // A pass that STARTS leaves a trace even when shutdown cuts it short before it can spawn
        // a child (the client refuses to once the flag is set, so the fake CLI never sees it, and
        // the index stays empty either way): its status history is written when the attempt ends.
        // Pair `a` finished a pass and has one; pair `c` never began one, and so has none.
        assert!(
            paths[0].1.with_extension("status.json").exists(),
            "the sidecar this test keys on is written by a pair's attempt (pair a ran one)"
        );
        assert!(
            !paths[2].1.with_extension("status.json").exists(),
            "the third pair never started a pass"
        );
    }

    #[test]
    fn a_per_pair_flag_with_a_two_pair_config_exits_non_zero_naming_it() {
        let directory = tempdir().expect("tempdir");
        let (config, _paths) = write_pairs_config(
            directory.path(),
            "",
            &[("a", "/Drive/A", 60), ("b", "/Drive/B", 60)],
        );
        // Sandboxed and bounded: if the flag rule were gone this would be a running daemon, and an
        // unbounded wait for it would hang the suite (and, with the machine's own socket and
        // runtime directory, put it on the real control socket). `DaemonProcess::start` is the
        // sandbox, and its drop kills whatever the bound below gives up on.
        let never_bound = directory.path().join("never-bound.sock");
        let fake = write_multi_root_proton_drive(directory.path(), None);
        let mut daemon = DaemonProcess::start(
            directory.path(),
            [
                OsStr::new("--config"),
                config.as_os_str(),
                OsStr::new("--socket-path"),
                never_bound.as_os_str(),
                // Without a CLI of its own this would be a daemon over the machine's real
                // `proton-drive` the moment the flag rule went.
                OsStr::new("--proton-cli"),
                fake.as_os_str(),
                // The opt-outs are per-pair flags too: a safeguard turned off for "the" pair, with
                // several, is the worst of the three ways to read it.
                OsStr::new("--no-delete-approval"),
                OsStr::new("--scan-interval-secs"),
                OsStr::new("5"),
            ],
        );
        let stderr_path = daemon.stderr_path.clone();
        let Some(status) = wait_for_exit(&mut daemon.child, Duration::from_secs(10)) else {
            panic!("a per-pair flag beside two pairs started a daemon instead of being refused");
        };
        assert!(
            !status.success(),
            "a per-pair flag beside two pairs is fatal"
        );
        let stderr = fs::read_to_string(&stderr_path).expect("read stderr log");
        for needle in ["--no-delete-approval", "--scan-interval-secs", "`a`", "`b`"] {
            assert!(stderr.contains(needle), "names {needle}: {stderr}");
        }
        assert!(
            !directory.path().join("never-bound.sock").exists(),
            "and nothing started"
        );

        // The daemon-wide flags are not per-pair: the same config starts with them.
        let socket_path = directory.path().join("daemon.sock");
        let mut daemon = DaemonProcess::spawn_with_config(&config, &socket_path, &fake);
        wait_for_socket(&socket_path, &mut daemon);
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

    /// A `proton-sync` command in the sandbox of `directory` (`common::sandboxed`): the control
    /// CLI resolves its default socket under `XDG_RUNTIME_DIR`, which is where the live daemon's is.
    fn proton_sync(directory: &Path) -> Command {
        common::sync_cli(directory)
    }

    /// `proton-sync --socket-path <socket_path>` in the sandbox of the socket's own directory.
    fn control_command(socket_path: &Path) -> Command {
        let mut command = proton_sync(socket_path.parent().expect("socket has a parent dir"));
        command.arg("--socket-path").arg(socket_path);
        command
    }

    /// Runs a control-CLI `command` to completion, bounded. **The one way these tests run it.**
    fn run_client(command: &mut Command) -> Output {
        common::run_bounded(command, common::RUN_BOUND)
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
        let output = run_client(control_command(socket_path).args(args));
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
        child: common::LoggedChild,
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

        /// A daemon over one folder pair given by flags, plus `extra_args`. Every daemon these
        /// tests start differs only by extra flags, so the argument list is written once here and
        /// the isolation environment and log capture once in `start`.
        fn spawn_with_args(
            local_root: &Path,
            socket_path: &Path,
            lockfile_path: &Path,
            db_path: &Path,
            proton_cli: &Path,
            extra_args: &[&str],
        ) -> Self {
            // The sandbox is the lockfile's directory (every caller keeps the lockfile, the index
            // and the socket together in the test's own tempdir).
            let sandbox = lockfile_path.parent().expect("lockfile has a parent dir");
            let mut args: Vec<&OsStr> = vec![
                OsStr::new("--local-root"),
                local_root.as_os_str(),
                OsStr::new("--remote-root"),
                OsStr::new("/Drive/RemoteFolder"),
                OsStr::new("--socket-path"),
                socket_path.as_os_str(),
                OsStr::new("--lockfile-path"),
                lockfile_path.as_os_str(),
                OsStr::new("--db-path"),
                db_path.as_os_str(),
                OsStr::new("--proton-cli"),
                proton_cli.as_os_str(),
                OsStr::new("--scan-interval-secs"),
                OsStr::new("60"),
                // Keep these process-level tests on the full-tree snapshot path (the default is
                // now event-driven, which would try to read the CLI keyring session at startup).
                OsStr::new("--no-events-driven"),
            ];
            args.extend(extra_args.iter().map(OsStr::new));
            Self::start(sandbox, args)
        }

        /// A daemon over a config file of several pairs, started with **only daemon-wide flags**
        /// (`--config`, `--socket-path`, `--proton-cli`): beside several pairs every per-pair flag
        /// — `--local-root`, `--no-events-driven`, `--scan-interval-secs` — is refused, which is why
        /// `spawn_with_args` cannot start one. The tables carry `events_driven = false` and a scan
        /// interval themselves. The isolation is `start`'s, like every other daemon here.
        fn spawn_with_config(config_path: &Path, socket_path: &Path, proton_cli: &Path) -> Self {
            Self::start(
                socket_path.parent().expect("socket has a parent dir"),
                [
                    OsStr::new("--config"),
                    config_path.as_os_str(),
                    OsStr::new("--socket-path"),
                    socket_path.as_os_str(),
                    OsStr::new("--proton-cli"),
                    proton_cli.as_os_str(),
                ],
            )
        }

        /// [`Self::spawn_with_config`] for a config whose pairs have `events_driven` on, against the
        /// scripted event stream `events` (see `common::ScriptedEvents`): the daemon runs its real
        /// session and events code, and every tool it shells for them is answered from the test's
        /// own files.
        fn spawn_with_config_and_scripted_events(
            config_path: &Path,
            socket_path: &Path,
            proton_cli: &Path,
            events: &common::ScriptedEvents,
        ) -> Self {
            let sandbox = socket_path.parent().expect("socket has a parent dir");
            let stderr_path = sandbox.join("daemon.stderr");
            let mut command = common::syncd(sandbox);
            command
                .args([
                    OsStr::new("--config"),
                    config_path.as_os_str(),
                    OsStr::new("--socket-path"),
                    socket_path.as_os_str(),
                    OsStr::new("--proton-cli"),
                    proton_cli.as_os_str(),
                ])
                .env("RUST_LOG", "warn");
            events.attach(&mut command);
            let child = common::spawn_logging(&mut command, &stderr_path);
            Self { child, stderr_path }
        }

        /// **The one place a `proton-syncd` is started.** Every daemon these tests run — and every
        /// one that is meant to be refused and must not become one — differs only by its flags, so
        /// the isolation, the log capture and the kill-on-drop live here once.
        ///
        /// `sandbox` is the test's own directory. `common::sandboxed` points `HOME`, the runtime
        /// dir, the state dir (the user-global single-instance lock: parallel daemons would
        /// contend on one machine-global `flock`, and a real `proton-syncd` on this machine would
        /// win — #77) and the data dir at it. The data dir is the trash: local deletions default
        /// to `local_delete_mode = "trash"`, and these are REAL daemons, so without it any test
        /// whose plan holds a LocalDelete moves its temp files into the developer's own
        /// `~/.local/share/Trash` on every `cargo test`. It is set for every start rather than for
        /// the tests that need it: the hazard belongs to the default, so a test that acquires a
        /// local delete later must not have to remember it. And the control socket is under the
        /// runtime dir by default: a start that forgot `--socket-path` would otherwise replace the
        /// live daemon's.
        fn start<S: AsRef<OsStr>>(sandbox: &Path, args: impl IntoIterator<Item = S>) -> Self {
            let stderr_path = sandbox.join("daemon.stderr");
            let mut command = common::syncd(sandbox);
            command.args(args).env("RUST_LOG", "warn");
            let child = common::spawn_logging(&mut command, &stderr_path);
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
    /// filesystem-watch events only accumulate `pending_changes` (see `Daemon::step_blocking` in
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
        let output = run_client(control_command(socket_path).arg("--json").arg(command));
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
        let output = run_client(control_command(socket_path).arg("--json").arg(command));
        serde_json::from_slice(&output.stdout).expect("control response JSON")
    }

    /// `run_control`, addressing one folder pair by name (#102 phase 3). `None` is `run_control`
    /// itself — every existing caller of that stays byte-identical.
    fn run_control_pair(socket_path: &Path, pair: Option<&str>, command: &str) -> Value {
        match pair {
            None => run_control(socket_path, command),
            Some(name) => run_control_args(socket_path, &["--pair", name, "--json", command]),
        }
    }

    /// Polls `status` until the daemon has completed at least `passes` reconcile attempts, for
    /// one named pair (`None` = the default pair). The control socket answers while a reconcile
    /// is in flight, so tests that assert on last-sync state must explicitly wait for the pass to
    /// finish instead of relying on the old accept-queue blocking.
    ///
    /// This is the shape every later multi-pair integration test is written against (#102 phase
    /// 3): a per-pair `reconcile_seq` is what makes waiting on ONE pair's pass — while another
    /// pair's passes advance their own counter — correct rather than accidental.
    fn wait_for_pair_reconcile_seq(
        socket_path: &Path,
        daemon: &mut DaemonProcess,
        pair: Option<&str>,
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
            let status = run_control_pair(socket_path, pair, "status");
            let seq = status["reconcile_seq"].as_u64().unwrap_or(0);
            let syncing = status["syncing"].as_bool().unwrap_or(false);
            if seq >= passes && !syncing {
                return status;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "timed out waiting for reconcile pass {passes} on pair {pair:?}\n{}",
            daemon.stderr_tail()
        );
    }

    /// As before #102 phase 3, unchanged: the default pair.
    fn wait_for_reconcile_seq(
        socket_path: &Path,
        daemon: &mut DaemonProcess,
        passes: u64,
    ) -> Value {
        wait_for_pair_reconcile_seq(socket_path, daemon, None, passes)
    }

    /// Runs the control CLI and returns its raw stdout, for subcommands whose output is
    /// human-readable text rather than JSON (`pending`, `approve`, `deny`).
    fn run_control_raw(socket_path: &Path, args: &[&str]) -> String {
        let output = run_client(control_command(socket_path).args(args));
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
    /// `write_delete_approval_proton_drive`'s mirror: the remote file **disappears** once the test
    /// drops a `.gone` marker beside the script, so the next pass plans a `LocalDelete` instead of a
    /// `RemoteDelete`. That is the only direction whose disposal `local_delete_mode` moves.
    fn write_local_delete_proton_drive(directory: &Path) -> PathBuf {
        let path = directory.join("fake-local-delete-proton-drive");
        // SHA-1 of the literal bytes "hello" (what `download` writes below).
        let hello_sha1 = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
  if [ -f "$0.gone" ]; then
    printf '{{"entries":[]}}\n'
  else
    printf '{{"entries":[{{"id":"remote-keep","name":"keep.txt","activeRevision":{{"claimedDigests":{{"sha1":"{hello_sha1}"}}}}}}]}}\n'
  fi
  exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "download" ]; then
  printf 'hello' > "$4/$(basename "$3")"
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

    /// Kills the process group of the fake blocking CLI when the test ends (PR #434 third review,
    /// L7). The daemon starts the fake in a group of its own (`run_once`), so killing the daemon
    /// does not reach it, and the fake used to wait for a release file that never came: 59 of them
    /// were found on one machine. The fake writes its pid to `<script>.pid`; this reads it and
    /// kills `-pid`, **only if** that process still runs the script (a recycled pid is somebody
    /// else's). Declare it before the daemon so the daemon is dropped first.
    struct KillBlockingUploadGroup {
        script: PathBuf,
    }

    impl Drop for KillBlockingUploadGroup {
        fn drop(&mut self) {
            let Ok(text) = fs::read_to_string(format!("{}.pid", self.script.display())) else {
                return;
            };
            let Ok(pid) = text.trim().parse::<u32>() else {
                return;
            };
            let still_the_script = fs::read(format!("/proc/{pid}/cmdline"))
                .map(|bytes| {
                    String::from_utf8_lossy(&bytes).contains(&*self.script.to_string_lossy())
                })
                .unwrap_or(false);
            if still_the_script {
                let _ = common::run_other_tool(
                    Command::new("kill")
                        .arg("-KILL")
                        .arg("--")
                        .arg(format!("-{pid}")),
                );
            }
        }
    }

    /// How many processes have `directory` in their command line: everything a test started in its
    /// own temporary directory (the pattern is bracketed so `pgrep` cannot match itself).
    fn processes_running_from(directory: &Path) -> usize {
        let text = directory.display().to_string();
        let (first, rest) = text.split_at(1);
        let pattern = format!("[{first}]{rest}/");
        let output = common::run_other_tool(Command::new("pgrep").arg("-f").arg(pattern));
        String::from_utf8_lossy(&output.stdout).lines().count()
    }

    fn wait_until_nothing_runs_from(directory: &Path, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if processes_running_from(directory) == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn the_fake_blocking_cli_leaves_when_the_process_that_started_it_does() {
        // PR #434 third review, L7. Nothing ever stopped this script once its test was over.
        let directory = tempdir().expect("tempdir");
        let fake = write_blocking_upload_proton_drive(directory.path(), "/Drive/R");
        let mut parent = common::sandboxed("sh", directory.path());
        parent
            .arg("-c")
            .arg("\"$0\" filesystem upload a b c & wait")
            .arg(&fake);
        let mut parent = common::spawn_logging(&mut parent, &directory.path().join("parent.log"));
        let started = PathBuf::from(format!("{}.started", fake.display()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !started.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(started.exists(), "the fake is blocked in its upload");
        assert!(
            processes_running_from(directory.path()) >= 2,
            "precondition: the parent and the fake are running"
        );

        parent
            .kill()
            .expect("kill the process that started the fake");
        parent.wait().expect("reap it");
        assert!(
            wait_until_nothing_runs_from(directory.path(), Duration::from_secs(5)),
            "the fake outlived its parent"
        );
    }

    #[test]
    fn the_group_killer_ends_the_fake_and_everything_it_started() {
        use std::os::unix::process::CommandExt;
        // A stand-in with the fake's shape: it writes its pid to `<script>.pid`, is the leader of
        // its own group, and starts a child that would outlive it.
        let directory = tempdir().expect("tempdir");
        let script = directory.path().join("stand-in");
        fs::write(
            &script,
            "#!/bin/sh\necho $$ > \"$0.pid\"\nsh -c 'sleep 300; :' \"$0.child\" &\nwait\n",
        )
        .expect("stand-in");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).expect("mode");
        let mut command = common::sandboxed(&script, directory.path());
        command.process_group(0);
        let mut child = common::spawn_logging(&mut command, &directory.path().join("child.log"));
        let pid_file = PathBuf::from(format!("{}.pid", script.display()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && !fs::read_to_string(&pid_file).is_ok_and(|text| !text.trim().is_empty())
        {
            thread::sleep(Duration::from_millis(25));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while processes_running_from(directory.path()) < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            processes_running_from(directory.path()) >= 2,
            "precondition: the stand-in and its child run"
        );

        drop(KillBlockingUploadGroup {
            script: script.clone(),
        });
        let _ = wait_for_exit(&mut child, Duration::from_secs(5)).expect("the leader is killed");
        assert!(
            wait_until_nothing_runs_from(directory.path(), Duration::from_secs(5)),
            "and so is the child it started"
        );
    }

    #[test]
    fn a_finished_daemon_test_leaves_no_fake_cli_running() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("blocking.txt"), b"content").expect("write fixture");
        let socket_path = directory.path().join("daemon.sock");
        let lockfile_path = directory.path().join("daemon.lock");
        let db_path = directory.path().join("sync_index.db");
        let fake = write_blocking_upload_proton_drive(directory.path(), "/Drive/RemoteFolder");
        {
            let _reaper = KillBlockingUploadGroup {
                script: fake.clone(),
            };
            let mut daemon = DaemonProcess::spawn_with_proton_timeout(
                &local_root,
                &socket_path,
                &lockfile_path,
                &db_path,
                &fake,
                2,
            );
            wait_for_socket(&socket_path, &mut daemon);
            wait_for_marker(
                &PathBuf::from(format!("{}.started", fake.display())),
                &mut daemon,
            );
            assert!(
                processes_running_from(directory.path()) >= 2,
                "precondition: the daemon and the fake it is blocked on run"
            );
        }
        assert!(
            wait_until_nothing_runs_from(directory.path(), Duration::from_secs(5)),
            "the daemon is dropped and nothing of the test is left running"
        );
    }

    fn write_blocking_upload_proton_drive(directory: &Path, remote_root: &str) -> PathBuf {
        let path = directory.join("fake-blocking-upload-proton-drive");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
parent=$PPID
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "{remote_root}" ]; then
    printf '{{"entries":[]}}\n'
    exit 0
fi
if [ "$1" = "filesystem" ] && [ "$2" = "upload" ]; then
    echo $$ > "$0.pid"
    touch "$0.started"
    # Wait for the release file, but never outlive the test: this script is a child of the daemon,
    # which a finished test kills, and nothing else would ever stop it (59 were found running, up
    # to 1.3 days old). It leaves when its parent is gone, and after a minute whatever happens.
    waited=0
    while [ ! -f "$0.release" ]; do
        kill -0 "$parent" 2>/dev/null || exit 0
        waited=$((waited + 1))
        [ "$waited" -le 1200 ] || exit 0
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
