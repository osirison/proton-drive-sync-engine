// The five tray glyphs (S8), tested where the fidelity gate cannot reach.
//
// The gate covers a great deal of this set — `10a Glyph states` compares all ten marks node by node,
// and disabling the monochrome fallback fails exactly the five nodes that carry a hue. What it
// cannot cover is here, and it is three things:
//
//   · THE TWO LISTS THAT MUST AGREE. `fixtures/tray.js` names the five forms in the order the sheet
//     lays them down the page, because `glyphFids` keys each cell by its position in that grid;
//     `ui/hexagon.js` names the same five as the forms it can draw. Neither imports the other — a
//     fixture module may not reach into `ui/` (see the header of `fixtures/frames.js`) — so nothing
//     but this test stops them drifting. And the drift is silent in the worst way: reorder one list
//     and every mark still renders, still stamps, and is compared against the wrong drawn cell.
//   · THE SIXTH FORM. `10-tray.md` is explicit that only five exist and that a solid filled hexagon
//     was drawn once by mistake and corrected. A gate cannot test for the absence of a state.
//   · THE KEY SHAPES `glyphFids` PRODUCES. The mapping is right today because the frame was mined
//     for it; these two assertions are what stop someone "simplifying" it back into `hexFids`, whose
//     path counts and indexed gradient are the in-window mark's and are both wrong here.
//
// No DOM in `gui/test` (deliberately — the fidelity gate is this frontend's real check), so nothing
// below renders a mark. Everything asserted is reachable without one.

import { test } from "node:test";
import assert from "node:assert/strict";
import { TRAY_GLYPH_STATES, trayGlyph, strokeForSize } from "../src/js/ui/hexagon.js";
import { glyphFids } from "../src/js/fixtures/fids.js";
import { TRAY_FIXTURES } from "../src/js/fixtures/tray.js";
import { TRAY_MENU } from "../src/js/ui/compact.js";

const SHEET = TRAY_FIXTURES["10a Glyph states"];

test("the sheet's row order and the component's forms are the same five, in the same order", () => {
  // Order, not membership: `glyphFids` maps glyph i onto grid cell 4 + 4·⌊i/2⌋ + i%2, so a
  // reordering maps every mark after the first onto a neighbouring state's cell — where an <svg>
  // also exists, so nothing fails to stamp and nothing fails to render.
  assert.deepEqual(SHEET.glyphs, TRAY_GLYPH_STATES);
});

test("the five forms are the five states the tray menu knows", () => {
  // The panel, the menu and the glyph are three surfaces describing one moment. If the glyph can be
  // in a state the menu has no rows for, the tray can draw a mark it cannot offer an action for.
  assert.deepEqual([...TRAY_GLYPH_STATES].sort(), Object.keys(TRAY_MENU).sort());
});

test("a sixth form is refused, and the refusal says why", () => {
  // 10-tray.md: "Only five forms exist. A solid filled hexagon is not a state — it was drawn that
  // way by mistake during design and corrected. Don't reintroduce it."
  assert.throws(
    () => trayGlyph({ state: "filled" }),
    (error) => /only five forms exist/.test(error.message) && /filled/.test(error.message),
  );
  // The throw is the lookup, ahead of any drawing — which is what lets this run without a DOM, and
  // also what makes it a real guard rather than a crash that happens to occur.
  assert.throws(() => trayGlyph({}), /not one of the five tray glyphs/);
});

test("the sheet is drawn at a size the stroke table actually measures", () => {
  // `strokeForSize` throws rather than interpolating, so a glyph at an unmeasured size is a runtime
  // error inside a tray icon — the least observable place in the app. 20 is what the sheet draws;
  // 10-tray.md's "15–20px" is the range the desktop may scale the SVG to, not a size to pick from.
  assert.equal(SHEET.glyphSize, 20);
  assert.equal(strokeForSize(SHEET.glyphSize, "tray"), 9);
});

test("glyphFids indexes a path only when the form draws two, and never indexes the gradient", () => {
  const fids = glyphFids(TRAY_GLYPH_STATES);
  // settled is glyph 0/1 and draws ONE path — the prototype keys it `path`, with no index. This is
  // the assertion that fails if someone reaches for `hexFids`, whose table says settled is 2 paths
  // (an outline plus the check that does not exist below 20px).
  assert.equal(fids.glyphPath(0, 0), "div[0]/div[4]/svg/path");
  // syncing is glyph 2/3 and draws a track plus one travelling segment.
  assert.equal(fids.glyphPath(2, 0), "div[0]/div[8]/svg/path[0]");
  assert.equal(fids.glyphPath(3, 1), "div[0]/div[9]/svg/path[1]");
  // ONE gradient, so no index. `hexFids` writes `lineargradient[i]` because the in-window syncing
  // mark carries two; the glyph carries one and the frame keys it bare.
  assert.equal(fids.glyphGradient(3), "div[0]/div[9]/svg/defs/lineargradient");
  assert.equal(fids.glyphStop(3, 0), "div[0]/div[9]/svg/defs/lineargradient/stop[0]");
  // needs-you is glyph 4/5: its mass is a circle, and its path has no path siblings.
  assert.equal(fids.glyphPath(4, 0), "div[0]/div[12]/svg/path");
  assert.equal(fids.glyphCircle(5), "div[0]/div[13]/svg/circle");
  // can't-reach is glyph 8/9, and the last row of the grid — the arithmetic has to still hold there.
  assert.equal(fids.glyphPath(8, 1), "div[0]/div[20]/svg/path[1]");
  assert.equal(fids.glyph(9), "div[0]/div[21]/svg");
});
