// The tray panel over a daemon that has stopped answering (review of #447, round 3: the #246 false
// all-clear, in the one surface that did not have the window's guard).
//
// The store keeps the last folder list across a failed status read, and `trayView` at two folders or more
// draws from the LIST — its worst folder's own panel, a pause row per folder — and not from the daemon's
// state. So a stopped daemon was drawn as `Up to date` with `Sync now`, `Pause docs` and `Pause photos`.
// This file drives the real store the way the panel does (`follows: "reply"`) and hands `trayView` what
// the panel hands it, so it fails if either half goes back to the stale list.

import { test } from "node:test";
import assert from "node:assert/strict";
import * as store from "../src/js/store.js";
import { trayView } from "../src/js/screens/tray.js";
import { MAIN, TRAY } from "../src/js/ui/copy.js";

store.configure({ follows: "reply" });

let issue = 0;
const next = () => (issue += 1);

const summary = (name) => ({
  name,
  paused: false,
  syncing: false,
  last_error: null,
  pending_changes: 0,
  pending_deletions: 0,
  last_sync_epoch_secs: 1_750_000_000,
});
const good = {
  state: "idle",
  selected: "docs",
  pairs: [summary("docs"), summary("photos")],
  pair_states: [
    { name: "docs", state: "idle", rank: 0 },
    { name: "photos", state: "idle", rank: 0 },
  ],
  response: {
    pair: "docs",
    paused: false,
    syncing: false,
    last_sync_epoch_secs: 1_750_000_000,
    last_error: null,
    pending_changes: 0,
    pairs: [summary("docs"), summary("photos")],
  },
};
// Rust's `status_payload(Err)`: no reply, no roster.
const outage = { state: "unreachable", error: "connect: no such file or directory", selected: "docs" };

/** What the panel hands `trayView` (`mountTrayPanel`), read from the store. */
const panelProps = () => ({
  daemonState: store.select.daemonState(),
  response: store.select.response(),
  conflicts: store.select.conflicts(),
  deletions: store.select.pendingDeletions(),
  pairs: store.select.livePairs(),
  pairStates: store.select.livePairStates(),
});

const rowsOf = (view) => (view.menuRows ?? []).map((row) => row.label);

test("the panel over a stopped daemon at two folders is the stopped-daemon panel, with no folder rows", () => {
  store.setStatus(good, next());
  const live = trayView(panelProps());
  assert.equal(live.pair, "docs", "the positive control: while it answers, the panel is the folder one");
  assert.ok(rowsOf(live).includes(TRAY.pausePair("photos")));

  store.setStatus(outage, next());
  const stopped = trayView(panelProps());

  // The panel a ONE-folder install draws over the same stopped daemon.
  const oneFolder = trayView({ daemonState: "unreachable", response: null });
  assert.deepEqual(stopped, oneFolder);
  assert.equal(stopped.pair, null);
  assert.equal(stopped.menuRows, null);
  const text = JSON.stringify(stopped);
  assert.ok(!text.includes(MAIN.compact.upToDate), "a stopped daemon is not up to date");
  assert.ok(!text.includes(TRAY.pausePair("docs")) && !text.includes(TRAY.pausePair("photos")));
});

test("the panel is the folder one again when the daemon answers", () => {
  store.setStatus(good, next());
  const view = trayView(panelProps());
  assert.equal(view.pair, "docs");
  assert.ok(rowsOf(view).includes(TRAY.pausePair("photos")));
});
