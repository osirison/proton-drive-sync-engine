// What a banner's buttons do, in what order (#102 phase 5e).
//
// `app.js` cannot be imported (it boots on load), so the SEQUENCE lives in `banner-actions.js` with every
// effect handed in, and this file runs it against effects that record what they were asked, in order.
// `fidelity:pairs` drives the same actions on the real page; this is the half that can name the order.

import { test } from "node:test";
import assert from "node:assert/strict";
import { runBannerAction } from "../src/js/banner-actions.js";

/** Effects that log into `calls`; `select` answers `selectable` (after one tick, as a request does). */
function harness({ selectable = true } = {}) {
  const calls = [];
  const note = (...entry) => calls.push(entry.map(String).join(":"));
  return {
    calls,
    fx: {
      keep: (folder) => note("keep", folder),
      tryAgain: () => note("tryAgain"),
      syncPair: (folder) => note("syncPair", folder),
      open: () => note("open"),
      select: async (folder) => {
        note("select-start", folder);
        await Promise.resolve();
        note("select-done", folder);
        return selectable;
      },
      navigate: (route) => note("navigate", route),
      warn: (message) => note("warn", message),
    },
  };
}

test("review and compare select the banner's folder before they navigate", async () => {
  for (const [action, kind, route] of [
    ["review", "deletion", "deletions"],
    ["compare", "conflict", "conflicts"],
    ["review", "conflict", "conflicts"],
  ]) {
    const { calls, fx } = harness();
    await runBannerAction({ kind, action, pair: "photos" }, fx);
    assert.deepEqual(calls, ["open", "select-start:photos", "select-done:photos", `navigate:${route}`]);
  }
});

test("a folder that cannot be selected opens the window and goes nowhere", async () => {
  // Navigating on would draw the Deletions queue of whichever folder happened to be selected, under a
  // banner that named another: a worse answer than the window alone.
  const { calls, fx } = harness({ selectable: false });
  await runBannerAction({ kind: "deletion", action: "review", pair: "gone" }, fx);
  assert.deepEqual(calls, ["open", "select-start:gone", "select-done:gone"]);
});

test("retry is a sync for the banner's folder, and keep is a keep for it", async () => {
  const retry = harness();
  await runBannerAction({ kind: "outage", action: "retry", pair: "photos" }, retry.fx);
  assert.deepEqual(
    retry.calls,
    ["syncPair:photos"],
    "not the tray's sync-everything row, and the window does not move",
  );
  const keep = harness();
  await runBannerAction({ kind: "deletion", action: "keep", pair: "photos" }, keep.fx);
  assert.deepEqual(keep.calls, ["keep:photos"]);
});

test("open selects the folder the outage is about and navigates nowhere", async () => {
  const { calls, fx } = harness();
  await runBannerAction({ kind: "outage", action: "open", pair: "photos" }, fx);
  assert.deepEqual(calls, ["open", "select-start:photos", "select-done:photos"]);
});

test("with no folder every action is what it was at one folder", async () => {
  for (const pair of [undefined, null, ""]) {
    const keep = harness();
    await runBannerAction({ kind: "deletion", action: "keep", pair }, keep.fx);
    assert.deepEqual(keep.calls, ["keep:null"]);
    const retry = harness();
    await runBannerAction({ kind: "outage", action: "retry", pair }, retry.fx);
    assert.deepEqual(retry.calls, ["tryAgain"]);
    const review = harness();
    await runBannerAction({ kind: "deletion", action: "review", pair }, review.fx);
    assert.deepEqual(
      review.calls,
      ["open", "navigate:deletions"],
      "no select, and the screen follows the open",
    );
    const open = harness();
    await runBannerAction({ kind: "outage", action: "open", pair }, open.fx);
    assert.deepEqual(open.calls, ["open"]);
  }
});

test("later does nothing, and an action nobody handles says so", async () => {
  const later = harness();
  await runBannerAction({ kind: "conflict", action: "later", pair: "photos" }, later.fx);
  assert.deepEqual(later.calls, []);
  const odd = harness();
  await runBannerAction({ kind: "conflict", action: "delete", pair: "photos" }, odd.fx);
  assert.deepEqual(odd.calls, ['warn:notification-action: no handler for "delete"']);
});
