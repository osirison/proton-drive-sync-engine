// Client state + derived selectors (F3). The single source of truth for the whole window, so the
// conflict count (which appears in five places), chip counts, and stat values are computed once.
//
// KEYED BY FOLDER PAIR (#102 phase 5a-2, brief section 4.6). Everything that describes ONE pair — its
// last status, its conflicts, its withheld deletions, the resolutions staged against it — lives in
// `byPair[name]`, and every `select.*` answers for the SELECTED pair's slice, so the screens that
// already read `store.select.*` are pair-scoped by construction and did not change. What is not
// per-pair stays at the top: which pair is selected, the roster of pairs and the state of each.
//
// WHAT THIS BUYS BEFORE ANY SELECTOR EXISTS: a late reply cannot land in the wrong place. A conflict
// scan that was issued for pair A and answers after the selection moved to B is FILED UNDER A, and B's
// screen never sees it — which is the "reply that lands after a switch is dropped" rule (4.6) done by
// construction instead of by a comparison every caller has to remember to make. And every list the
// store hands out carries the pair it belongs to (`item.pair`), so a click on a row acts on the pair
// the row was rendered for, never on whichever is selected when the click arrives.
//
// ONE PAIR LOOKS EXACTLY AS IT DID. A legacy daemon, every fixture and the moment before the first
// poll all file under `DEFAULT_PAIR`; there is one slice, it is the selected one, and the screens read
// what they always read.

import { pairTable } from "./pairmap.js";

const listeners = new Set();

/** The pair a legacy reply, a fixture, and the time before the first reply all belong to. */
export const DEFAULT_PAIR = "default";

/**
 * The date of a list that no request has filled, or that a caller did not date. Below every real clock reading
 * (the clock starts at 1, and a list is fresh only when its date is NOT BELOW a reading), so "never filled" and
 * "filled without a date" need no flag beside the number — a flag and a number that can disagree is two places
 * saying one thing.
 */
const UNDATED = -1;

const emptySlice = () => ({
  status: null, // last get_status payload for THIS pair: { state, response, error }
  statusIssue: 0, // which status REQUEST produced `status` — see `beginStatus`
  conflicts: [], // last scan_conflicts result for this pair (the unresolved set), each tagged `pair`
  conflictsIssue: UNDATED, // the status clock when the scan behind the list above LEFT — see `setConflicts`
  pendingDeletions: [], // this pair's withheld deletions (S9), each tagged `pair`
  deletionsFiled: false, // has a reply ever filled the list above? `[]` alone cannot say "not yet"
  deletionsIssue: UNDATED, // which status REQUEST produced that list — see `select.deletionsFreshOf`
  staged: {}, // path -> Resolution, for the Conflicts screen (staged, not yet applied)
});

const state = {
  /** The pair the window shows. `null` until a status has said. */
  selected: null,
  /** The newest request whose answer moved `selected`, so a slow old reply cannot move it back. */
  selectedIssue: 0,
  /**
   * What moves `selected`. The window FOLLOWS THE SELECTION (`payload.selected`, the app's choice);
   * the tray panel is pinned to the pair its rows act on and follows the pair each REPLY describes
   * (`configure`). One webview showing the selected pair next to a menu that pauses the default one
   * is the wrong-target action this whole phase exists to prevent.
   */
  follows: "selection",
  /** Every pair the daemon runs, as its last reply listed them (`PairSummary[]`). */
  pairs: [],
  /**
   * The folder the app remembered and the running daemon does not run (`payload.pair_unknown`), or null.
   * The window shows the default folder meanwhile, and this is what lets it say so (5c-1).
   */
  pairUnknown: null,
  /** The derived state of each, by name (`{ name, state }[]`), computed in Rust. */
  pairStates: [],
  /**
   * Which status REQUEST produced the roster above (`beginStatus`'s clock). The roster is the only place the
   * window learns what a folder that is not on screen is doing — whether it is paused — and a fact is only
   * evidence about something that happened AFTER the request that carried it left (`statusIssue` is the
   * same clock for the selected folder's own reply).
   */
  pairsIssue: 0,
  /**
   * The newest status request that FAILED without naming a folder — the poll, or a tray action, whose socket
   * did not answer. The roster above survives it (it is how the window still NAMES the folders), but it is
   * then older than the failure, and `select.rosterLive` is the one place that says so: a roster whose
   * request is not newer than the last failure describes a daemon that has since stopped answering.
   */
  pairsFailedIssue: 0,
  /**
   * What the roster has said about each folder over time, by name (`noteRoster`): the request that FIRST listed
   * it in this stretch of membership (`joined`), its pending-deletion count, and the request that first
   * reported THAT count (`countIssue`). A list fetched for a folder is evidence about the folder only if it was
   * fetched after these — see `select.deletionsFreshOf` and `select.conflictsFreshOf`.
   */
  rosterFacts: pairTable(),
  byPair: pairTable(),
  ledgerFilter: "all",
};

// A `Map`, never an object: a pair may be named `constructor` or `__proto__` (`pairmap.js`).
function sliceFor(name) {
  let slice = state.byPair.get(name);
  if (!slice) {
    slice = emptySlice();
    state.byPair.set(name, slice);
  }
  return slice;
}
const selectedName = () => state.selected ?? DEFAULT_PAIR;
const viewed = () => state.byPair.get(selectedName()) ?? EMPTY;
const EMPTY = Object.freeze(emptySlice());

/**
 * How many status requests have been ISSUED — the clock that says whether an answer is newer than
 * something that happened in the window (#335).
 *
 * WHY "ISSUED" AND NOT "RECEIVED". `state.status` is the last poll to have *completed*, which can
 * be older than it looks: a poll issued before some event lands after it, carrying evidence from
 * before. Anything deciding "did I observe this after X?" must compare against the moment the
 * request left, so `beginStatus` allocates the number and the answer carries it home.
 *
 * The one consumer today is the Settings screen's restart latch, where getting it wrong deletes the
 * state before it is ever drawn: the restart only reaches its stop-then-start path *because* the
 * daemon was up, so the last completed poll is necessarily reachable and would retire a latch that
 * says nothing is running.
 */
let statusesIssued = 0;

/** Allocate the id for a status request that is about to be issued. Call it BEFORE the request. */
export function beginStatus() {
  statusesIssued += 1;
  return statusesIssued;
}

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
function emit() {
  for (const fn of listeners) fn(state);
}

/**
 * Choose what moves the viewed pair. `"selection"` (the default) follows the app's choice;
 * `"reply"` follows the pair each status reply describes — the tray panel's mode, where the rows act
 * on the default pair and so the panel must show that pair whatever the window has selected.
 */
export function configure({ follows }) {
  if (follows !== "selection" && follows !== "reply") {
    throw new Error(`store.configure: follows must be "selection" or "reply", got ${follows}`);
  }
  state.follows = follows;
}

/**
 * The pair a payload is FILED under: the one its reply describes, else the one that was asked about.
 * Exported so a caller filing something that RIDES on the payload (its withheld deletions) files it
 * under the same pair, by the same rule, and not by a second reading of the payload.
 */
export function pairOf(payload, asked = null) {
  const described = payload?.response?.pair;
  if (described) return described;
  // A FAILURE IS ABOUT THE FOLDER THAT WAS ASKED FOR. Rust stamps a failed read with the SELECTED folder
  // (`selected`), which is who the window is showing and not who the request was for: a failed read of
  // another folder, filed under that, flipped the shown folder's chip, list and unsaved notice to
  // `unreachable` for a read that was never about it. Only a request that named a folder knows.
  if (asked) return asked;
  if (state.follows === "selection" && payload?.selected) return payload.selected;
  return selectedName();
}

/**
 * Publish a status answer. `issue` is the id [`beginStatus`] returned for the request that produced
 * it — **required**, and deliberately not defaulted to "allocate one now": a set-time id would
 * stamp a request issued *before* some event as though it had been issued after, which is the one
 * direction that is unsafe.
 *
 * The reply is filed under the pair it describes. The selected pair moves only if this reply is ABOUT
 * the pair it selects, and is the newest to say so (`issue` ≥ the last that did): replies can arrive
 * out of order, and an old one reporting the previous selection must not win it back.
 *
 * `asked` is the folder the request NAMED (`get_status({ pair })`), or `null` for the poll, which names none.
 * It only matters for a payload with no reply, which cannot say who it was about (see `pairOf`).
 */
export function setStatus(payload, issue, asked = null) {
  // A REFUSAL IS NOT AN OBSERVATION. A request the app turned back before it left — a name no folder
  // has — comes home as a payload with no reply, `pair_unknown` set, and a placeholder `state` that
  // `StatusPayload` documents a caller must not read. Filed, it would draw the folder it was about as
  // unreachable on a daemon that never stopped answering.
  if (payload?.pair_unknown && !payload.response) return;
  const name = pairOf(payload, asked);
  const slice = sliceFor(name);
  slice.status = payload;
  slice.statusIssue = issue;
  // The withheld deletions RIDE ON THE REPLY, so they are filed with it, in the same publish. Filed in
  // a second call after this one (as the poll used to), the pair that has just become selected showed
  // an EMPTY queue for one render — `Nothing waiting to be deleted` — before its own arrived, which
  // is a flash at start-up and, across a switch, the whole screen redrawn from nothing. A payload with
  // no reply (the socket failed) says nothing new about the queue, and leaves what was last seen.
  if (payload?.response) {
    slice.pendingDeletions = tagged(payload.response.pending_deletions, name);
    slice.deletionsFiled = true;
    // WHICH REQUEST PRODUCED IT, because the list is only as new as the request that left for it: the summary
    // that counts a folder's queue is refreshed by every poll, and this list by a read that lands later (and
    // not at all while the count is 0). Dated, a reader can tell a list from before the count changed.
    slice.deletionsIssue = issue;
  }

  // The roster. A reply from a daemon that predates the selector carries none, and that IS the
  // answer ("no pairs"); a payload with no reply at all (the socket failed) knows nothing new and
  // keeps what was last known — but it is evidence the daemon is not answering NOW, so it is dated
  // (`pairsFailedIssue`) and `select.rosterLive` stops vouching for what was kept.
  //
  // A ROSTER IS EVIDENCE ABOUT THE MOMENT ITS REQUEST LEFT. One from a request older than the one that
  // produced the roster already held is older data, and replacing it re-drew the folders as they were
  // (a pill that had rung stopped ringing, then rang again at the next poll). The comparison is `>=`,
  // as for the selection above: a request that is also the newest to say is not older than itself.
  const pairs = payload?.pairs ?? payload?.response?.pairs;
  if (issue >= state.pairsIssue) {
    if (Array.isArray(pairs)) {
      state.pairs = pairs;
      state.pairStates = Array.isArray(payload?.pair_states) ? payload.pair_states : [];
      state.pairsIssue = issue;
      noteRoster(pairs, issue);
    } else if (payload?.response) {
      state.pairs = [];
      state.pairStates = [];
      state.pairsIssue = issue;
      noteRoster([], issue);
    }
  }
  // A failure that NAMED a folder is that folder's, not the daemon's: the poll's own read decides whether the
  // roster is live, and a failed side read does not date it.
  if (!payload?.response && !Array.isArray(pairs) && !asked) {
    state.pairsFailedIssue = Math.max(state.pairsFailedIssue, issue);
  }

  // THE SELECTION MOVES ONLY ON A REPLY THAT DESCRIBES THE PAIR IT SELECTS. Rust stamps `selected` when
  // it BUILDS the payload, so a read that left for pair A before `select_pair(B)` and landed after it
  // describes A and says B: taking that `selected` moved the window to a pair with no status of its own
  // yet, and the hero drew "unreachable" until B's first reply arrived. Not moving leaves the window on
  // the pair it already has a status for (A, filed just above), and the first reply that is ABOUT B —
  // the next poll — carries it there with that status in hand. The same rule keeps a read that NAMES
  // another pair (`get_status(pair)`) from relocating the window, which its reply would otherwise do.
  const next = state.follows === "selection" ? (payload?.selected ?? name) : name;
  if (next === name && issue >= state.selectedIssue) {
    state.selected = next;
    state.selectedIssue = issue;
    // WHAT THE SELECTION READ SAID ABOUT THE FOLDER THAT WAS ASKED FOR. Only the reply that moves the
    // selection speaks for it: a read that NAMES another folder never carries the note, and letting it
    // set (or clear) this would flip the window's notice with every such read. A reply from a daemon
    // that predates the field has none, and that is the answer.
    state.pairUnknown = payload?.response ? (payload.pair_unknown ?? null) : state.pairUnknown;
  }
  emit();
}

/** Tag each item with the pair it belongs to — what lets a handler act on ITS pair, not the selected one. */
const tagged = (list, pair) => (Array.isArray(list) ? list : []).map((item) => ({ ...item, pair }));

/**
 * Record what a roster just said about each folder, so a list fetched for one can be dated against it.
 *
 * A folder's pending-deletion count is the only thing the roster says about its queue, and the list behind it
 * is a separate read. `countIssue` is the request that first reported the count the folder has NOW: it moves
 * when the count changes (including to and from 0) and stays while the same count is reported again, so a list
 * fetched after it is a list the count can be believed to explain. `joined` is the request that first listed
 * the folder in this stretch of membership: a folder removed and added again under the same name is another
 * folder, and whatever was held for the old one is not about it. Rebuilt from the roster each time, so a
 * folder it no longer lists is forgotten.
 */
function noteRoster(pairs, issue) {
  const facts = pairTable();
  for (const entry of pairs) {
    const name = entry?.name;
    if (typeof name !== "string") continue;
    const count = Number(entry.pending_deletions) > 0 ? Number(entry.pending_deletions) : 0;
    const before = state.rosterFacts.get(name);
    facts.set(name, {
      joined: before ? before.joined : issue,
      count,
      countIssue: before && before.count === count ? before.countIssue : issue,
    });
  }
  state.rosterFacts = facts;
}

/**
 * File a conflict scan under the pair it was asked for. `pair` is the pair the scan was ISSUED for,
 * captured by the caller before the request left; defaulting to the selected pair is only right for
 * a caller that did not wait.
 *
 * `issue` is the status clock (`select.statusesIssued`) read BEFORE the scan left, and it is what makes this
 * a scan the notifier may speak from: without it the list is held (the screens read it) but is not DATED, and
 * `select.conflictsFreshOf` answers no. Deliberately not defaulted to the clock at call time — for the same
 * reason `setStatus` does not allocate its own id: a set-time stamp would call a scan that left before some
 * event a scan that left after it. A caller that edits a list in place (a decision just taken) passes none,
 * and the list keeps the date of the scan it was edited from.
 */
export function setConflicts(list, pair = selectedName(), issue = null) {
  const slice = sliceFor(pair);
  slice.conflicts = tagged(list, pair);
  if (issue != null) slice.conflictsIssue = issue;
  emit();
}
export function setLedgerFilter(filter) {
  state.ledgerFilter = filter;
  emit();
}
export function stageResolution(path, choice, pair = selectedName()) {
  sliceFor(pair).staged[path] = choice;
  emit();
}

export const select = {
  /** The pair every other selector answers for. */
  pairName: () => selectedName(),
  /**
   * The pair a status has actually said is selected, or `null` before any has. `pairName` falls back to
   * `DEFAULT_PAIR` so a caller always has a name; this is the one that can tell "the default pair" from
   * "nothing has been heard yet", which is what noticing a SWITCH needs (the first answer is not one).
   */
  settledPair: () => state.selected,
  /** Every pair the daemon runs, as its last reply listed them. Empty for a daemon that predates the selector. */
  pairs: () => state.pairs,
  /** The derived state of each pair (`{ name, state }[]`). */
  pairStates: () => state.pairStates,
  /** The status request whose reply the roster above came from (see `state.pairsIssue`). */
  pairsIssue: () => state.pairsIssue,
  /**
   * Is the roster above what the daemon said LAST, or only what it said before it stopped answering?
   *
   * The store keeps the roster across a failed read, which is right for NAMING the folders and wrong for
   * saying what any of them is doing: every surface that draws a folder's state from it used to draw the
   * state of a daemon that was no longer there (`up to date`, `Sync now`, a pause row per folder — #246).
   * This is the one answer, and a surface that draws a state asks it rather than comparing clocks itself.
   * `unreachable` is also a state a reply can carry (`derive_state` gives it to exactly "the socket did not
   * answer"), so a payload that lists folders AND says so is not live either.
   */
  rosterLive: () => state.pairsIssue > state.pairsFailedIssue && select.daemonState() !== "unreachable",
  /** The roster, or none when it is not live — what a surface that DRAWS a state reads (see `rosterLive`). */
  livePairs: () => (select.rosterLive() ? state.pairs : []),
  livePairStates: () => (select.rosterLive() ? state.pairStates : []),
  /** The remembered folder the daemon does not run, or null (see `state.pairUnknown`). */
  pairUnknown: () => state.pairUnknown,
  /**
   * What the store holds for a folder OTHER than the selected one — the folder selector's marker and
   * counts. A folder never heard of answers empty, and `deletionsFiledOf` says whether "empty" is a
   * fact or only the absence of a reply.
   */
  conflictsOf: (pair) => state.byPair.get(pair)?.conflicts ?? [],
  pendingDeletionsOf: (pair) => state.byPair.get(pair)?.pendingDeletions ?? [],
  deletionsFiledOf: (pair) => state.byPair.get(pair)?.deletionsFiled ?? false,
  /**
   * May the list held for a folder that is NOT on screen be believed about its queue NOW? Only if it was
   * fetched after the roster first reported the count the folder has today. `deletionsFiledOf` says a list
   * was ever filled; this says it is not older than what it is meant to explain — a queue drained by
   * `Keep them` is not refetched at 0, so the list held afterwards still names what was kept, and the next
   * deletion would otherwise be read as that old list. A folder whose count is 0 has no list to believe: its
   * queue is empty, which the roster says.
   */
  deletionsFreshOf: (pair) => {
    const slice = state.byPair.get(pair);
    const fact = state.rosterFacts.get(pair);
    return Boolean(slice && fact && slice.deletionsIssue >= fact.countIssue);
  },
  /**
   * May the conflict scan held for a folder that is NOT on screen be believed? Only if a dated scan has landed
   * (`[]` alone cannot say "nothing found" from "not looked yet" — an undated list is not known) and it left
   * after the folder joined the roster — a folder removed and added again under the same name is another folder.
   */
  conflictsFreshOf: (pair) => {
    const slice = state.byPair.get(pair);
    const fact = state.rosterFacts.get(pair);
    return Boolean(slice && fact && slice.conflictsIssue >= fact.joined);
  },

  daemonState: () => viewed().status?.state ?? "unreachable",
  /** Which request the state above came home from, and the highest one issued (#335). */
  statusIssue: () => viewed().statusIssue ?? 0,
  statusesIssued: () => statusesIssued,
  response: () => viewed().status?.response ?? null,
  error: () => viewed().status?.error ?? null,
  // "unknown must never render as zero" — em-dash counters in these states.
  countersUnknown: () => ["unreachable", "firstRun"].includes(select.daemonState()),

  pendingChanges: () => (select.countersUnknown() ? null : (select.response()?.pending_changes ?? null)),
  planSummary: () => select.response()?.last_plan_summary ?? null,

  // THE single unresolved-conflict count feeding sidebar badge, tab header, needs-you, stat tile,
  // and ledger chip. Derived from the scanned sidecar set on disk.
  unresolvedConflictCount: () => viewed().conflicts.length,

  // Pending writes staged on the Conflicts screen (footer counter); "decide_later" is not a write.
  stagedWriteCount: () => Object.values(viewed().staged).filter((c) => c && c !== "decide_later").length,

  statCounters: () => {
    const unknown = select.countersUnknown();
    const summary = select.planSummary();
    return {
      pending_changes: unknown ? null : (select.response()?.pending_changes ?? null),
      conflicts: unknown ? null : select.unresolvedConflictCount(),
      destructive_actions: unknown ? null : summary ? summary.destructive_actions : null,
      skipped_unsupported: unknown ? null : summary ? summary.skipped_unsupported : null,
    };
  },

  conflicts: () => viewed().conflicts,
  pendingDeletions: () => viewed().pendingDeletions,
  ledgerFilter: () => state.ledgerFilter,
  raw: () => ({
    ...viewed(),
    selected: state.selected,
    pairs: state.pairs,
    ledgerFilter: state.ledgerFilter,
  }),
};
