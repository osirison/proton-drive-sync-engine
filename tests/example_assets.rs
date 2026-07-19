use proton_drive_sync_engine::config::{DaemonConfigInput, resolve_runtime_config};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[test]
fn example_config_resolves_to_valid_daemon_config() {
    let config_path = manifest_path("examples/proton-sync.toml");

    let (config, dry_run) = resolve_runtime_config(DaemonConfigInput {
        config: Some(config_path),
        ..DaemonConfigInput::default()
    })
    .expect("example config should resolve");

    assert!(!dry_run);
    assert_eq!(config.local_root, PathBuf::from("/home/me/ProtonDrive"));
    assert_eq!(config.remote_root, PathBuf::from("/Drive/RemoteFolder"));
    assert_eq!(config.scan_interval, Duration::from_secs(300));
    assert_eq!(config.proton_cli, PathBuf::from("proton-drive"));
    assert_eq!(
        config.include_patterns,
        vec!["Documents/**", "Projects/**/*.md"]
    );
    assert_eq!(config.exclude_patterns, vec!["**/*.tmp", "**/.DS_Store"]);
}

#[test]
fn example_systemd_service_points_at_example_config_location() {
    let service = fs::read_to_string(manifest_path("examples/systemd/proton-syncd.service"))
        .expect("example systemd service");

    assert!(service.contains("[Unit]"));
    assert!(service.contains("[Service]"));
    assert!(service.contains("[Install]"));
    assert!(service.contains("Environment=RUST_LOG=info"));
    assert!(service.contains(
        "ExecStart=%h/.cargo/bin/proton-syncd --config %h/.config/proton-sync/proton-sync.toml"
    ));
    assert!(service.contains("Restart=on-failure"));
    assert!(
        !service.contains("--dry-run"),
        "sample service should run the daemon, not dry-run mode"
    );
}

fn manifest_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}
