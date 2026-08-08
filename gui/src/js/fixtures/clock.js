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

/** Seconds since the epoch, `seconds` ago. The only moving value any fixture is allowed. */
export function ago(seconds) {
  return Math.floor(Date.now() / 1000) - seconds;
}
