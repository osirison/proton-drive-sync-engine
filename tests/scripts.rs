//! The shell scripts' reading of a config file (`setup.sh`'s `config_values`, which `uninstall.sh`
//! uses to find every pair's `.sync`), and what `uninstall.sh --dry-run` makes of it.
//!
//! **Nothing here touches the machine.** The helpers are sourced from `setup.sh` (its `main` is
//! guarded, so sourcing only defines functions) and read a temporary file; the end-to-end runs are
//! `uninstall.sh --dry-run` only, which returns before anything is removed, and `setup.sh`'s
//! `write_config` over a config that already exists, which only reads it. Those run in an
//! environment that is cleared and rebuilt inside the test's directory with `systemctl` and `cargo`
//! shadowed by stubs, so they read nothing from the machine either. That is the sandbox the PR
//! description gives for running the script by hand.
#![cfg(unix)]

mod common;

// The apt channel and the verification jobs (openspec change add-linux-release-pipeline, tasks 6.3
// and 6.6). They use this file's `PublishTree`, stubs and workflow readers through `super::*`.
#[path = "scripts/apt.rs"]
mod apt;
#[path = "scripts/apt_publish.rs"]
mod apt_publish;
#[path = "scripts/verify.rs"]
mod verify;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::tempdir;

/// Runs `bash -c <script>` with `setup.sh` sourced, in the test's own sandbox, and returns stdout.
fn bash_with_setup(directory: &Path, script: &str) -> String {
    let setup = Path::new(env!("CARGO_MANIFEST_DIR")).join("setup.sh");
    let mut command = common::sandboxed("bash", directory);
    command
        .arg("-c")
        .arg(format!("source \"$1\"; {script}"))
        .arg("bash")
        .arg(setup);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(
        output.status.success(),
        "bash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

fn values(directory: &Path, config: &str, key: &str) -> Vec<String> {
    let file = directory.join("config.toml");
    fs::write(&file, config).expect("write config");
    let mut command = common::sandboxed("bash", directory);
    command
        .arg("-c")
        .arg("source \"$1\"; config_values \"$2\" \"$3\"")
        .arg("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("setup.sh"))
        .arg(&file)
        .arg(key);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn has_inline_pairs(directory: &Path, config: &str) -> bool {
    let file = directory.join("inline.toml");
    fs::write(&file, config).expect("write config");
    let mut command = common::sandboxed("bash", directory);
    command
        .arg("-c")
        .arg("source \"$1\"; if config_has_inline_pairs \"$2\"; then echo yes; else echo no; fi")
        .arg("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("setup.sh"))
        .arg(&file);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("utf-8").trim() == "yes"
}

#[test]
fn config_values_reads_double_and_single_quoted_strings_in_file_order() {
    let directory = tempdir().expect("tempdir");
    let config = "\
# local_root = \"/commented/out\"
[[pair]]
name = \"docs\"
local_root = \"/home/me/Docs\"   # a trailing comment
[[pair]]
name = \"photos\"
local_root = '/home/me/Photos'
[[pair]]
name = \"odd\"
local_root = '/home/me/with space # and hash'
[[pair]]
name = \"hash\"
local_root = \"/home/me/a#b\"
";
    assert_eq!(
        values(directory.path(), config, "local_root"),
        [
            "/home/me/Docs",
            "/home/me/Photos",
            "/home/me/with space # and hash",
            "/home/me/a#b"
        ],
        "a single-quoted TOML string is a string, and a `#` inside quotes is not a comment"
    );
}

#[test]
fn config_values_gives_an_empty_value_as_an_empty_line_not_the_whole_line() {
    let directory = tempdir().expect("tempdir");
    assert_eq!(
        values(directory.path(), "local_root = \"\"\n", "local_root"),
        [""],
        "an empty string is empty; the caller drops empty values"
    );
}

#[test]
fn an_inline_pair_array_is_recognised_so_the_scripts_can_say_they_could_not_read_it() {
    let directory = tempdir().expect("tempdir");
    assert!(has_inline_pairs(
        directory.path(),
        "pair = [{ name = \"a\", local_root = \"/x\", remote_root = \"/Drive/x\" }]\n"
    ));
    assert!(has_inline_pairs(
        directory.path(),
        "  pair=[\n  { name = \"a\", local_root = \"/x\" },\n]\n"
    ));
    assert!(has_inline_pairs(directory.path(), "\"pair\" = []\n"));
    assert!(
        !has_inline_pairs(
            directory.path(),
            "[[pair]]\nname = \"a\"\nlocal_root = \"/x\"\n"
        ),
        "tables are what the scripts do read"
    );
    assert!(
        !has_inline_pairs(
            directory.path(),
            "# pair = [{ name = \"a\" }]\npair_count = 3\nlocal_root = \"/x\"\n"
        ),
        "a comment and a different key are not it"
    );
}

#[test]
fn the_number_of_pair_tables_is_counted_for_the_setup_preview_note() {
    let directory = tempdir().expect("tempdir");
    let file = directory.path().join("c.toml");
    fs::write(
        &file,
        "[[pair]]\nname = \"a\"\n[[pair]]\nname = \"b\"\n# [[pair]]\n",
    )
    .expect("write");
    let script = format!("count_pair_tables '{}'", file.display());
    assert_eq!(bash_with_setup(directory.path(), &script).trim(), "2");
    assert_eq!(
        bash_with_setup(
            directory.path(),
            "count_pair_tables /nonexistent/never.toml"
        )
        .trim(),
        "0",
        "no file, no tables, and no failure"
    );
}

#[test]
fn the_pair_table_count_and_the_inline_check_read_a_quoted_key_the_same_way() {
    // PR #434 third review, L6. `config_has_inline_pairs` accepted a quoted `pair` key and
    // `count_pair_tables` did not, so `[["pair"]]` tables counted as none: the preview note about
    // several pairs would not be said, and an inline check would call the same file inline-free.
    let directory = tempdir().expect("tempdir");
    let file = directory.path().join("quoted.toml");
    fs::write(
        &file,
        "[[pair]]\nname = \"a\"\n[[ pair ]]\nname = \"b\"\n[[\"pair\"]]\nname = \"c\"\n\
         [['pair']]\nname = \"d\"\n[[ \"pair\" ]] # a comment\nname = \"e\"\n\
         # [[\"pair\"]]\n[[pairs]]\n[[\"pair]]\n",
    )
    .expect("write");
    let script = format!("count_pair_tables '{}'", file.display());
    assert_eq!(
        bash_with_setup(directory.path(), &script).trim(),
        "5",
        "plain, spaced, double-quoted, single-quoted and quoted-with-a-comment tables; not a \
         comment, not `pairs`, not an unterminated quote"
    );
    assert!(has_inline_pairs(directory.path(), "'pair' = []\n"));
    assert!(has_inline_pairs(directory.path(), "\"pair\" = []\n"));
}

/// `setup.sh`'s start-up preview over `config`, with a stub in place of the daemon (it prints a
/// canned plan and starts nothing) and `--no-start` set, so the function ends at the preview.
/// Returns what it printed on both streams.
fn setup_preview(directory: &Path, config: &str) -> String {
    let stub = directory.join("stub-proton-syncd");
    fs::write(
        &stub,
        "#!/bin/sh\nprintf '{\"summary\":{\"uploads\":1,\"downloads\":2,\"destructive_actions\":0}}'\n",
    )
    .expect("stub daemon");
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).expect("stub mode");
    let file = directory.join("preview.toml");
    fs::write(&file, config).expect("write config");
    let mut command = common::sandboxed("bash", directory);
    command
        .arg("-c")
        .arg("source \"$1\"; no_start=true; assume_yes=false; preview_and_start \"$2\" \"$3\"")
        .arg("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("setup.sh"))
        .arg(&stub)
        .arg(&file);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("uploads=1 downloads=2"),
        "the canned plan was read: {text}"
    );
    text
}

#[test]
fn the_setup_preview_says_when_it_covered_only_the_default_pair() {
    let directory = tempdir().expect("tempdir");
    let one = setup_preview(
        directory.path(),
        "local_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
    );
    assert!(
        !one.contains("folder pairs"),
        "a one-pair config needs no caveat: {one}"
    );

    let tables = setup_preview(
        directory.path(),
        "[[pair]]\nname = \"a\"\n[[pair]]\nname = \"b\"\n[[pair]]\nname = \"c\"\n",
    );
    assert!(
        tables.contains("declares 3 folder pairs")
            && tables.contains("first (default) pair only")
            && tables.contains("--dry-run --pair NAME"),
        "{tables}"
    );

    let inline = setup_preview(
        directory.path(),
        "pair = [{ name = \"a\", local_root = \"/x\" }]\n",
    );
    assert!(
        inline.contains("inline array") && inline.contains("first (default) pair only"),
        "{inline}"
    );
}

/// `setup.sh`'s `write_config` over an EXISTING config file (so it only reads and reports; nothing
/// is written), with the folder pair the user just asked for. Returns what it printed on both
/// streams.
fn write_config_over(
    sandbox: &UninstallSandbox,
    existing: &str,
    local_root: &str,
    remote_root: &str,
) -> String {
    let file = sandbox.path().join("existing.toml");
    fs::write(&file, existing).expect("write config");
    let mut command = sandbox.command();
    command
        .arg("-c")
        .arg(
            "source \"$1\"; force_config=false; proton_cli=''; local_root=\"$3\"; \
             remote_root=\"$4\"; write_config \"$2\"",
        )
        .arg("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("setup.sh"))
        .arg(&file)
        .arg(local_root)
        .arg(remote_root);
    let output = common::run_bounded(&mut command, common::RUN_BOUND);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert_eq!(
        fs::read_to_string(&file).expect("read back"),
        existing,
        "an existing config is kept as it is"
    );
    text
}

#[test]
fn setup_says_when_an_existing_config_declares_its_pairs_in_a_way_it_cannot_check() {
    // PR #434 second review, H4. Removing the note from `write_config` survived every test: the
    // script would go on to compare nothing and say nothing, and a user who re-ran setup with a
    // different folder would be told their config was kept without being told it was not checked.
    let sandbox = UninstallSandbox::new();
    let inline = write_config_over(
        &sandbox,
        "pair = [{ name = \"a\", local_root = \"/x\", remote_root = \"/Drive/x\" }]\n",
        "/y",
        "/Drive/y",
    );
    assert!(
        inline.contains("declares its folder pairs as an inline array")
            && inline.contains("cannot read")
            && inline.contains("'/y' <-> '/Drive/y'"),
        "an inline array is named as unreadable, with the pair that was asked for: {inline}"
    );
    assert!(
        !inline.contains("NOT the"),
        "and it does not pretend to have compared: {inline}"
    );

    let tables = write_config_over(
        &sandbox,
        "[[pair]]\nname = \"a\"\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
        "/x",
        "/Drive/x",
    );
    assert!(
        !tables.contains("inline array") && !tables.contains("NOT the"),
        "a readable config that matches says nothing more: {tables}"
    );
    let differs = write_config_over(
        &sandbox,
        "[[pair]]\nname = \"a\"\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
        "/y",
        "/Drive/y",
    );
    assert!(
        differs.contains("NOT the") && !differs.contains("inline array"),
        "and one that differs is the mismatch warning, not the inline note: {differs}"
    );
}

/// A scratch tree for `uninstall.sh --dry-run`: every directory the script reads is under it, and
/// `systemctl` and `cargo` are stubs that report "nothing installed".
struct UninstallSandbox {
    directory: tempfile::TempDir,
}

impl UninstallSandbox {
    fn new() -> Self {
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        for name in [
            "home",
            "xdg-config",
            "xdg-data",
            "xdg-cache",
            "xdg-state",
            "runtime",
            "tmp",
            "bin",
        ] {
            fs::create_dir(root.join(name)).expect("sandbox directory");
        }
        for (name, body) in [
            ("systemctl", "#!/bin/sh\nexit 1\n"),
            ("cargo", "#!/bin/sh\nexit 0\n"),
        ] {
            let path = root.join("bin").join(name);
            fs::write(&path, body).expect("stub");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("stub mode");
        }
        Self { directory }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    /// A pair root with the `.sync` directory the script validates before it lists it.
    fn root_with_state(&self, name: &str) -> std::path::PathBuf {
        let root = self.path().join("roots").join(name);
        fs::create_dir_all(root.join(".sync")).expect("root with .sync");
        root
    }

    /// `bash` in the scratch tree's cleared environment: `HOME` and every XDG directory inside it,
    /// `TMPDIR` too, and `PATH` leading with the stub `secret-tool`/`curl` and then its own `bin`
    /// (the `systemctl` and `cargo` stubs). **Every script run that touches the uninstall sandbox
    /// starts here**, so a script that read the machine's environment would find the scratch
    /// tree's instead.
    fn command(&self) -> std::process::Command {
        let root = self.path();
        let mut command = common::sandboxed("bash", root);
        command
            .env_clear()
            .env("HOME", root.join("home"))
            .env(
                "PATH",
                common::path_with_stubs(&format!("{}:/usr/bin:/bin", root.join("bin").display())),
            )
            .env("XDG_CONFIG_HOME", root.join("xdg-config"))
            .env("XDG_DATA_HOME", root.join("xdg-data"))
            .env("XDG_CACHE_HOME", root.join("xdg-cache"))
            .env("XDG_STATE_HOME", root.join("xdg-state"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("TMPDIR", root.join("tmp"));
        command
    }

    /// `./uninstall.sh --dry-run --config <config>` and its combined output. Nothing is removed in
    /// a dry run, which is the only mode this ever uses.
    fn dry_run(&self, config: &Path) -> String {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("uninstall.sh");
        let mut command = self.command();
        command
            .arg(script)
            .arg("--dry-run")
            .arg("--config")
            .arg(config);
        let output = common::run_bounded(&mut command, common::RUN_BOUND);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{text}");
        assert!(
            text.contains("Dry run"),
            "the run ended at the dry-run return: {text}"
        );
        text
    }
}

#[test]
fn uninstall_dry_run_lists_every_pairs_state_directory_whatever_the_quoting() {
    let sandbox = UninstallSandbox::new();
    let docs = sandbox.root_with_state("docs");
    let photos = sandbox.root_with_state("photos");
    let config = sandbox.path().join("pairs.toml");
    fs::write(
        &config,
        format!(
            "[[pair]]\nname = \"docs\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/Docs\"\n\
             [[pair]]\nname = \"photos\"\nlocal_root = '{}'\nremote_root = '/Drive/Photos'\n",
            docs.display(),
            photos.display()
        ),
    )
    .expect("write config");

    let text = sandbox.dry_run(&config);
    for root in [&docs, &photos] {
        assert!(
            text.contains(&format!("{}", root.join(".sync").display())),
            "{} is listed for removal: {text}",
            root.display()
        );
    }
    assert!(
        !text.contains("inline array"),
        "and nothing is said about inline tables: {text}"
    );
}

#[test]
fn uninstall_dry_run_says_which_pairs_it_could_not_read_and_does_not_guess() {
    let sandbox = UninstallSandbox::new();
    let inline = sandbox.root_with_state("inline");
    let config = sandbox.path().join("inline.toml");
    // A pair written as an inline array, which the line-by-line reading cannot see.
    fs::write(
        &config,
        format!(
            "pair = [{{ name = \"inline\", local_root = \"{}\", remote_root = \"/Drive/I\" }}]\n",
            inline.display()
        ),
    )
    .expect("write config");

    let text = sandbox.dry_run(&config);
    assert!(
        text.contains("inline") && text.contains("NOT in the plan"),
        "a clear warning that some pairs could not be read: {text}"
    );
    assert!(
        text.contains("left in place"),
        "and what that means for their .sync directories: {text}"
    );
    assert!(
        !text.contains(&format!("{}", inline.join(".sync").display())),
        "the script does not guess a root out of text it cannot parse: {text}"
    );
}

#[test]
fn uninstall_never_lists_a_dot_sync_directly_under_home() {
    // PR #434 second review, H4; third review, L3. What this pins is the `$HOME` arm of the guard
    // in `validated_sync_state_dir`: removing it still passed, and a hand-edited
    // `local_root = "~"` (or the absolute home directory) with a `.sync` in it would then be
    // planned for removal. A dry run is enough to prove it, and nothing is removed.
    //
    // The `/` arm is NOT pinned and cannot be without a `.sync` at the real `/`: the guard compares
    // the lexical path after `cd && pwd`, which keeps a symlink's own name, so no sandbox path
    // resolves to `/`; and with that arm removed the later `[[ -d /.sync ]]` test refuses it on any
    // machine that has no `/.sync`. The `slash` pair below is kept as a smoke test, not a pin:
    // a root of `/` must not break the run or put a line for `/.sync` in the plan.
    let sandbox = UninstallSandbox::new();
    let home = sandbox.path().join("home");
    fs::create_dir(home.join(".sync")).expect("a .sync directly under HOME");
    fs::write(home.join(".sync").join("precious"), b"not engine state").expect("a file in it");
    let docs = sandbox.root_with_state("docs");
    let config = sandbox.path().join("pairs.toml");
    fs::write(
        &config,
        format!(
            "[[pair]]\nname = \"home\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/H\"\n\
             [[pair]]\nname = \"tilde\"\nlocal_root = \"~\"\nremote_root = \"/Drive/T\"\n\
             [[pair]]\nname = \"slash\"\nlocal_root = \"/\"\nremote_root = \"/Drive/S\"\n\
             [[pair]]\nname = \"docs\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/D\"\n",
            home.display(),
            docs.display()
        ),
    )
    .expect("write config");

    let text = sandbox.dry_run(&config);
    assert!(
        text.contains(&format!("{}", docs.join(".sync").display())),
        "the ordinary root is still listed, so the run read the config: {text}"
    );
    assert!(
        !text.contains(&format!("{}", home.join(".sync").display())),
        "a `.sync` directly under HOME is not engine state and is not planned: {text}"
    );
    assert!(
        !text.lines().any(|line| line.ends_with(": /.sync")),
        "and a root of `/` did not put `/.sync` in the plan: {text}"
    );
    assert!(
        home.join(".sync").join("precious").exists(),
        "and a dry run removed nothing"
    );
}

/// `setup.sh`'s dependency check, driven through the real `run_terminal_setup` / `run_gui_setup`
/// in the uninstall sandbox with `--no-build`. `proton-sync` is a stub that records its argv (one
/// argument per line) and exits with a chosen status; `proton-syncd` prints a canned plan;
/// `systemctl` logs its calls. Nothing is installed anywhere but the scratch tree.
struct SetupRun {
    sandbox: UninstallSandbox,
}

struct SetupOutcome {
    status: i32,
    text: String,
    argv: Option<Vec<String>>,
    systemctl_calls: String,
    unit_written: bool,
}

impl SetupRun {
    fn new(doctor_exit: i32) -> Self {
        let sandbox = UninstallSandbox::new();
        let root = sandbox.path();
        let argv = root.join("doctor-argv");
        // Built from parts: the isolation scan refuses a literal binary name in this file.
        let sync = format!("proton-{}", "sync");
        let syncd = format!("{sync}d");
        let stubs = [
            (
                sync.as_str(),
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\nexit {doctor_exit}\n",
                    argv.display()
                ),
            ),
            (
                syncd.as_str(),
                "#!/bin/sh\nprintf '{\"summary\":{\"uploads\":0,\"downloads\":0,\"destructive_actions\":0}}'\n"
                    .to_owned(),
            ),
            ("proton-sync-gui", "#!/bin/sh\nexit 0\n".to_owned()),
            (
                "systemctl",
                format!(
                    "#!/bin/sh\necho \"$@\" >> '{}'\nexit 0\n",
                    root.join("systemctl-calls").display()
                ),
            ),
        ];
        for (name, body) in stubs {
            let path = root.join("bin").join(name);
            fs::write(&path, body).expect("stub");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("stub mode");
        }
        Self { sandbox }
    }

    /// Runs `<function> <args>` after `parse_args <flags>` with `setup.sh` sourced.
    fn run(&self, function: &str, flags: &[&str]) -> SetupOutcome {
        let root = self.sandbox.path();
        let setup = Path::new(env!("CARGO_MANIFEST_DIR")).join("setup.sh");
        let mut command = self.sandbox.command();
        command
            .arg("-c")
            .arg(format!(
                "source \"$1\"; shift; parse_args \"$@\"; repo_root=.; {function}"
            ))
            .arg("bash")
            .arg(setup)
            .args(flags);
        let output = common::run_bounded(&mut command, common::RUN_BOUND);
        SetupOutcome {
            status: output.status.code().expect("exit code"),
            text: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            argv: fs::read_to_string(root.join("doctor-argv"))
                .ok()
                .map(|text| text.lines().map(str::to_owned).collect()),
            systemctl_calls: fs::read_to_string(root.join("systemctl-calls")).unwrap_or_default(),
            unit_written: root
                .join("xdg-config/systemd/user/proton-syncd.service")
                .exists(),
        }
    }

    fn config(&self) -> String {
        self.sandbox
            .path()
            .join("xdg-config/proton-sync/proton-sync.toml")
            .display()
            .to_string()
    }

    fn terminal(&self, extra: &[&str]) -> SetupOutcome {
        let local = self.sandbox.path().join("sync-folder");
        let local = local.display().to_string();
        let mut flags = vec![
            "--no-build",
            "--no-start",
            "-y",
            "--local-root",
            &local,
            "--remote-root",
            "/Drive/x",
        ];
        flags.extend_from_slice(extra);
        self.run("run_terminal_setup", &flags)
    }
}

#[test]
fn terminal_setup_runs_doctor_against_the_config_it_wrote_and_continues_on_ok() {
    let run = SetupRun::new(0);
    let outcome = run.terminal(&[]);
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert_eq!(
        outcome.argv,
        Some(vec!["--config".into(), run.config(), "doctor".into()]),
        "--config comes before the verb: {}",
        outcome.text
    );
    assert!(
        outcome.unit_written && outcome.systemctl_calls.contains("daemon-reload"),
        "setup went on to install the unit: {}",
        outcome.text
    );
}

#[test]
fn terminal_setup_passes_the_cli_path_after_doctor() {
    let run = SetupRun::new(0);
    let outcome = run.terminal(&["--proton-cli", "/opt/proton-drive"]);
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert_eq!(
        outcome.argv,
        Some(vec![
            "--config".into(),
            run.config(),
            "doctor".into(),
            "--proton-cli".into(),
            "/opt/proton-drive".into()
        ]),
        "{}",
        outcome.text
    );
}

#[test]
fn terminal_setup_stops_before_the_unit_on_a_required_problem_or_an_unusable_check() {
    for exit in [1, 2] {
        let run = SetupRun::new(exit);
        let outcome = run.terminal(&[]);
        assert_ne!(outcome.status, 0, "exit {exit}: {}", outcome.text);
        assert!(
            !outcome.unit_written && outcome.systemctl_calls.is_empty(),
            "exit {exit} installed or reloaded the unit: {}",
            outcome.text
        );
        assert!(
            !outcome.text.contains("Previewing the plan"),
            "exit {exit} reached the preview: {}",
            outcome.text
        );
    }
    // A usage error is not read as a result: it says the binary cannot check.
    let outcome = SetupRun::new(2).terminal(&[]);
    assert!(
        outcome.text.contains("cannot run the check"),
        "{}",
        outcome.text
    );
}

#[test]
fn terminal_setup_warns_and_continues_when_doctor_could_not_check() {
    let run = SetupRun::new(3);
    let outcome = run.terminal(&[]);
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert!(
        outcome.text.contains("could not be checked") && outcome.unit_written,
        "{}",
        outcome.text
    );
}

#[test]
fn ignore_checks_continues_past_a_required_problem() {
    let run = SetupRun::new(1);
    let outcome = run.terminal(&["--ignore-checks"]);
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert!(
        outcome
            .text
            .contains("Continuing because of --ignore-checks")
            && outcome.unit_written
            && outcome.text.contains("Previewing the plan"),
        "{}",
        outcome.text
    );
}

#[test]
fn gui_setup_runs_doctor_without_a_config_and_continues_on_a_required_problem() {
    let run = SetupRun::new(1);
    let outcome = run.run("run_gui_setup", &["--gui", "--no-build"]);
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert_eq!(
        outcome.argv,
        Some(vec!["doctor".into()]),
        "no --config in --gui mode: {}",
        outcome.text
    );
    assert!(
        outcome.text.contains("runs the same check") && outcome.text.contains("Done"),
        "{}",
        outcome.text
    );
}

#[test]
fn gui_setup_passes_the_cli_path_it_was_given_and_still_no_config() {
    let run = SetupRun::new(0);
    let outcome = run.run(
        "run_gui_setup",
        &["--gui", "--no-build", "--proton-cli", "/opt/proton-drive"],
    );
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert_eq!(
        outcome.argv,
        Some(vec![
            "doctor".into(),
            "--proton-cli".into(),
            "/opt/proton-drive".into()
        ]),
        "{}",
        outcome.text
    );
}

#[test]
fn sourcing_setup_runs_no_check_and_the_other_scripts_never_call_it() {
    let run = SetupRun::new(1);
    let outcome = run.run("true", &[]);
    assert_eq!(outcome.status, 0, "{}", outcome.text);
    assert!(
        outcome.argv.is_none() && outcome.systemctl_calls.is_empty(),
        "sourcing ran something: {}",
        outcome.text
    );
    for script in ["upgrade.sh", "uninstall.sh"] {
        let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(script))
            .expect("read script");
        assert!(
            !text.contains("run_dependency_check") && !text.contains("check_common_prereqs"),
            "{script} reaches the setup-only check"
        );
    }
}

// --- packaging/version.sh ---------------------------------------------------------------------
//
// `check` only reads files. `set` writes them, so it runs on a temporary copy of the files it
// touches, with `cargo` and `npm` shadowed by stubs: the real ones would regenerate the lockfiles
// (offline, but they are the tools' business and not this script's mapping). The expected strings
// below are literal examples of the mapping, not a second copy of it.

/// Every file `packaging/version.sh` reads or writes, relative to the repository root.
const VERSION_SITES: [&str; 13] = [
    "packaging/version.sh",
    "Cargo.toml",
    "gui/gui-core/Cargo.toml",
    "gui/src-tauri/Cargo.toml",
    "Cargo.lock",
    "gui/src-tauri/tauri.conf.json",
    "gui/package.json",
    "gui/package-lock.json",
    "packaging/rpm/proton-drive-sync-engine.spec",
    "packaging/arch/PKGBUILD",
    "packaging/debian/changelog",
    "packaging/freedesktop/app.protondrivesync.engine.metainfo.xml",
    "examples/packaging/release-assets.toml",
];

/// `bash <root>/packaging/version.sh <args>` in the test's sandbox; `stubs` is a directory placed
/// ahead of the rest of `PATH`.
fn version_script(
    root: &Path,
    directory: &Path,
    stubs: Option<&Path>,
    args: &[&str],
) -> std::process::Output {
    // The entry date is today's unless SOURCE_DATE_EPOCH is set; a fixed one keeps two runs
    // byte-identical even across midnight. 1700000000 is Tue, 14 Nov 2023 22:13:20 UTC.
    version_script_with_epoch(root, directory, stubs, args, "1700000000")
}

fn version_script_with_epoch(
    root: &Path,
    directory: &Path,
    stubs: Option<&Path>,
    args: &[&str],
    epoch: &str,
) -> std::process::Output {
    let mut command = common::sandboxed("bash", directory);
    command.arg(root.join("packaging/version.sh")).args(args);
    command.env("SOURCE_DATE_EPOCH", epoch);
    if let Some(stubs) = stubs {
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
        command.env(
            "PATH",
            common::path_with_stubs(&format!("{}:{path}", stubs.display())),
        );
    }
    common::run_other_tool(&mut command)
}

/// A copy of the version sites in `directory/tree`, and a directory of `cargo` and `npm` stubs.
fn copy_of_the_version_sites(directory: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let tree = directory.join("tree");
    for site in VERSION_SITES {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join(site);
        let to = tree.join(site);
        fs::create_dir_all(to.parent().expect("parent")).expect("directory");
        fs::copy(&from, &to).unwrap_or_else(|error| panic!("copy {site}: {error}"));
    }
    let stubs = directory.join("tool-stubs");
    fs::create_dir(&stubs).expect("stub directory");
    for name in ["cargo", "npm"] {
        let path = stubs.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("stub mode");
    }
    (tree, stubs)
}

#[test]
fn version_check_passes_on_the_repository() {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = version_script(root, directory.path(), None, &["check"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn version_check_names_the_file_the_expected_value_and_the_value_found() {
    let directory = tempdir().expect("tempdir");
    let (tree, _) = copy_of_the_version_sites(directory.path());
    let pkgbuild = tree.join("packaging/arch/PKGBUILD");
    let text = fs::read_to_string(&pkgbuild).expect("read PKGBUILD");
    let poisoned = text
        .lines()
        .map(|line| {
            if line.starts_with("pkgver=") {
                "pkgver=9.9.9beta9"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&pkgbuild, poisoned + "\n").expect("poison");
    let output = version_script(&tree, directory.path(), None, &["check"]);
    assert!(!output.status.success(), "a stale pkgver passed the check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("packaging/arch/PKGBUILD: pkgver: expected ")
            && stderr.contains(", found 9.9.9beta9"),
        "{stderr}"
    );
}

#[test]
fn version_set_writes_the_mapped_forms_on_a_copy() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let output = version_script(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let read = |site: &str| fs::read_to_string(tree.join(site)).expect(site);

    assert!(read("Cargo.toml").contains("\nversion = \"0.1.0-beta.1\"\n"));
    assert!(read("gui/src-tauri/tauri.conf.json").contains("\"version\": \"0.1.0-beta.1\""));
    let spec = read("packaging/rpm/proton-drive-sync-engine.spec");
    assert!(spec.contains("\n%global upstream_version 0.1.0-beta.1\n"));
    assert!(spec.contains("\nVersion:        0.1.0~beta.1\n"));
    assert!(spec.contains(" - 0.1.0~beta.1-1\n"), "{spec}");
    let pkgbuild = read("packaging/arch/PKGBUILD");
    assert!(pkgbuild.contains("\npkgver=0.1.0beta1\n"));
    assert!(pkgbuild.contains("\n_upstream_ver=0.1.0-beta.1\n"));
    assert!(
        read("packaging/debian/changelog")
            .starts_with("proton-drive-sync-engine (0.1.0~beta.1-1) ")
    );
    let metainfo = read("packaging/freedesktop/app.protondrivesync.engine.metainfo.xml");
    assert!(
        metainfo.contains("<release version=\"0.1.0~beta.1\"")
            && metainfo.contains("type=\"development\">"),
        "{metainfo}"
    );
    assert!(
        read("examples/packaging/release-assets.toml")
            .contains("proton-drive-sync-engine-0.1.0-beta.1.tar.gz")
    );

    // The stubs did not regenerate the lockfiles, so the check names exactly those and nothing
    // else: every site `set` writes itself agrees.
    let output = version_script(&tree, directory.path(), None, &["check"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let named: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.split(": ").next())
        .filter(|file| file.contains('/') || file.contains('.'))
        .filter(|file| !file.starts_with("version.sh"))
        .collect();
    assert!(
        named
            .iter()
            .all(|file| *file == "Cargo.lock" || *file == "gui/package-lock.json"),
        "{stderr}"
    );
}

#[test]
fn version_set_refuses_a_version_outside_the_accepted_shapes_and_changes_nothing() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let before: Vec<String> = VERSION_SITES
        .iter()
        .map(|site| fs::read_to_string(tree.join(site)).expect(site))
        .collect();
    for bad in ["0.1.0-beta", "0.1.0+b.1", "0.1.0-preview.1"] {
        let output = version_script(&tree, directory.path(), Some(&stubs), &["set", bad]);
        assert!(!output.status.success(), "set {bad} was accepted");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("X.Y.Z"),
            "{bad}: the refusal does not name the accepted shapes"
        );
    }
    let after: Vec<String> = VERSION_SITES
        .iter()
        .map(|site| fs::read_to_string(tree.join(site)).expect(site))
        .collect();
    assert_eq!(before, after);
}

// --- packaging/version.sh: superseded entries, all-or-nothing, revisions ----------------------

fn contents(tree: &Path) -> Vec<String> {
    VERSION_SITES
        .iter()
        .map(|site| fs::read_to_string(tree.join(site)).expect(site))
        .collect()
}

/// `set <version>` with the stub `cargo` and `npm`, which must succeed. Returns what it printed.
fn set_ok(tree: &Path, directory: &Path, stubs: &Path, version: &str) -> String {
    let output = version_script(tree, directory, Some(stubs), &["set", version]);
    assert!(
        output.status.success(),
        "set {version}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

const SPEC_FILE: &str = "packaging/rpm/proton-drive-sync-engine.spec";
const DEB_FILE: &str = "packaging/debian/changelog";
const META_FILE: &str = "packaging/freedesktop/app.protondrivesync.engine.metainfo.xml";

#[test]
fn set_drops_the_entries_a_lower_version_supersedes() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let said = set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    let read = |site: &str| fs::read_to_string(tree.join(site)).expect(site);
    let spec = read(SPEC_FILE);
    assert!(!spec.contains(" - 0.1.0-1\n"), "{spec}");
    assert!(spec.contains(" - 0.1.0~beta.1-1\n"));
    let deb = read(DEB_FILE);
    assert!(!deb.contains("(0.1.0-1)"), "{deb}");
    assert!(deb.starts_with("proton-drive-sync-engine (0.1.0~beta.1-1) "));
    let meta = read(META_FILE);
    assert!(!meta.contains("<release version=\"0.1.0\" "), "{meta}");
    assert_eq!(meta.matches("<release ").count(), 1);
    assert!(
        said.contains("dropping"),
        "each dropped entry is printed: {said}"
    );
}

#[test]
fn set_twice_for_one_version_gives_the_same_bytes() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    let once = contents(&tree);
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    assert_eq!(once, contents(&tree));
}

#[test]
fn the_final_release_goes_above_its_beta_with_one_entry_of_each() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    set_ok(&tree, directory.path(), &stubs, "0.1.0");
    let read = |site: &str| fs::read_to_string(tree.join(site)).expect(site);
    let spec = read(SPEC_FILE);
    assert_eq!(spec.matches(" - 0.1.0-1\n").count(), 1, "{spec}");
    assert_eq!(spec.matches(" - 0.1.0~beta.1-1\n").count(), 1);
    assert!(spec.find(" - 0.1.0-1\n") < spec.find(" - 0.1.0~beta.1-1\n"));
    let deb = read(DEB_FILE);
    assert!(
        deb.starts_with("proton-drive-sync-engine (0.1.0-1) "),
        "{deb}"
    );
    assert_eq!(deb.matches("(0.1.0-1)").count(), 1);
    assert_eq!(deb.matches("(0.1.0~beta.1-1)").count(), 1);
    let meta = read(META_FILE);
    let stable = meta.find("<release version=\"0.1.0\" ");
    let beta = meta.find("<release version=\"0.1.0~beta.1\" ");
    assert_eq!(meta.matches("<release version=\"0.1.0\" ").count(), 1);
    assert!(
        stable.is_some() && beta.is_some() && stable < beta,
        "{meta}"
    );
    assert!(meta.contains("type=\"stable\""));
}

#[test]
fn a_pre_release_orders_below_the_final_and_by_number_not_text() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    // beta.9 < beta.10 < rc.1 < final: setting a lower one drops every higher one.
    set_ok(&tree, directory.path(), &stubs, "0.1.0-rc.1");
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.10");
    let deb = fs::read_to_string(tree.join(DEB_FILE)).expect("deb");
    assert!(!deb.contains("rc.1"), "{deb}");
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.9");
    let deb = fs::read_to_string(tree.join(DEB_FILE)).expect("deb");
    assert!(
        !deb.contains("beta.10"),
        "beta.10 sorts above beta.9: {deb}"
    );
    assert!(deb.starts_with("proton-drive-sync-engine (0.1.0~beta.9-1) "));
}

#[test]
fn set_refuses_to_drop_an_entry_whose_tag_exists_and_changes_nothing() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let vcs = |args: &[&str]| {
        let mut command = common::sandboxed("git", directory.path());
        command
            .arg("-C")
            .arg(&tree)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args);
        let output = common::run_other_tool(&mut command);
        assert!(output.status.success(), "{args:?}: {output:?}");
    };
    vcs(&["init", "-q"]);
    vcs(&["commit", "-q", "--allow-empty", "-m", "x"]);
    vcs(&["tag", "v0.1.0"]);
    let before = contents(&tree);
    let output = version_script(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("v0.1.0"));
    assert_eq!(before, contents(&tree));
}

#[test]
fn a_failing_cargo_leaves_every_file_as_it_was() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    fs::write(
        stubs.join("cargo"),
        "#!/bin/sh\necho 'no cached crates' >&2\nexit 101\n",
    )
    .expect("stub");
    let before = contents(&tree);
    let output = version_script(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
    );
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cargo fetch"), "{stderr}");
    assert_eq!(before, contents(&tree));
}

#[test]
fn rerunning_for_the_same_version_keeps_a_deliberate_revision() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    let pkgbuild = tree.join("packaging/arch/PKGBUILD");
    let text = fs::read_to_string(&pkgbuild).expect("PKGBUILD");
    fs::write(&pkgbuild, text.replace("\npkgrel=1\n", "\npkgrel=3\n")).expect("write");
    let deb = tree.join(DEB_FILE);
    let text = fs::read_to_string(&deb).expect("deb");
    fs::write(&deb, text.replace("(0.1.0~beta.1-1)", "(0.1.0~beta.1-2)")).expect("write");
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    assert!(
        fs::read_to_string(&pkgbuild)
            .expect("PKGBUILD")
            .contains("\npkgrel=3\n")
    );
    assert!(
        fs::read_to_string(&deb)
            .expect("deb")
            .starts_with("proton-drive-sync-engine (0.1.0~beta.1-2) ")
    );
    // a different version starts again at 1
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.2");
    assert!(
        fs::read_to_string(&pkgbuild)
            .expect("PKGBUILD")
            .contains("\npkgrel=1\n")
    );
}

// --- packaging/version.sh: self-closing entries, a failed snapshot, tag-check notes, dates ------

#[test]
fn a_self_closing_last_release_is_dropped_and_the_closing_tags_survive() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let metainfo = tree.join(META_FILE);
    let text = fs::read_to_string(&metainfo).expect("metainfo");
    fs::write(
        &metainfo,
        text.replace(
            "  </releases>",
            "    <release version=\"9.0.0\" date=\"2026-01-01\"/>\n  </releases>",
        ),
    )
    .expect("write");
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    let meta = fs::read_to_string(&metainfo).expect("metainfo");
    assert!(!meta.contains("9.0.0"), "{meta}");
    assert!(
        meta.contains("</releases>") && meta.contains("</component>"),
        "{meta}"
    );
    assert_eq!(meta.matches("<release ").count(), 1);
}

#[test]
fn a_metainfo_that_lost_its_closing_tags_rolls_the_whole_set_back() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    // An entry the script cannot see the end of: no </release> and no self-closing slash.
    let metainfo = tree.join(META_FILE);
    let text = fs::read_to_string(&metainfo).expect("metainfo");
    fs::write(
        &metainfo,
        text.replace(
            "  </releases>",
            "    <release version=\"9.0.0\" date=\"2026-01-01\">\n  </releases>",
        ),
    )
    .expect("write");
    let before = contents(&tree);
    let output = version_script(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
    );
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(before, contents(&tree));
}

#[test]
fn a_failed_snapshot_restores_nothing_it_did_not_copy_and_leaks_no_directory() {
    use std::os::unix::fs::MetadataExt;
    if fs::metadata(env!("CARGO_MANIFEST_DIR"))
        .expect("metadata")
        .uid()
        == 0
        || fs::metadata("/proc/self").expect("metadata").uid() == 0
    {
        return; // a root process reads a mode-000 file; this case cannot be driven
    }
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let scratch = directory.path().join("tmp");
    fs::create_dir(&scratch).expect("tmp");
    let before = contents(&tree);
    let locked = tree.join("examples/packaging/release-assets.toml");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    let mut command = common::sandboxed("bash", directory.path());
    command
        .arg(tree.join("packaging/version.sh"))
        .args(["set", "0.1.0-beta.1"])
        .env("TMPDIR", &scratch);
    let path = std::env::var("PATH").unwrap_or_default();
    command.env(
        "PATH",
        common::path_with_stubs(&format!("{}:{path}", stubs.display())),
    );
    let output = common::run_other_tool(&mut command);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).expect("chmod back");
    assert!(!output.status.success(), "{output:?}");
    let leaked: Vec<_> = fs::read_dir(&scratch).expect("read").collect();
    assert!(
        leaked.is_empty(),
        "the snapshot directory leaked: {leaked:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("cp: cannot stat"),
        "the rollback copied a file that was never snapshotted: {stderr}"
    );
    assert_eq!(before, contents(&tree));
}

#[test]
fn an_alpha_sorts_below_every_beta_and_the_final() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    set_ok(&tree, directory.path(), &stubs, "0.1.0-alpha.1");
    let deb = fs::read_to_string(tree.join(DEB_FILE)).expect("deb");
    assert!(!deb.contains("beta.1"), "beta.1 sorts above alpha.1: {deb}");
    // and the final release is dropped by an alpha too
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    set_ok(&tree, directory.path(), &stubs, "0.1.0-alpha.1");
    let deb = fs::read_to_string(tree.join(DEB_FILE)).expect("deb");
    assert!(
        !deb.contains("(0.1.0-1)"),
        "0.1.0 sorts above alpha.1: {deb}"
    );
}

#[test]
fn the_entry_date_honours_source_date_epoch() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    set_ok(&tree, directory.path(), &stubs, "0.1.0-beta.1");
    // 1700000000 is Tue, 14 Nov 2023 22:13:20 UTC (version_script sets it).
    assert!(
        fs::read_to_string(tree.join(DEB_FILE))
            .expect("deb")
            .contains("Tue, 14 Nov 2023 22:13:20 +0000")
    );
    assert!(
        fs::read_to_string(tree.join(SPEC_FILE))
            .expect("spec")
            .contains("* Tue Nov 14 2023 ")
    );
    assert!(
        fs::read_to_string(tree.join(META_FILE))
            .expect("metainfo")
            .contains("date=\"2023-11-14\"")
    );
}

#[test]
fn the_tag_check_says_when_it_cannot_run_and_that_it_reads_local_tags_only() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    // The copy is not the top level of a repository.
    let output = version_script(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
    );
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("tag check skipped") && stderr.contains("is not in a git repository"),
        "{stderr}"
    );
}

// --- packaging/version.sh: a bad SOURCE_DATE_EPOCH, two releases on one line -------------------

#[test]
fn a_source_date_epoch_that_is_not_a_number_of_seconds_is_refused_and_nothing_changes() {
    for bad in [
        "garbage",
        "-5",
        " 5",
        "1e3",
        "0x10",
        "@5",
        "99999999999999999999",
    ] {
        let directory = tempdir().expect("tempdir");
        let (tree, stubs) = copy_of_the_version_sites(directory.path());
        let before = contents(&tree);
        let output = version_script_with_epoch(
            &tree,
            directory.path(),
            Some(&stubs),
            &["set", "0.1.0-beta.1"],
            bad,
        );
        assert_eq!(output.status.code(), Some(2), "{bad:?}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("SOURCE_DATE_EPOCH") && stderr.contains(bad),
            "{bad:?}: {stderr}"
        );
        assert_eq!(before, contents(&tree), "{bad:?} changed a file");
    }
}

#[test]
fn an_empty_source_date_epoch_falls_back_to_today() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let output = version_script_with_epoch(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
        "",
    );
    assert!(output.status.success(), "{output:?}");
    let meta = fs::read_to_string(tree.join(META_FILE)).expect("metainfo");
    assert!(!meta.contains("date=\"\""), "{meta}");
}

#[test]
fn two_release_tags_on_one_metainfo_line_are_refused_and_nothing_changes() {
    let directory = tempdir().expect("tempdir");
    let (tree, stubs) = copy_of_the_version_sites(directory.path());
    let metainfo = tree.join(META_FILE);
    let text = fs::read_to_string(&metainfo).expect("metainfo");
    fs::write(
        &metainfo,
        text.replace(
            "  </releases>",
            "    <release version=\"9.0.0\" date=\"2026-01-01\"/> <release version=\"0.0.5\" date=\"2026-01-01\"/>\n  </releases>",
        ),
    )
    .expect("write");
    let before = contents(&tree);
    let output = version_script(
        &tree,
        directory.path(),
        Some(&stubs),
        &["set", "0.1.0-beta.1"],
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("more than one <release"),
        "{output:?}"
    );
    assert_eq!(before, contents(&tree));
}

// --- packaging/arch/aur.sh ---------------------------------------------------------------------
//
// The script writes the AUR tree from the repository's PKGBUILD. `makepkg` exists only on Arch, so
// a stub that answers `--printsrcinfo` stands in; what these cases pin is the PKGBUILD the script
// writes and what it refuses, not makepkg (the Arch job in `packages.yml` runs the real one).

struct AurTree {
    directory: tempfile::TempDir,
    stubs: std::path::PathBuf,
    assets: std::path::PathBuf,
    upstream: String,
}

fn aur_tree() -> AurTree {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let arch = directory.path().join("tree/packaging/arch");
    fs::create_dir_all(&arch).expect("tree");
    for file in ["aur.sh", "PKGBUILD", "proton-drive-sync-engine.install"] {
        fs::copy(root.join("packaging/arch").join(file), arch.join(file)).expect(file);
    }
    let pkgbuild = fs::read_to_string(arch.join("PKGBUILD")).expect("PKGBUILD");
    let upstream = pkgbuild
        .lines()
        .find_map(|line| line.strip_prefix("_upstream_ver="))
        .expect("_upstream_ver")
        .to_owned();
    let stubs = directory.path().join("tool-stubs");
    fs::create_dir(&stubs).expect("stub directory");
    let makepkg = stubs.join("makepkg");
    fs::write(
        &makepkg,
        "#!/bin/sh\n[ \"$1\" = --printsrcinfo ] && { echo 'pkgbase = stub'; exit 0; }\nexit 1\n",
    )
    .expect("stub");
    fs::set_permissions(&makepkg, fs::Permissions::from_mode(0o700)).expect("stub mode");
    let assets = directory.path().join("assets");
    fs::create_dir(&assets).expect("assets");
    fs::write(
        assets.join(format!("proton-drive-sync-engine-{upstream}.tar.gz")),
        b"not really a tarball\n",
    )
    .expect("tarball");
    AurTree {
        directory,
        stubs,
        assets,
        upstream,
    }
}

fn aur_script(tree: &AurTree, args: &[&str]) -> std::process::Output {
    let mut command = common::sandboxed("bash", tree.directory.path());
    command
        .arg(tree.directory.path().join("tree/packaging/arch/aur.sh"))
        .args(args);
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
    command.env(
        "PATH",
        common::path_with_stubs(&format!("{}:{path}", tree.stubs.display())),
    );
    common::run_other_tool(&mut command)
}

fn sha256_of(path: &Path) -> String {
    let mut command = std::process::Command::new("sha256sum");
    command.arg(path);
    let output = common::run_other_tool(&mut command);
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .split_whitespace()
        .next()
        .expect("a sum")
        .to_owned()
}

#[test]
fn the_beta_tree_renames_the_package_and_carries_the_real_checksum() {
    let tree = aur_tree();
    let out = tree.directory.path().join("out");
    let output = aur_script(
        &tree,
        &[
            "beta",
            tree.assets.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(out.join("PKGBUILD")).expect("PKGBUILD");
    assert!(
        generated.contains(
            "\npkgname=proton-drive-sync-engine-beta\nprovides=(proton-drive-sync-engine)\nconflicts=(proton-drive-sync-engine)\n"
        ),
        "{generated}"
    );
    let sum = sha256_of(
        &tree
            .assets
            .join(format!("proton-drive-sync-engine-{}.tar.gz", tree.upstream)),
    );
    assert!(
        generated.contains(&format!("\nsha256sums=('{sum}')\n")),
        "{generated}"
    );
    assert!(!generated.contains("sha256sums=('SKIP')"), "{generated}");
    assert!(out.join("proton-drive-sync-engine.install").is_file());
    assert_eq!(
        fs::read_to_string(out.join(".SRCINFO")).expect(".SRCINFO"),
        "pkgbase = stub\n"
    );

    // Nothing else differs: every other line of the repository PKGBUILD is still there, in order.
    let original = fs::read_to_string(tree.directory.path().join("tree/packaging/arch/PKGBUILD"))
        .expect("original");
    let changed = |line: &&str| {
        line.starts_with("pkgname=")
            || line.starts_with("sha256sums=")
            || line.starts_with("provides=")
            || line.starts_with("conflicts=")
    };
    let kept = |text: &str| {
        text.lines()
            .filter(|line| !changed(line))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(kept(&original), kept(&generated));
}

#[test]
fn the_stable_tree_keeps_the_name_and_adds_no_provides() {
    let tree = aur_tree();
    let out = tree.directory.path().join("out");
    let output = aur_script(
        &tree,
        &[
            "stable",
            tree.assets.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(out.join("PKGBUILD")).expect("PKGBUILD");
    assert!(
        generated.contains("\npkgname=proton-drive-sync-engine\n"),
        "{generated}"
    );
    assert!(!generated.contains("\nprovides=") && !generated.contains("\nconflicts="));
}

#[test]
fn the_aur_script_refuses_what_it_cannot_make_a_correct_tree_from() {
    let tree = aur_tree();
    let assets = tree.assets.to_str().expect("utf-8").to_owned();
    let out = tree.directory.path().join("out");
    let out = out.to_str().expect("utf-8").to_owned();
    let refused = |args: &[&str], needle: &str| {
        let output = aur_script(&tree, args);
        assert!(!output.status.success(), "{args:?} was accepted");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{args:?}: {stderr}");
    };
    refused(&["nightly", &assets, &out], "not stable or beta");
    refused(&["beta", "/no/such/directory", &out], "does not exist");

    // A SHA256SUMS that disagrees with the tarball it lists.
    let tarball = format!("proton-drive-sync-engine-{}.tar.gz", tree.upstream);
    fs::write(
        tree.assets.join("SHA256SUMS"),
        format!("{}  {tarball}\n", "0".repeat(64)),
    )
    .expect("SHA256SUMS");
    refused(&["beta", &assets, &out], "SHA-256 is");
    // A SHA256SUMS that does not list it at all.
    fs::write(
        tree.assets.join("SHA256SUMS"),
        format!("{}  other.rpm\n", "0".repeat(64)),
    )
    .expect("SHA256SUMS");
    refused(&["beta", &assets, &out], "does not list");
    fs::remove_file(tree.assets.join("SHA256SUMS")).expect("remove");

    // No tarball.
    fs::remove_file(tree.assets.join(&tarball)).expect("remove");
    refused(&["beta", &assets, &out], "is missing");

    // An output directory that already holds something.
    fs::write(tree.assets.join(&tarball), b"x").expect("tarball");
    fs::create_dir(&out).expect("out");
    fs::write(Path::new(&out).join("leftover"), b"x").expect("leftover");
    refused(&["beta", &assets, &out], "not an empty directory");
}

#[test]
fn a_sha256sums_that_agrees_with_the_tarball_is_accepted() {
    let tree = aur_tree();
    let tarball = format!("proton-drive-sync-engine-{}.tar.gz", tree.upstream);
    let sum = sha256_of(&tree.assets.join(&tarball));
    fs::write(
        tree.assets.join("SHA256SUMS"),
        format!("{sum}  {tarball}\n"),
    )
    .expect("SHA256SUMS");
    let out = tree.directory.path().join("out");
    let output = aur_script(
        &tree,
        &[
            "beta",
            tree.assets.to_str().expect("utf-8"),
            out.to_str().expect("utf-8"),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- packaging/name-assets.sh ------------------------------------------------------------------
//
// The script reads a package's own version with `rpm`, `dpkg-deb` or `bsdtar`, asks
// `version.sh spell` what that version must be, and renames the file to the upstream version. The
// stubs below stand in for the three tools and print what the real ones printed for the packages
// built in podman on 2026-10-13: `rpm -qp --qf '%{VERSION}'` prints the version with no newline,
// `dpkg-deb -f <file> Version` prints `<version>-<revision>` and a newline, and `bsdtar -xOf <file>
// .PKGINFO` prints the build metadata with a `pkgver = <pkgver>-<pkgrel>` line. A stub exits 64
// for any other arguments, so a change to how the script calls a tool fails these cases. A "package"
// here is a file whose content is the version its stub reports.

struct NameTree {
    directory: tempfile::TempDir,
    stubs: std::path::PathBuf,
    packages: std::path::PathBuf,
}

const RPM_STUB: &str = "#!/bin/sh\n\
[ $# -eq 4 ] && [ \"$1\" = -qp ] && [ \"$2\" = --qf ] && [ \"$3\" = '%{VERSION}' ] || exit 64\n\
printf '%s' \"$(cat \"$4\")\"\n";
const DEB_STUB: &str = "#!/bin/sh\n\
[ $# -eq 3 ] && [ \"$1\" = -f ] && [ \"$3\" = Version ] || exit 64\n\
cat \"$2\"\n";
const BSDTAR_STUB: &str = "#!/bin/sh\n\
[ $# -eq 3 ] && [ \"$1\" = -xOf ] && [ \"$3\" = .PKGINFO ] || exit 64\n\
printf '# Generated by makepkg 7.1.0\\n# using fakeroot version 1.37.2\\n'\n\
printf 'pkgname = proton-drive-sync-engine\\npkgbase = proton-drive-sync-engine\\nxdata = pkgtype=pkg\\n'\n\
printf 'pkgver = %s\\n' \"$(cat \"$2\")\"\n\
printf 'pkgdesc = Two-way file sync between a local folder and Proton Drive\\nsize = 31895616\\narch = x86_64\\n'\n\
printf 'depend = gcc-libs\\ndepend = curl\\noptdepend = nemo-python: emblems\\n'\n";

fn name_tree() -> NameTree {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let packaging = directory.path().join("tree/packaging");
    fs::create_dir_all(&packaging).expect("tree");
    for file in ["name-assets.sh", "version.sh"] {
        fs::copy(root.join("packaging").join(file), packaging.join(file)).expect(file);
    }
    let stubs = directory.path().join("tool-stubs");
    fs::create_dir(&stubs).expect("stub directory");
    for (name, text) in [
        ("rpm", RPM_STUB),
        ("dpkg-deb", DEB_STUB),
        ("bsdtar", BSDTAR_STUB),
    ] {
        let path = stubs.join(name);
        fs::write(&path, text).expect("stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("stub mode");
    }
    let packages = directory.path().join("packages");
    fs::create_dir(&packages).expect("packages");
    NameTree {
        directory,
        stubs,
        packages,
    }
}

impl NameTree {
    /// A package file called `name` whose stub reports `version` (with the newline `dpkg-deb`
    /// prints and the others do not).
    fn package(&self, name: &str, version: &str) -> std::path::PathBuf {
        let path = self.packages.join(name);
        let text = if name.ends_with(".deb") {
            format!("{version}\n")
        } else {
            version.to_owned()
        };
        fs::write(&path, text).expect("package");
        path
    }

    fn run(&self, upstream: &str, files: &[&Path]) -> std::process::Output {
        let mut command = common::sandboxed("bash", self.directory.path());
        command
            .arg(self.directory.path().join("tree/packaging/name-assets.sh"))
            .arg(upstream)
            .args(files);
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
        command.env(
            "PATH",
            common::path_with_stubs(&format!("{}:{path}", self.stubs.display())),
        );
        common::run_other_tool(&mut command)
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.packages)
            .expect("read_dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

#[test]
fn each_format_is_renamed_to_the_upstream_version_and_keeps_its_revision() {
    // (file as the tool names it, version the package reports, upstream, file after)
    let cases = [
        (
            "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.x86_64.rpm",
            "0.1.0~beta.1",
            "0.1.0-beta.1",
            "proton-drive-sync-engine-0.1.0-beta.1-1.fc43.x86_64.rpm",
        ),
        (
            "proton-drive-sync-engine-0.1.0-1.fc43.x86_64.rpm",
            "0.1.0",
            "0.1.0",
            "proton-drive-sync-engine-0.1.0-1.fc43.x86_64.rpm",
        ),
        (
            "proton-drive-sync-engine_0.1.0~beta.1-1_amd64.deb",
            "0.1.0~beta.1-1",
            "0.1.0-beta.1",
            "proton-drive-sync-engine_0.1.0-beta.1-1_amd64.deb",
        ),
        (
            "proton-drive-sync-engine_0.1.0-1_amd64.deb",
            "0.1.0-1",
            "0.1.0",
            "proton-drive-sync-engine_0.1.0-1_amd64.deb",
        ),
        // A packaging-only rebuild raises the revision; the upstream part is what is compared.
        (
            "proton-drive-sync-engine_0.1.0~beta.1-2_amd64.deb",
            "0.1.0~beta.1-2",
            "0.1.0-beta.1",
            "proton-drive-sync-engine_0.1.0-beta.1-2_amd64.deb",
        ),
        (
            "proton-drive-sync-engine-0.1.0beta1-1-x86_64.pkg.tar.zst",
            "0.1.0beta1-1",
            "0.1.0-beta.1",
            "proton-drive-sync-engine-0.1.0-beta.1-1-x86_64.pkg.tar.zst",
        ),
        (
            "proton-drive-sync-engine-0.1.0-1-x86_64.pkg.tar.zst",
            "0.1.0-1",
            "0.1.0",
            "proton-drive-sync-engine-0.1.0-1-x86_64.pkg.tar.zst",
        ),
    ];
    for (file, reported, upstream, after) in cases {
        let tree = name_tree();
        let path = tree.package(file, reported);
        let output = tree.run(upstream, &[&path]);
        assert!(
            output.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(tree.names(), [after], "{file}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), after);
        assert!(!after.contains('~'));
    }
}

#[test]
fn several_files_are_renamed_in_one_call() {
    let tree = name_tree();
    let version = "0.1.0~beta.1";
    let source = tree.package(
        "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.src.rpm",
        version,
    );
    let main = tree.package(
        "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.x86_64.rpm",
        version,
    );
    let nemo = tree.package(
        "proton-drive-sync-engine-nemo-0.1.0~beta.1-1.fc43.noarch.rpm",
        version,
    );
    let output = tree.run("0.1.0-beta.1", &[&source, &main, &nemo]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        tree.names(),
        [
            "proton-drive-sync-engine-0.1.0-beta.1-1.fc43.src.rpm",
            "proton-drive-sync-engine-0.1.0-beta.1-1.fc43.x86_64.rpm",
            "proton-drive-sync-engine-nemo-0.1.0-beta.1-1.fc43.noarch.rpm",
        ]
    );
}

#[test]
fn name_assets_refuses_what_it_should_not_rename() {
    let tree = name_tree();
    let path = tree.package("proton-drive-sync-engine-0.1.0-1.fc43.x86_64.rpm", "0.1.0");
    let deb = tree.package("proton-drive-sync-engine_0.1.0-1_amd64.deb", "0.1.0-1");
    let arch = tree.package(
        "proton-drive-sync-engine-0.1.0-1-x86_64.pkg.tar.zst",
        "0.1.0-1",
    );
    let refused = |upstream: &str, files: &[&Path], needle: &str| {
        let output = tree.run(upstream, files);
        assert!(
            !output.status.success(),
            "{upstream} {files:?} was accepted"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{upstream} {files:?}: {stderr}");
    };
    // An upstream version version.sh would not accept.
    refused("garbage name", &[&path], "not an upstream version");
    refused("0.1.0-preview.1", &[&path], "not an upstream version");
    // A package that is not that version: another release, or a pre-release spelled as the tag.
    refused(
        "0.2.0",
        &[&path],
        "says version '0.1.0', but 0.2.0 is '0.2.0'",
    );
    refused(
        "0.1.0-beta.1",
        &[&path],
        "but 0.1.0-beta.1 is '0.1.0~beta.1' for a .rpm package",
    );
    refused("0.2.0", &[&deb], "for a .deb package");
    refused(
        "0.1.0-beta.1",
        &[&arch],
        "is '0.1.0beta1' for a .pkg.tar.zst package",
    );
    // A file that is not a package, a missing file, a name without the version, no arguments.
    let other = tree.packages.join("notes.txt");
    fs::write(&other, "x").expect("write");
    refused("0.1.0", &[&other], "not an .rpm, .deb or .pkg.tar.zst");
    refused(
        "0.1.0",
        &[&tree.packages.join("absent.rpm")],
        "is not a file",
    );
    let nameless = tree.package("proton.rpm", "0.1.0");
    refused(
        "0.1.0",
        &[&nameless],
        "does not contain the package version",
    );
    let usage = tree.run("0.1.0", &[]);
    assert!(!usage.status.success());
    assert!(String::from_utf8_lossy(&usage.stderr).contains("usage:"));
    // None of the refusals renamed a file.
    assert_eq!(
        tree.names(),
        [
            "notes.txt",
            "proton-drive-sync-engine-0.1.0-1-x86_64.pkg.tar.zst",
            "proton-drive-sync-engine-0.1.0-1.fc43.x86_64.rpm",
            "proton-drive-sync-engine_0.1.0-1_amd64.deb",
            "proton.rpm",
        ]
    );
}

#[test]
fn version_spell_prints_each_format_and_refuses_what_it_cannot_spell() {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (format, upstream, spelled) in [
        ("rpm", "0.1.0-beta.1", "0.1.0~beta.1"),
        ("deb", "0.1.0-beta.1", "0.1.0~beta.1-1"),
        ("arch", "0.1.0-beta.1", "0.1.0beta1"),
        ("rpm", "0.1.0", "0.1.0"),
        ("deb", "0.1.0", "0.1.0-1"),
        ("arch", "0.1.0", "0.1.0"),
    ] {
        let output = version_script(root, directory.path(), None, &["spell", format, upstream]);
        assert!(output.status.success(), "{format} {upstream}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{spelled}\n")
        );
    }
    for args in [
        ["spell", "rpm", "garbage"],
        ["spell", "rpm", "0.1.0-preview.1"],
        ["spell", "snap", "0.1.0"],
    ] {
        let output = version_script(root, directory.path(), None, &args);
        assert!(!output.status.success(), "{args:?} was accepted");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn a_failed_srcinfo_leaves_no_half_written_tree_and_a_rerun_works() {
    let tree = aur_tree();
    let out = tree.directory.path().join("out");
    let args = [
        "beta",
        tree.assets.to_str().expect("utf-8"),
        out.to_str().expect("utf-8"),
    ];
    let makepkg = tree.stubs.join("makepkg");
    let working = fs::read_to_string(&makepkg).expect("stub");
    fs::write(&makepkg, "#!/bin/sh\necho 'makepkg: broken' >&2\nexit 1\n").expect("failing stub");

    let failed = aur_script(&tree, &args);
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("makepkg --printsrcinfo failed"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(!out.exists(), "a half-written tree was left at the out-dir");
    let leftovers: Vec<_> = fs::read_dir(tree.directory.path())
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains("partial"))
        .collect();
    assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");

    // An empty directory the caller made first is still there and still usable.
    fs::create_dir(&out).expect("out");
    assert!(!aur_script(&tree, &args).status.success());
    assert!(out.is_dir() && fs::read_dir(&out).expect("read_dir").next().is_none());

    // The same command works once makepkg does, with no clean-up in between.
    fs::write(&makepkg, working).expect("working stub");
    let again = aur_script(&tree, &args);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(out.join("PKGBUILD").is_file() && out.join(".SRCINFO").is_file());
    let mode = fs::metadata(&out).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
}

#[test]
fn name_assets_compares_versions_exactly_not_by_prefix() {
    let tree = name_tree();
    // beta10 starts with beta1, and 0.1.0.1 starts with 0.1.0: neither is the version asked for.
    let arch = tree.package(
        "proton-drive-sync-engine-0.1.0beta10-1-x86_64.pkg.tar.zst",
        "0.1.0beta10-1",
    );
    let rpm = tree.package(
        "proton-drive-sync-engine-0.1.0~beta.10-1.fc43.x86_64.rpm",
        "0.1.0~beta.10",
    );
    let longer = tree.package(
        "proton-drive-sync-engine-0.1.0.1-1.fc43.x86_64.rpm",
        "0.1.0.1",
    );
    let deb = tree.package(
        "proton-drive-sync-engine_0.1.0~beta.10-1_amd64.deb",
        "0.1.0~beta.10-1",
    );
    for (file, upstream) in [
        (&arch, "0.1.0-beta.1"),
        (&rpm, "0.1.0-beta.1"),
        (&longer, "0.1.0"),
        (&deb, "0.1.0-beta.1"),
    ] {
        let output = tree.run(upstream, &[file]);
        assert!(
            !output.status.success(),
            "{file:?} was renamed for {upstream}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("the package says version"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // A pkgrel is a number, optionally "<number>.<number>". Anything else after the last "-" is
    // not a pkgrel, and the package is refused as unreadable instead of being guessed at.
    for (reported, accepted) in [
        ("0.1.0beta1-1", true),
        ("0.1.0beta1-12", true),
        ("0.1.0beta1-1.1", true),
        ("0.1.0beta1-1a", false),
        ("0.1.0beta1-x", false),
        ("0.1.0beta1-", false),
        ("0.1.0beta1", false),
        ("0.1.0beta1-1.", false),
    ] {
        let tree = name_tree();
        let name = format!("proton-drive-sync-engine-{reported}-x86_64.pkg.tar.zst");
        let package = tree.package(&name, reported);
        let output = tree.run("0.1.0-beta.1", &[&package]);
        assert_eq!(output.status.success(), accepted, "pkgver {reported}");
        if !accepted {
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("could not read the package version"),
                "pkgver {reported}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn a_bad_file_last_in_the_list_renames_nothing() {
    let tree = name_tree();
    let version = "0.1.0~beta.1";
    let first = tree.package(
        "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.src.rpm",
        version,
    );
    let second = tree.package(
        "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.x86_64.rpm",
        version,
    );
    let wrong = tree.package(
        "proton-drive-sync-engine-nemo-0.2.0-1.fc43.noarch.rpm",
        "0.2.0",
    );
    let output = tree.run("0.1.0-beta.1", &[&first, &second, &wrong]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("the package says version '0.2.0'"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "names were printed before the refusal"
    );
    assert_eq!(
        tree.names(),
        [
            "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.src.rpm",
            "proton-drive-sync-engine-0.1.0~beta.1-1.fc43.x86_64.rpm",
            "proton-drive-sync-engine-nemo-0.2.0-1.fc43.noarch.rpm",
        ]
    );

    // A rename that would overwrite a file already there is refused up front too.
    fs::remove_file(&wrong).expect("remove");
    fs::write(
        tree.packages
            .join("proton-drive-sync-engine-0.1.0-beta.1-1.fc43.x86_64.rpm"),
        "taken",
    )
    .expect("taken");
    let clash = tree.run("0.1.0-beta.1", &[&first, &second]);
    assert!(!clash.status.success());
    assert!(String::from_utf8_lossy(&clash.stderr).contains("refusing to overwrite"));
    assert!(first.exists() && second.exists());
}

fn aur_script_in(tree: &AurTree, cwd: &Path, args: &[&str]) -> std::process::Output {
    let mut command = common::sandboxed("bash", tree.directory.path());
    command
        .arg(tree.directory.path().join("tree/packaging/arch/aur.sh"))
        .args(args)
        .current_dir(cwd);
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
    command.env(
        "PATH",
        common::path_with_stubs(&format!("{}:{path}", tree.stubs.display())),
    );
    common::run_other_tool(&mut command)
}

#[test]
fn the_current_directory_can_be_the_out_dir() {
    let tree = aur_tree();
    let cwd = tree.directory.path().join("work");
    fs::create_dir(&cwd).expect("cwd");
    fs::set_permissions(&cwd, fs::Permissions::from_mode(0o700)).expect("mode");
    let output = aur_script_in(
        &tree,
        &cwd,
        &["beta", tree.assets.to_str().expect("utf-8"), "."],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut names: Vec<String> = fs::read_dir(&cwd)
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [".SRCINFO", "PKGBUILD", "proton-drive-sync-engine.install"]
    );
    // A directory the caller made keeps its own mode.
    let mode = fs::metadata(&cwd).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn a_symlink_to_an_empty_directory_can_be_the_out_dir() {
    let tree = aur_tree();
    let target = tree.directory.path().join("target");
    fs::create_dir(&target).expect("target");
    let link = tree.directory.path().join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let output = aur_script(
        &tree,
        &[
            "stable",
            tree.assets.to_str().expect("utf-8"),
            link.to_str().expect("utf-8"),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::symlink_metadata(&link)
            .expect("link")
            .file_type()
            .is_symlink()
    );
    assert!(target.join("PKGBUILD").is_file() && target.join(".SRCINFO").is_file());
    let leftovers: Vec<_> = fs::read_dir(tree.directory.path())
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains("partial"))
        .collect();
    assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");
}

// ---- packaging/smoke-test.sh (design D13) ----
//
// The script runs inside a clean container after a package install. Here it runs against a fake
// root (`SMOKE_ROOT`) and a fake /proc (`SMOKE_PROC`), with stubs for the binaries and `ldd`, so
// nothing on this machine is looked at or started. The stub `proton-sync` logs every argument
// list to `ran.log`: the script may run the CLIs only with `--version` and `doctor`.

struct SmokeTree {
    directory: tempfile::TempDir,
    root: std::path::PathBuf,
    proc: std::path::PathBuf,
    stubs: std::path::PathBuf,
}

const SMOKE_VERSION: &str = "0.1.0-beta.1";
// The fake root's binaries, spelled in two pieces: `tests/test_isolation.rs` fails a file with a
// string literal that ends in one of this crate's binary names (a bare name resolves to the
// installed one). These are files in a fake tree, never run by name.
const SMOKE_DAEMON: &str = concat!("proton", "-syncd");
const SMOKE_CLI: &str = concat!("proton", "-sync");
const DOCTOR_REPORT: &str = "  missing  proton-drive  proton-drive was not found on PATH\n\
           help: https://proton.me/drive/download\n";
const ARCH_NOTE: &str = "==> Proton's proton-drive command-line client must be installed and signed in\n\
     ==> Download it from https://proton.me/drive/download, then run: proton-drive auth login\n";

fn write_executable(path: &Path, text: &str) {
    write_file(path, text);
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("mode");
}

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    fs::write(path, text).expect("write");
}

/// A fake install of the main package for `family` (`rpm`, `deb` or `arch`) that passes.
fn smoke_tree(family: &str) -> SmokeTree {
    let directory = tempdir().expect("tempdir");
    let marks = directory.path();
    let root = marks.join("root");
    let proc = marks.join("proc");
    let stubs = marks.join("stubs");
    write_file(&proc.join("1/comm"), "init\n");
    fs::create_dir_all(&stubs).expect("stubs");
    let log = marks.join("ran.log");
    // Every stub binary records its arguments; the GUI must never be run at all.
    write_executable(
        &root.join(format!("usr/bin/{SMOKE_DAEMON}")),
        &format!(
            "#!/bin/sh\necho \"syncd $*\" >> '{log}'\n[ \"$1\" = --version ] || exit 99\necho 'proton-syncd {SMOKE_VERSION}'\n",
            log = log.display()
        ),
    );
    write_file(&marks.join("doctor.txt"), DOCTOR_REPORT);
    write_file(&marks.join("doctor.exit"), "1");
    // `doctor.socket` / `doctor.daemon` in the test's directory make the stub misbehave.
    write_executable(
        &root.join(format!("usr/bin/{SMOKE_CLI}")),
        &format!(
            "#!/bin/sh\necho \"sync $*\" >> '{log}'\n\
             case \"$1\" in\n\
             --version) echo 'proton-sync {SMOKE_VERSION}' ;;\n\
             doctor)\n\
               [ -e '{marks}/doctor.socket' ] && mkdir -p \"$XDG_RUNTIME_DIR/proton-sync\"\n\
               [ -e '{marks}/doctor.sockfile' ] && python3 -c 'import socket,sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])' \"$XDG_STATE_HOME/control.sock\"\n\
               [ -e '{marks}/doctor.daemon' ] && mkdir -p '{marks}/proc/4242' && echo proton-syncd > '{marks}/proc/4242/comm'\n\
               cat '{marks}/doctor.txt'; exit \"$(cat '{marks}/doctor.exit')\" ;;\n\
             *) exit 99 ;;\n\
             esac\n",
            log = log.display(),
            marks = marks.display()
        ),
    );
    write_executable(
        &root.join("usr/bin/proton-sync-gui"),
        &format!("#!/bin/sh\necho \"gui $*\" >> '{}'\n", log.display()),
    );
    write_file(
        &root.join("usr/lib/systemd/user/proton-syncd.service"),
        "[Service]\nExecStart=/usr/bin/proton-syncd --config %h/.config/proton-sync.toml\n",
    );
    for path in [
        "usr/share/applications/app.protondrivesync.engine.desktop",
        "usr/share/metainfo/app.protondrivesync.engine.metainfo.xml",
        "usr/share/icons/hicolor/256x256/apps/app.protondrivesync.engine.png",
        "usr/share/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg",
    ] {
        write_file(&root.join(path), "x\n");
    }
    let licences: &[&str] = if family == "deb" {
        &["usr/share/doc/proton-drive-sync-engine/copyright"]
    } else {
        &[
            "usr/share/licenses/proton-drive-sync-engine/LICENSE",
            "usr/share/licenses/proton-drive-sync-engine/OFL-instrument-sans.txt",
            "usr/share/licenses/proton-drive-sync-engine/OFL-ibm-plex-mono.txt",
        ]
    };
    for path in licences {
        write_file(&root.join(path), "x\n");
    }
    write_executable(
        &stubs.join("ldd"),
        "#!/bin/sh\necho '\tlibc.so.6 => /lib/libc.so.6 (0x1)'\n",
    );
    // The host tools the script and the stub binaries run, linked into one directory.
    let hostbin = marks.join("hostbin");
    fs::create_dir_all(&hostbin).expect("hostbin");
    for tool in [
        "bash", "sh", "env", "timeout", "mktemp", "chmod", "mkdir", "rm", "find", "grep", "tr",
        "cut", "cat", "paste", "python3",
    ] {
        let found = ["/usr/bin", "/bin"]
            .iter()
            .map(|dir| Path::new(dir).join(tool))
            .find(|path| path.exists())
            .unwrap_or_else(|| panic!("host tool {tool} not found"));
        std::os::unix::fs::symlink(found, hostbin.join(tool)).expect("symlink");
    }
    // The validators exist on a developer's machine too, and reject these placeholder files.
    for validator in ["desktop-file-validate", "appstreamcli"] {
        write_executable(&stubs.join(validator), "#!/bin/sh\nexit 0\n");
    }
    write_file(&marks.join("first.txt"), ARCH_NOTE);
    write_file(
        &marks.join("second.txt"),
        "reinstalling proton-drive-sync-engine...\n",
    );
    SmokeTree {
        directory,
        root,
        proc,
        stubs,
    }
}

impl SmokeTree {
    fn file(&self, name: &str) -> std::path::PathBuf {
        self.directory.path().join(name)
    }

    fn command(&self) -> std::process::Command {
        let mut command = common::sandboxed("bash", self.directory.path());
        command
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/smoke-test.sh"))
            .env("SMOKE_ROOT", &self.root)
            .env("SMOKE_PROC", &self.proc)
            // The sandbox's refusing `curl`/`secret-tool` (present, as the script's PATH check
            // wants), this test's stubs, then a directory of symlinks to the host tools the
            // script uses and nothing else: the host's `proton-drive`, `proton-sync` and
            // `proton-syncd` cannot be found.
            .env(
                "PATH",
                common::path_with_stubs(&format!(
                    "{}:{}",
                    self.stubs.display(),
                    self.file("hostbin").display()
                )),
            );
        command
    }

    fn run(&self, family: &str) -> std::process::Output {
        let mut command = self.command();
        command.arg(SMOKE_VERSION).arg(family);
        if family == "arch" {
            command
                .arg(self.file("first.txt"))
                .arg(self.file("second.txt"));
        }
        common::run_other_tool(&mut command)
    }

    fn ran(&self) -> Vec<String> {
        fs::read_to_string(self.file("ran.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn smoke_test_passes_a_good_install_in_each_family_and_runs_only_version_and_doctor() {
    for family in ["rpm", "deb", "arch"] {
        let tree = smoke_tree(family);
        let output = tree.run(family);
        assert!(output.status.success(), "{family}: {}", stderr_of(&output));
        assert!(
            !stderr_of(&output).contains("command not found"),
            "{family}"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("all checks passed"), "{family}: {stdout}");
        // A CI log must show that the validators ran, not only that nothing failed.
        for line in [
            "ok: desktop-file-validate accepts the desktop entry",
            "ok: appstreamcli accepts the metainfo",
        ] {
            assert!(stdout.contains(line), "{family}: no '{line}' in {stdout}");
        }
        // The spec: the binaries run only with --version, except `proton-sync doctor`; never the GUI.
        assert_eq!(
            tree.ran(),
            ["syncd --version", "sync --version", "sync doctor"],
            "{family}"
        );
    }
}

#[test]
fn smoke_test_names_each_thing_that_is_wrong_and_exits_1() {
    // (what is broken, family, how, the part of the failure message that names it)
    type Break = fn(&SmokeTree);
    let cases: Vec<(&str, &str, Break, &str)> = vec![
        (
            "daemon prints another version",
            "rpm",
            |t| {
                let path = t.root.join(format!("usr/bin/{SMOKE_DAEMON}"));
                let text = fs::read_to_string(&path)
                    .unwrap()
                    .replace(SMOKE_VERSION, "0.0.9");
                write_executable(&path, &text);
            },
            "proton-syncd --version printed 'proton-syncd 0.0.9'",
        ),
        (
            "control CLI is missing",
            "rpm",
            |t| fs::remove_file(t.root.join(format!("usr/bin/{SMOKE_CLI}"))).unwrap(),
            "/usr/bin/proton-sync is not installed",
        ),
        (
            "GUI is missing",
            "rpm",
            |t| fs::remove_file(t.root.join("usr/bin/proton-sync-gui")).unwrap(),
            "/usr/bin/proton-sync-gui is not installed",
        ),
        (
            "unit runs another program",
            "deb",
            |t| {
                write_file(
                    &t.root.join("usr/lib/systemd/user/proton-syncd.service"),
                    "[Service]\nExecStart=/usr/local/bin/proton-syncd\n",
                );
            },
            "ExecStart does not begin with /usr/bin/",
        ),
        (
            "unit is missing",
            "deb",
            |t| fs::remove_file(t.root.join("usr/lib/systemd/user/proton-syncd.service")).unwrap(),
            "proton-syncd.service is not installed",
        ),
        (
            "unit enabled for every user (the old debian/rules)",
            "deb",
            |t| {
                let wants = t.root.join("etc/systemd/user/default.target.wants");
                fs::create_dir_all(&wants).unwrap();
                std::os::unix::fs::symlink(
                    "/usr/lib/systemd/user/proton-syncd.service",
                    wants.join("proton-syncd.service"),
                )
                .unwrap();
            },
            "the install enabled the unit",
        ),
        (
            "unit enabled under another target",
            "rpm",
            |t| {
                let wants = t
                    .root
                    .join("usr/lib/systemd/user/graphical-session.target.wants");
                write_file(&wants.join("proton-syncd.service"), "x");
            },
            "the install enabled the unit",
        ),
        (
            "a library does not resolve",
            "rpm",
            |t| {
                write_executable(
                    &t.stubs.join("ldd"),
                    "#!/bin/sh\necho '\tlibwebkit2gtk-4.1.so.0 => not found'\n",
                );
            },
            "ldd proton-sync-gui",
        ),
        (
            "desktop entry does not validate",
            "rpm",
            |t| {
                write_executable(
                    &t.stubs.join("desktop-file-validate"),
                    "#!/bin/sh\nexit 1\n",
                )
            },
            "desktop-file-validate rejects",
        ),
        (
            "metainfo does not validate",
            "rpm",
            |t| write_executable(&t.stubs.join("appstreamcli"), "#!/bin/sh\nexit 3\n"),
            "appstreamcli rejects",
        ),
        (
            "a licence text is missing",
            "arch",
            |t| {
                fs::remove_file(
                    t.root
                        .join("usr/share/licenses/proton-drive-sync-engine/OFL-ibm-plex-mono.txt"),
                )
                .unwrap();
            },
            "OFL-ibm-plex-mono.txt is not installed",
        ),
        (
            "the deb has no copyright file",
            "deb",
            |t| {
                fs::remove_file(
                    t.root
                        .join("usr/share/doc/proton-drive-sync-engine/copyright"),
                )
                .unwrap();
            },
            "copyright is not installed",
        ),
        (
            "the desktop entry is missing",
            "rpm",
            |t| {
                fs::remove_file(
                    t.root
                        .join("usr/share/applications/app.protondrivesync.engine.desktop"),
                )
                .unwrap()
            },
            "app.protondrivesync.engine.desktop is not installed",
        ),
        (
            "the metainfo is missing",
            "rpm",
            |t| {
                fs::remove_file(
                    t.root
                        .join("usr/share/metainfo/app.protondrivesync.engine.metainfo.xml"),
                )
                .unwrap()
            },
            "app.protondrivesync.engine.metainfo.xml is not installed",
        ),
        (
            "the 256px icon is missing",
            "deb",
            |t| {
                fs::remove_file(
                    t.root.join(
                        "usr/share/icons/hicolor/256x256/apps/app.protondrivesync.engine.png",
                    ),
                )
                .unwrap()
            },
            "256x256/apps/app.protondrivesync.engine.png is not installed",
        ),
        (
            "the scalable icon is missing",
            "arch",
            |t| {
                fs::remove_file(
                    t.root.join(
                        "usr/share/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg",
                    ),
                )
                .unwrap()
            },
            "scalable/apps/app.protondrivesync.engine.svg is not installed",
        ),
        (
            "unit ExecStart is a longer name that starts the same",
            "rpm",
            |t| {
                write_file(
                    &t.root.join("usr/lib/systemd/user/proton-syncd.service"),
                    "[Service]\nExecStart=/usr/bin/proton-syncdX\n",
                );
            },
            "ExecStart does not begin with /usr/bin/",
        ),
        (
            "unit enabled under /etc/xdg/systemd",
            "deb",
            |t| {
                write_file(
                    &t.root
                        .join("etc/xdg/systemd/user/default.target.wants/proton-syncd.service"),
                    "x",
                );
            },
            "the install enabled the unit",
        ),
        (
            "unit required by a target",
            "rpm",
            |t| {
                write_file(
                    &t.root
                        .join("etc/systemd/user/default.target.requires/proton-syncd.service"),
                    "x",
                );
            },
            "the install enabled the unit",
        ),
        (
            "ldd exits non-zero without printing 'not found'",
            "rpm",
            |t| {
                write_executable(
                    &t.stubs.join("ldd"),
                    "#!/bin/sh\necho '\tnot a dynamic executable'\nexit 1\n",
                );
            },
            "not a dynamic executable",
        ),
        (
            "ldd exits non-zero and prints nothing",
            "rpm",
            |t| write_executable(&t.stubs.join("ldd"), "#!/bin/sh\nexit 2\n"),
            "ldd proton-sync-gui (exit 2)",
        ),
        (
            "the daemon's version only ends the same as the release's",
            "rpm",
            |t| {
                let path = t.root.join(format!("usr/bin/{SMOKE_DAEMON}"));
                let text = fs::read_to_string(&path)
                    .unwrap()
                    .replace(SMOKE_VERSION, &format!("1{SMOKE_VERSION}"));
                write_executable(&path, &text);
            },
            "printed 'proton-syncd 10.1.0-beta.1'",
        ),
        (
            "doctor's missing line is capitalised",
            "rpm",
            |t| {
                write_file(
                    &t.file("doctor.txt"),
                    "  Missing  proton-drive  x\n  help: https://proton.me/drive/download\n",
                );
            },
            "did not report proton-drive as missing",
        ),
        (
            "doctor reports another check missing and only mentions proton-drive",
            "rpm",
            |t| {
                write_file(
                    &t.file("doctor.txt"),
                    "  missing  session-bus  no proton-drive session\n  ok       proton-drive  1.4\n  help: https://proton.me/drive/download\n",
                );
            },
            "did not report proton-drive as missing",
        ),
        (
            "the first pacman output is missing",
            "arch",
            |t| fs::remove_file(t.file("first.txt")).unwrap(),
            "first.txt: no such pacman output",
        ),
        (
            "the second pacman output is missing",
            "arch",
            |t| fs::remove_file(t.file("second.txt")).unwrap(),
            "second.txt: no such pacman output",
        ),
        (
            "doctor's missing line has text before it",
            "rpm",
            |t| {
                write_file(
                    &t.file("doctor.txt"),
                    "xx missing proton-drive x\n  help: https://proton.me/drive/download\n",
                );
            },
            "did not report proton-drive as missing",
        ),
        (
            "doctor's missing line is about a longer check name",
            "rpm",
            |t| {
                write_file(
                    &t.file("doctor.txt"),
                    "  missing  proton-drive-foo  x\n  help: https://proton.me/drive/download\n",
                );
            },
            "did not report proton-drive as missing",
        ),
        (
            "doctor exits 0",
            "rpm",
            |t| write_file(&t.file("doctor.exit"), "0"),
            "proton-sync doctor exited 0, expected 1",
        ),
        (
            "doctor exits 3 (could not check)",
            "rpm",
            |t| write_file(&t.file("doctor.exit"), "3"),
            "proton-sync doctor exited 3, expected 1",
        ),
        (
            "doctor does not print the download URL",
            "rpm",
            |t| {
                write_file(
                    &t.file("doctor.txt"),
                    "  missing  proton-drive  proton-drive was not found on PATH\n",
                );
            },
            "did not print https://proton.me/drive/download",
        ),
        (
            "doctor does not report proton-drive missing",
            "rpm",
            |t| {
                write_file(
                    &t.file("doctor.txt"),
                    "  ok       proton-drive  proton-drive 1.4.2\n  help: https://proton.me/drive/download\n",
                );
            },
            "did not report proton-drive as missing",
        ),
        (
            "doctor starts the daemon",
            "rpm",
            |t| write_file(&t.file("doctor.daemon"), ""),
            "a proton-syncd process exists after proton-sync doctor",
        ),
        (
            "doctor leaves a control socket directory",
            "rpm",
            |t| write_file(&t.file("doctor.socket"), ""),
            "left a socket or a socket directory",
        ),
        (
            "doctor binds a control socket outside the runtime directory",
            "rpm",
            |t| write_file(&t.file("doctor.sockfile"), ""),
            // The message lists what it found (through `paste`).
            "control.sock",
        ),
        (
            "a daemon is already running",
            "rpm",
            |t| write_file(&t.proc.join("77/comm"), "proton-syncd\n"),
            "already running before the doctor check",
        ),
        (
            "proton-drive is on PATH",
            "rpm",
            |t| write_executable(&t.stubs.join("proton-drive"), "#!/bin/sh\nexit 0\n"),
            "needs a container without proton-drive",
        ),
        (
            "the Arch install printed no download URL",
            "arch",
            |t| write_file(&t.file("first.txt"), "then run: proton-drive auth login\n"),
            "pacman -U did not print https://proton.me/drive/download",
        ),
        (
            "the Arch install printed no sign-in command",
            "arch",
            |t| {
                write_file(
                    &t.file("first.txt"),
                    "see https://proton.me/drive/download\n",
                );
            },
            "did not print 'proton-drive auth login'",
        ),
        (
            "the Arch reinstall printed the note again",
            "arch",
            |t| write_file(&t.file("second.txt"), ARCH_NOTE),
            "printed again on a reinstall",
        ),
    ];
    for (what, family, break_it, expected) in cases {
        let tree = smoke_tree(family);
        break_it(&tree);
        let output = tree.run(family);
        let stderr = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{what}: {stderr}");
        // A command the script calls that is neither a builtin nor in `hostbin` or the stubs.
        assert!(!stderr.contains("command not found"), "{what}: {stderr}");
        // One broken thing is one failure: no check blames the wrong cause.
        assert_eq!(stderr.matches("FAIL:").count(), 1, "{what}: {stderr}");
        assert!(
            stderr.contains(expected),
            "{what}: wanted '{expected}' in:\n{stderr}"
        );
    }
}

#[test]
fn smoke_test_reports_every_failure_not_just_the_first() {
    let tree = smoke_tree("rpm");
    write_file(&tree.file("doctor.exit"), "0");
    fs::remove_file(tree.root.join("usr/bin/proton-sync-gui")).unwrap();
    let stderr = stderr_of(&tree.run("rpm"));
    assert!(
        stderr.contains("proton-sync-gui is not installed"),
        "{stderr}"
    );
    assert!(stderr.contains("expected 1"), "{stderr}");
    assert!(stderr.contains("2 check(s) failed"), "{stderr}");
}

#[test]
fn smoke_test_without_a_fake_root_runs_nothing_on_the_host() {
    for family in ["rpm", "deb"] {
        let tree = smoke_tree(family);
        let mut command = tree.command();
        command
            .env_remove("SMOKE_ROOT")
            .arg(SMOKE_VERSION)
            .arg(family);
        let output = common::run_other_tool(&mut command);
        assert_eq!(output.status.code(), Some(64), "{family}");
        assert!(
            stderr_of(&output).contains("SMOKE_ROOT is empty"),
            "{family}"
        );
        assert!(output.stdout.is_empty(), "no check ran: {output:?}");
        assert!(
            tree.ran().is_empty(),
            "no fake binary ran: {:?}",
            tree.ran()
        );
    }
}

#[test]
fn smoke_test_cannot_find_the_hosts_binaries_on_its_path() {
    let tree = smoke_tree("rpm");
    // The PATH `run` gives the script, probed by a bash with the same environment. The names are
    // spelled in pieces for `tests/test_isolation.rs`.
    let mut probe = tree.command();
    probe.args([
        "-c",
        "for n in proton-drive proton-sync proton-syncd; do command -v $n && exit 1; done; exit 0",
    ]);
    // `command()` already carries the script as its first argument; replace it by a bare probe.
    let mut bare = common::sandboxed("bash", tree.directory.path());
    for (key, value) in probe.get_envs() {
        if let Some(value) = value {
            bare.env(key, value);
        }
    }
    bare.args([
        "-c",
        "for n in proton-drive proton-sync proton-syncd; do command -v $n && exit 1; done; exit 0",
    ]);
    let output = common::run_other_tool(&mut bare);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn smoke_test_refuses_arguments_it_does_not_understand() {
    let tree = smoke_tree("rpm");
    for args in [
        vec![],
        vec![SMOKE_VERSION],
        vec![SMOKE_VERSION, "msi"],
        vec![SMOKE_VERSION, "arch", "only-one-file"],
        vec![SMOKE_VERSION, "rpm", "extra"],
    ] {
        let mut command = tree.command();
        command.args(&args);
        let output = common::run_other_tool(&mut command);
        assert_eq!(
            output.status.code(),
            Some(64),
            "{args:?}: {}",
            stderr_of(&output)
        );
        assert!(stderr_of(&output).contains("usage:"), "{args:?}");
    }
    assert!(tree.ran().is_empty(), "nothing ran: {:?}", tree.ran());
}

// ---- packages.yml's `packages-ok` aggregator ----
//
// The one required check on main: a wrong answer here blocks every pull request (or lets a failed
// smoke install through). The script is lifted out of the workflow file and run over the `needs`
// JSON the runner would give it, so what runs here is the text that ships.

/// The `run:` block of `packages-ok`'s "Check the results" step.
fn aggregator_script() -> String {
    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/packages.yml"),
    )
    .expect("packages.yml");
    let job = workflow
        .split("\n  packages-ok:\n")
        .nth(1)
        .expect("the packages-ok job");
    let step = job
        .split("      - name: Check the results\n")
        .nth(1)
        .expect("its step");
    let body = step
        .split("        run: |\n")
        .nth(1)
        .expect("its run block");
    body.lines()
        .take_while(|line| line.is_empty() || line.starts_with("          "))
        .map(|line| line.strip_prefix("          ").unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The job names `packages-ok` lists in `needs`.
fn aggregator_needs() -> Vec<String> {
    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/packages.yml"),
    )
    .expect("packages.yml");
    let job = workflow
        .split("\n  packages-ok:\n")
        .nth(1)
        .expect("the packages-ok job");
    let line = job
        .lines()
        .find_map(|line| line.trim().strip_prefix("needs: ["))
        .expect("needs");
    line.trim_end_matches(']')
        .split(',')
        .map(|name| name.trim().to_owned())
        .collect()
}

/// Runs the aggregator on `needs` (job -> result) with `build` as `changes`' output.
fn aggregate(build: &str, results: &[(&str, &str)]) -> std::process::Output {
    let directory = tempdir().expect("tempdir");
    let mut needs = serde_json::Map::new();
    for (job, result) in results {
        let mut entry = serde_json::json!({ "result": result, "outputs": {} });
        // `build` "none" is a `changes` job that published no output at all.
        if *job == "changes" && build != "none" {
            entry["outputs"] = serde_json::json!({ "build": build, "fedora": "43" });
        }
        needs.insert((*job).to_owned(), entry);
    }
    let mut command = common::sandboxed("bash", directory.path());
    command
        .arg("-c")
        .arg(aggregator_script())
        .env("NEEDS", serde_json::Value::Object(needs).to_string());
    common::run_other_tool(&mut command)
}

/// Every job in `needs` with `result`, except the ones in `except`.
fn all_jobs<'a>(result: &'a str, except: &[(&'a str, &'a str)]) -> Vec<(String, String)> {
    aggregator_needs()
        .into_iter()
        .map(|job| {
            let own = except
                .iter()
                .find(|(name, _)| *name == job)
                .map_or(result, |(_, own)| *own);
            (job, own.to_owned())
        })
        .collect()
}

fn aggregate_all(build: &str, jobs: &[(String, String)]) -> std::process::Output {
    let pairs: Vec<(&str, &str)> = jobs
        .iter()
        .map(|(job, result)| (job.as_str(), result.as_str()))
        .collect();
    aggregate(build, &pairs)
}

#[test]
fn packages_ok_lists_every_build_and_smoke_job() {
    let needs = aggregator_needs();
    for job in [
        "changes",
        "tarball",
        "rpm",
        "deb",
        "arch",
        "smoke-rpm",
        "smoke-deb",
        "smoke-arch",
        "apt-repo",
        "release-assets",
    ] {
        assert!(
            needs.iter().any(|name| name == job),
            "{job} not in {needs:?}"
        );
    }
    // And no job in the workflow that builds or tests is missing from the list.
    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/packages.yml"),
    )
    .expect("packages.yml");
    let jobs: Vec<&str> = workflow
        .lines()
        .skip_while(|line| *line != "jobs:")
        .filter_map(|line| {
            let name = line.strip_prefix("  ")?.strip_suffix(':')?;
            (!name.starts_with(' ') && !name.starts_with('#') && !name.is_empty()).then_some(name)
        })
        .collect();
    for job in jobs.iter().filter(|job| **job != "packages-ok") {
        assert!(needs.iter().any(|name| name == job), "{job} is not gated");
    }
    // A smoke job installs what its build job made, so it must wait for it (and is then skipped
    // with it when the builds are not needed).
    for (smoke, build) in [
        ("smoke-rpm", "rpm"),
        ("smoke-deb", "deb"),
        ("smoke-arch", "arch"),
        // The apt repository job installs the deb the deb job made.
        ("apt-repo", "deb"),
    ] {
        let block = workflow
            .split(&format!("\n  {smoke}:\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("job {smoke}"));
        let line = block
            .lines()
            .find(|line| line.starts_with("    needs:"))
            .unwrap_or_else(|| panic!("{smoke} has no needs"));
        assert!(
            line.contains(build),
            "{smoke} does not wait for {build}: {line}"
        );
    }
}

#[test]
fn packages_ok_passes_when_everything_passed_or_the_builds_were_not_needed() {
    // Every job succeeded.
    let output = aggregate_all("true", &all_jobs("success", &[]));
    assert!(output.status.success(), "{}", stderr_of(&output));
    // `changes` said no build: every other job skipped (build and smoke alike).
    let output = aggregate_all("false", &all_jobs("skipped", &[("changes", "success")]));
    assert!(output.status.success(), "{}", stderr_of(&output));
}

#[test]
fn packages_ok_fails_for_a_failed_or_cancelled_job_whether_or_not_builds_were_needed() {
    for build in ["true", "false"] {
        for job in aggregator_needs().iter().filter(|job| *job != "changes") {
            for bad in ["failure", "cancelled"] {
                let output = aggregate_all(
                    build,
                    &all_jobs(
                        if build == "true" {
                            "success"
                        } else {
                            "skipped"
                        },
                        &[("changes", "success"), (job, bad)],
                    ),
                );
                let stderr = stderr_of(&output);
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "{job} {bad} (build={build}): {stderr}"
                );
                assert!(
                    stderr.contains(&format!("{job}: {bad}")),
                    "{job} {bad}: {stderr}"
                );
            }
        }
    }
}

#[test]
fn packages_ok_fails_when_a_job_that_had_to_run_was_skipped() {
    // Builds were needed, so a smoke job that never ran is not a pass.
    for job in aggregator_needs().iter().filter(|job| *job != "changes") {
        let output = aggregate_all("true", &all_jobs("success", &[(job, "skipped")]));
        assert_eq!(
            output.status.code(),
            Some(1),
            "{job}: {}",
            stderr_of(&output)
        );
    }
}

#[test]
fn packages_ok_fails_when_the_change_check_gave_no_answer() {
    for build in ["", "none", "maybe"] {
        let output = aggregate_all(build, &all_jobs("success", &[]));
        assert_eq!(output.status.code(), Some(1), "build '{build}'");
    }
    let output = aggregate_all("true", &all_jobs("success", &[("changes", "failure")]));
    assert_eq!(output.status.code(), Some(1));
    // `changes` skipped or failed with build=false is no answer either.
    let output = aggregate_all("false", &all_jobs("skipped", &[("changes", "skipped")]));
    assert_eq!(output.status.code(), Some(1));
}

// --- The release workflow: version.sh's tag questions, the gate and the draft script ----------------

#[test]
fn version_answers_the_tag_questions_the_release_workflow_asks() {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ask = |args: &[&str]| version_script(root, directory.path(), None, args);
    for (tag, upstream) in [
        ("v0.1.0", "0.1.0"),
        ("v0.1.0-beta.1", "0.1.0-beta.1"),
        ("v10.20.30-rc.12", "10.20.30-rc.12"),
        ("v1.0.0-alpha.1", "1.0.0-alpha.1"),
    ] {
        let output = ask(&["tag-version", tag]);
        assert!(output.status.success(), "{tag}: {}", stderr_of(&output));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{upstream}\n")
        );
    }
    // Everything outside the accepted shape is refused with nothing on stdout.
    for tag in [
        "0.1.0",
        "v0.1",
        "v0.1.0-beta",
        "v0.1.0-beta.0",
        "v0.1.0-preview.1",
        "v01.0.0",
        "v0.1.0+b.1",
        "v0.1.0-beta.1 ",
        "V0.1.0",
        "",
    ] {
        let output = ask(&["tag-version", tag]);
        assert_eq!(output.status.code(), Some(1), "'{tag}' was accepted");
        assert!(output.stdout.is_empty(), "'{tag}'");
        assert!(stderr_of(&output).contains("not a release tag"), "'{tag}'");
    }
    for (version, answer) in [
        ("0.1.0", "false"),
        ("0.1.0-beta.1", "true"),
        ("2.0.0-rc.3", "true"),
    ] {
        let output = ask(&["is-prerelease", version]);
        assert!(output.status.success(), "{version}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{answer}\n")
        );
    }
    // A version of the wrong shape is an error, not "false".
    for version in ["v0.1.0", "0.1", "0.1.0-beta"] {
        let output = ask(&["is-prerelease", version]);
        assert_eq!(output.status.code(), Some(1), "{version}");
        assert!(output.stdout.is_empty());
    }
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");
    let expected = cargo
        .lines()
        .find_map(|line| line.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("version line");
    let output = ask(&["current"]);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{expected}\n")
    );
}

/// A git repository in `directory/tree` holding a copy of the version sites and `gate.sh`, with
/// the fixed author a commit needs.
struct GateTree {
    directory: tempfile::TempDir,
    tree: std::path::PathBuf,
}

fn real_git() -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git on PATH")
}

fn gate_tree() -> GateTree {
    let directory = tempdir().expect("tempdir");
    let (tree, _stubs) = copy_of_the_version_sites(directory.path());
    let release = tree.join("packaging/release");
    fs::create_dir_all(&release).expect("release directory");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release/gate.sh"),
        release.join("gate.sh"),
    )
    .expect("gate.sh");
    let gate = GateTree { directory, tree };
    gate.git(&["init", "-q", "-b", "main"]);
    gate.commit("first");
    gate
}

impl GateTree {
    fn git_command(&self, args: &[&str]) -> std::process::Command {
        let mut command = common::sandboxed("git", self.directory.path());
        command
            .current_dir(&self.tree)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args);
        command
    }

    fn git(&self, args: &[&str]) -> String {
        let output = common::run_other_tool(&mut self.git_command(args));
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            stderr_of(&output)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Commits everything and returns the commit.
    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn set_main(&self, commit: &str) {
        self.git(&["update-ref", "refs/remotes/origin/main", commit]);
    }

    /// Runs the gate for `tag`; `path_first` is a directory placed ahead of `PATH`.
    fn gate(&self, tag: &str, path_first: Option<&Path>) -> std::process::Output {
        let mut command = common::sandboxed("bash", self.directory.path());
        command
            .arg(self.tree.join("packaging/release/gate.sh"))
            .arg(tag)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        if let Some(first) = path_first {
            let path = std::env::var("PATH").expect("PATH");
            command.env(
                "PATH",
                common::path_with_stubs(&format!("{}:{path}", first.display())),
            );
        }
        common::run_other_tool(&mut command)
    }
}

#[test]
fn the_gate_passes_a_matching_tag_on_main() {
    let gate = gate_tree();
    let commit = gate.commit("second");
    gate.set_main(&commit);
    let output = gate.gate("v0.1.0", None);
    assert!(output.status.success(), "{}", stderr_of(&output));
}

#[test]
fn the_gate_passes_a_commit_that_is_behind_the_tip_of_main() {
    let gate = gate_tree();
    let tagged = gate.commit("tagged");
    let tip = gate.commit("later work on main");
    gate.set_main(&tip);
    gate.git(&["checkout", "-q", "--detach", &tagged]);
    let output = gate.gate("v0.1.0", None);
    assert!(output.status.success(), "{}", stderr_of(&output));
}

#[test]
fn the_gate_refuses_a_commit_that_is_not_an_ancestor_of_main() {
    let gate = gate_tree();
    let main = gate.commit("on main");
    gate.set_main(&main);
    gate.git(&["checkout", "-q", "-b", "branch"]);
    gate.commit("only on a branch");
    let output = gate.gate("v0.1.0", None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("not an ancestor of refs/remotes/origin/main"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn the_gate_refuses_when_origin_main_cannot_be_read() {
    // A tag-push checkout has no origin/main until it is fetched. Not knowing is not a pass.
    let gate = gate_tree();
    let output = gate.gate("v0.1.0", None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("cannot read refs/remotes/origin/main"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn the_gate_refuses_when_git_cannot_answer_the_ancestry_question() {
    // `git merge-base --is-ancestor` exits 128 when it cannot read a ref. A shim that makes it do
    // that: only exit status 0 may pass.
    let gate = gate_tree();
    let commit = gate.commit("second");
    gate.set_main(&commit);
    let shim = gate.directory.path().join("git-shim");
    fs::create_dir(&shim).expect("shim directory");
    write_executable(
        &shim.join("git"),
        &format!(
            "#!/bin/sh\nfor argument in \"$@\"; do\n  if [ \"$argument\" = merge-base ]; then\n    echo 'fatal: stub: cannot read a ref' >&2\n    exit 128\n  fi\ndone\nexec {} \"$@\"\n",
            real_git().display()
        ),
    );
    let output = gate.gate("v0.1.0", Some(&shim));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("could not tell whether"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn the_gate_names_both_versions_when_the_tag_and_cargo_toml_differ() {
    let gate = gate_tree();
    let commit = gate.commit("second");
    gate.set_main(&commit);
    for tag in ["v0.1.0-beta.2", "v0.1.1", "v0.2.0"] {
        let output = gate.gate(tag, None);
        let stderr = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{tag}: {stderr}");
        assert!(stderr.contains(&tag[1..]), "{tag}: {stderr}");
        assert!(
            stderr.contains("Cargo.toml at this commit says 0.1.0"),
            "{stderr}"
        );
    }
}

#[test]
fn the_gate_refuses_a_tag_of_the_wrong_shape() {
    let gate = gate_tree();
    let commit = gate.commit("second");
    gate.set_main(&commit);
    for tag in ["0.1.0", "v0.1", "v0.1.0-beta", "v0.1.0+b.1", "vv0.1.0"] {
        let output = gate.gate(tag, None);
        assert_eq!(output.status.code(), Some(1), "{tag}");
        assert!(stderr_of(&output).contains("not a release tag"), "{tag}");
    }
}

#[test]
fn the_gate_runs_the_version_check() {
    // Cargo.toml and the tag agree and the commit is on main; one other site does not.
    let gate = gate_tree();
    let pkgbuild = gate.tree.join("packaging/arch/PKGBUILD");
    let text = fs::read_to_string(&pkgbuild).expect("PKGBUILD");
    fs::write(&pkgbuild, text.replacen("pkgver=0.1.0", "pkgver=0.1.1", 1)).expect("edit");
    let commit = gate.commit("drifted");
    gate.set_main(&commit);
    let output = gate.gate("v0.1.0", None);
    let stderr = stderr_of(&output);
    assert_ne!(output.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("PKGBUILD"), "{stderr}");
}

// --- draft.sh, with a stub `gh` -------------------------------------------------------------------

const GH_STUB: &str = r#"#!/bin/sh
# Logs every call. A read (`gh api repos/... --paginate`) is answered from $STUB_GH_RELEASES. Any
# write (release create, upload, delete-asset, edit, or an api call with a method) is logged
# and refused with status 99 unless $STUB_GH_ALLOW_WRITES is set, so a call a test did not expect
# fails loudly.
printf '%s\n' "$*" >> "$STUB_GH_LOG"
previous=""
for argument in "$@"; do
  if [ "$previous" = --notes-file ]; then cp "$argument" "$STUB_GH_NOTES"; fi
  previous="$argument"
done
case "$1 $2" in
  "api "*)
    for argument in "$@"; do
      case "$argument" in -X | --method | --input | -f | -F) echo "stub gh: api write" >&2; exit 99 ;; esac
    done
    # A listing without --paginate would return only the first page of releases.
    case " $* " in *" --paginate "*) ;; *) echo "stub gh: a releases listing without --paginate" >&2; exit 98 ;; esac
    if [ -n "$STUB_GH_API_FAILS" ]; then echo "stub gh: api failed" >&2; exit 1; fi
    cat "$STUB_GH_RELEASES"
    ;;
  "release create" | "release upload" | "release delete-asset" | "release edit" | "release delete")
    if [ -z "$STUB_GH_ALLOW_WRITES" ]; then echo "stub gh: unexpected write: $*" >&2; exit 99; fi
    if [ "$2" = create ]; then echo "https://github.com/o/r/releases/tag/untagged-draft"; fi
    ;;
  *)
    echo "stub gh: unexpected call: $*" >&2
    exit 98
    ;;
esac
"#;

struct DraftTree {
    directory: tempfile::TempDir,
    tree: std::path::PathBuf,
    assets: std::path::PathBuf,
    stubs: std::path::PathBuf,
}

fn draft_tree() -> DraftTree {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tree = directory.path().join("tree");
    fs::create_dir_all(tree.join("packaging/release")).expect("tree");
    for file in [
        "packaging/version.sh",
        "packaging/release/draft.sh",
        "packaging/release-notes.md",
    ] {
        fs::copy(root.join(file), tree.join(file))
            .unwrap_or_else(|error| panic!("{file}: {error}"));
    }
    let stubs = directory.path().join("gh-stubs");
    fs::create_dir(&stubs).expect("stubs");
    write_executable(&stubs.join("gh"), GH_STUB);
    let assets = directory.path().join("assets");
    fs::create_dir(&assets).expect("assets");
    DraftTree {
        directory,
        tree,
        assets,
        stubs,
    }
}

/// The nine files a release holds besides SHA256SUMS, named for `upstream`.
fn asset_names(upstream: &str) -> Vec<String> {
    vec![
        format!("proton-drive-sync-engine-{upstream}.tar.gz"),
        format!("proton-drive-sync-engine-{upstream}-1.fc43.src.rpm"),
        format!("proton-drive-sync-engine-{upstream}-1.fc43.x86_64.rpm"),
        format!("proton-drive-sync-engine-nautilus-{upstream}-1.fc43.noarch.rpm"),
        format!("proton-drive-sync-engine-nemo-{upstream}-1.fc43.noarch.rpm"),
        format!("proton-drive-sync-engine_{upstream}-1_amd64.deb"),
        format!("proton-sync-nautilus_{upstream}-1_all.deb"),
        format!("proton-sync-nemo_{upstream}-1_all.deb"),
        format!("proton-drive-sync-engine-{upstream}-1-x86_64.pkg.tar.zst"),
    ]
}

fn release_json(id: u64, tag: &str, draft: bool, assets: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "tag_name": tag,
        "draft": draft,
        "html_url": format!("https://github.com/o/r/releases/{id}"),
        "assets": assets.iter().map(|name| serde_json::json!({ "name": name })).collect::<Vec<_>>(),
    })
}

impl DraftTree {
    fn fill(&self, upstream: &str) {
        for name in asset_names(upstream) {
            fs::write(self.assets.join(&name), format!("contents of {name}")).expect("asset");
        }
    }

    /// Runs `draft.sh <tag> assets` with `pages` (one JSON array each, as `--paginate` prints them)
    /// as the API's answer. Returns the output and the stub's log.
    fn run(
        &self,
        tag: &str,
        pages: &[Vec<serde_json::Value>],
        allow_writes: bool,
    ) -> (std::process::Output, Vec<String>) {
        let releases = self.directory.path().join("releases.json");
        let text: String = pages
            .iter()
            .map(|page| serde_json::Value::Array(page.clone()).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&releases, text).expect("releases");
        self.run_with(tag, &releases, allow_writes, false)
    }

    fn run_with(
        &self,
        tag: &str,
        releases: &Path,
        allow_writes: bool,
        api_fails: bool,
    ) -> (std::process::Output, Vec<String>) {
        let log = self.directory.path().join("gh.log");
        let mut command = self.command();
        command
            .arg(tag)
            .arg(&self.assets)
            .env("STUB_GH_LOG", &log)
            .env("STUB_GH_RELEASES", releases)
            .env("STUB_GH_NOTES", self.directory.path().join("notes.copy"));
        if allow_writes {
            command.env("STUB_GH_ALLOW_WRITES", "1");
        }
        if api_fails {
            command.env("STUB_GH_API_FAILS", "1");
        }
        let output = common::run_other_tool(&mut command);
        // The real `gh` must never be reached: this stub directory is ahead of the sandbox's
        // refusing `gh`, and a run that lost it would have recorded a hit there.
        let hits = common::stub_hits(&command);
        assert!(hits.is_empty(), "the test reached a refusing stub: {hits}");
        let calls = fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        (output, calls)
    }

    /// `bash draft.sh` in the sandbox, with the stub `gh` first on `PATH` (ahead of the sandbox's
    /// refusing one) and no way to reach the host's GitHub: no token, an empty `gh` config
    /// directory and a host that does not resolve.
    fn command(&self) -> std::process::Command {
        let mut command = common::sandboxed("bash", self.directory.path());
        command
            .arg(self.tree.join("packaging/release/draft.sh"))
            .env("GITHUB_REPOSITORY", "o/r")
            .env("GH_TOKEN", "")
            .env("GITHUB_TOKEN", "")
            .env("GH_CONFIG_DIR", self.directory.path().join("gh-config"))
            .env("GH_HOST", "gh.invalid");
        let mut path = std::ffi::OsString::from(&self.stubs);
        path.push(":");
        path.push(common::path_with_stubs(
            &std::env::var("PATH").expect("PATH"),
        ));
        command.env("PATH", path);
        command
    }

    /// `draft.sh --check-assets <assets dir> <version>`.
    fn check_assets(&self, version: &str) -> std::process::Output {
        let mut command = self.command();
        command.arg("--check-assets").arg(&self.assets).arg(version);
        common::run_other_tool(&mut command)
    }

    fn notes(&self) -> String {
        fs::read_to_string(self.directory.path().join("notes.copy")).expect("the notes file gh saw")
    }

    fn names_in_assets(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.assets)
            .expect("assets")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

fn writes(calls: &[String]) -> Vec<&String> {
    calls
        .iter()
        .filter(|call| {
            [
                "release create",
                "release upload",
                "release delete-asset",
                "release edit",
                "release delete",
            ]
            .iter()
            .any(|write| call.starts_with(write))
        })
        .collect()
}

#[test]
fn a_tag_with_no_release_creates_a_draft_pre_release_with_exactly_the_assets() {
    let tree = draft_tree();
    tree.fill("0.1.0-beta.1");
    let other = release_json(1, "v0.0.9", false, &["old.tar.gz"]);
    let (output, calls) = tree.run("v0.1.0-beta.1", &[vec![other]], true);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let writes = writes(&calls);
    assert_eq!(writes.len(), 1, "{calls:?}");
    let create = writes[0];
    assert!(
        create.starts_with("release create v0.1.0-beta.1 "),
        "{create}"
    );
    for flag in [
        "--draft",
        "--verify-tag",
        "--generate-notes",
        "--notes-file",
        "--prerelease",
        "--repo o/r",
    ] {
        assert!(create.contains(flag), "{flag} missing: {create}");
    }
    // The nine assets and SHA256SUMS, nothing else.
    let mut expected = asset_names("0.1.0-beta.1");
    expected.push("SHA256SUMS".to_owned());
    for name in &expected {
        assert!(
            create.contains(&format!("/assets/{name} "))
                || create.contains(&format!("/assets/{name}")),
            "{name}: {create}"
        );
    }
    assert_eq!(create.matches("/assets/").count(), 10, "{create}");
    // SHA256SUMS lists the other nine, in the form sha256sum -c reads, and checks out.
    let sums = fs::read_to_string(tree.assets.join("SHA256SUMS")).expect("SHA256SUMS");
    let mut listed: Vec<&str> = sums
        .lines()
        .map(|line| line.split_once("  ").expect("hash, two spaces, name").1)
        .collect();
    listed.sort_unstable();
    let mut want = asset_names("0.1.0-beta.1");
    want.sort();
    assert_eq!(listed, want);
    let mut check = common::sandboxed("sha256sum", tree.directory.path());
    check
        .current_dir(&tree.assets)
        .args(["--check", "--strict", "SHA256SUMS"]);
    let checked = common::run_other_tool(&mut check);
    assert!(checked.status.success(), "{}", stderr_of(&checked));
    assert!(expected.iter().all(|name| !name.contains('~')));
    assert_eq!(tree.names_in_assets().len(), 10);
}

#[test]
fn a_final_version_is_not_marked_pre_release() {
    let tree = draft_tree();
    tree.fill("0.1.0");
    let (output, calls) = tree.run("v0.1.0", &[vec![]], true);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let writes = writes(&calls);
    assert_eq!(writes.len(), 1, "{calls:?}");
    assert!(writes[0].contains("--draft"));
    assert!(!writes[0].contains("--prerelease"), "{}", writes[0]);
}

#[test]
fn the_notes_header_links_the_docs_states_the_prerequisite_and_warns_only_for_a_pre_release() {
    for (upstream, tag, pre) in [
        ("0.1.0-beta.1", "v0.1.0-beta.1", true),
        ("0.1.0", "v0.1.0", false),
    ] {
        let tree = draft_tree();
        tree.fill(upstream);
        let (output, _) = tree.run(tag, &[vec![]], true);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let notes = tree.notes();
        assert!(
            notes.contains(
                "https://osirison.github.io/proton-drive-sync-engine/distribution/packages/"
            ),
            "{notes}"
        );
        assert!(
            notes.contains("https://proton.me/drive/download"),
            "{notes}"
        );
        assert!(notes.contains("proton-drive auth login"), "{notes}");
        assert!(!notes.contains("<!--"), "a marker was left in: {notes}");
        assert_eq!(
            notes.contains("This is a pre-release"),
            pre,
            "{tag}: {notes}"
        );
        // No install commands are copied here: those are the packages page's.
        for command in ["dnf install", "apt install", "apt-get install", "pacman -"] {
            assert!(!notes.contains(command), "{command}: {notes}");
        }
        // The prerequisite comes first for a final release, and after the warning for a beta.
        assert!(
            notes.trim_start().starts_with(if pre {
                "> **This is a pre-release"
            } else {
                "## Before you install"
            }),
            "{notes}"
        );
    }
}

#[test]
fn a_draft_that_exists_is_replaced_not_duplicated() {
    let tree = draft_tree();
    tree.fill("0.1.0-beta.1");
    let kept = asset_names("0.1.0-beta.1")[0].clone();
    let draft = release_json(
        7,
        "v0.1.0-beta.1",
        true,
        &[&kept, "SHA256SUMS", "stale-from-an-earlier-run.deb"],
    );
    // Another tag's draft and published release are none of this run's business.
    let others = vec![
        release_json(8, "v0.1.0-beta.2", true, &["x"]),
        release_json(9, "v0.0.9", false, &["y"]),
    ];
    let (output, calls) = tree.run("v0.1.0-beta.1", &[vec![draft.clone()], others], true);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let writes = writes(&calls);
    assert_eq!(writes.len(), 2, "{calls:?}");
    assert!(
        writes[0].starts_with("release upload v0.1.0-beta.1 "),
        "{}",
        writes[0]
    );
    assert!(writes[0].contains("--clobber") && writes[0].contains("--repo o/r"));
    assert_eq!(writes[0].matches("/assets/").count(), 10);
    assert_eq!(
        writes[1].as_str(),
        "release delete-asset v0.1.0-beta.1 stale-from-an-earlier-run.deb --repo o/r --yes"
    );
    assert!(!calls.iter().any(|call| call.starts_with("release create")));
}

#[test]
fn a_draft_with_no_stale_asset_deletes_nothing() {
    let tree = draft_tree();
    tree.fill("0.1.0");
    let names = asset_names("0.1.0");
    let mut held: Vec<&str> = names.iter().map(String::as_str).collect();
    held.push("SHA256SUMS");
    let (output, calls) = tree.run(
        "v0.1.0",
        &[vec![release_json(7, "v0.1.0", true, &held)]],
        true,
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let writes = writes(&calls);
    assert_eq!(writes.len(), 1, "{calls:?}");
    assert!(writes[0].starts_with("release upload "));
}

#[test]
fn a_draft_on_a_later_page_of_the_listing_is_found() {
    let tree = draft_tree();
    tree.fill("0.1.0-beta.1");
    let filler: Vec<_> = (100..130)
        .map(|id| release_json(id, "v0.0.1", false, &[]))
        .collect();
    let draft = release_json(7, "v0.1.0-beta.1", true, &["old"]);
    let (output, calls) = tree.run("v0.1.0-beta.1", &[filler, vec![draft]], true);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(calls.iter().any(|call| call.starts_with("release upload ")));
    assert!(
        !calls.iter().any(|call| call.starts_with("release create")),
        "{calls:?}"
    );
}

#[test]
fn two_drafts_for_one_tag_fail_naming_both_and_change_nothing() {
    let tree = draft_tree();
    tree.fill("0.1.0-beta.1");
    let drafts = vec![
        release_json(4242, "v0.1.0-beta.1", true, &[]),
        release_json(4343, "v0.1.0-beta.1", true, &[]),
    ];
    let (output, calls) = tree.run("v0.1.0-beta.1", &[drafts], false);
    let stderr = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("4242") && stderr.contains("4343"),
        "{stderr}"
    );
    assert!(writes(&calls).is_empty(), "{calls:?}");
    assert!(
        !tree.assets.join("SHA256SUMS").exists(),
        "SHA256SUMS written"
    );
}

#[test]
fn a_published_release_fails_the_run_and_changes_nothing() {
    for also_a_draft in [false, true] {
        let tree = draft_tree();
        tree.fill("0.1.0-beta.1");
        let mut page = vec![release_json(5, "v0.1.0-beta.1", false, &["a"])];
        if also_a_draft {
            page.push(release_json(6, "v0.1.0-beta.1", true, &["b"]));
        }
        let (output, calls) = tree.run("v0.1.0-beta.1", &[page], false);
        let stderr = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains("already published"), "{stderr}");
        assert!(writes(&calls).is_empty(), "{calls:?}");
        assert!(
            !tree.assets.join("SHA256SUMS").exists(),
            "SHA256SUMS written"
        );
        assert_eq!(tree.names_in_assets().len(), 9);
    }
}

#[test]
fn a_listing_that_fails_stops_the_run_before_any_write() {
    let tree = draft_tree();
    tree.fill("0.1.0");
    let releases = tree.directory.path().join("empty.json");
    fs::write(&releases, "[]").expect("releases");
    let (output, calls) = tree.run_with("v0.1.0", &releases, true, true);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(writes(&calls).is_empty(), "{calls:?}");
}

#[test]
fn a_bad_tag_stops_the_run_before_gh_is_called() {
    let tree = draft_tree();
    tree.fill("0.1.0");
    for tag in ["0.1.0", "v0.1", "v0.1.0-beta"] {
        let (output, calls) = tree.run(tag, &[vec![]], true);
        assert_eq!(output.status.code(), Some(1), "{tag}");
        assert!(calls.is_empty(), "{tag}: {calls:?}");
    }
}

#[test]
fn assets_that_are_not_the_expected_set_are_refused_before_any_write() {
    // (what to change, what the message must say)
    type Change = fn(&DraftTree);
    let cases: [(&str, Change, &str); 6] = [
        (
            "a tilde in a name",
            |t| {
                fs::write(
                    t.assets
                        .join("proton-drive-sync-engine_0.1.0~beta.1-1_amd64.deb"),
                    "x",
                )
                .unwrap();
            },
            "contains '~'",
        ),
        (
            "a name without the version",
            |t| {
                fs::write(t.assets.join("proton-drive-sync-engine-9.9.9.tar.gz"), "x").unwrap();
            },
            "is not named for version",
        ),
        (
            "an unexpected file",
            |t| {
                fs::write(t.assets.join("notes.txt-0.1.0"), "x").unwrap();
            },
            "unexpected asset",
        ),
        (
            "a missing deb",
            |t| {
                fs::remove_file(t.assets.join("proton-sync-nemo_0.1.0-1_all.deb")).unwrap();
            },
            "expected 1 tarball",
        ),
        (
            "a second tarball",
            |t| {
                fs::write(
                    t.assets.join("proton-drive-sync-engine-0.1.0-extra.tar.gz"),
                    "x",
                )
                .unwrap();
            },
            "is not named for version",
        ),
        (
            "a missing source rpm",
            |t| {
                fs::remove_file(
                    t.assets
                        .join("proton-drive-sync-engine-0.1.0-1.fc43.src.rpm"),
                )
                .unwrap();
            },
            "expected 1 tarball",
        ),
    ];
    for (what, change, message) in cases {
        let tree = draft_tree();
        tree.fill("0.1.0");
        change(&tree);
        let (output, calls) = tree.run("v0.1.0", &[vec![]], false);
        let stderr = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{what}: {stderr}");
        assert!(stderr.contains(message), "{what}: {stderr}");
        assert!(writes(&calls).is_empty(), "{what}: {calls:?}");
    }
}

#[test]
fn a_stale_sha256sums_is_replaced_and_does_not_list_itself() {
    let tree = draft_tree();
    tree.fill("0.1.0");
    fs::write(
        tree.assets.join("SHA256SUMS"),
        "0000  proton-drive-sync-engine-0.1.0.tar.gz\n",
    )
    .unwrap();
    let (output, _) = tree.run("v0.1.0", &[vec![]], true);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let sums = fs::read_to_string(tree.assets.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 9, "{sums}");
    assert!(!sums.contains("SHA256SUMS"));
    assert!(!sums.contains("0000  "));
}

// --- the workflow files -------------------------------------------------------------------------

fn workflow(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".github/workflows")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The text of job `name` of a workflow, up to the next job.
fn job_block(text: &str, name: &str) -> String {
    let header = format!("  {name}:");
    let mut lines = text.lines().skip_while(|line| *line != header.as_str());
    let mut block = vec![lines.next().unwrap_or_else(|| panic!("no job {name}"))];
    for line in lines {
        let is_job_key = line.starts_with("  ")
            && !line.starts_with("   ")
            && !line.trim_start().starts_with('#')
            && line.trim_end().ends_with(':');
        if is_job_key {
            break;
        }
        block.push(line);
    }
    block.join("\n")
}

#[test]
fn the_release_workflow_runs_on_version_tags_and_only_the_draft_job_can_write() {
    let release = workflow("release.yml");
    let on = release.split("\npermissions:").next().expect("on block");
    assert!(on.contains("tags:\n      - \"v*\""), "{on}");
    assert!(
        !on.contains("branches:") && !on.contains("pull_request") && !on.contains("schedule"),
        "{on}"
    );
    // Top-level permissions read-only; exactly one `contents: write`, in the draft job.
    assert!(release.contains("\npermissions:\n  contents: read\n"));
    assert_eq!(release.matches("contents: write").count(), 1);
    assert!(job_block(&release, "draft").contains("contents: write"));
    for call in ["ci", "packages"] {
        let block = job_block(&release, call);
        assert!(block.contains("needs: gate"), "{call}: {block}");
        assert!(
            block.contains(&format!(
                "uses: ./.github/workflows/{}.yml",
                if call == "ci" { "ci" } else { "packages" }
            )),
            "{block}"
        );
        // A caller job grants what the called file asks for, which is contents: read and nothing else.
        assert!(
            block.contains("permissions:\n      contents: read\n"),
            "{block}"
        );
    }
    let draft = job_block(&release, "draft");
    assert!(draft.contains("needs: [gate, ci, packages]"), "{draft}");
    // The four artifacts are downloaded by name.
    for artifact in [
        "source-tarball",
        "rpm-packages",
        "deb-packages",
        "arch-package",
    ] {
        assert!(draft.contains(&format!("name: {artifact}\n")), "{artifact}");
    }
    // The gate fetches what its ancestry check needs, and runs the script.
    let gate = job_block(&release, "gate");
    assert!(gate.contains("fetch-depth: 0"), "{gate}");
    assert!(
        gate.contains("git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main"),
        "{gate}"
    );
    assert!(gate.contains("packaging/release/gate.sh"), "{gate}");
    // The logic lives in the scripts the tests run, not in the workflow.
    assert!(draft.contains("packaging/release/draft.sh"));
    assert!(
        !release.contains("gh release"),
        "gh release is called by draft.sh only"
    );
}

#[test]
fn the_called_workflows_ask_only_for_what_their_caller_grants() {
    for name in ["ci.yml", "packages.yml"] {
        let text = workflow(name);
        assert!(
            text.contains("\nworkflow_call:") || text.contains("\n  workflow_call:"),
            "{name}"
        );
        // One permissions block, at the top, contents: read; none on a job.
        let permissions: Vec<&str> = text
            .lines()
            .filter(|line| line.trim_start().starts_with("permissions:"))
            .collect();
        assert_eq!(
            permissions,
            ["permissions:"],
            "{name} sets permissions below the top level"
        );
        assert!(
            text.contains("\npermissions:\n  contents: read\n"),
            "{name}"
        );
    }
    // packages.yml's concurrency group is per run for everything but a pull request, so a tag
    // run's group never meets the caller's (release.yml's is named after the ref).
    let packages = workflow("packages.yml");
    let group = packages
        .lines()
        .find(|line| line.trim_start().starts_with("group: packages-"))
        .expect("group");
    assert!(group.contains("github.run_id"), "{group}");
    assert!(
        group.contains("github.event_name == 'pull_request'"),
        "{group}"
    );
    assert!(workflow("release.yml").contains("group: release-${{ github.ref }}"));
}

#[test]
fn no_other_workflow_runs_on_a_tag_push() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    for entry in fs::read_dir(&directory).expect("workflows") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        if name == "release.yml" || !name.ends_with(".yml") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("workflow");
        let on = text
            .split("\non:")
            .nth(1)
            .and_then(|rest| rest.split("\npermissions:").next())
            .map(|rest| rest.split("\njobs:").next().expect("on block"))
            .unwrap_or_else(|| panic!("{name}: no on: block"));
        assert!(!on.contains("tags:"), "{name} names tags");
        // A `push:` trigger without a branch or path filter fires for tags too. On the same
        // indentation, `push:` must be followed by `branches:` (or `paths:`).
        if let Some(after) = on.split("\n  push:").nth(1) {
            let filter = after
                .lines()
                .skip(1)
                .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
                .unwrap_or("");
            assert!(
                filter.trim_start().starts_with("branches:")
                    || filter.trim_start().starts_with("paths:"),
                "{name}: push is not filtered to branches: {filter}"
            );
        }
    }
}

// --- Fix round 1 ------------------------------------------------------------------------------

#[test]
fn the_gate_compares_with_the_remote_main_and_not_a_tag_called_origin_main() {
    // `origin/main` is looked up under refs/tags/ before refs/remotes/, so a tag of that name on a
    // branch commit would be compared with itself and pass.
    let gate = gate_tree();
    let main = gate.commit("on main");
    gate.set_main(&main);
    gate.git(&["checkout", "-q", "-b", "branch"]);
    let branch = gate.commit("only on a branch");
    gate.git(&["tag", "origin/main", &branch]);
    let output = gate.gate("v0.1.0", None);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("not an ancestor of refs/remotes/origin/main"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn version_current_refuses_a_cargo_toml_with_no_version() {
    let directory = tempdir().expect("tempdir");
    let (tree, _stubs) = copy_of_the_version_sites(directory.path());
    let manifest = tree.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("Cargo.toml");
    fs::write(&manifest, text.replacen("version = \"0.1.0\"", "", 1)).expect("edit");
    let output = version_script(&tree, directory.path(), None, &["current"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        stderr_of(&output).contains("no version"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn an_empty_release_listing_is_not_read_as_no_release() {
    // The API prints [] when there are none. Nothing at all would have created a second draft.
    let tree = draft_tree();
    tree.fill("0.1.0");
    let releases = tree.directory.path().join("nothing.json");
    fs::write(&releases, "").expect("releases");
    let (output, calls) = tree.run_with("v0.1.0", &releases, true, false);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("came back empty"),
        "{}",
        stderr_of(&output)
    );
    assert!(writes(&calls).is_empty(), "{calls:?}");
}

#[test]
fn notes_with_broken_pre_release_markers_are_refused_for_every_tag() {
    let block = "<!-- pre-release -->\nA warning.\n<!-- /pre-release -->\n";
    let cases = [
        ("the start marker missing", "Intro.\nA warning.\n<!-- /pre-release -->\n## Install\nsteps\n".to_owned()),
        ("the end marker missing", "Intro.\n<!-- pre-release -->\nA warning.\n## Install\nsteps\n".to_owned()),
        ("the end before the start", "<!-- /pre-release -->\nA warning.\n<!-- pre-release -->\n## Install\nsteps\n".to_owned()),
        ("a duplicated pair", format!("{block}## Install\nsteps\n{block}")),
        ("a duplicated start", "<!-- pre-release -->\n<!-- pre-release -->\nA warning.\n<!-- /pre-release -->\n## Install\n".to_owned()),
    ];
    for (what, text) in cases {
        for (upstream, tag) in [("0.1.0", "v0.1.0"), ("0.1.0-beta.1", "v0.1.0-beta.1")] {
            let tree = draft_tree();
            tree.fill(upstream);
            fs::write(tree.tree.join("packaging/release-notes.md"), &text).expect("notes");
            let (output, calls) = tree.run(tag, &[vec![]], true);
            let stderr = stderr_of(&output);
            assert_eq!(output.status.code(), Some(1), "{what} {tag}: {stderr}");
            assert!(stderr.contains("exactly one"), "{what} {tag}: {stderr}");
            assert!(calls.is_empty(), "{what} {tag}: {calls:?}");
        }
    }
}

#[test]
fn notes_that_are_empty_once_the_block_is_dropped_are_refused() {
    let tree = draft_tree();
    tree.fill("0.1.0");
    fs::write(
        tree.tree.join("packaging/release-notes.md"),
        "<!-- pre-release -->\nOnly a warning.\n<!-- /pre-release -->\n",
    )
    .expect("notes");
    let (output, calls) = tree.run("v0.1.0", &[vec![]], true);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("notes are empty"),
        "{}",
        stderr_of(&output)
    );
    assert!(writes(&calls).is_empty(), "{calls:?}");
}

#[test]
fn an_asset_named_for_another_version_is_refused_even_when_it_contains_this_one() {
    // 0.1.0 is a substring of every 0.1.0-beta.N name.
    for (assets_for, tag) in [
        ("0.1.0-beta.1", "v0.1.0"),
        ("0.1.0", "v0.1.0-beta.1"),
        ("0.1.0-beta.10", "v0.1.0-beta.1"),
        ("0.1.0-beta.1", "v0.1.0-beta.10"),
    ] {
        let tree = draft_tree();
        tree.fill(assets_for);
        let (output, calls) = tree.run(tag, &[vec![]], false);
        let stderr = stderr_of(&output);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{assets_for} for {tag}: {stderr}"
        );
        assert!(
            stderr.contains("is not named for version"),
            "{assets_for} for {tag}: {stderr}"
        );
        assert!(writes(&calls).is_empty(), "{calls:?}");
    }
}

#[test]
fn check_assets_accepts_the_expected_set_and_refuses_any_other_without_touching_gh() {
    let tree = draft_tree();
    tree.fill("0.1.0-beta.1");
    let output = tree.check_assets("0.1.0-beta.1");
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("9 assets"));
    // It writes nothing: no SHA256SUMS.
    assert!(!tree.assets.join("SHA256SUMS").exists());
    // The set it would refuse: an extra debuginfo RPM, a wrong version, a bad version argument.
    fs::write(
        tree.assets
            .join("proton-drive-sync-engine-debuginfo-0.1.0-beta.1-1.fc43.x86_64.rpm"),
        "x",
    )
    .expect("extra");
    let output = tree.check_assets("0.1.0-beta.1");
    assert_eq!(output.status.code(), Some(1));
    fs::remove_file(
        tree.assets
            .join("proton-drive-sync-engine-debuginfo-0.1.0-beta.1-1.fc43.x86_64.rpm"),
    )
    .expect("remove");
    assert_eq!(tree.check_assets("0.1.0").status.code(), Some(1));
    assert_eq!(tree.check_assets("0.1").status.code(), Some(1));
}

// --- release.yml: what a `run:` may contain, and its permissions, exactly ---------------------------

/// The text of every `run:` step: a block scalar (`|`, `>`, with any chomping or indent mark), a
/// value on the following lines, or an inline value.
fn run_blocks(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let body = trimmed.strip_prefix("- ").map_or(trimmed, str::trim_start);
        if let Some(rest) = body.strip_prefix("run:") {
            let key_indent = lines[i].len() - body.len();
            let rest = rest.trim();
            if rest.is_empty() || rest.starts_with('|') || rest.starts_with('>') {
                let mut block = String::new();
                i += 1;
                while i < lines.len()
                    && (lines[i].trim().is_empty() || indent(lines[i]) > key_indent)
                {
                    block.push_str(lines[i]);
                    block.push('\n');
                    i += 1;
                }
                blocks.push(block);
                continue;
            }
            blocks.push(rest.to_owned());
        }
        i += 1;
    }
    blocks
}

#[test]
fn run_blocks_reads_every_form_of_a_run_value() {
    let sample = "jobs:\n  a:\n    steps:\n      - run: echo ${{ inline }}\n      - name: x\n        run: |\n          one ${{ pipe }}\n\n          two\n        env:\n          K: ${{ not.a.run }}\n      - name: y\n        run: >-\n          folded ${{ gt }}\n      - run:\n          next ${{ line }}\n      # run: ${{ commented }}\n      - name: z\n        run: plain\n";
    let blocks = run_blocks(sample);
    assert_eq!(blocks.len(), 5, "{blocks:?}");
    assert!(blocks[0].contains("${{ inline }}"));
    assert!(
        blocks[1].contains("${{ pipe }}")
            && blocks[1].contains("two")
            && !blocks[1].contains("not.a.run")
    );
    assert!(blocks[2].contains("${{ gt }}"));
    assert!(blocks[3].contains("${{ line }}"));
    assert_eq!(blocks[4], "plain");
    assert!(!blocks.iter().any(|block| block.contains("commented")));
}

/// Every line of `text` that holds `${{` and is not in a place where an expression is safe, as
/// (line number, line). Fails closed: it does not parse YAML, it asks of each such line whether it
/// is a plain `key: value` line and either
///
/// * its key is `group` directly under `concurrency:` (the only expression-bearing key of
///   `release.yml` outside `env:`; a run's group name is not executed), or `if` (evaluated by the
///   runner, never by a shell); or
/// * it is a direct child of an `env:` mapping, judged by indentation against the nearest
///   less-indented line. `env:` values reach a script as environment variables; they are not
///   pasted into a shell. (`with:` is not allowed: an input such as `script:` of
///   `actions/github-script` is run.)
///
/// A continuation line, a flow-style step, a quoted key, a list item, a comment and a `run:` line
/// all fail, whatever YAML form they take.
fn unsafe_expression_lines(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if !line.contains("${{") {
            continue;
        }
        let trimmed = line.trim_start();
        // A plain key: letters, digits, `_` and `-`, then `: `.
        let key = trimmed.split_once(": ").map(|(key, _)| key).filter(|key| {
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        });
        let parent = lines[..at]
            .iter()
            .rev()
            .find(|earlier| {
                !earlier.trim().is_empty()
                    && !earlier.trim_start().starts_with('#')
                    && indent(earlier) < indent(line)
            })
            .map(|earlier| earlier.trim());
        let safe = match key {
            Some("if") => true,
            Some("group") => parent == Some("concurrency:"),
            Some(_) => parent == Some("env:"),
            None => false,
        };
        if !safe {
            found.push((at + 1, (*line).to_owned()));
        }
    }
    found
}

/// The expression lines `release.yml` holds, trimmed, and how many times each. A new expression
/// needs an edit here AND a pass of `unsafe_expression_lines`.
const RELEASE_PINNED_EXPRESSION_LINES: [(&str, usize); 4] = [
    ("group: release-${{ github.ref }}", 1),
    ("TAG: ${{ github.ref_name }}", 2),
    ("GH_TOKEN: ${{ github.token }}", 1),
    ("COPR_TOKEN_EXPIRES: ${{ vars.COPR_TOKEN_EXPIRES }}", 1),
];

/// What differs between the lines of `text` that hold a `{` and the pinned lines it is given, plus any
/// line holding a backslash. Pinning on `{`, not just `${{`, catches a double-quoted scalar that
/// YAML joins across a line break (`"…$\` then `{{ x }}"`, or one brace per line), where no single
/// line holds `${{`. A backslash anywhere is refused: it is that join, and also the escapes
/// (`\x24\x7b\x7b`, `$…`) that YAML decodes into `${{` with no literal brace in the file.
fn expression_pin_problems(text: &str, pinned_lines: &[(&str, usize)]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: Vec<(String, usize)> = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains('\\') {
            problems.push(format!("line {} holds a backslash: {trimmed}", at + 1));
        }
        if !line.contains('{') {
            continue;
        }
        match seen.iter_mut().find(|(known, _)| known == trimmed) {
            Some((_, count)) => *count += 1,
            None => seen.push((trimmed.to_owned(), 1)),
        }
        let allowed = pinned_lines.iter().any(|(pinned, _)| *pinned == trimmed);
        if !allowed {
            problems.push(format!(
                "line {} is not a pinned expression line: {trimmed}",
                at + 1
            ));
        }
    }
    for &(pinned, count) in pinned_lines {
        let found = seen
            .iter()
            .find(|(known, _)| known == pinned)
            .map_or(0, |(_, found)| *found);
        if found != count {
            problems.push(format!(
                "`{pinned}` appears {found} times, expected {count}"
            ));
        }
    }
    problems
}

#[test]
fn an_expression_is_safe_only_in_the_places_the_release_workflow_uses_them() {
    // The forms that hide an expression in a shell command, which a `run:` reader missed.
    let cases = [
        ("inline", "      - run: gate.sh ${{ github.ref_name }}\n"),
        (
            "continued scalar",
            "        run: gate.sh\n          \"${{ github.ref_name }}\"\n",
        ),
        (
            "multi-line scalar",
            "        run: first line\n          second ${{ github.ref_name }} line\n",
        ),
        (
            "flow style",
            "      - { name: x, run: \"gate.sh ${{ github.ref_name }}\" }\n",
        ),
        (
            "quoted key",
            "        \"run\": gate.sh ${{ github.ref_name }}\n",
        ),
        (
            "block scalar",
            "        run: |\n          gate.sh ${{ github.ref_name }}\n",
        ),
        ("comment", "        # ${{ github.ref_name }}\n"),
        (
            "env value continued",
            "        env:\n          TAG: x\n            ${{ github.ref_name }}\n",
        ),
        (
            "group outside concurrency",
            "jobs:\n  group: ${{ github.ref }}\n",
        ),
        (
            "with input (github-script runs `script`)",
            "        with:\n          script: ${{ github.ref_name }}\n",
        ),
    ];
    for (what, text) in cases {
        assert!(
            !unsafe_expression_lines(text).is_empty(),
            "{what} was accepted"
        );
    }
    for (what, text) in [
        (
            "env",
            "        env:\n          TAG: ${{ github.ref_name }}\n",
        ),
        ("if", "    if: ${{ github.ref_name == 'x' }}\n"),
        (
            "group",
            "concurrency:\n  group: release-${{ github.ref }}\n",
        ),
        (
            "env after a comment",
            "        env:\n          # note\n\n          TAG: ${{ github.ref_name }}\n",
        ),
    ] {
        assert!(
            unsafe_expression_lines(text).is_empty(),
            "{what} was refused"
        );
    }
}

#[test]
fn the_pinned_expression_lines_are_pinned_exactly() {
    let real = "concurrency:\n  group: release-${{ github.ref }}\nenv:\n  TAG: ${{ github.ref_name }}\n  GH_TOKEN: ${{ github.token }}\n  COPR_TOKEN_EXPIRES: ${{ vars.COPR_TOKEN_EXPIRES }}\nenv:\n  TAG: ${{ github.ref_name }}\n";
    assert!(expression_pin_problems(real, &RELEASE_PINNED_EXPRESSION_LINES).is_empty());
    let cases = [
        // One safe line swapped for another expression, keeping the count at four.
        (
            "swapped for a with input",
            real.replacen(
                "  GH_TOKEN: ${{ github.token }}",
                "  script: ${{ github.ref_name }}",
                1,
            ),
        ),
        // A pinned line more often than the file has it, and less often.
        (
            "one more",
            format!("{real}env:\n  TAG: ${{{{ github.ref_name }}}}\n"),
        ),
        (
            "one fewer",
            real.replacen("env:\n  TAG: ${{ github.ref_name }}\n", "", 1),
        ),
        // A double-quoted scalar YAML joins across a line break: no line holds `${{`.
        (
            "split $ and {{",
            format!("{real}run: \"gate.sh $\\\n  {{{{ github.ref_name }}}}\"\n"),
        ),
        (
            "one brace per line",
            format!("{real}run: \"gate.sh $\\\n  {{\\\n  {{ github.ref_name }}}}\"\n"),
        ),
        (
            "\\u escapes",
            format!("{real}run: \"gate.sh \\u0024\\u007b\\u007b github.ref_name }}}}\"\n"),
        ),
        (
            "\\x escapes",
            format!("{real}run: \"gate.sh \\x24\\x7b\\x7b github.ref_name }}}}\"\n"),
        ),
        ("a flow mapping", format!("{real}- {{ name: x }}\n")),
    ];
    for (what, text) in cases {
        assert!(
            !expression_pin_problems(&text, &RELEASE_PINNED_EXPRESSION_LINES).is_empty(),
            "{what} was accepted"
        );
    }
}

#[test]
fn no_run_step_of_the_release_workflow_holds_an_expression() {
    // `${{ }}` in a `run:` is pasted into the shell before it runs, so an event field there is a
    // script injection. The rule is inverted so it fails closed: every line of the file that holds
    // an expression must be one of the safe places in `unsafe_expression_lines`, whatever YAML form
    // a run step takes.
    let release = workflow("release.yml");
    let unsafe_lines = unsafe_expression_lines(&release);
    assert!(
        unsafe_lines.is_empty(),
        "an expression outside env:, with:, if: and concurrency.group: {unsafe_lines:?}"
    );
    // The lines the real file holds, exactly: the concurrency group and three `env:` values.
    let problems = expression_pin_problems(&release, &RELEASE_PINNED_EXPRESSION_LINES);
    assert!(
        problems.is_empty(),
        "the file's expression lines changed: review each one: {problems:?}"
    );
    // The run-step reader finds the steps (it is also what the test above is cross-checked with).
    let blocks = run_blocks(&release);
    assert!(
        blocks.len() >= 3,
        "found {} run steps: {blocks:?}",
        blocks.len()
    );
    for block in &blocks {
        assert!(
            !block.contains("${{"),
            "an expression in a run step: {block}"
        );
    }
    // The values the scripts read arrive through the environment instead.
    assert!(release.contains("TAG: ${{ github.ref_name }}"));
}

/// The lines of every `permissions:` mapping, in file order.
fn permission_blocks(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut blocks = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if line.trim() == "permissions:" {
            blocks.push(
                lines[at + 1..]
                    .iter()
                    .take_while(|next| indent(next) > indent(line))
                    .map(|next| next.trim().to_owned())
                    .collect(),
            );
        }
    }
    blocks
}

#[test]
fn the_release_workflow_pins_its_permissions_and_its_other_settings_exactly() {
    let release = workflow("release.yml");
    // Top level, `ci`, `packages`: contents read and nothing else. `draft`: contents write only.
    assert_eq!(
        permission_blocks(&release),
        [
            vec!["contents: read"],
            vec!["contents: read"],
            vec!["contents: read"],
            vec!["contents: write"],
        ]
    );
    assert!(release.contains("  cancel-in-progress: false\n"));
    assert!(!release.contains("cancel-in-progress: true"));
    assert!(release.contains("\ndefaults:\n  run:\n    shell: bash\n"));
    let draft = job_block(&release, "draft");
    assert_eq!(
        draft.matches("          path: release-files\n").count(),
        4,
        "{draft}"
    );
    assert!(
        draft.contains("          GH_TOKEN: ${{ github.token }}\n"),
        "{draft}"
    );
    assert!(
        draft.contains("          persist-credentials: false\n"),
        "{draft}"
    );
    // Only the draft job reads the token.
    assert_eq!(release.matches("github.token").count(), 1);
}

#[test]
fn packages_checks_the_release_asset_set_on_every_build() {
    let workflow = workflow("packages.yml");
    let job = job_block(&workflow, "release-assets");
    // It waits for all four build jobs and is skipped with them.
    for needed in ["changes", "tarball", "rpm", "deb", "arch"] {
        assert!(job.contains(needed), "{needed}: {job}");
    }
    assert!(
        job.contains("needs: [changes, tarball, rpm, deb, arch]"),
        "{job}"
    );
    assert!(
        job.contains("if: needs.changes.outputs.build == 'true'"),
        "{job}"
    );
    for artifact in [
        "source-tarball",
        "rpm-packages",
        "deb-packages",
        "arch-package",
    ] {
        assert!(job.contains(&format!("name: {artifact}\n")), "{artifact}");
    }
    // The release's own check, with the version Cargo.toml states.
    assert!(
        job.contains(
            r#"packaging/release/draft.sh --check-assets release-files "$(packaging/version.sh current)""#
        ),
        "{job}"
    );
    // And it gates the merge.
    assert!(
        aggregator_needs()
            .iter()
            .any(|name| name == "release-assets")
    );
}

#[test]
fn the_asset_download_directory_is_not_a_directory_of_the_repository() {
    // The artifacts are downloaded into the checkout. The first version used `assets`, which the
    // repository has (assets/icon.svg), and the asset check found the icon among the packages.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, job) in [("packages.yml", "release-assets"), ("release.yml", "draft")] {
        let block = job_block(&workflow(file), job);
        let paths: Vec<&str> = block
            .lines()
            .filter_map(|line| line.trim().strip_prefix("path: "))
            .collect();
        assert_eq!(paths.len(), 4, "{file} {job}: {paths:?}");
        for path in paths {
            assert!(
                !root.join(path).exists(),
                "{file}: {path} exists in the repository"
            );
        }
    }
}

#[test]
fn every_package_name_is_checked_for_the_version_not_only_the_tarball() {
    // Each package alone is renamed for another version; the tarball stays right.
    for (assets_for, other) in [("0.1.0", "0.1.0-beta.1"), ("0.1.0-beta.1", "0.1.0-beta.10")] {
        for name in asset_names(assets_for).into_iter().skip(1) {
            let tree = draft_tree();
            tree.fill(assets_for);
            fs::rename(
                tree.assets.join(&name),
                tree.assets.join(name.replacen(assets_for, other, 1)),
            )
            .expect("rename");
            let output = tree.check_assets(assets_for);
            let stderr = stderr_of(&output);
            assert_eq!(output.status.code(), Some(1), "{name} as {other}: {stderr}");
            assert!(
                stderr.contains("is not named for version"),
                "{name}: {stderr}"
            );
        }
    }
}

// --- the publish scripts: route, token-expiry, credentials, preflight, copr, aur, aggregate -------
//
// Every script runs in the sandbox with stub `gh`, `copr-cli` and `ssh` first on PATH (ahead of the
// sandbox's refusing ones, so a call a test did not expect fails the test), no tokens, and
// `git` with no system or global config. The AUR remote is a local bare repository.

const PUBLISH_GH_STUB: &str = r#"#!/bin/sh
# Logs every call. Reads: `api repos/o/r/releases/tags/<tag>` (answered from $STUB_RELEASE_JSON) and
# `release download` (copies the --pattern files from $STUB_ASSETS). Anything else is refused.
printf '%s\n' "$*" >> "$STUB_GH_LOG"
case "$1" in
  api)
    if [ -n "$STUB_GH_API_FAILS" ]; then echo "stub gh: api failed" >&2; exit 1; fi
    case "$2" in
      repos/o/r/releases/tags/*) cat "$STUB_RELEASE_JSON" ;;
      *) echo "stub gh: unexpected api call: $*" >&2; exit 98 ;;
    esac
    ;;
  release)
    if [ "$2" != download ]; then echo "stub gh: unexpected: $*" >&2; exit 99; fi
    shift 2
    dir=""; patterns=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --dir) dir="$2"; shift 2 ;;
        --pattern) patterns="$patterns $2"; shift 2 ;;
        *) shift ;;
      esac
    done
    for pattern in $patterns; do /bin/cp "$STUB_ASSETS/$pattern" "$dir/$pattern" || exit 1; done
    ;;
  *) echo "stub gh: unexpected call: $*" >&2; exit 98 ;;
esac
"#;

const COPR_STUB: &str = r#"#!/bin/sh
# Logs every call. `build` and `watch-build` are refused (status 99) unless $STUB_COPR_ALLOW_BUILD is
# set, so a dry run that reaches one fails loudly.
printf '%s\n' "$*" >> "$STUB_COPR_LOG"
if [ "$1" != --config ]; then echo "stub copr-cli: no --config: $*" >&2; exit 98; fi
if [ ! -s "$2" ]; then echo "stub copr-cli: the config file is missing" >&2; exit 6; fi
shift 2
case "$1" in
  whoami)
    if [ -n "$STUB_COPR_WHOAMI_FAILS" ]; then echo "Error: bad config" >&2; exit 6; fi
    echo tester
    ;;
  list-packages)
    if [ -n "$STUB_COPR_LIST_FAILS" ]; then echo "Error: Project $2 does not exist." >&2; exit 1; fi
    file="$STUB_COPR_PACKAGES/$(echo "$2" | tr / _).json"
    if [ -f "$file" ]; then cat "$file"; else echo '[]'; fi
    ;;
  build)
    if [ -z "$STUB_COPR_ALLOW_BUILD" ]; then echo "stub copr-cli: unexpected build: $*" >&2; exit 99; fi
    if [ -n "$STUB_COPR_BUILD_FAILS" ]; then echo "Error: build refused" >&2; exit 1; fi
    if [ -n "$STUB_COPR_BUILD_PRINTS_NO_ID" ]; then echo "Build was added"; exit 0; fi
    n=$(grep -c ' build ' "$STUB_COPR_LOG")
    echo "Build was added to x:"
    echo "  https://copr.example.invalid/build/$((7000 + n))/"
    echo "Created builds: $((7000 + n))"
    ;;
  watch-build)
    if [ -z "$STUB_COPR_ALLOW_BUILD" ]; then echo "stub copr-cli: unexpected watch: $*" >&2; exit 99; fi
    shift
    for id in "$@"; do
      case " $STUB_COPR_FAIL_IDS " in *" $id "*) echo "Build(s) $id failed." >&2; exit 4 ;; esac
    done
    ;;
  *) echo "stub copr-cli: unexpected call: $*" >&2; exit 98 ;;
esac
"#;

const SSH_STUB: &str = r#"#!/bin/sh
# `ssh -T aur@...`: the AUR has no shell, so it greets and exits 1 (success), or ssh itself fails (255).
printf '%s\n' "$*" >> "$STUB_SSH_LOG"
case "$STUB_SSH_MODE" in
  greeting)
    echo "Welcome to AUR, tester! Interactive shell is disabled."
    echo "Try \`ssh help\` for a list of commands."
    exit 1
    ;;
  denied) echo "aur@aur.archlinux.org: Permission denied (publickey)." >&2; exit 255 ;;
  banner) echo "Some other banner"; exit 1 ;;
  *) echo "stub ssh: unexpected call: $*" >&2; exit 98 ;;
esac
"#;

/// Stands in for `docker run`: logs the arguments, refuses a work directory the container user could
/// not enter (what a real container's uid split does), then plays the container by running the
/// mounted script on the work directory with the container stubs first on PATH.
const DOCKER_STUB: &str = r#"#!/bin/bash
printf '%s\n' "$*" >> "$STUB_DOCKER_LOG"
[ "$1" = run ] || { echo "stub docker: unexpected: $*" >&2; exit 98; }
work=""; script=""; uid=""; channels=()
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --rm) shift ;;
    --network) shift 2 ;;
    -e) uid="${2#HOST_UID=}"; shift 2 ;;
    -v)
      case "$2" in
        *:/work) work="${2%:/work}" ;;
        *:/aur-container.sh:ro) script="${2%:/aur-container.sh:ro}" ;;
      esac
      shift 2 ;;
    archlinux:base-devel) shift; shift; shift; channels=("$@"); break ;;
    *) shift ;;
  esac
done
mode=$(stat -c %a "$work")
case "$mode" in
  *[1357]) ;;
  *) echo "bash: line 1: cd: /work: Permission denied (the work directory is mode $mode)" >&2; exit 1 ;;
esac
AUR_WORK_DIR="$work" HOST_UID="$uid" PATH="$STUB_CONTAINER_BIN:$PATH" exec bash "$script" "${channels[@]}"
"#;

/// A tool aur-container.sh calls as root in the container: logged, does nothing.
const CONTAINER_LOG_STUB: &str =
    "#!/bin/sh\nprintf '%s %s\\n' \"$(basename \"$0\")\" \"$*\" >> \"$STUB_CONTAINER_LOG\"\n";

/// `su builder -s /bin/bash -c CMD`: logged; runs CMD as the current user unless $STUB_SU_SKIP, and
/// fails when $STUB_SU_FAILS.
const CONTAINER_SU_STUB: &str = r#"#!/bin/sh
printf 'su %s\n' "$*" >> "$STUB_CONTAINER_LOG"
[ -z "$STUB_SU_FAILS" ] || exit 1
[ -z "$STUB_SU_SKIP" ] || exit 0
shift 3
exec bash "$@"
"#;

/// The secret half of a throwaway Ed25519 key, armored, made once per test binary.
fn test_signing_key() -> &'static str {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let directory = tempdir().expect("tempdir");
        let home = directory.path().join("gpg");
        fs::create_dir(&home).expect("gnupg home");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("mode");
        let gpg = |arguments: &[&str]| {
            let mut command = common::sandboxed("gpg", directory.path());
            command.env("GNUPGHOME", &home).args(arguments);
            common::run_other_tool(&mut command)
        };
        let made = gpg(&[
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-gen-key",
            "preflight test <preflight@example.invalid>",
            "ed25519",
            "sign",
            "never",
        ]);
        assert!(made.status.success(), "{}", stderr_of(&made));
        let exported = gpg(&["--batch", "--armor", "--export-secret-keys"]);
        assert!(exported.status.success(), "{}", stderr_of(&exported));
        let mut command = common::sandboxed("gpgconf", directory.path());
        command
            .env("GNUPGHOME", &home)
            .args(["--kill", "gpg-agent"]);
        common::run_other_tool(&mut command);
        String::from_utf8(exported.stdout).expect("armored key")
    })
}

struct PublishTree {
    directory: tempfile::TempDir,
    stubs: std::path::PathBuf,
}

impl PublishTree {
    fn new() -> Self {
        let directory = tempdir().expect("tempdir");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let tree = directory.path().join("tree/packaging");
        fs::create_dir_all(tree.join("release")).expect("tree");
        fs::copy(root.join("packaging/version.sh"), tree.join("version.sh")).expect("version.sh");
        for entry in fs::read_dir(root.join("packaging/release")).expect("release dir") {
            let path = entry.expect("entry").path();
            fs::copy(
                &path,
                tree.join("release").join(path.file_name().expect("name")),
            )
            .expect("copy a release script");
        }
        let stubs = directory.path().join("publish-stubs");
        fs::create_dir(&stubs).expect("stubs");
        write_executable(&stubs.join("gh"), PUBLISH_GH_STUB);
        write_executable(&stubs.join("copr-cli"), COPR_STUB);
        write_executable(&stubs.join("ssh"), SSH_STUB);
        write_executable(&stubs.join("docker"), DOCKER_STUB);
        // What aur-container.sh calls inside the container: it must not run as root here.
        let container = directory.path().join("container-stubs");
        fs::create_dir(&container).expect("container stubs");
        write_executable(&container.join("useradd"), CONTAINER_LOG_STUB);
        write_executable(&container.join("chown"), CONTAINER_LOG_STUB);
        write_executable(&container.join("su"), CONTAINER_SU_STUB);
        fs::create_dir(directory.path().join("packages")).expect("packages");
        PublishTree { directory, stubs }
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.directory.path().join(name)
    }

    fn script(&self, name: &str) -> std::path::PathBuf {
        self.path("tree/packaging/release").join(name)
    }

    fn command(&self, script: &str) -> std::process::Command {
        let mut command = common::sandboxed("bash", self.directory.path());
        command
            .arg(self.script(script))
            .current_dir(self.directory.path())
            .env("GITHUB_REPOSITORY", "o/r")
            .env("GITHUB_REPOSITORY_OWNER", "o")
            .env("GH_TOKEN", "")
            .env("GITHUB_TOKEN", "")
            .env("GH_CONFIG_DIR", self.path("gh-config"))
            .env("GH_HOST", "gh.invalid")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GITHUB_ENV", self.path("github-env"))
            .env("RUNNER_TEMP", self.path("runner-temp"))
            .env("PUBLISH_RESULTS_DIR", self.path("results"))
            .env("PUBLISH_CREDENTIALS_DIR", self.path("credentials"))
            .env("STUB_GH_LOG", self.path("gh.log"))
            .env("STUB_RELEASE_JSON", self.path("release.json"))
            .env("STUB_ASSETS", self.path("assets"))
            .env("STUB_COPR_LOG", self.path("copr.log"))
            .env("STUB_COPR_PACKAGES", self.path("packages"))
            .env("STUB_SSH_LOG", self.path("ssh.log"))
            .env("STUB_DOCKER_LOG", self.path("docker.log"))
            .env("STUB_CONTAINER_LOG", self.path("container.log"))
            .env("STUB_CONTAINER_BIN", self.path("container-stubs"))
            .env("TODAY", "2027-01-01");
        for name in [
            "DRY_RUN",
            "PUBLISH_DRY_RUN",
            "PUBLISH_CHANNELS",
            "COPR_OWNER",
            "COPR_TOKEN_EXPIRES",
            "COPR_CONFIG_FILE",
            "AUR_SSH_KEY_FILE",
            "APT_SIGNING_KEY_FILE",
            "APT_DEPLOY_KEY_FILE",
            "APT_REMOTE_URL",
            "AUR_GIT_NAME",
            "AUR_GIT_EMAIL",
            "AUR_REMOTE_BASE",
            "AUR_GENERATOR",
            "AUR_CLONE_BASE",
            "GITHUB_PATH",
            // What the apt and verification scripts read: a value on the host must not leak in.
            "APT_WITHDRAWN",
            "APT_GIT_NAME",
            "APT_GIT_EMAIL",
            "APT_FINGERPRINT_FILE",
            "APT_CLIENT_ROOT",
            "PUBLISH_GPG_TMPDIR",
            "PUBLISH_TAG",
            "VERIFY_APT_URI",
            "VERIFY_APT_KEY_URL",
            "VERIFY_APT_WAIT_SECS",
            "VERIFY_APT_POLL_SECS",
            "VERIFY_COPR_WAIT_SECS",
            "VERIFY_COPR_POLL_SECS",
            "VERIFY_EXPECT_VERIFIER",
            "VERIFY_AUR_HOME",
            "VERIFY_AUR_SUDOERS",
        ] {
            command.env_remove(name);
        }
        let mut path = std::ffi::OsString::from(&self.stubs);
        path.push(":");
        path.push(common::path_with_stubs(
            &std::env::var("PATH").expect("PATH"),
        ));
        command.env("PATH", path);
        command
    }

    /// Runs `script` with `arguments` and the extra environment, and fails the test if the run
    /// reached one of the sandbox's refusing stubs.
    fn run(
        &self,
        script: &str,
        arguments: &[&str],
        environment: &[(&str, &str)],
    ) -> std::process::Output {
        let mut command = self.command(script);
        command.args(arguments);
        for (name, value) in environment {
            command.env(name, value);
        }
        let output = common::run_other_tool(&mut command);
        let hits = common::stub_hits(&command);
        assert!(hits.is_empty(), "the test reached a refusing stub: {hits}");
        output
    }

    fn log(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.path(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The release the stub `gh` answers with.
    fn release(
        &self,
        tag: &str,
        draft: serde_json::Value,
        prerelease: serde_json::Value,
        assets: &[&str],
    ) {
        let assets: Vec<serde_json::Value> = assets
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": format!("https://example.invalid/releases/download/{tag}/{name}"),
                })
            })
            .collect();
        fs::write(
            self.path("release.json"),
            serde_json::json!({
                "tag_name": tag,
                "draft": draft,
                "prerelease": prerelease,
                "assets": assets,
            })
            .to_string(),
        )
        .expect("release.json");
    }

    fn results(&self, step: &str) -> Option<String> {
        fs::read_to_string(self.path("results").join(step))
            .ok()
            .map(|text| text.trim().to_owned())
    }
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// --- route.sh (task 6.1) ------------------------------------------------------------------------

#[test]
fn routing_maps_the_flag_to_the_channels_and_fails_closed_on_anything_else() {
    let tree = PublishTree::new();
    let pre = tree.run("route.sh", &["--prerelease", "true"], &[]);
    assert!(pre.status.success());
    assert_eq!(stdout_of(&pre), "beta\n");
    let final_release = tree.run("route.sh", &["--prerelease", "false"], &[]);
    assert_eq!(stdout_of(&final_release), "beta\nstable\n");
    for flag in ["", "True", "1", "null", "yes", " true"] {
        let output = tree.run("route.sh", &["--prerelease", flag], &[]);
        assert_eq!(output.status.code(), Some(1), "flag '{flag}'");
        assert_eq!(stdout_of(&output), "", "flag '{flag}' printed a channel");
    }
}

#[test]
fn a_pre_release_goes_to_beta_only_and_a_final_release_to_beta_and_stable() {
    let tree = PublishTree::new();
    tree.release("v0.1.0-beta.1", false.into(), true.into(), &["x.tar.gz"]);
    let output = tree.run("route.sh", &["v0.1.0-beta.1"], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "beta\n");
    let env = fs::read_to_string(tree.path("github-env")).expect("GITHUB_ENV");
    assert!(env.contains("PUBLISH_TAG=v0.1.0-beta.1\n"), "{env}");
    assert!(env.contains("PUBLISH_DRY_RUN=false\n"), "{env}");
    assert!(env.contains("PUBLISH_CHANNELS=beta\n"), "{env}");

    let tree = PublishTree::new();
    tree.release("v0.1.0", false.into(), false.into(), &["x.tar.gz"]);
    let output = tree.run("route.sh", &["v0.1.0"], &[]);
    assert_eq!(stdout_of(&output), "beta\nstable\n");
    let env = fs::read_to_string(tree.path("github-env")).expect("GITHUB_ENV");
    assert!(env.contains("PUBLISH_CHANNELS=beta stable\n"), "{env}");
    // It only reads the release.
    let calls = tree.log("gh.log");
    assert_eq!(calls, ["api repos/o/r/releases/tags/v0.1.0"], "{calls:?}");
}

#[test]
fn routing_publishes_to_nothing_when_it_cannot_be_sure() {
    type Case = (
        &'static str,
        &'static str,
        serde_json::Value,
        serde_json::Value,
        bool,
    );
    // (what, tag, draft, prerelease, API answers)
    let cases: [Case; 8] = [
        ("a draft", "v0.1.0", true.into(), false.into(), true),
        (
            "prerelease null",
            "v0.1.0",
            false.into(),
            serde_json::Value::Null,
            true,
        ),
        (
            "prerelease a string",
            "v0.1.0",
            false.into(),
            "false".into(),
            true,
        ),
        (
            "a beta marked as final",
            "v0.1.0-beta.1",
            false.into(),
            false.into(),
            true,
        ),
        (
            "a final marked as pre-release",
            "v0.1.0",
            false.into(),
            true.into(),
            true,
        ),
        (
            "a draft flag of null",
            "v0.1.0",
            serde_json::Value::Null,
            false.into(),
            true,
        ),
        (
            "the API failing",
            "v0.1.0",
            false.into(),
            false.into(),
            false,
        ),
        (
            "a tag of the wrong shape",
            "v0.1",
            false.into(),
            false.into(),
            true,
        ),
    ];
    for (what, tag, draft, prerelease, api_works) in cases {
        let tree = PublishTree::new();
        tree.release(tag, draft, prerelease, &["x.tar.gz"]);
        let environment: &[(&str, &str)] = if api_works {
            &[]
        } else {
            &[("STUB_GH_API_FAILS", "1")]
        };
        let output = tree.run("route.sh", &[tag], environment);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert_eq!(stdout_of(&output), "", "{what}: printed a channel");
        assert!(
            !tree.path("github-env").exists(),
            "{what}: the later steps were handed a channel list"
        );
    }
    // The API answering with another release than the one asked for.
    let tree = PublishTree::new();
    tree.release("v0.2.0", false.into(), false.into(), &["x.tar.gz"]);
    let output = tree.run("route.sh", &["v0.1.0"], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout_of(&output), "");
    // A dry_run that is not true or false never means "publish for real".
    let tree = PublishTree::new();
    tree.release("v0.1.0", false.into(), false.into(), &["x.tar.gz"]);
    for bad in ["yes", "TRUE", "1"] {
        let output = tree.run("route.sh", &["v0.1.0"], &[("DRY_RUN", bad)]);
        assert_eq!(output.status.code(), Some(1), "dry_run '{bad}'");
        assert!(
            tree.log("gh.log").is_empty(),
            "dry_run '{bad}' read the API"
        );
    }
}

#[test]
fn a_dry_run_logs_the_targets_and_reads_nothing_but_the_release() {
    let tree = PublishTree::new();
    tree.release("v0.1.0-beta.1", false.into(), true.into(), &["x.tar.gz"]);
    let output = tree.run("route.sh", &["v0.1.0-beta.1"], &[("DRY_RUN", "true")]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let text = stderr_of(&output);
    assert!(text.contains("DRY RUN"), "{text}");
    assert!(text.contains("beta -> COPR project proton-drive-sync-engine-beta, AUR package proton-drive-sync-engine-beta"), "{text}");
    assert!(!text.contains("stable ->"), "{text}");
    assert!(
        fs::read_to_string(tree.path("github-env"))
            .expect("env")
            .contains("PUBLISH_DRY_RUN=true\n")
    );

    let tree = PublishTree::new();
    tree.release("v0.1.0", false.into(), false.into(), &["x.tar.gz"]);
    let output = tree.run("route.sh", &["v0.1.0"], &[("DRY_RUN", "true")]);
    let text = stderr_of(&output);
    assert!(text.contains("beta -> COPR project"), "{text}");
    assert!(
        text.contains(
            "stable -> COPR project proton-drive-sync-engine, AUR package proton-drive-sync-engine"
        ),
        "{text}"
    );
    // Only the one read: a dry run's route step has no way to write.
    assert_eq!(tree.log("gh.log"), ["api repos/o/r/releases/tags/v0.1.0"]);
}

// --- token-expiry.sh ----------------------------------------------------------------------------

#[test]
fn the_token_expiry_check_fails_on_a_passed_empty_or_unparseable_date_and_names_the_variable() {
    let tree = PublishTree::new();
    let check = |value: &str| {
        tree.run(
            "token-expiry.sh",
            &["check"],
            &[("COPR_TOKEN_EXPIRES", value)],
        )
    };
    // TODAY is 2027-01-01: the last day still works, the day after does not.
    for (value, days) in [
        ("2027-06-01", 151),
        ("2027-01-15", 14),
        ("2027-01-02", 1),
        ("2027-01-01", 0),
    ] {
        let output = check(value);
        assert!(output.status.success(), "{value}: {}", stderr_of(&output));
        assert!(
            stdout_of(&output).contains(&format!("({days} days left)")),
            "{value}: {}",
            stdout_of(&output)
        );
    }
    for (what, value) in [
        ("yesterday", "2026-12-31"),
        ("empty", ""),
        ("no zero padding", "2027-4-10"),
        ("an impossible day", "2027-02-30"),
        ("a day-first date", "10/04/2027"),
        ("a time of day", "2027-06-01T00:00:00Z"),
        ("words", "soon"),
    ] {
        let output = check(value);
        assert_eq!(output.status.code(), Some(1), "{what}");
        let text = stderr_of(&output);
        assert!(text.contains("COPR_TOKEN_EXPIRES"), "{what}: {text}");
        assert!(
            text.contains("COPR_CONFIG") && text.contains("copr.fedorainfracloud.org/api"),
            "{what}: {text}"
        );
    }
    // The variable not being set at all is the empty case.
    let output = tree.run("token-expiry.sh", &["check"], &[]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn the_token_expiry_warning_never_fails_and_speaks_within_fourteen_days_or_when_it_cannot_judge() {
    let tree = PublishTree::new();
    let warn = |value: &str| {
        tree.run(
            "token-expiry.sh",
            &["warn"],
            &[("COPR_TOKEN_EXPIRES", value)],
        )
    };
    // Silent while there are more than fourteen days.
    for value in ["2027-06-01", "2027-01-16"] {
        let output = warn(value);
        assert!(output.status.success());
        assert_eq!(stdout_of(&output), "", "{value}");
    }
    // Fourteen days or fewer, today, passed, empty and unparseable: one warning annotation, exit 0.
    for value in [
        "2027-01-15",
        "2027-01-02",
        "2027-01-01",
        "2026-12-01",
        "",
        "2027-4-10",
        "nonsense",
    ] {
        let output = warn(value);
        assert!(output.status.success(), "{value}: {}", stderr_of(&output));
        let text = stdout_of(&output);
        assert_eq!(text.matches("::warning").count(), 1, "{value}: {text}");
        assert!(text.contains("COPR"), "{value}: {text}");
    }
}

// --- credentials.sh -----------------------------------------------------------------------------

#[test]
fn the_credentials_are_written_private_an_empty_secret_gets_no_file_and_remove_deletes_them() {
    let tree = PublishTree::new();
    let output = tree.run(
        "credentials.sh",
        &["write"],
        &[
            ("COPR_CONFIG", "[copr-cli]\nlogin = x"),
            ("AUR_SSH_KEY", "key text without a final newline"),
            ("APT_SIGNING_KEY", ""),
            ("APT_DEPLOY_KEY", "deploy"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let directory = tree.path("credentials");
    let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode(&directory), 0o700);
    for file in ["copr-config", "aur-ssh-key", "apt-deploy-key"] {
        assert_eq!(mode(&directory.join(file)), 0o600, "{file}");
        assert!(
            fs::read_to_string(directory.join(file))
                .expect("file")
                .ends_with('\n')
        );
    }
    assert!(
        !directory.join("apt-signing-key").exists(),
        "an empty secret got a file"
    );
    let env = fs::read_to_string(tree.path("github-env")).expect("env");
    assert!(
        env.contains(&format!(
            "COPR_CONFIG_FILE={}/copr-config\n",
            directory.display()
        )),
        "{env}"
    );
    assert!(env.contains("AUR_SSH_KEY_FILE="), "{env}");
    assert!(env.contains("APT_DEPLOY_KEY_FILE="), "{env}");
    assert!(!env.contains("APT_SIGNING_KEY_FILE"), "{env}");
    // No secret text reaches the logs or the environment file.
    assert!(!env.contains("login = x") && !stderr_of(&output).contains("login = x"));

    let removed = tree.run("credentials.sh", &["remove"], &[]);
    assert!(removed.status.success());
    assert!(!directory.exists());
}

// --- preflight.sh (task 6.2) --------------------------------------------------------------------

impl PublishTree {
    /// Credential files that all work, and the environment that points the preflight at them.
    /// Returns the environment as (name, value) pairs; a test changes one and runs `preflight.sh`.
    fn good_credentials(&self) -> Vec<(String, String)> {
        let directory = self.path("creds");
        fs::create_dir_all(&directory).expect("creds");
        fs::write(
            directory.join("copr"),
            "[copr-cli]\nlogin = x\nusername = tester\n",
        )
        .expect("copr");
        fs::write(directory.join("aur"), "not a real key\n").expect("aur");
        fs::write(directory.join("signing"), test_signing_key()).expect("signing");
        fs::write(directory.join("deploy"), "not a real key\n").expect("deploy");
        // The apt repository: a local bare repository the deploy key "reaches".
        let apt = self.path("apt.git");
        let mut init = common::sandboxed("git", self.directory.path());
        init.args(["init", "--bare", "--quiet"]).arg(&apt);
        let output = common::run_other_tool(&mut init);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let pair = |name: &str, value: String| (name.to_owned(), value);
        vec![
            pair(
                "COPR_CONFIG_FILE",
                directory.join("copr").display().to_string(),
            ),
            pair(
                "AUR_SSH_KEY_FILE",
                directory.join("aur").display().to_string(),
            ),
            pair(
                "APT_SIGNING_KEY_FILE",
                directory.join("signing").display().to_string(),
            ),
            pair(
                "APT_DEPLOY_KEY_FILE",
                directory.join("deploy").display().to_string(),
            ),
            pair("APT_REMOTE_URL", apt.display().to_string()),
            pair("COPR_TOKEN_EXPIRES", "2027-06-01".to_owned()),
            pair("STUB_SSH_MODE", "greeting".to_owned()),
        ]
    }

    fn preflight(&self, environment: &[(String, String)]) -> std::process::Output {
        let pairs: Vec<(&str, &str)> = environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        self.run("preflight.sh", &[], &pairs)
    }
}

fn with(environment: &[(String, String)], name: &str, value: &str) -> Vec<(String, String)> {
    let mut changed: Vec<(String, String)> = environment
        .iter()
        .filter(|(known, _)| known != name)
        .cloned()
        .collect();
    changed.push((name.to_owned(), value.to_owned()));
    changed
}

#[test]
fn a_preflight_with_four_working_credentials_passes_and_asks_the_aur_with_the_pinned_host() {
    let tree = PublishTree::new();
    let environment = tree.good_credentials();
    let output = tree.preflight(&environment);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(tree.results("preflight").as_deref(), Some("ok"));
    // copr-cli is given the config file, and the ssh call is the AUR greeting probe with the key,
    // host-key checking on and the pinned file.
    let copr = tree.log("copr.log");
    assert!(
        copr.iter().any(|call| call.ends_with("copr whoami")),
        "{copr:?}"
    );
    let ssh = tree.log("ssh.log");
    assert_eq!(ssh.len(), 1, "{ssh:?}");
    for needle in [
        "-T aur@aur.archlinux.org",
        "StrictHostKeyChecking=yes",
        "IdentitiesOnly=yes",
        "BatchMode=yes",
        "packaging/release/known_hosts",
    ] {
        assert!(ssh[0].contains(needle), "{needle} missing from {}", ssh[0]);
    }
    // `ssh -T` to the AUR exits 1 on success; that is not a failure.
    assert!(stderr_of(&output).contains("Welcome to AUR"));
}

#[test]
fn each_failing_credential_is_named_with_its_renewal_step_and_the_others_are_still_tried() {
    let tree = PublishTree::new();
    let good = tree.good_credentials();
    // (what, changed environment, the credential it must name, a word of its renewal step)
    type Case = (
        &'static str,
        Vec<(String, String)>,
        &'static str,
        &'static str,
    );
    let cases: Vec<Case> = vec![
        (
            "a COPR config that does not parse",
            with(&good, "STUB_COPR_WHOAMI_FAILS", "1"),
            "COPR_CONFIG",
            "copr.fedorainfracloud.org/api",
        ),
        (
            "a COPR token past its day",
            with(&good, "COPR_TOKEN_EXPIRES", "2026-12-31"),
            "COPR_TOKEN_EXPIRES",
            "COPR_CONFIG",
        ),
        (
            "no COPR_TOKEN_EXPIRES at all",
            with(&good, "COPR_TOKEN_EXPIRES", ""),
            "COPR_TOKEN_EXPIRES",
            "YYYY-MM-DD",
        ),
        (
            "an AUR key the server refuses",
            with(&good, "STUB_SSH_MODE", "denied"),
            "AUR_SSH_KEY",
            "aur.archlinux.org/account",
        ),
        (
            "an AUR answer that is not the greeting",
            with(&good, "STUB_SSH_MODE", "banner"),
            "AUR_SSH_KEY",
            "SSH Public Key",
        ),
        (
            "a signing key that is not a key",
            with(
                &good,
                "APT_SIGNING_KEY_FILE",
                &good_path(&good, "AUR_SSH_KEY_FILE"),
            ),
            "APT_SIGNING_KEY",
            "docs/releasing.md",
        ),
        (
            "a deploy key whose repository cannot be reached",
            with(&good, "APT_REMOTE_URL", "/nonexistent/apt.git"),
            "APT_DEPLOY_KEY",
            "Deploy keys",
        ),
    ];
    for (what, environment, credential, renewal) in cases {
        let output = tree.preflight(&environment);
        assert_eq!(output.status.code(), Some(1), "{what}");
        let text = stderr_of(&output);
        let failed_lines: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("FAILED: "))
            .collect();
        assert!(
            failed_lines.iter().any(|line| line.contains(credential)),
            "{what}: {text}"
        );
        assert!(
            text.contains(renewal),
            "{what}: no renewal step '{renewal}' in {text}"
        );
        assert!(text.contains("1 credential(s) failed"), "{what}: {text}");
        assert_eq!(
            tree.results("preflight").as_deref(),
            Some("failed"),
            "{what}"
        );
    }
    // The two AUR failures are told apart: ssh's own failure (255) and an answer that is no greeting.
    let denied = tree.preflight(&with(&good, "STUB_SSH_MODE", "denied"));
    assert!(stderr_of(&denied).contains("could not connect or authenticate"));
    let banner = tree.preflight(&with(&good, "STUB_SSH_MODE", "banner"));
    assert!(stderr_of(&banner).contains("did not greet us"));
    // A missing secret (no file at all) is a failure of that credential too.
    for (variable, credential) in [
        ("COPR_CONFIG_FILE", "COPR_CONFIG"),
        ("AUR_SSH_KEY_FILE", "AUR_SSH_KEY"),
        ("APT_SIGNING_KEY_FILE", "APT_SIGNING_KEY"),
        ("APT_DEPLOY_KEY_FILE", "APT_DEPLOY_KEY"),
    ] {
        let output = tree.preflight(&with(&good, variable, ""));
        assert_eq!(output.status.code(), Some(1), "{variable}");
        assert!(
            stderr_of(&output).contains(&format!(
                "FAILED: {credential}: the secret is empty or not set"
            )),
            "{variable}: {}",
            stderr_of(&output)
        );
    }
}

fn good_path(environment: &[(String, String)], name: &str) -> String {
    environment
        .iter()
        .find(|(known, _)| known == name)
        .map(|(_, value)| value.clone())
        .expect("a path")
}

#[test]
fn a_preflight_lists_every_failing_credential_not_just_the_first() {
    let tree = PublishTree::new();
    let mut environment = tree.good_credentials();
    environment = with(&environment, "STUB_COPR_WHOAMI_FAILS", "1");
    environment = with(&environment, "STUB_SSH_MODE", "denied");
    environment = with(&environment, "APT_REMOTE_URL", "/nonexistent/apt.git");
    let output = tree.preflight(&environment);
    assert_eq!(output.status.code(), Some(1));
    let text = stderr_of(&output);
    assert!(
        text.contains("3 credential(s) failed; no channel step will run"),
        "{text}"
    );
    for credential in ["COPR_CONFIG", "AUR_SSH_KEY", "APT_DEPLOY_KEY"] {
        assert!(
            text.contains(&format!("- {credential}")),
            "{credential}: {text}"
        );
    }
    // The signing key passed and is not blamed.
    assert!(!text.contains("- APT_SIGNING_KEY"), "{text}");
}

// --- copr.sh (task 6.5) -------------------------------------------------------------------------

const SRPM: &str = "proton-drive-sync-engine-0.1.0-beta.1-1.fc43.src.rpm";

impl PublishTree {
    fn copr_setup(&self, tag: &str, srpms: &[&str]) -> Vec<(&'static str, String)> {
        let prerelease = tag.contains('-');
        self.release(tag, false.into(), prerelease.into(), srpms);
        fs::write(self.path("copr-config"), "[copr-cli]\nusername = tester\n").expect("config");
        vec![
            ("COPR_OWNER", "ns".to_owned()),
            (
                "COPR_CONFIG_FILE",
                self.path("copr-config").display().to_string(),
            ),
            ("PUBLISH_DRY_RUN", "false".to_owned()),
            ("STUB_COPR_ALLOW_BUILD", "1".to_owned()),
        ]
    }

    fn packages_json(&self, project: &str, builds: &[(u64, &str, &str)]) {
        let builds: Vec<serde_json::Value> = builds
            .iter()
            .map(|(id, state, version)| {
                serde_json::json!({ "id": id, "state": state, "source_package": { "name": "proton-drive-sync-engine", "version": version } })
            })
            .collect();
        fs::write(
            self.path("packages").join(format!("{}.json", project.replace('/', "_"))),
            serde_json::json!([
                { "name": "some-other-package", "builds": [ { "id": 1, "state": "succeeded", "source_package": { "version": "0.1.0~beta.1-1" } } ] },
                { "name": "proton-drive-sync-engine", "builds": builds, "latest_build": null, "latest_succeeded_build": null }
            ])
            .to_string(),
        )
        .expect("packages");
    }

    fn copr(
        &self,
        tag: &str,
        channels: &str,
        extra: &[(&str, &str)],
        setup: &[(&'static str, String)],
    ) -> std::process::Output {
        let mut environment: Vec<(&str, &str)> =
            setup.iter().map(|(n, v)| (*n, v.as_str())).collect();
        environment.push(("PUBLISH_CHANNELS", channels));
        environment.extend_from_slice(extra);
        self.run("copr.sh", &[tag], &environment)
    }

    /// The calls of the copr-cli log that start with `verb` (after the --config argument).
    fn copr_calls(&self, verb: &str) -> Vec<String> {
        self.log("copr.log")
            .into_iter()
            .filter_map(|line| {
                let rest = line.split_once(' ')?.1.split_once(' ')?.1.to_owned();
                rest.starts_with(verb).then_some(rest)
            })
            .collect()
    }
}

#[test]
fn copr_submits_every_routed_project_before_it_waits_and_waits_on_the_returned_ids() {
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0", &["proton-drive-sync-engine-0.1.0-1.fc43.src.rpm"]);
    let output = tree.copr("v0.1.0", "beta stable", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let url = "https://example.invalid/releases/download/v0.1.0/proton-drive-sync-engine-0.1.0-1.fc43.src.rpm";
    assert_eq!(
        tree.copr_calls("build"),
        [
            format!("build --nowait ns/proton-drive-sync-engine-beta {url}"),
            format!("build --nowait ns/proton-drive-sync-engine {url}"),
        ]
    );
    // Both submitted (no -r: every chroot of the project) before the first wait.
    let all: Vec<String> = tree.log("copr.log");
    let first_watch = all
        .iter()
        .position(|line| line.contains(" watch-build "))
        .expect("a watch");
    let last_build = all
        .iter()
        .rposition(|line| line.contains(" build "))
        .expect("a build");
    assert!(last_build < first_watch, "{all:?}");
    assert!(
        all.iter()
            .all(|line| !line.contains(" -r ") && !line.contains("--chroot")),
        "{all:?}"
    );
    let mut watched = tree.copr_calls("watch-build");
    watched.sort();
    assert_eq!(watched, ["watch-build 7001", "watch-build 7002"]);
    assert_eq!(tree.results("copr").as_deref(), Some("ok"));
}

#[test]
fn copr_builds_only_the_beta_project_for_a_pre_release() {
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let builds = tree.copr_calls("build");
    assert_eq!(builds.len(), 1, "{builds:?}");
    assert!(
        builds[0].contains("ns/proton-drive-sync-engine-beta "),
        "{builds:?}"
    );
    assert!(
        tree.log("copr.log")
            .iter()
            .all(|line| !line.contains("ns/proton-drive-sync-engine "))
    );
}

#[test]
fn copr_skips_a_project_that_already_has_a_succeeded_build_of_this_version_and_only_this_version() {
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    // COPR's version carries the release (and a dist tag); only the part before the last dash counts.
    tree.packages_json(
        "ns/proton-drive-sync-engine-beta",
        &[
            (5, "failed", "0.1.0~beta.1-1.fc43"),
            (4, "succeeded", "0.1.0~beta.1-1.fc43"),
        ],
    );
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(tree.copr_calls("build").is_empty() && tree.copr_calls("watch-build").is_empty());
    assert!(stderr_of(&output).contains("already has a succeeded build of 0.1.0~beta.1"));
    assert_eq!(tree.results("copr").as_deref(), Some("ok"));

    // A succeeded build of another version (the final, the next beta) is not this version.
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    tree.packages_json(
        "ns/proton-drive-sync-engine-beta",
        &[
            (4, "succeeded", "0.1.0-1.fc43"),
            (3, "succeeded", "0.1.0~beta.10-1"),
            (2, "succeeded", "0.1.0~beta.1.1-1"),
        ],
    );
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(tree.copr_calls("build").len(), 1);
}

#[test]
fn copr_rebuilds_after_a_failed_build_and_waits_for_one_that_is_still_running() {
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    tree.packages_json(
        "ns/proton-drive-sync-engine-beta",
        &[
            (9, "failed", "0.1.0~beta.1-1"),
            (8, "canceled", "0.1.0~beta.1-1"),
        ],
    );
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(tree.copr_calls("build").len(), 1);

    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    tree.packages_json(
        "ns/proton-drive-sync-engine-beta",
        &[(9, "running", "0.1.0~beta.1-1.fc43")],
    );
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        tree.copr_calls("build").is_empty(),
        "a second build was submitted"
    );
    assert_eq!(tree.copr_calls("watch-build"), ["watch-build 9"]);
}

#[test]
fn copr_fails_the_step_naming_the_project_when_a_build_fails_and_still_waits_for_the_other() {
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0", &["proton-drive-sync-engine-0.1.0-1.fc43.src.rpm"]);
    // The beta project's build (7001) fails; the stable one (7002) is still watched.
    let output = tree.copr(
        "v0.1.0",
        "beta stable",
        &[("STUB_COPR_FAIL_IDS", "7001")],
        &setup,
    );
    assert_eq!(output.status.code(), Some(1));
    let text = stderr_of(&output);
    assert!(
        text.contains("COPR failed for: ns/proton-drive-sync-engine-beta"),
        "{text}"
    );
    assert!(
        !text.contains("failed for: ns/proton-drive-sync-engine-beta ns/proton-drive-sync-engine"),
        "{text}"
    );
    assert_eq!(tree.copr_calls("watch-build").len(), 2);
    assert_eq!(tree.results("copr").as_deref(), Some("failed"));
}

#[test]
fn copr_never_reads_a_failed_or_odd_answer_as_no_builds() {
    // (what, extra environment, a file to put in place of the project's answer)
    for (what, extra, body) in [
        (
            "a listing that fails",
            vec![("STUB_COPR_LIST_FAILS", "1")],
            None,
        ),
        (
            "a listing that is not a list",
            vec![],
            Some("{\"error\": \"x\"}"),
        ),
        ("a listing that is not JSON", vec![], Some("<html>")),
        (
            "a build that prints no id",
            vec![("STUB_COPR_BUILD_PRINTS_NO_ID", "1")],
            None,
        ),
        (
            "a build that is refused",
            vec![("STUB_COPR_BUILD_FAILS", "1")],
            None,
        ),
    ] {
        let tree = PublishTree::new();
        let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
        if let Some(body) = body {
            fs::write(
                tree.path("packages/ns_proton-drive-sync-engine-beta.json"),
                body,
            )
            .expect("body");
        }
        let output = tree.copr("v0.1.0-beta.1", "beta", &extra, &setup);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(
            stderr_of(&output).contains("COPR failed for: ns/proton-drive-sync-engine-beta"),
            "{what}"
        );
        assert!(
            tree.copr_calls("watch-build").is_empty(),
            "{what}: waited on nothing"
        );
        assert_eq!(tree.results("copr").as_deref(), Some("failed"), "{what}");
    }
}

#[test]
fn copr_needs_one_source_rpm_for_the_version_and_a_namespace() {
    for (what, assets) in [
        ("none", vec![]),
        (
            "two",
            vec![SRPM, "proton-drive-sync-engine-0.1.0-beta.1-2.fc44.src.rpm"],
        ),
        (
            "another version's",
            vec!["proton-drive-sync-engine-0.1.0-1.fc43.src.rpm"],
        ),
    ] {
        let tree = PublishTree::new();
        let setup = tree.copr_setup("v0.1.0-beta.1", &assets);
        let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
        assert_eq!(output.status.code(), Some(1), "{what}");
        assert!(
            stderr_of(&output).contains("exactly one source RPM"),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(tree.copr_calls("build").is_empty(), "{what}");
    }
    // The namespace comes from the workflow's COPR_OWNER and has no default here.
    let tree = PublishTree::new();
    let mut setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    setup.retain(|(name, _)| *name != "COPR_OWNER");
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr_of(&output).contains("COPR_OWNER is not set"));
    assert!(tree.log("copr.log").is_empty());
    // A channel list route.sh would never write is refused.
    let tree = PublishTree::new();
    let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    for channels in ["", "stable", "beta nightly"] {
        let output = tree.copr("v0.1.0-beta.1", channels, &[], &setup);
        assert_eq!(output.status.code(), Some(1), "'{channels}'");
        assert!(tree.copr_calls("build").is_empty());
    }
}

#[test]
fn a_copr_dry_run_reads_but_never_builds_or_waits() {
    let tree = PublishTree::new();
    let mut setup = tree.copr_setup("v0.1.0", &["proton-drive-sync-engine-0.1.0-1.fc43.src.rpm"]);
    setup.retain(|(name, _)| *name != "PUBLISH_DRY_RUN" && *name != "STUB_COPR_ALLOW_BUILD");
    setup.push(("PUBLISH_DRY_RUN", "true".to_owned()));
    let output = tree.copr("v0.1.0", "beta stable", &[], &setup);
    // STUB_COPR_ALLOW_BUILD is unset, so the stub would refuse (99) any build or watch.
    assert!(output.status.success(), "{}", stderr_of(&output));
    let log = tree.log("copr.log");
    assert!(
        log.iter().all(|line| line.contains(" list-packages ")),
        "{log:?}"
    );
    let text = stderr_of(&output);
    assert!(
        text.contains(
            "DRY RUN: would run 'copr-cli build --nowait ns/proton-drive-sync-engine-beta https://"
        ),
        "{text}"
    );
    assert!(
        text.contains("would run 'copr-cli build --nowait ns/proton-drive-sync-engine https://"),
        "{text}"
    );
    assert_eq!(tree.results("copr").as_deref(), Some("ok"));
}

// --- aur-publish.sh (task 6.4), against local bare repositories standing in for the AUR ----------

const MAKEPKG_STUB: &str = "#!/bin/sh\n\
[ \"$1\" = --printsrcinfo ] || exit 1\n\
name=$(sed -n 's/^pkgname=//p' PKGBUILD | head -n1)\n\
echo \"pkgbase = $name\"\n\
grep '^sha256sums' PKGBUILD\n";

impl PublishTree {
    fn git(&self, repository: &Path, arguments: &[&str]) -> String {
        let mut command = common::sandboxed("git", self.directory.path());
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-C")
            .arg(repository)
            .args(arguments);
        let output = common::run_other_tool(&mut command);
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            stderr_of(&output)
        );
        stdout_of(&output).trim().to_owned()
    }

    /// An empty bare repository for each package, as the AUR gives a package that does not exist
    /// yet when it is cloned. Its HEAD is not master, which is all the AUR takes.
    fn aur_remote(&self, packages: &[&str]) {
        fs::create_dir_all(self.path("aur")).expect("aur");
        for package in packages {
            let repository = self.path("aur").join(format!("{package}.git"));
            let mut command = common::sandboxed("git", self.directory.path());
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .args(["init", "--bare", "--quiet", "--initial-branch=trunk"])
                .arg(&repository);
            let output = common::run_other_tool(&mut command);
            assert!(output.status.success(), "{}", stderr_of(&output));
        }
    }

    /// The release's tarball and SHA256SUMS in `assets/`. `variant` changes the tarball's bytes and
    /// so its checksum, which is what the PKGBUILD carries.
    fn aur_assets(&self, variant: &str) -> String {
        self.aur_assets_with(variant, "")
    }

    /// As [`Self::aur_assets`], but the `aur.sh` in the tarball runs the real one and then `hook`
    /// (shell, with `$tree` the channel's output directory): a hostile script in a tagged tarball.
    fn aur_assets_with(&self, variant: &str, hook: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let pkgbuild = fs::read_to_string(root.join("packaging/arch/PKGBUILD")).expect("PKGBUILD");
        let upstream = pkgbuild
            .lines()
            .find_map(|line| line.strip_prefix("_upstream_ver="))
            .expect("_upstream_ver")
            .to_owned();
        let stage = self.path("stage");
        let _ = fs::remove_dir_all(&stage);
        let top = stage.join(format!("proton-drive-sync-engine-{upstream}"));
        fs::create_dir_all(top.join("packaging/arch")).expect("stage");
        for file in ["aur.sh", "PKGBUILD", "proton-drive-sync-engine.install"] {
            fs::copy(
                root.join("packaging/arch").join(file),
                top.join("packaging/arch").join(file),
            )
            .expect(file);
        }
        fs::write(top.join("variant"), variant).expect("variant");
        if !hook.is_empty() {
            let arch = top.join("packaging/arch");
            fs::rename(arch.join("aur.sh"), arch.join("aur-real.sh")).expect("rename");
            write_executable(
                &arch.join("aur.sh"),
                &format!(
                    "#!/usr/bin/env bash\nset -e\nbash \"$(dirname \"$0\")/aur-real.sh\" \"$@\"\ntree=\"$3\"\n{hook}\n"
                ),
            );
        }
        fs::create_dir_all(self.path("assets")).expect("assets");
        let tarball = format!("proton-drive-sync-engine-{upstream}.tar.gz");
        let mut tar = common::sandboxed("tar", self.directory.path());
        tar.arg("-czf")
            .arg(self.path("assets").join(&tarball))
            .arg("-C")
            .arg(&stage)
            .arg(format!("proton-drive-sync-engine-{upstream}"));
        let output = common::run_other_tool(&mut tar);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let sum = sha256_of(&self.path("assets").join(&tarball));
        fs::write(
            self.path("assets/SHA256SUMS"),
            format!("{sum}  {tarball}\n"),
        )
        .expect("sums");
        write_executable(&self.stubs.join("makepkg"), MAKEPKG_STUB);
        upstream
    }

    fn aur(
        &self,
        upstream: &str,
        channels: &str,
        dry_run: bool,
        extra: &[(&str, &str)],
    ) -> std::process::Output {
        let key = self.path("aur-key");
        fs::write(&key, "not a real key\n").expect("key");
        let base = format!("file://{}", self.path("aur").display());
        let tag = format!("v{upstream}");
        let mut environment: Vec<(&str, &str)> = vec![
            ("PUBLISH_CHANNELS", channels),
            ("PUBLISH_DRY_RUN", if dry_run { "true" } else { "false" }),
            ("AUR_GENERATOR", "host"),
            ("AUR_REMOTE_BASE", &base),
            ("AUR_GIT_NAME", "Release Bot"),
            ("AUR_GIT_EMAIL", "bot@example.invalid"),
        ];
        let key_text = key.display().to_string();
        environment.push(("AUR_SSH_KEY_FILE", &key_text));
        environment.extend_from_slice(extra);
        self.run("aur-publish.sh", &[&tag], &environment)
    }

    fn commits(&self, package: &str) -> usize {
        let repository = self.path("aur").join(format!("{package}.git"));
        let refs = self.git(&repository, &["for-each-ref", "--format=%(refname)"]);
        if refs.is_empty() {
            return 0;
        }
        self.git(&repository, &["rev-list", "--count", "refs/heads/master"])
            .parse()
            .expect("a count")
    }
}

#[test]
fn the_first_push_creates_the_package_a_rerun_makes_no_commit_and_a_changed_tree_makes_one() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta", "proton-drive-sync-engine"]);
    let upstream = tree.aur_assets("one");
    tree.release(&format!("v{upstream}"), false.into(), false.into(), &["x"]);

    let first = tree.aur(&upstream, "beta stable", false, &[]);
    assert!(first.status.success(), "{}", stderr_of(&first));
    for package in ["proton-drive-sync-engine-beta", "proton-drive-sync-engine"] {
        let repository = tree.path("aur").join(format!("{package}.git"));
        assert_eq!(tree.commits(package), 1, "{package}");
        // Exactly the three files, on master, whatever the bare repository's own HEAD said.
        let files = tree.git(
            &repository,
            &["ls-tree", "-r", "--name-only", "refs/heads/master"],
        );
        assert_eq!(
            files, ".SRCINFO\nPKGBUILD\nproton-drive-sync-engine.install",
            "{package}"
        );
        assert_eq!(
            tree.git(
                &repository,
                &["log", "-1", "--format=%an <%ae>%n%s", "refs/heads/master"]
            ),
            format!("Release Bot <bot@example.invalid>\nUpdate to {upstream}")
        );
        let srcinfo = tree.git(&repository, &["show", "refs/heads/master:.SRCINFO"]);
        assert!(
            srcinfo.starts_with(&format!("pkgbase = {package}\n")),
            "{srcinfo}"
        );
        assert!(
            tree.git(&repository, &["for-each-ref", "--format=%(refname)"]) == "refs/heads/master",
            "something other than master was pushed"
        );
    }
    // The beta tree carries its own name; the stable tree does not.
    let beta = tree.git(
        &tree.path("aur/proton-drive-sync-engine-beta.git"),
        &["show", "refs/heads/master:PKGBUILD"],
    );
    assert!(
        beta.contains("pkgname=proton-drive-sync-engine-beta\n")
            && beta.contains("conflicts=(proton-drive-sync-engine)"),
        "{beta}"
    );

    // The same release again: nothing differs, nothing is committed.
    let again = tree.aur(&upstream, "beta stable", false, &[]);
    assert!(again.status.success(), "{}", stderr_of(&again));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 1);
    assert_eq!(tree.commits("proton-drive-sync-engine"), 1);
    assert_eq!(
        stderr_of(&again)
            .matches("equals the AUR head: no commit")
            .count(),
        2,
        "{}",
        stderr_of(&again)
    );

    // A different tarball changes the checksum in the PKGBUILD: exactly one new commit each.
    tree.aur_assets("two");
    let changed = tree.aur(&upstream, "beta stable", false, &[]);
    assert!(changed.status.success(), "{}", stderr_of(&changed));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 2);
    assert_eq!(tree.commits("proton-drive-sync-engine"), 2);
    assert_eq!(tree.results("aur").as_deref(), Some("ok"));
}

#[test]
fn a_pre_release_pushes_only_the_beta_package() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta", "proton-drive-sync-engine"]);
    let upstream = tree.aur_assets("one");
    let output = tree.aur(&upstream, "beta", false, &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 1);
    assert_eq!(tree.commits("proton-drive-sync-engine"), 0);
}

#[test]
fn an_aur_dry_run_builds_the_tree_and_clones_but_pushes_and_commits_nothing() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta"]);
    let upstream = tree.aur_assets("one");
    let output = tree.aur(&upstream, "beta", true, &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("DRY RUN: proton-drive-sync-engine-beta differs from the AUR head: would commit and push to master"), "{}", stderr_of(&output));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 0);
    assert_eq!(tree.results("aur").as_deref(), Some("ok"));
}

#[test]
fn a_channel_whose_package_cannot_be_cloned_fails_the_step_naming_it_and_the_other_still_pushes() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine"]);
    let upstream = tree.aur_assets("one");
    let output = tree.aur(&upstream, "beta stable", false, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("the AUR step failed for: beta"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(
        tree.commits("proton-drive-sync-engine"),
        1,
        "the stable channel did not run"
    );
    assert_eq!(tree.results("aur").as_deref(), Some("failed"));
}

#[test]
fn the_aur_step_refuses_what_it_cannot_make_a_correct_tree_from_and_pushes_nothing() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta", "proton-drive-sync-engine"]);
    let upstream = tree.aur_assets("one");
    // 1. The release's SHA256SUMS does not match its tarball.
    fs::write(
        tree.path("assets/SHA256SUMS"),
        format!(
            "{}  proton-drive-sync-engine-{upstream}.tar.gz\n",
            "0".repeat(64)
        ),
    )
    .expect("sums");
    let output = tree.aur(&upstream, "beta", false, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("SHA256SUMS lists '"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 0);
    // 2. No commit identity.
    tree.aur_assets("one");
    let output = tree.aur(&upstream, "beta", false, &[("AUR_GIT_EMAIL", "")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("AUR_GIT_NAME and AUR_GIT_EMAIL"),
        "{}",
        stderr_of(&output)
    );
    // 3. A dry-run flag that is missing.
    let output = tree.aur(&upstream, "beta", false, &[("PUBLISH_DRY_RUN", "")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("PUBLISH_DRY_RUN"),
        "{}",
        stderr_of(&output)
    );
    // 4. A channel list without beta.
    let output = tree.aur(&upstream, "stable", false, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 0);
    assert_eq!(tree.commits("proton-drive-sync-engine"), 0);
    // 5. A release that lacks the tarball.
    fs::remove_file(tree.path(&format!(
        "assets/proton-drive-sync-engine-{upstream}.tar.gz"
    )))
    .expect("remove");
    let output = tree.aur(&upstream, "beta", false, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 0);
}

// --- aggregate.sh (the final step) --------------------------------------------------------------

#[test]
fn the_final_step_fails_naming_every_step_that_did_not_succeed() {
    let tree = PublishTree::new();
    let record = |name: &str, text: &str| {
        fs::create_dir_all(tree.path("results")).expect("results");
        fs::write(tree.path("results").join(name), format!("{text}\n")).expect("record");
    };
    let aggregate = || tree.run("aggregate.sh", &["preflight", "aur", "copr"], &[]);
    // A step with no record at all did not run: nothing is fine by default.
    let output = aggregate();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("these steps did not succeed: preflight aur copr"),
        "{}",
        stderr_of(&output)
    );
    for (what, results, expected) in [
        (
            "all ok",
            [("preflight", "ok"), ("aur", "ok"), ("copr", "ok")],
            None,
        ),
        (
            "copr failed",
            [("preflight", "ok"), ("aur", "ok"), ("copr", "failed")],
            Some("copr".to_owned()),
        ),
        (
            "aur failed",
            [("preflight", "ok"), ("aur", "failed"), ("copr", "ok")],
            Some("aur".to_owned()),
        ),
        (
            "two failed",
            [("preflight", "ok"), ("aur", "failed"), ("copr", "failed")],
            Some("aur copr".to_owned()),
        ),
        (
            "preflight failed",
            [("preflight", "failed"), ("aur", "ok"), ("copr", "ok")],
            Some("preflight".to_owned()),
        ),
        (
            "junk is not ok",
            [("preflight", "ok"), ("aur", "okay"), ("copr", "ok")],
            Some("aur".to_owned()),
        ),
    ] {
        for (name, text) in results {
            record(name, text);
        }
        let output = aggregate();
        match expected {
            None => assert!(output.status.success(), "{what}: {}", stderr_of(&output)),
            Some(names) => {
                assert_eq!(output.status.code(), Some(1), "{what}");
                assert!(
                    stderr_of(&output).contains(&format!("these steps did not succeed: {names}.")),
                    "{what}: {}",
                    stderr_of(&output)
                );
            }
        }
    }
    // A step missing from the results while the rest are ok.
    fs::remove_file(tree.path("results/copr")).expect("remove");
    record("preflight", "ok");
    record("aur", "ok");
    let output = aggregate();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("copr: missing"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn a_step_that_dies_is_recorded_as_failed_and_only_a_finished_step_is_ok() {
    // copr.sh with an unreadable tag dies after it recorded itself; the record says failed.
    let tree = PublishTree::new();
    let output = tree.run(
        "copr.sh",
        &["not-a-tag"],
        &[
            ("PUBLISH_CHANNELS", "beta"),
            ("PUBLISH_DRY_RUN", "false"),
            ("COPR_OWNER", "ns"),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(tree.results("copr").as_deref(), Some("failed"));
}

// --- the host keys are the services' published ones ----------------------------------------------

#[test]
fn the_pinned_host_keys_are_the_published_fingerprints_of_the_two_hosts() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release/known_hosts"),
    )
    .expect("known_hosts");
    assert_eq!(text.lines().count(), 2, "{text}");
    // Fingerprints as the services publish them: the AUR's in the Arch wiki's AUR submission
    // guidelines, GitHub's in "GitHub's SSH key fingerprints" (docs.github.com).
    for (host, fingerprint) in [
        (
            "aur.archlinux.org",
            "SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4",
        ),
        (
            "github.com",
            "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU",
        ),
    ] {
        let line = text
            .lines()
            .find(|line| line.starts_with(&format!("{host} ssh-ed25519 ")))
            .unwrap_or_else(|| panic!("no key for {host}"));
        let file = tempdir().expect("tempdir");
        fs::write(file.path().join("key"), format!("{line}\n")).expect("key");
        let mut command = common::sandboxed("ssh-keygen", file.path());
        command.arg("-lf").arg(file.path().join("key"));
        let output = common::run_other_tool(&mut command);
        assert!(output.status.success(), "{}", stderr_of(&output));
        assert!(
            stdout_of(&output).contains(fingerprint),
            "{host}: {}",
            stdout_of(&output)
        );
    }
}

// --- publish.yml and release.yml ----------------------------------------------------------------

const PUBLISH_PINNED_EXPRESSION_LINES: [(&str, usize); 13] = [
    (
        "group: publish-${{ github.event.release.tag_name || inputs.tag }}",
        1,
    ),
    // The publish job and the three verification jobs, each with the read-only token.
    ("GH_TOKEN: ${{ github.token }}", 4),
    // route.sh runs in the publish job and in each of the three verification jobs.
    ("TAG: ${{ github.event.release.tag_name || inputs.tag }}", 4),
    ("DRY_RUN: ${{ inputs.dry_run }}", 1),
    ("COPR_CONFIG: ${{ secrets.COPR_CONFIG }}", 1),
    ("AUR_SSH_KEY: ${{ secrets.AUR_SSH_KEY }}", 1),
    ("APT_SIGNING_KEY: ${{ secrets.APT_SIGNING_KEY }}", 1),
    ("APT_DEPLOY_KEY: ${{ secrets.APT_DEPLOY_KEY }}", 1),
    ("COPR_TOKEN_EXPIRES: ${{ vars.COPR_TOKEN_EXPIRES }}", 1),
    ("APT_WITHDRAWN: ${{ vars.APT_WITHDRAWN }}", 1),
    ("IMAGE: fedora:${{ matrix.fedora }}", 1),
    ("IMAGE: ${{ matrix.image }}", 1),
    ("VERIFY_EXPECT_VERIFIER: ${{ matrix.verifier }}", 1),
];

#[test]
fn the_publish_workflow_has_the_triggers_the_design_names_and_no_others() {
    let publish = workflow("publish.yml");
    let on = publish.split("\npermissions:").next().expect("on block");
    assert!(on.contains("\non:\n  release:\n    types: [published]\n  workflow_dispatch:\n    inputs:\n      tag:\n"), "{on}");
    assert!(on.contains("\n      dry_run:\n"), "{on}");
    assert!(
        on.contains("type: boolean\n        default: true\n"),
        "a dispatch must default to a dry run"
    );
    assert!(
        !on.contains("push:")
            && !on.contains("pull_request")
            && !on.contains("schedule")
            && !on.contains("branches:")
            && !on.contains("tags:"),
        "{on}"
    );
    // `released` does not fire for pre-releases.
    assert!(!on.contains("released"));
}

#[test]
fn only_the_publish_job_reads_secrets_and_it_runs_in_the_release_environment() {
    let publish = workflow("publish.yml");
    let job = job_block(&publish, "publish");
    assert_eq!(
        publish.matches("environment:").count(),
        1,
        "one environment, one approval"
    );
    assert!(
        job.contains("    environment: release\n"),
        "the release environment line is missing"
    );
    assert_eq!(publish.matches("secrets.").count(), 4);
    assert_eq!(
        job.matches("secrets.").count(),
        4,
        "a secret outside the publish job"
    );
    for name in [
        "COPR_CONFIG",
        "AUR_SSH_KEY",
        "APT_SIGNING_KEY",
        "APT_DEPLOY_KEY",
    ] {
        assert_eq!(
            publish.matches(&format!("secrets.{name} ")).count(),
            1,
            "{name} is read once"
        );
    }
    // The secrets are read in ONE step, the one that writes them to files.
    let step = publish
        .split("      - name: ")
        .find(|step| step.starts_with("Write the credentials"))
        .expect("step");
    let step = step.split("\n      - ").next().expect("one step");
    assert_eq!(
        step.matches("secrets.").count(),
        4,
        "the credentials step must read exactly the four secrets"
    );
    // Permissions: contents read at the top, and no job asks for more.
    assert_eq!(permission_blocks(&publish), [vec!["contents: read"]]);
    assert!(!publish.contains("contents: write") && !publish.contains("write-all"));
    // The default token reads: it is the only token, and it is not a secret. One in the publish
    // job and one in each verification job.
    assert_eq!(publish.matches("github.token").count(), 4);
    // One run at a time per tag, never cancelled.
    assert!(publish.contains("concurrency:\n  group: publish-${{ github.event.release.tag_name || inputs.tag }}\n  cancel-in-progress: false\n"));
    assert!(!publish.contains("cancel-in-progress: true"));
    // Every checkout leaves no credential in .git.
    assert!(publish.matches("uses: actions/checkout@").count() >= 1);
    assert_eq!(
        publish.matches("uses: actions/checkout@").count(),
        publish
            .matches("        with:\n          persist-credentials: false\n")
            .count()
    );
    assert!(publish.contains("\ndefaults:\n  run:\n    shell: bash\n"));
}

#[test]
fn no_run_step_of_the_publish_workflow_holds_an_expression_and_its_expression_lines_are_pinned() {
    let publish = workflow("publish.yml");
    let unsafe_lines = unsafe_expression_lines(&publish);
    assert!(
        unsafe_lines.is_empty(),
        "an expression outside env:, if: and concurrency.group: {unsafe_lines:?}"
    );
    let problems = expression_pin_problems(&publish, &PUBLISH_PINNED_EXPRESSION_LINES);
    assert!(
        problems.is_empty(),
        "the file's expression lines changed: review each one: {problems:?}"
    );
    let blocks = run_blocks(&publish);
    assert!(
        blocks.len() >= 6,
        "found {} run steps: {blocks:?}",
        blocks.len()
    );
    for block in &blocks {
        assert!(
            !block.contains("${{"),
            "an expression in a run step: {block}"
        );
    }
    // The pin test bites on this file's shapes too: a secret in a run, a swapped line, an extra one.
    let poisoned = publish.replacen(
        "run: packaging/release/credentials.sh remove",
        "run: echo ${{ secrets.COPR_CONFIG }}",
        1,
    );
    assert!(!unsafe_expression_lines(&poisoned).is_empty());
    let swapped = publish.replacen(
        "COPR_CONFIG: ${{ secrets.COPR_CONFIG }}",
        "COPR_CONFIG: ${{ secrets.AUR_SSH_KEY }}",
        1,
    );
    assert!(!expression_pin_problems(&swapped, &PUBLISH_PINNED_EXPRESSION_LINES).is_empty());
}

#[test]
fn the_publish_steps_run_in_order_and_each_channel_runs_after_a_good_preflight_even_if_the_other_failed()
 {
    let publish = workflow("publish.yml");
    let job = job_block(&publish, "publish");
    let position = |needle: &str| {
        job.find(needle)
            .unwrap_or_else(|| panic!("no step '{needle}'"))
    };
    let order = [
        "name: Choose the channels",
        "name: Check out the tag and run its gate",
        "name: Install copr-cli",
        "name: Write the credentials",
        "name: Preflight",
        // Apt before the AUR before COPR: the apt step is the quickest and the COPR step waits on builds.
        "name: apt\n",
        "name: AUR",
        "name: COPR",
        "name: Check every step succeeded",
        "name: Remove the credentials",
    ];
    let positions: Vec<usize> = order.iter().map(|name| position(name)).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{positions:?}"
    );
    // Channel steps: not after a cancel, only after a successful preflight; never `always()` (a
    // person cancelling the run must stop the publishing), and never continue-on-error.
    for step in ["apt", "AUR", "COPR"] {
        let block = job
            .split("      - name: ")
            .find(|s| s.starts_with(&format!("{step}\n")))
            .unwrap_or_else(|| panic!("no step {step}"));
        let block = block.split("\n      - ").next().expect("one step");
        assert!(
            block.contains("if: \"!cancelled() && steps.preflight.outcome == 'success'\""),
            "{step}: {block}"
        );
        assert!(!block.contains("always()"), "{step}: {block}");
    }
    assert!(!job.contains("continue-on-error"));
    assert!(job.contains("id: preflight\n"));
    // The final step names the steps it checks; the cleanup always runs.
    assert!(job.contains("if: \"always() && steps.route.outcome == 'success'\"\n        run: packaging/release/aggregate.sh preflight apt aur copr\n"), "the final step is missing, ungated, or does not list every step");
    assert!(
        job.contains("if: always()\n        run: packaging/release/credentials.sh remove\n"),
        "the cleanup step is missing or ungated"
    );
    // Each step calls a script; the logic is not in the workflow.
    for call in [
        "packaging/release/route.sh \"$TAG\"",
        "packaging/release/tag-checkout.sh \"$PUBLISH_TAG\"",
        "packaging/release/preflight.sh",
        "packaging/release/apt-publish.sh \"$PUBLISH_TAG\"",
        "packaging/release/aur-publish.sh \"$PUBLISH_TAG\"",
        "packaging/release/copr.sh \"$PUBLISH_TAG\"",
    ] {
        assert!(job.contains(&format!("run: {call}\n")), "{call}");
    }
    // The aggregate names exactly the steps that record a result, in step order (each of them
    // calls record_on_exit with its own name).
    for step in ["apt", "aur", "copr"] {
        assert!(
            job.contains(&format!("id: {step}\n")),
            "no step with id {step}"
        );
    }
    // The three verification jobs exist, run after the publish job and only for a real run.
    for name in ["verify-copr", "verify-aur", "verify-apt"] {
        let block = job_block(&publish, name);
        assert!(block.contains("    needs: publish\n"), "{name}: {block}");
        assert!(
            block.contains("    if: \"!inputs.dry_run\"\n"),
            "{name}: {block}"
        );
    }
    // A release is never published from a workflow, and nothing writes to a release.
    assert!(
        !publish.contains("gh release"),
        "a release command in the publish workflow"
    );
}

#[test]
fn the_copr_namespace_is_written_once_in_the_workflow_and_nowhere_in_the_scripts() {
    let publish = workflow("publish.yml");
    assert_eq!(
        publish.matches("osirison").count(),
        1,
        "the COPR namespace must be written once"
    );
    assert_eq!(
        publish.matches("\nenv:\n  COPR_OWNER: osirison\n").count(),
        1
    );
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release");
    for entry in fs::read_dir(&directory).expect("release dir") {
        let path = entry.expect("entry").path();
        let text = fs::read_to_string(&path).expect("script");
        // The one other place the name appears is the apt repository's public address, the GitHub
        // Pages host of the repository's owner (design D10): a different namespace, which a user
        // types and so cannot be read from a variable. It is allowed in that file, on that one
        // line, and nowhere else.
        let allowed = "readonly SITE_URL=\"https://osirison.github.io/proton-drive-sync-engine-apt/\"";
        let is_client = path.file_name().is_some_and(|name| name == "apt-client.sh");
        let remaining = if is_client {
            assert_eq!(text.matches(allowed).count(), 1, "{}", path.display());
            text.replacen(allowed, "", 1)
        } else {
            text
        };
        assert!(
            !remaining.contains("osirison"),
            "{} holds the COPR namespace",
            path.display()
        );
        let text = remaining;
        // None of the scripts writes to a release or the API.
        if path.file_name().is_some_and(|name| name != "draft.sh") {
            for write in [
                "release create",
                "release edit",
                "release delete",
                "release upload",
                "--method",
                "gh api -X",
                "-X POST",
            ] {
                assert!(!text.contains(write), "{} holds '{write}'", path.display());
            }
        }
    }
}

#[test]
fn release_yml_warns_about_the_copr_token_through_env_and_its_pins_are_updated_on_purpose() {
    let release = workflow("release.yml");
    assert!(release.contains(
        "      - name: Warn when the COPR token is about to expire\n        env:\n          COPR_TOKEN_EXPIRES: ${{ vars.COPR_TOKEN_EXPIRES }}\n        run: packaging/release/token-expiry.sh warn\n"
    ), "the release workflow lacks the expiry warning step");
    assert!(job_block(&release, "gate").contains("token-expiry.sh warn"));
    assert_eq!(release.matches("vars.").count(), 1);
    assert!(unsafe_expression_lines(&release).is_empty());
    assert!(expression_pin_problems(&release, &RELEASE_PINNED_EXPRESSION_LINES).is_empty());
}

#[test]
fn a_publish_script_that_loses_its_stubs_reaches_a_refusing_one_and_the_test_notices() {
    // Without the stub directory ahead of the sandbox's, `copr-cli` and `ssh` resolve to the
    // sandbox's refusing stubs (and never to the host's), which record the call.
    let tree = PublishTree::new();
    let environment = tree.good_credentials();
    let mut command = tree.command("preflight.sh");
    command.env(
        "PATH",
        common::path_with_stubs(&std::env::var("PATH").expect("PATH")),
    );
    for (name, value) in &environment {
        command.env(name, value);
    }
    let output = common::run_other_tool(&mut command);
    assert_eq!(output.status.code(), Some(1));
    let hits = common::stub_hits(&command);
    assert!(hits.contains("copr-cli --config"), "{hits}");
    assert!(hits.contains("ssh -i"), "{hits}");
    assert!(tree.log("copr.log").is_empty() && tree.log("ssh.log").is_empty());
}

#[test]
fn a_tree_for_another_package_than_the_channel_names_is_never_pushed() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta", "proton-drive-sync-engine"]);
    let upstream = tree.aur_assets("one");
    // A makepkg that prints another package base than the PKGBUILD's.
    write_executable(
        &tree.stubs.join("makepkg"),
        "#!/bin/sh\n[ \"$1\" = --printsrcinfo ] && echo 'pkgbase = some-other-package'\n",
    );
    let output = tree.aur(&upstream, "beta", false, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("is for the package 'some-other-package'"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 0);
}

// --- fix round 1 ---------------------------------------------------------------------------------

fn uid_of(path: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).expect("metadata").uid()
}

// F1 -----------------------------------------------------------------------------------------------

#[test]
fn the_container_path_gets_a_work_directory_the_container_user_can_enter_and_exact_docker_arguments()
 {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta", "proton-drive-sync-engine"]);
    let upstream = tree.aur_assets("one");
    // The default generator is the container; the stub docker refuses a work directory whose mode
    // leaves out the others' execute bit (mktemp makes 0700, which a user of another uid cannot enter).
    let output = tree.aur(
        &upstream,
        "beta stable",
        false,
        &[("AUR_GENERATOR", "docker")],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 1);
    assert_eq!(tree.commits("proton-drive-sync-engine"), 1);

    let log = tree.log("docker.log");
    assert_eq!(log.len(), 1, "{log:?}");
    let words: Vec<&str> = log[0].split_whitespace().collect();
    let uid = uid_of(&tree.path("tree")).to_string();
    let script = format!(
        "{}:/aur-container.sh:ro",
        tree.script("aur-container.sh").display()
    );
    // The whole argument list: network off, the runner's uid as the only environment variable, two
    // mounts (the work directory and the read-only script), the image, the script and the channels.
    assert_eq!(
        &words[..5],
        ["run", "--rm", "--network", "none", "-e"],
        "{log:?}"
    );
    assert_eq!(words[5], format!("HOST_UID={uid}"), "{log:?}");
    assert_eq!(words[6], "-v");
    let work = words[7].strip_suffix(":/work").expect("the work mount");
    assert_eq!(words[8], "-v");
    assert_eq!(words[9], script);
    assert_eq!(
        &words[10..13],
        ["archlinux:base-devel", "bash", "/aur-container.sh"]
    );
    assert_eq!(&words[13..], ["beta", "stable"]);
    // Nothing else is shared: not the home directory, not the credentials, not the key.
    let home = tree.directory.path().display().to_string();
    assert_ne!(work, home);
    assert!(
        !work.contains("credentials") && !work.starts_with(&format!("{home}/")),
        "{work}"
    );
    assert!(
        !log[0].contains("aur-key") && !log[0].contains("credentials"),
        "{log:?}"
    );
    assert_eq!(log[0].matches(" -v ").count(), 2);
    assert_eq!(log[0].matches(" -e ").count(), 1);
}

#[test]
fn aur_container_makes_the_work_directory_traversable_and_hands_it_back_at_exit_even_on_failure() {
    let tree = PublishTree::new();
    let work = tree.path("cwork");
    for name in ["src", "assets"] {
        fs::create_dir_all(work.join(name)).expect("dir");
    }
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).expect("mode");
    let run = |channels: &[&str], uid: &str, extra: &[(&str, &str)]| {
        let mut command = tree.command("aur-container.sh");
        command.args(channels);
        let mut path = std::ffi::OsString::from(tree.path("container-stubs"));
        path.push(":");
        path.push(std::env::var("PATH").expect("PATH"));
        command
            .env("PATH", path)
            .env("AUR_WORK_DIR", &work)
            .env("HOST_UID", uid)
            .env("STUB_SU_SKIP", "1");
        for (name, value) in extra {
            command.env(name, value);
        }
        common::run_other_tool(&mut command)
    };
    let output = run(&["beta", "stable"], "4242", &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let mode = fs::metadata(&work).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "the container user must be able to enter /work"
    );
    let log = tree.log("container.log");
    let w = work.display();
    assert_eq!(log[0], "useradd -m builder");
    assert!(log.contains(&format!("chown builder {w}/trees")), "{log:?}");
    assert!(
        log.contains(&format!("chown -R builder {w}/src {w}/assets")),
        "{log:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|line| line.starts_with("su builder"))
            .count(),
        2,
        "{log:?}"
    );
    assert_eq!(
        log.last().expect("log"),
        &format!("chown -R 4242 {w}"),
        "{log:?}"
    );

    // aur.sh failing still hands the directory back, and the exit is non-zero.
    let _ = fs::remove_file(tree.path("container.log"));
    let _ = fs::remove_dir(work.join("trees"));
    let failed = run(&["beta"], "4242", &[("STUB_SU_FAILS", "1")]);
    assert_ne!(failed.status.code(), Some(0));
    assert_eq!(
        tree.log("container.log").last().expect("log"),
        &format!("chown -R 4242 {w}")
    );
}

#[test]
fn aur_container_refuses_an_unknown_channel_or_a_bad_uid_before_it_touches_anything() {
    let tree = PublishTree::new();
    let work = tree.path("cwork");
    fs::create_dir_all(work.join("src")).expect("dir");
    for (what, channels, uid) in [
        ("an unknown channel", vec!["nightly"], "1"),
        ("one unknown among good ones", vec!["beta", "nightly"], "1"),
        ("a path as channel", vec!["../x"], "1"),
        ("a non-numeric uid", vec!["beta"], "abc"),
        ("an empty uid", vec!["beta"], ""),
        ("no channel", vec![], "1"),
    ] {
        let mut command = tree.command("aur-container.sh");
        command.args(&channels);
        let mut path = std::ffi::OsString::from(tree.path("container-stubs"));
        path.push(":");
        path.push(std::env::var("PATH").expect("PATH"));
        command
            .env("PATH", path)
            .env("AUR_WORK_DIR", &work)
            .env("HOST_UID", uid);
        let output = common::run_other_tool(&mut command);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(
            tree.log("container.log").is_empty(),
            "{what}: ran {:?}",
            tree.log("container.log")
        );
    }
}

// F2 -----------------------------------------------------------------------------------------------

#[test]
fn a_tree_with_a_link_or_anything_but_the_three_plain_files_is_refused_and_nothing_is_pushed() {
    let bait_text = "BAIT-FILE-TEXT";
    // (what, hook run by the hostile aur.sh in the tree directory $tree)
    let cases = [
        (
            "a symlinked PKGBUILD",
            "rm \"$tree/PKGBUILD\"; ln -s \"$BAIT\" \"$tree/PKGBUILD\"",
        ),
        (
            "a symlinked .SRCINFO",
            "rm \"$tree/.SRCINFO\"; ln -s \"$BAIT\" \"$tree/.SRCINFO\"",
        ),
        (
            "a symlinked install script",
            "rm \"$tree/proton-drive-sync-engine.install\"; ln -s \"$BAIT\" \"$tree/proton-drive-sync-engine.install\"",
        ),
        (
            "a symlinked tree directory",
            "mv \"$tree\" \"$tree.real\"; ln -s \"$tree.real\" \"$tree\"",
        ),
        (
            "a FIFO",
            "rm \"$tree/proton-drive-sync-engine.install\"; mkfifo \"$tree/proton-drive-sync-engine.install\"",
        ),
        ("an extra file", "echo x > \"$tree/extra\""),
        ("an extra directory", "mkdir \"$tree/sub\""),
    ];
    for (what, hook) in cases {
        let tree = PublishTree::new();
        tree.aur_remote(&["proton-drive-sync-engine-beta"]);
        let bait = tree.path("the-bait");
        fs::write(&bait, bait_text).expect("bait");
        let hook = format!("BAIT='{}'\n{hook}", bait.display());
        let upstream = tree.aur_assets_with("one", &hook);
        let output = tree.aur(&upstream, "beta", false, &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert_eq!(
            tree.commits("proton-drive-sync-engine-beta"),
            0,
            "{what} was pushed"
        );
        assert!(
            !stderr_of(&output).contains(bait_text),
            "{what}: the bait reached the log"
        );
        assert_eq!(tree.results("aur").as_deref(), Some("failed"), "{what}");
    }
}

// F3 -----------------------------------------------------------------------------------------------

impl GateTree {
    /// A repository with the publish scripts, an `origin` that is a bare repository next to it, and
    /// `main` pushed. Returns the origin's path.
    fn with_origin(&self) -> std::path::PathBuf {
        let release = self.tree.join("packaging/release");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release");
        for file in ["tag-checkout.sh", "publish-lib.sh"] {
            fs::copy(root.join(file), release.join(file)).expect(file);
        }
        self.commit("publish scripts");
        let origin = self.directory.path().join("origin.git");
        let mut init = common::sandboxed("git", self.directory.path());
        init.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["init", "--bare", "-q", "-b", "main"])
            .arg(&origin);
        assert!(common::run_other_tool(&mut init).status.success());
        self.git(&["remote", "add", "origin", &origin.display().to_string()]);
        self.git(&["push", "-q", "origin", "main"]);
        origin
    }

    fn tag_checkout(&self, tag: &str) -> std::process::Output {
        let mut command = common::sandboxed("bash", self.directory.path());
        command
            .arg(self.tree.join("packaging/release/tag-checkout.sh"))
            .arg(tag)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GH_TOKEN");
        common::run_other_tool(&mut command)
    }
}

#[test]
fn the_tag_is_checked_out_and_gated_before_anything_else_and_a_hand_made_tag_off_main_fails() {
    // A tag on a commit that main does not hold, dispatched from the branch that has it.
    let gate = gate_tree();
    gate.with_origin();
    gate.git(&["checkout", "-q", "-b", "side"]);
    let side = gate.commit("only on side");
    gate.git(&["tag", "v0.1.0", &side]);
    gate.git(&["push", "-q", "origin", "side", "v0.1.0"]);
    let output = gate.tag_checkout("v0.1.0");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("is not an ancestor of"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn a_tag_on_main_is_checked_out_over_the_dispatched_branch_and_a_bad_tag_is_refused() {
    let gate = gate_tree();
    gate.with_origin();
    let on_main = gate.git(&["rev-parse", "HEAD"]);
    gate.git(&["tag", "v0.1.0", &on_main]);
    gate.git(&["tag", "v0.2.0", &on_main]);
    gate.git(&["push", "-q", "origin", "v0.1.0", "v0.2.0"]);
    // The run was started from a branch with a newer commit; the tag's tree is what runs.
    gate.git(&["checkout", "-q", "-b", "side"]);
    fs::write(gate.tree.join("only-on-side"), "x").expect("file");
    gate.commit("side");
    let output = gate.tag_checkout("v0.1.0");
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(gate.git(&["rev-parse", "HEAD"]), on_main);
    assert!(!gate.tree.join("only-on-side").exists());
    // A tag that is not the Cargo.toml version, one that does not exist, one of the wrong shape.
    let mismatch = gate.tag_checkout("v0.2.0");
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(stderr_of(&mismatch).contains("0.2.0") && stderr_of(&mismatch).contains("0.1.0"));
    let missing = gate.tag_checkout("v0.1.5");
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stderr_of(&missing).contains("cannot fetch the tag"),
        "{}",
        stderr_of(&missing)
    );
    assert_eq!(gate.tag_checkout("v0.1").status.code(), Some(1));
}

// F4 -----------------------------------------------------------------------------------------------

fn assert_pinned_ssh_options(tree: &PublishTree, line: &str, key: &Path) {
    for needle in [
        format!("-i {} ", key.display()),
        "IdentitiesOnly=yes".to_owned(),
        "BatchMode=yes".to_owned(),
        "StrictHostKeyChecking=yes".to_owned(),
        format!(
            "UserKnownHostsFile={}",
            tree.script("known_hosts").display()
        ),
        "GlobalKnownHostsFile=/dev/null".to_owned(),
    ] {
        assert!(line.contains(&needle), "'{needle}' missing from: {line}");
    }
}

#[test]
fn every_ssh_use_pins_the_host_keys_and_the_key_for_the_aur_probe_the_apt_check_and_the_aur_clone()
{
    let tree = PublishTree::new();
    let good = tree.good_credentials();
    let deploy = tree.path("creds/deploy");
    let aur_key = tree.path("creds/aur");
    // The apt remote is an ssh URL, so git runs GIT_SSH_COMMAND, which resolves to the stub ssh.
    let environment = with(&good, "APT_REMOTE_URL", "ssh://git@apt.invalid/apt.git");
    let output = tree.preflight(&environment);
    assert_eq!(output.status.code(), Some(1));
    let ssh = tree.log("ssh.log");
    assert_eq!(ssh.len(), 2, "{ssh:?}");
    assert!(ssh[0].contains("-T aur@aur.archlinux.org"), "{}", ssh[0]);
    assert_pinned_ssh_options(&tree, &ssh[0], &aur_key);
    assert!(ssh[1].contains("apt.invalid"), "{}", ssh[1]);
    assert_pinned_ssh_options(&tree, &ssh[1], &deploy);

    // The AUR clone: an ssh base URL.
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta"]);
    let upstream = tree.aur_assets("one");
    let output = tree.aur(
        &upstream,
        "beta",
        false,
        &[
            ("AUR_REMOTE_BASE", "ssh://aur@aur.invalid"),
            ("STUB_SSH_MODE", "greeting"),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let ssh = tree.log("ssh.log");
    assert!(!ssh.is_empty(), "git did not run the ssh command");
    assert_pinned_ssh_options(&tree, &ssh[0], &tree.path("aur-key"));
}

#[test]
fn the_throwaway_gnupg_home_is_removed_whether_the_signing_check_passes_or_fails() {
    let tree = PublishTree::new();
    let base = tree.path("gpg-tmp");
    fs::create_dir(&base).expect("tmp");
    let good = with(
        &tree.good_credentials(),
        "PUBLISH_GPG_TMPDIR",
        &base.display().to_string(),
    );
    let output = tree.preflight(&good);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_dir(&base).expect("dir").count(),
        0,
        "a gpg home was left behind"
    );
    let bad = with(
        &good,
        "APT_SIGNING_KEY_FILE",
        &good_path(&good, "AUR_SSH_KEY_FILE"),
    );
    let output = tree.preflight(&bad);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        fs::read_dir(&base).expect("dir").count(),
        0,
        "a gpg home was left behind"
    );
}

#[test]
fn credentials_are_private_even_when_the_caller_has_umask_zero() {
    let tree = PublishTree::new();
    let mut command = common::sandboxed("bash", tree.directory.path());
    command
        .arg("-c")
        .arg("umask 000; exec bash \"$0\" write")
        .arg(tree.script("credentials.sh"))
        .env("PUBLISH_CREDENTIALS_DIR", tree.path("credentials"))
        .env("GITHUB_ENV", tree.path("github-env"))
        .env("COPR_CONFIG", "a")
        .env("AUR_SSH_KEY", "b")
        .env("APT_SIGNING_KEY", "c")
        .env("APT_DEPLOY_KEY", "d");
    let output = common::run_other_tool(&mut command);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let mode = |path: &Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode(&tree.path("credentials")), 0o700);
    for file in [
        "copr-config",
        "aur-ssh-key",
        "apt-signing-key",
        "apt-deploy-key",
    ] {
        assert_eq!(mode(&tree.path("credentials").join(file)), 0o600, "{file}");
    }
}

#[test]
fn every_release_script_starts_with_set_euo_pipefail() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release");
    let mut checked = 0;
    for entry in fs::read_dir(&directory).expect("release dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|extension| extension != "sh") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("script");
        let first_code = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'));
        if path
            .file_name()
            .is_some_and(|name| name == "publish-lib.sh")
        {
            // Sourced, never run: it sets nothing of its own.
            assert_ne!(first_code, Some("set -euo pipefail"));
            continue;
        }
        assert_eq!(first_code, Some("set -euo pipefail"), "{}", path.display());
        assert!(
            text.starts_with("#!/usr/bin/env bash\n"),
            "{}",
            path.display()
        );
        checked += 1;
    }
    assert!(checked >= 11, "only {checked} scripts found");
}

#[test]
fn the_sandbox_replaces_every_tool_a_publish_script_could_reach_the_host_with() {
    for tool in ["gh", "copr-cli", "ssh", "docker", "secret-tool", "curl"] {
        assert!(
            common::STUBBED_TOOLS.contains(&tool),
            "{tool} is not a refusing stub"
        );
    }
}

#[test]
fn the_publish_checkout_keeps_no_credentials_and_fetches_the_history_the_gate_needs() {
    let publish = workflow("publish.yml");
    // The ref (a pinned commit) is not part of what this checks: skip the step's own line,
    // then compare what follows it.
    let step = publish
        .split("      - uses: actions/checkout@")
        .nth(1)
        .and_then(|rest| rest.split_once('\n'))
        .map(|(_, after)| after)
        .expect("a checkout step");
    assert!(
        step.starts_with(
            "        with:\n          persist-credentials: false\n          fetch-depth: 0\n"
        ),
        "the checkout step"
    );
}

/// The value of a `uses` key on one workflow line, or `None` when the line is not a `uses:` line
/// (a comment, another key, or text that merely contains the word).
fn uses_value(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("- ").map_or(line, str::trim_start);
    let key_end = line.find(':')?;
    let key = line[..key_end].trim_end();
    let key = [('"', '"'), ('\'', '\'')]
        .iter()
        .find_map(|&(open, close)| key.strip_prefix(open)?.strip_suffix(close))
        .unwrap_or(key);
    (key == "uses").then(|| line[key_end + 1..].trim())
}

#[test]
fn every_workflow_step_names_an_action_by_commit_with_its_version_in_a_comment() {
    // A tag can be moved upstream; a commit cannot. Dependabot keeps the pin and the comment
    // together, so a pinned line is `owner/repo[/path]@<40 hex> # <version>`. Dependabot only
    // maintains version-shaped comments, so the comment must be `v<digit>...`; the one action
    // with no release tag carries its own exact comment.
    const NO_TAG: (&str, &str) = ("dtolnay/rust-toolchain", "stable branch, no release tag");
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let (mut pinned, mut local, mut files) = (0, 0, 0);
    for entry in fs::read_dir(&directory).expect("workflows") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        // GitHub loads both extensions.
        if !(name.ends_with(".yml") || name.ends_with(".yaml")) {
            continue;
        }
        files += 1;
        for line in fs::read_to_string(&path).expect("workflow").lines() {
            let Some(reference) = uses_value(line) else {
                continue;
            };
            if reference.starts_with("./") {
                local += 1;
                continue;
            }
            let (action, rest) = reference
                .split_once('@')
                .unwrap_or_else(|| panic!("{name}: no @ in `{line}`"));
            let (sha, comment) = rest
                .split_once('#')
                .unwrap_or_else(|| panic!("{name}: no version comment in `{line}`"));
            let (sha, comment) = (sha.trim(), comment.trim());
            assert!(
                sha.len() == 40 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
                "{name}: `{line}` is not pinned to a full commit"
            );
            assert!(
                action.split('/').count() >= 2 && !action.contains(char::is_whitespace),
                "{name}: `{line}` is not owner/repo[/path]"
            );
            if action == NO_TAG.0 {
                assert_eq!(
                    comment, NO_TAG.1,
                    "{name}: `{line}` must carry the comment `{}`",
                    NO_TAG.1
                );
            } else {
                let mut chars = comment.chars();
                assert!(
                    chars.next() == Some('v') && chars.next().is_some_and(|c| c.is_ascii_digit()),
                    "{name}: comment `{comment}` in `{line}` is not a version (`v` then a digit)"
                );
            }
            pinned += 1;
        }
    }
    // Not vacuous: there are workflows, pinned lines and the two local references.
    assert!(
        files >= 8 && pinned >= 40 && local >= 2,
        "floor not met: workflow files {files} (expected >= 8), pinned `uses:` lines {pinned} \
         (expected >= 40), local `./` references {local} (expected >= 2)"
    );
}

// F5, F6 -------------------------------------------------------------------------------------------

impl PublishTree {
    /// One package with one build in `state`, no version yet, and the given source URL (if any).
    fn unversioned_build(&self, project: &str, state: &str, url: Option<&str>) {
        let mut source = serde_json::json!({ "name": "proton-drive-sync-engine", "version": null });
        if let Some(url) = url {
            source["url"] = url.into();
        }
        fs::write(
            self.path("packages")
                .join(format!("{}.json", project.replace('/', "_"))),
            serde_json::json!([{
                "name": "proton-drive-sync-engine",
                "builds": [{ "id": 9, "state": state, "source_package": source }],
                "latest_build": null,
                "latest_succeeded_build": null
            }])
            .to_string(),
        )
        .expect("packages");
    }
}

#[test]
fn a_running_build_with_no_version_yet_is_this_release_only_when_its_source_names_the_srpm() {
    let ours = format!("https://copr.example.invalid/results/x/srpm-builds/9/{SRPM}");
    let other = "https://copr.example.invalid/results/x/srpm-builds/9/proton-drive-sync-engine-0.1.0-beta.2-1.fc43.src.rpm";
    // (what, state, url, builds submitted, watched)
    for (what, state, url, builds, watched) in [
        (
            "running, our file",
            "running",
            Some(ours.as_str()),
            0,
            vec!["watch-build 9"],
        ),
        (
            "importing, our file",
            "importing",
            Some(ours.as_str()),
            0,
            vec!["watch-build 9"],
        ),
        (
            "running, another file",
            "running",
            Some(other),
            1,
            vec!["watch-build 7001"],
        ),
        (
            "running, no source URL",
            "running",
            None,
            1,
            vec!["watch-build 7001"],
        ),
        (
            "succeeded without a version is not a success to rely on",
            "succeeded",
            Some(ours.as_str()),
            1,
            vec!["watch-build 7001"],
        ),
        (
            "failed, our file",
            "failed",
            Some(ours.as_str()),
            1,
            vec!["watch-build 7001"],
        ),
    ] {
        let tree = PublishTree::new();
        let setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
        tree.unversioned_build("ns/proton-drive-sync-engine-beta", state, url);
        let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
        assert!(output.status.success(), "{what}: {}", stderr_of(&output));
        assert_eq!(tree.copr_calls("build").len(), builds, "{what}");
        assert_eq!(tree.copr_calls("watch-build"), watched, "{what}");
    }
}

#[test]
fn a_dry_run_that_meets_a_running_build_says_it_would_wait_and_never_calls_watch_build() {
    let tree = PublishTree::new();
    let mut setup = tree.copr_setup("v0.1.0-beta.1", &[SRPM]);
    setup.retain(|(name, _)| *name != "PUBLISH_DRY_RUN" && *name != "STUB_COPR_ALLOW_BUILD");
    setup.push(("PUBLISH_DRY_RUN", "true".to_owned()));
    tree.packages_json(
        "ns/proton-drive-sync-engine-beta",
        &[(9, "running", "0.1.0~beta.1-1")],
    );
    let output = tree.copr("v0.1.0-beta.1", "beta", &[], &setup);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("would wait for builds 9"),
        "{}",
        stderr_of(&output)
    );
    assert!(tree.copr_calls("watch-build").is_empty() && tree.copr_calls("build").is_empty());
}

#[test]
fn a_token_date_far_beyond_a_tokens_life_is_refused_by_the_check_and_warned_by_the_warning() {
    let tree = PublishTree::new();
    let check = |value: &str| {
        tree.run(
            "token-expiry.sh",
            &["check"],
            &[("COPR_TOKEN_EXPIRES", value)],
        )
    };
    // TODAY is 2027-01-01: 2027-07-20 is 200 days away and fine, the next day is not.
    assert!(check("2027-07-20").status.success());
    for value in ["2027-07-21", "9999-12-31"] {
        let output = check(value);
        assert_eq!(output.status.code(), Some(1), "{value}");
        let text = stderr_of(&output);
        assert!(
            text.contains("COPR_TOKEN_EXPIRES") && text.contains("180 days"),
            "{value}: {text}"
        );
    }
    let warn = |value: &str| {
        tree.run(
            "token-expiry.sh",
            &["warn"],
            &[("COPR_TOKEN_EXPIRES", value)],
        )
    };
    assert_eq!(stdout_of(&warn("2027-07-20")), "");
    for value in ["2027-07-21", "9999-12-31"] {
        let output = warn(value);
        assert!(output.status.success());
        assert_eq!(
            stdout_of(&output).matches("::warning").count(),
            1,
            "{value}"
        );
        assert!(stdout_of(&output).contains("COPR_TOKEN_EXPIRES"));
    }
}

#[test]
fn a_failing_git_status_fails_the_aur_channel_instead_of_reading_as_no_change() {
    let tree = PublishTree::new();
    tree.aur_remote(&["proton-drive-sync-engine-beta"]);
    let upstream = tree.aur_assets("one");
    // A git that behaves except for `status`.
    let wrapper = format!(
        "#!/bin/sh\nfor argument in \"$@\"; do [ \"$argument\" = status ] && exit 1; done\nexec {} \"$@\"\n",
        real_git().display()
    );
    write_executable(&tree.stubs.join("git"), &wrapper);
    let output = tree.aur(&upstream, "beta", false, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("git status failed"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(tree.commits("proton-drive-sync-engine-beta"), 0);
}

// --- fix round 2 ---------------------------------------------------------------------------------

/// A git that logs its argv and the environment it was given, and answers the three calls
/// tag-checkout.sh makes (fetch, rev-parse, checkout) without a repository.
const GIT_LOGGING_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$STUB_GIT_ARGV"
{
  echo "call: $*"
  echo "GIT_CONFIG_COUNT=${GIT_CONFIG_COUNT-unset}"
  echo "GIT_CONFIG_KEY_0=${GIT_CONFIG_KEY_0-unset}"
  echo "GIT_CONFIG_VALUE_0=${GIT_CONFIG_VALUE_0-unset}"
  echo "GIT_TERMINAL_PROMPT=${GIT_TERMINAL_PROMPT-unset}"
} >> "$STUB_GIT_ENV"
case " $* " in
  *" rev-parse "*) echo 0123456789abcdef0123456789abcdef01234567 ;;
esac
"#;

#[test]
fn the_token_reaches_git_only_through_its_environment_and_is_never_printed() {
    let tree = PublishTree::new();
    write_executable(&tree.stubs.join("git"), GIT_LOGGING_STUB);
    // The gate that tag-checkout.sh runs last; this test is about the fetches.
    write_executable(&tree.script("gate.sh"), "#!/bin/sh\nexit 0\n");
    let token = "s3cr3t-token";
    let mut encode = common::sandboxed("bash", tree.directory.path());
    encode
        .arg("-c")
        .arg(format!("printf 'x-access-token:%s' {token} | base64 -w0"));
    let encoded = stdout_of(&common::run_other_tool(&mut encode));
    assert!(!encoded.is_empty());
    let header = format!("AUTHORIZATION: basic {encoded}");

    let output = tree.run(
        "tag-checkout.sh",
        &["v0.1.0"],
        &[
            ("GH_TOKEN", token),
            (
                "STUB_GIT_ARGV",
                &tree.path("git.argv").display().to_string(),
            ),
            ("STUB_GIT_ENV", &tree.path("git.env").display().to_string()),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));

    // Every git call got the header through GIT_CONFIG_*, and a prompt-less terminal.
    let env = fs::read_to_string(tree.path("git.env")).expect("git env log");
    let calls = env.matches("call: ").count();
    assert!(calls >= 4, "{env}");
    assert_eq!(env.matches("GIT_CONFIG_COUNT=1\n").count(), calls, "{env}");
    assert_eq!(
        env.matches("GIT_CONFIG_KEY_0=http.extraheader\n").count(),
        calls,
        "{env}"
    );
    assert_eq!(
        env.matches(&format!("GIT_CONFIG_VALUE_0={header}\n"))
            .count(),
        calls,
        "{env}"
    );
    assert_eq!(
        env.matches("GIT_TERMINAL_PROMPT=0\n").count(),
        calls,
        "{env}"
    );

    // Neither the token, its base64 form nor the whole header is in git's argv, stdout or stderr.
    let argv = fs::read_to_string(tree.path("git.argv")).expect("git argv log");
    for (where_, text) in [
        ("git's argv", argv.as_str()),
        ("stdout", &stdout_of(&output)),
        ("stderr", &stderr_of(&output)),
    ] {
        for needle in [token, encoded.as_str(), header.as_str(), "AUTHORIZATION"] {
            assert!(!text.contains(needle), "{where_} holds the token material");
        }
    }

    // Without a token nothing is passed at all.
    let _ = fs::remove_file(tree.path("git.env"));
    let output = tree.run(
        "tag-checkout.sh",
        &["v0.1.0"],
        &[
            (
                "STUB_GIT_ARGV",
                &tree.path("git.argv").display().to_string(),
            ),
            ("STUB_GIT_ENV", &tree.path("git.env").display().to_string()),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let env = fs::read_to_string(tree.path("git.env")).expect("git env log");
    assert_eq!(
        env.matches("GIT_CONFIG_COUNT=unset\n").count(),
        env.matches("call: ").count()
    );
}

#[test]
fn copr_cli_is_installed_from_the_hash_locked_file_with_hashes_required_and_no_dependency_lookup() {
    let publish = workflow("publish.yml");
    let installs: Vec<&str> = publish
        .lines()
        .filter(|line| line.contains(" install "))
        .collect();
    // The only other install is apt-utils, for apt-ftparchive, from the runner's own distribution.
    assert_eq!(
        installs,
        [
            "          \"$RUNNER_TEMP/copr-venv/bin/pip\" install --quiet --require-hashes --no-deps -r packaging/release/copr-cli-requirements.txt",
            "          sudo apt-get install -y -qq --no-install-recommends apt-utils",
        ]
    );
    let requirements = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release/copr-cli-requirements.txt"),
    )
    .expect("requirements");
    // Each requirement starts at a `name==version` line and runs to the next; each has a SHA-256.
    let mut entries: Vec<String> = Vec::new();
    for line in requirements.lines() {
        let starts_entry = line
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && line.contains("==");
        if starts_entry {
            entries.push(line.to_owned());
        } else if let Some(last) = entries.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    assert!(entries.len() >= 5, "{} requirements", entries.len());
    assert!(
        entries
            .iter()
            .any(|entry| entry.starts_with("copr-cli==2.6 ")),
        "copr-cli==2.6 is not pinned"
    );
    for entry in &entries {
        assert!(
            entry.contains("--hash=sha256:"),
            "no hash for: {}",
            entry.lines().next().unwrap_or("")
        );
    }
}

// --- dependabot-security-fix.sh, with a stub `gh` ----------------------------------------------------
//
// The script decides whether a Dependabot pull request fixes an open security alert (#494). It is
// run for real; only `gh` is a stub that logs its arguments and answers from a file, so the real
// GitHub is never reached. The fixture alerts have the shape of alert 35 of this repository
// (`postcss-selector-parser`, `website/package-lock.json`, `< 6.1.5`, patched `6.1.5`).

const SECURITY_GH_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$STUB_GH_LOG"
case "$1 $2" in
  "api --paginate") ;;
  *) echo "stub gh: unexpected call: $*" >&2; exit 98 ;;
esac
if [ -n "$STUB_GH_EXIT" ]; then echo "gh: HTTP 403: Resource not accessible by personal access token" >&2; exit "$STUB_GH_EXIT"; fi
cat "$STUB_GH_REPLY"
"#;

#[derive(Clone)]
struct SecurityCase {
    token: &'static str,
    names: &'static str,
    group: &'static str,
    ecosystem: &'static str,
    directory: &'static str,
    previous: &'static str,
    new: &'static str,
}

impl SecurityCase {
    /// The real case: `postcss-selector-parser` 6.1.4 to 6.1.5 in `/website`.
    fn base() -> Self {
        Self {
            token: "token",
            names: "postcss-selector-parser",
            group: "",
            ecosystem: "npm_and_yarn",
            directory: "/website",
            previous: "6.1.4",
            new: "6.1.5",
        }
    }
}

struct SecurityRun {
    output: std::process::Output,
    github_output: String,
    calls: Vec<String>,
}

fn alert(
    number: u64,
    state: &str,
    name: &str,
    manifest: &str,
    range: &str,
    patched: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "state": state,
        "dependency": {
            "package": { "ecosystem": "npm", "name": name },
            "manifest_path": manifest,
            "scope": "runtime",
            "relationship": "transitive",
        },
        "security_advisory": { "ghsa_id": "GHSA-xxxx-xxxx-xxxx", "severity": "medium" },
        "security_vulnerability": {
            "package": { "ecosystem": "npm", "name": name },
            "severity": "medium",
            "vulnerable_version_range": range,
            "first_patched_version": patched.map(|p| serde_json::json!({ "identifier": p })),
        },
    })
}

/// Alert 35 as GitHub returned it.
fn alert_35() -> serde_json::Value {
    alert(
        35,
        "open",
        "postcss-selector-parser",
        "website/package-lock.json",
        "< 6.1.5",
        Some("6.1.5"),
    )
}

fn security_run_raw(case: &SecurityCase, reply: &str, gh_exit: Option<i32>) -> SecurityRun {
    let directory = tempdir().expect("tempdir");
    let stubs = directory.path().join("stubs");
    write_executable(&stubs.join("gh"), SECURITY_GH_STUB);
    let reply_file = directory.path().join("reply.json");
    fs::write(&reply_file, reply).expect("reply");
    let log = directory.path().join("gh.log");
    let github_output = directory.path().join("github-output");
    fs::write(&github_output, "").expect("output");
    let mut command = common::sandboxed("bash", directory.path());
    let mut path = std::ffi::OsString::from(&stubs);
    path.push(":");
    path.push(common::path_with_stubs(
        &std::env::var("PATH").expect("PATH"),
    ));
    command
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".github/scripts/dependabot-security-fix.sh"),
        )
        .env("PATH", path)
        .env("GH_TOKEN", case.token)
        .env("GITHUB_TOKEN", "")
        .env("GH_CONFIG_DIR", directory.path().join("gh-config"))
        .env("GH_HOST", "gh.invalid")
        .env("REPOSITORY", "o/r")
        .env("ECOSYSTEM", case.ecosystem)
        .env("DIRECTORY", case.directory)
        .env("DEP_NAMES", case.names)
        .env("DEP_GROUP", case.group)
        .env("PREVIOUS", case.previous)
        .env("NEW", case.new)
        .env("GITHUB_OUTPUT", &github_output)
        .env("STUB_GH_LOG", &log)
        .env("STUB_GH_REPLY", &reply_file);
    if let Some(code) = gh_exit {
        command.env("STUB_GH_EXIT", code.to_string());
    }
    let output = common::run_other_tool(&mut command);
    let hits = common::stub_hits(&command);
    assert!(hits.is_empty(), "the test reached a refusing stub: {hits}");
    SecurityRun {
        output,
        github_output: fs::read_to_string(&github_output).expect("github output"),
        calls: fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
    }
}

/// One JSON array per page, as `gh api --paginate` prints them.
fn security_run(case: &SecurityCase, pages: &[Vec<serde_json::Value>]) -> SecurityRun {
    let reply: Vec<String> = pages
        .iter()
        .map(|page| serde_json::Value::Array(page.clone()).to_string())
        .collect();
    security_run_raw(case, &reply.join("\n"), None)
}

fn security_stdout(run: &SecurityRun) -> String {
    String::from_utf8_lossy(&run.output.stdout).into_owned()
}

/// Asserts a clean exit that wrote exactly `fix=<expected>`.
fn assert_fix(run: &SecurityRun, expected: bool) {
    assert!(run.output.status.success(), "{:?}", run.output);
    assert_eq!(
        run.github_output,
        format!("fix={expected}\n"),
        "{}",
        security_stdout(run)
    );
}

/// `alert_35` with one field replaced, for the "is it still checked" cases.
fn security_decides(case: &SecurityCase, alerts: Vec<serde_json::Value>) -> bool {
    let run = security_run(case, &[alerts]);
    assert!(run.output.status.success(), "{:?}", run.output);
    match run.github_output.as_str() {
        "fix=true\n" => true,
        "fix=false\n" => false,
        other => panic!("unexpected output {other:?}"),
    }
}

#[test]
fn a_real_fix_arms_and_says_which_alert_matched() {
    let run = security_run(&SecurityCase::base(), &[vec![alert_35()]]);
    assert_fix(&run, true);
    assert!(
        security_stdout(&run).contains("alert 35: match"),
        "{}",
        security_stdout(&run)
    );
}

#[test]
fn a_minor_bump_above_the_patched_version_arms_when_the_old_version_was_vulnerable() {
    let case = SecurityCase {
        new: "6.2.0",
        ..SecurityCase::base()
    };
    assert_fix(&security_run(&case, &[vec![alert_35()]]), true);
}

#[test]
fn a_stale_open_alert_does_not_arm_a_bump_of_an_already_fixed_package() {
    // The alert stays open for about an hour after its fix merges: undici 8.12.3 is already past
    // the range, so 8.13.0 is an ordinary minor bump and needs review.
    let stale = alert(
        32,
        "open",
        "undici",
        "website/package-lock.json",
        "< 8.12.3",
        Some("8.12.3"),
    );
    let case = SecurityCase {
        names: "undici",
        previous: "8.12.3",
        new: "8.13.0",
        ..SecurityCase::base()
    };
    let run = security_run(&case, &[vec![stale]]);
    assert_fix(&run, false);
    assert!(security_stdout(&run).contains("not inside the vulnerable range"));
}

#[test]
fn a_new_version_below_the_patched_one_does_not_arm() {
    let case = SecurityCase {
        new: "6.1.4",
        previous: "6.1.3",
        ..SecurityCase::base()
    };
    let run = security_run(&case, &[vec![alert_35()]]);
    assert_fix(&run, false);
    // The new version is still inside the alert's own range.
    assert!(security_stdout(&run).contains("alert 35: blocks"));
}

#[test]
fn a_new_version_outside_the_range_but_below_the_patched_one_does_not_arm() {
    // `= 1.2.0` patched 1.3.0: 1.2.5 is not vulnerable by the range, but is not the patched version.
    let run = security_run(
        &foo_case("1.2.0", "1.2.5"),
        &[vec![foo_alert(1, "= 1.2.0", "1.3.0")]],
    );
    assert_fix(&run, false);
    assert!(security_stdout(&run).contains("below the patched version"));
}

#[test]
fn an_alert_with_no_patched_version_does_not_arm() {
    let alert = alert(
        35,
        "open",
        "postcss-selector-parser",
        "website/package-lock.json",
        "< 6.1.5",
        None,
    );
    assert_fix(&security_run(&SecurityCase::base(), &[vec![alert]]), false);
}

#[test]
fn more_than_one_dependency_or_a_group_never_reaches_the_api() {
    for case in [
        SecurityCase {
            names: "postcss-selector-parser,undici",
            ..SecurityCase::base()
        },
        SecurityCase {
            group: "npm-minor",
            ..SecurityCase::base()
        },
    ] {
        let run = security_run(&case, &[vec![alert_35()]]);
        assert_fix(&run, false);
        assert!(run.calls.is_empty(), "{:?}", run.calls);
    }
}

#[test]
fn the_servers_filter_is_not_trusted() {
    let case = SecurityCase::base();
    let other_package = alert(
        35,
        "open",
        "undici",
        "website/package-lock.json",
        "< 6.1.5",
        Some("6.1.5"),
    );
    let other_manifest = alert(
        35,
        "open",
        "postcss-selector-parser",
        "gui/package-lock.json",
        "< 6.1.5",
        Some("6.1.5"),
    );
    let mut fixed = alert_35();
    fixed["state"] = "fixed".into();
    let mut dismissed = alert_35();
    dismissed["state"] = "dismissed".into();
    for wrong in [other_package, other_manifest, fixed, dismissed] {
        assert!(!security_decides(&case, vec![wrong.clone()]), "{wrong}");
        // The matching alert beside it still arms: the wrong one is skipped, not fatal.
        assert!(security_decides(&case, vec![wrong, alert_35()]));
    }
    // The advisory's package name is checked too, not only the dependency's.
    let mut mismatched = alert_35();
    mismatched["security_vulnerability"]["package"]["name"] = "undici".into();
    assert!(!security_decides(&case, vec![mismatched]));
    let mut dependency_only = alert_35();
    dependency_only["dependency"]["package"]["name"] = "undici".into();
    assert!(!security_decides(&case, vec![dependency_only]));
}

#[test]
fn a_two_comparator_range_must_hold_on_both_sides() {
    let range = ">= 8.0.0, < 8.12.3";
    let alert = || {
        alert(
            32,
            "open",
            "undici",
            "website/package-lock.json",
            range,
            Some("8.12.3"),
        )
    };
    let base = SecurityCase {
        names: "undici",
        ..SecurityCase::base()
    };
    let at = |previous: &'static str| SecurityCase {
        previous,
        new: "8.13.0",
        ..base.clone()
    };
    assert!(security_decides(&at("8.0.0"), vec![alert()]));
    assert!(security_decides(&at("8.12.2"), vec![alert()]));
    assert!(
        !security_decides(&at("7.9.9"), vec![alert()]),
        "below the lower bound"
    );
    assert!(
        !security_decides(&at("8.12.3"), vec![alert()]),
        "at the upper bound"
    );
}

#[test]
fn a_prerelease_or_an_unknown_range_never_matches() {
    let case = SecurityCase::base();
    for (previous, new) in [
        ("6.1.4-beta.1", "6.1.5"),
        ("6.1.4", "6.2.0-rc.1"),
        ("6.1.4+x", "6.1.5"),
    ] {
        let case = SecurityCase {
            previous,
            new,
            ..case.clone()
        };
        let run = security_run(&case, &[vec![alert_35()]]);
        assert_fix(&run, false);
        assert!(
            run.calls.is_empty(),
            "a bad version is refused before the API: {:?}",
            run.calls
        );
    }
    let with = |range: &str, patched: &str| {
        alert(
            35,
            "open",
            "postcss-selector-parser",
            "website/package-lock.json",
            range,
            Some(patched),
        )
    };
    for (range, patched) in [
        ("< 6.1.5-beta", "6.1.5"),
        ("~> 6.1", "6.1.5"),
        ("< 6.1.5 || > 7", "6.1.5"),
        ("< 6.1.5,", "6.1.5"),
        ("", "6.1.5"),
        ("< 6.1.5", "6.1.5-rc.1"),
        ("< 6.1.5", ""),
    ] {
        assert!(
            !security_decides(&case, vec![with(range, patched)]),
            "{range:?} {patched:?}"
        );
    }
}

#[test]
fn versions_compare_numerically_with_missing_parts_as_zero() {
    // The range `= 1.2` holds for 1.2.0, and 1.10.0 is above a patched 1.9.0 (text order says no).
    let with = |range: &str, patched: &str| {
        alert(
            7,
            "open",
            "left-pad",
            "website/package-lock.json",
            range,
            Some(patched),
        )
    };
    let case = |previous: &'static str, new: &'static str| SecurityCase {
        names: "left-pad",
        previous,
        new,
        ..SecurityCase::base()
    };
    assert!(security_decides(
        &case("1.2.0", "1.10.0"),
        vec![with("= 1.2", "1.9.0")]
    ));
    assert!(security_decides(
        &case("1.2", "1.10.0"),
        vec![with("= 1.2.0", "1.9.0")]
    ));
    assert!(!security_decides(
        &case("1.2.0", "1.9.0"),
        vec![with("= 1.2", "1.10.0")]
    ));
    assert!(security_decides(
        &case("1.2.0", "1.03.0"),
        vec![with("<= 1.2", "1.02.0")]
    ));
}

#[test]
fn a_failed_lookup_fails_the_job_and_never_reads_as_no_alerts() {
    let run = security_run_raw(&SecurityCase::base(), "[]", Some(1));
    assert!(!run.output.status.success(), "{:?}", run.output);
    assert!(
        !run.github_output.contains("fix=true"),
        "{}",
        run.github_output
    );
    assert!(
        String::from_utf8_lossy(&run.output.stderr).contains("refusing to treat"),
        "{:?}",
        run.output
    );
}

#[test]
fn a_reply_that_is_not_a_list_of_alerts_fails_the_job() {
    for reply in [
        "<html>bad gateway</html>",
        "{\"message\":\"Not Found\"}",
        "",
        "[1,",
        "\"x\"",
    ] {
        let run = security_run_raw(&SecurityCase::base(), reply, None);
        assert!(!run.output.status.success(), "{reply:?}: {:?}", run.output);
        assert!(!run.github_output.contains("fix=true"), "{reply:?}");
    }
}

#[test]
fn an_empty_token_says_fix_false_without_calling_gh() {
    let case = SecurityCase {
        token: "",
        ..SecurityCase::base()
    };
    let run = security_run(&case, &[vec![alert_35()]]);
    assert_fix(&run, false);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
    assert!(security_stdout(&run).contains("DEPENDABOT_ALERTS_TOKEN"));
}

#[test]
fn the_manifest_comes_from_the_ecosystem_and_directory_and_the_query_is_encoded() {
    let query = |case: &SecurityCase| {
        let run = security_run(case, &[vec![]]);
        assert_fix(&run, false);
        assert_eq!(run.calls.len(), 1, "{:?}", run.calls);
        run.calls[0].clone()
    };
    assert_eq!(
        query(&SecurityCase::base()),
        "api --paginate repos/o/r/dependabot/alerts?state=open&package=postcss-selector-parser&manifest=website%2Fpackage-lock.json&per_page=100"
    );
    let root = SecurityCase {
        ecosystem: "cargo",
        directory: "/",
        names: "serde",
        ..SecurityCase::base()
    };
    assert!(query(&root).contains("&package=serde&manifest=Cargo.lock&"));
    let gui = SecurityCase {
        directory: "/gui/",
        names: "@scope/pkg",
        ..SecurityCase::base()
    };
    assert!(query(&gui).contains("&package=%40scope%2Fpkg&manifest=gui%2Fpackage-lock.json&"));
}

#[test]
fn an_unknown_ecosystem_a_bad_directory_or_a_bad_name_never_reaches_the_api() {
    for case in [
        SecurityCase {
            ecosystem: "github_actions",
            directory: "/",
            ..SecurityCase::base()
        },
        SecurityCase {
            directory: "/../etc",
            ..SecurityCase::base()
        },
        SecurityCase {
            directory: "website",
            ..SecurityCase::base()
        },
        SecurityCase {
            names: "bad name&state=all",
            ..SecurityCase::base()
        },
    ] {
        let run = security_run(&case, &[vec![alert_35()]]);
        assert_fix(&run, false);
        assert!(run.calls.is_empty(), "{:?}", run.calls);
    }
}

#[test]
fn a_matching_alert_on_a_later_page_is_found() {
    let filler = alert(
        30,
        "open",
        "other",
        "website/package-lock.json",
        "< 1.0.0",
        Some("1.0.0"),
    );
    let run = security_run(&SecurityCase::base(), &[vec![filler], vec![alert_35()]]);
    assert_fix(&run, true);
    assert!(security_stdout(&run).contains("alert 35: match"));
}

/// An open `foo` alert in the website manifest.
fn foo_alert(number: u64, range: &str, patched: &str) -> serde_json::Value {
    alert(
        number,
        "open",
        "foo",
        "website/package-lock.json",
        range,
        Some(patched),
    )
}

fn foo_case(previous: &'static str, new: &'static str) -> SecurityCase {
    SecurityCase {
        names: "foo",
        previous,
        new,
        ..SecurityCase::base()
    }
}

#[test]
fn the_new_version_must_share_a_compatibility_line_with_the_patched_one() {
    // (previous, new, range, patched, arms): the line is the patched version's leading components
    // up to and including its first non-zero one.
    let table: [(&str, &str, &str, &str, bool); 14] = [
        ("1.2.0", "3.0.0", "< 1.2.5", "1.2.5", false),
        ("1.2.0", "1.9.0", "< 1.2.5", "1.2.5", true),
        ("1.2.4", "1.2.5", "< 1.2.5", "1.2.5", true),
        ("0.3.0", "0.5.0", "< 0.3.2", "0.3.2", false),
        ("0.3.0", "0.3.9", "< 0.3.2", "0.3.2", true),
        ("0.0.2", "0.0.4", "< 0.0.3", "0.0.3", false),
        ("0.0.2", "0.0.3", "< 0.0.3", "0.0.3", true),
        // A fix that needs a major still matches.
        ("1.9.0", "2.0.1", "< 2.0.0", "2.0.0", true),
        ("1.9.0", "2.0.0", "< 2.0.0", "2.0.0", true),
        // Real cases of this repository.
        ("8.11.2", "8.12.3", "< 8.12.2", "8.12.2", true),
        ("6.1.4", "6.1.5", "< 6.1.5", "6.1.5", true),
        // Missing components are zero.
        ("1.2", "1.4", "< 1.3", "1.3", true),
        ("0.0.0", "0.0.1", "< 0.0.1", "0.0.1", true),
        ("1.2.0", "2", "< 1.2.5", "1.2.5", false),
    ];
    for (previous, new, range, patched, arms) in table {
        assert_eq!(
            security_decides(&foo_case(previous, new), vec![foo_alert(1, range, patched)]),
            arms,
            "{previous} -> {new}, {range}, patched {patched}"
        );
    }
}

#[test]
fn a_new_version_inside_another_open_alerts_range_does_not_arm() {
    let first = foo_alert(1, ">= 1.0, < 1.5", "1.5.0");
    let second = foo_alert(2, ">= 1.6, < 1.9", "1.9.0");
    // The case of the review: a bump across two lines is refused (by either check).
    let far = foo_case("1.2.0", "2.1.0");
    let high = foo_alert(3, ">= 2.0, < 2.5", "2.5.0");
    assert!(!security_decides(&far, vec![first.clone(), high]));
    let case = foo_case("1.2.0", "1.7.0");
    // Alone, the first alert is fixed by the bump.
    assert!(security_decides(&case, vec![first.clone()]));
    let run = security_run(&case, &[vec![first.clone(), second.clone()]]);
    assert_fix(&run, false);
    assert!(
        security_stdout(&run).contains("alert 2: blocks"),
        "{}",
        security_stdout(&run)
    );
    // The order and the page do not matter.
    let run = security_run(&case, &[vec![second.clone()], vec![first.clone()]]);
    assert_fix(&run, false);
    // An alert that fails the state, name or manifest checks is not another open alert.
    let mut closed = second.clone();
    closed["state"] = "fixed".into();
    let mut other_manifest = second.clone();
    other_manifest["dependency"]["manifest_path"] = "gui/package-lock.json".into();
    let mut other_name = second;
    other_name["dependency"]["package"]["name"] = "bar".into();
    assert!(security_decides(
        &case,
        vec![first, closed, other_manifest, other_name]
    ));
}

#[test]
fn an_unreadable_range_on_another_open_alert_blocks() {
    let first = foo_alert(1, ">= 1.0, < 1.5", "1.5.0");
    let case = foo_case("1.2.0", "1.7.0");
    for range in [
        "~> 1.6",
        "",
        "< 1.9 || > 3",
        ">= 1.6, < 1.9-beta",
        "> 5, < 1.9-beta",
        "< 1.9,",
    ] {
        let run = security_run(&case, &[vec![first.clone(), foo_alert(2, range, "1.9.0")]]);
        assert_fix(&run, false);
        assert!(
            security_stdout(&run).contains("alert 2: blocks"),
            "{range:?}: {}",
            security_stdout(&run)
        );
    }
    // A range that is readable and does not contain the new version does not block.
    assert!(security_decides(
        &case,
        vec![first, foo_alert(2, ">= 1.8, < 1.9", "1.9.0")]
    ));
}

#[test]
fn a_new_version_that_is_not_above_the_previous_one_does_not_arm() {
    // 1.2.0 -> 1.1.0 would fix `>= 1.0, < 1.5` (patched 1.0.5) on every other check.
    let alert = || foo_alert(1, ">= 1.0, < 1.5", "1.0.5");
    let downgrade = security_run(&foo_case("1.2.0", "1.1.0"), &[vec![alert()]]);
    assert_fix(&downgrade, false);
    assert!(downgrade.calls.is_empty(), "{:?}", downgrade.calls);
    let same = security_run(&foo_case("1.2.0", "1.2.0"), &[vec![alert()]]);
    assert_fix(&same, false);
    assert!(same.calls.is_empty(), "{:?}", same.calls);
    assert!(security_decides(&foo_case("1.2.0", "1.5.0"), vec![alert()]));
}

#[test]
fn a_root_directory_spelled_with_slashes_only_is_not_the_root() {
    // `//` would reduce to the root manifest, and so to the root's alerts.
    let root_alert = alert(9, "open", "serde", "Cargo.lock", "< 1.0.5", Some("1.0.5"));
    for directory in ["/", "//", "///"] {
        let case = SecurityCase {
            ecosystem: "cargo",
            directory,
            names: "serde",
            previous: "1.0.4",
            new: "1.0.5",
            ..SecurityCase::base()
        };
        let run = security_run(&case, &[vec![root_alert.clone()]]);
        assert_fix(&run, directory == "/");
        assert_eq!(run.calls.is_empty(), directory != "/", "{directory}");
    }
}

#[test]
fn a_version_component_too_long_for_bash_arithmetic_is_refused_before_the_api() {
    let alert = || foo_alert(1, "< 6.1.5", "6.1.5");
    let long = |new: &'static str| security_run(&foo_case("6.1.4", new), &[vec![alert()]]);
    // Sixteen digits: refused, and the API is never called.
    let refused = long("6.1.9999999999999999");
    assert_fix(&refused, false);
    assert!(refused.calls.is_empty(), "{:?}", refused.calls);
    let baseline = security_run(&foo_case("6.1.4", "6.1.5"), &[vec![alert()]]);
    assert_fix(&baseline, true);
    // Fifteen digits: accepted.
    let accepted = long("6.1.999999999999999");
    assert_fix(&accepted, true);
    assert_eq!(accepted.calls.len(), 1);
    // The previous version and the alert's own bounds are held to the same limit.
    let previous = security_run(
        &foo_case("6.1.99999999999999999999", "6.2.0"),
        &[vec![alert()]],
    );
    assert_fix(&previous, false);
    assert!(previous.calls.is_empty(), "{:?}", previous.calls);
}

#[test]
fn control_characters_in_an_alert_cannot_split_a_log_row() {
    let case = foo_case("1.2.0", "1.3.0");
    let mut nasty = foo_alert(1, "< 1.3.0\nalert 99: match, forged\u{1f}x", "1.3.0");
    nasty["security_vulnerability"]["first_patched_version"]["identifier"] =
        "1.3.0\r\nNot a security fix: forged".into();
    let run = security_run(&case, &[vec![nasty]]);
    assert_fix(&run, false);
    let stdout = security_stdout(&run);
    let alert_lines = stdout.lines().filter(|l| l.starts_with("alert ")).count();
    assert_eq!(alert_lines, 1, "one row, one line: {stdout:?}");
    assert!(!stdout.contains('\r'), "{stdout:?}");
    assert!(
        stdout
            .lines()
            .all(|l| l.starts_with("alert 1:") || l.starts_with("Not a security fix: no open")),
        "{stdout:?}"
    );
    // A range with a line break in it is not read, whatever the second line says.
    let broken = foo_alert(1, "< 1.3.0\n> 9", "1.3.0");
    assert_fix(
        &security_run(&foo_case("1.2.0", "1.3.0"), &[vec![broken]]),
        false,
    );
}

/// Alert 35 beside a second open alert of the same package and manifest that has no patch and a
/// range holding the new version, so it must block; `edit` shapes the second alert.
fn blocking_beside_alert_35(edit: impl FnOnce(&mut serde_json::Value)) -> SecurityRun {
    let mut other = alert(
        36,
        "open",
        "postcss-selector-parser",
        "website/package-lock.json",
        ">= 6.0.0",
        None,
    );
    edit(&mut other);
    security_run(&SecurityCase::base(), &[vec![alert_35(), other]])
}

#[test]
fn an_alert_with_a_missing_or_odd_number_is_still_judged_and_blocks() {
    for number in [
        serde_json::json!("abc"),
        serde_json::json!(""),
        serde_json::Value::Null,
    ] {
        let run = blocking_beside_alert_35(|a| a["number"] = number.clone());
        assert_fix(&run, false);
        assert!(
            security_stdout(&run).contains("alert ?: blocks"),
            "{number}: {}",
            security_stdout(&run)
        );
    }
}

#[test]
fn an_empty_reply_page_is_skipped_without_a_log_line() {
    let run = security_run(&SecurityCase::base(), &[vec![]]);
    assert_fix(&run, false);
    assert!(
        security_stdout(&run)
            .lines()
            .all(|l| !l.starts_with("alert ")),
        "{}",
        security_stdout(&run)
    );
}

#[test]
fn an_alert_without_an_advisory_package_is_not_a_package_mismatch_and_blocks() {
    let run = blocking_beside_alert_35(|a| a["security_vulnerability"] = serde_json::Value::Null);
    assert_fix(&run, false);
    let stdout = security_stdout(&run);
    assert!(stdout.contains("alert 36: blocks"), "{stdout}");
    assert!(!stdout.contains("package is not this one"), "{stdout}");
    // A present, different advisory name is still a mismatch and does not block.
    let run = blocking_beside_alert_35(|a| {
        a["security_vulnerability"]["package"]["name"] = "undici".into();
    });
    assert_fix(&run, true);
}

#[test]
fn a_leading_zero_component_is_decimal_in_the_version_compare() {
    // 08 and 09 are not valid octal: without the 10# prefix bash stops with an arithmetic error.
    let run = security_run(
        &foo_case("1.08.0", "1.10.0"),
        &[vec![foo_alert(1, "< 1.10.0", "1.10.0")]],
    );
    assert_fix(&run, true);
    let run = security_run(
        &foo_case("1.07.0", "1.09.0"),
        &[vec![foo_alert(1, "< 1.09.0", "1.09.0")]],
    );
    assert_fix(&run, true);
}

#[test]
fn a_leading_zero_component_is_decimal_in_the_compatibility_line_check() {
    // 010 is 10, not octal 8. The version compare is right either way here (new equals the patched
    // version), so only the line check can tell: it must see 010 equal to 10 and share the line.
    let run = security_run(
        &foo_case("0.9.0", "0.010.0"),
        &[vec![foo_alert(1, "< 0.10.0", "0.10.0")]],
    );
    assert_fix(&run, true);
}

// --- dependabot-auto-merge.yml, as text -------------------------------------------------------------

fn auto_merge_workflow() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/dependabot-auto-merge.yml"),
    )
    .expect("dependabot-auto-merge.yml")
}

/// The workflow without whole-line comments.
fn without_comments(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text of the job `name` (two-space indent under `jobs:`), to the next job or the end.
fn workflow_job(text: &str, name: &str) -> String {
    let start = text
        .find(&format!("\n  {name}:\n"))
        .unwrap_or_else(|| panic!("no job {name}"));
    let rest = &text[start + 1..];
    let end = rest
        .match_indices("\n  ")
        .find(|(at, _)| {
            let line = &rest[at + 1..];
            !line.starts_with("   ")
                && line
                    .split('\n')
                    .next()
                    .is_some_and(|l| l.trim_end().ends_with(':'))
        })
        .map_or(rest.len(), |(at, _)| at);
    rest[..end].to_owned()
}

/// The text of the step whose `- name:` is `name`, in `text` (a workflow or one job).
fn workflow_step(text: &str, name: &str) -> String {
    let start = text
        .find(&format!("      - name: {name}\n"))
        .unwrap_or_else(|| panic!("no step {name}"));
    let rest = &text[start + 1..];
    let end = rest.find("\n      - name: ").unwrap_or(rest.len());
    rest[..end].to_owned()
}

#[test]
fn the_alerts_token_reaches_only_the_lookup_step() {
    let text = without_comments(&auto_merge_workflow());
    assert_eq!(text.matches("DEPENDABOT_ALERTS_TOKEN").count(), 1, "{text}");
    let decide = workflow_job(&text, "decide");
    assert!(
        workflow_step(&decide, "Look for a security fix")
            .contains("GH_TOKEN: ${{ secrets.DEPENDABOT_ALERTS_TOKEN }}")
    );
    assert!(!text.contains("alert-lookup"));
    assert!(!text.contains("github-token"));
    assert!(!workflow_step(&decide, "Fetch Dependabot metadata").contains("with:"));
}

#[test]
fn no_expression_is_written_into_a_run_block() {
    let text = without_comments(&auto_merge_workflow());
    let lines: Vec<&str> = text.lines().collect();
    let mut runs = 0;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("run:") else {
            continue;
        };
        runs += 1;
        let indent = line.len() - trimmed.len();
        assert!(!rest.contains("${{"), "{line}");
        for next in &lines[index + 1..] {
            let next_indent = next.len() - next.trim_start().len();
            if !next.trim().is_empty() && next_indent <= indent {
                break;
            }
            assert!(!next.contains("${{"), "an expression inside run: {next}");
        }
    }
    assert_eq!(runs, 2, "the lookup and the merge");
}

#[test]
fn the_decision_script_comes_from_the_base_commit_without_credentials() {
    let text = without_comments(&auto_merge_workflow());
    let decide = workflow_job(&text, "decide");
    let checkout = workflow_step(
        &decide,
        "Check out the decision script from the base commit",
    );
    assert!(
        checkout.contains("persist-credentials: false"),
        "{checkout}"
    );
    assert!(
        checkout.contains("ref: ${{ github.event.pull_request.base.sha }}"),
        "{checkout}"
    );
    assert!(checkout.contains("uses: actions/checkout@"), "{checkout}");
    // The script is run from that checkout.
    assert!(decide.contains("run: bash .github/scripts/dependabot-security-fix.sh"));
}

#[test]
fn the_decision_job_reads_and_only_the_merge_job_writes() {
    let text = without_comments(&auto_merge_workflow());
    let decide = workflow_job(&text, "decide");
    let merge = workflow_job(&text, "auto-merge");
    assert!(
        decide.contains("permissions:\n      contents: read\n      pull-requests: read\n"),
        "{decide}"
    );
    assert!(!decide.contains("write"), "{decide}");
    assert!(
        merge.contains("permissions:\n      contents: write\n      pull-requests: write\n"),
        "{merge}"
    );
    // The write rights appear nowhere else: not in the workflow default either.
    assert_eq!(text.matches("write").count(), 2, "{text}");
    assert!(text.contains("\npermissions:\n  contents: read\n"));
}

#[test]
fn the_merge_job_runs_no_repository_code_and_holds_no_alerts_token() {
    let text = without_comments(&auto_merge_workflow());
    let merge = workflow_job(&text, "auto-merge");
    assert!(!merge.contains("checkout"), "{merge}");
    assert!(!merge.contains("DEPENDABOT_ALERTS_TOKEN"), "{merge}");
    assert!(!merge.contains("uses:"), "{merge}");
    assert_eq!(merge.matches("run:").count(), 1, "{merge}");
    assert!(merge.contains("gh pr merge --auto --squash \"$PR_URL\""));
    assert!(merge.contains("GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}"));
}

#[test]
fn the_merge_waits_for_the_decision_and_a_patch_bump_still_arms() {
    let text = without_comments(&auto_merge_workflow());
    let decide = workflow_job(&text, "decide");
    let merge = workflow_job(&text, "auto-merge");
    // A failed or skipped `decide` skips the merge: the job `if:` has no status function, so
    // GitHub adds the implicit success(), which includes every job in `needs`.
    assert!(merge.contains("needs: decide\n"), "{merge}");
    assert!(
        merge.contains("if: needs.decide.outputs.arm == 'true'\n"),
        "{merge}"
    );
    for status in ["always()", "failure()", "cancelled()", "success()"] {
        assert!(!merge.contains(status), "{status}: {merge}");
    }
    // The output is the lookup's fix OR a patch bump. The lookup is skipped on a patch bump, so
    // its output is empty there and the second operand alone arms.
    assert!(
        decide.contains(
            "arm: ${{ steps.security.outputs.fix == 'true' || steps.meta.outputs.update-type == 'version-update:semver-patch' }}"
        ),
        "{decide}"
    );
    let lookup = workflow_step(&decide, "Look for a security fix");
    assert!(lookup.contains("if: steps.meta.outputs.update-type != 'version-update:semver-patch'"));
    // Only Dependabot's own PRs reach the decision, and so the merge.
    assert!(decide.contains(
        "if: github.actor == 'dependabot[bot]' && github.event.pull_request.user.login == 'dependabot[bot]'"
    ));
    assert!(!merge.contains("github.actor"));
}
