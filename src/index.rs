use crate::sync::{ConflictNaming, SyncAction, UnsyncableItem, UnsyncableReason};
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
CREATE TABLE IF NOT EXISTS warm_start_state (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    warm_starts_since_full_walk INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS unsyncable_items (
    path BLOB PRIMARY KEY,
    entity_kind TEXT NOT NULL,
    reason TEXT NOT NULL,
    first_seen INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_passes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    outcome TEXT NOT NULL,
    changed INTEGER NOT NULL,
    failed INTEGER NOT NULL,
    bytes_up INTEGER NOT NULL,
    bytes_down INTEGER NOT NULL,
    error TEXT
);
CREATE INDEX IF NOT EXISTS idx_sync_passes_started_at ON sync_passes(started_at);
CREATE TABLE IF NOT EXISTS sync_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pass_id INTEGER NOT NULL,
    path BLOB NOT NULL,
    source_path BLOB,
    action TEXT NOT NULL,
    bytes INTEGER,
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sync_events_at ON sync_events(at);
CREATE INDEX IF NOT EXISTS idx_sync_events_path ON sync_events(path);
CREATE TABLE IF NOT EXISTS withheld_deletions (
    path BLOB NOT NULL,
    direction TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    PRIMARY KEY (path, direction)
);
CREATE TABLE IF NOT EXISTS agreed_line_summaries (
    path BLOB NOT NULL,
    agreed_digest TEXT NOT NULL,
    line_digests TEXT NOT NULL,
    recorded_at INTEGER NOT NULL,
    PRIMARY KEY (path, agreed_digest)
);
CREATE INDEX IF NOT EXISTS idx_agreed_line_summaries_at ON agreed_line_summaries(recorded_at);
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

/// One entry the local walk **dropped because the engine cannot sync it** — a socket, a symlink, a
/// FIFO, a device node (#232). The local half of [`crate::sync::UnsyncableItem`], minus the
/// first-seen stamp, which belongs to whoever merges these into the standing list
/// (`daemon::record_unsyncable`); the scan itself observes only what is there now.
///
/// This is deliberately **not** every entry the walk skipped. A path an exclude glob, an include
/// filter, `.proton-sync.toml`, a conflict sidecar or the `.sync` state directory hides is
/// *excluded*, not unsyncable — the user's own rules are the other group on the same dialog, and
/// conflating the two would file a rule the user wrote under "cannot be synced at all".
///
/// Serializable because a one-shot report carries it too (#315: `sync::DryRunReport::cannot_sync`).
/// It is the *observation*, so it has no first-seen stamp to serialize and deliberately no
/// `entity_kind` either — the reason is what says what the entry really is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsyncableEntry {
    /// Relative to the scan root. Lossy on the wire like every other path this engine publishes
    /// (see [`crate::lossy_path`]) — and a dropped entry is disproportionately likely to be exactly
    /// the kind of name that needs it.
    #[serde(with = "crate::lossy_path")]
    pub relative_path: PathBuf,
    pub reason: UnsyncableReason,
}

/// Everything one local stat-walk observed: the entities it kept, and the entries it dropped as
/// unsyncable. One struct so the two can only ever come from the same walk — a caller that merges
/// the second into a standing list is asserting that the first is a complete view of the tree at
/// the same moment, and two separately-obtained halves could not carry that.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalScan {
    pub entities: HashMap<PathBuf, LocalEntityState>,
    /// Ordered by discovery, which is `read_dir` order — the caller sorts or keys as it needs.
    pub unsyncable: Vec<UnsyncableEntry>,
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    ignored_relative_paths: Vec<PathBuf>,
    include_patterns: GlobSet,
    include_pattern_strings: Vec<String>,
    has_include_patterns: bool,
    exclude_patterns: GlobSet,
    /// How conflict sidecars are spelled, so the scanner ignores exactly what the planner writes.
    /// Carried here rather than read from a constant because `conflict_suffix` is configurable —
    /// a scanner using a different suffix from the planner uploads the engine's own sidecars.
    conflict_naming: ConflictNaming,
}

impl ScanOptions {
    /// `naming` must be the one the planner is running with (`DaemonConfig::conflict_naming`);
    /// [`ConflictNaming::default`] is correct only where nothing on disk is being classified —
    /// glob validation, and tests that never write a sidecar.
    pub fn new(
        root: &Path,
        ignored_paths: &[PathBuf],
        include_patterns: &[String],
        exclude_patterns: &[String],
        naming: &ConflictNaming,
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
            conflict_naming: naming.clone(),
        })
    }

    pub fn allows_relative_file(&self, relative_path: &Path) -> bool {
        if should_ignore_relative_path(relative_path, &self.conflict_naming)
            || self.is_configured_ignored(relative_path)
        {
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
            || (!is_never_synced_directory(relative_path)
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
/// Runs on every `initialize_schema` and is idempotent: once no TEXT keys remain, every
/// statement matches nothing (so the separate screen statement re-runs cleanly after a
/// crash between it and the batch). A partially upgraded database can already hold a stale TEXT
/// row and a newer BLOB row for the same logical path (the duplicate an upgraded build
/// wrote); the delete drops the stale TEXT twin first so the `CAST` cannot hit a PRIMARY
/// KEY conflict, keeping the newer BLOB row.
///
/// Lossy legacy keys are dropped, not migrated (issue #75). [`path_key`] built its TEXT
/// keys with `to_string_lossy`, so a non-UTF-8 filename was stored with U+FFFD in place
/// of the offending bytes; `CAST(... AS BLOB)` preserves those replacement bytes, and the
/// result can never equal `index_key(actual_path)`. That row would be a permanent phantom
/// baseline: the planner reads it as locally deleted and plans a spurious `RemoteDelete`
/// of a still-present remote copy. Dropping fails safe instead — a path with no base
/// record falls through to `plan_bootstrap_entity_action`, which re-adopts an
/// already-agreeing pair via `AutoLink` and never deletes. The screen is complete by
/// construction (lossy conversion can only insert U+FFFD) and its one false positive — a
/// filename that genuinely contains U+FFFD, whose `CAST` would have been correct — costs
/// a single re-adoption pass. The `typeof` guard keeps BLOB rows whose bytes happen to be
/// U+FFFD: those were written byte-exactly by current code.
fn normalize_legacy_text_keys(connection: &Connection) -> AppResult<()> {
    let dropped = connection.execute(
        "DELETE FROM file_index \
          WHERE typeof(file_path) = 'text' AND instr(file_path, char(65533)) > 0",
        [],
    )?;
    if dropped > 0 {
        tracing::warn!(
            dropped,
            "dropped legacy TEXT index rows whose keys were lossily encoded (non-UTF-8 \
             filenames); their baselines re-derive from ground truth on the next reconcile \
             rather than planning a spurious remote delete"
        );
    }
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

/// Corpus size of the index: how many files it tracks and how many bytes they come to (#207).
///
/// `Eq` + `Copy` so a caller can cheaply tell a recomputed value from the cached one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexTotals {
    pub files: u64,
    pub bytes: u64,
}

/// Reads [`IndexTotals`] with one aggregate query.
///
/// **`entity_kind = 'file'` is load-bearing, not a tidy-up.** `file_index` stores directories as
/// rows too (with `file_size` 0), so a bare `COUNT(*)` reports the corpus as larger than it is —
/// the exact trap an earlier investigation fell into. `SUM` is unaffected by them, but the two must
/// agree on what they are counting or the pair describes no single set.
///
/// Daemon-side on purpose: a legacy `file_index` predating the `entity_kind` column would make this
/// predicate a hard error, and only the daemon runs the migration that adds it (a read-only GUI
/// connection cannot). By the time the daemon queries, the column exists.
pub fn index_totals(connection: &Connection) -> AppResult<IndexTotals> {
    let (files, bytes) = connection.query_row(
        "SELECT COUNT(*), COALESCE(SUM(file_size), 0) FROM file_index WHERE entity_kind = 'file'",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    Ok(IndexTotals {
        files: files.max(0) as u64,
        bytes: bytes.max(0) as u64,
    })
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
    // NOTE (issue #71(c), deliberately NOT implemented). The issue proposes a "durable cure":
    // when a row takes over a `proton_id`, clear that id from any other row in the same
    // transaction so the composed id is always unique. That is a REGRESSION, not a fix.
    //
    // A remote rename+edit plans `Download(b.txt)` + `LocalDelete(a.txt)`; with the local
    // delete-approval guard on (the default) the delete is WITHHELD while the download commits a
    // new `b.txt` row carrying the same `volumeId~nodeId` as the surviving `a.txt` row — a
    // transient duplicate. If this upsert cleared `a.txt`'s id, the next pass (the move event is
    // still in the delta because the held cursor did not advance) would find NO duplicate, so
    // `reconstruct_remote` would COMPLETE instead of falling back: it seeds `a.txt` as
    // remote-present but, with its id gone, the replayed rename resolves only `b.txt`, leaving
    // `a.txt` as a PHANTOM remote entry. The planner then reads `a.txt` as present on both sides,
    // drops the withheld `LocalDelete`, and the cursor advances past the move — violating the
    // "a withheld LocalDelete holds the event cursor" invariant and losing a real deletion.
    //
    // The tension is fundamental at this layer: reconstruction completing requires no duplicate,
    // but correctly removing `a.txt` from the reconstructed map requires `a.txt` to stay linked to
    // the node's uid — which IS the duplicate. The harm is already prevented WITHOUT clearing the
    // id: `path_for_proton_id` (ambiguous id → `None`) and `reconstruct_remote` seeding (duplicate
    // id → `FallbackToSnapshot`) make every incremental pass fall back to a full snapshot while the
    // duplicate exists, which lists the real remote and re-derives the withheld `LocalDelete`. The
    // only cost is that fallback until the user resolves the approval — correctness-preserving and
    // self-healing. Regression guard:
    // `daemon::tests::a_rename_edit_duplicate_proton_id_does_not_drop_the_withheld_local_delete`.
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

/// Loads the **one** stored cursor, or `None` when there is no row or more than one.
///
/// The caller uses the row's `scope_id` as the volume id when nothing else names it (an index with
/// no composed `proton_id` — a brand-new sync, or an all-Proton-native remote). That inference is
/// only sound while the row is unambiguous: one database per sync root, one volume per root, so a
/// single row *is* this root's volume. Two rows (a future multi-scope world, or the account-wide
/// `"core"` stream alongside a volume) name no single volume, and `None` keeps the caller on the
/// safe full-tree walk. `"core"` is excluded outright — it is an account stream, never a volume.
pub fn load_sole_event_cursor(connection: &Connection) -> AppResult<Option<EventCursor>> {
    let mut statement = connection.prepare(
        "SELECT scope_id, last_event_id, updated_at FROM remote_event_cursor
         WHERE scope_id <> 'core' LIMIT 2",
    )?;
    let mut cursors = statement
        .query_map([], |row| {
            Ok(EventCursor {
                scope_id: row.get(0)?,
                last_event_id: row.get(1)?,
                updated_at: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if cursors.len() != 1 {
        return Ok(None);
    }
    Ok(cursors.pop())
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

/// Drops every row the daemon derives sync decisions from, in one transaction: the baseline
/// (`file_index`), the event cursors, the persisted warm-start counter, the standing delete
/// approvals, and the display-only unsyncable list.
///
/// This is `proton-sync reset-index` (G23/#237's *Reset the index*), and it **truncates rather
/// than removes the database file**: the file is open in a running daemon, and unlinking it under
/// a live connection leaves that daemon writing to an orphaned inode. Truncating in-process, from
/// the main loop between passes, keeps every invariant instead — an empty baseline *is* the
/// bootstrap condition, and a cleared cursor makes the next pass a full walk, not a warm start.
///
/// Not destructive to user data: with an empty index the next pass plans a bootstrap, which
/// `AutoLink`s an already-agreeing local/remote pair rather than re-transferring it. The approvals
/// go with it because each is pinned to a `fingerprint` derived from the baseline this erases —
/// keeping them would leave standing consent for a deletion nothing can still describe.
/// How many agreed-line summaries the index keeps (#217).
///
/// A standing per-file cost, so it is bounded like [`HistoryRetention`] rather than left to grow.
/// At the [`crate::ancestor::MAX_SUMMARY_LINES`] cap a row is ~64 KiB of hex, but the ordinary
/// source file is a few hundred lines and a few KiB — so this is single-digit megabytes for a
/// tree of ordinary text files, and the cap is what stops one pathological repository from
/// deciding the number.
///
/// Eviction is by **age**, oldest first, and a missing summary is the say-less case. That is the
/// property that makes any bound defensible here: nothing breaks when a row goes, the card simply
/// draws one line fewer.
pub const MAX_AGREED_SUMMARIES: usize = 20_000;

/// Record the agreed version's line summary for `path`, keyed by the digest it was agreed at.
///
/// **Keyed by `(path, agreed_digest)`, and that is what gives "dropped when the conflict resolves"
/// for free** (#217): resolving syncs new content under a new digest, so the old row is superseded
/// and ages out. There is no resolve hook to forget to call, and no window in which a stale summary
/// answers for a version that is no longer the ancestor.
///
/// `INSERT OR REPLACE` rather than `INSERT OR IGNORE`: re-agreeing on the same digest is the same
/// content, so the summary is identical and the write only refreshes `recorded_at`, which is what
/// keeps a file that is synced often from ageing out ahead of one nobody touches.
pub fn store_agreed_summary(
    connection: &Connection,
    path: &Path,
    agreed_digest: &str,
    summary: &crate::ancestor::LineSummary,
    recorded_at_epoch_secs: u64,
) -> AppResult<()> {
    connection.execute(
        "INSERT OR REPLACE INTO agreed_line_summaries \
         (path, agreed_digest, line_digests, recorded_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            // `index_key`, not `path_key`: byte-exact, so two paths differing only in invalid
            // UTF-8 cannot share a row — the same rule every other per-path table here follows.
            index_key(path),
            agreed_digest,
            summary.encode(),
            i64::try_from(recorded_at_epoch_secs).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(())
}

/// The agreed version's summary for `path` at `agreed_digest`, or `None`.
///
/// `None` is ordinary and has many causes — never summarised (binary, or past the caps), evicted,
/// or the baseline has moved since. Every one of them means the same thing to the caller: say less.
pub fn load_agreed_summary(
    connection: &Connection,
    path: &Path,
    agreed_digest: &str,
) -> AppResult<Option<crate::ancestor::LineSummary>> {
    let mut statement = connection.prepare(
        "SELECT line_digests FROM agreed_line_summaries WHERE path = ?1 AND agreed_digest = ?2",
    )?;
    let mut rows = statement.query(params![index_key(path), agreed_digest])?;
    match rows.next()? {
        Some(row) => {
            let stored: String = row.get(0)?;
            Ok(Some(crate::ancestor::LineSummary::decode(&stored)))
        }
        None => Ok(None),
    }
}

/// The **most recent** agreed summary for `path`, whatever digest it was agreed at.
///
/// This is the read the conflict card needs, and it cannot key on the index row's digest: by the
/// time a conflict exists, `SyncAction::Conflict` has already upserted `FileRecord::from_local`, so
/// the row carries the *diverged local* file's hash. The newest row for a path is the last version
/// the two sides agreed on, because [`store_agreed_summary`] is only ever called for a `Synced`
/// record — the digest key is there to supersede, not to look up by.
pub fn newest_agreed_summary(
    connection: &Connection,
    path: &Path,
) -> AppResult<Option<crate::ancestor::LineSummary>> {
    let mut statement = connection.prepare(
        "SELECT line_digests FROM agreed_line_summaries WHERE path = ?1 \
         ORDER BY recorded_at DESC, rowid DESC LIMIT 1",
    )?;
    let mut rows = statement.query(params![index_key(path)])?;
    match rows.next()? {
        Some(row) => {
            let stored: String = row.get(0)?;
            Ok(Some(crate::ancestor::LineSummary::decode(&stored)))
        }
        None => Ok(None),
    }
}

/// Drop the oldest summaries past [`MAX_AGREED_SUMMARIES`].
///
/// Called from the same place the history prune is, so a pass that wrote nothing pays nothing.
pub fn prune_agreed_summaries(connection: &Connection) -> AppResult<()> {
    connection.execute(
        "DELETE FROM agreed_line_summaries WHERE rowid NOT IN \
         (SELECT rowid FROM agreed_line_summaries ORDER BY recorded_at DESC, rowid DESC LIMIT ?1)",
        params![MAX_AGREED_SUMMARIES as i64],
    )?;
    Ok(())
}

pub fn reset_index_state(connection: &Connection) -> AppResult<()> {
    // A guarded transaction, not a `BEGIN; …; COMMIT;` batch. `execute_batch` returns on the first
    // failing statement with the `BEGIN` still open — SQLite does not implicitly roll back — and
    // this runs on the DAEMON'S LIVE CONNECTION mid-life, where the failure does not stop the
    // process: `reconcile_blocking` records the error and the loop carries on, so every later
    // `commit_checkpoint` would then fail with "cannot start a transaction within a transaction"
    // until a restart. The guard rolls back on drop, leaving the connection usable and the state
    // whole. `unchecked_transaction` because the caller holds `&Connection`, matching
    // `replace_unsyncable_items`.
    let transaction = connection.unchecked_transaction()?;
    for table in [
        "file_index",
        "remote_event_cursor",
        "warm_start_state",
        "delete_approvals",
        "unsyncable_items",
        // The ancestor summaries are things the daemon has LEARNED about content it agreed on, so
        // a reset that kept them would leave a conflict card answering from a baseline the reset
        // discarded (#217).
        "agreed_line_summaries",
    ] {
        transaction.execute(&format!("DELETE FROM {table}"), [])?;
    }
    transaction.commit()?;
    Ok(())
}

/// Loads the persisted "warm starts since the last full walk" counter, or `0` if none is recorded.
///
/// Distinct from the daemon's in-memory `incremental_passes_since_full_scan`: that one counts
/// event-driven passes *within a single run* to drive the opt-in periodic in-run resync; this one
/// persists **across process restarts** so the warm-start path (event-driven reconcile on the first
/// pass after boot) can force a self-healing full walk every N warm starts. A single-row table
/// (`id = 0`) keyed to this database — one database per sync root, one daemon per root.
pub fn load_warm_start_count(connection: &Connection) -> AppResult<u64> {
    let count: Option<i64> = connection
        .query_row(
            "SELECT warm_starts_since_full_walk FROM warm_start_state WHERE id = 0",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(count.unwrap_or(0).max(0) as u64)
}

/// Records the "warm starts since the last full walk" counter (see [`load_warm_start_count`]).
pub fn store_warm_start_count(connection: &Connection, count: u64) -> AppResult<()> {
    // Saturate rather than `as`-cast into the signed column: a warm-start count is a small integer
    // in practice, but a defensive saturate ensures a pathologically large value can never wrap to
    // a negative (which `load_warm_start_count` would then clamp back to 0).
    let stored = i64::try_from(count).unwrap_or(i64::MAX);
    connection.execute(
        r#"
        INSERT INTO warm_start_state (id, warm_starts_since_full_walk)
        VALUES (0, ?1)
        ON CONFLICT(id) DO UPDATE SET warm_starts_since_full_walk = excluded.warm_starts_since_full_walk
        "#,
        params![stored],
    )?;
    Ok(())
}

/// Every entity the daemon currently holds as unsyncable, **ordered by path**.
///
/// The list is maintained by `daemon::record_unsyncable`, and a full-tree walk is not its only
/// writer: any pass may add a skip it planned or drop a path it planned a real action for, while
/// only a full walk may prune by absence. Each row keeps the epoch it was *first* seen at rather
/// than the epoch of the pass that last re-derived it, which is what makes "stuck since March"
/// answerable. Path order is deliberate — it is stable across passes and independent of when a row
/// joined, and it is the order `proton-sync status` renders within each cause.
///
/// Persisted because a skipped entity is never recorded in `file_index`: it is absent from the
/// baseline `reconstruct::reconstruct_remote` overlays, so an incremental pass simply does not plan
/// it and cannot re-derive it. With event-driven detection on by default and a warm start on boot,
/// a purely in-memory list would be empty for most of a daemon's life and every restart would
/// re-hide exactly what #295 is about.
///
/// Keyed on the byte-exact [`index_key`] BLOB encoding, not TEXT: the `unrepresentable_path`
/// reason is *precisely* the non-UTF-8 paths a lossy TEXT key mangles (see
/// [`normalize_legacy_text_keys`] for what that cost `file_index`).
pub fn load_unsyncable_items(connection: &Connection) -> AppResult<Vec<UnsyncableItem>> {
    let mut statement = connection.prepare(
        "SELECT path, entity_kind, reason, first_seen FROM unsyncable_items ORDER BY path",
    )?;
    let items = statement
        .query_map([], |row| {
            let entity_kind: String = row.get(1)?;
            let reason: String = row.get(2)?;
            Ok(UnsyncableItem {
                path: read_index_key_column(row, 0)?,
                entity_kind: entity_kind.parse().map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, err)
                })?,
                reason: UnsyncableReason::from_token(&reason),
                first_seen_epoch_secs: row.get::<_, i64>(3)?.max(0) as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

/// Replaces the stored unsyncable list wholesale (see [`load_unsyncable_items`]). One transaction,
/// so a crash mid-write cannot leave a half list; display-only data, so it is written outside the
/// reconcile's checkpoint transactions and records no side effect.
pub fn replace_unsyncable_items(
    connection: &Connection,
    items: &[UnsyncableItem],
) -> AppResult<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM unsyncable_items", [])?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO unsyncable_items (path, entity_kind, reason, first_seen) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for item in items {
            statement.execute(params![
                index_key(&item.path),
                item.entity_kind.as_str(),
                item.reason.as_str(),
                i64::try_from(item.first_seen_epoch_secs).unwrap_or(i64::MAX)
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Pass and path history (#213 / #229 / #238 / #230 / #190 / #191).
//
// ONE schema, six queries. `sync_passes` is the retained **rollup** (one row per notable pass);
// `sync_events` is the expiring **detail** (one row per side effect that landed). Every number a
// caller can read comes from exactly one of them:
//
//   * per-pass duration / kind / outcome  -> `recent_passes`      (#229, #238)
//   * when the last full sweep ran        -> `last_full_sweep`    (#238)
//   * byte totals per direction, windowed -> `byte_totals_since`  (#191) — the ROLLUP, always
//   * what moved and when (global / path) -> `file_events`        (#230, #190)
//
// `sync_events.bytes` is per-row display data and is NEVER summed into a total: totals come from
// the rollup, which outlives the detail (see [`HistoryRetention`]). Two sources for one number is
// how they drift.
// ---------------------------------------------------------------------------------------------

/// Which remote-discovery strategy a pass actually ran — the fact `Full sweep now` needs in order
/// to say when the last full sweep was (#238), and which the daemon already branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    /// A full-tree remote walk (`daemon::bootstrap_reconcile`) — O(folders).
    FullSweep,
    /// The first pass after boot, replaying the event cursor instead of walking (ADR 0004).
    WarmStart,
    /// Steady-state event-driven pass — O(changes).
    Incremental,
}

impl PassKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullSweep => "full-sweep",
            Self::WarmStart => "warm-start",
            Self::Incremental => "incremental",
        }
    }
}

/// How a recorded pass ended. Four states, each enumerated everywhere it is read: a trailing arm
/// absorbing an undrawn one is how a failed pass came to render as "everything is up to date"
/// (#246). Distinct from [`crate::daemon::PassOutcome`], which is the in-process return value and
/// has no way to say "failed" (that is its `Err`) or "never finished".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOutcomeKind {
    /// Every planned action landed.
    Clean,
    /// The plan ran to the end; some items' side effects failed (#136).
    Partial,
    /// The pass itself could not proceed (a scan, a listing, a commit, a shutdown mid-plan).
    Failed,
    /// Written at the pass's first committed checkpoint and never updated — the process died
    /// mid-pass. Not reachable from any graceful path; see [`begin_pass`].
    Interrupted,
}

impl PassOutcomeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

/// One recorded pass. `kind` and `outcome` ride as open string tokens rather than enums for the
/// same reason [`crate::ipc::SyncActivity::phase`] does: a client renders an unrecognized token
/// verbatim instead of failing to parse the whole reply, so a future variant needs no lockstep
/// upgrade. The tokens are [`PassKind::as_str`] and [`PassOutcomeKind::as_str`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassRecord {
    pub id: i64,
    pub started_epoch_secs: u64,
    /// Measured, not `finished - started`: most passes are sub-second and the `6a Activity passes`
    /// chart draws them as bar heights.
    pub duration_ms: u64,
    pub kind: String,
    pub outcome: String,
    /// Side-effecting actions that **landed** (committed events). Distinct from
    /// [`crate::ipc::PassProgress::changes`], which is what the in-flight pass *planned*.
    pub changed: usize,
    /// Items whose action failed (#136); `0` unless `outcome` is `partial`.
    pub failed: usize,
    pub bytes_uploaded: u64,
    pub bytes_downloaded: u64,
    #[serde(default)]
    pub error: Option<String>,
}

/// One side effect that landed. The unit behind `Last things to move` (#230) and `This file's
/// history` (#190).
///
/// `epoch_secs` is stamped when the **side effect completed** (`daemon::PassLog::note`), not when
/// the row was committed — the checkpoint transaction follows, and one checkpoint can carry several
/// actions, so rows sharing a commit still carry distinct times. Worth stating because
/// commit-after-side-effects is a load-bearing invariant here (ADR 0003): a reader who took `at`
/// for the commit boundary would infer an ordering guarantee it does not carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEvent {
    /// Where the entity IS after the action — a move records its **destination**, so a per-path
    /// lookup on the file's current path finds the move that brought it there.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    /// Where a move came from; `None` for every other action.
    #[serde(default, with = "crate::lossy_path::optional")]
    pub source_path: Option<PathBuf>,
    pub action: SyncAction,
    /// Bytes this one action moved, when the action moves bytes and the size is known.
    /// Per-row display data only — never summed (see the module note above).
    #[serde(default)]
    pub bytes: Option<u64>,
    pub epoch_secs: u64,
    /// The [`PassRecord::id`] this event belongs to. A correlation key, **not** a foreign key: the
    /// two tables have independent retention, so a very old event may outlive its pass row.
    pub pass_id: i64,
}

/// Bytes moved per direction over a window, from the pass rollup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteTotals {
    /// Start of the window; `0` means "everything retained". A pass is counted in the window it
    /// *started* in.
    pub since_epoch_secs: u64,
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
}

/// A slice of the per-file feed, plus the counts a caller renders around it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHistory {
    /// Newest first, capped at the requested limit.
    pub events: Vec<FileEvent>,
    /// Matching events in the window, past the cap.
    pub total: usize,
    /// Distinct paths in the window — the `7 files in the last 3 days` count (#230).
    pub files: usize,
    /// Bytes moved over the same window, from the rollup (never from `events`).
    ///
    /// `None` for a **path-filtered** query: the rollup is per pass and holds no paths, so the
    /// only total it can offer there is the window's whole traffic — a number that reads as the
    /// file's own and is not. Summing the event rows instead would make this the second place
    /// computing a byte total, which is the thing the module note above forbids.
    #[serde(default)]
    pub totals: Option<ByteTotals>,
}

/// What the history tables keep. An unbounded per-action log is a defect, not a feature: the
/// events-driven daemon polls every 30s, so a live account runs ~2900 passes/day, and one
/// bootstrap of a 5000-file account writes 5000 event rows.
///
/// Two bounds per table (age and rows) because either alone fails: an age bound lets one bootstrap
/// dominate, a row bound alone lets a dormant daemon keep rows forever. Passes are kept longer and
/// wider than events on purpose — the rollup is what answers "2 days ago" and the byte totals after
/// the detail has aged out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryRetention {
    pub max_passes: usize,
    pub max_pass_age_secs: u64,
    pub max_events: usize,
    pub max_event_age_secs: u64,
}

impl Default for HistoryRetention {
    fn default() -> Self {
        Self {
            // ~2000 notable passes and a year: months of a normal account's history, and still
            // bounded under a failure storm (every failed pass is notable).
            max_passes: 2_000,
            max_pass_age_secs: 365 * 24 * 60 * 60,
            // ~20k rows at roughly 120 bytes is a couple of MB, and survives four full bootstraps
            // of a 5000-file account.
            max_events: 20_000,
            max_event_age_secs: 90 * 24 * 60 * 60,
        }
    }
}

/// Opens a pass row at the pass's first committed checkpoint and returns its id.
///
/// Written with `outcome = interrupted` and a zero duration deliberately: those are the true
/// values for a pass that has started and not finished, so a process killed mid-pass leaves an
/// honest row and needs no startup repair sweep. [`finish_pass`] overwrites them on every
/// graceful ending, including a failure — so `interrupted` is only ever reachable by a crash.
///
/// Called from inside the checkpoint transaction that carries the events it will own, so a
/// history row never precedes the side effect it describes (ADR 0003).
pub fn begin_pass(connection: &Connection, started_at: u64, kind: PassKind) -> AppResult<i64> {
    connection.execute(
        "INSERT INTO sync_passes \
         (started_at, duration_ms, kind, outcome, changed, failed, bytes_up, bytes_down, error) \
         VALUES (?1, 0, ?2, ?3, 0, 0, 0, 0, NULL)",
        params![
            i64::try_from(started_at).unwrap_or(i64::MAX),
            kind.as_str(),
            PassOutcomeKind::Interrupted.as_str()
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

/// Seals the pass row opened by [`begin_pass`] with what the pass turned out to be.
#[allow(clippy::too_many_arguments)]
pub fn finish_pass(
    connection: &Connection,
    id: i64,
    duration_ms: u64,
    outcome: PassOutcomeKind,
    changed: usize,
    failed: usize,
    bytes_uploaded: u64,
    bytes_downloaded: u64,
    error: Option<&str>,
) -> AppResult<()> {
    connection.execute(
        "UPDATE sync_passes SET duration_ms = ?2, outcome = ?3, changed = ?4, failed = ?5, \
         bytes_up = ?6, bytes_down = ?7, error = ?8 WHERE id = ?1",
        params![
            id,
            i64::try_from(duration_ms).unwrap_or(i64::MAX),
            outcome.as_str(),
            i64::try_from(changed).unwrap_or(i64::MAX),
            i64::try_from(failed).unwrap_or(i64::MAX),
            i64::try_from(bytes_uploaded).unwrap_or(i64::MAX),
            i64::try_from(bytes_downloaded).unwrap_or(i64::MAX),
            error
        ],
    )?;
    Ok(())
}

/// Appends the events a checkpoint is committing. Same transaction as the index mutations they
/// describe — see [`crate::daemon::commit_checkpoint`].
pub fn insert_file_events(
    connection: &Connection,
    pass_id: i64,
    events: &[FileEvent],
) -> AppResult<()> {
    if events.is_empty() {
        return Ok(());
    }
    let mut statement = connection.prepare(
        "INSERT INTO sync_events (pass_id, path, source_path, action, bytes, at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for event in events {
        statement.execute(params![
            pass_id,
            index_key(&event.path),
            event.source_path.as_deref().map(index_key),
            event.action.as_str(),
            event
                .bytes
                .map(|bytes| i64::try_from(bytes).unwrap_or(i64::MAX)),
            i64::try_from(event.epoch_secs).unwrap_or(i64::MAX)
        ])?;
    }
    Ok(())
}

/// Applies [`HistoryRetention`] to both tables. Cheap enough to run after every pass that wrote
/// rows (each predicate is an indexed range or a rowid comparison), and never run by a pass that
/// wrote none.
///
/// The newest full sweep is exempt from **both** pass bounds. Without that exemption a chronically
/// failing daemon — every failed pass is notable, so ~2900 rows/day — evicts the full-sweep row
/// within hours, and `Full sweep now` loses its "last one N days ago" exactly when the user is
/// debugging why syncing is broken.
pub fn prune_history(
    connection: &Connection,
    now_epoch_secs: u64,
    retention: HistoryRetention,
) -> AppResult<()> {
    let now = i64::try_from(now_epoch_secs).unwrap_or(i64::MAX);
    // `IS NOT`, not `<>`: on a database with no full sweep yet the subquery is NULL, and `<>` NULL
    // is NULL — which would silently delete nothing at all.
    const KEEP_NEWEST_SWEEP: &str =
        "AND id IS NOT (SELECT MAX(id) FROM sync_passes WHERE kind = 'full-sweep')";
    connection.execute(
        &format!("DELETE FROM sync_passes WHERE started_at < ?1 {KEEP_NEWEST_SWEEP}"),
        params![now.saturating_sub(i64::try_from(retention.max_pass_age_secs).unwrap_or(i64::MAX))],
    )?;
    // "At or below the (N+1)th-newest row", which keeps exactly N. A NULL subquery (fewer than
    // N+1 rows) makes the predicate NULL and deletes nothing, which is exactly right.
    connection.execute(
        &format!(
            "DELETE FROM sync_passes WHERE id <= \
             (SELECT id FROM sync_passes ORDER BY id DESC LIMIT 1 OFFSET ?1) {KEEP_NEWEST_SWEEP}"
        ),
        params![i64::try_from(retention.max_passes).unwrap_or(i64::MAX)],
    )?;
    connection.execute(
        "DELETE FROM sync_events WHERE at < ?1",
        params![
            now.saturating_sub(i64::try_from(retention.max_event_age_secs).unwrap_or(i64::MAX))
        ],
    )?;
    connection.execute(
        "DELETE FROM sync_events WHERE id <= \
         (SELECT id FROM sync_events ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        params![i64::try_from(retention.max_events).unwrap_or(i64::MAX)],
    )?;
    Ok(())
}

/// The most recent passes, newest first (#229).
pub fn recent_passes(connection: &Connection, limit: usize) -> AppResult<Vec<PassRecord>> {
    let mut statement = connection.prepare(&format!("{PASS_COLUMNS} ORDER BY id DESC LIMIT ?1"))?;
    let passes = statement
        .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], read_pass)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(passes)
}

/// The most recent full-tree walk, whenever it ran (#238). A separate query rather than a scan of
/// [`recent_passes`]: with the periodic resync off by default a full sweep happens at boot and
/// almost never again, so it is normally far outside any recent window.
pub fn last_full_sweep(connection: &Connection) -> AppResult<Option<PassRecord>> {
    let mut statement = connection.prepare(&format!(
        "{PASS_COLUMNS} WHERE kind = ?1 ORDER BY id DESC LIMIT 1"
    ))?;
    let record = statement
        .query_row(params![PassKind::FullSweep.as_str()], read_pass)
        .optional()?;
    Ok(record)
}

/// Bytes moved per direction since `since_epoch_secs` (#191), summed over the pass rollup. The
/// single source for a byte total — see the module note.
pub fn byte_totals_since(connection: &Connection, since_epoch_secs: u64) -> AppResult<ByteTotals> {
    let (uploaded, downloaded): (i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(bytes_up), 0), COALESCE(SUM(bytes_down), 0) \
         FROM sync_passes WHERE started_at >= ?1",
        params![i64::try_from(since_epoch_secs).unwrap_or(i64::MAX)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ByteTotals {
        since_epoch_secs,
        uploaded_bytes: uploaded.max(0) as u64,
        downloaded_bytes: downloaded.max(0) as u64,
    })
}

/// The per-file feed, newest first. `path` filters to one entity's own history (#190) — matching
/// its move **destinations** as well, so a file's history survives being moved; `None` is the
/// global recent feed (#230). `since_epoch_secs` of `0` means "everything retained".
pub fn file_events(
    connection: &Connection,
    path: Option<&Path>,
    since_epoch_secs: u64,
    limit: usize,
) -> AppResult<FileHistory> {
    let since = i64::try_from(since_epoch_secs).unwrap_or(i64::MAX);
    let key = path.map(index_key);
    // One predicate, three uses (page, count, distinct paths) — built once so the three numbers
    // can never describe different sets.
    let filter = "at >= ?1 AND (?2 IS NULL OR path = ?2)";
    let mut statement = connection.prepare(&format!(
        "SELECT pass_id, path, source_path, action, bytes, at FROM sync_events \
         WHERE {filter} ORDER BY id DESC LIMIT ?3"
    ))?;
    let events = statement
        .query_map(
            params![since, key, i64::try_from(limit).unwrap_or(i64::MAX)],
            read_file_event,
        )?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let (total, files): (i64, i64) = connection.query_row(
        &format!("SELECT COUNT(*), COUNT(DISTINCT path) FROM sync_events WHERE {filter}"),
        params![since, key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let totals = match path {
        Some(_) => None,
        None => Some(byte_totals_since(connection, since_epoch_secs)?),
    };
    Ok(FileHistory {
        events,
        total: total.max(0) as usize,
        files: files.max(0) as usize,
        totals,
    })
}

/// The most recent side effect at `path` that actually **moved bytes** — the newest event row
/// whose action has a [`SyncAction::transfer_direction`] — or `None` (#233).
///
/// This is the only remote-side timestamp the engine has, and it is a fact about a transfer this
/// daemon performed, never a local file's mtime. `file_index` holds no remote revision time: the
/// CLI listing is parsed for `activeRevision.claimedDigests.sha1` and nothing else, so there is no
/// column to read and none is added — the history log written behind every landed side effect
/// (#308) already answers "when did bytes last cross for this path", which is the question.
///
/// **`None` means the engine has no record of moving this path's bytes**, which is four honest
/// cases and not an error: nothing ever transferred; the last transfer aged out of
/// [`HistoryRetention`] (20k rows / 90 days); the file was adopted rather than transferred
/// (`AutoLink` moves no bytes); or it has been moved since, because an event row keeps the path the
/// action landed at and this query does not chase `source_path` chains backwards.
///
/// The direction on the returned event is load-bearing for anything that renders it: a `Download`
/// (or a conflict sidecar fetch, which records the **file's** path, not the sidecar's) says when
/// *this computer* received bytes, and only an `Upload` says when Proton Drive did.
///
/// Scans newest-first and stops at the first row that HAS a direction, which is not necessarily the
/// first row: `AutoLink`, `Purge`, a delete and both directory creations all land at a path and move
/// no bytes, so a file adopted and then re-adopted several times is read through until the transfer
/// under them. The query is streamed and the statement dropped at the first hit, so nothing past it
/// is read — but "one row" would be wrong, and the difference is the whole reason the direction
/// filter cannot be pushed into the SQL (it is a property of the action, not a column).
pub fn last_transfer(connection: &Connection, path: &Path) -> AppResult<Option<FileEvent>> {
    let mut statement = connection.prepare(
        "SELECT pass_id, path, source_path, action, bytes, at FROM sync_events \
         WHERE path = ?1 ORDER BY id DESC",
    )?;
    let rows = statement.query_map(params![index_key(path)], read_file_event)?;
    for row in rows {
        // A row whose action token this build cannot name is `None` here, and an unknown action
        // has no known direction either — skipping it is the same decision `file_events` makes.
        if let Some(event) = row?
            && event.action.transfer_direction().is_some()
        {
            return Ok(Some(event));
        }
    }
    Ok(None)
}

/// Every distinct path in the event log within a window. Small by construction (the log is bounded
/// by [`HistoryRetention`]), and the only way to reach a **non-UTF-8** path from the wire: those
/// are published as `to_string_lossy`, so a selector a user copies off the screen no longer has the
/// bytes the BLOB key was built from. Callers match the rendering — see
/// `daemon::resolve_history_path`.
pub fn distinct_event_paths(
    connection: &Connection,
    since_epoch_secs: u64,
) -> AppResult<Vec<PathBuf>> {
    let mut statement =
        connection.prepare("SELECT DISTINCT path FROM sync_events WHERE at >= ?1")?;
    let paths = statement
        .query_map(
            params![i64::try_from(since_epoch_secs).unwrap_or(i64::MAX)],
            |row| read_index_key_column(row, 0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(paths)
}

const PASS_COLUMNS: &str = "SELECT id, started_at, duration_ms, kind, outcome, changed, failed, \
                            bytes_up, bytes_down, error FROM sync_passes";

/// Rows this build cannot name are **skipped, not fatal**: an action token written by a newer
/// daemon must not make the whole feed unreadable. Display data — nothing here plans anything.
fn read_file_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<FileEvent>> {
    let token: String = row.get(3)?;
    let Some(action) = SyncAction::from_token(&token) else {
        return Ok(None);
    };
    Ok(Some(FileEvent {
        pass_id: row.get(0)?,
        path: read_index_key_column(row, 1)?,
        source_path: match row.get_ref(2)? {
            rusqlite::types::ValueRef::Null => None,
            _ => Some(read_index_key_column(row, 2)?),
        },
        action,
        bytes: row
            .get::<_, Option<i64>>(4)?
            .map(|bytes| bytes.max(0) as u64),
        epoch_secs: row.get::<_, i64>(5)?.max(0) as u64,
    }))
}

fn read_pass(row: &rusqlite::Row<'_>) -> rusqlite::Result<PassRecord> {
    Ok(PassRecord {
        id: row.get(0)?,
        started_epoch_secs: row.get::<_, i64>(1)?.max(0) as u64,
        duration_ms: row.get::<_, i64>(2)?.max(0) as u64,
        kind: row.get(3)?,
        outcome: row.get(4)?,
        changed: row.get::<_, i64>(5)?.max(0) as usize,
        failed: row.get::<_, i64>(6)?.max(0) as usize,
        bytes_uploaded: row.get::<_, i64>(7)?.max(0) as u64,
        bytes_downloaded: row.get::<_, i64>(8)?.max(0) as u64,
        error: row.get(9)?,
    })
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

/// When a withheld deletion was **first** held, from the `withheld_deletions` table.
///
/// `PendingDeletion::detected_epoch_secs` is the age of the *pass* that re-derived the withheld
/// action, not of the deletion (#225): the gate stamps `now` on every item it withholds, and a pass
/// cannot idle-skip while anything is pending, so a three-day-old deletion reported an age of
/// seconds. This row is the missing fact, and it is persisted for the same reason
/// [`load_unsyncable_items`] is — a queue that survives restarts must have an age that does too.
///
/// Keyed like `delete_approvals` on `(path, direction)` and pinned to the same `fingerprint`: a
/// different fingerprint at the same path is a *different* deletion, so it re-stamps rather than
/// inheriting the previous one's age. `index_key` BLOB keys, byte-exact like every other path
/// column here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithheldDeletion {
    pub path: PathBuf,
    pub direction: crate::sync::DeleteDirection,
    pub fingerprint: String,
    pub first_seen_epoch_secs: u64,
}

/// Every stored withheld-deletion row (unordered — the caller keys them by `(path, direction)`).
pub fn load_withheld_deletions(connection: &Connection) -> AppResult<Vec<WithheldDeletion>> {
    let mut statement = connection
        .prepare("SELECT path, direction, fingerprint, first_seen FROM withheld_deletions")?;
    let items = statement
        .query_map([], |row| {
            let direction: String = row.get(1)?;
            Ok(WithheldDeletion {
                path: read_index_key_column(row, 0)?,
                direction: direction.parse().map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, err)
                })?,
                fingerprint: row.get(2)?,
                first_seen_epoch_secs: row.get::<_, i64>(3)?.max(0) as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

/// Replaces the stored withheld-deletion set wholesale (see [`WithheldDeletion`]). The gate's output
/// **is** the complete set of currently-withheld deletions, so a replace is the honest write and it
/// also clears the table on the first pass that withholds nothing. Carrying `first_seen` forward is
/// the caller's job (it holds the previous rows). One transaction, and display-only data — written
/// outside the reconcile's checkpoint transactions, recording no side effect.
pub fn replace_withheld_deletions(
    connection: &Connection,
    items: &[WithheldDeletion],
) -> AppResult<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM withheld_deletions", [])?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO withheld_deletions (path, direction, fingerprint, first_seen) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for item in items {
            statement.execute(params![
                index_key(&item.path),
                item.direction.as_str(),
                item.fingerprint,
                i64::try_from(item.first_seen_epoch_secs).unwrap_or(i64::MAX)
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// Purges the baseline record at `root` **and every record beneath it**, returning how many rows
/// went. This is how a refused deletion (`keep`) stops being a deletion: with no baseline record the
/// surviving side is no longer "a thing that was deleted", it is a thing the engine has never seen,
/// and the bootstrap arm adopts it back onto the other side.
///
/// A subtree, not one row, because a directory deletion is planned **recursively** — one
/// `LocalDelete`/`RemoteDelete` for the folder with every descendant action suppressed. Purging the
/// folder alone would re-adopt the folder while each surviving child record went on deriving its own
/// withheld delete.
///
/// Component-wise via [`Path::starts_with`], never a byte prefix: `photos/2019x` is not under
/// `photos/2019`.
///
/// Returns the paths it purged, because the caller has to answer for each of them: a purged record
/// leaves any standing approval at that path pointing at nothing the user can see (see
/// `daemon::apply_keep_command`).
///
/// Opens **no transaction of its own** — the caller wraps it, because a refusal also revokes the
/// approvals it replaces and the two must land together.
pub fn purge_subtree_records(connection: &Connection, root: &Path) -> AppResult<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = {
        let mut statement = connection.prepare("SELECT file_path FROM file_index")?;
        let rows = statement
            .query_map([], |row| read_index_key_column(row, 0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .filter(|path| path.starts_with(root))
            .collect()
    };
    for path in &paths {
        purge_record(connection, path)?;
    }
    Ok(paths)
}

/// Unfiltered scan with the **default** sidecar naming. Only for callers that classify nothing —
/// tests and the live smoke test; the daemon always goes through `scan_options_from_config`.
pub fn scan_local_files(root: &Path) -> AppResult<HashMap<PathBuf, LocalFileState>> {
    let options = ScanOptions::new(root, &[], &[], &[], &ConflictNaming::default())?;
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
    scan_local_entities_observed(root, options, known, None)
}

/// Observer invoked once per file the scan visits, just *before* the file is stat'd and
/// (when its cached hash cannot be reused) SHA-1 hashed: `(files_seen_so_far, absolute_path)`.
/// Hashing a large changed file is where a slow scan spends its time, so the most recently
/// reported file is exactly what the scan is working on. Display-only — implementations must
/// be cheap and must not fail.
pub type ScanObserver<'a> = &'a dyn Fn(u64, &Path);

/// Like [`scan_local_entities_reusing_hashes`] with a per-file [`ScanObserver`], so a caller
/// (the daemon) can surface live scan progress while a large tree is hashed.
pub fn scan_local_entities_observed(
    root: &Path,
    options: &ScanOptions,
    known: &HashMap<PathBuf, FileRecord>,
    observer: Option<ScanObserver<'_>>,
) -> AppResult<HashMap<PathBuf, LocalEntityState>> {
    Ok(scan_local_tree(root, options, known, observer)?.entities)
}

/// The full local stat-walk: every entity the engine can sync, **and** every entry it dropped as
/// unsyncable (#232). The other `scan_local_*` functions are this one with the second half thrown
/// away; a caller that reports what cannot be synced wants this one.
///
/// There is no partial form. The walk always starts at `root` and visits the whole tree (minus
/// untraversable subtrees), which is what lets a caller treat the absence of a path from
/// [`LocalScan::unsyncable`] as evidence rather than as a gap.
pub fn scan_local_tree(
    root: &Path,
    options: &ScanOptions,
    known: &HashMap<PathBuf, FileRecord>,
    observer: Option<ScanObserver<'_>>,
) -> AppResult<LocalScan> {
    let context = WalkContext {
        root,
        options,
        known,
        observer,
    };
    let mut scan = LocalScan::default();
    let mut files_seen = 0u64;
    visit_directory(&context, root, &mut scan, &mut files_seen)?;
    Ok(scan)
}

/// The inputs [`visit_directory`] carries unchanged down the recursion, bundled so the recursive
/// call stays under clippy's argument limit and so adding an input is one field, not one parameter
/// at every level.
struct WalkContext<'a> {
    root: &'a Path,
    options: &'a ScanOptions,
    known: &'a HashMap<PathBuf, FileRecord>,
    observer: Option<ScanObserver<'a>>,
}

/// See [`scan_local_files`] for the default-naming caveat.
pub fn scan_local_entities(root: &Path) -> AppResult<HashMap<PathBuf, LocalEntityState>> {
    let options = ScanOptions::new(root, &[], &[], &[], &ConflictNaming::default())?;
    scan_local_entities_with_options(root, &options)
}

fn visit_directory(
    context: &WalkContext<'_>,
    directory: &Path,
    scan: &mut LocalScan,
    files_seen: &mut u64,
) -> AppResult<()> {
    let WalkContext {
        root,
        options,
        known,
        observer,
    } = *context;
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
                    scan.entities.insert(
                        state.relative_path.clone(),
                        LocalEntityState::Directory(state),
                    );
                }
                // A NotFound surfacing here is this child directory's own `read_dir`
                // failing (the directory vanished mid-scan) — deeper vanishes were
                // already skipped inside the recursion.
                vanished_entry_to_skip(visit_directory(context, &path, scan, files_seen))?;
            }
            continue;
        }
        // Everything below this point is a non-directory entry, and the two ways it can be
        // dropped are two different facts about it (#232). The rule test comes FIRST, so a
        // socket the user's own exclude glob already hides is never also reported as
        // unsyncable: "you told it to skip these" and "these can't be synced at all" are the
        // two groups on one dialog, and a path that answers to a rule belongs to the first.
        if !options.allows_relative_file(relative_path) {
            continue;
        }
        // A symlink, socket, FIFO or device node. It was already dropped here before #232 — the
        // only change is that the drop is now *reported* instead of silent.
        if !file_type.is_file() {
            scan.unsyncable.push(UnsyncableEntry {
                relative_path: relative_path.to_path_buf(),
                reason: local_unsyncable_reason(&file_type),
            });
            continue;
        }
        if let Some(observe) = observer {
            *files_seen += 1;
            observe(*files_seen, &path);
        }
        let Some(state) =
            vanished_entry_to_skip(local_file_state_reusing_hash(root, &path, known))?
        else {
            continue;
        };
        scan.entities
            .insert(state.relative_path.clone(), LocalEntityState::File(state));
    }
    Ok(())
}

/// Why the walk cannot sync a local entry that is neither a regular file nor a directory (#232).
///
/// `file_type` comes from [`fs::DirEntry::file_type`], which does **not** traverse a symlink — so a
/// link to a regular file lands here rather than being followed, which is the behaviour this engine
/// has always had. Reporting it does not change it: following links would let the tree escape its
/// own root, cycle, and store the target's bytes under a second name.
///
/// The final arm is unreachable on a POSIX filesystem (the seven types above it are all of them)
/// and exists so an entry the scan drops is always *named*. A silent `_` here would be the shape
/// this whole issue is about.
fn local_unsyncable_reason(file_type: &fs::FileType) -> UnsyncableReason {
    use std::os::unix::fs::FileTypeExt;

    if file_type.is_symlink() {
        UnsyncableReason::LocalSymlink
    } else if file_type.is_socket() {
        UnsyncableReason::LocalSocket
    } else if file_type.is_fifo() {
        UnsyncableReason::LocalFifo
    } else if file_type.is_block_device() || file_type.is_char_device() {
        UnsyncableReason::LocalDevice
    } else {
        UnsyncableReason::LocalSpecialFile
    }
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

pub fn should_ignore_path(path: &Path, naming: &ConflictNaming) -> bool {
    should_ignore_relative_path(path, naming)
}

fn should_ignore_relative_path(relative_path: &Path, naming: &ConflictNaming) -> bool {
    if naming.is_conflict_copy(relative_path) || is_never_synced_directory(relative_path) {
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

/// The directories that are **never** a user's content, whoever created them: the engine's own
/// `.sync` state directory, a leftover download-staging directory, and a FreeDesktop trash.
///
/// ONE DEFINITION BECAUSE THERE ARE TWO READERS, and they used to disagree. A file is filtered by
/// [`should_ignore_relative_path`], but a *directory* is filtered by
/// [`ScanOptions::allows_directory_traversal`], which kept its own copy of this list — so adding the
/// trash rule to the first one alone excluded every trashed file while still scanning, planning and
/// watching the trash directories themselves. Both call this now; a fourth rule added here reaches
/// both by construction.
///
/// The name-based rules in [`should_ignore_relative_path`] are deliberately NOT here: a conflict
/// sidecar, `sync_index.db` and `.proton-sync.toml` name files, and folding them in would newly stop
/// traversal into a *directory* that happens to share one of those names.
fn is_never_synced_directory(relative_path: &Path) -> bool {
    is_download_scratch_path(relative_path)
        || is_sync_state_path(relative_path)
        || is_trash_dir_path(relative_path)
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

/// True when any component of `relative_path` is a FreeDesktop trash directory, or lives inside
/// one — `.Trash` (whose per-user subdirectories are `.Trash/$uid`) or `.Trash-$uid`.
///
/// WHY THIS EXISTS AT ALL. Local deletions go to the desktop trash by default
/// ([`crate::trash`]). When a pair's `local_root` is itself a mount point, the spec puts that
/// filesystem's trash *inside* the synced root — so without this the next pass would scan every
/// file the user just deleted and upload it back to Proton, turning a deletion into a round trip.
///
/// AT ANY DEPTH, unlike [`is_sync_state_path`], because a nested mount point inside the root is a
/// real arrangement and its trash lands at *its* top directory rather than at the root's.
///
/// ANY UID, not this process's. A trash directory is not the user's content under any uid, and
/// matching only one would leave a root shared between two accounts syncing the other's deleted
/// files. `$uid` is required to be digits, so a user's own `.Trash-notes` or `.Trashcan` is
/// ordinary data and still syncs.
///
/// Matched byte-exactly so a component is compared regardless of UTF-8 validity, like
/// [`is_download_scratch_path`].
fn is_trash_dir_path(relative_path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    relative_path.components().any(|component| {
        let bytes = component.as_os_str().as_bytes();
        bytes == b".Trash"
            || bytes
                .strip_prefix(b".Trash-".as_slice())
                .is_some_and(|uid| !uid.is_empty() && uid.iter().all(u8::is_ascii_digit))
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
    crate::lexically_normalized(&absolute)
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
    fn reset_index_state_clears_every_table_the_daemon_decides_from() {
        // `proton-sync reset-index` (G23/#237). Written against the table list rather than one
        // sample row, because the failure mode is a table someone forgets to add here when they add
        // it to the schema — a stale cursor or a stale approval surviving a "reset" is worse than
        // no reset at all.
        let directory = tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("db");
        upsert_record(&connection, &known_file_record("keep.txt", 1, 1, "hash")).expect("record");
        store_event_cursor(&connection, "vol", "cursor-0", 1).expect("cursor");
        store_warm_start_count(&connection, 7).expect("warm start count");
        upsert_delete_approval(
            &connection,
            Path::new("keep.txt"),
            crate::sync::DeleteDirection::Local,
            "fingerprint",
            1,
        )
        .expect("approval");

        reset_index_state(&connection).expect("reset");

        assert!(load_index(&connection).expect("index").is_empty());
        assert!(
            load_event_cursor(&connection, "vol")
                .expect("cursor")
                .is_none()
        );
        assert_eq!(load_warm_start_count(&connection).expect("count"), 0);
        assert!(
            !matching_delete_approval(
                &connection,
                Path::new("keep.txt"),
                crate::sync::DeleteDirection::Local,
                "fingerprint",
            )
            .expect("approval"),
        );
        // And the file is still a working database, not a removed one: the daemon holds this
        // connection open across the reset.
        upsert_record(&connection, &known_file_record("again.txt", 1, 1, "hash"))
            .expect("still writable");
        assert_eq!(load_index(&connection).expect("index").len(), 1);
    }

    #[test]
    fn a_failed_reset_rolls_back_and_leaves_the_connection_usable() {
        // The daemon does not exit on a failed pass, so a half-applied reset would leave its live
        // connection inside an open transaction and every later checkpoint would fail with
        // "cannot start a transaction within a transaction". Forced here by removing a table the
        // reset deletes from, which makes the fourth statement fail with the first three applied.
        let directory = tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("db");
        upsert_record(&connection, &known_file_record("keep.txt", 1, 1, "hash")).expect("record");
        connection
            .execute_batch("DROP TABLE delete_approvals")
            .expect("drop");

        reset_index_state(&connection).expect_err("the reset must fail");

        assert_eq!(
            load_index(&connection).expect("index").len(),
            1,
            "the rows deleted before the failure must roll back, not vanish half-way"
        );
        // The decisive half: a connection left mid-transaction cannot open another one.
        let transaction = connection
            .unchecked_transaction()
            .expect("the connection must not be stuck inside a transaction");
        transaction.commit().expect("commit");
    }

    #[test]
    fn a_scan_ignores_the_sidecars_its_own_configured_suffix_names() {
        // The scanner and the planner have to agree about what a sidecar looks like, or the engine
        // uploads the file it just wrote. `ScanOptions` carries the naming for exactly that reason,
        // and the second half of this test is what the old compiled-in constant could not express:
        // under a custom suffix a `.proton-cloud` file is ORDINARY USER DATA and must sync.
        let directory = tempdir().expect("tempdir");
        for name in ["keep.txt", "keep.from-cloud.txt", "legacy.proton-cloud.txt"] {
            std::fs::write(directory.path().join(name), b"x").expect("write");
        }

        let naming = ConflictNaming::new("from-cloud").expect("suffix");
        let options =
            ScanOptions::new(directory.path(), &[], &[], &[], &naming).expect("scan options");
        let files = scan_local_files_with_options(directory.path(), &options).expect("scan");

        assert!(
            !files.contains_key(Path::new("keep.from-cloud.txt")),
            "the configured suffix must be ignored: {files:?}"
        );
        assert!(
            files.contains_key(Path::new("legacy.proton-cloud.txt")),
            "a name matching only the DEFAULT suffix is ordinary data here — the orphaning that \
             `sync::changing_the_suffix_orphans_sidecars_written_under_the_old_one` records"
        );
        assert!(files.contains_key(Path::new("keep.txt")));
    }

    #[test]
    fn scan_observer_reports_each_visited_file_with_a_running_count() {
        let directory = tempdir().expect("tempdir");
        let nested = directory.path().join("sub");
        std::fs::create_dir(&nested).expect("nested dir");
        std::fs::write(directory.path().join("one.txt"), b"one").expect("write one");
        std::fs::write(nested.join("two.txt"), b"two").expect("write two");
        let options = ScanOptions::new(directory.path(), &[], &[], &[], &ConflictNaming::default())
            .expect("options");

        let seen = std::cell::RefCell::new(Vec::new());
        let observer = |count: u64, path: &Path| {
            seen.borrow_mut().push((count, path.to_path_buf()));
        };
        let entities = scan_local_entities_observed(
            directory.path(),
            &options,
            &HashMap::new(),
            Some(&observer),
        )
        .expect("observed scan");

        assert_eq!(entities.len(), 3, "two files plus the nested directory");
        let mut seen = seen.into_inner();
        seen.sort_by_key(|(count, _)| *count);
        assert_eq!(
            seen.len(),
            2,
            "one callback per visited file, none for directories"
        );
        assert_eq!(
            seen.iter().map(|(count, _)| *count).collect::<Vec<_>>(),
            vec![1, 2],
            "the count runs across the whole scan"
        );
        let paths: Vec<_> = seen.iter().map(|(_, path)| path.clone()).collect();
        assert!(paths.contains(&directory.path().join("one.txt")));
        assert!(paths.contains(&nested.join("two.txt")));
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
    fn scanner_ignores_a_freedesktop_trash_directory_inside_the_root() {
        // A pair whose `local_root` is itself a mount point gets that filesystem's trash created
        // INSIDE the synced root the first time a local deletion applies. Without the exclusion the
        // next pass scans everything the user just deleted and uploads it straight back to Proton.
        // Both spellings the spec defines, and a nested mount point's trash as well as the root's.
        let directory = tempdir().expect("tempdir");
        let uid_trash = directory.path().join(".Trash-1000/files");
        let shared_trash = directory.path().join(".Trash/1000/files");
        let nested_mount_trash = directory.path().join("media/disk/.Trash-1000/files");
        fs::create_dir_all(&uid_trash).expect("uid trash");
        fs::create_dir_all(&shared_trash).expect("shared trash");
        fs::create_dir_all(&nested_mount_trash).expect("nested trash");
        fs::write(uid_trash.join("doomed.txt"), b"deleted").expect("trashed file");
        fs::write(shared_trash.join("also-doomed.txt"), b"deleted").expect("trashed file");
        fs::write(nested_mount_trash.join("deep.txt"), b"deleted").expect("trashed file");
        fs::write(directory.path().join("keep.txt"), b"keep").expect("keep file");

        let entities = scan_local_entities(directory.path()).expect("scan entities");

        assert!(entities.contains_key(Path::new("keep.txt")));
        assert!(
            entities.keys().all(|path| !is_trash_dir_path(path)),
            "no trash directory or its contents may be scanned: {entities:?}"
        );
    }

    #[test]
    fn scan_options_reject_trash_paths_for_the_watcher_and_the_base_index_filter() {
        // The same three readers the scratch exclusion has: the scanner above, plus these two.
        let options = ScanOptions::new(
            Path::new("/root"),
            &[],
            &[],
            &[],
            &ConflictNaming::default(),
        )
        .expect("scan options");
        assert!(!options.allows_relative_file(Path::new(".Trash-1000/files/doomed.txt")));
        assert!(!options.allows_relative_directory(Path::new(".Trash-1000")));
        assert!(!options.allows_relative_file(Path::new(".Trash/1000/files/doomed.txt")));
        assert!(!options.allows_relative_directory(Path::new(".Trash")));
        // A nested mount point's trash, which `is_sync_state_path`'s first-component rule misses.
        assert!(!options.allows_relative_directory(Path::new("media/disk/.Trash-1000")));
        assert!(!options.allows_relative_file(Path::new("media/disk/.Trash-1000/files/x.txt")));
    }

    #[test]
    fn a_users_own_directory_that_merely_looks_like_a_trash_still_syncs() {
        // The predicate must not over-match: `$uid` is digits, so everything here is ordinary user
        // data. Getting this wrong silently stops syncing a folder and reports nothing.
        let options = ScanOptions::new(
            Path::new("/root"),
            &[],
            &[],
            &[],
            &ConflictNaming::default(),
        )
        .expect("scan options");
        for path in [
            "Trash/notes.txt",
            ".Trashcan/notes.txt",
            ".Trash-notes/notes.txt",
            ".Trash-/notes.txt",
            ".Trash-1000a/notes.txt",
            "photos/.Trashy/notes.txt",
            "Trash.txt",
            ".Trash.txt",
        ] {
            assert!(
                options.allows_relative_file(Path::new(path)),
                "{path} is the user's own data and must still sync"
            );
        }
        for path in ["Trash", ".Trashcan", ".Trash-notes", "photos/.Trashy"] {
            assert!(
                options.allows_relative_directory(Path::new(path)),
                "{path} is the user's own folder and must still sync"
            );
        }
    }

    #[test]
    fn scan_options_reject_download_scratch_paths() {
        let options = ScanOptions::new(
            Path::new("/root"),
            &[],
            &[],
            &[],
            &ConflictNaming::default(),
        )
        .expect("scan options");
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
        let options = ScanOptions::new(directory.path(), &[], &[], &[], &ConflictNaming::default())
            .expect("options");
        let known = HashMap::new();
        let context = WalkContext {
            root: directory.path(),
            options: &options,
            known: &known,
            observer: None,
        };
        let mut scan = LocalScan::default();
        assert!(
            vanished_entry_to_skip(visit_directory(&context, &missing, &mut scan, &mut 0))
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
            &ConflictNaming::default(),
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
            &ConflictNaming::default(),
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
        let options = ScanOptions::new(
            &dotted_root,
            std::slice::from_ref(&db_path),
            &[],
            &[],
            &ConflictNaming::default(),
        )
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
        let options = ScanOptions::new(
            &root,
            std::slice::from_ref(&dotted_db),
            &[],
            &[],
            &ConflictNaming::default(),
        )
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

        let options = ScanOptions::new(
            directory.path(),
            std::slice::from_ref(&db_path),
            &[],
            &[],
            &ConflictNaming::default(),
        )
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

        let options = ScanOptions::new(directory.path(), &[], &[], &[], &ConflictNaming::default())
            .expect("scan options");
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
        let options =
            ScanOptions::new(Path::new("root"), &[], &[], &[], &ConflictNaming::default())
                .expect("scan options");

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
            &ConflictNaming::default(),
        )
        .expect("scan options");

        let files = scan_local_files_with_options(directory.path(), &options).expect("scan files");

        assert_eq!(files.len(), 1);
        assert!(files.contains_key(Path::new("docs/keep.md")));
    }

    #[test]
    fn index_totals_count_files_only_and_never_the_directory_rows() {
        // The trap this predicate exists for: `file_index` stores directories as rows too, so a
        // bare COUNT(*) reports a corpus larger than any set the user recognises. An earlier
        // investigation was misled by exactly that.
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        for (path, kind, size) in [
            ("a.txt", EntityKind::File, 1_000u64),
            ("b.txt", EntityKind::File, 2_500),
            ("docs", EntityKind::Directory, 0),
            ("docs/c.txt", EntityKind::File, 500),
        ] {
            upsert_record(
                &connection,
                &FileRecord {
                    file_path: PathBuf::from(path),
                    entity_kind: kind,
                    file_size: size,
                    mtime: 1,
                    sha1_hash: Some("hash".to_owned()),
                    proton_id: None,
                    sync_status: SyncStatus::Synced,
                },
            )
            .expect("upsert");
        }

        let totals = index_totals(&connection).expect("totals");
        assert_eq!(totals.files, 3, "the directory row is not a file");
        assert_eq!(totals.bytes, 4_000);
    }

    #[test]
    fn index_totals_are_zero_rather_than_an_error_on_an_empty_index() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        assert_eq!(
            index_totals(&connection).expect("totals"),
            IndexTotals { files: 0, bytes: 0 },
            "an empty index is a real answer; COALESCE is what keeps SUM from returning NULL"
        );
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
        assert!(should_ignore_path(
            Path::new(".proton-sync.toml"),
            &ConflictNaming::default()
        ));
        assert!(should_ignore_path(
            Path::new("a/b/.proton-sync.toml"),
            &ConflictNaming::default()
        ));
        // A same-named directory or unrelated file is not ignored.
        assert!(!should_ignore_path(
            Path::new("a/proton-sync.toml"),
            &ConflictNaming::default()
        ));
        assert!(!should_ignore_path(
            Path::new("notes.txt"),
            &ConflictNaming::default()
        ));
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
    fn the_sole_event_cursor_names_the_volume_only_while_unambiguous() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");

        assert!(
            load_sole_event_cursor(&connection)
                .expect("load empty")
                .is_none(),
            "nothing stored names no volume"
        );

        store_event_cursor(&connection, "vol-1", "cursor-a", 100).expect("store volume");
        store_event_cursor(&connection, "core", "core-cursor", 100).expect("store core");
        assert_eq!(
            load_sole_event_cursor(&connection)
                .expect("load")
                .expect("the one volume row")
                .scope_id,
            "vol-1",
            "the account-wide core stream is not a volume and must be ignored"
        );

        store_event_cursor(&connection, "vol-2", "cursor-b", 100).expect("store second volume");
        assert!(
            load_sole_event_cursor(&connection)
                .expect("load ambiguous")
                .is_none(),
            "two volume rows name no single volume"
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
        let options = ScanOptions::new(directory.path(), &[], &[], &[], &ConflictNaming::default())
            .expect("options");
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
        let options = ScanOptions::new(directory.path(), &[], &[], &[], &ConflictNaming::default())
            .expect("options");
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
    fn insert_legacy_text_row(connection: &Connection, relative_path: &Path, sha1: &str, id: &str) {
        connection
            .execute(
                "INSERT INTO file_index \
                 (file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status) \
                 VALUES (?1, 'file', 3, 7, ?2, ?3, 'synced')",
                params![path_key(relative_path), sha1, id],
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
            insert_legacy_text_row(&connection, Path::new("dir/notes.txt"), "hash", "pid");
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
            insert_legacy_text_row(&connection, Path::new("a.txt"), "old", "pid-old");
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
    fn migration_drops_lossy_legacy_text_keys_instead_of_casting_them() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        // Doubly-legacy precondition: a pre-BLOB database holding a non-UTF-8 filename,
        // whose TEXT key path_key wrote through to_string_lossy.
        let lossy_path = PathBuf::from(OsString::from_vec(b"dir/re\xffport.txt".to_vec()));
        let legacy_key = path_key(&lossy_path);
        assert!(
            legacy_key.contains('\u{fffd}'),
            "precondition: the legacy TEXT key is the lossy encoding"
        );

        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("sync_index.db");
        {
            let connection = Connection::open(&db_path).expect("open");
            connection.execute_batch(SCHEMA).expect("schema");
            insert_legacy_text_row(&connection, &lossy_path, "lossy", "pid-lossy");
            insert_legacy_text_row(&connection, Path::new("dir/ok.txt"), "clean", "pid-ok");
        }

        let connection = open_database(&db_path).expect("open database");

        // CAST(lossy TEXT AS BLOB) keeps the U+FFFD bytes, so the migrated key could never
        // equal index_key(lossy_path): the row would be a permanent phantom baseline the
        // planner reads as locally deleted and answers with a spurious RemoteDelete.
        let phantom: i64 = connection
            .query_row(
                "SELECT count(*) FROM file_index WHERE file_path = ?1",
                params![legacy_key.clone().into_bytes()],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(phantom, 0, "the lossy key must not survive the migration");
        let total: i64 = connection
            .query_row("SELECT count(*) FROM file_index", [], |row| row.get(0))
            .expect("count");
        assert_eq!(total, 1, "only the non-lossy row may remain");

        let index = load_index(&connection).expect("load index");
        assert!(
            !index.contains_key(Path::new(&legacy_key)),
            "no phantom baseline row may reach the planner"
        );

        // The screen is keyed on U+FFFD, not on "legacy": a losslessly encoded TEXT key
        // still migrates to BLOB and stays point-queryable.
        assert_eq!(key_storage_class(&connection), "blob");
        let record = get_record(&connection, Path::new("dir/ok.txt"))
            .expect("get_record")
            .expect("a non-lossy legacy row must survive the migration");
        assert_eq!(record.sha1_hash.as_deref(), Some("clean"));
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

    #[cfg(unix)]
    #[test]
    fn unsyncable_items_round_trip_including_a_non_utf8_path() {
        // The `unrepresentable_path` reason is PRECISELY the non-UTF-8 paths a lossy TEXT key
        // mangles, so the list is keyed on the byte-exact BLOB encoding like `file_index` — see
        // `normalize_legacy_text_keys` for what the TEXT key cost the baseline (#75).
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("open");
        assert!(
            load_unsyncable_items(&connection).expect("load").is_empty(),
            "a fresh database has no unsyncable items"
        );

        let items = vec![
            UnsyncableItem {
                path: PathBuf::from(OsStr::from_bytes(b"caf\xe9.txt")),
                entity_kind: EntityKind::File,
                reason: UnsyncableReason::UnrepresentablePath,
                first_seen_epoch_secs: 100,
            },
            UnsyncableItem {
                path: PathBuf::from("Unsorted/Networth"),
                entity_kind: EntityKind::File,
                reason: UnsyncableReason::RemoteNotDownloadable,
                first_seen_epoch_secs: 200,
            },
        ];
        replace_unsyncable_items(&connection, &items).expect("store");
        let loaded = load_unsyncable_items(&connection).expect("load");
        assert_eq!(loaded.len(), 2);
        assert!(
            loaded.iter().any(|item| item.path == items[0].path),
            "the non-UTF-8 path must come back byte-exact, not lossily: {loaded:?}"
        );
        assert!(loaded.contains(&items[1]));

        // Wholesale replacement, so a shrinking list actually shrinks.
        replace_unsyncable_items(&connection, &items[1..]).expect("replace");
        assert_eq!(
            load_unsyncable_items(&connection).expect("load"),
            items[1..]
        );
    }

    /// A root holding one ordinary file plus every local kind the walk cannot sync, and the scan of
    /// it. `mkfifo` is shelled rather than pulled in as a dependency; a device node needs root and
    /// is therefore not exercised here — `UnsyncableReason::LocalDevice` is covered only by the
    /// classifier's own arm.
    #[cfg(unix)]
    fn scan_a_root_of_special_files(
        options: Option<ScanOptions>,
    ) -> (tempfile::TempDir, LocalScan) {
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        fs::write(root.join("real.txt"), b"bytes").expect("regular file");
        fs::create_dir(root.join("folder")).expect("directory");

        let _listener =
            std::os::unix::net::UnixListener::bind(root.join("session.sock")).expect("socket");
        std::os::unix::fs::symlink(root.join("real.txt"), root.join("link-to-file"))
            .expect("symlink to a file");
        std::os::unix::fs::symlink(root.join("folder"), root.join("link-to-folder"))
            .expect("symlink to a folder");
        std::os::unix::fs::symlink(root.join("nowhere"), root.join("broken-link"))
            .expect("dangling symlink");
        let fifo = std::process::Command::new("mkfifo")
            .arg(root.join("pipe"))
            .status()
            .expect("run mkfifo");
        assert!(fifo.success(), "mkfifo must succeed");

        let options = options.unwrap_or_else(|| {
            ScanOptions::new(root, &[], &[], &[], &ConflictNaming::default()).expect("options")
        });
        let scan = scan_local_tree(root, &options, &HashMap::new(), None).expect("scan");
        (directory, scan)
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_symlink_or_fifo_is_named_instead_of_silently_dropped() {
        // #232: these were dropped in the same `continue` as an excluded path, so nothing ever
        // recorded them. The DROP itself is unchanged — the walk still keeps only regular files
        // and directories — and that is what the entity assertions below pin.
        let (_directory, scan) = scan_a_root_of_special_files(None);

        let mut reported: Vec<(String, &str)> = scan
            .unsyncable
            .iter()
            .map(|entry| {
                (
                    entry.relative_path.display().to_string(),
                    entry.reason.as_str(),
                )
            })
            .collect();
        reported.sort();
        assert_eq!(
            reported,
            vec![
                ("broken-link".to_owned(), "local_symlink"),
                ("link-to-file".to_owned(), "local_symlink"),
                ("link-to-folder".to_owned(), "local_symlink"),
                ("pipe".to_owned(), "local_fifo"),
                ("session.sock".to_owned(), "local_socket"),
            ],
            "every non-regular entry is named, and a symlink is one whatever it points at"
        );

        let mut kept: Vec<String> = scan
            .entities
            .keys()
            .map(|path| path.display().to_string())
            .collect();
        kept.sort();
        assert_eq!(
            kept,
            vec!["folder".to_owned(), "real.txt".to_owned()],
            "reporting a symlink must not start following one: the tree the engine syncs is \
             byte-identical to what it was before"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_not_descended_into() {
        // The other half of "reporting does not change behaviour": if `link-to-folder` were
        // traversed, `folder`'s contents would appear twice under two names — and a link pointing
        // at an ancestor would not terminate at all.
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        fs::create_dir(root.join("folder")).expect("directory");
        fs::write(root.join("folder/inside.txt"), b"bytes").expect("file");
        std::os::unix::fs::symlink(root.join("folder"), root.join("mirror")).expect("symlink");

        let options =
            ScanOptions::new(root, &[], &[], &[], &ConflictNaming::default()).expect("options");
        let scan = scan_local_tree(root, &options, &HashMap::new(), None).expect("scan");

        assert!(scan.entities.contains_key(Path::new("folder/inside.txt")));
        assert!(
            !scan.entities.contains_key(Path::new("mirror/inside.txt")),
            "the link was not followed: {:?}",
            scan.entities.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            scan.unsyncable
                .iter()
                .map(|entry| entry.relative_path.display().to_string())
                .collect::<Vec<_>>(),
            vec!["mirror".to_owned()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_an_exclude_rule_hides_is_excluded_not_unsyncable() {
        // The two groups on one dialog are "you told it to skip these" and "these can't be synced
        // at all". A path that answers to a rule the user wrote belongs to the first, so the rule
        // test runs BEFORE the file-type test and this reports nothing.
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        let _listener =
            std::os::unix::net::UnixListener::bind(root.join("session.sock")).expect("socket");
        let options = ScanOptions::new(
            root,
            &[],
            &[],
            &["*.sock".to_owned()],
            &ConflictNaming::default(),
        )
        .expect("options");

        let scan = scan_local_tree(root, &options, &HashMap::new(), None).expect("scan");
        assert!(
            scan.unsyncable.is_empty(),
            "an excluded path is the user's own rule, not a limitation of the engine: {:?}",
            scan.unsyncable
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_include_filter_hides_a_socket_from_the_unsyncable_list_too() {
        // An include filter is the same statement said the other way round ("only these"), and the
        // one gate covers both because `allows_relative_file` is the one predicate.
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        let _listener =
            std::os::unix::net::UnixListener::bind(root.join("session.sock")).expect("socket");
        let options = ScanOptions::new(
            root,
            &[],
            &["**/*.txt".to_owned()],
            &[],
            &ConflictNaming::default(),
        )
        .expect("options");

        let scan = scan_local_tree(root, &options, &HashMap::new(), None).expect("scan");
        assert!(scan.unsyncable.is_empty(), "{:?}", scan.unsyncable);
    }

    #[cfg(unix)]
    #[test]
    fn the_engines_own_state_directory_is_never_reported_as_unsyncable() {
        // `.sync` holds the lockfile and the SQLite index; nothing in it is the user's data, and a
        // socket the engine itself parked there would still not be a thing the user must fix.
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        fs::create_dir(root.join(crate::paths::SYNC_STATE_DIR_NAME)).expect("state dir");
        let _listener = std::os::unix::net::UnixListener::bind(
            root.join(crate::paths::SYNC_STATE_DIR_NAME)
                .join("ipc.sock"),
        )
        .expect("socket");

        let options =
            ScanOptions::new(root, &[], &[], &[], &ConflictNaming::default()).expect("options");
        let scan = scan_local_tree(root, &options, &HashMap::new(), None).expect("scan");
        assert!(scan.unsyncable.is_empty(), "{:?}", scan.unsyncable);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_named_fifo_is_reported_by_its_kind_not_by_its_name() {
        // Precedence, pinned: a FIFO whose name is also unrepresentable is a FIFO. The
        // `unrepresentable_path` reason is the planner's, and it only ever applies to entities the
        // scan KEPT — this one never reaches the planner at all.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        let name = OsStr::from_bytes(b"caf\xe9.pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(root.join(name))
            .status()
            .expect("run mkfifo");
        assert!(status.success());

        let options =
            ScanOptions::new(root, &[], &[], &[], &ConflictNaming::default()).expect("options");
        let scan = scan_local_tree(root, &options, &HashMap::new(), None).expect("scan");

        assert_eq!(scan.unsyncable.len(), 1);
        assert_eq!(scan.unsyncable[0].reason, UnsyncableReason::LocalFifo);
        assert_eq!(
            scan.unsyncable[0].relative_path,
            PathBuf::from(name),
            "and the path stays byte-exact, because the store's key is a BLOB"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_older_scan_entry_points_still_answer_only_with_entities() {
        // `scan_local_tree` is the richer form, not a replacement: every existing caller keeps the
        // same answer, so nothing else in the engine changed shape.
        let (_directory, scan) = scan_a_root_of_special_files(None);
        let directory = tempdir().expect("tempdir");
        fs::write(directory.path().join("real.txt"), b"bytes").expect("file");
        let files = scan_local_files(directory.path()).expect("scan files");
        assert_eq!(files.len(), 1);
        assert!(!scan.unsyncable.is_empty());
    }

    #[test]
    fn last_transfer_is_the_newest_side_effect_that_actually_moved_bytes() {
        // #233. `Purge` and `AutoLink` land at the same path and move nothing, so neither may be
        // read as "when Proton Drive last had these bytes".
        let directory = tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("open");
        let path = Path::new("docs/spec.md");

        assert!(
            last_transfer(&connection, path).expect("query").is_none(),
            "nothing has ever transferred, and that is a None rather than a zero"
        );

        let pass = begin_pass(&connection, 1_000, PassKind::Incremental).expect("pass");
        let event = |action: SyncAction, bytes: Option<u64>, at: u64| FileEvent {
            path: path.to_path_buf(),
            source_path: None,
            action,
            bytes,
            epoch_secs: at,
            pass_id: pass,
        };
        insert_file_events(
            &connection,
            pass,
            &[
                event(SyncAction::Download, Some(10), 1_000),
                event(SyncAction::Upload, Some(20), 2_000),
                event(SyncAction::AutoLink, None, 3_000),
                event(SyncAction::Purge, None, 4_000),
            ],
        )
        .expect("events");

        let found = last_transfer(&connection, path)
            .expect("query")
            .expect("some");
        assert_eq!(found.action, SyncAction::Upload);
        assert_eq!(found.epoch_secs, 2_000);
        assert_eq!(found.bytes, Some(20));
        assert_eq!(
            found.action.transfer_direction(),
            Some(crate::sync::TransferDirection::Up),
            "and the direction is the action's own, never a second copy of the rule"
        );
    }

    #[test]
    fn last_transfer_answers_about_one_path_only() {
        // A sibling's upload must never become this file's "received" time.
        let directory = tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("open");
        let pass = begin_pass(&connection, 1_000, PassKind::Incremental).expect("pass");
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: PathBuf::from("other.txt"),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(5),
                epoch_secs: 9_000,
                pass_id: pass,
            }],
        )
        .expect("events");

        assert!(
            last_transfer(&connection, Path::new("docs/spec.md"))
                .expect("query")
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn last_transfer_keys_on_the_byte_exact_path() {
        // The same BLOB key `file_index` uses: two names differing only in invalid bytes are two
        // files, and a lossy key would answer for the wrong one.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("open");
        let raw = PathBuf::from(OsStr::from_bytes(b"caf\xe9.txt"));
        let pass = begin_pass(&connection, 1_000, PassKind::Incremental).expect("pass");
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: raw.clone(),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(5),
                epoch_secs: 7_000,
                pass_id: pass,
            }],
        )
        .expect("events");

        assert_eq!(
            last_transfer(&connection, &raw)
                .expect("query")
                .expect("some")
                .epoch_secs,
            7_000
        );
        assert!(
            last_transfer(&connection, Path::new("caf\u{fffd}.txt"))
                .expect("query")
                .is_none(),
            "the lossy rendering is a different key and must not match"
        );
    }

    #[test]
    fn withheld_deletions_round_trip_and_an_old_database_gains_the_table() {
        // The queue survives restarts, so its age has to (#225) — including on a database written
        // before this table existed, which `CREATE TABLE IF NOT EXISTS` must add cleanly.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir().expect("tempdir");
        let db_path = directory.path().join("index.db");
        {
            let connection = Connection::open(&db_path).expect("open");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE file_index (
                        file_path TEXT PRIMARY KEY,
                        entity_kind TEXT NOT NULL DEFAULT 'file',
                        file_size INTEGER NOT NULL,
                        mtime INTEGER NOT NULL,
                        sha1_hash TEXT,
                        proton_id TEXT,
                        sync_status TEXT NOT NULL
                    );
                    "#,
                )
                .expect("pre-existing schema");
        }
        let connection = open_database(&db_path).expect("open database upgrades cleanly");
        assert!(
            load_withheld_deletions(&connection)
                .expect("load")
                .is_empty(),
            "an upgraded database has no withheld deletions"
        );

        let items = vec![
            WithheldDeletion {
                path: PathBuf::from(OsStr::from_bytes(b"caf\xe9.txt")),
                direction: crate::sync::DeleteDirection::Local,
                fingerprint: "sha1-of-the-file".to_owned(),
                first_seen_epoch_secs: 100,
            },
            // Same path, other direction — the key is the pair, so both rows coexist.
            WithheldDeletion {
                path: PathBuf::from(OsStr::from_bytes(b"caf\xe9.txt")),
                direction: crate::sync::DeleteDirection::Remote,
                fingerprint: "sha1-of-the-file".to_owned(),
                first_seen_epoch_secs: 200,
            },
        ];
        replace_withheld_deletions(&connection, &items).expect("store");
        let mut loaded = load_withheld_deletions(&connection).expect("load");
        loaded.sort_by_key(|item| item.first_seen_epoch_secs);
        assert_eq!(loaded, items, "byte-exact path, both directions, both ages");

        // Wholesale replacement: the empty write is how a pass that withholds nothing clears it.
        replace_withheld_deletions(&connection, &[]).expect("clear");
        assert!(
            load_withheld_deletions(&connection)
                .expect("load")
                .is_empty()
        );
    }

    #[test]
    fn purging_a_subtree_takes_the_descendants_and_nothing_that_merely_shares_a_prefix() {
        let directory = tempfile::tempdir().expect("tempdir");
        let connection = open_database(&directory.path().join("index.db")).expect("open");
        for path in ["photos", "photos/2019/a.jpg", "photosx", "photosx/b.jpg"] {
            upsert_record(
                &connection,
                &FileRecord {
                    file_path: PathBuf::from(path),
                    entity_kind: EntityKind::File,
                    file_size: 1,
                    mtime: 1,
                    sha1_hash: Some("hash".to_owned()),
                    proton_id: None,
                    sync_status: SyncStatus::Synced,
                },
            )
            .expect("record");
        }

        let purged = purge_subtree_records(&connection, Path::new("photos")).expect("purge");

        assert_eq!(purged.len(), 2, "the folder and its one descendant");
        for gone in ["photos", "photos/2019/a.jpg"] {
            assert!(
                get_record(&connection, Path::new(gone))
                    .expect("lookup")
                    .is_none()
            );
        }
        for kept in ["photosx", "photosx/b.jpg"] {
            assert!(
                get_record(&connection, Path::new(kept))
                    .expect("lookup")
                    .is_some(),
                "{kept} shares a byte prefix and is not under photos"
            );
        }
    }

    #[test]
    fn warm_start_state_is_added_to_a_preexisting_database_and_defaults_to_zero() {
        // Upgrade path: an existing user's index.db predates the `warm_start_state` table.
        // `open_database` runs `execute_batch(SCHEMA)` unconditionally (no version gate), and the
        // table is `CREATE TABLE IF NOT EXISTS`, so opening an old DB must add it cleanly, leave
        // existing rows intact, and report a zero warm-start count.
        let directory = tempdir().expect("tempdir");
        let db_path = directory.path().join("sync_index.db");
        {
            // A pre-warm-start database: the current tables minus `warm_start_state`, with a row.
            let connection = Connection::open(&db_path).expect("open");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE file_index (
                        file_path TEXT PRIMARY KEY,
                        entity_kind TEXT NOT NULL DEFAULT 'file',
                        file_size INTEGER NOT NULL,
                        mtime INTEGER NOT NULL,
                        sha1_hash TEXT,
                        proton_id TEXT,
                        sync_status TEXT NOT NULL
                    );
                    "#,
                )
                .expect("preexisting schema");
            connection
                .execute(
                    "INSERT INTO file_index \
                     (file_path, entity_kind, file_size, mtime, sha1_hash, proton_id, sync_status) \
                     VALUES (?1, 'file', 5, 9, 'hash', 'pid', 'synced')",
                    params![path_key(Path::new("notes.txt"))],
                )
                .expect("preexisting row");
        }

        // Reopen through the real entry point, which must add `warm_start_state`.
        let connection = open_database(&db_path).expect("open database upgrades cleanly");

        assert_eq!(
            load_warm_start_count(&connection).expect("load count"),
            0,
            "an upgraded database reports a zero warm-start count"
        );
        assert!(
            get_record(&connection, Path::new("notes.txt"))
                .expect("get_record")
                .is_some(),
            "the pre-existing file_index row must survive the upgrade"
        );

        // And the new table is fully usable after the upgrade.
        store_warm_start_count(&connection, 7).expect("store count");
        assert_eq!(load_warm_start_count(&connection).expect("reload"), 7);
    }

    // ---- pass and path history --------------------------------------------------------------

    fn history_db() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory database");
        initialize_schema(&connection).expect("schema");
        connection
    }

    /// A sealed pass row, `n` seconds ago.
    fn record_pass(
        connection: &Connection,
        started_at: u64,
        kind: PassKind,
        outcome: PassOutcomeKind,
        changed: usize,
        bytes_up: u64,
        bytes_down: u64,
    ) -> i64 {
        let id = begin_pass(connection, started_at, kind).expect("begin");
        finish_pass(
            connection, id, 42, outcome, changed, 0, bytes_up, bytes_down, None,
        )
        .expect("finish");
        id
    }

    #[test]
    fn a_pass_starts_interrupted_and_is_sealed_by_finishing() {
        // The row opens with the values that are TRUE of a pass that has started and not finished,
        // so a process killed mid-pass leaves an honest row and no startup repair sweep is needed.
        let connection = history_db();
        let id = begin_pass(&connection, 100, PassKind::WarmStart).expect("begin");
        let open = &recent_passes(&connection, 10).expect("recent")[0];
        assert_eq!(open.outcome, "interrupted");
        assert_eq!(open.duration_ms, 0);
        assert_eq!(open.kind, "warm-start");

        finish_pass(
            &connection,
            id,
            1500,
            PassOutcomeKind::Partial,
            4,
            2,
            10,
            20,
            Some("2 item(s) failed to sync"),
        )
        .expect("finish");
        let sealed = &recent_passes(&connection, 10).expect("recent")[0];
        assert_eq!(sealed.outcome, "partial");
        assert_eq!(sealed.duration_ms, 1500);
        assert_eq!(sealed.changed, 4);
        assert_eq!(sealed.failed, 2);
        assert_eq!(sealed.bytes_uploaded, 10);
        assert_eq!(sealed.bytes_downloaded, 20);
        assert_eq!(sealed.error.as_deref(), Some("2 item(s) failed to sync"));
    }

    #[test]
    fn recent_passes_are_newest_first_and_capped() {
        let connection = history_db();
        for start in 0..5 {
            record_pass(
                &connection,
                start,
                PassKind::Incremental,
                PassOutcomeKind::Clean,
                1,
                0,
                0,
            );
        }
        let recent = recent_passes(&connection, 3).expect("recent");
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].started_epoch_secs, 4);
        assert_eq!(recent[2].started_epoch_secs, 2);
    }

    #[test]
    fn a_failure_storm_never_evicts_the_last_full_sweep() {
        // Every non-clean pass is notable, so a chronically broken daemon writes thousands of rows
        // a day. Without the exemption the row cap would prune the full-sweep row within hours —
        // losing `Last one 2 days ago` exactly when the user is debugging why sync is broken.
        let connection = history_db();
        let sweep = record_pass(
            &connection,
            1,
            PassKind::FullSweep,
            PassOutcomeKind::Clean,
            0,
            0,
            0,
        );
        for start in 2..30u64 {
            record_pass(
                &connection,
                start,
                PassKind::Incremental,
                PassOutcomeKind::Failed,
                0,
                0,
                0,
            );
        }
        let retention = HistoryRetention {
            max_passes: 5,
            ..HistoryRetention::default()
        };
        prune_history(&connection, 1_000, retention).expect("prune");

        assert_eq!(
            last_full_sweep(&connection).expect("sweep").map(|p| p.id),
            Some(sweep),
            "the newest full sweep survives the row cap"
        );
        // The cap still binds everything else: 5 recent rows plus the pinned sweep.
        let all = recent_passes(&connection, 100).expect("recent");
        assert_eq!(all.len(), 6);
    }

    #[test]
    fn an_aged_out_full_sweep_also_survives_the_age_bound() {
        let connection = history_db();
        let sweep = record_pass(
            &connection,
            10,
            PassKind::FullSweep,
            PassOutcomeKind::Clean,
            0,
            0,
            0,
        );
        record_pass(
            &connection,
            11,
            PassKind::Incremental,
            PassOutcomeKind::Clean,
            1,
            0,
            0,
        );
        let retention = HistoryRetention {
            max_pass_age_secs: 5,
            ..HistoryRetention::default()
        };
        prune_history(&connection, 1_000, retention).expect("prune");
        assert_eq!(
            last_full_sweep(&connection).expect("sweep").map(|p| p.id),
            Some(sweep)
        );
        assert_eq!(recent_passes(&connection, 100).expect("recent").len(), 1);
    }

    #[test]
    fn pruning_an_empty_history_deletes_nothing_and_does_not_error() {
        // The full-sweep exemption is a NULL subquery here; `IS NOT` keeps that from turning the
        // whole predicate NULL (which `<>` would, silently deleting nothing forever after).
        let connection = history_db();
        prune_history(&connection, 1_000, HistoryRetention::default()).expect("prune");
        record_pass(
            &connection,
            1,
            PassKind::Incremental,
            PassOutcomeKind::Clean,
            1,
            0,
            0,
        );
        prune_history(
            &connection,
            1_000,
            HistoryRetention {
                max_pass_age_secs: 1,
                ..HistoryRetention::default()
            },
        )
        .expect("prune");
        assert!(
            recent_passes(&connection, 10).expect("recent").is_empty(),
            "with no full sweep to pin, the age bound still applies"
        );
    }

    #[test]
    fn the_event_log_is_bounded_by_both_age_and_rows() {
        let connection = history_db();
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        let events: Vec<FileEvent> = (0..10)
            .map(|n| FileEvent {
                path: PathBuf::from(format!("f{n}.txt")),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(1),
                epoch_secs: n * 100,
                pass_id: pass,
            })
            .collect();
        insert_file_events(&connection, pass, &events).expect("insert");

        // Age first: everything before 500 goes.
        prune_history(
            &connection,
            1_000,
            HistoryRetention {
                max_event_age_secs: 500,
                ..HistoryRetention::default()
            },
        )
        .expect("prune");
        assert_eq!(
            file_events(&connection, None, 0, 100)
                .expect("events")
                .total,
            5
        );

        // Then the row cap.
        prune_history(
            &connection,
            1_000,
            HistoryRetention {
                max_events: 2,
                ..HistoryRetention::default()
            },
        )
        .expect("prune");
        let kept = file_events(&connection, None, 0, 100).expect("events");
        assert_eq!(kept.total, 2);
        assert_eq!(kept.events[0].path, PathBuf::from("f9.txt"));
    }

    #[test]
    fn a_move_is_found_at_the_path_it_moved_to() {
        // #190 looks a file's history up by the path it has NOW. A move recorded at its source
        // would be missing from exactly that query — the file's own move.
        let connection = history_db();
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: PathBuf::from("archive/notes.md"),
                source_path: Some(PathBuf::from("notes.md")),
                action: SyncAction::MoveLocal,
                bytes: None,
                epoch_secs: 10,
                pass_id: pass,
            }],
        )
        .expect("insert");

        let found =
            file_events(&connection, Some(Path::new("archive/notes.md")), 0, 10).expect("events");
        assert_eq!(found.total, 1);
        assert_eq!(
            found.events[0].source_path,
            Some(PathBuf::from("notes.md")),
            "where it came from is still recoverable"
        );
        assert_eq!(
            file_events(&connection, Some(Path::new("notes.md")), 0, 10)
                .expect("events")
                .total,
            0
        );
    }

    #[test]
    fn the_feed_counts_events_and_distinct_files_over_the_same_window() {
        // `7 files in the last 3 days` is a DISTINCT-path count over the window, not a row count:
        // a file edited four times is one file.
        let connection = history_db();
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        let mut events = Vec::new();
        for n in 0..4 {
            events.push(FileEvent {
                path: PathBuf::from("busy.txt"),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(10),
                epoch_secs: 900 + n,
                pass_id: pass,
            });
        }
        events.push(FileEvent {
            path: PathBuf::from("old.txt"),
            source_path: None,
            action: SyncAction::Download,
            bytes: Some(5),
            epoch_secs: 10,
            pass_id: pass,
        });
        insert_file_events(&connection, pass, &events).expect("insert");

        let filtered = file_events(&connection, Some(Path::new("busy.txt")), 0, 100)
            .expect("path-filtered events");
        assert_eq!(filtered.total, 4);
        assert!(
            filtered.totals.is_none(),
            "a path-filtered reply carries no byte total: the rollup has no paths, and the \
             window-wide number would read as this file's own"
        );

        let window = file_events(&connection, None, 500, 100).expect("events");
        assert_eq!(window.total, 4, "four events");
        assert_eq!(window.files, 1, "one file");
        let everything = file_events(&connection, None, 0, 100).expect("events");
        assert_eq!(everything.total, 5);
        assert_eq!(everything.files, 2);
        // The cap limits the page, never the counts.
        let paged = file_events(&connection, None, 0, 2).expect("events");
        assert_eq!(paged.events.len(), 2);
        assert_eq!(paged.total, 5);
    }

    #[test]
    fn byte_totals_come_from_the_rollup_and_outlive_the_detail() {
        // #191's totals are summed over `sync_passes`, never over `sync_events` — which is what
        // keeps them right after the per-file detail has aged out. Two sources for one number is
        // how they drift.
        let connection = history_db();
        let pass = record_pass(
            &connection,
            1_000,
            PassKind::Incremental,
            PassOutcomeKind::Clean,
            2,
            4096,
            8192,
        );
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: PathBuf::from("a.bin"),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(4096),
                epoch_secs: 1_000,
                pass_id: pass,
            }],
        )
        .expect("insert");

        let totals = byte_totals_since(&connection, 500).expect("totals");
        assert_eq!(totals.uploaded_bytes, 4096);
        assert_eq!(totals.downloaded_bytes, 8192);
        assert_eq!(totals.since_epoch_secs, 500);

        // Drop every event row; the totals do not move.
        connection
            .execute("DELETE FROM sync_events", [])
            .expect("clear events");
        let after = byte_totals_since(&connection, 500).expect("totals");
        assert_eq!(after.uploaded_bytes, 4096);
        assert_eq!(after.downloaded_bytes, 8192);

        // A pass is counted in the window it STARTED in.
        assert_eq!(
            byte_totals_since(&connection, 1_001)
                .expect("totals")
                .uploaded_bytes,
            0
        );
    }

    #[test]
    fn an_action_token_this_build_does_not_know_is_skipped_not_fatal() {
        // A newer daemon's token must not make the whole feed unreadable — this is display data.
        let connection = history_db();
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: PathBuf::from("known.txt"),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(1),
                epoch_secs: 5,
                pass_id: pass,
            }],
        )
        .expect("insert");
        connection
            .execute(
                "INSERT INTO sync_events (pass_id, path, source_path, action, bytes, at) \
                 VALUES (?1, ?2, NULL, 'teleport', NULL, 6)",
                params![pass, index_key(Path::new("future.txt"))],
            )
            .expect("insert future row");

        let feed = file_events(&connection, None, 0, 100).expect("events");
        assert_eq!(feed.events.len(), 1);
        assert_eq!(feed.events[0].path, PathBuf::from("known.txt"));
        // The counts are SQL-side and still see the row: they describe the window, not this
        // build's vocabulary.
        assert_eq!(feed.total, 2);
    }

    #[test]
    fn history_survives_a_non_utf8_path() {
        use std::os::unix::ffi::OsStrExt;
        let connection = history_db();
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"dir/re\xffport.txt"));
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: path.clone(),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(1),
                epoch_secs: 5,
                pass_id: pass,
            }],
        )
        .expect("insert");
        let found = file_events(&connection, Some(&path), 0, 10).expect("events");
        assert_eq!(
            found.events[0].path, path,
            "byte-exact through the BLOB key"
        );
    }
}
