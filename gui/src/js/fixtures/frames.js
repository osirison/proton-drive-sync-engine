// The fixture registry (F9) — one deterministic dataset per in-scope frame label, selected by
// `?frame=<label>`.
//
// The same data drives the fidelity harness and the browser design preview, so a frame that passes
// CI is a frame a human can open and look at. That is the whole point: a gate whose inputs nobody
// can see is a gate nobody trusts. `fixtures/preview.js` is the human half; `tools/fidelity/` is the
// machine half; this file is what both of them read.
//
// WHAT IS HERE AND WHAT IS NOT. This module assembles and resolves; it holds no data of its own. One
// module per screen family, named for the frame set it covers, so an S-task edits exactly one file:
//
//   main.js   2a ×6      conflicts.js  3a ×3      deletions.js    4a ×4      plan.js   5a ×3
//   activity.js 6a+7a ×6 settings.js   8a ×5      onboarding.js   9a ×5      tray.js  10a ×6
//   notifications.js 11a ×5            light.js  12a ×8
//
// plus `fids.js` (the node-key tables — the prototype's tree, not the app's data) and `clock.js`
// (the one value a fixture may read from the wall clock). Every one of them is a leaf or near-leaf:
// nothing under `ui/` may be imported except `copy.js` and `format.js`, because `ui/compact.js` and
// `ui/chrome.js` import `fid` from HERE and `import-x/no-cycle` is an error.
//
// A DATASET IS NOT A MAPPING, and keeping the two apart is what makes the numbers honest. All 51
// frames have a dataset and, since S10, all 51 carry a `fids` map (11 when F9 wrote this line, 43
// before the light set landed). `check-fixtures.mjs` gates the first count and `assert.mjs` reports
// the second, so adding forty datasets cannot make the style gate look like it grew teeth it did
// not grow. It did not move 11/51 by one frame.

import { MAIN_FIXTURES } from "./main.js";
import { CONFLICT_FIXTURES } from "./conflicts.js";
import { DELETION_FIXTURES } from "./deletions.js";
import { PLAN_FIXTURES } from "./plan.js";
import { ACTIVITY_FIXTURES } from "./activity.js";
import { SETTINGS_FIXTURES } from "./settings.js";
import { ONBOARDING_FIXTURES } from "./onboarding.js";
import { TRAY_FIXTURES } from "./tray.js";
import { NOTIFICATION_FIXTURES } from "./notifications.js";
import { LIGHT_FIXTURES } from "./light.js";

/**
 * Every in-scope frame, keyed by the label `extract.mjs` reads off the prototype. The key must match
 * `frames/index.json` exactly — `check-fixtures.mjs` compares the two sets in both directions, so a
 * typo in a 24-character label is a build failure rather than a frame that quietly renders the
 * generic mock.
 */
export const FIXTURES = {
  ...MAIN_FIXTURES,
  ...CONFLICT_FIXTURES,
  ...DELETION_FIXTURES,
  ...PLAN_FIXTURES,
  ...ACTIVITY_FIXTURES,
  ...SETTINGS_FIXTURES,
  ...ONBOARDING_FIXTURES,
  ...TRAY_FIXTURES,
  ...NOTIFICATION_FIXTURES,
  ...LIGHT_FIXTURES,
};

/** The frame the URL asks for, or null for the live app. */
export function activeFrame() {
  if (typeof location === "undefined") return null;
  return new URLSearchParams(location.search).get("frame");
}

/**
 * Resolve `sameAs`, which is how the light set says "the same data, with the tokens swapped".
 *
 * Light is a token swap, not different data: `12a Settled light` is `2a Settled` rendered under the
 * light palette, and writing its dataset out again would be two copies that can disagree. So a light
 * fixture names its dark twin and carries only what genuinely differs.
 *
 * THE MAPPING IS INHERITED TOO, since S10. It was not: `fids` came from the entry alone, because a
 * light twin could not pass the gate — the prototype draws all sixty frames on one dark page, so a
 * `12a` node that set no colour was extracted as the dark text tier and a correctly-light app failed
 * on every one. That was the ground truth being wrong, not the mapping, and `extract.mjs` now records
 * which colours came from the page rather than from the frame (DEVIATIONS §58b).
 *
 * With that fixed there is nothing left for the asymmetry to protect, and one thing it costs: a
 * hand-written light `fids` is the second copy of a table that can drift from the first. The trees
 * are not similar, they are the SAME — `check-fixtures.mjs` now fails the build if a `sameAs` pair's
 * node keys ever stop matching, so inheriting the mapping is checked rather than assumed.
 */
export function resolveFixture(label, seen = new Set()) {
  const entry = FIXTURES[label];
  if (!entry) return null;
  if (!entry.sameAs) return entry;
  // FAIL CLOSED on a broken chain, both here and one line down. The first version returned the
  // partially-resolved entry instead — so a cycle or a missing link produced a fixture with a
  // dangling `sameAs` and none of the twin's data, which renders as a frame that is quietly wrong
  // rather than absent, and made `check-fixtures.mjs`'s "sameAs chain does not resolve" branch
  // unreachable while it claimed to be checking exactly this. `null` is what both callers already
  // handle: the gate fails the build, and the preview draws its no-fixture diagnostic.
  if (seen.has(label)) return null;
  seen.add(label);
  const twin = resolveFixture(entry.sameAs, seen);
  if (!twin) return null;
  const { sameAs: _twinLabel, ...own } = entry;
  return { ...twin, ...own };
}

/** The fixture for the selected frame, or null. */
export function activeFixture() {
  const label = activeFrame();
  return label ? resolveFixture(label) : null;
}

/**
 * Stamp `data-fid` on a node, if a frame is selected and it has a key for this slot. A no-op in the
 * live app, so the attribute never ships to a user — it exists only for the harness and the preview.
 */
export function fid(node, slot, ...args) {
  const fixture = activeFixture();
  const declared = fixture?.fids?.[slot];
  if (!node || declared == null) return node;
  // A FACTORY MAY ANSWER "NOT IN THIS FRAME". S9 needed it twice over: the Settings pill row gains a
  // fifth tab no `8a` frame draws, and one of the three `11a In situ` banners draws its app name at
  // a letter-spacing the other four banners do not. Both are one node inside a run the rest of which
  // is mapped, and the alternative — stamping `…:undefined` — fails as "no such node key", which is
  // the message for a stale mapping and would send the next reader looking for a re-extraction.
  const key = typeof declared === "function" ? declared(...args) : declared;
  if (key == null) return node;
  node.setAttribute("data-fid", `${activeFrame()}:${key}`);
  return node;
}

/**
 * THE SEVEN LIGHT TWINS ARE MAPPED BY INHERITANCE, and it is worth saying why here rather than
 * leaving the reader to infer it from four lines of spread syntax.
 *
 * They were mapped once before, by hand, run, and taken back out. The panel needed no new code in
 * light — the same fixture under `prefers-color-scheme: light` reproduced
 * `12a Compact settled/syncing/needs light` at every colour those frames actually DECLARE. What it
 * could not reproduce was the colour they INHERIT: the prototype draws all sixty frames on one dark
 * page, so every node in a `12a` frame that sets no colour of its own inherits `#F2F4F7` from that
 * page. The app in light mode inherits `#14161A`, correctly, and failed on all 142 of them — 142
 * failures, one class, zero real.
 *
 * That was never a mapping problem. `extract.mjs` records per node which colour properties came from
 * the page rather than from the frame, and `assert.mjs` declines to compare those on a light frame
 * and prints how many it declined — so the fixtures say what they know and stop asserting what they
 * do not. DEVIATIONS §58b carries the measurement that started it.
 *
 * With the ground truth fixed, a hand-written light `fids` would be the second copy of a table that
 * already exists — the exact shape this build has been bitten by before. The seven light frames are
 * their dark twins node for node (26/26, 60/60, 75/75, 63/63, 12/12, 36/36, 13/13, same keys, same
 * text, measured), so the mapping is the twin's, and `check-fixtures.mjs` fails the build the day
 * that stops being true.
 */
