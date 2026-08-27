// The main screen (S1) — one hexagon, one sentence, and the seam.
//
// `03-main-screen.md`. It replaces the v1 Overview's nine competing regions with three states drawn
// on ONE skeleton, and the whole design rests on a property that is easy to lose in a rewrite:
// **the hexagon never moves.** The hero is a fixed 394px block that centres it, so settled, syncing,
// paused and unreachable all put the mark on the same 168 pixels and only what surrounds it changes.
// A hero sized to its content would drift by a line's height every time the sub-line changed.
//
// THREE THINGS THIS MODULE OWNS THAT NOTHING ELSE CAN.
//
//   · WHICH STATE IS SHOWING. `14-behaviour-and-state.md`: "needs-decision is additive, not
//     exclusive". Conflicts and withheld deletions coexist with settled, syncing and paused — the
//     hexagon carries the TRANSFER state and the band carries the decisions. `2a Needs you` is the
//     proof: it is `2a Syncing` with a band under it, down to the same gradient ids (DEVIATIONS §24
//     — there is no crimson hero, and the mark's numeral is 3 transfers, not 3 decisions).
//   · THE STRUCTURE, not just the styling. The screen is two or three SIBLINGS of the window root
//     (`shell.css`: "no wrapper element, so a screen's flex:1 block is a direct flex child exactly
//     as drawn"), which is why `renderMain` returns an array and `app.js` splices it in.
//   · WHEN TO PATCH AND WHEN TO REBUILD. The shell re-renders on every ~2s status poll. Rebuilding
//     the mark there restarts both travelling segments from 0% — the failure `updateHexagon` exists
//     to prevent — so a poll patches text and the numeral in place, and only a genuine state change
//     builds a new mark, behind the 220ms crossfade the design asks for.
//
// WHAT PHASE 1 CANNOT DRAW, all recorded in DEVIATIONS.md §63 with the issue that closes each:
// the settled sub-line's `12,480 files · 41.2 GB` (G7/#207), the footer's `386 MB sent · 1.1 GB
// received today` (G2/#191 — the shell draws the folder pair instead), and the per-file progress bar
// (#98 — `bytes_total` and `bytes_done` are never both present, so no percentage exists to draw).
//
// The transfer LIST is no longer among them (#211): `activity.transfers` is a bounded window of the
// rows in flight and the ones queued behind them, and `activity.transfers_remaining` is what sizes
// `+n more`. The bar inside those rows is still #98's and still absent.

import { el } from "../ui/el.js";
import { MAIN, TRAY } from "../ui/copy.js";
import { bytes, clock, since } from "../ui/format.js";
import { renderHexagon, updateHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button } from "../ui/controls.js";
import { transferRow, eyebrow, severityOfItem } from "../ui/rows.js";
import { attentionBand, bandButton } from "../ui/bands.js";
import { fid } from "../fixtures/frames.js";

/** The hero mark, at the one size this screen draws (`01-foundations.md` §6, `strokeForSize`). */
const HERO_SIZE = 168;

/**
 * The design's own cap: "Transfer rows appear in flight order, cap at ~6 visible with `+n more` in
 * mono if exceeded." It is also what keeps the columns inside the grid — they are drawn
 * `overflow:visible` like 21 of the 22 in-scope windows, so nothing clips a seventh row.
 */
const MAX_ROWS = 6;

// ------------------------------------------------------------------------------ the states ----

/**
 * Whether a remembered `start_service` failure has stopped being the reason for anything.
 *
 * A start failure describes a STOPPED SERVICE. The moment the control socket answers — by whatever
 * route, this button, the tray row, Settings' restart, a `systemctl` in a terminal — the sentence it
 * explained is no longer on screen, and the string is a fact about a superseded attempt.
 *
 * It has to be cleared rather than merely hidden, because `quotedError` asks only which HERO is
 * showing and a later outage puts the screen back in the same one. Left alone, the next time the
 * daemon went down the block would quote a failure from minutes earlier as that outage's diagnosis —
 * in the one block whose whole job is to be the account of why. `app.js` already carries this exact
 * rule for `onboardingFailure` ("a merge that failed against a daemon that then came up is not
 * onboarding's problem any more"); this is the same rule for the same reason.
 *
 * NOT cleared by navigating away and back: while the daemon is still down, the last attempt's reason
 * is still the right answer, and forgetting it would leave the button unexplained again.
 */
export const clearsStartError = (daemonState) => daemonState !== "unreachable";

/**
 * Which hero the moment is in. Deliberately NOT the daemon's derived `state` verbatim: `running`
 * splits by whether a pass is actually in flight, and the two decision states are a question about
 * the queue rather than about the daemon.
 *
 * Order is the order the design puts them in. Unreachable outranks everything because it is the one
 * state where nothing else on the screen can be trusted to be current; paused outranks a decision
 * because the sentence "nothing will move until you resume" is true of the decisions too.
 */
export function heroStateOf({ daemonState, syncing, waiting, pending = 0 }) {
  if (daemonState === "unreachable") return "unreachable";
  // BEFORE `syncing`, and before the settled fall-through, which is where it landed first and is a
  // false all-clear: a daemon whose Proton session has lapsed is reachable, reports nothing in
  // flight, and would otherwise draw `Everything is up to date` over a sync that cannot happen.
  // `routes.js` releases the onboarding latch on this state specifically so the main screen can
  // carry it — "we must actually hand off to the main screen's Re-authenticate action rather than
  // trap the user in the wizard" — so a fall-through is a broken hand-off, not just a missing state.
  if (daemonState === "authExpired") return "authExpired";
  // BEFORE the `pending` fall-through below, which is the second half of #246: `derive_state` used
  // to answer `idle` for a daemon whose last pass failed, and even once it stopped, this function
  // would have answered `syncing` for the same daemon the moment anything was queued behind the
  // failure. Both draw a sentence that is not true. A pass actually in flight never reaches here as
  // `failed` — `derive_state` ranks `syncing` above the failure it may be retrying — so this arm
  // cannot hide live work.
  if (daemonState === "failed") return "failed";
  if (daemonState === "paused") return "paused";
  // `pending` AS WELL AS `syncing`, and leaving it out put two contradictory sentences in one
  // window. A filesystem-watch event only accumulates `pending_changes`; it never starts a reconcile
  // (`daemon.rs`), so for up to a scan interval after an edit the daemon reports `syncing: false`
  // with a non-empty queue. `gui-core`'s `derive_state` already calls that `Running`, so the header
  // chip read `syncing` while the hero underneath said `Everything is up to date` — about the same
  // file, at the same moment. Queued work is not settled. §67d.
  if (syncing || pending > 0) return "syncing";
  // `14-behaviour-and-state.md`: "Only when nothing is transferring does the hexagon itself take the
  // decision form." No frame draws it at 168px — DEVIATIONS §24 measured the crimson mark as
  // existing only at ≤72px — so this is prose-normative and unverified by the gate. §67.
  if (waiting > 0) return "decision";
  return "settled";
}

/**
 * The screen's whole view model, derived once so the render and the patch cannot disagree about it.
 *
 * `activity` is the live pass (`SyncActivity`); `summary` is the plan the daemon published before
 * the transfers started, which is where `2 leaving, 1 arriving` comes from — `uploads` and
 * `downloads` on `last_plan_summary`, not a count of the rows drawn.
 */
export function mainView(props = {}) {
  const {
    daemonState = "unreachable",
    response = null,
    conflicts = [],
    deletions = [],
    localRoot = null,
    remoteRoot = null,
    starting = false,
    startError = null,
  } = props;

  const activity = response?.activity ?? null;
  const summary = response?.last_plan_summary ?? null;
  const queued = response?.pending_changes ?? null;
  const waiting = conflicts.length + deletions.length;
  const hero = heroStateOf({
    daemonState,
    syncing: Boolean(response?.syncing),
    waiting,
    pending: queued ?? 0,
  });

  /**
   * HOW MANY CHANGES THIS PASS IS MOVING — and it is not `pending_changes` alone.
   *
   * `pending_changes` is the filesystem-watch queue: paths a local `notify` event dirtied, cleared
   * after each successful reconcile (`daemon.rs`). It says nothing about the remote side, so a pass
   * driven entirely by Proton — a second device uploading, the first reconcile after a restart —
   * carries an EMPTY queue while downloading, and the headline read `Syncing 0 changes` with a
   * literal `0` inside the mark.
   *
   * The plan knows: `uploads + downloads` is what the pass will actually move, in both directions,
   * and the daemon publishes it before the transfers start. Deletions are excluded on purpose —
   * a withheld one is a decision, and "the count in the hexagon is transfers, not decisions".
   *
   * Both numbers are the daemon's; neither is derived from the other. The queue is the answer while
   * work is waiting and no plan exists yet, and the plan is the answer once there is one. On both
   * drawn frames they agree at 3, which is why nothing in the gate moves.
   */
  const moving = summary ? summary.uploads + summary.downloads : null;
  const changes = response?.syncing ? (moving ?? queued) : queued;

  return {
    hero,
    waiting,
    conflicts,
    deletions,
    localRoot,
    remoteRoot,
    pending: changes,
    queued,
    summary,
    activity,
    lastSync: response?.last_sync_epoch_secs ?? null,
    // The daemon's own account of why the last pass failed, carried down to the block that quotes
    // it. Never formatted, never truncated, never joined to a sentence of ours (voice rule 4).
    error: response?.last_error ?? null,
    // The other two facts a stopped daemon has, and neither is in a reply — there isn't one. The
    // caller owns both: `starting` is a click that has not come back yet, and `startError` is why
    // the last attempt did not work. `start_service` is the one command on this screen that REJECTS
    // rather than folding its failure into a payload, and its message ("no systemd unit … and no
    // config file at …") is the only thing that says which of the two ways it failed — so it is
    // quoted, in the block a failed pass already uses. Without it the button is the dead control
    // #224 and #227 record: pressed, nothing visible, no reason given.
    starting,
    startError,
    // "The count in the hexagon is transfers, not decisions" — the decisions are in the chip and the
    // band. `null` renders no numeral at all rather than a zero.
    numeral: hero === "syncing" ? changes : hero === "decision" ? waiting : null,
    transfers: transfersOf(activity),
    // The daemon's count of everything this pass has left to move, which is what sizes `+n more`.
    // `null` means unknown (not executing, or a daemon predating #211) — never 0.
    transfersRemaining: activity?.transfers_remaining ?? null,
  };
}

/**
 * The rows the columns draw, from the transfer window the reply carries (#211).
 *
 * TWO WIRE SHAPES, ONE LIST. A current daemon sends `transfers`; one predating #211 sends only the
 * singular `transfer`, which meant exactly "the row in flight". Reading the list when it has rows
 * and falling back to the mirror otherwise is the same rule the engine writes down once in
 * `SyncActivity::active_transfer`. It is NOT a merge: the daemon derives the mirror FROM the list,
 * so a reply carrying both carries the same transfer twice and reading the list wins.
 *
 * `detail` is the chip, and what goes in it is a fact about the row rather than one thing:
 *
 *   · a queued row says `queued`, which is what `2a Syncing` draws in that slot — same 39.61px chip
 *     as the size on the row above it, measured;
 *   · a batched download says `25 files`, because one row stands for a whole chunk landing in one
 *     folder and a folder name under a `←` would otherwise read as a filename;
 *   · everything else is the size when the daemon knows it, which is uploads only — a remote listing
 *     carries no size, so a download's chip is omitted rather than em-dashed (§63).
 *
 * EXPORTED, AND THE TRAY IMPORTS IT rather than keeping its own — the same rule, and for the same
 * reason, as `heroStateOf` two functions up: the window and the panel are one moment seen twice, and
 * the panel had a second copy of this that had already drifted (it read only the singular field, so
 * the day the list landed it would have kept drawing one row while the window drew six). `compact`
 * drops the chip only: `10a Syncing`'s rows are flat and neither of them carries one.
 */
export function transfersOf(activity, { compact = false } = {}) {
  const rows = activity?.transfers?.length
    ? activity.transfers
    : activity?.transfer
      ? [activity.transfer]
      : [];
  return rows.map((t) => ({
    direction: t.direction === "download" ? "down" : "up",
    name: t.path,
    detail: compact ? null : detailOf(t),
    // Verbatim, defaulting exactly as the wire does (an absent `state` is `active`, which is what
    // the singular field meant before the list existed). A token this build does not know draws an
    // unstyled row rather than being coerced into `active` — a row is still a real transfer.
    state: t.state ?? "active",
    // No percentage is computable from a reply that never carries both ends of one — see the
    // module header and `transferRow`'s own note. `null` means "no track", not "0%".
    progress: t.bytes_done != null && t.bytes_total != null ? t.bytes_done / t.bytes_total : null,
    // Carried so `hiddenTransfers` can weigh a batched row as its whole chunk.
    files: t.files ?? null,
  }));
}

function detailOf(t) {
  if (t.state === "queued") return MAIN.queued;
  if (t.files != null) return MAIN.batchFiles(t.files);
  return t.bytes_total == null ? null : bytes(t.bytes_total);
}

/**
 * How many transfers `+n more` stands for.
 *
 * NOT `transfers.length - shown.length`: the daemon caps the window it sends, so the list can never
 * be longer than what is drawn and that subtraction is always 0 — the `+n more` node would be dead
 * code that looks live. `transfers_remaining` is the daemon's own count of everything this pass has
 * left, and a batched row weighs as its whole chunk, which is the same arithmetic
 * `SyncActivity::transfers_past_the_window` does on the other side of the wire.
 *
 * `null` remaining is UNKNOWN — an older daemon, or a pass not executing yet — and yields no node
 * rather than `+0 more`.
 */
export function hiddenTransfers(remaining, shown) {
  if (remaining == null) return 0;
  const named = shown.reduce((total, t) => total + (t.files ?? 1), 0);
  return Math.max(0, remaining - named);
}

// ------------------------------------------------------------------------------ the copy ----

/** The headline, per state. */
export function headlineOf(v) {
  switch (v.hero) {
    case "syncing":
      return MAIN.syncing(v.pending);
    case "paused":
      return MAIN.paused;
    case "unreachable":
      // NOT `TRAY.unreachableTitle`, which is the deck's outage sentence and says `Can't reach
      // Proton Drive`. This state is the CONTROL SOCKET not answering — Proton is not on the far end
      // of that round trip, and a daemon that is running and cannot reach Proton lands in `failed`
      // or `authExpired` instead. Saying the wrong one here also left the button underneath
      // unexplained. DEVIATIONS §95.
      return MAIN.notRunning;
    case "authExpired":
      return MAIN.authExpired;
    case "failed":
      return MAIN.failed;
    case "decision":
      return MAIN.compact.needYou(v.waiting);
    default:
      return MAIN.settled;
  }
}

/**
 * The sub-line, per state — the one place Phase 1's data gaps are visible on this screen.
 *
 * Settled draws `last synced 2 minutes ago · 12,480 files · 41.2 GB` and the reply carries only the
 * timestamp: no command reports an index-wide file count or byte total (G7, #207). The clause is
 * omitted rather than faked, which is the fallback `14-behaviour-and-state.md` prescribes for a
 * missing capability, and `MAIN.settledSub` is left for the day #207 lands.
 */
export function subOf(v) {
  switch (v.hero) {
    case "syncing":
      // `?? null`, never `?? 0` — see `syncingSub`. The elapsed time is the PASS's, not the
      // phase's: `since_epoch_secs` is reset by `begin_activity` on every phase change, so a pass
      // walking scanning-local → listing-remote → executing counted up and jumped back to zero
      // three times. `activity.pass.started_epoch_secs` (#213) is the pass as a unit and does not.
      // The phase's start stays as the fallback for a daemon too old to send the block.
      return MAIN.syncingSub(
        since(v.activity?.pass?.started_epoch_secs ?? v.activity?.since_epoch_secs ?? v.lastSync),
        v.summary?.uploads ?? null,
        v.summary?.downloads ?? null,
      );
    case "paused":
      return MAIN.pausedSub(v.pending, clock(v.lastSync));
    case "unreachable":
      // A plain string, where every sibling here is a template. `v.pending` comes from a reply and
      // this is the state with no reply, so the count was never `0` — it was unknown, and
      // `unreachableBody(0)` rendered `0 changes are waiting`, a false all-clear on the one screen
      // that cannot see anything. Nothing to drop beats remembering to drop it.
      return MAIN.notRunningSub;
    case "authExpired":
      return MAIN.authExpiredSub(v.pending);
    case "failed":
      // The reassurance only. The daemon's string is a BLOCK below the hero, not a clause here:
      // the hero is a fixed 394px centring column, so a second line in it moves the hexagon — the
      // one thing `03-main-screen.md` says this screen must never do — and an error can be any
      // length at all. See `fillFailed`, which puts `.main-failed-error` inside the `.main-failed`
      // block `renderMain` creates.
      return MAIN.failedSub(v.pending);
    default:
      return MAIN.settledSubTime(since(v.lastSync));
  }
}

/** The sub-line the syncing hero shows once something is waiting: `3 other changes are waiting…`. */
function syncingSubWithDecisions(v) {
  return v.waiting > 0 ? MAIN.otherWaiting(v.waiting) : subOf(v);
}

function subTextOf(v) {
  return v.hero === "syncing" ? syncingSubWithDecisions(v) : subOf(v);
}

// ------------------------------------------------------------------------------ the pieces ----

/**
 * Which of the five forms the mark takes. `10-tray.md`: **only five forms exist** — a solid filled
 * hexagon is not a state and must not be reintroduced.
 *
 * `authExpired` shares the struck mark with `unreachable`, which is the design's own grouping:
 * `11-notifications.md` puts "an outage, expired session, or full disk" behind one struck `#FF3B3B`
 * icon. Both mean *Proton is out of reach*; only the sentence underneath differs.
 *
 * `failed` is the third of that trio — a full disk IS a failed pass — and
 * `14-behaviour-and-state.md`'s state diagram already says where it goes: "unreachable is entered
 * after a failed pass and retry". The mark is the same; the sentence and the quoted string differ.
 */
const MARK_STATE = {
  syncing: "syncing",
  decision: "needsNumeral",
  paused: "paused",
  unreachable: "unreachable",
  authExpired: "unreachable",
  failed: "unreachable",
  settled: "settled",
};

function heroMark(v) {
  const state = MARK_STATE[v.hero];
  if (!state) throw new Error(`main: no mark measured for hero state "${v.hero}"`);
  return renderHexagon({
    size: HERO_SIZE,
    state,
    // `masked` tracks whether the mark sits OVER the seam, not which state it is in (DEVIATIONS
    // §25) — so it follows the same condition the seam does, and `2a Settled`'s mark correctly
    // carries no fill.
    masked: v.hero === "syncing",
    numeral: v.numeral,
    class: "main-mark",
  });
}

/** One side of the seam: the direction's label, and the folder it stands for. */
function sideLabel(side, root) {
  const up = side === "local";
  return el(
    "div",
    { class: `main-side main-side-${side}` },
    eyebrow({
      tone: up ? "up" : "down",
      text: up ? MAIN.sideLocal : MAIN.sideRemote,
      align: up ? "start" : "end",
    }),
    el("div", { class: "main-side-path" }, root ?? "—"),
  );
}

/**
 * WHICH buttons the hero has, as data — the rendering is `heroActions` below.
 *
 * Split out and exported for the same reason `heroStateOf` is: this is a decision per state, the
 * states that get it wrong are the ones no frame draws, and the alternative is a DOM. Every other
 * check in this file's suite reads a plain object, and a branch that returns the wrong control is
 * exactly the failure a fidelity gate cannot see — it compares one rendering of one drawn frame.
 *
 * `on` is a KEY into the handlers object rather than a function, which is what keeps this pure.
 * `null` means the button is inert on purpose; nothing else may leave it unset.
 */
export function heroActionsOf(v) {
  if (v.hero === "paused") {
    return [{ label: MAIN.resume, kind: "secondaryOutlined", on: "onResume" }];
  }
  if (v.hero === "unreachable") {
    // THE DAEMON IS NOT RUNNING, and this branch is split out of the three-way one below because
    // `Try again now` there is `onSyncNow` — an IPC round trip to the control socket. `unreachable`
    // is precisely the state in which that socket did not answer, so the retry was a button that
    // could not do what it said: it re-asked a process that is not there, failed the same way, and
    // redrew the same screen. The one control that changes anything is starting the service.
    //
    // `starting` disables it while `start_service` is in flight, which is seconds and not
    // milliseconds: `systemctl --user start` blocks until the unit reports started. A control that
    // looks untouched for three seconds gets clicked again, which is the same complaint the delete
    // approvals answered with a busy-disable (#140).
    //
    // The DISABLED ATTRIBUTE and not a `…Disabled` KIND. The design tokenises a disabled fill for
    // `primary` and `destructive` only — there is no `secondaryDisabled` — and inventing one means
    // two new tokens in both themes and a contrast check for a state no frame draws. `button` drops
    // the click handler for either form (`onClick: disabled || role.disabled ? null : onClick`), so
    // the trap that matters is covered: this button cannot be disabled-looking and live.
    return [
      v.starting
        ? { label: MAIN.starting, kind: "secondaryOutlined", on: null, disabled: true }
        : { label: TRAY.start, kind: "secondaryOutlined", on: "onStartService" },
    ];
  }
  if (v.hero === "authExpired" || v.hero === "failed") {
    // `Try again now` and not `11a Outage`'s `Sign in`: NOTHING IN THE COMMAND SURFACE SIGNS IN.
    // Re-authentication is `proton-drive login` in a terminal — the daemon reuses that CLI's keyring
    // session — so a `Sign in` button here would be a control with no action behind it, which is
    // worse than the honest one. Retrying is exactly right once the user has signed in elsewhere.
    // DEVIATIONS §67.
    //
    // BOTH OF THESE ANSWER, which is what separates them from `unreachable` above and is why that
    // one moved out: `syncnow` is an IPC round trip, so it does something here — it runs the pass
    // that failed, or the pass a re-signed-in session can now finish — and did nothing at all on a
    // socket with no daemon behind it. `Pause` is dropped from both: nothing is moving to pause.
    return [{ label: TRAY.tryAgain, kind: "secondaryOutlined", on: "onSyncNow" }];
  }
  const buttons = [];
  if (v.hero !== "syncing") buttons.push({ label: MAIN.syncNow, kind: "secondaryOutlined", on: "onSyncNow" });
  // Filled while syncing and outlined when settled: the mid-sync button sits ON the seam and its
  // own fill is what masks the hairline behind it (`seam.js` rule 3 — pass `surface:null` and keep
  // the button's fill). Both are the same colour role; only the surface differs.
  buttons.push({
    label: MAIN.pause,
    kind: v.hero === "syncing" ? "secondaryFilled" : "quietOutlined",
    on: "onPause",
  });
  return buttons;
}

/**
 * The hero's buttons, rendered.
 *
 * `Sync now` DISAPPEARS mid-sync — "it's meaningless mid-sync" — and that is the only reason the
 * count of buttons changes, which is why `updateMain` treats a change here as a rebuild of the row
 * rather than something to patch. (`starting` is the one prop that changes the row WITHOUT changing
 * the hero, and `updateMain` rebuilds on it explicitly.)
 */
function heroActions(v, handlers) {
  const buttons = heroActionsOf(v).map((spec) =>
    action(spec.label, spec.kind, spec.on ? handlers[spec.on] : null, Boolean(spec.disabled)),
  );
  return el("div", { class: "main-actions" }, buttons);
}

function action(label, kind, onClick, disabled = false) {
  return button({ kind, size: "bar", fontSize: "13.5px", label, onClick: onClick ?? null, disabled });
}

/**
 * The attention band's rows, one per CATEGORY — never one per item. Two conflicts and a deletion
 * queue are one interruption; three stacked boxes would read as three (`bands.js`).
 *
 * A band routes and never acts: both buttons open the screen that owns the decision. `bands.js`
 * enforces that by only offering the `decision` kind, and it is the reason there is no `Approve` here.
 */
/**
 * The band's deletion queue split by WHERE EACH ROW ENDS UP — three destinations, not two.
 *
 * `remote` acts on Proton Drive, so that copy goes to Proton's Trash. `local` acts on this
 * computer, and what that means is `local_delete_mode`'s answer, asked through `severityOfItem` so
 * this is not a second copy of the rule the Deletions screen sorts its columns by.
 *
 * WHY IT IS ITS OWN FUNCTION. Folding both recoverable kinds into one count made the band say
 * `2 go to Proton's Trash` about files sitting on this disk — the wrong trash, in the sentence
 * whose whole job is telling you what you are about to lose. `columnCopy` refuses exactly this one
 * screen later. Exported because the decision is here and the sentence is only its rendering.
 */
export function deletionCountsOf(deletions) {
  const permanent = deletions.filter((d) => severityOfItem(d) === "permanent").length;
  const protonTrash = deletions.filter((d) => d?.direction === "remote").length;
  return { permanent, protonTrash, localTrash: deletions.length - permanent - protonTrash };
}

function bandItems(v, handlers) {
  const items = [];
  if (v.conflicts.length) {
    items.push({
      tone: "decision",
      title: MAIN.band.conflictTitle(v.conflicts.length),
      // The path is the first conflict's, which is the whole story only while there is one of them.
      // The deck gives no plural form for this clause; S2 owns the queue and can settle it there.
      note: MAIN.band.conflictSub(v.conflicts[0].original),
      action: bandButton({ label: MAIN.band.conflictAction, onClick: handlers.onConflicts }),
    });
  }
  if (v.deletions.length) {
    // ASKED OF `severityOf`, not re-derived. `local` applies the delete on this computer — the
    // permanent one — and `remote` moves Proton's copy to the Trash; column and direction name the
    // same side from opposite ends, and `fixtures/deletions.js` documents the pairing at length.
    // This line used to test `d.direction === "local"` itself, which is a second copy of the rule
    // the Deletions screen sorts its two columns by: the band's `N remove permanently` and the
    // screen's left column would then be free to disagree about the same list.
    const { permanent, protonTrash, localTrash } = deletionCountsOf(v.deletions);
    items.push({
      tone: "destructive",
      title: MAIN.band.deletionTitle(v.deletions.length),
      note: MAIN.band.deletionSub(permanent, protonTrash, localTrash),
      action: bandButton({ label: MAIN.band.deletionAction, onClick: handlers.onDeletions }),
    });
  }
  return items;
}

// ------------------------------------------------------------------------------ mount ----

/**
 * What is currently on screen, so a poll can patch it. Module-level because exactly one main screen
 * exists at a time — the same reasoning `app.js`'s `dom` cache is built on, one level in.
 */
let view = null;

/** Build the screen. Returns the window-root siblings, in order. */
export function renderMain(props = {}) {
  const v = mainView(props);
  const handlers = props.handlers ?? {};

  const hero = el("div", { class: "main-hero" });
  const mark = heroMark(v);
  const headline = el("div", { class: "main-headline" }, headlineOf(v));
  const sub = el("div", { class: "main-sub" }, subTextOf(v));
  const glow = el("div", { class: "hex-glow", "aria-hidden": "true" });
  const seam = renderSeam({ site: seamSiteOf(v) });
  const sides = [sideLabel("local", v.localRoot), sideLabel("remote", v.remoteRoot)];

  // The seam is the FIRST child and nothing wraps it: everything positioned after it in the DOM
  // paints over it, which is half of how the masks work (`seam.js` rule 3).
  if (v.hero === "syncing") hero.append(seam, ...sides);
  else if (v.hero === "settled") hero.append(glow);
  hero.append(mark, headline, sub);
  const actions = heroActions(v, handlers);
  hero.append(actions);
  applyMasks(v, { headline, sub });

  const columns = el("div", { class: "main-columns" });
  const spacer = el("div", { class: "main-spacer" });
  const failed = el("div", { class: "main-failed" });
  const bandWrap = el("div", { class: "main-band-wrap" });

  view = {
    v,
    handlers,
    hero,
    mark,
    headline,
    sub,
    glow,
    seam,
    sides,
    actions,
    columns,
    spacer,
    failed,
    bandWrap,
  };
  fillColumns(v);
  fillFailed(v);
  fillBand(v, handlers);
  stampFids(v);
  return blocksOf(v);
}

/**
 * Patch what is on screen. Returns the (possibly reordered) blocks so the shell can splice them, or
 * `null` when nothing is mounted.
 *
 * NOTHING HERE REBUILDS THE MARK UNLESS ITS STATE CHANGED. That is the whole point of the function:
 * `2a Syncing` → `2a Needs you` is a band appearing under an unchanged hexagon, and re-rendering it
 * would restart `hexup` and `hexdn` from 0% twice a second.
 */
export function updateMain(props = {}) {
  if (!view) return null;
  const next = mainView(props);
  const handlers = props.handlers ?? view.handlers;
  const prev = view.v;

  if (next.hero !== prev.hero) {
    crossfadeMark(next);
    // The blocks the hero grows and loses with its state. Everything is prepended, so the mark, the
    // headline and the sub-line are never re-parented — a moved node restarts its own animations,
    // which is the failure this whole update path exists to avoid.
    if (next.hero === "syncing") {
      // REBUILT, never re-attached. The seam held here was built for the site the screen mounted in,
      // and entering syncing while a decision is already waiting needs the SHORT one — re-attaching
      // the mount-time `mainHero` runs a 150px overhang straight into the attention band, which is
      // the rule-2 violation `auditSeams` exists to catch and which no frame exercises, because a
      // frame is one rendering and this is a transition between two.
      view.seam = renderSeam({ site: seamSiteOf(next) });
      view.hero.prepend(view.seam, ...view.sides);
    } else {
      view.seam.remove();
      for (const side of view.sides) side.remove();
    }
    if (next.hero === "settled") view.hero.prepend(view.glow);
    else view.glow.remove();
    const actions = heroActions(next, handlers);
    view.actions.replaceWith(actions);
    view.actions = actions;
  } else if (next.hero === "syncing" && bandShowing(next) !== bandShowing(prev)) {
    // The seam shortens to stop above the band and lengthens again when it goes: two drawn sites,
    // not one site with a computed height (`seam.js` SEAM_SITES).
    const seam = renderSeam({ site: seamSiteOf(next) });
    view.seam.replaceWith(seam);
    view.seam = seam;
  }

  // `starting` IS THE ONE PROP THAT CHANGES THE BUTTONS WITHOUT CHANGING THE HERO, and the rebuild
  // above only runs on a hero change — so without this the busy state was unreachable: the click
  // set the flag, the ~2s poll called `updateMain`, and the row on screen stayed the row built
  // before the click. A separate `if` rather than a fourth arm of that chain, because it is not an
  // alternative to any of them; when the hero DID change, the rebuild has already covered it.
  if (next.hero === prev.hero && next.starting !== prev.starting) {
    const actions = heroActions(next, handlers);
    view.actions.replaceWith(actions);
    view.actions = actions;
  }

  updateHexagon(view.mark, { numeral: next.numeral });
  setText(view.headline, headlineOf(next));
  setText(view.sub, subTextOf(next));
  view.sides[0].querySelector(".main-side-path").textContent = next.localRoot ?? "—";
  view.sides[1].querySelector(".main-side-path").textContent = next.remoteRoot ?? "—";
  applyMasks(next, view);

  view.v = next;
  view.handlers = handlers;
  fillColumns(next);
  fillFailed(next);
  fillBand(next, handlers, bandShowing(next) && !bandShowing(prev));
  stampFids(next);
  return blocksOf(next);
}

/** Forget the mounted screen. The shell removes the nodes; this drops the references to them. */
export function unmountMain() {
  endFade?.();
  view = null;
}

// ------------------------------------------------------------------------------ internals ----

const bandShowing = (v) => v.waiting > 0;
const seamSiteOf = (v) => (bandShowing(v) ? "mainHeroAttention" : "mainHero");

/**
 * Which block fills the space under the hero — and it is a THREE-way answer, not two.
 *
 * The middle block is what holds the window open to its full height, so exactly one of these is
 * always present: the transfer columns while syncing, the daemon's quoted string on a failed pass,
 * and an empty `flex:1` spacer otherwise. A failed pass with no string to quote (nothing sets
 * `failed` without one, but the screen takes its props from a caller and not from `derive_state`)
 * falls back to the spacer rather than drawing an empty box — the failure mode #247 exists for.
 */
function blocksOf(v) {
  const middle = v.hero === "syncing" ? view.columns : quotingError(v) ? view.failed : view.spacer;
  const blocks = [view.hero, middle];
  if (bandShowing(v)) blocks.push(view.bandWrap);
  return blocks;
}

/**
 * The string the middle block quotes, or `null` — and it comes from one of two places.
 *
 * A failed PASS is the daemon's `last_error`. A failed START has no daemon to have said anything, so
 * it is the `start_service` rejection instead. Both are machine text quoted verbatim in mono (voice
 * rule 4), both go in the same capped, scrolling block, and neither state draws it in a frame.
 *
 * ONE FUNCTION rather than a condition here and the string picked again in `fillFailed`: they are
 * the same question asked twice, and the second copy is where the two answers drift apart.
 */
export function quotedError(v) {
  if (v.hero === "failed") return v.error ?? null;
  if (v.hero === "unreachable") return v.startError ?? null;
  return null;
}

const quotingError = (v) => Boolean(quotedError(v));

/** Only write when it changed: an unchanged assignment still invalidates layout for the whole line. */
function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

/**
 * The seam mask, applied and removed with the seam itself.
 *
 * Both padding values are the frame's, not a tier: the 32px headline takes 18px and the 13px mono
 * sub-line takes 16px with 2px above and below — `seam.js` §37 records that no function of font-size
 * reproduces the set, so a screen quotes its own frame.
 */
function applyMasks(v, nodes) {
  const masked = v.hero === "syncing";
  seamMask(nodes.headline, masked ? { pad: 18 } : { pad: null, surface: null });
  seamMask(nodes.sub, masked ? { pad: 16, padY: 2 } : { pad: null, surface: null });
  if (!masked) {
    // `seamMask` only ever sets, so clearing is the caller's job. `position` stays: every hero
    // element is `position:relative` in `2a Settled` too, to stack over the glow.
    for (const node of [nodes.headline, nodes.sub]) {
      node.style.removeProperty("background");
      node.style.removeProperty("padding");
    }
  }
}

/**
 * Swap the mark for one in the new state, over 220ms, WITHOUT MOVING IT.
 *
 * A true crossfade needs both marks on screen at once, and the incoming one must not take space or
 * the hero reflows and the hexagon jumps — which is the one thing `03-main-screen.md` says it must
 * never do, and a line item on the design's own acceptance checklist. So the incoming mark is
 * absolutely positioned over the outgoing one's exact box for the length of the fade and then drops
 * back into flow, at which point the outgoing one is removed.
 *
 * A `transition` rather than an `animation`: `animation-name` and its three friends are asserted
 * properties, and a fade declared as an animation would sit on the mark at rest and fail every frame
 * that maps it. A transition is invisible to a gate that reads a static tree, which is exactly right
 * for something that only exists between two states.
 */
let fadeTimer = null;
let endFade = null;

function crossfadeMark(next) {
  // A second state change inside 220ms lands here with a fade still running. Settle the first one
  // now rather than layering two: the alternative leaves an absolutely-positioned mark that the
  // next call adopts as its outgoing node and never puts back into flow.
  endFade?.();

  const outgoing = view.mark;
  const incoming = heroMark(next);
  // Pinned to the outgoing mark's exact box. An SVG element has no `offsetTop`, so this is measured
  // rather than read — and the hero has no border, so its border box and its padding box share an
  // origin and the two rects subtract cleanly.
  const from = outgoing.getBoundingClientRect();
  const hero = view.hero.getBoundingClientRect();
  incoming.classList.add("is-entering");
  incoming.style.top = `${from.top - hero.top}px`;
  incoming.style.left = `${from.left - hero.left}px`;
  outgoing.after(incoming);
  view.mark = incoming;

  // The incoming mark stays OUT OF FLOW for the whole fade. Dropping it in while the outgoing one is
  // still there would put two 168px marks in a centring column and move the hexagon — the one thing
  // this screen must never do.
  endFade = () => {
    clearTimeout(fadeTimer);
    fadeTimer = null;
    endFade = null;
    outgoing.remove();
    incoming.classList.remove("is-entering");
    incoming.style.removeProperty("top");
    incoming.style.removeProperty("left");
    incoming.style.removeProperty("opacity");
  };

  // Two frames: one for the engine to accept the starting opacity, one to start the transition from
  // it. Setting both in the same frame is a style change with nothing to transition from.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      incoming.style.opacity = "1";
      outgoing.style.opacity = "0";
    }),
  );
  incoming.addEventListener("transitionend", () => endFade?.(), { once: true });
  // Under `prefers-reduced-motion` `main.css` drops the transition and `transitionend` never fires.
  // Without this the outgoing mark would sit over the new one forever.
  fadeTimer = setTimeout(() => endFade?.(), 400);
}

/**
 * What a list is made of, as a string — so a poll that changes nothing rebuilds nothing.
 *
 * THIS IS NOT AN OPTIMISATION AND THE COMMENT ON `app.js`'s `dom` cache says why. The shell
 * re-renders every ~2 seconds; `replaceChildren` on a block a user has tabbed into drops focus to
 * `<body>` inside two ticks. The band holds `Compare` and `Review` — the two controls this screen
 * exists to offer — so rebuilding it on a timer makes them unreachable from the keyboard, which
 * `14-behaviour-and-state.md` requires "because this is a desktop app". Same hazard the shell hit,
 * one layer in, and the same answer: rebuild on a change, never on a tick.
 *
 * `JSON.stringify` RATHER THAN A JOIN, because every separator is a wrong answer here. The parts are
 * filenames and copy: any printable character can occur inside one, so `["ab", "c"]` and
 * `["a", "bc"]` collide and the list silently keeps a stale row. Picking a character no filename can
 * contain lands on a non-printable one — this line held a literal U+0001 for exactly that reason,
 * which Copilot caught, and a control character in source is what an editor or a diff quietly eats.
 * The encoding is unambiguous by construction, needs no magic value, and is legible.
 */
const signature = (parts) => JSON.stringify(parts);

/**
 * The two columns and their rows. Rebuilt when the set of rows changes, which is a row's whole
 * identity here — a file name, a direction, a size and a fraction. There is no per-row animation to
 * preserve, so nothing finer than "did the list change" is needed.
 */
function fillColumns(v) {
  const sig =
    v.hero !== "syncing"
      ? ""
      : signature([
          // `state` is in the identity because a row flipping queued → active is the change that
          // grows it a progress track and a body wrapper, and it can happen with the name, the
          // direction and the (absent) fraction all unchanged — the poll would keep the stale row.
          ...v.transfers.map(
            (t) => `${t.direction}|${t.name}|${t.detail}|${t.progress}|${t.state}|${t.files}`,
          ),
          // And the tail count, so `+n more` counting down is a rebuild even when the six drawn
          // rows are identical.
          v.transfersRemaining,
        ]);
  if (view.columnsSig === sig) return;
  view.columnsSig = sig;

  if (v.hero !== "syncing") {
    view.columns.replaceChildren();
    return;
  }
  const left = el("div", { class: "main-column main-column-left" });
  const right = el("div", { class: "main-column main-column-right" });
  const shown = v.transfers.slice(0, MAX_ROWS);
  for (const t of shown) (t.direction === "up" ? left : right).append(transferRow(t));
  const hidden = hiddenTransfers(v.transfersRemaining, shown);
  if (hidden > 0) right.append(el("div", { class: "main-more" }, MAIN.andMore(hidden)));
  view.columns.replaceChildren(left, right);
}

// The two columns are always both drawn, even when one is empty: the grid's 1fr 1fr is what puts the
// seam between them, and a single child would centre one column across the whole width.

/**
 * The daemon's exact string, quoted, in the block below the hero (#246).
 *
 * NOT A LINE IN THE HERO, and the reason is the property the whole screen is built on: the hero is
 * a fixed 394px block that CENTRES its column, so a fourth line in it moves the hexagon by half a
 * line — "the hexagon does not move between states of the same screen", checklist item 2. Down here
 * the block is `flex:1` in the space the spacer would have held, and nothing above it shifts.
 *
 * Same treatment the string gets on three other screens — `.pl-failed-error` (S4's failed
 * rehearsal), `.ob-working-error` (S7's failed first sync) and `.pass-error` (S5's failed pass):
 * mono, on `--surface`, so it reads as a quotation rather than as more of our sentence. Voice rule
 * 4 — never paraphrase a daemon error. It passes through no formatter at all.
 *
 * The height is CAPPED and it scrolls, which is `.ob-working-error`'s lesson rather than
 * `.pl-failed-error`'s: a daemon that fails with a long stderr grows this block until it paints
 * over the footer (DEVIATIONS §79f, measured at 10 KB). Rebuilt only when the string changes, on
 * the ~2s poll's own rule — a `replaceChildren` here would drop a selection mid-drag, and this is
 * the one block on the screen whose text somebody has a reason to select and copy.
 */
function fillFailed(v) {
  const quoted = quotedError(v);
  const sig = quoted ?? "";
  if (view.failedSig === sig) return;
  view.failedSig = sig;
  if (!quoted) {
    view.failed.replaceChildren();
    return;
  }
  view.failed.replaceChildren(el("div", { class: "main-failed-error" }, quoted));
}

/**
 * The band, and the one animation on this screen that must not exist on a first render.
 *
 * `arriving` is true only when a decision turns up on a screen that was already showing — the
 * design's own trigger ("New decision arrives → attention band slides in"). Mounting straight into
 * the banded state is not an arrival, and declaring the slide unconditionally would leave the
 * fidelity gate measuring `opacity:0` on a node the frame records at 1, because the harness freezes
 * every animation at its first keyframe. See `main.css`.
 */
function fillBand(v, handlers, arriving = false) {
  const items = bandItems(v, handlers);
  const sig = signature(items.map((i) => `${i.title}|${i.note}`));
  if (view.bandSig === sig) return;
  view.bandSig = sig;

  if (!items.length) {
    view.bandWrap.replaceChildren();
    view.bandWrap.classList.remove("is-entering");
    return;
  }
  view.bandWrap.replaceChildren(attentionBand({ items }));
  if (!arriving) return;
  // Removed and re-added around a forced reflow, not simply added: adding a class an element already
  // carries does not restart its animation, so a SECOND decision arriving after a first would appear
  // without sliding. It can already be there — under `prefers-reduced-motion` the animation is `none`
  // and `animationend` never fires to take it off.
  view.bandWrap.classList.remove("is-entering");
  void view.bandWrap.offsetWidth;
  view.bandWrap.classList.add("is-entering");
  view.bandWrap.addEventListener("animationend", () => view.bandWrap.classList.remove("is-entering"), {
    once: true,
  });
}

/**
 * Hand every mapped node its `data-fid`. A no-op in the live app — `fid()` only stamps when a
 * `?frame=` label is selected and that frame declares the slot — so this costs nothing at runtime
 * and is the only thing that makes the style gate able to see this screen at all.
 *
 * Stamped after every rebuild rather than once at mount, because the columns and the band are
 * replaced wholesale and a fresh node carries no attribute.
 */
function stampFids(v) {
  fid(view.hero, "hero");
  fid(view.mark, "hexagon");
  for (const [i, path] of [...view.mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
  fid(view.mark.querySelector("text"), "hexNumeral");
  const defs = view.mark.querySelector("defs");
  if (defs) {
    fid(defs, "hexDefs");
    for (const [i, grad] of [...defs.children].entries()) {
      fid(grad, "hexGradient", i);
      for (const [j, stop] of [...grad.children].entries()) fid(stop, "hexStop", i, j);
    }
  }
  fid(view.headline, "headline");
  fid(view.sub, "sub");
  fid(view.actions, "actions");
  for (const [i, btn] of [...view.actions.children].entries()) fid(btn, "action", i);

  if (v.hero === "syncing") {
    fid(view.seam, "seam");
    fid(view.sides[0], "sideLocal");
    fid(view.sides[0].children[0], "sideLocalLabel");
    fid(view.sides[0].children[1], "sideLocalPath");
    fid(view.sides[1], "sideRemote");
    fid(view.sides[1].children[0], "sideRemoteLabel");
    fid(view.sides[1].children[1], "sideRemotePath");
    fid(view.columns, "columns");
    fid(view.columns.children[0], "columnLeft");
    fid(view.columns.children[1], "columnRight");
    // EVERY row in BOTH columns, keyed by side and position (#211). Which of them the frame
    // actually declares is `mainFids`' business — a factory slot may answer `null` for a row this
    // frame does not draw, or for a child whose shape differs from the drawn one. The `+n more`
    // node is a child of the right column too, so the row query is scoped to `.transfer-row`.
    // 0 = left, 1 = right — an INDEX, not a name, because `mainFids` is probed with numbers.
    for (const [side, column] of [
      [0, view.columns.children[0]],
      [1, view.columns.children[1]],
    ]) {
      const drawn = [...(column?.querySelectorAll(".transfer-row") ?? [])];
      for (const [i, row] of drawn.entries()) {
        fid(row, "transferRow", side, i);
        fid(row.querySelector(".transfer-body"), "transferBody", side, i);
        fid(row.querySelector(".transfer-name"), "transferName", side, i);
        fid(row.querySelector(".transfer-detail"), "transferDetail", side, i);
        fid(row.querySelector(".transfer-arrow"), "transferArrow", side, i);
        fid(row.querySelector(".transfer-track"), "transferTrack", side, i);
        fid(row.querySelector(".transfer-fill"), "transferFill", side, i);
      }
    }
  } else {
    fid(view.glow, "glow");
    fid(view.spacer, "spacer");
  }

  if (bandShowing(v)) {
    fid(view.bandWrap, "bandWrap");
    const band = view.bandWrap.firstElementChild;
    fid(band, "band");
    for (const [i, item] of [...(band?.children ?? [])].entries()) {
      fid(item, "bandItem", i);
      fid(item.querySelector(".dot"), "bandDot", i);
      const body = item.querySelector(".band-item-body");
      fid(body, "bandBody", i);
      fid(body?.children[0], "bandTitle", i);
      fid(body?.children[1], "bandNote", i);
      fid(item.querySelector("button"), "bandAction", i);
    }
  }
}
