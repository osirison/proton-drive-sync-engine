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

/// How a query matched, best first. The ordering IS the ranking — `Ord` derives from the variant
/// order, so an exact path always outranks a name, and a name always outranks "contains".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchRank {
    /// The whole relative path, byte for byte.
    ExactPath,
    /// The whole relative path, ignoring case.
    ExactPathFolded,
    /// The file's own name (`spec.md` for `docs/spec.md`), ignoring case.
    ExactName,
    /// A trailing run of whole components (`docs/spec.md` for `notes/docs/spec.md`).
    PathSuffix,
    /// The name contains the query.
    NameContains,
    /// Anything else in the path does.
    PathContains,
}

/// One matched record, and how it matched.
pub struct IndexMatch {
    pub path: PathBuf,
    pub record: FileRecord,
    rank: MatchRank,
}

/// Rank a stored path against a folded query, or `None` when it does not match at all.
fn rank_of(path: &Path, query: &str, folded_query: &str) -> Option<MatchRank> {
    let text = path.to_string_lossy();
    if text == query {
        return Some(MatchRank::ExactPath);
    }
    let folded = text.to_lowercase();
    if folded == folded_query {
        return Some(MatchRank::ExactPathFolded);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name == folded_query {
        return Some(MatchRank::ExactName);
    }
    // A COMPONENT BOUNDARY, not a plain `ends_with`: `docs/spec.md` must not match
    // `mydocs/spec.md`, whose trailing characters are identical and whose folder is not.
    if let Some(head) = folded.strip_suffix(folded_query)
        && head.ends_with('/')
    {
        return Some(MatchRank::PathSuffix);
    }
    if name.contains(folded_query) {
        return Some(MatchRank::NameContains);
    }
    if folded.contains(folded_query) {
        return Some(MatchRank::PathContains);
    }
    None
}

/// Every record whose path matches `query`, best match first, capped at `limit`.
///
/// A FULL TABLE SCAN, deliberately: the index has one key (the byte-exact path) and no name column,
/// so "everything called spec.md" cannot be a point query. That is why the caller must be off the
/// UI thread — see `commands::search_files`.
///
/// Returns the capped list and the TOTAL number of matches, so a caller can say what it is not
/// showing. Empty and whitespace-only queries match nothing rather than everything.
pub fn search_records(
    connection: &Connection,
    query: &str,
    limit: usize,
) -> Result<(Vec<IndexMatch>, usize), String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let folded_query = query.to_lowercase();
    let index =
        proton_drive_sync_engine::index::load_index(connection).map_err(|e| e.to_string())?;

    let mut matches: Vec<IndexMatch> = index
        .into_iter()
        .filter_map(|(path, record)| {
            rank_of(&path, query, &folded_query).map(|rank| IndexMatch { path, record, rank })
        })
        .collect();
    let total = matches.len();
    // Rank, then the shortest path, then alphabetical — so the answer is stable across runs
    // (`load_index` returns a HashMap, whose order is not).
    matches.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
            .then_with(|| a.path.cmp(&b.path))
    });
    matches.truncate(limit);
    Ok((matches, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proton_drive_sync_engine::index::{EntityKind, SyncStatus};

    #[test]
    fn opening_a_missing_index_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("sync_index.db");
        assert!(open_readonly(&missing, DEFAULT_BUSY_TIMEOUT).is_err());
    }

    fn record(path: &str) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::File,
            file_size: 10,
            mtime: 0,
            sha1_hash: Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string()),
            proton_id: None,
            sync_status: SyncStatus::Synced,
        }
    }

    /// An index holding these paths, opened read-only the way the commands open it.
    fn index_of(paths: &[&str]) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sync_index.db");
        let writer = proton_drive_sync_engine::index::open_database(&db).unwrap();
        proton_drive_sync_engine::index::initialize_schema(&writer).unwrap();
        for path in paths {
            proton_drive_sync_engine::index::upsert_record(&writer, &record(path)).unwrap();
        }
        drop(writer);
        let connection = open_readonly(&db, DEFAULT_BUSY_TIMEOUT).unwrap();
        (dir, connection)
    }

    fn found(connection: &Connection, query: &str) -> Vec<String> {
        search_records(connection, query, 50)
            .unwrap()
            .0
            .into_iter()
            .map(|m| m.path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_bare_name_finds_the_file_wherever_it_is() {
        // The gap this closes: `path_sync_status("spec.md")` answers "not tracked" for this index.
        let (_dir, connection) = index_of(&["docs/spec.md", "notes/other.md"]);
        assert_eq!(found(&connection, "spec.md"), vec!["docs/spec.md"]);
    }

    #[test]
    fn the_exact_path_outranks_a_name_that_merely_contains_it() {
        let (_dir, connection) = index_of(&["docs/spec.md", "archive/old-spec.md", "spec.md"]);
        assert_eq!(
            found(&connection, "spec.md"),
            vec!["spec.md", "docs/spec.md", "archive/old-spec.md"]
        );
    }

    #[test]
    fn a_trailing_path_matches_only_on_a_component_boundary() {
        let (_dir, connection) = index_of(&["notes/docs/spec.md", "mydocs/spec.md"]);
        let ranked = search_records(&connection, "docs/spec.md", 50).unwrap().0;
        assert_eq!(ranked[0].path, PathBuf::from("notes/docs/spec.md"));
        assert_eq!(ranked[0].rank, MatchRank::PathSuffix);
        // `mydocs/spec.md` ends with the same characters and is a different folder, so it is only
        // ever the weakest kind of match.
        assert_eq!(ranked[1].rank, MatchRank::PathContains);
    }

    #[test]
    fn case_does_not_matter_and_the_exact_case_still_wins() {
        let (_dir, connection) = index_of(&["Docs/Spec.md", "docs/spec.md"]);
        assert_eq!(found(&connection, "docs/spec.md")[0], "docs/spec.md");
        assert_eq!(found(&connection, "SPEC.MD").len(), 2);
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        let (_dir, connection) = index_of(&["docs/spec.md"]);
        assert!(found(&connection, "   ").is_empty());
        assert_eq!(search_records(&connection, "", 50).unwrap().1, 0);
    }

    #[test]
    fn the_cap_limits_the_list_and_not_the_count() {
        let (_dir, connection) = index_of(&["a/note.md", "b/note.md", "c/note.md"]);
        let (matches, total) = search_records(&connection, "note", 2).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(total, 3);
    }

    #[test]
    fn the_order_is_stable_across_runs() {
        // `load_index` hands back a HashMap, so an unsorted answer would shuffle between runs and
        // the screen would draw a different "first match" each time.
        let (_dir, connection) = index_of(&["b/note.md", "a/note.md", "c/note.md", "note.md"]);
        let first = found(&connection, "note");
        for _ in 0..5 {
            assert_eq!(found(&connection, "note"), first);
        }
        assert_eq!(first[0], "note.md");
    }
}
