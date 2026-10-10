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

test("who the folder on screen waits for is read from the payload its state came in", () => {
  store.configure({ follows: "selection" });
  store.setStatus(
    {
      ...payload(["documents", "photos"], "photos", "photos"),
      state: "queued",
      waiting_for: "documents",
    },
    next(),
  );
  assert.equal(store.select.daemonState(), "queued");
  assert.equal(store.select.waitingFor(), "documents");
  // The next payload for the folder says nothing is waiting any more: nothing is remembered.
  store.setStatus({ ...payload(["documents", "photos"], "photos", "photos"), state: "idle" }, next());
  assert.equal(store.select.waitingFor(), null);
  // Another folder's payload is not this folder's wait.
  store.setStatus(
    { ...payload(["documents", "photos"], "documents", "photos"), state: "queued", waiting_for: "photos" },
    next(),
  );
  assert.equal(store.select.waitingFor(), null, "photos is selected, and its own payload said nothing");
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
  const withheld = { path: "x", direction: "local" };
  store.setStatus(payload(["docs", "photos"], "photos", "docs", { pending_deletions: [withheld] }), next());
  store.setStatus(payload(["docs", "photos"], "docs", "docs", { pending_deletions: [withheld] }), next());
  // The SAME path in the same direction in two folders: two deletions, each tagged with its own.
  assert.deepEqual(store.select.pendingDeletions(), [{ ...withheld, pair: "docs" }]);
  store.setStatus(payload(["docs", "photos"], "photos", "photos", { pending_deletions: [withheld] }), next());
  assert.deepEqual(store.select.pendingDeletions(), [{ ...withheld, pair: "photos" }]);
  // Filed WITH the status, so a pair that has just become selected never shows an empty queue between
  // its status and its deletions.
  store.setStatus(payload(["docs", "photos"], "docs", "docs", { pending_deletions: [] }), next());
  assert.deepEqual(store.select.pendingDeletions(), []);
  // An outage says nothing new about the queue, and forgets none of it.
  store.setStatus(payload(["docs", "photos"], "docs", "docs", { pending_deletions: [withheld] }), next());
  store.setStatus({ state: "unreachable", error: "daemon unreachable", selected: "docs" }, next());
  assert.deepEqual(store.select.pendingDeletions(), [{ ...withheld, pair: "docs" }]);
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

test("a reply in flight across a switch is filed under its own pair and does not move the selection to one with no status", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs", { pending_changes: 4 }), next());
  assert.equal(store.select.pairName(), "docs");
  // `select_pair(photos)` lands while a status read for docs is out. Rust stamps `selected` when the
  // reply is BUILT, so the reply describes docs and says photos is selected.
  store.setStatus(payload(["docs", "photos"], "docs", "photos", { pending_changes: 5 }), next());
  assert.equal(store.select.pairName(), "docs", "a reply about docs cannot carry the window to photos");
  assert.equal(
    store.select.daemonState(),
    "idle",
    "photos has no status yet and must not be drawn as unreachable",
  );
  assert.equal(
    store.select.response().pending_changes,
    5,
    "the reply is still filed, under the pair it described",
  );
  // The first reply that DESCRIBES photos is what moves the window there, with its own status in hand.
  store.setStatus(payload(["docs", "photos"], "photos", "photos", { pending_changes: 9 }), next());
  assert.equal(store.select.pairName(), "photos");
  assert.equal(store.select.daemonState(), "idle");
  assert.equal(store.select.response().pending_changes, 9);
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

// ---- the roster is evidence dated by the request that carried it (review of #447, round 3) ----

/** A reply that lists `names`, each paused or not as `paused` says, for the poll (no folder named). */
const listing = (names, selected, paused = []) => ({
  ...payload(names, selected, selected),
  pairs: names.map((name) => summary(name, { paused: paused.includes(name) })),
});
const OUTAGE = { state: "unreachable", error: "connect: no such file or directory", selected: "docs" };

test("a roster is live until a poll fails, and live again with the next reply that lists it", () => {
  store.configure({ follows: "selection" });
  store.setStatus(listing(["docs", "photos"], "docs"), next());
  assert.equal(store.select.rosterLive(), true);
  assert.equal(store.select.livePairs().length, 2);
  assert.equal(store.select.livePairStates().length, 2);

  store.setStatus(OUTAGE, next());
  // The folders are still NAMED (the window draws its list from them), but nothing says what they are doing.
  assert.equal(store.select.pairs().length, 2, "the roster is kept across the outage");
  assert.equal(store.select.rosterLive(), false);
  assert.deepEqual(store.select.livePairs(), [], "a surface that draws a state is handed no folders");
  assert.deepEqual(store.select.livePairStates(), []);

  store.setStatus(listing(["docs", "photos"], "docs"), next());
  assert.equal(store.select.rosterLive(), true);
  assert.equal(store.select.livePairs().length, 2);
});

test("a roster from a request older than the outage is kept for its names but is not live", () => {
  store.configure({ follows: "selection" });
  store.setStatus(listing(["docs", "photos"], "docs"), next());
  const slow = next(); // left before the socket failed
  const failed = next();
  store.setStatus(OUTAGE, failed);
  store.setStatus(listing(["docs", "photos", "music"], "docs"), slow);
  assert.equal(store.select.pairs().length, 3, "it is newer than the roster held, so its names are taken");
  assert.equal(
    store.select.rosterLive(),
    false,
    "…but a request that left before the failure cannot undo it",
  );
});

test("a reply that says unreachable is not live even though it lists folders", () => {
  store.configure({ follows: "selection" });
  store.setStatus({ ...listing(["docs", "photos"], "docs"), state: "unreachable" }, next());
  assert.equal(store.select.rosterLive(), false);
  assert.deepEqual(store.select.livePairs(), []);
});

test("a failed read of a folder that was NAMED is filed under it and leaves the folder on screen alone", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs", { pending_changes: 4 }), next());
  // Rust stamps a failed read with the SELECTED folder, which is not the folder the request was for.
  store.setStatus({ ...OUTAGE, selected: "docs" }, next(), "photos");
  assert.equal(store.select.pairName(), "docs");
  assert.equal(store.select.daemonState(), "idle", "docs was not read, so docs is not unreachable");
  assert.equal(store.select.response().pending_changes, 4, "and what it last said is still there");
  assert.equal(store.select.rosterLive(), true, "a failed side read does not date the roster");
  assert.equal(store.pairOf(OUTAGE, "photos"), "photos");
  assert.equal(store.pairOf(OUTAGE), "docs", "the poll names none, so it is the selected folder's");
  // The poll failing is the daemon's: that one IS filed under the folder on screen.
  store.setStatus(OUTAGE, next());
  assert.equal(store.select.daemonState(), "unreachable");
});

test("a late reply carrying an older roster does not replace the newer one", () => {
  store.configure({ follows: "selection" });
  const older = next(); // left first
  const newer = next();
  // The newer reply says photos has something waiting, and the older one — from before — says it had not.
  store.setStatus(
    {
      ...listing(["docs", "photos"], "docs"),
      pairs: [summary("docs"), summary("photos", { pending_deletions: 2 })],
    },
    newer,
  );
  store.setStatus(
    { ...listing(["docs", "photos"], "docs"), pairs: [summary("docs"), summary("photos")] },
    older,
  );
  assert.equal(store.select.pairs().find((p) => p.name === "photos").pending_deletions, 2);
  assert.equal(store.select.pairsIssue(), newer);
  // A reply that lists none is an answer too, and is ordered the same way.
  store.setStatus({ state: "idle", response: { pending_changes: 0 } }, older);
  assert.equal(store.select.pairs().length, 2, "an old reply that lists none does not clear the roster");
  store.setStatus({ state: "idle", response: { pending_changes: 0 } }, next());
  assert.deepEqual(store.select.pairs(), []);
});
