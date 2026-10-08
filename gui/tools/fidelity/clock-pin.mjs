// The instant every fidelity page load sees, set from OUTSIDE the page (#102 phase 5a-2, K1).
//
// WHY IT EXISTS. `fixtures/clock.js` freezes `Date.now()` for a `?frame=` page, but it freezes it at
// whatever the machine's clock reads when THAT page's script runs. Two loads of one URL therefore
// freeze at two different instants, and anything a frame prints from the clock as an ABSOLUTE time
// differs between them whenever a minute boundary falls in the gap. `9a Consent` prints
// `since ${clock(ago(60))}`, so CI rendered "since 15:17" and then "since 15:18" from one URL and the
// n1 gate reported a frame that "is not deterministic" — a gate failing at random, which is the
// failure mode its own header exists to rule out. `clock.js` says absolute times must be written
// literally in a fixture; eight frames do not (`ago()` through `clock()` in `screens/main.js`), and
// rewriting the fixtures' strings is the wrong fix, because the app formatting an epoch is what those
// frames exist to exercise.
//
// THE FIX IS THAT THE LOADS DO NOT DISAGREE ABOUT WHAT TIME IT IS. Before each load the gate installs
// a `Date` whose `now()` — and whose argument-less constructor — answer one fixed instant. `clock.js`
// then freezes THAT, so `ago(60)`, `since()` and `clock()` read the same second in every load of every
// frame on every machine. It is the existing hook (`Date.now`, read once at load) fed a fixed input,
// not a second clock; and `performance.now` is not pinned because nothing in `gui/src` reads it.
//
// THE TIME ZONE IS PINNED WITH IT. `clock()` and `monthYear()` format in the machine's zone, so a
// fixed instant alone would still print `10:29` on one runner and `11:29` on another. UTC is the
// zone, for the same reason `prefers-color-scheme` is pinned per frame: a headless browser's default
// is a property of the platform.
//
// THE SKEW IS HOW THE PIN IS KEPT HONEST. `armClock(page, { realClockSkewMs })` moves the REAL clock
// before the pin replaces it, which the page can never observe while the pin holds — and which, the
// moment the pin stops holding, is exactly the minute boundary that used to fall between two loads
// only sometimes. The n1 gate arms every load with a different skew of more than a minute, so a
// regression here fails every run on the same eight frames instead of one run in three on whichever
// one the boundary landed on. `FIDELITY_CLOCK_POISON=1` is that regression on demand (the convention
// of `S10_CONTRAST_POISON`): it leaves the skew and drops the pin, and the gate must go red.

/** 2026-01-15 10:30:30 UTC. Mid-minute and mid-hour, so neither a rounding nor a carry is near. */
export const PINNED_NOW_MS = Date.UTC(2026, 0, 15, 10, 30, 30);

/** The zone `clock()` and `monthYear()` format in. */
export const PINNED_TIME_ZONE = "UTC";

/**
 * A skew larger than a minute, so two loads armed with consecutive multiples of it can never agree on
 * a minute — whatever second either started on.
 */
export const SKEW_STEP_MS = 61_000;

/**
 * Runs IN THE PAGE before any of its scripts, so it is serialised by puppeteer and closes over
 * nothing. `pinnedMs === null` leaves the real clock, moved by `skewMs` — the unpinned condition the
 * gate proves itself against. Otherwise every way a script asks for the current time answers
 * `pinnedMs`: `Date.now()`, `new Date()` and `Date()`. A date built FROM a value is untouched.
 *
 * A static a script ASSIGNS (`Date.now = () => frozen`, which is what `clock.js` does) is honoured, and
 * kept off the real `Date`: the pinned page still lets `clock.js` freeze its own reading, and the
 * unpinned one — the condition the gate proves itself against — freezes at the instant the load ran,
 * which is the behaviour being guarded against and not a different one.
 *
 * `scope` is a parameter so the same function runs against a stand-in global in a unit test.
 */
export function installClock(pinnedMs, skewMs, scope = globalThis) {
  const RealDate = scope.Date;
  const realNow = RealDate.now.bind(RealDate);
  const now = pinnedMs === null ? () => realNow() + skewMs : () => pinnedMs;
  const assigned = new Map();
  scope.Date = new Proxy(RealDate, {
    construct: (target, args, newTarget) =>
      Reflect.construct(target, args.length ? args : [now()], newTarget),
    apply: () => new RealDate(now()).toString(),
    get: (target, prop) => {
      if (assigned.has(prop)) return assigned.get(prop);
      return prop === "now" ? now : Reflect.get(target, prop, target);
    },
    set: (_target, prop, value) => {
      assigned.set(prop, value);
      return true;
    },
  });
}

const armed = new WeakMap();

/**
 * Arm `page` so that its NEXT navigation (and every one after, until re-armed) runs on the pinned
 * clock. Idempotent: a page armed twice carries one script, the latest, so a gate can re-arm before
 * every load with a different skew.
 */
export async function armClock(page, { realClockSkewMs = 0 } = {}) {
  const previous = armed.get(page);
  if (previous) await page.removeScriptToEvaluateOnNewDocument(previous);
  await page.emulateTimezone(PINNED_TIME_ZONE);
  const poisoned = process.env.FIDELITY_CLOCK_POISON === "1";
  const { identifier } = await page.evaluateOnNewDocument(
    installClock,
    poisoned ? null : PINNED_NOW_MS,
    realClockSkewMs,
  );
  armed.set(page, identifier);
}
