//! A source scan that holds the GUI crates' tests off the machine's real state (#102 phase 5a).
//!
//! `RuntimePaths::resolve` and `gui_config_path` ask the ENVIRONMENT where the daemon's config is
//! (`$XDG_CONFIG_HOME`, else `~/.config`). In a test that is the developer's own file, and a test
//! that saved even once afterwards held a state pointing at it: the next save wrote it, and the
//! `gui.toml` beside it (`gui_prefs_path` follows the managed config path). Nothing failed. The
//! fix is that every re-resolve and every test names its file (`RuntimePaths::resolve_at`); this is
//! what keeps it that way, because a convention that only a review enforces is gone by the next PR.
//!
//! **Exactly four lines in the whole crate may call either function**, listed below by file and
//! text. Anything else — in a test or out of one — fails, so a new call has to be argued for here,
//! in a diff a reviewer reads, rather than slipping in beside a `#[test]`.
//!
//! The search patterns are built at run time (`concat`ed from halves) so this file does not contain
//! them. The allow-list below spells the allowed lines out, so this file is the one the scan does not
//! read (it makes no call). Comment lines are skipped; a `/* */` block is not understood, and
//! a pattern inside one is reported (it fails closed).
//!
//! What it does **not** catch: a call reached without the name (a re-export under another name, a
//! macro that builds the path), and the environment read by hand (`std::env::var("XDG_CONFIG_HOME")`
//! in a test). The first is a review's job; the second would need its own pattern.

use std::path::{Path, PathBuf};

/// `(file, the line, trimmed)` — the only places allowed to name either function.
const ALLOWED: [(&str, &str); 4] = [
    // The one production caller: the process's startup, which has no session to take a path from.
    (
        "lib.rs",
        ".manage(Mutex::new(config_path::RuntimePaths::resolve()))",
    ),
    // The definitions, and the one place the definition of `resolve` asks the environment.
    ("config_path.rs", "pub fn resolve() -> Self {"),
    ("config_path.rs", "Self::resolve_at(&gui_config_path())"),
    ("config_path.rs", "pub fn gui_config_path() -> PathBuf {"),
];

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
    let roots = [manifest.join("src"), manifest.join("../gui-core/src")];
    // Built from halves so this file never contains them.
    let forbidden = [
        ["RuntimePaths", "::resolve("].concat(),
        ["gui_config", "_path("].concat(),
    ];
    let violations = scan(&roots, &forbidden);
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
