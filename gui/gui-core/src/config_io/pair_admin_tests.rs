//! Adding, removing and promoting folder pairs (#102 phase 5b-2), over the corpus of single-pair files
//! the repo knows how to write (`testdata/promotion-corpus/`).
//!
//! The oracles are not this module's: the daemon's own resolver says whether a result starts, the
//! engine's `pair_views` says what it reads, `toml` (not the editor that did the moving) says which
//! settings are where, and — in `tests/shell_reader.rs` — the install scripts' reader says what
//! `uninstall.sh` will find.

use super::*;
use proton_drive_sync_engine::config::{DaemonConfigInput, resolve_runtime_configs};
use std::fs;

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/promotion-corpus");

/// `(file name, text)` for every single-pair file in the corpus, in name order. Files that end in
/// `.promoted.toml` are the reviewed expected OUTPUT of the promotion, not inputs.
fn corpus() -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = fs::read_dir(CORPUS)
        .expect("the corpus directory")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "toml")
                && !path.to_string_lossy().ends_with(".promoted.toml")
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(&path).expect("read a corpus file"),
            )
        })
        .collect();
    files.sort();
    assert!(
        files.len() >= 12,
        "the corpus is missing files: {}",
        files.len()
    );
    files
}

fn doc(text: &str) -> ConfigDoc {
    ConfigDoc::from_toml_str(text).expect("a corpus file is TOML")
}

fn init(name: &str, local_root: &str, remote_root: &str) -> PairInit {
    PairInit {
        name: name.to_owned(),
        local_root: local_root.to_owned(),
        remote_root: remote_root.to_owned(),
        exclude: Vec::new(),
    }
}

/// Which per-pair spelling of a key a root-level entry is, by the ENGINE's own table.
fn scope_of(key: &str) -> Option<KeyScope> {
    ConfigKey::from_spelling(key).map(ConfigKey::scope)
}

/// The daemon's own resolver, as the binary calls it, over `text`. The pair names it resolved.
fn resolved_pair_names(text: &str) -> Result<Vec<String>, String> {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("proton-sync.toml");
    fs::write(&path, text).expect("write config");
    resolve_runtime_configs(DaemonConfigInput {
        config: Some(path),
        ..DaemonConfigInput::default()
    })
    .map(|configs| configs.pairs.iter().map(|pair| pair.name.clone()).collect())
    .map_err(|error| error.to_string())
}

// ---- promotion ---------------------------------------------------------------------------------------

#[test]
fn promote_is_meaning_preserving() {
    for (name, text) in corpus() {
        let mut promoted = doc(&text);
        promoted
            .promote_to_pair_tables()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let after = promoted.to_toml_string();
        assert_eq!(promoted.layout(), PairLayout::Tables, "{name}");
        // The two oracles that are not this module's.
        validate_file_config_text_ok(&after, &name);
        assert_eq!(
            pair_views(&after).unwrap(),
            pair_views(&text).unwrap(),
            "{name}: the engine reads a different pair after the promotion"
        );
        // The daemon's own verdict is the same before and after: it starts on the result exactly
        // when it started on the input, and a file with no folder yet is refused in the same words.
        assert_eq!(
            resolved_pair_names(&after),
            resolved_pair_names(&text),
            "{name}: the daemon's resolver reads the promoted file differently"
        );
        // Key for key, by `toml` and not by the editor: the same settings, now in `pair[0]`.
        let (old, new) = (parse(&text), parse(&after));
        let table = new["pair"].as_array().expect("[[pair]]")[0]
            .as_table()
            .expect("a table")
            .clone();
        assert_eq!(table["name"].as_str(), Some(DEFAULT_PAIR_NAME), "{name}");
        for (key, value) in &old {
            match scope_of(key) {
                Some(KeyScope::Pair) => {
                    assert_eq!(
                        table.get(key),
                        Some(value),
                        "{name}: {key} must move, intact"
                    );
                    assert!(
                        !new.contains_key(key),
                        "{name}: {key} must leave the top level"
                    );
                }
                _ => assert_eq!(new.get(key), Some(value), "{name}: {key} must stay put"),
            }
        }
        assert_eq!(
            table.len(),
            1 + old
                .keys()
                .filter(|key| scope_of(key) == Some(KeyScope::Pair))
                .count(),
            "{name}: the table holds its name and exactly the per-pair keys"
        );
    }
}

fn validate_file_config_text_ok(text: &str, name: &str) {
    proton_drive_sync_engine::config::validate_file_config_text(text)
        .unwrap_or_else(|error| panic!("{name}: the engine refuses the result: {error}"));
}

fn parse(text: &str) -> toml::Table {
    text.parse().expect("valid TOML")
}

#[test]
fn a_daemon_scope_key_stays_at_the_root() {
    // After a promotion the top level holds daemon-wide keys and the `pair` array and NOTHING else;
    // before the table, in the file's own order, byte for byte.
    for (name, text) in corpus() {
        let mut promoted = doc(&text);
        promoted.promote_to_pair_tables().unwrap();
        let after = promoted.to_toml_string();
        let root: toml::Table = parse(&after);
        for key in root.keys() {
            assert!(
                key == "pair" || scope_of(key) == Some(KeyScope::Daemon),
                "{name}: `{key}` is left at the top level"
            );
        }
        let before_the_table = after.split("[[pair]]").next().unwrap();
        for line in text.lines().filter(|line| {
            let key = line.split('=').next().unwrap_or("").trim();
            scope_of(key) == Some(KeyScope::Daemon) && line.contains('=')
        }) {
            assert!(
                before_the_table.contains(&format!("{line}\n")),
                "{name}: the daemon-wide line {line:?} was changed or moved:\n{after}"
            );
        }
        // Their order is the file's.
        let positions: Vec<usize> = text
            .lines()
            .filter(|line| {
                scope_of(line.split('=').next().unwrap_or("").trim()) == Some(KeyScope::Daemon)
            })
            .filter_map(|line| before_the_table.find(line))
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{name}: reordered"
        );
    }
}

/// Every comment (a `#` to the end of its line) in `text`, in order.
fn comments(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.find('#').map(|at| line[at..].trim_end().to_owned()))
        .collect()
}

#[test]
fn promote_keeps_the_bytes_it_did_not_move() {
    for (name, text) in corpus() {
        let mut promoted = doc(&text);
        promoted.promote_to_pair_tables().unwrap();
        let after = promoted.to_toml_string();
        // No comment is lost, none is made up, and none changes a character — only where it sits.
        let (mut before, mut now) = (comments(&text), comments(&after));
        before.sort();
        now.sort();
        assert_eq!(
            before, now,
            "{name}: the comments differ after the promotion"
        );
        // The text's trailing newline (or lack of one) is the file's own.
        if !text.is_empty() {
            assert_eq!(
                text.ends_with('\n'),
                after.ends_with('\n'),
                "{name}: the trailing newline changed"
            );
        }
        // And the reviewed output, for the files that have one: the bytes a person would see.
        let golden = format!("{CORPUS}/{}.promoted.toml", name.trim_end_matches(".toml"));
        if let Ok(expected) = fs::read_to_string(&golden) {
            assert_eq!(
                after, expected,
                "{name}: the promoted bytes differ from the reviewed ones"
            );
        }
    }
}

#[test]
fn promote_moves_a_comment_with_its_key_and_leaves_a_daemon_comment_where_it_was() {
    let mut document = doc(
        "# about the socket\nsocket_path = \"/run/s.sock\"\n\n# about the folder\n\
         local_root = \"/x\"   # trailing\nremote_root = \"/Drive/x\"\n",
    );
    document.promote_to_pair_tables().unwrap();
    assert_eq!(
        document.to_toml_string(),
        "# about the socket\nsocket_path = \"/run/s.sock\"\n\n[[pair]]\nname = \"default\"\n\
         \n# about the folder\nlocal_root = \"/x\"   # trailing\nremote_root = \"/Drive/x\"\n"
    );
}

#[test]
fn a_file_header_stays_at_the_top_and_a_comment_on_a_key_goes_with_it() {
    // Separated from the first key by a blank line, it is the file's: it stays above everything,
    // including when the daemon-wide key that follows had a comment of its own.
    let mut document = doc(
        "# title of the file\n\n# about the folder\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n\n\
         # about logging\nlog_level = \"debug\"\n",
    );
    document.promote_to_pair_tables().unwrap();
    assert_eq!(
        document.to_toml_string(),
        "# title of the file\n\n# about logging\nlog_level = \"debug\"\n\n[[pair]]\nname = \"default\"\n\
         # about the folder\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n"
    );
    // Directly on the key, with no blank line between, it is the key's.
    let mut attached = doc("# the folder\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n");
    attached.promote_to_pair_tables().unwrap();
    assert_eq!(
        attached.to_toml_string(),
        "[[pair]]\nname = \"default\"\n# the folder\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n"
    );
}

#[test]
fn promotion_is_a_no_op_on_tables_and_refused_on_an_inline_array() {
    let mut tables =
        doc("[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n");
    let before = tables.to_toml_string();
    tables.promote_to_pair_tables().unwrap();
    assert_eq!(tables.to_toml_string(), before);

    let mut inline =
        doc("pair = [{ name = \"a\", local_root = \"/a\", remote_root = \"/Drive/a\" }]\n");
    let before = inline.to_toml_string();
    assert!(matches!(
        inline.promote_to_pair_tables(),
        Err(ConfigError::InlinePairs { .. })
    ));
    assert_eq!(inline.to_toml_string(), before);
}

#[test]
fn a_promotion_that_the_engine_would_refuse_is_refused_and_leaves_the_document_alone() {
    // Not a promotion problem: the input is a config the daemon would not start on (a relative
    // socket). Promoting it must not paper over that, and must not half-apply.
    let mut broken =
        doc("socket_path = \"relative.sock\"\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n");
    let before = broken.to_toml_string();
    let error = broken.promote_to_pair_tables().unwrap_err();
    assert!(matches!(error, ConfigError::Invalid(_)), "{error}");
    assert_eq!(broken.to_toml_string(), before);
}

// ---- adding ------------------------------------------------------------------------------------------

/// The roots the corpus' own files use all start `/home/u/`; this one is apart from every one.
fn the_new_pair() -> PairInit {
    PairInit {
        exclude: vec!["**/*.cache".to_owned()],
        ..init("added", "/home/u/Added", "/Drive/Added")
    }
}

/// Files whose single pair cannot take a second one, and the words of the engine's refusal.
fn refused_second_pair(name: &str) -> Option<&'static str> {
    match name {
        // No folder at all yet: "every `[[pair]]` table must set both `local_root` and `remote_root`".
        "08-empty.toml" | "09-daemon-keys-only.toml" => {
            Some("must set both `local_root` and `remote_root`")
        }
        // M3: a preview rehearses one pair.
        "10-every-pair-key.toml" => {
            Some("`dry_run = true`, which cannot be used with 2 folder pairs")
        }
        _ => None,
    }
}

#[test]
fn add_pair_output_is_accepted_by_the_daemons_resolver_with_two_pairs() {
    for (name, text) in corpus() {
        let mut document = doc(&text);
        let result = document.add_pair(the_new_pair());
        if let Some(needle) = refused_second_pair(&name) {
            let error = result.expect_err(&format!("{name}: a second pair must be refused"));
            assert!(error.to_string().contains(needle), "{name}: {error}");
            assert_eq!(
                document.to_toml_string(),
                text,
                "{name}: a refusal leaves the document alone"
            );
            continue;
        }
        result.unwrap_or_else(|error| panic!("{name}: {error}"));
        let after = document.to_toml_string();
        let names = resolved_pair_names(&after)
            .unwrap_or_else(|error| panic!("{name}: the daemon's resolver refuses it: {error}"));
        assert_eq!(names, ["default", "added"], "{name}");
        assert_eq!(document.layout(), PairLayout::Tables, "{name}");
        // The pair that was there is still the pair that was there.
        let (before, now) = (pair_views(&text).unwrap(), pair_views(&after).unwrap());
        assert_eq!(&now[..1], &before[..], "{name}: the first pair changed");
        assert_eq!(
            now[1].local_root.as_deref(),
            Some(Path::new("/home/u/Added")),
            "{name}"
        );
    }
}

#[test]
fn a_third_pair_is_appended_after_the_second_and_changes_no_byte_before_it() {
    let mut document = doc(
        "# hand-written\nlog_level = \"info\"\n\n# first\n[[pair]]\nname = \"documents\"\n\
         local_root = \"/home/me/Documents\"   # docs root\nremote_root = \"/Drive/Docs\"\n\n\
         # second\n[[pair]]\nname = \"photos\"\nlocal_root = \"/home/me/Pictures\"\n\
         remote_root = \"/Drive/Photos\"\nscan_interval_secs = 600\n",
    );
    let before = document.to_toml_string();
    document
        .add_pair(init("music", "/home/me/Music", "/Drive/Music"))
        .unwrap();
    let after = document.to_toml_string();
    let appended = after
        .strip_prefix(&before)
        .unwrap_or_else(|| panic!("an add must only append:\n{after}"));
    assert_eq!(
        appended,
        "\n[[pair]]\nname = \"music\"\nlocal_root = \"/home/me/Music\"\nremote_root = \"/Drive/Music\"\n"
    );
    assert_eq!(document.pair_names(), ["documents", "photos", "music"]);
}

#[test]
fn the_added_table_is_one_scalar_per_line_with_no_inline_table_and_no_dotted_key() {
    let mut document = doc("local_root = \"/home/u/A\"\nremote_root = \"/Drive/A\"\n");
    document.add_pair(the_new_pair()).unwrap();
    let after = document.to_toml_string();
    let added = after.rsplit("[[pair]]").next().unwrap();
    assert_eq!(
        added,
        "\nname = \"added\"\nlocal_root = \"/home/u/Added\"\nremote_root = \"/Drive/Added\"\n\
         exclude = [\"**/*.cache\"]\n"
    );
    for line in after.lines() {
        assert!(
            !line.contains('{') && !line.split('=').next().unwrap_or("").trim().contains('.')
                || line.trim_start().starts_with('['),
            "an inline table or a dotted key was written: {line:?}"
        );
    }
}

#[test]
fn a_failed_add_leaves_the_file_untouched() {
    // Every way an add can be refused, against both an implicit file (which an add promotes FIRST,
    // so a refusal after the promotion is the interesting case) and a table file. The document, and
    // so any file saved from it, is byte-identical afterwards.
    let implicit = "# keep me\nlocal_root = \"/home/u/A\"\nremote_root = \"/Drive/A\"\nlog_level = \"debug\"\n";
    let tables = "log_level = \"debug\"\n\n[[pair]]\nname = \"a\"\nlocal_root = \"/home/u/A\"\n\
                  remote_root = \"/Drive/A\"\n";
    let bad: Vec<(PairInit, &str)> = vec![
        (init("a b", "/home/u/B", "/Drive/B"), "may use only"),
        (init("..", "/home/u/B", "/Drive/B"), "path component"),
        (init("-x", "/home/u/B", "/Drive/B"), "starts with `-`"),
        (init("b", "/home/u/A/inside", "/Drive/B"), "is inside"),
        (init("b", "/home/u/B", "/Drive/A"), "remote_root"),
        (init("b", "", "/Drive/B"), "local_root"),
        (init("b", "/home/u/B", ""), "remote_root"),
        (init("b", "/home/u/B\n", "/Drive/B"), "control character"),
        (init("b", "/home/u/\"B\"", "/Drive/B"), "do not unescape"),
        (init("b", "/home/u/B", "/Drive/\\B"), "do not unescape"),
    ];
    for (text, same_name_in_another_case) in [(implicit, "DEFAULT"), (tables, "A")] {
        let mut bad = bad.clone();
        bad.push((
            init(same_name_in_another_case, "/home/u/B", "/Drive/B"),
            "without regard to case",
        ));
        for (candidate, needle) in &bad {
            let mut document = doc(text);
            let error = document
                .add_pair(candidate.clone())
                .expect_err(&format!("{candidate:?} must be refused"))
                .to_string();
            assert!(
                error.contains(needle),
                "{candidate:?}: expected {needle:?} in {error}"
            );
            assert_eq!(
                document.to_toml_string(),
                text,
                "{candidate:?}: a refused add changed the document"
            );
        }
    }
}

#[test]
fn a_pair_name_the_engine_refuses_is_the_engines_sentence() {
    for (name, existing) in [
        ("a b", vec!["default"]),
        ("Docs", vec!["docs"]),
        ("default", vec!["x"]),
    ] {
        let mut text = String::new();
        for pair in &existing {
            text.push_str(&format!(
                "[[pair]]\nname = \"{pair}\"\nlocal_root = \"/home/u/{pair}\"\nremote_root = \"/Drive/{pair}\"\n\n"
            ));
        }
        let mut document = doc(&text);
        let error = document
            .add_pair(init(name, "/home/u/new", "/Drive/new"))
            .unwrap_err();
        let engine = validate_pair_name_among(name, &existing, existing.len()).unwrap_err();
        assert!(matches!(error, ConfigError::PairName(_)), "{error:?}");
        assert_eq!(
            error.to_string(),
            engine,
            "{name}: the sentence must be the engine's, verbatim"
        );
    }
}

#[test]
fn a_single_pair_file_that_says_dry_run_true_is_refused_when_a_second_pair_is_added() {
    let text = "dry_run = true\nlocal_root = \"/home/u/A\"\nremote_root = \"/Drive/A\"\n";
    let mut document = doc(text);
    let error = document
        .add_pair(init("b", "/home/u/B", "/Drive/B"))
        .unwrap_err();
    // The engine's M3 sentence, whole: the file reader and the add give one answer.
    let engine = proton_drive_sync_engine::config::validate_file_config_text(
        "[[pair]]\nname = \"default\"\nlocal_root = \"/home/u/A\"\nremote_root = \"/Drive/A\"\n\
         dry_run = true\n\n[[pair]]\nname = \"b\"\nlocal_root = \"/home/u/B\"\nremote_root = \"/Drive/B\"\n",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.to_string().ends_with(&engine),
        "{error}\n  vs\n{engine}"
    );
    assert!(error.to_string().contains("`dry_run = true`"), "{error}");
    assert_eq!(document.to_toml_string(), text);
}

#[test]
fn an_inline_array_is_never_edited_by_add_or_remove() {
    let text = "pair = [{ name = \"a\", local_root = \"/a\", remote_root = \"/Drive/a\" }]\n";
    let mut document = doc(text);
    assert!(matches!(
        document.add_pair(init("b", "/b", "/Drive/b")),
        Err(ConfigError::InlinePairs { .. })
    ));
    assert!(matches!(
        document.remove_pair("a"),
        Err(ConfigError::InlinePairs { .. })
    ));
    assert_eq!(document.to_toml_string(), text);
}

// ---- removing ----------------------------------------------------------------------------------------

const THREE_PAIRS: &str = "# hand-written\nlog_level = \"info\"\n\n# first\n[[pair]]\n\
    name = \"documents\"\nlocal_root = \"/home/me/Documents\"\nremote_root = \"/Drive/Docs\"\n\n\
    # second\n[[pair]]\nname = \"photos\"\nlocal_root = \"/home/me/Pictures\"\n\
    remote_root = \"/Drive/Photos\"\n\n# third\n[[pair]]\nname = \"music\"\n\
    local_root = \"/home/me/Music\"\nremote_root = \"/Drive/Music\"\n";

#[test]
fn removing_a_pair_removes_its_table_and_nothing_else() {
    let mut document = doc(THREE_PAIRS);
    document.remove_pair("photos").unwrap();
    assert_eq!(
        document.to_toml_string(),
        "# hand-written\nlog_level = \"info\"\n\n# first\n[[pair]]\nname = \"documents\"\n\
         local_root = \"/home/me/Documents\"\nremote_root = \"/Drive/Docs\"\n\n\
         # third\n[[pair]]\nname = \"music\"\nlocal_root = \"/home/me/Music\"\n\
         remote_root = \"/Drive/Music\"\n"
    );
    assert_eq!(document.pair_names(), ["documents", "music"]);
}

#[test]
fn removing_the_first_pair_makes_the_next_one_the_default_and_a_two_pair_file_stays_in_tables() {
    let mut document = doc(THREE_PAIRS);
    document.remove_pair("documents").unwrap();
    assert_eq!(document.pair_names(), ["photos", "music"]);
    document.remove_pair("photos").unwrap();
    // One table left: still `[[pair]]` — the layout is never rewritten behind a person's back.
    assert_eq!(document.layout(), PairLayout::Tables);
    assert_eq!(document.pair_names(), ["music"]);
    resolved_pair_names(&document.to_toml_string()).expect("the daemon starts on it");
}

#[test]
fn removing_the_last_pair_is_refused_in_every_layout() {
    let one_table = "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n";
    let implicit = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n";
    for (text, name) in [
        (one_table, "a"),
        (implicit, DEFAULT_PAIR_NAME),
        ("", DEFAULT_PAIR_NAME),
    ] {
        let mut document = doc(text);
        let error = document.remove_pair(name).unwrap_err();
        assert!(
            matches!(error, ConfigError::LastPair { .. }),
            "{text:?}: {error:?}"
        );
        assert!(error.to_string().contains("only folder pair"), "{error}");
        assert_eq!(document.to_toml_string(), text);
    }
}

#[test]
fn removing_a_pair_the_file_does_not_have_names_the_ones_it_does() {
    let mut document = doc(THREE_PAIRS);
    let error = document.remove_pair("videos").unwrap_err();
    match &error {
        ConfigError::NoSuchPair { known, .. } => {
            assert_eq!(known, &["documents", "photos", "music"])
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(document.to_toml_string(), THREE_PAIRS);
    // An implicit file has exactly one pair, called `default`.
    let mut implicit = doc("local_root = \"/a\"\nremote_root = \"/Drive/a\"\n");
    assert!(matches!(
        implicit.remove_pair("other"),
        Err(ConfigError::NoSuchPair { .. })
    ));
}

// ---- the meaning check ------------------------------------------------------------------------------

#[test]
fn the_meaning_check_sees_a_moved_changed_or_dropped_setting_and_ignores_the_layout() {
    // What the self-check of every rewrite compares. It must tell these apart, or "the rewrite changed
    // nothing" is a sentence nobody can fail.
    let implicit = "log_level = \"info\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                    scan_interval_secs = 60\n[delete_approval]\nremote = false\n";
    let tables = "log_level = \"info\"\n[[pair]]\nname = \"default\"\nlocal_root = \"/a\"\n\
                  remote_root = \"/Drive/a\"\nscan_interval_secs = 60\n[pair.delete_approval]\n\
                  remote = false\n";
    let meaning = |text: &str| Meaning::of(text).unwrap();
    assert_eq!(
        meaning(implicit),
        meaning(tables),
        "the same settings, laid out two ways"
    );
    // Dotted, inline and table spellings of one nested table are one setting.
    let dotted = tables.replace(
        "[pair.delete_approval]\nremote = false\n",
        "delete_approval.remote = false\n",
    );
    assert_eq!(meaning(tables), meaning(&dotted));

    for (what, changed) in [
        (
            "a dropped setting",
            tables.replace("scan_interval_secs = 60\n", ""),
        ),
        ("a changed value", tables.replace("= 60", "= 61")),
        (
            "a changed nested value",
            tables.replace("remote = false", "remote = true"),
        ),
        (
            "a daemon key moved into the pair",
            tables.replace(
                "log_level = \"info\"\n[[pair]]\nname = \"default\"\n",
                "[[pair]]\nname = \"default\"\nlog_level = \"info\"\n",
            ),
        ),
        (
            "a per-pair key left at the top",
            tables.replace("scan_interval_secs = 60\n", "").replace(
                "log_level = \"info\"\n",
                "log_level = \"info\"\nscan_interval_secs = 60\n",
            ),
        ),
        (
            "a renamed pair",
            tables.replace("name = \"default\"", "name = \"other\""),
        ),
        (
            "an invented key",
            tables.replace(
                "scan_interval_secs = 60\n",
                "scan_interval_secs = 60\nevents_driven = true\n",
            ),
        ),
    ] {
        assert_ne!(
            meaning(implicit),
            meaning(&changed),
            "{what} must change the meaning"
        );
    }
}
