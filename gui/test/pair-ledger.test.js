// The pair-switch ledger (#102 phase 5a-2, brief section 4.6).
//
// `app.js` keeps its state in module-level `let`s — 93 of them — because `render()` rebuilds the body
// from scratch twice a second and a screen that owned its own state would forget which conflict you
// were on. That is fine for one folder pair and is where a SECOND one goes wrong: roughly forty of
// those bindings describe ONE pair (which conflict is open, which deletion's typed gate is armed, the
// plan whose token applies to one pair only, a lookup half-typed against one index), and what each of
// them does when the pair on screen changes is a decision nobody wrote down. The bugs that decision
// produces are the repo's most-recorded kind — state that outlives its subject — and they are the
// kind no fidelity frame can see, because a frame renders one pair.
//
// So every top-level binding is classified here, exhaustively, in the idiom the engine uses for its
// config keys (`ConfigKey::scope` has no `_` arm): a binding added to `app.js` and not named below
// fails this test, and the person adding it has to answer the question instead of the next switch.
//
//   GLOBAL    a switch does not touch it. The route, the dialogs, the poll, the save/restart in flight
//             (a restart is daemon-wide, A11), the first-run flow (never entered at two pairs, E14).
//   KEYED     it survives a switch because what it holds is keyed by pair (`pair\0path`), so another
//             pair's entries are never read as this one's.
//   RESET     it describes one pair's screen and is dropped by `noticeSelection()` — the function
//             `render()` calls first — BEFORE anything is drawn from it. This is the only class the
//             test checks against code, because it is the only one that is a promise about behaviour:
//             each such binding must really be cleared by that function or a reset it calls.
//   DEFERRED  pair-bound and NOT handled yet, naming the change that does. Honest rather than hidden:
//             the Settings screen still edits the config FILE's top level (`read_config` has no pair
//             argument until 5b-1), so its staged edits cannot be keyed by a pair they are not about.
//
// `node --test`, no DOM: the checks read `app.js` as text, like the release-set pin beside
// `onboarding-latch.test.js`, because the module cannot be imported (it boots on load).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as espree from "espree";
import { itemKey, statusKey } from "../src/js/screens/deletions.js";

const GLOBAL = "global";
const KEYED = "keyed";
const RESET = "reset";
const DEFERRED = "deferred";

/**
 * `[name, disposition, why]`. A name ending in `*` is a prefix. Order is reading order, not priority:
 * every binding must match exactly one row.
 */
const LEDGER = [
  // ---- the shell: a switch re-keys what the route shows, it does not move the route ----
  ["route", GLOBAL, "the root or door showing; a switch re-keys what it shows"],
  ["screenStack", GLOBAL, "body-replacing overlays"],
  ["dialogOverlay", GLOBAL, "the floating dialog"],
  ["dialogReturn", GLOBAL, "where focus returns to"],
  ["menuOpen", GLOBAL, "the ⋯ menu"],
  ["dom", GLOBAL, "cached nodes of the shell"],
  ["pollTimer", GLOBAL, "the poll's timer"],
  ["statusPolled", GLOBAL, "whether any poll has completed"],
  ["configLoaded", GLOBAL, "whether the config file has been read once"],
  ["configError", GLOBAL, "why the last read of it failed"],
  ["onboardingLatch", GLOBAL, "the first-run takeover; never armed at two pairs (E14)"],
  ["viewedPair", GLOBAL, "the memory of the switch itself"],
  ["serviceStarting", GLOBAL, "a start of the one daemon"],
  ["serviceStartError", GLOBAL, "why that failed"],
  ["openerError", GLOBAL, "the last refused open; cleared on navigation"],
  ["NOTIFIER_KEY", GLOBAL, "a constant"],
  ["LOOKUP_DEBOUNCE_MS", GLOBAL, "a constant"],
  ["PAUSE_ATTEMPTS", GLOBAL, "a constant"],
  ["PROPOSED_LOCAL", GLOBAL, "a constant of the first-run flow"],
  ["PROPOSED_REMOTE", GLOBAL, "a constant of the first-run flow"],
  ["stagedList", GLOBAL, "a function over the Settings edits (deferred below)"],
  ["activityInputRef", GLOBAL, "a DOM reference, not state about a pair"],
  ["notifyPolicy", GLOBAL, "a GUI-local setting in gui.toml, not a pair's"],
  ["notifyPolicyEdit", GLOBAL, "its staged edit"],
  ["notifyPolicyLoaded", GLOBAL, "whether it has been read"],
  ["cli*", GLOBAL, "the proton-drive CLI check; one binary for every pair"],

  // ---- the first-run flow: one pair by construction ----
  ["onboarding*", GLOBAL, "the first-folder flow; never entered at two pairs, so a switch cannot reach it"],

  // ---- the conflicts screen ----
  ["conflictIndex", RESET, "which conflict is open — an index into ONE pair's list"],
  ["conflictDiffOpen", RESET, "the disclosure under it"],
  ["conflictPair", RESET, "the two versions' bytes of that conflict"],
  ["conflictPairKey", RESET, "which conflict they belong to"],
  ["conflictPairInFlight", RESET, "a read for the old pair must not land in the new one"],
  ["conflictsSettled", RESET, "the tally of THIS visit's decisions"],
  ["conflictShowing", RESET, "the conflict the body was last built for"],
  ["conflictKeyOf", GLOBAL, "a pure function: the (pair, path) that names a conflict"],
  ["lastConflictScan", RESET, "so the first scan of the pair now shown is immediate"],

  // ---- the deletions screen ----
  ["deletionArmed", RESET, "an armed typed-DELETE field belongs to one row of one pair"],
  ["deletionBusy", RESET, "commands in flight keyed by row"],
  ["deletionStatusInFlight", RESET, "lookups in flight keyed by row"],
  ["deletionStatuses", KEYED, "`statusKey`: pair and path"],
  ["deletionsDecided", KEYED, "`itemKey`: pair, path and direction"],

  // ---- the plan screen ----
  ["planDryRun", RESET, "the plan in hand is one pair's"],
  ["planError", RESET, "its refusal"],
  ["planCheckedAt", RESET, "when it landed"],
  ["planAnswered", RESET, "which rehearsal it answers"],
  ["planPair", RESET, "the pair the plan was made for"],
  ["planSeq", RESET, "bumped, so a reply for the old pair is dropped where it lands"],
  [
    "planWaiting",
    GLOBAL,
    "the one-child-at-a-time guard (#23) spans pairs: a second rehearsal waits for the first",
  ],

  // ---- the activity screen ----
  ["activityTab", RESET, "per-visit"],
  ["activityQuery", RESET, "a path typed against one index"],
  ["activityLookup", RESET, "its answer"],
  ["activityMatches", RESET, "the several it matched"],
  ["activityChosen", RESET, "the one picked from them"],
  ["activityLookupInFlight", RESET, "cleared, so a reply from the old pair is dropped"],
  ["activityLookupTimer", RESET, "the debounce"],
  ["activityPendingShown", RESET, "the transfer dialog's latch"],
  ["activityPendingTransfer", RESET, "the transfer it describes"],
  ["skipRuleReport", RESET, "counts over one pair's folder"],
  ["skipRuleAsked", RESET, "so the walk is asked again for the new pair"],

  // ---- Settings: a save and its restart are daemon-wide (A11); the edits are the FILE's top level ----
  ["settingsTab", GLOBAL, "which tab; a switch does not move it"],
  ["settingsSaving", GLOBAL, "a save restarts the one daemon"],
  ["settingsSweeping", GLOBAL, "one sweep at a time"],
  ["settingsRestarting", GLOBAL, "the restart in flight"],
  ["settingsSaveOutcome", GLOBAL, "how the last restart ended"],
  ["settingsError", GLOBAL, "why the last save was refused"],
  [
    "settingsEdits",
    DEFERRED,
    "5b-1: staged edits become one object per pair when Settings reads `read_config(pair)`",
  ],
  ["settingsDrafts", DEFERRED, "5b-1: the half-typed skip rule belongs to the pair being edited"],
  ["settingsScheduleMonthly", DEFERRED, "5b-1: the schedule editor is per pair"],
  ["settingsNotice", DEFERRED, "5b-1: a notice about a pair's save"],
  [
    "configInfo",
    DEFERRED,
    "5b-1: `read_config` returns the file's top level; `configByPair` arrives with `read_config(pair)`",
  ],
  ["notifierState", DEFERRED, "5e: the notifier's memory becomes per pair (`kind@name`)"],
];

const app = readFileSync(fileURLToPath(new URL("../src/js/app.js", import.meta.url)), "utf8");

/**
 * Every top-level binding of `app.js`: what a `let`, `const` or `var` at the top of the module declares.
 *
 * READ FROM THE SYNTAX TREE, and that is the point of the change from the line-start regex this was:
 * `/^(?:let|const|var)\s+([A-Za-z_$][\w$]*)/gm` saw the FIRST identifier after the keyword and nothing
 * else, so `const { x } = …` (a destructuring declares `x`, which the regex read as the brace) and the
 * second name in `let a = 1, b = 2` were bindings the ledger never heard of — and an unclassified
 * binding is the whole failure this file exists to refuse. Both were measured: added to `app.js`, the
 * ledger passed 3/3. The tree answers it for every declaration form (object and array patterns, nested,
 * defaults, rest) without a second regex to keep in step with the first, and counts nothing that only
 * looks like a declaration (a comment, a template, an indented `let` in a block).
 *
 * `espree` is the parser `eslint` itself runs, so it is present wherever the lint gate is.
 */
function bindings(source) {
  const program = espree.parse(source, { ecmaVersion: 2023, sourceType: "module" });
  const names = [];
  const collect = (pattern) => {
    switch (pattern.type) {
      case "Identifier":
        names.push(pattern.name);
        break;
      case "ObjectPattern":
        for (const property of pattern.properties) collect(property.value ?? property.argument);
        break;
      case "ArrayPattern":
        for (const element of pattern.elements) if (element) collect(element);
        break;
      case "AssignmentPattern":
        collect(pattern.left);
        break;
      case "RestElement":
        collect(pattern.argument);
        break;
      default:
        assert.fail(`a ${pattern.type} binding pattern: teach bindings() to read it`);
    }
  };
  for (const node of program.body) {
    if (node.type !== "VariableDeclaration") continue;
    for (const declarator of node.declarations) collect(declarator.id);
  }
  return names;
}

/** The ledger row for a binding: an exact name beats a prefix, so a prefix can have one exception. */
function rowFor(name) {
  const exact = LEDGER.filter(([pattern]) => pattern === name);
  if (exact.length) return exact;
  return LEDGER.filter(([pattern]) => pattern.endsWith("*") && name.startsWith(pattern.slice(0, -1)));
}

/** A top-level `function <name>(…) {…}` body, by brace depth. */
function functionBody(source, name) {
  const start = source.search(new RegExp(`^(?:async )?function ${name}\\(`, "m"));
  assert.notEqual(start, -1, `app.js no longer has ${name}() — if it moved, move this test with it`);
  const open = source.indexOf("{", source.indexOf(")", start));
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === "{") depth += 1;
    if (source[i] === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(open, i + 1);
    }
  }
  assert.fail(`${name}() has no closing brace`);
  return "";
}

test("pair_ledger_names_every_top_level_binding", () => {
  const names = bindings(app);
  assert.ok(names.length > 60, `the scan found ${names.length} bindings: it is looking at the wrong thing`);

  const unclassified = names.filter((name) => rowFor(name).length === 0);
  assert.deepEqual(
    unclassified,
    [],
    "these app.js bindings are not in the pair ledger. Say what a switch of folder pair does to each " +
      "(global, keyed, reset or deferred) in gui/test/pair-ledger.test.js — a binding added without an " +
      "answer is the next 'state that outlived its subject' bug",
  );

  const ambiguous = names.filter((name) => rowFor(name).length > 1);
  assert.deepEqual(ambiguous, [], "a binding matches two ledger rows, so it has two answers");

  // And the other direction: a row that names nothing is a ledger that has rotted.
  const used = new Set(names.flatMap((name) => rowFor(name).map(([pattern]) => pattern)));
  const stale = LEDGER.map(([pattern]) => pattern).filter((pattern) => !used.has(pattern));
  assert.deepEqual(stale, [], "ledger rows that match no binding in app.js — delete them");
});

test("every_binding_the_ledger_says_is_reset_is_reset_before_anything_is_drawn", () => {
  // `noticeSelection` is the one place a switch is handled. It calls `reset…Screen()` for the screens
  // and clears the few bindings that have no reset of their own. The union of those bodies is what
  // "RESET" has to be true of.
  const notice = functionBody(app, "noticeSelection");
  const called = [...notice.matchAll(/\b(reset\w+)\(\)/g)].map((m) => m[1]);
  assert.ok(called.length >= 3, `noticeSelection calls ${called} — the screens' resets are gone`);
  const resetting = [notice, ...called.map((name) => functionBody(app, name))].join("\n");

  const resetBindings = bindings(app).filter((name) => rowFor(name)[0]?.[1] === RESET);
  assert.ok(resetBindings.length >= 25, `the ledger lost its RESET rows (${resetBindings.length})`);
  const notReset = resetBindings.filter(
    (name) => !new RegExp(`\\b${name}\\s*(?:=|\\+=)|\\b${name}\\.clear\\(`).test(resetting),
  );
  assert.deepEqual(
    notReset,
    [],
    "the ledger says these are dropped on a switch, and noticeSelection (or a reset it calls) does " +
      "not touch them — the promise in the ledger is the thing under test",
  );

  // And it runs FIRST in render(): resetting after drawing would paint the old pair's state onto the
  // new pair's screen for one frame, which is the whole bug.
  assert.match(
    app,
    /function render\(\) \{\n {2}noticeSelection\(\);/,
    "render() must notice a switch before it reads any state",
  );
});

test("the_deletion_keys_the_ledger_calls_keyed_really_carry_the_pair", () => {
  // The same path withheld in the same direction in two folders is two deletions.
  const row = { path: "notes.txt", direction: "local" };
  assert.notEqual(itemKey({ ...row, pair: "docs" }), itemKey({ ...row, pair: "photos" }));
  assert.notEqual(statusKey({ ...row, pair: "docs" }), statusKey({ ...row, pair: "photos" }));
  // …and one pair's rows are told apart by path and direction as before.
  assert.notEqual(itemKey({ ...row, pair: "docs" }), itemKey({ ...row, pair: "docs", direction: "remote" }));
  assert.equal(
    itemKey({ ...row, pair: "docs" }),
    itemKey({ ...row, pair: "docs", fingerprint: "different" }),
    "a fingerprint is what an armed gate is PINNED to, not part of the row's identity",
  );
  // The prune in `visibleDeletions` is by this prefix, so it must be how the key starts.
  assert.ok(itemKey({ ...row, pair: "docs" }).startsWith("docs\u0000"));
  assert.ok(statusKey({ ...row, pair: "docs" }).startsWith("docs\u0000"));
});

test("the_binding_scan_reads_every_declaration_form", () => {
  // The scan is what the ledger's exhaustiveness rests on, so it is checked on its own input rather
  // than only on whatever `app.js` happens to contain today. Each line is a form the line-start regex
  // it replaced got wrong (destructuring, a second declarator) or could only get right by luck.
  const found = bindings(`
const plain = 1;
const { a, b: renamed, c = 3, d: { deep }, ...rest } = source;
const [first, , third = 3, [nested], ...tail] = list;
let one = 1, two = 2, three;
var legacy, { viaVar } = source;
let
  onNextLine = 1;
function code() { const insideAFunction = 1; return insideAFunction; }
{ let insideABlock = 1; }
// const inAComment = 1;
const text = \`
const insideATemplate = 1;
\`;
`);
  assert.deepEqual(found, [
    "plain",
    "a",
    "renamed",
    "c",
    "deep",
    "rest",
    "first",
    "third",
    "nested",
    "tail",
    "one",
    "two",
    "three",
    "legacy",
    "viaVar",
    "onNextLine",
    "text",
  ]);
});
