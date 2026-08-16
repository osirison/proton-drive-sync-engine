// `trayView` (S8) — which panel the tray shows, from a status reply.
//
// The fidelity gate compares the four drawn `10a` panels and cannot say anything about WHICH of them
// a given reply should produce. That mapping is the whole of this module and every bug in it is
// silent: the panel renders, the menu opens, and the sentence is about a different moment than the
// one the user is in.
//
// The three states below that no frame draws are the reason this file exists — `firstRun`,
// `authExpired` and now `failed` (#246). Each would otherwise reach a 362px window with no takeover
// in front of it and no reviewer looking at it, and two of the three fall through to the SETTLED
// copy if nothing maps them, which is the false all-clear in the surface nobody inspects.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { trayView } from "../src/js/screens/tray.js";
import { TRAY_MENU, menuSignature } from "../src/js/ui/compact.js";
import { MAIN, TRAY } from "../src/js/ui/copy.js";

/**
 * Every `DaemonState`, READ OFF THE RUST rather than typed here again.
 *
 * The enum is the webview's input and it lives in another language, so nothing but this makes a
 * variant added there fail anything over here. That is not hypothetical: `trayMenu` THROWS on a key
 * it does not know, so a state the daemon can produce and the tray cannot map is a panel that stops
 * opening at all — in a borderless window with no devtools. The list was hand-written until `failed`
 * (#246) was added on the Rust side and this file would have gone on testing the six it knew about.
 *
 * `serde(rename_all = "camelCase")` is what the wire carries, so the variant names are lowered the
 * same way here.
 */
const DAEMON_STATES = (() => {
  const source = readFileSync(fileURLToPath(new URL("../gui-core/src/state.rs", import.meta.url)), "utf8");
  const body = source.match(/pub enum DaemonState \{([\s\S]*?)\n\}/);
  assert.ok(body, "could not find `pub enum DaemonState` in gui-core/src/state.rs");
  const names = [...body[1].matchAll(/^ {4}([A-Z][A-Za-z]*),$/gm)].map(
    ([, name]) => name[0].toLowerCase() + name.slice(1),
  );
  assert.ok(names.length >= 6, `only found ${names.length} variants — the regex has drifted`);
  return names;
})();

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
  for (const daemonState of DAEMON_STATES) {
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

test("a failed pass keeps the struck mark and KEEPS the row that can fix it", () => {
  // #246's tray half, and the mirror image of the test above it. Same struck form — a failed pass is
  // the third of `11-notifications.md`'s one-icon trio — and the opposite menu answer: this daemon
  // IS answering, so `Try again now` reaches it and runs the pass that failed.
  const view = trayView({
    daemonState: "failed",
    response: reply({ pending_changes: 4, last_error: "proton-drive list failed: os error 2" }),
  });
  assert.equal(view.state, "unreachable");
  assert.notEqual(view.headline, MAIN.compact.upToDate, "the false all-clear #246 is about");
  assert.equal(view.headline, MAIN.failed);
  assert.equal(view.sub, MAIN.failedSub(4));
  // `outage`, which is what this set is called now that it serves this state ALONE. It was
  // `unreachable`, shared with the daemon-is-not-running state, where the same `Try again now`
  // dispatched a `Syncnow` at a socket that had just refused the connection.
  assert.equal(view.menuState, "outage");
  assert.ok(TRAY_MENU[view.menuState].some((row) => row.label === TRAY.tryAgain));
  // The daemon's string is NOT in the panel: 362px has no block to quote one in, and a truncated
  // stderr is the paraphrase voice rule 4 forbids. The window carries it.
  assert.ok(!JSON.stringify(view).includes("os error 2"));
});

test("a failed pass with an empty queue keeps the reassurance and drops the count", () => {
  // `0 changes are waiting` reads as an all-clear in the one place that must not give one, and the
  // queue is genuinely empty on a pass driven from Proton's side. The reassurance survives alone.
  const view = trayView({ daemonState: "failed", response: reply({ pending_changes: 0 }) });
  assert.equal(view.sub, "Nothing is lost.");
});

test("the panel form does not identify the menu, so a patch cannot be gated on it alone", () => {
  // The desync `data-menu` exists for. `updateCompactPanel` rejects a patch on `data-state`, which
  // is the panel FORM — and the form and the rows are deliberately two different mappings of one
  // daemon state ("the panel takes the form and the menu takes the cause"). Three hero states share
  // the struck form; two of them want different rows. So `failed` → `authExpired` between two polls
  // patched `Proton Drive is asking you to sign in again` over a menu still offering `Try again
  // now` — the row MENU_STATE's own comment forbids. DEVIATIONS §90f, found by review.
  const failed = trayView({ daemonState: "failed", response: reply({ last_error: "os error 2" }) });
  const expired = trayView({ daemonState: "authExpired", response: reply() });
  assert.equal(failed.state, expired.state, "the premise: one form");
  assert.notEqual(failed.menuState, expired.menuState, "and two menus");
  // Which is what the signature has to separate, since the form cannot.
  assert.notEqual(menuSignature(TRAY_MENU[failed.menuState]), menuSignature(TRAY_MENU[expired.menuState]));
  // Ids, not labels: a click dispatches on the id, and two sets can draw the same words.
  assert.match(menuSignature(TRAY_MENU[failed.menuState]), /tryAgain/);
  assert.doesNotMatch(menuSignature(TRAY_MENU[expired.menuState]), /tryAgain/);
  // Separators count — a set that lost one is a different menu, and every id would still match.
  assert.notEqual(
    menuSignature(TRAY_MENU.settled),
    menuSignature(TRAY_MENU.settled.filter((r) => !r.separator)),
  );
});

test("a daemon that is not running says so, and is offered the row that starts it", () => {
  // `derive_state` answers `unreachable` for ONE thing: the control socket did not answer. Proton is
  // not on the far end of that round trip, so the deck's `Can't reach Proton Drive` was a diagnosis
  // of the wrong machine — and the menu under it offered `Try again now`, a `Syncnow` down the very
  // socket that had just refused. Both halves are fixed here. DEVIATIONS §95.
  const view = trayView({ daemonState: "unreachable", response: null });
  assert.equal(view.headline, MAIN.notRunning);
  assert.notEqual(view.headline, TRAY.unreachableTitle, "the outage sentence belongs to `failed`");
  assert.equal(view.sub, MAIN.notRunningSub);
  assert.equal(view.menuState, "notRunning");
  const rows = TRAY_MENU[view.menuState];
  assert.ok(rows.some((row) => row.label === TRAY.start));
  assert.ok(!rows.some((row) => row.label === TRAY.tryAgain), "nothing to retry against");
  // 14-behaviour-and-state.md: a null summary means unknown, never zero. `0 changes are waiting` is
  // a false all-clear at the exact moment the app cannot see anything — and the reply that would
  // carry the count is the one that did not arrive. Met here by having no clause to fill.
  assert.ok(!/\bchanges?\b/.test(view.sub), view.sub);
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

test("the panel takes two rows however long the window is", () => {
  // Unreachable before #211 — the reply could describe one transfer, so nothing ever handed this
  // panel a list. It can now hand it six, and `10a Syncing` draws two in a 362px panel whose height
  // sizes the tray window, with no `+n more` line to absorb the rest.
  const transfers = ["a", "b", "c", "d", "e", "f"].map((name) => ({
    direction: "download",
    path: `${name}.bin`,
    state: name === "a" ? "active" : "queued",
  }));
  const view = trayView({
    daemonState: "running",
    response: reply({
      syncing: true,
      pending_changes: 6,
      activity: { phase: "executing", transfers, transfers_remaining: 6 },
    }),
  });
  assert.equal(view.transfers.length, 2);
  assert.deepEqual(
    view.transfers.map((t) => t.name),
    ["a.bin", "b.bin"],
    "the first two of the window, in flight order",
  );
  // The count is not lost with the rows: the headline still says how many changes there are.
  assert.equal(view.count, 6);
});

test("only the syncing panel carries transfer rows", () => {
  const transfer = { path: "docs/spec.md", direction: "upload", bytes_done: 32, bytes_total: 64 };
  const syncing = trayView({
    daemonState: "running",
    response: reply({ syncing: true, pending_changes: 1, activity: { phase: "executing", transfer } }),
  });
  // No `detail`: the panel's rows are flat and neither drawn compact row carries a size chip, which
  // is the one thing the shared mapper does differently for the tray (`compact: true`).
  assert.deepEqual(syncing.transfers, [
    { direction: "up", name: "docs/spec.md", detail: null, state: "active", progress: 0.5, files: null },
  ]);
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
