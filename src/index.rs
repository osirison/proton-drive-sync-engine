use crate::{AppResult, boxed_error};
use rusqlite::{Connection, OptionalExtension, params};
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
    file_size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    sha1_hash TEXT NOT NULL,
    proton_id TEXT,
    sync_status TEXT NOT NULL
);
"#;

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
    pub file_size: u64,
    pub mtime: i64,
    pub sha1_hash: String,
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
            file_size: local.file_size,
            mtime: local.mtime,
            sha1_hash: local.sha1_hash.clone(),
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

pub fn open_database(path: &Path) -> AppResult<Connection> {
    let connection = Connection::open(path)?;
    initialize_schema(&connection)?;
    Ok(connection)
}

pub fn initialize_schema(connection: &Connection) -> AppResult<()> {
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

pub fn load_index(connection: &Connection) -> AppResult<HashMap<PathBuf, FileRecord>> {
    let mut statement = connection.prepare(
        "SELECT file_path, file_size, mtime, sha1_hash, proton_id, sync_status FROM file_index",
    )?;
    let rows = statement.query_map([], |row| {
        let status: String = row.get(5)?;
        Ok(FileRecord {
            file_path: PathBuf::from(row.get::<_, String>(0)?),
            file_size: row.get::<_, i64>(1)? as u64,
            mtime: row.get(2)?,
            sha1_hash: row.get(3)?,
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
        "SELECT file_path, file_size, mtime, sha1_hash, proton_id, sync_status FROM file_index WHERE file_path = ?1",
    )?;
    let record = statement
        .query_row(params![path_key(relative_path)], |row| {
            let status: String = row.get(5)?;
            Ok(FileRecord {
                file_path: PathBuf::from(row.get::<_, String>(0)?),
                file_size: row.get::<_, i64>(1)? as u64,
                mtime: row.get(2)?,
                sha1_hash: row.get(3)?,
                proton_id: row.get(4)?,
                sync_status: SyncStatus::from_str(&status).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, err)
                })?,
            })
        })
        .optional()?;
    Ok(record)
}

pub fn upsert_record(connection: &Connection, record: &FileRecord) -> AppResult<()> {
    connection.execute(
        r#"
        INSERT INTO file_index (file_path, file_size, mtime, sha1_hash, proton_id, sync_status)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(file_path) DO UPDATE SET
            file_size = excluded.file_size,
            mtime = excluded.mtime,
            sha1_hash = excluded.sha1_hash,
            proton_id = excluded.proton_id,
            sync_status = excluded.sync_status
        "#,
        params![
            path_key(&record.file_path),
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
        params![path_key(relative_path)],
    )?;
    Ok(())
}

pub fn purge_record(connection: &Connection, relative_path: &Path) -> AppResult<()> {
    connection.execute(
        "DELETE FROM file_index WHERE file_path = ?1",
        params![path_key(relative_path)],
    )?;
    Ok(())
}

pub fn scan_local_files(root: &Path) -> AppResult<HashMap<PathBuf, LocalFileState>> {
    let mut files = HashMap::new();
    visit_directory(root, root, &mut files)?;
    Ok(files)
}

fn visit_directory(
    root: &Path,
    directory: &Path,
    files: &mut HashMap<PathBuf, LocalFileState>,
) -> AppResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit_directory(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() || should_ignore_path(&path) {
            continue;
        }
        let state = local_file_state(root, &path)?;
        files.insert(state.relative_path.clone(), state);
    }
    Ok(())
}

pub fn should_ignore_path(path: &Path) -> bool {
    if crate::sync::is_conflict_copy(path) {
        return true;
    }
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("sync_index.db")
    )
}

pub fn local_file_state(root: &Path, absolute_path: &Path) -> AppResult<LocalFileState> {
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

    Ok(LocalFileState {
        relative_path: relative_path.to_path_buf(),
        absolute_path: absolute_path.to_path_buf(),
        file_size: metadata.len(),
        mtime,
        sha1_hash: compute_sha1(absolute_path)?,
    })
}

pub fn compute_sha1(path: &Path) -> AppResult<String> {
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
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
    fn database_round_trip_preserves_status() {
        let connection = Connection::open_in_memory().expect("connection");
        initialize_schema(&connection).expect("schema");
        let record = FileRecord {
            file_path: PathBuf::from("notes.txt"),
            file_size: 4,
            mtime: 123,
            sha1_hash: "abcd".to_owned(),
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
}
