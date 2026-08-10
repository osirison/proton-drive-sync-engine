// S5's model, and it is almost entirely UNDRAWN CODE.
//
// `7a File lookup` draws ONE of five verdicts. `6a Activity passes` draws ONE of four summary
// shapes and six of a possible fourteen counters. `7a Never synced` draws its sentence at two
// groups where the shipped screen can only ever source one. So the fidelity gate — which compares
// the app against the frames — cannot see most of what this screen does, and the copy gate cannot
// either: it compares the deck to the frames and never renders the app at all.
//
// That is the same hole S4 found and filled the same way (`plan.test.js` pins `PLAN.destructive*`,
// two templates no frame draws). This project's own record says undrawn states are where the bugs
// are — S1: 9, C1–C5: 14, S2: 5, S3: 11+4, none caught by any gate — so these are the assertions
// standing in for the frames that were never drawn.

import test from "node:test";
import assert from "node:assert/strict";

import {
  verdictOf,
  passesSummaryOf,
  neverSyncedFrom,
  footerVariantOf,
  elideId,
} from "../src/js/screens/activity.js";
import { ACTIVITY } from "../src/js/ui/copy.js";
import { cardinal } from "../src/js/ui/format.js";

// ---- the lookup's five verdicts -------------------------------------------------------------

const tracked = (over = {}) => ({
  tracked: true,
  sync_status: "synced",
  entity_kind: "file",
  file_size: 1_200_000,
  mtime: 1_700_000_000,
  proton_id: "8b3c1f2a~4c8f2e7d10b64f2ca39c5e0b8d7f9a21",
  ...over,
});

test("a synced file is the drawn verdict, and its `since` clause needs a time", () => {
  assert.equal(verdictOf(tracked(), "14:32").title, ACTIVITY.lookup.safe);
  assert.equal(verdictOf(tracked(), "14:32").sub, ACTIVITY.lookup.safeSub("14:32"));
  assert.equal(verdictOf(tracked(), "14:32").mark, "settled");
  // No time, no claim about when. The sentence is `Identical … since X`; without an X there is
  // nothing true to put in it, so the clause goes rather than rendering an em-dash inside prose.
  assert.equal(verdictOf(tracked(), null).sub, null);
  assert.equal(verdictOf(tracked(), null).title, ACTIVITY.lookup.safe);
});

test("a modified file says the copy here is newer, and does not wear the settled mark", () => {
  const v = verdictOf(tracked({ sync_status: "modified" }), "14:32");
  assert.equal(v.title, ACTIVITY.lookup.changed);
  assert.equal(v.sub, ACTIVITY.lookup.changedSub);
  assert.notEqual(v.mark, "settled");
});

test("a conflicted file says both sides changed and nothing is lost", () => {
  const v = verdictOf(tracked({ sync_status: "conflict" }), "14:32");
  assert.equal(v.title, ACTIVITY.lookup.conflict);
  assert.match(v.sub, /Nothing is lost/);
  assert.notEqual(v.mark, "settled");
});

test("a path the index has never seen is a miss, with no mark at all", () => {
  for (const status of [null, undefined, { tracked: false }, { tracked: false, sync_status: null }]) {
    const v = verdictOf(status, "14:32");
    assert.equal(v.title, ACTIVITY.lookup.noMatch, `for ${JSON.stringify(status)}`);
    // A miss is not a state a file is IN, so it gets no hexagon. Drawing the settled mark over
    // "no file by that name" would put the safe mark on a file that does not exist.
    assert.equal(v.mark, null);
  }
});

test("a directory is answered as a folder rather than as a file that is fine", () => {
  const v = verdictOf(tracked({ entity_kind: "directory" }), "14:32");
  assert.equal(v.title, ACTIVITY.lookup.folder);
  assert.notEqual(v.mark, "settled");
});

// THE ARM NOBODY DESIGNED FOR. `path_sync_status` documents three values for `sync_status` and the
// engine could grow a fourth; the dangerous default is the reassuring one, so this pins that an
// unrecognised status never comes back as "safely on both sides".
test("an unrecognised sync_status fails closed — never the settled mark, never the safe words", () => {
  for (const unknown of ["", "pending", "PENDING", "synced ", null, undefined, 7]) {
    const v = verdictOf(tracked({ sync_status: unknown }), "14:32");
    assert.notEqual(v.mark, "settled", `mark for ${JSON.stringify(unknown)}`);
    assert.notEqual(v.title, ACTIVITY.lookup.safe, `title for ${JSON.stringify(unknown)}`);
  }
});

// ---- the passes summary ----------------------------------------------------------------------

const clean = (secs) => ({ epoch_secs: secs, last_error: null });
const failed = (secs) => ({ epoch_secs: secs, last_error: "proton-drive: connection timed out after 60s" });

test("the drawn summary renders from the drawn numbers", () => {
  // OLDEST FIRST, as the wire sends it — `daemon.rs` pushes each pass and drains the front.
  const entries = [...Array(19)].map((_, i) => clean(i)).concat([clean(19)]);
  entries[8] = failed(8);
  assert.equal(passesSummaryOf(entries), ACTIVITY.passes.summary(19, 20, 1, true));
});

test("no history at all has no sentence — the caller omits the line rather than claim zero passes", () => {
  assert.equal(passesSummaryOf([]), null);
});

test("every pass clean says so without the of-the-last construction", () => {
  const s = passesSummaryOf([clean(1), clean(2), clean(3)]);
  assert.equal(s, "All 3 recent passes finished cleanly.");
  assert.doesNotMatch(s, /failed/);
});

test("one clean pass is singular", () => {
  assert.equal(passesSummaryOf([clean(1)]), "All 1 recent pass finished cleanly.");
});

// `recovered` is the whole reason the summary takes four arguments: "retried on its own" is a claim
// about the ORDER of the history, not about any one entry, and it is false in precisely the state
// where a user most needs the truth.
test("a failure that is still the newest pass is NOT described as retried", () => {
  const s = passesSummaryOf([clean(1), clean(2), failed(3)]);
  assert.match(s, /The most recent one failed\./);
  assert.doesNotMatch(s, /retried/);
});

test("a failure with a later success is described as retried", () => {
  const s = passesSummaryOf([clean(1), failed(2), clean(3)]);
  assert.match(s, /retried on its own/);
  assert.doesNotMatch(s, /The most recent one failed/);
});

test("more than one failure agrees in number", () => {
  const s = passesSummaryOf([failed(1), failed(2), clean(3)]);
  assert.match(s, /Two failed and retried on their own\./);
});

// ---- the never-synced band ---------------------------------------------------------------------

const rule = (over = {}) => ({
  pattern: "*.tmp",
  files: 2,
  bytes: 2_940_000,
  unique_files: 2,
  unique_bytes: 2_940_000,
  samples: [],
  ...over,
});

test("no report and no matching rule both mean no band", () => {
  assert.equal(neverSyncedFrom(null), null);
  assert.equal(neverSyncedFrom({ rules: [] }), null);
  // A rule that matched nothing is not a reason to tell someone files are never synced.
  assert.equal(neverSyncedFrom({ rules: [rule({ unique_files: 0 })] }), null);
});

test("the count is UNIQUE files, so a path two rules match is one file", () => {
  const report = { rules: [rule(), rule({ pattern: "*.bak", unique_files: 1, files: 3 })] };
  assert.equal(neverSyncedFrom(report).total, 3);
  assert.equal(neverSyncedFrom(report).rules.length, 2);
});

// The `Can't be synced` group has no Phase-1 source (#232), so the live sentence always renders at
// `cannot: 0` — and `cardinal(0)` is `zero`, which would read as a sentence about a group nobody
// measured.
test("with no can't-be-synced group the sentence stops after the first clause", () => {
  const s = ACTIVITY.neverSyncedSub(2, 0);
  assert.equal(s, "They sit in your folder but aren't copied anywhere. Two match a rule you wrote.");
  assert.doesNotMatch(s, /zero/i);
  assert.doesNotMatch(s, /can't be synced at all/);
});

test("both groups present renders the drawn sentence, with the second clause lower-cased", () => {
  assert.equal(
    ACTIVITY.neverSyncedSub(2, 2),
    "They sit in your folder but aren't copied anywhere. Two match a rule you wrote; two can't be synced at all.",
  );
});

test("one file matching a rule agrees in number", () => {
  assert.match(ACTIVITY.neverSyncedSub(1, 0), /One matches a rule you wrote\./);
});

test("cardinal's mid register lower-cases the spelled forms and leaves digits alone", () => {
  assert.equal(cardinal(2), "Two");
  assert.equal(cardinal(2, "mid"), "two");
  assert.equal(cardinal(11, "mid"), "11");
});

// ---- the four templates that were ungrammatical at one --------------------------------------
//
// Every drawn instance of each of these is plural, so they were written with the plural baked in
// and rendered `1 files are never synced` the first time a live count reached them. All four states
// are reachable: one excluded file, one file moved, one day.

test("the counted sentences agree in number at one", () => {
  assert.equal(ACTIVITY.neverSyncedTitle(1), "1 file is never synced");
  assert.equal(ACTIVITY.neverSyncedTitle(4), "4 files are never synced");
  assert.equal(ACTIVITY.neverSyncedDialog.title(1), "1 file is never synced");
  assert.equal(ACTIVITY.lastToMoveSub(1, 1), "1 file in the last 1 day");
  assert.equal(ACTIVITY.lastToMoveSub(7, 3), "7 files in the last 3 days");
  assert.equal(ACTIVITY.allFiles(1), "All 1 file");
  assert.equal(ACTIVITY.allFiles(7), "All 7 files");
});

// ---- the rest of the model --------------------------------------------------------------------

test("the footer variant is per TAB, not per route", () => {
  assert.equal(footerVariantOf({ tab: "files" }), "tight");
  assert.equal(footerVariantOf({ tab: "passes" }), "standard");
  // The files tab is the default, including when the screen has not said which tab it is on.
  assert.equal(footerVariantOf({}), "tight");
  assert.equal(footerVariantOf(null), "tight");
});

test("the id elide takes the NODE half, because the volume half never changes", () => {
  assert.equal(elideId("8b3c1f2a~4c8f2e7d10b64f2ca39c5e0b8d7f9a21"), "4c8f…9a21");
  // Short enough to show whole, and a bare node id with no volume prefix.
  assert.equal(elideId("8b3c1f2a~abc123"), "abc123");
  assert.equal(elideId("4c8f2e7d10b64f2ca39c5e0b8d7f9a21"), "4c8f…9a21");
});
