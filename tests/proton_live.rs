use proton_drive_sync_engine::proton::{CommandPolicy, ProtonClient, ProtonDriveClient};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

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
