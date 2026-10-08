---
trigger: npm run fidelity, fidelity:n1, "the SAME url rendered two different DOMs", "never stopped changing", is-entering, since 15:17, FIDELITY_CLOCK_POISON, clock-pin, armClock, fixtures/clock.js
depends_on: gui/src/js/screens/main.js, gui/tools/fidelity/clock-pin.mjs, gui/tools/fidelity/check-n1-identity.mjs, gui/src/js/fixtures/clock.js
recorded: 2026-10-08
---

# `fidelity:n1` said one URL rendered two different DOMs, and the only difference was a minute

**Symptom:** CI's `fidelity` job failed, and the same commit passed on a re-run.

```
9a Consent: the SAME url rendered two different DOMs, so the comparison below means nothing
    authored: "... 0 changes have piled up since 15:17. ..."
    one pair: "... 0 changes have piled up since 15:18. ..."
```

**Cause (measured):** `fixtures/clock.js` freezes `Date.now()` at the moment a `?frame=` page loads, so
two loads of one URL freeze at two instants. Eight frames print an absolute time computed from it
(`clock(ago(60))` — `11a In situ`, `12a Conflict light`, `3a Conflict`, `6a Activity passes`,
`6a Details`, `7a File pending`, `7a Never synced`, `9a Consent`), and the two renders differ whenever a
minute boundary falls between the loads. The gate renders each frame up to three times, so a run fails
whenever any of the eight straddles a boundary (about one run in three by estimate; the rate was not
measured, the mechanism was — a forced 61-second skew changes exactly these eight).

**Fix:** every gate that opens a `?frame=` page calls `armClock(page)` (`clock-pin.mjs`) before its first
navigation. That installs a fixed instant (`2026-01-15 10:30:30 UTC`) and a UTC time zone in the page
before `clock.js` reads the clock, so every load of every frame sees the same second. A new gate that
loads frames must do the same — `gui/test/clock-pin.test.js` fails if it does not.

**Prove the pin is holding:** `FIDELITY_CLOCK_POISON=1 npm run fidelity:n1` drops the pin and keeps the
skew the gate adds (each load's real clock reads 61 seconds more than the previous load's). It must exit
1 and name exactly those eight frames. If it passes, the pin is not what the gate is running on.

**A second cause, same message (fixed after the clock pin):** the clock pin took the failure from one run
in three to about one in ten, and the remainder was a frame caught inside the band's 220 ms entrance
animation. `settledHtml` called the DOM settled after three equal samples 100 ms apart, which spans 200 ms;
`fillBand` takes `is-entering` off only on `animationend`, so under load a load's samples could all fall
before the class came off and the next load's after (`9a Consent`, reproduced 1 run in 10 with five n1
runs in parallel). `settledHtml` now also requires `document.getAnimations()` to hold no FINITE animation
(an infinite one, `breathe` or `blip`, changes no markup and never ends) on each of the three samples, and
a page that has not settled in 8 s FAILS. To see the difference on demand, set `--t-appear: 1500ms` in
`tokens.css` and settle `9a Consent` both ways: the old rule returns a DOM with `is-entering` on it, the
new one waits it out.

**Do not fix it in the fixtures.** The frames exist to exercise the app formatting an epoch; writing
the times as literals would stop testing that. `clock.js`'s header says absolute times should be
literal and these eight predate that rule being enforced.

**Running the gates:** `PUPPETEER_CACHE_DIR=$REAL_HOME/.cache/puppeteer` under a scratch `HOME`; see
`npm-run-fidelity-with-a-scratch-home.md`. `fidelity:n1` alone takes about two minutes.
