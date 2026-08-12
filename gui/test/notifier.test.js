// The four triggers (C6, #179) — what interrupts, and what stays silent.
//
// EVERY RULE HERE FAILS QUIETLY IN ONE OF TWO DIRECTIONS. Too eager and a person switches
// notifications off, and then the one that mattered never arrives; too shy and files are deleted
// while the app says nothing. Neither is visible in a screenshot, so nothing else in this repo can
// check any of it — the fidelity gate compares five drawings and has no opinion about when they
// appear.

import { test } from "node:test";
import assert from "node:assert/strict";
import { COALESCE_MS, OUTAGE_AFTER_SECS, decide, emptyState } from "../src/js/notifier.js";
import { NOTIFY_POLICIES } from "../src/js/screens/settings.js";
import { readFileSync } from "node:fs";

const NOW_MS = 1_800_000_000_000;
const NOW_SECS = NOW_MS / 1000;

const deletion = (over = {}) => ({
  path: "photos/2019",
  direction: "local",
  entity_kind: "directory",
  fingerprint: "abc",
  ...over,
});

const view = (over = {}) => ({
  daemonState: "idle",
  response: {
    pending_changes: 0,
    last_sync_epoch_secs: NOW_SECS - 60,
    pending_deletions: [],
    ...(over.response ?? {}),
  },
  conflicts: over.conflicts ?? [],
  ...(over.daemonState ? { daemonState: over.daemonState } : {}),
});

/** One tick, from a state that has already witnessed an unsynced daemon and said nothing recently. */
const tick = (over = {}, state = { ...emptyState(), sawUnsynced: true, said: { firstSync: "first" } }) =>
  decide({ state, view: view(over), policy: over.policy ?? "only_when_needed", nowMs: NOW_MS });

test("a permanent deletion interrupts; a recoverable one does not", () => {
  // The whole design in one assertion: `direction: local` removes the file from disk with no trash,
  // and `remote` puts Proton's copy in Proton's Trash. Only the first can cost you anything.
  assert.equal(tick({ response: { pending_deletions: [deletion()] } }).event?.kind, "deletion");
  assert.equal(tick({ response: { pending_deletions: [deletion({ direction: "remote" })] } }).event, null);
});

test("a direction nobody anticipated warns rather than stays silent", () => {
  // `severityOf` answers `permanent` for anything that is not the literal `remote`. Written down
  // because the opposite default is the natural one to write and it fails in the direction that
  // costs files.
  assert.equal(
    tick({ response: { pending_deletions: [deletion({ direction: "sideways" })] } }).event?.kind,
    "deletion",
  );
});

test("the same queue does not say the same thing twice", () => {
  const first = tick({ response: { pending_deletions: [deletion()] } });
  assert.equal(first.event.kind, "deletion");
  // A tick a full window later, with nothing changed.
  const again = decide({
    state: first.state,
    view: view({ response: { pending_deletions: [deletion()] } }),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 10,
  });
  assert.equal(again.event, null);
  // A second item IS a new thing to say.
  const more = decide({
    state: first.state,
    view: view({
      response: { pending_deletions: [deletion(), deletion({ path: "docs/old", fingerprint: "d" })] },
    }),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 10,
  });
  assert.equal(more.event?.paths.length, 2);
});

test("a decided-and-returned deletion is a new thing to say", () => {
  // The signature carries the fingerprint, not just the path: an approval is pinned to one, so the
  // same path coming back with a different fingerprint is a different decision.
  const first = tick({ response: { pending_deletions: [deletion()] } });
  const returned = decide({
    state: first.state,
    view: view({ response: { pending_deletions: [deletion({ fingerprint: "zzz" })] } }),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 10,
  });
  assert.equal(returned.event?.kind, "deletion");
});

test("nothing stacks inside the 30-second window, and a deletion still jumps it", () => {
  const conflict = tick({ conflicts: [{ path: "notes/todo.txt" }] });
  assert.equal(conflict.event.kind, "conflict");

  // Five seconds later, a second conflict: held, because a banner is already on screen.
  const soon = decide({
    state: conflict.state,
    view: view({ conflicts: [{ path: "notes/todo.txt" }, { path: "docs/plan.md" }] }),
    policy: "only_when_needed",
    nowMs: NOW_MS + 5_000,
  });
  assert.equal(soon.event, null);

  // Five seconds later still, a permanent deletion: NOT held. Waiting 25 seconds to say that files
  // are about to be deleted, because a conflict banner went up, is the wrong way round.
  const urgent = decide({
    state: conflict.state,
    view: view({ response: { pending_deletions: [deletion()] } }),
    policy: "only_when_needed",
    nowMs: NOW_MS + 5_000,
  });
  assert.equal(urgent.event?.kind, "deletion");

  // And after the window, the held conflict arrives — as ONE grouped banner, not the two it missed.
  const later = decide({
    state: conflict.state,
    view: view({ conflicts: [{ path: "notes/todo.txt" }, { path: "docs/plan.md" }] }),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS + 1,
  });
  assert.equal(later.event?.paths.length, 2);
});

test("the first sync is announced once, and only where this install watched the wait", () => {
  // A GUI installed on a machine that has been syncing for a year must not announce that the first
  // sync just finished. `last_sync_epoch_secs` is set on every successful pass and says nothing
  // about which one it was.
  const cold = decide({ state: emptyState(), view: view(), policy: "only_when_needed", nowMs: NOW_MS });
  assert.equal(cold.event, null);
  assert.equal(cold.state.sawUnsynced, false);

  // Now watch the transition: a daemon that has never synced, then one that has.
  const watched = decide({
    state: emptyState(),
    view: view({ response: { last_sync_epoch_secs: null } }),
    policy: "only_when_needed",
    nowMs: NOW_MS,
  });
  assert.equal(watched.event, null);
  assert.equal(watched.state.sawUnsynced, true);

  const done = decide({
    state: watched.state,
    view: view(),
    policy: "only_when_needed",
    nowMs: NOW_MS + 60_000,
  });
  assert.equal(done.event?.kind, "firstSync");
  // Once, ever.
  const again = decide({
    state: done.state,
    view: view(),
    policy: "only_when_needed",
    nowMs: NOW_MS + 600_000,
  });
  assert.equal(again.event, null);
});

test("an unreachable daemon is not a witness that nothing has ever synced", () => {
  // No reply at all is not the daemon saying "nothing has synced" — it is the daemon saying nothing.
  const out = decide({
    state: emptyState(),
    view: { daemonState: "unreachable", response: null, conflicts: [] },
    policy: "only_when_needed",
    nowMs: NOW_MS,
  });
  assert.equal(out.state.sawUnsynced, false);
});

test("the outage fires at a day, not before, and once per episode", () => {
  const almost = { response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS + 1 } };
  assert.equal(tick(almost).event, null);

  const day = { response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS, pending_changes: 61 } };
  const first = tick(day);
  assert.equal(first.event?.kind, "outage");
  assert.equal(first.event.changes, 61);

  // Two seconds later, still down: silence. The alternative is this banner every poll for a day.
  const repeat = decide({
    state: first.state,
    view: view(day),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 10,
  });
  assert.equal(repeat.event, null);

  // A new outage after a good pass is a new episode.
  const fresh = decide({
    state: first.state,
    view: view({ response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS - 500 } }),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 10,
  });
  assert.equal(fresh.event?.kind, "outage");
});

test("the outage names the cause it actually has", () => {
  const day = { response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS } };
  assert.equal(tick({ ...day, daemonState: "authExpired" }).event.cause, "auth");
  assert.equal(tick({ ...day, daemonState: "unreachable" }).event.cause, "unreachable");
});

test("`never` interrupts about nothing, and changes nothing else", () => {
  // The card's own promise: "the menu bar glyph still changes, and things still wait for you rather
  // than happening on their own". Nothing in this module is ever sent to the daemon, so the second
  // half holds by construction; this is the first half.
  const loud = {
    response: { pending_deletions: [deletion()], last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS },
    conflicts: [{ path: "a" }, { path: "b" }],
  };
  assert.equal(tick({ ...loud, policy: "never" }).event, null);
  // …and the deletion still reaches the queue, which is the same object the screen draws from.
  assert.equal(tick(loud).event?.kind, "deletion");
});

test("`only_permanent_deletions` lets exactly one event through", () => {
  const policy = "only_permanent_deletions";
  assert.equal(tick({ conflicts: [{ path: "a" }], policy }).event, null);
  assert.equal(
    tick({ response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS }, policy }).event,
    null,
  );
  assert.equal(tick({ response: { pending_deletions: [deletion()] }, policy }).event?.kind, "deletion");
});

test("an unknown policy is the default, not silence", () => {
  // A hand-edited `gui.toml` must not be able to switch the safety banner off by typo.
  assert.equal(
    tick({ response: { pending_deletions: [deletion()] }, policy: "sometimes" }).event?.kind,
    "deletion",
  );
});

test("the three policy values are the ones the Rust side stores", () => {
  // TWO LANGUAGES, ONE VOCABULARY. The webview writes these tokens and `gui_prefs.rs` parses them;
  // a rename on either side would leave `write_notify_policy` rejecting every save, and nothing else
  // in either test suite would notice.
  const rust = readFileSync(new URL("../gui-core/src/gui_prefs.rs", import.meta.url), "utf8");
  for (const { id } of NOTIFY_POLICIES) {
    assert.match(rust, new RegExp(`"${id}"`), `gui_prefs.rs does not know "${id}"`);
  }
});

test("a daemon that is simply down still gets the outage banner", () => {
  // AN UNREACHABLE DAEMON SENDS NO REPLY, so `response` is null on every tick and there is nothing
  // live to measure "nothing has synced for a day" against — which would have made the trigger
  // silent for the one outage most worth naming. The last epoch seen is remembered instead.
  const seen = decide({ state: emptyState(), view: view(), policy: "only_when_needed", nowMs: NOW_MS });
  assert.equal(seen.state.lastSeenSync, NOW_SECS - 60);

  const down = decide({
    state: { ...seen.state, sawUnsynced: true, said: { firstSync: "first" } },
    view: { daemonState: "unreachable", response: null, conflicts: [] },
    policy: "only_when_needed",
    nowMs: (NOW_SECS + OUTAGE_AFTER_SECS) * 1000,
  });
  assert.equal(down.event?.kind, "outage");
  assert.equal(down.event.cause, "unreachable");
  // …and the count is dropped rather than rendered as zero, because there is no reply to count.
  assert.equal(down.event.changes, null);
});

test("a deliberate pause is not an outage", () => {
  // `pause and resume` is one of the twelve categories that stay silent on purpose, and a folder
  // paused over a long weekend crosses the day threshold on its own.
  const day = { response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS, paused: true } };
  assert.equal(tick(day).event, null);
  assert.equal(tick({ ...day, daemonState: "paused" }).event, null);
  // The same age, not paused, still interrupts — or the test above would pass on a broken trigger.
  assert.equal(
    tick({ response: { last_sync_epoch_secs: NOW_SECS - OUTAGE_AFTER_SECS } }).event?.kind,
    "outage",
  );
});

test("a banner whose subject is gone comes down, and can be said again", () => {
  const first = tick({ conflicts: [{ path: "notes/todo.txt" }] });
  assert.equal(first.event.kind, "conflict");

  // Resolved in the window: nothing to say, and the live banner is asked to close.
  const cleared = decide({
    state: first.state,
    view: view(),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 2,
  });
  assert.equal(cleared.event, null);
  assert.equal(cleared.resolved, true);
  assert.equal(cleared.state.said.conflict, undefined, "the signature outlived its queue");

  // The identical conflict on another day is a new thing to say, not a repeat.
  const again = decide({
    state: cleared.state,
    view: view({ conflicts: [{ path: "notes/todo.txt" }] }),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 4,
  });
  assert.equal(again.event?.kind, "conflict");
});

test("the first sync is not un-said when it stops being current", () => {
  // Every other kind forgets its signature once its queue empties. `firstSync` may not: "once, at
  // the end of a long wait" is once, and it has no queue to empty.
  const watched = decide({
    state: emptyState(),
    view: view({ response: { last_sync_epoch_secs: null } }),
    policy: "only_when_needed",
    nowMs: NOW_MS,
  });
  const done = decide({
    state: watched.state,
    view: view(),
    policy: "only_when_needed",
    nowMs: NOW_MS + 60_000,
  });
  assert.equal(done.event?.kind, "firstSync");
  const later = decide({
    state: done.state,
    view: view(),
    policy: "only_when_needed",
    nowMs: NOW_MS + COALESCE_MS * 100,
  });
  assert.equal(later.event, null);
  assert.equal(later.state.said.firstSync, "first");
  // …and it does not ask for a banner to be closed either: nothing is waiting on it.
  assert.equal(later.resolved, false);
});

test("a daemon restart does not silence the outage trigger for ever", () => {
  // `last_sync_epoch_secs` DOES NOT SURVIVE A DAEMON RESTART: `ControlShared::new` starts it at
  // `None` and only a successful pass sets it. So a daemon that restarts and then cannot sync — an
  // expired session, a full disk, precisely what this trigger is for — reports a live `null` for
  // ever, and a trigger that measured only the live value would never cross any threshold at all.
  const seen = decide({ state: emptyState(), view: view(), policy: "only_when_needed", nowMs: NOW_MS });
  const restarted = decide({
    state: { ...seen.state, sawUnsynced: true, said: { firstSync: "first" } },
    view: view({ response: { last_sync_epoch_secs: null, pending_changes: 4 } }),
    policy: "only_when_needed",
    nowMs: (NOW_SECS + OUTAGE_AFTER_SECS) * 1000,
  });
  assert.equal(restarted.event?.kind, "outage");
  assert.equal(restarted.event.changes, 4);
});

test("a machine that has never synced gets no outage banner", () => {
  // The remembered value is what makes the test above work, and this is the state it must not
  // invent one for: nothing has ever synced, so there is nothing to have stopped. That is
  // onboarding's, and `firstRun` is what draws it.
  const fresh = decide({
    state: emptyState(),
    view: view({ response: { last_sync_epoch_secs: null } }),
    policy: "only_when_needed",
    nowMs: (NOW_SECS + OUTAGE_AFTER_SECS) * 1000,
  });
  assert.equal(fresh.event, null);
});
