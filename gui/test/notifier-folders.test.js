// The notifier at two folders or more (#102 phase 5e).
//
// `notifier.test.js` is the notifier at ONE folder and is untouched by this change: every case in it
// still passes with the decision function it always had. This file is what is new — the same four
// triggers, decided for several folders at once, against ONE banner.
//
// WHAT CAN GO WRONG HAS THE SAME TWO DIRECTIONS AS EVER, MULTIPLIED. Too eager: the same queue said again
// because adding a second folder made the first forget, or one banner per folder stacked on the screen.
// Too shy: files about to go in `photos` and nothing said because `documents` had a banner up, or a stale
// reading of a folder nothing has looked at since said aloud. Nothing here is visible in a screenshot.
//
// The store is driven for real (`store.setStatus`, `setConflicts`) and `notifierViews` reads it, so a test
// that fails here fails on the wiring `app.js` uses and not on a fixture of it. One file, one process, one
// store: the cases are ordered, and each sets up the store it needs.

import { test } from "node:test";
import assert from "node:assert/strict";
import * as store from "../src/js/store.js";
import {
  COALESCE_MS,
  OUTAGE_AFTER_SECS,
  decide,
  emptyState,
  keyOf,
  notifierViews,
  restoreState,
} from "../src/js/notifier.js";

const NOW_MS = 1_800_000_000_000;
const NOW_SECS = NOW_MS / 1000;

let issue = 0;
const next = () => (issue += 1);

const deletion = (over = {}) => ({
  path: "old/2019",
  direction: "local",
  entity_kind: "directory",
  fingerprint: "abc",
  ...over,
});

/** A roster entry, as the daemon lists it. */
const summary = (name, over = {}) => ({
  name,
  paused: false,
  syncing: false,
  last_error: null,
  pending_changes: 0,
  pending_deletions: 0,
  last_sync_epoch_secs: NOW_SECS - 60,
  ...over,
});

/** A status reply about `name` that lists `pairs`, as the poll receives it with `docs` selected. */
function reply(name, pairs, { queue = [], states = {}, response = {} } = {}) {
  return {
    state: states[name] ?? "idle",
    selected: "docs",
    pairs,
    pair_states: pairs.map((entry) => ({ name: entry.name, state: states[entry.name] ?? "idle", rank: 0 })),
    response: {
      pair: name,
      paused: false,
      syncing: false,
      last_error: null,
      pending_changes: 0,
      last_sync_epoch_secs: NOW_SECS - 60,
      ...response,
      pending_deletions: queue,
      pairs,
    },
  };
}

/** The daemon's `status_payload(Err)`: no reply and no roster. */
const outage = { state: "unreachable", error: "connect: no such file or directory", selected: "docs" };

const policy = "only_when_needed";

/** One tick, the way `app.js` makes it: views from the store, then `decide`. */
function tick(state, nowMs = NOW_MS) {
  const { views, roster } = notifierViews(store.select);
  return decide({ state, views, roster, policy, nowMs });
}

/** A state that has watched everything there is to watch and said nothing recently. */
const settled = () => ({ ...emptyState(), sawUnsynced: true, said: { firstSync: "first" } });

/** The store as the poll leaves it: `docs` selected and answering, `photos` summarised and (once fetched) filed. */
function load({ docs = {}, photos = {}, photosQueue = [], docsQueue = [], conflicts = {} } = {}) {
  const pairs = [
    summary("docs", docs),
    summary("photos", { pending_deletions: photosQueue.length, ...photos }),
  ];
  store.setStatus(reply("docs", pairs, { queue: docsQueue }), next());
  // The poll fetches a folder that is not on screen only while its summary counts a queue (E6).
  if (photosQueue.length) store.setStatus(reply("photos", pairs, { queue: photosQueue }), next(), "photos");
  store.setConflicts(conflicts.docs ?? [], "docs");
  store.setConflicts(conflicts.photos ?? [], "photos");
}

// ------------------------------------------------------------------------------ one folder ----

test("a state saved before folders is read as the default folder's, and nothing it said is said again", () => {
  // The shape every install has in localStorage today: bare kinds, the two witnesses at the top.
  const saved = JSON.parse(
    JSON.stringify({
      said: { deletion: `old/2019\u0001abc`, firstSync: "first" },
      lastAt: NOW_MS - 3_600_000,
      lastKind: "deletion",
      sawUnsynced: true,
      lastSeenSync: NOW_SECS - 100,
    }),
  );
  const restored = restoreState(saved);
  assert.equal(restored.lastPair, null);
  assert.deepEqual(restored.seen, {});
  assert.equal(restored.said.deletion, saved.said.deletion);

  // Two folders now. The default one (`docs`) has the same queue it had; `photos` is new.
  load({ docsQueue: [deletion()], docs: { pending_deletions: 1 } });
  const { event, state } = tick(restored);
  assert.equal(event, null, "the default folder's old memory was read as the default folder's");
  assert.equal(state.said.deletion, saved.said.deletion);
  assert.equal(
    Object.keys(state.said).some((key) => key.includes("@")),
    false,
    "no `kind@name` for the default folder",
  );
});

test("the default folder is remembered under the bare kind, whatever it is called", () => {
  // `n1_decisions_are_unchanged`'s other half: a second folder appearing must not make the first one
  // forget, so its keys are the ones a one-folder install already has.
  load({ docsQueue: [deletion()], docs: { pending_deletions: 1 } });
  const first = tick(settled());
  assert.equal(first.event.kind, "deletion");
  assert.equal(first.event.pair, "docs");
  assert.deepEqual(Object.keys(first.state.said).sort(), ["deletion", "firstSync"]);
  assert.equal(keyOf("deletion", null), "deletion");
  assert.equal(keyOf("deletion", "photos"), "deletion@photos");
});

test("a state that is not a state is an empty one, and a broken `seen` is dropped entry by entry", () => {
  assert.deepEqual(restoreState(null), emptyState());
  assert.deepEqual(restoreState("nope"), emptyState());
  assert.deepEqual(restoreState({ said: 3 }), emptyState());
  const mixed = restoreState({
    said: {},
    lastPair: 7,
    seen: {
      "@photos": { sawUnsynced: true, lastSeenSync: 5 },
      "@junk": 3,
      "@odd": { sawUnsynced: "yes", lastSeenSync: "later" },
      bare: { sawUnsynced: true },
    },
  });
  assert.equal(mixed.lastPair, null);
  assert.deepEqual(mixed.seen, {
    "@photos": { sawUnsynced: true, lastSeenSync: 5 },
    "@odd": { sawUnsynced: false, lastSeenSync: null },
  });
  assert.deepEqual(restoreState({ said: {}, seen: [1, 2] }).seen, {});
});

// ----------------------------------------------------------------------------- two folders ----

test("the same queue in two folders is two things to say, and never two banners at once", () => {
  load({
    docsQueue: [deletion()],
    docs: { pending_deletions: 1 },
    photosQueue: [deletion()],
  });
  const first = tick(settled());
  assert.equal(first.event.kind, "deletion");
  assert.equal(first.event.pair, "docs", "folders are decided in the order the daemon lists them");

  // Inside the window: held. One banner at a time, whatever folder it is about.
  const held = tick(first.state, NOW_MS + 5_000);
  assert.equal(held.event, null);

  // After it: the other folder's, which has the identical signature and is a different thing to say.
  const second = tick(first.state, NOW_MS + COALESCE_MS + 1);
  assert.equal(second.event.kind, "deletion");
  assert.equal(second.event.pair, "photos");
  assert.ok("deletion@photos" in second.state.said);

  // And then both have been said.
  assert.equal(tick(second.state, NOW_MS + COALESCE_MS * 10).event, null);
});

test("coalescing is global: a banner for one folder holds the next folder's, and a deletion still jumps it", () => {
  load({ conflicts: { docs: [{ path: "a.txt" }] } });
  const first = tick(settled());
  assert.equal(first.event.kind, "conflict");
  assert.equal(first.event.pair, "docs");

  // `photos` now has a conflict too, five seconds later: the window is the banner's, not the folder's.
  load({ conflicts: { docs: [{ path: "a.txt" }], photos: [{ path: "b.txt" }] } });
  assert.equal(tick(first.state, NOW_MS + 5_000).event, null);

  // A permanent deletion in `photos` is more serious than what is on screen and jumps the window.
  load({
    conflicts: { docs: [{ path: "a.txt" }], photos: [{ path: "b.txt" }] },
    photosQueue: [deletion()],
  });
  const urgent = tick(first.state, NOW_MS + 5_000);
  assert.equal(urgent.event.kind, "deletion");
  assert.equal(urgent.event.pair, "photos");

  // And the held conflict arrives once the window is over.
  load({ conflicts: { docs: [{ path: "a.txt" }], photos: [{ path: "b.txt" }] } });
  const later = tick(first.state, NOW_MS + COALESCE_MS + 1);
  assert.equal(later.event.kind, "conflict");
  assert.equal(later.event.pair, "photos");
});

test("a deletion jumps a banner for a different folder, and the folder it replaced is not forgotten", () => {
  load({ conflicts: { docs: [{ path: "a.txt" }] } });
  const first = tick(settled());
  assert.equal(first.state.lastPair, null, "the live banner is about the default folder");
  load({ conflicts: { docs: [{ path: "a.txt" }] }, photosQueue: [deletion()] });
  const urgent = tick(first.state, NOW_MS + 1_000);
  assert.equal(urgent.state.lastPair, "photos");
  assert.equal(urgent.state.said.conflict, first.state.said.conflict, "docs' conflict is still remembered");
});

test("a banner comes down when ITS folder's subject is gone, not when any folder still has one", () => {
  load({ conflicts: { docs: [{ path: "a.txt" }], photos: [{ path: "b.txt" }] } });
  const first = tick(settled());
  assert.equal(first.event.pair, "docs");
  // `docs`' conflict is resolved; `photos` still has one. The banner on screen was about `docs`'.
  load({ conflicts: { photos: [{ path: "b.txt" }] } });
  const after = tick(first.state, NOW_MS + 2_000);
  assert.equal(after.resolved, true);
  // …and it is not withdrawn while its own folder still has the conflict.
  load({ conflicts: { docs: [{ path: "a.txt" }], photos: [] } });
  assert.equal(tick(first.state, NOW_MS + 2_000).resolved, false);
});

test("a paused folder is not an outage, and an unpaused one is", () => {
  const old = NOW_SECS - OUTAGE_AFTER_SECS - 100;
  // `photos` has not synced in more than a day and is paused: a weekend away, not an outage. `docs`
  // (the folder on screen) is fresh and is NOT paused, so a paused flag read from the shown folder
  // would call `photos`' silence an outage.
  load({ photos: { last_sync_epoch_secs: old, paused: true } });
  const paused = tick(settled());
  assert.equal(paused.event, null);

  load({ photos: { last_sync_epoch_secs: old, paused: false } });
  const down = tick(settled());
  assert.equal(down.event.kind, "outage");
  assert.equal(down.event.pair, "photos");
  assert.equal(down.event.cause, "unreachable");
});

test("the outage cause is each folder's own state", () => {
  const old = NOW_SECS - OUTAGE_AFTER_SECS - 100;
  const pairs = [summary("docs"), summary("photos", { last_sync_epoch_secs: old })];
  const expired = reply("docs", pairs, { states: { photos: "authExpired" } });
  store.setStatus(expired, next());
  assert.equal(tick(settled()).event.cause, "auth");
});

test("a folder's first sync is announced once, per folder", () => {
  // `docs` has been watched for a long time. `photos` has just been added: nothing has synced in it.
  const base = { ...settled(), lastSeenSync: NOW_SECS - 100 };
  load({ photos: { last_sync_epoch_secs: null } });
  const watching = tick(base);
  assert.equal(watching.event, null);
  assert.equal(watching.state.seen["@photos"].sawUnsynced, true, "watched, for `photos`");
  assert.equal(watching.state.sawUnsynced, true);

  // Its first sync lands: announced, for `photos`, and `docs`' own memory is not what silenced it.
  load({ photos: { last_sync_epoch_secs: NOW_SECS - 5 } });
  const done = tick(watching.state, NOW_MS + 60_000);
  assert.equal(done.event.kind, "firstSync");
  assert.equal(done.event.pair, "photos");
  assert.equal(done.state.said.firstSync, "first", "docs' own `firstSync` is untouched");
  assert.equal(done.state.said["firstSync@photos"], "first");

  // Once.
  assert.equal(tick(done.state, NOW_MS + 600_000).event, null);
});

test("a folder this install never watched unsynced is not announced as a first sync", () => {
  // The same rule as ever, per folder: `last_sync_epoch_secs` being set says nothing about whether the
  // first sync just ended. `photos` is established and was never seen without one.
  load({});
  assert.equal(tick({ ...settled(), lastSeenSync: NOW_SECS - 100 }).event, null);
});

// ------------------------------------------------------------------------------- the roster ----

test("with the daemon stopped, a stale roster speaks for no other folder", () => {
  // #246's shape, said aloud. The store keeps the last roster across a failed read — that is how the
  // window still NAMES its folders — and it holds `photos` with a permanent deletion waiting and a last
  // sync more than a day old. A banner built from that would describe a folder nothing has looked at
  // since the daemon stopped. The live accessors are the answer, so the other folder says nothing; the
  // folder on screen is read from its own slice, as at one folder.
  const old = NOW_SECS - OUTAGE_AFTER_SECS - 100;
  load({ photosQueue: [deletion()], photos: { last_sync_epoch_secs: old } });
  // The positive control: while the daemon answers, `photos` IS worth a banner.
  assert.equal(tick(settled()).event.pair, "photos");

  store.setStatus(outage, next());
  assert.equal(store.select.rosterLive(), false);
  assert.equal(store.select.pairs().length, 2, "the roster is still held, for naming");
  const { views, roster } = notifierViews(store.select);
  assert.deepEqual(
    views.map((view) => view.pair),
    ["docs"],
    "only the folder on screen is read",
  );
  assert.equal(roster, null, "nothing is learnt about which folders exist from a roster that is not live");
  const stopped = tick(settled());
  assert.notEqual(stopped.event?.pair, "photos");
  assert.equal(stopped.event?.kind === "deletion", false);

  // Back: the roster is live again and `photos` speaks again.
  load({ photosQueue: [deletion()], photos: { last_sync_epoch_secs: old } });
  assert.equal(tick(settled()).event.pair, "photos");
});

test("while the daemon is down, what was said about the other folders is not forgotten", () => {
  const base = settled();
  load({ photosQueue: [deletion()] });
  const said = tick(base);
  assert.equal(said.event.pair, "photos");
  store.setStatus(outage, next());
  const down = tick(said.state, NOW_MS + COALESCE_MS * 10);
  assert.ok("deletion@photos" in down.state.said, "no evidence is not an empty queue");
  // The daemon returns with the same queue: it was said, and is not said again.
  load({ photosQueue: [deletion()] });
  assert.equal(tick(down.state, NOW_MS + COALESCE_MS * 20).event, null);
  // A banner about it is not withdrawn on no evidence either.
  store.setStatus(outage, next());
  assert.equal(tick(said.state, NOW_MS + 1_000).resolved, false);
});

test("a queue is read only while its summary counts one", () => {
  // The poll fetches a folder's withheld deletions only while the summary says it has some, and a list
  // fetched earlier is not evidence about a queue the summary now counts as empty.
  load({ photosQueue: [deletion()] });
  assert.equal(tick(settled()).event.pair, "photos");
  // The queue drained: the summary says 0, and the list the store still holds is stale.
  const pairs = [summary("docs"), summary("photos", { pending_deletions: 0 })];
  store.setStatus(reply("docs", pairs), next());
  assert.equal(store.select.pendingDeletionsOf("photos").length, 1, "the stale list is still held");
  const { views } = notifierViews(store.select);
  assert.deepEqual(views.find((view) => view.pair === "photos").response.pending_deletions, []);

  // And a summary that counts a queue nobody has fetched yet says nothing until it is.
  const counted = [summary("docs"), summary("ghost", { pending_deletions: 3 })];
  store.setStatus(reply("docs", counted), next());
  const ghost = notifierViews(store.select).views.find((view) => view.pair === "ghost");
  assert.deepEqual(ghost.response.pending_deletions, []);
});

test("below two folders there is one view, and it names no folder", () => {
  store.setStatus(reply("docs", [summary("docs")]), next());
  const { views } = notifierViews(store.select);
  assert.equal(views.length, 1);
  assert.equal(views[0].pair, null);
  assert.equal(views[0].isDefault, true);
  // A daemon that predates the selector lists none.
  store.setStatus(
    { ...reply("docs", []), pairs: undefined, response: { ...reply("docs", []).response, pairs: undefined } },
    next(),
  );
  assert.equal(notifierViews(store.select).views[0].pair, null);
});

test("a folder the daemon no longer runs is forgotten, and a banner about it is withdrawn", () => {
  load({ photosQueue: [deletion()] });
  const said = tick(settled());
  assert.equal(said.event.pair, "photos");
  assert.ok("deletion@photos" in said.state.said);
  assert.equal(said.state.lastPair, "photos");

  // `photos` is removed from the config and the daemon restarted: the roster is live and lists `docs` alone.
  store.setStatus(reply("docs", [summary("docs")]), next());
  const after = tick(said.state, NOW_MS + 5_000);
  assert.equal(
    Object.keys(after.state.said).some((key) => key.endsWith("@photos")),
    false,
  );
  assert.equal(after.resolved, true, "its buttons would act on a folder that is not there");
  assert.equal(after.state.lastPair, null);

  // Added again under the same name, it announces its own first sync: nothing of the old one survived.
  const again = { ...after.state, sawUnsynced: true };
  load({ photos: { last_sync_epoch_secs: null } });
  const watched = tick(again, NOW_MS + 60_000);
  assert.equal(watched.state.seen["@photos"].sawUnsynced, true);
});

test("a folder called `constructor` is an ordinary folder", () => {
  const pairs = [summary("docs"), summary("constructor", { pending_deletions: 1 })];
  store.setStatus(reply("docs", pairs), next());
  store.setStatus(reply("constructor", pairs, { queue: [deletion()] }), next(), "constructor");
  const first = tick(settled());
  assert.equal(first.event.pair, "constructor");
  assert.ok("deletion@constructor" in first.state.said);
  const again = tick(first.state, NOW_MS + COALESCE_MS * 10);
  assert.equal(again.event, null);
  assert.deepEqual(Object.keys(first.state.seen), ["@constructor"]);
});
