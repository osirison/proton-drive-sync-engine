// Four facts out of the main screen (S1) that a passing fidelity gate would not defend.
//
// The gate compares ONE rendering of each frame against one drawing of it. Everything below is
// either a decision the frames make once and the code has to make every time, or a state no frame
// draws at all — and each one fails quietly, which is the bar `rows.test.js` sets for earning a test
// in a frontend that is otherwise checked by the gate.
//
//   · WHICH HERO A MOMENT IS IN. `2a Needs you` is the syncing hero with a band under it, so
//     "syncing" and "a decision is waiting" are not alternatives (DEVIATIONS §24). Get the priority
//     wrong and every drawn frame still passes, because each one only exercises one branch.
//   · THE BAND'S SENTENCES. The deck writes them at one conflict and two deletions; the screen
//     renders them from live counts. The copy gate now checks the drawn instance (its `DRAWN`
//     table), and nothing checks that 2 permanent + 0 recoverable does not print a clause about
//     zero files going to the Trash.
//   · THE TRANSFER ROW'S PROGRESS. `TransferActivity` never carries both `bytes_total` and
//     `bytes_done`, so a percentage is unreachable and the bar must not be drawn (#98). A `0` here
//     instead of a `null` is a bar at 0% on a file that is uploading fine.
//   · PAUSED AND UNREACHABLE. Neither has a `2a` frame, so the gate cannot see them at all.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  clearsStartError,
  deletionCountsOf,
  headlineOf,
  hiddenTransfers,
  heroActionsOf,
  heroStateOf,
  mainView,
  quotedError,
  subOf,
} from "../src/js/screens/main.js";
import { MAIN, TRAY } from "../src/js/ui/copy.js";
import { cardinal } from "../src/js/ui/format.js";

const state = (over = {}) => ({ daemonState: "idle", syncing: false, waiting: 0, pending: 0, ...over });

test("a decision does not replace the syncing hero — it is additive", () => {
  // `2a Needs you`: three things waiting AND a pass in flight. The mark stays the syncing one and
  // the band carries the decisions, which is what the frame draws.
  assert.equal(heroStateOf(state({ daemonState: "running", syncing: true, waiting: 3 })), "syncing");
});

test("only when nothing is transferring does the mark itself take the decision form", () => {
  // 14-behaviour-and-state.md, in those words.
  assert.equal(heroStateOf(state({ waiting: 3 })), "decision");
  assert.equal(heroStateOf(state()), "settled");
});

test("unreachable and paused outrank everything below them", () => {
  // Unreachable first: it is the one state where nothing else on the screen is known to be current.
  assert.equal(heroStateOf(state({ daemonState: "unreachable", syncing: true, waiting: 4 })), "unreachable");
  // Paused over a decision: "nothing will move until you resume" is true of the decisions too.
  assert.equal(heroStateOf(state({ daemonState: "paused", waiting: 2 })), "paused");
});

test("an expired sign-in never falls through to `Everything is up to date`", () => {
  // The bug this test exists for, found in review: with no `authExpired` arm, a reachable daemon
  // that cannot talk to Proton reports nothing in flight and nothing waiting, and the fall-through
  // draws a false all-clear over a sync that cannot happen. `routes.js` releases the onboarding
  // latch on exactly this state so the main screen can carry it.
  assert.equal(heroStateOf(state({ daemonState: "authExpired" })), "authExpired");
  assert.notEqual(heroStateOf(state({ daemonState: "authExpired" })), "settled");
  // Still below unreachable, and it does not become the syncing hero if a pass is somehow in flight.
  assert.equal(heroStateOf(state({ daemonState: "authExpired", syncing: true })), "authExpired");
});

test("the sign-in hero quotes the deck's one sentence, split in two", () => {
  // Both halves are checked verbatim against `11a Outage` by the copy gate; this pins the split.
  assert.equal(
    `${MAIN.authExpired}. ${MAIN.authExpiredSub(61)}`,
    "Proton Drive is asking you to sign in again. 61 changes are waiting — nothing is lost.",
  );
});

test("a pass that failed never falls through to `Everything is up to date`", () => {
  // #246. The same shape as the `authExpired` test above and the one it was found next to: every
  // arm of the derivation is a state the daemon is IN, a failed pass is none of them, and the
  // fall-through drew the app's strongest all-clear over a sync that did not happen.
  assert.equal(heroStateOf(state({ daemonState: "failed" })), "failed");
  assert.notEqual(heroStateOf(state({ daemonState: "failed" })), "settled");
  // AND NOT `syncing`, which is the half a `derive_state` fix alone would not have covered: this
  // function calls a non-empty watch queue `syncing` on its own, so a failure with anything queued
  // behind it would have read `Syncing 5 changes` instead. The failure outranks the queue.
  assert.equal(heroStateOf(state({ daemonState: "failed", pending: 5 })), "failed");
  // And a decision waiting does not displace it either — the band still draws, additively.
  assert.equal(heroStateOf(state({ daemonState: "failed", waiting: 2 })), "failed");
});

test("the failed hero quotes the daemon and says nothing is lost", () => {
  const v = mainView({
    daemonState: "failed",
    response: { pending_changes: 4, last_error: "proton-drive list failed: os error 2" },
  });
  assert.equal(v.hero, "failed");
  // Verbatim, through no formatter — voice rule 4, and the whole point of the state.
  assert.equal(v.error, "proton-drive list failed: os error 2");
  assert.equal(MAIN.failedSub(4), "Nothing is lost. 4 changes are waiting and will go on the next try.");
  // `0` drops the clause rather than reading as an all-clear; so does an absent reply.
  assert.equal(MAIN.failedSub(0), "Nothing is lost.");
  assert.equal(MAIN.failedSub(null), "Nothing is lost.");
  assert.equal(MAIN.failedSub(1), "Nothing is lost. 1 change is waiting and will go on the next try.");
});

test("the band's titles reproduce the drawn sentences at the drawn counts", () => {
  assert.equal(MAIN.band.conflictTitle(1), "One file changed on both sides");
  assert.equal(MAIN.band.deletionTitle(2), "Two deletions are waiting on you");
  assert.equal(
    MAIN.band.deletionSub(1, 1),
    "1 removes from this computer permanently · 1 goes to Proton's Trash",
  );
});

test("a category with nothing in it gets no clause, and both halves agree on number", () => {
  // "2 remove … permanently · 0 go to Proton's Trash" names a thing that is not happening, in the
  // one sentence whose job is telling you what you are about to lose.
  assert.equal(MAIN.band.deletionSub(2, 0), "2 remove from this computer permanently");
  assert.equal(MAIN.band.deletionSub(0, 3), "3 go to Proton's Trash");
  assert.equal(MAIN.band.deletionSub(0, 0), "");
  assert.equal(MAIN.band.deletionTitle(1), "One deletion is waiting on you");
  assert.equal(MAIN.band.conflictTitle(4), "Four files changed on both sides");
});

test("the band names the trash a deletion is actually going to", () => {
  // REPRODUCED BEFORE IT WAS FIXED. With `local_delete_mode = "trash"` the default, a queue of two
  // recoverable local deletions rendered `2 go to Proton's Trash` — the two-clause template folded
  // every non-permanent row into the Proton clause, and both of those files are on this disk.
  // `columnCopy` refuses the same mistake one screen later; this is the band's version of it.
  assert.equal(MAIN.band.deletionSub(0, 0, 2), "2 go to this computer's Trash");
  assert.equal(MAIN.band.deletionSub(0, 1, 1), "1 goes to this computer's Trash · 1 goes to Proton's Trash");
  assert.equal(
    MAIN.band.deletionSub(1, 1, 1),
    "1 removes from this computer permanently · 1 goes to this computer's Trash · 1 goes to Proton's Trash",
  );
  // The third count defaults to 0, which is what keeps the DRAWN assertion above byte-identical.
  assert.equal(MAIN.band.deletionSub(1, 1), MAIN.band.deletionSub(1, 1, 0));
});

test("the screen splits its own queue into those three counts", () => {
  // The split is the caller's, not the template's. `remote` acts on Proton, `local` acts here, and
  // `severityOfItem` is the one thing asked whether acting here is permanent.
  const sentence = (deletions) => {
    const { permanent, protonTrash, localTrash } = deletionCountsOf(deletions);
    return MAIN.band.deletionSub(permanent, protonTrash, localTrash);
  };
  assert.deepEqual(
    deletionCountsOf([
      { path: "a", direction: "remote", disposal: "recoverable" },
      { path: "b", direction: "local", disposal: "recoverable" },
      { path: "c", direction: "local", disposal: "permanent" },
    ]),
    { permanent: 1, protonTrash: 1, localTrash: 1 },
  );
  assert.equal(
    sentence([
      { path: "a", direction: "remote", disposal: "recoverable" },
      { path: "b", direction: "local", disposal: "recoverable" },
      { path: "c", direction: "local", disposal: "permanent" },
    ]),
    "1 removes from this computer permanently · 1 goes to this computer's Trash · 1 goes to Proton's Trash",
  );
  // Fail closed: an older daemon sends no `disposal`, and that row really is a permanent delete.
  assert.deepEqual(deletionCountsOf([{ path: "a", direction: "local" }]), {
    permanent: 1,
    protonTrash: 0,
    localTrash: 0,
  });
  // The default queue: every row recoverable, and none of them Proton's.
  assert.equal(
    sentence([
      { path: "a", direction: "local", disposal: "recoverable" },
      { path: "b", direction: "local", disposal: "recoverable" },
    ]),
    "2 go to this computer's Trash",
  );
});

test("queued-but-not-started work is not settled", () => {
  // A filesystem-watch event only accumulates `pending_changes`; it never starts a reconcile. So the
  // daemon reports `syncing: false` with a non-empty queue for up to a scan interval — and
  // gui-core's `derive_state` already calls that `Running`, so the header chip said `syncing` while
  // the hero underneath said `Everything is up to date`, about the same file at the same moment.
  assert.equal(heroStateOf(state({ syncing: false, pending: 5 })), "syncing");
  assert.equal(heroStateOf(state({ syncing: false, pending: 0 })), "settled");
});

test("the change count is the plan's transfers while syncing, not the local watch queue", () => {
  // `pending_changes` is local-only, so a pass driven entirely by Proton carries an empty queue
  // while downloading — and the headline read `Syncing 0 changes` with a literal 0 in the mark.
  const remoteDriven = mainView({
    daemonState: "running",
    response: {
      syncing: true,
      pending_changes: 0,
      last_plan_summary: { uploads: 0, downloads: 7, remote_deletes: 2 },
    },
  });
  assert.equal(remoteDriven.pending, 7, "downloads count, deletions do not");
  assert.equal(remoteDriven.numeral, 7);

  // Before the plan exists there is no transfer count, so the queue answers instead.
  const stillScanning = mainView({
    daemonState: "running",
    response: { syncing: true, pending_changes: 4, last_plan_summary: null },
  });
  assert.equal(stillScanning.pending, 4);
});

test("`0 leaving, 0 arriving` is never printed for a plan that does not exist yet", () => {
  assert.equal(MAIN.syncingSub("14 seconds ago", null, null), "started 14 seconds ago");
  assert.equal(MAIN.syncingSub("14 seconds ago", 2, 1), "started 14 seconds ago · 2 leaving, 1 arriving");
  // A plan that genuinely moves nothing in one direction is a real answer, and keeps its clause.
  assert.equal(MAIN.syncingSub("1 minute ago", 0, 9), "started 1 minute ago · 0 leaving, 9 arriving");
});

test("every counted sentence agrees in number at one", () => {
  // Every drawn instance in the deck is plural — `Syncing 3 changes`, `7 changes have piled up`,
  // `4 changes are waiting`, `3 things need you` — so the plural was baked into the template and the
  // first live count of 1 rendered `1 other changes are waiting on you`. Found by looking at the
  // running app, not by any gate: the gate compares the frames, and no frame draws a one.
  assert.equal(MAIN.syncing(1), "Syncing 1 change");
  assert.equal(MAIN.otherWaiting(1), "1 other change is waiting on you");
  assert.match(MAIN.pausedSub(1, "13:20"), /^1 change has piled up/);
  assert.equal(MAIN.authExpiredSub(1), "1 change is waiting — nothing is lost.");
  assert.equal(MAIN.compact.needYou(1), "1 thing needs you");
  assert.match(TRAY.unreachableBody(1), /1 change is waiting and will go/);

  // And every drawn instance is byte-identical, which is what the copy gate re-checks.
  assert.equal(MAIN.syncing(3), "Syncing 3 changes");
  assert.equal(MAIN.otherWaiting(3), "3 other changes are waiting on you");
  assert.equal(MAIN.compact.needYou(3), "3 things need you");
  assert.equal(
    TRAY.unreachableBody(4),
    "Nothing is lost. 4 changes are waiting and will go as soon as it's back.",
  );
  // Zero takes the plural, which is what English does: "0 changes are waiting".
  assert.equal(MAIN.syncing(0), "Syncing 0 changes");
});

test("an unknown pending count never renders as zero", () => {
  // 14-behaviour-and-state.md, `gui-core`'s `DaemonState::Unreachable` doc and
  // `store.select.countersUnknown()` all say the same thing: a missing number is UNKNOWN, never
  // zero. When the daemon is unreachable there is no reply at all, so `0 changes are waiting` would
  // be a false all-clear at the exact moment the app cannot see anything — and an em-dash mid
  // sentence is not English, so the clause goes instead.
  assert.equal(TRAY.unreachableBody(null), "Nothing is lost.");
  assert.equal(TRAY.unreachableBody(undefined), "Nothing is lost.");
  assert.equal(
    TRAY.unreachableBody(4),
    "Nothing is lost. 4 changes are waiting and will go as soon as it's back.",
  );
  // Zero is a real answer the daemon can give, and it is not the same as no answer.
  assert.match(TRAY.unreachableBody(0), /^Nothing is lost\. 0 changes/);
});

test("cardinal spells the small counts a sentence opens with and hands the rest back", () => {
  assert.equal(cardinal(1), "One");
  assert.equal(cardinal(10), "Ten");
  // Above ten the design stops spelling — `11-notifications.md` writes `5 files changed on both
  // sides` for the grouped banner, so the line is a style choice rather than a rule.
  assert.equal(cardinal(11), "11");
  assert.equal(cardinal(1200), "1,200");
  assert.equal(cardinal(null), "—");
  assert.equal(cardinal(1.5), "1.5");
});

// ---- a daemon that is not running (DEVIATIONS §95) ----
//
// The state has no frame, and until now it had no control either: the hero offered `Try again now`,
// which is `onSyncNow`, which is a round trip down the control socket — the socket whose silence is
// the definition of this state. Everything below is a branch the fidelity gate cannot reach.

const view = (over = {}) => mainView({ daemonState: "unreachable", response: null, ...over });

test("a stopped daemon is offered the row that starts it, not the one that retries a sync", () => {
  const actions = heroActionsOf(view());
  assert.equal(actions.length, 1);
  assert.equal(actions[0].label, TRAY.start);
  assert.equal(actions[0].on, "onStartService");
  // The mistake this replaces, named so a revert reads as a failure rather than a diff.
  assert.notEqual(actions[0].label, TRAY.tryAgain);
  assert.notEqual(actions[0].on, "onSyncNow");
});

test("the two states that ARE answering keep the retry", () => {
  // `syncnow` reaches a daemon that replies, so `Try again now` does what it says for both of these
  // — which is why they stayed behind when `unreachable` moved out of the shared branch.
  for (const hero of ["failed", "authExpired"]) {
    const actions = heroActionsOf({ hero });
    assert.equal(actions.length, 1, hero);
    assert.equal(actions[0].label, TRAY.tryAgain, hero);
    assert.equal(actions[0].on, "onSyncNow", hero);
  }
});

test("the start button goes inert while the start is in flight, and carries no handler when it does", () => {
  // `systemctl --user start` blocks until the unit reports started, so this is on screen for
  // seconds. A DISABLED KIND WOULD HAVE DROPPED THE HANDLER SILENTLY (`button` sets `onClick: null`
  // for one), which is fine here because the button is meant to be inert — but the same fact is why
  // the armed one must NOT be disabled. Both directions are asserted.
  const busy = heroActionsOf(view({ starting: true }));
  assert.equal(busy[0].label, MAIN.starting);
  assert.equal(busy[0].disabled, true);
  assert.equal(busy[0].on, null, "an inert button must not also be wired");

  const armed = heroActionsOf(view({ starting: false }));
  assert.ok(!armed[0].disabled, "the armed button must not be disabled — it would drop its handler");
  assert.equal(armed[0].on, "onStartService");
});

test("the headline names the daemon, because the socket is what did not answer", () => {
  // THROUGH `headlineOf` AND `subOf`, not against the constants. The first version of this test
  // asserted properties of MAIN.notRunning itself — that it is not TRAY.unreachableTitle, that it
  // says nothing about Proton — which no edit to this screen can falsify: reverting the two `case
  // "unreachable"` arms left it green. It pinned the deck, and the deck was never what changed.
  //
  // `Can't reach Proton Drive` is the deck's OUTAGE sentence and it is a claim about the wrong
  // machine: Proton takes no part in a control-socket round trip. A daemon that is up and cannot
  // reach Proton derives `failed` or `authExpired`, both of which still say so.
  const v = view();
  assert.equal(headlineOf(v), MAIN.notRunning);
  assert.notEqual(headlineOf(v), TRAY.unreachableTitle);
  assert.doesNotMatch(headlineOf(v), /Proton/);
  // The sub-line carries no count clause at all, rather than one that has to remember to drop
  // itself: there is no reply here, so the number was never `0` — it was unknown. Asserted at a
  // pending count of 4, the value that made the old `TRAY.unreachableBody(v.pending)` produce one.
  assert.equal(subOf(v), MAIN.notRunningSub);
  assert.doesNotMatch(subOf({ ...v, pending: 4 }), /\bchanges?\b/);
  assert.match(subOf(v), /Nothing is lost/);
  // The two states that DO mean Proton keep their own sentences — the arm above must not swallow them.
  assert.equal(headlineOf({ hero: "failed" }), MAIN.failed);
  assert.equal(headlineOf({ hero: "authExpired" }), MAIN.authExpired);
});

test("a start that failed is quoted, and only under the hero it explains", () => {
  // THROUGH `quotedError`, which is what picks the string the `.main-failed` block renders. The
  // first version asserted `failed.startError` matched the string it had passed in one line earlier
  // — a round trip through mainView, proving nothing about the branch that draws it. Deleting that
  // branch entirely left the suite green.
  //
  // `start_service` REJECTS — unlike every control-socket command, which folds its failure into the
  // payload — and its message names which of the two ways it failed (no systemd unit, no config
  // file). Swallowed, the button is the dead control #224 and #227 record.
  const why = "couldn't start via systemd (Unit not found) and there is no config file";
  assert.equal(quotedError(view({ startError: why })), why);
  assert.equal(quotedError(view()), null, "no attempt yet is not an empty quotation");
  // A failed PASS still quotes the DAEMON's string in the same block, and the two never cross: a
  // start failure under a settled hero would be a stopped-service reason on a running daemon.
  assert.equal(quotedError({ hero: "failed", error: "os error 2" }), "os error 2");
  assert.equal(quotedError({ hero: "failed", startError: why }), null);
  assert.equal(quotedError({ hero: "settled", startError: why }), null);
  assert.equal(quotedError({ hero: "syncing", startError: why }), null);
});

test("the transfer window reads the list, and falls back to the singular field", () => {
  // TWO WIRE SHAPES. A current daemon sends `transfers`; one predating #211 sends only `transfer`,
  // which meant "the row in flight". Reading the list when it has rows is what keeps a reply
  // carrying both — the daemon derives the mirror FROM the list — from drawing the same file twice.
  const activity = {
    phase: "executing",
    transfers: [
      { direction: "upload", path: "docs/spec.md", bytes_total: 1200000, state: "active" },
      { direction: "upload", path: "notes/scratch.md", bytes_total: 8400, state: "queued" },
      { direction: "download", path: "q3.pdf", state: "queued" },
    ],
    transfers_remaining: 3,
    transfer: { direction: "upload", path: "docs/spec.md", bytes_total: 1200000 },
  };
  const rows = mainView({ response: { syncing: true, activity } }).transfers;
  assert.equal(rows.length, 3, "the list, not the list plus its own mirror");
  assert.equal(rows[0].detail, "1.2 MB", "an upload in flight shows the size the scan measured");
  assert.equal(rows[1].detail, MAIN.queued, "a queued row's chip is the word `2a Syncing` draws");
  assert.equal(rows[2].direction, "down");
  // Never a fraction: no reply carries both ends of one (#98). `null` is "no track", not 0%.
  assert.equal(rows[0].progress, null);

  // A batched download is ONE row over a folder's chunk, and says so rather than reading as a file.
  const batch = mainView({
    response: {
      syncing: true,
      activity: {
        phase: "executing",
        transfers: [{ direction: "download", path: "photos/2024", state: "active", files: 25 }],
        transfers_remaining: 25,
      },
    },
  }).transfers;
  assert.equal(batch[0].detail, "25 files");

  // The older shape still draws its one row, with the state the field used to imply.
  const legacy = mainView({
    response: {
      syncing: true,
      activity: { phase: "executing", transfer: { direction: "download", path: "a.bin" } },
    },
  });
  assert.deepEqual(
    legacy.transfers.map((t) => [t.direction, t.name, t.state]),
    [["down", "a.bin", "active"]],
  );
  assert.equal(legacy.transfersRemaining, null, "unknown, and never rendered as `+0 more`");
});

test("`+n more` is the daemon's count, and a batched row weighs as its whole chunk", () => {
  // NOT `transfers.length - shown.length`: the daemon caps the window it sends, so that subtraction
  // is always 0 and the `+n more` node would be dead code that looks live. Every number below is
  // computed here so the claim and the assertion cannot drift apart.
  const q = (name) => ({ files: null, name });
  assert.equal(hiddenTransfers(118, [{ files: 25 }, q("a"), q("b")]), 91, "118 - (25 + 1 + 1)");
  assert.equal(hiddenTransfers(3, [q("a"), q("b"), q("c")]), 0, "the window names everything left");
  assert.equal(hiddenTransfers(0, []), 0, "a pass with no transfers left");
  // `null` is UNKNOWN — not executing, or a daemon predating the field — and yields no node at all
  // rather than `+0 more`, which is the same rule the sub-line's null clauses follow.
  assert.equal(hiddenTransfers(null, [q("a")]), 0);
  // A remainder smaller than what is drawn cannot go negative into the copy deck.
  assert.equal(hiddenTransfers(1, [q("a"), q("b")]), 0);
});

test("a start failure stops being the reason the moment the socket answers", () => {
  // Found by review. `quotedError` asks only which HERO is showing, and a later outage puts the
  // screen back in the same one — so a remembered failure that nothing retires gets drawn as the
  // diagnosis of an outage it predates, under the block whose whole job is to say why.
  //
  // The routes that make it reachable are the ones that do NOT go through the button: the tray's own
  // `Start the sync service` row starts the daemon entirely in Rust, Settings' restart has its own
  // path, and a terminal has no path at all. All three leave the JS latch untouched.
  assert.equal(clearsStartError("unreachable"), false, "still down — the reason still stands");
  for (const answered of ["idle", "running", "paused", "failed", "authExpired", "firstRun"]) {
    assert.equal(clearsStartError(answered), true, answered);
  }
});
