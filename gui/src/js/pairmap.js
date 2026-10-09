// The one way a table keyed by a folder pair's NAME is made (#102 phase 5c-1, carried from the
// review of #445).
//
// A pair's name is text the person chose, within `[A-Za-z0-9._-]{1,64}`, and the engine refuses
// neither `constructor`, `toString`, `hasOwnProperty`, `valueOf` nor `__proto__`. Every table here that
// was a plain object (`{}`) answered those names from `Object.prototype` instead of from itself:
// `byPair["constructor"]` is the `Object` function, so `??=` saw a value, kept it, and the slice a
// status was filed into was a function; `settingsByPair["__proto__"]` is the prototype itself, so
// a staged edit for that folder was written onto every object in the page. A `Map` has no such
// names, which is the whole reason this file exists instead of a convention.
//
// ONE HELPER, NOT A HABIT. `new Map()` at three sites is three chances to write `{}` at a fourth, and
// `gui/test/pairmap.test.js` reads the sources and fails on a per-pair table that is not made here.
// The updater copies, because the callers that need one (`configByPair`, `settingsByPair`) replace
// the table on every write — an identity that moves is how "nothing was staged since" is tested.

/** An empty table keyed by pair name. */
export function pairTable() {
  return new Map();
}

/** A copy of `table` with `pair` set to `value`. Never mutates the table it was given. */
export function withPair(table, pair, value) {
  const next = new Map(table);
  next.set(pair, value);
  return next;
}
