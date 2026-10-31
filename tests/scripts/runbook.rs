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
    // A failure names the item by its place in the lists here, never by formatting the secret's name.
    for (index, name) in [
        "COPR_CONFIG",
        "AUR_SSH_KEY",
        "APT_SIGNING_KEY",
        "APT_DEPLOY_KEY",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            secrets.contains(name),
            "the reader did not find known secret number {index} (of {} found)",
            secrets.len()
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
    // Lists 0, 1 and 2 are the secrets, the variables and the env values; an entry is named by its
    // place in its sorted list, never by formatting it.
    for (list, names) in [&secrets, &variables, &env].into_iter().enumerate() {
        for (index, name) in names.iter().enumerate() {
            assert!(
                docs.contains(&format!("`{name}`")),
                "docs/releasing.md does not name entry {index} of list {list} that the workflows read"
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

// --- the order of the setup, and the table against the workflows ---------------------------------

/// The byte offset of every line of `text` that starts (after indentation) with `prefix`: the
/// commands of the runbook's code blocks.
fn command_lines(text: &str, prefix: &str) -> Vec<usize> {
    let mut offset = 0;
    let mut found = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with(prefix) {
            found.push(offset);
        }
        offset += line.len();
    }
    found
}

/// Every command in the runbook's code blocks that starts with `prefix`, with its `\` continuation
/// lines joined, as (name it sets, whole command).
fn logical_commands(text: &str, prefix: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at].trim_start();
        if let Some(rest) = line.strip_prefix(prefix) {
            let mut whole = line.to_string();
            while whole.ends_with('\\') && at + 1 < lines.len() {
                at += 1;
                whole.pop();
                whole.push(' ');
                whole.push_str(lines[at].trim());
            }
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            found.push((name, whole));
        }
        at += 1;
    }
    found
}

/// Each job of a workflow as (name, text), comments left out. Jobs are the two-space keys under
/// `jobs:`.
fn jobs_of(workflow_text: &str) -> Vec<(String, String)> {
    let mut jobs: Vec<(String, String)> = Vec::new();
    let mut in_jobs = false;
    for line in workflow_text.lines() {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }
        if !in_jobs || line.trim_start().starts_with('#') {
            continue;
        }
        let is_job_key = line.starts_with("  ")
            && !line.starts_with("   ")
            && line.trim_end().ends_with(':')
            && !line.trim().is_empty();
        if is_job_key {
            jobs.push((line.trim().trim_end_matches(':').to_string(), String::new()));
        } else if let Some((_, text)) = jobs.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    jobs
}

/// The rows of the runbook's table of secrets and variables as (name, kind, where it is set).
fn table_rows(docs: &str) -> Vec<(String, String, String)> {
    docs.lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            (cells.len() == 5 && matches!(cells[1], "secret" | "variable")).then(|| {
                (
                    cells[0].trim_matches('`').to_string(),
                    cells[1].to_string(),
                    cells[2].to_string(),
                )
            })
        })
        .collect()
}

#[test]
fn the_environment_is_created_before_the_first_command_that_needs_it() {
    let docs = repository_file("docs/releasing.md");
    let environment = docs
        .find("\n### The `release` environment")
        .expect("the runbook has a section that creates the release environment");
    let secret_commands = command_lines(&docs, "gh secret set");
    // The reader must find the four commands, or an empty list would pass.
    assert!(
        secret_commands.len() >= 4,
        "found {} `gh secret set` commands",
        secret_commands.len()
    );
    for at in secret_commands {
        assert!(
            at > environment,
            "a `gh secret set` command (byte {at}) comes before the section that creates the \
             environment (byte {environment}): `--env` on a missing environment fails with a 404"
        );
    }
    // The page says so, and the environment is made with no secret in it.
    assert!(docs.contains("needs the environment to exist"));
}

#[test]
fn the_table_gives_each_secret_and_variable_the_kind_and_place_the_workflows_use_and_the_commands_set_it_there()
 {
    let docs = repository_file("docs/releasing.md");
    let rows = table_rows(&docs);
    // The reader finds all six, or a broken one would pass vacuously.
    assert_eq!(rows.len(), 6, "{rows:?}");
    let set_secret = logical_commands(&docs, "gh secret set");
    let set_variable = logical_commands(&docs, "gh variable set");
    let mut seen_in_workflows = BTreeSet::new();
    for file in ["publish.yml", "release.yml"] {
        for (job, text) in jobs_of(&workflow(file)) {
            let has_environment = text
                .lines()
                .any(|line| line.trim_start().starts_with("environment:"));
            for (prefix, kind) in [("secrets.", "secret"), ("vars.", "variable")] {
                for name in names_after(&text, prefix) {
                    seen_in_workflows.insert(name.clone());
                    let row = rows
                        .iter()
                        .find(|(row_name, _, _)| *row_name == name)
                        .unwrap_or_else(|| {
                            panic!("{file} job {job} reads a {kind} the table does not list")
                        });
                    assert_eq!(
                        row.1, kind,
                        "{file} job {job}: the table has the wrong kind"
                    );
                    if kind == "secret" {
                        assert!(
                            has_environment,
                            "{file} job {job} reads a secret outside an environment"
                        );
                    }
                }
            }
        }
    }
    for (name, kind, place) in &rows {
        assert!(
            seen_in_workflows.contains(name),
            "the table lists {name}, which no workflow reads"
        );
        let secret_command = set_secret.iter().find(|(set, _)| set == name);
        let variable_command = set_variable.iter().find(|(set, _)| set == name);
        if kind == "secret" {
            assert!(place.contains("environment `release`"), "{name}: {place}");
            let (_, command) =
                secret_command.unwrap_or_else(|| panic!("no `gh secret set {name}`"));
            assert!(command.contains("--env release"), "{command}");
            assert!(
                variable_command.is_none(),
                "{name} is set as a variable too"
            );
        } else {
            assert!(
                place.contains("repository") && !place.contains("environment"),
                "{name}: {place}"
            );
            let (_, command) =
                variable_command.unwrap_or_else(|| panic!("no `gh variable set {name}`"));
            assert!(!command.contains("--env"), "{command}");
            assert!(secret_command.is_none(), "{name} is set as a secret too");
        }
    }
}
