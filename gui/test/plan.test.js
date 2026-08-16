// The plan screen's pure decisions (S4). A frame is one plan — nine actions with a remote delete,
// seven files moving — so the style gate defends none of what is here: two deletions, a deletion
// applied on this computer, a `purge`, an adoption or type clash the safe screen has no column for,
// a summary claiming a conflict that is not there, a rehearsal that failed.
//
// Two invariants carry the file:
//   · `isGated` is not `isDisplayDestructive`. Collapsed one way, a `purge` puts the typed-DELETE
//     gate in front of somebody for nothing; collapsed the other, a real deletion loses its tint.
//   · `bodyOf` keys on "everything is a file crossing the seam", not "nothing is destructive" — the
//     safe screen has two file lists and nowhere to draw a conflict, which would go missing.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  GATE_WORD,
  bodyOf,
  checkingProgressText,
  filterableFor,
  footerKindOf,
  gateSatisfied,
  gatedKind,
  hiddenActions,
  isDisplayDestructive,
  isGated,
  markOf,
  pathOf,
  sideOf,
  sortedForDisplay,
  summarise,
} from "../src/js/screens/plan.js";
import { MAIN, PLAN } from "../src/js/ui/copy.js";
import { outcomeOf } from "../src/js/ui/format.js";

const row = (path, action, extra = {}) => ({
  path,
  destination_path: null,
  action,
  entity_kind: "file",
  conflict_path: null,
  remote_id: null,
  ...extra,
});

/**
 * `5a Plan`'s own nine, row for row, in the daemon's emission order rather than the drawn one. It
 * has to be the fixture's nine: the counts asserted below are the ones the frame draws (`3`
 * leaving, `2` arriving, `9 actions`), and a shorter plan drops the multi-upload and multi-download
 * cases — the only ones where a per-side count can disagree with a per-side list.
 */
const NINE = [
  row("docs/spec.md", "upload"),
  row("photos/trip/img_0042.jpg", "upload"),
  row("notes/scratch.md", "upload"),
  row("photos/trip", "create_remote_directory", { entity_kind: "directory" }),
  row("archive/old-notes.md", "remote_delete"),
  row("reports/q3-summary.pdf", "download"),
  row("design/logo.svg", "download"),
  row("notes/old.md", "move_local", { destination_path: "notes/archive/old.md" }),
  row("notes/todo.txt", "conflict", { conflict_path: "notes/todo.proton-cloud.txt" }),
];

const payload = (plan) => ({ report: { plan }, requires_delete_gate: false, files_at_risk: [] });

// ------------------------------------------------------------------------ the two destructive sets

test("a purge is tinted but never gated", () => {
  assert.equal(isDisplayDestructive("purge"), true);
  assert.equal(isGated("purge"), false);
  // Both halves of the pair the daemon actually gates.
  for (const action of ["remote_delete", "local_delete"]) {
    assert.equal(isDisplayDestructive(action), true, action);
    assert.equal(isGated(action), true, action);
  }
});

test("a purge-only plan draws no band and no gate", () => {
  const model = summarise([row("stale-record", "purge"), row("docs/spec.md", "upload")]);
  assert.equal(model.gated.length, 0);
  // …and it still gets the list body, because a purge is not a file crossing the seam.
  assert.equal(bodyOf({ dryRun: payload([row("stale-record", "purge")]) }), "plan");
});

test("nothing but a file transfer is gated", () => {
  for (const action of ["upload", "download", "conflict", "auto_link", "move_local"]) {
    assert.equal(isGated(action), false, action);
  }
});

// ------------------------------------------------------------------------------- which body

test("the drawn plan is the list and the drawn safe plan is the hero", () => {
  assert.equal(bodyOf({ dryRun: payload(NINE) }), "plan");
  const safe = NINE.filter((r) => r.action !== "remote_delete" && r.action !== "conflict");
  assert.equal(bodyOf({ dryRun: payload(safe) }), "safe");
});

test("a plan the safe screen cannot hold gets the list, however harmless it is", () => {
  // None of these is destructive and none is a file crossing the seam, so the safe screen would
  // draw two empty columns and lose the action.
  for (const action of ["conflict", "auto_link", "type_conflict", "skip_unsupported", "purge"]) {
    assert.equal(bodyOf({ dryRun: payload([row("a", "upload"), row("b", action)]) }), "plan", action);
  }
});

test("an action nobody has drawn is never dropped off the safe screen", () => {
  assert.equal(sideOf("something_new"), null);
  assert.equal(bodyOf({ dryRun: payload([row("a", "something_new")]) }), "plan");
  // …and it still gets a mark and a row rather than a blank.
  assert.notEqual(markOf("something_new").glyph, null);
});

test("checking outranks everything, and a failure outranks a plan", () => {
  assert.equal(bodyOf({ dryRun: payload(NINE), checking: true }), "checking");
  assert.equal(bodyOf({ error: "boom", checking: true }), "checking");
  assert.equal(bodyOf({ error: "boom" }), "failed");
  // Both at once is unreachable (`ensurePlan` clears one when it sets the other), but the order
  // still matters: a failed rehearsal leaves the plan unverified, and drawing it under a live
  // `Run this sync` would offer to run what the app has just failed to check.
  assert.equal(bodyOf({ dryRun: payload(NINE), error: "boom" }), "failed");
  // Nothing at all is the state between the call and its answer, not an empty plan.
  assert.equal(bodyOf({}), "checking");
});

test("an empty plan is safe, not a list", () => {
  assert.equal(bodyOf({ dryRun: payload([]) }), "safe");
  // …with its own words rather than the safe screen's: `Nothing gets deleted` over a count of
  // nought is true and says nothing.
  assert.equal(summarise([]).total, 0);
  assert.equal(PLAN.nothingTitle, "Nothing needs to move");
  assert.match(PLAN.nothingSub, /^Both sides already match\./);
});

test("the safe sentence agrees with its own count, including at one and at nought", () => {
  // `plural` agrees the noun only; the verb has to be agreed alongside it. Nought is reachable
  // without the plan being empty — a plan can be one new folder.
  assert.match(PLAN.safeSub(1), /^One file moves,/);
  assert.match(PLAN.safeSub(5), /^Five files move,/);
  assert.match(PLAN.safeSub(0), /^No files move,/);
});

test("only the checking body wears the four doors", () => {
  assert.equal(footerKindOf({ checking: true }), "doors");
  assert.equal(footerKindOf({ dryRun: payload(NINE) }), "actionBar");
  assert.equal(footerKindOf({ error: "boom" }), "actionBar");
});

// ----------------------------------------------------------------------------------- ordering

test("destructive rows float to the top and everything else keeps the daemon's order", () => {
  const sorted = sortedForDisplay(NINE);
  assert.equal(sorted[0].action, "remote_delete");
  assert.deepEqual(
    sorted.slice(1).map((r) => r.path),
    [
      "docs/spec.md",
      "photos/trip/img_0042.jpg",
      "notes/scratch.md",
      "photos/trip",
      "reports/q3-summary.pdf",
      "design/logo.svg",
      "notes/old.md",
      "notes/todo.txt",
    ],
  );
  // The input is not mutated: the caller's plan is the payload's own array.
  assert.equal(NINE[0].path, "docs/spec.md");
});

test("two deletions keep their own relative order", () => {
  const plan = [row("a", "upload"), row("b", "remote_delete"), row("c", "local_delete")];
  assert.deepEqual(
    sortedForDisplay(plan).map((r) => r.path),
    ["b", "c", "a"],
  );
});

// -------------------------------------------------------------------------------- the counts

test("the side counts are files, and the folder and the rename are sentences", () => {
  const model = summarise(NINE);
  // The frame's own numerals: `3` over `files` on the left, `2` on the right, `9 actions` in the
  // tally.
  assert.equal(model.total, 9);
  assert.equal(model.uploads, 3);
  assert.equal(model.downloads, 2);
  assert.equal(model.newFolders, 1);
  assert.equal(model.renames, 1);
  assert.equal(model.conflicts, 1);
  // The new folder (leaving) and the rename (arriving) are listed but not counted as files — the
  // distinction `5a Plan safe` makes saying `Five files move` over seven actions, so each side
  // lists one more row than it counts.
  assert.equal(model.leaving.length, 4);
  assert.equal(model.arriving.length, 3);
  assert.equal(PLAN.safeSub(model.uploads + model.downloads), PLAN.safeSub(5));
});

test("the two moves are on opposite sides of the seam", () => {
  assert.equal(sideOf("move_local"), "arriving");
  assert.equal(sideOf("move_remote"), "leaving");
});

// ------------------------------------------------------------------------------------- copy

test("the summary claims a conflict only when there is one", () => {
  assert.equal(PLAN.actionSummary(9, 1), "9 actions · 1 conflict kept as both copies");
  assert.equal(PLAN.actionSummary(9, 0), "9 actions");
  assert.equal(PLAN.actionSummary(1, 0), "1 action");
  assert.equal(PLAN.actionSummary(4, 2), "4 actions · 2 conflicts kept as both copies");
});

test("the rehearsal sentence drops its irreversible clause when nothing is", () => {
  assert.match(PLAN.sub(1), /^One of them can't be undone\./);
  assert.match(PLAN.sub(2), /^Two of them can't be undone\./);
  assert.equal(PLAN.sub(0), "Everything here is a rehearsal — nothing has changed yet.");
});

test("the band names the side the file is being removed from", () => {
  assert.match(PLAN.destructiveRemote("a/b.md"), /^a\/b\.md is removed from Proton Drive\./);
  assert.match(PLAN.destructiveLocal("a/b.md"), /^a\/b\.md is removed from this computer\./);
  // The mirror is the same sentence with the sides swapped, and neither promises a way back.
  for (const sentence of [PLAN.destructiveRemote("x"), PLAN.destructiveLocal("x")]) {
    assert.match(sentence, /nothing will bring it back\.$/);
  }
  assert.equal(PLAN.destructiveMany(3), "3 files are removed for good. Nothing will bring them back.");
});

test("the band's title agrees with the number of files in it", () => {
  assert.equal(PLAN.destructiveTitle(1), "One file gets deleted for good");
  assert.equal(PLAN.destructiveTitle(2), "Two files get deleted for good");
});

test("a whole folder is not called a file", () => {
  // `plan_sync` emits LocalDelete/RemoteDelete with EntityKind::Directory when a subtree went
  // cleanly on one side — the largest loss this band describes.
  const folder = row("photos/2019", "local_delete", { entity_kind: "directory" });
  const file = row("a.md", "remote_delete");
  assert.equal(gatedKind([folder]), "folder");
  assert.equal(gatedKind([file]), "file");
  assert.equal(gatedKind([folder, file]), "thing");
  assert.equal(gatedKind([]), "thing");
  assert.equal(PLAN.destructiveTitle(1, "folder"), "One folder gets deleted for good");
  assert.equal(PLAN.destructiveTitle(2, "thing"), "Two things get deleted for good");
  assert.equal(
    PLAN.destructiveMany(3, "folder"),
    "3 folders are removed for good. Nothing will bring them back.",
  );
});

test("a folder's consequence names what is inside it", () => {
  assert.equal(
    PLAN.destructiveLocal("photos/2019", true),
    "photos/2019 and everything inside it is removed from this computer. It's already gone from " +
      "Proton Drive, so nothing will bring it back.",
  );
  // The clause is appended after the path, so the mono span in the band still wraps the path alone.
  assert.match(PLAN.destructiveRemote("photos/2019", true), /^photos\/2019 and everything inside it /);
});

test("the side unit omits a size it was not given", () => {
  assert.equal(PLAN.sideUnit(3, "4.1 MB"), "files, 4.1 MB");
  assert.equal(PLAN.sideUnit(3, null), "files");
  assert.equal(PLAN.sideUnit(1, null), "file");
});

test("both directions are counted, and each sentence names its own side", () => {
  // The engine emits `create_local_directory` and `move_remote` too; counting only their mirrors
  // left a side with no sentence and — before the row test in `seamBlock` — no column at all.
  const model = summarise([
    row("here/new", "create_local_directory", { entity_kind: "directory" }),
    row("there/new", "create_remote_directory", { entity_kind: "directory" }),
    row("a.md", "move_remote", { destination_path: "b.md" }),
    row("c.md", "move_local", { destination_path: "d.md" }),
  ]);
  assert.equal(model.newFolders, 2);
  assert.equal(model.renames, 2);
  assert.equal(model.leaving.length, 2);
  assert.equal(model.arriving.length, 2);
  // Nought files either way, and neither side is empty.
  assert.equal(model.uploads, 0);
  assert.equal(model.downloads, 0);
  assert.match(PLAN.plusFolderHere(1), /created on this computer/);
  assert.match(PLAN.plusRenameThere(1), /renamed on Proton Drive to match\.$/);
});

test("the folder and rename sentences count", () => {
  assert.equal(PLAN.plusFolder(1), "Plus one new folder created on Proton Drive to hold them.");
  assert.equal(PLAN.plusFolder(2), "Plus two new folders created on Proton Drive to hold them.");
  assert.equal(PLAN.plusRename(1), "One file you renamed will be renamed here to match.");
  assert.equal(PLAN.plusRename(3), "Three files you renamed will be renamed here to match.");
});

// ------------------------------------------------------------------------------- the outcomes

test("every action the engine can emit has an outcome in the plan register", () => {
  // `SyncAction`'s own variants (src/sync.rs, snake_case). A blank is a row that names a file and
  // says nothing about what happens to it. Four of these are chosen copy rather than measured, and
  // are recorded in DEVIATIONS §76.
  const actions = [
    "upload",
    "download",
    "create_remote_directory",
    "create_local_directory",
    "move_local",
    "move_remote",
    "auto_link",
    "conflict",
    "type_conflict",
    "remote_delete",
    "local_delete",
    "purge",
    "skip_unsupported",
  ];
  for (const action of actions) {
    assert.equal(typeof outcomeOf(action, "plan"), "string", action);
    assert.equal(typeof outcomeOf(action, "row"), "string", action);
  }
});

test("the two moves no longer say the same thing", () => {
  // F7 gave both the drawn `moved to match Proton`, which is right for a rename made on Proton and
  // says the opposite for one made here.
  assert.notEqual(outcomeOf("move_local", "plan"), outcomeOf("move_remote", "plan"));
  assert.equal(outcomeOf("move_local", "plan"), "moved to match Proton");
});

test("an action the engine cannot emit still returns nothing rather than a guess", () => {
  assert.equal(outcomeOf("something_new", "plan"), null);
});

// ------------------------------------------------------------------------------------ marks

test("a conflict draws a ring and a deletion draws a crimson cross", () => {
  assert.equal(markOf("conflict").glyph, null);
  assert.deepEqual(markOf("remote_delete"), { glyph: "✕", tone: "destructive" });
  // A purge is tinted with the deletions but is not crimson: it takes nothing away.
  assert.equal(markOf("purge").tone, "quiet");
});

test("the arrows follow direction, not action", () => {
  assert.equal(markOf("upload").tone, "up");
  assert.equal(markOf("download").tone, "down");
  assert.equal(markOf("create_remote_directory").tone, "up");
  assert.equal(markOf("create_local_directory").tone, "down");
});

// -------------------------------------------------------------------------------- the gate

test("the gate is case-sensitive and exact", () => {
  assert.equal(gateSatisfied(GATE_WORD), true);
  for (const value of ["delete", "Delete", " DELETE", "DELETE ", "DELETED", "", null, undefined]) {
    assert.equal(gateSatisfied(value), false, JSON.stringify(value));
  }
});

// ---------------------------------------------------------------------------------- paths

test("a move draws both of its ends in one row", () => {
  assert.equal(
    pathOf(row("notes/old.md", "move_local", { destination_path: "notes/archive/old.md" })),
    "notes/old.md → notes/archive/old.md",
  );
  assert.equal(pathOf(row("docs/spec.md", "upload")), "docs/spec.md");
});

// ------------------------------------------------------------------- the rehearsal's progress

test("the progress line needs a numerator, and drops the denominator rather than inventing one", () => {
  // #209. `null` is the pass that has not reached the local scan yet — the line is absent, not
  // `0 of 12,480`, which reads as stalled.
  assert.equal(checkingProgressText(null), null);
  assert.equal(checkingProgressText({}), null);
  assert.equal(checkingProgressText({ total: 12_480 }), null);
  // Both halves: the drawn line.
  assert.equal(checkingProgressText({ scanned: 8431, total: 12_480 }), PLAN.checkingProgress(8431, 12_480));
  // A FIRST RUN has an empty index, and a denominator of zero is not a denominator.
  assert.equal(checkingProgressText({ scanned: 8431, total: 0 }), PLAN.checkingProgressBare(8431));
  assert.equal(checkingProgressText({ scanned: 8431, total: null }), PLAN.checkingProgressBare(8431));
  // Zero scanned is a real answer — the walk started and has seen nothing yet.
  assert.equal(checkingProgressText({ scanned: 0, total: 12_480 }), PLAN.checkingProgress(0, 12_480));
});

// --------------------------------------------------------------------------- the filtered apply

test("a windowed reply counts the whole plan, never the rows it happened to carry", () => {
  // #100's reply is bounded (`PLAN_ACTIONS_*`), so `report.plan` can be fewer rows describing the
  // same plan. The title must not claim the shorter number — the token applies all of it.
  const rows = [row("a.txt", "upload"), row("b.txt", "upload")];
  assert.equal(summarise(rows).total, 2);
  assert.equal(summarise(rows, 12_480).total, 12_480);
  // Never below the rows in hand: a count under the list beside it would be visibly wrong.
  assert.equal(summarise(rows, 1).total, 2);
  assert.equal(summarise(rows, null).total, 2);
  // The per-direction counts stay what the window really holds — they describe drawn rows.
  assert.equal(summarise(rows, 12_480).uploads, 2);
});

test("`+n more` is sized from the daemon's total and not from the list it was handed", () => {
  // #319. THE WHOLE POINT IS THE SUBTRACTION'S LEFT-HAND SIDE. `report.plan` is a window
  // (`PLAN_ACTIONS_MAX_LIMIT` 5000) and `report.summary.total` counts the plan, so anything
  // computed from the rows alone is 0 for ever and the node is dead code that looks live —
  // `hiddenTransfers` on the main screen carries the same warning for the same reason.
  //
  // Driven from a `DryRunPayload`-shaped report through the call the screen really makes, not from
  // a hand-made model: the two seams that could disagree are `summarise`'s arguments and this
  // subtraction, and a test that builds the model itself asserts neither.
  const windowed = (carried, total) => {
    const plan = Array.from({ length: carried }, (_, i) => row(`bulk/${i}.txt`, "upload"));
    const report = { summary: { total }, plan };
    return hiddenActions(summarise(report.plan, report.summary.total));
  };

  assert.equal(windowed(5000, 12_480), 7480, "12,480 planned, 5,000 carried");
  assert.equal(MAIN.andMore(windowed(5000, 12_480)), "+7,480 more");

  // A WHOLE PLAN IN HAND DRAWS NOTHING, and never `+0 more`. The child `--dry-run` path returns
  // every row, so this is the ordinary case and not an edge one.
  assert.equal(windowed(9, 9), 0);
  assert.equal(windowed(0, 0), 0);

  // A summary the daemon did not send (an older reply) leaves `summarise` counting the rows, so
  // the two numbers are the same one and the answer is "nothing hidden" rather than a guess.
  assert.equal(hiddenActions(summarise([row("a.txt", "upload")], null)), 0);
  assert.equal(hiddenActions(summarise([row("a.txt", "upload")])), 0);

  // NEVER NEGATIVE. `summarise` already floors `total` at the rows in hand, and this floors again:
  // a reply whose summary undercounts its own rows must not produce `+-3 more`.
  assert.equal(windowed(5, 2), 0);
  assert.equal(hiddenActions({ total: 2, rows: [1, 2, 3, 4, 5] }), 0);
  assert.equal(hiddenActions(undefined), 0);
});

test("a plan nobody is holding cannot be run without its deletions", () => {
  // #192's button names a token, and a plan from the `--dry-run` child (onboarding, before any
  // daemon exists) has none — so the button is hidden rather than faked, `06-plan.md`'s own rule.
  const plan = { report: { plan: [row("gone.txt", "remote_delete")] } };
  assert.equal(filterableFor({ dryRun: plan }), false);
  assert.equal(filterableFor({ dryRun: { ...plan, token: "" } }), false);
  assert.equal(filterableFor({ dryRun: { ...plan, token: 42 } }), false);
  assert.equal(filterableFor({ dryRun: { ...plan, token: "1:abc" } }), true);
});
