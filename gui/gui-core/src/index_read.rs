//! Thin, `SQLITE_BUSY`-tolerant reads of the sync index for the file-manager emblems (S10).
//!
//! The index DB runs **without WAL**, so a reader can collide with the daemon's write
//! transactions. We open read-only with a busy timeout and reuse the engine's own queries
//! (`get_record`, `path_for_proton_id`) so key encoding (byte-exact, possibly non-UTF-8) is never
//! reimplemented. Full emblem behaviour (mapping the 3 `sync_status` values + derived states) is
//! built in S10; this is the shared open/query surface.

use crate::wire::FileRecord;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default busy timeout for index reads, giving the daemon's (non-WAL) write transactions time to
/// complete rather than failing immediately with `SQLITE_BUSY`.
pub const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_millis(3000);

/// Open the index read-only with a busy timeout. Read-only so an emblem provider can never mutate
/// the daemon's store.
pub fn open_readonly(db_path: &Path, busy_timeout: Duration) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open index {}: {e}", db_path.display()))?;
    connection
        .busy_timeout(busy_timeout)
        .map_err(|e| e.to_string())?;
    Ok(connection)
}

/// The stored record for a path (relative to the local root), or `None` if the path is untracked.
pub fn record_for_path(
    connection: &Connection,
    relative: &Path,
) -> Result<Option<FileRecord>, String> {
    proton_drive_sync_engine::index::get_record(connection, relative).map_err(|e| e.to_string())
}

/// The local path for a stored remote node id (composed `volumeId~nodeId`), or `None`.
pub fn path_for_id(connection: &Connection, proton_id: &str) -> Result<Option<PathBuf>, String> {
    proton_drive_sync_engine::index::path_for_proton_id(connection, proton_id)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_missing_index_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("sync_index.db");
        assert!(open_readonly(&missing, DEFAULT_BUSY_TIMEOUT).is_err());
    }
}
