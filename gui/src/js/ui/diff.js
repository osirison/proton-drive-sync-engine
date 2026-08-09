// What differs between two versions of a file, in words (C3, #176).
//
// `04-conflicts.md` gives the version cards three parts, and this module is the second one:
//
//   1. What happened — `You added a line, 5 minutes ago`
//   2. What differs, in words — `Yours has buy milk where Proton's has something else, and is
//      otherwise the same.`
//   3. The metadata row — bytes, line count, edit time
//
// The doc says (2) "needs a diff summary from the daemon, not just a byte diff". It does not:
// `read_conflict_pair` already returns both texts, so the whole thing is computable in the browser
// with no new socket verb and no daemon work. That is the entire point of C3.
//
// PART (1) IS NOT COMPUTABLE AND THIS MODULE DOES NOT PRETEND OTHERWISE. `You added a line` is a
// statement about your version against the **last agreed** version, and no last-agreed version
// exists anywhere on this machine: the conflict sidecar is Proton's *current* copy, the local file
// is yours, and the index keeps the baseline's SHA-1 without its content. Against Proton's copy
// alone the same edit reads as a removal, so attributing a change to one side is not a harder
// version of this problem — it is a different one, needing a capability nothing has. The relative
// time in that sentence comes from the mtimes and is fine; the verb is a gap.
//
// THE FALLBACK IS SILENCE, NOT A DIFF. `04-conflicts.md` is explicit: if a summary can't be
// generated, fall back to the metadata row alone and **do not** show the raw diff there — that is
// what the `See the exact differences` disclosure is for. So every function here returns `null`
// rather than a hedge, and `describe()` refuses any shape whose sentence the copy deck does not
// actually draw.

/**
 * How much diffing is worth doing. `read_conflict_pair` already caps a side at 512 KB, which is
 * still ~10,000 lines a side — an O(n·m) table over that is 100M cells for one settings sentence.
 *
 * The cap applies to the MIDDLE, after the common prefix and suffix are trimmed, so it bounds the
 * part that actually differs rather than the file. A one-line change in a 9,000-line file trims to
 * a 1×1 middle and is summarised; two files that share nothing are refused, and a card that says
 * nothing is the documented outcome for that.
 *
 * Counted as the table `lcsOps` actually allocates — `(n + 1) × (m + 1)`, not `n × m` — so the
 * number here is the number of cells, and the guard cannot be passed by an input that then
 * allocates more than it promised.
 */
export const MAX_DIFF_CELLS = 250_000;

/**
 * How much of a changed line the sentence quotes before it stops.
 *
 * No frame pins a limit — every drawn quote is a shopping-list line — but the quote lands in an
 * inline 12px mono span inside a 13px sentence, and one minified line would push the card out of
 * the window. Truncation is visible (`⋯`, the ellipsis the prototype itself uses) rather than
 * silent, because the sentence presents the quote as the file's own words.
 */
export const MAX_QUOTE_CHARS = 60;

/**
 * Split a file into lines.
 *
 * Line endings are normalised first. `read_conflict_pair` returns raw UTF-8 with no newline
 * handling, so a sidecar written on another platform against a locally-written original differs on
 * **every** line at the byte level — a summary reading `400 lines differ` when the files are
 * identical in content is the worst kind of wrong here, since it argues for discarding a version.
 *
 * The empty trailing element a final newline produces is dropped: `4 lines` is what the metadata
 * row draws for the four-line file, and `split("\n").length` would say five.
 */
export function lines(text) {
  if (text == null || text === "") return [];
  const split = text.replace(/\r\n?/g, "\n").split("\n");
  if (split.length > 0 && split[split.length - 1] === "") split.pop();
  return split;
}

/**
 * Truncate a quoted line to [`MAX_QUOTE_CHARS`], marking the cut.
 *
 * Cut on CODE POINTS, not UTF-16 units. `slice()` will happily split a surrogate pair, and the
 * lone half renders as `�` — attributed, in a sentence that presents the quote as the file's own
 * words, to the user's file. Any line whose 60th unit is the lead of an emoji or a non-BMP script
 * hits it.
 */
export function quote(line) {
  const trimmed = line.trim();
  const points = Array.from(trimmed);
  if (points.length <= MAX_QUOTE_CHARS) return trimmed;
  return `${points.slice(0, MAX_QUOTE_CHARS).join("").trimEnd()}⋯`;
}

/**
 * A line-level comparison of two versions.
 *
 * Returns `null` when the two are too far apart to compare inside [`MAX_DIFF_CELLS`], or when
 * either side has no text at all (binary, too large, or missing — `ConflictSide.text` is `null` for
 * each of those, and they are metadata-row cases by design).
 *
 * The shape is deliberately about *positions*, not about who did what:
 *   · `changed` — pairs that occupy the same place in both files and differ
 *   · `onlyMine` / `onlyTheirs` — lines present on one side and not the other
 *   · `identical` — lines both files agree on
 */
export function compare(mineText, theirsText) {
  if (typeof mineText !== "string" || typeof theirsText !== "string") return null;
  const mine = lines(mineText);
  const theirs = lines(theirsText);

  // Trim the agreeing head and tail. Beyond being the whole reason a large file is affordable, it
  // is also what makes "an extra line AT THE END" answerable: a suffix that agrees means the extra
  // line is not at the end, and the trim is where that becomes visible.
  let head = 0;
  while (head < mine.length && head < theirs.length && mine[head] === theirs[head]) head += 1;
  let tail = 0;
  while (
    tail < mine.length - head &&
    tail < theirs.length - head &&
    mine[mine.length - 1 - tail] === theirs[theirs.length - 1 - tail]
  ) {
    tail += 1;
  }

  const midMine = mine.slice(head, mine.length - tail);
  const midTheirs = theirs.slice(head, theirs.length - tail);
  if ((midMine.length + 1) * (midTheirs.length + 1) > MAX_DIFF_CELLS) return null;

  const ops = lcsOps(midMine, midTheirs);

  // Pair removals with insertions BLOCK BY BLOCK, not one op at a time. Without the pairing every
  // edit reads as "removed a line and added a line", which is true and useless — the sentence the
  // design wants is about a line that *changed*.
  //
  // The block is the load-bearing part. `lcsOps` emits a contiguous edited region as k consecutive
  // removals followed by k consecutive insertions, so a look-one-ahead pairing matches only the
  // LAST removal with the FIRST insertion and orphans the rest: two lines edited in place came back
  // as one changed line plus one line each side supposedly gained. That not only invents content —
  // it slips under the `changed.length > 1` refusal, so the card confidently described a file it
  // had misread. Consuming a maximal run and pairing `min(removals, insertions)` keeps the count
  // honest, which is what the refusal is defending.
  const changed = [];
  const onlyMineOps = [];
  const onlyTheirsOps = [];
  for (let i = 0; i < ops.length;) {
    if (ops[i].kind === "keep") {
      i += 1;
      continue;
    }
    const removals = [];
    const insertions = [];
    let end = i;
    while (end < ops.length && ops[end].kind !== "keep") {
      (ops[end].kind === "remove" ? removals : insertions).push({ line: ops[end].line, at: end });
      end += 1;
    }
    const pairs = Math.min(removals.length, insertions.length);
    for (let k = 0; k < pairs; k += 1) {
      changed.push({ mine: removals[k].line, theirs: insertions[k].line });
    }
    onlyMineOps.push(...removals.slice(pairs));
    onlyTheirsOps.push(...insertions.slice(pairs));
    i = end;
  }

  // "At the end" means **nothing else comes after it** — the trimmed tail is empty, and every op
  // following this one is another extra on the same side. Keying on the last `keep` instead called
  // a line inserted mid-file "at the end" whenever the edit *after* it was a change rather than an
  // agreement, which is an ordinary shape: insert a line, then edit the last one.
  const trailingRun = (owned) => {
    const indices = new Set(owned.map((entry) => entry.at));
    let boundary = ops.length;
    while (boundary > 0 && indices.has(boundary - 1)) boundary -= 1;
    return boundary;
  };
  const mark = (owned) => {
    const from = tail > 0 ? ops.length : trailingRun(owned);
    return owned.map((entry) => ({ line: entry.line, atEnd: entry.at >= from }));
  };
  const onlyMine = mark(onlyMineOps);
  const onlyTheirs = mark(onlyTheirsOps);

  const identical = head + tail + ops.filter((op) => op.kind === "keep").length;
  // Counted per FILE POSITION, not per side: a line dropped from one side and a different line
  // appended to the other occupy one row each in the diff panel, and adding the two lists would
  // make `N lines differ · M lines identical` sum to more lines than the longer file has. With the
  // max, `differing + identical === max(mineLines, theirsLines)` — which is exactly the identity the
  // drawn `2 lines differ · 3 lines identical` (a five-line file) asserts.
  const differing = changed.length + Math.max(onlyMine.length, onlyTheirs.length);
  return {
    changed,
    onlyMine,
    onlyTheirs,
    identical,
    /** Lines that are not the same on both sides — what `2 lines differ` counts. */
    differing,
    mineLines: mine.length,
    theirsLines: theirs.length,
    /**
     * The two files differ as bytes but not as lines — a trailing newline on one side, or a line
     * ending the other side does not use.
     *
     * It has to be said out loud because every sentence downstream would otherwise be a lie in the
     * reassuring direction: `Zero lines differ`, `otherwise the same`, a diff panel with nothing
     * highlighted. A conflict exists — the daemon wrote a sidecar — so "no difference" is the one
     * answer that is certainly wrong.
     */
    invisibleDifference: differing === 0 && mineText !== theirsText,
  };
}

/** Longest-common-subsequence ops over two small line arrays: keep / remove (mine) / insert (theirs). */
function lcsOps(mine, theirs) {
  const rows = mine.length + 1;
  const columns = theirs.length + 1;
  const table = new Uint32Array(rows * columns);
  for (let i = mine.length - 1; i >= 0; i -= 1) {
    for (let j = theirs.length - 1; j >= 0; j -= 1) {
      table[i * columns + j] =
        mine[i] === theirs[j]
          ? table[(i + 1) * columns + (j + 1)] + 1
          : Math.max(table[(i + 1) * columns + j], table[i * columns + (j + 1)]);
    }
  }

  const ops = [];
  let i = 0;
  let j = 0;
  while (i < mine.length && j < theirs.length) {
    if (mine[i] === theirs[j]) {
      ops.push({ kind: "keep", line: mine[i] });
      i += 1;
      j += 1;
    } else if (table[(i + 1) * columns + j] >= table[i * columns + (j + 1)]) {
      ops.push({ kind: "remove", line: mine[i] });
      i += 1;
    } else {
      ops.push({ kind: "insert", line: theirs[j] });
      j += 1;
    }
  }
  while (i < mine.length) {
    ops.push({ kind: "remove", line: mine[i] });
    i += 1;
  }
  while (j < theirs.length) {
    ops.push({ kind: "insert", line: theirs[j] });
    j += 1;
  }
  return ops;
}

/**
 * The facts one card's sentence is built from, or `null` when this comparison is not one the deck
 * has a sentence for.
 *
 * `side` is `"mine"` or `"theirs"`. The returned shape is deliberately not a string: the words
 * belong to `ui/copy.js`, and this module owning them would put two of the deck's sentences
 * somewhere the copy gate does not look.
 *
 * EACH CARD DESCRIBES ITS OWN SIDE, and the drawn sentences are what settle that. The left card
 * says `and is otherwise the same` on a pair where Proton's has a line yours does not — so
 * "otherwise the same" cannot mean "the files are otherwise equal". It means *this side introduces
 * nothing else*, and the other side's extra line is the other card's sentence to write. Reading it
 * the symmetric way makes both drawn sentences unreachable at the inputs their own frame draws.
 *
 * The refusals, and why each is a refusal rather than a longer sentence:
 *   · **more than one changed line** — the sentence quotes the changed content, and quoting one of
 *     four changes describes the file incorrectly rather than incompletely. `comparison.changed`
 *     is still there for a caller that would rather count than quote.
 *   · **an extra line that is not at the end** — the only extra-line clause the design draws is
 *     `an extra line at the end`, and an insertion in the middle is not that.
 *   · **a difference no line shows** — see `invisibleDifference`.
 * Every refusal lands on the documented fallback: the metadata row, alone.
 */
export function summariseSide(comparison, side) {
  if (!comparison) return null;
  if (side !== "mine" && side !== "theirs") {
    throw new Error(`side must be "mine" or "theirs", got ${JSON.stringify(side)}`);
  }
  const { changed, onlyMine, onlyTheirs, invisibleDifference } = comparison;
  if (invisibleDifference) return null;
  if (changed.length > 1) return null;

  const ours = side === "mine" ? onlyMine : onlyTheirs;
  if (ours.some((extra) => !extra.atEnd)) return null;
  if (changed.length === 0 && ours.length === 0) return null;

  // `an extra line at the end` claims this side GAINED a line, and only the line counts can confirm
  // that. LCS scores a moved line as a removal plus an insertion, so a file whose lines were merely
  // reordered produces a trailing "extra" on one side while both files have the same length — and
  // the card would report a line that was never added. Requiring the extras to equal the real gain
  // refuses that, and every other shape where the two disagree.
  const gain = Math.max(
    0,
    side === "mine"
      ? comparison.mineLines - comparison.theirsLines
      : comparison.theirsLines - comparison.mineLines,
  );
  if (ours.length !== gain) return null;

  // A changed line that is blank or only whitespace has nothing to put in the mono span, and
  // `Yours has  where Proton's has something else` is not a sentence. The headline and the
  // metadata row still stand; this one line goes.
  const quoted = changed.length === 1 ? quote(changed[0][side]) : null;
  if (changed.length === 1 && !quoted) return null;
  // Trailing whitespace is a real per-line difference and an invisible one. Quoting both sides
  // renders two cards saying `Yours has buy milk where Proton's has something else` and
  // `Proton's has buy milk where yours has something else` — the same characters, each insisting
  // the other is different. Nothing showable, so nothing said.
  if (changed.length === 1 && quoted === quote(changed[0][side === "mine" ? "theirs" : "mine"])) {
    return null;
  }

  return {
    /** The changed line as THIS side has it — the content the card quotes in inline mono. */
    quoted,
    /** How many lines this side has that the other does not, all of them at the end. */
    extraAtEnd: ours.length,
    /** True when the quoted line is the only difference in the file. */
    otherwiseSame: changed.length === 1 && ours.length === 0,
  };
}
