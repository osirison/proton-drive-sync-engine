// What the notifier may say about a folder that is not on screen, and when it knows enough to say it
// (#102 phase 5e, review round).
//
// `11-notifications.md`: "Only what is known is said." A folder that is not on screen is read from two
// different places — the roster's SUMMARY, which the poll refreshes every tick and which carries a COUNT,
// and the LIST behind that count, which a separate request fetches and which lands later. Two ways of
// saying something false came out of treating the second as if it were as fresh as the first:
//
//   · A LIST OLDER THAN THE COUNT IT IS MEANT TO EXPLAIN. `Keep them` drains a folder's queue, the summary
//     falls to 0 and nothing refetches at 0, so the list the store holds still names the file that was
//     kept. A new deletion arrives, the summary counts 1, and the notifier — running in the same tick as
//     the fire-and-forget fetch — builds its banner from the old list: it names a file that was already
//     kept, and `Keep them` on it keeps one it never named.
//   · A FOLDER WHOSE DATA HAS NOT LANDED IS NOT A FOLDER WITH NOTHING IN IT. On the first poll after a
//     launch the summary counts a queue and the list has not arrived; the conflict scan has not run. The
//     notifier read both as empty, forgot what it had said, and said it again when they landed — once per
//     launch, for every standing banner of every folder that is not selected.
//
// Each case below drives the REAL store, on a store instance of its own (the module is loaded again under
// a different query, so one case's folders are not the next one's) — and mirrors how `app.js` calls it:
// `beginStatus` before a request, `statusesIssued` before a scan.

import { test } from "node:test";
import assert from "node:assert/strict";
import { decide, emptyState, notifierViews } from "../src/js/notifier.js";

const NOW_MS = 1_800_000_000_000;
const NOW_SECS = NOW_MS / 1000;
const policy = "only_when_needed";

let launches = 0;
/** A store of its own: a launch of the app, with nothing heard yet. */
const launch = () => import(`../src/js/store.js?launch=${(launches += 1)}`);

const deletion = (path) => ({ path, direction: "local", entity_kind: "file", fingerprint: `fp-${path}` });
const conflict = (path) => ({ original: path, sidecar: `${path}.proton-cloud`, path });

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

function reply(name, pairs, queue = []) {
  return {
    state: "idle",
    selected: "docs",
    pairs,
    pair_states: pairs.map((entry) => ({ name: entry.name, state: "idle", rank: 0 })),
    response: {
      pair: name,
      paused: false,
      syncing: false,
      last_error: null,
      pending_changes: 0,
      last_sync_epoch_secs: NOW_SECS - 60,
      pending_deletions: queue,
      pairs,
    },
  };
}

/**
 * The daemon and the poll, as `app.js` drives a store: `docs` is on screen, `photos` is not.
 * `names` is the roster; `queued` is how many withheld deletions the summary counts for `photos`.
 */
function daemonWindow(store, names = ["docs", "photos"]) {
  const roster = (queued) =>
    names.map((name) => summary(name, name === "photos" ? { pending_deletions: queued } : {}));
  return {
    /** The poll: the shown folder's status, whose roster carries every folder's COUNT. */
    poll(queued) {
      store.setStatus(reply("docs", roster(queued)), store.beginStatus());
    },
    /** `refreshOtherPair`'s read of photos' queue — it counts what it was just told. */
    fetch(queue) {
      store.setStatus(reply("photos", roster(queue.length), queue), store.beginStatus(), "photos");
    },
    /** `refreshOtherPair`'s conflict scan: the clock is read BEFORE the request leaves. */
    scan(conflicts) {
      const issued = store.select.statusesIssued();
      store.setConflicts(conflicts.map(conflict), "photos", issued);
    },
    /** One evaluation of the notifier, the way `evaluateNotifications` makes it. */
    tick(state, nowMs = NOW_MS) {
      const { views, roster: live } = notifierViews(store.select);
      return decide({ state, views, roster: live, policy, nowMs });
    },
  };
}

/** A state that has watched everything there is to watch and has said nothing yet. */
const settled = () => ({ ...emptyState(), sawUnsynced: true, said: { firstSync: "first" } });

const photosView = (store) => notifierViews(store.select).views.find((view) => view.pair === "photos");

// ------------------------------------------------------------------------ a list older than its count ----

test("a queue fetched before the count last changed is not read as the queue that count reports", async () => {
  const store = await launch();
  const w = daemonWindow(store);
  w.poll(1);
  w.fetch([deletion("x.txt")]);
  assert.equal(store.select.deletionsFreshOf("photos"), true, "fetched after the summary that counted it");

  // The same count reported again: the list is still the one it explains.
  w.poll(1);
  assert.equal(store.select.deletionsFreshOf("photos"), true);

  // The queue drains. The summary says 0, and a list held from before is not evidence about anything.
  w.poll(0);
  assert.equal(store.select.deletionsFreshOf("photos"), false);

  // A new deletion: the summary counts 1 again, and the list held is the OLD one's.
  w.poll(1);
  assert.equal(store.select.pendingDeletionsOf("photos").length, 1, "the old list is still held");
  assert.equal(store.select.deletionsFreshOf("photos"), false, "older than the count it would explain");

  // Its own read lands: now it is.
  w.fetch([deletion("y.txt")]);
  assert.equal(store.select.deletionsFreshOf("photos"), true);
  assert.deepEqual(
    store.select.pendingDeletionsOf("photos").map((item) => item.path),
    ["y.txt"],
  );
});

test("a read that left before the queue drained cannot vouch for the queue that came after", async () => {
  const store = await launch();
  const w = daemonWindow(store);
  w.poll(1);
  // The read for x.txt leaves (issue n), and is slow...
  const slow = reply(
    "photos",
    [summary("docs"), summary("photos", { pending_deletions: 1 })],
    [deletion("x.txt")],
  );
  const left = store.beginStatus();
  // ...while the queue drains and a different deletion arrives, both seen by the poll.
  w.poll(0);
  w.poll(1);
  // The slow read lands now, carrying x.txt, from before both.
  store.setStatus(slow, left, "photos");
  assert.equal(
    store.select.deletionsFreshOf("photos"),
    false,
    "its request left before the count last changed",
  );
  assert.equal(photosView(store).response.pending_deletions.length, 0, "the notifier does not build from it");
});

test("a banner never names a file that was already kept, and the next banner names only the new one", async () => {
  const store = await launch();
  const w = daemonWindow(store);
  // photos has x.txt waiting; the poll sees it, the read lands, the banner goes up.
  w.poll(1);
  w.scan([]);
  w.fetch([deletion("x.txt")]);
  const first = w.tick(settled());
  assert.equal(first.event.kind, "deletion");
  assert.equal(first.event.pair, "photos");
  assert.deepEqual(first.event.paths, ["x.txt"]);

  // `Keep them` on that banner: the daemon applies it, the queue is empty, and nothing refetches at 0.
  w.poll(0);
  const drained = w.tick(first.state, NOW_MS + 2_000);
  assert.equal(drained.event, null);
  assert.equal(drained.resolved, true, "the banner's subject is gone");

  // A NEW deletion, y.txt. This tick runs while its read is still in flight.
  w.poll(1);
  const racing = w.tick(drained.state, NOW_MS + 4_000);
  assert.equal(
    racing.event,
    null,
    `said ${JSON.stringify(racing.event)} about a list that has not been read`,
  );

  // The read lands. The banner is about y.txt and nothing else.
  w.fetch([deletion("y.txt")]);
  const second = w.tick(racing.state, NOW_MS + 6_000);
  assert.equal(second.event.kind, "deletion");
  assert.equal(second.event.pair, "photos");
  assert.deepEqual(second.event.paths, ["y.txt"]);
});

test("the notifier is told which kinds of a folder it has not heard", async () => {
  const store = await launch();
  const w = daemonWindow(store);
  w.poll(1);
  const before = photosView(store);
  assert.deepEqual([...before.unknown].sort(), ["conflict", "deletion"]);
  assert.deepEqual(before.response.pending_deletions, []);
  assert.deepEqual(before.conflicts, []);

  w.fetch([deletion("x.txt")]);
  assert.deepEqual(photosView(store).unknown, ["conflict"]);
  w.scan([]);
  assert.deepEqual(photosView(store).unknown, []);

  // A summary that counts nothing needs no list to be known.
  w.poll(0);
  assert.deepEqual(photosView(store).unknown, []);
  w.poll(1);
  assert.deepEqual(photosView(store).unknown, ["deletion"]);
});

// -------------------------------------------------------------------- data that has not landed yet ----

test("a standing deletion banner is not said again by the next launch while its queue is still being read", async () => {
  // LAUNCH 1: photos' deletion is read and said.
  const one = await launch();
  const a = daemonWindow(one);
  a.poll(1);
  a.scan([]);
  a.fetch([deletion("x.txt")]);
  const said = a.tick(settled());
  assert.equal(said.event.kind, "deletion");
  const saved = JSON.parse(JSON.stringify(said.state));

  // LAUNCH 2, a new store and the saved state, well after the window. The first poll counts the queue and
  // its read has not landed; the conflict scan has not run either.
  const two = await launch();
  const b = daemonWindow(two);
  b.poll(1);
  const first = b.tick(saved, NOW_MS + 120_000);
  assert.equal(first.event, null);
  assert.ok("deletion@photos" in first.state.said, "what was said is remembered until the queue is read");
  assert.equal(first.resolved, false, "a banner is not withdrawn on no evidence");
  assert.equal(first.state.lastKind, "deletion");

  // The reads land: the same queue, which was said.
  b.fetch([deletion("x.txt")]);
  b.scan([]);
  const landed = b.tick(first.state, NOW_MS + 122_000);
  assert.equal(landed.event, null, "said again after a relaunch");
  assert.ok("deletion@photos" in landed.state.said);
});

test("a standing conflict banner is not said again by the next launch while its scan has not run", async () => {
  const one = await launch();
  const a = daemonWindow(one);
  a.poll(0);
  a.scan(["note.txt"]);
  const said = a.tick(settled());
  assert.equal(said.event.kind, "conflict");
  assert.equal(said.event.pair, "photos");
  const saved = JSON.parse(JSON.stringify(said.state));

  const two = await launch();
  const b = daemonWindow(two);
  b.poll(0);
  const first = b.tick(saved, NOW_MS + 120_000);
  assert.equal(first.event, null);
  assert.ok("conflict@photos" in first.state.said);
  assert.equal(first.resolved, false);

  b.scan(["note.txt"]);
  const landed = b.tick(first.state, NOW_MS + 122_000);
  assert.equal(landed.event, null, "said again after a relaunch");
  assert.ok("conflict@photos" in landed.state.said);
});

test("what has landed IS evidence: a queue that is read empty, or a scan that finds nothing, is forgotten", async () => {
  // The other direction, so the rule above cannot be read as "never forget": once the data has landed an
  // empty answer is an answer, and the identical conflict made again tomorrow is a new thing to say.
  const one = await launch();
  const a = daemonWindow(one);
  a.poll(0);
  a.scan(["note.txt"]);
  const said = a.tick(settled());
  const saved = JSON.parse(JSON.stringify(said.state));

  const two = await launch();
  const b = daemonWindow(two);
  b.poll(0);
  b.scan([]);
  const empty = b.tick(saved, NOW_MS + 120_000);
  assert.equal("conflict@photos" in empty.state.said, false);
  assert.equal(empty.resolved, true, "the conflict it was about is gone");

  // And a summary that counts 0 needs no read at all: the queue is empty, and that is known.
  const three = await launch();
  const c = daemonWindow(three);
  c.poll(0);
  c.scan([]);
  const drained = c.tick({ ...saved, said: { ...saved.said, "deletion@photos": "stale" } }, NOW_MS + 120_000);
  assert.equal("deletion@photos" in drained.state.said, false);
});

test("a scan that was issued before the folder joined the roster is not about the folder it joined as", async () => {
  const store = await launch();
  const w = daemonWindow(store);
  w.poll(0);
  w.scan(["note.txt"]);
  assert.equal(store.select.conflictsFreshOf("photos"), true);

  // photos is removed, then added again under the same name: a different folder with an old one's scan held.
  w.poll(0);
  const alone = daemonWindow(store, ["docs"]);
  alone.poll(0);
  w.poll(0);
  assert.equal(store.select.conflictsOf("photos").length, 1, "the old scan is still held");
  assert.equal(store.select.conflictsFreshOf("photos"), false, "it predates the folder's membership");
  assert.deepEqual(photosView(store).conflicts, [], "and the notifier does not read it");

  w.scan([]);
  assert.equal(store.select.conflictsFreshOf("photos"), true);
});

test("a conflict list set without saying which request it came from is not dated, and so is not known", async () => {
  // The caller that edits a list in place after a decision (`app.js`) must not re-date it as a fresh scan:
  // an undated list is the safe direction — the notifier says less, never more.
  const store = await launch();
  const w = daemonWindow(store);
  w.poll(0);
  store.setConflicts([conflict("note.txt")], "photos");
  assert.equal(store.select.conflictsOf("photos").length, 1);
  assert.equal(store.select.conflictsFreshOf("photos"), false);
  w.scan(["note.txt"]);
  assert.equal(store.select.conflictsFreshOf("photos"), true);
  const dated = store.select.conflictsOf("photos");
  store.setConflicts([], "photos");
  assert.equal(
    store.select.conflictsFreshOf("photos"),
    true,
    "an edit in place keeps the date of the scan it edits",
  );
  assert.notDeepEqual(store.select.conflictsOf("photos"), dated);
});

test("below two folders every kind is known, whatever the store has heard", async () => {
  const store = await launch();
  store.setStatus(reply("docs", [summary("docs")]), store.beginStatus());
  const { views } = notifierViews(store.select);
  assert.equal(views.length, 1);
  assert.equal(views[0].pair, null);
  assert.equal(views[0].unknown, undefined);
});

test("the shown folder is always known, whatever has landed for the others", async () => {
  const store = await launch();
  const w = daemonWindow(store);
  w.poll(1);
  const shown = notifierViews(store.select).views.find((view) => view.pair === "docs");
  assert.equal(shown.unknown, undefined);
});

test("a folder whose kinds are unknown says nothing of them and leaves the rest of its decisions alone", async () => {
  // photos has not synced for more than a day: an outage is known from the SUMMARY, and said, even though
  // neither list has landed.
  const store = await launch();
  const w = daemonWindow(store);
  store.setStatus(
    reply("docs", [summary("docs"), summary("photos", { last_sync_epoch_secs: NOW_SECS - 90_000 })]),
    store.beginStatus(),
  );
  const result = w.tick(settled(), NOW_MS);
  assert.equal(result.event.kind, "outage");
  assert.equal(result.event.pair, "photos");
});
