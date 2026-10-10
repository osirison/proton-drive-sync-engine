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

// ---- two folders or more (#102 phase 5d) --------------------------------------------------------
//
// Below two the panel is exactly what it was (`pair` and `menuRows` are `null`); at two or more it is
// the WORST folder's own hero, named, over a pause row for each folder. The worst folder is the one
// with the highest `rank` — Rust's `severity`, carried on every `pair_states` entry — so none of this
// ranks anything.

const summary = (name, over = {}) => ({
  name,
  local_root: `/home/u/${name}`,
  remote_root: `/Drive/${name}`,
  db_path: `/home/u/${name}/.sync/sync_index.db`,
  paused: false,
  syncing: false,
  reconcile_seq: 1,
  last_sync_epoch_secs: 1_800_000_000,
  last_error: null,
  pending_changes: 0,
  pending_deletions: 0,
  ...over,
});

/** `[name, state, rank]` → the `pair_states` entries the payload carries. */
const derived = (...rows) => rows.map(([name, state, rank]) => ({ name, state, rank }));

test("below two folders the panel is the one it always was", () => {
  for (const pairs of [[], [summary("docs")]]) {
    const view = trayView({
      daemonState: "idle",
      response: reply({ pair: "docs" }),
      pairs,
      pairStates: derived(...pairs.map((p) => [p.name, "idle", 0])),
    });
    assert.equal(view.pair, null);
    assert.equal(view.menuRows, null, "so the fixed rows are drawn");
    assert.equal(view.headline, MAIN.compact.upToDate);
  }
});

test("two folders show the worst one's own panel, named, with a pause row for each", () => {
  // `documents` (the folder the reply describes) is fine and `photos` failed: the panel is photos'.
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [summary("documents"), summary("photos", { last_error: "boom", pending_changes: 4 })],
    pairStates: derived(["documents", "idle", 0], ["photos", "failed", 5]),
  });
  assert.equal(view.pair, "photos");
  assert.equal(view.headline, MAIN.failed);
  assert.equal(view.sub, MAIN.failedSub(4));
  assert.equal(view.menuState, "outage");
  const ids = view.menuRows.filter((row) => !row.separator).map((row) => row.id);
  assert.deepEqual(ids, ["tryAgain", "open", "pause@photos", "pause@documents", "quit"]);
  // The daemon's string is not in the panel (voice rule 4), at several folders as at one.
  assert.ok(!JSON.stringify(view).includes("boom"));
});

test("the folder the reply describes keeps what only a full reply has", () => {
  // The described folder is the worst here (it is syncing and the other is idle), so the panel is its
  // own with its live transfer rows — a summary has none.
  const transfer = { path: "docs/spec.md", direction: "upload", bytes_done: 32, bytes_total: 64 };
  const view = trayView({
    daemonState: "running",
    response: reply({
      pair: "documents",
      syncing: true,
      pending_changes: 1,
      activity: { phase: "executing", transfer },
    }),
    pairs: [summary("documents", { syncing: true, pending_changes: 1 }), summary("photos")],
    pairStates: derived(["documents", "running", 3], ["photos", "idle", 0]),
  });
  assert.equal(view.pair, "documents");
  assert.equal(view.state, "syncing");
  assert.equal(view.transfers.length, 1);
});

test("another folder syncing is drawn from its summary: its count, and no rows it cannot know", () => {
  const view = trayView({
    daemonState: "idle",
    // The reply is about `documents`, which is idle and carries no activity; `photos` is the one moving.
    response: reply({ pair: "documents" }),
    pairs: [summary("documents"), summary("photos", { syncing: true, pending_changes: 7 })],
    pairStates: derived(["documents", "idle", 0], ["photos", "running", 3]),
  });
  assert.equal(view.pair, "photos");
  assert.equal(view.state, "syncing");
  assert.equal(view.headline, MAIN.syncing(7));
  assert.deepEqual(view.transfers, []);
  // The aggregate is `Running`, so the syncing set: `Sync now` is still there because `documents` is idle.
  assert.equal(view.menuState, "syncing");
  assert.ok(view.menuRows.some((row) => row.id === "syncNow"));
});

test("folders that tie put the first the daemon lists first", () => {
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [summary("documents"), summary("photos")],
    pairStates: derived(["documents", "idle", 0], ["photos", "idle", 0]),
  });
  assert.equal(view.pair, "documents");
  assert.equal(view.headline, MAIN.compact.upToDate);
});

test("a paused folder is the panel's folder while nothing outranks it, and offers Resume", () => {
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [summary("documents"), summary("photos", { paused: true, pending_changes: 7 })],
    pairStates: derived(["documents", "idle", 0], ["photos", "paused", 1]),
  });
  assert.equal(view.pair, "photos");
  assert.equal(view.state, "paused");
  assert.equal(view.headline, MAIN.paused);
  const ids = view.menuRows.filter((row) => !row.separator).map((row) => row.id);
  assert.ok(ids.includes("resume@photos") && ids.includes("pause@documents"), ids.join());
  // Not paused as a whole: `documents` still syncs, so `Sync now` and `Close window` are there.
  assert.ok(ids.includes("syncNow") && ids.includes("closeWindow"));
});

test("a payload that cannot say what state a folder is in is not drawn as several folders", () => {
  // Defaulting the missing one to `idle` would be the false all-clear (#246) in the surface nobody
  // inspects. The panel simply makes no claim about several folders.
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [summary("documents"), summary("photos", { last_error: "boom" })],
    pairStates: derived(["documents", "idle", 0]),
  });
  assert.equal(view.pair, null);
  assert.equal(view.menuRows, null);
});

test("the folder rows go through the same cap the corpus asserts", () => {
  const names = ["a", "b", "c", "d", "e", "f", "g"];
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "a" }),
    pairs: names.map((name) => summary(name)),
    pairStates: derived(...names.map((name) => [name, "idle", 0])),
  });
  const ids = view.menuRows.filter((row) => !row.separator).map((row) => row.id);
  assert.equal(ids.filter((id) => id.startsWith("pause@")).length, 5);
  assert.ok(ids.includes("more"));
});

// ---- what `Needs you` counts at several folders (review of #443) -----------------------------------------
//
// `10-tray.md` says the panel counts "the deletions across folders, and conflicts only for the folder it
// scans". It counted the panel folder's own, so a withheld deletion in another folder was hidden behind
// `Up to date` or `Paused` with no way to reach it. These hold the sentence to the code.

const three = [{ path: "a" }, { path: "b" }, { path: "c" }];

test("deletions waiting in another folder are not hidden behind Up to date", () => {
  // Two idle folders; the one the panel names first (`documents`) has nothing, `photos` has three.
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [summary("documents"), summary("photos", { pending_deletions: 3 })],
    pairStates: derived(["documents", "idle", 0], ["photos", "idle", 0]),
  });
  assert.equal(view.state, "needsYou");
  assert.equal(view.count, 3);
  assert.equal(view.headline, MAIN.compact.needYou(3));
  assert.equal(view.pair, "photos", "the panel is the folder that holds them, not the one beside it");
  assert.equal(view.action.label, MAIN.compact.review);
  assert.equal(view.action.id, "review@photos", "and Review opens the window on that folder");
});

test("deletions waiting in a folder are not hidden behind another folder's pause", () => {
  // `documents` holds three; `photos` is paused, which outranks idle — and used to be the whole panel.
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    conflicts: [],
    deletions: three,
    pairs: [summary("documents", { pending_deletions: 3 }), summary("photos", { paused: true })],
    pairStates: derived(["documents", "idle", 0], ["photos", "paused", 1]),
  });
  assert.equal(view.state, "needsYou");
  assert.equal(view.count, 3);
  assert.equal(view.pair, "documents");
  assert.equal(view.action.id, "review@documents");
  // The paused folder is still one tap away: its row is in the group, offering Resume.
  const ids = view.menuRows.filter((row) => !row.separator).map((row) => row.id);
  assert.ok(ids.includes("resume@photos"), ids.join());
});

test("a decision never outranks a folder that is moving or wrong", () => {
  // The paused/idle exception is deliberately that narrow: syncing, a failed pass and the rest keep
  // their panel (at one folder, `Syncing…` has no `Review them` either).
  for (const [state, rank, over, form] of [
    ["running", 3, { syncing: true, pending_changes: 4 }, "syncing"],
    ["failed", 5, { last_error: "boom", pending_changes: 4 }, "unreachable"],
  ]) {
    const view = trayView({
      daemonState: "idle",
      response: reply({ pair: "documents" }),
      pairs: [summary("documents", { pending_deletions: 3 }), summary("photos", over)],
      pairStates: derived(["documents", "idle", 0], ["photos", state, rank]),
    });
    assert.equal(view.pair, "photos", state);
    assert.equal(view.state, form, state);
    assert.equal(view.action, undefined, `${state}: no Review over a panel that is not about a decision`);
  }
});

test("a paused folder keeps its pause, whatever it holds, and is not preferred for holding it", () => {
  // Its own deletions wait behind the pause exactly as they do at one folder ("paused outranks a
  // decision"), and no OTHER folder is up to date with a decision to put in front of it. Two paused
  // folders: the panel is the first the daemon lists, not the one that happens to hold the deletions.
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [
      summary("documents"),
      summary("photos", { paused: true }),
      summary("music", { paused: true, pending_deletions: 3 }),
    ],
    pairStates: derived(["documents", "idle", 0], ["photos", "paused", 1], ["music", "paused", 1]),
  });
  assert.equal(view.pair, "photos");
  assert.equal(view.state, "paused");
  assert.equal(view.action, undefined);
});

test("the count is every folder's deletions and the scanned folder's conflicts, each once", () => {
  const view = trayView({
    daemonState: "idle",
    // The reply describes `documents`: its two conflicts come from the disk walk the tray runs for it, and
    // its three deletions are in BOTH the reply's list and its summary — one set of deletions, not two.
    response: reply({ pair: "documents" }),
    conflicts: [{ original: "x" }, { original: "y" }],
    deletions: three,
    pairs: [
      summary("documents", { pending_deletions: 3 }),
      summary("photos", { pending_deletions: 4 }),
      summary("music"),
    ],
    pairStates: derived(["documents", "idle", 0], ["photos", "idle", 0], ["music", "idle", 0]),
  });
  assert.equal(view.count, 2 + 3 + 4);
  assert.equal(view.pair, "documents", "the first the daemon lists that holds a decision");
  assert.equal(view.action.id, "review@documents");
});

test("a conflict is the scanned folder's, never another folder's", () => {
  // The walk is for `documents`. A reply describing `photos` that carries a conflict list does not put
  // those conflicts on `documents`'s account.
  const view = trayView({
    daemonState: "idle",
    response: reply({ pair: "photos" }),
    conflicts: [{ original: "x" }],
    pairs: [summary("documents"), summary("photos")],
    pairStates: derived(["documents", "idle", 0], ["photos", "idle", 0]),
  });
  assert.equal(view.count, 1);
  assert.equal(view.pair, "photos", "the scanned folder holds it");
});

test("Review names its folder at several folders and is the bare id at one", () => {
  const one = trayView({
    daemonState: "idle",
    response: reply({ pair: "docs" }),
    deletions: three,
    pairs: [summary("docs", { pending_deletions: 3 })],
    pairStates: derived(["docs", "idle", 0]),
  });
  assert.equal(one.pair, null);
  assert.equal(one.state, "needsYou");
  assert.equal(one.action.id, "review", "unchanged below two folders");
  assert.equal(one.count, 3);
});

test("a summary that carries no deletion count counts none, and spoils nobody else's", () => {
  const bare = summary("documents");
  delete bare.pending_deletions;
  const alone = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [bare, summary("photos")],
    pairStates: derived(["documents", "idle", 0], ["photos", "idle", 0]),
  });
  assert.equal(alone.state, "settled");
  assert.equal(alone.action, undefined);

  // …and beside a folder that does hold some, the count is theirs and a number (an absent field read as
  // `undefined` turns the whole sum into NaN, which compares false against everything).
  const beside = trayView({
    daemonState: "idle",
    response: reply({ pair: "documents" }),
    pairs: [bare, summary("photos", { pending_deletions: 3 })],
    pairStates: derived(["documents", "idle", 0], ["photos", "idle", 0]),
  });
  assert.equal(beside.state, "needsYou");
  assert.equal(beside.count, 3);
  assert.equal(beside.pair, "photos");
});

// ---- a paused hero names its folder at several folders (review of #443) -----------------------------------

test("a paused folder's sub-line names the folder at several folders and is the old sentence at one", () => {
  const paused = (pairs, pairStates) =>
    trayView({
      daemonState: "paused",
      response: reply({ pair: "documents", paused: true, pending_changes: 7 }),
      pairs,
      pairStates,
    });

  const many = paused(
    [summary("documents", { paused: true, pending_changes: 7 }), summary("photos")],
    derived(["documents", "paused", 1], ["photos", "idle", 0]),
  );
  assert.equal(many.state, "paused");
  assert.equal(many.pair, "documents");
  assert.match(
    many.sub,
    /^7 changes have piled up since .*\. Nothing in documents will move until you resume\.$/,
  );
  assert.equal(many.sub, TRAY.pausedSubPair(7, many.sub.match(/since (.*?)\./)[1], "documents"));

  // One folder, or a daemon that lists none: the sentence this panel has always said.
  for (const pairs of [[], [summary("documents", { paused: true, pending_changes: 7 })]]) {
    const one = paused(pairs, derived(...pairs.map((p) => [p.name, "paused", 1])));
    assert.equal(one.pair, null);
    assert.match(one.sub, /Nothing will move until you resume\.$/);
    assert.ok(!one.sub.includes("documents"));
  }
});

test("the two paused sentences share their first half and differ only in the folder", () => {
  assert.equal(
    MAIN.pausedSub(7, "13:20"),
    "7 changes have piled up since 13:20. Nothing will move until you resume.",
  );
  assert.equal(
    TRAY.pausedSubPair(7, "13:20", "documents"),
    "7 changes have piled up since 13:20. Nothing in documents will move until you resume.",
  );
  // The counted form agrees with the singular, in both.
  assert.match(MAIN.pausedSub(1, "13:20"), /^1 change has piled up/);
  assert.match(TRAY.pausedSubPair(1, "13:20", "photos"), /^1 change has piled up/);
});

// ---- a folder that has not had its turn (#455 and the live report) ----------------------------------------
//
// Beside other folders a folder with no finished pass is `queued`, never `idle` and never the wizard's
// `firstRun`. Its panel says so and names what it waits for; it offers no `Open Drive Sync` button, because
// the sentence that button belonged to ("choose your two folders") is addressed to someone who has not.

/** `[name, state, rank, waiting_for?]` → `pair_states` entries as Rust sends them. */
const derivedWaiting = (...rows) =>
  rows.map(([name, state, rank, waiting_for]) => ({
    name,
    state,
    rank,
    ...(waiting_for ? { waiting_for } : {}),
  }));

test("a_folder_waiting_behind_a_paused_pass_is_a_panel_that_names_it_and_the_folder_it_waits_for", () => {
  // A pause does not stop a pass already running, so `documents` is paused (rank 1) and still the one
  // `photos` waits for. `photos` (rank 2) is the worst folder, and the panel is its own.
  const view = trayView({
    daemonState: "paused",
    response: reply({ pair: "documents", paused: true }),
    pairs: [
      summary("documents", { paused: true, syncing: true }),
      summary("photos", { last_sync_epoch_secs: null }),
    ],
    pairStates: derivedWaiting(["documents", "paused", 1], ["photos", "queued", 2, "documents"]),
  });
  assert.equal(view.pair, "photos");
  assert.equal(view.headline, "Waiting for documents");
  assert.equal(view.sub, "Folders sync one at a time. photos is waiting for documents to finish.");
  assert.notEqual(view.headline, MAIN.compact.upToDate);
  assert.equal(view.state, "needsYou", "the form a folder with no pass has always had: it has not synced");
  assert.equal(view.count, null, "a count inside the mark would be a queue of zero things");
  assert.equal(view.action, undefined, "nothing to open the window for, and nothing to decide");
  // Every folder keeps its pause row.
  const ids = view.menuRows.filter((row) => !row.separator).map((row) => row.id);
  assert.deepEqual(ids, ["open", "syncNow", "pause@photos", "resume@documents", "closeWindow", "quit"]);
});

test("with_nothing_running_a_folder_without_a_pass_is_a_starting_panel", () => {
  const view = trayView({
    daemonState: "queued",
    response: reply({ pair: "documents", last_sync_epoch_secs: null }),
    pairs: [
      summary("documents", { last_sync_epoch_secs: null }),
      summary("photos", { last_sync_epoch_secs: null }),
    ],
    pairStates: derivedWaiting(["documents", "queued", 2], ["photos", "queued", 2]),
  });
  assert.equal(view.pair, "documents", "ties go to the first the daemon lists");
  assert.equal(view.headline, "Starting to sync");
  assert.equal(view.sub, "documents starts on its own.");
  assert.equal(view.action, undefined);
  assert.equal(view.menuState, "settled");
  const ids = view.menuRows.filter((row) => !row.separator).map((row) => row.id);
  assert.ok(ids.includes("pause@documents") && ids.includes("pause@photos"), ids.join());
});

test("one_folder_keeps_the_first_run_panel_it_always_had", () => {
  // N=1: the wizard's state, the tray's two sentences and the button that opens the window.
  const view = trayView({ daemonState: "firstRun", response: reply({ last_sync_epoch_secs: null }) });
  assert.equal(view.headline, TRAY.nothingSyncedYet);
  assert.equal(view.sub, TRAY.nothingSyncedYetSub);
  assert.equal(view.action.id, "open");
  assert.equal(view.pair, null);
});

test("a_queued_folder_beside_a_running_one_leaves_the_running_panel_in_front", () => {
  const view = trayView({
    daemonState: "running",
    response: reply({ pair: "documents", syncing: true }),
    pairs: [summary("documents", { syncing: true }), summary("photos", { last_sync_epoch_secs: null })],
    pairStates: derivedWaiting(["documents", "running", 3], ["photos", "queued", 2, "documents"]),
  });
  assert.equal(view.pair, "documents");
  assert.equal(view.state, "syncing");
});
