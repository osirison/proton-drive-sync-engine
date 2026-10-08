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
fn uninstall_never_lists_a_dot_sync_directly_under_home_or_an_unsafe_root() {
    // PR #434 second review, H4. The guard in `validated_sync_state_dir` that refuses `/` and `$HOME`
    // as a pair root had no test: removing it still passed, and a hand-edited `local_root = "~"`
    // (or the absolute home directory) with a `.sync` in it would then be planned for removal. A
    // dry run is enough to prove it, and nothing is removed.
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
        home.join(".sync").join("precious").exists(),
        "and a dry run removed nothing"
    );
}
