//! The apt channel (openspec change add-linux-release-pipeline, task 6.3, design D10, D11):
//! `version.sh sort-key`, `apt-client.sh` (the one definition of the text a user types) and
//! `apt-publish.sh`, in publish mode and in `--local` mode.
//!
//! Every script runs on temporary files with a stub `gh` (a releases listing and per-tag assets), a
//! stub `apt-ftparchive` (unless a test is about the real one), and a LOCAL bare repository in place
//! of the apt repository. Signing keys are throwaway Ed25519 keys made here. No network, no real
//! `gh`, no key of the user's.
use super::*;
use std::process::Output;

// --- tools a developer's machine may lack ---------------------------------------------------------

pub(super) fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            fs::metadata(directory.join(tool))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    })
}

/// True when every tool is on `PATH`. When one is missing the test FAILS if `CI` is set (a check that
/// skips on the runner is a hole in the gate; ci.yml's `rust` job installs them) and otherwise
/// prints why it is skipping.
pub(super) fn have_tools(test: &str, tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools
        .iter()
        .copied()
        .filter(|tool| !on_path(tool))
        .collect();
    let ci = std::env::var_os("CI").is_some_and(|value| !value.is_empty());
    match tool_verdict(test, &missing, ci) {
        Ok(go_on) => {
            if !go_on {
                eprintln!(
                    "SKIPPED {test}: {missing:?} not on PATH (this test fails when CI is set)"
                );
            }
            go_on
        }
        Err(message) => panic!("{message}"),
    }
}

/// What a test does about missing tools: `Ok(true)` goes on, `Ok(false)` skips, `Err` fails.
fn tool_verdict(test: &str, missing: &[&str], ci: bool) -> Result<bool, String> {
    if missing.is_empty() {
        Ok(true)
    } else if ci {
        Err(format!(
            "{test}: {missing:?} not on PATH and CI is set: install them in ci.yml's rust job"
        ))
    } else {
        Ok(false)
    }
}

#[test]
fn missing_tools_skip_a_test_on_a_developers_machine_and_fail_it_in_ci() {
    assert_eq!(tool_verdict("t", &[], false), Ok(true));
    assert_eq!(tool_verdict("t", &[], true), Ok(true));
    assert_eq!(tool_verdict("t", &["apt-ftparchive"], false), Ok(false));
    let failure = tool_verdict("t", &["apt-ftparchive"], true).expect_err("CI must fail");
    assert!(
        failure.contains("apt-ftparchive") && failure.contains("CI is set"),
        "{failure}"
    );
    // Nothing in the workflow's job list was missed: the runner has what the tests ask for.
    let ci = workflow("ci.yml");
    let rust = job_block(&ci, "rust");
    assert!(
        rust.contains("      - name: Install apt-utils\n        run: sudo apt-get install -y --no-install-recommends apt-utils\n"),
        "ci.yml's rust job must install apt-utils, or the real-apt-ftparchive tests fail there"
    );
    let install = rust.find("Install apt-utils").expect("step");
    assert!(install < rust.find("name: Test").expect("test step"));
}

// --- version.sh sort-key against dpkg -------------------------------------------------------------

/// In strictly ascending version order.
const VERSION_CORPUS: [&str; 29] = [
    "0.0.1",
    "0.1.0-alpha.1",
    "0.1.0-alpha.2",
    "0.1.0-alpha.10",
    "0.1.0-beta.1",
    "0.1.0-beta.2",
    "0.1.0-beta.9",
    "0.1.0-beta.10",
    "0.1.0-beta.11",
    "0.1.0-rc.1",
    "0.1.0-rc.2",
    "0.1.0-rc.10",
    "0.1.0",
    "0.1.1-alpha.1",
    "0.1.1",
    "0.2.0-beta.1",
    "0.2.0",
    "0.9.0",
    "0.10.0",
    "0.10.1",
    "1.0.0-alpha.1",
    "1.0.0-rc.1",
    "1.0.0",
    "1.9.9",
    "1.10.0",
    "2.0.0",
    "10.0.0",
    "10.2.3-rc.12",
    "10.2.3",
];

fn version_line(directory: &Path, args: &[&str]) -> String {
    let output = version_script(Path::new(env!("CARGO_MANIFEST_DIR")), directory, None, args);
    assert!(output.status.success(), "{args:?}: {}", stderr_of(&output));
    stdout_of(&output).trim().to_owned()
}

#[test]
fn sort_key_gives_the_order_dpkg_gives_the_deb_spellings() {
    if !have_tools("sort_key", &["dpkg"]) {
        return;
    }
    let directory = tempdir().expect("tempdir");
    let keyed: Vec<(String, String)> = VERSION_CORPUS
        .iter()
        .map(|version| {
            (
                version_line(directory.path(), &["sort-key", version]),
                version_line(directory.path(), &["spell", "deb", version]),
            )
        })
        .collect();
    // The keys are strictly ascending as plain text (the script's callers sort with LC_ALL=C).
    for pair in keyed.windows(2) {
        assert!(pair[0].0 < pair[1].0, "{pair:?}");
    }
    // Every pair, not just neighbours: dpkg agrees that the earlier deb version is the lesser.
    for (at, (_, earlier)) in keyed.iter().enumerate() {
        for (_, later) in &keyed[at + 1..] {
            let mut command = common::sandboxed("dpkg", directory.path());
            command.args(["--compare-versions", earlier, "lt", later]);
            let output = common::run_other_tool(&mut command);
            assert!(
                output.status.success(),
                "dpkg does not order {earlier} below {later}"
            );
        }
    }
    // Shuffled input sorts back to the corpus, by key alone.
    let mut shuffled: Vec<(String, &str)> = keyed
        .iter()
        .zip(VERSION_CORPUS)
        .rev()
        .map(|((key, _), version)| (key.clone(), version))
        .collect();
    shuffled.sort();
    let sorted: Vec<&str> = shuffled.iter().map(|(_, version)| *version).collect();
    assert_eq!(sorted, VERSION_CORPUS);
}

#[test]
fn the_deb_spelling_is_the_rpm_spelling_and_a_revision_because_verify_apt_depends_on_it() {
    let directory = tempdir().expect("tempdir");
    for version in VERSION_CORPUS {
        let rpm = version_line(directory.path(), &["spell", "rpm", version]);
        let deb = version_line(directory.path(), &["spell", "deb", version]);
        assert_eq!(deb, format!("{rpm}-1"), "{version}");
    }
}

#[test]
fn sort_key_refuses_what_is_not_a_version_and_numbers_too_long_to_pad() {
    let directory = tempdir().expect("tempdir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for bad in [
        "",
        "v0.1.0",
        "0.1",
        "0.1.0-beta",
        "0.1.0-beta.0",
        "0.1.0-snapshot.1",
        "01.0.0",
        "1234567890.0.0",
        "0.1.0-beta.1234567890",
    ] {
        let output = version_script(root, directory.path(), None, &["sort-key", bad]);
        assert_eq!(output.status.code(), Some(1), "'{bad}'");
        assert!(stdout_of(&output).is_empty(), "'{bad}'");
    }
}

// --- apt-client.sh --------------------------------------------------------------------------------

/// Design D10, "Users install it with" (shown with `beta`), word for word.
const D10_BETA: &str = "sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://osirison.github.io/proton-drive-sync-engine-apt/proton-drive-sync-engine.gpg \\
  | sudo tee /etc/apt/keyrings/proton-drive-sync-engine.gpg >/dev/null
sudo tee /etc/apt/sources.list.d/proton-drive-sync-engine.sources >/dev/null <<'EOF'
Types: deb
URIs: https://osirison.github.io/proton-drive-sync-engine-apt/
Suites: beta
Components: main
Architectures: amd64
Signed-By: /etc/apt/keyrings/proton-drive-sync-engine.gpg
EOF
sudo apt update && sudo apt install proton-drive-sync-engine
";

fn client(args: &[&str], environment: &[(&str, &str)]) -> Output {
    let directory = tempdir().expect("tempdir");
    let mut command = common::sandboxed("bash", directory.path());
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release/apt-client.sh"))
        .args(args)
        .env_remove("APT_FINGERPRINT_FILE")
        .env_remove("APT_CLIENT_ROOT");
    for (name, value) in environment {
        command.env(name, value);
    }
    common::run_other_tool(&mut command)
}

#[test]
fn the_text_a_user_types_is_the_text_the_design_shows_and_the_design_file_agrees() {
    let output = client(&["print", "beta"], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), D10_BETA);
    // The same words in the design, while the change is still unarchived there.
    let design = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("openspec/changes/add-linux-release-pipeline/design.md");
    if let Ok(design) = fs::read_to_string(design) {
        assert!(
            design.contains(&format!("```sh\n{D10_BETA}```")),
            "design D10 and the pinned text differ"
        );
    }
    // Stable changes one line: the suite.
    let stable = client(&["print", "stable"], &[]);
    assert_eq!(
        stdout_of(&stable),
        D10_BETA.replace("Suites: beta", "Suites: stable")
    );
}

#[test]
fn the_text_options_serve_the_jobs_that_run_it_elsewhere() {
    let output = client(
        &[
            "print",
            "stable",
            "--uri",
            "http://127.0.0.1:8080/",
            "--no-sudo",
            "--no-install",
        ],
        &[],
    );
    let text = stdout_of(&output);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(!text.contains("sudo"), "{text}");
    assert!(!text.contains("apt update"), "{text}");
    assert!(
        text.contains("curl -fsSL http://127.0.0.1:8080/proton-drive-sync-engine.gpg \\\n"),
        "{text}"
    );
    assert!(text.contains("URIs: http://127.0.0.1:8080/\n"), "{text}");
    assert!(text.contains("Suites: stable\n"), "{text}");
    assert!(text.ends_with("EOF\n"), "{text}");
    // An explicit key URL wins over the one derived from the repository's.
    let output = client(
        &[
            "print",
            "beta",
            "--uri",
            "http://h/",
            "--key-url",
            "http://k/key.gpg",
        ],
        &[],
    );
    let text = stdout_of(&output);
    assert!(text.contains("curl -fsSL http://k/key.gpg \\\n"), "{text}");
    assert!(text.contains("URIs: http://h/\n"), "{text}");
    // A test prefix moves the two directories under /etc and nothing else.
    let output = client(&["print", "beta"], &[("APT_CLIENT_ROOT", "/tmp/r")]);
    let text = stdout_of(&output);
    assert!(
        text.contains("sudo install -d -m 0755 /tmp/r/etc/apt/keyrings\n"),
        "{text}"
    );
    assert!(
        text.contains("Signed-By: /tmp/r/etc/apt/keyrings/proton-drive-sync-engine.gpg\n"),
        "{text}"
    );
}

#[test]
fn the_text_is_refused_for_a_suite_or_option_it_does_not_have() {
    for (args, code) in [
        (vec!["print", "nightly"], 1),
        (vec!["print"], 64),
        (vec!["print", "beta", "--frobnicate"], 64),
        (vec!["print", "beta", "--uri"], 64),
        (vec!["nonsense"], 64),
        (vec![], 64),
        (vec!["fingerprint-command", "extra"], 64),
    ] {
        let output = client(&args, &[]);
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        assert!(stdout_of(&output).is_empty(), "{args:?}");
    }
}

#[test]
fn the_recorded_fingerprints_are_read_strictly_and_none_recorded_is_a_refusal() {
    // The shipped file is the placeholder until the key exists (task 5.3): nothing is signed.
    let shipped = client(&["fingerprints"], &[]);
    assert_eq!(shipped.status.code(), Some(1));
    assert!(
        stderr_of(&shipped).contains("no signing-key fingerprint is recorded"),
        "{}",
        stderr_of(&shipped)
    );
    let directory = tempdir().expect("tempdir");
    let file = directory.path().join("fingerprints.txt");
    let (a, b) = ("A".repeat(40), "0123456789ABCDEF".repeat(2) + "01234567");
    // Comments, blanks, groups of spaces and a repeat are all fine; the output is sorted, once each.
    let grouped: String = b
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).expect("ascii"))
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(
        &file,
        format!("# the key\n\n{b}  # trailing\n{a}\n{grouped}\n"),
    )
    .expect("write");
    let output = client(
        &["fingerprints"],
        &[("APT_FINGERPRINT_FILE", file.to_str().expect("utf-8"))],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), format!("{b}\n{a}\n"));
    for (what, text) in [
        ("lower case", "a".repeat(40)),
        ("too short", "A".repeat(39)),
        ("too long", "A".repeat(41)),
        ("not hex", "G".repeat(40)),
        ("only comments", "# nothing\n".to_owned()),
    ] {
        fs::write(&file, format!("{text}\n")).expect("write");
        let output = client(
            &["fingerprints"],
            &[("APT_FINGERPRINT_FILE", file.to_str().expect("utf-8"))],
        );
        assert_eq!(output.status.code(), Some(1), "{what}");
        assert!(stdout_of(&output).is_empty(), "{what}");
    }
    let output = client(
        &["fingerprints"],
        &[(
            "APT_FINGERPRINT_FILE",
            directory.path().join("missing").to_str().expect("utf-8"),
        )],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr_of(&output).contains("does not exist"));
}

// --- throwaway keys -------------------------------------------------------------------------------

pub(super) struct AptKey {
    /// The armored secret key, subkeys included.
    pub(super) secret: String,
    /// The binary public key, as the repository serves it.
    pub(super) public: Vec<u8>,
    pub(super) fingerprint: String,
}

fn generate_key(uid: &str, with_subkey: bool) -> AptKey {
    let directory = tempdir().expect("tempdir");
    let home = directory.path().join("g");
    fs::create_dir(&home).expect("gnupg home");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("mode");
    // A failure names the call by its number, not by its arguments (they hold the key's user id).
    let calls = std::cell::Cell::new(0_usize);
    let gpg = |arguments: &[&str]| {
        calls.set(calls.get() + 1);
        let mut command = common::sandboxed("gpg", directory.path());
        command
            .env("GNUPGHOME", &home)
            .args(["--batch", "--pinentry-mode", "loopback", "--passphrase", ""])
            .args(arguments);
        let output = common::run_other_tool(&mut command);
        assert!(
            output.status.success(),
            "gpg call number {} failed: {}",
            calls.get(),
            stderr_of(&output)
        );
        output
    };
    gpg(&["--quick-gen-key", uid, "ed25519", "sign", "never"]);
    let listing = stdout_of(&gpg(&["--with-colons", "--list-secret-keys"]));
    let fingerprint = listing
        .lines()
        .skip_while(|line| !line.starts_with("sec:"))
        .find(|line| line.starts_with("fpr:"))
        .and_then(|line| line.split(':').nth(9))
        .expect("a fingerprint")
        .to_owned();
    if with_subkey {
        gpg(&["--quick-add-key", &fingerprint, "cv25519", "encr", "never"]);
    }
    let secret =
        String::from_utf8(gpg(&["--armor", "--export-secret-keys"]).stdout).expect("armor");
    let public = gpg(&["--export"]).stdout;
    let mut command = common::sandboxed("gpgconf", directory.path());
    command
        .env("GNUPGHOME", &home)
        .args(["--kill", "gpg-agent"]);
    common::run_other_tool(&mut command);
    AptKey {
        secret,
        public,
        fingerprint,
    }
}

/// Three keys made once per test binary: the "old" key, the "new" key, a third that nothing records.
pub(super) fn apt_keys() -> &'static [AptKey; 3] {
    static KEYS: std::sync::OnceLock<[AptKey; 3]> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| {
        [
            generate_key("apt old <old@example.invalid>", false),
            generate_key("apt new <new@example.invalid>", false),
            generate_key("apt other <other@example.invalid>", false),
        ]
    })
}

#[test]
fn the_fingerprint_command_shows_the_primary_key_and_not_a_subkey() {
    if !have_tools("fingerprint_command", &["gpg"]) {
        return;
    }
    let key = generate_key("apt sub <sub@example.invalid>", true);
    let directory = tempdir().expect("tempdir");
    let root = directory.path().join("root");
    let keyring = root.join("etc/apt/keyrings");
    fs::create_dir_all(&keyring).expect("keyring dir");
    // The public half, binary, exactly what the repository serves as the .gpg file.
    let home = directory.path().join("h");
    fs::create_dir(&home).expect("home");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("mode");
    let secret = directory.path().join("secret.asc");
    fs::write(&secret, &key.secret).expect("secret");
    let gpg = |arguments: &[&str]| {
        let mut command = common::sandboxed("gpg", directory.path());
        command
            .env("GNUPGHOME", &home)
            .arg("--batch")
            .args(arguments);
        let output = common::run_other_tool(&mut command);
        assert!(output.status.success(), "{}", stderr_of(&output));
        output
    };
    gpg(&["--import", secret.to_str().expect("utf-8")]);
    let public = gpg(&["--export"]).stdout;
    fs::write(keyring.join("proton-drive-sync-engine.gpg"), public).expect("public key");
    let text = stdout_of(&client(
        &["fingerprint-command"],
        &[("APT_CLIENT_ROOT", root.to_str().expect("utf-8"))],
    ));
    assert!(
        text.starts_with(&format!(
            "gpg --show-keys --with-colons {}/etc/apt/keyrings/proton-drive-sync-engine.gpg | awk",
            root.display()
        )),
        "{text}"
    );
    let script = directory.path().join("show.sh");
    fs::write(&script, &text).expect("script");
    let mut command = common::sandboxed("bash", directory.path());
    command.arg(&script).env("GNUPGHOME", &home);
    let output = common::run_other_tool(&mut command);
    assert!(output.status.success(), "{}", stderr_of(&output));
    // Two lines would mean the subkey's fingerprint was printed as well.
    let printed = stdout_of(&output);
    assert!(
        printed == format!("{}\n", key.fingerprint),
        "the fingerprint command printed {} lines, not the one primary fingerprint",
        printed.lines().count()
    );
    let mut command = common::sandboxed("gpgconf", directory.path());
    command
        .env("GNUPGHOME", &home)
        .args(["--kill", "gpg-agent"]);
    common::run_other_tool(&mut command);
}
