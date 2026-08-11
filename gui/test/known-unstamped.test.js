// The unstamped-slot gate's classifier. It decides whether a block that renders nothing is a
// recorded Phase-1 omission or a failure, so every branch below is a place where being slightly too
// generous switches the gate off — which is what it spent its first life as, a printout nobody had
// to act on.
//
// `classifyUnstamped` is pure and takes its observations as an argument precisely so this file can
// drive it without depending on what an earlier test happened to call. `isKnown` next door cannot
// be tested this way, and that asymmetry is deliberate.

import { test } from "node:test";
import assert from "node:assert/strict";
import { classifyUnstamped, KNOWN_UNSTAMPED } from "../tools/fidelity/known-deviations.mjs";

/** The observation shape `assert.mjs` pushes: one drawn-but-unstamped slot. */
const seen = (row) => ({ frame: row.frame, slot: row.slot, key: row.key });
const all = () => KNOWN_UNSTAMPED.map(seen);

test("every row is a complete row", () => {
  // A row with no issue is an excuse rather than a record, and the whole mechanism rests on the
  // difference. Same bar as KNOWN_DEVIATIONS.
  for (const row of KNOWN_UNSTAMPED) {
    for (const field of ["frame", "slot", "key", "issue", "why"]) {
      assert.equal(typeof row[field], "string", `${row.frame} · ${row.slot} has no ${field}`);
      assert.ok(row[field].length > 0, `${row.frame} · ${row.slot} has an empty ${field}`);
    }
    assert.match(row.issue, /^#\d+$/, `${row.frame} · ${row.slot} must name an issue`);
  }
});

test("no two rows claim the same slot", () => {
  // The classifier keys on `frame|slot`, so a duplicate would sit in the map unreachable and its
  // staleness would never be reported.
  const ids = KNOWN_UNSTAMPED.map((r) => `${r.frame}|${r.slot}`);
  assert.equal(new Set(ids).size, ids.length);
});

test("a recorded slot is explained, not a failure", () => {
  const { recorded, unexplained, stale } = classifyUnstamped(all());
  assert.equal(recorded.length, KNOWN_UNSTAMPED.length);
  assert.deepEqual(unexplained, []);
  assert.deepEqual(stale, []);
  // The report prints the issue beside the slot, so it has to survive the classification.
  for (const r of recorded) assert.match(r.issue, /^#\d+$/);
});

test("A SLOT NOBODY RECORDED IS A FAILURE", () => {
  // The gate's whole point. S5's never-synced dialog rendered an empty body and every gate passed;
  // this is the branch that would have caught it.
  const { recorded, unexplained } = classifyUnstamped([
    { frame: "2a Settled", slot: "sub", key: "div[0]/div[2]" },
  ]);
  assert.equal(unexplained.length, 1);
  assert.equal(unexplained[0].slot, "sub");
  assert.deepEqual(recorded, []);
});

test("a row nobody observed is stale, and says which kind", () => {
  // The capability landed and the app now stamps the slot, or the frame stopped being mapped. Both
  // want the row deleted, and neither is something to infer — so it fails.
  const { stale, recorded } = classifyUnstamped([]);
  assert.equal(stale.length, KNOWN_UNSTAMPED.length);
  assert.equal(recorded.length, 0);
  for (const s of stale) assert.equal(s.was, null, "an unobserved row has no observed key");
});

test("the pinned key is what catches a node that moved", () => {
  // The frame still draws nothing here and the slot is still unstamped, so a match on `frame|slot`
  // alone would call this explained. It is not: the mapping now names a different node, and whether
  // the recorded reason still applies to THAT node is a human's call.
  const [first] = KNOWN_UNSTAMPED;
  const moved = [{ ...seen(first), key: "div[9]/div[9]" }];
  const { recorded, unexplained, stale } = classifyUnstamped(moved);
  assert.deepEqual(recorded, []);
  assert.deepEqual(unexplained, [], "a moved node is stale, not unexplained — one finding, not two");
  assert.equal(stale.length, KNOWN_UNSTAMPED.length, "the moved row, plus the rows nobody observed");
  const movedRow = stale.find((s) => s.slot === first.slot && s.frame === first.frame);
  assert.equal(movedRow.was, "div[9]/div[9]", "the report has to name where it went");
});

test("one recorded slot does not vouch for its siblings", () => {
  // Four rows, one cause (#98), and it would be easy to write a classifier that passes the lot once
  // any of them matches. The track landing without the fill is a real state and has to be visible.
  const [first, ...rest] = all();
  const { recorded, stale } = classifyUnstamped([first]);
  assert.equal(recorded.length, 1);
  assert.equal(stale.length, rest.length);
});

test("an unexplained slot is reported even when every recorded one is present", () => {
  // The two lists are independent: a green recorded set must not mask a new finding, which is the
  // failure mode that made the original report useless.
  const { recorded, unexplained, stale } = classifyUnstamped([
    ...all(),
    { frame: "4a Deletions", slot: "headline", key: "div[0]/div[0]" },
  ]);
  assert.equal(recorded.length, KNOWN_UNSTAMPED.length);
  assert.equal(unexplained.length, 1);
  assert.deepEqual(stale, []);
});
