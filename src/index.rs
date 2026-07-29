use crate::{AppResult, boxed_error};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::UNIX_EPOCH;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS file_index (
    file_path TEXT PRIMARY KEY,
    entity_kind TEXT NOT NULL DEFAULT 'file',
    file_size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    sha1_hash TEXT,
    proton_id TEXT,
    sync_status TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS remote_event_cursor (
    scope_id TEXT PRIMARY KEY,
    last_event_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS delete_approvals (
    path BLOB NOT NULL,
    direction TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    approved_at INTEGER NOT NULL,
    PRIMARY KEY (path, direction)
);
"#;

/// Speeds up [`path_for_proton_id`] (turning a volume event's node id into its local path).
/// Created *after* [`migrate_file_index_schema`] because that migration rebuilds `file_index`
/// and would otherwise drop an index created alongside the table.
const PROTON_ID_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_file_index_proton_id ON file_index(proton_id);";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    File,
    Directory,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityKind {
    type Err = Box<dyn std::error::Error + Send + Sync>;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "file" => Ok(Self::File),
            "directory" => Ok(Self::Directory),
            other => Err(boxed_error(format!("unknown entity kind: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    Synced,
    Modified,
    Conflict,
}

impl SyncStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::Modified => "modified",
            Self::Conflict => "conflict",
        }
    }
}

impl fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SyncStatus {
    type Err = Box<dyn std::error::Error + Send + Sync>;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "synced" => Ok(Self::Synced),
            "modified" => Ok(Self::Modified),
            "conflict" => Ok(Self::Conflict),
            other => Err(boxed_error(format!("unknown sync status: {other}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub file_path: PathBuf,
    pub entity_kind: EntityKind,
    pub file_size: u64,
    pub mtime: i64,
    pub sha1_hash: Option<String>,
    pub proton_id: Option<String>,
    pub sync_status: SyncStatus,
}

impl FileRecord {
    pub fn from_local(
        relative_path: PathBuf,
        local: &LocalFileState,
        proton_id: Option<String>,
        sync_status: SyncStatus,
    ) -> Self {
        Self {
            file_path: relative_path,
            entity_kind: EntityKind::File,
            file_size: local.file_size,
            mtime: local.mtime,
            sha1_hash: Some(local.sha1_hash.clone()),
            proton_id,
            sync_status,
        }
    }

    pub fn from_local_directory(
        relative_path: PathBuf,
        local: &LocalDirectoryState,
        proton_id: Option<String>,
        sync_status: SyncStatus,
    ) -> Self {
        Self {
            file_path: relative_path,
            entity_kind: EntityKind::Directory,
            file_size: 0,
            mtime: local.mtime,
            sha1_hash: None,
            proton_id,
            sync_status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFileState {
    pub relative_path: PathBuf,
    pub absolute_path: PathBuf,
    pub file_size: u64,
    pub mtime: i64,
    pub sha1_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDirectoryState {
    pub relative_path: PathBuf,
    pub absolute_path: PathBuf,
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEntityState {
    File(LocalFileState),
    Directory(LocalDirectoryState),
}

impl LocalEntityState {
    pub fn relative_path(&self) -> &Path {
        match self {
            Self::File(file) => &file.relative_path,
            Self::Directory(directory) => &directory.relative_path,
        }
    }

    pub fn kind(&self) -> EntityKind {
        match self {
            Self::File(_) => EntityKind::File,
            Self::Directory(_) => EntityKind::Directory,
        }
    }

    pub fn as_file(&self) -> Option<&LocalFileState> {
        match self {
            Self::File(file) => Some(file),
            Self::Directory(_) => None,
        }
    }

    pub fn as_directory(&self) -> Option<&LocalDirectoryState> {
        match self {
            Self::File(_) => None,
            Self::Directory(directory) => Some(directory),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    ignored_relative_paths: Vec<PathBuf>,
    include_patterns: GlobSet,
    include_pattern_strings: Vec<String>,
    has_include_patterns: bool,
    exclude_patterns: GlobSet,
}

impl ScanOptions {
    pub fn new(
        root: &Path,
        ignored_paths: &[PathBuf],
        include_patterns: &[String],
        exclude_patterns: &[String],
    ) -> AppResult<Self> {
        let ignored_relative_paths = ignored_paths
            .iter()
            .flat_map(|path| {
                // SQLite writes transient sidecars next to its database file (`<db>-journal`,
                // `<db>-wal`, `<db>-shm`). When a state file such as the index DB is relocated
                // inside the sync root (issue #73), those siblings must be ignored alongside it
                // or a scan racing a live transaction uploads them as user data. Expanding every
                // ignored path is harmless: the siblings of non-DB entries simply never exist.
                std::iter::once(path.clone()).chain(sqlite_sidecar_paths(path))
            })
            .filter_map(|path| normalize_ignored_path(root, &path))
            .collect();

        Ok(Self {
            ignored_relative_paths,
            include_patterns: build_glob_set(include_patterns)?,
            include_pattern_strings: include_patterns.to_vec(),
            has_include_patterns: !include_patterns.is_empty(),
            exclude_patterns: build_glob_set(exclude_patterns)?,
        })
    }

    pub fn allows_relative_file(&self, relative_path: &Path) -> bool {
        if should_ignore_relative_path(relative_path) || self.is_configured_ignored(relative_path) {
            return false;
        }
        if self.matches(&self.exclude_patterns, relative_path) {
            return false;
        }
        !self.has_include_patterns || self.matches(&self.include_patterns, relative_path)
    }

    /// Whether a directory subtree should be traversed while scanning.
    ///
    /// Traversal only honors ignore/exclude rules so that include patterns
    /// scoped to files deeper in the tree (including unanchored patterns
    /// such as `**/*.md`) are still discovered during the walk.
    fn allows_directory_traversal(&self, relative_path: &Path) -> bool {
        relative_path.as_os_str().is_empty()
            || (!is_download_scratch_path(relative_path)
                && !is_sync_state_path(relative_path)
                && !self.is_configured_ignored(relative_path)
                && !self.matches(&self.exclude_patterns, relative_path))
    }

    /// Whether a directory itself should be planned as a sync entity (for
    /// example, created remotely or locally when empty). This additionally
    /// honors include patterns so directories outside the included scope are
    /// not planned as entities even though ignore/exclude checks alone would
    /// allow traversal through them.
    pub fn allows_relative_directory(&self, relative_path: &Path) -> bool {
        if !self.allows_directory_traversal(relative_path) {
            return false;
        }
        if relative_path.as_os_str().is_empty() || !self.has_include_patterns {
            return true;
        }
        if self.matches(&self.include_patterns, relative_path) {
            return true;
        }
        let prefix = format!("{}/", path_key(relative_path));
        self.include_pattern_strings
            .iter()
            .any(|pattern| pattern.starts_with(&prefix))
    }

    fn is_configured_ignored(&self, relative_path: &Path) -> bool {
        self.ignored_relative_paths
            .iter()
            .any(|ignored| ignored == relative_path)
    }

    fn matches(&self, patterns: &GlobSet, relative_path: &Path) -> bool {
        if patterns.is_match(relative_path) {
            return true;
        }
        let key = path_key(relative_path);
        patterns.is_match(Path::new(&key))
    }
}

pub fn open_database(path: &Path) -> AppResult<Connection> {
    let connection = Connection::open(path)?;
    // The daemon core and its control-socket task each hold a connection to this database
    // (reconcile commits vs. approval writes). Both writes are short; a busy timeout makes the
    // rare collision wait instead of surfacing SQLITE_BUSY.
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    initialize_schema(&connection)?;
    Ok(connection)
}

pub fn load_existing_index(path: &Path) -> AppResult<HashMap<PathBuf, FileRecord>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    load_index(&connection)
}

pub fn initialize_schema(connection: &Connection) -> AppResult<()> {
    connection.execute_batch(SCHEMA)?;
    migrate_file_index_schema(connection)?;
    normalize_legacy_text_keys(connection)?;
    // After any file_index rebuild, so the index survives the migration.
    connection.execute_batch(PROTON_ID_INDEX)?;
    Ok(())
}

/// Normalizes any `file_index.file_path` primary key still stored using SQLite's TEXT
/// storage class to the byte-exact BLOB encoding that [`index_key`] produces and every
/// point query ([`get_record`], [`upsert_record`], [`mark_modified`], [`purge_record`])
/// binds.
///
/// Builds predating the byte-exact key encoding wrote keys as TEXT (via the lossy
/// [`path_key`]). SQLite compares a BLOB-bound parameter against a TEXT-stored value as
/// unequal — BLOB sorts after TEXT, and TEXT affinity does not coerce a BLOB operand to
/// text — so after an upgrade every point query silently misses those legacy rows:
/// `mark_modified`/`purge_record` become no-ops and `upsert_record` inserts a second
/// BLOB-keyed row instead of updating, leaving orphaned and duplicated baseline rows.
/// (The full-table scan in [`load_index`] is unaffected because
/// [`read_index_key_column`] reads either storage class as raw bytes.)
///
/// Runs on every `initialize_schema` and is idempotent: once no TEXT keys remain, both
/// statements match nothing. A partially upgraded database can already hold a stale TEXT
/// row and a newer BLOB row for the same logical path (the duplicate an upgraded build
/// wrote); the delete drops the stale TEXT twin first so the `CAST` cannot hit a PRIMARY
/// KEY conflict, keeping the newer BLOB row.
fn normalize_legacy_text_keys(connection: &Connection) -> AppResult<()> {
    connection.execute_batch(
        r#"
        BEGIN;
        DELETE FROM file_index
         WHERE typeof(file_path) = 'text'
           AND CAST(file_path AS BLOB) IN (
               SELECT file_path FROM file_index WHERE typeof(file_path) = 'blob'
           );
        UPDATE file_index
           SET file_path = CAST(file_path AS BLOB)
         WHERE typeof(file_path) = 'text';
        COMMIT;
        "#,
    )?;
    Ok(())
}

fn migrate_file_index_schema(connection: &Connection) -> AppResult<()> {
    let columns = table_columns(connection, "file_index")?;
    let has_entity_kind = columns.iter().any(|column| column == "entity_kind");
    let sha1_not_null = table_column_not_null(connection, "file_index", "sha1_hash")?;
    if has_entity_kind && !sha1_not_null {
        return Ok(());
    }

    // Wrap the rename/create/copy/drop sequence in a single transaction. SQLite DDL is
    // transactional, so if the process crashes partway through, the whole migration rolls
    // back to the original `file_index` rather than leaving it renamed away but not yet
    // recreated (which would break daemon startup or lose the baseline).
    connection.execute_batch(
        r#"
        BEGIN;
        ALTER TABLE file_index RENAME TO file_index_old;
        CREATE TABLE file_index (
            file_path TEXT PRIMARY KEY,
            entity_kind TEXT NOT NULL DEFAULT 'file',
            file_size INTEGER NOT NULL,
            mtime INTEGER NOT NULL,
            sha1_hash TEXT,
            proton_id TEXT,
            sync_status TEXT NOT NULL
        );
        INSERT INTO file_index (file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status)
        SELECT file_path, 'file', file_size, mtime, sha1_hash, proton_id, sync_status
        FROM file_index_old;
        DROP TABLE file_index_old;
        COMMIT;
        "#,
    )?;
    Ok(())
}

fn table_columns(connection: &Connection, table_name: &str) -> AppResult<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row?);
    }
    Ok(columns)
}

fn table_column_not_null(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> AppResult<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)? != 0))
    })?;
    for row in rows {
        let (name, not_null) = row?;
        if name == column_name {
            return Ok(not_null);
        }
    }
    Ok(false)
}

pub fn load_index(connection: &Connection) -> AppResult<HashMap<PathBuf, FileRecord>> {
    let columns = table_columns(connection, "file_index")?;
    if !columns.iter().any(|column| column == "entity_kind") {
        return load_legacy_file_index(connection);
    }

    let mut statement = connection.prepare(
        "SELECT file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status FROM file_index",
    )?;
    let rows = statement.query_map([], |row| {
        let kind: String = row.get(1)?;
        let status: String = row.get(6)?;
        Ok(FileRecord {
            file_path: read_index_key_column(row, 0)?,
            entity_kind: EntityKind::from_str(&kind).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, err)
            })?,
            file_size: row.get::<_, i64>(2)? as u64,
            mtime: row.get(3)?,
            sha1_hash: row.get(4)?,
            proton_id: row.get(5)?,
            sync_status: SyncStatus::from_str(&status).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, err)
            })?,
        })
    })?;

    let mut index = HashMap::new();
    for row in rows {
        let record = row?;
        index.insert(record.file_path.clone(), record);
    }
    Ok(index)
}

fn load_legacy_file_index(connection: &Connection) -> AppResult<HashMap<PathBuf, FileRecord>> {
    let mut statement = connection.prepare(
        "SELECT file_path, file_size, mtime, sha1_hash, proton_id, sync_status FROM file_index",
    )?;
    let rows = statement.query_map([], |row| {
        let status: String = row.get(5)?;
        Ok(FileRecord {
            file_path: PathBuf::from(row.get::<_, String>(0)?),
            entity_kind: EntityKind::File,
            file_size: row.get::<_, i64>(1)? as u64,
            mtime: row.get(2)?,
            sha1_hash: Some(row.get(3)?),
            proton_id: row.get(4)?,
            sync_status: SyncStatus::from_str(&status).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, err)
            })?,
        })
    })?;

    let mut index = HashMap::new();
    for row in rows {
        let record = row?;
        index.insert(record.file_path.clone(), record);
    }
    Ok(index)
}

pub fn get_record(connection: &Connection, relative_path: &Path) -> AppResult<Option<FileRecord>> {
    let mut statement = connection.prepare(
        "SELECT file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status FROM file_index WHERE file_path = ?1",
    )?;
    let record = statement
        .query_row(params![index_key(relative_path)], |row| {
            let kind: String = row.get(1)?;
            let status: String = row.get(6)?;
            Ok(FileRecord {
                file_path: read_index_key_column(row, 0)?,
                entity_kind: EntityKind::from_str(&kind).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, err)
                })?,
                file_size: row.get::<_, i64>(2)? as u64,
                mtime: row.get(3)?,
                sha1_hash: row.get(4)?,
                proton_id: row.get(5)?,
                sync_status: SyncStatus::from_str(&status).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, err)
                })?,
            })
        })
        .optional()?;
    Ok(record)
}

pub fn upsert_record(connection: &Connection, record: &FileRecord) -> AppResult<()> {
    connection.execute(
        r#"
        INSERT INTO file_index (file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(file_path) DO UPDATE SET
            entity_kind = excluded.entity_kind,
            file_size = excluded.file_size,
            mtime = excluded.mtime,
            sha1_hash = excluded.sha1_hash,
            proton_id = excluded.proton_id,
            sync_status = excluded.sync_status
        "#,
        params![
            index_key(&record.file_path),
            record.entity_kind.as_str(),
            record.file_size as i64,
            record.mtime,
            record.sha1_hash,
            record.proton_id,
            record.sync_status.as_str(),
        ],
    )?;
    Ok(())
}

pub fn mark_modified(connection: &Connection, relative_path: &Path) -> AppResult<()> {
    connection.execute(
        "UPDATE file_index SET sync_status = 'modified' WHERE file_path = ?1",
        params![index_key(relative_path)],
    )?;
    Ok(())
}

pub fn purge_record(connection: &Connection, relative_path: &Path) -> AppResult<()> {
    connection.execute(
        "DELETE FROM file_index WHERE file_path = ?1",
        params![index_key(relative_path)],
    )?;
    Ok(())
}

/// Resolves a remote node id (`proton_id` — the composed `volumeId~nodeId` that a
/// `filesystem list` reports and the reconcile stores) to its indexed relative path.
///
/// This is the reverse of the normal path-keyed lookups and exists for **event-driven
/// reconcile**: a volume event carries only node ids, so a deletion/update/move must be mapped
/// back to a local path via the baseline index. A volume event's raw `LinkID` is bridged into
/// this id space with [`crate::events::node_uid`]. Returns `None` when no synced record holds
/// that id (e.g. a node created since the last full listing, whose id we have not recorded
/// yet — the caller then falls back to a targeted listing or a full scan).
///
/// Also returns `None` when *more than one* indexed path holds the id. Duplicate `proton_id`
/// rows are a reachable persisted state (issue #71): a withheld `LocalDelete` keeps the old
/// row while a `Download` commits a new row carrying the same id. Picking either row would be
/// arbitrary, so an ambiguous id is treated as unresolvable and the caller's safe fallback
/// (targeted listing / snapshot) takes over.
pub fn path_for_proton_id(connection: &Connection, proton_id: &str) -> AppResult<Option<PathBuf>> {
    let mut statement =
        connection.prepare("SELECT file_path FROM file_index WHERE proton_id = ?1 LIMIT 2")?;
    let mut paths = statement
        .query_map(params![proton_id], |row| read_index_key_column(row, 0))?
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > 1 {
        tracing::warn!(
            proton_id,
            "multiple indexed paths hold this proton_id (e.g. a withheld delete alongside a \
             re-downloaded copy); treating the id as unresolvable so the reconcile falls back \
             to a listing or snapshot"
        );
        return Ok(None);
    }
    Ok(paths.pop())
}

/// A persisted remote-event-stream cursor for one event scope. `scope_id` is Proton's
/// `treeEventScopeId` — a volume id for volume events (or `"core"` for the account stream).
/// `last_event_id` is the point the engine has fully processed; `updated_at` (epoch seconds,
/// supplied by the caller) records when, so freshness/age checks stay out of this pure layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCursor {
    pub scope_id: String,
    pub last_event_id: String,
    pub updated_at: i64,
}

/// Loads the stored cursor for `scope_id`, or `None` if none has been recorded yet.
pub fn load_event_cursor(
    connection: &Connection,
    scope_id: &str,
) -> AppResult<Option<EventCursor>> {
    let mut statement = connection.prepare(
        "SELECT scope_id, last_event_id, updated_at FROM remote_event_cursor WHERE scope_id = ?1",
    )?;
    let cursor = statement
        .query_row(params![scope_id], |row| {
            Ok(EventCursor {
                scope_id: row.get(0)?,
                last_event_id: row.get(1)?,
                updated_at: row.get(2)?,
            })
        })
        .optional()?;
    Ok(cursor)
}

/// Records (inserts or replaces) the cursor for `scope_id`.
pub fn store_event_cursor(
    connection: &Connection,
    scope_id: &str,
    last_event_id: &str,
    updated_at: i64,
) -> AppResult<()> {
    connection.execute(
        r#"
        INSERT INTO remote_event_cursor (scope_id, last_event_id, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(scope_id) DO UPDATE SET
            last_event_id = excluded.last_event_id,
            updated_at = excluded.updated_at
        "#,
        params![scope_id, last_event_id, updated_at],
    )?;
    Ok(())
}

/// Drops the cursor for `scope_id`, forcing the next pass to re-bootstrap from the latest
/// event id (used when the server signals a required full refresh).
pub fn clear_event_cursor(connection: &Connection, scope_id: &str) -> AppResult<()> {
    connection.execute(
        "DELETE FROM remote_event_cursor WHERE scope_id = ?1",
        params![scope_id],
    )?;
    Ok(())
}

/// A user's standing approval for one pending deletion, from the `delete_approvals` table. The
/// `fingerprint` pins the approval to the *exact* thing the user saw (a file's baseline SHA-1, or a
/// directory's `proton_id`): if the entity's content changes before the delete is applied, the
/// re-derived action no longer matches and the stale approval is inert. `path` + `direction` is the
/// primary key, so approving a path twice replaces the prior approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteApproval {
    pub path: PathBuf,
    pub direction: crate::sync::DeleteDirection,
    pub fingerprint: String,
    pub approved_at: i64,
}

/// Records (or replaces) the user's approval to delete `path` in `direction`.
pub fn upsert_delete_approval(
    connection: &Connection,
    path: &Path,
    direction: crate::sync::DeleteDirection,
    fingerprint: &str,
    approved_at: i64,
) -> AppResult<()> {
    connection.execute(
        r#"
        INSERT INTO delete_approvals (path, direction, fingerprint, approved_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(path, direction) DO UPDATE SET
            fingerprint = excluded.fingerprint,
            approved_at = excluded.approved_at
        "#,
        params![
            index_key(path),
            direction.as_str(),
            fingerprint,
            approved_at
        ],
    )?;
    Ok(())
}

/// Whether the user has a standing approval for exactly this deletion — same path, same direction,
/// and same `fingerprint`. A mismatched fingerprint (the entity changed since approval) does not
/// match, so a stale approval never authorizes a different deletion.
pub fn matching_delete_approval(
    connection: &Connection,
    path: &Path,
    direction: crate::sync::DeleteDirection,
    fingerprint: &str,
) -> AppResult<bool> {
    let mut statement = connection.prepare(
        "SELECT 1 FROM delete_approvals WHERE path = ?1 AND direction = ?2 AND fingerprint = ?3",
    )?;
    let found = statement
        .query_row(
            params![index_key(path), direction.as_str(), fingerprint],
            |_row| Ok(()),
        )
        .optional()?;
    Ok(found.is_some())
}

/// Removes any approval for `path` in `direction`. Used both to *consume* an approval inside the
/// post-success reconcile transaction (once the delete has actually run) and to *revoke* one on a
/// `deny`. A no-op when none exists.
pub fn delete_delete_approval(
    connection: &Connection,
    path: &Path,
    direction: crate::sync::DeleteDirection,
) -> AppResult<()> {
    connection.execute(
        "DELETE FROM delete_approvals WHERE path = ?1 AND direction = ?2",
        params![index_key(path), direction.as_str()],
    )?;
    Ok(())
}

/// All standing approvals (unordered). Used for status/inspection and tests.
pub fn load_delete_approvals(connection: &Connection) -> AppResult<Vec<DeleteApproval>> {
    let mut statement = connection
        .prepare("SELECT path, direction, fingerprint, approved_at FROM delete_approvals")?;
    let approvals = statement
        .query_map([], |row| {
            let direction: String = row.get(1)?;
            Ok(DeleteApproval {
                path: read_index_key_column(row, 0)?,
                direction: direction.parse().map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, err)
                })?,
                fingerprint: row.get(2)?,
                approved_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(approvals)
}

pub fn scan_local_files(root: &Path) -> AppResult<HashMap<PathBuf, LocalFileState>> {
    let options = ScanOptions::new(root, &[], &[], &[])?;
    scan_local_files_with_options(root, &options)
}

pub fn scan_local_files_with_options(
    root: &Path,
    options: &ScanOptions,
) -> AppResult<HashMap<PathBuf, LocalFileState>> {
    Ok(scan_local_entities_with_options(root, options)?
        .into_iter()
        .filter_map(|(path, state)| match state {
            LocalEntityState::File(file) => Some((path, file)),
            LocalEntityState::Directory(_) => None,
        })
        .collect())
}

pub fn scan_local_entities_with_options(
    root: &Path,
    options: &ScanOptions,
) -> AppResult<HashMap<PathBuf, LocalEntityState>> {
    scan_local_entities_reusing_hashes(root, options, &HashMap::new())
}

/// Like [`scan_local_entities_with_options`] but reuses each file's recorded SHA-1 when its
/// size and mtime are unchanged from `known` (the loaded base index), so an unchanged tree
/// is not fully re-hashed on every periodic reconcile. See [`local_file_state_reusing_hash`]
/// for the quick-check and its trade-off.
pub fn scan_local_entities_reusing_hashes(
    root: &Path,
    options: &ScanOptions,
    known: &HashMap<PathBuf, FileRecord>,
) -> AppResult<HashMap<PathBuf, LocalEntityState>> {
    let mut entities = HashMap::new();
    visit_directory(root, root, options, known, &mut entities)?;
    Ok(entities)
}

pub fn scan_local_entities(root: &Path) -> AppResult<HashMap<PathBuf, LocalEntityState>> {
    let options = ScanOptions::new(root, &[], &[], &[])?;
    scan_local_entities_with_options(root, &options)
}

fn visit_directory(
    root: &Path,
    directory: &Path,
    options: &ScanOptions,
    known: &HashMap<PathBuf, FileRecord>,
    entities: &mut HashMap<PathBuf, LocalEntityState>,
) -> AppResult<()> {
    // Note: this `read_dir` is NOT vanish-tolerant on purpose. For a *child* directory the
    // recursion call below maps its NotFound to a skip, but the top-level call must still
    // fail when the scan root itself is gone — treating a missing root as an empty tree
    // would plan a mass remote delete.
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        // An entry can vanish between `read_dir` listing it and any follow-up syscall
        // (editors replace files during save; build tools delete temp files). A vanished
        // entry is skipped instead of failing the whole scan (issue #76); every other
        // error still propagates. See `vanished_entry_to_skip`.
        let Some(file_type) = vanished_entry_to_skip(entry.file_type().map_err(Into::into))? else {
            continue;
        };
        let relative_path = path.strip_prefix(root).map_err(|err| {
            boxed_error(format!(
                "failed to compute relative path for {} against {}: {err}",
                path.display(),
                root.display()
            ))
        })?;
        if file_type.is_dir() {
            if options.allows_directory_traversal(relative_path) {
                if options.allows_relative_directory(relative_path) {
                    let Some(state) = vanished_entry_to_skip(local_directory_state(root, &path))?
                    else {
                        continue;
                    };
                    entities.insert(
                        state.relative_path.clone(),
                        LocalEntityState::Directory(state),
                    );
                }
                // A NotFound surfacing here is this child directory's own `read_dir`
                // failing (the directory vanished mid-scan) — deeper vanishes were
                // already skipped inside the recursion.
                vanished_entry_to_skip(visit_directory(root, &path, options, known, entities))?;
            }
            continue;
        }
        if !file_type.is_file() || !options.allows_relative_file(relative_path) {
            continue;
        }
        let Some(state) =
            vanished_entry_to_skip(local_file_state_reusing_hash(root, &path, known))?
        else {
            continue;
        };
        entities.insert(state.relative_path.clone(), LocalEntityState::File(state));
    }
    Ok(())
}

/// Maps a per-entry scan error to a skip when the entry vanished between `read_dir` listing
/// it and the follow-up stat/open (issue #76: an editor save/replace race would otherwise
/// abort the entire scan and reconcile pass): `NotFound` becomes `Ok(None)` ("gone — skip
/// it"), every other error — permissions in particular — still fails the scan, since silently
/// dropping an unreadable file would replan it as locally deleted.
fn vanished_entry_to_skip<T>(result: AppResult<T>) -> AppResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_error| io_error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub fn should_ignore_path(path: &Path) -> bool {
    should_ignore_relative_path(path)
}

fn should_ignore_relative_path(relative_path: &Path) -> bool {
    if crate::sync::is_conflict_copy(relative_path)
        || is_download_scratch_path(relative_path)
        || is_sync_state_path(relative_path)
    {
        return true;
    }
    // The per-directory settings file is machine-local policy: ignore it at any depth (like the
    // legacy DB name) so it is never scanned, planned, base-index-filtered, or watched for upload.
    // The legacy DB name also covers its transient SQLite sidecars (`-journal`/`-wal`/`-shm`).
    let file_name = relative_path.file_name().and_then(|value| value.to_str());
    file_name == Some("sync_index.db")
        || file_name == Some("sync_index.db-journal")
        || file_name == Some("sync_index.db-wal")
        || file_name == Some("sync_index.db-shm")
        || file_name == Some(crate::dirconfig::DIRECTORY_CONFIG_FILE_NAME)
}

/// True when `relative_path` is the per-root `.sync` state directory or anything inside it (the
/// engine's own SQLite index, its status/metrics sidecars, and the instance lockfile — see
/// [`crate::paths::sync_state_dir`]). Only a *top-level* `.sync` is the state directory; a `.sync`
/// nested deeper in the tree is ordinary user data and syncs normally, so only the first path
/// component is checked. Honoring it in the scanner, the base-index filter, and the watcher keeps
/// the engine's own state from ever being planned for upload.
fn is_sync_state_path(relative_path: &Path) -> bool {
    relative_path.components().next().is_some_and(|component| {
        component.as_os_str() == std::ffi::OsStr::new(crate::paths::SYNC_STATE_DIR_NAME)
    })
}

/// True when any component of `relative_path` is (or lives inside) a download-staging
/// scratch directory created by `ProtonDriveClient::download` (see
/// [`crate::DOWNLOAD_SCRATCH_PREFIX`]). Such a directory lives inside the synced root so
/// the final download move is an atomic same-filesystem rename; it is normally removed
/// once the download completes, but a hard crash mid-download can leave one behind.
/// Ignoring the prefix in the scanner, the base-index filter, and the watcher keeps a
/// leftover scratch directory and its partial file from being uploaded to the remote as
/// junk. Matched byte-exactly so a component is compared regardless of UTF-8 validity.
fn is_download_scratch_path(relative_path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    relative_path.components().any(|component| {
        component
            .as_os_str()
            .as_bytes()
            .starts_with(crate::DOWNLOAD_SCRATCH_PREFIX.as_bytes())
    })
}

/// The transient sibling files SQLite creates next to a database file: the rollback
/// journal, write-ahead log, and shared-memory index. Built by appending to the full
/// file name (SQLite's own convention: `custom.db` → `custom.db-journal`), not by
/// replacing the extension.
fn sqlite_sidecar_paths(path: &Path) -> impl Iterator<Item = PathBuf> {
    ["-journal", "-wal", "-shm"].into_iter().map(|suffix| {
        let mut sibling = path.as_os_str().to_os_string();
        sibling.push(suffix);
        PathBuf::from(sibling)
    })
}

/// Resolves one configured ignore path (the index DB, its sidecars, the lockfile, …) to a
/// root-relative entry for [`ScanOptions`], or `None` when it lies outside the root (nothing
/// to ignore there).
///
/// Matching is best-effort canonical, not purely lexical (issue #73): a relative `root`
/// combined with an absolute `path` — or `..`/symlink spellings of either — must still
/// anchor, otherwise the ignore silently drops and the engine scans, hashes, and uploads its
/// own live SQLite DB. A `path` that cannot be anchored under `root` but is itself relative
/// keeps the pre-existing behavior of being treated as already root-relative.
fn normalize_ignored_path(root: &Path, path: &Path) -> Option<PathBuf> {
    // Fast lexical path first: identical spellings strip directly.
    if let Ok(stripped) = path.strip_prefix(root)
        && let Some(normalized) = crate::validate_relative_path(stripped)
    {
        return Some(normalized);
    }
    // Lexical stripping failed (or left `..` components behind): compare canonical forms.
    if let Ok(stripped) =
        canonicalize_best_effort(path).strip_prefix(canonicalize_best_effort(root))
        && let Some(normalized) = crate::validate_relative_path(stripped)
    {
        return Some(normalized);
    }
    if path.is_relative() {
        return crate::validate_relative_path(path);
    }
    None
}

/// Canonicalizes `path` when it exists on disk; otherwise falls back to anchoring it against
/// the current working directory and resolving `.`/`..` components lexically. Used only for
/// prefix *matching* in [`normalize_ignored_path`], never for filesystem access.
fn canonicalize_best_effort(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(current_dir) = std::env::current_dir() {
        current_dir.join(path)
    } else {
        path.to_path_buf()
    };
    lexically_normalized(&absolute)
}

/// Resolves `.` and `..` components without touching the filesystem. `..` above the root is
/// clamped (POSIX: `/..` is `/`); a leading `..` on a relative path is preserved so the
/// result never claims a location the input did not name.
fn lexically_normalized(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => normalized.push(Component::ParentDir.as_os_str()),
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn build_glob_set(patterns: &[String]) -> AppResult<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

pub fn local_file_state(root: &Path, absolute_path: &Path) -> AppResult<LocalFileState> {
    local_file_state_reusing_hash(root, absolute_path, &HashMap::new())
}

/// Like [`local_file_state`] but consults `known` (a `relative_path -> FileRecord` map,
/// typically the loaded base index) as an rsync-style quick-check: when the file's current
/// size and mtime both match the recorded values, its stored SHA-1 is reused instead of
/// re-streaming the entire file through the hasher. A missing record, a type change, or a
/// size/mtime change forces a re-hash.
///
/// mtime is compared at the second resolution the record stores, so a content change that
/// preserves both the size and the mtime-second is not detected until the next
/// size/mtime-changing edit — the standard quick-check trade-off, taken here to avoid
/// re-reading and re-hashing an unchanged multi-gigabyte tree on every periodic reconcile.
pub fn local_file_state_reusing_hash(
    root: &Path,
    absolute_path: &Path,
    known: &HashMap<PathBuf, FileRecord>,
) -> AppResult<LocalFileState> {
    let metadata = fs::metadata(absolute_path)?;
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|err| {
            boxed_error(format!(
                "mtime before epoch for {}: {err}",
                absolute_path.display()
            ))
        })?
        .as_secs() as i64;
    let relative_path = absolute_path.strip_prefix(root).map_err(|err| {
        boxed_error(format!(
            "failed to compute relative path for {} against {}: {err}",
            absolute_path.display(),
            root.display()
        ))
    })?;
    let file_size = metadata.len();

    let sha1_hash = match known.get(relative_path) {
        Some(record)
            if record.entity_kind == EntityKind::File
                && record.file_size == file_size
                && record.mtime == mtime =>
        {
            match &record.sha1_hash {
                Some(hash) => hash.clone(),
                None => compute_sha1(absolute_path)?,
            }
        }
        _ => compute_sha1(absolute_path)?,
    };

    Ok(LocalFileState {
        relative_path: relative_path.to_path_buf(),
        absolute_path: absolute_path.to_path_buf(),
        file_size,
        mtime,
        sha1_hash,
    })
}

pub fn local_directory_state(root: &Path, absolute_path: &Path) -> AppResult<LocalDirectoryState> {
    let metadata = fs::metadata(absolute_path)?;
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|err| {
            boxed_error(format!(
                "mtime before epoch for {}: {err}",
                absolute_path.display()
            ))
        })?
        .as_secs() as i64;
    let relative_path = absolute_path.strip_prefix(root).map_err(|err| {
        boxed_error(format!(
            "failed to compute relative path for {} against {}: {err}",
            absolute_path.display(),
            root.display()
        ))
    })?;

    Ok(LocalDirectoryState {
        relative_path: relative_path.to_path_buf(),
        absolute_path: absolute_path.to_path_buf(),
        mtime,
    })
}

pub fn compute_sha1(path: &Path) -> AppResult<String> {
    use std::fmt::Write as _;

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 8 * 1024];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hex, "{byte:02x}").expect("writing hex digits to a String cannot fail");
    }
    Ok(hex)
}

pub fn path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Byte-exact primary-key encoding for `file_index.file_path`, used for every SQL
/// read/write of a specific row. Unlike `path_key` (which lossily converts each
/// component to UTF-8 for glob-pattern matching), this preserves the original bytes
/// of non-UTF-8 path components so two different non-UTF-8 paths can never collide on
/// the same database key. Bound as a `Vec<u8>` parameter, it is stored using SQLite's
/// BLOB storage class even in this TEXT-affinity column: SQLite's column-affinity
/// rules only convert INTEGER/REAL input for a TEXT-affinity column, never TEXT or
/// BLOB input, so no schema migration is required.
fn index_key(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    let mut key = Vec::new();
    for component in path.components() {
        if !key.is_empty() {
            key.push(b'/');
        }
        key.extend_from_slice(component.as_os_str().as_bytes());
    }
    key
}

/// Reconstructs the `PathBuf` written by `index_key`. Reads the column as raw bytes
/// regardless of whether it is stored using SQLite's TEXT or BLOB storage class, so
/// rows written by older code (as TEXT, via `path_key`) remain readable without a
/// migration.
fn read_index_key_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let value = row.get_ref(index)?;
    let data_type = value.data_type();
    let bytes = value.as_bytes().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, data_type, Box::new(error))
    })?;
    Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn computes_empty_file_sha1() {
        let directory = tempdir().expect("tempdir");
        let file_path = directory.path().join("empty.txt");
        File::create(&file_path).expect("empty file");

        let hash = compute_sha1(&file_path).expect("sha1 hash");

        assert_eq!(hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn scans_files_and_ignores_conflict_copies_and_db() {
        let directory = tempdir().expect("tempdir");
        let keep_path = directory.path().join("keep.txt");
        let conflict_path = directory.path().join("keep.proton-cloud.txt");
        let db_path = directory.path().join("sync_index.db");
        std::fs::write(&keep_path, b"keep").expect("write keep");
        std::fs::write(&conflict_path, b"conflict").expect("write conflict");
        std::fs::write(&db_path, b"db").expect("write db");

        let files = scan_local_files(directory.path()).expect("scan files");

        assert_eq!(files.len(), 1);
        assert!(files.contains_key(Path::new("keep.txt")));
    }

    #[test]
    fn scanner_ignores_orphaned_download_scratch_directories() {
        // A download scratch directory left behind by a hard crash (SIGKILL/OOM/power
        // loss) mid-download persists inside the synced root. It must never be scanned or
        // planned, otherwise the bootstrap planner would upload the junk directory and
        // its partial file to the remote. Scratch dirs appear next to their download
        // target, so cover both a top-level and a nested one.
        let directory = tempdir().expect("tempdir");
        let scratch_top = directory
            .path()
            .join(format!("{}1234-9876", crate::DOWNLOAD_SCRATCH_PREFIX));
        let nested_parent = directory.path().join("reports");
        let scratch_nested = nested_parent.join(format!("{}5-6", crate::DOWNLOAD_SCRATCH_PREFIX));
        fs::create_dir(&scratch_top).expect("top scratch dir");
        fs::create_dir_all(&scratch_nested).expect("nested scratch dir");
        fs::write(scratch_top.join("budget.xlsx"), b"partial").expect("top partial");
        fs::write(scratch_nested.join("deep.bin"), b"partial").expect("nested partial");
        fs::write(nested_parent.join("real.txt"), b"real").expect("real file");
        fs::write(directory.path().join("keep.txt"), b"keep").expect("keep file");

        let entities = scan_local_entities(directory.path()).expect("scan entities");

        assert!(entities.contains_key(Path::new("keep.txt")));
        assert!(entities.contains_key(Path::new("reports/real.txt")));
        assert!(
            entities.keys().all(|path| !is_download_scratch_path(path)),
            "no scratch directory or its contents may be scanned: {entities:?}"
        );
    }

    #[test]
    fn scan_options_reject_download_scratch_paths() {
        let options = ScanOptions::new(Path::new("/root"), &[], &[], &[]).expect("scan options");
        let scratch_file =
            PathBuf::from(format!("{}9-9/budget.xlsx", crate::DOWNLOAD_SCRATCH_PREFIX));
        let scratch_dir = PathBuf::from(format!("{}9-9", crate::DOWNLOAD_SCRATCH_PREFIX));
        assert!(
            !options.allows_relative_file(&scratch_file),
            "the watcher and base-index filter must skip scratch-dir files"
        );
        assert!(!options.allows_relative_directory(&scratch_dir));
        // A nested scratch directory (staged next to a deep download target) is skipped too.
        let nested = PathBuf::from(format!("reports/{}9-9", crate::DOWNLOAD_SCRATCH_PREFIX));
        assert!(!options.allows_relative_directory(&nested));
        // A normally-named path is still allowed.
        assert!(options.allows_relative_file(Path::new("reports/real.txt")));
    }

    #[test]
    fn scans_directory_entities_including_empty_directories() {
        let directory = tempdir().expect("tempdir");
        fs::create_dir(directory.path().join("empty")).expect("empty dir");
        fs::create_dir(directory.path().join("with-file")).expect("with-file dir");
        fs::write(directory.path().join("with-file/note.txt"), b"note").expect("note file");

        let entities = scan_local_entities(directory.path()).expect("scan entities");

        assert!(matches!(
            entities.get(Path::new("empty")),
            Some(LocalEntityState::Directory(_))
        ));
        assert!(matches!(
            entities.get(Path::new("with-file")),
            Some(LocalEntityState::Directory(_))
        ));
        assert!(matches!(
            entities.get(Path::new("with-file/note.txt")),
            Some(LocalEntityState::File(_))
        ));
        assert!(
            !entities.contains_key(Path::new("")),
            "scanner must not emit the sync root as a directory entity"
        );
    }

    #[test]
    fn a_vanished_entry_is_skipped_but_other_errors_still_fail_the_scan() {
        // A true mid-scan vanish (listed by read_dir, gone before stat/open) cannot be
        // reproduced deterministically, so this exercises the exact seam `visit_directory`
        // routes every per-entry operation through: the real call chains' NotFound must map
        // to a skip, and only NotFound may.
        let directory = tempdir().expect("tempdir");
        let missing = directory.path().join("gone.txt");

        assert!(
            vanished_entry_to_skip(local_file_state_reusing_hash(
                directory.path(),
                &missing,
                &HashMap::new(),
            ))
            .expect("a vanished file maps to a skip, not a scan failure")
            .is_none()
        );
        assert!(
            vanished_entry_to_skip(local_directory_state(directory.path(), &missing))
                .expect("a vanished directory maps to a skip")
                .is_none()
        );
        // A child directory that vanished before its own read_dir (the recursion call site).
        let options = ScanOptions::new(directory.path(), &[], &[], &[]).expect("options");
        let mut entities = HashMap::new();
        assert!(
            vanished_entry_to_skip(visit_directory(
                directory.path(),
                &missing,
                &options,
                &HashMap::new(),
                &mut entities,
            ))
            .expect("a directory vanishing before its read_dir maps to a skip")
            .is_none()
        );

        // Any other error propagates unchanged: a permission failure must still fail the
        // scan rather than silently dropping files (which would replan them as deleted).
        let denied: AppResult<()> =
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied").into());
        assert!(
            vanished_entry_to_skip(denied).is_err(),
            "only NotFound may be treated as a vanish"
        );
        let ok: AppResult<u32> = Ok(7);
        assert_eq!(vanished_entry_to_skip(ok).expect("ok"), Some(7));
    }

    #[test]
    fn scan_options_ignore_configured_index_path() {
        let directory = tempdir().expect("tempdir");
        let state_directory = directory.path().join(".state");
        fs::create_dir(&state_directory).expect("state dir");
        let custom_db_path = state_directory.join("custom.db");
        let keep_path = directory.path().join("keep.txt");
        std::fs::write(&custom_db_path, b"db").expect("write custom db");
        std::fs::write(&keep_path, b"keep").expect("write keep");
        let options = ScanOptions::new(
            directory.path(),
            std::slice::from_ref(&custom_db_path),
            &[],
            &[],
        )
        .expect("scan options");

        let files = scan_local_files_with_options(directory.path(), &options).expect("scan files");

        assert_eq!(files.len(), 1);
        assert!(files.contains_key(Path::new("keep.txt")));
        assert!(
            !files.contains_key(Path::new(".state/custom.db")),
            "configured index path must not be considered sync data"
        );
    }

    #[test]
    fn scan_options_normalize_configured_index_below_relative_root() {
        let options = ScanOptions::new(
            Path::new("sync-root"),
            &[PathBuf::from("sync-root/state/custom.db")],
            &[],
            &[],
        )
        .expect("scan options");

        assert!(
            !options.allows_relative_file(Path::new("state/custom.db")),
            "relative db paths joined under a relative local root must be ignored"
        );
    }

    #[test]
    fn scan_options_canonicalize_root_and_db_path_before_ignore_matching() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("x");
        let state = root.join(".state");
        fs::create_dir_all(&state).expect("dirs");
        let db_path = state.join("custom.db");
        std::fs::write(&db_path, b"db").expect("write db");
        std::fs::write(root.join("keep.txt"), b"keep").expect("write keep");

        // A `..`-spelled root does not lexically prefix the plainly-spelled db path — the
        // issue #73 shape (relative/`..`/symlink root vs absolute db path). Without canonical
        // matching the ignore silently drops and the engine scans/uploads its own live DB.
        let dotted_root = directory.path().join("x").join("..").join("x");
        let options = ScanOptions::new(&dotted_root, std::slice::from_ref(&db_path), &[], &[])
            .expect("scan options");
        assert!(
            !options.allows_relative_file(Path::new(".state/custom.db")),
            "a db path that only matches the root after canonicalization must still be ignored"
        );
        let files = scan_local_files_with_options(&dotted_root, &options).expect("scan files");
        assert!(files.contains_key(Path::new("keep.txt")));
        assert!(
            !files.contains_key(Path::new(".state/custom.db")),
            "the engine's own DB must never be scanned as sync data: {files:?}"
        );

        // The reverse spelling — plain root, `..`-spelled db path that does not even exist
        // yet — resolves through the lexical-normalization fallback.
        let dotted_db = root
            .join(".state")
            .join("..")
            .join(".state")
            .join("other.db");
        let options = ScanOptions::new(&root, std::slice::from_ref(&dotted_db), &[], &[])
            .expect("scan options");
        assert!(
            !options.allows_relative_file(Path::new(".state/other.db")),
            "a not-yet-created db path spelled with `..` must normalize into the ignore set"
        );
    }

    #[test]
    fn scan_options_ignore_sqlite_sidecars_of_the_configured_db_path() {
        let directory = tempdir().expect("tempdir");
        let state = directory.path().join(".state");
        fs::create_dir(&state).expect("state dir");
        let db_path = state.join("custom.db");
        std::fs::write(&db_path, b"db").expect("write db");
        std::fs::write(state.join("custom.db-journal"), b"j").expect("write journal");
        std::fs::write(state.join("custom.db-wal"), b"w").expect("write wal");
        std::fs::write(state.join("custom.db-shm"), b"s").expect("write shm");
        std::fs::write(directory.path().join("keep.txt"), b"keep").expect("write keep");

        let options = ScanOptions::new(directory.path(), std::slice::from_ref(&db_path), &[], &[])
            .expect("scan options");

        assert!(!options.allows_relative_file(Path::new(".state/custom.db-journal")));
        assert!(!options.allows_relative_file(Path::new(".state/custom.db-wal")));
        assert!(!options.allows_relative_file(Path::new(".state/custom.db-shm")));
        let files = scan_local_files_with_options(directory.path(), &options).expect("scan files");
        assert_eq!(
            files.len(),
            1,
            "a relocated DB's transient SQLite files must never sync: {files:?}"
        );
        assert!(files.contains_key(Path::new("keep.txt")));
    }

    #[test]
    fn scan_ignores_top_level_sync_state_dir_but_keeps_nested_dot_sync() {
        let directory = tempdir().expect("tempdir");
        // Engine state under the top-level .sync/ must never be treated as sync data.
        let state = directory.path().join(".sync");
        fs::create_dir(&state).expect("state dir");
        std::fs::write(state.join("sync_index.db"), b"db").expect("db");
        std::fs::write(state.join("proton-sync.lock"), b"lock").expect("lock");
        std::fs::write(state.join("sync_index.status.json"), b"[]").expect("status");
        // A .sync directory nested deeper in the tree is ordinary user data and must sync normally.
        let nested = directory.path().join("docs").join(".sync");
        fs::create_dir_all(&nested).expect("nested dir");
        std::fs::write(nested.join("notes.txt"), b"notes").expect("nested file");
        std::fs::write(directory.path().join("keep.txt"), b"keep").expect("keep");

        let options = ScanOptions::new(directory.path(), &[], &[], &[]).expect("scan options");
        let files = scan_local_files_with_options(directory.path(), &options).expect("scan files");

        assert!(files.contains_key(Path::new("keep.txt")));
        assert!(
            files.contains_key(Path::new("docs/.sync/notes.txt")),
            "a .sync directory nested below the root is user data and must be synced"
        );
        assert!(
            !files.keys().any(|path| path.starts_with(".sync")),
            "nothing under the top-level .sync state directory may be planned: {:?}",
            files.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn scan_options_reject_the_sync_state_dir_and_its_contents() {
        let options = ScanOptions::new(Path::new("root"), &[], &[], &[]).expect("scan options");

        assert!(!options.allows_relative_directory(Path::new(".sync")));
        assert!(!options.allows_relative_file(Path::new(".sync/sync_index.db")));
        assert!(!options.allows_relative_file(Path::new(".sync/proton-sync.lock")));
        assert!(!options.allows_relative_file(Path::new(".sync/nested/anything")));
        // Only the *top-level* .sync is the state directory; a nested one is user data.
        assert!(options.allows_relative_directory(Path::new("docs/.sync")));
        assert!(options.allows_relative_file(Path::new("docs/.sync/notes.txt")));
    }

    #[test]
    fn scan_options_apply_include_and_exclude_patterns() {
        let directory = tempdir().expect("tempdir");
        let docs = directory.path().join("docs");
        let images = directory.path().join("images");
        fs::create_dir(&docs).expect("docs dir");
        fs::create_dir(&images).expect("images dir");
        std::fs::write(docs.join("keep.md"), b"keep").expect("write keep");
        std::fs::write(docs.join("drop.tmp"), b"drop").expect("write drop");
        std::fs::write(images.join("skip.png"), b"skip").expect("write skip");
        let options = ScanOptions::new(
            directory.path(),
            &[],
            &["docs/**".to_owned()],
            &["**/*.tmp".to_owned()],
        )
        .expect("scan options");

        let files = scan_local_files_with_options(directory.path(), &options).expect("scan files");

        assert_eq!(files.len(), 1);
        assert!(files.contains_key(Path::new("docs/keep.md")));
    }

    #[test]
    fn database_round_trip_preserves_status() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        let record = FileRecord {
            file_path: PathBuf::from("notes.txt"),
            entity_kind: EntityKind::File,
            file_size: 4,
            mtime: 123,
            sha1_hash: Some("abcd".to_owned()),
            proton_id: Some("remote-1".to_owned()),
            sync_status: SyncStatus::Conflict,
        };
        upsert_record(&connection, &record).expect("upsert");
        mark_modified(&connection, Path::new("notes.txt")).expect("mark modified");
        let loaded = get_record(&connection, Path::new("notes.txt"))
            .expect("get record")
            .expect("record exists");

        assert_eq!(loaded.sync_status, SyncStatus::Modified);
        assert_eq!(loaded.proton_id.as_deref(), Some("remote-1"));
    }

    #[test]
    fn database_round_trip_preserves_directory_records_without_hashes() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        let directory_state = LocalDirectoryState {
            relative_path: PathBuf::from("empty"),
            absolute_path: PathBuf::from("/tmp/empty"),
            mtime: 123,
        };
        let record = FileRecord::from_local_directory(
            PathBuf::from("empty"),
            &directory_state,
            Some("remote-dir-id".to_owned()),
            SyncStatus::Synced,
        );

        upsert_record(&connection, &record).expect("upsert directory");
        let loaded = get_record(&connection, Path::new("empty"))
            .expect("get record")
            .expect("record exists");

        assert_eq!(loaded.entity_kind, EntityKind::Directory);
        assert_eq!(loaded.sha1_hash, None);
        assert_eq!(loaded.proton_id.as_deref(), Some("remote-dir-id"));
        assert_eq!(loaded.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn delete_approval_round_trip_matches_only_the_same_fingerprint() {
        use crate::sync::DeleteDirection;

        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");

        upsert_delete_approval(
            &connection,
            Path::new("notes.txt"),
            DeleteDirection::Local,
            "hash-a",
            100,
        )
        .expect("upsert approval");

        // Exact match (path + direction + fingerprint) is approved.
        assert!(
            matching_delete_approval(
                &connection,
                Path::new("notes.txt"),
                DeleteDirection::Local,
                "hash-a"
            )
            .expect("match")
        );
        // A different fingerprint (the file changed since approval) must NOT match.
        assert!(
            !matching_delete_approval(
                &connection,
                Path::new("notes.txt"),
                DeleteDirection::Local,
                "hash-b"
            )
            .expect("match")
        );
        // The other direction is a distinct, independently-keyed approval.
        assert!(
            !matching_delete_approval(
                &connection,
                Path::new("notes.txt"),
                DeleteDirection::Remote,
                "hash-a"
            )
            .expect("match")
        );

        // Consuming (or revoking) removes it.
        delete_delete_approval(&connection, Path::new("notes.txt"), DeleteDirection::Local)
            .expect("delete approval");
        assert!(
            !matching_delete_approval(
                &connection,
                Path::new("notes.txt"),
                DeleteDirection::Local,
                "hash-a"
            )
            .expect("match")
        );
        assert!(
            load_delete_approvals(&connection)
                .expect("load approvals")
                .is_empty()
        );
    }

    #[test]
    fn per_directory_config_file_is_ignored_at_any_depth() {
        assert!(should_ignore_path(Path::new(".proton-sync.toml")));
        assert!(should_ignore_path(Path::new("a/b/.proton-sync.toml")));
        // A same-named directory or unrelated file is not ignored.
        assert!(!should_ignore_path(Path::new("a/proton-sync.toml")));
        assert!(!should_ignore_path(Path::new("notes.txt")));
    }

    #[cfg(unix)]
    #[test]
    fn database_distinguishes_non_utf8_paths_that_collide_under_lossy_conversion() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");

        let path_a = PathBuf::from(OsStr::from_bytes(b"fo\x80o.txt"));
        let path_b = PathBuf::from(OsStr::from_bytes(b"fo\x81o.txt"));
        assert_eq!(
            path_a.to_string_lossy(),
            path_b.to_string_lossy(),
            "test paths must actually collide under lossy UTF-8 conversion for this \
             test to be meaningful"
        );

        let record_a = FileRecord {
            file_path: path_a.clone(),
            entity_kind: EntityKind::File,
            file_size: 1,
            mtime: 1,
            sha1_hash: Some("abcd".to_owned()),
            proton_id: Some("remote-a".to_owned()),
            sync_status: SyncStatus::Synced,
        };
        let record_b = FileRecord {
            file_path: path_b.clone(),
            entity_kind: EntityKind::File,
            file_size: 2,
            mtime: 2,
            sha1_hash: Some("ef01".to_owned()),
            proton_id: Some("remote-b".to_owned()),
            sync_status: SyncStatus::Synced,
        };

        upsert_record(&connection, &record_a).expect("upsert a");
        upsert_record(&connection, &record_b).expect("upsert b");

        let loaded_a = get_record(&connection, &path_a)
            .expect("get a")
            .expect("a exists");
        let loaded_b = get_record(&connection, &path_b)
            .expect("get b")
            .expect("b exists");

        assert_eq!(loaded_a.file_path, path_a);
        assert_eq!(loaded_a.proton_id.as_deref(), Some("remote-a"));
        assert_eq!(loaded_b.file_path, path_b);
        assert_eq!(loaded_b.proton_id.as_deref(), Some("remote-b"));

        let index = load_index(&connection).expect("load index");
        assert_eq!(
            index.len(),
            2,
            "both non-UTF-8 paths must coexist as distinct rows instead of colliding"
        );
    }

    #[test]
    fn event_cursor_round_trips_and_clears() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");

        assert!(
            load_event_cursor(&connection, "vol-1")
                .expect("load absent")
                .is_none(),
            "an unrecorded scope has no cursor"
        );

        store_event_cursor(&connection, "vol-1", "cursor-a", 100).expect("store");
        let loaded = load_event_cursor(&connection, "vol-1")
            .expect("load")
            .expect("cursor exists");
        assert_eq!(
            loaded,
            EventCursor {
                scope_id: "vol-1".to_owned(),
                last_event_id: "cursor-a".to_owned(),
                updated_at: 100,
            }
        );

        // Upsert in place, not a duplicate row.
        store_event_cursor(&connection, "vol-1", "cursor-b", 200).expect("update");
        assert_eq!(
            load_event_cursor(&connection, "vol-1")
                .expect("load")
                .expect("cursor")
                .last_event_id,
            "cursor-b"
        );
        let count: i64 = connection
            .query_row("SELECT count(*) FROM remote_event_cursor", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(
            count, 1,
            "storing must update the existing scope row in place"
        );

        // A second scope is independent.
        store_event_cursor(&connection, "core", "core-cursor", 300).expect("store core");
        assert_eq!(
            load_event_cursor(&connection, "core")
                .expect("load core")
                .expect("core cursor")
                .last_event_id,
            "core-cursor"
        );

        clear_event_cursor(&connection, "vol-1").expect("clear");
        assert!(
            load_event_cursor(&connection, "vol-1")
                .expect("load after clear")
                .is_none(),
            "clearing removes the cursor"
        );
        assert!(
            load_event_cursor(&connection, "core")
                .expect("load core")
                .is_some(),
            "clearing one scope must not affect another"
        );
    }

    #[test]
    fn path_for_proton_id_resolves_a_node_id_and_misses_cleanly() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        let record = FileRecord {
            file_path: PathBuf::from("docs/report.txt"),
            entity_kind: EntityKind::File,
            file_size: 3,
            mtime: 7,
            sha1_hash: Some("abcd".to_owned()),
            proton_id: Some("vol~node-42".to_owned()),
            sync_status: SyncStatus::Synced,
        };
        upsert_record(&connection, &record).expect("upsert");

        assert_eq!(
            path_for_proton_id(&connection, "vol~node-42").expect("lookup"),
            Some(PathBuf::from("docs/report.txt"))
        );
        assert_eq!(
            path_for_proton_id(&connection, "vol~unknown").expect("lookup missing"),
            None,
            "an unrecorded node id resolves to None"
        );

        // A record without a proton_id must never be matched by an id lookup.
        let no_id = FileRecord {
            file_path: PathBuf::from("local-only.txt"),
            entity_kind: EntityKind::File,
            file_size: 1,
            mtime: 1,
            sha1_hash: Some("ef01".to_owned()),
            proton_id: None,
            sync_status: SyncStatus::Modified,
        };
        upsert_record(&connection, &no_id).expect("upsert no-id");
        assert_eq!(
            path_for_proton_id(&connection, "vol~node-42").expect("lookup"),
            Some(PathBuf::from("docs/report.txt")),
            "the id lookup is unaffected by rows lacking a proton_id"
        );

        // After purge the id no longer resolves.
        purge_record(&connection, Path::new("docs/report.txt")).expect("purge");
        assert_eq!(
            path_for_proton_id(&connection, "vol~node-42").expect("lookup after purge"),
            None
        );
    }

    #[test]
    fn path_for_proton_id_returns_none_when_two_paths_share_the_id() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        // Reachable persisted state (issue #71): a withheld LocalDelete keeps the old row
        // while a Download commits a new row carrying the same proton_id.
        for path in ["old/report.txt", "new/report.txt"] {
            let record = FileRecord {
                file_path: PathBuf::from(path),
                entity_kind: EntityKind::File,
                file_size: 3,
                mtime: 7,
                sha1_hash: Some("abcd".to_owned()),
                proton_id: Some("vol~node-9".to_owned()),
                sync_status: SyncStatus::Synced,
            };
            upsert_record(&connection, &record).expect("upsert");
        }

        assert_eq!(
            path_for_proton_id(&connection, "vol~node-9").expect("lookup"),
            None,
            "an id held by two distinct paths is ambiguous and must resolve to None so the \
             caller falls back to a listing/snapshot instead of picking an arbitrary row"
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_for_proton_id_reads_non_utf8_paths() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        let path = PathBuf::from(OsStr::from_bytes(b"weird\x80name.bin"));
        let record = FileRecord {
            file_path: path.clone(),
            entity_kind: EntityKind::File,
            file_size: 1,
            mtime: 1,
            sha1_hash: Some("abcd".to_owned()),
            proton_id: Some("vol~n1".to_owned()),
            sync_status: SyncStatus::Synced,
        };
        upsert_record(&connection, &record).expect("upsert");
        assert_eq!(
            path_for_proton_id(&connection, "vol~n1").expect("lookup"),
            Some(path),
            "the reverse lookup must reconstruct non-UTF-8 paths like the point queries do"
        );
    }

    #[test]
    fn cursor_table_and_proton_id_index_are_added_to_a_pre_existing_database() {
        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("sync_index.db");
        {
            // A database with only the original file_index table (no cursor table / index).
            let connection = Connection::open(&db_path).expect("open");
            connection
                .execute_batch(
                    r#"CREATE TABLE file_index (
                        file_path TEXT PRIMARY KEY,
                        entity_kind TEXT NOT NULL DEFAULT 'file',
                        file_size INTEGER NOT NULL,
                        mtime INTEGER NOT NULL,
                        sha1_hash TEXT,
                        proton_id TEXT,
                        sync_status TEXT NOT NULL
                    );"#,
                )
                .expect("legacy schema");
        }

        // Reopen through the real entry point, which must add the new table and index.
        let connection = open_database(&db_path).expect("open database");
        store_event_cursor(&connection, "vol", "c1", 1).expect("cursor table usable after upgrade");
        assert_eq!(
            load_event_cursor(&connection, "vol")
                .expect("load")
                .expect("cursor")
                .last_event_id,
            "c1"
        );
        let has_index: bool = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_file_index_proton_id'",
                [],
                |row| Ok(row.get::<_, i64>(0)? > 0),
            )
            .expect("index query");
        assert!(
            has_index,
            "the proton_id lookup index must be created on upgrade"
        );
    }

    #[test]
    fn path_keys_normalize_trailing_separators() {
        assert_eq!(path_key(Path::new("dir/subdir")), "dir/subdir");
        assert_eq!(path_key(Path::new("dir/subdir/")), "dir/subdir");
    }

    #[test]
    fn load_existing_index_returns_empty_for_missing_database() {
        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("missing.db");

        let index = load_existing_index(&db_path).expect("load missing index");

        assert!(
            index.is_empty(),
            "missing index should dry-run as bootstrap"
        );
        assert!(
            !db_path.exists(),
            "read-only dry-run index loading must not create a database file"
        );
    }

    #[test]
    fn local_file_state_uses_relative_paths() {
        let directory = tempdir().expect("tempdir");
        let nested_dir = directory.path().join("nested");
        fs::create_dir(&nested_dir).expect("nested dir");
        let file_path = nested_dir.join("file.txt");
        let mut file = File::create(&file_path).expect("file");
        writeln!(file, "hello").expect("write");

        let state = local_file_state(directory.path(), &file_path).expect("state");

        assert_eq!(state.relative_path, PathBuf::from("nested/file.txt"));
    }

    fn known_file_record(path: &str, size: u64, mtime: i64, sha1: &str) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::File,
            file_size: size,
            mtime,
            sha1_hash: Some(sha1.to_owned()),
            proton_id: None,
            sync_status: SyncStatus::Synced,
        }
    }

    #[test]
    fn scan_reuses_stored_hash_when_size_and_mtime_match() {
        let directory = tempdir().expect("tempdir");
        let file_path = directory.path().join("stable.txt");
        fs::write(&file_path, b"content").expect("file");
        let actual = local_file_state(directory.path(), &file_path).expect("state");

        // A base record with the same size + mtime but a sentinel hash: the quick-check
        // must reuse the sentinel instead of re-streaming the file through SHA-1.
        let mut known = HashMap::new();
        known.insert(
            PathBuf::from("stable.txt"),
            known_file_record(
                "stable.txt",
                actual.file_size,
                actual.mtime,
                "reused-sentinel",
            ),
        );
        let options = ScanOptions::new(directory.path(), &[], &[], &[]).expect("options");
        let entities =
            scan_local_entities_reusing_hashes(directory.path(), &options, &known).expect("scan");

        match entities.get(Path::new("stable.txt")) {
            Some(LocalEntityState::File(file)) => assert_eq!(
                file.sha1_hash, "reused-sentinel",
                "an unchanged file must reuse its recorded hash, not re-hash"
            ),
            other => panic!("expected a file entity, got {other:?}"),
        }
    }

    #[test]
    fn scan_recomputes_hash_when_mtime_differs() {
        let directory = tempdir().expect("tempdir");
        let file_path = directory.path().join("edited.txt");
        fs::write(&file_path, b"content").expect("file");
        let actual = local_file_state(directory.path(), &file_path).expect("state");

        // Same size but a stale mtime must force a real re-hash, not reuse the stale one.
        let mut known = HashMap::new();
        known.insert(
            PathBuf::from("edited.txt"),
            known_file_record(
                "edited.txt",
                actual.file_size,
                actual.mtime - 1,
                "stale-sentinel",
            ),
        );
        let options = ScanOptions::new(directory.path(), &[], &[], &[]).expect("options");
        let entities =
            scan_local_entities_reusing_hashes(directory.path(), &options, &known).expect("scan");

        match entities.get(Path::new("edited.txt")) {
            Some(LocalEntityState::File(file)) => assert_eq!(
                file.sha1_hash, actual.sha1_hash,
                "a size/mtime mismatch must recompute the real hash, not reuse the stale one"
            ),
            other => panic!("expected a file entity, got {other:?}"),
        }
    }

    /// Inserts a row keyed the way pre-BLOB builds did: a Rust `String` bind, stored
    /// under SQLite's TEXT storage class.
    fn insert_legacy_text_row(connection: &Connection, relative_path: &str, sha1: &str, id: &str) {
        connection
            .execute(
                "INSERT INTO file_index \
                 (file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status) \
                 VALUES (?1, 'file', 3, 7, ?2, ?3, 'synced')",
                params![path_key(Path::new(relative_path)), sha1, id],
            )
            .expect("insert legacy text row");
    }

    fn key_storage_class(connection: &Connection) -> String {
        connection
            .query_row(
                "SELECT typeof(file_path) FROM file_index LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("typeof(file_path)")
    }

    #[test]
    fn migration_normalizes_legacy_text_keys_so_point_queries_match() {
        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("sync_index.db");
        {
            let connection = Connection::open(&db_path).expect("open");
            connection.execute_batch(SCHEMA).expect("schema");
            insert_legacy_text_row(&connection, "dir/notes.txt", "hash", "pid");
            assert_eq!(
                key_storage_class(&connection),
                "text",
                "pre-upgrade builds stored the key as TEXT"
            );
        }

        // Reopen through the real daemon entry point, which runs the migration.
        let connection = open_database(&db_path).expect("open database");
        assert_eq!(
            key_storage_class(&connection),
            "blob",
            "the migration must normalize legacy TEXT keys to BLOB storage"
        );

        let record = get_record(&connection, Path::new("dir/notes.txt"))
            .expect("get_record")
            .expect("legacy row must be found via a BLOB point query after migration");
        assert_eq!(record.sha1_hash.as_deref(), Some("hash"));
        assert_eq!(record.proton_id.as_deref(), Some("pid"));

        // upsert must update in place, not insert a duplicate BLOB row.
        let mut updated = record.clone();
        updated.sha1_hash = Some("hash2".to_owned());
        updated.sync_status = SyncStatus::Modified;
        upsert_record(&connection, &updated).expect("upsert");
        let count: i64 = connection
            .query_row("SELECT count(*) FROM file_index", [], |row| row.get(0))
            .expect("count");
        assert_eq!(
            count, 1,
            "upsert against a migrated key must update in place"
        );
        assert_eq!(
            get_record(&connection, Path::new("dir/notes.txt"))
                .expect("get_record")
                .expect("record")
                .sha1_hash
                .as_deref(),
            Some("hash2")
        );

        // purge must remove the row.
        purge_record(&connection, Path::new("dir/notes.txt")).expect("purge");
        assert!(
            get_record(&connection, Path::new("dir/notes.txt"))
                .expect("get_record")
                .is_none(),
            "purge must delete the migrated row"
        );
    }

    #[test]
    fn migration_dedups_text_twin_when_a_newer_blob_row_exists() {
        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("sync_index.db");
        {
            let connection = Connection::open(&db_path).expect("open");
            connection.execute_batch(SCHEMA).expect("schema");
            // A stale TEXT row plus the newer BLOB duplicate an upgraded build wrote for
            // the same logical path.
            insert_legacy_text_row(&connection, "a.txt", "old", "pid-old");
            connection
                .execute(
                    "INSERT INTO file_index \
                     (file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status) \
                     VALUES (?1, 'file', 3, 7, 'new', 'pid-new', 'synced')",
                    params![index_key(Path::new("a.txt"))],
                )
                .expect("insert newer blob row");
            let count: i64 = connection
                .query_row("SELECT count(*) FROM file_index", [], |row| row.get(0))
                .expect("count");
            assert_eq!(count, 2, "both a TEXT and a BLOB row exist pre-migration");
        }

        let connection = open_database(&db_path).expect("open database");
        let count: i64 = connection
            .query_row("SELECT count(*) FROM file_index", [], |row| row.get(0))
            .expect("count");
        assert_eq!(
            count, 1,
            "the stale TEXT twin must be dropped, keeping the newer BLOB row"
        );
        let record = get_record(&connection, Path::new("a.txt"))
            .expect("get_record")
            .expect("surviving row");
        assert_eq!(
            record.sha1_hash.as_deref(),
            Some("new"),
            "the newer BLOB row must win the dedup"
        );
        assert_eq!(record.proton_id.as_deref(), Some("pid-new"));
    }

    #[test]
    fn legacy_schema_without_entity_kind_is_migrated_in_place() {
        // Exercises the actual rebuild path in migrate_file_index_schema (a pre-entity_kind
        // table with a NOT NULL sha1_hash), which the other migration tests skip because
        // they start from the current schema. Also confirms the now-transactional rebuild
        // preserves rows and drops the temporary table.
        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("sync_index.db");
        {
            let connection = Connection::open(&db_path).expect("open");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE file_index (
                        file_path TEXT PRIMARY KEY,
                        file_size INTEGER NOT NULL,
                        mtime INTEGER NOT NULL,
                        sha1_hash TEXT NOT NULL,
                        proton_id TEXT,
                        sync_status TEXT NOT NULL
                    );
                    "#,
                )
                .expect("legacy schema");
            connection
                .execute(
                    "INSERT INTO file_index \
                     (file_path, file_size, mtime, sha1_hash, proton_id, sync_status) \
                     VALUES (?1, 5, 9, 'hash', 'pid', 'synced')",
                    params![path_key(Path::new("notes.txt"))],
                )
                .expect("legacy row");
        }

        // Reopen through the real entry point, which migrates the legacy table.
        let connection = open_database(&db_path).expect("open database");

        let columns = table_columns(&connection, "file_index").expect("columns");
        assert!(
            columns.iter().any(|column| column == "entity_kind"),
            "the entity_kind column must be added by migration: {columns:?}"
        );
        assert!(
            table_columns(&connection, "file_index_old")
                .expect("old table query")
                .is_empty(),
            "the temporary migration table must be dropped"
        );

        let record = get_record(&connection, Path::new("notes.txt"))
            .expect("get_record")
            .expect("the row must survive migration");
        assert_eq!(record.entity_kind, EntityKind::File);
        assert_eq!(record.file_size, 5);
        assert_eq!(record.mtime, 9);
        assert_eq!(record.sha1_hash.as_deref(), Some("hash"));
        assert_eq!(record.proton_id.as_deref(), Some("pid"));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }
}
