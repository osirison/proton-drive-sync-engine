// The unclaimed-census classifier (#250). Its sibling next door decides whether a block that
// renders NOTHING is recorded; this one decides whether a node the frame DRAWS and no slot names is
// recorded. Same failure mode, mirrored: being slightly too generous switches the gate off.
//
// WHY THIS FILE EXISTS AT ALL. `classifyUnclaimed`'s four schema arms shipped verified only by six
// hand-run poisons — a browser run each, sixty seconds each, run by whoever remembered. That is not
// a gate; it is a ritual. Three of the four arms exist BECAUSE the first version of the classifier
// let a fault through silently, so the arms themselves are the regression, and a regression nothing
// runs in CI is one refactor from being gone.
//
// It drives the classifier directly rather than through `assert.mjs`, which needs puppeteer and the
// whole 51-frame render. The observation shape is the only coupling, and it is one object literal.

import { test } from "node:test";
import assert from "node:assert/strict";
import { classifyUnclaimed, KNOWN_UNCLAIMED } from "../tools/fidelity/known-deviations.mjs";

/** The observation shape `assert.mjs` pushes: one drawn node no slot claims. */
const seen = (frame, key) => ({ frame, key, tag: "div" });
/** Every key the real list declares, as if the gate had observed all of them. */
const allObserved = () => KNOWN_UNCLAIMED.flatMap((r) => r.keys.map((k) => seen(r.frame, k)));

// The five classes are a closed set in the classifier; repeated here so a silent widening of
// `UNCLAIMED_CLASSES` (which is not exported) fails rather than passing by construction.
const CLASSES = ["scenery", "unmappable", "decision", "issue", "mapping"];

test("the real list is clean, and every key it declares is recorded", () => {
  const { recorded, unexplained, stale, malformed } = classifyUnclaimed(allObserved());
  assert.deepEqual(malformed, [], "KNOWN_UNCLAIMED has a schema fault");
  assert.deepEqual(unexplained, [], "an observed key nothing declares");
  assert.deepEqual(stale, [], "a declared key nothing observed");
  assert.equal(
    recorded.length,
    KNOWN_UNCLAIMED.reduce((n, r) => n + r.keys.length, 0),
  );
});

test("every entry is a complete entry", () => {
  for (const row of KNOWN_UNCLAIMED) {
    assert.ok(typeof row.frame === "string" && row.frame.length > 0);
    assert.ok(CLASSES.includes(row.class), `${row.frame}: class "${row.class}"`);
    assert.ok(typeof row.why === "string" && row.why.length > 0, `${row.frame} has no why`);
    assert.ok(Array.isArray(row.keys) && row.keys.length > 0, `${row.frame} has no keys`);
    // MEMBERSHIP IS PINNED, NEVER PREFIXED. A key ending in `/` or naming no index would excuse
    // whatever a re-extracted frame added beneath it — a suppression that widens itself.
    for (const key of row.keys) {
      assert.ok(typeof key === "string" && key.length > 0, `${row.frame} has an empty key`);
      assert.ok(!key.endsWith("/"), `${row.frame}: ${key} reads as a prefix`);
    }
    if (row.class === "issue" || row.class === "mapping") {
      assert.match(row.issue ?? "", /^#\d+$/, `${row.frame} (${row.class}) must name an issue`);
    }
  }
});

// The four faults below all passed the FIRST version of this classifier, which is why each is a
// test rather than a comment. Each poisons a copy of one real entry, so the fixture cannot drift
// from the shape the gate actually reads.
const poison = (mutate) => {
  const row = { ...KNOWN_UNCLAIMED[0], keys: [...KNOWN_UNCLAIMED[0].keys] };
  mutate(row);
  return row;
};
/** Runs the classifier over ONE entry, by observing exactly the keys it declares. */
const classifyOne = (row) => {
  const saved = KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, row);
  try {
    return classifyUnclaimed(row.keys.map((k) => seen(row.frame, k)));
  } finally {
    KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, ...saved);
  }
};

test("an unknown class is one fault, and is not also told it needs an issue", () => {
  const { malformed } = classifyOne(poison((r) => (r.class = "sceneryy")));
  assert.equal(malformed.length, 1, "an unknown class must produce exactly one fault");
  assert.match(malformed[0].text, /class "sceneryy" is not one of/);
});

test("a key repeated inside one entry is a duplicate, not a collision", () => {
  const { malformed } = classifyOne(poison((r) => r.keys.push(r.keys[0])));
  assert.equal(malformed.length, 1, "one repeated key must produce exactly one fault");
  assert.match(malformed[0].text, /appears twice in one entry/);
});

test("a mapping or issue entry with no issue number is malformed", () => {
  for (const cls of ["mapping", "issue"]) {
    const { malformed } = classifyOne(
      poison((r) => {
        r.class = cls;
        delete r.issue;
      }),
    );
    assert.equal(malformed.length, 1, `${cls} with no issue`);
    assert.match(malformed[0].text, /needs an issue number/);
  }
});

test("two entries claiming one key is a fault, and neither silently shadows the other", () => {
  // The worst of the four: a `decision` entry could fully shadow a `mapping` one, and neither would
  // go stale, so the census would report a wrong reason for ever.
  const a = { frame: "F", class: "decision", why: "w", keys: ["div[0]"] };
  const b = { frame: "F", class: "mapping", issue: "#1", why: "w", keys: ["div[0]"] };
  const saved = KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, a, b);
  try {
    const { malformed } = classifyUnclaimed([seen("F", "div[0]")]);
    assert.equal(malformed.length, 1);
    assert.match(malformed[0].text, /claimed by two entries \(decision and mapping\)/);
  } finally {
    KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, ...saved);
  }
});

test("faults are counted by ENTRY INDEX, not by frame label", () => {
  // Six frames hold two entries each, so counting distinct labels reports two broken entries as
  // one. `assert.mjs` takes `new Set(malformed.map((m) => m.entry)).size`, which this pins.
  const a = { frame: "F", class: "nope", why: "w", keys: ["div[0]"] };
  const b = { frame: "F", class: "alsonope", why: "w", keys: ["div[1]"] };
  const saved = KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, a, b);
  try {
    const { malformed } = classifyUnclaimed([]);
    assert.equal(malformed.length, 2, "two entries, two faults");
    assert.equal(new Set(malformed.map((m) => m.entry)).size, 2, "and TWO entries, not one");
  } finally {
    KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, ...saved);
  }
});

test("an observed key nothing declares is unexplained, and a declared key nothing draws is stale", () => {
  // The census's two directions. Both are what `assert.mjs` exits 1 on.
  const row = { frame: "F", class: "scenery", why: "w", keys: ["div[0]"] };
  const saved = KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, row);
  try {
    const gone = classifyUnclaimed([]);
    assert.equal(gone.stale.length, 1);
    assert.deepEqual(gone.stale[0].gone, ["div[0]"]);
    const extra = classifyUnclaimed([seen("F", "div[0]"), seen("F", "div[9]")]);
    assert.deepEqual(extra.unexplained, [seen("F", "div[9]")]);
    assert.equal(extra.stale.length, 0);
  } finally {
    KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, ...saved);
  }
});

test("a frame is part of a key's identity, so one frame's entry never vouches for another's", () => {
  // The same rule KNOWN_UNSTAMPED has, and the reason S10 found six missing light-theme rows: a
  // dark frame's declaration must not cover its light twin.
  const row = { frame: "2a Settled", class: "scenery", why: "w", keys: ["div[0]"] };
  const saved = KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, row);
  try {
    const { unexplained } = classifyUnclaimed([seen("12a Settled light", "div[0]")]);
    assert.equal(unexplained.length, 1, "the light twin must not be covered by the dark row");
  } finally {
    KNOWN_UNCLAIMED.splice(0, KNOWN_UNCLAIMED.length, ...saved);
  }
});
