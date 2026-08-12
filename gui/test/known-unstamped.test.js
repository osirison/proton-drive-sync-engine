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

test("no two rows claim the same node", () => {
  // The classifier keys on `frame|slot|key`, so a duplicate would sit in the map unreachable and its
  // staleness would never be reported. The KEY is in that identity because a factory slot covers a
  // run of siblings — `9a Review`'s `fact` is four nodes and could be four different reasons — so
  // `frame|slot` repeats legitimately and only the triple is unique.
  const ids = KNOWN_UNSTAMPED.map((r) => `${r.frame}|${r.slot}|${r.key}`);
  assert.equal(new Set(ids).size, ids.length);
});

test("one key of a factory slot does not vouch for the rest of it", () => {
  // The reason identity is the triple. `5a Plan safe` has five `sideRowNote` rows: record one, and
  // the other four must still be findings rather than absorbed by the shared slot name.
  const family = KNOWN_UNSTAMPED.filter((r) => r.frame === "5a Plan safe" && r.slot === "sideRowNote");
  assert.ok(family.length > 1, "this test needs a slot recorded at more than one key");
  const { recorded, stale } = classifyUnstamped([seen(family[0])]);
  assert.equal(recorded.length, 1);
  assert.equal(recorded[0].key, family[0].key);
  for (const sibling of family.slice(1)) {
    assert.ok(
      stale.some((s) => s.key === sibling.key),
      `${sibling.key} was absorbed by a sibling's row`,
    );
  }
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
  for (const s of stale) assert.equal(s.alsoUnstamped, null, "an unobserved row has nothing to point at");
});

test("the pinned key is what catches a node that moved", () => {
  // The frame still draws nothing here and the slot is still unstamped, so identity that ignored the
  // key would call this explained and stand behind a node nobody measured. Instead it fails twice —
  // the row goes stale and the new key arrives unexplained — and the stale line points at the new key
  // so the reader does not have to match the two lists up by eye.
  const [first] = KNOWN_UNSTAMPED;
  const { recorded, unexplained, stale } = classifyUnstamped([{ ...seen(first), key: "div[9]/div[9]" }]);
  assert.deepEqual(recorded, []);
  assert.equal(unexplained.length, 1, "the node it moved to is a finding in its own right");
  assert.equal(unexplained[0].key, "div[9]/div[9]");
  assert.equal(stale.length, KNOWN_UNSTAMPED.length, "the moved row, plus the rows nobody observed");
  const movedRow = stale.find((s) => s.slot === first.slot && s.frame === first.frame);
  assert.deepEqual(movedRow.alsoUnstamped, ["div[9]/div[9]"], "the report has to name the candidate");
});

test("the candidate list names every one, because picking one would be a guess", () => {
  // A run of siblings can GAIN a member in the same edit that moves one, so "the same slot is also
  // unstamped over there" can be true of several keys at once and none of them is knowably the
  // destination. Naming the first would put an arbitrary one on the report as fact.
  const family = KNOWN_UNSTAMPED.filter((r) => r.frame === "5a Plan safe" && r.slot === "sideRowNote");
  assert.ok(family.length > 1, "this test needs a slot recorded at more than one key");
  const { stale } = classifyUnstamped([
    { ...seen(family[0]), key: "div[9]/moved" },
    { ...seen(family[0]), key: "div[9]/added" },
  ]);
  const row = stale.find((s) => s.key === family[0].key);
  assert.deepEqual(row.alsoUnstamped, ["div[9]/moved", "div[9]/added"]);
});

test("a stale row only points at a moved node on its OWN frame and slot", () => {
  // The hint is a convenience and must not become a wrong claim. An unexplained finding elsewhere is
  // not where this row's node went.
  const [first] = KNOWN_UNSTAMPED;
  const { stale } = classifyUnstamped([{ frame: "4a Deletions", slot: "headline", key: "div[9]" }]);
  const row = stale.find((s) => s.frame === first.frame && s.slot === first.slot);
  assert.equal(row.alsoUnstamped, null);
});

test("one recorded slot does not vouch for its siblings", () => {
  // Rows cluster by cause — four on #98, five on #191, four on #242 — and it would be easy to write
  // a classifier that passes a whole cluster once any of it matches. The track landing without the
  // fill is a real state and has to be visible.
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
