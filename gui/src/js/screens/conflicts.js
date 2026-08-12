// The conflicts screen (S2) — one decision, safely.
//
// `04-conflicts.md`. The v1 screen was a 236px list beside a line-numbered diff with four
// equal-weight buttons, and it got two things wrong: a list of conflicts is a list of postponed
// decisions, and a diff answers "what differs" when the question is "which do I want". So **one
// conflict fills the window**, the file sits ON the seam, and the diff is one click behind a
// disclosure.
//
// THREE THINGS THIS MODULE OWNS.
//
//   · WHICH OF THE THREE BODIES IS SHOWING. Card view, diff view, and the cleared state are one
//     screen, not three: the queue and the open conflict are the same state, and the disclosure is
//     a flag on it. `bodyOf` is the whole decision.
//   · THE QUEUE POSITION, across a rescan. Settling a conflict shortens the list, and the position
//     has to survive that without skipping the file that slid into the slot. See `advanceAfter`.
//   · WHAT A TYPE CONFLICT DOES NOT GET. A folder here and a file on Proton has no diff to show, so
//     the disclosure is not drawn at all — `Conflict.kind` is what makes that answerable (it landed
//     with this task; DEVIATIONS §74).
//
// KEEP BOTH IS THE BRIGHTEST THING ON THE SCREEN, and that is a safety property rather than a
// styling choice: it is the option that loses nothing, so it wears the maximum-contrast fill while
// both discarding options wear the decision outline. The arrow glyphs keep their SIDE colours even
// though the titles are crimson. If a future change makes a destructive button louder than this
// one, the screen has stopped doing its job.
//
// WHAT PHASE 1 CANNOT DRAW, each recorded in DEVIATIONS.md §74 with the issue that closes it:
// the cards' first line (`You added a line, 5 minutes ago` — #217, no last-agreed version exists),
// the meta line's `last agreed 3 hours ago` (same gap), the non-text side-by-side preview (no
// command serves file bytes), and the light theme's assertions (§58b, S10's extractor work).

import { el } from "../ui/el.js";
import { CONFLICTS } from "../ui/copy.js";
import { fileSize } from "../ui/format.js";
import { renderHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button } from "../ui/controls.js";
import { dot, eyebrow } from "../ui/rows.js";
import { alignedRows, compare, lines, summariseSide } from "../ui/diff.js";
import { fid } from "../fixtures/frames.js";

/** The on-seam mark in the card view, and the compressed one in the diff view. */
const HERO_SIZE = 44;
const COMPACT_SIZE = 34;
/** The settled mark on the cleared state. */
const CLEARED_SIZE = 96;

/**
 * Which body to draw.
 *
 * Cleared outranks everything because an empty queue makes every other body a window onto nothing;
 * the diff flag is only meaningful once a conflict is open, and only for a conflict that HAS a
 * diff — a type conflict cannot reach it (the disclosure is never drawn), but a stale flag left
 * over from the previous conflict could, so it is filtered here rather than trusted.
 */
export function bodyOf({ conflicts = [], index = 0, diffOpen = false } = {}) {
  if (!conflicts.length) return "cleared";
  const current = conflicts[Math.min(index, conflicts.length - 1)];
  return diffOpen && current?.kind !== "type" ? "diff" : "card";
}

/**
 * Where the queue lands after the conflict at `index` is settled and the rescan returns `next`.
 *
 * NOT `index` UNCHANGED AND NOT `index + 1`. Settling removes an item, so the file that was at
 * `index + 1` slides into `index`: keeping the index shows the next conflict (right), and
 * incrementing it SKIPS that file (wrong, and silently — it just never gets asked about). The only
 * adjustment needed is at the end of the list, where the old index is now past it.
 *
 * `Decide later` is the opposite case and does not come through here: nothing is removed, so it
 * genuinely advances by one, and wraps rather than dead-ending on the last item.
 */
export function advanceAfter(index, next) {
  if (!next.length) return 0;
  return Math.min(index, next.length - 1);
}

/** `Decide later` — skip without resolving; the item stays in the queue. */
export function skipTo(index, total) {
  return total > 0 ? (index + 1) % total : 0;
}

// ------------------------------------------------------------------------------- the pager ----

/**
 * `1 of 3` and the two arrows.
 *
 * The disabled `‹` is a per-instance `--btn-fg` override rather than a new kind: it is the only
 * disabled-looking quiet button in the app, and `.btn:disabled` deliberately only sets
 * `cursor:default` without recolouring, so the colour has to come from somewhere. A KIND for one
 * caller would be a sixth entry in a table that five screens read.
 */
function pager({ index, total, onPrev, onNext, padBottom = false }) {
  const step = (glyph, disabled, onClick) => {
    const node = button({
      kind: disabled ? "quietOutlined" : "secondaryFilled",
      size: "icon",
      label: glyph,
      fontSize: "13px",
      disabled,
      onClick: disabled ? null : onClick,
    });
    node.style.width = "30px";
    node.style.height = "30px";
    if (disabled) node.style.setProperty("--btn-fg", "var(--text-disabled)");
    return node;
  };

  const wrap = fid(
    el(
      "div",
      { class: "cf-pager" },
      fid(el("span", { class: "cf-position" }, CONFLICTS.position(index + 1, total)), "position"),
      fid(step("‹", index === 0, onPrev), "pagerPrev"),
      fid(step("›", index >= total - 1, onNext), "pagerNext"),
    ),
    "pager",
  );
  if (padBottom) wrap.style.paddingBottom = "4px";
  return wrap;
}

// -------------------------------------------------------------------------- the card view ----

/**
 * One version card: what happened, what differs, and the metadata row.
 *
 * `side` is `"mine"` or `"theirs"` — the same two words `ui/diff.js` uses, so the sentence and the
 * card can never be built for different halves of the same file.
 *
 * THE FIRST LINE IS A CONSTANT AND IT IS THE ONE PART THAT IS NOT LIVE. `You added a line` is a
 * claim against the last agreed version, whose content exists nowhere on the machine (#217). It is
 * rendered from the deck so the frame matches; a live app draws a sentence it cannot have computed,
 * which is why §74 records it rather than the screen quietly omitting it.
 */
function versionCard({ side, pair, facts }) {
  const source = side === "mine" ? pair?.original : pair?.sidecar;
  const text = source?.text ?? null;
  // COUNTED BY `lines()`, not by a second copy of its rules. This was its own `split` — the same
  // CRLF normalisation and trailing-newline drop, written out again — and the two had already
  // drifted: `"".split("\n")` is `[""]`, so an empty-but-readable file drew `1 line` on the card
  // while the diff panel beneath it drew none. Whatever the panel counts as a line IS the number the
  // card should say, so the card asks it rather than agreeing with it.
  const lineCount = text == null ? null : lines(text).length;
  // The two columns are one shape drawn twice, so the fid tables index them rather than naming
  // them — 0 is yours, 1 is Proton's, in the order the grid puts them.
  const at = side === "mine" ? 0 : 1;

  const card = fid(el("div", { class: "cf-card" }), "card", at);
  card.append(
    fid(
      el(
        "div",
        { class: "cf-card-happened" },
        side === "mine" ? CONFLICTS.mineChange : CONFLICTS.theirsChange,
      ),
      "cardHappened",
      at,
    ),
  );

  // What differs, in words — and silence rather than a hedge when the grammar does not cover it.
  // `04-conflicts.md`: fall back to the metadata row alone, and never to the raw diff, which is
  // what the disclosure is for.
  const sentence = CONFLICTS.versionDiff(side, facts);
  if (sentence) {
    const prose = proseWithQuote(sentence, facts?.quoted);
    fid(prose, "cardProse", at);
    fid(prose.querySelector(".cf-quote"), "cardQuote", at);
    card.append(prose);
  }

  const meta = fid(el("div", { class: "cf-card-meta" }), "cardMeta", at);
  // NOT `?? 0`. The pair arrives one render after the screen does, and a `0 bytes` drawn while it is
  // still loading is a claim rather than a gap — indistinguishable from a real empty file, on a card
  // whose whole purpose is telling two versions apart by their facts. `fileSize(null)` is the
  // em-dash, which format.js reserves for exactly this and nothing else.
  meta.append(el("span", {}, fileSize(source?.size)));
  if (lineCount != null) meta.append(el("span", {}, CONFLICTS.lineCount(lineCount)));
  if (source?.mtime_epoch_secs != null) {
    meta.append(el("span", {}, CONFLICTS.edited(source.mtime_epoch_secs)));
  }
  // Stamped by POSITION after the fact, because the row is built conditionally: a pair with no
  // readable text has no line count, and hard-coding `span[1]` for `edited` would then map the
  // timestamp onto the frame's line-count node.
  for (const [j, item] of [...meta.children].entries()) fid(item, "cardMetaItem", at, j);
  card.append(meta);
  return card;
}

/**
 * The prose sentence with its quoted content in inline mono.
 *
 * Split on the quote rather than built from parts, because the deck owns the whole sentence and a
 * template that emitted three pieces would put the grammar in two places. `indexOf` and not a
 * regex: the quote is file content and can contain anything a regex would read as syntax.
 */
function proseWithQuote(sentence, quoted) {
  const node = el("div", { class: "cf-card-prose" });
  const at = quoted ? sentence.indexOf(quoted) : -1;
  if (at < 0) {
    node.textContent = sentence;
    return node;
  }
  node.append(
    sentence.slice(0, at),
    el("span", { class: "cf-quote" }, quoted),
    sentence.slice(at + quoted.length),
  );
  return node;
}

function cardBody({ conflict, pair, comparison, onOpenDiff }) {
  const body = fid(el("div", { class: "cf-body" }), "body");
  body.append(fid(renderSeam({ site: "conflictBody" }), "seam"));

  // The file, on the seam. The mark does NOT declare `flex:none` here — 43 of the 53 drawn marks
  // do not, and this is one of them (the 34px mark in the diff view is the opposite).
  const onSeam = fid(el("div", { class: "cf-onseam" }), "onSeam");
  onSeam.append(
    fid(
      renderHexagon({
        size: HERO_SIZE,
        state: "needsNumeral",
        tone: "decision",
        masked: true,
        numeral: 3,
      }),
      "hexagon",
    ),
  );
  const path = fid(el("div", { class: "cf-path" }, conflict.original), "path");
  // `position: false` — unlike the main hero's headline, these two are static and their PARENT
  // carries the stacking, so masking must not turn them into positioned elements.
  seamMask(path, { pad: 14, position: false });
  const meta = fid(
    el("div", { class: "cf-onseam-meta" }, CONFLICTS.meta(fileKindOf(conflict, pair, comparison))),
    "onSeamMeta",
  );
  seamMask(meta, { pad: 14, padY: 2, position: false });
  onSeam.append(path, meta);
  body.append(onSeam);

  const cards = fid(el("div", { class: "cf-cards" }), "cards");
  for (const [at, side] of ["mine", "theirs"].entries()) {
    const cell = fid(el("div", { class: `cf-cell cf-cell-${side}` }), "cardCol", at);
    cell.append(
      fid(
        eyebrow({
          tone: side === "mine" ? "up" : "down",
          align: side === "mine" ? "start" : "end",
          text: side === "mine" ? CONFLICTS.mine : CONFLICTS.theirs,
        }),
        "cardEyebrow",
        at,
      ),
      versionCard({ side, pair, facts: summariseSide(comparison, side) }),
    );
    cards.append(cell);
  }
  body.append(cards);

  // A type conflict has no diff, so the disclosure is not drawn at all — not disabled, not empty.
  // Nor is it drawn when a side has no text to compare (binary, too large, or vanished): the
  // disclosure would open onto nothing.
  if (conflict.kind !== "type" && comparison) {
    const wrap = fid(el("div", { class: "cf-disclose" }), "disclose");
    const btn = button({
      kind: "quietOutlined",
      size: "standard",
      label: CONFLICTS.showDiff,
      padding: "8px 16px",
      onClick: onOpenDiff,
    });
    // The button's own fill IS the seam mask. As a custom property, not an inline background —
    // an inline background beats `.btn:hover { background: var(--btn-bg-hover) }` and kills hover.
    btn.style.setProperty("--btn-bg", "var(--surface)");
    wrap.append(fid(btn, "discloseBtn"));
    body.append(wrap);
  }
  return body;
}

/**
 * The three choices, and the note beneath them.
 *
 * The order is the drawn order and it is load-bearing: discard-mine, keep-both, discard-theirs puts
 * the safe option physically between the two that lose something.
 */
function choices({ conflict, onChoose, onLater }) {
  const grid = fid(el("div", { class: "cf-choices" }), "choices");
  const make = (at, resolution, kind, title, body, glyph, tone) => {
    const node = fid(
      button({
        kind,
        size: "choice",
        label: title,
        sublabel: body,
        glyph,
        glyphTone: tone,
        onClick: () => onChoose(resolution),
      }),
      "choice",
      at,
    );
    // Stamped off the built button rather than threaded through `button()`: the control owns the
    // two-tier structure and the screen owns the mapping, and handing controls.js a fid table would
    // make every other screen's buttons its business too.
    fid(node.querySelector(".btn-choice-row"), "choiceRow", at);
    fid(node.querySelector(".btn-glyph"), "choiceGlyph", at);
    fid(node.querySelector(".btn-choice-name"), "choiceName", at);
    fid(node.querySelector(".btn-choice-sub"), "choiceSub", at);
    fid(node.querySelector(".btn-choice-sub .mono"), "choiceSubMono", at);
    return node;
  };
  grid.append(
    make(0, "keep_mine", "decisionChoice", CONFLICTS.keepMine, CONFLICTS.keepMineSub, "→", "up"),
    make(
      1,
      "keep_both",
      "primaryChoice",
      CONFLICTS.keepBoth,
      monoInProse(CONFLICTS.keepBothSub(sidecarName(conflict)), sidecarName(conflict)),
      "⇄",
      "onPrimary",
    ),
    make(2, "use_proton", "decisionChoice", CONFLICTS.useTheirs, CONFLICTS.useTheirsSub, "←", "down"),
  );

  const note = fid(
    el(
      "div",
      { class: "cf-note" },
      fid(el("span", {}, CONFLICTS.cannotUndo), "noteText"),
      fid(el("span", { class: "cf-spacer" }), "noteSpacer"),
      fid(
        button({
          kind: "quietOutlined",
          size: "standard",
          label: CONFLICTS.later,
          padding: "8px 15px",
          onClick: onLater,
        }),
        "later",
      ),
    ),
    "note",
  );
  return fid(el("div", { class: "cf-choices-block" }, grid, note), "choicesBlock");
}

/**
 * Lift one substring of a sentence into mono, without the copy deck learning about markup.
 *
 * `keepBothSub` stays a flat sentence because that is what the copy gate compares — the whole
 * string, against the frame's own text. The FRAME, though, draws `todo.proton-cloud.txt` in
 * 11px IBM Plex Mono inside a 12px sans sentence, so the DOM needs a span the copy does not carry.
 * Splitting here keeps the grammar in one place; the same trade `proseWithQuote` makes above, and
 * the same `indexOf` rather than a regex, because a filename can contain anything.
 *
 * Returns an array, which `el` flattens — so a sentence whose name is missing renders as itself.
 */
function monoInProse(sentence, name) {
  const at = name ? sentence.indexOf(name) : -1;
  if (at < 0) return [sentence];
  return [sentence.slice(0, at), el("span", { class: "mono" }, name), sentence.slice(at + name.length)];
}

/** The sidecar's own file name, which `keepBothSub` quotes back to the user. */
function sidecarName(conflict) {
  const parts = String(conflict.sidecar).split("/");
  return parts[parts.length - 1];
}

/**
 * `a plain text file` / `a folder` — the type half of the meta line.
 *
 * Deliberately coarse. The pair says whether the original is readable text and `kind` says whether
 * it is a folder; nothing distinguishes an SVG from a note, because an SVG is valid UTF-8 and reads
 * exactly the same way. Naming a type we cannot tell apart would be worse than naming a category.
 */
export function fileKindOf(conflict, pair, comparison) {
  if (conflict.kind === "type") return CONFLICTS.kindFolder;
  // NOT YET KNOWN. The pair lands a render after the screen does, and `a plain text file` drawn in
  // that gap is a guess about a file nobody has opened — the same claim-without-a-fact as the
  // `0 bytes` the size row used to draw. The clause is dropped instead; `.cf-onseam-meta` holds its
  // height so the cards below do not jump when the answer arrives.
  if (!pair) return null;
  // ASKED OF THE COMPARISON, not of `original.binary_or_large`. Checking only the local side said
  // `a plain text file` whenever the SIDECAR was the unreadable one — while the disclosure beneath
  // it was hidden, because `compare()` needs both texts to be strings. The line claimed plain text
  // and the screen refused to show any: a contradiction a reader can see, on the screen where the
  // whole question is which version to keep.
  //
  // `comparison` is the right thing to ask because it is the same predicate the disclosure uses, so
  // the two cannot disagree by construction. It also covers the case `binary_or_large` cannot see —
  // a side that is missing, which `read_conflict_pair` reports as `text: null` with the flag FALSE
  // (DEVIATIONS §70b) — and the too-large-to-diff case, which `kindBinary` is already worded for:
  // "binary, too large, or vanished, which `ConflictSide` cannot tell apart".
  return comparison ? CONFLICTS.kindText : CONFLICTS.kindBinary;
}

// -------------------------------------------------------------------------- the diff view ----

/**
 * The disclosure. The seam becomes the diff's GUTTER — `1fr 1px 1fr` with the centre cell filled
 * flat — which is the one place in the app where the seam is a structural column rather than a
 * drawn line, so it is a grid cell here and not `renderSeam`.
 *
 * THE DIFF VIEW DRAWS NO CHOICE BUTTONS. `04-conflicts.md` says the disclosure "replaces the
 * version cards", and the frame replaces everything below the compressed header — the three
 * choices, the note and `Decide later` are all absent from it. The frame wins on geometry under
 * IMPLEMENTATION-PLAN §1.3, and `Hide differences` is the way back to deciding. §74.
 */
function diffBody({ pair, comparison, queue, index, onHideDiff, onOpenBoth }) {
  const rows = alignedRows(pair?.original?.text, pair?.sidecar?.text) ?? [];

  const panel = fid(el("div", { class: "cf-diff-panel" }), "diffPanel");
  const column = (side, at) => {
    const cell = fid(el("div", { class: "cf-diff-col" }), "diffCol", at);
    for (const [i, row] of rows.entries()) cell.append(diffLine(row, side, at, i));
    return cell;
  };
  // The 1px column between the halves — `split` rather than `gutter`, which in a diff means the
  // line-number channel (`.cf-diff-n`) and would name two different things one word apart.
  panel.append(
    column("mine", 0),
    fid(el("div", { class: "cf-diff-split" }), "diffSplit"),
    column("theirs", 1),
  );

  const labels = fid(
    el(
      "div",
      { class: "cf-diff-labels" },
      fid(eyebrow({ tone: "up", align: "start", text: CONFLICTS.mineShort }), "diffLabel", 0),
      fid(eyebrow({ tone: "down", align: "end", text: CONFLICTS.theirsShort }), "diffLabel", 1),
    ),
    "diffLabels",
  );

  const counts = fid(
    el(
      "div",
      { class: "cf-diff-counts" },
      fid(
        el("span", {}, CONFLICTS.diffCounts(comparison?.differing ?? 0, comparison?.identical ?? 0) ?? ""),
        "diffCountsText",
      ),
      fid(el("span", { class: "cf-spacer" }), "diffCountsSpacer"),
      fid(
        button({ kind: "quietOutlined", size: "small", label: CONFLICTS.openBoth, onClick: onOpenBoth }),
        "openBoth",
      ),
      fid(
        button({ kind: "quietOutlined", size: "small", label: CONFLICTS.hideDiff, onClick: onHideDiff }),
        "hideDiff",
      ),
    ),
    "diffCounts",
  );

  const content = fid(el("div", { class: "cf-diff-content" }, labels, panel, counts), "body");
  const remaining = queueList(queue, index);
  if (remaining) content.append(remaining);
  return content;
}

/**
 * One side of one row.
 *
 * THE ABSENT ROW IS NOT SYMMETRIC. The side that HAS the line is drawn exactly like a changed line
 * — tinted, numbered in its side colour — and only the empty side gets the placeholder. Treating
 * `absent` as a third visual kind on both sides would leave the gained line looking unchanged,
 * which is the one thing the row exists to point at.
 */
function diffLine(row, side, at, i) {
  const cell = row[side];
  const line = fid(el("div", { class: "cf-diff-line" }), "diffLine", at, i);
  if (!cell) {
    line.append(
      fid(el("span", { class: "cf-diff-n cf-diff-n-absent" }, "·"), "diffN", at, i),
      fid(
        el("span", { class: "cf-diff-text cf-diff-text-absent" }, CONFLICTS.absentLine(side)),
        "diffText",
        at,
        i,
      ),
    );
    return line;
  }
  const tinted = row.kind !== "unchanged";
  if (tinted) line.classList.add(side === "mine" ? "is-changed-mine" : "is-changed-theirs");
  line.append(
    fid(el("span", { class: "cf-diff-n" }, String(cell.n)), "diffN", at, i),
    fid(el("span", { class: "cf-diff-text" }, cell.text), "diffText", at, i),
  );
  return line;
}

/** `Still waiting after this one` — the rest of the queue, as a list rather than a sidebar. */
function queueList(queue, index) {
  const rest = queue.filter((_, i) => i !== index);
  if (!rest.length) return null;
  const rows = fid(el("div", { class: "cf-queue-rows" }), "queueRows");
  // `i` is the position IN THIS LIST (what the fid tables index) and `at` the position in the whole
  // queue (what `2 of 3` counts). They differ by one from the open conflict onward, and swapping
  // them maps every row after the current one onto its neighbour's node.
  //
  // Derived rather than looked up. `queue.indexOf(item)` says the same thing and says it in O(n) per
  // row, but the arithmetic is also the more honest form: it states the relationship this comment
  // describes instead of re-deriving it from object identity, which only holds while `rest` is a
  // filter of `queue` and would go quietly wrong the day it is rebuilt from a rescan.
  for (const [i, item] of rest.entries()) {
    const at = i < index ? i : i + 1;
    rows.append(
      fid(
        el(
          "div",
          { class: "cf-queue-row" },
          fid(dot({ tone: "decision", size: 6 }), "queueDot", i),
          fid(el("span", { class: "cf-queue-path" }, item.original), "queuePath", i),
          fid(
            el(
              "span",
              { class: "cf-queue-reason" },
              item.kind === "type" ? CONFLICTS.typeConflict : CONFLICTS.bothChanged,
            ),
            "queueReason",
            i,
          ),
          fid(el("span", { class: "cf-queue-pos" }, CONFLICTS.position(at + 1, queue.length)), "queuePos", i),
        ),
        "queueRow",
        i,
      ),
    );
  }
  return fid(
    el(
      "div",
      { class: "cf-queue" },
      fid(eyebrow({ tone: "neutral", text: CONFLICTS.stillWaiting }), "queueEyebrow"),
      rows,
    ),
    "queue",
  );
}

// ----------------------------------------------------------------------- the cleared state ----

/**
 * Nothing left to decide.
 *
 * DRAWN 522px WIDE, AND THE APP CANNOT BE. The Tauri window is a fixed, non-resizable 1040×764 and
 * `conflicts` is routed as a full-window screen, so this renders as a centred 522 column inside the
 * window rather than as a narrow window — the closest thing to the drawing that the shell can
 * produce. No gate can tell: the frame's root box is un-comparable (its `⋯` characters are outside
 * the bundled unicode ranges, which taints the root and every ancestor chain through it). §74.
 */
function clearedBody({ settled, onBack }) {
  // FLAT, with no inner wrapper: the frame's body is one block and the mapping is positional, so a
  // centring shell around a column would be a node with no key beneath it. conflicts.css does the
  // centring with `margin: 0 auto` for exactly that reason.
  const body = fid(el("div", { class: "cf-cleared" }), "cleared");
  const mark = fid(renderHexagon({ size: CLEARED_SIZE, state: "settled" }), "hexagon");
  // The mark's own two paths, the way S1 and S3 stamp theirs. The `<svg>` alone leaves the ring and
  // the tick — the geometry, the stroke and the fill — uncompared (#248).
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
  body.append(
    mark,
    fid(el("div", { class: "cf-cleared-title" }, CONFLICTS.clearedTitle), "clearedTitle"),
    fid(el("div", { class: "cf-cleared-sub" }, CONFLICTS.clearedSub(settled)), "clearedSub"),
    fid(
      button({
        kind: "secondaryOutlined",
        size: "bar",
        label: CONFLICTS.back,
        padding: "10px 20px",
        onClick: onBack,
      }),
      "clearedBack",
    ),
  );
  return body;
}

// ------------------------------------------------------------------------------ the screen ----

/**
 * Render the conflicts screen as an ARRAY of window-root siblings — never a wrapper.
 *
 * `shell.css` gives the window `display:flex; flex-direction:column`, and the body block is the
 * `flex:1` child of the window itself. A wrapper would make the seam's `left:50%` resolve against
 * the wrapper rather than the 1040px window, which is exactly the geometry the seam encodes.
 */
export function renderConflicts(state) {
  const {
    conflicts = [],
    index = 0,
    diffOpen = false,
    pair = null,
    settled = null,
    onChoose = () => {},
    onLater = () => {},
    onOpenDiff = () => {},
    onHideDiff = () => {},
    onOpenBoth = () => {},
    onBack = () => {},
    onPrev = () => {},
    onNext = () => {},
  } = state;

  const body = bodyOf({ conflicts, index, diffOpen });
  if (body === "cleared") return [clearedBody({ settled, onBack })];

  const at = Math.min(index, conflicts.length - 1);
  const conflict = conflicts[at];
  const comparison = compare(pair?.original?.text, pair?.sidecar?.text);

  if (body === "diff") {
    const head = fid(
      el(
        "div",
        { class: "cf-diff-head" },
        fid(
          renderHexagon({
            size: COMPACT_SIZE,
            state: "needsNumeral",
            tone: "decision",
            masked: true,
            numeral: conflicts.length,
            flexNone: true,
          }),
          "hexagon",
        ),
        fid(
          el(
            "div",
            { class: "cf-diff-headtext" },
            fid(el("div", { class: "cf-path-plain" }, conflict.original), "pathPlain"),
            fid(
              el(
                "div",
                { class: "cf-diff-summary" },
                CONFLICTS.diffSummary(comparison?.differing ?? 0) ?? "",
              ),
              "diffSummary",
            ),
          ),
          "diffHeadText",
        ),
        pager({ index: at, total: conflicts.length, onPrev, onNext }),
      ),
      "diffHead",
    );
    return [head, diffBody({ pair, comparison, queue: conflicts, index: at, onHideDiff, onOpenBoth })];
  }

  const titleRow = fid(
    el(
      "div",
      { class: "cf-title-row" },
      fid(
        el(
          "div",
          { class: "cf-title-text" },
          fid(el("div", { class: "cf-title" }, CONFLICTS.title), "title"),
          fid(el("div", { class: "cf-sub" }, CONFLICTS.sub), "sub"),
        ),
        "titleText",
      ),
      pager({ index: at, total: conflicts.length, onPrev, onNext, padBottom: true }),
    ),
    "titleRow",
  );

  return [
    titleRow,
    cardBody({ conflict, pair, comparison, onOpenDiff }),
    choices({ conflict, onChoose, onLater }),
  ];
}
