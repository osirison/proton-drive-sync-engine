//! What each side of a conflict did to the **last agreed version** (#217, #347) — **pure**.
//!
//! # The sentence this exists for, and why it needs a third version
//!
//! A conflict card draws two lines per side. The second — *what differs* — compares the two live
//! versions and has always worked. The first — *what happened*, `You added a line` — is a claim
//! about one side against the version both sides last agreed on, and until this module it was a
//! **hard-coded constant drawn as if it were live** (#347): every conflict, including a 4 GB video,
//! read `You added a line, 5 minutes ago`.
//!
//! It cannot be answered from the two live versions. Against Proton's current copy alone the very
//! same edit reads as a *removal*, which is the trap #217 names — so this is not a harder version
//! of the diff problem, it is a different one, and it needs the ancestor.
//!
//! # Where the ancestor is, and where it is not
//!
//! `B` — the agreed content — exists only between the last successful sync and the first local
//! edit. By the time a conflict is planned the local file is `L`, the sidecar is `R`, and
//! `SyncAction::Conflict`'s own arm upserts `FileRecord::from_local`, replacing `digest(B)` with
//! `digest(L)`. So at conflict time `B` is on neither side and in no index row: **capture has to
//! happen when a file becomes agreed**, not when it conflicts. See the comment on #217.
//!
//! # A summary, never the bytes
//!
//! One digest per line plus the count. That is enough to separate *added a line* from *changed a
//! line* from *removed a line*, which is the whole vocabulary the card needs, and it is not a second
//! copy of every file the engine has ever synced.
//!
//! # Everything uncertain says less
//!
//! No summary, a file that is not UTF-8, one past the caps, a diff past its cutoff — every one of
//! them answers `None`, and the card then draws only what it can compute. A missing answer must be
//! ordinary rather than an error, because the alternative is the failure this module removes: a
//! confident sentence nothing supports.

use std::fmt::Write as _;

use sha1::{Digest, Sha1};

/// Files larger than this are not summarised. A conflict on one is the say-less case by
/// construction — which is #347's own headline example, the 4 GB video whose card claimed a line
/// had been added to it.
pub const MAX_SUMMARY_BYTES: usize = 256 * 1024;

/// Nor are files with more lines than this. The diff below is O(n·m) in the *differing* region, and
/// a bound on the input is what keeps a pathological file from costing a conflict screen its
/// responsiveness.
pub const MAX_SUMMARY_LINES: usize = 4096;

/// The diff gives up past this many cells, exactly as the webview's own `MAX_DIFF_CELLS` does.
const MAX_DIFF_CELLS: usize = 250_000;

/// One digest per line of the agreed version.
///
/// Stored, so its encoding is deliberately boring: the hex digests joined by `\n`. A `Vec<String>`
/// on the wire and a single TEXT column in the database, which is one fewer decision than a blob
/// format and readable when someone is looking at the table by hand.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineSummary {
    lines: Vec<String>,
}

impl LineSummary {
    /// Summarise `text`, or `None` when it is too big to be worth it.
    ///
    /// The **caller** decides what is text: this takes a `&str`, so a file that is not valid UTF-8
    /// never reaches it and is a say-less case one layer up.
    pub fn of(text: &str) -> Option<Self> {
        if text.len() > MAX_SUMMARY_BYTES {
            return None;
        }
        let lines = split_lines(text);
        if lines.len() > MAX_SUMMARY_LINES {
            return None;
        }
        Some(Self {
            lines: lines.iter().map(|line| line_digest(line)).collect(),
        })
    }

    /// How many lines the agreed version had.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The stored form: hex digests, one per line, newline-joined.
    pub fn encode(&self) -> String {
        self.lines.join("\n")
    }

    /// Read the stored form back. An empty string is a **zero-line** summary, which is a real value
    /// (an empty agreed file), so this cannot be `Option`-typed on emptiness.
    pub fn decode(stored: &str) -> Self {
        Self {
            lines: if stored.is_empty() {
                Vec::new()
            } else {
                stored.split('\n').map(str::to_owned).collect()
            },
        }
    }
}

/// What one side did to the agreed version, in the three counts the card's vocabulary needs.
///
/// Counted by **file position**, the same rule `ui/diff.js` states for the live comparison: a line
/// replaced is one `changed`, not one removed plus one added, or `2 lines differ · 3 lines
/// identical` would not sum to the length of the longer file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VersionFacts {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

impl VersionFacts {
    /// Nothing moved. Reachable for a real conflict: two files can differ as bytes and not as lines
    /// (a trailing newline, a line ending), which the live comparison calls an *invisible
    /// difference* — and this side may genuinely be the agreed version while the other moved.
    pub fn is_unchanged(self) -> bool {
        self.added == 0 && self.changed == 0 && self.removed == 0
    }
}

/// Split `text` into lines the way the conflict screen counts them.
///
/// **CRLF normalised, and the empty element a final newline produces is dropped.** Both rules come
/// from `gui/src/js/ui/diff.js`'s `lines()`, and they are not cosmetic: the card draws `4 lines`
/// for a four-line file, `split('\n')` says five, and the panel beneath the card already counts it
/// this way. A second splitter that disagreed would make the verb and the panel describe different
/// files — which has happened here before, between the card's own count and the panel's.
pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    // `\r\n` and a bare `\r` both end a line, so split on `\n` after treating a lone `\r` as one.
    let mut lines: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(['\n', '\r']) {
        lines.push(&rest[..at]);
        let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[at + skip..];
    }
    if !rest.is_empty() {
        lines.push(rest);
    }
    lines
}

/// A line's digest. Truncated to 8 bytes: this compares lines *within one file's history*, so the
/// collision budget is the file's own line count rather than the world's, and 64 bits over a few
/// thousand lines is far past what the sentence's confidence needs.
fn line_digest(line: &str) -> String {
    let full = Sha1::digest(line.as_bytes());
    let mut hex = String::with_capacity(16);
    for byte in &full[..8] {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// How `current` moved from `ancestor`. `None` when the two are too far apart to compare inside
/// [`MAX_DIFF_CELLS`] — the say-less case, not an error.
///
/// **This is not `ui/diff.js`'s `compare`, and the duplication is deliberate.** That one answers a
/// different question with different inputs and a different output: what differs between the two
/// *live* versions, as quoted lines at drawn positions. This one answers how each side moved from a
/// version that exists only as digests, as counts. Sharing the algorithm would mean either shipping
/// the agreed version's line *strings* across the wire — the storage #217's decision rejected — or
/// a second line splitter in JS, which is the duplication this codebase has already been bitten by.
/// So the shared part is forty lines of LCS, stated twice, rather than a format nobody wanted.
pub fn compare_to_ancestor(ancestor: &LineSummary, current: &LineSummary) -> Option<VersionFacts> {
    let (before, after) = (&ancestor.lines, &current.lines);

    // Trim the agreeing head and tail first. Beyond making a large file affordable, it is what
    // keeps the counts about the edit rather than about the file.
    let mut head = 0;
    while head < before.len() && head < after.len() && before[head] == after[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < before.len() - head
        && tail < after.len() - head
        && before[before.len() - 1 - tail] == after[after.len() - 1 - tail]
    {
        tail += 1;
    }
    let before = &before[head..before.len() - tail];
    let after = &after[head..after.len() - tail];

    if (before.len() + 1).checked_mul(after.len() + 1)? > MAX_DIFF_CELLS {
        return None;
    }

    // Longest common subsequence over the differing middle, then pair what is left.
    let columns = after.len() + 1;
    let mut table = vec![0u32; (before.len() + 1) * columns];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i * columns + j] = if before[i] == after[j] {
                table[(i + 1) * columns + (j + 1)] + 1
            } else {
                table[(i + 1) * columns + j].max(table[i * columns + (j + 1)])
            };
        }
    }

    let (mut only_before, mut only_after) = (0usize, 0usize);
    let (mut i, mut j) = (0usize, 0usize);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            i += 1;
            j += 1;
        } else if table[(i + 1) * columns + j] >= table[i * columns + (j + 1)] {
            only_before += 1;
            i += 1;
        } else {
            only_after += 1;
            j += 1;
        }
    }
    only_before += before.len() - i;
    only_after += after.len() - j;

    // A line present only in the ancestor and one present only in the current version, at the same
    // position, are one line CHANGED. What is left over on either side is a removal or an addition.
    let changed = only_before.min(only_after);
    Some(VersionFacts {
        added: only_after - changed,
        changed,
        removed: only_before - changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(before: &str, after: &str) -> VersionFacts {
        compare_to_ancestor(
            &LineSummary::of(before).expect("ancestor"),
            &LineSummary::of(after).expect("current"),
        )
        .expect("inside the cutoff")
    }

    #[test]
    fn lines_are_counted_the_way_the_conflict_screen_counts_them() {
        // Both rules are `ui/diff.js`'s, and both matter: the card draws `4 lines` for a four-line
        // file, and a second splitter that disagreed would make the card's verb and the panel
        // beneath it describe different files.
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("a\nb"), vec!["a", "b"]);
        // The empty element a final newline produces is dropped.
        assert_eq!(split_lines("a\nb\n"), vec!["a", "b"]);
        // CRLF, and a lone CR, both end a line.
        assert_eq!(split_lines("a\r\nb\r\n"), vec!["a", "b"]);
        assert_eq!(split_lines("a\rb"), vec!["a", "b"]);
        // A blank line in the middle is a line; only the trailing one goes.
        assert_eq!(split_lines("a\n\nb\n"), vec!["a", "", "b"]);
        // A file that is nothing but a newline is one empty line, not zero and not two.
        assert_eq!(split_lines("\n"), vec![""]);
    }

    #[test]
    fn the_three_verbs_the_card_needs_are_told_apart() {
        let base = "# Todo\n- buy milk\n- call Alice\n";
        // Added.
        assert_eq!(
            facts(base, "# Todo\n- buy milk\n- call Alice\n- ship v1\n"),
            VersionFacts {
                added: 1,
                changed: 0,
                removed: 0
            }
        );
        // Changed — a replaced line is ONE change, not a removal plus an addition, or
        // `N differ · M identical` would not sum to the longer file's length.
        assert_eq!(
            facts(base, "# Todo\n- buy oat milk\n- call Alice\n"),
            VersionFacts {
                added: 0,
                changed: 1,
                removed: 0
            }
        );
        // Removed.
        assert_eq!(
            facts(base, "# Todo\n- call Alice\n"),
            VersionFacts {
                added: 0,
                changed: 0,
                removed: 1
            }
        );
        // And the combination the drawn card names: `Changed a line and added one`.
        assert_eq!(
            facts(base, "# Todo\n- buy oat milk\n- call Alice\n- relax\n"),
            VersionFacts {
                added: 1,
                changed: 1,
                removed: 0
            }
        );
        // Nothing moved. Reachable for a real conflict, because two files can differ as bytes and
        // not as lines — so `is_unchanged` is a state the card has to be able to say.
        assert!(facts(base, base).is_unchanged());
    }

    #[test]
    fn the_drawn_conflict_frames_are_reconciled_by_exactly_one_ancestor() {
        // #217's adjacent question, answered with the frames' own numbers. `3a Conflict diff` draws
        //
        //   yours    # Todo | - buy milk     | - call Alice | - ship v1
        //   Proton's # Todo | - buy oat milk | - call Alice | - ship v1 | - relax
        //   footer   2 lines differ · 3 lines identical
        //
        // which is self-consistent. `3a Conflict` draws `You added a line` and `Changed a line and
        // added one` beside it, and those two force `ancestor == yours` — under which the local
        // file never moved, the planner reaches `(Unchanged, Changed)` and plans a plain download.
        // THE DRAWN CARD SENTENCES DESCRIBE A STATE THIS ENGINE CANNOT PRODUCE.
        let mine = "# Todo\n- buy milk\n- call Alice\n- ship v1\n";
        let theirs = "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n";

        // The one ancestor that makes the drawn diff true AND is a genuine two-sided conflict.
        let ancestor = "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n";
        assert_eq!(
            facts(ancestor, mine),
            VersionFacts {
                added: 0,
                changed: 1,
                removed: 0
            },
            "you changed a line"
        );
        assert_eq!(
            facts(ancestor, theirs),
            VersionFacts {
                added: 1,
                changed: 0,
                removed: 0
            },
            "Proton added a line"
        );

        // And the ancestor the drawn CARDS imply is the local file itself, which is not a conflict.
        assert!(facts(mine, mine).is_unchanged());
    }

    #[test]
    fn a_file_too_large_or_too_long_has_no_summary_and_that_is_ordinary() {
        // #347's headline case is a 4 GB video whose card claimed a line had been added to it. The
        // say-less path is the first-class one, so these answer `None` rather than erroring.
        let big = "x".repeat(MAX_SUMMARY_BYTES + 1);
        assert!(LineSummary::of(&big).is_none());
        let many = "a\n".repeat(MAX_SUMMARY_LINES + 1);
        assert!(LineSummary::of(&many).is_none());
        // Exactly at the caps is still summarised — an off-by-one here silently drops a whole band
        // of ordinary files into the say-less path.
        assert!(LineSummary::of(&"x".repeat(MAX_SUMMARY_BYTES)).is_some());
        assert_eq!(
            LineSummary::of(&"a\n".repeat(MAX_SUMMARY_LINES))
                .expect("at the cap")
                .line_count(),
            MAX_SUMMARY_LINES
        );
    }

    #[test]
    fn a_summary_round_trips_through_its_stored_form() {
        for text in ["", "\n", "a", "a\nb\n", "a\n\nb"] {
            let summary = LineSummary::of(text).expect("summary");
            assert_eq!(
                LineSummary::decode(&summary.encode()),
                summary,
                "round trip for {text:?}"
            );
        }
        // An empty stored value is a ZERO-line summary, not a one-line one — the difference between
        // "the agreed version was empty" and "the agreed version had one blank line", which are two
        // different files and two different sentences.
        assert_eq!(LineSummary::decode("").line_count(), 0);
        assert_eq!(LineSummary::of("").expect("empty").encode(), "");
        assert_eq!(LineSummary::of("\n").expect("newline").line_count(), 1);
    }

    #[test]
    fn every_edit_shape_is_counted_by_position_and_never_double_counted() {
        // The property the card's arithmetic rests on: whatever the shape, the counts describe one
        // move per file position, so `added + changed` never exceeds the current version's length
        // and `removed + changed` never exceeds the ancestor's.
        let shapes: &[(&str, &str)] = &[
            ("", "a\n"),
            ("a\n", ""),
            ("a\nb\nc\n", "c\nb\na\n"),
            ("a\nb\nc\n", "a\nx\nc\n"),
            ("a\nb\nc\n", "a\nb\nc\nd\ne\n"),
            ("a\nb\nc\nd\ne\n", "a\nc\ne\n"),
            ("a\n", "b\n"),
            ("a\nb\n", "b\na\n"),
        ];
        for (before, after) in shapes {
            let summary = |t: &str| LineSummary::of(t).expect("summary");
            let (b, a) = (summary(before), summary(after));
            let f = compare_to_ancestor(&b, &a).expect("inside the cutoff");
            assert!(
                f.added + f.changed <= a.line_count(),
                "{before:?} -> {after:?} claims {f:?} against {} current lines",
                a.line_count()
            );
            assert!(
                f.removed + f.changed <= b.line_count(),
                "{before:?} -> {after:?} claims {f:?} against {} ancestor lines",
                b.line_count()
            );
            // And an unchanged verdict is reserved for genuinely identical line sequences.
            assert_eq!(f.is_unchanged(), b == a, "{before:?} -> {after:?}");
        }
    }
}
