//! Every docs link `doctor::links` can emit points at a real heading (D9). Spawns nothing: it
//! reads the troubleshooting page and the Astro config from the source tree.
//!
//! A heading a check links to uses only ASCII letters, digits, spaces and hyphens, so its slug is
//! the heading lower-cased with each space made a hyphen, whichever slugger the site uses.

use proton_drive_sync_engine::doctor::links;
use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The troubleshooting page's source path under the docs content directory.
fn page_source() -> String {
    let page = links::TROUBLESHOOTING_PAGE.trim_matches('/');
    format!("website/src/content/docs/{page}.md")
}

/// Heading texts of a markdown page, skipping fenced code blocks.
fn headings(markdown: &str) -> Vec<String> {
    let mut in_fence = false;
    let mut found = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence && line.starts_with('#') {
            let text = line.trim_start_matches('#');
            if text.starts_with(' ') {
                found.push(text.trim().to_owned());
            }
        }
    }
    found
}

fn slug(heading: &str) -> String {
    heading.to_lowercase().replace(' ', "-")
}

fn plain(heading: &str) -> bool {
    heading
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
}

#[test]
fn every_anchor_a_check_can_emit_is_the_slug_of_exactly_one_plain_heading() {
    let source = page_source();
    let page = read(&source);
    let headings = headings(&page);

    for (check, anchor) in links::ANCHORS {
        let matching: Vec<&String> = headings
            .iter()
            .filter(|heading| slug(heading) == anchor)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "check `{check}` links to #{anchor}, which must be the slug of exactly one heading in \
             {source}; found {matching:?}"
        );
        assert!(
            plain(matching[0]),
            "the heading {:?} for #{anchor} in {source} must use only ASCII letters, digits, \
             spaces and hyphens",
            matching[0]
        );
    }
}

#[test]
fn no_two_checks_share_an_anchor() {
    let mut anchors: Vec<&str> = links::ANCHORS.iter().map(|(_, anchor)| *anchor).collect();
    anchors.sort_unstable();
    anchors.dedup();
    assert_eq!(anchors.len(), links::ANCHORS.len());
}

#[test]
fn the_sites_default_site_and_base_make_the_docs_site_the_module_names() {
    let config = read("website/astro.config.mjs");
    let default_of = |name: &str| -> String {
        let marker = "?? '";
        let line = config
            .lines()
            .find(|line| line.starts_with(&format!("const {name} =")))
            .unwrap_or_else(|| panic!("website/astro.config.mjs has no `const {name} =` line"));
        let start = line
            .find(marker)
            .unwrap_or_else(|| panic!("`const {name}` has no `?? '…'` default: {line}"))
            + marker.len();
        let end = line[start..]
            .find('\'')
            .unwrap_or_else(|| panic!("unterminated default in: {line}"));
        line[start..start + end].to_owned()
    };

    let site = default_of("site");
    let base = default_of("base");

    assert_eq!(
        format!("{}{}", site.trim_end_matches('/'), base),
        links::DOCS_SITE,
        "website/astro.config.mjs defaults (site {site:?}, base {base:?}) must make DOCS_SITE"
    );
}
