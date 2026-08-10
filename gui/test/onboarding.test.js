// S7's model. Everything here is a state the fidelity gate cannot see: the five `9a` frames are one
// datum each, and the flow is a sequence.
//
// The two that matter most are safety properties rather than layout ones. `Nothing will be deleted ·
// on either side` is the sentence `09-onboarding.md` calls the whole point of step 2, and it is
// asserted against a plan that WOULD delete something — a state no frame draws. And the fact rows
// are keyed by the row they stand for rather than by their position, because the app draws three of
// the frame's four: an app-order index compares a ringed dot against a filled one and the gate calls
// it a colour bug.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  bodyOf,
  actionsThatHappen,
  pairReady,
  factRows,
  mergeFooterText,
  mergeOutcomeOf,
  onboardingBarShape,
} from "../src/js/screens/onboarding.js";
import { ONBOARDING } from "../src/js/ui/copy.js";
import { bytes } from "../src/js/ui/format.js";

const summary = (over = {}) => ({
  total: 474,
  uploads: 128,
  downloads: 341,
  conflicts: 2,
  skipped_unsupported: 3,
  destructive_actions: 0,
  ...over,
});

const report = (over = {}) => ({ report: { summary: summary(over), plan: [] } });

// ---- which body ------------------------------------------------------------------------------

test("step 1 draws step 1, whatever the rehearsal is doing", () => {
  // The pair can be re-chosen with a plan already in hand — `Back` moves the token, not the step.
  assert.equal(bodyOf({ step: "folders", dryRun: report() }), "folders");
  assert.equal(bodyOf({ step: "folders", checking: true }), "folders");
  assert.equal(bodyOf({}), "folders");
});

test("step 2 works it out before it says anything, and says the daemon's words when it fails", () => {
  assert.equal(bodyOf({ step: "review" }), "checking");
  assert.equal(bodyOf({ step: "review", dryRun: report(), checking: true }), "checking");
  assert.equal(bodyOf({ step: "review", dryRun: report() }), "review");
  assert.equal(bodyOf({ step: "review", error: "no such file" }), "failed");
  // A failure outranks a stale plan: the plan on screen would be one the config no longer describes.
  assert.equal(bodyOf({ step: "review", dryRun: report(), error: "no such file" }), "failed");
});

// ---- the counts ------------------------------------------------------------------------------

test("`See all N actions` names what will happen, not what the plan holds", () => {
  // `SkipUnsupported` IS a plan row, so `total` is 474 and 471 is what is left after them.
  assert.equal(actionsThatHappen(summary()), 471);
  assert.equal(actionsThatHappen(summary({ skipped_unsupported: 0 })), 474);
  assert.equal(actionsThatHappen(null), null);
  // Never negative, whatever a malformed summary claims.
  assert.equal(actionsThatHappen({ total: 1, skipped_unsupported: 9 }), 0);
});

test("a pair with an empty side is not a pair", () => {
  assert.equal(pairReady({ local: "~/ProtonDrive", remote: "/Drive/RemoteFolder" }), true);
  assert.equal(pairReady({ local: "", remote: "/Drive/RemoteFolder" }), false);
  assert.equal(pairReady({ local: "~/ProtonDrive", remote: "   " }), false);
  assert.equal(pairReady({}), false);
  assert.equal(pairReady(), false);
});

test("the footer bar's shape moves when the pair arms it, so the button is repainted", () => {
  const empty = { step: "folders", local: "~/ProtonDrive", remote: "" };
  assert.notEqual(onboardingBarShape(empty), onboardingBarShape({ ...empty, remote: "/Drive" }));
});

// ---- the fact rows ---------------------------------------------------------------------------

test("the fact rows are keyed by the drawn row they stand for", () => {
  // Row 0 (`11,798 files already match`) is #242 and never drawn, so the app's first row is the
  // frame's second. The index is what the gate compares against, not the position in this list.
  assert.deepEqual(
    factRows(summary()).map((r) => r.at),
    [1, 2, 3],
  );
});

test("the already-matching row has no source and is never invented", () => {
  const labels = factRows(summary()).map((r) => r.label);
  assert.ok(!labels.some((l) => l.includes("already match")));
});

test("`Nothing will be deleted` is only said of a plan that deletes nothing", () => {
  const rows = factRows(summary({ destructive_actions: 1 }));
  assert.ok(!rows.some((r) => r.label === ONBOARDING.nothingDeleted));
  // …and it IS said when the plan is clean, which is the whole point of the step.
  assert.ok(factRows(summary()).some((r) => r.label === ONBOARDING.nothingDeleted));
});

test("a count that reads zero gets no row", () => {
  const rows = factRows(summary({ conflicts: 0, skipped_unsupported: 0 }));
  assert.deepEqual(
    rows.map((r) => r.at),
    [3],
  );
});

test("no summary draws no facts at all — an empty strip, never a strip of zeroes", () => {
  assert.deepEqual(factRows(null), []);
});

test("the unsyncable row counts and does not name the kinds", () => {
  // The drawn sentence says `a socket and two shortcuts` and nothing enumerates them (#232).
  const row = factRows(summary()).find((r) => r.at === 2);
  assert.equal(row.label, "3 files can't be synced");
  assert.equal(row.note, ONBOARDING.skipped);
});

// ---- the merge footer ------------------------------------------------------------------------

test("the merge footer speaks only of the plan that was approved", () => {
  assert.equal(mergeFooterText({ summary: summary() }), "nothing deleted · 2 conflicts kept as copies");
  assert.equal(mergeFooterText({ summary: summary({ conflicts: 0 }) }), "nothing deleted");
  assert.equal(
    mergeFooterText({ summary: summary({ conflicts: 1 }) }),
    "nothing deleted · 1 conflict kept as copies",
  );
  // A plan that would delete something cannot claim the first clause.
  assert.equal(
    mergeFooterText({ summary: summary({ destructive_actions: 2 }) }),
    "2 conflicts kept as copies",
  );
  // Nothing in hand says nothing, and the caller omits the node rather than rendering "".
  assert.equal(mergeFooterText({}), "");
});

// ---- the copy the flow adds ------------------------------------------------------------------

test("the consent sub-line drops its clause rather than counting to zero", () => {
  assert.equal(ONBOARDING.doneSubPhase1(0), "Nothing was deleted.");
  assert.ok(ONBOARDING.doneSubPhase1(1).includes("1 file is waiting"));
  assert.ok(ONBOARDING.doneSubPhase1(2).includes("2 files are waiting"));
});

test("a whole number of gigabytes is not written with a trailing .0", () => {
  // `9a Folders` draws `500 GB` and `9a Review` draws `214 GB`; the one-decimal rule wrote `214.0`.
  assert.equal(bytes(214_000_000_000), "214 GB");
  assert.equal(bytes(500_000_000_000), "500 GB");
  assert.equal(bytes(41_200_000_000), "41.2 GB");
  assert.equal(bytes(96_000), "96 KB");
});

test("the free-space line states what C4 answers and claims nothing about what it needs", () => {
  assert.equal(ONBOARDING.freeSpaceHave(214_000_000_000), "You have 214 GB.");
  assert.ok(!ONBOARDING.freeSpaceHave(1).includes("Needs"));
});

// ---- has the merge finished, and did it work -------------------------------------------------

const reply = (over = {}) => ({
  syncing: false,
  paused: false,
  reconcile_seq: 1,
  last_sync_epoch_secs: 1_754_000_000,
  last_error: null,
  ...over,
});

test("a merge that has not answered, or is still running, is not a merge that finished", () => {
  assert.equal(mergeOutcomeOf(null, 0), "waiting");
  assert.equal(mergeOutcomeOf(reply({ syncing: true }), 0), "waiting");
  // The pass counter has not moved past where it stood when the merge started.
  assert.equal(mergeOutcomeOf(reply({ reconcile_seq: 4 }), 4), "waiting");
  assert.equal(mergeOutcomeOf(reply({ reconcile_seq: 3 }), 4), "waiting");
});

test("a COMPLETED pass is not a SUCCESSFUL one", () => {
  // `reconcile_blocking` bumps the counter either way and records the reason. Without the
  // `last_error` arm the consent dialog opens over a merge that did nothing, saying `Both sides now
  // match` and `Nothing was deleted`.
  assert.equal(mergeOutcomeOf(reply({ reconcile_seq: 5 }), 4), "done");
  assert.equal(
    mergeOutcomeOf(reply({ reconcile_seq: 5, last_error: "proton-drive: not logged in" }), 4),
    "failed",
  );
});

test("with no counter to compare against, a recorded sync is this one", () => {
  // The daemon had never answered when `Start the first sync` was pressed, so there is no seq to
  // beat and `last_sync_epoch_secs` is the only evidence a pass ran at all.
  assert.equal(mergeOutcomeOf(reply({ last_sync_epoch_secs: null }), null), "waiting");
  assert.equal(mergeOutcomeOf(reply(), null), "done");
  assert.equal(mergeOutcomeOf(reply({ last_error: "boom" }), null), "failed");
});
