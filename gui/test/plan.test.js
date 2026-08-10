// The plan screen's pure decisions (S4), tested on their own because a passing style gate would not
// defend one of them: every one is about a plan no `5a` frame draws.
//
// A frame is ONE plan. What the gate checks is nine actions with a remote delete, and seven files
// moving. What it cannot check is a plan that deletes two things, one that deletes from THIS
// computer, one carrying a `purge` (tinted like a deletion and never gated), one carrying an
// adoption or a type clash the safe screen has no column for, a plan with no conflict beside a
// summary that claims one, or a rehearsal that failed. Those are where this screen goes wrong, so
// those are what is here.
//
// The two that would fail loudest and quietest at once:
//
//   · `isGated` against `isDisplayDestructive`. Collapse them and a `purge` — an index row for a
//     file already gone from both sides — puts the typed-DELETE gate in front of somebody for
//     nothing, which is how a gate stops meaning anything. Collapse them the other way and a real
//     deletion loses its tint.
//   · `bodyOf`. The safe screen has two lists of files and nowhere to put anything else, so if it
//     is chosen on "nothing is destructive" rather than on "everything is a file crossing the seam",
//     a plan with a conflict in it renders a complete, calm screen with an action silently missing.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  GATE_WORD,
  bodyOf,
  footerKindOf,
  gateSatisfied,
  isDisplayDestructive,
  isGated,
  markOf,
  pathOf,
  sideOf,
  sortedForDisplay,
  summarise,
} from "../src/js/screens/plan.js";
import { PLAN } from "../src/js/ui/copy.js";
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

/** `5a Plan`'s own nine, in the daemon's emission order rather than the drawn one. */
const NINE = [
  row("docs/spec.md", "upload"),
  row("photos/trip", "create_remote_directory", { entity_kind: "directory" }),
  row("archive/old-notes.md", "remote_delete"),
  row("reports/q3-summary.pdf", "download"),
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
  // Not one of these is destructive, and not one of them is a file crossing the seam — so the safe
  // screen would draw two empty columns and forget the action entirely.
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
  // BOTH AT ONCE IS UNREACHABLE — `ensurePlan` clears one when it sets the other — and the order is
  // still worth pinning, because the two readings are not equally safe. A rehearsal that failed
  // leaves the plan on screen unverified, and drawing it under a live `Run this sync` would offer to
  // run something the app has just failed to check.
  assert.equal(bodyOf({ dryRun: payload(NINE), error: "boom" }), "failed");
  // Nothing at all is the state between the call and its answer, not an empty plan.
  assert.equal(bodyOf({}), "checking");
});

test("an empty plan is safe, not a list", () => {
  assert.equal(bodyOf({ dryRun: payload([]) }), "safe");
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
    ["docs/spec.md", "photos/trip", "reports/q3-summary.pdf", "notes/old.md", "notes/todo.txt"],
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
  assert.equal(model.uploads, 1);
  assert.equal(model.downloads, 1);
  assert.equal(model.newFolders, 1);
  assert.equal(model.renames, 1);
  assert.equal(model.conflicts, 1);
  // The new folder is on the leaving side and the rename on the arriving one, but neither is counted
  // as a file — that is the distinction `5a Plan safe` makes when it says `Five files move` over
  // seven actions.
  assert.equal(model.leaving.length, 2);
  assert.equal(model.arriving.length, 2);
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

test("the side unit omits a size it was not given", () => {
  assert.equal(PLAN.sideUnit(3, "4.1 MB"), "files, 4.1 MB");
  assert.equal(PLAN.sideUnit(3, null), "files");
  assert.equal(PLAN.sideUnit(1, null), "file");
});

test("the folder and rename sentences count", () => {
  assert.equal(PLAN.plusFolder(1), "Plus one new folder created on Proton Drive to hold them.");
  assert.equal(PLAN.plusFolder(2), "Plus two new folders created on Proton Drive to hold them.");
  assert.equal(PLAN.plusRename(1), "One file you renamed will be renamed here to match.");
  assert.equal(PLAN.plusRename(3), "Three files you renamed will be renamed here to match.");
});

// ------------------------------------------------------------------------------- the outcomes

test("every action the engine can emit has an outcome in the plan register", () => {
  // The list is `SyncAction`'s own variants (src/sync.rs, snake_case). A blank here is a row that
  // names your file and says nothing about what happens to it — which is the whole of what this
  // screen is for. Four of these were null until S4 and are recorded in DEVIATIONS §76 as chosen
  // copy rather than measured.
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
  // F7 gave both the drawn `moved to match Proton`, which is right for a rename that happened on
  // Proton and says the opposite for one you made here.
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
  // A purge is tinted with the deletions and is NOT crimson: it takes nothing away from you.
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
