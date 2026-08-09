// ui/diff.js (C3) — the prose diff summary behind the conflict version cards.
//
// Tested rather than left to the fidelity gate for the reason S1 wrote down: the gate compares one
// rendering of each drawn frame, and every interesting case here is a file nobody drew. `3a Conflict`
// draws exactly two shopping lists; a CRLF sidecar, a trailing-newline-only difference and a 500 KB
// file are all invisible to it, and two of those three make the card claim the versions agree.
//
// The frames' own two files are the first two tests, so the drawn sentences stay ground truth.

import { test } from "node:test";
import assert from "node:assert/strict";
import { alignedRows, compare, lines, quote, summariseSide, MAX_QUOTE_CHARS } from "../src/js/ui/diff.js";
import { fileSize, EM_DASH } from "../src/js/ui/format.js";
import { CONFLICTS } from "../src/js/ui/copy.js";

// The two versions `3a Conflict` is drawn from: yours has `buy milk`, Proton's has `buy oat milk`
// and one line more at the end.
const MINE = "milk\nbuy milk\neggs\nbread\n";
const THEIRS = "milk\nbuy oat milk\neggs\nbread\ncoffee\n";

test("the drawn pair produces the drawn sentences", () => {
  const comparison = compare(MINE, THEIRS);
  assert.equal(comparison.changed.length, 1);
  assert.deepEqual(comparison.changed[0], { mine: "buy milk", theirs: "buy oat milk" });
  assert.equal(comparison.onlyMine.length, 0);
  assert.equal(comparison.onlyTheirs.length, 1);
  assert.equal(comparison.onlyTheirs[0].atEnd, true, "the extra line is at the end");

  assert.equal(
    CONFLICTS.versionDiff("mine", summariseSide(comparison, "mine")),
    "Yours has buy milk where Proton's has something else, and is otherwise the same.",
  );
  assert.equal(
    CONFLICTS.versionDiff("theirs", summariseSide(comparison, "theirs")),
    "Proton's has buy oat milk and an extra line at the end.",
  );
});

test("the counts under the diff are the drawn ones", () => {
  const comparison = compare(MINE, THEIRS);
  // `2 lines differ · 3 lines identical`: the changed pair and the extra line differ; milk, eggs
  // and bread agree.
  assert.equal(comparison.differing, 2);
  assert.equal(comparison.identical, 3);
  assert.equal(CONFLICTS.diffSummary(2), "Two lines differ. Everything else in the file matches.");
  assert.equal(CONFLICTS.diffCounts(2, 3), "2 lines differ · 3 lines identical");
});

test("the metadata row's line counts ignore the trailing newline", () => {
  // The frame draws `4 lines` and `5 lines`; split("\n").length would say 5 and 6.
  assert.equal(lines(MINE).length, 4);
  assert.equal(lines(THEIRS).length, 5);
});

test("the count sentences agree in number at one", () => {
  assert.equal(CONFLICTS.diffSummary(1), "One line differs. Everything else in the file matches.");
  assert.equal(CONFLICTS.diffCounts(1, 1), "1 line differs · 1 line identical");
});

test("a CRLF sidecar does not make every line differ", () => {
  // A sidecar written on another platform. Byte-wise nothing matches; in content, one line does.
  const crlf = THEIRS.replace(/\n/g, "\r\n");
  const comparison = compare(MINE, crlf);
  assert.equal(comparison.differing, 2, "the same two differences as the LF pair");
  assert.equal(comparison.identical, 3);
});

test("a difference that is only a trailing newline is refused, not called sameness", () => {
  // The files DO differ — the daemon wrote a sidecar — but not on any line. Saying `otherwise the
  // same` here would argue for discarding a version over a difference the screen never showed.
  const comparison = compare("a\nb", "a\nb\n");
  assert.equal(comparison.differing, 0);
  assert.equal(comparison.invisibleDifference, true);
  assert.equal(summariseSide(comparison, "mine"), null);
  assert.equal(summariseSide(comparison, "theirs"), null);
});

test("truly identical text is not an invisible difference", () => {
  const comparison = compare(MINE, MINE);
  assert.equal(comparison.differing, 0);
  assert.equal(comparison.invisibleDifference, false);
  assert.equal(summariseSide(comparison, "mine"), null, "nothing to say either way");
});

test("a side missing its text is the metadata row, not an empty file", () => {
  // `ConflictSide` carries `text: null` for binary, too-large AND missing files — and for a missing
  // file `binary_or_large` is FALSE, so a guard on that flag alone reads a vanished file as empty
  // and renders "Proton's has nothing" out of thin air.
  assert.equal(compare(MINE, null), null);
  assert.equal(compare(null, THEIRS), null);
  assert.equal(compare(undefined, undefined), null);
});

test("more than one changed line falls back rather than quoting one of them", () => {
  const comparison = compare("a\nb\nc\n", "A\nb\nC\n");
  assert.equal(comparison.changed.length, 2);
  assert.equal(
    summariseSide(comparison, "mine"),
    null,
    "quoting one of two changes describes the file wrongly, not partially",
  );
});

test("each card describes its own side, so a removal is the other side's gain", () => {
  // Yours has a line Proton's does not. That is a sentence for YOUR card — Proton's card has
  // nothing of its own to report, and says nothing rather than describing your file for you.
  const comparison = compare("a\nb\n", "a\n");
  assert.deepEqual(summariseSide(comparison, "mine"), {
    quoted: null,
    extraAtEnd: 1,
    otherwiseSame: false,
  });
  assert.equal(
    CONFLICTS.versionDiff("mine", summariseSide(comparison, "mine")),
    "Yours has an extra line at the end.",
  );
  assert.equal(summariseSide(comparison, "theirs"), null, "nothing of its own to say");
});

test("`otherwise the same` is about this side, which is what the frame draws", () => {
  // The drawn left card claims it while Proton's has an extra line. Reading the clause as "the
  // files are otherwise equal" makes the drawn sentence unreachable at its own frame's input.
  const facts = summariseSide(compare(MINE, THEIRS), "mine");
  assert.equal(facts.otherwiseSame, true);
  assert.equal(facts.extraAtEnd, 0);
});

test("an extra line in the middle is refused — the only drawn clause says at the end", () => {
  const comparison = compare("a\nc\n", "a\nb\nc\n");
  assert.equal(comparison.onlyTheirs.length, 1);
  assert.equal(comparison.onlyTheirs[0].atEnd, false);
  assert.equal(summariseSide(comparison, "theirs"), null);
});

test("several extra lines at the end are counted", () => {
  const comparison = compare("a\n", "a\nb\nc\n");
  const facts = summariseSide(comparison, "theirs");
  assert.deepEqual(facts, { quoted: null, extraAtEnd: 2, otherwiseSame: false });
  assert.equal(CONFLICTS.versionDiff("theirs", facts), "Proton's has 2 extra lines at the end.");
});

test("a blank changed line is dropped rather than quoted as nothing", () => {
  const comparison = compare("a\n   \nb\n", "a\nx\nb\n");
  assert.equal(comparison.changed.length, 1);
  assert.equal(summariseSide(comparison, "mine"), null, "nothing to put in the mono span");
  assert.ok(summariseSide(comparison, "theirs"), "the side with content still speaks");
});

test("a very long line is quoted up to the cap and says it was cut", () => {
  const long = "x".repeat(MAX_QUOTE_CHARS + 40);
  assert.equal(quote(long).length, MAX_QUOTE_CHARS + 1);
  assert.ok(quote(long).endsWith("⋯"));
  assert.equal(quote("  padded  "), "padded", "and quotes are trimmed");
});

test("two files with nothing in common are refused instead of freezing the window", () => {
  // 512 KB of text is ~10^4 lines a side, and an O(n·m) table over that is 10^8 cells on the
  // WebKit main thread. The trim makes the realistic case cheap; this is the case it cannot help.
  const mine = Array.from({ length: 800 }, (_, i) => `mine ${i}`).join("\n");
  const theirs = Array.from({ length: 800 }, (_, i) => `theirs ${i}`).join("\n");
  assert.equal(compare(mine, theirs), null);
});

test("a one-line change inside a large file is still summarised", () => {
  // The trim is what makes this affordable: the middle is 1×1 however big the file is.
  const body = Array.from({ length: 5000 }, (_, i) => `line ${i}`);
  const mine = body.join("\n");
  const changed = [...body];
  changed[2500] = "line 2500 edited";
  const comparison = compare(mine, changed.join("\n"));
  assert.equal(comparison.changed.length, 1);
  assert.deepEqual(summariseSide(comparison, "theirs"), {
    quoted: "line 2500 edited",
    extraAtEnd: 0,
    otherwiseSame: true,
  });
});

// ---- the eight defects an adversarial review found, none of which any gate could see ----

test("two lines edited in place are TWO changed lines, not one change plus two inventions", () => {
  // lcsOps emits a contiguous edited block as k removals THEN k insertions. Pairing one op ahead
  // matched the last removal with the first insertion and orphaned the rest — so the file came back
  // as one changed line plus a line each side had supposedly gained, which also slipped under the
  // `changed.length > 1` refusal. The card then described a file it had misread, confidently.
  const comparison = compare("eggs\ncoffee\ntea\n", "eggs\nmilk\nbread\n");
  assert.equal(comparison.changed.length, 2, "two lines changed");
  assert.deepEqual(comparison.onlyMine, [], "nothing was removed");
  assert.deepEqual(comparison.onlyTheirs, [], "nothing was added");
  assert.equal(summariseSide(comparison, "mine"), null, "and the refusal fires");
});

test("an unequal block pairs what it can and reports the remainder", () => {
  const comparison = compare("a\nb\nc\nz\n", "A\nz\n");
  assert.equal(comparison.changed.length, 1);
  assert.equal(comparison.onlyMine.length, 2, "the two unpaired removals");
  assert.equal(comparison.onlyTheirs.length, 0);
});

test("a leftover line is not 'at the end' when the changed line's counterpart follows it", () => {
  // The block's unpaired removals sit BEFORE its insertions in op order, so nothing agreed follows
  // them and the old rule — "after the last keep" — called them appends. Both would then have
  // passed the line-count check (3 - 1 = 2 = two extras), and the card would have read
  // `Yours has a and 2 extra lines at the end.` about a line that was changed, not added.
  const comparison = compare("a\nb\nc\n", "X\n");
  assert.deepEqual(comparison.changed, [{ mine: "a", theirs: "X" }]);
  assert.deepEqual(
    comparison.onlyMine.map((extra) => extra.atEnd),
    [false, false],
    "the insertion at the end of the block comes after them",
  );
  assert.equal(summariseSide(comparison, "mine"), null, "so the sentence falls back");
});

test("a reordered file has gained nothing, and says nothing", () => {
  // LCS scores a moved line as a removal plus an insertion, so `milk` looks appended — while both
  // files are three lines long. `Proton's has an extra line at the end.` would be false.
  const comparison = compare("milk\neggs\nbread\n", "eggs\nbread\nmilk\n");
  assert.equal(comparison.mineLines, comparison.theirsLines);
  assert.equal(summariseSide(comparison, "theirs"), null);
  assert.equal(summariseSide(comparison, "mine"), null);
});

test("the counts partition the longer file rather than double-counting a swap", () => {
  // One line dropped from the top and a different one appended at the bottom: adding the two sides'
  // extras made `differ + identical` exceed the file's own length.
  const comparison = compare("alpha\nbeta\ngamma\n", "beta\ngamma\ndelta\n");
  assert.equal(
    comparison.differing + comparison.identical,
    Math.max(comparison.mineLines, comparison.theirsLines),
  );
  // And the drawn frame's numbers still hold: 2 + 3 = 5 = the longer file.
  const drawn = compare(MINE, THEIRS);
  assert.equal(drawn.differing + drawn.identical, Math.max(drawn.mineLines, drawn.theirsLines));
});

test("a difference only in trailing whitespace is not quoted twice as itself", () => {
  // Both cards would quote `buy milk` while each insists the other side has something else.
  const comparison = compare("buy milk  \n", "buy milk\n");
  assert.equal(comparison.changed.length, 1);
  assert.equal(summariseSide(comparison, "mine"), null);
  assert.equal(summariseSide(comparison, "theirs"), null);
});

test("truncation never cuts a character in half", () => {
  const line = "x".repeat(MAX_QUOTE_CHARS - 1) + "😀 and more text after it";
  const quoted = quote(line);
  assert.ok(quoted.endsWith("⋯"));
  assert.doesNotMatch(
    quoted,
    /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/,
    "a lone surrogate renders as ? and gets attributed to the user's file",
  );
  assert.ok(quoted.includes("😀"), "the emoji is kept whole");
});

test("the disclosure refuses to count a difference no line shows", () => {
  // `cardinal(0)` is the deliberately lower-cased `zero`, so the header would open a sentence with
  // it — under a heading claiming the rest of the file matches, about a file the daemon just wrote
  // a conflict sidecar for.
  const comparison = compare("a\nb", "a\nb\n");
  assert.equal(comparison.differing, 0);
  assert.equal(CONFLICTS.diffSummary(comparison.differing), null);
  assert.equal(CONFLICTS.diffCounts(comparison.differing, comparison.identical), null);
});

test("the template answers null for the case that falls back, rather than throwing", () => {
  // `summariseSide` returns null for every comparison outside the drawn grammar — including a
  // multi-line edit, which is what most real conflicts are. S2's most-travelled path therefore feeds
  // this template null, and a TypeError there is a blank card rather than the documented fallback.
  const multiLine = compare("a\nb\nc\n", "A\nb\nC\n");
  assert.equal(summariseSide(multiLine, "mine"), null);
  assert.equal(CONFLICTS.versionDiff("mine", summariseSide(multiLine, "mine")), null);
  assert.equal(CONFLICTS.versionDiff("theirs", null), null);
  assert.equal(CONFLICTS.versionDiff("mine", undefined), null);
});

test("an unknown side is a programming error, not a silent default", () => {
  const comparison = compare(MINE, THEIRS);
  assert.throws(() => summariseSide(comparison, "left"), /must be "mine" or "theirs"/);
});

// ---- the diff panel's rows (S2) ----

test("the drawn panel is five rows over a four-line and a five-line file", () => {
  // `04-conflicts.md` makes the seam the diff's gutter, so the two sides must stay LEVEL: the row
  // is the unit, not the line. The frame draws four rows on the left against five on the right,
  // row 2 highlighted as a changed pair and row 5 absent on the left.
  const rows = alignedRows(MINE, THEIRS);
  assert.deepEqual(
    rows.map((r) => r.kind),
    ["unchanged", "changed", "unchanged", "unchanged", "absent"],
  );
  assert.deepEqual(rows[1], {
    kind: "changed",
    mine: { n: 2, text: "buy milk" },
    theirs: { n: 2, text: "buy oat milk" },
  });
  assert.equal(rows[4].mine, null, "nothing on the left — the panel draws the placeholder");
  assert.deepEqual(rows[4].theirs, { n: 5, text: "coffee" });
});

test("line numbers count each side's OWN file and skip on an absent row", () => {
  // The left column of a five-row panel over a four-line file reads 1,2,3,4 with a gap — not 1..5.
  const rows = alignedRows(MINE, THEIRS);
  assert.deepEqual(
    rows.map((r) => r.mine?.n ?? null),
    [1, 2, 3, 4, null],
  );
  assert.deepEqual(
    rows.map((r) => r.theirs?.n ?? null),
    [1, 2, 3, 4, 5],
  );
});

test("the rows agree with the counts drawn beneath them", () => {
  const rows = alignedRows(MINE, THEIRS);
  const comparison = compare(MINE, THEIRS);
  assert.equal(rows.filter((r) => r.kind === "unchanged").length, comparison.identical);
  assert.equal(rows.filter((r) => r.kind !== "unchanged").length, comparison.differing);
  assert.equal(
    CONFLICTS.diffCounts(comparison.differing, comparison.identical),
    "2 lines differ · 3 lines identical",
  );
});

test("the panel and the cards cannot disagree about what changed", () => {
  // Both go through the same block pairing. Two lines edited in place is two CHANGED rows — not one
  // change plus two absents, which is what a second implementation of that loop produced.
  const rows = alignedRows("eggs\ncoffee\ntea\n", "eggs\nmilk\nbread\n");
  assert.deepEqual(
    rows.map((r) => r.kind),
    ["unchanged", "changed", "changed"],
  );
  assert.equal(compare("eggs\ncoffee\ntea\n", "eggs\nmilk\nbread\n").changed.length, 2);
});

test("a line each side lacks produces two absent rows, not one merged row", () => {
  const rows = alignedRows("alpha\nbeta\ngamma\n", "beta\ngamma\ndelta\n");
  assert.deepEqual(
    rows.map((r) => [r.kind, r.mine?.text ?? null, r.theirs?.text ?? null]),
    [
      ["absent", "alpha", null],
      ["unchanged", "beta", "beta"],
      ["unchanged", "gamma", "gamma"],
      ["absent", null, "delta"],
    ],
  );
});

test("the panel refuses exactly what the cards refuse", () => {
  // The disclosure has nothing to open onto when a side has no text at all.
  assert.equal(alignedRows(MINE, null), null);
  assert.equal(alignedRows(null, THEIRS), null);
  const wide = Array.from({ length: 800 }, (_, i) => `mine ${i}`).join("\n");
  const other = Array.from({ length: 800 }, (_, i) => `theirs ${i}`).join("\n");
  assert.equal(alignedRows(wide, other), null);
  assert.equal(compare(wide, other), null);
});

test("identical files are all unchanged rows", () => {
  const rows = alignedRows(MINE, MINE);
  assert.equal(rows.length, 4);
  assert.ok(rows.every((r) => r.kind === "unchanged"));
});

// ---- what the review found: two ways the card could state a fact it does not have ----

test("an empty-but-readable file has no lines, and the card counts them the panel's way", () => {
  // The card's own `split` said 1 — `"".split("\n")` is `[""]` — while the panel beneath it drew
  // none. `versionCard` now asks `lines()` instead of reimplementing it, so there is one answer.
  assert.equal(lines("").length, 0);
  assert.equal(lines(null).length, 0);
  // And the two drawn files still count the way the metadata row draws them.
  assert.equal(lines("# Todo\n- buy milk\n- call Alice\n- ship v1\n").length, 4);
  assert.equal(lines("# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n").length, 5);
});

test("an unread size is an em-dash, not `0 bytes`", () => {
  // The pair arrives a render after the screen does. `fileSize(source?.size ?? 0)` drew `0 bytes`
  // in the gap — indistinguishable from a real empty file, on a card whose job is telling two
  // versions apart by their facts.
  assert.equal(fileSize(undefined), EM_DASH);
  assert.equal(fileSize(null), EM_DASH);
  assert.equal(fileSize(0), "0 bytes");
  assert.equal(fileSize(41), "41 bytes");
});

test("a quoted line loses its list marker but keeps a numbered item's number", () => {
  // `3a Conflict` quotes `buy milk` out of a line reading `- buy milk`.
  assert.equal(quote("- buy milk"), "buy milk");
  assert.equal(quote("* buy milk"), "buy milk");
  assert.equal(quote("+ buy milk"), "buy milk");
  // Not `1.` — the number is content the reader may be pointing at, and dropping it would quote
  // `1. call Alice` and `2. call Alice` identically.
  assert.equal(quote("1. call Alice"), "1. call Alice");
  // A bare hyphen with no space is a word, not a marker.
  assert.equal(quote("-buy milk"), "-buy milk");
});

test("`both versions` never agrees with the file count", () => {
  // `both` is inherently two — the two versions of ONE file — while the number in front of it
  // counts files. Agreeing the noun with that number gave `one kept both version`.
  assert.equal(
    CONFLICTS.clearedSub({ total: 1, keptBoth: 1, tookProton: 0 }),
    "You settled 1 file. One kept both versions.",
  );
  // The drawn sentence, unchanged.
  assert.equal(
    CONFLICTS.clearedSub({ total: 3, keptBoth: 2, tookProton: 1 }),
    "You settled 3 files. Two kept both versions, one took Proton's copy.",
  );
  // A mix the deck has no wording for drops the clause rather than inventing grammar.
  assert.equal(CONFLICTS.clearedSub({ total: 3, keptBoth: 1, tookProton: 0 }), "You settled 3 files.");
});
