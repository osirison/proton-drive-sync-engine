use crate::index::{FileRecord, LocalFileState};
use crate::proton::RemoteFile;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    Upload,
    Download,
    AutoLink,
    Conflict,
    RemoteDelete,
    LocalDelete,
    Purge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub path: PathBuf,
    pub action: SyncAction,
    pub conflict_path: Option<PathBuf>,
    pub remote_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileDelta {
    Missing,
    Unchanged,
    Changed,
    /// Remote file exists but its hash is unavailable (e.g. no activeRevision digest).
    Unknown,
}

pub fn plan_sync(
    local_files: &HashMap<PathBuf, LocalFileState>,
    remote_files: &HashMap<PathBuf, RemoteFile>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Vec<PlannedAction> {
    let mut paths = BTreeSet::new();
    paths.extend(local_files.keys().cloned());
    paths.extend(remote_files.keys().cloned());
    paths.extend(base_index.keys().cloned());

    let bootstrap = base_index.is_empty();
    paths
        .into_iter()
        .filter_map(|path| {
            let local = local_files.get(&path);
            let remote = remote_files.get(&path);
            match base_index.get(&path) {
                Some(base) if !bootstrap => plan_ongoing_action(&path, local, remote, base),
                _ => plan_bootstrap_action(&path, local, remote),
            }
        })
        .collect()
}

fn plan_bootstrap_action(
    path: &Path,
    local: Option<&LocalFileState>,
    remote: Option<&RemoteFile>,
) -> Option<PlannedAction> {
    match (local, remote) {
        (Some(_), None) => Some(PlannedAction::new(path, SyncAction::Upload, None)),
        (None, Some(remote)) => Some(PlannedAction::new(
            path,
            SyncAction::Download,
            Some(remote.id.clone()),
        )),
        (Some(local), Some(remote)) => {
            if remote.sha1_hash.as_deref() == Some(local.sha1_hash.as_str()) {
                Some(PlannedAction::new(
                    path,
                    SyncAction::AutoLink,
                    Some(remote.id.clone()),
                ))
            } else {
                Some(PlannedAction::conflict(path, Some(remote.id.clone())))
            }
        }
        (None, None) => None,
    }
}

fn plan_ongoing_action(
    path: &Path,
    local: Option<&LocalFileState>,
    remote: Option<&RemoteFile>,
    base: &FileRecord,
) -> Option<PlannedAction> {
    let local_delta = delta_from_base(
        local.map(|file| file.sha1_hash.as_str()),
        Some(base.sha1_hash.as_str()),
    );
    let remote_delta = remote_file_delta(remote, base);

    match (local_delta, remote_delta) {
        (FileDelta::Changed, FileDelta::Unchanged) => Some(PlannedAction::new(
            path,
            SyncAction::Upload,
            base.proton_id.clone(),
        )),
        (FileDelta::Unchanged, FileDelta::Changed) => Some(PlannedAction::new(
            path,
            SyncAction::Download,
            remote_id(remote, base),
        )),
        (FileDelta::Missing, FileDelta::Unchanged) => Some(PlannedAction::new(
            path,
            SyncAction::RemoteDelete,
            base.proton_id.clone(),
        )),
        (FileDelta::Unchanged, FileDelta::Missing) => Some(PlannedAction::new(
            path,
            SyncAction::LocalDelete,
            base.proton_id.clone(),
        )),
        (FileDelta::Changed, FileDelta::Changed)
        | (FileDelta::Changed, FileDelta::Missing)
        | (FileDelta::Missing, FileDelta::Changed) => {
            Some(PlannedAction::conflict(path, remote_id(remote, base)))
        }
        (FileDelta::Missing, FileDelta::Missing) => Some(PlannedAction::new(
            path,
            SyncAction::Purge,
            base.proton_id.clone(),
        )),
        (FileDelta::Unchanged, FileDelta::Unchanged) => None,
        // Remote file is present but its hash is unavailable – apply non-destructive handling
        // to avoid destroying local or remote data based on incomplete information.
        (FileDelta::Unchanged, FileDelta::Unknown) => None,
        (FileDelta::Changed, FileDelta::Unknown) => {
            Some(PlannedAction::conflict(path, remote_id(remote, base)))
        }
        (FileDelta::Missing, FileDelta::Unknown) => None,
        // Unknown is only ever produced for the remote delta; this arm is unreachable.
        (FileDelta::Unknown, _) => None,
    }
}

fn delta_from_base(current_hash: Option<&str>, base_hash: Option<&str>) -> FileDelta {
    match (current_hash, base_hash) {
        (Some(current), Some(base)) if current == base => FileDelta::Unchanged,
        (Some(_), _) => FileDelta::Changed,
        (None, Some(_)) => FileDelta::Missing,
        (None, None) => FileDelta::Missing,
    }
}

/// Compute a remote file's delta against the base index, correctly distinguishing
/// between a remote file that is absent (`Missing`) and one that exists but whose
/// hash is unavailable (`Unknown`).
fn remote_file_delta(remote: Option<&RemoteFile>, base: &FileRecord) -> FileDelta {
    match remote {
        None => FileDelta::Missing,
        Some(file) => match file.sha1_hash.as_deref() {
            None => FileDelta::Unknown,
            Some(hash) if hash == base.sha1_hash.as_str() => FileDelta::Unchanged,
            Some(_) => FileDelta::Changed,
        },
    }
}

fn remote_id(remote: Option<&RemoteFile>, base: &FileRecord) -> Option<String> {
    remote
        .map(|file| file.id.clone())
        .or_else(|| base.proton_id.clone())
}

impl PlannedAction {
    fn new(path: &Path, action: SyncAction, remote_id: Option<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            action,
            conflict_path: None,
            remote_id,
        }
    }

    fn conflict(path: &Path, remote_id: Option<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            action: SyncAction::Conflict,
            conflict_path: Some(conflict_copy_path(path)),
            remote_id,
        }
    }
}

pub fn conflict_copy_path(path: &Path) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path.file_stem().and_then(|value| value.to_str());
    let extension = path.extension().and_then(|value| value.to_str());

    let file_name = match (stem, extension) {
        (Some(stem), Some(extension)) => format!("{stem}.proton-cloud.{extension}"),
        _ => match path.file_name().and_then(|value| value.to_str()) {
            Some(name) => format!("{name}.proton-cloud"),
            None => "proton-cloud".to_owned(),
        },
    };

    parent.join(file_name)
}

pub fn is_conflict_copy(path: &Path) -> bool {
    match path.file_name().and_then(|value| value.to_str()) {
        Some(name) => name.contains(".proton-cloud.") || name.ends_with(".proton-cloud"),
        None => false,
    }
}

pub fn original_from_conflict_copy(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    if let Some((stem, extension)) = file_name.rsplit_once(".proton-cloud.") {
        return Some(
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
                .join(format!("{stem}.{extension}")),
        );
    }
    let stem = file_name.strip_suffix(".proton-cloud")?;
    Some(
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join(stem),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{FileRecord, LocalFileState, SyncStatus};
    use crate::proton::RemoteFile;

    fn local(path: &str, hash: &str) -> LocalFileState {
        LocalFileState {
            relative_path: PathBuf::from(path),
            absolute_path: PathBuf::from(format!("/tmp/{path}")),
            file_size: 1,
            mtime: 1,
            sha1_hash: hash.to_owned(),
        }
    }

    fn remote(path: &str, id: &str, hash: Option<&str>) -> RemoteFile {
        RemoteFile {
            path: PathBuf::from(path),
            id: id.to_owned(),
            name: Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_owned(),
            sha1_hash: hash.map(ToOwned::to_owned),
        }
    }

    fn base(path: &str, id: Option<&str>, hash: &str) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            file_size: 1,
            mtime: 1,
            sha1_hash: hash.to_owned(),
            proton_id: id.map(ToOwned::to_owned),
            sync_status: SyncStatus::Synced,
        }
    }

    #[test]
    fn creates_conflict_copy_names() {
        assert_eq!(
            conflict_copy_path(Path::new("notes.txt")),
            PathBuf::from("notes.proton-cloud.txt")
        );
        assert_eq!(
            original_from_conflict_copy(Path::new("notes.proton-cloud.txt")),
            Some(PathBuf::from("notes.txt"))
        );
        assert!(is_conflict_copy(Path::new("nested/notes.proton-cloud.txt")));
    }

    #[test]
    fn bootstrap_detects_conflicts_and_links() {
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        local_files.insert(PathBuf::from("notes.txt"), local("notes.txt", "same"));
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "id-1", Some("same")),
        );
        remote_files.insert(
            PathBuf::from("draft.txt"),
            remote("draft.txt", "id-2", Some("remote")),
        );
        local_files.insert(
            PathBuf::from("conflict.txt"),
            local("conflict.txt", "local"),
        );
        remote_files.insert(
            PathBuf::from("conflict.txt"),
            remote("conflict.txt", "id-3", Some("remote")),
        );

        let planned = plan_sync(&local_files, &remote_files, &HashMap::new());

        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("notes.txt")
                    && action.action == SyncAction::AutoLink)
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("draft.txt")
                    && action.action == SyncAction::Download)
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("conflict.txt")
                    && action.action == SyncAction::Conflict)
        );
    }

    #[test]
    fn ongoing_matrix_prefers_expected_actions() {
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();

        local_files.insert(
            PathBuf::from("upload.txt"),
            local("upload.txt", "new-local"),
        );
        remote_files.insert(
            PathBuf::from("upload.txt"),
            remote("upload.txt", "id-upload", Some("old")),
        );
        base_index.insert(
            PathBuf::from("upload.txt"),
            base("upload.txt", Some("id-upload"), "old"),
        );

        local_files.insert(PathBuf::from("download.txt"), local("download.txt", "same"));
        remote_files.insert(
            PathBuf::from("download.txt"),
            remote("download.txt", "id-download", Some("fresh")),
        );
        base_index.insert(
            PathBuf::from("download.txt"),
            base("download.txt", Some("id-download"), "same"),
        );

        base_index.insert(
            PathBuf::from("delete-remote.txt"),
            base("delete-remote.txt", Some("id-delete-remote"), "same"),
        );
        remote_files.insert(
            PathBuf::from("delete-remote.txt"),
            remote("delete-remote.txt", "id-delete-remote", Some("same")),
        );

        local_files.insert(
            PathBuf::from("delete-local.txt"),
            local("delete-local.txt", "same"),
        );
        base_index.insert(
            PathBuf::from("delete-local.txt"),
            base("delete-local.txt", Some("id-delete-local"), "same"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("upload.txt")
                    && action.action == SyncAction::Upload)
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("download.txt")
                    && action.action == SyncAction::Download)
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("delete-local.txt")
                    && action.action == SyncAction::LocalDelete)
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("delete-remote.txt")
                    && action.action == SyncAction::RemoteDelete)
        );
    }

    #[test]
    fn remote_present_without_hash_is_non_destructive() {
        // A remote entry that exists but has no activeRevision digest must never
        // trigger LocalDelete or Purge, regardless of local state.

        // Case 1: local unchanged, remote present without hash → no action.
        {
            let mut local_files = HashMap::new();
            let mut remote_files = HashMap::new();
            let mut base_index = HashMap::new();
            local_files.insert(PathBuf::from("stable.txt"), local("stable.txt", "hash"));
            remote_files.insert(
                PathBuf::from("stable.txt"),
                remote("stable.txt", "id-1", None),
            );
            base_index.insert(
                PathBuf::from("stable.txt"),
                base("stable.txt", Some("id-1"), "hash"),
            );
            let planned = plan_sync(&local_files, &remote_files, &base_index);
            assert!(
                !planned.iter().any(|a| a.path == Path::new("stable.txt")
                    && matches!(a.action, SyncAction::LocalDelete | SyncAction::Purge)),
                "unchanged local + remote-no-hash must not delete local data"
            );
            assert!(
                planned.iter().all(|a| a.path != Path::new("stable.txt")),
                "unchanged local + remote-no-hash must produce no action"
            );
        }

        // Case 2: local changed, remote present without hash → conflict (safe).
        {
            let mut local_files = HashMap::new();
            let mut remote_files = HashMap::new();
            let mut base_index = HashMap::new();
            local_files.insert(PathBuf::from("edited.txt"), local("edited.txt", "new-hash"));
            remote_files.insert(
                PathBuf::from("edited.txt"),
                remote("edited.txt", "id-2", None),
            );
            base_index.insert(
                PathBuf::from("edited.txt"),
                base("edited.txt", Some("id-2"), "old-hash"),
            );
            let planned = plan_sync(&local_files, &remote_files, &base_index);
            assert!(
                planned
                    .iter()
                    .any(|a| a.path == Path::new("edited.txt") && a.action == SyncAction::Conflict),
                "changed local + remote-no-hash must resolve to Conflict"
            );
        }

        // Case 3: local missing, remote present without hash → no destructive action.
        {
            let mut remote_files = HashMap::new();
            let mut base_index = HashMap::new();
            remote_files.insert(PathBuf::from("gone.txt"), remote("gone.txt", "id-3", None));
            base_index.insert(
                PathBuf::from("gone.txt"),
                base("gone.txt", Some("id-3"), "hash"),
            );
            let planned = plan_sync(&HashMap::new(), &remote_files, &base_index);
            assert!(
                !planned.iter().any(|a| a.path == Path::new("gone.txt")
                    && matches!(
                        a.action,
                        SyncAction::RemoteDelete | SyncAction::Purge | SyncAction::LocalDelete
                    )),
                "missing local + remote-no-hash must not destroy remote data"
            );
        }
    }
}
