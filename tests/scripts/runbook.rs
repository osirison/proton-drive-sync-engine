//! The maintainer runbook `docs/releasing.md` (task 6.7) against what the workflows read and what
//! the tree holds. A page that names a setting no workflow reads, or omits one it does, or names a
//! script that is gone, sends the maintainer to a step that does not work. Pure text, no process.
use super::*;
use std::collections::BTreeSet;

fn repository_file(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// Every `<prefix>NAME` in `text` where NAME is upper-case letters, digits and underscores.
fn names_after(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        rest = &rest[at + prefix.len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        if !name.is_empty() {
            found.insert(name);
        }
    }
    found
}

/// The keys of a workflow's top-level `env:` block (two-space indent, up to the next top-level key).
fn top_level_env_names(workflow_text: &str) -> BTreeSet<String> {
    workflow_text
        .lines()
        .skip_while(|line| *line != "env:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("  "))
        .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
        .filter_map(|line| line.split(':').next())
        .map(str::to_string)
        .collect()
}

/// Every repository path of the kinds the runbook names, as it writes them.
fn named_paths(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for prefix in ["packaging/", ".github/workflows/", "docs/", "tests/"] {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            let tail = &rest[at..];
            let token: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "_./-".contains(*c))
                .collect();
            let after = tail[token.len()..].chars().next();
            rest = &rest[at + prefix.len()..];
            // `packaging/release/*.sh` and `packaging/release/verify-<family>.sh` name no file.
            if matches!(after, Some('*' | '<' | '{')) {
                continue;
            }
            let token = token.trim_end_matches(['.', '-']);
            if !token.ends_with('/') && !token.is_empty() {
                found.insert(token.to_string());
            }
        }
    }
    found
}

#[test]
fn the_runbook_names_every_secret_variable_and_setting_the_publish_workflow_reads() {
    let docs = repository_file("docs/releasing.md");
    let publish = workflow("publish.yml");
    let release = workflow("release.yml");
    let both = format!("{publish}\n{release}");
    let secrets = names_after(&both, "secrets.");
    let variables = names_after(&both, "vars.");
    let env = top_level_env_names(&publish);
    // The extraction itself must find what is known to be there, or a broken reader would pass.
    for name in [
        "COPR_CONFIG",
        "AUR_SSH_KEY",
        "APT_SIGNING_KEY",
        "APT_DEPLOY_KEY",
    ] {
        assert!(
            secrets.contains(name),
            "no secret {name} found: {secrets:?}"
        );
    }
    for name in ["COPR_TOKEN_EXPIRES", "APT_WITHDRAWN"] {
        assert!(
            variables.contains(name),
            "no variable {name} found: {variables:?}"
        );
    }
    for name in [
        "COPR_OWNER",
        "AUR_GIT_NAME",
        "APT_GIT_NAME",
        "APT_GIT_EMAIL",
    ] {
        assert!(env.contains(name), "no env value {name} found: {env:?}");
    }
    for (kind, names) in [
        ("secret", &secrets),
        ("variable", &variables),
        ("env value", &env),
    ] {
        for name in names {
            assert!(
                docs.contains(&format!("`{name}`")),
                "docs/releasing.md does not name the {kind} `{name}` that the workflows read"
            );
        }
    }
}

#[test]
fn every_script_and_file_the_runbook_names_exists_and_every_script_the_workflows_run_is_named() {
    let docs = repository_file("docs/releasing.md");
    let named = named_paths(&docs);
    assert!(
        named.len() >= 20,
        "the reader found too few paths ({}): {named:?}",
        named.len()
    );
    for path in &named {
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(path).is_file(),
            "docs/releasing.md names {path}, which is not a file in the repository"
        );
    }
    // The other way: a script the release workflows run is a step the maintainer may need to know.
    let both = format!("{}\n{}", workflow("publish.yml"), workflow("release.yml"));
    let run_by_the_workflows: BTreeSet<String> = named_paths(&both)
        .into_iter()
        .filter(|path| path.starts_with("packaging/release/") && path.ends_with(".sh"))
        .collect();
    assert!(
        run_by_the_workflows.len() >= 10,
        "the reader found too few scripts in the workflows: {run_by_the_workflows:?}"
    );
    for script in &run_by_the_workflows {
        assert!(
            named.contains(script),
            "the workflows run {script} but docs/releasing.md does not name it"
        );
    }
}
