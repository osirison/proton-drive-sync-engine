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
import { nextOnboardingLatch } from "../src/js/routes.js";

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
