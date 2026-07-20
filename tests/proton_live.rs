use proton_drive_sync_engine::proton::{CommandPolicy, ProtonClient, ProtonDriveClient};
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
