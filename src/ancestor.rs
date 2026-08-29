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
/// Counted the way `ui/diff.js`'s `pairBlocks` counts, because the panel drawn under this sentence
/// is built from that: a removal and an insertion **inside one contiguous run** are one `changed`,
/// and runs separated by an agreeing line are never paired with each other. Pairing globally is
/// wrong and was measurably wrong — a line removed at the top and a different line added at the
/// bottom read as "one line changed" while the disclosure beneath found zero changed pairs.
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

    // The op stream, in file order: kept lines, lines only the ancestor had, lines only the current
    // version has.
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum Op {
        Keep,
        OnlyBefore,
        OnlyAfter,
    }
    let mut ops: Vec<Op> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            ops.push(Op::Keep);
            i += 1;
            j += 1;
        } else if table[(i + 1) * columns + j] >= table[i * columns + (j + 1)] {
            ops.push(Op::OnlyBefore);
            i += 1;
        } else {
            ops.push(Op::OnlyAfter);
            j += 1;
        }
    }
    ops.extend(std::iter::repeat_n(Op::OnlyBefore, before.len() - i));
    ops.extend(std::iter::repeat_n(Op::OnlyAfter, after.len() - j));

    // PAIRED WITHIN EACH CONTIGUOUS RUN, never across the whole file — and this is the rule, not an
    // optimisation. `ui/diff.js`'s `pairBlocks` pairs per block, and the panel drawn directly under
    // this sentence is built from it: pairing globally called a line removed at the top and a
    // different line added at the bottom "one line changed", while the panel beneath found zero
    // changed pairs and one line exclusive to each side. The card and its own disclosure then
    // contradicted each other, which `pairBlocks`' doc already names as worse than either alone.
    let (mut added, mut changed, mut removed) = (0usize, 0usize, 0usize);
    let mut at = 0;
    while at < ops.len() {
        if ops[at] == Op::Keep {
            at += 1;
            continue;
        }
        let start = at;
        while at < ops.len() && ops[at] != Op::Keep {
            at += 1;
        }
        let block = &ops[start..at];
        let only_before = block.iter().filter(|op| **op == Op::OnlyBefore).count();
        let only_after = block.len() - only_before;
        let paired = only_before.min(only_after);
        changed += paired;
        removed += only_before - paired;
        added += only_after - paired;
    }
    Some(VersionFacts {
        added,
        changed,
        removed,
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
    fn the_drawn_conflict_cards_describe_a_state_no_ancestor_produces() {
        // #217's adjacent question, answered with the frames' own numbers. `3a Conflict diff` draws
        //
        //   yours    # Todo | - buy milk     | - call Alice | - ship v1     (4 lines)
        //   Proton's # Todo | - buy oat milk | - call Alice | - ship v1 | - relax  (5 lines)
        //   footer   2 lines differ · 3 lines identical
        //
        // which is self-consistent. `3a Conflict` draws `You added a line` and `Changed a line and
        // added one` beside it, and THOSE TWO CANNOT BOTH BE TRUE OF ANY ANCESTOR.
        //
        // The derivation is a length identity, not a search: for either side,
        // `added - removed == |side| - |ancestor|`. `You added a line` gives `1 - 0 == 4 - |B|`, so
        // `|B| == 3`. `Changed a line and added one` gives `1 - 0 == 5 - |B|`, so `|B| == 4`. No
        // ancestor has both lengths. (An earlier version of this test claimed the two "force
        // ancestor == yours" and that exactly ONE ancestor reconciles the frames — adversarial
        // review refuted both: the drawn diff constrains only yours against Proton's, so the
        // ancestors consistent with it are infinite, and `- buy soy milk` is another.)
        //
        // What the fixture needs is therefore not "the" ancestor but A defensible one: consistent
        // with the drawn diff and a genuine two-sided conflict. This is the one it carries.
        let mine = "# Todo\n- buy milk\n- call Alice\n- ship v1\n";
        let theirs = "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n";

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

        // The length identity itself, so the refutation is asserted rather than argued in a comment.
        let lines_of = |t: &str| LineSummary::of(t).expect("summary").line_count() as i64;
        let implied = |side: &str, added: i64, removed: i64| lines_of(side) - (added - removed);
        assert_eq!(
            implied(mine, 1, 0),
            3,
            "`You added a line` implies a 3-line ancestor"
        );
        assert_eq!(
            implied(theirs, 1, 0),
            4,
            "`Changed a line and added one` implies a 4-line ancestor"
        );

        // And another ancestor reconciles the diff just as well, which is why "exactly one" was
        // wrong: the drawn diff says nothing about the ancestor at all.
        let alternative = "# Todo\n- buy soy milk\n- call Alice\n- ship v1\n";
        assert_eq!(
            facts(alternative, mine),
            VersionFacts {
                added: 0,
                changed: 1,
                removed: 0
            }
        );
    }

    #[test]
    fn a_removal_and_an_unrelated_addition_are_not_one_changed_line() {
        // FOUND BY ADVERSARIAL REVIEW, and it contradicted the panel drawn directly beneath the
        // sentence. Pairing across the whole file called a line removed at the top and a different
        // line added at the bottom "one line changed", while `ui/diff.js`'s `compare` on the same
        // two files reported zero changed pairs and one line exclusive to each side — the card and
        // its own disclosure disagreeing, which is the failure `pairBlocks` names as worse than
        // either alone.
        assert_eq!(
            facts(
                "alpha\nbeta\ngamma\ndelta\n",
                "beta\ngamma\ndelta\nepsilon\n"
            ),
            VersionFacts {
                added: 1,
                changed: 0,
                removed: 1
            },
            "a removal and an addition separated by agreeing lines are two moves, not one"
        );
        // Five and five, the review's larger case.
        assert_eq!(
            facts(
                "a1\na2\na3\na4\na5\nkeep1\nkeep2\n",
                "keep1\nkeep2\nz1\nz2\nz3\nz4\nz5\n"
            ),
            VersionFacts {
                added: 5,
                changed: 0,
                removed: 5
            }
        );
        // And the pairing still happens INSIDE a run, which is what makes `changed` a real verb.
        assert_eq!(
            facts("a\nb\nc\n", "a\nx\ny\nc\n"),
            VersionFacts {
                added: 1,
                changed: 1,
                removed: 0
            },
            "one replaced line and one inserted line in the same run"
        );
    }

    /// The corpus both diffs are held to. **Kept byte-identical in `gui/test/diff.test.js`**, where
    /// `ui/diff.js`'s `compare` is asserted to the same numbers — because the card's first line and
    /// the panel drawn beneath it come from two different implementations, and the only way that
    /// stays true is to check it rather than to say it in a comment. Verified differentially over
    /// these twenty pairs; the four that used to disagree are the first four.
    const AGREEING_CORPUS: &[(&str, &str, [usize; 3])] = &[
        // added, changed, removed
        (
            "alpha\nbeta\ngamma\ndelta",
            "beta\ngamma\ndelta\nepsilon",
            [1, 0, 1],
        ),
        (
            "a1\na2\na3\na4\na5\nkeep1\nkeep2",
            "keep1\nkeep2\nz1\nz2\nz3\nz4\nz5",
            [5, 0, 5],
        ),
        ("head\nx\ntail", "head\ny\nz\ntail", [1, 1, 0]),
        ("head\nx\ny\ntail", "head\nz\ntail", [0, 1, 1]),
        ("a\nb\nc", "a\nx\ny\nc", [1, 1, 0]),
        ("a\nb\nc", "a\nb\nc", [0, 0, 0]),
        ("a\nb\nc\nd\ne", "a\nc\ne", [0, 0, 2]),
        ("l1\nl2\nl3\nl4\nl5\nl6", "l1\nX\nl3\nl4\nY\nl6", [0, 2, 0]),
        ("a\na\na", "a\na", [0, 0, 1]),
        ("p\nq", "p\nq\nr\ns", [2, 0, 0]),
    ];

    #[test]
    fn the_rust_and_javascript_diffs_agree_on_a_shared_corpus() {
        for (before, after, [added, changed, removed]) in AGREEING_CORPUS {
            assert_eq!(
                facts(before, after),
                VersionFacts {
                    added: *added,
                    changed: *changed,
                    removed: *removed
                },
                "{before:?} -> {after:?}"
            );
        }
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
