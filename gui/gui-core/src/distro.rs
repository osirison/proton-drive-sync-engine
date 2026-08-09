//! Which distribution is this, so the CLI-missing screen can show a command that works (C5, #178).
//!
//! `9a CLI missing` is the one onboarding frame that only ever appears when something has already
//! gone wrong: the `proton-drive` CLI this app drives is not installed. It draws one sentence and
//! one command box, and the command has to be the right one or the screen is worse than useless —
//! a person who runs `sudo apt install proton-drive` on Fedora learns nothing except that the app
//! is guessing.
//!
//! So detection is deliberately conservative. `09-onboarding.md` and
//! `14-behaviour-and-state.md`'s fallback table agree on the rule: **if detection fails, show the
//! tarball instructions rather than guessing a package manager.** Every path through this module
//! that is not certain returns [`None`], and [`None`] is the tarball.
//!
//! Parsing is over `/etc/os-release` *text* so it is testable without a matching machine — the
//! reader is one line at the bottom.

use std::path::Path;

/// A distribution we know how to install on: the id that selects the install command, plus the name
/// the sentence uses.
///
/// The two are separate on purpose. The id is ours — a small closed set the UI has a command for —
/// while the name is what the machine should be called. On Linux Mint (`ID=linuxmint`,
/// `ID_LIKE="ubuntu debian"`) that is `Detected Linux Mint` above an `apt` command, where an
/// id-only design would have had to choose between naming the wrong distribution and offering no
/// command at all.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Distro {
    /// The package family: one of `debian`, `fedora`, `arch`, `suse`, `alpine`. Closed set — the UI
    /// carries one install command per value and nothing else is reachable.
    pub id: String,
    /// What to write after `Detected`.
    ///
    /// A **short brand name** when we recognise the distribution itself — `Debian`, not the `NAME`
    /// field's `Debian GNU/Linux` and certainly not `PRETTY_NAME`'s `Debian GNU/Linux 12
    /// (bookworm)`. `9a CLI missing` draws `Detected Debian`, and a sentence that names a version
    /// nobody asked about reads as a diagnostic rather than as an aside.
    ///
    /// For a distribution we know only through its `ID_LIKE` parent, there is no brand name to look
    /// up, so its own `NAME` is used — better `Detected Foo Linux` above an `apt` command than
    /// `Detected Debian` on a machine that is not Debian.
    pub name: String,
}

/// `ID` / `ID_LIKE` values we have an install command for, mapped to the family id and the short
/// brand name the `Detected …` sentence uses. Lookup is exact; the order is for reading only.
const KNOWN: &[(&str, &str, &str)] = &[
    // family id, os-release id, short brand name
    ("debian", "debian", "Debian"),
    ("debian", "ubuntu", "Ubuntu"),
    ("debian", "linuxmint", "Linux Mint"),
    ("debian", "pop", "Pop!_OS"),
    ("debian", "elementary", "elementary OS"),
    ("debian", "raspbian", "Raspberry Pi OS"),
    ("fedora", "fedora", "Fedora"),
    ("fedora", "rhel", "Red Hat Enterprise Linux"),
    ("fedora", "centos", "CentOS"),
    ("fedora", "rocky", "Rocky Linux"),
    ("fedora", "almalinux", "AlmaLinux"),
    ("arch", "arch", "Arch Linux"),
    ("arch", "manjaro", "Manjaro"),
    ("arch", "endeavouros", "EndeavourOS"),
    ("suse", "opensuse", "openSUSE"),
    ("suse", "opensuse-tumbleweed", "openSUSE Tumbleweed"),
    ("suse", "opensuse-leap", "openSUSE Leap"),
    ("suse", "sles", "SUSE Linux Enterprise Server"),
    ("alpine", "alpine", "Alpine Linux"),
];

/// Identify the distribution from the contents of an `/etc/os-release` file.
///
/// Resolution order, and each step's reason:
/// 1. `ID` against the known table — the distribution naming itself.
/// 2. each `ID_LIKE` token in order — the distribution naming its parent, which is a statement by
///    the distribution and not an inference by us. First hit wins, because `ID_LIKE` is ordered
///    closest-first by convention.
/// 3. otherwise [`None`] — the tarball.
///
/// The *name* follows which step matched — see [`Distro::name`].
pub fn parse_os_release(text: &str) -> Option<Distro> {
    let fields = parse_fields(text);
    let lookup = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };

    let known = |candidate: &str| {
        let lowered = candidate.to_ascii_lowercase();
        KNOWN
            .iter()
            .find(|(_, id, _)| *id == lowered)
            .map(|(family, _, proper)| (*family, *proper))
    };

    // Step 1 — the distribution names itself, and we know it. Its brand name is ours to write.
    if let Some((family, proper)) = lookup("ID").and_then(known) {
        return Some(Distro {
            id: family.to_string(),
            name: proper.to_string(),
        });
    }

    // Step 2 — the distribution names a parent we know. That is the distribution's own statement
    // about itself, not an inference of ours, so the parent's package manager is the right one; but
    // the name has to be the machine's, because the parent's brand name would be a different OS.
    for candidate in lookup("ID_LIKE").unwrap_or_default().split_whitespace() {
        if let Some((family, proper)) = known(candidate) {
            return Some(Distro {
                id: family.to_string(),
                name: lookup("NAME")
                    .filter(|value| !value.is_empty())
                    .unwrap_or(proper)
                    .to_string(),
            });
        }
    }

    // Step 3 — the tarball.
    None
}

/// Split `os-release` into `KEY=value` pairs, unquoting values the way the format specifies.
///
/// The file is shell-syntax-ish: values may be bare, single-quoted or double-quoted, comments start
/// with `#`, and blank lines are allowed. Anything that is not a `KEY=value` line is skipped rather
/// than treated as an error — a file with one odd line still identifies the distribution, and the
/// alternative (failing the whole parse) would show the tarball to someone we could have helped.
fn parse_fields(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), unquote(value.trim())))
        })
        .collect()
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        // Only the escapes the format actually uses inside double quotes. A `\` before anything
        // else is kept, because it is far likelier to be a literal backslash in a distribution's
        // name than an escape we failed to implement.
        let inner = &value[1..value.len() - 1];
        if bytes[0] == b'"' {
            return inner
                .replace("\\\"", "\"")
                .replace("\\$", "$")
                .replace("\\\\", "\\");
        }
        return inner.to_string();
    }
    value.to_string()
}

/// Read and identify the distribution, `None` when the file is missing, unreadable, not UTF-8, or
/// names nothing we have a command for.
pub fn detect(os_release_path: &Path) -> Option<Distro> {
    parse_os_release(&std::fs::read_to_string(os_release_path).ok()?)
}

/// Where `os-release` lives. `/etc` first (a local override), then the `/usr/lib` copy the spec
/// says shipping packages should install — a stateless system has only the latter.
pub const OS_RELEASE_PATHS: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

/// Identify this machine from the first of [`OS_RELEASE_PATHS`] that can be read.
///
/// **Select the file, then parse it — never fall through on a parse result.** `detect` answers
/// `None` both for "no such file" and for "this file names a distribution we have no command for",
/// and a `find_map` over the two paths cannot tell them apart. On a machine whose `/etc/os-release`
/// says `ID=mydistro` while the base package's `/usr/lib/os-release` still says `ID=debian`, that
/// would report Debian — reading a file the machine had explicitly overridden, to contradict it.
pub fn detect_here() -> Option<Distro> {
    OS_RELEASE_PATHS
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .as_deref()
        .and_then(parse_os_release)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBIAN: &str = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
ID=debian
HOME_URL="https://www.debian.org/"
"#;

    const MINT: &str = r#"NAME="Linux Mint"
VERSION="21.3 (Virginia)"
ID=linuxmint
ID_LIKE="ubuntu debian"
"#;

    const NIXOS: &str = r#"NAME=NixOS
ID=nixos
VERSION_ID="24.05"
"#;

    #[test]
    fn a_recognised_distribution_gets_its_short_brand_name() {
        // The frame draws `Detected Debian`. os-release says `Debian GNU/Linux` and PRETTY_NAME
        // says `Debian GNU/Linux 12 (bookworm)`; echoing either turns an aside into a diagnostic.
        let distro = parse_os_release(DEBIAN).expect("debian");
        assert_eq!(distro.id, "debian");
        assert_eq!(distro.name, "Debian");
    }

    #[test]
    fn a_recognised_derivative_is_called_by_its_own_name_and_takes_its_parents_command() {
        let distro = parse_os_release(MINT).expect("mint is in the table");
        assert_eq!(distro.id, "debian", "apt");
        assert_eq!(distro.name, "Linux Mint");
    }

    #[test]
    fn an_unknown_derivative_of_a_known_parent_uses_its_own_name_field() {
        // We have never heard of it, so there is no brand name to look up — but it told us it is
        // Debian-like, and `Detected Debian` on a machine that is not Debian would be worse than
        // either the truth or silence.
        let distro = parse_os_release("ID=foolinux\nID_LIKE=debian\nNAME=\"Foo Linux\"\n")
            .expect("resolves through ID_LIKE");
        assert_eq!(distro.id, "debian");
        assert_eq!(distro.name, "Foo Linux");
    }

    #[test]
    fn id_like_is_tried_in_order_so_the_closest_parent_wins() {
        let distro = parse_os_release("ID=whatever\nID_LIKE=\"fedora debian\"\n").expect("fedora");
        assert_eq!(distro.id, "fedora");
    }

    #[test]
    fn an_unknown_distribution_is_the_tarball_and_never_a_guess() {
        assert_eq!(parse_os_release(NIXOS), None);
        assert_eq!(parse_os_release(""), None);
        assert_eq!(parse_os_release("ID=\n"), None);
        assert_eq!(parse_os_release("nonsense without an equals sign"), None);
    }

    #[test]
    fn the_parents_name_is_the_floor_when_an_unknown_derivative_has_no_usable_name() {
        let distro = parse_os_release("ID=whatever\nID_LIKE=fedora\n").expect("fedora family");
        assert_eq!(distro.name, "Fedora");
        let empty =
            parse_os_release("ID=whatever\nID_LIKE=fedora\nNAME=\"\"\n").expect("fedora family");
        assert_eq!(empty.name, "Fedora", "an empty NAME is not a name");
    }

    #[test]
    fn values_are_unquoted_and_comments_ignored() {
        let text = "# a comment\n\nNAME='Arch Linux'\nID=arch\n";
        let distro = parse_os_release(text).expect("arch");
        assert_eq!(distro.id, "arch");
        assert_eq!(distro.name, "Arch Linux");
    }

    #[test]
    fn an_escaped_quote_inside_a_name_survives() {
        let distro = parse_os_release("ID=x\nID_LIKE=debian\nNAME=\"Bob\\\"s Linux\"\n")
            .expect("debian family");
        assert_eq!(distro.name, "Bob\"s Linux");
    }

    #[test]
    fn ids_are_matched_case_insensitively() {
        assert_eq!(
            parse_os_release("ID=Fedora\n").map(|d| d.id),
            Some("fedora".into())
        );
    }

    #[test]
    fn one_malformed_line_does_not_lose_the_distribution() {
        let distro = parse_os_release("garbage\nID=alpine\n").expect("alpine");
        assert_eq!(distro.id, "alpine");
    }

    #[test]
    fn a_missing_file_is_none_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(&dir.path().join("os-release")), None);
    }

    #[test]
    fn every_family_in_the_table_is_one_the_ui_has_a_command_for() {
        // The UI's install-command map is keyed on these five and nothing else; a sixth added here
        // without a command would render an empty command box.
        for (family, _, _) in KNOWN {
            assert!(
                ["debian", "fedora", "arch", "suse", "alpine"].contains(family),
                "unknown package family `{family}` — add its command to the copy deck first"
            );
        }
    }
}
