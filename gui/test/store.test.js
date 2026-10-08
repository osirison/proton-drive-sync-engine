// The client store, keyed by folder pair (#102 phase 5a-2, brief section 4.6).
//
// `store.js` is a module-level singleton and every test below shares it, so each begins by saying
// which pair it is about and none relies on a clean slate: they assert on slices they filed
// themselves. (`node --test` runs each FILE in its own process, so nothing leaks between files.)

import { test } from "node:test";
import assert from "node:assert/strict";
import * as store from "../src/js/store.js";

let issue = 0;
const next = () => {
  issue += 1;
  return issue;
};

const summary = (name, over = {}) => ({
  name,
  local_root: `/l/${name}`,
  remote_root: `/Drive/${name}`,
  db_path: `/l/${name}/.sync/sync_index.db`,
  paused: false,
  syncing: false,
  reconcile_seq: 0,
  last_sync_epoch_secs: 1,
  last_error: null,
  pending_changes: 0,
  pending_deletions: 0,
  ...over,
});

/** A payload as Rust sends it for a daemon that lists `names`, describing `described`, with `selected` chosen. */
const payload = (names, described, selected, over = {}) => ({
  state: "idle",
  selected,
  pairs: names.map((name) => summary(name)),
  pair_states: names.map((name) => ({ name, state: "idle" })),
  response: { pair: described, pairs: names.map((name) => summary(name)), pending_changes: 0, ...over },
});

test("a legacy reply, a fixture and the moment before any reply all belong to the one default pair", () => {
  store.configure({ follows: "selection" });
  store.setStatus({ state: "running", response: { syncing: true, pending_changes: 3 } }, next());
  assert.equal(store.select.pairName(), "default");
  assert.equal(store.select.daemonState(), "running");
  assert.equal(store.select.response().pending_changes, 3);
  assert.deepEqual(store.select.pairs(), [], "a daemon that predates the selector lists no pairs");
});

test("every select answers for the selected pair's slice", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs", { pending_changes: 4 }), next());
  store.setStatus(payload(["docs", "photos"], "photos", "docs", { pending_changes: 9 }), next());
  // Two replies filed, one under each pair; the selection is `docs`, so that is what is read.
  assert.equal(store.select.pairName(), "docs");
  assert.equal(store.select.response().pending_changes, 4);
  assert.deepEqual(
    store.select.pairs().map((p) => p.name),
    ["docs", "photos"],
  );
  assert.equal(store.select.pairStates().length, 2);
});

test("a scan is filed under the pair it was issued for, whatever is selected when it lands", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  const conflict = { original: "a.txt", sidecar: "a.proton-cloud.txt", kind: "content" };
  // Issued while `photos` was shown, answering after the selection moved to `docs`.
  store.setConflicts([conflict], "photos");
  assert.deepEqual(store.select.conflicts(), [], "docs does not see a conflict found in photos");
  assert.equal(store.select.unresolvedConflictCount(), 0);
  // And back: it is still photos' conflict, and it says so.
  store.setStatus(payload(["docs", "photos"], "photos", "photos"), next());
  assert.equal(store.select.pairName(), "photos");
  assert.deepEqual(store.select.conflicts(), [{ ...conflict, pair: "photos" }]);
});

test("a withheld deletion carries the pair whose queue it is in", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  store.setPendingDeletions([{ path: "x", direction: "local" }], "photos");
  store.setPendingDeletions([{ path: "x", direction: "local" }], "docs");
  // The SAME path in the same direction in two folders: two deletions, each tagged with its own.
  assert.deepEqual(store.select.pendingDeletions(), [{ path: "x", direction: "local", pair: "docs" }]);
  store.setStatus(payload(["docs", "photos"], "photos", "photos"), next());
  assert.deepEqual(store.select.pendingDeletions(), [{ path: "x", direction: "local", pair: "photos" }]);
});

test("a slow old reply cannot move the selection back", () => {
  store.configure({ follows: "selection" });
  const stale = next(); // issued first
  const fresh = next(); // issued after the selection moved
  store.setStatus(payload(["docs", "photos"], "photos", "photos"), fresh);
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), stale);
  assert.equal(store.select.pairName(), "photos", "the newest request to say wins the selection");
  // …but the old reply is still filed, under the pair it described.
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  assert.equal(store.select.pairName(), "docs");
});

test("a payload with no reply keeps the roster, and a reply that lists none clears it", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  store.setStatus({ state: "unreachable", error: "daemon unreachable", selected: "docs" }, next());
  assert.equal(store.select.daemonState(), "unreachable");
  assert.equal(store.select.pairs().length, 2, "an outage forgets nothing the app knew");
  store.setStatus({ state: "idle", response: { pending_changes: 0 } }, next());
  assert.deepEqual(store.select.pairs(), [], "a daemon that lists no pairs IS the answer");
});

test("an unreachable payload is filed under the selected pair, so that pair reads as unreachable", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  store.setStatus({ state: "unreachable", error: "daemon unreachable", selected: "docs" }, next());
  assert.equal(store.select.daemonState(), "unreachable");
  assert.equal(store.select.response(), null);
  assert.equal(store.select.countersUnknown(), true);
});

test("the tray panel's mode follows the pair each reply describes, not the app's selection", () => {
  // The panel's rows act on the default pair, so the panel must show it whatever the window chose.
  store.configure({ follows: "reply" });
  try {
    store.setStatus(payload(["docs", "photos"], "docs", "photos", { pending_changes: 2 }), next());
    assert.equal(store.select.pairName(), "docs", "`selected` says photos; the reply describes docs");
    assert.equal(store.select.response().pending_changes, 2);
    // An outage keeps the panel on the pair it was showing.
    store.setStatus({ state: "unreachable", error: "daemon unreachable", selected: "photos" }, next());
    assert.equal(store.select.pairName(), "docs");
    assert.equal(store.select.daemonState(), "unreachable");
  } finally {
    store.configure({ follows: "selection" });
  }
  assert.throws(() => store.configure({ follows: "nonsense" }), /follows must be/);
});

test("pairOf files a payload by the rule setStatus uses", () => {
  store.configure({ follows: "selection" });
  assert.equal(store.pairOf(payload(["a", "b"], "b", "a")), "b", "the pair the reply describes");
  assert.equal(store.pairOf({ state: "unreachable", selected: "a" }), "a", "else the one asked about");
});
