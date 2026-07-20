use crate::index::{
    EntityKind, FileRecord, LocalDirectoryState, LocalEntityState, LocalFileState, SyncStatus,
};
use crate::proton::{RemoteDirectory, RemoteEntity, RemoteFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    Upload,
    Download,
    CreateRemoteDirectory,
    CreateLocalDirectory,
    MoveLocal,
    MoveRemote,
    AutoLink,
    Conflict,
    TypeConflict,
    RemoteDelete,
    LocalDelete,
    Purge,
    SkipUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedAction {
    pub path: PathBuf,
    pub destination_path: Option<PathBuf>,
    pub action: SyncAction,
    pub entity_kind: EntityKind,
    pub conflict_path: Option<PathBuf>,
    pub remote_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunReport {
    pub summary: PlanSummary,
    pub plan: Vec<PlannedAction>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSummary {
    pub total: usize,
    pub uploads: usize,
    pub downloads: usize,
    pub remote_directories_created: usize,
    pub local_directories_created: usize,
    pub local_moves: usize,
    pub remote_moves: usize,
    pub auto_links: usize,
    pub conflicts: usize,
    pub type_conflicts: usize,
    pub remote_deletes: usize,
    pub local_deletes: usize,
    pub purges: usize,
    pub skipped_unsupported: usize,
    pub destructive_actions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileDelta {
    Missing,
    Unchanged,
    Changed,
    /// Remote file exists but its hash is unavailable (e.g. no activeRevision digest).
    Unknown,
    /// Remote file exists but the Proton CLI cannot download it as bytes.
    Unsupported,
}

pub fn plan_sync(
    local_files: &HashMap<PathBuf, LocalFileState>,
    remote_files: &HashMap<PathBuf, RemoteFile>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Vec<PlannedAction> {
    let local_entities = local_files
        .iter()
        .map(|(path, file)| (path.clone(), LocalEntityState::File(file.clone())))
        .collect();
    let remote_entities = remote_files
        .iter()
        .map(|(path, file)| (path.clone(), RemoteEntity::File(file.clone())))
        .collect();
    plan_sync_entities(&local_entities, &remote_entities, base_index)
}

pub fn plan_sync_entities(
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Vec<PlannedAction> {
    let mut paths = BTreeSet::new();
    paths.extend(local_entities.keys().cloned());
    paths.extend(remote_entities.keys().cloned());
    paths.extend(base_index.keys().cloned());
    let (transition_actions, suppressed_paths) =
        plan_file_path_transitions(local_entities, remote_entities, base_index);

    let bootstrap = base_index.is_empty();
    let mut plan = transition_actions;
    plan.extend(
        paths
            .into_iter()
            .filter(|path| !suppressed_paths.contains(path))
            .filter_map(|path| {
                let local = local_entities.get(&path);
                let remote = remote_entities.get(&path);
                if is_type_conflict(local, remote, base_index.get(&path)) {
                    return Some(PlannedAction::new(
                        &path,
                        SyncAction::TypeConflict,
                        entity_kind_for_path(local, remote, base_index.get(&path)),
                        remote.and_then(RemoteEntity::remote_id),
                    ));
                }
                match base_index.get(&path) {
                    Some(base) if base.entity_kind == EntityKind::Directory && !bootstrap => {
                        plan_ongoing_directory_action(
                            &path,
                            local.and_then(LocalEntityState::as_directory),
                            remote.and_then(RemoteEntity::as_directory),
                            base,
                            local_entities,
                            remote_entities,
                            base_index,
                            &suppressed_paths,
                        )
                    }
                    Some(base) if !bootstrap => plan_ongoing_file_action(
                        &path,
                        local.and_then(LocalEntityState::as_file),
                        remote.and_then(RemoteEntity::as_file),
                        base,
                    ),
                    _ => plan_bootstrap_entity_action(&path, local, remote),
                }
            }),
    );
    suppress_actions_covered_by_directory_deletes(plan)
}

fn plan_file_path_transitions(
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> (Vec<PlannedAction>, BTreeSet<PathBuf>) {
    let mut actions = Vec::new();
    let mut suppressed_paths = BTreeSet::new();
    let mut paths: Vec<_> = base_index.keys().cloned().collect();
    paths.sort();

    for path in paths {
        if suppressed_paths.contains(&path) {
            continue;
        }
        let Some(base) = base_index.get(&path) else {
            continue;
        };
        let action =
            plan_remote_file_move(&path, local_entities, remote_entities, base_index, base)
                .or_else(|| {
                    plan_local_rename_as_remote_move(
                        &path,
                        local_entities,
                        remote_entities,
                        base_index,
                        base,
                    )
                });
        if let Some(action) = action {
            suppressed_paths.insert(action.path.clone());
            if let Some(destination_path) = action.destination_path.clone() {
                suppressed_paths.insert(destination_path);
            }
            actions.push(action);
        }
    }

    (actions, suppressed_paths)
}

fn plan_remote_file_move(
    old_path: &Path,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
    base: &FileRecord,
) -> Option<PlannedAction> {
    let base_hash = base_file_hash(base)?;
    let local = local_entities.get(old_path)?.as_file()?;
    if local.sha1_hash != base_hash || remote_entities.contains_key(old_path) {
        return None;
    }
    let (new_path, remote) = unique_remote_move_destination(
        old_path,
        base,
        base_hash,
        local_entities,
        remote_entities,
        base_index,
    )?;
    Some(PlannedAction::move_local(
        old_path,
        &new_path,
        Some(remote.id.clone()),
    ))
}

fn plan_local_rename_as_remote_move(
    old_path: &Path,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
    base: &FileRecord,
) -> Option<PlannedAction> {
    let base_hash = base_file_hash(base)?;
    if local_entities.contains_key(old_path) {
        return None;
    }
    let remote = remote_entities.get(old_path)?.as_file()?;
    if remote.sha1_hash.as_deref() != Some(base_hash) {
        return None;
    }
    let new_path = unique_local_move_destination(
        old_path,
        base_hash,
        local_entities,
        remote_entities,
        base_index,
    )?;
    Some(PlannedAction::move_remote(
        old_path,
        Some(&new_path),
        Some(remote.id.clone()),
    ))
}

fn base_file_hash(base: &FileRecord) -> Option<&str> {
    if base.entity_kind == EntityKind::File {
        base.sha1_hash.as_deref()
    } else {
        None
    }
}

fn unique_remote_move_destination<'a>(
    old_path: &Path,
    base: &FileRecord,
    base_hash: &str,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &'a HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Option<(PathBuf, &'a RemoteFile)> {
    let candidates: Vec<_> = remote_entities
        .iter()
        .filter_map(|(path, entity)| {
            let remote = entity.as_file()?;
            if path == old_path
                || local_entities.contains_key(path)
                || base_index.contains_key(path)
                || remote.sha1_hash.as_deref() != Some(base_hash)
            {
                return None;
            }
            if let Some(base_id) = base.proton_id.as_deref()
                && remote.id != base_id
            {
                return None;
            }
            Some((path.clone(), remote))
        })
        .collect();
    if candidates.len() == 1 {
        candidates.into_iter().next()
    } else {
        None
    }
}

fn unique_local_move_destination(
    old_path: &Path,
    base_hash: &str,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Option<PathBuf> {
    let candidates: Vec<_> = local_entities
        .iter()
        .filter_map(|(path, entity)| {
            let local = entity.as_file()?;
            if path == old_path
                || remote_entities.contains_key(path)
                || base_index.contains_key(path)
                || local.sha1_hash != base_hash
            {
                return None;
            }
            Some(path.clone())
        })
        .collect();
    if candidates.len() == 1 {
        candidates.into_iter().next()
    } else {
        None
    }
}

fn is_type_conflict(
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
    base: Option<&FileRecord>,
) -> bool {
    let local_kind = local.map(LocalEntityState::kind);
    let remote_kind = remote.map(remote_entity_kind);
    if local_kind.is_some() && remote_kind.is_some() && local_kind != remote_kind {
        return true;
    }
    let Some(base_kind) = base.map(|record| record.entity_kind) else {
        return false;
    };
    local_kind.is_some_and(|kind| kind != base_kind)
        || remote_kind.is_some_and(|kind| kind != base_kind)
}

fn entity_kind_for_path(
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
    base: Option<&FileRecord>,
) -> EntityKind {
    local
        .map(LocalEntityState::kind)
        .or_else(|| remote.map(remote_entity_kind))
        .or_else(|| base.map(|record| record.entity_kind))
        .unwrap_or(EntityKind::File)
}

fn remote_entity_kind(remote: &RemoteEntity) -> EntityKind {
    match remote {
        RemoteEntity::File(_) => EntityKind::File,
        RemoteEntity::Directory(_) => EntityKind::Directory,
    }
}

fn plan_bootstrap_entity_action(
    path: &Path,
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
) -> Option<PlannedAction> {
    match (local, remote) {
        (Some(LocalEntityState::Directory(_)), None) => Some(PlannedAction::new(
            path,
            SyncAction::CreateRemoteDirectory,
            EntityKind::Directory,
            None,
        )),
        (None, Some(RemoteEntity::Directory(remote))) => Some(PlannedAction::new(
            path,
            SyncAction::CreateLocalDirectory,
            EntityKind::Directory,
            remote.id.clone(),
        )),
        (Some(LocalEntityState::Directory(_)), Some(RemoteEntity::Directory(remote))) => {
            Some(PlannedAction::new(
                path,
                SyncAction::AutoLink,
                EntityKind::Directory,
                remote.id.clone(),
            ))
        }
        (Some(LocalEntityState::File(local)), remote) => {
            plan_bootstrap_file_action(path, Some(local), remote.and_then(RemoteEntity::as_file))
        }
        (None, Some(RemoteEntity::File(remote))) => {
            plan_bootstrap_file_action(path, None, Some(remote))
        }
        (Some(_), Some(remote)) => Some(PlannedAction::new(
            path,
            SyncAction::TypeConflict,
            entity_kind_for_path(local, Some(remote), None),
            remote.remote_id(),
        )),
        (None, None) => None,
    }
}

fn plan_bootstrap_file_action(
    path: &Path,
    local: Option<&LocalFileState>,
    remote: Option<&RemoteFile>,
) -> Option<PlannedAction> {
    match (local, remote) {
        (Some(_), None) => Some(PlannedAction::new(
            path,
            SyncAction::Upload,
            EntityKind::File,
            None,
        )),
        (None, Some(remote)) if remote.downloadable => Some(PlannedAction::new(
            path,
            SyncAction::Download,
            EntityKind::File,
            Some(remote.id.clone()),
        )),
        (None, Some(remote)) => Some(PlannedAction::new(
            path,
            SyncAction::SkipUnsupported,
            EntityKind::File,
            Some(remote.id.clone()),
        )),
        (Some(local), Some(remote)) => {
            if remote.sha1_hash.as_deref() == Some(local.sha1_hash.as_str()) {
                Some(PlannedAction::new(
                    path,
                    SyncAction::AutoLink,
                    EntityKind::File,
                    Some(remote.id.clone()),
                ))
            } else if remote.downloadable {
                Some(PlannedAction::conflict(path, Some(remote.id.clone())))
            } else {
                Some(PlannedAction::new(
                    path,
                    SyncAction::SkipUnsupported,
                    EntityKind::File,
                    Some(remote.id.clone()),
                ))
            }
        }
        (None, None) => None,
    }
}

fn plan_ongoing_file_action(
    path: &Path,
    local: Option<&LocalFileState>,
    remote: Option<&RemoteFile>,
    base: &FileRecord,
) -> Option<PlannedAction> {
    let base_hash = base.sha1_hash.as_deref()?;
    let local_delta = delta_from_base(local.map(|file| file.sha1_hash.as_str()), Some(base_hash));
    let remote_delta = remote_file_delta(remote, base);

    if base.sync_status == SyncStatus::Modified
        && let Some(action) =
            plan_modified_record_action(path, local, remote, base, local_delta, remote_delta)
    {
        return Some(action);
    }

    match (local_delta, remote_delta) {
        (FileDelta::Changed, FileDelta::Unchanged) => Some(PlannedAction::new(
            path,
            SyncAction::Upload,
            EntityKind::File,
            base.proton_id.clone(),
        )),
        (FileDelta::Unchanged, FileDelta::Changed) => Some(PlannedAction::new(
            path,
            SyncAction::Download,
            EntityKind::File,
            remote_id(remote, base),
        )),
        (FileDelta::Missing, FileDelta::Unchanged) => Some(PlannedAction::new(
            path,
            SyncAction::RemoteDelete,
            EntityKind::File,
            remote_id(remote, base),
        )),
        (FileDelta::Unchanged, FileDelta::Missing) => Some(PlannedAction::new(
            path,
            SyncAction::LocalDelete,
            EntityKind::File,
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
            EntityKind::File,
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
        (FileDelta::Unchanged, FileDelta::Unsupported)
        | (FileDelta::Changed, FileDelta::Unsupported)
        | (FileDelta::Missing, FileDelta::Unsupported) => Some(PlannedAction::new(
            path,
            SyncAction::SkipUnsupported,
            EntityKind::File,
            remote_id(remote, base),
        )),
        // Unknown is only ever produced for the remote delta; this arm is unreachable.
        (FileDelta::Unknown | FileDelta::Unsupported, _) => {
            unreachable!("local delta is never Unknown or Unsupported")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_ongoing_directory_action(
    path: &Path,
    local: Option<&LocalDirectoryState>,
    remote: Option<&RemoteDirectory>,
    base: &FileRecord,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
    suppressed_paths: &BTreeSet<PathBuf>,
) -> Option<PlannedAction> {
    match (local.is_some(), remote.is_some()) {
        (true, true) => None,
        // Directory still exists locally but is gone remotely. Recreate it remotely
        // unless every tracked descendant independently proves the whole subtree was
        // cleanly removed remotely, in which case propagate the deletion locally.
        (true, false) => {
            if directory_subtree_is_deletion_consistent(
                path,
                local_entities,
                remote_entities,
                base_index,
                suppressed_paths,
            ) {
                Some(PlannedAction::new(
                    path,
                    SyncAction::LocalDelete,
                    EntityKind::Directory,
                    base.proton_id.clone(),
                ))
            } else {
                Some(PlannedAction::new(
                    path,
                    SyncAction::CreateRemoteDirectory,
                    EntityKind::Directory,
                    base.proton_id.clone(),
                ))
            }
        }
        // Directory still exists remotely but is gone locally. Recreate it locally
        // unless the subtree proof shows the whole tree was cleanly removed locally,
        // in which case propagate the deletion remotely.
        (false, true) => {
            let remote_id = remote
                .and_then(|directory| directory.id.clone())
                .or_else(|| base.proton_id.clone());
            if directory_subtree_is_deletion_consistent(
                path,
                local_entities,
                remote_entities,
                base_index,
                suppressed_paths,
            ) {
                Some(PlannedAction::new(
                    path,
                    SyncAction::RemoteDelete,
                    EntityKind::Directory,
                    remote_id,
                ))
            } else {
                Some(PlannedAction::new(
                    path,
                    SyncAction::CreateLocalDirectory,
                    EntityKind::Directory,
                    remote_id,
                ))
            }
        }
        (false, false) => Some(PlannedAction::new(
            path,
            SyncAction::Purge,
            EntityKind::Directory,
            base.proton_id.clone(),
        )),
    }
}

/// Returns true only when every base-index descendant of `directory_path` independently
/// resolves to a deletion-consistent outcome (`RemoteDelete`, `LocalDelete`, or `Purge`).
/// This proves it is safe to propagate the directory's one-sided absence as a recursive
/// delete instead of recreating it; any descendant with a different resolution (upload,
/// download, conflict, auto-link, a directory recreate, an unsupported skip, or a path
/// transition) causes the proof to fail so the caller falls back to the non-destructive
/// recreate behavior.
fn directory_subtree_is_deletion_consistent(
    directory_path: &Path,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
    suppressed_paths: &BTreeSet<PathBuf>,
) -> bool {
    base_index
        .iter()
        .filter(|(path, _)| is_strict_descendant(directory_path, path))
        .all(|(path, base)| {
            if suppressed_paths.contains(path) {
                return false;
            }
            let local = local_entities.get(path);
            let remote = remote_entities.get(path);
            let action = match base.entity_kind {
                EntityKind::Directory => plan_ongoing_directory_action(
                    path,
                    local.and_then(LocalEntityState::as_directory),
                    remote.and_then(RemoteEntity::as_directory),
                    base,
                    local_entities,
                    remote_entities,
                    base_index,
                    suppressed_paths,
                ),
                EntityKind::File => plan_ongoing_file_action(
                    path,
                    local.and_then(LocalEntityState::as_file),
                    remote.and_then(RemoteEntity::as_file),
                    base,
                ),
            };
            matches!(
                action.map(|planned| planned.action),
                Some(SyncAction::RemoteDelete | SyncAction::LocalDelete | SyncAction::Purge)
            )
        })
}

/// Returns true when `candidate` is strictly nested under `ancestor` (never equal to it).
pub(crate) fn is_strict_descendant(ancestor: &Path, candidate: &Path) -> bool {
    candidate != ancestor && candidate.starts_with(ancestor)
}

/// Removes any planned action whose path is nested under a directory that is itself
/// planned for a recursive `RemoteDelete`/`LocalDelete`, since the recursive removal
/// already covers every descendant and re-applying an individual action for it would
/// be redundant (and could fail if the recursive removal already ran first).
fn suppress_actions_covered_by_directory_deletes(plan: Vec<PlannedAction>) -> Vec<PlannedAction> {
    let deleted_directories: Vec<PathBuf> = plan
        .iter()
        .filter(|action| {
            action.entity_kind == EntityKind::Directory
                && matches!(
                    action.action,
                    SyncAction::RemoteDelete | SyncAction::LocalDelete
                )
        })
        .map(|action| action.path.clone())
        .collect();
    if deleted_directories.is_empty() {
        return plan;
    }
    plan.into_iter()
        .filter(|action| {
            !deleted_directories
                .iter()
                .any(|directory| is_strict_descendant(directory, &action.path))
        })
        .collect()
}

fn plan_modified_record_action(
    path: &Path,
    local: Option<&LocalFileState>,
    remote: Option<&RemoteFile>,
    base: &FileRecord,
    local_delta: FileDelta,
    remote_delta: FileDelta,
) -> Option<PlannedAction> {
    local?;

    if base.proton_id.is_none() && remote.is_none() {
        return Some(PlannedAction::new(
            path,
            SyncAction::Upload,
            EntityKind::File,
            None,
        ));
    }

    match (local_delta, remote_delta) {
        (FileDelta::Unchanged, FileDelta::Unchanged) => Some(PlannedAction::new(
            path,
            SyncAction::AutoLink,
            EntityKind::File,
            remote_id(remote, base),
        )),
        (FileDelta::Unchanged, FileDelta::Changed | FileDelta::Missing) => {
            Some(PlannedAction::new(
                path,
                SyncAction::Upload,
                EntityKind::File,
                remote_id(remote, base),
            ))
        }
        (FileDelta::Unchanged, FileDelta::Unknown) => {
            Some(PlannedAction::conflict(path, remote_id(remote, base)))
        }
        (FileDelta::Unchanged, FileDelta::Unsupported) => Some(PlannedAction::new(
            path,
            SyncAction::SkipUnsupported,
            EntityKind::File,
            remote_id(remote, base),
        )),
        _ => None,
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
    let Some(base_hash) = base.sha1_hash.as_deref() else {
        return FileDelta::Unknown;
    };
    match remote {
        None => FileDelta::Missing,
        Some(file) if !file.downloadable => FileDelta::Unsupported,
        Some(file) => match file.sha1_hash.as_deref() {
            None => FileDelta::Unknown,
            Some(hash) if hash == base_hash => FileDelta::Unchanged,
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
    fn new(
        path: &Path,
        action: SyncAction,
        entity_kind: EntityKind,
        remote_id: Option<String>,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: None,
            action,
            entity_kind,
            conflict_path: None,
            remote_id,
        }
    }

    fn conflict(path: &Path, remote_id: Option<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: None,
            action: SyncAction::Conflict,
            entity_kind: EntityKind::File,
            conflict_path: Some(conflict_copy_path(path)),
            remote_id,
        }
    }

    fn move_local(path: &Path, destination_path: &Path, remote_id: Option<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: Some(destination_path.to_path_buf()),
            action: SyncAction::MoveLocal,
            entity_kind: EntityKind::File,
            conflict_path: None,
            remote_id,
        }
    }

    fn move_remote(
        path: &Path,
        destination_path: Option<&Path>,
        remote_id: Option<String>,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: destination_path.map(Path::to_path_buf),
            action: SyncAction::MoveRemote,
            entity_kind: EntityKind::File,
            conflict_path: None,
            remote_id,
        }
    }
}

impl DryRunReport {
    pub fn new(plan: Vec<PlannedAction>) -> Self {
        Self {
            summary: PlanSummary::from_plan(&plan),
            plan,
        }
    }
}

impl PlanSummary {
    pub fn from_plan(plan: &[PlannedAction]) -> Self {
        let mut summary = Self {
            total: plan.len(),
            ..Self::default()
        };
        for action in plan {
            match action.action {
                SyncAction::Upload => summary.uploads += 1,
                SyncAction::Download => summary.downloads += 1,
                SyncAction::CreateRemoteDirectory => summary.remote_directories_created += 1,
                SyncAction::CreateLocalDirectory => summary.local_directories_created += 1,
                SyncAction::MoveLocal => summary.local_moves += 1,
                SyncAction::MoveRemote => summary.remote_moves += 1,
                SyncAction::AutoLink => summary.auto_links += 1,
                SyncAction::Conflict => summary.conflicts += 1,
                SyncAction::TypeConflict => summary.type_conflicts += 1,
                SyncAction::RemoteDelete => summary.remote_deletes += 1,
                SyncAction::LocalDelete => summary.local_deletes += 1,
                SyncAction::Purge => summary.purges += 1,
                SyncAction::SkipUnsupported => summary.skipped_unsupported += 1,
            }
        }
        summary.destructive_actions =
            summary.remote_deletes + summary.local_deletes + summary.purges;
        summary
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
    use crate::index::{EntityKind, FileRecord, LocalDirectoryState, LocalFileState, SyncStatus};
    use crate::proton::{RemoteDirectory, RemoteEntity, RemoteFile};

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
            downloadable: true,
        }
    }

    fn unsupported_remote(path: &str, id: &str) -> RemoteFile {
        RemoteFile {
            downloadable: false,
            ..remote(path, id, None)
        }
    }

    fn local_directory(path: &str) -> LocalEntityState {
        LocalEntityState::Directory(LocalDirectoryState {
            relative_path: PathBuf::from(path),
            absolute_path: PathBuf::from(format!("/tmp/{path}")),
            mtime: 1,
        })
    }

    fn remote_directory(path: &str, id: Option<&str>) -> RemoteEntity {
        RemoteEntity::Directory(RemoteDirectory {
            path: PathBuf::from(path),
            id: id.map(ToOwned::to_owned),
            name: Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_owned(),
        })
    }

    fn base(path: &str, id: Option<&str>, hash: &str) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::File,
            file_size: 1,
            mtime: 1,
            sha1_hash: Some(hash.to_owned()),
            proton_id: id.map(ToOwned::to_owned),
            sync_status: SyncStatus::Synced,
        }
    }

    fn modified_base(path: &str, id: Option<&str>, hash: &str) -> FileRecord {
        FileRecord {
            sync_status: SyncStatus::Modified,
            ..base(path, id, hash)
        }
    }

    fn directory_base(path: &str, id: Option<&str>) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::Directory,
            file_size: 0,
            mtime: 1,
            sha1_hash: None,
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

    #[test]
    fn modified_new_local_record_uploads_instead_of_deleting_local_file() {
        let mut local_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(PathBuf::from("new.txt"), local("new.txt", "new-hash"));
        base_index.insert(
            PathBuf::from("new.txt"),
            modified_base("new.txt", None, "new-hash"),
        );

        let planned = plan_sync(&local_files, &HashMap::new(), &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].path, PathBuf::from("new.txt"));
        assert_eq!(planned[0].action, SyncAction::Upload);
        assert_eq!(planned[0].remote_id, None);
    }

    #[test]
    fn modified_conflict_resolution_uploads_local_when_remote_differs() {
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(
            PathBuf::from("notes.txt"),
            local("notes.txt", "local-resolution"),
        );
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("remote-conflict")),
        );
        base_index.insert(
            PathBuf::from("notes.txt"),
            modified_base("notes.txt", Some("remote-id"), "local-resolution"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].path, PathBuf::from("notes.txt"));
        assert_eq!(planned[0].action, SyncAction::Upload);
        assert_eq!(planned[0].remote_id.as_deref(), Some("remote-id"));
    }

    #[test]
    fn modified_existing_file_with_remote_change_still_conflicts() {
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(PathBuf::from("notes.txt"), local("notes.txt", "local-new"));
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("remote-new")),
        );
        base_index.insert(
            PathBuf::from("notes.txt"),
            modified_base("notes.txt", Some("remote-id"), "base-hash"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].path, PathBuf::from("notes.txt"));
        assert_eq!(planned[0].action, SyncAction::Conflict);
        assert_eq!(planned[0].remote_id.as_deref(), Some("remote-id"));
    }

    #[test]
    fn unsupported_remote_file_is_skipped_without_download_or_conflict() {
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("Untitled spreadsheet"),
            unsupported_remote("Untitled spreadsheet", "sheet-id"),
        );

        let planned = plan_sync(&HashMap::new(), &remote_files, &HashMap::new());

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action, SyncAction::SkipUnsupported);
        assert_eq!(planned[0].remote_id.as_deref(), Some("sheet-id"));

        let mut local_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(
            PathBuf::from("Untitled spreadsheet"),
            local("Untitled spreadsheet", "local-hash"),
        );
        base_index.insert(
            PathBuf::from("Untitled spreadsheet"),
            base("Untitled spreadsheet", Some("sheet-id"), "base-hash"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action, SyncAction::SkipUnsupported);
        assert_eq!(planned[0].remote_id.as_deref(), Some("sheet-id"));
    }

    #[test]
    fn directory_entities_plan_safe_create_and_purge_actions() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("local-empty"), local_directory("local-empty"));
        remote_entities.insert(
            PathBuf::from("remote-empty"),
            remote_directory("remote-empty", Some("remote-dir-id")),
        );
        local_entities.insert(PathBuf::from("linked"), local_directory("linked"));
        remote_entities.insert(
            PathBuf::from("linked"),
            remote_directory("linked", Some("linked-id")),
        );
        base_index.insert(
            PathBuf::from("deleted-empty"),
            directory_base("deleted-empty", Some("deleted-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(planned.iter().any(|action| {
            action.path == Path::new("local-empty")
                && action.action == SyncAction::CreateRemoteDirectory
                && action.entity_kind == EntityKind::Directory
        }));
        assert!(planned.iter().any(|action| {
            action.path == Path::new("remote-empty")
                && action.action == SyncAction::CreateLocalDirectory
                && action.remote_id.as_deref() == Some("remote-dir-id")
        }));
        assert!(planned.iter().any(|action| {
            action.path == Path::new("linked")
                && action.action == SyncAction::AutoLink
                && action.entity_kind == EntityKind::Directory
        }));
        assert!(planned.iter().any(|action| {
            action.path == Path::new("deleted-empty") && action.action == SyncAction::Purge
        }));
    }

    #[test]
    fn clean_directory_deletion_propagates_recursively_to_remote() {
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        // "docs" and its only child were previously synced; both are now gone locally
        // (e.g. `rm -rf docs`) while the remote side is unchanged.
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/report.txt"),
            RemoteEntity::File(remote("docs/report.txt", "report-id", Some("same-hash"))),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/report.txt"),
            base("docs/report.txt", Some("report-id"), "same-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(
            planned.len(),
            1,
            "the descendant file's own delete should be suppressed by the recursive \
             directory delete: {planned:?}"
        );
        assert_eq!(planned[0].path, PathBuf::from("docs"));
        assert_eq!(planned[0].action, SyncAction::RemoteDelete);
        assert_eq!(planned[0].entity_kind, EntityKind::Directory);
        assert_eq!(planned[0].remote_id.as_deref(), Some("docs-id"));
    }

    #[test]
    fn clean_nested_directory_deletion_propagates_through_multiple_levels() {
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        // "docs" contains "docs/archive", which contains "docs/archive/report.txt".
        // The whole tree was removed locally in one operation, so only the topmost
        // directory should be recursively deleted remotely.
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/archive"),
            remote_directory("docs/archive", Some("archive-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/archive/report.txt"),
            RemoteEntity::File(remote(
                "docs/archive/report.txt",
                "report-id",
                Some("same-hash"),
            )),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/archive"),
            directory_base("docs/archive", Some("archive-id")),
        );
        base_index.insert(
            PathBuf::from("docs/archive/report.txt"),
            base("docs/archive/report.txt", Some("report-id"), "same-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(
            planned.len(),
            1,
            "only the top-level directory delete should remain after suppressing \
             covered descendants: {planned:?}"
        );
        assert_eq!(planned[0].path, PathBuf::from("docs"));
        assert_eq!(planned[0].action, SyncAction::RemoteDelete);
    }

    #[test]
    fn directory_deletion_falls_back_to_recreate_when_descendant_diverges() {
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        // "docs" is gone locally, but its child file was independently modified on
        // the remote side after the last sync, so this is a genuine conflict rather
        // than a clean subtree deletion. The directory must not be recursively
        // deleted out from under the diverging descendant.
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/report.txt"),
            RemoteEntity::File(remote("docs/report.txt", "report-id", Some("changed-hash"))),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/report.txt"),
            base("docs/report.txt", Some("report-id"), "original-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::CreateLocalDirectory),
            "an ambiguous descendant must keep the directory recreate fallback: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/report.txt")
                    && action.action == SyncAction::Conflict),
            "the diverging descendant should still be reported as its own conflict: {planned:?}"
        );
    }

    #[test]
    fn mixed_file_and_directory_entities_plan_type_conflict() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        local_entities.insert(PathBuf::from("same-name"), local_directory("same-name"));
        remote_entities.insert(
            PathBuf::from("same-name"),
            RemoteEntity::File(remote("same-name", "file-id", Some("hash"))),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &HashMap::new());

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].path, PathBuf::from("same-name"));
        assert_eq!(planned[0].action, SyncAction::TypeConflict);
        assert_eq!(planned[0].remote_id.as_deref(), Some("file-id"));
    }

    #[test]
    fn remote_file_rename_plans_local_move_when_id_and_hash_are_unique() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(
            PathBuf::from("old-name.txt"),
            LocalEntityState::File(local("old-name.txt", "same-hash")),
        );
        remote_entities.insert(
            PathBuf::from("new-name.txt"),
            RemoteEntity::File(remote("new-name.txt", "stable-id", Some("same-hash"))),
        );
        base_index.insert(
            PathBuf::from("old-name.txt"),
            base("old-name.txt", Some("stable-id"), "same-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action, SyncAction::MoveLocal);
        assert_eq!(planned[0].path, PathBuf::from("old-name.txt"));
        assert_eq!(
            planned[0].destination_path.as_deref(),
            Some(Path::new("new-name.txt"))
        );
        assert_eq!(planned[0].remote_id.as_deref(), Some("stable-id"));
    }

    #[test]
    fn local_file_rename_plans_verified_remote_move() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(
            PathBuf::from("new-name.txt"),
            LocalEntityState::File(local("new-name.txt", "same-hash")),
        );
        remote_entities.insert(
            PathBuf::from("old-name.txt"),
            RemoteEntity::File(remote("old-name.txt", "stable-id", Some("same-hash"))),
        );
        base_index.insert(
            PathBuf::from("old-name.txt"),
            base("old-name.txt", Some("stable-id"), "same-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action, SyncAction::MoveRemote);
        assert_eq!(planned[0].path, PathBuf::from("old-name.txt"));
        assert_eq!(
            planned[0].destination_path.as_deref(),
            Some(Path::new("new-name.txt"))
        );
        assert_eq!(planned[0].remote_id.as_deref(), Some("stable-id"));
    }

    #[test]
    fn ambiguous_file_rename_candidates_are_not_inferred() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(
            PathBuf::from("old-name.txt"),
            LocalEntityState::File(local("old-name.txt", "same-hash")),
        );
        remote_entities.insert(
            PathBuf::from("candidate-a.txt"),
            RemoteEntity::File(remote("candidate-a.txt", "candidate-a", Some("same-hash"))),
        );
        remote_entities.insert(
            PathBuf::from("candidate-b.txt"),
            RemoteEntity::File(remote("candidate-b.txt", "candidate-b", Some("same-hash"))),
        );
        base_index.insert(
            PathBuf::from("old-name.txt"),
            base("old-name.txt", None, "same-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            planned
                .iter()
                .all(|action| action.action != SyncAction::MoveLocal),
            "ambiguous content matches must not be inferred as a rename: {planned:?}"
        );
    }

    #[test]
    fn remote_delete_uses_live_remote_id_when_base_id_is_missing() {
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        remote_files.insert(
            PathBuf::from("orphan-id.txt"),
            remote("orphan-id.txt", "remote-id", Some("old-hash")),
        );
        base_index.insert(
            PathBuf::from("orphan-id.txt"),
            base("orphan-id.txt", None, "old-hash"),
        );

        let planned = plan_sync(&HashMap::new(), &remote_files, &base_index);

        let action = planned
            .iter()
            .find(|action| action.path == Path::new("orphan-id.txt"))
            .expect("remote delete action should be planned");
        assert_eq!(action.action, SyncAction::RemoteDelete);
        assert_eq!(action.remote_id.as_deref(), Some("remote-id"));
    }

    #[test]
    fn planned_action_serializes_for_dry_run_output() {
        let action = PlannedAction::conflict(Path::new("notes.txt"), Some("remote-id".to_owned()));

        let json = serde_json::to_string(&action).expect("serialize planned action");

        assert!(
            json.contains(r#""action":"conflict""#),
            "dry-run JSON should expose a stable snake_case action name"
        );
        assert!(
            json.contains(r#""conflict_path":"notes.proton-cloud.txt""#),
            "dry-run JSON should expose the planned conflict destination"
        );
        assert!(
            json.contains(r#""remote_id":"remote-id""#),
            "dry-run JSON should include the remote identifier when available"
        );
    }

    #[test]
    fn dry_run_report_summarizes_planned_actions() {
        let report = DryRunReport::new(vec![
            PlannedAction::new(
                Path::new("upload.txt"),
                SyncAction::Upload,
                EntityKind::File,
                None,
            ),
            PlannedAction::new(
                Path::new("download.txt"),
                SyncAction::Download,
                EntityKind::File,
                Some("remote-id".to_owned()),
            ),
            PlannedAction::new(
                Path::new("delete.txt"),
                SyncAction::RemoteDelete,
                EntityKind::File,
                Some("delete-id".to_owned()),
            ),
            PlannedAction::conflict(Path::new("conflict.txt"), Some("conflict-id".to_owned())),
            PlannedAction::new(
                Path::new("sheet"),
                SyncAction::SkipUnsupported,
                EntityKind::File,
                Some("sheet-id".to_owned()),
            ),
        ]);

        assert_eq!(report.summary.total, 5);
        assert_eq!(report.summary.uploads, 1);
        assert_eq!(report.summary.downloads, 1);
        assert_eq!(report.summary.remote_deletes, 1);
        assert_eq!(report.summary.conflicts, 1);
        assert_eq!(report.summary.skipped_unsupported, 1);
        assert_eq!(report.summary.destructive_actions, 1);
        assert_eq!(report.plan.len(), 5);
    }
}
