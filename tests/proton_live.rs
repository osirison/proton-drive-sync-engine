use proton_drive_sync_engine::proton::{ProtonClient, ProtonDriveClient};
use std::env;
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
    let client = ProtonDriveClient::with_timeout(executable, Duration::from_secs(30));

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
