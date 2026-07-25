//! Readers for the daemon's on-disk JSON state sidecars.
//!
//! The daemon writes two machine-readable JSON files next to the SQLite index (derived from the
//! DB path via `with_extension`): `<db>.metrics.json` (a full [`MetricsSnapshot`] at startup and
//! after each sync) and `<db>.status.json` (the last-20 [`StatusHistoryEntry`] list, the activity
//! ledger / history source). These are the GUI's preferred disk source for live state because the
//! index DB runs **without WAL** — a reader polling it races the daemon's write transactions and
//! hits `SQLITE_BUSY`. The daemon writes these JSON files atomically (temp file + rename), so a
//! polling reader never observes a partially written sidecar.

use crate::wire::{MetricsSnapshot, StatusHistoryEntry};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// Path of the metrics sidecar for a given index DB path (`sync_index.db` → `sync_index.metrics.json`).
pub fn metrics_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("metrics.json")
}

/// Path of the status-history sidecar (`sync_index.db` → `sync_index.status.json`).
pub fn status_history_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("status.json")
}

/// Why reading a sidecar failed. `NotFound` is distinct so the UI can treat "the daemon has not
/// written one yet" as first-run rather than as an error.
#[derive(Debug)]
pub enum SidecarError {
    NotFound(PathBuf),
    Io(String),
    Decode(String),
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SidecarError::NotFound(p) => write!(f, "sidecar not found: {}", p.display()),
            SidecarError::Io(m) => write!(f, "sidecar read error: {m}"),
            SidecarError::Decode(m) => write!(f, "sidecar decode error: {m}"),
        }
    }
}

impl std::error::Error for SidecarError {}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, SidecarError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SidecarError::NotFound(path.to_path_buf()));
        }
        Err(e) => return Err(SidecarError::Io(e.to_string())),
    };
    serde_json::from_str(&text).map_err(|e| SidecarError::Decode(e.to_string()))
}

/// Read the full live-state snapshot from `<db>.metrics.json`.
pub fn read_metrics(db_path: &Path) -> Result<MetricsSnapshot, SidecarError> {
    read_json(&metrics_path(db_path))
}

/// Read the activity/history entries from `<db>.status.json` (up to the daemon's 20-entry limit).
pub fn read_status_history(db_path: &Path) -> Result<Vec<StatusHistoryEntry>, SidecarError> {
    read_json(&status_history_path(db_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_paths_derive_from_the_db_stem() {
        let db = Path::new("/home/u/ProtonDrive/.sync/sync_index.db");
        assert_eq!(
            metrics_path(db),
            Path::new("/home/u/ProtonDrive/.sync/sync_index.metrics.json")
        );
        assert_eq!(
            status_history_path(db),
            Path::new("/home/u/ProtonDrive/.sync/sync_index.status.json")
        );
    }

    // A realistic metrics.json as the daemon actually serializes it, incl. a populated nested
    // PlanSummary and a pending deletion — this catches on-disk shape surprises a self-serialized
    // round trip cannot (the counters we read for the stat tiles live *inside* the summary).
    const REAL_METRICS_JSON: &str = r#"{
      "generated_epoch_secs": 1750000123,
      "status": "running",
      "paused": false,
      "pending_changes": 4,
      "last_sync_epoch_secs": 1750000000,
      "last_error": null,
      "last_plan_summary": {
        "total": 6, "uploads": 2, "downloads": 1,
        "remote_directories_created": 0, "local_directories_created": 0,
        "local_moves": 0, "remote_moves": 0, "auto_links": 0,
        "conflicts": 1, "type_conflicts": 0,
        "remote_deletes": 1, "local_deletes": 0, "purges": 1,
        "skipped_unsupported": 1, "destructive_actions": 2
      },
      "last_successful_sync_summary": null,
      "status_history_entries": 3,
      "pending_deletions": [
        {"path":"docs/old.txt","direction":"local","entity_kind":"file","fingerprint":"abc","detected_epoch_secs":1750000100}
      ]
    }"#;

    #[test]
    fn reads_a_real_metrics_sidecar_including_the_nested_summary() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sync_index.db");
        std::fs::write(metrics_path(&db), REAL_METRICS_JSON).unwrap();

        let metrics = read_metrics(&db).unwrap();
        assert_eq!(metrics.pending_changes, 4);
        let summary = metrics.last_plan_summary.expect("summary present");
        // The stat tiles read these from *inside* the summary, not the top level.
        assert_eq!(summary.conflicts, 1);
        assert_eq!(summary.destructive_actions, 2);
        assert_eq!(summary.skipped_unsupported, 1);
        assert_eq!(metrics.pending_deletions.len(), 1);
        assert_eq!(metrics.pending_deletions[0].fingerprint, "abc");
    }

    #[test]
    fn missing_sidecar_is_not_found_not_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sync_index.db");
        let err = read_metrics(&db).unwrap_err();
        assert!(matches!(err, SidecarError::NotFound(_)), "got {err:?}");
    }
}
