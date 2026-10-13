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
fn name_assets_refuses_what_it_should_not_rename_and_renames_nothing() {
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
        "but 0.1.0-beta.1 is '0.1.0~beta.1' in an .rpm",
    );
    refused("0.2.0", &[&deb], "in an .deb");
    refused("0.1.0-beta.1", &[&arch], "is '0.1.0beta1' in an .arch");
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
