// The folder selector's pure half, and what the main screen says about two folders (#102 phase 5c-1).
//
// `selector.js`, `chrome.js` and `main.js` build DOM, and `node --test` has none — what they DRAW is held
// by the fidelity gate (eight new frames) and by `fidelity:pairs`, which drives the real page. What is
// here is the part that is a decision rather than a rendering: which folders the list names and with
// which word, when another folder is "waiting" (the ring on the pill), what the hero's buttons say, and
// what the notice block says for each of the three things it can be about. Each fails quietly.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  markerOf,
  othersProblem,
  othersWaiting,
  pillLabel,
  selectorRows,
  stateWordOf,
} from "../src/js/ui/selector.js";
import { headlineOf, heroActionsOf, mainView, noticeOf, subOf } from "../src/js/screens/main.js";
import { CHROME, MAIN, TRAY } from "../src/js/ui/copy.js";
import * as store from "../src/js/store.js";

/** Names a plain object answers for: every one is a legal folder name. */
const AWKWARD = ["constructor", "toString", "hasOwnProperty", "valueOf", "isPrototypeOf", "__proto__"];

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

// ---- the list ---------------------------------------------------------------------------------

test("a_state_is_one_word_and_an_unknown_state_is_none", () => {
  assert.equal(stateWordOf("idle"), "up to date");
  assert.equal(stateWordOf("running"), "syncing");
  assert.equal(stateWordOf("paused"), "paused");
  assert.equal(stateWordOf("failed"), "sync failed");
  // Exhaustive over what Rust serialises: nothing the deck has a word for is left without one.
  for (const state of ["idle", "running", "paused", "failed", "authExpired", "unreachable", "firstRun"]) {
    assert.ok(stateWordOf(state), `${state} has a word`);
  }
  // A state this build was never told about says NOTHING — not `up to date` (#246), not a made-up word.
  for (const state of [null, undefined, "", "weird", ...AWKWARD]) {
    assert.equal(stateWordOf(state), "", `${String(state)} is not a state`);
  }
});

test("the_rows_are_the_daemons_folders_in_its_order_each_with_the_state_rust_derived", () => {
  const rows = selectorRows({
    pairs: [summary("documents"), summary("photos"), summary("music")],
    pairStates: [
      { name: "photos", state: "running", rank: 2 },
      { name: "documents", state: "idle", rank: 0 },
    ],
    selected: "photos",
    waiting: (name) => (name === "music" ? 3 : 0),
  });
  assert.deepEqual(
    rows.map((row) => [row.name, row.selected, row.state, row.waiting]),
    [
      ["documents", false, "idle", 0],
      ["photos", true, "running", 0],
      // A folder Rust derived no state for has none — the row will draw no word for it.
      ["music", false, null, 3],
    ],
  );
});

test("a_folder_named_like_an_object_property_gets_its_own_state_and_its_own_row", () => {
  // `pairStates` is read by NAME. An object keyed by it answers `constructor` from `Object.prototype`.
  for (const name of AWKWARD) {
    const rows = selectorRows({
      pairs: [summary("documents"), summary(name)],
      pairStates: [{ name: "documents", state: "idle", rank: 0 }],
      selected: "documents",
    });
    assert.equal(rows[1].name, name);
    assert.equal(rows[1].state, null, `${name} has no derived state, and must not borrow a function's`);
    assert.equal(stateWordOf(rows[1].state), "");
  }
  const withState = selectorRows({
    pairs: [summary("constructor")],
    pairStates: [{ name: "constructor", state: "paused", rank: 1 }],
    selected: "constructor",
  });
  assert.equal(withState[0].state, "paused");
});

// ---- the ring ---------------------------------------------------------------------------------

test("the_ring_is_for_another_folder_and_only_for_a_decision_waiting_there", () => {
  const rows = (waiting) =>
    ["documents", "photos"].map((name) => ({
      name,
      selected: name === "documents",
      state: "idle",
      waiting: waiting[name],
    }));
  assert.equal(othersWaiting(rows({ documents: 0, photos: 0 })), false);
  assert.equal(othersWaiting(rows({ documents: 0, photos: 2 })), true);
  // THE SELECTED FOLDER'S OWN QUEUE IS THE CHIP'S AND THE BAND'S, never the pill's (decision D4).
  assert.equal(
    othersWaiting(rows({ documents: 5, photos: 0 })),
    false,
    "a decision waiting in the folder on screen is already the chip and the band — the pill does not repeat it",
  );
  assert.equal(othersWaiting([]), false);
});

test("the_pill_says_which_folder_and_that_another_is_asking_for_a_person", () => {
  assert.equal(pillLabel("documents", null), "Folder documents");
  assert.match(
    pillLabel("documents", "decision"),
    /^Folder documents\. Another folder has something waiting\.$/,
  );
  assert.match(pillLabel("documents", "problem"), /^Folder documents\. Another folder has a problem\.$/);
});

// ---- the second form of the marker: another folder has a PROBLEM (maintainer decision, #102, 2026-10-09) ----

/** Rows as `selectorRows` builds them, from `[name, state, waiting]`, with the first one selected. */
const rowsOf = (...entries) =>
  entries.map(([name, state, waiting = 0], index) => ({ name, selected: index === 0, state, waiting }));

test("another_folder_that_failed_marks_the_pill_in_the_problem_form", () => {
  const rows = rowsOf(["documents", "idle"], ["photos", "failed"]);
  assert.equal(othersProblem(rows), true);
  assert.equal(markerOf(rows), "problem");
});

test("a_folder_that_is_unavailable_marks_the_pill_like_one_that_failed", () => {
  // An unplugged drive: its pair publishes the reason as `last_error` and Rust derives `failed` for it
  // from the summary (`an_unavailable_pair_derives_failed_from_its_summary`). Built the way the window
  // gets it — a roster entry with that summary and the derived state beside it.
  const unavailable = summary("drive", {
    last_error: "the sync folder /mnt/usb/Sync is not available",
    last_sync_epoch_secs: 1_750_000_000,
  });
  const rows = selectorRows({
    pairs: [summary("documents"), unavailable],
    pairStates: [
      { name: "documents", state: "idle", rank: 0 },
      { name: "drive", state: "failed", rank: 4 },
    ],
    selected: "documents",
  });
  assert.equal(rows[1].state, "failed");
  assert.equal(markerOf(rows), "problem");
});

test("a_folder_the_person_paused_does_not_mark_the_pill", () => {
  // Paused is a choice, and the list says it. Neither form: a ring would claim a person is needed.
  const rows = rowsOf(["documents", "idle"], ["music", "paused", 0]);
  assert.equal(othersProblem(rows), false);
  assert.equal(markerOf(rows), null);
});

test("the_folder_on_screen_never_marks_its_own_pill_and_the_process_wide_states_do_not_either", () => {
  // The selected folder's failure is the chip and the hero, already. Signed out and unreachable are
  // process-wide: every row would say it at once, so there is no "other" folder to point at.
  assert.equal(markerOf(rowsOf(["documents", "failed"], ["photos", "idle"])), null);
  for (const state of ["authExpired", "unreachable", "firstRun", "running", "idle", "paused", null]) {
    assert.equal(markerOf(rowsOf(["documents", "idle"], ["photos", state])), null, String(state));
  }
  assert.equal(markerOf([]), null);
});

test("when_a_decision_waits_in_one_folder_and_another_has_failed_the_problem_form_wins", () => {
  const rows = rowsOf(["documents", "idle"], ["photos", "idle", 2], ["archive", "failed"]);
  assert.equal(othersWaiting(rows), true);
  assert.equal(othersProblem(rows), true);
  assert.equal(markerOf(rows), "problem");
  // And the ring is still the ring when nothing has failed.
  assert.equal(markerOf(rowsOf(["documents", "idle"], ["photos", "idle", 2])), "decision");
});

// ---- the list when the daemon stopped answering (the review of #447) ----

test("a_stopped_daemon_leaves_no_row_saying_up_to_date_and_nothing_to_ring_about", () => {
  // The store keeps the last roster and states across a failed read. Handed to the list as they are, they
  // read `up to date` beside a chip that says `unreachable`.
  const args = {
    pairs: [summary("documents"), summary("photos"), summary("archive")],
    pairStates: [
      { name: "documents", state: "idle", rank: 0 },
      { name: "photos", state: "idle", rank: 0 },
      { name: "archive", state: "failed", rank: 4 },
    ],
    selected: "documents",
    waiting: (name) => (name === "photos" ? 2 : 0),
  };
  const live = selectorRows({ ...args, reachable: true });
  assert.deepEqual(
    live.map((row) => row.state),
    ["idle", "idle", "failed"],
  );
  assert.equal(markerOf(live), "problem");

  const stale = selectorRows({ ...args, reachable: false });
  assert.deepEqual(
    stale.map((row) => [row.name, row.state, row.waiting]),
    [
      ["documents", "unreachable", 0],
      ["photos", "unreachable", 0],
      ["archive", "unreachable", 0],
    ],
  );
  for (const row of stale) assert.equal(stateWordOf(row.state), CHROME.pair.states.unreachable);
  assert.equal(
    markerOf(stale),
    null,
    "a marker drawn from the last answer is a marker for a state nobody has seen",
  );
  // The default is the reachable one: a caller that does not know passes nothing and gets the live rows.
  assert.deepEqual(selectorRows(args), live);
});

// ---- the hero says which folder ---------------------------------------------------------------

const settled = (over = {}) =>
  mainView({ daemonState: "idle", response: { pending_changes: 0, last_sync_epoch_secs: 1 }, ...over });
const paused = (over = {}) =>
  mainView({
    daemonState: "paused",
    response: { paused: true, pending_changes: 7, last_sync_epoch_secs: 1 },
    ...over,
  });

test("at_two_folders_the_pause_button_names_its_folder_and_at_one_it_does_not", () => {
  // D11. The hero's Pause acts on the folder the hero is about; the label says which.
  const many = heroActionsOf(settled({ pair: "photos" }));
  assert.deepEqual(
    many.map((action) => action.label),
    [MAIN.syncNow, TRAY.pausePair("photos")],
  );
  assert.equal(many[1].label, "Pause photos");
  // THE ONE-FOLDER APP IS WHAT IT WAS (D2): no name, the deck's own word.
  assert.deepEqual(
    heroActionsOf(settled()).map((action) => action.label),
    [MAIN.syncNow, MAIN.pause],
  );
  assert.deepEqual(
    heroActionsOf(settled({ pair: null })).map((action) => action.label),
    [MAIN.syncNow, MAIN.pause],
  );
});

test("at_two_folders_resume_names_its_folder_and_the_paused_sentence_names_it_too", () => {
  const many = paused({ pair: "photos" });
  assert.deepEqual(
    heroActionsOf(many).map((action) => [action.label, action.on]),
    [["Resume photos", "onResume"]],
  );
  // `Nothing will move` under one folder's name, while another syncs, would say more than is true.
  assert.match(subOf(many), /Nothing in photos will move until you resume\.$/);
  assert.equal(headlineOf(many), MAIN.paused);
  // One folder: the sentence and the button it always had.
  const one = paused();
  assert.deepEqual(
    heroActionsOf(one).map((action) => action.label),
    [MAIN.resume],
  );
  assert.match(subOf(one), /Nothing will move until you resume\.$/);
});

test("a_folder_named_like_an_object_property_is_named_on_its_pause_button", () => {
  for (const name of AWKWARD) {
    assert.equal(heroActionsOf(settled({ pair: name }))[1].label, `Pause ${name}`);
  }
});

// ---- the notice block -------------------------------------------------------------------------

test("a_folder_the_daemon_does_not_run_gets_a_sentence_and_the_one_thing_to_do_about_it", () => {
  const spec = noticeOf({ kind: "pairNotRunning", name: "photos", busy: false, failed: false, reason: null });
  assert.equal(spec.title, "photos isn't being synced yet");
  assert.equal(spec.note, MAIN.notice.pairNotRunningSub);
  assert.match(spec.note, /^Nothing is lost\./, "reassurance before the problem (voice rule 3)");
  assert.deepEqual(spec.action, {
    label: MAIN.notice.restartSyncing,
    on: "onRestartSyncing",
    disabled: false,
  });
  assert.equal(spec.reason, null);

  // Mid-click: the button is inert and says so; the handler is not wired (`calledThrough` is not built).
  const busy = noticeOf({ kind: "pairNotRunning", name: "photos", busy: true, failed: false, reason: null });
  assert.equal(busy.action.label, MAIN.notice.restarting);
  assert.equal(busy.action.disabled, true);

  // A restart that did not work: the second sentence changes, and the daemon's own words are QUOTED
  // under it rather than joined to ours (voice rule 4) — and only when there are any.
  const failed = noticeOf({
    kind: "pairNotRunning",
    name: "photos",
    busy: false,
    failed: true,
    reason: "unit not found",
  });
  assert.equal(failed.note, MAIN.notice.restartFailed);
  assert.equal(failed.reason, "unit not found");
  assert.equal(
    noticeOf({ kind: "pairNotRunning", name: "p", busy: false, failed: true, reason: null }).reason,
    null,
  );
  // …and the failure's reason is NOT quoted once the restart is not a failure.
  assert.equal(
    noticeOf({ kind: "pairNotRunning", name: "p", busy: false, failed: false, reason: "stale" }).reason,
    null,
  );
});

test("a_pause_the_daemon_could_not_save_says_so_in_the_daemons_words_and_has_no_button", () => {
  for (const [kind, title, sub] of [
    ["pauseUnsaved", MAIN.notice.pauseUnsaved, MAIN.notice.pauseUnsavedSub],
    ["resumeUnsaved", MAIN.notice.resumeUnsaved, MAIN.notice.resumeUnsavedSub],
  ]) {
    const spec = noticeOf({ kind, reason: "attempt to write a readonly database" });
    assert.equal(spec.title, title);
    assert.equal(spec.note, sub);
    assert.equal(spec.reason, "attempt to write a readonly database");
    assert.equal(spec.action, null, "it states a fact about a restart; there is nothing to press");
  }
  // The two halves differ in the direction a restart would take the folder.
  assert.match(MAIN.notice.pauseUnsavedSub, /will not be paused/);
  assert.match(MAIN.notice.resumeUnsavedSub, /may be paused again/);
});

test("no_notice_and_a_kind_this_build_does_not_know_draw_nothing", () => {
  assert.equal(noticeOf(null), null);
  assert.equal(noticeOf(undefined), null);
  assert.equal(noticeOf({ kind: "from-a-newer-build" }), null);
  assert.equal(settled().notice, null);
});

// ---- the store: what the window needs of folders that are not on screen -----------------------------

let issue = 1000;
const next = () => (issue += 1);

const payload = (names, described, selected, over = {}, response = {}) => ({
  state: "idle",
  selected,
  pairs: names.map((n) => summary(n)),
  pair_states: names.map((n) => ({ name: n, state: "idle", rank: 0 })),
  response: { pair: described, pairs: names.map((n) => summary(n)), pending_changes: 0, ...response },
  ...over,
});

test("a_refusal_is_not_an_observation", () => {
  // A request the app turned back — a folder nobody has — comes home with no reply, the name in
  // `pair_unknown`, and a placeholder `state` a caller must not read. Filed, it draws the folder it was
  // about as unreachable on a daemon that never stopped answering.
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  assert.equal(store.select.daemonState(), "idle");
  store.setStatus(
    { state: "unreachable", error: "no folder pair named nope", pair_unknown: "nope", selected: "docs" },
    next(),
  );
  assert.equal(store.select.daemonState(), "idle", "the refusal moved nothing");
  assert.equal(store.select.pairUnknown(), null);
});

test("the_folder_a_selection_read_fell_back_from_is_remembered_until_a_read_says_otherwise", () => {
  store.configure({ follows: "selection" });
  // The window asked for `photos`, which the daemon does not run, and was answered about `docs`.
  store.setStatus(payload(["docs", "music"], "docs", "docs", { pair_unknown: "photos" }), next());
  assert.equal(store.select.pairUnknown(), "photos");
  // A read that NAMES another folder never speaks for the selection: it neither sets nor clears it.
  store.setStatus(payload(["docs", "music"], "music", "docs"), next());
  assert.equal(store.select.pairUnknown(), "photos", "a named read about music said nothing about photos");
  // The next selection read that is not a fall-back clears it.
  store.setStatus(payload(["docs", "music"], "docs", "docs"), next());
  assert.equal(store.select.pairUnknown(), null);
});

test("another_folders_queue_is_known_only_once_a_reply_has_filled_it", () => {
  store.configure({ follows: "selection" });
  store.setStatus(payload(["docs", "photos"], "docs", "docs"), next());
  assert.equal(store.select.deletionsFiledOf("photos"), false, "nothing has been read for photos");
  assert.deepEqual(store.select.pendingDeletionsOf("photos"), []);
  // A named read about photos files its queue under photos and leaves the selection where it was.
  store.setStatus(
    payload(
      ["docs", "photos"],
      "photos",
      "docs",
      {},
      { pending_deletions: [{ path: "a.txt", direction: "remote", fingerprint: "f" }] },
    ),
    next(),
  );
  assert.equal(store.select.deletionsFiledOf("photos"), true);
  assert.deepEqual(
    store.select.pendingDeletionsOf("photos").map((d) => [d.path, d.pair]),
    [["a.txt", "photos"]],
  );
  assert.equal(store.select.pairName(), "docs");
  assert.deepEqual(store.select.pendingDeletions(), [], "docs' own queue is not photos'");
  // A folder never heard of answers empty, and an awkward name is not an object property.
  for (const name of AWKWARD) {
    assert.deepEqual(store.select.conflictsOf(name), []);
    assert.equal(store.select.deletionsFiledOf(name), false);
  }
});

test("the_deck_has_a_word_for_every_state_the_selector_can_draw", () => {
  assert.deepEqual(
    Object.keys(CHROME.pair.states).sort(),
    ["authExpired", "failed", "firstRun", "idle", "paused", "running", "unreachable"].sort(),
  );
});
