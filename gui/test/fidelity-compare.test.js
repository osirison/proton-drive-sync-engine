// The comparator behind the style gate. The F8 issue's acceptance criterion is "a deliberately-wrong
// hex must fail the build", and a gate nobody has watched fail is a gate nobody should trust — every
// tolerance below is a place where being slightly too generous silently switches the gate off.

import { test } from "node:test";
import assert from "node:assert/strict";
import { compare, valueOf, INITIAL } from "../tools/fidelity/props.mjs";

test("A DELIBERATELY WRONG HEX FAILS", () => {
  // The acceptance criterion, spelled out. Both sides come out of getComputedStyle, so both are
  // already rgb() — no parsing, no rounding, no room to disagree.
  assert.equal(compare("color", "rgb(242, 244, 247)", "rgb(242, 244, 247)"), null);
  assert.ok(compare("color", "rgb(242, 244, 247)", "rgb(242, 244, 248)"), "one unit off must fail");
  assert.ok(compare("background-color", "rgb(10, 11, 13)", "rgb(11, 11, 13)"));
});

test("alpha is part of the colour", () => {
  assert.ok(compare("background-color", "rgba(255, 107, 107, 0.35)", "rgba(255, 107, 107, 0.32)"));
});

test("lengths agree within half a pixel and not beyond", () => {
  assert.equal(compare("padding-top", "14px", "14px"), null);
  assert.equal(compare("padding-top", "14px", "14.4px"), null, "0.4px is inside the tolerance");
  assert.equal(compare("padding-top", "14px", "13.5px"), null, "exactly 0.5px is inside");
  assert.ok(compare("padding-top", "14px", "14.6px"), "0.6px is outside");
  assert.ok(compare("padding-top", "14px", "16px"), "2px is a real difference");
});

test("font-family compares on the first name only", () => {
  // The fallback stack is a deployment detail and the two documents' stacks differ by design.
  assert.equal(
    compare("font-family", '"Instrument Sans", system-ui, sans-serif', '"Instrument Sans", sans-serif'),
    null,
  );
  assert.ok(compare("font-family", '"Instrument Sans", sans-serif', '"IBM Plex Mono", monospace'));
});

test("line-height:normal is a wildcard in either direction", () => {
  // Resolving `normal` compares font metrics rather than design — noisy on day one, and a noisy
  // gate gets ignored. Recorded in props.mjs.
  assert.equal(compare("line-height", "normal", "22.4px"), null);
  assert.equal(compare("line-height", "19px", "normal"), null);
  // But two real values still have to agree.
  assert.ok(compare("line-height", "19px", "24px"));
});

test("a wildcard does not leak to other properties", () => {
  assert.ok(compare("font-size", "normal", "13px"), "only line-height is wildcarded");
});

test("an omitted property reads back as its CSS initial value", () => {
  // The fixtures drop properties at their initial to stay diffable; `valueOf` is what makes that
  // lossless. If these disagreed, every dropped property would silently stop being asserted.
  assert.equal(valueOf({}, "margin-top"), "0px");
  assert.equal(valueOf({}, "display"), "block");
  assert.equal(valueOf({}, "background-color"), "rgba(0, 0, 0, 0)");
  assert.equal(valueOf({ "margin-top": "26px" }, "margin-top"), "26px");
});

test("every omittable property has an initial, and it round-trips", () => {
  for (const [prop, initial] of Object.entries(INITIAL)) {
    assert.equal(valueOf({}, prop), initial, `${prop} has no usable initial`);
    assert.equal(
      compare(prop, valueOf({}, prop), initial),
      null,
      `${prop} does not compare equal to its own initial`,
    );
  }
});

test("inherited properties are NOT omittable", () => {
  // Their computed value depends on an ancestor, so "absent" could not be resolved to anything.
  for (const prop of ["color", "font-size", "font-family", "font-weight", "letter-spacing", "text-align"]) {
    assert.equal(INITIAL[prop], undefined, `${prop} must not have an assumed initial`);
  }
});
