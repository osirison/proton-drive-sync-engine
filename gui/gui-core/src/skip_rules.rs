//! What each skip rule is hiding **right now** (C2, #175).
//!
//! Settings → *What to skip* is the only screen in the app that shows a rule's consequences rather
//! than its text: `hiding 4 files, 3.1 GB in total`, `Skipping 2 files right now` with the paths,
//! and `Matching nothing · no such folder here any more — safe to remove` on a rule that has
//! outlived the folder it was written for. A forgotten rule is invisible without those numbers,
//! which is the whole reason the tab exists.
//!
//! # Why this walks the disk and not the index
//!
//! Issue #175 proposes matching each glob against the sync index. That reads well and returns zero
//! for almost every rule a real user has, because of an invariant the engine states plainly:
//! selective-sync filters are applied to the local scan, the remote listing *and* the base-index
//! records **before planning**, so an excluded file is never inserted in the first place. A rule
//! that has been in the config since before its files existed therefore matches **no index record
//! at all** — and the tab would draw `Matching nothing · safe to remove` over a rule that is, at
//! that moment, hiding a 40 GB export folder. Removing it would start uploading all of it.
//!
//! The only rows an index match would find are the opposite case: files synced *before* the rule
//! was added, whose records linger because excluded records are deliberately never purged as
//! "missing". That is a real set, but it is the leftovers, not the answer to "what is this rule
//! hiding right now".
//!
//! So this walks the local tree. It is a **metadata-only** walk — `read_dir` plus one `stat` per
//! file, no SHA-1 — which is why it does not reuse `index::scan_local_files_with_options`: that one
//! hashes, and hashing a sync root to render a settings tab would be minutes of CPU for four
//! numbers. It stays honest about the daemon's own boundaries by asking the engine's
//! [`ScanOptions`] every question about what counts, rather than reimplementing glob semantics
//! beside it.

use proton_drive_sync_engine::index::ScanOptions;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How many samples of a rule's matches to carry for the `Skipping 2 files right now` sub-line.
/// The frame names them individually, so this is a display cap, not a page size.
pub const MAX_SAMPLES: usize = 4;

/// What one exclude rule is hiding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuleUsage {
    /// The glob exactly as it appears in the config, so the row can be matched back to its input.
    pub pattern: String,
    /// Files this rule hides. Directories are not counted — the tab counts what you would lose.
    pub files: u64,
    /// Their total size in bytes.
    pub bytes: u64,
    /// Files **only this rule** hides, and their bytes.
    ///
    /// What `One rule removed — 4 files, 3.1 GB will start syncing.` has to count. A file another
    /// rule also hides does not start syncing when this one goes, so quoting `files` there would
    /// overstate the consequence of a removal on precisely the screen where someone is deciding
    /// whether to remove it.
    pub unique_files: u64,
    pub unique_bytes: u64,
    /// Up to [`MAX_SAMPLES`] matched paths, relative to the local root, in walk order.
    pub samples: Vec<PathBuf>,
    /// For a rule anchored at a literal folder (`Exports/**`, `scratch/`), whether that folder still
    /// exists on disk. `None` when the rule has no literal folder to check (`*.psd`).
    ///
    /// This is what separates the tab's two "matches nothing" wordings: a rule matching nothing
    /// whose folder is *gone* is safe to remove, and a rule matching nothing whose folder is still
    /// there is merely idle — removing that one would start syncing whatever lands in it next.
    pub folder_exists: Option<bool>,
    /// Why this rule could not be measured — an unparseable glob, and nothing else.
    ///
    /// Per-rule rather than fatal because the tab has an **Add** row: someone typing `*.{psd` is
    /// mid-word, and blanking every other rule's numbers while they finish the sentence would make
    /// the tab unusable exactly when it is being used. A rule with an error carries zeroes, which
    /// is why the error has to be rendered — zero and "unmeasurable" are the same numbers.
    pub error: Option<String>,
}

impl RuleUsage {
    /// Whether the tab should draw this rule as `Matching nothing`.
    pub fn matches_nothing(&self) -> bool {
        self.error.is_none() && self.files == 0
    }

    /// Whether the tab may add `no such folder here any more — safe to remove`. Deliberately
    /// stricter than [`Self::matches_nothing`]: it requires the folder to be *known* gone, so a
    /// rule with no folder to check never claims one vanished.
    pub fn is_stale_folder(&self) -> bool {
        self.matches_nothing() && self.folder_exists == Some(false)
    }
}

/// Every rule's usage plus the totals the tab's header draws.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkipRuleReport {
    pub rules: Vec<RuleUsage>,
    /// Distinct files hidden by **at least one** rule. Not the sum of `rules[].files`: two rules
    /// may hide the same file, and `hiding 4 files, 3.1 GB in total` is a count of files, not of
    /// matches.
    pub total_files: u64,
    /// Their total size in bytes, on the same distinct-file basis.
    pub total_bytes: u64,
    /// Files the walk considered — everything the daemon would sync if there were no rules at all.
    /// Gives the tab a denominator and gives a reviewer a way to tell "nothing matched" from
    /// "nothing was there".
    pub considered_files: u64,
    /// Directories that could not be read (permissions, or a directory that vanished mid-walk).
    /// The numbers above are then a floor rather than a total, and the tab must not present a floor
    /// as a fact.
    pub unreadable_directories: u64,
}

/// Measure what `exclude_patterns` are hiding under `local_root`.
///
/// `ignored_paths` is the daemon's own state-file list (`db_path`, the lockfile, …) — the same
/// argument [`ScanOptions::new`] takes, so a state file that has been relocated inside the sync root
/// is excluded from the denominator instead of being reported as a file some rule is hiding.
///
/// `exclude_patterns` is whatever the tab currently shows — the **staged** rule set, not the saved
/// config. `8a Skip rules` is drawn mid-edit with a pending removal, and the Add row implies
/// pricing a pattern that has never been saved, so the caller passes what is on screen.
pub fn measure(
    local_root: &Path,
    exclude_patterns: &[String],
    ignored_paths: &[PathBuf],
) -> Result<SkipRuleReport, String> {
    let to_error = |e: Box<dyn std::error::Error + Send + Sync>| e.to_string();

    // Three matchers, and each answers a different question.
    //
    //   baseline  — no rules at all: "would the daemon sync this file if the config had no
    //               excludes?" This is the denominator, and it is what keeps `.sync/`, the download
    //               scratch and the ignored state files out of every count below.
    //   combined  — every valid rule at once: the distinct-file totals, with no set of matched
    //               paths to hold in memory.
    //   per-rule  — one rule alone: that rule's own row.
    //
    // Only the baseline is fatal, because a failure there is not about any rule the user typed.
    let baseline = ScanOptions::new(local_root, ignored_paths, &[], &[]).map_err(to_error)?;

    let mut rules = Vec::with_capacity(exclude_patterns.len());
    let mut per_rule = Vec::with_capacity(exclude_patterns.len());
    for pattern in exclude_patterns {
        let compiled = ScanOptions::new(
            local_root,
            ignored_paths,
            &[],
            std::slice::from_ref(pattern),
        );
        let error = compiled.as_ref().err().map(|e| e.to_string());
        rules.push(RuleUsage {
            pattern: pattern.clone(),
            files: 0,
            bytes: 0,
            unique_files: 0,
            unique_bytes: 0,
            samples: Vec::new(),
            folder_exists: literal_folder_prefix(pattern)
                .map(|prefix| local_root.join(prefix).is_dir()),
            error,
        });
        per_rule.push(compiled.ok());
    }

    let valid: Vec<String> = exclude_patterns
        .iter()
        .zip(per_rule.iter())
        .filter(|(_, compiled)| compiled.is_some())
        .map(|(pattern, _)| pattern.clone())
        .collect();
    let combined = ScanOptions::new(local_root, ignored_paths, &[], &valid).map_err(to_error)?;

    let mut report = SkipRuleReport {
        rules,
        total_files: 0,
        total_bytes: 0,
        considered_files: 0,
        unreadable_directories: 0,
    };

    let mut walker = Walk {
        root: local_root,
        baseline: &baseline,
        combined: &combined,
        per_rule: &per_rule,
        report: &mut report,
        seen_directories: HashSet::new(),
    };
    walker.visit(local_root, Path::new(""));

    Ok(report)
}

struct Walk<'a> {
    root: &'a Path,
    baseline: &'a ScanOptions,
    combined: &'a ScanOptions,
    /// `None` for a rule whose glob did not compile — it matches nothing rather than everything.
    per_rule: &'a [Option<ScanOptions>],
    report: &'a mut SkipRuleReport,
    /// Canonical paths of directories already entered, so a symlink loop cannot spin the walk.
    seen_directories: HashSet<PathBuf>,
}

impl Walk<'_> {
    fn visit(&mut self, absolute: &Path, relative: &Path) {
        // A rule's own directory must still be *entered*: `allows_relative_directory` would prune
        // `scratch/` for the very rule whose files we are here to count, so traversal is decided by
        // the baseline and only the files are classified.
        if !relative.as_os_str().is_empty() && !self.baseline.allows_relative_directory(relative) {
            return;
        }
        match std::fs::canonicalize(absolute) {
            // A directory reached twice through symlinks is walked once. Counting it twice would
            // double a rule's byte total, which is the number the user is deciding on.
            Ok(canonical) => {
                if !self.seen_directories.insert(canonical) {
                    return;
                }
            }
            Err(_) => {
                self.report.unreadable_directories += 1;
                return;
            }
        }

        let entries = match std::fs::read_dir(absolute) {
            Ok(entries) => entries,
            Err(_) => {
                self.report.unreadable_directories += 1;
                return;
            }
        };

        for entry in entries {
            let Ok(entry) = entry else {
                self.report.unreadable_directories += 1;
                continue;
            };
            let child_relative = relative.join(entry.file_name());
            // `file_type` here does NOT follow the link, so a symlink is classified as a symlink
            // rather than as whatever it points at. The engine does not sync them and neither side
            // of this count should invent them.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                self.visit(&entry.path(), &child_relative);
            } else if file_type.is_file() {
                self.classify(&entry, &child_relative);
            }
        }
    }

    fn classify(&mut self, entry: &std::fs::DirEntry, relative: &Path) {
        if !self.baseline.allows_relative_file(relative) {
            return;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        self.report.considered_files += 1;

        if !self.combined.allows_relative_file(relative) {
            self.report.total_files += 1;
            self.report.total_bytes = self.report.total_bytes.saturating_add(size);
        }

        let mut sole_match = None;
        let mut matches = 0u32;
        for (index, options) in self.per_rule.iter().enumerate() {
            let Some(options) = options else { continue };
            if options.allows_relative_file(relative) {
                continue;
            }
            matches += 1;
            sole_match = Some(index);
            let usage = &mut self.report.rules[index];
            usage.files += 1;
            usage.bytes = usage.bytes.saturating_add(size);
            if usage.samples.len() < MAX_SAMPLES {
                usage.samples.push(relative.to_path_buf());
            }
        }
        // Removing a rule only starts syncing the files no *other* rule still hides.
        if matches == 1 {
            let usage = &mut self.report.rules[sole_match.expect("one match has an index")];
            usage.unique_files += 1;
            usage.unique_bytes = usage.unique_bytes.saturating_add(size);
        }
        let _ = self.root;
    }
}

/// The leading run of literal path components in a glob — the folder a rule is anchored at.
///
/// `Exports/**` → `Exports`, `a/b/*.psd` → `a/b`, `*.psd` → `None`, `**/node_modules` → `None`.
/// Only used to tell "the folder is gone" from "the folder is empty"; a rule with no literal prefix
/// simply does not make that claim.
fn literal_folder_prefix(pattern: &str) -> Option<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in pattern.split('/') {
        if component.is_empty() {
            continue;
        }
        if component.contains(['*', '?', '[', ']', '{', '}']) {
            break;
        }
        prefix.push(component);
    }
    // A pattern that is *entirely* literal names a single path, not a folder to check for absence:
    // `notes/todo.txt` matching nothing means the file is gone, which the count already says.
    if prefix.as_os_str().is_empty()
        || prefix.as_os_str().len() == pattern.trim_end_matches('/').len()
    {
        return None;
    }
    Some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, bytes: usize) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn measure_at(root: &Path, patterns: &[&str]) -> SkipRuleReport {
        let owned: Vec<String> = patterns.iter().map(|p| (*p).to_string()).collect();
        measure(root, &owned, &[]).expect("valid globs")
    }

    #[test]
    fn a_rule_counts_the_files_it_hides_and_their_bytes() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "notes/todo.txt", 10);
        write(dir.path(), "art/logo.psd", 100);
        write(dir.path(), "art/icon.psd", 200);

        let report = measure_at(dir.path(), &["**/*.psd"]);
        assert_eq!(report.rules[0].files, 2);
        assert_eq!(report.rules[0].bytes, 300);
        assert_eq!(report.considered_files, 3);
        assert_eq!(report.total_files, 2);
        assert_eq!(report.total_bytes, 300);
    }

    #[test]
    fn the_total_counts_a_doubly_matched_file_once() {
        // `hiding N files` is a count of files. Summing the rows would say 2 and lose the point.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "art/logo.psd", 100);

        let report = measure_at(dir.path(), &["**/*.psd", "art/**"]);
        assert_eq!(report.rules[0].files, 1);
        assert_eq!(report.rules[1].files, 1);
        assert_eq!(report.total_files, 1, "one file, hidden twice");
        assert_eq!(report.total_bytes, 100);
    }

    #[test]
    fn removing_a_rule_only_frees_the_files_no_other_rule_hides() {
        // `One rule removed — N files will start syncing` is a promise about what happens next.
        // A file two rules hide keeps being hidden, so it is not part of that number.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "art/logo.psd", 100); // both rules
        write(dir.path(), "art/notes.txt", 20); // `art/**` only
        write(dir.path(), "photos/old.psd", 7); // `**/*.psd` only

        let report = measure_at(dir.path(), &["**/*.psd", "art/**"]);

        let psd = &report.rules[0];
        assert_eq!((psd.files, psd.bytes), (2, 107));
        assert_eq!(
            (psd.unique_files, psd.unique_bytes),
            (1, 7),
            "logo.psd stays hidden by art/**"
        );

        let art = &report.rules[1];
        assert_eq!((art.files, art.bytes), (2, 120));
        assert_eq!((art.unique_files, art.unique_bytes), (1, 20));

        assert_eq!(report.total_files, 3, "three distinct files hidden");
    }

    #[test]
    fn one_unparseable_rule_does_not_blank_the_others() {
        // The tab has an Add row: `*.{psd` is someone mid-word, not a broken config. Every other
        // rule must keep its numbers while they finish typing.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "art/logo.psd", 100);

        let report = measure_at(dir.path(), &["*.{psd", "art/**"]);

        assert!(report.rules[0].error.is_some(), "the bad glob says why");
        assert_eq!(report.rules[0].files, 0);
        assert!(
            !report.rules[0].matches_nothing(),
            "unmeasurable must not read as `Matching nothing` — the numbers are identical"
        );

        assert_eq!(report.rules[1].files, 1, "the good rule is unaffected");
        assert!(report.rules[1].error.is_none());
        assert_eq!(
            report.total_files, 1,
            "and the totals ignore the broken rule"
        );
        assert_eq!(
            report.rules[1].unique_files, 1,
            "a rule that matches nothing cannot share a file"
        );
    }

    #[test]
    fn several_extra_files_at_the_end_of_a_folder_are_all_that_rules_doing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "scratch/a.bin", 5);
        let report = measure_at(dir.path(), &["scratch/**"]);
        assert_eq!(report.rules[0].unique_files, 1, "sole match");
        assert_eq!(report.rules[0].unique_bytes, 5);
    }

    #[test]
    fn a_rule_is_measured_even_though_its_own_folder_is_excluded_from_the_sync() {
        // The trap: the daemon's scan prunes `scratch/` and never sees inside it. This walk must,
        // or every directory rule would report zero — the exact false "safe to remove" this module
        // exists to avoid.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "scratch/a.bin", 5);
        write(dir.path(), "scratch/deep/b.bin", 7);
        write(dir.path(), "keep.txt", 1);

        let report = measure_at(dir.path(), &["scratch/**"]);
        assert_eq!(report.rules[0].files, 2);
        assert_eq!(report.rules[0].bytes, 12);
        assert_eq!(report.considered_files, 3);
    }

    #[test]
    fn samples_are_capped_and_the_count_is_not() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MAX_SAMPLES + 3) {
            write(dir.path(), &format!("scratch/f{i}.bin"), 1);
        }
        let report = measure_at(dir.path(), &["scratch/**"]);
        assert_eq!(report.rules[0].files as usize, MAX_SAMPLES + 3);
        assert_eq!(report.rules[0].samples.len(), MAX_SAMPLES);
    }

    #[test]
    fn the_engines_own_ignores_are_not_attributed_to_any_rule() {
        // `.sync/` is the app's state directory. It is not "hidden by a rule" and must not appear
        // in the denominator either.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".sync/sync_index.db", 4096);
        write(dir.path(), "keep.txt", 1);

        let report = measure_at(dir.path(), &["**/*.db"]);
        assert_eq!(report.considered_files, 1);
        assert_eq!(report.rules[0].files, 0, ".sync is never a rule's doing");
        assert_eq!(report.total_files, 0);
    }

    #[test]
    fn a_vanished_folder_is_stale_and_an_empty_one_is_only_idle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Exports")).unwrap();
        write(dir.path(), "keep.txt", 1);

        let report = measure_at(dir.path(), &["Exports/**", "Archive/**", "*.psd"]);

        let exports = &report.rules[0];
        assert!(exports.matches_nothing());
        assert_eq!(exports.folder_exists, Some(true));
        assert!(
            !exports.is_stale_folder(),
            "an empty folder that still exists is not safe to remove"
        );

        let archive = &report.rules[1];
        assert!(archive.is_stale_folder(), "the folder is gone");

        let psd = &report.rules[2];
        assert!(psd.matches_nothing());
        assert_eq!(psd.folder_exists, None, "no folder to make a claim about");
        assert!(!psd.is_stale_folder());
    }

    #[test]
    fn literal_folder_prefixes() {
        assert_eq!(
            literal_folder_prefix("Exports/**"),
            Some(PathBuf::from("Exports"))
        );
        assert_eq!(
            literal_folder_prefix("a/b/*.psd"),
            Some(PathBuf::from("a/b"))
        );
        assert_eq!(literal_folder_prefix("*.psd"), None);
        assert_eq!(literal_folder_prefix("**/node_modules"), None);
        assert_eq!(
            literal_folder_prefix("notes/todo.txt"),
            None,
            "a fully literal path names a file, not a folder to check"
        );
    }

    #[test]
    fn an_unreadable_directory_is_reported_rather_than_counted_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            measure_at(dir.path(), &["*.psd"]).unreadable_directories,
            0,
            "a clean tree reports none"
        );
        // A missing root cannot be canonicalized, so the walk records it instead of reporting an
        // empty, clean tree.
        let missing = dir.path().join("gone");
        let report = measure(&missing, &["*.psd".to_string()], &[]).unwrap();
        assert_eq!(report.unreadable_directories, 1);
        assert_eq!(report.considered_files, 0);
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_loop_terminates_and_does_not_double_count() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "scratch/a.bin", 5);
        std::os::unix::fs::symlink(dir.path().join("scratch"), dir.path().join("scratch/self"))
            .unwrap();

        let report = measure_at(dir.path(), &["scratch/**"]);
        assert_eq!(report.rules[0].files, 1, "counted once, not forever");
        assert_eq!(report.rules[0].bytes, 5);
    }
}
