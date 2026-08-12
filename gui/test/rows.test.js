// Two facts out of ui/rows.js that a passing style gate would not defend, tested on their own
// because both are pure and both fail quietly.
//
// This frontend is tested selectively, the way onboarding-latch.test.js explains — most of it is
// better checked by the fidelity gate than by assertions. These two earn it:
//
//   · The transfer row's slot order is stated BACKWARDS in `03-main-screen.md` and again in issue
//     #169. Anyone who reads either document and "corrects" the code produces a screen that still
//     looks reasonable, and the style gate only catches it once S1 maps `2a Syncing` to a fixture.
//     Until then this test is the only thing between the frames and the prose.
//   · `splitEmphasis` falls back to the whole sentence. The copy deck owns those sentences and the
//     emphasis is a substring of one, so a `copy.js` edit can stop matching at any time. Losing a
//     bold span is fine; losing the sentence would fail the copy gate far from the cause.

import { test } from "node:test";
import assert from "node:assert/strict";
import { splitEmphasis, transferSlotOrder } from "../src/js/ui/rows.js";

test("the arrow sits beside the seam, which is what the frames draw", () => {
  // Leaving: the arrow is LAST, so it lands next to the centre line and points across it.
  assert.deepEqual(transferSlotOrder("up"), ["name", "detail", "arrow"]);
  // Arriving: the arrow is FIRST, for the same reason on the other side.
  assert.deepEqual(transferSlotOrder("down"), ["arrow", "name", "detail"]);
});

test("it is a rotation, not a mirror — the name reads before the size on both sides", () => {
  const up = transferSlotOrder("up");
  const down = transferSlotOrder("down");
  assert.ok(up.indexOf("name") < up.indexOf("detail"));
  assert.ok(down.indexOf("name") < down.indexOf("detail"), "a true mirror would put the size first");
});

test("an unknown direction throws rather than guessing a side", () => {
  assert.throws(() => transferSlotOrder("left"), /must be "up" or "down"/);
  assert.throws(() => transferSlotOrder(undefined), /must be "up" or "down"/);
});

test("splitEmphasis splits around the substring", () => {
  assert.deepEqual(splitEmphasis("Deleting this removes 8.4 GB from disk.", "8.4 GB"), [
    "Deleting this removes ",
    "8.4 GB",
    " from disk.",
  ]);
});

test("a substring the copy no longer contains keeps the whole sentence", () => {
  const sentence = "Deleting this removes 8.4 GB from disk.";
  assert.deepEqual(splitEmphasis(sentence, "1,204 photos"), [sentence]);
  assert.deepEqual(splitEmphasis(sentence, null), [sentence]);
  assert.deepEqual(splitEmphasis(sentence, ""), [sentence]);
});

test("the whole sentence survives the split", () => {
  const sentence = "Deleting it on Proton moves it to Proton Drive's Trash, where you can get it back.";
  for (const substring of ["Proton Drive's Trash", "nothing that appears", null]) {
    assert.equal(splitEmphasis(sentence, substring).join(""), sentence);
  }
});

test("emphasis at the very start and the very end still round-trips", () => {
  assert.deepEqual(splitEmphasis("8.4 GB goes", "8.4 GB"), ["", "8.4 GB", " goes"]);
  assert.deepEqual(splitEmphasis("it goes to Trash", "Trash"), ["it goes to ", "Trash", ""]);
});
