use proton_drive_sync_engine::index::compute_sha1;
use proton_drive_sync_engine::proton::{CommandPolicy, ProtonClient, ProtonDriveClient};
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
