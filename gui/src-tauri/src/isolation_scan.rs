//! A source scan that holds the GUI crates' tests off the machine's real state (#102 phase 5a).
//!
//! `RuntimePaths::resolve` and `gui_config_path` ask the ENVIRONMENT where the daemon's config is
//! (`$XDG_CONFIG_HOME`, else `~/.config`). In a test that is the developer's own file, and a test
//! that saved even once afterwards held a state pointing at it: the next save wrote it, and the
//! `gui.toml` beside it (`gui_prefs_path` follows the managed config path). Nothing failed. The
//! fix is that every re-resolve and every test names its file (`RuntimePaths::resolve_at`); this is
//! what keeps it that way, because a convention that only a review enforces is gone by the next PR.
//!
//! **Exactly five lines in the whole crate may reach the environment** (four for the config path,
//! one for the control socket), listed below by file and text. Anything else — in a test or out of
//! one — fails, so a new call has to be argued for here, in a diff a reviewer reads, rather than
//! slipping in beside a `#[test]`.
//!
//! **What counts as reaching it is a shape, not two spellings.** The scan began as two literal
//! strings and an `as` walked past both: `use ...::RuntimePaths as Rp;` then `Rp::resolve()`. It
//! now holds three routes, whatever the names are renamed to ([`reaches_the_environment`]): the
//! config-path function by name, a zero-argument `::resolve` on any path (the one constructor that
//! asks the environment, and the only `resolve` that takes nothing), and an alias of the type. The
//! literal patterns stay as well, and `src-tauri` additionally forbids naming the control socket's
//! environment default, which `RuntimePaths::resolve_at` refuses in a test build
//! (`config_path::environment_default_socket`) and which a test could otherwise write in by hand.
//!
//! The search patterns are built at run time (`concat`ed from halves) so this file does not contain
//! them. The allow-list below spells the allowed lines out, so this file is the one the scan does not
//! read (it makes no call). Comment lines are skipped; a `/* */` block is not understood, and
//! a pattern inside one is reported (it fails closed).
//!
//! What it does **not** catch: a macro that builds the path, a `use` glob that renames nothing and
//! reaches the environment through a function this file does not know the name of, and the
//! environment read by hand (`std::env::var("XDG_CONFIG_HOME")` in a test). The first two are a
//! review's job; the third would need its own pattern.

use std::path::{Path, PathBuf};

/// `(file, the line, trimmed)` — the only places allowed to reach the environment.
const ALLOWED: [(&str, &str); 5] = [
    // The one production caller: the process's startup, which has no session to take a path from.
    (
        "lib.rs",
        ".manage(Mutex::new(config_path::RuntimePaths::resolve()))",
    ),
    // The definitions, and the one place the definition of `resolve` asks the environment.
    ("config_path.rs", "pub fn resolve() -> Self {"),
    ("config_path.rs", "Self::resolve_at(&gui_config_path())"),
    ("config_path.rs", "pub fn gui_config_path() -> PathBuf {"),
    // The production socket default. The test build's twin of this function refuses instead.
    ("config_path.rs", "gui_core::ipc::default_socket_path()"),
];

/// Whether `c` can be part of an identifier.
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `word` occurs in `text` as a whole identifier: `gui_config_path` is, `gui_config_path_in`
/// is not.
fn names_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// A `::resolve` that takes nothing: called with an empty argument list, or not called at all (taken
/// as a function value). `resolve_at`, `resolve_pair` and any `resolve(argument)` are different
/// functions and do not match.
fn names_a_zero_argument_resolve(text: &str) -> bool {
    let needle = "::resolve";
    text.match_indices(needle).any(|(start, _)| {
        let rest = &text[start + needle.len()..];
        if rest.chars().next().is_some_and(is_ident) {
            return false;
        }
        match rest.strip_prefix('(') {
            // Something between the parentheses: another function by the same name. A line that ends
            // inside them is read as the empty call it most likely is, because failing closed costs
            // one allow-list line and failing open costs the real config.
            Some(arguments) => {
                let arguments = arguments.trim_start();
                arguments.is_empty() || arguments.starts_with(')')
            }
            None => true,
        }
    })
}

/// A second name for `RuntimePaths`: `... RuntimePaths as Rp` or `type Rp = ...::RuntimePaths;`. The
/// type inside another (`type Paths<'a> = State<'a, Mutex<RuntimePaths>>`) is a use, not an alias.
fn aliases_the_runtime_paths(text: &str) -> bool {
    let word = "RuntimePaths";
    let renamed = text.match_indices(word).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let rest = &text[start + word.len()..];
        !before.is_some_and(is_ident) && rest.trim_start().starts_with("as ")
    });
    let declared = {
        let without_visibility = text
            .trim_start()
            .trim_start_matches("pub(crate)")
            .trim_start_matches("pub")
            .trim_start();
        without_visibility
            .strip_prefix("type ")
            .is_some_and(|rest| {
                rest.split_once('=').is_some_and(|(_, target)| {
                    target
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .rsplit("::")
                        .next()
                        == Some(word)
                })
            })
    };
    renamed || declared
}

/// The three routes to the environment's config, independent of what anything is called.
fn reaches_the_environment(text: &str) -> bool {
    names_word(text, &["gui_config", "_path"].concat())
        || names_a_zero_argument_resolve(text)
        || aliases_the_runtime_paths(text)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn scan(roots: &[PathBuf], forbidden: &[String]) -> Vec<String> {
    let mut files = Vec::new();
    for root in roots {
        rust_files(root, &mut files);
    }
    assert!(
        files.len() > 10,
        "the scan found {} files: it is looking in the wrong place",
        files.len()
    );
    let mut violations = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        // This file states the rule, so its allow-list names the lines it allows. It makes no call.
        if name == "isolation_scan.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap();
        for (number, line) in source.lines().enumerate() {
            let text = line.trim();
            if text.starts_with("//") {
                continue;
            }
            // `fn resolve()` and `fn gui_config_path()` are the DEFINITIONS and spell the name
            // without the path prefix; the allow-list names them. Every other mention is a call.
            if !forbidden
                .iter()
                .any(|pattern| text.contains(pattern.as_str()))
                && !reaches_the_environment(text)
            {
                continue;
            }
            if ALLOWED.contains(&(name.as_str(), text)) {
                continue;
            }
            violations.push(format!("{name}:{}: {text}", number + 1));
        }
    }
    violations
}

#[test]
fn no_gui_test_resolves_the_real_environment() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Built from halves so this file never contains them.
    let mut violations = scan(&[manifest.join("src")], &{
        let mut patterns = literal_patterns();
        patterns.extend(tauri_only_patterns());
        patterns
    });
    violations.extend(scan(
        &[manifest.join("../gui-core/src")],
        &literal_patterns(),
    ));
    assert!(
        violations.is_empty(),
        "these lines ask the environment where the config is. Name the file instead \
         (`RuntimePaths::resolve_at(<a path in a temp dir>)`), or add the line to ALLOWED with the \
         reason:\n{}",
        violations.join("\n")
    );
}

/// The scan must be able to fail: shown against a source that breaks the rule.
#[test]
fn the_scan_sees_a_call_it_has_not_been_told_about() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..11 {
        std::fs::write(dir.path().join(format!("f{i}.rs")), "fn ok() {}\n").unwrap();
    }
    std::fs::write(
        dir.path().join("bad.rs"),
        format!(
            "#[test]\nfn t() {{\n    let _ = {}::{}();\n}}\n",
            "RuntimePaths", "resolve"
        ),
    )
    .unwrap();
    let forbidden = [["RuntimePaths", "::resolve("].concat()];
    let found = scan(&[dir.path().to_owned()], &forbidden);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("bad.rs:3"), "{found:?}");
}

/// What the scan reports for a source of one file, under the same literal patterns the real scan
/// uses. Padded to eleven files because the scan refuses to trust a directory with fewer.
fn scan_source(source: &str) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..11 {
        std::fs::write(dir.path().join(format!("f{i}.rs")), "fn ok() {}\n").unwrap();
    }
    std::fs::write(dir.path().join("bad.rs"), source).unwrap();
    scan(&[dir.path().to_owned()], &literal_patterns())
}

/// The literal patterns both the real scan and its probes use.
fn literal_patterns() -> Vec<String> {
    vec![
        ["RuntimePaths", "::resolve("].concat(),
        ["gui_config", "_path("].concat(),
    ]
}

/// Patterns for `src-tauri` alone: `gui-core` defines the socket default and tests it, and has no
/// command to misdirect.
fn tauri_only_patterns() -> Vec<String> {
    vec![["default_socket", "_path("].concat()]
}

/// The scan used to match those two literals and nothing else, so renaming the type — one `as` —
/// walked past it: `use ...::RuntimePaths as Rp;` then `Rp::resolve()` resolved the environment's
/// config in a test and the scan stayed green (reproduced by the review of PR #436). What it holds
/// now is the *shape* of the three routes in, each of which has to be spelled somewhere a line can
/// be read:
///
/// 1. the config-path function by name, whatever it is renamed to afterwards (`... as where`);
/// 2. a zero-argument `::resolve` on ANY path or alias, called or taken as a value — the one
///    constructor that asks the environment, and the only `resolve` that takes nothing;
/// 3. an alias of the type itself, by `as` or by `type X = ...`.
///
/// Each line below is its own source, so a rule that catches the pair only by catching one half is
/// not credited with both.
#[test]
fn the_scan_sees_the_environment_reached_through_an_alias() {
    let must_be_flagged = [
        "use crate::config_path::RuntimePaths as Rp;",
        "use crate::config_path::{PairRef, RuntimePaths as Rp};",
        "    let _ = Rp::resolve();",
        "    let _ = config_path::RuntimePaths::resolve();",
        "    let _ = <Rp>::resolve();",
        "    let load = Rp::resolve;",
        "    let build = [0].map(|_| Rp::resolve ());",
        "type Rp = crate::config_path::RuntimePaths;",
        "pub(crate) type Paths = RuntimePaths;",
        "use crate::config_path::gui_config_path as where_it_is;",
        "    let load = gui_config_path;",
    ];
    for line in must_be_flagged {
        let found = scan_source(&format!("#[test]\nfn t() {{\n{line}\n}}\n"));
        assert_eq!(found.len(), 1, "not flagged: {line}\n{found:?}");
        assert!(found[0].starts_with("bad.rs:3"), "{line}: {found:?}");
    }
}

/// And the rules are no wider than the thing they hold: every use that names a file, or does
/// something other than ask the environment, stays legal. A scan that cried wolf at
/// `resolve_at` would be argued out of by the next person who met it.
#[test]
fn the_scan_leaves_every_use_that_names_its_file_alone() {
    let fine = [
        "    let paths = RuntimePaths::resolve_at(&dir.path().join(\"proton-sync.toml\"));",
        "    let paths = Rp::resolve_at(&path);",
        "    let paths = RuntimePaths::resolve_with_default_socket(&path, default);",
        "    let pair = paths.resolve_pair(None).unwrap();",
        "    let path = gui_config_path_in(Some(home), None);",
        "    let paths: RuntimePaths = Default::default();",
        "use crate::config_path::{PairRef, RuntimePaths};",
        "type Shared = Mutex<RuntimePaths>;",
        "    let answer = resolver.resolve(&request);",
        "    // RuntimePaths::resolve() is only ever named in a comment.",
    ];
    for line in fine {
        let found = scan_source(&format!("#[test]\nfn t() {{\n{line}\n}}\n"));
        assert!(found.is_empty(), "flagged but legal: {line}\n{found:?}");
    }
}

/// Seen from the other side: a test that names the environment's control socket itself, instead of
/// getting the refusal `resolve_at` gives it, is a deliberate act and the scan says so. This one is
/// for `src-tauri` only (see `no_gui_test_resolves_the_real_environment`): `gui-core` defines and
/// tests the default.
#[test]
fn the_scan_sees_a_test_that_dials_the_environments_socket_on_purpose() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..11 {
        std::fs::write(dir.path().join(format!("f{i}.rs")), "fn ok() {}\n").unwrap();
    }
    std::fs::write(
        dir.path().join("bad.rs"),
        "fn t() {\n    paths.socket_path = gui_core::ipc::default_socket_path();\n}\n",
    )
    .unwrap();
    let found = scan(&[dir.path().to_owned()], &tauri_only_patterns());
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("bad.rs:2"), "{found:?}");
}
