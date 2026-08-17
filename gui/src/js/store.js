// Client state + derived selectors (F3). The single source of truth for the whole window, so the
// conflict count (which appears in five places), chip counts, and stat values are computed once.

const listeners = new Set();

const state = {
  status: null, // last get_status payload: { state, response, error }
  statusIssue: 0, // which status REQUEST produced `status` — see `beginStatus`
  conflicts: [], // last scan_conflicts result (the unresolved set)
  pendingDeletions: [], // withheld deletions (S9)
  staged: {}, // path -> Resolution, for the Conflicts screen (staged, not yet applied)
  ledgerFilter: "all",
};

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
 * Publish a status answer. `issue` is the id [`beginStatus`] returned for the request that produced
 * it — **required**, and deliberately not defaulted to "allocate one now": a set-time id would
 * stamp a request issued *before* some event as though it had been issued after, which is the one
 * direction that is unsafe.
 */
export function setStatus(payload, issue) {
  state.status = payload;
  state.statusIssue = issue;
  emit();
}
export function setConflicts(list) {
  state.conflicts = Array.isArray(list) ? list : [];
  emit();
}
export function setPendingDeletions(list) {
  state.pendingDeletions = Array.isArray(list) ? list : [];
  emit();
}
export function setLedgerFilter(filter) {
  state.ledgerFilter = filter;
  emit();
}
export function stageResolution(path, choice) {
  state.staged[path] = choice;
  emit();
}

export const select = {
  daemonState: () => state.status?.state ?? "unreachable",
  /** Which request the state above came home from, and the highest one issued (#335). */
  statusIssue: () => state.statusIssue ?? 0,
  statusesIssued: () => statusesIssued,
  response: () => state.status?.response ?? null,
  error: () => state.status?.error ?? null,
  // "unknown must never render as zero" — em-dash counters in these states.
  countersUnknown: () => ["unreachable", "firstRun"].includes(select.daemonState()),

  pendingChanges: () => (select.countersUnknown() ? null : (select.response()?.pending_changes ?? null)),
  planSummary: () => select.response()?.last_plan_summary ?? null,

  // THE single unresolved-conflict count feeding sidebar badge, tab header, needs-you, stat tile,
  // and ledger chip. Derived from the scanned sidecar set on disk.
  unresolvedConflictCount: () => state.conflicts.length,

  // Pending writes staged on the Conflicts screen (footer counter); "decide_later" is not a write.
  stagedWriteCount: () => Object.values(state.staged).filter((c) => c && c !== "decide_later").length,

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

  conflicts: () => state.conflicts,
  pendingDeletions: () => state.pendingDeletions,
  ledgerFilter: () => state.ledgerFilter,
  raw: () => state,
};
