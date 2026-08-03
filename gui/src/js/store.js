// Client state + derived selectors (F3). The single source of truth for the whole window, so the
// conflict count (which appears in five places), chip counts, and stat values are computed once.

const listeners = new Set();

const state = {
  status: null, // last get_status payload: { state, response, error }
  conflicts: [], // last scan_conflicts result (the unresolved set)
  pendingDeletions: [], // withheld deletions (S9)
  staged: {}, // path -> Resolution, for the Conflicts screen (staged, not yet applied)
  ledgerFilter: "all",
};

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
function emit() {
  for (const fn of listeners) fn(state);
}

export function setStatus(payload) {
  state.status = payload;
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
