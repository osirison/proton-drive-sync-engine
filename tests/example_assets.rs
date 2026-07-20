use proton_drive_sync_engine::config::{DaemonConfigInput, resolve_runtime_config};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use toml::Value;

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
    assert_eq!(config.proton_timeout, Duration::from_secs(60));
    assert_eq!(config.proton_list_attempts, 2);
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

#[test]
fn example_systemd_installer_has_safe_defaults() {
    let script_path = manifest_path("examples/systemd/install-user-service.sh");
    let script = fs::read_to_string(&script_path).expect("example installer script");

    assert!(script.starts_with("#!/usr/bin/env bash"));
    assert!(script.contains("set -euo pipefail"));
    assert!(script.contains("install -m 600"));
    assert!(script.contains("install -m 644"));
    assert!(script.contains("systemctl --user daemon-reload"));
    assert!(script.contains("--force-config"));
    assert!(script.contains("--enable"));
    assert!(script.contains("--start"));
    assert!(
        !script.contains("sudo"),
        "user service installer must not require elevated privileges"
    );

    #[cfg(unix)]
    {
        let mode = fs::metadata(&script_path)
            .expect("installer metadata")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "installer should be executable");
    }
}

#[test]
fn release_asset_manifest_points_at_existing_distribution_assets() {
    let manifest = fs::read_to_string(manifest_path("examples/packaging/release-assets.toml"))
        .expect("release asset manifest");
    let manifest = toml::from_str::<Value>(&manifest).expect("parse release manifest");

    assert_eq!(
        manifest["binaries"]["daemon"].as_str(),
        Some("proton-syncd")
    );
    assert_eq!(
        manifest["binaries"]["control_cli"].as_str(),
        Some("proton-sync")
    );

    for key_path in [
        ["config", "sample"],
        ["systemd", "unit"],
        ["systemd", "install_helper"],
        ["packaging", "archive_helper"],
    ] {
        let asset_path = manifest[key_path[0]][key_path[1]]
            .as_str()
            .expect("manifest asset path");
        assert!(
            fs::metadata(manifest_path(asset_path)).is_ok(),
            "missing release asset {asset_path}"
        );
    }
}

#[test]
fn release_archive_helper_has_expected_packaging_contract() {
    let script_path = manifest_path("examples/packaging/build-release-archive.sh");
    let script = fs::read_to_string(&script_path).expect("release archive helper");

    assert!(script.starts_with("#!/usr/bin/env bash"));
    assert!(script.contains("set -euo pipefail"));
    assert!(script.contains("cargo build"));
    assert!(script.contains("--release --bins --locked"));
    assert!(script.contains("tar -C"));
    assert!(script.contains("proton-syncd"));
    assert!(script.contains("proton-sync"));
    assert!(script.contains("install-user-service.sh"));

    #[cfg(unix)]
    {
        let mode = fs::metadata(&script_path)
            .expect("archive helper metadata")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "archive helper should be executable");
    }
}

fn manifest_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}
