//! The install scripts' reading of a config the app wrote (#102 phase 5b-2, brief §3.2 / B3).
//!
//! `uninstall.sh` finds each pair's folder with `config_values` (`setup.sh`): a line grep for a key
//! that starts its own line, in a basic or a literal string, with no unescaping and no notion of a
//! table. The daemon reads the same file with a TOML parser, and **a pair the grep cannot see is a
//! `.sync` directory the uninstaller leaves behind** (or, read wrongly, one it purges that is not the
//! pair's). So the differential: for every file `promote_to_pair_tables`, `add_pair` and
//! `remove_pair` produce, the roots the shell lists are exactly the roots the engine's `pair_views`
//! returns, in file order.
//!
//! **Nothing here touches the machine.** The only thing run is `bash`, with a cleared environment, to
//! call the one function `config_values` over a temporary file; `setup.sh`'s `main` is guarded, so
//! sourcing it only defines functions (the sandbox of the engine's own `tests/scripts.rs`).
#![cfg(unix)]

use proton_sync_gui_core::config_io::{ConfigDoc, PairInit, expand_config_path, pair_views};
use std::path::Path;
use std::process::Command;

fn setup_script() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../setup.sh")
}

/// What `config_values FILE KEY` prints, one value per line, in a cleared environment.
fn shell_values(directory: &Path, text: &str, key: &str) -> Vec<String> {
    let file = directory.join("config.toml");
    std::fs::write(&file, text).expect("write the config");
    let output = Command::new("bash")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", directory)
        .arg("-c")
        .arg("source \"$1\"; config_values \"$2\" \"$3\"")
        .arg("bash")
        .arg(setup_script())
        .arg(&file)
        .arg(key)
        .output()
        .expect("run bash");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The corpus of single-pair files shared with the unit tests.
fn corpus() -> Vec<(String, String)> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/promotion-corpus");
    let mut files: Vec<(String, String)> = std::fs::read_dir(directory)
        .expect("corpus")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "toml")
                && !path.to_string_lossy().ends_with(".promoted.toml")
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&path).expect("read"),
            )
        })
        .collect();
    files.sort();
    files
}

/// The roots the shell CAN see, as the ENGINE reads them: `~` expanded the way it expands them, and
/// only for a pair whose table spells the key `local_root`.
///
/// **The grep knows one spelling.** `config_values` matches `local_root` and nothing else, so a
/// hand-written `local-root = …` (a spelling the daemon accepts) is invisible to `uninstall.sh`,
/// before a promotion and after it. That is `setup.sh`'s limit and not a thing a rewrite can fix, so
/// the differential asks the question that can be answered: the shell sees exactly the pairs that
/// spell the key its way, none lost to a promotion, none lost to an add, in the engine's order.
fn engine_roots(text: &str) -> Vec<String> {
    let root: toml::Table = text.parse().expect("toml");
    let spelled_for_the_shell: Vec<bool> = match root.get("pair").and_then(toml::Value::as_array) {
        Some(tables) => tables
            .iter()
            .map(|table| table.get("local_root").is_some())
            .collect(),
        None => vec![root.contains_key("local_root")],
    };
    pair_views(text)
        .expect("the engine reads the file")
        .into_iter()
        .zip(spelled_for_the_shell)
        .filter(|(_, visible)| *visible)
        .filter_map(|(view, _)| view.local_root)
        .map(|root| root.display().to_string())
        .collect()
}

/// The roots the SHELL lists, put through the same `~` expansion (the scripts expand it themselves,
/// later, with the shell's own rule; the point here is which strings they find, in which order).
fn shell_roots(directory: &Path, text: &str) -> Vec<String> {
    shell_values(directory, text, "local_root")
        .into_iter()
        .map(|root| expand_config_path(root, "local_root").display().to_string())
        .collect()
}

fn init(name: &str, local_root: &str) -> PairInit {
    PairInit {
        name: name.to_owned(),
        local_root: local_root.to_owned(),
        remote_root: format!("/Drive/{name}"),
        exclude: vec!["**/*.cache".to_owned()],
    }
}

#[test]
fn the_shell_reader_and_pair_views_agree() {
    let directory = tempfile::tempdir().expect("tempdir");
    let mut checked = 0;
    for (name, text) in corpus() {
        // The input itself: the scripts have always read it.
        assert_eq!(
            shell_roots(directory.path(), &text),
            engine_roots(&text),
            "{name} as written"
        );

        // Promoted, then added to twice, then the first and the middle removed again — a file the app
        // writes only ever goes through these four operations, so the grep sees every state of it.
        let mut document = ConfigDoc::from_toml_str(&text).expect("toml");
        document.promote_to_pair_tables().expect(&name);
        let mut states = vec![("promoted", document.to_toml_string())];
        // `dry_run = true` and an empty file refuse a second pair (the engine's own rule); the
        // promoted state above is what is checked for them.
        if document
            .add_pair(init("added-one", "/home/u/Added One"))
            .is_ok()
        {
            states.push(("one added", document.to_toml_string()));
            document
                .add_pair(init("added-two", "/home/u/it's a folder"))
                .expect(&name);
            states.push(("two added", document.to_toml_string()));
            document.remove_pair("added-one").expect(&name);
            states.push(("middle removed", document.to_toml_string()));
        }
        for (step, state) in states {
            assert_eq!(
                shell_roots(directory.path(), &state),
                engine_roots(&state),
                "{name}, {step}:\n{state}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "the differential ran over {checked} states");
}

#[test]
fn an_added_root_is_listed_whatever_quote_style_the_value_needs() {
    // A space and an apostrophe are legal in a folder name and need no escape in a basic string.
    let directory = tempfile::tempdir().expect("tempdir");
    let mut document =
        ConfigDoc::from_toml_str("local_root = \"/home/u/A\"\nremote_root = \"/Drive/A\"\n")
            .expect("toml");
    document
        .add_pair(init("quoted", "/home/u/Bob's Files (2024) #1"))
        .expect("add");
    let text = document.to_toml_string();
    assert_eq!(
        shell_values(directory.path(), &text, "local_root"),
        ["/home/u/A", "/home/u/Bob's Files (2024) #1"],
        "{text}"
    );
}
