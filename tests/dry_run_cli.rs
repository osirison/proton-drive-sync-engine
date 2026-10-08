mod common;

#[cfg(unix)]
mod unix_tests {
    use crate::common;
    use serde_json::Value;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::time::Duration;
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

        let output = run_to_completion(
            syncd_command(directory.path())
                .arg("--local-root")
                .arg(&local_root)
                .arg("--remote-root")
                .arg("/Drive/RemoteFolder")
                .arg("--db-path")
                .arg(&db_path)
                .arg("--proton-cli")
                .arg(&fake_proton_drive)
                .arg("--dry-run"),
        );

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
    fn dry_run_cli_plans_remote_root_creation_when_root_is_missing() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let db_path = local_root.join("custom-index.db");
        let fake_proton_drive = write_missing_root_proton_drive(
            directory.path(),
            "/my-files/demo/",
            "Node not found: demo",
        );

        let output = run_to_completion(
            syncd_command(directory.path())
                .arg("--local-root")
                .arg(&local_root)
                .arg("--remote-root")
                .arg("/my-files/demo/")
                .arg("--db-path")
                .arg(&db_path)
                .arg("--proton-cli")
                .arg(&fake_proton_drive)
                .arg("--dry-run"),
        );

        assert_success(&output);
        assert!(
            !db_path.exists(),
            "dry-run must not create or update the configured index"
        );
        let report = parse_report(&output.stdout);
        let plan = plan(&report);

        assert_eq!(report["summary"]["total"].as_u64(), Some(1));
        assert_eq!(
            report["summary"]["remote_directories_created"].as_u64(),
            Some(1)
        );
        assert!(
            plan.iter().any(|action| {
                action["path"] == ""
                    && action["action"] == "create_remote_directory"
                    && action["entity_kind"] == "directory"
            }),
            "missing configured remote root should be represented as a remote directory creation: {plan:?}"
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

        let output = run_to_completion(
            syncd_command(directory.path())
                .arg("--config")
                .arg(&config_path),
        );

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

        let output = run_to_completion(
            syncd_command(directory.path())
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
                .arg("--dry-run"),
        );

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

    /// #300 at the boundary that broke: `proton-syncd --dry-run` writing the report to stdout.
    /// One non-UTF-8 filename anywhere under the root used to fail the WHOLE document
    /// (`Error("path contains invalid UTF-8 characters")`), exit 1 with nothing printed, and blank
    /// the GUI's plan screen — and #270 guaranteed such a path always reaches the report by
    /// planning it rather than dropping it.
    #[test]
    fn dry_run_cli_reports_a_plan_containing_a_non_utf8_filename() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("ordinary.txt"), b"local").expect("utf-8 local file");
        let odd_name = OsStr::from_bytes(b"caf\xe9.txt");
        fs::write(local_root.join(odd_name), b"odd").expect("non-UTF-8 local file");
        let db_path = local_root.join("custom-index.db");
        let fake_proton_drive =
            write_fake_proton_drive(directory.path(), "/Drive/RemoteFolder", "");

        let output = run_to_completion(
            syncd_command(directory.path())
                .arg("--local-root")
                .arg(&local_root)
                .arg("--remote-root")
                .arg("/Drive/RemoteFolder")
                .arg("--db-path")
                .arg(&db_path)
                .arg("--proton-cli")
                .arg(&fake_proton_drive)
                .arg("--dry-run"),
        );

        assert_success(&output);
        let report = parse_report(&output.stdout);
        let plan = plan(&report);

        assert_eq!(report["summary"]["total"].as_u64(), Some(2));
        assert_eq!(report["summary"]["uploads"].as_u64(), Some(1));
        assert_eq!(report["summary"]["skipped_unsupported"].as_u64(), Some(1));
        assert!(
            plan.iter()
                .any(|action| action["path"] == "ordinary.txt" && action["action"] == "upload"),
            "the odd filename must cost the report one ROW, not the whole document: {plan:?}"
        );
        let skipped = plan
            .iter()
            .find(|action| action["action"] == "skip_unsupported")
            .expect("the unsyncable path is reported rather than dropped");
        assert_eq!(
            skipped["path"].as_str(),
            Some("caf\u{fffd}.txt"),
            "the row carries the lossy rendering — a display form, never a selector"
        );
        assert_eq!(
            skipped["skip_reason"].as_str(),
            Some("unrepresentable_path"),
            "and it says why, so the row is actionable (#295)"
        );
    }

    // ---------------------------------------------------------------------------------------
    // #102 phase 4c: `--dry-run` with several pairs previews ONE pair, the default unless
    // `--pair` names another. The fake CLI answers per remote root, so a preview of the wrong
    // pair is visible in the report rather than only in the flags.
    // ---------------------------------------------------------------------------------------

    /// A fake `proton-drive` for two remote roots, each with one remote-only file:
    /// `/Drive/A` has `a-remote.txt`, `/Drive/B` has `b-remote.txt`. Any other root is unexpected.
    fn write_two_root_proton_drive(directory: &Path) -> PathBuf {
        let path = directory.join("fake-two-root-proton-drive");
        let entry = |name: &str, root: &str, id: &str, sha1: &str| {
            format!(
                r#"{{"id":"{id}","name":"{name}","path":"{root}/{name}","activeRevision":{{"claimedDigests":{{"sha1":"{sha1}"}}}}}}"#
            )
        };
        let a = entry(
            "a-remote.txt",
            "/Drive/A",
            "a-remote-id",
            "1111111111111111111111111111111111111111",
        );
        let b = entry(
            "b-remote.txt",
            "/Drive/B",
            "b-remote-id",
            "2222222222222222222222222222222222222222",
        );
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
  case "$4" in
    /Drive/A) printf '{{"entries":[%s]}}\n' '{a}'; exit 0 ;;
    /Drive/B) printf '{{"entries":[%s]}}\n' '{b}'; exit 0 ;;
  esac
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

    /// Two pairs, `a` (the default) and `b`, each with a local-only file, plus the fake above.
    /// Returns the config path and both index paths (which a preview must never create).
    fn two_pair_dry_run_setup(directory: &Path, extra: &str) -> (PathBuf, PathBuf, PathBuf) {
        let fake = write_two_root_proton_drive(directory);
        let mut db_paths = Vec::new();
        let mut text = format!("proton_cli = \"{}\"\n{extra}", fake.display());
        for (name, remote) in [("a", "/Drive/A"), ("b", "/Drive/B")] {
            let local_root = directory.join(name).join("local");
            fs::create_dir_all(&local_root).expect("local root");
            fs::write(local_root.join(format!("{name}-local.txt")), b"local").expect("local file");
            let db_path = directory.join(name).join("state").join("index.db");
            text.push_str(&format!(
                "\n[[pair]]\nname = \"{name}\"\nlocal_root = \"{}\"\nremote_root = \"{remote}\"\n\
                 db_path = \"{}\"\nlockfile_path = \"{}\"\n",
                local_root.display(),
                db_path.display(),
                directory
                    .join(name)
                    .join("state")
                    .join("daemon.lock")
                    .display(),
            ));
            db_paths.push(db_path);
        }
        let config = directory.join("pairs.toml");
        fs::write(&config, text).expect("write config");
        let b = db_paths.pop().expect("b");
        let a = db_paths.pop().expect("a");
        (config, a, b)
    }

    /// How long a preview or a refusal may take. Both are over in well under a second; a run
    /// still going after this is no longer one of those.
    const RUN_BOUND: Duration = Duration::from_secs(20);

    /// A `proton-syncd` command **in a sandbox of its own**, for [`run_to_completion`].
    ///
    /// Every process-global default a daemon would reach is pointed into `directory`
    /// (`common::sandboxed`: `HOME`, the runtime dir, the state dir with the user-global lock,
    /// the data dir with the trash) and the control socket is named explicitly as well. These
    /// tests exist to show that a run is previewed or REFUSED, and the way such a test fails is
    /// that the run turns into a real daemon: one that bound the machine's default socket would
    /// replace the live daemon's control socket and delete it on exit. (That happened, once,
    /// under a deliberately broken `--pair` check.) So the sandbox is not optional, and it lives
    /// in one place: every run in this file starts here.
    fn syncd_command(directory: &Path) -> Command {
        let mut command = common::sandboxed(env!("CARGO_BIN_EXE_proton-syncd"), directory);
        command
            .arg("--socket-path")
            .arg(directory.join("never-bound.sock"))
            .env("RUST_LOG", "error");
        command
    }

    /// Runs `command` to completion, bounded: a child still running after [`RUN_BOUND`] is killed
    /// and the test fails saying so, instead of hanging.
    fn run_to_completion(command: &mut Command) -> Output {
        common::run_bounded(command, RUN_BOUND)
    }

    /// `proton-syncd --config <config> <args>`, run to completion in [`syncd_command`]'s sandbox.
    fn run_syncd(config: &Path, directory: &Path, args: &[&str]) -> Output {
        run_to_completion(
            syncd_command(directory)
                .arg("--config")
                .arg(config)
                .args(args),
        )
    }

    fn preview(config: &Path, directory: &Path, extra_args: &[&str]) -> Output {
        let mut args = vec!["--dry-run"];
        args.extend_from_slice(extra_args);
        run_syncd(config, directory, &args)
    }

    fn planned_paths(report: &Value) -> Vec<String> {
        plan(report)
            .iter()
            .filter_map(|action| action["path"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn dry_run_previews_the_default_pair_unless_pair_names_another() {
        let directory = tempdir().expect("tempdir");
        let (config, a_db, b_db) = two_pair_dry_run_setup(directory.path(), "");

        // No selector: the default pair, the first table.
        let output = preview(&config, directory.path(), &[]);
        assert_success(&output);
        let paths = planned_paths(&parse_report(&output.stdout));
        assert_eq!(
            {
                let mut sorted = paths.clone();
                sorted.sort();
                sorted
            },
            ["a-local.txt", "a-remote.txt"],
            "the default pair's plan and nothing of b's: {paths:?}"
        );

        // `--pair b`: the other one, and nothing of a's.
        let output = preview(&config, directory.path(), &["--pair", "b"]);
        assert_success(&output);
        let paths = planned_paths(&parse_report(&output.stdout));
        assert_eq!(
            {
                let mut sorted = paths.clone();
                sorted.sort();
                sorted
            },
            ["b-local.txt", "b-remote.txt"],
            "b's plan and nothing of a's: {paths:?}"
        );
        assert!(
            !a_db.exists() && !b_db.exists(),
            "a preview creates no index, for either pair"
        );
    }

    #[test]
    fn an_unknown_preview_pair_is_refused_naming_the_configured_pairs() {
        let directory = tempdir().expect("tempdir");
        let (config, _, _) = two_pair_dry_run_setup(directory.path(), "");

        let output = preview(&config, directory.path(), &["--pair", "nope"]);
        assert!(
            !output.status.success(),
            "an unknown pair must not fall back to the default one"
        );
        assert!(output.stdout.is_empty(), "and prints no report");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("`nope`") && stderr.contains("`a` and `b`"),
            "names what was asked for and what exists: {stderr}"
        );
        // Matched exactly, like the wire.
        let output = preview(&config, directory.path(), &["--pair", "B"]);
        assert!(!output.status.success(), "`B` is not `b`");

        // Without --dry-run there is nothing for --pair to select: the daemon runs every pair.
        let output = run_syncd(&config, directory.path(), &["--pair", "b"]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--pair only selects"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn pair_selects_a_preview_whether_dry_run_came_from_the_flag_or_the_file() {
        // One pair, `dry_run = true` in the FILE and no `--dry-run` on the command line: the run is
        // a preview, so `--pair` must be accepted. A clap `requires = "dry_run"` would refuse it
        // before the file was read.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("only-here.txt"), b"x").expect("local file");
        let fake = write_fake_proton_drive(directory.path(), "/Drive/Only", "");
        let config = directory.path().join("one.toml");
        fs::write(
            &config,
            format!(
                "local_root = \"{}\"\nremote_root = \"/Drive/Only\"\ndb_path = \"{}\"\n\
                 proton_cli = \"{}\"\ndry_run = true\n",
                local_root.display(),
                directory.path().join("index.db").display(),
                fake.display()
            ),
        )
        .expect("write config");

        let output = run_syncd(&config, directory.path(), &["--pair", "default"]);
        assert_success(&output);
        assert_eq!(
            planned_paths(&parse_report(&output.stdout)),
            ["only-here.txt"]
        );

        // And from the flag, over a file that says nothing.
        fs::write(
            &config,
            format!(
                "local_root = \"{}\"\nremote_root = \"/Drive/Only\"\ndb_path = \"{}\"\n\
                 proton_cli = \"{}\"\n",
                local_root.display(),
                directory.path().join("index.db").display(),
                fake.display()
            ),
        )
        .expect("rewrite config");
        let output = preview(&config, directory.path(), &["--pair", "default"]);
        assert_success(&output);
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

    fn write_missing_root_proton_drive(
        directory: &Path,
        remote_root: &str,
        stderr: &str,
    ) -> PathBuf {
        let path = directory.join("fake-proton-drive-missing-root");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ] && [ "$4" = "{remote_root}" ]; then
  echo "{stderr}" >&2
  exit 1
fi
echo "unexpected proton-drive args: $*" >&2
exit 64
"#
            ),
        )
        .expect("fake missing-root proton-drive script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }
}
