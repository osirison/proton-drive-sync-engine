// The test nextOnboardingLatch's own comment has been asking for since PR #131 — "Pure and side
// effect free so it can be unit-tested" — and which F4 (#168) requires be carried forward with the
// function, because F4 deletes the module it used to live in.
//
// This is worth a test in a way most of this frontend is not. The latch is the reason onboarding is
// reachable at all on a fresh machine, and every one of its branches is a bug that already shipped
// once: without it, writing the folder pair in step 2 flipped the entry condition false and the
// next 2 s poll ejected the user out of the wizard and onto the unreachable screen. None of that is
// visible from reading the function.
//
// `node --test` rather than a framework: it is in Node 22, which the CI frontend job already
// installs, and the function is pure so it needs no DOM.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { nextOnboardingLatch, releasesOnboarding, entersOnboardingTakeover } from "../src/js/routes.js";

// (prev, daemonState, hasConfigPair, configLoaded, statusPolled)
const latch = nextOnboardingLatch;

test("any reachable daemon state releases the latch, whatever we were doing", () => {
  for (const state of ["idle", "running", "paused", "authExpired", "failed"]) {
    assert.equal(latch(true, state, false, true, true), false, `${state} should release`);
    assert.equal(latch(false, state, false, true, true), false, `${state} should stay released`);
  }
});

test("authExpired releases rather than trapping the user in a wizard that cannot fix it", () => {
  // Onboarding cannot re-authenticate the Proton CLI; the main screen has the action that can.
  assert.equal(latch(true, "authExpired", true, true, true), false);
});

test("a failed first sync releases too — the wizard cannot fix a daemon error", () => {
  // #246 in the direction that is easy to get backwards. Before the `failed` state existed, a failed
  // first pass derived to `idle`, released the latch, and handed off to a main screen saying
  // `Everything is up to date` — the bug. Adding the state and NOT adding it here would have fixed
  // the sentence and trapped the user in two steps that cannot put `proton-drive` back on the PATH.
  assert.equal(latch(true, "failed", true, true, true), false);
  assert.equal(latch(true, "failed", false, true, true), false, "nor with no pair written yet");
});

test("the release set has exactly one definition, and app.js asks for it", () => {
  // THE BUG THIS FILE'S OWN FIX SHIPPED WITH. `render()` kept a second copy of the release list,
  // under a comment claiming it was "EXACTLY `nextOnboardingLatch`'s RELEASE SET" — and that copy
  // gates the sticky `onboardingFailure`, which SHORT-CIRCUITS this module:
  //
  //     onboardingStage !== null ? false : onboardingFailure ? true : nextOnboardingLatch(…)
  //
  // So the lists disagreeing did not draw a mismatched screen. It made the `failed` arm above dead
  // code on the one path it was written for, and latched the wizard shut on a failed first sync —
  // the inverse of the hand-off, and undismissable. Every gate stayed green: nothing renders the
  // takeover against a failed daemon. DEVIATIONS §90f.
  //
  // Reading the source is the only check available for a caller this suite cannot execute (no DOM,
  // deliberately), and it is the construction `tray-view.test.js` already uses against `state.rs`.
  // What it defends is the shape, not the one state: any hand-written chain of daemon-state
  // comparisons near `reachable` fails it.
  const app = readFileSync(fileURLToPath(new URL("../src/js/app.js", import.meta.url)), "utf8");
  const line = app.split("\n").find((l) => l.includes("const reachable ="));
  assert.ok(line, "app.js no longer has a `reachable` — if the latch moved, move this test with it");
  assert.match(line, /releasesOnboarding\(/, "app.js is deriving the release set itself again");
  assert.doesNotMatch(line, /===\s*"/, "a hand-written state list is back in app.js");

  // And the export itself, so a caller has something to ask.
  for (const state of ["idle", "running", "paused", "authExpired", "failed"]) {
    assert.equal(releasesOnboarding(state), true, `${state} should release`);
  }
  for (const state of ["firstRun", "unreachable"]) {
    assert.equal(releasesOnboarding(state), false, `${state} is an entry trigger, not a release`);
  }
});

test("firstRun always enters — the canonical signal", () => {
  assert.equal(latch(false, "firstRun", false, true, true), true);
  assert.equal(latch(false, "firstRun", true, true, true), true, "a configured pair does not veto firstRun");
  assert.equal(latch(false, "firstRun", false, false, false), true, "nor do the un-loaded gates");
});

test("a genuinely fresh machine enters: polled, config read, and no pair", () => {
  assert.equal(latch(false, "unreachable", false, true, true), true);
});

test("an unreachable daemon does NOT enter before a poll has completed", () => {
  // `unreachable` is also the pre-poll default, so without this gate every cold start would flash
  // the wizard on its way to the real state.
  assert.equal(latch(false, "unreachable", false, true, false), false);
});

test("an unreachable daemon does NOT enter before the config file has been read", () => {
  // "No pair" and "we have not looked yet" are the same value until configLoaded says otherwise.
  assert.equal(latch(false, "unreachable", false, false, true), false);
});

test("an unreachable daemon with a folder pair holds, rather than entering", () => {
  // A daemon configured elsewhere and simply not running is not a fresh machine.
  assert.equal(latch(false, "unreachable", true, true, true), false);
});

test("THE REGRESSION: writing the pair mid-flow does not eject the user", () => {
  // Step 2 writes the folder pair. The daemon is not up yet, so the state is still `unreachable`,
  // and `hasConfigPair` has just flipped true — the exact input that a naive `!hasPair` entry
  // condition would read as "not a fresh machine, leave onboarding". The latch holds `prev`.
  assert.equal(latch(true, "unreachable", true, true, true), true);
});

test("an unknown or future daemon state holds whatever we were doing", () => {
  assert.equal(latch(true, "somethingNew", false, true, true), true);
  assert.equal(latch(false, "somethingNew", false, true, true), false);
});

test("the latch is a pure function of its arguments", () => {
  const args = [true, "unreachable", false, true, true];
  const first = latch(...args);
  assert.equal(latch(...args), first);
  assert.equal(latch(...args), first);
});

// ---- entering the takeover (#337) --------------------------------------------------------------
//
// `onboardingDetour` (#244) is cleared in two places: `resetOnboardingFlow`, when the flow ends by
// completing, and `render`'s arm edge, when it instead ends by the latch releasing out from under
// an open detour — the daemon becomes reachable and synced (`releasesOnboarding`) while the user is
// still on step 1 or step 2, `Start the first sync` never clicked. Without the second place, a later
// re-entry (e.g. `reset-index` returning the daemon to `firstRun`) opens the takeover back inside
// whichever sub-screen was left open, which is exactly the state `resetOnboardingFlow`'s own comment
// forbids.
//
// `entersOnboardingTakeover(prev, next)` is that edge, and it has to be the false→true one — not
// "released" (`true, false`) and not "latched at all" (`next` alone) — or one of two things breaks:
// clearing on release fires the instant `Start the first sync` is clicked (`onboardingStage`
// forcing the latch false while the merge dialog floats over the main screen), which is harmless
// there only by coincidence (a detour cannot be open once step 2's footer offers that button); and
// clearing on "latched" fires on every render of a session already running, which would forbid a
// detour from ever staying open — `onDetour` sets it from INSIDE an armed takeover, and the very
// next render would null it back out.

test("arming the takeover is what clears a detour left over from an earlier session", () => {
  assert.equal(entersOnboardingTakeover(false, true), true);
});

test("a detour opened during a single continuous takeover session must survive", () => {
  // The latch was already true and stays true — nothing about this render is an entry.
  assert.equal(entersOnboardingTakeover(true, true), false);
});

test("releasing is not the edge this clears on", () => {
  // The alternative fix: clearing when the latch goes true→false instead. Pinned false on purpose —
  // `render` must not call this on release, only on arm (see the source-pin test below).
  assert.equal(entersOnboardingTakeover(true, false), false);
});

test("never having been onboarding at all is not an entry either", () => {
  assert.equal(entersOnboardingTakeover(false, false), false);
});

test("`render` clears the detour on the SAME edge it clears the main app's own leftovers on", () => {
  // A source-text pin, like the release-set test above: `entersOnboardingTakeover` being correct in
  // isolation proves nothing about app.js if the call site clears the detour on a different edge, on
  // no edge, or not at all. This anchors the whole sequence — arm fires, and the detour is one of
  // the things it discards — the way `onboarding.test.js` cannot, since it only ever sees `bodyOf`'s
  // already-resolved `detour` prop and not the module state that feeds it.
  const app = readFileSync(fileURLToPath(new URL("../src/js/app.js", import.meta.url)), "utf8");

  assert.match(
    app,
    /entersOnboardingTakeover,\s*\n\} from "\.\/routes\.js";/,
    "app.js must import the shared edge rather than re-deriving it",
  );

  const start = app.indexOf("if (entersOnboardingTakeover(");
  assert.notEqual(start, -1, "the arm edge is no longer where render() checks it");
  const end = app.indexOf("\n  }", start);
  assert.notEqual(end, -1, "the arm block has no end");
  const block = app.slice(start, end);

  assert.match(block, /onboardingDetour = null;/, "arming must clear the detour, per #337");
  // The two pre-existing layers, so a future edit cannot "fix" #337 by deleting these instead.
  assert.match(block, /screenStack = \[\];/);
  assert.match(block, /dialogOverlay = null;/);
  assert.match(block, /dialogReturn = null;/);

  // And exactly one such block — a second copy of the edge is this codebase's most-repeated bug.
  const occurrences = app.split("if (entersOnboardingTakeover(").length - 1;
  assert.equal(occurrences, 1, "entersOnboardingTakeover must be asked once, not re-derived");
});
