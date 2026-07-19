#[cfg(unix)]
mod unix_tests {
    use serde_json::Value;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn dry_run_cli_outputs_plan_without_creating_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("local-only.txt"), b"local").expect("local file");
        let db_path = local_root.join("custom-index.db");
        let fake_proton_drive = write_fake_proton_drive(directory.path());

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

        assert!(
            output.status.success(),
            "dry-run should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !db_path.exists(),
            "dry-run must not create or update the configured index"
        );
        let plan: Vec<Value> = serde_json::from_slice(&output.stdout).expect("dry-run JSON plan");

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

    fn write_fake_proton_drive(directory: &Path) -> PathBuf {
        let path = directory.join("fake-proton-drive");
        fs::write(
            &path,
            r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "/Drive/RemoteFolder" ]; then
  cat <<'JSON'
{
  "entries": [
    {
      "id": "remote-only-id",
      "name": "remote-only.txt",
      "path": "/Drive/RemoteFolder/remote-only.txt",
      "activeRevision": {
        "claimedDigests": {
          "sha1": "1111111111111111111111111111111111111111"
        }
      }
    }
  ]
}
JSON
  exit 0
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#,
        )
        .expect("fake proton-drive script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }
}
