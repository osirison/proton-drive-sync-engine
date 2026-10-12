//! Nothing live tells a person to sign in with the wrong command (openspec `add-dependency-check`,
//! design D13, task 6.2).
//!
//! The CLI's sign-in is `proton-drive auth login`. The same words without `auth` were spelled in
//! the daemon, the control CLI, `setup.sh`, the GUI and the docs site for as long as nothing read
//! them back; a plain `git grep` also misses the form a line break splits in two. This reads the
//! live sources, joins their lines and fails on the wrong command, naming the file and the line.
//! It spawns nothing and adds no dependency.
//!
//! `docs/` and `openspec/` are not read: they are dated records and plans that quote the wrong
//! command on purpose.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

/// Directories read, relative to the crate root.
const ROOTS: [&str; 9] = [
    "src",
    "tests",
    "gui/src",
    "gui/src-tauri/src",
    "gui/gui-core/src",
    "gui/tools",
    "website/src/content",
    "packaging",
    "examples",
];

/// Single files read.
const FILES: [&str; 4] = ["README.md", "setup.sh", "upgrade.sh", "uninstall.sh"];

/// Directories never entered: dependencies and build output are not ours.
const SKIPPED_DIRECTORIES: [&str; 3] = ["node_modules", "target", ".git"];

/// The wrong command, built from parts so that this file does not match itself.
fn needle() -> String {
    ["proton-drive", "login"].join(" ")
}

/// `text` as one line of lowercase words, plus the source line of every byte of it.
///
/// * a line break is whitespace, and so is a trailing `\` (a string or shell continuation);
/// * a comment or quote leader (`//`, `///`, `//!`, `#`, `*`, `>`) at the start of a line is
///   dropped, so a sentence a comment wraps reads as one;
/// * backticks and quotes are dropped, so a command in code formatting or in
///   quotes matches like a bare one;
/// * runs of whitespace collapse to one space.
fn normalised(text: &str) -> (String, Vec<usize>) {
    let mut joined = String::new();
    let mut lines = Vec::new();
    let mut push = |c: char, line: usize| {
        let space = c.is_whitespace();
        if space && joined.ends_with(' ') {
            return;
        }
        let c = if space { ' ' } else { c.to_ascii_lowercase() };
        joined.push(c);
        lines.extend(std::iter::repeat_n(line, c.len_utf8()));
    };
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim_end();
        let line = line.strip_suffix('\\').unwrap_or(line);
        let body = line.trim_start_matches(|c: char| c.is_whitespace() || "/#*>!".contains(c));
        for c in body.chars().filter(|c| !"`\"'".contains(*c)) {
            push(c, number);
        }
        push(' ', number);
    }
    (joined, lines)
}

/// The source line of each occurrence of the wrong command in `text`.
fn wrong_commands(text: &str) -> Vec<usize> {
    let (joined, lines) = normalised(text);
    let needle = needle();
    joined
        .match_indices(&needle)
        .map(|(at, _)| lines[at])
        .collect()
}

fn files_under(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if !SKIPPED_DIRECTORIES.iter().any(|skipped| name == *skipped) {
                files_under(&path, found);
            }
        } else {
            found.push(path);
        }
    }
}

#[test]
fn no_live_source_names_the_wrong_sign_in_command() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for directory in ROOTS {
        files_under(&root.join(directory), &mut files);
    }
    files.extend(FILES.iter().map(|file| root.join(file)));

    let mut offences = Vec::new();
    let mut read = 0;
    for file in &files {
        // A file that is not UTF-8 text (an image, a font) cannot hold the command.
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        read += 1;
        for line in wrong_commands(&text) {
            offences.push(format!(
                "{}:{line}",
                file.strip_prefix(root).unwrap_or(file).display()
            ));
        }
    }
    assert!(
        read > 100,
        "the scan read only {read} files: are the roots right?"
    );
    assert!(
        offences.is_empty(),
        "the sign-in command is `proton-drive auth login`; these name another one:\n  {}",
        offences.join("\n  ")
    );
}

#[test]
fn the_scan_sees_the_forms_a_plain_grep_misses() {
    let needle = needle();
    let (first, second) = needle.split_once(' ').expect("two words");
    let at_line = |text: String| wrong_commands(&text);

    assert_eq!(at_line(format!("run `{needle}` now")), [1]);
    assert_eq!(at_line(format!("Run `{first}\n  {second}`; then")), [1]);
    assert_eq!(at_line(format!("// {first}\n// {second}")), [1]);
    assert_eq!(at_line(format!("/// {first}\n///   {second}")), [1]);
    assert_eq!(at_line(format!("x\n> {first}\n> {second}")), [2]);
    assert_eq!(at_line(format!("\"{first} \\\n     {second}\"")), [1]);
    assert_eq!(at_line(format!("err 'commonly: run \\'{needle}\\''")), [1]);
    assert_eq!(at_line(format!("{first}   \t{second}")), [1]);

    // The right command, and the words in another order, are not the wrong one.
    assert!(at_line(format!("{first} auth {second}")).is_empty());
    assert!(at_line(format!("{first} auth\n  {second}")).is_empty());
    assert!(at_line(format!("{second} {first}")).is_empty());
}
