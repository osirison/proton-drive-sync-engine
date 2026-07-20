#[cfg(unix)]
mod unix_tests {
    use serde_json::Value;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use tempfile::tempdir;

    #[test]
    fn dry_run_cli_outputs_report_without_creating_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("local-only.txt"), b"local").expect("local file");
        let db_path = local_root.join("custom-index.db");
        let fake_proton_drive = write_fake_proton_drive(
            directory.path(),
            "/Drive/RemoteFolder",
            r#"    {
      "id": "remote-only-id",
      "name": "remote-only.txt",
      "path": "/Drive/RemoteFolder/remote-only.txt",
      "activeRevision": {
        "claimedDigests": {
          "sha1": "1111111111111111111111111111111111111111"
        }
      }
    }"#,
        );

        let output = Command::new(env!("CARGO_BIN_EXE_proton-syncd"))
            .arg("--local-root")
            .arg(&local_root)
            .arg("--remote-root")
            .arg("/Drive/RemoteFolder")
            .arg("--db-path")
            .arg(&db_path)
            .arg("--proton-cli")
            .arg(&fake_proton_drive)
            .arg("--dry-run")
            .env("RUST_LOG", "error")
            .output()
            .expect("run proton-syncd dry-run");

        assert_success(&output);
        assert!(
            !db_path.exists(),
            "dry-run must not create or update the configured index"
        );
        let report = parse_report(&output.stdout);
        let plan = plan(&report);

        assert_eq!(report["summary"]["total"].as_u64(), Some(2));
        assert_eq!(report["summary"]["uploads"].as_u64(), Some(1));
        assert_eq!(report["summary"]["downloads"].as_u64(), Some(1));
        assert_eq!(report["summary"]["destructive_actions"].as_u64(), Some(0));
        assert!(
            plan.iter().any(|action| {
                action["path"] == "local-only.txt" && action["action"] == "upload"
            }),
            "local-only file should be planned for upload: {plan:?}"
        );
        assert!(
            plan.iter().any(|action| {
                action["path"] == "remote-only.txt"
                    && action["action"] == "download"
                    && action["remote_id"] == "remote-only-id"
            }),
            "remote-only file should be planned for download: {plan:?}"
        );
    }

    #[test]
    fn config_file_drives_dry_run_cli() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("config-local.txt"), b"local").expect("local file");
        let db_path = directory.path().join("configured-index.db");
        let fake_proton_drive = write_fake_proton_drive(
            directory.path(),
            "/Drive/ConfiguredRoot",
            r#"    {
      "id": "config-remote-id",
      "name": "config-remote.txt",
      "path": "/Drive/ConfiguredRoot/config-remote.txt",
      "activeRevision": {
        "claimedDigests": {
          "sha1": "2222222222222222222222222222222222222222"
        }
      }
    }"#,
        );
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            format!(
                r#"local_root = "{}"
remote_root = "/Drive/ConfiguredRoot"
db_path = "{}"
proton_cli = "{}"
dry_run = true
"#,
                local_root.display(),
                db_path.display(),
                fake_proton_drive.display()
            ),
        )
        .expect("write config");

        let output = Command::new(env!("CARGO_BIN_EXE_proton-syncd"))
            .arg("--config")
            .arg(&config_path)
            .env("RUST_LOG", "error")
            .output()
            .expect("run proton-syncd dry-run from config");

        assert_success(&output);
        assert!(
            !db_path.exists(),
            "config-file dry-run must not create or update the configured index"
        );
        let report = parse_report(&output.stdout);
        let plan = plan(&report);

        assert_eq!(report["summary"]["total"].as_u64(), Some(2));
        assert!(
            plan.iter().any(|action| {
                action["path"] == "config-local.txt" && action["action"] == "upload"
            }),
            "config local file should be planned for upload: {plan:?}"
        );
        assert!(
            plan.iter().any(|action| {
                action["path"] == "config-remote.txt" && action["remote_id"] == "config-remote-id"
            }),
            "config remote file should be planned for download: {plan:?}"
        );
    }

    #[test]
    fn dry_run_cli_applies_include_and_exclude_filters() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("Documents")).expect("documents root");
        fs::create_dir_all(local_root.join("Other")).expect("other root");
        fs::write(local_root.join("Documents/local-keep.md"), b"keep").expect("kept local file");
        fs::write(local_root.join("Documents/local-skip.tmp"), b"skip").expect("tmp local file");
        fs::write(local_root.join("Other/local-ignore.md"), b"ignore").expect("ignored local file");
        let db_path = local_root.join("custom-index.db");
        let fake_proton_drive = write_fake_proton_drive(
            directory.path(),
            "/Drive/RemoteFolder",
            r#"    {
      "id": "remote-keep-id",
      "name": "remote-keep.md",
      "path": "/Drive/RemoteFolder/Documents/remote-keep.md",
      "activeRevision": {
        "claimedDigests": {
          "sha1": "3333333333333333333333333333333333333333"
        }
      }
    },
    {
      "id": "remote-skip-id",
      "name": "remote-skip.tmp",
      "path": "/Drive/RemoteFolder/Documents/remote-skip.tmp",
      "activeRevision": {
        "claimedDigests": {
          "sha1": "4444444444444444444444444444444444444444"
        }
      }
    },
    {
      "id": "remote-ignore-id",
      "name": "remote-ignore.md",
      "path": "/Drive/RemoteFolder/Other/remote-ignore.md",
      "activeRevision": {
        "claimedDigests": {
          "sha1": "5555555555555555555555555555555555555555"
        }
      }
    }"#,
        );

        let output = Command::new(env!("CARGO_BIN_EXE_proton-syncd"))
            .arg("--local-root")
            .arg(&local_root)
            .arg("--remote-root")
            .arg("/Drive/RemoteFolder")
            .arg("--db-path")
            .arg(&db_path)
            .arg("--proton-cli")
            .arg(&fake_proton_drive)
            .arg("--include")
            .arg("Documents/**")
            .arg("--exclude")
            .arg("**/*.tmp")
            .arg("--dry-run")
            .env("RUST_LOG", "error")
            .output()
            .expect("run proton-syncd filtered dry-run");

        assert_success(&output);
        let report = parse_report(&output.stdout);
        let plan = plan(&report);

        assert_eq!(report["summary"]["total"].as_u64(), Some(3));
        assert_eq!(
            report["summary"]["remote_directories_created"].as_u64(),
            Some(1)
        );
        assert_eq!(
            report["summary"]["local_directories_created"].as_u64(),
            Some(0)
        );
        assert!(
            plan.iter().any(|action| {
                action["path"] == "Documents/local-keep.md" && action["action"] == "upload"
            }),
            "included local file should be planned for upload: {plan:?}"
        );
        assert!(
            plan.iter().any(|action| {
                action["path"] == "Documents/remote-keep.md" && action["action"] == "download"
            }),
            "included remote file should be planned for download: {plan:?}"
        );
        assert!(
            plan.iter().any(|action| {
                action["path"] == "Documents"
                    && action["action"] == "create_remote_directory"
                    && action["entity_kind"] == "directory"
            }),
            "included local directory should be planned for remote creation: {plan:?}"
        );
        assert!(
            plan.iter().all(|action| !action["path"]
                .as_str()
                .unwrap_or_default()
                .ends_with(".tmp")),
            "excluded tmp paths should not appear in the plan: {plan:?}"
        );
        assert!(
            plan.iter().all(|action| !action["path"]
                .as_str()
                .unwrap_or_default()
                .starts_with("Other/")),
            "non-included paths should not appear in the plan: {plan:?}"
        );
    }

    fn assert_success(output: &Output) {
        assert!(
            output.status.success(),
            "dry-run should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn parse_report(stdout: &[u8]) -> Value {
        serde_json::from_slice(stdout).expect("dry-run JSON report")
    }

    fn plan(report: &Value) -> &[Value] {
        report["plan"].as_array().expect("dry-run report plan")
    }

    fn write_fake_proton_drive(directory: &Path, remote_root: &str, entries: &str) -> PathBuf {
        let path = directory.join("fake-proton-drive");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "{remote_root}" ]; then
  cat <<'JSON'
{{
  "entries": [
{entries}
  ]
}}
JSON
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
