// The plan screen (S4) — the rehearsal.
//
// `06-plan.md`. The v1 screen opened with TEN identical stat tiles — `total`, `uploads`,
// `downloads`, `remote_directories_created`, `local_directories_created`, `local_moves`,
// `remote_moves`, `auto_links`, `conflicts`, `type_conflicts` — seven of which read zero, over a raw
// table with `action` / `path` / `entity` / `remote_id` columns. It answered *what are the counts*
// when the only question anybody opens this screen with is *is it safe to run*.
//
// So: a sentence, a verdict, and NOTHING THAT READS ZERO GETS A TILE. An absent category is absent,
// which is why a simple plan produces a short screen and a dangerous one produces a tall one.
//
// SIX THINGS THIS MODULE OWNS.
//
//   · WHICH OF THE FOUR BODIES IS SHOWING. `bodyOf` — and there are four, not the three the frames
//     draw: `14-behaviour-and-state.md`'s error table specifies a failed rehearsal ("show the daemon
//     string, offer `Check again`") that no frame draws, and without it a machine with no
//     `proton-syncd` on its PATH renders a blank window.
//   · WHICH SIDE OF THE SEAM AN ACTION IS ON, and whether it has one at all. `sideOf`, whose `null`
//     answer is what decides between the two plan bodies — see `bodyOf`.
//   · THE ORDER OF THE LIST, which is `plan.rs::sorted_for_display` and is deliberately a second
//     copy of it. See `sortedForDisplay`.
//   · THE TWO DESTRUCTIVE SETS, WHICH ARE NOT THE SAME SET. `plan.rs` encodes the distinction the
//     design conflated: a `purge` is display-destructive (tinted, sorted first) and never gated,
//     because it destroys no user data — it forgets an index row for a file already gone from both
//     sides. The band and the typed word key on the GATED set; the tint keys on the display set.
//   · THAT THE GATE IS ONE PREDICATE. `gateSatisfied` is asked by the field's listener and again by
//     the click, so the button's paint and the run decision cannot disagree — the rule S3's card
//     gate is built on.
//   · WHICH FOOTER THE WINDOW HAS. This is the one screen in the design whose footer changes with
//     its own state: `5a Plan` and `5a Plan safe` draw an action bar, `5a Checking` draws the four
//     doors. `footerKindOf` is the shell's question and this module's answer. DEVIATIONS §76.
//
// WHAT PHASE 1 CANNOT DRAW, each recorded in DEVIATIONS.md §76 with the issue that closes it:
// every byte total (`files, 4.1 MB`, and the per-file sizes on the safe screen — G2 #191; no
// dry-run field carries a size), the checking screen's `8,431 of 12,480 files` (G9 #209 has no
// progress channel, G7 #207 no index-wide count), `Run it without the deletion` (G3 #192, which
// `06-plan.md` says to hide rather than fake), and `Leave it alone`, which is the same filtered
// apply reached from the band.

import { el } from "../ui/el.js";
import { PLAN } from "../ui/copy.js";
import { count, outcomeOf, since } from "../ui/format.js";
import { renderHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button, deleteGate, gateGroup, setButtonKind } from "../ui/controls.js";
import { eyebrow, planActionRow, planSideRow } from "../ui/rows.js";
import { noticeBand } from "../ui/bands.js";
import { renderActionBar } from "../ui/chrome.js";
import { fid } from "../fixtures/frames.js";

/** The word, in the one place the field and the run decision both read it from. */
export const GATE_WORD = "DELETE";

/** The marks, at the three sizes this screen draws them (`01-foundations.md` §6, `strokeForSize`). */
const BAND_MARK = 34;
const SAFE_MARK = 88;
const CHECKING_MARK = 104;

/**
 * How many rows the action list shows before it has to scroll.
 *
 * MEASURED off the frame rather than chosen: `5a Plan` draws nine 33px rows in a 319px block, so a
 * tenth is the first to fall off the bottom. At or under that count the list keeps the frame's
 * `overflow:hidden` — an asserted property — and over it the list scrolls rather than pushing the
 * footer out of a window that cannot grow. Same shape as S3's `is-scrollable` columns.
 */
const ROWS_THAT_FIT = 9;

// ------------------------------------------------------------------------------ the model ----

/**
 * Which side of the seam an action is on, or `null` for one that is on neither.
 *
 * `null` IS THE INTERESTING ANSWER. A deletion, a conflict, an adoption, a purge and a type clash
 * are not files crossing the seam — they are things happening to a file already on one side or on
 * both — so the safe screen, whose entire content is two lists of files moving, has nowhere to put
 * them. That is what `bodyOf` keys on, and it is why an unrecognised action answers `null` too: a
 * variant this table has never heard of must not be quietly dropped off the screen whose job is
 * telling you everything that would happen.
 *
 * The two moves go by the side the change LANDS on: `move_local` applies a rename that happened on
 * Proton, so it arrives; `move_remote` applies one you made here, so it leaves.
 */
export function sideOf(action) {
  switch (action) {
    case "upload":
    case "create_remote_directory":
    case "move_remote":
      return "leaving";
    case "download":
    case "create_local_directory":
    case "move_local":
      return "arriving";
    default:
      return null;
  }
}

/** Tinted and sorted first — `plan.rs::is_display_destructive`, `purge` included. */
export function isDisplayDestructive(action) {
  return action === "remote_delete" || action === "local_delete" || action === "purge";
}

/**
 * Behind the typed word — `SyncAction::delete_direction().is_some()`, i.e. the rows that remove real
 * user data. A `purge` is NEVER here: it clears an index record for something already gone from both
 * sides, and making it arm the gate would teach people to type the word for nothing.
 */
export function isGated(action) {
  return action === "remote_delete" || action === "local_delete";
}

/**
 * The plan in drawn order: display-destructive rows first, otherwise the daemon's own order.
 *
 * A SECOND COPY OF `plan.rs::sorted_for_display`, ON PURPOSE. That function lives in gui-core and
 * cannot be reached from here — `run_dry_run` returns the parsed `DryRunReport` verbatim, so what
 * arrives over the wire is the daemon's emission order with nothing sorted. Reusing the Rust one
 * would mean changing what the command returns, which changes a payload three other things parse.
 * So the ordering lives in both, and this comment is the pointer between them: change one, change
 * the other. `Array#sort` is stable everywhere this runs, which is what "otherwise the daemon's
 * order" rests on.
 */
export function sortedForDisplay(plan = []) {
  const first = (row) => (isDisplayDestructive(row.action) ? 0 : 1);
  return [...plan].sort((a, b) => first(a) - first(b));
}

/**
 * Which body to draw.
 *
 * THE SAFE SCREEN IS NOT "NO DELETIONS", it is "every action is a file crossing the seam", and the
 * difference is what stops the screen lying by omission. `5a Plan safe` draws its two sides as lists
 * of files and has nowhere else to put a row — so a plan holding a conflict, an adoption or a purge
 * would render as a screen that silently forgot it. Those get the list form, which has a row for
 * everything; the destructive band appears within it only when something is actually gated. Both
 * drawn frames come out exactly as drawn: nine actions with a delete → the list, seven files
 * moving → the hero.
 *
 * `checking` outranks a payload, the way S3's `empty` outranks `armed`: a re-check is in flight, so
 * the plan on screen is the OLD one, and drawing it under a live `Run this sync` would offer to run
 * something the app has already been told to stop believing.
 */
export function bodyOf({ dryRun = null, checking = false, error = null } = {}) {
  if (checking) return "checking";
  if (error) return "failed";
  if (!dryRun) return "checking";
  const plan = dryRun.report?.plan ?? [];
  return plan.every((row) => sideOf(row.action)) ? "safe" : "plan";
}

/**
 * Which footer the window wears. The one screen in the design where this is a per-STATE question:
 * both 1040 plan frames draw a footer action bar and `5a Checking` draws the four doors.
 *
 * The failed body takes the bar, because `Check again` has to live somewhere and the doors are not
 * somewhere — `14-behaviour-and-state.md` says the error state offers it.
 */
export function footerKindOf(state = {}) {
  return bodyOf(state) === "checking" ? "doors" : "actionBar";
}

/** The gate's one predicate. Case-sensitive: `delete` is a word people type by habit. */
export function gateSatisfied(value) {
  return value === GATE_WORD;
}

/**
 * Everything the screen counts, counted once.
 *
 * THE SIDE COUNTS ARE FILES AND NOT ACTIONS. `5a Plan` draws `3` over `files, 4.1 MB` for a plan
 * whose leaving side is three uploads AND a new folder — the folder is the sentence underneath
 * ("Plus one new folder created on Proton Drive to hold them."), not a fourth file. `5a Plan safe`
 * makes the same distinction twice over: seven actions, `Five files move`.
 */
export function summarise(plan = []) {
  const rows = sortedForDisplay(plan);
  const countOf = (...actions) => rows.filter((row) => actions.includes(row.action)).length;
  return {
    rows,
    total: rows.length,
    uploads: countOf("upload"),
    downloads: countOf("download"),
    newFolders: countOf("create_remote_directory"),
    renames: countOf("move_local"),
    conflicts: countOf("conflict"),
    gated: rows.filter((row) => isGated(row.action)),
    leaving: rows.filter((row) => sideOf(row.action) === "leaving"),
    arriving: rows.filter((row) => sideOf(row.action) === "arriving"),
  };
}

/**
 * The mark for one planned action, and its tone.
 *
 * `glyph: null` means the row draws a RING instead of a character — the conflict, the one row that
 * is not a direction: nothing moves and nothing is lost, both copies are kept. `＋` is the FULLWIDTH
 * plus (U+FF0B), which is what the design draws and is nowhere near `+` in Unicode; the fidelity
 * harness exempts its width because no bundled face covers it.
 */
export function markOf(action) {
  switch (action) {
    case "upload":
      return { glyph: "→", tone: "up" };
    case "download":
      return { glyph: "←", tone: "down" };
    case "create_remote_directory":
      return { glyph: "＋", tone: "up" };
    case "create_local_directory":
      return { glyph: "＋", tone: "down" };
    case "move_local":
    case "move_remote":
      return { glyph: "↷", tone: "quiet" };
    case "remote_delete":
    case "local_delete":
      return { glyph: "✕", tone: "destructive" };
    // Tinted like a deletion because it is sorted with them, but NOT crimson: a purge takes nothing
    // away from you, and a red ✕ beside a path is the app saying it does.
    case "purge":
      return { glyph: "✕", tone: "quiet" };
    case "conflict":
      return { glyph: null, tone: "quiet" };
    default:
      // Not a guess and not a blank: an action nobody has drawn still gets a row, its path and its
      // outcome, with a mark that says "something happens here" rather than one claiming a direction.
      return { glyph: "·", tone: "quiet" };
  }
}

/**
 * What a row's path reads as. A move draws BOTH ends in one row
 * (`notes/old.md → notes/archive/old.md`), which is the frame's own form and the only place the plan
 * list is not one path per row.
 */
export function pathOf(row) {
  return row.destination_path ? `${row.path} → ${row.destination_path}` : row.path;
}

// ---------------------------------------------------------------------------- the sections ----

/** `The next sync moves 9 things` + the rehearsal sentence, and the quiet `Check again`. */
function titleRow(model, handlers) {
  const text = fid(
    el(
      "div",
      { class: "pl-title-text" },
      fid(el("div", { class: "pl-title" }, PLAN.title(model.total)), "title"),
      fid(el("div", { class: "pl-sub" }, PLAN.sub(model.gated.length)), "sub"),
    ),
    "titleText",
  );
  return fid(
    el(
      "div",
      { class: "pl-title-row" },
      text,
      fid(checkAgain({ handlers, size: "standard", padding: "9px 16px" }), "checkAgain"),
    ),
    "titleRow",
  );
}

/** The quiet re-check, at whichever of its two drawn sizes the caller sits at. */
function checkAgain({ handlers, size, padding, fontSize = "12.5px", kind = "secondary" }) {
  return button({
    kind,
    size,
    label: PLAN.checkAgain,
    padding,
    radius: size === "bar" ? "var(--r-10)" : "var(--r-9)",
    fontSize,
    onClick: () => handlers.onCheck?.(),
  });
}

/**
 * The seam block: two side counts, and under each of them whatever this body puts there.
 *
 * ONE BUILDER FOR BOTH FRAMES, because they are the same block with a different third child —
 * `5a Plan` puts a sentence under each count and `5a Plan safe` puts the files themselves. Written
 * as two builders they drift, and the thing that must not drift is the geometry either side of the
 * seam: a `1fr 1fr` grid with a 30px gutter each side of the centre line, the right column
 * right-aligned in its entirety. That is what makes the seam read as a divide rather than as a rule
 * between two tables.
 */
function seamBlock({ model, site, detail, aligned }) {
  // `aligned` IS MEASURED AND NOT INHERITED FROM THE IDEA. `5a Plan` right-aligns the whole arriving
  // column, because everything in it is text; `5a Plan safe` right-aligns only its eyebrow and
  // pushes the count over with `justify-content`, because the third child is a list of ROWS and a
  // right-aligned column would drag every path in it away from the glyph beside it. Two frames, two
  // answers, and the gate reads `text-align` on the column itself — so this is a property of the
  // block rather than of the seam.
  const block = fid(el("div", { class: `pl-seam-block${aligned ? " is-aligned" : ""}` }), "seamBlock");
  block.append(fid(renderSeam({ site }), "seam"));
  const sides = fid(el("div", { class: "pl-sides" }), "sides");
  for (const [s, side] of ["leaving", "arriving"].entries()) {
    const leaving = side === "leaving";
    const tail = detail(side, s);
    // NOTHING THAT READS ZERO GETS A TILE — `06-plan.md`'s own emphasis, and a side with no files
    // and nothing to say under it is exactly that. Both drawn frames have traffic in both
    // directions, so nothing settles this by drawing it; the rule does. Most real plans are
    // one-directional, so this is the common case rather than an edge.
    if (!(leaving ? model.uploads : model.downloads) && !tail) continue;
    const column = fid(el("div", { class: `pl-side${leaving ? "" : " is-arriving"}` }), "side", s);
    // PLACED RATHER THAN FLOWED, the same guard S3's queue columns carry. A grid child with no
    // sibling falls into the FIRST cell, so a plan with nothing leaving would draw `Arriving from
    // Proton` on this computer's side of the seam — the one arrangement that makes the screen lie.
    column.style.gridColumn = String(s + 1);
    column.append(
      fid(
        eyebrow({
          tone: leaving ? "up" : "down",
          text: leaving ? PLAN.leaving : PLAN.arriving,
          align: leaving ? "start" : "end",
        }),
        "sideLabel",
        s,
      ),
    );
    const files = leaving ? model.uploads : model.downloads;
    column.append(
      fid(
        el(
          "div",
          { class: "pl-count" },
          // Rendered from a number, never a literal — and the 42px numeral is its own node because
          // the design sets it at its own size beside a 14px unit.
          fid(el("span", { class: "pl-numeral" }, count(files)), "sideNumeral", s),
          // `null` is the whole of G2 (#191): no dry-run field carries a byte total, so `, 4.1 MB`
          // is omitted rather than filled with a plausible number. DEVIATIONS §76.
          fid(el("span", { class: "pl-unit" }, PLAN.sideUnit(files, null)), "sideUnit", s),
        ),
        "sideCount",
        s,
      ),
    );
    if (tail) column.append(tail);
    sides.append(column);
  }
  block.append(sides);
  return block;
}

/** `5a Plan`'s third child per side: the sentence about what is NOT a file. */
function sideNote(model) {
  return (side, s) => {
    const n = side === "leaving" ? model.newFolders : model.renames;
    if (!n) return null;
    const text = side === "leaving" ? PLAN.plusFolder(n) : PLAN.plusRename(n);
    return fid(el("div", { class: "pl-side-note" }, text), "sideNote", s);
  };
}

/** `5a Plan safe`'s third child per side: the files themselves, one row each. */
function sideList(model) {
  return (side, s) => {
    const rows = side === "leaving" ? model.leaving : model.arriving;
    if (!rows.length) return null;
    const list = fid(el("div", { class: "pl-side-list" }), "sideList", s);
    for (const [i, row] of rows.entries()) {
      const { glyph, tone } = markOf(row.action);
      // A SIZE OR A WORD, drawn differently on purpose (see `planSideRow`) — and `noteFor` answering
      // null IS the size case: a file row's note is its size, which Phase 1 cannot report (#191), so
      // it draws no note at all rather than an em-dash claiming the daemon was asked and did not
      // know. It keeps the file row's brighter path either way, because what the colour tracks is
      // what the ROW is, not whether this build could fill its last slot.
      const note = noteFor(row.action);
      const node = fid(
        planSideRow({ glyph, tone, path: pathOf(row), note, noteIsSize: note === null }),
        "sideRow",
        s,
        i,
      );
      fid(node.children[0], "sideRowGlyph", s, i);
      fid(node.children[1], "sideRowPath", s, i);
      fid(node.children[2], "sideRowNote", s, i);
      list.append(node);
    }
    return list;
  };
}

/**
 * What the band's title should call the things it is about to lose.
 *
 * A whole-subtree deletion is a real planned action (`plan_sync` emits `LocalDelete`/`RemoteDelete`
 * with `EntityKind::Directory` when a directory went cleanly on one side), and it is the largest
 * loss this screen can describe — so the noun has to agree with it. A set holding both gets
 * `thing`, because either noun would be wrong about half of it.
 */
export function gatedKind(gated = []) {
  const kinds = new Set(gated.map((row) => (row.entity_kind === "directory" ? "folder" : "file")));
  return kinds.size === 1 ? [...kinds][0] : "thing";
}

/** `new folder` / `moved`, or nothing at all for a file whose size Phase 1 cannot report. */
function noteFor(action) {
  if (action === "create_remote_directory" || action === "create_local_directory") return PLAN.newFolder;
  if (action === "move_local" || action === "move_remote") return PLAN.moved;
  return null;
}

/**
 * The destructive band — the dangerous thing breaking out of the seam.
 *
 * IT IS NEVER JUST A ROW IN A LIST, which is `06-plan.md`'s own emphasis and the reason this block
 * exists at all: the tinted row below says the same words, and a row is something you scroll past.
 *
 * `Leave it alone` IS NOT DRAWN, by the same rule the footer's second button follows. Read either
 * way — drop this one action and run the rest, or refuse this deletion durably — it needs a
 * capability Phase 1 does not have (G3 #192, or #224's durable refusal), and `06-plan.md` says to
 * hide the button rather than fake it. A drawn button that quietly does nothing would be worst of
 * all right here: it is the escape hatch on the one screen where somebody is looking for one.
 */
function destructiveBand(model) {
  const gated = model.gated;
  const one = gated.length === 1 ? gated[0] : null;
  const kind = gatedKind(gated);
  const mark = fid(
    renderHexagon({ size: BAND_MARK, state: "warning", tone: "destructive", flexNone: true }),
    "bandMark",
  );
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "bandMarkPath", i);
  fid(mark.querySelector("circle"), "bandMarkDot");

  const band = fid(
    noticeBand({
      tone: "destructive",
      mark,
      title: PLAN.destructiveTitle(gated.length, kind),
      note: one ? consequence(one) : [PLAN.destructiveMany(gated.length, kind)],
    }),
    "band",
  );
  fid(band.querySelector(".band-notice-body"), "bandBody");
  fid(band.querySelector(".band-notice-title"), "bandTitle");
  fid(band.querySelector(".band-notice-note"), "bandNote");
  fid(band.querySelector(".band-notice-note .mono"), "bandNotePath");
  return fid(el("div", { class: "pl-band-wrap" }, band), "bandWrap");
}

/**
 * One deletion's sentence, as the three pieces the band's note holds directly: the prose before the
 * path, the path in mono, and the prose after. An array rather than a wrapper span, because the
 * frame draws exactly one element inside that note and a wrapper would be a second.
 *
 * SPLIT ON WHERE THE TEMPLATE PUTS THE PATH, never on where the path first appears in the finished
 * string. The same guard S3's `armedSentence` carries and for the same reason: `indexOf(path)` finds
 * the first textual match, and a file called `is` matches inside the prose before its own slot.
 * Rendering the template around a marker asks the deck where its own hole is. `U+0001` cannot occur
 * in a path — and is written as an escape, because a literal control character in a source file is
 * its own bug (tools/check-sources.mjs).
 */
function consequence(row) {
  const template = row.action === "local_delete" ? PLAN.destructiveLocal : PLAN.destructiveRemote;
  const inside = row.entity_kind === "directory";
  const sentence = template(row.path, inside);
  const MARKER = "\u0001";
  const at = template(MARKER, inside).indexOf(MARKER);
  if (at < 0) return [sentence];
  return [
    sentence.slice(0, at),
    el("span", { class: "mono" }, row.path),
    sentence.slice(at + row.path.length),
  ];
}

/** `Every action, in order` · the mono tally · every row. */
function actionList(model) {
  const head = fid(
    el(
      "div",
      { class: "pl-list-head" },
      fid(eyebrow({ tone: "neutral", text: PLAN.everyAction }), "listLabel"),
      fid(el("span", { class: "shell-spacer" }), "listSpacer"),
      fid(
        el("span", { class: "pl-list-count" }, PLAN.actionSummary(model.total, model.conflicts)),
        "listCount",
      ),
    ),
    "listHead",
  );
  const rows = fid(
    el("div", { class: "pl-rows" + (model.rows.length > ROWS_THAT_FIT ? " is-scrollable" : "") }),
    "rows",
  );
  for (const [i, row] of model.rows.entries()) {
    const { glyph, tone } = markOf(row.action);
    const node = fid(
      planActionRow({
        glyph,
        tone,
        path: pathOf(row),
        outcome: outcomeOf(row.action, "plan"),
        // THE TWO SETS AGAIN, one flag each. The tint groups the rows `sorted_for_display` floats to
        // the top; the emphasis marks the ones that take something away. A `purge` is in the first
        // set and not the second, which is the whole reason `plan.rs` keeps them apart.
        tinted: isDisplayDestructive(row.action),
        destructive: isGated(row.action),
      }),
      "row",
      i,
    );
    fid(node.children[0], "rowGlyph", i);
    fid(node.children[1], "rowPath", i);
    fid(node.children[2], "rowOutcome", i);
    rows.append(node);
  }
  return fid(el("div", { class: "pl-list" }, head, rows), "list");
}

// -------------------------------------------------------------------------------- the body ----

/** State A — a plan that would destroy something, or one the two side lists cannot hold. */
function planBody(model, handlers) {
  const blocks = [
    titleRow(model, handlers),
    seamBlock({ model, site: "planTotals", detail: sideNote(model), aligned: true }),
  ];
  if (model.gated.length) blocks.push(destructiveBand(model));
  blocks.push(actionList(model));
  return blocks;
}

/**
 * State B — the ordinary safe plan. The screen shrinks to a hero and two short lists.
 *
 * A PLAN WITH NOTHING IN IT IS THIS BODY TOO, which is `14-behaviour-and-state.md`'s own routing
 * ("Plan · Empty: safe-plan variant") and is the likeliest thing anybody sees: you click `Plan a
 * sync` on a folder that is already in sync. No frame draws it, and the safe screen left unchanged
 * says `Nothing gets deleted` over `zero files move` above two columns of `0` — three ways of
 * saying nothing happens, none of them the sentence a person came for. So the hero says what is
 * true and the seam block goes entirely: there are no sides when nothing crosses.
 */
function safeBody(model) {
  const empty = model.total === 0;
  const hero = fid(el("div", { class: "pl-hero" }), "hero");
  // `heroSeam`, NOT `seam`. `5a Plan safe` draws TWO seams — the hero's and the list block's, a
  // continuation pair overlapping by 40px so the joint is invisible — and a slot name stamps a key,
  // so sharing one would give two nodes the same `data-fid` and compare both against one drawn line.
  hero.append(fid(renderSeam({ site: "planSafeHero" }), "heroSeam"));
  const mark = fid(
    renderHexagon({ size: SAFE_MARK, state: "settled", masked: true, class: "pl-hero-mark" }),
    "heroMark",
  );
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "heroMarkPath", i);
  const title = fid(
    el("div", { class: "pl-hero-title" }, empty ? PLAN.nothingTitle : PLAN.safeTitle),
    "heroTitle",
  );
  const sub = fid(
    el(
      "div",
      { class: "pl-hero-sub" },
      empty ? PLAN.nothingSub : PLAN.safeSub(model.uploads + model.downloads),
    ),
    "heroSub",
  );
  // Masked, both of them: the seam runs 40px past the bottom of this block and straight through the
  // two lines. The pads are the frame's own — 18px on the 28px headline, 18px and 2px on the 13.5px
  // sentence — and `position` is what actually does the hiding (F3 rule 3: an absolute seam paints
  // above a static sibling's text however late in the DOM that sibling sits).
  seamMask(title, { pad: 18 });
  seamMask(sub, { pad: 18, padY: 2 });
  hero.append(mark, title, sub);
  return [
    hero,
    empty ? null : seamBlock({ model, site: "planSafeList", detail: sideList(model), aligned: false }),
    // The empty flex:1 block the frame draws between the lists and the footer. A real node rather
    // than a margin: the footer is a child of the WINDOW, so something has to take up the slack.
    fid(el("div", { class: "pl-spacer" }), "tailSpacer"),
  ].filter(Boolean);
}

/**
 * State C — working it out.
 *
 * 520px wide where the shell is a fixed, non-resizable 1040, which is `3a Conflicts cleared`'s and
 * `4a Empty`'s situation and gets their answer: a centred 520 column, the closest the window can
 * get, with the difference recorded rather than faked (#221, §76).
 *
 * NO NUMERAL. The mark is reading, not moving — `dryRun` is F2's own flag for it and carries the
 * thinner, faster dash the frame draws (`40 260` at 2.4s/3.2s against the syncing mark's `62 238` at
 * 3.2s/4.4s).
 */
function checkingBody(handlers) {
  const body = fid(el("div", { class: "pl-checking" }), "checking");
  body.append(fid(renderSeam({ site: "checkingDialog" }), "checkingSeam"));
  const mark = fid(
    renderHexagon({
      size: CHECKING_MARK,
      state: "syncing",
      dryRun: true,
      masked: true,
      class: "pl-checking-mark",
    }),
    "checkingMark",
  );
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "checkingMarkPath", i);
  fid(mark.querySelector("defs"), "checkingMarkDefs");
  for (const [i, node] of [...mark.querySelectorAll("linearGradient")].entries()) {
    fid(node, "checkingMarkGradient", i);
    for (const [j, stop] of [...node.querySelectorAll("stop")].entries()) {
      fid(stop, "checkingMarkStop", i, j);
    }
  }
  const title = fid(el("div", { class: "pl-checking-title" }, PLAN.checkingTitle), "checkingTitle");
  const sub = fid(el("div", { class: "pl-checking-sub" }, PLAN.checkingSub), "checkingSub");
  seamMask(title, { pad: 16 });
  seamMask(sub, { pad: 14, padY: 2 });
  // `8,431 of 12,480 files` IS NOT DRAWN. Its two halves are two different missing capabilities that
  // happen to meet in one sentence — `run_dry_run` is a single command with no progress channel
  // (G9 #209) and nothing reports an index-wide file count (G7 #207) — so the line is omitted whole.
  // Half of it is a fraction with no denominator. DEVIATIONS §76.
  const stop = fid(
    button({
      kind: "secondary",
      size: "standard",
      label: PLAN.stop,
      padding: "9px 18px",
      radius: "var(--r-9)",
      fontSize: "12.5px",
      class: "pl-stop",
      onClick: () => handlers.onStop?.(),
    }),
    "stop",
  );
  // THE ONE BUTTON IN THE APP THAT MASKS WITH ITS OWN FILL. `06-plan.md` is explicit — "`Stop` is
  // `background:#0A0B0D`, not transparent, so the seam passes behind it" — and the frame draws the
  // fill without the `position` that makes it work. Positioned here, because an unpositioned mask is
  // exactly the `1a Compact` bug F3 records: the seam paints over the fill and the app ships a
  // hairline through the word. The `position` difference is recorded in §76.
  seamMask(stop, { pad: null });
  body.append(mark, title, sub, stop);
  return [body];
}

/**
 * The fourth body, which no frame draws: the rehearsal could not run.
 *
 * `14-behaviour-and-state.md`'s error table specifies it in prose — "dry run failed → show the
 * daemon string, offer `Check again`" — and it is the state a machine with no `proton-syncd` on its
 * PATH lands in on the first click. THE STRING IS QUOTED, NEVER PARAPHRASED (voice rule 4): it is
 * the difference between somebody who can search for their problem and somebody who cannot, and it
 * is why this app has no error formatter anywhere.
 *
 * Composed from drawn parts rather than invented: the centred 520 column of `4a Empty`, the decision
 * mark `3a Conflicts cleared` draws at 88, and `6a Activity passes`' quoted-error block. Nothing
 * here is a new shape; only the arrangement is S4's.
 */
function failedBody(error) {
  const body = fid(el("div", { class: "pl-failed" }), "failed");
  const mark = renderHexagon({ size: SAFE_MARK, state: "warning", tone: "decision" });
  body.append(
    mark,
    el("div", { class: "pl-failed-title" }, PLAN.failedTitle),
    el("div", { class: "pl-failed-sub" }, PLAN.failedSub),
    el("div", { class: "pl-failed-error mono" }, error),
  );
  return [body];
}

// ------------------------------------------------------------------------------ the footer ----

/**
 * The footer action bar, which on this screen is part of the screen rather than chrome.
 *
 * IT HOLDS THE GATE, which is why it is built here and patched rather than rebuilt: the shell
 * re-renders on every ~2s status poll, and a rebuilt bar is a half-typed `DELETE` destroyed. Same
 * hazard as S3's card gate, one layer out — see `updatePlanBar`.
 *
 * `Run it without the deletion` is NOT DRAWN (G3 #192). `06-plan.md`: "if unavailable, hide the
 * button rather than faking it." The one drawn button keeps its own identity in the mapping — it is
 * the frame's SECOND button, not its first.
 */
export function renderPlanBar(state = {}) {
  const v = viewOf(state);
  const handlers = state.handlers ?? {};
  const bar = buildBar(v, handlers, state);
  bar.dataset.shape = shapeOf(v);
  return fid(bar, "bar");
}

function buildBar(v, handlers, state) {
  if (v.body === "failed") {
    // THE LOUD ONE, where every other `Check again` in this screen is quiet. Maximum contrast is the
    // primary action and the primary action here is the only one — there is no plan to run, and
    // re-checking is both the way out and completely safe. The same rule that makes `Keep it` the
    // brightest button on the deletions screen.
    return renderActionBar({
      primary: fid(
        checkAgain({ handlers, size: "bar", kind: "primary", padding: "11px 22px", fontSize: "13px" }),
        "checkAgain",
      ),
    });
  }
  const gated = v.model.gated.length > 0;
  // BUILT LIVE AND THEN DISABLED, never built disabled. `button()` attaches no listener at all when
  // the KIND is a disabled one (`onClick: disabled || role.disabled ? null : onClick`), and
  // `setButtonKind` only repaints — so a button born `primaryDisabled` and later armed paints as a
  // live white primary, takes focus, and does nothing at all by pointer or by Enter. That is the
  // whole screen's action, inert, on exactly the plans the gate exists for. The browser refuses to
  // dispatch a click on a disabled element, so the guard is unchanged: the listener cannot fire
  // until `setButtonKind` clears `disabled`, and `runNow` asks the field again anyway.
  const run = fid(
    button({
      kind: "primary",
      size: "bar",
      label: PLAN.run,
      padding: "11px 22px",
      radius: "var(--r-10)",
      fontSize: "13px",
      onClick: () => runNow(v, state),
    }),
    "run",
  );
  if (gated) setButtonKind(run, "primaryDisabled");
  const bar = renderActionBar({
    consequence: gated ? gateBlock(run) : fid(el("span", { class: "pl-checked" }, checkedText(v)), "checked"),
    // The re-check moves into the bar exactly when the title row above it has gone: `5a Plan` draws
    // it beside the title, `5a Plan safe` in the footer. One button, two homes, never both — and the
    // question is which BODY is showing, not whether the plan is gated. An ungated plan that still
    // gets the list body (a conflict, an adoption, a purge) has the title row and its button, so
    // keying on the gate drew the same control twice in one window.
    secondary:
      v.body === "safe"
        ? fid(checkAgain({ handlers, size: "bar", padding: "11px 20px", fontSize: "13px" }), "checkAgain")
        : null,
    primary: run,
  });
  // THE GROUP THE FIELD MAY MOVE WITHIN IS THE BAR, and this one line is what makes the gate
  // completable at all. `deleteGate` clears on blur unless focus lands inside `[data-delete-gate]`,
  // and the button it unlocks is two siblings away — so without the attribute HERE, tabbing from the
  // field to `Run this sync` blurs it, the field clears, the button disables mid-Tab, and focus ends
  // up on nothing. Measured, not reasoned about: the first version of this screen had the attribute
  // on the gate block alone and the keyboard could not reach the button at all, which is the exact
  // trap `deleteGate`'s own comment records from S3. DEVIATIONS §55a and §76.
  if (gated) gateGroup(bar);
  fid(bar.querySelector(".shell-spacer"), "barSpacer");
  return bar;
}

/**
 * The 190px field and the sentence beside it.
 *
 * `data-delete-gate` GOES ON THE PAIR THE FIELD MAY MOVE WITHIN, and here that pair is the block
 * holding the field and its explanation — but the button it unlocks is two siblings away, in the bar
 * itself. `deleteGate` clears on blur unless focus lands inside `[data-delete-gate]`, so the
 * attribute is on the BAR: reaching `Run this sync` from the field, by pointer or by Tab, is the
 * second half of the same act and not an abandonment. That is the trap `deleteGate`'s own comment
 * records from S3, where the first version made the gate impossible to complete at all.
 */
function gateBlock(run) {
  // ONE EXPRESSION FOR THE BUTTON'S STATE, asked by the field's listener and by nothing else. The
  // kind is repainted rather than the `disabled` flag toggled, because every colour a kind carries
  // is written as an INLINE custom property and no stylesheet rule can reach past it.
  const repaint = () => setButtonKind(run, gateSatisfied(field.value) ? "primary" : "primaryDisabled");
  const field = deleteGate({ word: GATE_WORD, onChange: repaint });
  field.placeholder = PLAN.gate;
  fid(field, "gateField");
  return fid(
    el(
      "div",
      { class: "pl-gate" },
      field,
      fid(el("span", { class: "pl-gate-why" }, PLAN.gateWhy), "gateWhy"),
    ),
    "gate",
  );
}

/** `Checked 40 seconds ago against both sides.` — a relative time, so it is patched in place. */
function checkedText(v) {
  return v.checkedAt == null ? "" : PLAN.checkedAgo(since(v.checkedAt));
}

/**
 * Run the plan.
 *
 * ASKED OF THE FIELD, not of a value remembered when the word matched: the field is the only thing
 * that knows what is in it after the clear-on-blur has fired. One predicate, asked twice — by the
 * field's listener to paint the button and by this to act — so the two cannot disagree.
 *
 * WHAT THE TYPED WORD CANNOT DO is authorise the deletion, and that is an engine gap rather than a
 * shortcut taken here. `approve` matches against the daemon's CURRENT `pending_deletions` snapshot,
 * and at plan time nothing is pending — the delete has not been withheld yet, because no pass has
 * reached it. So there is nothing to approve in advance: `Run this sync` asks for a sync and nothing
 * more.
 *
 * WHAT HAPPENS NEXT DEPENDS ON THE DAEMON'S OWN GUARD, and it is not one answer. With delete
 * approval on — the default, both directions — the pass withholds the delete exactly as it would
 * have anyway and it arrives on the Deletions screen to be answered there: agreed to twice, which is
 * safe and is not what the design asks for. With it OFF (`--no-delete-approval`, a `[delete_approval]`
 * false, or a `.proton-sync.toml` that turns the guard off for that subtree) the pass deletes, and
 * the word typed here is the only thing that stood in front of it — which is the job the design
 * gives it, arrived at from the other direction. DEVIATIONS §76.
 */
function runNow(v, state) {
  if (v.model.gated.length) {
    const field = document.querySelector(".pl-gate .delete-gate");
    if (!field || !gateSatisfied(field.value)) return;
  }
  state.handlers?.onRun?.();
}

/** The bar's shape — what a rebuild would change, as opposed to what a patch can carry. */
function shapeOf(v) {
  if (v.body === "failed" || v.body === "checking") return v.body;
  return `${v.body}|${v.model.gated.length > 0}`;
}

/**
 * Patch the bar across a poll instead of rebuilding it — the gate's whole reason for living here.
 *
 * Returns false when the bar's SHAPE has changed (a different body, or a plan that has gained or
 * lost its gate), which is the caller's signal to rebuild. Everything else is the relative time,
 * which ticks every second and is the one thing on this screen that counts up.
 */
export function updatePlanBar(bar, state = {}) {
  if (!bar) return false;
  const v = viewOf(state);
  if (bar.dataset.shape !== shapeOf(v)) return false;
  const checked = bar.querySelector(".pl-checked");
  if (checked) {
    const text = checkedText(v);
    if (checked.textContent !== text) checked.textContent = text;
  }
  return true;
}

// ------------------------------------------------------------------------------ the screen ----

/**
 * What the last render was built from, so the next one can decide whether to build at all.
 *
 * THIS SCREEN MAY NOT BE REBUILT ON THE POLL, for S3's reason and one of its own: the gate is a
 * focused `<input>` whose contents clear on blur BY DESIGN, so a rebuild every two seconds wipes a
 * half-typed word; and the checking body runs two CSS animations, which `replaceChildren` restarts
 * from 0% — the failure `updateHexagon` exists to prevent.
 */
let view = null;

/** The props every render path needs, normalised once. */
function viewOf(state) {
  return {
    body: bodyOf(state),
    model: summarise(state.dryRun?.report?.plan ?? []),
    error: state.error ?? null,
    checkedAt: state.checkedAt ?? null,
  };
}

/**
 * Everything that changes the DOM, as one comparable string.
 *
 * WHAT THE CURRENT BODY DRAWS, and nothing else. The checking body draws two fixed sentences and a
 * running mark, so it depends on nothing at all — which is exactly right, because folding anything
 * into it would restart the animation on the next poll. The plan bodies are keyed on the rows
 * themselves: a re-check returning the same plan must not rebuild the screen, and one returning a
 * different plan must.
 */
function signatureOf(v) {
  if (v.body === "checking") return "checking";
  if (v.body === "failed") return JSON.stringify(["failed", v.error]);
  return JSON.stringify([
    v.body,
    v.model.rows.map((row) => [row.path, row.destination_path, row.action, row.entity_kind]),
  ]);
}

/**
 * Render the plan screen as an ARRAY of window-root siblings — never a wrapper.
 *
 * Same rule as S2 and S3: `shell.css` makes the window the flex column, and the seam's `left: 50%`
 * has to resolve against the window rather than against a wrapper this screen invented.
 */
export function renderPlan(state = {}) {
  const v = viewOf(state);
  const handlers = state.handlers ?? {};
  const nodes =
    v.body === "checking"
      ? checkingBody(handlers)
      : v.body === "failed"
        ? failedBody(v.error)
        : v.body === "safe"
          ? safeBody(v.model)
          : planBody(v.model, handlers);
  view = { sig: signatureOf(v), nodes };
  return nodes;
}

/**
 * The poll's path: rebuild only when something the body draws has moved.
 *
 * Returns the new blocks when it rebuilt and `null` when it did not, which is the shape `app.js`'s
 * main-screen and deletions branches already expect.
 */
export function updatePlan(state = {}) {
  if (!view) return null;
  const v = viewOf(state);
  if (signatureOf(v) === view.sig) return null;
  return renderPlan(state);
}

/** Drop the cached view — the screen is going away, and the next mount must build from scratch. */
export function unmountPlan() {
  view = null;
}
