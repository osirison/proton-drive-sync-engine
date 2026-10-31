//! `apt-publish.sh` (task 6.3, design D10, D11): which releases make which suite, the checks on what
//! a release holds, withdrawal, signing, and when a push happens. Publish mode had never run before
//! these tests; the listing, the withdrawn tags and the push are first exercised here.
use super::apt::{AptKey, apt_keys, have_tools};
use super::*;
use std::process::Output;

/// A stand-in for `gh` in the apt step. `api repos/o/r/releases?per_page=100 --paginate` prints the
/// pages in `$STUB_RELEASE_PAGES` one after another (as `gh --paginate` does for arrays: `[..][..]`),
/// and `release download <tag> --repo o/r --dir D --pattern P...` copies what the patterns match in
/// `$STUB_ASSETS/<tag>/`, failing like gh when a pattern matches nothing.
const APT_GH_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$STUB_GH_LOG"
case "$1" in
  api)
    [ "$2" = "repos/o/r/releases?per_page=100" ] || { echo "stub gh: unexpected api call: $*" >&2; exit 98; }
    [ "$3" = --paginate ] || { echo "stub gh: the listing must use --paginate" >&2; exit 97; }
    if [ -n "$STUB_GH_API_FAILS" ]; then echo "stub gh: api failed" >&2; exit 1; fi
    cat "$STUB_RELEASE_PAGES"/*.json
    ;;
  release)
    [ "$2" = download ] || { echo "stub gh: unexpected: $*" >&2; exit 99; }
    tag="$3"; shift 3
    dir=""; repo=""; patterns=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --dir) dir="$2"; shift 2 ;;
        --repo) repo="$2"; shift 2 ;;
        --pattern) patterns="$patterns $2"; shift 2 ;;
        *) echo "stub gh: unexpected argument $1" >&2; exit 96 ;;
      esac
    done
    [ "$repo" = o/r ] || { echo "stub gh: wrong repo '$repo'" >&2; exit 95; }
    [ -d "$STUB_ASSETS/$tag" ] || { echo "release not found" >&2; exit 1; }
    set -f
    for pattern in $patterns; do
      set +f
      found=0
      for file in "$STUB_ASSETS/$tag"/$pattern; do
        [ -e "$file" ] || continue
        /bin/cp "$file" "$dir/" && found=1
      done
      set -f
      [ "$found" = 1 ] || { echo "no assets match the file pattern $pattern" >&2; exit 1; }
    done
    ;;
  *) echo "stub gh: unexpected call: $*" >&2; exit 98 ;;
esac
"#;

/// A stand-in for `apt-ftparchive` that keeps the order of the FileList (the real tool does too), so
/// the script's own sorting decides the bytes of `Packages`. `Release` changes its `Date:` on every
/// call, as the real one does.
const APT_FTPARCHIVE_STUB: &str = r#"#!/bin/bash
printf '%s\n' "$*" >> "$STUB_FTP_LOG"
case "$1" in
  generate)
    conf="$2"
    archive=$(sed -n 's/^Dir { ArchiveDir "\([^"]*\)".*/\1/p' "$conf")
    grep '^BinDirectory' "$conf" | while IFS= read -r line; do
      packages=$(sed -n 's/.*Packages "\([^"]*\)".*/\1/p' <<<"$line")
      list=$(sed -n 's/.*FileList "\([^"]*\)".*/\1/p' <<<"$line")
      : > "$archive/$packages"
      [ -z "$STUB_FTP_EMPTY" ] || continue
      while IFS= read -r deb; do
        [ -n "$deb" ] || continue
        printf 'Filename: %s\nSHA256: %s\n\n' "$deb" "$(sha256sum "$archive/$deb" | cut -d' ' -f1)" >> "$archive/$packages"
      done < "$list"
    done
    ;;
  *)
    dir="${*: -1}"; suite=""
    for argument in "$@"; do
      case "$argument" in APT::FTPArchive::Release::Suite=*) suite="${argument#*=}" ;; esac
    done
    printf 'Origin: proton-drive-sync-engine\nSuite: %s\nDate: %s\nSHA256:\n' "$suite" "$(date -u +%s%N)"
    packages="$dir/main/binary-amd64/Packages"
    printf ' %s %s main/binary-amd64/Packages\n' "$(sha256sum "$packages" | cut -d' ' -f1)" "$(stat -c %s "$packages")"
    ;;
esac
"#;

/// A stand-in for `gpgv` that refuses one file, named by its last path component in
/// `$STUB_GPGV_REFUSES`, and is the real `gpgv` for everything else.
const APT_GPGV_STUB: &str = r#"#!/bin/sh
for argument in "$@"; do
  [ "${argument##*/}" = "$STUB_GPGV_REFUSES" ] && { echo "stub gpgv: refused $argument" >&2; exit 1; }
done
exec /usr/bin/gpgv "$@"
"#;

/// A stand-in for `ssh` that records its arguments and fails like an unreachable host.
const APT_SSH_STUB: &str = "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$STUB_SSH_LOG\"\necho 'ssh: stub: no route' >&2\nexit 255\n";

const MAIN_DEB: &str = "proton-drive-sync-engine";

impl PublishTree {
    /// The stubs the apt step needs: its own `gh` and `apt-ftparchive`.
    fn apt_stubs(&self) {
        write_executable(&self.stubs.join("gh"), APT_GH_STUB);
        write_executable(&self.stubs.join("apt-ftparchive"), APT_FTPARCHIVE_STUB);
    }

    /// The three debs of a release and its SHA256SUMS, in `assets/v<upstream>/`.
    fn apt_release_assets(&self, upstream: &str) -> Vec<String> {
        let directory = self.path("assets").join(format!("v{upstream}"));
        fs::create_dir_all(&directory).expect("assets");
        // Real debs: the script reads their control files, so bytes that only look like debs stop it.
        let mut names = Vec::new();
        let mut sums = String::new();
        for (package, architecture) in [
            (MAIN_DEB, "amd64"),
            ("proton-sync-nautilus", "all"),
            ("proton-sync-nemo", "all"),
        ] {
            let built = real_deb(&directory, package, upstream, architecture);
            let name = built
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            sums.push_str(&format!("{}  {name}\n", sha256_of(&built)));
            names.push(name);
        }
        // draft.sh lists every asset; the tarball is not downloaded by this step.
        sums.push_str(&format!(
            "{}  proton-drive-sync-engine-{upstream}.tar.gz\n",
            "0".repeat(64)
        ));
        fs::write(directory.join("SHA256SUMS"), sums).expect("sums");
        names
    }

    /// The listing the stub `gh` answers with: `(tag, draft, prerelease)`, two rows per page.
    fn apt_listing(&self, rows: &[(String, bool, bool)]) {
        let directory = self.path("release-pages");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("pages");
        for (page, chunk) in rows.chunks(2).enumerate() {
            let items: Vec<serde_json::Value> = chunk
                .iter()
                .map(|(tag, draft, prerelease)| {
                    serde_json::json!({
                        "tag_name": tag, "draft": draft, "prerelease": prerelease,
                        "created_at": "2026-01-01T00:00:00Z",
                    })
                })
                .collect();
            fs::write(
                directory.join(format!("{page:02}.json")),
                serde_json::Value::Array(items).to_string(),
            )
            .expect("page");
        }
    }

    /// Published releases for these upstream versions, listed in this order, with their assets.
    fn apt_world(&self, upstreams: &[&str]) {
        self.apt_stubs();
        let mut rows = Vec::new();
        for upstream in upstreams {
            self.apt_release_assets(upstream);
            rows.push((format!("v{upstream}"), false, upstream.contains('-')));
        }
        self.apt_listing(&rows);
    }

    /// An empty bare repository standing in for the apt repository; every push to it is logged.
    /// Returns its URL.
    fn apt_remote(&self, name: &str) -> String {
        let repository = self.path(name);
        let mut command = common::sandboxed("git", self.directory.path());
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["init", "--bare", "--quiet", "--initial-branch=trunk"])
            .arg(&repository);
        let output = common::run_other_tool(&mut command);
        assert!(output.status.success(), "{}", stderr_of(&output));
        write_executable(
            &repository.join("hooks/post-receive"),
            &format!(
                "#!/bin/sh\nwhile read old new ref; do echo \"$ref $new\" >> '{}'; done\n",
                self.path(&format!("{name}-pushes.log")).display()
            ),
        );
        format!("file://{}", repository.display())
    }

    fn apt_pushes(&self, name: &str) -> Vec<String> {
        self.log(&format!("{name}-pushes.log"))
    }

    fn remote_refs(&self, name: &str) -> String {
        self.git(&self.path(name), &["for-each-ref", "--format=%(refname)"])
    }

    fn remote_show(&self, name: &str, file: &str) -> String {
        self.git(&self.path(name), &["show", &format!("gh-pages:{file}")])
    }

    fn remote_files(&self, name: &str) -> Vec<String> {
        self.git(
            &self.path(name),
            &["ls-tree", "-r", "--name-only", "gh-pages"],
        )
        .lines()
        .map(str::to_owned)
        .collect()
    }

    /// The `Filename:` lines of a suite's Packages file as published, in file order.
    fn remote_packages(&self, name: &str, suite: &str) -> Vec<String> {
        self.remote_show(name, &format!("dists/{suite}/main/binary-amd64/Packages"))
            .lines()
            .filter_map(|line| line.strip_prefix("Filename: "))
            .map(str::to_owned)
            .collect()
    }

    /// Runs the apt step for `tag` against the repository at `remote`, signing with `keys` and
    /// recording those keys' fingerprints (a test overrides any of it through `options`).
    fn apt_publish(
        &self,
        tag: &str,
        dry_run: bool,
        remote: &str,
        keys: &[&AptKey],
        options: &[(&str, &str)],
    ) -> Output {
        let signing = self.path("apt-signing-key");
        fs::write(
            &signing,
            keys.iter()
                .map(|key| key.secret.as_str())
                .collect::<String>(),
        )
        .expect("signing key");
        let recorded = self.path("apt-fingerprints.txt");
        let mut fingerprints: Vec<&str> = keys.iter().map(|key| key.fingerprint.as_str()).collect();
        fingerprints.sort_unstable();
        fs::write(&recorded, fingerprints.join("\n") + "\n").expect("fingerprints");
        let deploy = self.path("apt-deploy-key");
        fs::write(&deploy, "not a real key\n").expect("deploy key");
        fs::create_dir_all(self.path("gpgtmp")).expect("gpg tmp");
        let paths: Vec<(&str, String)> = vec![
            ("APT_SIGNING_KEY_FILE", signing.display().to_string()),
            ("APT_DEPLOY_KEY_FILE", deploy.display().to_string()),
            ("APT_FINGERPRINT_FILE", recorded.display().to_string()),
            (
                "PUBLISH_GPG_TMPDIR",
                self.path("gpgtmp").display().to_string(),
            ),
            (
                "STUB_RELEASE_PAGES",
                self.path("release-pages").display().to_string(),
            ),
            (
                "STUB_FTP_LOG",
                self.path("ftparchive.log").display().to_string(),
            ),
        ];
        let mut environment: Vec<(&str, &str)> = vec![
            (
                "PUBLISH_CHANNELS",
                if tag.contains('-') {
                    "beta"
                } else {
                    "beta stable"
                },
            ),
            ("PUBLISH_DRY_RUN", if dry_run { "true" } else { "false" }),
            ("APT_GIT_NAME", "Release Bot"),
            ("APT_GIT_EMAIL", "bot@example.invalid"),
            ("APT_REMOTE_URL", remote),
        ];
        for (name, value) in &paths {
            environment.push((name, value));
        }
        environment.extend_from_slice(options);
        self.run("apt-publish.sh", &[tag], &environment)
    }

    /// The `release download` calls the stub `gh` saw, as tags, in order.
    fn apt_downloads(&self) -> Vec<String> {
        self.log("gh.log")
            .iter()
            .filter_map(|line| line.strip_prefix("release download "))
            .map(|rest| rest.split(' ').next().unwrap_or_default().to_owned())
            .collect()
    }

    /// The throwaway GnuPG homes still on disk after the script ended.
    fn gpg_homes_left(&self) -> Vec<String> {
        fs::read_dir(self.path("gpgtmp"))
            .map(|entries| {
                entries
                    .map(|entry| {
                        entry
                            .expect("entry")
                            .file_name()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A clone of the repository's gh-pages in `into`.
    fn apt_checkout(&self, name: &str, into: &str) -> std::path::PathBuf {
        let destination = self.path(into);
        self.git(
            self.directory.path(),
            &[
                "clone",
                "--quiet",
                "--branch",
                "gh-pages",
                self.path(name).to_str().expect("utf-8"),
                destination.to_str().expect("utf-8"),
            ],
        );
        destination
    }

    /// The signatures `gpgv` accepts on `file` (clearsigned, or detached over `data`) against the
    /// keyring: one `VALIDSIG` line per signature, split into fields.
    fn gpgv_valid_signatures(
        &self,
        keyring: &Path,
        file: &Path,
        data: Option<&Path>,
    ) -> Vec<Vec<String>> {
        let home = self.path("gpgv-home");
        fs::create_dir_all(&home).expect("home");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("mode");
        let mut command = common::sandboxed("gpgv", self.directory.path());
        command
            .arg("--homedir")
            .arg(&home)
            .args(["--status-fd", "1", "--keyring"])
            .arg(keyring)
            .arg(file);
        if let Some(data) = data {
            command.arg(data);
        }
        let output = common::run_other_tool(&mut command);
        assert!(
            output.status.success(),
            "gpgv {file:?}: {}",
            stderr_of(&output)
        );
        stdout_of(&output)
            .lines()
            .filter_map(|line| line.strip_prefix("[GNUPG:] VALIDSIG "))
            .map(|rest| rest.split(' ').map(str::to_owned).collect())
            .collect()
    }
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn debs_of(releases: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    for upstream in releases {
        for name in [
            format!("{MAIN_DEB}_{upstream}-1_amd64.deb"),
            format!("proton-sync-nautilus_{upstream}-1_all.deb"),
            format!("proton-sync-nemo_{upstream}-1_all.deb"),
        ] {
            names.push(format!("pool/main/p/proton-drive-sync-engine/{name}"));
        }
    }
    sorted(names)
}

// --- which releases -------------------------------------------------------------------------------

#[test]
fn beta_is_the_newest_three_of_all_releases_and_stable_the_newest_three_finals() {
    let tree = PublishTree::new();
    // Listed neither by version nor by date; beta.10 and beta.9 are what a text sort gets wrong.
    tree.apt_world(&[
        "0.4.0-beta.1",
        "0.2.1",
        "0.1.0-beta.10",
        "0.3.0",
        "0.1.0-beta.9",
        "0.4.0-beta.2",
        "0.2.0",
        "0.1.0",
    ]);
    // A draft is not published, and a tag that is no release tag is not a release of this package.
    let mut rows: Vec<(String, bool, bool)> = [
        "0.4.0-beta.1",
        "0.2.1",
        "0.1.0-beta.10",
        "0.3.0",
        "0.1.0-beta.9",
        "0.4.0-beta.2",
        "0.2.0",
        "0.1.0",
    ]
    .iter()
    .map(|upstream| (format!("v{upstream}"), false, upstream.contains('-')))
    .collect();
    rows.push(("v9.9.9".to_owned(), true, false));
    rows.push(("nightly".to_owned(), false, true));
    tree.apt_listing(&rows);
    let remote = tree.apt_remote("apt.git");
    let key = &apt_keys()[0];

    let output = tree.apt_publish("v0.4.0-beta.2", false, &remote, &[key], &[]);
    let text = stderr_of(&output);
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("suite beta: 0.4.0-beta.2 0.4.0-beta.1 0.3.0\n"),
        "{text}"
    );
    assert!(text.contains("suite stable: 0.3.0 0.2.1 0.2.0\n"), "{text}");
    assert!(text.contains("ignoring the release nightly"), "{text}");
    assert_eq!(tree.results("apt").as_deref(), Some("ok"));
    // Exactly the five releases the two suites use are downloaded, once each, with the two patterns.
    assert_eq!(
        sorted(tree.apt_downloads()),
        [
            "v0.2.0",
            "v0.2.1",
            "v0.3.0",
            "v0.4.0-beta.1",
            "v0.4.0-beta.2"
        ]
    );
    assert!(
        tree.log("gh.log").iter().any(|line| {
            line.starts_with("release download v0.3.0 --repo o/r --dir ")
                && line.ends_with(" --pattern *.deb --pattern SHA256SUMS")
        }),
        "{:?}",
        tree.log("gh.log")
    );
    // The listing was asked for once, paginated.
    assert_eq!(
        tree.log("gh.log")
            .iter()
            .filter(|line| line.starts_with("api repos/o/r/releases?per_page=100 --paginate"))
            .count(),
        1
    );
    // The published tree: each suite lists exactly its releases' debs, in sorted order.
    assert_eq!(
        tree.remote_packages("apt.git", "beta"),
        debs_of(&["0.4.0-beta.2", "0.4.0-beta.1", "0.3.0"])
    );
    assert_eq!(
        tree.remote_packages("apt.git", "stable"),
        debs_of(&["0.3.0", "0.2.1", "0.2.0"])
    );
    let files = tree.remote_files("apt.git");
    let pool: Vec<&String> = files
        .iter()
        .filter(|file| file.starts_with("pool/"))
        .collect();
    assert_eq!(
        pool.len(),
        15,
        "the union of the two suites' releases, three debs each: {pool:?}"
    );
    assert!(files.iter().all(|file| !file.contains('~')), "{files:?}");
    for expected in [
        ".nojekyll",
        "proton-drive-sync-engine.gpg",
        "proton-drive-sync-engine.asc",
        "dists/beta/InRelease",
        "dists/beta/Release",
        "dists/beta/Release.gpg",
        "dists/stable/InRelease",
        "dists/stable/Release",
        "dists/stable/Release.gpg",
    ] {
        assert!(
            files.iter().any(|file| file == expected),
            "{expected}: {files:?}"
        );
    }
    assert!(
        tree.remote_show("apt.git", "dists/beta/Release")
            .contains("Suite: beta\n")
    );
    // One commit on gh-pages, by the release identity, naming the version, and nothing but gh-pages.
    let repository = tree.path("apt.git");
    assert_eq!(
        tree.git(&repository, &["rev-list", "--count", "gh-pages"]),
        "1"
    );
    assert_eq!(
        tree.git(
            &repository,
            &["log", "-1", "--format=%an <%ae>%n%s", "gh-pages"]
        ),
        "Release Bot <bot@example.invalid>\nPublish 0.4.0-beta.2"
    );
    assert_eq!(tree.remote_refs("apt.git"), "refs/heads/gh-pages");
    assert!(
        tree.gpg_homes_left().is_empty(),
        "{:?}",
        tree.gpg_homes_left()
    );
}

#[test]
fn stable_does_not_exist_until_a_final_release_is_published() {
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0-beta.1", "0.1.0-beta.2"]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.1.0-beta.2", false, &remote, &[&apt_keys()[0]], &[]);
    let text = stderr_of(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("suite stable: not created"), "{text}");
    let files = tree.remote_files("apt.git");
    assert!(files.iter().any(|file| file == "dists/beta/InRelease"));
    assert!(
        files.iter().all(|file| !file.starts_with("dists/stable")),
        "{files:?}"
    );
    // Nothing published at all is an error, not an empty repository.
    tree.apt_listing(&[("v0.1.0".to_owned(), true, false)]);
    let other = tree.apt_remote("other.git");
    let output = tree.apt_publish("v0.1.0", false, &other, &[&apt_keys()[0]], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("there is no published release"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(tree.remote_refs("other.git"), "");
    assert_eq!(tree.results("apt").as_deref(), Some("failed"));
}

#[test]
fn the_tree_does_not_depend_on_the_order_the_api_lists_the_releases_in() {
    let tree = PublishTree::new();
    let forward = [
        "0.1.0-beta.1",
        "0.1.0-beta.2",
        "0.1.0",
        "0.2.0-beta.1",
        "0.2.0",
    ];
    tree.apt_world(&forward);
    let first = tree.apt_remote("first.git");
    let second = tree.apt_remote("second.git");
    let key = &apt_keys()[0];
    let output = tree.apt_publish("v0.2.0", false, &first, &[key], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let reversed: Vec<&str> = forward.iter().rev().copied().collect();
    tree.apt_world(&reversed);
    let output = tree.apt_publish("v0.2.0", false, &second, &[key], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    for suite in ["beta", "stable"] {
        let path = format!("dists/{suite}/main/binary-amd64/Packages");
        assert_eq!(
            tree.remote_show("first.git", &path),
            tree.remote_show("second.git", &path),
            "{suite}"
        );
    }
    let sorted_files = |name: &str| {
        tree.remote_files(name)
            .into_iter()
            .filter(|file| file.starts_with("pool/"))
            .collect::<Vec<_>>()
    };
    assert_eq!(sorted_files("first.git"), sorted_files("second.git"));
}

#[test]
fn a_run_that_changes_nothing_pushes_nothing_and_a_changed_release_set_pushes_once() {
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0-beta.1", "0.1.0"]);
    let remote = tree.apt_remote("apt.git");
    let key = &apt_keys()[0];
    let first = tree.apt_publish("v0.1.0", false, &remote, &[key], &[]);
    assert!(first.status.success(), "{}", stderr_of(&first));
    assert_eq!(tree.apt_pushes("apt.git").len(), 1);
    let head = tree.git(&tree.path("apt.git"), &["rev-parse", "gh-pages"]);
    let release = tree.remote_show("apt.git", "dists/beta/Release");

    // The same releases again, listed the other way round: the stub's Release has a new Date, and the
    // signatures differ, and still nothing is pushed.
    tree.apt_world(&["0.1.0", "0.1.0-beta.1"]);
    let again = tree.apt_publish("v0.1.0", false, &remote, &[key], &[]);
    assert!(again.status.success(), "{}", stderr_of(&again));
    assert!(
        stderr_of(&again).contains("gh-pages already holds this tree"),
        "{}",
        stderr_of(&again)
    );
    assert_eq!(
        tree.apt_pushes("apt.git").len(),
        1,
        "a second push was made"
    );
    assert_eq!(
        tree.git(&tree.path("apt.git"), &["rev-parse", "gh-pages"]),
        head
    );
    assert_eq!(tree.remote_show("apt.git", "dists/beta/Release"), release);
    assert_eq!(tree.results("apt").as_deref(), Some("ok"));

    // A new release changes the pool and Packages: one push, still a single commit.
    tree.apt_world(&["0.1.0-beta.1", "0.1.0", "0.1.1"]);
    let changed = tree.apt_publish("v0.1.1", false, &remote, &[key], &[]);
    assert!(changed.status.success(), "{}", stderr_of(&changed));
    assert_eq!(tree.apt_pushes("apt.git").len(), 2);
    assert_eq!(
        tree.git(&tree.path("apt.git"), &["rev-list", "--count", "gh-pages"]),
        "1"
    );
    assert_ne!(
        tree.git(&tree.path("apt.git"), &["rev-parse", "gh-pages"]),
        head
    );
    assert!(
        tree.remote_files("apt.git")
            .iter()
            .any(|file| file.ends_with("proton-drive-sync-engine_0.1.1-1_amd64.deb"))
    );
}

#[test]
fn a_dry_run_makes_a_test_signature_and_pushes_nothing() {
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0-beta.1", "0.1.0"]);
    let remote = tree.apt_remote("apt.git");
    let key = &apt_keys()[0];
    let output = tree.apt_publish("v0.1.0", true, &remote, &[key], &[]);
    let text = stderr_of(&output);
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("made a test signature over beta/Release (verified and discarded)"),
        "{text}"
    );
    assert!(text.contains("DRY RUN: "), "{text}");
    assert!(text.contains("has no gh-pages branch yet"), "{text}");
    assert!(!text.contains("pushed gh-pages"), "{text}");
    assert_eq!(
        tree.remote_refs("apt.git"),
        "",
        "a dry run created a branch"
    );
    assert!(tree.apt_pushes("apt.git").is_empty());
    // The releases and the assets were read: that is what a dry run is for.
    assert_eq!(sorted(tree.apt_downloads()), ["v0.1.0", "v0.1.0-beta.1"]);
    assert!(
        tree.gpg_homes_left().is_empty(),
        "{:?}",
        tree.gpg_homes_left()
    );
    assert_eq!(tree.results("apt").as_deref(), Some("ok"));

    // After a real publish of the same releases, a dry run finds the same tree there: it would
    // push nothing, and says so. (Writing the public keys is not signing.)
    let real = tree.apt_publish("v0.1.0", false, &remote, &[key], &[]);
    assert!(real.status.success(), "{}", stderr_of(&real));
    let dry = tree.apt_publish("v0.1.0", true, &remote, &[key], &[]);
    let text = stderr_of(&dry);
    assert!(dry.status.success(), "{text}");
    assert!(text.contains("gh-pages already holds this tree"), "{text}");
    assert!(!text.contains("would force-push"), "{text}");
    assert_eq!(tree.apt_pushes("apt.git").len(), 1);
    // A dry run with a different release set says what a real run would do, and still does not.
    tree.apt_world(&["0.1.0-beta.1", "0.1.0", "0.1.1"]);
    let dry = tree.apt_publish("v0.1.1", true, &remote, &[key], &[]);
    assert!(
        stderr_of(&dry).contains("DRY RUN: gh-pages of "),
        "{}",
        stderr_of(&dry)
    );
    assert!(
        stderr_of(&dry).contains("would force-push a single commit"),
        "{}",
        stderr_of(&dry)
    );
    assert_eq!(tree.apt_pushes("apt.git").len(), 1);
    assert!(tree.gpg_homes_left().is_empty());
}

// --- what a release must hold ---------------------------------------------------------------------

#[test]
fn a_release_whose_debs_do_not_match_its_checksums_stops_the_run_and_pushes_nothing() {
    let main = format!("{MAIN_DEB}_0.2.0-1_amd64.deb");
    let nemo = "proton-sync-nemo_0.2.0-1_all.deb";
    type Edit = Box<dyn Fn(&Path)>;
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "a wrong checksum",
            Box::new({
                let main = main.clone();
                move |dir: &Path| {
                    let sums = fs::read_to_string(dir.join("SHA256SUMS")).expect("sums");
                    let real = sums
                        .lines()
                        .find(|l| l.ends_with(&main))
                        .expect("line")
                        .split(' ')
                        .next()
                        .expect("sum")
                        .to_owned();
                    fs::write(dir.join("SHA256SUMS"), sums.replace(&real, &"f".repeat(64)))
                        .expect("write");
                }
            }),
            "SHA256SUMS lists ffffffff",
        ),
        (
            "a deb the sums do not list",
            Box::new(move |dir: &Path| {
                let sums = fs::read_to_string(dir.join("SHA256SUMS")).expect("sums");
                let kept: String = sums
                    .lines()
                    .filter(|l| !l.ends_with(nemo))
                    .map(|l| format!("{l}\n"))
                    .collect();
                fs::write(dir.join("SHA256SUMS"), kept).expect("write");
            }),
            "SHA256SUMS lists proton-sync-nemo_0.2.0-1_all.deb not at all",
        ),
        (
            "a deb listed twice",
            Box::new(move |dir: &Path| {
                let sums = fs::read_to_string(dir.join("SHA256SUMS")).expect("sums");
                let line = sums
                    .lines()
                    .find(|l| l.ends_with(nemo))
                    .expect("line")
                    .to_owned();
                fs::write(dir.join("SHA256SUMS"), format!("{sums}{line}\n")).expect("write");
            }),
            "more than once",
        ),
        (
            "an empty SHA256SUMS",
            Box::new(|dir: &Path| fs::write(dir.join("SHA256SUMS"), "").expect("write")),
            "the release has no SHA256SUMS",
        ),
        (
            "no SHA256SUMS asset",
            Box::new(|dir: &Path| fs::remove_file(dir.join("SHA256SUMS")).expect("remove")),
            "cannot download the .deb assets and SHA256SUMS of v0.2.0",
        ),
        (
            "no deb at all",
            Box::new(|dir: &Path| {
                for entry in fs::read_dir(dir).expect("dir") {
                    let path = entry.expect("entry").path();
                    if path.extension().is_some_and(|e| e == "deb") {
                        fs::remove_file(path).expect("remove");
                    }
                }
            }),
            "cannot download the .deb assets and SHA256SUMS of v0.2.0",
        ),
        (
            "a deb named for another version",
            Box::new({
                let main = main.clone();
                move |dir: &Path| {
                    fs::rename(dir.join(&main), dir.join(main.replace("0.2.0", "0.2.1")))
                        .expect("rename");
                }
            }),
            "is not a deb named for this version",
        ),
        (
            "two main debs",
            Box::new(move |dir: &Path| {
                // A second, honest deb: revision 2, named and labelled as such.
                let renamed = format!("{MAIN_DEB}_0.2.0-2_amd64.deb");
                deb_file(&dir.join(&renamed), MAIN_DEB, "0.2.0-2", "amd64");
                let sums = fs::read_to_string(dir.join("SHA256SUMS")).expect("sums");
                let sum = sha256_of(&dir.join(&renamed));
                fs::write(dir.join("SHA256SUMS"), format!("{sums}{sum}  {renamed}\n"))
                    .expect("write");
            }),
            "must hold exactly one proton-drive-sync-engine_*_amd64.deb",
        ),
    ];
    for (what, edit, message) in cases {
        let tree = PublishTree::new();
        tree.apt_world(&["0.1.0", "0.2.0"]);
        edit(&tree.path("assets/v0.2.0"));
        let remote = tree.apt_remote("apt.git");
        let output = tree.apt_publish("v0.2.0", false, &remote, &[&apt_keys()[0]], &[]);
        let text = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{what}: {text}");
        assert!(text.contains(message), "{what}: {text}");
        assert_eq!(
            tree.remote_refs("apt.git"),
            "",
            "{what}: something was pushed"
        );
        assert_eq!(tree.results("apt").as_deref(), Some("failed"), "{what}");
        assert!(tree.gpg_homes_left().is_empty(), "{what}");
    }
}

/// Lists `name` in `dir/SHA256SUMS` with the SHA-256 of the file as it is now (replacing any line
/// for that name), so that only what the file says about itself can give it away.
fn list_in_sums(dir: &Path, name: &str) {
    let sums = fs::read_to_string(dir.join("SHA256SUMS")).expect("sums");
    let mut kept: String = sums
        .lines()
        .filter(|line| !line.ends_with(&format!("  {name}")))
        .map(|line| format!("{line}\n"))
        .collect();
    kept.push_str(&format!("{}  {name}\n", sha256_of(&dir.join(name))));
    fs::write(dir.join("SHA256SUMS"), kept).expect("sums");
}

/// Puts `name` in `dir` as a real deb whose control file says `package`, `version` and
/// `architecture`, listed in SHA256SUMS with its real SHA-256.
fn put_deb(dir: &Path, name: &str, package: &str, version: &str, architecture: &str) {
    let _ = fs::remove_file(dir.join(name));
    deb_file(&dir.join(name), package, version, architecture);
    list_in_sums(dir, name);
}

impl PublishTree {
    /// `apt-publish.sh --local <out> <upstream>=<dir>...`, signed with the first throwaway key.
    fn apt_local(&self, out: &Path, specs: &[(&str, &Path)], options: &[(&str, &str)]) -> Output {
        fs::write(self.path("key"), &apt_keys()[0].secret).expect("key");
        fs::create_dir_all(self.path("gpgtmp")).expect("tmp");
        let mut arguments = vec!["--local".to_owned(), out.display().to_string()];
        arguments.extend(
            specs
                .iter()
                .map(|(upstream, dir)| format!("{upstream}={}", dir.display())),
        );
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let key = self.path("key").display().to_string();
        let tmp = self.path("gpgtmp").display().to_string();
        let log = self.path("ftparchive.log").display().to_string();
        let mut environment: Vec<(&str, &str)> = vec![
            ("APT_SIGNING_KEY_FILE", &key),
            ("PUBLISH_GPG_TMPDIR", &tmp),
            ("STUB_FTP_LOG", &log),
        ];
        environment.extend_from_slice(options);
        self.run("apt-publish.sh", &arguments, &environment)
    }
}

// --- what a deb's own control file must say ---------------------------------------------------------

/// What a failed run must leave: nothing pushed, nothing read by apt-ftparchive, no key on disk, and
/// the step recorded as failed.
fn assert_refused_before_the_tree(tree: &PublishTree, what: &str, output: &Output, message: &str) {
    let text = stderr_of(output);
    assert_eq!(output.status.code(), Some(1), "{what}: {text}");
    assert!(text.contains(message), "{what}: {text}");
    assert_eq!(
        tree.remote_refs("apt.git"),
        "",
        "{what}: something was pushed"
    );
    assert!(
        tree.log("ftparchive.log").is_empty(),
        "{what}: apt-ftparchive read a deb that was refused"
    );
    assert_eq!(tree.results("apt").as_deref(), Some("failed"), "{what}");
    assert!(tree.gpg_homes_left().is_empty(), "{what}");
}

#[test]
fn a_deb_whose_control_file_does_not_match_its_name_is_never_signed_into_the_repository() {
    if !have_tools("deb_control", &["dpkg-deb", "gpg", "gpgv"]) {
        return;
    }
    let main = format!("{MAIN_DEB}_0.2.0-1_amd64.deb");
    let nautilus = "proton-sync-nautilus_0.2.0-1_all.deb";
    let nemo = "proton-sync-nemo_0.2.0-1_all.deb";
    type Edit = Box<dyn Fn(&Path)>;
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "the review's deb: named for the main package, control Package sudo, Version 99.0-1",
            Box::new({
                let main = main.clone();
                move |dir: &Path| put_deb(dir, &main, "sudo", "99.0-1", "amd64")
            }),
            "control file says Package 'sudo'",
        ),
        (
            "a Package that is another package of the project",
            Box::new(move |dir: &Path| {
                put_deb(dir, nemo, "proton-sync-nautilus", "0.2.0-1", "all")
            }),
            "control file says Package 'proton-sync-nautilus'",
        ),
        (
            "a Version of another release",
            Box::new(move |dir: &Path| {
                put_deb(dir, nautilus, "proton-sync-nautilus", "99.0-1", "all")
            }),
            "control file says Version '99.0-1'",
        ),
        (
            "an epoch in front of the right version",
            Box::new({
                let main = main.clone();
                move |dir: &Path| put_deb(dir, &main, MAIN_DEB, "9:0.2.0-1", "amd64")
            }),
            "control file says Version '9:0.2.0-1'",
        ),
        (
            "a revision other than the name's",
            Box::new(move |dir: &Path| put_deb(dir, nemo, "proton-sync-nemo", "0.2.0-2", "all")),
            "control file says Version '0.2.0-2'",
        ),
        (
            "an Architecture other than the name's",
            Box::new(move |dir: &Path| put_deb(dir, nemo, "proton-sync-nemo", "0.2.0-1", "amd64")),
            "control file says Architecture 'amd64'",
        ),
        (
            "a Package outside the set the project ships, named for itself",
            Box::new(|dir: &Path| {
                put_deb(dir, "sudo_0.2.0-1_all.deb", "sudo", "0.2.0-1", "all");
            }),
            "'sudo' is not a package this project ships",
        ),
        (
            "bytes that are not a deb at all",
            Box::new(move |dir: &Path| {
                fs::write(dir.join(nemo), "not a deb\n").expect("write");
                list_in_sums(dir, nemo);
            }),
            "control file cannot be read",
        ),
    ];
    for (what, edit, message) in cases {
        let tree = PublishTree::new();
        tree.apt_world(&["0.1.0", "0.2.0"]);
        edit(&tree.path("assets/v0.2.0"));
        let remote = tree.apt_remote("apt.git");
        let output = tree.apt_publish("v0.2.0", false, &remote, &[&apt_keys()[0]], &[]);
        assert_refused_before_the_tree(&tree, what, &output, message);
    }
}

#[test]
fn a_machine_without_dpkg_deb_is_told_so_and_not_that_every_deb_is_unreadable() {
    if !have_tools("no_dpkg_deb", &["dpkg-deb", "gpg", "gpgv"]) {
        return;
    }
    let tree = PublishTree::new();
    tree.apt_world(&["0.2.0"]);
    // A PATH holding every tool this one does except dpkg-deb (the first of two equal names wins).
    let farm = tree.path("no-dpkg-deb");
    fs::create_dir(&farm).expect("farm");
    for directory in std::env::split_paths(&std::env::var_os("PATH").expect("PATH")) {
        for entry in fs::read_dir(&directory).into_iter().flatten().flatten() {
            if entry.file_name() != "dpkg-deb" {
                let _ = std::os::unix::fs::symlink(entry.path(), farm.join(entry.file_name()));
            }
        }
    }
    let path = format!("{}:{}", tree.stubs.display(), farm.display());
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish(
        "v0.2.0",
        false,
        &remote,
        &[&apt_keys()[0]],
        &[("PATH", &path)],
    );
    assert_refused_before_the_tree(&tree, "no dpkg-deb", &output, "dpkg-deb is not installed");
}

#[test]
fn a_prerelease_deb_passes_with_the_tilde_spelling_and_is_refused_with_the_name_spelling() {
    if !have_tools("deb_control_beta", &["dpkg-deb", "gpg", "gpgv"]) {
        return;
    }
    // The honest release: the asset names say `0.2.0-beta.1-1`, the control files say `0.2.0~beta.1-1`.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0", "0.2.0-beta.1"]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.2.0-beta.1", false, &remote, &[&apt_keys()[0]], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(tree.apt_pushes("apt.git").len(), 1);
    // An epoch in front of the right pre-release spelling is another version: it would sort above
    // every release of this project.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0", "0.2.0-beta.1"]);
    put_deb(
        &tree.path("assets/v0.2.0-beta.1"),
        &format!("{MAIN_DEB}_0.2.0-beta.1-1_amd64.deb"),
        MAIN_DEB,
        "9:0.2.0~beta.1-1",
        "amd64",
    );
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.2.0-beta.1", false, &remote, &[&apt_keys()[0]], &[]);
    assert_refused_before_the_tree(
        &tree,
        "epoch",
        &output,
        "control file says Version '9:0.2.0~beta.1-1'",
    );
    // The name's spelling in the control file is not the deb's version.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0", "0.2.0-beta.1"]);
    put_deb(
        &tree.path("assets/v0.2.0-beta.1"),
        &format!("{MAIN_DEB}_0.2.0-beta.1-1_amd64.deb"),
        MAIN_DEB,
        "0.2.0-beta.1-1",
        "amd64",
    );
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.2.0-beta.1", false, &remote, &[&apt_keys()[0]], &[]);
    assert_refused_before_the_tree(
        &tree,
        "name spelling",
        &output,
        "control file says Version '0.2.0-beta.1-1'",
    );
}

#[test]
fn the_architecture_a_symlink_and_an_empty_packages_file_each_stop_the_step() {
    if !have_tools("deb_rules", &["dpkg-deb", "gpg", "gpgv"]) {
        return;
    }
    // Only amd64 and all debs are published (the suites list one architecture): an honest arm64 deb,
    // named and labelled arm64, is still refused.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0", "0.2.0"]);
    put_deb(
        &tree.path("assets/v0.2.0"),
        "proton-sync-nemo_0.2.0-1_arm64.deb",
        "proton-sync-nemo",
        "0.2.0-1",
        "arm64",
    );
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.2.0", false, &remote, &[&apt_keys()[0]], &[]);
    assert_refused_before_the_tree(
        &tree,
        "arm64",
        &output,
        "'proton-sync-nemo_0.2.0-1_arm64.deb' is not a deb named for this version",
    );

    // A deb that is a symlink, and one that is a directory, are not plain files (local mode reads a
    // directory as it is; the download step could not make either).
    for (what, make) in [("a symlink", true), ("a directory", false)] {
        let tree = PublishTree::new();
        tree.apt_world(&["0.2.0"]);
        let dir = tree.path("assets/v0.2.0");
        let nemo = dir.join("proton-sync-nemo_0.2.0-1_all.deb");
        if make {
            let elsewhere = tree.path("elsewhere.deb");
            fs::rename(&nemo, &elsewhere).expect("move");
            std::os::unix::fs::symlink(&elsewhere, &nemo).expect("symlink");
        } else {
            fs::remove_file(&nemo).expect("remove");
            fs::create_dir(&nemo).expect("directory");
        }
        let out = tree.path("out");
        let output = tree.apt_local(&out, &[("0.2.0", &dir)], &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(
            stderr_of(&output).contains("proton-sync-nemo_0.2.0-1_all.deb is not a plain file"),
            "{what}: {}",
            stderr_of(&output)
        );
        assert!(!out.exists(), "{what}: a tree was written");
    }

    // apt-ftparchive that writes no package: the suite would be signed empty.
    let tree = PublishTree::new();
    tree.apt_world(&["0.2.0"]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish(
        "v0.2.0",
        false,
        &remote,
        &[&apt_keys()[0]],
        &[("STUB_FTP_EMPTY", "1")],
    );
    let text = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("the beta suite has an empty Packages file"),
        "{text}"
    );
    assert_eq!(tree.remote_refs("apt.git"), "");
    assert_eq!(tree.results("apt").as_deref(), Some("failed"));
    assert!(tree.gpg_homes_left().is_empty());
}

#[test]
fn a_signature_the_published_key_does_not_verify_is_never_pushed() {
    if !have_tools("gpgv_self_check", &["dpkg-deb", "gpg", "gpgv"]) {
        return;
    }
    // The real gpgv accepts a signature made with the key that was just exported, so the check can
    // only be reached by a gpgv that refuses one file.
    for (file, message) in [
        (
            "InRelease",
            "gpgv does not accept beta/InRelease with the published key",
        ),
        (
            "Release.gpg",
            "gpgv does not accept beta/Release.gpg with the published key",
        ),
    ] {
        let tree = PublishTree::new();
        tree.apt_world(&["0.2.0-beta.1"]);
        write_executable(&tree.stubs.join("gpgv"), APT_GPGV_STUB);
        let remote = tree.apt_remote("apt.git");
        let output = tree.apt_publish(
            "v0.2.0-beta.1",
            false,
            &remote,
            &[&apt_keys()[0]],
            &[("STUB_GPGV_REFUSES", file)],
        );
        let text = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{file}: {text}");
        assert!(text.contains(message), "{file}: {text}");
        assert_eq!(
            tree.remote_refs("apt.git"),
            "",
            "{file}: something was pushed"
        );
        assert_eq!(tree.results("apt").as_deref(), Some("failed"), "{file}");
        assert!(tree.gpg_homes_left().is_empty(), "{file}");
    }
}

#[test]
fn a_bad_signature_already_on_gh_pages_is_repaired_by_the_next_publish() {
    if !have_tools("repair_signatures", &["dpkg-deb", "gpg", "gpgv", "git"]) {
        return;
    }
    for file in ["InRelease", "Release.gpg"] {
        let tree = PublishTree::new();
        tree.apt_world(&["0.2.0-beta.1"]);
        let remote = tree.apt_remote("apt.git");
        let key = &apt_keys()[0];
        let first = tree.apt_publish("v0.2.0-beta.1", false, &remote, &[key], &[]);
        assert!(first.status.success(), "{}", stderr_of(&first));
        assert_eq!(tree.apt_pushes("apt.git").len(), 1);
        // The same release set again changes nothing and pushes nothing...
        let same = tree.apt_publish("v0.2.0-beta.1", false, &remote, &[key], &[]);
        assert!(
            stderr_of(&same).contains("gh-pages already holds this tree"),
            "{file}: {}",
            stderr_of(&same)
        );
        assert_eq!(tree.apt_pushes("apt.git").len(), 1, "{file}");
        // ...until a signature on the branch stops verifying (every file the decision compares is
        // still equal).
        let bad = tree.apt_checkout("apt.git", "bad");
        let path = bad.join("dists/beta").join(file);
        let mut bytes = fs::read(&path).expect("signature file");
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&path, bytes).expect("corrupt");
        tree.git(
            &bad,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-a",
                "-m",
                "corrupt",
            ],
        );
        tree.git(
            &bad,
            &[
                "push",
                "--quiet",
                "--force",
                "origin",
                "HEAD:refs/heads/gh-pages",
            ],
        );
        assert_eq!(
            tree.apt_pushes("apt.git").len(),
            2,
            "{file}: the corruption push"
        );
        let repaired = tree.apt_publish("v0.2.0-beta.1", false, &remote, &[key], &[]);
        let text = stderr_of(&repaired);
        assert!(repaired.status.success(), "{file}: {text}");
        assert!(
            !text.contains("gh-pages already holds this tree"),
            "{file}: {text}"
        );
        assert_eq!(
            tree.apt_pushes("apt.git").len(),
            3,
            "{file}: exactly one repair push"
        );
        let good = tree.apt_checkout("apt.git", "good");
        let keyring = good.join("proton-drive-sync-engine.gpg");
        let release = good.join("dists/beta/Release");
        let (signed, data) = if file == "InRelease" {
            (good.join("dists/beta/InRelease"), None)
        } else {
            (good.join("dists/beta/Release.gpg"), Some(release.as_path()))
        };
        assert_eq!(
            tree.gpgv_valid_signatures(&keyring, &signed, data).len(),
            1,
            "{file}"
        );
    }
}

// --- the tag being published ----------------------------------------------------------------------

#[test]
fn a_tag_that_is_not_in_the_listing_stops_the_step_and_a_deliberate_rebuild_does_not() {
    if !have_tools("tag_in_listing", &["dpkg-deb", "gpg", "gpgv"]) {
        return;
    }
    let key = &apt_keys()[0];
    // The listing lags the release that was just published: the tree would not hold it.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0"]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.2.0", false, &remote, &[key], &[]);
    let text = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("v0.2.0 is not among the published releases GitHub lists"),
        "{text}"
    );
    assert!(
        text.contains("wait a minute and run the step again"),
        "{text}"
    );
    assert_eq!(tree.remote_refs("apt.git"), "");
    assert!(tree.apt_downloads().is_empty(), "something was downloaded");
    assert_eq!(tree.results("apt").as_deref(), Some("failed"));
    // A draft is not a published release: the listing has it, the tree must not.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0"]);
    tree.apt_listing(&[
        ("v0.1.0".to_owned(), false, false),
        ("v0.2.0".to_owned(), true, false),
    ]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.2.0", false, &remote, &[key], &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert_eq!(tree.remote_refs("apt.git"), "");

    // A withdrawn tag is the one deliberate case: the run rebuilds the tree without it, and says so.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0-beta.1", "0.1.0"]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish(
        "v0.1.0",
        false,
        &remote,
        &[key],
        &[("APT_WITHDRAWN", "v0.1.0")],
    );
    let text = stderr_of(&output);
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("v0.1.0 is in APT_WITHDRAWN: this is a deliberate rebuild without it"),
        "{text}"
    );
    assert_eq!(
        sorted(tree.remote_packages("apt.git", "beta")),
        debs_of(&["0.1.0-beta.1"])
    );

    // A tag older than the newest three is outside the window and still in the listing: dispatching
    // it on purpose publishes what the window holds.
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0", "0.2.0", "0.3.0", "0.4.0", "0.5.0"]);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.1.0", false, &remote, &[key], &[]);
    let text = stderr_of(&output);
    assert!(output.status.success(), "{text}");
    assert_eq!(
        sorted(tree.remote_packages("apt.git", "beta")),
        debs_of(&["0.3.0", "0.4.0", "0.5.0"])
    );
}

#[test]
fn a_release_whose_prerelease_flag_disagrees_with_its_tag_stops_the_run_when_it_would_be_used() {
    for (tag, flag) in [("v0.2.0", true), ("v0.2.0-beta.1", false)] {
        let tree = PublishTree::new();
        tree.apt_world(&["0.1.0", "0.2.0-beta.1", "0.2.0"]);
        let upstream = tag.trim_start_matches('v');
        let rows: Vec<(String, bool, bool)> = ["0.1.0", "0.2.0-beta.1", "0.2.0"]
            .iter()
            .map(|u| {
                (
                    format!("v{u}"),
                    false,
                    if *u == upstream {
                        flag
                    } else {
                        u.contains('-')
                    },
                )
            })
            .collect();
        tree.apt_listing(&rows);
        let remote = tree.apt_remote("apt.git");
        let output = tree.apt_publish("v0.2.0", false, &remote, &[&apt_keys()[0]], &[]);
        let text = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{tag}: {text}");
        assert!(
            text.contains(&format!("the release {tag} is marked prerelease={flag}")),
            "{tag}: {text}"
        );
        assert_eq!(tree.remote_refs("apt.git"), "", "{tag}");
    }
    // A wrong flag on a release the suites do not use is no reason to stop.
    let tree = PublishTree::new();
    tree.apt_world(&["0.0.1", "0.1.0", "0.2.0", "0.3.0", "0.4.0"]);
    let rows: Vec<(String, bool, bool)> = ["0.0.1", "0.1.0", "0.2.0", "0.3.0", "0.4.0"]
        .iter()
        .map(|u| (format!("v{u}"), false, *u == "0.0.1"))
        .collect();
    tree.apt_listing(&rows);
    let remote = tree.apt_remote("apt.git");
    let output = tree.apt_publish("v0.4.0", false, &remote, &[&apt_keys()[0]], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(!tree.apt_downloads().contains(&"v0.0.1".to_owned()));
}

// --- withdrawal ------------------------------------------------------------------------------------

#[test]
fn a_withdrawn_tag_is_left_out_and_the_variable_is_optional_and_never_a_glob() {
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0", "0.2.0", "0.3.0", "0.4.0"]);
    let remote = tree.apt_remote("apt.git");
    let key = &apt_keys()[0];
    // Unset (the variable is optional): all four are candidates, the newest three are used.
    let output = tree.apt_publish("v0.4.0", true, &remote, &[key], &[]);
    assert!(
        output.status.success(),
        "unset APT_WITHDRAWN: {}",
        stderr_of(&output)
    );
    assert!(
        stderr_of(&output).contains("suite beta: 0.4.0 0.3.0 0.2.0\n"),
        "{}",
        stderr_of(&output)
    );
    // Empty is the same.
    let output = tree.apt_publish("v0.4.0", true, &remote, &[key], &[("APT_WITHDRAWN", "")]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    // Space and comma separated; the next release takes the withdrawn one's place.
    for value in [
        "v0.3.0",
        "v0.3.0,v0.9.9",
        " v0.3.0  v0.9.9 ",
        "v0.9.9, v0.3.0",
    ] {
        let output = tree.apt_publish("v0.4.0", true, &remote, &[key], &[("APT_WITHDRAWN", value)]);
        let text = stderr_of(&output);
        assert!(output.status.success(), "'{value}': {text}");
        assert!(
            text.contains("leaving out v0.3.0: it is in APT_WITHDRAWN"),
            "'{value}': {text}"
        );
        assert!(
            text.contains("suite beta: 0.4.0 0.2.0 0.1.0\n"),
            "'{value}': {text}"
        );
    }
    // Withdrawing the newest one lets the older ones back in as the newest.
    let output = tree.apt_publish(
        "v0.3.0",
        true,
        &remote,
        &[key],
        &[("APT_WITHDRAWN", "v0.4.0 v0.3.0")],
    );
    assert!(
        stderr_of(&output).contains("suite beta: 0.2.0 0.1.0\n"),
        "{}",
        stderr_of(&output)
    );
    // A name that is not a release tag is an error, not a silent no-op.
    for bad in ["nightly", "0.3.0", "v0.3"] {
        let output = tree.apt_publish("v0.4.0", true, &remote, &[key], &[("APT_WITHDRAWN", bad)]);
        assert_eq!(output.status.code(), Some(1), "'{bad}'");
        assert!(
            stderr_of(&output).contains(&format!(
                "APT_WITHDRAWN names '{bad}', which is not a release tag"
            )),
            "{}",
            stderr_of(&output)
        );
    }
    // A glob is never expanded: `v*` is not a tag, and a file called v0.3.0 in the working
    // directory must not turn it into one.
    fs::write(tree.path("v0.3.0"), "").expect("decoy");
    let output = tree.apt_publish("v0.4.0", true, &remote, &[key], &[("APT_WITHDRAWN", "v*")]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("APT_WITHDRAWN names 'v*'"),
        "{}",
        stderr_of(&output)
    );
}

// --- the signing key and the fingerprints ---------------------------------------------------------

#[test]
fn nothing_is_signed_with_a_key_that_is_not_the_recorded_one_or_while_none_is_recorded() {
    let keys = apt_keys();
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0"]);
    let remote = tree.apt_remote("apt.git");
    // The shipped fingerprint file is the placeholder: refuse, even for a dry run.
    for dry_run in [false, true] {
        let shipped = tree.script("apt-key-fingerprints.txt");
        let output = tree.apt_publish(
            "v0.1.0",
            dry_run,
            &remote,
            &[&keys[0]],
            &[("APT_FINGERPRINT_FILE", shipped.to_str().expect("utf-8"))],
        );
        let text = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "dry_run={dry_run}: {text}");
        assert!(
            text.contains("no signing-key fingerprint is recorded"),
            "{text}"
        );
        assert!(tree.gpg_homes_left().is_empty(), "dry_run={dry_run}");
    }
    // Recorded: the other key. Recorded: one of the two keys that sign. Recorded: both, signing with one.
    let other = tree.path("other-fingerprints.txt");
    let cases = [
        (
            "another key",
            vec![keys[1].fingerprint.clone()],
            vec![&keys[0]],
        ),
        (
            "one of two",
            vec![keys[0].fingerprint.clone()],
            vec![&keys[0], &keys[1]],
        ),
        (
            "a key missing",
            vec![keys[0].fingerprint.clone(), keys[1].fingerprint.clone()],
            vec![&keys[0]],
        ),
    ];
    for (what, recorded, signing) in cases {
        fs::write(&other, recorded.join("\n") + "\n").expect("write");
        let output = tree.apt_publish(
            "v0.1.0",
            false,
            &remote,
            &signing,
            &[("APT_FINGERPRINT_FILE", other.to_str().expect("utf-8"))],
        );
        let text = stderr_of(&output);
        assert_eq!(output.status.code(), Some(1), "{what}: {text}");
        assert!(
            text.contains("apt-key-fingerprints.txt records"),
            "{what}: {text}"
        );
        assert!(text.contains("nothing is signed"), "{what}: {text}");
        assert_eq!(tree.remote_refs("apt.git"), "", "{what}");
        assert!(
            tree.gpg_homes_left().is_empty(),
            "{what}: the throwaway GNUPGHOME is still there"
        );
        assert_eq!(tree.results("apt").as_deref(), Some("failed"), "{what}");
    }
    // A key file that is missing, empty or not a key.
    for (what, content, message) in [
        ("empty", "", "APT_SIGNING_KEY_FILE is not a readable file"),
        (
            "not a key",
            "hello\n",
            "gpg could not import the signing key",
        ),
    ] {
        fs::write(tree.path("bad-key"), content).expect("write");
        let output = tree.apt_publish(
            "v0.1.0",
            false,
            &remote,
            &[&keys[0]],
            &[(
                "APT_SIGNING_KEY_FILE",
                tree.path("bad-key").to_str().expect("utf-8"),
            )],
        );
        assert_eq!(output.status.code(), Some(1), "{what}");
        assert!(
            stderr_of(&output).contains(message),
            "{what}: {}",
            stderr_of(&output)
        );
    }
    let output = tree.apt_publish(
        "v0.1.0",
        false,
        &remote,
        &[&keys[0]],
        &[(
            "APT_SIGNING_KEY_FILE",
            tree.path("missing").to_str().expect("utf-8"),
        )],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("APT_SIGNING_KEY_FILE is not a readable file"),
        "{}",
        stderr_of(&output)
    );
    assert!(tree.gpg_homes_left().is_empty());
}

#[test]
fn every_key_in_the_file_signs_every_file_with_sha512_and_the_key_files_hold_them_all() {
    if !have_tools("two_key_signing", &["gpg", "gpgv"]) {
        return;
    }
    let keys = apt_keys();
    for (what, signing) in [
        ("one key", vec![&keys[0]]),
        ("two keys, a rotation's overlap", vec![&keys[0], &keys[1]]),
    ] {
        let tree = PublishTree::new();
        tree.apt_world(&["0.1.0-beta.1", "0.1.0"]);
        let remote = tree.apt_remote("apt.git");
        let output = tree.apt_publish("v0.1.0", false, &remote, &signing, &[]);
        assert!(output.status.success(), "{what}: {}", stderr_of(&output));
        let checkout = tree.apt_checkout("apt.git", "checkout");
        let keyring = checkout.join("proton-drive-sync-engine.gpg");
        let mut expected: Vec<String> = signing.iter().map(|key| key.fingerprint.clone()).collect();
        expected.sort();
        for suite in ["beta", "stable"] {
            let dists = checkout.join("dists").join(suite);
            let clearsigned = tree.gpgv_valid_signatures(&keyring, &dists.join("InRelease"), None);
            let detached = tree.gpgv_valid_signatures(
                &keyring,
                &dists.join("Release.gpg"),
                Some(&dists.join("Release")),
            );
            for (file, signatures) in [("InRelease", clearsigned), ("Release.gpg", detached)] {
                // One VALIDSIG per key (a one-key ring on a two-signature file would hide the second).
                let found: Vec<String> =
                    sorted(signatures.iter().map(|fields| fields[0].clone()).collect());
                assert_eq!(found, expected, "{what}: {suite}/{file}");
                for fields in &signatures {
                    // VALIDSIG fingerprint date timestamp expire version reserved pk-algo hash-algo ...
                    assert_eq!(fields[6], "22", "{what}: {suite}/{file} is not EdDSA");
                    assert_eq!(
                        fields[7], "10",
                        "{what}: {suite}/{file} is not signed over SHA-512"
                    );
                }
            }
        }
        // The armored file holds the same keys as the binary one.
        let asc = fs::read_to_string(checkout.join("proton-drive-sync-engine.asc")).expect("asc");
        assert!(
            asc.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"),
            "{what}"
        );
        for path in [
            "proton-drive-sync-engine.gpg",
            "proton-drive-sync-engine.asc",
        ] {
            let mut command = common::sandboxed("gpg", tree.directory.path());
            command
                .env("GNUPGHOME", tree.path("gpgv-home"))
                .args(["--batch", "--show-keys", "--with-colons"])
                .arg(checkout.join(path));
            let listing = stdout_of(&common::run_other_tool(&mut command));
            let mut held: Vec<String> = listing
                .lines()
                .filter(|line| line.starts_with("fpr:"))
                .filter_map(|line| line.split(':').nth(9))
                .map(str::to_owned)
                .collect();
            held.sort();
            assert_eq!(held, expected, "{what}: {path}");
        }
        assert!(tree.gpg_homes_left().is_empty(), "{what}");
    }
}

// --- pushing -----------------------------------------------------------------------------------------

#[test]
fn the_push_goes_through_ssh_with_the_key_and_the_pinned_host_keys_and_nothing_else() {
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0"]);
    write_executable(&tree.stubs.join("ssh"), APT_SSH_STUB);
    let output = tree.apt_publish(
        "v0.1.0",
        false,
        "ssh://git@apt.example.invalid/proton-drive-sync-engine-apt.git",
        &[&apt_keys()[0]],
        &[],
    );
    let text = stderr_of(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("cannot read ssh://git@apt.example.invalid/"),
        "{text}"
    );
    let calls = tree.log("ssh.log");
    assert!(!calls.is_empty(), "git never called ssh");
    let key = tree.path("apt-deploy-key");
    let known_hosts = tree.script("known_hosts");
    for call in &calls {
        for expected in [
            format!("-i {}", key.display()),
            "-o IdentitiesOnly=yes".to_owned(),
            "-o BatchMode=yes".to_owned(),
            "-o StrictHostKeyChecking=yes".to_owned(),
            format!("-o UserKnownHostsFile={}", known_hosts.display()),
            "-o GlobalKnownHostsFile=/dev/null".to_owned(),
        ] {
            assert!(call.contains(&expected), "missing '{expected}' in: {call}");
        }
        assert!(call.contains("apt.example.invalid"), "{call}");
    }
    assert!(known_hosts.exists());
    // No clone, no push: the read failed first.
    assert_eq!(tree.results("apt").as_deref(), Some("failed"));
    assert!(tree.gpg_homes_left().is_empty());
}

#[test]
fn the_push_needs_the_deploy_key_and_an_identity_and_the_step_needs_what_route_wrote() {
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0"]);
    let remote = tree.apt_remote("apt.git");
    let key = &apt_keys()[0];
    let missing = tree.path("no-such-key");
    let output = tree.apt_publish(
        "v0.1.0",
        false,
        &remote,
        &[key],
        &[("APT_DEPLOY_KEY_FILE", missing.to_str().expect("utf-8"))],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("APT_DEPLOY_KEY_FILE is not a readable file"),
        "{}",
        stderr_of(&output)
    );
    let output = tree.apt_publish("v0.1.0", false, &remote, &[key], &[("APT_GIT_NAME", "")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("APT_GIT_NAME and APT_GIT_EMAIL are not set"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(tree.remote_refs("apt.git"), "");
    // What route.sh writes: a missing or odd value stops the step before anything is read.
    for (name, value) in [
        ("PUBLISH_CHANNELS", ""),
        ("PUBLISH_CHANNELS", "stable"),
        ("PUBLISH_CHANNELS", "beta nightly"),
        ("PUBLISH_DRY_RUN", ""),
        ("PUBLISH_DRY_RUN", "yes"),
    ] {
        let before = tree.log("gh.log").len();
        let output = tree.apt_publish("v0.1.0", false, &remote, &[key], &[(name, value)]);
        assert_eq!(output.status.code(), Some(1), "{name}='{value}'");
        assert_eq!(
            tree.log("gh.log").len(),
            before,
            "{name}='{value}': gh was called"
        );
    }
    for tag in ["0.1.0", "v0.1", "latest"] {
        let output = tree.apt_publish(tag, false, &remote, &[key], &[]);
        assert_eq!(output.status.code(), Some(1), "{tag}");
    }
    // A listing that fails is a failed step, not an empty repository.
    let output = tree.apt_publish(
        "v0.1.0",
        false,
        &remote,
        &[key],
        &[("STUB_GH_API_FAILS", "1")],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("cannot list the releases of o/r"),
        "{}",
        stderr_of(&output)
    );
    assert_eq!(tree.remote_refs("apt.git"), "");
    assert_eq!(tree.results("apt").as_deref(), Some("failed"));
}

// --- the local mode and the real apt-ftparchive ----------------------------------------------------

#[test]
fn the_local_mode_makes_a_signed_tree_from_directories_and_downloads_and_pushes_nothing() {
    if !have_tools("local_mode", &["gpg", "gpgv"]) {
        return;
    }
    let tree = PublishTree::new();
    tree.apt_world(&["0.1.0-beta.1", "0.1.0"]);
    let key = &apt_keys()[0];
    fs::write(tree.path("key"), &key.secret).expect("key");
    fs::create_dir_all(tree.path("gpgtmp")).expect("tmp");
    let out = tree.path("out");
    let environment = [
        (
            "APT_SIGNING_KEY_FILE",
            tree.path("key").display().to_string(),
        ),
        (
            "PUBLISH_GPG_TMPDIR",
            tree.path("gpgtmp").display().to_string(),
        ),
        (
            "STUB_FTP_LOG",
            tree.path("ftparchive.log").display().to_string(),
        ),
    ];
    let environment: Vec<(&str, &str)> =
        environment.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let beta = tree.path("assets/v0.1.0-beta.1");
    let stable = tree.path("assets/v0.1.0");
    let output = tree.run(
        "apt-publish.sh",
        &[
            "--local",
            out.to_str().expect("utf-8"),
            &format!("0.1.0-beta.1={}", beta.display()),
            &format!("0.1.0={}", stable.display()),
        ],
        &environment,
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(tree.log("gh.log").is_empty(), "a local run called gh");
    assert!(tree.log("ssh.log").is_empty());
    for suite in ["beta", "stable"] {
        let signatures = tree.gpgv_valid_signatures(
            &out.join("proton-drive-sync-engine.gpg"),
            &out.join("dists").join(suite).join("InRelease"),
            None,
        );
        assert_eq!(signatures.len(), 1, "{suite}");
        assert_eq!(signatures[0][0], key.fingerprint, "{suite}");
    }
    assert!(tree.gpg_homes_left().is_empty());
    // Mistakes in the arguments.
    let output = tree.run(
        "apt-publish.sh",
        &["--local", out.to_str().expect("utf-8"), "nonsense"],
        &environment,
    );
    assert_eq!(output.status.code(), Some(1));
    let output = tree.run(
        "apt-publish.sh",
        &["--local", out.to_str().expect("utf-8")],
        &environment,
    );
    assert_eq!(output.status.code(), Some(1));
    let output = tree.run("apt-publish.sh", &["--frobnicate"], &environment);
    assert_eq!(output.status.code(), Some(1));
}

/// A minimal real .deb: `dpkg-deb --build` of a control file only.
fn real_deb(
    directory: &Path,
    package: &str,
    upstream: &str,
    architecture: &str,
) -> std::path::PathBuf {
    let version = format!("{}-1", upstream.replace('-', "~"));
    let file = directory.join(format!("{package}_{upstream}-1_{architecture}.deb"));
    deb_file(&file, package, &version, architecture);
    file
}

/// A real .deb at `file` whose control file says exactly `package`, `version` and `architecture`,
/// whatever `file` is called: how a test makes a deb whose name and control disagree. The bytes
/// depend only on the arguments (SOURCE_DATE_EPOCH), so a release built twice is the same release.
fn deb_file(file: &Path, package: &str, version: &str, architecture: &str) {
    if !super::apt::on_path("dpkg-deb") {
        panic!(
            "dpkg-deb is not on PATH: install dpkg (apt-publish.sh needs it, and so do these tests)"
        );
    }
    let directory = file.parent().expect("a directory");
    let root = directory.join(format!(
        ".build-{}",
        file.file_name().expect("name").to_string_lossy()
    ));
    fs::create_dir_all(root.join("DEBIAN")).expect("DEBIAN");
    fs::write(
        root.join("DEBIAN/control"),
        format!("Package: {package}\nVersion: {version}\nArchitecture: {architecture}\nMaintainer: t <t@example.invalid>\nDescription: test package\n"),
    )
    .expect("control");
    let mut command = common::sandboxed("dpkg-deb", directory);
    command
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .args(["--root-owner-group", "--build"])
        .arg(&root)
        .arg(file);
    let output = common::run_other_tool(&mut command);
    assert!(output.status.success(), "{}", stderr_of(&output));
    fs::remove_dir_all(&root).expect("remove the build directory");
}

#[test]
fn the_real_apt_ftparchive_makes_the_same_packages_files_whatever_the_order_of_the_inputs() {
    if !have_tools(
        "real_apt_ftparchive",
        &["apt-ftparchive", "dpkg-deb", "gpg", "gpgv"],
    ) {
        return;
    }
    let tree = PublishTree::new(); // no stub: the real tool is the first apt-ftparchive on PATH
    let key = &apt_keys()[0];
    fs::write(tree.path("key"), &key.secret).expect("key");
    fs::create_dir_all(tree.path("gpgtmp")).expect("tmp");
    let environment = [
        (
            "APT_SIGNING_KEY_FILE",
            tree.path("key").display().to_string(),
        ),
        (
            "PUBLISH_GPG_TMPDIR",
            tree.path("gpgtmp").display().to_string(),
        ),
    ];
    let environment: Vec<(&str, &str)> =
        environment.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let mut specs = Vec::new();
    for upstream in ["0.1.0-beta.9", "0.1.0-beta.10", "0.1.0", "0.2.0"] {
        let directory = tree.path(&format!("real/v{upstream}"));
        fs::create_dir_all(&directory).expect("dir");
        let mut sums = String::new();
        for (package, architecture) in [
            (MAIN_DEB, "amd64"),
            ("proton-sync-nautilus", "all"),
            ("proton-sync-nemo", "all"),
        ] {
            let built = real_deb(&directory, package, upstream, architecture);
            sums.push_str(&format!(
                "{}  {}\n",
                sha256_of(&built),
                built.file_name().expect("name").to_string_lossy()
            ));
        }
        fs::write(directory.join("SHA256SUMS"), sums).expect("sums");
        specs.push(format!("{upstream}={}", directory.display()));
    }
    let run = |name: &str, order: &[usize]| {
        let out = tree.path(name);
        let mut arguments = vec!["--local".to_owned(), out.display().to_string()];
        arguments.extend(order.iter().map(|at| specs[*at].clone()));
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = tree.run("apt-publish.sh", &arguments, &environment);
        assert!(output.status.success(), "{}", stderr_of(&output));
        out
    };
    let forward = run("forward", &[0, 1, 2, 3]);
    let backward = run("backward", &[3, 2, 1, 0]);
    for suite in ["beta", "stable"] {
        let relative = format!("dists/{suite}/main/binary-amd64/Packages");
        let (a, b) = (
            fs::read(forward.join(&relative)).expect("a"),
            fs::read(backward.join(&relative)).expect("b"),
        );
        assert!(!a.is_empty(), "{suite}");
        assert_eq!(
            a, b,
            "{suite}: Packages differs with the order of the inputs"
        );
        let text = String::from_utf8(a).expect("utf-8");
        // Real stanzas: the package, the revision's tilde spelling and the pool path.
        assert!(
            text.contains("Package: proton-drive-sync-engine\n"),
            "{text}"
        );
        assert!(
            text.contains("Filename: pool/main/p/proton-drive-sync-engine/"),
            "{text}"
        );
        let release =
            fs::read_to_string(forward.join(format!("dists/{suite}/Release"))).expect("Release");
        assert!(release.contains(&format!("Suite: {suite}\n")), "{release}");
        assert!(
            release.contains("Architectures: amd64\n") && release.contains("Components: main\n"),
            "{release}"
        );
        // Release states the SHA-256 of the Packages file it covers.
        assert!(
            release.contains(&sha256_of(&forward.join(&relative))),
            "{release}"
        );
        let signatures = tree.gpgv_valid_signatures(
            &forward.join("proton-drive-sync-engine.gpg"),
            &forward.join(format!("dists/{suite}/InRelease")),
            None,
        );
        assert_eq!(signatures.len(), 1);
    }
    // beta holds the newest three of all four (beta.10, 0.1.0, 0.2.0); stable the finals only.
    let beta =
        fs::read_to_string(forward.join("dists/beta/main/binary-amd64/Packages")).expect("beta");
    assert!(
        beta.contains("Version: 0.2.0-1\n") && beta.contains("Version: 0.1.0~beta.10-1\n"),
        "{beta}"
    );
    assert!(!beta.contains("Version: 0.1.0~beta.9-1\n"), "{beta}");
    let stable = fs::read_to_string(forward.join("dists/stable/main/binary-amd64/Packages"))
        .expect("stable");
    assert!(!stable.contains('~'), "{stable}");
}

#[test]
fn the_pull_request_fixture_makes_trees_for_the_old_key_both_keys_and_the_new_key() {
    if !have_tools(
        "apt_local_fixture",
        &["apt-ftparchive", "dpkg-deb", "gpg", "gpgv"],
    ) {
        return;
    }
    let tree = PublishTree::new();
    let debs = tree.path("debs");
    fs::create_dir_all(&debs).expect("debs");
    for (package, architecture) in [
        (MAIN_DEB, "amd64"),
        ("proton-sync-nautilus", "all"),
        ("proton-sync-nemo", "all"),
    ] {
        real_deb(&debs, package, "0.1.0-beta.1", architecture);
    }
    let work = tree.path("fixture");
    let output = tree.run(
        "apt-local-fixture.sh",
        &[
            debs.to_str().expect("utf-8"),
            work.to_str().expect("utf-8"),
            "0.1.0-beta.1",
        ],
        &[],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(work.join("suites.txt"))
            .expect("suites")
            .trim(),
        "beta"
    );
    let old = fs::read_to_string(work.join("fingerprint.txt"))
        .expect("fingerprint")
        .trim()
        .to_owned();
    assert_eq!(old.len(), 40);
    let signers = |tree_dir: &Path, ring: &Path| -> Vec<String> {
        sorted(
            tree.gpgv_valid_signatures(ring, &tree_dir.join("dists/beta/InRelease"), None)
                .iter()
                .map(|fields| fields[0].clone())
                .collect(),
        )
    };
    let serve = work.join("serve");
    let single = signers(&serve, &serve.join("proton-drive-sync-engine.gpg"));
    assert_eq!(single, std::slice::from_ref(&old));
    // The overlap release is signed by both keys; the new one by one, and not the old.
    let dual = signers(
        &serve.join("dual"),
        &serve.join("dual/proton-drive-sync-engine.gpg"),
    );
    assert_eq!(dual.len(), 2);
    assert!(dual.contains(&old));
    let new = signers(
        &serve.join("new"),
        &serve.join("new/proton-drive-sync-engine.gpg"),
    );
    assert_eq!(new.len(), 1);
    assert_ne!(new[0], old);
    assert!(serve.join("keys/old.gpg").exists() && serve.join("keys/new.gpg").exists());
}
