//! Dry-run plan parsing and the destructive-action classification the Plan-preview screen needs.
//!
//! The daemon's `--dry-run` writes a machine-readable `{ "summary": PlanSummary, "plan": [rows] }`
//! object to stdout. This module parses it (reusing [`DryRunReport`]) and encodes the one safety
//! distinction the design conflated: **the typed-DELETE gate is not the same set as the tinted
//! rows.**
//!
//! - *Display-destructive* (tinted red, sorted first) = `remote_delete | local_delete | purge` —
//!   matches `summary.destructive_actions`.
//! - *Gated* (requires typing `DELETE`) = [`SyncAction::delete_direction`]`().is_some()` =
//!   `remote_delete | local_delete` only. `purge` is index-only cleanup that destroys no user
//!   data and must **never** force the confirmation.

use crate::wire::{DryRunReport, PlannedAction, SyncAction};
use std::path::PathBuf;

/// Parse the daemon's dry-run stdout into a [`DryRunReport`].
pub fn parse_dry_run(json: &str) -> Result<DryRunReport, String> {
    serde_json::from_str(json).map_err(|e| format!("parse dry-run plan: {e}"))
}

/// Whether an action is shown as destructive (tinted + sorted first).
///
/// **Delegated, not re-enumerated.** This used to spell out `RemoteDelete | LocalDelete | Purge`,
/// which was a second copy of the engine's own definition — and since #192 that set is no longer
/// display-only: `apply --skip-destructive` drops exactly the rows
/// [`SyncAction::is_destructive`] names, so a screen tinting a different set would show one thing
/// and run another.
pub fn is_display_destructive(action: SyncAction) -> bool {
    action.is_destructive()
}

/// Whether the plan must be gated behind the typed-`DELETE` confirmation. Keys on
/// [`SyncAction::delete_direction`], so it is `true` only when the plan will delete real user data
/// (`remote_delete` / `local_delete`) and **never** for a `purge`-only plan.
pub fn requires_delete_gate(report: &DryRunReport) -> bool {
    report
        .plan
        .iter()
        .any(|action| action.action.delete_direction().is_some())
}

/// The user-data files a destructive apply would remove, so the gate copy can name them. Only
/// gated actions contribute; `purge` (no data loss) does not appear here.
pub fn files_at_risk(report: &DryRunReport) -> Vec<PathBuf> {
    report
        .plan
        .iter()
        .filter(|action| action.action.delete_direction().is_some())
        .map(|action| action.path.clone())
        .collect()
}

/// The plan rows ordered for display: display-destructive rows first, otherwise stable in the
/// daemon's original order.
pub fn sorted_for_display(report: &DryRunReport) -> Vec<&PlannedAction> {
    let mut rows: Vec<&PlannedAction> = report.plan.iter().collect();
    // `false` sorts before `true`, so map "is destructive" → `false` to float it to the top.
    rows.sort_by_key(|action| !is_display_destructive(action.action));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mixed plan: an upload, a remote delete (gated), and a purge (display-destructive but NOT
    // gated). Serialized exactly as the daemon emits it.
    const MIXED_PLAN_JSON: &str = r#"{
      "summary": {
        "total": 3, "uploads": 1, "downloads": 0,
        "remote_directories_created": 0, "local_directories_created": 0,
        "local_moves": 0, "remote_moves": 0, "auto_links": 0,
        "conflicts": 0, "type_conflicts": 0,
        "remote_deletes": 1, "local_deletes": 0, "purges": 1,
        "skipped_unsupported": 0, "destructive_actions": 2
      },
      "plan": [
        {"path":"a.txt","destination_path":null,"action":"upload","entity_kind":"file","conflict_path":null,"remote_id":null},
        {"path":"gone.txt","destination_path":null,"action":"remote_delete","entity_kind":"file","conflict_path":null,"remote_id":"vol~node"},
        {"path":"stale-record","destination_path":null,"action":"purge","entity_kind":"file","conflict_path":null,"remote_id":null}
      ]
    }"#;

    // A plan whose only destructive row is a purge — must NOT trip the DELETE gate.
    const PURGE_ONLY_JSON: &str = r#"{
      "summary": {
        "total": 1, "uploads": 0, "downloads": 0,
        "remote_directories_created": 0, "local_directories_created": 0,
        "local_moves": 0, "remote_moves": 0, "auto_links": 0,
        "conflicts": 0, "type_conflicts": 0,
        "remote_deletes": 0, "local_deletes": 0, "purges": 1,
        "skipped_unsupported": 0, "destructive_actions": 1
      },
      "plan": [
        {"path":"stale-record","destination_path":null,"action":"purge","entity_kind":"file","conflict_path":null,"remote_id":null}
      ]
    }"#;

    #[test]
    fn parses_summary_and_rows() {
        let report = parse_dry_run(MIXED_PLAN_JSON).unwrap();
        assert_eq!(report.summary.total, 3);
        assert_eq!(report.plan.len(), 3);
        assert_eq!(report.plan[1].action, SyncAction::RemoteDelete);
        assert_eq!(report.plan[1].remote_id.as_deref(), Some("vol~node"));
        // remote_id is null for a new upload — the design's "every row has a remote id" is false.
        assert_eq!(report.plan[0].remote_id, None);
    }

    #[test]
    fn gate_keys_on_delete_direction_not_the_destructive_count() {
        let mixed = parse_dry_run(MIXED_PLAN_JSON).unwrap();
        assert!(requires_delete_gate(&mixed), "remote_delete must gate");

        let purge_only = parse_dry_run(PURGE_ONLY_JSON).unwrap();
        assert_eq!(purge_only.summary.destructive_actions, 1);
        assert!(
            !requires_delete_gate(&purge_only),
            "a purge-only plan must NOT force the typed-DELETE gate"
        );
    }

    #[test]
    fn files_at_risk_names_only_gated_deletions() {
        let report = parse_dry_run(MIXED_PLAN_JSON).unwrap();
        let at_risk = files_at_risk(&report);
        assert_eq!(at_risk, vec![PathBuf::from("gone.txt")]);
        // the purge path ("stale-record") is not user data at risk
        assert!(!at_risk.contains(&PathBuf::from("stale-record")));
    }

    #[test]
    fn display_sort_floats_destructive_rows_first() {
        let report = parse_dry_run(MIXED_PLAN_JSON).unwrap();
        let ordered = sorted_for_display(&report);
        // remote_delete and purge (both display-destructive) come before the upload.
        assert!(is_display_destructive(ordered[0].action));
        assert!(is_display_destructive(ordered[1].action));
        assert_eq!(ordered[2].action, SyncAction::Upload);
    }
}
