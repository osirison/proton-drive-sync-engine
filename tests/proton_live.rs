use proton_drive_sync_engine::index::{
    FileRecord, SyncStatus, compute_sha1, local_file_state, scan_local_entities,
};
use proton_drive_sync_engine::proton::{
    CommandPolicy, ProtonClient, ProtonDriveClient, RemoteEntity,
};
use proton_drive_sync_engine::sync::{
    SyncAction, is_conflict_copy, original_from_conflict_copy, plan_sync_entities,
};
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;

const LIVE_E2E_REMOTE_ROOT_PREFIX: &str = "proton-sync-e2e-";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveE2eConfig {
    remote_root: PathBuf,
    executable: PathBuf,
    timeout_secs: u64,
    list_attempts: usize,
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_REMOTE_ROOT and an authenticated proton-drive CLI"]
fn live_proton_drive_filesystem_list_smoke() {
    let remote_root = PathBuf::from(
        env::var_os("PROTON_SYNC_LIVE_REMOTE_ROOT")
            .expect("set PROTON_SYNC_LIVE_REMOTE_ROOT to a safe Proton Drive folder"),
    );
    let executable = env::var_os("PROTON_SYNC_LIVE_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("proton-drive"));
    let timeout_secs = parse_positive_u64_env(
        env::var_os("PROTON_SYNC_LIVE_TIMEOUT_SECS"),
        30,
        "PROTON_SYNC_LIVE_TIMEOUT_SECS",
    )
    .expect("valid live timeout setting");
    let list_attempts = parse_positive_usize_env(
        env::var_os("PROTON_SYNC_LIVE_LIST_ATTEMPTS"),
        2,
        "PROTON_SYNC_LIVE_LIST_ATTEMPTS",
    )
    .expect("valid live list attempts setting");
    let client = ProtonDriveClient::with_command_policy(
        executable,
        CommandPolicy::new(Duration::from_secs(timeout_secs), list_attempts),
    );

    let files = client
        .list(&remote_root)
        .expect("live proton-drive filesystem list should succeed");

    for (relative_path, file) in files {
        assert_eq!(
            relative_path, file.path,
            "parser map key should match the parsed remote file path"
        );
        assert!(
            !file.id.is_empty(),
            "live Proton Drive file entries should include ids"
        );
    }
}

/// #59 follow-up, still unverified offline: nothing in the repo pins the CLI's **failed**
/// wrapped-value shape, so `proton::collect_node`'s undecodable-node guard shipped blind. The
/// guard fires only on a value that is PRESENT but unreadable (`{"ok": false, ...}`, `{}`,
/// `{"value": null}`, a non-string scalar); it cannot see an absent or `null` field, because
/// those are legitimate and common. This read-only probe reports which shapes a real account
/// actually emits for `id`/`uid`/`name`/`path`, and asserts the guard does not fire on a healthy
/// listing. Run it against an account that has a degraded (undecryptable-name) node to learn
/// whether the guard can ever fire in practice.
#[test]
#[ignore = "requires PROTON_SYNC_LIVE_REMOTE_ROOT and an authenticated proton-drive CLI"]
fn live_wrapped_value_shapes_for_the_undecodable_node_guard() {
    let remote_root = PathBuf::from(
        env::var_os("PROTON_SYNC_LIVE_REMOTE_ROOT")
            .expect("set PROTON_SYNC_LIVE_REMOTE_ROOT to a safe Proton Drive folder"),
    );
    let executable = env::var_os("PROTON_SYNC_LIVE_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("proton-drive"));

    let output = std::process::Command::new(&executable)
        .args([
            OsString::from("filesystem"),
            OsString::from("list"),
            OsString::from("--json"),
        ])
        .arg(remote_root.as_os_str())
        .output()
        .expect("run proton-drive filesystem list");
    assert!(
        output.status.success(),
        "proton-drive list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("CLI listing should be JSON");

    let mut shapes: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    report_wrapped_shapes(&value, &mut shapes);
    for (shape, count) in &shapes {
        println!("{shape}: {count}");
    }
    assert!(
        !shapes.is_empty(),
        "the listing exposed no id/uid/name/path fields at all — the probe needs updating"
    );

    let client = ProtonDriveClient::new(executable);
    client
        .list(&remote_root)
        .expect("the undecodable-node guard must not fire on a healthy listing");
}

/// Tallies `field=shape` counts for the identity/locator fields the #59 guard keys on.
fn report_wrapped_shapes(
    value: &serde_json::Value,
    shapes: &mut std::collections::BTreeMap<String, usize>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                report_wrapped_shapes(item, shapes);
            }
        }
        serde_json::Value::Object(object) => {
            for field in ["id", "uid", "name", "path"] {
                let shape = match object.get(field) {
                    None => "absent",
                    Some(serde_json::Value::Null) => "null",
                    Some(serde_json::Value::String(_)) => "bare string",
                    Some(serde_json::Value::Object(wrapper)) => {
                        match (wrapper.get("ok"), wrapper.get("value")) {
                            (_, Some(serde_json::Value::String(_))) => {
                                "wrapper with a string value"
                            }
                            (Some(serde_json::Value::Bool(false)), _) => "wrapper with ok=false",
                            _ => "wrapper without a usable value",
                        }
                    }
                    Some(_) => "non-string scalar",
                };
                *shapes.entry(format!("{field}={shape}")).or_default() += 1;
            }
            for nested in ["children", "entries", "files"] {
                if let Some(nested) = object.get(nested) {
                    report_wrapped_shapes(nested, shapes);
                }
            }
        }
        _ => {}
    }
}

fn parse_positive_u64_env(
    value: Option<OsString>,
    default_value: u64,
    name: &str,
) -> Result<u64, String> {
    let Some(value) = value else {
        return Ok(default_value);
    };
    let value = value
        .into_string()
        .map_err(|_| format!("{name} must be valid UTF-8"))?;
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{name} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(parsed)
}

fn parse_positive_usize_env(
    value: Option<OsString>,
    default_value: usize,
    name: &str,
) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(default_value);
    };
    let value = value
        .into_string()
        .map_err(|_| format!("{name} must be valid UTF-8"))?;
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(parsed)
}

fn live_e2e_config_from_env() -> Result<LiveE2eConfig, String> {
    parse_live_e2e_config(
        env::var_os("PROTON_SYNC_LIVE_E2E"),
        env::var_os("PROTON_SYNC_LIVE_REMOTE_ROOT"),
        env::var_os("PROTON_SYNC_LIVE_CLI"),
        env::var_os("PROTON_SYNC_LIVE_TIMEOUT_SECS"),
        env::var_os("PROTON_SYNC_LIVE_LIST_ATTEMPTS"),
    )
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_E2E=1 and a disposable Proton Drive folder"]
fn mutating_live_e2e_gate_loads_safe_environment() {
    let config = live_e2e_config_from_env().expect("safe mutating live E2E configuration");
    let _client = ProtonDriveClient::with_command_policy(
        config.executable,
        CommandPolicy::new(
            Duration::from_secs(config.timeout_secs),
            config.list_attempts,
        ),
    );
    let run_folder = unique_live_run_folder(
        "mutating-live-e2e-gate-loads-safe-environment",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("current time after epoch")
            .as_secs(),
        std::process::id(),
    );

    assert!(
        run_folder.to_string_lossy().starts_with("run-"),
        "run folder should be scoped under the disposable live root"
    );
}

/// Ensures the disposable run folder (and everything under it) is trashed even if an
/// assertion earlier in the test panics, so a failed live run never leaves debris behind
/// in the operator's Proton Drive account. Trash (not permanent delete) is used so an
/// operator can still recover manually from an unexpected mid-test failure.
struct LiveRunCleanup<'a> {
    client: &'a ProtonDriveClient,
    run_root: PathBuf,
    completed: bool,
}

impl Drop for LiveRunCleanup<'_> {
    fn drop(&mut self) {
        match self.client.delete(&self.run_root) {
            Ok(()) if self.completed => {}
            Ok(()) => eprintln!(
                "live E2E cleanup: trashed disposable run folder {} after an early failure",
                self.run_root.display()
            ),
            Err(error) => eprintln!(
                "live E2E cleanup FAILED for {}: {error}. Trash this folder manually.",
                self.run_root.display()
            ),
        }
    }
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_E2E=1 and a disposable Proton Drive folder"]
fn mutating_live_e2e_exercises_upload_download_rename_move_delete() {
    let config = live_e2e_config_from_env().expect("safe mutating live E2E configuration");
    let client = ProtonDriveClient::with_command_policy(
        config.executable.clone(),
        CommandPolicy::new(
            Duration::from_secs(config.timeout_secs),
            config.list_attempts,
        ),
    );

    // The disposable prefix root (e.g. `/my-files/proton-sync-e2e-...`) may not exist
    // yet on a fresh account; create it against its own parent before scoping a unique
    // run folder underneath it.
    let root_parent = config
        .remote_root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .expect("PROTON_SYNC_LIVE_REMOTE_ROOT must have a parent folder");
    let root_name = config
        .remote_root
        .file_name()
        .map(PathBuf::from)
        .expect("PROTON_SYNC_LIVE_REMOTE_ROOT must have a folder name");
    client
        .ensure_directory(root_parent, &root_name)
        .expect("disposable live E2E root should be creatable");

    let run_folder = unique_live_run_folder(
        "mutating-live-e2e-exercises-upload-download-rename-move-delete",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("current time after epoch")
            .as_secs(),
        std::process::id(),
    );
    client
        .ensure_directory(&config.remote_root, &run_folder)
        .expect("unique live E2E run folder should be creatable");
    let run_root = config.remote_root.join(&run_folder);
    let mut cleanup = LiveRunCleanup {
        client: &client,
        run_root: run_root.clone(),
        completed: false,
    };

    let local_dir = tempdir().expect("local temp directory");
    let uploaded_content = b"proton-sync live E2E upload/download roundtrip content\n";
    let upload_path = local_dir.path().join("hello.txt");
    fs::write(&upload_path, uploaded_content).expect("write local upload fixture");
    let expected_sha1 = compute_sha1(&upload_path).expect("compute local fixture sha1");

    // Upload.
    client
        .upload(&upload_path, &run_root, Path::new("hello.txt"))
        .expect("live upload should succeed");
    let after_upload = client.list(&run_root).expect("list after upload");
    let uploaded = after_upload
        .get(Path::new("hello.txt"))
        .expect("uploaded file should be listed under the run folder");
    assert!(
        uploaded.downloadable,
        "a plain text upload should be downloadable"
    );
    if let Some(remote_hash) = &uploaded.sha1_hash {
        assert_eq!(
            remote_hash, &expected_sha1,
            "remote SHA-1 should match the uploaded content once available"
        );
    }
    let uploaded_id = uploaded.id.clone();

    // Download roundtrip. `download()` always names the local file after the remote
    // file's own basename (it only uses the destination's *parent* as the target
    // folder), so the download directory must be separate from the upload fixture's
    // directory and the destination's basename must match the remote basename.
    let download_dir = tempdir().expect("local download directory");
    let download_path = download_dir.path().join("hello.txt");
    client
        .download(&run_root.join("hello.txt"), &download_path)
        .expect("live download should succeed");
    let downloaded_content = fs::read(&download_path).expect("read downloaded fixture");
    assert_eq!(
        downloaded_content, uploaded_content,
        "downloaded content should match the uploaded content byte-for-byte"
    );

    // Rename in place.
    client
        .rename_or_move(&run_root, Path::new("hello.txt"), Path::new("renamed.txt"))
        .expect("live rename should succeed");
    let after_rename = client.list(&run_root).expect("list after rename");
    assert!(
        !after_rename.contains_key(Path::new("hello.txt")),
        "the old name should no longer be listed after a rename"
    );
    let renamed = after_rename
        .get(Path::new("renamed.txt"))
        .expect("the new name should be listed after a rename");
    assert_eq!(
        renamed.id, uploaded_id,
        "rename should preserve the remote file identity"
    );

    // Move into a subfolder.
    client
        .ensure_directory(&run_root, Path::new("moved"))
        .expect("live subfolder creation should succeed");
    client
        .rename_or_move(
            &run_root,
            Path::new("renamed.txt"),
            Path::new("moved/renamed.txt"),
        )
        .expect("live move should succeed");
    let after_move = client.list(&run_root).expect("list after move");
    assert!(
        !after_move.contains_key(Path::new("renamed.txt")),
        "the old parent location should no longer be listed after a move"
    );
    let moved = after_move
        .get(Path::new("moved/renamed.txt"))
        .expect("the new nested location should be listed after a move");
    assert_eq!(
        moved.id, uploaded_id,
        "move should preserve the remote file identity"
    );

    // Delete.
    client
        .delete(&run_root.join("moved/renamed.txt"))
        .expect("live delete should succeed");
    let after_delete = client.list(&run_root).expect("list after delete");
    assert!(
        !after_delete.contains_key(Path::new("moved/renamed.txt")),
        "the file should no longer be listed after being trashed"
    );

    cleanup.completed = true;
}

/// Ensures the disposable prefix root exists (creating it against its own parent if this
/// is a fresh account) and creates a fresh, uniquely named run folder underneath it,
/// returning the run folder's full path relative to nothing (i.e. the absolute remote
/// path). Shared by every mutating live E2E test added after the original upload/
/// download/rename/move/delete scenario, to avoid repeating the same root-bootstrap
/// dance in each one.
fn ensure_live_run_root(
    client: &ProtonDriveClient,
    config: &LiveE2eConfig,
    test_name: &str,
) -> PathBuf {
    let root_parent = config
        .remote_root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .expect("PROTON_SYNC_LIVE_REMOTE_ROOT must have a parent folder");
    let root_name = config
        .remote_root
        .file_name()
        .map(PathBuf::from)
        .expect("PROTON_SYNC_LIVE_REMOTE_ROOT must have a folder name");
    client
        .ensure_directory(root_parent, &root_name)
        .expect("disposable live E2E root should be creatable");

    let run_folder = unique_live_run_folder(
        test_name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("current time after epoch")
            .as_secs(),
        std::process::id(),
    );
    client
        .ensure_directory(&config.remote_root, &run_folder)
        .expect("unique live E2E run folder should be creatable");
    config.remote_root.join(&run_folder)
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_E2E=1 and a disposable Proton Drive folder"]
fn mutating_live_e2e_verifies_directory_id_exposure() {
    let config = live_e2e_config_from_env().expect("safe mutating live E2E configuration");
    let client = ProtonDriveClient::with_command_policy(
        config.executable.clone(),
        CommandPolicy::new(
            Duration::from_secs(config.timeout_secs),
            config.list_attempts,
        ),
    );
    let run_root = ensure_live_run_root(
        &client,
        &config,
        "mutating-live-e2e-verifies-directory-id-exposure",
    );
    let mut cleanup = LiveRunCleanup {
        client: &client,
        run_root: run_root.clone(),
        completed: false,
    };

    client
        .ensure_directory(&run_root, Path::new("docs"))
        .expect("live nested folder creation should succeed");

    let entities = client
        .list_entities(&run_root)
        .expect("live list_entities should succeed");
    let docs = entities
        .get(Path::new("docs"))
        .and_then(RemoteEntity::as_directory)
        .expect("the created folder should be listed as a directory entity");
    assert!(
        docs.id.as_deref().is_some_and(|id| !id.is_empty()),
        "a real Proton Drive folder should expose a non-empty id, which the directory \
         proton_id backfill and directory rename/move detection both depend on"
    );

    cleanup.completed = true;
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_E2E=1 and a disposable Proton Drive folder"]
fn mutating_live_e2e_trashes_nested_folder_contents() {
    let config = live_e2e_config_from_env().expect("safe mutating live E2E configuration");
    let client = ProtonDriveClient::with_command_policy(
        config.executable.clone(),
        CommandPolicy::new(
            Duration::from_secs(config.timeout_secs),
            config.list_attempts,
        ),
    );
    let run_root = ensure_live_run_root(
        &client,
        &config,
        "mutating-live-e2e-trashes-nested-folder-contents",
    );
    let mut cleanup = LiveRunCleanup {
        client: &client,
        run_root: run_root.clone(),
        completed: false,
    };

    client
        .ensure_directory(&run_root, Path::new("folder-to-trash"))
        .expect("live nested folder creation should succeed");
    let local_dir = tempdir().expect("local temp directory");
    let nested_upload_path = local_dir.path().join("nested.txt");
    fs::write(
        &nested_upload_path,
        b"nested file inside a folder to be trashed\n",
    )
    .expect("write nested upload fixture");
    client
        .upload(
            &nested_upload_path,
            &run_root,
            Path::new("folder-to-trash/nested.txt"),
        )
        .expect("live nested upload should succeed");

    let before_trash = client
        .list_entities(&run_root)
        .expect("list_entities before trash");
    assert!(
        before_trash.contains_key(Path::new("folder-to-trash")),
        "the folder should be listed before it is trashed"
    );
    assert!(
        before_trash.contains_key(Path::new("folder-to-trash/nested.txt")),
        "the nested file should be listed before its parent folder is trashed"
    );

    client
        .delete(&run_root.join("folder-to-trash"))
        .expect("live folder trash should succeed");

    let after_trash = client
        .list_entities(&run_root)
        .expect("list_entities after trash");
    assert!(
        !after_trash.contains_key(Path::new("folder-to-trash")),
        "the trashed folder itself should no longer be listed: {after_trash:?}"
    );
    assert!(
        !after_trash.contains_key(Path::new("folder-to-trash/nested.txt")),
        "the trashed folder's nested file should no longer be listed: {after_trash:?}"
    );

    cleanup.completed = true;
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_E2E=1 and a disposable Proton Drive folder"]
fn mutating_live_e2e_verifies_directory_rename_and_move() {
    let config = live_e2e_config_from_env().expect("safe mutating live E2E configuration");
    let client = ProtonDriveClient::with_command_policy(
        config.executable.clone(),
        CommandPolicy::new(
            Duration::from_secs(config.timeout_secs),
            config.list_attempts,
        ),
    );
    let run_root = ensure_live_run_root(
        &client,
        &config,
        "mutating-live-e2e-verifies-directory-rename-and-move",
    );
    let mut cleanup = LiveRunCleanup {
        client: &client,
        run_root: run_root.clone(),
        completed: false,
    };

    client
        .ensure_directory(&run_root, Path::new("old-folder"))
        .expect("live folder creation should succeed");
    let created_id = client
        .list_entities(&run_root)
        .expect("list_entities after create")
        .get(Path::new("old-folder"))
        .and_then(RemoteEntity::as_directory)
        .expect("the created folder should be listed as a directory entity")
        .id
        .clone();

    // Rename in place.
    client
        .rename_or_move(&run_root, Path::new("old-folder"), Path::new("new-folder"))
        .expect("live folder rename should succeed");
    let after_rename = client
        .list_entities(&run_root)
        .expect("list_entities after rename");
    assert!(
        !after_rename.contains_key(Path::new("old-folder")),
        "the old folder name should no longer be listed after a rename"
    );
    let renamed = after_rename
        .get(Path::new("new-folder"))
        .and_then(RemoteEntity::as_directory)
        .expect("the new folder name should be listed after a rename");
    assert_eq!(
        renamed.id, created_id,
        "renaming a folder should preserve its remote identity"
    );

    // Move into a subfolder.
    client
        .ensure_directory(&run_root, Path::new("parent"))
        .expect("live parent folder creation should succeed");
    client
        .rename_or_move(
            &run_root,
            Path::new("new-folder"),
            Path::new("parent/new-folder"),
        )
        .expect("live folder move should succeed");
    let after_move = client
        .list_entities(&run_root)
        .expect("list_entities after move");
    assert!(
        !after_move.contains_key(Path::new("new-folder")),
        "the old parent location should no longer be listed after a move"
    );
    let moved = after_move
        .get(Path::new("parent/new-folder"))
        .and_then(RemoteEntity::as_directory)
        .expect("the new nested location should be listed after a move");
    assert_eq!(
        moved.id, created_id,
        "moving a folder should preserve its remote identity"
    );

    cleanup.completed = true;
}

#[test]
#[ignore = "requires PROTON_SYNC_LIVE_E2E=1 and a disposable Proton Drive folder"]
fn mutating_live_e2e_resolves_conflict_by_downloading_a_sidecar_copy() {
    let config = live_e2e_config_from_env().expect("safe mutating live E2E configuration");
    let client = ProtonDriveClient::with_command_policy(
        config.executable.clone(),
        CommandPolicy::new(
            Duration::from_secs(config.timeout_secs),
            config.list_attempts,
        ),
    );
    let run_root = ensure_live_run_root(
        &client,
        &config,
        "mutating-live-e2e-resolves-conflict-by-downloading-a-sidecar-copy",
    );
    let mut cleanup = LiveRunCleanup {
        client: &client,
        run_root: run_root.clone(),
        completed: false,
    };

    // Establish a synced baseline that both sides start from, mirroring what the
    // daemon's SQLite index would hold after a prior successful reconcile.
    let local_dir = tempdir().expect("local temp directory");
    let local_path = local_dir.path().join("notes.txt");
    fs::write(&local_path, b"synced baseline content\n").expect("write baseline fixture");
    client
        .upload(&local_path, &run_root, Path::new("notes.txt"))
        .expect("live baseline upload should succeed");
    let baseline_local =
        local_file_state(local_dir.path(), &local_path).expect("scan baseline local file state");
    let baseline_remote_id = client
        .list(&run_root)
        .expect("list after baseline upload")
        .get(Path::new("notes.txt"))
        .expect("baseline file should be listed")
        .id
        .clone();
    let mut base_index = HashMap::new();
    base_index.insert(
        PathBuf::from("notes.txt"),
        FileRecord::from_local(
            PathBuf::from("notes.txt"),
            &baseline_local,
            Some(baseline_remote_id),
            SyncStatus::Synced,
        ),
    );

    // Diverge the local copy from the baseline.
    fs::write(&local_path, b"locally edited content\n").expect("write local divergence");

    // Diverge the remote copy from the baseline by re-uploading a same-named file from
    // a separate source directory (`upload` always names the remote file after the
    // local source's own basename). This exercises upload-to-an-existing-path
    // revisioning, which the daemon relies on whenever a previously synced file
    // changes again.
    let remote_divergence_dir = tempdir().expect("remote divergence source directory");
    let remote_divergence_path = remote_divergence_dir.path().join("notes.txt");
    fs::write(&remote_divergence_path, b"remotely edited content\n")
        .expect("write remote divergence fixture");
    client
        .upload(&remote_divergence_path, &run_root, Path::new("notes.txt"))
        .expect("live remote divergence upload should succeed");
    let after_remote_divergence = client
        .list(&run_root)
        .expect("list after remote divergence upload");
    assert_eq!(
        after_remote_divergence.len(),
        1,
        "re-uploading to an existing remote path should update it in place rather than \
         create a duplicate entry, but the run folder now lists: {after_remote_divergence:?}"
    );

    // Capture real local/remote state exactly as the daemon would before reconciling.
    let local_entities =
        scan_local_entities(local_dir.path()).expect("scan local entities before reconcile");
    let remote_entities = client
        .list_entities(&run_root)
        .expect("list_entities before reconcile");

    let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);
    let conflict = planned
        .iter()
        .find(|action| action.path == Path::new("notes.txt"))
        .expect("notes.txt should have a planned action");
    assert_eq!(
        conflict.action,
        SyncAction::Conflict,
        "diverging both sides from a synced baseline should plan a conflict: {planned:?}"
    );
    let conflict_path = conflict
        .conflict_path
        .clone()
        .expect("a conflict action should carry a sidecar conflict_path");
    assert!(
        is_conflict_copy(&conflict_path),
        "the planned conflict_path should look like a conflict sidecar: {}",
        conflict_path.display()
    );
    assert_eq!(
        original_from_conflict_copy(&conflict_path).as_deref(),
        Some(Path::new("notes.txt")),
        "the conflict sidecar name should map back to the original path"
    );

    // Execute the conflict resolution against the real service the same way the
    // daemon's `SyncAction::Conflict` execution arm does: download the remote's
    // competing version into the sidecar path, leaving the local file untouched.
    let sidecar_destination = local_dir.path().join(&conflict_path);
    client
        .download(&run_root.join("notes.txt"), &sidecar_destination)
        .expect("live conflict sidecar download should succeed");
    let sidecar_content = fs::read(&sidecar_destination).expect("read downloaded sidecar");
    assert_eq!(
        sidecar_content, b"remotely edited content\n",
        "the sidecar copy should contain the remote's competing content"
    );
    let local_content = fs::read(&local_path).expect("read local file after conflict resolution");
    assert_eq!(
        local_content, b"locally edited content\n",
        "the original local file should be left untouched by conflict resolution"
    );

    cleanup.completed = true;
}

fn parse_live_e2e_config(
    enabled: Option<OsString>,
    remote_root: Option<OsString>,
    executable: Option<OsString>,
    timeout_secs: Option<OsString>,
    list_attempts: Option<OsString>,
) -> Result<LiveE2eConfig, String> {
    if enabled.as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Err("set PROTON_SYNC_LIVE_E2E=1 to enable mutating live E2E tests".to_owned());
    }
    let remote_root = remote_root
        .map(PathBuf::from)
        .ok_or_else(|| "set PROTON_SYNC_LIVE_REMOTE_ROOT to a disposable test folder".to_owned())?;
    validate_live_e2e_remote_root(&remote_root)?;
    let executable = executable
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("proton-drive"));
    let timeout_secs = parse_positive_u64_env(timeout_secs, 30, "PROTON_SYNC_LIVE_TIMEOUT_SECS")?;
    let list_attempts =
        parse_positive_usize_env(list_attempts, 2, "PROTON_SYNC_LIVE_LIST_ATTEMPTS")?;

    Ok(LiveE2eConfig {
        remote_root,
        executable,
        timeout_secs,
        list_attempts,
    })
}

fn validate_live_e2e_remote_root(remote_root: &Path) -> Result<(), String> {
    if !remote_root.is_absolute() {
        return Err("PROTON_SYNC_LIVE_REMOTE_ROOT must be an absolute path \
             (e.g. /Drive/proton-sync-e2e-demo)"
            .to_owned());
    }
    let name = remote_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "PROTON_SYNC_LIVE_REMOTE_ROOT must have a valid folder name".to_owned())?;
    if !name.starts_with(LIVE_E2E_REMOTE_ROOT_PREFIX) {
        return Err(format!(
            "PROTON_SYNC_LIVE_REMOTE_ROOT basename must start with {LIVE_E2E_REMOTE_ROOT_PREFIX}"
        ));
    }
    Ok(())
}

fn unique_live_run_folder(test_name: &str, epoch_secs: u64, process_id: u32) -> PathBuf {
    PathBuf::from(format!(
        "run-{epoch_secs}-{process_id}-{}",
        sanitize_run_component(test_name)
    ))
}

fn sanitize_run_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[test]
fn live_policy_env_uses_defaults_when_unset() {
    assert_eq!(parse_positive_u64_env(None, 30, "TIMEOUT"), Ok(30));
    assert_eq!(parse_positive_usize_env(None, 2, "ATTEMPTS"), Ok(2));
}

#[test]
fn live_policy_env_accepts_positive_values() {
    assert_eq!(
        parse_positive_u64_env(Some(OsString::from("45")), 30, "TIMEOUT"),
        Ok(45)
    );
    assert_eq!(
        parse_positive_usize_env(Some(OsString::from("4")), 2, "ATTEMPTS"),
        Ok(4)
    );
}

#[test]
fn live_policy_env_rejects_zero_values() {
    assert_eq!(
        parse_positive_u64_env(Some(OsString::from("0")), 30, "TIMEOUT"),
        Err("TIMEOUT must be greater than zero".to_owned())
    );
    assert_eq!(
        parse_positive_usize_env(Some(OsString::from("0")), 2, "ATTEMPTS"),
        Err("ATTEMPTS must be greater than zero".to_owned())
    );
}

#[test]
fn mutating_live_e2e_gate_requires_explicit_enablement() {
    assert_eq!(
        parse_live_e2e_config(
            None,
            Some(OsString::from("/Drive/proton-sync-e2e-demo")),
            None,
            None,
            None,
        ),
        Err("set PROTON_SYNC_LIVE_E2E=1 to enable mutating live E2E tests".to_owned())
    );
}

#[test]
fn mutating_live_e2e_gate_requires_disposable_remote_root_prefix() {
    assert_eq!(
        parse_live_e2e_config(
            Some(OsString::from("1")),
            Some(OsString::from("/Drive/Important")),
            None,
            None,
            None,
        ),
        Err("PROTON_SYNC_LIVE_REMOTE_ROOT basename must start with proton-sync-e2e-".to_owned())
    );
}

#[test]
fn mutating_live_e2e_gate_rejects_a_relative_remote_root() {
    // A value with the disposable prefix but no leading `/` must still be refused, since a
    // relative path could be resolved by the proton-drive CLI against an unintended base.
    assert_eq!(
        parse_live_e2e_config(
            Some(OsString::from("1")),
            Some(OsString::from("proton-sync-e2e-demo")),
            None,
            None,
            None,
        ),
        Err("PROTON_SYNC_LIVE_REMOTE_ROOT must be an absolute path \
             (e.g. /Drive/proton-sync-e2e-demo)"
            .to_owned())
    );
}

#[test]
fn mutating_live_e2e_gate_accepts_safe_configuration() {
    let config = parse_live_e2e_config(
        Some(OsString::from("1")),
        Some(OsString::from("/Drive/proton-sync-e2e-demo")),
        Some(OsString::from("/usr/local/bin/proton-drive")),
        Some(OsString::from("45")),
        Some(OsString::from("4")),
    )
    .expect("safe live E2E config");

    assert_eq!(
        config.remote_root,
        PathBuf::from("/Drive/proton-sync-e2e-demo")
    );
    assert_eq!(
        config.executable,
        PathBuf::from("/usr/local/bin/proton-drive")
    );
    assert_eq!(config.timeout_secs, 45);
    assert_eq!(config.list_attempts, 4);
}

#[test]
fn live_run_folder_names_are_unique_and_path_safe() {
    assert_eq!(
        unique_live_run_folder("pause/resume uploads", 1_786_224_000, 42),
        PathBuf::from("run-1786224000-42-pause-resume-uploads")
    );
}
