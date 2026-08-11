// `trayView` (S8) — which panel the tray shows, from a status reply.
//
// The fidelity gate compares the four drawn `10a` panels and cannot say anything about WHICH of them
// a given reply should produce. That mapping is the whole of this module and every bug in it is
// silent: the panel renders, the menu opens, and the sentence is about a different moment than the
// one the user is in.
//
// The two states below that no frame draws are the reason this file exists. Both would otherwise
// reach a 362px window with no takeover in front of them and no reviewer looking at them.

import { test } from "node:test";
import assert from "node:assert/strict";
import { trayView } from "../src/js/screens/tray.js";
import { TRAY_MENU } from "../src/js/ui/compact.js";
import { MAIN, TRAY } from "../src/js/ui/copy.js";

const reply = (over = {}) => ({
  paused: false,
  syncing: false,
  pending_changes: 0,
  last_sync_epoch_secs: 1_800_000_000,
  ...over,
});

test("every daemon state lands on a form the panel draws and rows the menu has", () => {
  // `trayMenu` THROWS on a key it does not know, and this panel lives in a borderless window with no
  // devtools and no error surface — an unmapped state is a tray that silently stops opening. So the
  // exhaustive check is the point, not the individual answers.
  const states = ["idle", "running", "paused", "authExpired", "unreachable", "firstRun"];
  for (const daemonState of states) {
    const view = trayView({ daemonState, response: reply() });
    assert.ok(view.state, `${daemonState} produced no panel state`);
    assert.ok(TRAY_MENU[view.menuState], `${daemonState} → menu "${view.menuState}" has no rows`);
    assert.ok(view.headline, `${daemonState} produced no headline`);
  }
});

test("a daemon that has never synced does not say everything is up to date", () => {
  // The bug this whole branch exists to prevent. S1's derivation has no `firstRun` case — the
  // onboarding takeover intercepts it before the main screen renders — so falling through to it
  // would draw the settled hexagon and `Up to date` over a daemon that has never copied a file.
  const view = trayView({ daemonState: "firstRun", response: reply() });
  assert.notEqual(view.headline, MAIN.compact.upToDate);
  assert.equal(view.headline, TRAY.nothingSyncedYet);
  assert.equal(view.state, "needsYou");
  // No numeral: nothing is waiting, and a `0` inside the mark would present an empty queue as a
  // decision. `renderHexagon` draws no <text> node at all for a null.
  assert.equal(view.count, null);
});

test("an expired session keeps the struck mark and loses the row that cannot fix it", () => {
  const view = trayView({ daemonState: "authExpired", response: reply({ pending_changes: 61 }) });
  // The form is shared with `unreachable` — 11-notifications.md puts an outage and an expired
  // session behind one struck icon.
  assert.equal(view.state, "unreachable");
  // The sentence is not. This is the deck's own split of the outage banner's two sentences.
  assert.equal(view.headline, MAIN.authExpired);
  assert.equal(view.sub, MAIN.authExpiredSub(61));
  // And the menu is not: `Try again now` retries a sync, which is not what an expired session needs.
  assert.equal(view.menuState, "deferToWindow");
  assert.ok(!TRAY_MENU[view.menuState].some((row) => row.label === TRAY.tryAgain));
});

test("an unreachable daemon drops the count clause rather than claiming zero", () => {
  // 14-behaviour-and-state.md: a null summary means unknown, never zero. `0 changes are waiting` is
  // a false all-clear at the exact moment the app cannot see anything — and the reply that would
  // carry the count is the one that did not arrive.
  const view = trayView({ daemonState: "unreachable", response: null });
  assert.equal(view.headline, TRAY.unreachableTitle);
  assert.equal(view.sub, "Nothing is lost.");
  assert.equal(view.menuState, "unreachable");
  // `retrying in 40s · last reached 13:58` is drawn and nothing in the reply can produce it.
  assert.equal(view.meta, null);
});

test("queued work reads as syncing, exactly as the window reads it", () => {
  // The rule S1 paid for: a filesystem-watch event accumulates `pending_changes` without starting a
  // reconcile, so for up to a scan interval the daemon reports `syncing: false` with a non-empty
  // queue. If the tray disagreed with the window here, the two would describe the same file
  // differently at the same moment — which is the reason this module imports `heroStateOf`.
  const view = trayView({ daemonState: "running", response: reply({ syncing: false, pending_changes: 3 }) });
  assert.equal(view.state, "syncing");
  assert.equal(view.headline, MAIN.syncing(3));
});

test("the count in the mark is the plan's transfers, not the watch queue", () => {
  // A pass driven entirely by Proton carries an empty local queue while downloading, and the
  // headline used to read `Syncing 0 changes` with a literal 0 inside the mark.
  const view = trayView({
    daemonState: "running",
    response: reply({
      syncing: true,
      pending_changes: 0,
      last_plan_summary: { uploads: 1, downloads: 4, conflicts: 0, destructive_actions: 0 },
    }),
  });
  assert.equal(view.count, 5);
  assert.equal(view.headline, MAIN.syncing(5));
});

test("paused outranks a decision, and unreachable outranks everything", () => {
  // Both orderings are S1's and both are load-bearing: "nothing will move until you resume" is true
  // of the decisions too, and an unreachable daemon makes every other number on the panel stale.
  const waiting = { conflicts: [{}], deletions: [{}, {}] };
  assert.equal(
    trayView({ daemonState: "paused", response: reply({ paused: true }), ...waiting }).state,
    "paused",
  );
  assert.equal(trayView({ daemonState: "unreachable", response: null, ...waiting }).state, "unreachable");
  // With neither, the decisions surface — and bring the count and the button with them.
  const decision = trayView({ daemonState: "idle", response: reply(), ...waiting });
  assert.equal(decision.state, "needsYou");
  assert.equal(decision.count, 3);
  assert.equal(decision.action.label, MAIN.compact.review);
});

test("only the syncing panel carries transfer rows", () => {
  const transfer = { path: "docs/spec.md", direction: "upload", bytes_done: 32, bytes_total: 64 };
  const syncing = trayView({
    daemonState: "running",
    response: reply({ syncing: true, pending_changes: 1, activity: { phase: "executing", transfer } }),
  });
  assert.deepEqual(syncing.transfers, [{ direction: "up", name: "docs/spec.md", progress: 0.5 }]);
  // The same activity on a paused daemon draws no rows — the panel would otherwise show a file
  // moving under the sentence "nothing will move until you resume".
  const paused = trayView({
    daemonState: "paused",
    response: reply({ paused: true, activity: { phase: "executing", transfer } }),
  });
  assert.deepEqual(paused.transfers, []);
});

test("a download with no known size gets a row and no progress track", () => {
  // `null` means "no track", not 0%. A remote listing carries no size, so this is the ordinary case
  // for anything arriving — and a 0% bar that never moves reads as a stall.
  const view = trayView({
    daemonState: "running",
    response: reply({
      syncing: true,
      pending_changes: 1,
      activity: { phase: "executing", transfer: { path: "q3.pdf", direction: "download" } },
    }),
  });
  assert.equal(view.transfers[0].direction, "down");
  assert.equal(view.transfers[0].progress, null);
});
