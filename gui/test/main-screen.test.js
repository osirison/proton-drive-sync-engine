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
import { heroStateOf, mainView } from "../src/js/screens/main.js";
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
