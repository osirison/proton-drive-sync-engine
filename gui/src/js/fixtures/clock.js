// The fixtures' clock convention (F9), in one place because it has to be one rule for all 51 frames
// rather than a judgement per frame.
//
// A fixture may not compute a displayed string — that is the rule the whole set is built on, and
// this module is the single exception that proves it. The distinction is what the APP does with the
// value, not what kind of value it is:
//
//   · RELATIVE renders — `2 minutes ago`, `14s ago`, `last agreed 3 hours ago` — are pinned as an
//     offset from now. `ago(120)` is always "2 minutes ago" whatever the wall clock says, so the
//     frame reproduces on any machine at any hour, and the app's own formatter is exercised rather
//     than bypassed.
//
//   · ABSOLUTE renders — `14:38`, `13:58`, `edited 14:41` — are written LITERALLY in the fixture,
//     never derived here. An epoch formatted as a clock time depends on the machine's timezone and
//     changes across midnight; the frame draws one exact string, and the only way to reproduce it
//     everywhere is to write it. The tray fixtures did this from the start (`TRAY.retrying("40s",
//     "13:58")`) and the rest of the set follows them.
//
// So: `ago()` for anything the app renders as a duration, a string literal for anything it renders
// as a time. There is no third case, and a fixture reaching for `new Date()` is a bug.

// THE CLOCK IS FROZEN WHILE A FRAME IS SELECTED, and without this the paragraph above is a claim
// rather than a fact.
//
// `ago(120)` is only "always 2 minutes ago" if nothing moves between the fixture being built and the
// app formatting it. `since()` reads `Date.now()` at render, so the gap is however long the harness
// takes to get to that frame — and every relative string has a boundary somewhere in it. `9a Review`
// found it in CI: `ago(40)` renders `worked out 40 seconds ago` at 165.00px for the first twenty
// seconds and `worked out 1 minute ago` at 151.81px after that, and a loaded runner reaching the
// frame a minute after load failed the build on a screen the commit had not touched. Measured, and
// 151.81/672.17 are exactly the numbers CI reported.
//
// Freezing removes the class rather than widening the margin: every frame now renders the same
// string on a fast machine and a slow one, which is what a 51-frame pixel gate needs from its clock.
//
// SCOPED TO `?frame=`, so the shipped app is untouched — the attribute-stamping in `frames.js` is
// gated the same way and for the same reason. Two things in the app read the clock for something
// other than display (a 15s conflict-rescan throttle and the notifier's rate limit); under a frozen
// clock neither advances, which in a fixture-driven preview is correct — the data comes from the
// fixture, not from a rescan.
if (typeof location !== "undefined" && new URLSearchParams(location.search).has("frame")) {
  const frozen = Date.now();
  Date.now = () => frozen;
}

/** Seconds since the epoch, `seconds` ago. The only moving value any fixture is allowed. */
export function ago(seconds) {
  return Math.floor(Date.now() / 1000) - seconds;
}
