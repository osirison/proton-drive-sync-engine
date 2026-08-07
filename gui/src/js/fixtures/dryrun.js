// The dry-run payload's own shape (F9) — shared by `plan.js` (`5a`) and `onboarding.js` (`9a Review`),
// the two families whose frames are driven by `run_dry_run`.
//
// WHY A SUMMARY IS DERIVED AND NEVER WRITTEN. `DryRunReport` is `{ summary, plan }` parsed verbatim
// from the daemon's stdout (gui-core/src/plan.rs), and the daemon builds the summary from the plan:
// `PlanSummary::from_plan` sets `total: plan.len()` and increments exactly one counter per action
// (src/sync.rs). So the two halves cannot disagree in anything the daemon could emit — `total` is
// `plan.length`, and the per-action counters sum to it.
//
// Hand-writing the summary beside the plan breaks that quietly, and it already did: `9a Review`
// shipped `total: 471` against a sum of 474 and a `plan: []`, from a comment that added three of the
// four contributing counters. A fixture that cannot be produced by the thing it imitates is worse
// than a missing one, because the screen written against it will look right.
//
// So: build the plan, call `summaryOf` on it, and the invariant holds by construction.

/**
 * A planned action, with the two fields that are almost always null spelled out.
 *
 * `remote_id` is the asymmetry `plan.rs`'s own test pins: a row for something that does not exist on
 * Proton yet (an upload, a folder to create) has none, and a row about an existing node carries the
 * composed `volumeId~nodeId`. The design doc's "every row has a remote id" is false, and a fixture
 * that filled them all in would hide that from S4.
 */
export function action(path, act, extra = {}) {
  return {
    path,
    destination_path: null,
    action: act,
    entity_kind: "file",
    conflict_path: null,
    remote_id: null,
    ...extra,
  };
}

/** `SyncAction` → the `PlanSummary` counter it increments, exactly as `from_plan` matches. */
const COUNTER = {
  upload: "uploads",
  download: "downloads",
  create_remote_directory: "remote_directories_created",
  create_local_directory: "local_directories_created",
  move_local: "local_moves",
  move_remote: "remote_moves",
  auto_link: "auto_links",
  conflict: "conflicts",
  type_conflict: "type_conflicts",
  remote_delete: "remote_deletes",
  local_delete: "local_deletes",
  purge: "purges",
  skip_unsupported: "skipped_unsupported",
};

/**
 * `PlanSummary::from_plan`, in JavaScript. Every counter is written out because the daemon writes
 * every one, and `destructive_actions` is derived last from the three it sums — the DISPLAY set
 * (`purge` included), which is deliberately not the gated set. `plan.rs` encodes that distinction
 * and the design conflated it; `requires_delete_gate` keys on the other one.
 *
 * An unknown action name throws rather than being ignored. A silently-uncounted row would produce
 * a summary whose parts do not sum to its total — the exact defect this module exists to prevent —
 * and the daemon's enum is closed, so anything not in `COUNTER` is a typo in a fixture.
 */
export function summaryOf(plan) {
  const summary = {
    total: plan.length,
    uploads: 0,
    downloads: 0,
    remote_directories_created: 0,
    local_directories_created: 0,
    local_moves: 0,
    remote_moves: 0,
    auto_links: 0,
    conflicts: 0,
    type_conflicts: 0,
    remote_deletes: 0,
    local_deletes: 0,
    purges: 0,
    skipped_unsupported: 0,
    destructive_actions: 0,
  };
  for (const row of plan) {
    const counter = COUNTER[row.action];
    if (!counter) throw new Error(`no PlanSummary counter for action "${row.action}"`);
    summary[counter] += 1;
  }
  summary.destructive_actions = summary.remote_deletes + summary.local_deletes + summary.purges;
  return summary;
}

/**
 * `n` rows of one action, named so a reader can tell generated bulk from a row that matters.
 *
 * `9a Review` is the only frame that needs this: a first-sync merge is 474 rows, and the screen draws
 * only their counts. Writing 474 literals would bury the four that carry meaning; writing `plan: []`
 * beside a summary claiming 474 is the payload the daemon cannot emit. Generating them keeps both
 * halves true, and the paths are deterministic because a fixture may not vary between runs.
 */
export function bulk(prefix, act, n, extra = {}) {
  return Array.from({ length: n }, (_, i) => action(`${prefix}/${String(i).padStart(4, "0")}`, act, extra));
}
