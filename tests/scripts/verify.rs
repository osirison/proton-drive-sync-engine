//! The verification jobs (task 6.6, design D12): `verify-channel.sh`, `verify-copr.sh`,
//! `verify-aur.sh`, `verify-apt.sh`, and the shape of the jobs in publish.yml and packages.yml that
//! run them. The scripts run on the host here with stub `docker`, `dnf`, `rpm`, `pacman`, `makepkg`,
//! `apt-get`, `dpkg-query`, `apt-cache`, `sudo` and `curl`; their directories under `/etc` and
//! `/home` are moved into the test's own directory by the seams the scripts name.
use super::apt::apt_keys;
use super::*;
use std::process::Output;

/// Environment variables for a script run, as `(name, value)`.
type Env<'a> = Vec<(&'a str, &'a str)>;
/// A table row: what the case is, the script's arguments, and its environment.
type Case<'a, E> = (&'a str, Vec<&'a str>, E);

// --- verify-channel.sh -----------------------------------------------------------------------------

const VERIFY_DOCKER_STUB: &str = "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$STUB_DOCKER_LOG\"\n[ -z \"$STUB_DOCKER_FAILS\" ] || exit 3\n";

impl PublishTree {
    fn verify_channel(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
        write_executable(&self.stubs.join("docker"), VERIFY_DOCKER_STUB);
        self.run("verify-channel.sh", arguments, environment)
    }
}

#[test]
fn a_verification_runs_the_scripts_of_the_tag_in_a_clean_container_with_the_checkout_read_only() {
    let tree = PublishTree::new();
    let root = tree.path("tree");
    // A pre-release: beta only. The environment passed through is exactly what is set, in the
    // script's order.
    let output = tree.verify_channel(
        &["copr", "fedora:43"],
        &[
            ("PUBLISH_TAG", "v0.1.0-beta.1"),
            ("PUBLISH_CHANNELS", "beta"),
            ("COPR_OWNER", "ns"),
            ("VERIFY_APT_WAIT_SECS", "0"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        tree.log("docker.log"),
        [format!(
            "run --rm -v {}:/repo:ro -e COPR_OWNER=ns -e VERIFY_APT_WAIT_SECS=0 fedora:43 bash /repo/packaging/release/verify-copr.sh 0.1.0-beta.1 beta",
            root.display()
        )]
    );
    assert!(
        stderr_of(&output).contains(
            "copr, v0.1.0-beta.1 (version 0.1.0-beta.1), channels: beta, image fedora:43"
        ),
        "{}",
        stderr_of(&output)
    );
    // A final release checks both channels; nothing is passed that is not set.
    let output = tree.verify_channel(
        &["apt", "debian:13"],
        &[
            ("PUBLISH_TAG", "v0.2.0"),
            ("PUBLISH_CHANNELS", "beta stable"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        tree.log("docker.log")[1],
        format!(
            "run --rm -v {}:/repo:ro debian:13 bash /repo/packaging/release/verify-apt.sh 0.2.0 beta stable",
            root.display()
        )
    );
    let output = tree.verify_channel(
        &["aur", "archlinux:base-devel"],
        &[
            ("PUBLISH_TAG", "v0.2.0"),
            ("PUBLISH_CHANNELS", "beta stable"),
            ("VERIFY_EXPECT_VERIFIER", "gpgv"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        tree.log("docker.log")[2],
        format!(
            "run --rm -v {}:/repo:ro -e VERIFY_EXPECT_VERIFIER=gpgv archlinux:base-devel bash /repo/packaging/release/verify-aur.sh 0.2.0 beta stable",
            root.display()
        )
    );
}

#[test]
fn a_verification_that_is_not_given_what_route_wrote_never_starts_a_container() {
    let tree = PublishTree::new();
    let good = [
        ("PUBLISH_TAG", "v0.1.0-beta.1"),
        ("PUBLISH_CHANNELS", "beta"),
    ];
    let with = |name: &str, value: Option<&str>| -> Vec<(String, String)> {
        let mut environment: Vec<(String, String)> = good
            .iter()
            .filter(|(n, _)| *n != name)
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect();
        if let Some(value) = value {
            environment.push((name.to_owned(), value.to_owned()));
        }
        environment
    };
    let cases: Vec<Case<Vec<(String, String)>>> = vec![
        (
            "no tag",
            vec!["copr", "fedora:43"],
            with("PUBLISH_TAG", None),
        ),
        (
            "a malformed tag",
            vec!["copr", "fedora:43"],
            with("PUBLISH_TAG", Some("v0.1")),
        ),
        (
            "a tag without its v",
            vec!["copr", "fedora:43"],
            with("PUBLISH_TAG", Some("0.1.0")),
        ),
        (
            "no channels",
            vec!["copr", "fedora:43"],
            with("PUBLISH_CHANNELS", None),
        ),
        (
            "stable alone",
            vec!["copr", "fedora:43"],
            with("PUBLISH_CHANNELS", Some("stable")),
        ),
        (
            "an unknown channel",
            vec!["copr", "fedora:43"],
            with("PUBLISH_CHANNELS", Some("beta nightly")),
        ),
        (
            "an unknown family",
            vec!["rpm", "fedora:43"],
            with("PUBLISH_CHANNELS", Some("beta")),
        ),
        (
            "no image",
            vec!["copr"],
            with("PUBLISH_CHANNELS", Some("beta")),
        ),
        (
            "a third argument",
            vec!["copr", "fedora:43", "extra"],
            with("PUBLISH_CHANNELS", Some("beta")),
        ),
    ];
    for (what, arguments, environment) in cases {
        let environment: Vec<(&str, &str)> = environment
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        let output = tree.verify_channel(&arguments, &environment);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(
            tree.log("docker.log").is_empty(),
            "{what}: a container was started"
        );
    }
    // The refusal for a missing tag names the cause (route.sh did not run), not just a bad tag.
    let output = tree.verify_channel(&["copr", "fedora:43"], &[("PUBLISH_CHANNELS", "beta")]);
    assert!(
        stderr_of(&output).contains("PUBLISH_TAG is not set: route.sh sets it first"),
        "{}",
        stderr_of(&output)
    );
    // A container that fails fails the job with a non-zero status.
    let output = tree.verify_channel(
        &["apt", "ubuntu:22.04"],
        &[
            ("PUBLISH_TAG", "v0.1.0-beta.1"),
            ("PUBLISH_CHANNELS", "beta"),
            ("STUB_DOCKER_FAILS", "1"),
        ],
    );
    assert_eq!(output.status.code(), Some(3));
}

// --- verify-copr.sh --------------------------------------------------------------------------------

/// `dnf -y install 'dnf-command(copr)'`, `-y copr enable P`, `-y --refresh install NAME`,
/// `-y remove NAME`, `-y copr disable P`. A name installs when it is in $STUB_DNF_HAVE (space
/// separated), or from the second attempt on when it is $STUB_DNF_LATE.
const DNF_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$STUB_DNF_LOG"
case "$*" in
  "-y install dnf-command(copr)") ;;
  "-y copr enable "*) [ -z "$STUB_DNF_ENABLE_FAILS" ] || { echo "Error: no such project" >&2; exit 1; } ;;
  "-y --refresh install "*)
    all="$*"; want="${all#-y --refresh install }"
    case " $STUB_DNF_HAVE " in *" $want "*) exit 0 ;; esac
    if [ "$want" = "$STUB_DNF_LATE" ] && [ "$(grep -c -- '--refresh install' "$STUB_DNF_LOG")" -ge 2 ]; then exit 0; fi
    echo "Error: Unable to find a match: $want" >&2
    exit 1
    ;;
  "-y remove "*) ;;
  "-y copr disable "*) ;;
  *) echo "stub dnf: unexpected: $*" >&2; exit 98 ;;
esac
"#;

const RPM_STUB: &str = "#!/bin/sh\nprintf 'rpm %s\\n' \"$*\" >> \"$STUB_DNF_LOG\"\nprintf '%s' \"$STUB_RPM_VERSION\"\n";

impl PublishTree {
    fn verify_copr(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
        write_executable(&self.stubs.join("dnf"), DNF_STUB);
        write_executable(&self.stubs.join("rpm"), RPM_STUB);
        let log = self.path("dnf.log").display().to_string();
        let mut all: Vec<(&str, &str)> = vec![
            ("COPR_OWNER", "ns"),
            ("VERIFY_COPR_WAIT_SECS", "0"),
            ("VERIFY_COPR_POLL_SECS", "0"),
            ("STUB_DNF_LOG", &log),
        ];
        all.extend_from_slice(environment);
        self.run("verify-copr.sh", arguments, &all)
    }
}

#[test]
fn verify_copr_enables_each_project_installs_the_exact_version_and_checks_what_rpm_says() {
    let tree = PublishTree::new();
    let output = tree.verify_copr(
        &["0.2.0", "beta", "stable"],
        &[
            ("STUB_DNF_HAVE", "proton-drive-sync-engine-0.2.0"),
            ("STUB_RPM_VERSION", "0.2.0"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let beta = "ns/proton-drive-sync-engine-beta";
    let stable = "ns/proton-drive-sync-engine";
    assert_eq!(
        tree.log("dnf.log"),
        [
            "-y install dnf-command(copr)".to_owned(),
            format!("-y copr enable {beta}"),
            "-y --refresh install proton-drive-sync-engine-0.2.0".to_owned(),
            "rpm -q --qf %{VERSION} proton-drive-sync-engine".to_owned(),
            "-y remove proton-drive-sync-engine".to_owned(),
            format!("-y copr disable {beta}"),
            format!("-y copr enable {stable}"),
            "-y --refresh install proton-drive-sync-engine-0.2.0".to_owned(),
            "rpm -q --qf %{VERSION} proton-drive-sync-engine".to_owned(),
            "-y remove proton-drive-sync-engine".to_owned(),
            format!("-y copr disable {stable}"),
        ]
    );
    // A pre-release is asked for with the tilde spelling, which COPR's RPMs carry.
    let tree = PublishTree::new();
    let output = tree.verify_copr(
        &["0.1.0-beta.1", "beta"],
        &[
            ("STUB_DNF_HAVE", "proton-drive-sync-engine-0.1.0~beta.1"),
            ("STUB_RPM_VERSION", "0.1.0~beta.1"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        tree.log("dnf.log")
            .contains(&"-y --refresh install proton-drive-sync-engine-0.1.0~beta.1".to_owned())
    );
}

#[test]
fn verify_copr_fails_when_the_channel_lacks_the_version_or_installs_another_one() {
    // The project has a different version of the same package: the request names the exact one.
    let tree = PublishTree::new();
    let output = tree.verify_copr(
        &["0.1.0-beta.1", "beta"],
        &[
            ("STUB_DNF_HAVE", "proton-drive-sync-engine-0.1.0~beta.2 proton-drive-sync-engine-0.1.0~beta.10 proton-drive-sync-engine-0.1.0"),
            ("STUB_RPM_VERSION", "0.1.0~beta.1"),
        ],
    );
    let text = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("has no proton-drive-sync-engine-0.1.0~beta.1 for this Fedora release after 0s: it is missing from the channel"), "{text}");
    assert!(text.contains("ns/proton-drive-sync-engine-beta"), "{text}");
    // Nothing was removed or disabled after the failure, and nothing else was tried.
    assert!(
        !tree
            .log("dnf.log")
            .iter()
            .any(|line| line.contains("remove") || line.contains("disable"))
    );
    // rpm reporting another version than the one asked for fails, even when dnf was content.
    let tree = PublishTree::new();
    let output = tree.verify_copr(
        &["0.1.0-beta.1", "beta"],
        &[
            ("STUB_DNF_HAVE", "proton-drive-sync-engine-0.1.0~beta.1"),
            ("STUB_RPM_VERSION", "0.1.0~beta.2"),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr_of(&output).contains("rpm reports proton-drive-sync-engine 0.1.0~beta.2 installed from ns/proton-drive-sync-engine-beta, expected 0.1.0~beta.1"), "{}", stderr_of(&output));
    // A project that cannot be enabled.
    let tree = PublishTree::new();
    let output = tree.verify_copr(&["0.1.0", "beta"], &[("STUB_DNF_ENABLE_FAILS", "1")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output)
            .contains("cannot enable the COPR project ns/proton-drive-sync-engine-beta"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn verify_copr_tries_again_until_the_wait_is_over_and_refuses_what_it_was_not_given() {
    let tree = PublishTree::new();
    let output = tree.verify_copr(
        &["0.1.0", "stable"],
        &[
            ("VERIFY_COPR_WAIT_SECS", "30"),
            ("STUB_DNF_LATE", "proton-drive-sync-engine-0.1.0"),
            ("STUB_RPM_VERSION", "0.1.0"),
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let installs = tree
        .log("dnf.log")
        .iter()
        .filter(|line| line.contains("--refresh install"))
        .count();
    assert_eq!(installs, 2, "the second attempt succeeds");
    assert!(
        stderr_of(&output).contains("not installable yet; trying again in 0s"),
        "{}",
        stderr_of(&output)
    );

    let tree = PublishTree::new();
    let mut cases: Vec<Case<Env>> = vec![
        (
            "a non-numeric wait",
            vec!["0.1.0", "beta"],
            vec![("VERIFY_COPR_WAIT_SECS", "soon")],
        ),
        (
            "a non-numeric poll",
            vec!["0.1.0", "beta"],
            vec![("VERIFY_COPR_POLL_SECS", "1s")],
        ),
        ("no channel", vec!["0.1.0"], vec![]),
        ("a bad version", vec!["0.1", "beta"], vec![]),
    ];
    for (what, arguments, environment) in cases.drain(..) {
        let output = tree.verify_copr(&arguments, &environment);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
    }
    // The namespace is the workflow's COPR_OWNER; there is no default.
    let mut command = tree.command("verify-copr.sh");
    command.args(["0.1.0", "beta"]);
    let output = common::run_other_tool(&mut command);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("COPR_OWNER is not set"),
        "{}",
        stderr_of(&output)
    );
    assert!(
        tree.log("dnf.log").is_empty(),
        "dnf ran before the refusals: {:?}",
        tree.log("dnf.log")
    );
}

// --- verify-aur.sh ---------------------------------------------------------------------------------

const PACMAN_STUB: &str = r#"#!/bin/sh
printf 'pacman %s\n' "$*" >> "$STUB_CONTAINER_LOG"
case "$1" in
  -Syu | -U | -Rns) ;;
  -Q) echo "$2 $STUB_PACMAN_VERSION" ;;
  *) echo "stub pacman: unexpected: $*" >&2; exit 98 ;;
esac
"#;

/// Runs in the clone: makes the package file the PKGBUILD names (or two, or fails).
const MAKEPKG_VERIFY_STUB: &str = r#"#!/bin/sh
printf 'makepkg %s in %s\n' "$*" "$(basename "$PWD")" >> "$STUB_CONTAINER_LOG"
[ -z "$STUB_MAKEPKG_FAILS" ] || exit 1
name=$(sed -n 's/^pkgname=//p' PKGBUILD)
ver=$(sed -n 's/^pkgver=//p' PKGBUILD)
: > "$name-$ver-1-x86_64.pkg.tar.zst"
[ -z "$STUB_MAKEPKG_TWO" ] || : > "$name-doc-$ver-1-x86_64.pkg.tar.zst"
"#;

impl PublishTree {
    /// Bare repositories standing in for the AUR: `(package, pkgver)`, each with one PKGBUILD commit.
    fn aur_clone_source(&self, packages: &[(&str, &str)]) {
        for (package, pkgver) in packages {
            let bare = self.path("aur-clone").join(format!("{package}.git"));
            let work = self.path(&format!("aur-work-{package}"));
            for repository in [&bare, &work] {
                fs::create_dir_all(repository).expect("dir");
            }
            let git = |repository: &Path, arguments: &[&str]| {
                self.git(repository, arguments);
            };
            git(
                &bare,
                &["init", "--bare", "--quiet", "--initial-branch=master"],
            );
            git(&work, &["init", "--quiet", "--initial-branch=master"]);
            fs::write(
                work.join("PKGBUILD"),
                format!("pkgname={package}\npkgver={pkgver}\npkgrel=1\n"),
            )
            .expect("PKGBUILD");
            git(&work, &["add", "PKGBUILD"]);
            git(
                &work,
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "pkgbuild",
                ],
            );
            git(
                &work,
                &["push", "--quiet", bare.to_str().expect("utf-8"), "master"],
            );
        }
    }

    fn verify_aur(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
        write_executable(&self.stubs.join("pacman"), PACMAN_STUB);
        write_executable(&self.stubs.join("makepkg"), MAKEPKG_VERIFY_STUB);
        write_executable(&self.stubs.join("useradd"), CONTAINER_LOG_STUB);
        write_executable(&self.stubs.join("chown"), CONTAINER_LOG_STUB);
        write_executable(&self.stubs.join("su"), CONTAINER_SU_STUB);
        fs::create_dir_all(self.path("builder-home")).expect("home");
        let base = format!("file://{}", self.path("aur-clone").display());
        let home = self.path("builder-home").display().to_string();
        let sudoers = self.path("sudoers-builder").display().to_string();
        let mut all: Vec<(&str, &str)> = vec![
            ("AUR_CLONE_BASE", &base),
            ("VERIFY_AUR_HOME", &home),
            ("VERIFY_AUR_SUDOERS", &sudoers),
        ];
        all.extend_from_slice(environment);
        self.run("verify-aur.sh", arguments, &all)
    }

    fn container_log(&self) -> Vec<String> {
        self.log("container.log")
    }
}

#[test]
fn verify_aur_builds_the_package_the_aur_holds_and_requires_pacman_to_report_its_version() {
    let tree = PublishTree::new();
    tree.aur_clone_source(&[
        ("proton-drive-sync-engine-beta", "0.2.0beta1"),
        ("proton-drive-sync-engine", "0.2.0"),
    ]);
    let output = tree.verify_aur(
        &["0.2.0", "beta", "stable"],
        &[("STUB_PACMAN_VERSION", "0.2.0-1")],
    );
    // beta's PKGBUILD is at 0.2.0beta1, not 0.2.0: the beta package is not at the release.
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(stderr_of(&output).contains("the AUR package proton-drive-sync-engine-beta is at pkgver '0.2.0beta1', not 0.2.0: it is missing from the channel"), "{}", stderr_of(&output));
    assert!(
        !tree
            .container_log()
            .iter()
            .any(|line| line.starts_with("makepkg") || line.starts_with("su ")),
        "a build was started: {:?}",
        tree.container_log()
    );

    let tree = PublishTree::new();
    tree.aur_clone_source(&[
        ("proton-drive-sync-engine-beta", "0.2.0"),
        ("proton-drive-sync-engine", "0.2.0"),
    ]);
    let output = tree.verify_aur(
        &["0.2.0", "beta", "stable"],
        &[("STUB_PACMAN_VERSION", "0.2.0-3")],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let log = tree.container_log();
    let builds: Vec<&String> = log
        .iter()
        .filter(|line| line.starts_with("makepkg"))
        .collect();
    assert_eq!(builds.len(), 2, "{log:?}");
    assert_eq!(
        builds[0],
        "makepkg -s --noconfirm in proton-drive-sync-engine-beta"
    );
    assert_eq!(
        builds[1],
        "makepkg -s --noconfirm in proton-drive-sync-engine"
    );
    // The build ran as the build user, the package was installed from the file and removed again.
    assert_eq!(
        log.iter()
            .filter(|line| line.starts_with("su builder -s /bin/bash -c cd "))
            .count(),
        2,
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.starts_with("pacman -U --noconfirm ")
                && line.ends_with("proton-drive-sync-engine-beta-0.2.0-1-x86_64.pkg.tar.zst")),
        "{log:?}"
    );
    assert!(
        log.contains(&"pacman -Rns --noconfirm proton-drive-sync-engine-beta".to_owned()),
        "{log:?}"
    );
    // Nothing outside the test's directory was written: the sudoers entry went to the named file.
    assert_eq!(
        fs::read_to_string(tree.path("sudoers-builder")).expect("sudoers"),
        "builder ALL=(ALL) NOPASSWD: ALL\n"
    );
}

#[test]
fn verify_aur_fails_for_a_stale_package_a_missing_one_a_bad_build_and_a_wrong_installed_version() {
    // (what, packages in the AUR, extra environment, expected message)
    let cases: Vec<(&str, Env, Env, &str)> = vec![
        (
            "a PKGBUILD at an older release",
            vec![("proton-drive-sync-engine-beta", "0.1.0beta0")],
            vec![("STUB_PACMAN_VERSION", "0.1.0beta1-1")],
            "is at pkgver '0.1.0beta0', not 0.1.0beta1",
        ),
        (
            "a package the AUR does not have",
            vec![],
            vec![("STUB_PACMAN_VERSION", "0.1.0beta1-1")],
            "cannot clone the AUR package proton-drive-sync-engine-beta",
        ),
        (
            "a build that fails",
            vec![("proton-drive-sync-engine-beta", "0.1.0beta1")],
            vec![
                ("STUB_MAKEPKG_FAILS", "1"),
                ("STUB_PACMAN_VERSION", "0.1.0beta1-1"),
            ],
            "makepkg failed for proton-drive-sync-engine-beta",
        ),
        (
            "a build that makes two files",
            vec![("proton-drive-sync-engine-beta", "0.1.0beta1")],
            vec![
                ("STUB_MAKEPKG_TWO", "1"),
                ("STUB_PACMAN_VERSION", "0.1.0beta1-1"),
            ],
            "makepkg made 2 package files",
        ),
        (
            "pacman reporting another release",
            vec![("proton-drive-sync-engine-beta", "0.1.0beta1")],
            vec![("STUB_PACMAN_VERSION", "0.1.0beta2-1")],
            "pacman reports proton-drive-sync-engine-beta 0.1.0beta2-1 installed, expected 0.1.0beta1-<pkgrel>",
        ),
        (
            "pacman reporting a version that matches only as a pattern",
            vec![("proton-drive-sync-engine-beta", "0.1.0beta1")],
            vec![("STUB_PACMAN_VERSION", "0x1x0beta1-1")],
            "expected 0.1.0beta1-<pkgrel>",
        ),
        (
            "pacman reporting no release number",
            vec![("proton-drive-sync-engine-beta", "0.1.0beta1")],
            vec![("STUB_PACMAN_VERSION", "0.1.0beta1")],
            "expected 0.1.0beta1-<pkgrel>",
        ),
    ];
    for (what, packages, environment, message) in cases {
        let tree = PublishTree::new();
        tree.aur_clone_source(&packages);
        fs::create_dir_all(tree.path("aur-clone")).expect("base");
        let output = tree.verify_aur(&["0.1.0-beta.1", "beta"], &environment);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(
            stderr_of(&output).contains(message),
            "{what}: {}",
            stderr_of(&output)
        );
    }
    // A repository without a PKGBUILD.
    let tree = PublishTree::new();
    let bare = tree.path("aur-clone/proton-drive-sync-engine-beta.git");
    fs::create_dir_all(&bare).expect("dir");
    tree.git(&bare, &["init", "--bare", "--quiet"]);
    let output = tree.verify_aur(&["0.1.0-beta.1", "beta"], &[]);
    assert_eq!(output.status.code(), Some(1));
    // (an empty clone has no PKGBUILD; the message is the clone's or ours, but never a pass)
}

// --- verify-apt.sh ---------------------------------------------------------------------------------

const APT_GET_STUB: &str = r#"#!/bin/sh
printf 'apt-get %s\n' "$*" >> "$STUB_APT_LOG"
case "$*" in
  *update*)
    n=$(grep -c '^apt-get.* update' "$STUB_APT_LOG")
    if [ -n "$STUB_APT_UPDATE_FAILS_AFTER" ] && [ "$n" -gt "$STUB_APT_UPDATE_FAILS_AFTER" ]; then
      echo "E: Failed to fetch" >&2; exit 100
    fi
    ;;
  *"install -y proton-drive-sync-engine="*) [ -z "$STUB_APT_INSTALL_FAILS" ] || exit 100 ;;
esac
"#;

/// `-W -f='${Status}' PKG` for the verifier probe, `-W -f='${Version}' PKG` after the install.
const DPKG_QUERY_STUB: &str = r#"#!/bin/sh
case "$2" in
  *Status*)
    case " $STUB_INSTALLED " in *" $3 "*) printf 'install ok installed' ;; *) exit 1 ;; esac
    ;;
  *Version*) printf '%s' "$STUB_DPKG_VERSION" ;;
  *) exit 98 ;;
esac
"#;

const APT_CACHE_STUB: &str = "#!/bin/sh\n[ \"$1 $2\" = 'madison proton-drive-sync-engine' ] || exit 98\ncat \"$STUB_MADISON\"\n";

const CURL_KEY_STUB: &str =
    "#!/bin/sh\nprintf 'curl %s\\n' \"$*\" >> \"$STUB_APT_LOG\"\ncat \"$STUB_KEY_FILE\"\n";

const SUDO_STUB: &str = "#!/bin/sh\nexec \"$@\"\n";

const SMOKE_STUB: &str = "#!/bin/sh\nprintf 'smoke %s\\n' \"$*\" >> \"$STUB_APT_LOG\"\n[ -z \"$STUB_SMOKE_FAILS\" ] || exit 1\n";

/// A madison line, as apt prints it for a package a repository lists.
fn madison(version: &str, suite: &str) -> String {
    format!(
        "proton-drive-sync-engine | {version} | https://osirison.github.io/proton-drive-sync-engine-apt {suite}/main amd64 Packages\n"
    )
}

impl PublishTree {
    /// verify-apt.sh with the apt tools stubbed. `madison` is what `apt-cache madison` prints;
    /// the served key is `apt_keys()[0]`'s and the recorded fingerprint is too, unless overridden.
    fn verify_apt(
        &self,
        arguments: &[&str],
        madison: &str,
        environment: &[(&str, &str)],
    ) -> Output {
        for (name, text) in [
            ("apt-get", APT_GET_STUB),
            ("dpkg-query", DPKG_QUERY_STUB),
            ("apt-cache", APT_CACHE_STUB),
            ("curl", CURL_KEY_STUB),
            ("sudo", SUDO_STUB),
        ] {
            write_executable(&self.stubs.join(name), text);
        }
        write_executable(&self.path("tree/packaging/smoke-test.sh"), SMOKE_STUB);
        let key = &apt_keys()[0];
        fs::write(self.path("served-key.gpg"), &key.public).expect("key");
        fs::write(self.path("madison.txt"), madison).expect("madison");
        fs::write(self.path("recorded.txt"), format!("{}\n", key.fingerprint)).expect("recorded");
        // /etc/apt/sources.list.d exists on every apt system; the documented text does not make it.
        fs::create_dir_all(self.path("client-root/etc/apt/sources.list.d")).expect("root");
        let paths: Vec<(&str, String)> = vec![
            (
                "APT_CLIENT_ROOT",
                self.path("client-root").display().to_string(),
            ),
            (
                "APT_FINGERPRINT_FILE",
                self.path("recorded.txt").display().to_string(),
            ),
            ("STUB_APT_LOG", self.path("apt.log").display().to_string()),
            (
                "STUB_MADISON",
                self.path("madison.txt").display().to_string(),
            ),
            (
                "STUB_KEY_FILE",
                self.path("served-key.gpg").display().to_string(),
            ),
        ];
        let mut all: Vec<(&str, &str)> = vec![
            ("VERIFY_APT_WAIT_SECS", "0"),
            ("VERIFY_APT_POLL_SECS", "0"),
            ("STUB_INSTALLED", ""),
        ];
        for (name, value) in &paths {
            all.push((name, value));
        }
        all.extend_from_slice(environment);
        self.run("verify-apt.sh", arguments, &all)
    }
}

#[test]
fn verify_apt_runs_the_documented_text_checks_the_key_installs_the_exact_version_and_smokes_it() {
    let tree = PublishTree::new();
    let output = tree.verify_apt(
        &["0.1.0-beta.1", "beta"],
        &madison("0.1.0~beta.1-1", "beta"),
        &[
            ("STUB_DPKG_VERSION", "0.1.0~beta.1-1"),
            ("STUB_INSTALLED", "gpgv"),
            ("VERIFY_EXPECT_VERIFIER", "gpgv"),
        ],
    );
    let text = stderr_of(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("signature verifiers installed before the first update of the repository: gpgv=yes sqv=no"), "{text}");
    assert!(
        text.contains("the key's fingerprint is the recorded one"),
        "{text}"
    );
    let log = tree.log("apt.log");
    // The key came from the public address, the way the docs say.
    assert!(log.contains(&"curl -fsSL https://osirison.github.io/proton-drive-sync-engine-apt/proton-drive-sync-engine.gpg".to_owned()), "{log:?}");
    // The sources file the text wrote names the suite and the key it wrote.
    let root = tree.path("client-root");
    let sources =
        fs::read_to_string(root.join("etc/apt/sources.list.d/proton-drive-sync-engine.sources"))
            .expect("sources");
    assert!(
        sources.contains("Suites: beta\n")
            && sources.contains(&format!(
                "Signed-By: {}/etc/apt/keyrings/proton-drive-sync-engine.gpg\n",
                root.display()
            )),
        "{sources}"
    );
    assert_eq!(
        fs::read(root.join("etc/apt/keyrings/proton-drive-sync-engine.gpg")).expect("key"),
        apt_keys()[0].public
    );
    // The exact version is installed (with recommends), then the smoke test runs for the release.
    assert!(
        log.contains(&"apt-get install -y proton-drive-sync-engine=0.1.0~beta.1-1".to_owned()),
        "{log:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|line| line.starts_with("smoke "))
            .collect::<Vec<_>>(),
        ["smoke 0.1.0-beta.1 deb"]
    );
    // gnupg only after the repository was updated; the prerequisites before.
    let position = |needle: &str| {
        log.iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle}: {log:?}"))
    };
    assert!(position("--no-install-recommends sudo curl ca-certificates") < position("curl -fsSL"));
    assert!(position("curl -fsSL") < position("--no-install-recommends gnupg"));
    assert!(
        position("--no-install-recommends gnupg")
            < position("install -y proton-drive-sync-engine=")
    );
}

#[test]
fn verify_apt_checks_every_routed_suite_but_installs_and_smokes_once() {
    let tree = PublishTree::new();
    let listing = madison("0.2.0-1", "beta") + &madison("0.2.0-1", "stable");
    let output = tree.verify_apt(
        &["0.2.0", "beta", "stable"],
        &listing,
        &[("STUB_DPKG_VERSION", "0.2.0-1")],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let log = tree.log("apt.log");
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("install -y proton-drive-sync-engine="))
            .count(),
        1
    );
    assert_eq!(
        log.iter().filter(|line| line.starts_with("smoke ")).count(),
        1
    );
    assert_eq!(
        log.iter().filter(|line| line.starts_with("curl ")).count(),
        2,
        "the text runs for each suite"
    );
    // The newest revision of the release in the suite is the one installed.
    let tree = PublishTree::new();
    let listing =
        madison("0.2.0-1", "stable") + &madison("0.2.0-2", "stable") + &madison("0.2.0-1", "beta");
    let output = tree.verify_apt(
        &["0.2.0", "stable"],
        &listing,
        &[("STUB_DPKG_VERSION", "0.2.0-2")],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        tree.log("apt.log")
            .contains(&"apt-get install -y proton-drive-sync-engine=0.2.0-2".to_owned())
    );
}

#[test]
fn verify_apt_fails_when_a_suite_does_not_list_the_release_and_installs_nothing() {
    // (what, madison text, suites)
    let cases: Vec<(&str, String, Vec<&str>)> = vec![
        ("nothing listed", String::new(), vec!["beta"]),
        (
            "another beta.10 for beta.1",
            madison("0.1.0~beta.10-1", "beta"),
            vec!["beta"],
        ),
        (
            "another release",
            madison("0.1.0~beta.2-1", "beta"),
            vec!["beta"],
        ),
        (
            "the other suite only",
            madison("0.1.0~beta.1-1", "stable"),
            vec!["beta"],
        ),
    ];
    for (what, listing, suites) in cases {
        let tree = PublishTree::new();
        let mut arguments = vec!["0.1.0-beta.1"];
        arguments.extend(suites);
        let output = tree.verify_apt(
            &arguments,
            &listing,
            &[("STUB_DPKG_VERSION", "0.1.0~beta.1-1")],
        );
        let text = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{what}: {text}");
        assert!(text.contains("lists no version 0.1.0~beta.1-<revision> of proton-drive-sync-engine after waiting 0s: it is missing from the channel"), "{what}: {text}");
        assert!(
            !tree
                .log("apt.log")
                .iter()
                .any(|line| line.contains("install -y proton-drive-sync-engine=")),
            "{what}"
        );
        assert!(
            !tree
                .log("apt.log")
                .iter()
                .any(|line| line.starts_with("smoke ")),
            "{what}"
        );
    }
    // The first suite is fine, the second lacks the release: the run still fails.
    let tree = PublishTree::new();
    let output = tree.verify_apt(
        &["0.2.0", "beta", "stable"],
        &madison("0.2.0-1", "beta"),
        &[("STUB_DPKG_VERSION", "0.2.0-1")],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("the suite stable of the repository lists no version"),
        "{}",
        stderr_of(&output)
    );
    // An `apt update` that fails after the first is "not there yet", and then missing.
    let tree = PublishTree::new();
    let output = tree.verify_apt(
        &["0.1.0-beta.1", "beta"],
        &madison("0.1.0~beta.1-1", "beta"),
        &[("STUB_APT_UPDATE_FAILS_AFTER", "1")],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("apt update failed"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn verify_apt_requires_the_expected_verifier_the_recorded_key_and_the_installed_version() {
    let ok = madison("0.1.0~beta.1-1", "beta");
    // (what, environment, expected message)
    let cases: Vec<(&str, Env, &str)> = vec![
        (
            "both verifiers where gpgv alone is expected",
            vec![
                ("STUB_INSTALLED", "gpgv sqv"),
                ("VERIFY_EXPECT_VERIFIER", "gpgv"),
            ],
            "expected gpgv alone to verify, found gpgv=yes sqv=yes",
        ),
        (
            "gpgv where sqv is expected",
            vec![
                ("STUB_INSTALLED", "gpgv"),
                ("VERIFY_EXPECT_VERIFIER", "sqv"),
            ],
            "expected sqv alone to verify, found gpgv=yes sqv=no",
        ),
        (
            "no verifier at all",
            vec![("VERIFY_EXPECT_VERIFIER", "gpgv")],
            "expected gpgv alone to verify, found gpgv=no sqv=no",
        ),
        (
            "an unknown verifier name",
            vec![("VERIFY_EXPECT_VERIFIER", "gpg")],
            "VERIFY_EXPECT_VERIFIER is 'gpg': gpgv or sqv",
        ),
        (
            "a non-numeric wait",
            vec![("VERIFY_APT_WAIT_SECS", "later")],
            "must be numbers",
        ),
        (
            "a different version installed",
            vec![("STUB_DPKG_VERSION", "0.1.0~beta.1-2")],
            "dpkg reports proton-drive-sync-engine 0.1.0~beta.1-2 installed, expected 0.1.0~beta.1-1",
        ),
        (
            "an install that fails",
            vec![
                ("STUB_APT_INSTALL_FAILS", "1"),
                ("STUB_DPKG_VERSION", "0.1.0~beta.1-1"),
            ],
            "apt could not install proton-drive-sync-engine=0.1.0~beta.1-1",
        ),
        (
            "a smoke test that fails",
            vec![
                ("STUB_SMOKE_FAILS", "1"),
                ("STUB_DPKG_VERSION", "0.1.0~beta.1-1"),
            ],
            "the smoke test failed",
        ),
    ];
    for (what, environment, message) in cases {
        let tree = PublishTree::new();
        let output = tree.verify_apt(&["0.1.0-beta.1", "beta"], &ok, &environment);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(
            stderr_of(&output).contains(message),
            "{what}: {}",
            stderr_of(&output)
        );
    }
    // The key the repository serves is not the recorded one: nothing is installed.
    let tree = PublishTree::new();
    let other = tree.path("other-recorded.txt");
    fs::write(&other, format!("{}\n", apt_keys()[1].fingerprint)).expect("write");
    let output = tree.verify_apt(
        &["0.1.0-beta.1", "beta"],
        &ok,
        &[
            ("APT_FINGERPRINT_FILE", other.to_str().expect("utf-8")),
            ("STUB_DPKG_VERSION", "0.1.0~beta.1-1"),
        ],
    );
    let text = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains(&format!(
            "the key the repository serves has the fingerprint(s) '{}' but '{}' is recorded",
            apt_keys()[0].fingerprint,
            apt_keys()[1].fingerprint
        )),
        "{text}"
    );
    assert!(
        !tree
            .log("apt.log")
            .iter()
            .any(|line| line.contains("install -y proton-drive-sync-engine="))
    );
    // The placeholder file records nothing: also a failure, never a skipped check.
    let tree = PublishTree::new();
    let placeholder = tree.script("apt-key-fingerprints.txt");
    let output = tree.verify_apt(
        &["0.1.0-beta.1", "beta"],
        &ok,
        &[("APT_FINGERPRINT_FILE", placeholder.to_str().expect("utf-8"))],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("no signing-key fingerprint is recorded"),
        "{}",
        stderr_of(&output)
    );
    // Arguments.
    for arguments in [vec!["0.1.0-beta.1"], vec!["0.1", "beta"]] {
        let tree = PublishTree::new();
        let output = tree.verify_apt(&arguments, &ok, &[]);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
    }
}

// --- the jobs in the workflows ---------------------------------------------------------------------

/// The quoted items of `key: ["a", "b"]` in a job's text.
fn matrix_list(job: &str, key: &str) -> Vec<String> {
    let line = job
        .lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key}: [")))
        .unwrap_or_else(|| panic!("no `{key}: [` in the job"));
    line.trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .collect()
}

/// The `(image, verifier)` pairs of a job's `matrix.include`.
fn image_pairs(job: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut image = None;
    for line in job.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("- image: ") {
            image = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("verifier: ") {
            pairs.push((
                image.take().expect("an image before its verifier"),
                value.to_owned(),
            ));
        }
    }
    pairs
}

#[test]
fn the_verification_jobs_follow_the_publish_job_outside_its_environment_and_with_no_secret() {
    let publish = workflow("publish.yml");
    for (name, call) in [
        (
            "verify-copr",
            "run: packaging/release/verify-channel.sh copr \"$IMAGE\"",
        ),
        (
            "verify-aur",
            "run: packaging/release/verify-channel.sh aur archlinux:base-devel",
        ),
        (
            "verify-apt",
            "run: packaging/release/verify-channel.sh apt \"$IMAGE\"",
        ),
    ] {
        let job = job_block(&publish, name);
        assert!(job.contains("    needs: publish\n"), "{name}");
        assert!(
            job.contains("    if: \"!inputs.dry_run\"\n"),
            "{name}: a dry run publishes nothing to find"
        );
        assert!(!job.contains("environment"), "{name} is in an environment");
        assert!(!job.contains("secrets."), "{name} reads a secret");
        assert!(
            !job.contains("permissions"),
            "{name} changes the permissions"
        );
        assert!(!job.contains("continue-on-error"), "{name}");
        assert!(
            !job.contains("always()"),
            "{name}: a cancelled run must not verify"
        );
        // The token is the read-only default, used to read the release.
        assert_eq!(
            job.matches("GH_TOKEN: ${{ github.token }}").count(),
            1,
            "{name}"
        );
        // The three steps every job repeats, in order, then the check; the tag's scripts run.
        let route = job
            .find("run: packaging/release/route.sh \"$TAG\"\n")
            .unwrap_or_else(|| panic!("{name}: route"));
        let checkout = job
            .find("run: packaging/release/tag-checkout.sh \"$PUBLISH_TAG\"\n")
            .unwrap_or_else(|| panic!("{name}: tag checkout"));
        let verify = job.find(call).unwrap_or_else(|| panic!("{name}: {call}"));
        assert!(route < checkout && checkout < verify, "{name}");
        assert!(job.contains("persist-credentials: false"), "{name}");
        assert!(
            job.contains("fetch-depth: 0"),
            "{name}: the gate needs the history"
        );
    }
}

#[test]
fn the_fedora_releases_agree_between_the_build_the_smoke_install_and_the_copr_verification() {
    let packages = workflow("packages.yml");
    let publish = workflow("publish.yml");
    let oldest = packages
        .lines()
        .find_map(|line| line.trim().strip_prefix("FEDORA_OLDEST: "))
        .expect("FEDORA_OLDEST")
        .trim_matches('"')
        .to_owned();
    let smoke = matrix_list(&job_block(&packages, "smoke-rpm"), "fedora");
    let verify = matrix_list(&job_block(&publish, "verify-copr"), "fedora");
    assert_eq!(
        smoke, verify,
        "smoke-rpm and verify-copr check different Fedora releases"
    );
    let numbers: Vec<u32> = smoke
        .iter()
        .map(|release| release.parse().expect("a release number"))
        .collect();
    assert_eq!(
        numbers.iter().min().copied(),
        Some(oldest.parse().expect("a number")),
        "the oldest release in the lists is the one the RPM is built on"
    );
    assert!(numbers.len() >= 2, "{smoke:?}");
}

#[test]
fn the_apt_distributions_agree_between_the_pull_request_job_the_smoke_install_and_the_verification()
{
    let packages = workflow("packages.yml");
    let publish = workflow("publish.yml");
    let pull_request = image_pairs(&job_block(&packages, "apt-repo"));
    let published = image_pairs(&job_block(&publish, "verify-apt"));
    assert_eq!(
        pull_request, published,
        "apt-repo and verify-apt check different distributions or verifiers"
    );
    assert!(
        pull_request.len() >= 3,
        "the oldest apt, a recent one and the newest: {pull_request:?}"
    );
    let smoke_images = matrix_list(&job_block(&packages, "smoke-deb"), "image");
    for (image, verifier) in &pull_request {
        assert!(
            smoke_images.contains(image),
            "{image} is verified but not smoke-installed"
        );
        assert!(["gpgv", "sqv"].contains(&verifier.as_str()), "{verifier}");
    }
    // The oldest and the newest of the five the deb is installed on are both checked.
    assert!(
        pull_request
            .iter()
            .any(|(image, _)| image == "ubuntu:22.04")
    );
    assert!(
        pull_request
            .iter()
            .any(|(image, _)| image == "ubuntu:26.04")
    );
}

#[test]
fn the_final_step_names_the_steps_that_record_a_result() {
    let publish = workflow("publish.yml");
    let line = publish
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("run: packaging/release/aggregate.sh ")
        })
        .expect("the aggregate step");
    let names: Vec<&str> = line.split(' ').collect();
    assert_eq!(names, ["preflight", "apt", "aur", "copr"]);
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/release");
    for (name, script) in [
        ("preflight", "preflight.sh"),
        ("apt", "apt-publish.sh"),
        ("aur", "aur-publish.sh"),
        ("copr", "copr.sh"),
    ] {
        let text = fs::read_to_string(directory.join(script)).expect("script");
        assert!(
            text.contains(&format!("record_on_exit {name}\n")),
            "{script} does not record {name}"
        );
        assert!(text.contains("finish_recording"), "{script}");
    }
    // Every script that records a result is in the list: a recorded step the aggregate ignores
    // would fail silently.
    for entry in fs::read_dir(&directory).expect("dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "sh") {
            let text = fs::read_to_string(&path).expect("script");
            for recorded in text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("record_on_exit "))
            {
                if recorded.starts_with('$') || recorded.starts_with('"') || recorded == "NAME:" {
                    continue;
                }
                assert!(
                    names.contains(&recorded),
                    "{} records '{recorded}', which the final step does not check",
                    path.display()
                );
            }
        }
    }
}
