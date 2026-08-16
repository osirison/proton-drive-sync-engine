// The plan screen (S4) — the rehearsal. `06-plan.md`. Nothing that reads zero gets a tile.
//
// Four bodies, not the three the frames draw: `14-behaviour-and-state.md`'s error table specifies a
// failed rehearsal ("show the daemon string, offer `Check again`") that no frame draws, and without
// it a machine with no `proton-syncd` on its PATH renders a blank window. This is also the one
// screen whose footer changes with its own state (`footerKindOf`, DEVIATIONS §76).
//
// Phase 1 gaps, each in DEVIATIONS.md §76 with the issue that closes it: every byte total (G2 #191
// — no dry-run field carries a size), the checking screen's `8,431 of 12,480 files` (G9 #209 has no
// progress channel, G7 #207 no index-wide count), `Run it without the deletion` and `Leave it
// alone` (G3 #192; `06-plan.md` says to hide rather than fake).

import { el } from "../ui/el.js";
// `MAIN.andMore` is the `+n more` template, borrowed rather than copied (#319). It is the same
// sentence about the same thing — a bounded window with a daemon-sized remainder behind it — and
// `MAIN.authExpiredSub` is already quoted by a screen that is not the main one, for the same
// reason: a second identical template is two places to change and one place to forget.
import { MAIN, PLAN } from "../ui/copy.js";
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

// ------------------------------------------------------------------------------ the model ----

/**
 * Which side of the seam an action is on, or `null` for one that is on neither.
 *
 * `null` is what `bodyOf` keys on: a deletion, conflict, adoption, purge, type clash — or any
 * unrecognised action, which must not be dropped off the screen — is not a file crossing the seam,
 * and the safe body is two lists of files moving with nowhere else to put a row.
 *
 * The moves go by the side the change lands on: `move_local` arrives, `move_remote` leaves.
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
 * Behind the typed word — `SyncAction::delete_direction().is_some()`, the rows that remove real user
 * data. Never a `purge`: that clears an index record for something already gone from both sides.
 */
export function isGated(action) {
  return action === "remote_delete" || action === "local_delete";
}

/**
 * The plan in drawn order: display-destructive rows first, otherwise the daemon's own order.
 *
 * A deliberate second copy of `plan.rs::sorted_for_display`, which lives in gui-core and cannot be
 * reached from here — `run_dry_run` returns the parsed `DryRunReport` verbatim, in emission order.
 * Change one, change the other. The index is an explicit tie-break: the daemon's order does not
 * depend on sort stability, which nothing here tests in WebKitGTK — the engine that ships.
 */
export function sortedForDisplay(plan = []) {
  const first = (row) => (isDisplayDestructive(row.action) ? 0 : 1);
  return plan
    .map((row, at) => ({ row, at }))
    .sort((a, b) => first(a.row) - first(b.row) || a.at - b.at)
    .map(({ row }) => row);
}

/**
 * Which body to draw.
 *
 * `safe` is "every action is a file crossing the seam", not "no deletions": `5a Plan safe` has
 * nowhere to put a conflict, an adoption or a purge, so those take the list body, which has a row
 * for everything and shows the destructive band only when something is gated. `checking` outranks a
 * payload — the plan on screen is the old one, and drawing it under a live `Run this sync` would
 * offer to run something already disbelieved.
 */
export function bodyOf({ dryRun = null, checking = false, error = null } = {}) {
  if (checking) return "checking";
  if (error) return "failed";
  if (!dryRun) return "checking";
  return isSafeBody(modelFor(dryRun)) ? "safe" : "plan";
}

/**
 * THE SAFE BODY IS ONLY EVER REACHED BY A PLAN THAT ARRIVED WHOLE (#324).
 *
 * Two conditions, and the second is the one a window can break. `rows.every(sideOf)` is a statement
 * about the ROWS IN HAND: with a bounded reply (`PLAN_ACTIONS_*`) a 12,480-row plan whose first 500
 * rows are all transfers takes this arm while a conflict sits past the cap — drawn nowhere, because
 * the safe body has no row for one. Worse, this body's every figure is now the daemon's whole-plan
 * count (`summarise`), and its two side lists can only ever be the window: `Five files move` over a
 * truncated list, with no `+n more` line to say so, is the mixture #324 exists to remove.
 *
 * So a windowed plan takes the LIST body, which has a row for every action and names its own
 * truncation (`hiddenActions`). Below the cap the window IS the plan, so `every` is exact and this
 * changes nothing about the two frames that draw it.
 */
function isSafeBody(model) {
  return hiddenActions(model) === 0 && model.rows.every((row) => sideOf(row.action));
}

/** The model for a `DryRunPayload`, from the one call every path here makes. */
function modelFor(dryRun) {
  return summarise(dryRun?.report?.plan ?? [], dryRun?.report?.summary ?? null);
}

/**
 * Which footer the window wears — a per-state question only on this screen: both 1040 plan frames
 * draw a footer action bar and `5a Checking` draws the four doors. The failed body takes the bar,
 * because `14-behaviour-and-state.md` gives the error state a `Check again` and the doors cannot
 * hold it.
 */
export function footerKindOf(state = {}) {
  return bodyOf(state) === "checking" ? "doors" : "actionBar";
}

/** The gate's one predicate. Case-sensitive: `delete` is a word people type by habit. */
export function gateSatisfied(value) {
  return value === GATE_WORD;
}

/**
 * Everything the screen counts, counted once — and **every count is the daemon's** (#324).
 *
 * ONE SENTENCE MAY NOT HAVE TWO SOURCES. `report.plan` is a bounded window (`PLAN_ACTIONS_*`) while
 * `report.summary` describes the whole plan, so a model mixing the two puts `12,480 actions` beside
 * a conflict count that only counted the conflicts that fit. Every figure below therefore comes off
 * `PlanSummary`, which carries one field per `SyncAction` variant, and the window is used for
 * exactly one thing: the LISTS, which are rows and cannot be drawn for a row that was not sent.
 *
 * The row fallback is for a reply that carries no summary at all — an older daemon, or a hand-made
 * payload in a test. It is the pre-#324 behaviour, correct whenever the window is the whole plan,
 * which is every reply the `--dry-run` child produces.
 *
 * The side counts are files, not actions: `5a Plan` draws `3` for a leaving side of three uploads
 * and a new folder — the folder is the sentence underneath. `5a Plan safe`, likewise: seven actions,
 * `Five files move`. The per-side folder and rename counts are kept apart rather than summed,
 * because each names the side it happens on and `sideNote` draws four different sentences from
 * them; `PlanSummary` splits them the same way, so this is its shape and not a second one.
 */
export function summarise(plan = [], summary = null) {
  const rows = sortedForDisplay(plan);
  const countOf = (...actions) => rows.filter((row) => actions.includes(row.action)).length;
  // The daemon's field when it sent one, the window's count when it did not. Asked per field rather
  // than per reply: `PlanSummary` gained `matched_files` and `upload_bytes` as `Option`s, so a
  // present summary is not a promise that every key is present.
  const counted = (key, ...actions) =>
    typeof summary?.[key] === "number" ? summary[key] : countOf(...actions);
  return {
    rows,
    // The daemon's own count when it gave one and it exceeds the rows in hand — a windowed reply
    // is fewer rows describing the same plan. Never *below* the rows: a count under the list
    // beside it would be visibly wrong, and `hiddenActions` would go negative. Spelled out rather
    // than routed through `counted`, whose row fallback for `total` would be a count of nothing.
    total: typeof summary?.total === "number" ? Math.max(summary.total, rows.length) : rows.length,
    uploads: counted("uploads", "upload"),
    downloads: counted("downloads", "download"),
    // Both directions, each on its own side. The engine emits `create_local_directory` and
    // `move_remote` as well, and a side counting only its mirror loses its sentence — and, before
    // the count test in `seamBlock`, its column.
    remoteFolders: counted("remote_directories_created", "create_remote_directory"),
    localFolders: counted("local_directories_created", "create_local_directory"),
    remoteMoves: counted("remote_moves", "move_remote"),
    localMoves: counted("local_moves", "move_local"),
    // `conflicts` ALONE, never plus `type_conflicts`: the head's clause is `kept as both copies`,
    // which is what a `Conflict` sidecar is. A `TypeConflict` is a folder against a file — it takes
    // the list body's own row and has no copy to keep.
    conflicts: counted("conflicts", "conflict"),
    // THE THREE ROW LISTS, and they stay the window's. `gated` is complete whatever the cap —
    // `daemon::StoredPlan::outcome` builds the window destructive-first — which is what lets the
    // typed-`DELETE` gate and the band be drawn from rows at all.
    gated: rows.filter((row) => isGated(row.action)),
    leaving: rows.filter((row) => sideOf(row.action) === "leaving"),
    arriving: rows.filter((row) => sideOf(row.action) === "arriving"),
  };
}

/**
 * One side of the seam, as the three counts a side draws: its files, its new folders, its renames.
 *
 * One definition for both bodies. `seamBlock` asks it whether the column exists at all and draws the
 * numeral from it; `sideNote` writes its sentence from the same three numbers, where it used to
 * filter the side's rows a second time — which was the same window-vs-summary mixture one level
 * down, and would have said `Plus one new folder` about a plan with four hundred of them.
 */
function sideCounts(model, side) {
  const leaving = side === "leaving";
  return {
    rows: leaving ? model.leaving : model.arriving,
    files: leaving ? model.uploads : model.downloads,
    folders: leaving ? model.remoteFolders : model.localFolders,
    renames: leaving ? model.remoteMoves : model.localMoves,
  };
}

/**
 * The mark for one planned action, and its tone.
 *
 * `glyph: null` draws a ring instead of a character — the conflict, the one row that is not a
 * direction. `＋` is the fullwidth plus (U+FF0B) the design draws, not `+`; the fidelity harness
 * exempts its width because no bundled face covers it.
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
    // Tinted like a deletion because it sorts with them, but not crimson: a purge takes nothing away.
    case "purge":
      return { glyph: "✕", tone: "quiet" };
    case "conflict":
      return { glyph: null, tone: "quiet" };
    default:
      // An undrawn action still gets a row, with a mark that claims no direction.
      return { glyph: "·", tone: "quiet" };
  }
}

/**
 * What a row's path reads as. A move draws both ends in one row
 * (`notes/old.md → notes/archive/old.md`), the only place the plan list is not one path per row.
 */
export function pathOf(row) {
  return row.destination_path ? `${row.path} → ${row.destination_path}` : row.path;
}

/**
 * How many plan rows the list is NOT showing (#319).
 *
 * SIZED FROM THE DAEMON'S COUNT, NEVER FROM THE LIST'S LENGTH — `hiddenTransfers` on the main
 * screen is the same arithmetic for the same reason, and the reason is that the only two numbers in
 * hand are both about the same window unless one of them comes off the wire. `ReviewedPlan` bounds
 * the rows it carries (`PLAN_ACTIONS_DEFAULT_LIMIT` 500, `PLAN_ACTIONS_MAX_LIMIT` 5000) while
 * `summary.total` counts the whole plan, so this subtraction is the difference between the plan and
 * the rendering of it. `total - rows.length` computed from the rows alone would be 0 for ever.
 *
 * `model.total` is already `max(reported, rows.length)` (see `summarise`), so this can never go
 * negative and a summary the daemon did not send yields `0` — no node — rather than a guess.
 *
 * A window is safe to draw at all only because every DESTRUCTIVE row is in it whatever the cap
 * (`daemon::StoredPlan::outcome`) and because applying names the plan's *token* rather than its
 * rows. So what is behind this line is only ever ordinary work; the deletions are all on screen and
 * the typed-`DELETE` gate has seen every one of them.
 */
export function hiddenActions(model) {
  return Math.max(0, (model?.total ?? 0) - (model?.rows?.length ?? 0));
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
 * One builder for both frames — the same block with a different third child (`5a Plan` a sentence,
 * `5a Plan safe` the files themselves). The geometry must not drift between them: a `1fr 1fr` grid
 * with a 30px gutter each side of the centre line, the right column right-aligned in its entirety.
 */
function seamBlock({ model, site, detail, aligned, bounded = false }) {
  // `aligned` is measured per frame: `5a Plan` right-aligns the whole arriving column (all text);
  // `5a Plan safe` right-aligns only its eyebrow and pushes the count over with `justify-content`,
  // because its third child is rows and a right-aligned column would drag each path off its glyph.
  // The gate reads `text-align` on the column itself, so it is a property of the block, not the seam.
  //
  // `bounded` is the OTHER frame's difference, and it is about height rather than alignment: only
  // the safe body hangs a list off each count, so only it can be taller than the window (#267). The
  // block below a 300px hero is the one that has to give, and the flag says which block that is
  // rather than reading it off `aligned` — the two happen to disagree on both frames today, and a
  // third body drawing an aligned list would inherit the wrong answer silently.
  const classes = ["pl-seam-block", aligned ? "is-aligned" : null, bounded ? "is-bounded" : null];
  const block = fid(el("div", { class: classes.filter(Boolean).join(" ") }), "seamBlock");
  block.append(fid(renderSeam({ site }), "seam"));
  const sides = fid(el("div", { class: "pl-sides" }), "sides");
  for (const [s, side] of ["leaving", "arriving"].entries()) {
    const leaving = side === "leaving";
    const counts = sideCounts(model, side);
    const tail = detail(side, s);
    // Nothing that reads zero gets a tile (`06-plan.md`). Asked of everything the side HAS, not of
    // its file count: a side whose only action is a folder create or a rename has something to show
    // and a count of nought. Both drawn frames have traffic both ways, so only the rule settles it.
    //
    // The daemon's three counts, not the side's rows: a windowed reply can carry no row for a side
    // the plan really has work on, and dropping the column would then hide a number this block is
    // about to draw. Safe in the other direction too — the safe body, whose rows ARE the tile, is
    // unreachable for a windowed plan (`isSafeBody`), so a drawn column can never hold an empty list.
    if (!counts.files && !counts.folders && !counts.renames) continue;
    const column = fid(el("div", { class: `pl-side${leaving ? "" : " is-arriving"}` }), "side", s);
    // Placed, not flowed: a grid child with no sibling falls into the first cell, so a plan with
    // nothing leaving would draw `Arriving from Proton` on this computer's side of the seam.
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
    const files = counts.files;
    column.append(
      fid(
        el(
          "div",
          { class: "pl-count" },
          // The 42px numeral is its own node: the design sets it at its own size beside a 14px unit.
          fid(el("span", { class: "pl-numeral" }, count(files)), "sideNumeral", s),
          // `null` is G2 (#191): no dry-run field carries a byte total, so `, 4.1 MB` is omitted
          // rather than filled with a plausible number. DEVIATIONS §76.
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

/** `5a Plan`'s third child per side: the sentence about what is not a file. */
function sideNote(model) {
  return (side, s) => {
    const leaving = side === "leaving";
    const { folders, renames } = sideCounts(model, side);
    // Four sentences, not two: a folder can be created on either side and a rename applied on
    // either side, and each names where it happens. Only the two `5a Plan` draws are in the deck.
    const parts = [
      folders ? (leaving ? PLAN.plusFolder(folders) : PLAN.plusFolderHere(folders)) : null,
      renames ? (leaving ? PLAN.plusRenameThere(renames) : PLAN.plusRename(renames)) : null,
    ].filter(Boolean);
    if (!parts.length) return null;
    return fid(el("div", { class: "pl-side-note" }, parts.join(" ")), "sideNote", s);
  };
}

/**
 * `5a Plan safe`'s third child per side: the files themselves, one row each.
 *
 * NO CAP, and the block around it scrolls instead (`.pl-seam-block.is-bounded`, plan.css). A row
 * per action is what the frame draws and what the state means — every file that moves, named — and
 * the number that fits is not a constant to write down here: six rows fit under a 300px hero today,
 * five would if the footer ever grew a line, and `.pl-rows` already lost that argument once
 * (`ROWS_THAT_FIT`). A list that is longer than its room is the shell's own scroll case (02-shell.md).
 */
function sideList(model) {
  return (side, s) => {
    const rows = side === "leaving" ? model.leaving : model.arriving;
    if (!rows.length) return null;
    const list = fid(el("div", { class: "pl-side-list" }), "sideList", s);
    for (const [i, row] of rows.entries()) {
      const { glyph, tone } = markOf(row.action);
      // A size and a word draw differently (see `planSideRow`), and `noteFor` answering null is the
      // size case: a file row's note is its size, which Phase 1 cannot report (#191), so it draws no
      // note rather than an em-dash. `noteIsSize` keeps the file row's brighter path regardless.
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
 * What the band's title calls the things it is about to lose.
 *
 * A whole-subtree deletion is a real planned action (`plan_sync` emits `LocalDelete`/`RemoteDelete`
 * with `EntityKind::Directory`), so the noun has to agree with it. A mixed set gets `thing`, because
 * either noun would be wrong about half of it.
 */
export function gatedKind(gated = []) {
  const kinds = new Set(gated.map((row) => (row.entity_kind === "directory" ? "folder" : "file")));
  return kinds.size === 1 ? [...kinds][0] : "thing";
}

const FOLDER_ACTIONS = new Set(["create_remote_directory", "create_local_directory"]);
const MOVE_ACTIONS = new Set(["move_local", "move_remote"]);

/** `new folder` / `moved`, or nothing at all for a file whose size Phase 1 cannot report (#191). */
function noteFor(action) {
  if (FOLDER_ACTIONS.has(action)) return PLAN.newFolder;
  if (MOVE_ACTIONS.has(action)) return PLAN.moved;
  return null;
}

/**
 * The destructive band — the dangerous thing breaking out of the seam, never just a tinted row in a
 * list (`06-plan.md`).
 *
 * `Leave it alone` is not drawn: read either way — drop this action and run the rest, or refuse the
 * deletion durably — it needs a capability Phase 1 does not have (G3 #192, or #224's durable
 * refusal), and `06-plan.md` says to hide the button rather than fake it.
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
 * One deletion's sentence as the three pieces the band's note holds directly: prose, the path in
 * mono, prose. An array rather than a wrapper span — the frame draws exactly one element in that
 * note.
 *
 * Split on where the template puts the path, never on `indexOf(path)` in the finished string: a file
 * called `is` matches inside the prose before its own slot. `U+0001` cannot occur in a path, and is
 * written as an escape because a literal control character in a source file is its own bug
 * (tools/check-sources.mjs).
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
  // ALWAYS SCROLLABLE, and the count that used to decide it is gone. `ROWS_THAT_FIT` was a fixed
  // number for a height that is not fixed: a destructive band whose note wraps to a second line
  // takes another 20px off the list, and the row that no longer fits was clipped by
  // `overflow:hidden` with no way to reach it. A scroller costs nothing when everything fits —
  // `overflow-y:auto` draws no bar — and is right in every state, including the ones no frame draws.
  const rows = fid(el("div", { class: "pl-rows is-scrollable" }), "rows");
  for (const [i, row] of model.rows.entries()) {
    const { glyph, tone } = markOf(row.action);
    const node = fid(
      planActionRow({
        glyph,
        tone,
        path: pathOf(row),
        outcome: outcomeOf(row.action, "plan"),
        // Two sets, one flag each: the tint groups what `sorted_for_display` floats to the top, the
        // emphasis marks what takes data away. A `purge` is in the first set and not the second.
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
  // THE LINE THAT SAYS THE LIST IS A WINDOW (#319). Inside the scroller and after the last row,
  // because it is a fact about where the list stops rather than a fact about the plan — the title
  // above already states the plan's own size, and this is the only place the two numbers can be
  // seen to differ.
  //
  // NO FRAME DRAWS IT: none of the three plan frames has a plan long enough to truncate, and no
  // fixture reaches the daemon's cap. So it is an undrawn state, it stamps no slot, and it is
  // deliberately not in `fids.js` — a declared slot whose key exists in no frame fails
  // `check-fixtures.mjs`, and `KNOWN_UNSTAMPED` is for a node the frame draws and the app cannot,
  // which is the opposite of this.
  const hidden = hiddenActions(model);
  if (hidden > 0) rows.append(el("div", { class: "pl-more" }, MAIN.andMore(hidden)));
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
 * State B — the ordinary safe plan: a hero and two short lists.
 *
 * An empty plan routes here too (`14-behaviour-and-state.md`: "Plan · Empty: safe-plan variant"),
 * and no frame draws it. The hero swaps its copy and the seam block goes entirely, rather than
 * drawing `zero files move` over two columns of `0`.
 */
function safeBody(model) {
  const empty = model.total === 0;
  const hero = fid(el("div", { class: "pl-hero" }), "hero");
  // `heroSeam`, not `seam`: `5a Plan safe` draws two seams (a continuation pair overlapping by 40px)
  // and a slot name stamps a `data-fid`, so sharing one compares both nodes against one drawn line.
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
  // Both masked: the seam runs 40px past the bottom of this block, through both lines. Pads are the
  // frame's own — 18px on the 28px headline, 18px and 2px on the 13.5px sentence — and `position` is
  // what does the hiding (F3 rule 3: an absolute seam paints above a static sibling's text).
  seamMask(title, { pad: 18 });
  seamMask(sub, { pad: 18, padY: 2 });
  hero.append(mark, title, sub);
  return [
    hero,
    empty
      ? null
      : seamBlock({
          model,
          site: "planSafeList",
          detail: sideList(model),
          aligned: false,
          bounded: true,
        }),
    // The empty flex:1 block the frame draws between the lists and the footer. A real node rather
    // than a margin: the footer is a child of the window, so something has to take up the slack.
    fid(el("div", { class: "pl-spacer" }), "tailSpacer"),
  ].filter(Boolean);
}

/**
 * State C — working it out.
 *
 * Drawn 520px wide where the shell is a fixed 1040: `3a Conflicts cleared` and `4a Empty` take the
 * same answer, a centred 520 column, with the difference recorded rather than faked (#221, §76).
 *
 * No numeral — the mark is reading, not moving. F2's `dryRun` flag carries the thinner, faster dash
 * the frame draws (`40 260` at 2.4s/3.2s against the syncing mark's `62 238` at 3.2s/4.4s).
 */
function checkingBody(handlers, progress) {
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
  // `8,431 of 12,480 files`. Both halves have a source now: the rehearsal runs on the daemon's
  // main loop (#209), so `activity.files_scanned` describes THIS walk, and `index_totals.files` is
  // the corpus size (#207). Drawn only when the numerator exists — the pass may not have reached
  // the local scan yet — and the denominator is dropped rather than invented when the index is
  // empty (a first run has nothing to count against), which is the one clause the design's fixed
  // fraction cannot express. DEVIATIONS §76.
  const progressText = checkingProgressText(progress);
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
  // The one button in the app that masks with its own fill. `06-plan.md`: "`Stop` is
  // `background:#0A0B0D`, not transparent, so the seam passes behind it" — but the frame draws the
  // fill without the `position` that makes it work, and an unpositioned mask is the `1a Compact` bug
  // F3 records (the seam paints over the fill). The `position` difference is recorded in §76.
  seamMask(stop, { pad: null });
  body.append(mark, title, sub, stop);
  // Appended CONDITIONALLY, never as a `null` child: `Element.append(null)` inserts the literal
  // string "null", which no gate here can see (every frame's fixture carries the progress, so the
  // absent case is undrawn).
  if (progressText != null) insertCheckingProgress(body, progressText);
  return [body];
}

/**
 * The fourth body, which no frame draws: the rehearsal could not run.
 *
 * `14-behaviour-and-state.md`'s error table specifies it in prose — "dry run failed → show the
 * daemon string, offer `Check again`" — and it is where a machine with no `proton-syncd` on its PATH
 * lands. The daemon string is quoted, never paraphrased (voice rule 4). Shapes are borrowed:
 * `4a Empty`'s centred 520 column, `3a Conflicts cleared`'s 88px decision mark, `6a Activity
 * passes`' quoted-error block.
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
 * It holds the gate, so it is built here and patched rather than rebuilt: the shell re-renders on
 * every ~2s status poll, and a rebuilt bar destroys a half-typed `DELETE` — see `updatePlanBar`.
 *
 * `Run it without the deletion` (#192) is drawn when the plan is gated **and** the daemon holds
 * the plan — i.e. `dryRun.token` exists. Without a token there is nothing to apply by name: that is
 * the onboarding path, where the plan came from a `proton-syncd --dry-run` child no daemon is
 * holding, and `06-plan.md` says hide the button rather than fake it.
 *
 * `Leave it alone` (the band's own escape hatch) stays hidden. It reads two ways — drop this one
 * action and run the rest, or refuse this deletion durably — and the second is #224, which the
 * `Keep it` flow on the Deletions screen owns. Drawing it here would make one button mean whichever
 * the reader assumed.
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
    // Loud, where every other `Check again` on this screen is quiet: there is no plan to run, so
    // re-checking is the only action here and it is safe.
    return renderActionBar({
      primary: fid(
        checkAgain({ handlers, size: "bar", kind: "primary", padding: "11px 22px", fontSize: "13px" }),
        "checkAgain",
      ),
    });
  }
  const gated = v.model.gated.length > 0;
  // Built live and then disabled, never built disabled: `button()` attaches no listener when the
  // kind is a disabled one (`onClick: disabled || role.disabled ? null : onClick`) and
  // `setButtonKind` only repaints, so a button born `primaryDisabled` and later armed would paint
  // live and do nothing. The guard is unchanged — the browser refuses to dispatch a click on a
  // disabled element, and `runNow` asks the field again anyway.
  const run = fid(
    button({
      kind: "primary",
      size: "bar",
      label: PLAN.run,
      padding: "11px 22px",
      radius: "var(--r-10)",
      fontSize: "13px",
      onClick: () => runNow(v, state, run),
    }),
    "run",
  );
  if (gated) setButtonKind(run, "primaryDisabled");
  // The frame's first button. `secondary` is the same slot `Check again` takes on the safe body,
  // and the two never coexist: a safe plan has no destructive row to run without.
  const runWithout =
    gated && v.filterable
      ? fid(
          button({
            kind: "secondary",
            size: "bar",
            label: PLAN.runWithout,
            padding: "11px 20px",
            radius: "var(--r-10)",
            fontSize: "13px",
            onClick: () => handlers.onRunWithout?.(),
          }),
          "runWithout",
        )
      : null;
  const bar = renderActionBar({
    consequence: gated ? gateBlock(run) : fid(el("span", { class: "pl-checked" }, checkedText(v)), "checked"),
    // The re-check moves into the bar exactly when the title row has gone: `5a Plan` draws it beside
    // the title, `5a Plan safe` in the footer. The question is which body is showing, not whether
    // the plan is gated — an ungated plan on the list body still has the title row and its button,
    // so keying on the gate draws the same control twice in one window.
    secondary:
      v.body === "safe"
        ? fid(checkAgain({ handlers, size: "bar", padding: "11px 20px", fontSize: "13px" }), "checkAgain")
        : runWithout,
    primary: run,
  });
  // The group the field may move within is the bar, not the gate block. `deleteGate` clears on blur
  // unless focus lands inside `[data-delete-gate]`, and the button it unlocks is two siblings away —
  // with the attribute on the gate block alone, tabbing from the field to `Run this sync` clears the
  // field, disables the button mid-Tab, and leaves focus on nothing. DEVIATIONS §55a and §76.
  if (gated) gateGroup(bar);
  fid(bar.querySelector(".shell-spacer"), "barSpacer");
  return bar;
}

/**
 * The 190px field and the sentence beside it. `data-delete-gate` is not here — it goes on the bar,
 * see `buildBar`.
 */
function gateBlock(run) {
  // One expression for the button's state, asked by the field's listener. The kind is repainted
  // rather than the `disabled` flag toggled, because every colour a kind carries is written as an
  // inline custom property no stylesheet rule can reach past.
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
 * Asked of the field, not of a value remembered when the word matched: the field is the only thing
 * that knows what is in it after clear-on-blur has fired. One predicate, shared with the field's
 * listener, so paint and decision cannot disagree.
 *
 * The typed word does not authorise the deletion: `approve` matches the daemon's current
 * `pending_deletions`, and at plan time nothing is pending. Whether the delete then happens is the
 * daemon's own guard — on by default, in which case the pass withholds it for the Deletions screen;
 * off (`--no-delete-approval`, `[delete_approval]` false, or a `.proton-sync.toml` for that subtree)
 * and the pass deletes with this word the only thing that stood in front of it. DEVIATIONS §76.
 */
function runNow(v, state, run) {
  if (v.model.gated.length) {
    // Scoped to the button's own group, not the document: `gateGroup` marks the bar, so the field
    // this button unlocks is the one inside it. A document query would find any other `.delete-gate`.
    const field = run?.closest("[data-delete-gate]")?.querySelector(".delete-gate");
    if (!field || !gateSatisfied(field.value)) return;
  }
  state.handlers?.onRun?.();
}

/** The bar's shape — what a rebuild would change, as opposed to what a patch can carry. */
function shapeOf(v) {
  if (v.body === "failed" || v.body === "checking") return v.body;
  // `filterable` is in the shape because it decides whether a BUTTON exists: a patch cannot add one,
  // so a plan that gained a token across a poll has to rebuild the bar.
  return `${v.body}|${v.model.gated.length > 0}|${v.filterable}`;
}

/**
 * Patch the bar across a poll instead of rebuilding it — the gate's reason for living here.
 *
 * Returns false when the bar's shape has changed (a different body, or a plan that gained or lost
 * its gate), the caller's signal to rebuild. Everything else is the relative time.
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
 * The screen may not be rebuilt on the poll: the gate is a focused `<input>` that clears on blur by
 * design, and the checking body's two CSS animations restart from 0% under `replaceChildren` — the
 * failure `updateHexagon` exists to prevent.
 */
let view = null;

/** The props every render path needs, normalised once. */
function viewOf(state) {
  return {
    body: bodyOf(state),
    // THE WHOLE SUMMARY, not just its `total` (#324). `report.plan` may be a bounded window when the
    // daemon computed it (#100's reply cap) while every field of `report.summary` describes the whole
    // plan — so passing one field and counting the rest from the window is what made the head say
    // `12,480 actions · 1 conflict` about a plan with four hundred conflicts.
    model: modelFor(state.dryRun),
    error: state.error ?? null,
    checkedAt: state.checkedAt ?? null,
    filterable: filterableFor(state),
    progress: state.progress ?? null,
  };
}

/**
 * Whether `Run it without the deletion` can do anything (#192): the daemon holds this plan and will
 * apply it by name. A plan from the `--dry-run` child has no token and no holder, so the button is
 * hidden rather than faked — `06-plan.md`'s own rule.
 */
export function filterableFor(state = {}) {
  const token = state.dryRun?.token;
  return typeof token === "string" && token.length > 0;
}

/**
 * `8,431 of 12,480 files`, or the honest fragment of it (#209).
 *
 * `null` when the pass has not reached the local scan, so the line is absent rather than `0 of …`.
 * `PLAN.checkingProgressBare` when the corpus size is unknown or zero — a first run has an empty
 * index, and a denominator of zero is not a denominator.
 */
export function checkingProgressText(progress) {
  const scanned = progress?.scanned;
  if (typeof scanned !== "number") return null;
  const total = progress?.total;
  return typeof total === "number" && total > 0
    ? PLAN.checkingProgress(scanned, total)
    : PLAN.checkingProgressBare(scanned);
}

/**
 * Everything the current body draws, as one comparable string, and nothing else. The checking body
 * depends on nothing, or folding anything in would restart its animation on the next poll; the plan
 * bodies key on the rows themselves, so a re-check returning the same plan does not rebuild.
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
 * Render the plan screen as an array of window-root siblings, never a wrapper: `shell.css` makes the
 * window the flex column, and the seam's `left: 50%` has to resolve against the window.
 */
export function renderPlan(state = {}) {
  const v = viewOf(state);
  const handlers = state.handlers ?? {};
  const nodes =
    v.body === "checking"
      ? checkingBody(handlers, v.progress)
      : v.body === "failed"
        ? failedBody(v.error)
        : v.body === "safe"
          ? safeBody(v.model)
          : planBody(v.model, handlers);
  view = { sig: signatureOf(v), nodes };
  return nodes;
}

/**
 * The poll's path: rebuild only when something the body draws has moved. Returns the new blocks, or
 * `null` when it did not rebuild — the shape `app.js`'s main-screen and deletions branches expect.
 */
export function updatePlan(state = {}) {
  if (!view) return null;
  const v = viewOf(state);
  if (signatureOf(v) === view.sig) {
    patchCheckingProgress(v);
    return null;
  }
  return renderPlan(state);
}

/** The progress line, built and masked in one place so the first render and the patch agree. */
function insertCheckingProgress(body, text) {
  const node = fid(el("div", { class: "pl-checking-progress" }, text), "checkingProgress");
  seamMask(node, { pad: 14, padY: 2 });
  // Before `Stop`, which is the frame's order and the order `.pl-stop`'s 22px gap is measured from.
  body.insertBefore(node, body.querySelector(".pl-stop"));
  return node;
}

/**
 * Add, move or drop the progress line without rebuilding the body (#209).
 *
 * `signatureOf` returns a constant for the checking body on purpose — folding anything into it
 * would restart the mark's two CSS animations from 0% on every poll, which is the failure
 * `updateHexagon` exists to prevent. So the line has to be maintained in place, exactly as the
 * bar's `Checked N ago` is.
 *
 * **Adding it here is not an optimisation, it is the only way it is ever drawn.** The screen renders
 * the checking body the instant the rehearsal is *requested* — before the daemon has acked it, let
 * alone started scanning — so the first render never has a number and a patch that could only
 * update existing text would leave the line permanently absent. Inserting a sibling is safe where
 * `replaceChildren` is not: the mark's own element is untouched, so its animations do not restart.
 */
function patchCheckingProgress(v) {
  if (v.body !== "checking") return;
  const body = view?.nodes?.[0];
  if (!body?.querySelector) return;
  const text = checkingProgressText(v.progress);
  const node = body.querySelector(".pl-checking-progress");
  if (text == null) {
    node?.remove();
  } else if (!node) {
    insertCheckingProgress(body, text);
  } else if (node.textContent !== text) {
    node.textContent = text;
  }
}

/** Drop the cached view — the screen is going away, and the next mount must build from scratch. */
export function unmountPlan() {
  view = null;
}
