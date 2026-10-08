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

const listeners = new Set();

/** The pair a legacy reply, a fixture, and the time before the first reply all belong to. */
export const DEFAULT_PAIR = "default";

const emptySlice = () => ({
  status: null, // last get_status payload for THIS pair: { state, response, error }
  statusIssue: 0, // which status REQUEST produced `status` — see `beginStatus`
  conflicts: [], // last scan_conflicts result for this pair (the unresolved set), each tagged `pair`
  pendingDeletions: [], // this pair's withheld deletions (S9), each tagged `pair`
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
  /** The derived state of each, by name (`{ name, state }[]`), computed in Rust. */
  pairStates: [],
  byPair: {},
  ledgerFilter: "all",
};

const sliceFor = (name) => (state.byPair[name] ??= emptySlice());
const selectedName = () => state.selected ?? DEFAULT_PAIR;
const viewed = () => state.byPair[selectedName()] ?? EMPTY;
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
export function pairOf(payload) {
  const described = payload?.response?.pair;
  if (described) return described;
  if (state.follows === "selection" && payload?.selected) return payload.selected;
  return selectedName();
}

/**
 * Publish a status answer. `issue` is the id [`beginStatus`] returned for the request that produced
 * it — **required**, and deliberately not defaulted to "allocate one now": a set-time id would
 * stamp a request issued *before* some event as though it had been issued after, which is the one
 * direction that is unsafe.
 *
 * The reply is filed under the pair it describes. The selected pair moves only if this reply is the
 * newest to say so (`issue` ≥ the last that did): replies can arrive out of order, and an old one
 * reporting the previous selection must not win it back.
 */
export function setStatus(payload, issue) {
  const name = pairOf(payload);
  const slice = sliceFor(name);
  slice.status = payload;
  slice.statusIssue = issue;
  // The withheld deletions RIDE ON THE REPLY, so they are filed with it, in the same publish. Filed in
  // a second call after this one (as the poll used to), the pair that has just become selected showed
  // an EMPTY queue for one render — `Nothing waiting to be deleted` — before its own arrived, which
  // is a flash at start-up and, across a switch, the whole screen redrawn from nothing. A payload with
  // no reply (the socket failed) says nothing new about the queue, and leaves what was last seen.
  if (payload?.response) slice.pendingDeletions = tagged(payload.response.pending_deletions, name);

  // The roster. A reply from a daemon that predates the selector carries none, and that IS the
  // answer ("no pairs"); a payload with no reply at all (the socket failed) knows nothing new and
  // keeps what was last known.
  const pairs = payload?.pairs ?? payload?.response?.pairs;
  if (Array.isArray(pairs)) {
    state.pairs = pairs;
    state.pairStates = Array.isArray(payload?.pair_states) ? payload.pair_states : [];
  } else if (payload?.response) {
    state.pairs = [];
    state.pairStates = [];
  }

  const next = state.follows === "selection" ? (payload?.selected ?? name) : name;
  if (issue >= state.selectedIssue) {
    state.selected = next;
    state.selectedIssue = issue;
  }
  emit();
}

/** Tag each item with the pair it belongs to — what lets a handler act on ITS pair, not the selected one. */
const tagged = (list, pair) => (Array.isArray(list) ? list : []).map((item) => ({ ...item, pair }));

/**
 * File a conflict scan under the pair it was asked for. `pair` is the pair the scan was ISSUED for,
 * captured by the caller before the request left; defaulting to the selected pair is only right for
 * a caller that did not wait.
 */
export function setConflicts(list, pair = selectedName()) {
  sliceFor(pair).conflicts = tagged(list, pair);
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
