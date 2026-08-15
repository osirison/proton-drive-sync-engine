use crate::index::{
    EntityKind, FileRecord, LocalDirectoryState, LocalEntityState, LocalFileState, SyncStatus,
};
use crate::proton::{RemoteDirectory, RemoteEntity, RemoteFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedAction {
    pub path: PathBuf,
    pub destination_path: Option<PathBuf>,
    pub action: SyncAction,
    pub entity_kind: EntityKind,
    pub conflict_path: Option<PathBuf>,
    pub remote_id: Option<String>,
    /// How the executor must materialize `conflict_path`: `false` = download the remote node,
    /// `true` = copy the surviving local file (the remote node is confirmed gone, so a download
    /// would fail every pass). Only meaningful when `conflict_path` is set.
    /// `#[serde(default)]` for wire compat with dry-run output written before this field existed.
    #[serde(default)]
    pub sidecar_from_local_copy: bool,
}

/// The direction a deletion propagates, used by the delete-approval guard, the persistent
/// approval store, and the control protocol. It is the stable identity that distinguishes the
/// two data-losing actions (see [`SyncAction::delete_direction`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteDirection {
    /// Propagate a *local* deletion by deleting the copy on Proton Drive (`RemoteDelete`).
    Remote,
    /// Propagate a *remote* deletion/trash by deleting the copy on the local disk (`LocalDelete`).
    Local,
}

impl DeleteDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Local => "local",
        }
    }
}

impl std::fmt::Display for DeleteDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DeleteDirection {
    type Err = Box<dyn std::error::Error + Send + Sync>;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "remote" => Ok(Self::Remote),
            "local" => Ok(Self::Local),
            other => Err(crate::boxed_error(format!(
                "unknown delete direction: {other}"
            ))),
        }
    }
}

impl SyncAction {
    /// The deletion direction this action propagates, or `None` for non-destructive actions and
    /// for `Purge` (index-only cleanup that destroys no user data and is never gated).
    pub fn delete_direction(self) -> Option<DeleteDirection> {
        match self {
            Self::RemoteDelete => Some(DeleteDirection::Remote),
            Self::LocalDelete => Some(DeleteDirection::Local),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    let deletion_verdicts = compute_directory_deletion_verdicts(
        local_entities,
        remote_entities,
        base_index,
        &suppressed_paths,
    );
    let mut plan = transition_actions;
    plan.extend(
        paths
            .into_iter()
            .filter(|path| !suppressed_paths.contains(path))
            .filter_map(|path| {
                plan_entity_action(
                    &path,
                    local_entities.get(&path),
                    remote_entities.get(&path),
                    base_index.get(&path),
                    bootstrap,
                    &deletion_verdicts,
                )
            }),
    );
    suppress_actions_covered_by_directory_deletes(plan)
}

/// Resolves a single path from the live entities and the base row together. This is the one
/// definition of "what does this path plan to"; [`compute_directory_deletion_verdicts`] proves
/// its subtrees through it so a directory delete is only ever proved against the actions that
/// will really be planned. Dispatching the proof on `base.entity_kind` alone narrowed a live
/// entity of the *other* kind away as absent on both sides, scored it a clean `Purge`, and proved
/// a recursive delete that destroyed the only copy of it (#282).
fn plan_entity_action(
    path: &Path,
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
    base: Option<&FileRecord>,
    bootstrap: bool,
    deletion_verdicts: &HashMap<PathBuf, bool>,
) -> Option<PlannedAction> {
    if !is_representable_remotely(path) {
        // #270. The path cannot cross the CLI's JSON boundary intact, so nothing that touches the
        // remote may be planned for it — not an upload (whose copy comes back under the lossy key
        // and forks), not a delete of a node this key can never name, not a conflict whose sidecar
        // has no remote revision to fetch. Reported rather than dropped: the entity is real and a
        // user has to know it is not being synced (#232). Gating HERE rather than at the call site
        // is what makes the deletion proof inherit it — `SkipUnsupported` is not a delete, so a
        // subtree holding one can never be proved clean, and the recursive delete that would
        // otherwise destroy this file's only copy is never authorised.
        return Some(PlannedAction::new(
            path,
            SyncAction::SkipUnsupported,
            entity_kind_for_path(local, remote, base),
            None,
        ));
    }
    if is_reconciled_directory_file_clash(local, remote, base) {
        // A directory already permanently claimed this path against a clashing remote file on a
        // prior reconcile (see SD-09); the remote file's continued presence is a tolerated,
        // already resolved state, not a fresh conflict to reprocess every pass.
        return None;
    }
    if is_live_kind_clash(local, remote) {
        return Some(plan_type_conflict_action(path, local, remote, base));
    }
    if only_base_kind_is_stale(local, remote, base) {
        // No live clash — the surviving side(s) agree and it is the *base* row that still
        // describes the old kind (a synced directory deleted everywhere, then a file uploaded
        // under the same name, and the mirror cases). Plan the surviving entity's bootstrap
        // action: its upsert replaces the stale row wholesale (`entity_kind` included), so the
        // path converges in THIS pass (#47).
        return plan_bootstrap_entity_action(path, local, remote);
    }
    match base {
        Some(base) if base.entity_kind == EntityKind::Directory && !bootstrap => {
            plan_ongoing_directory_action(
                path,
                local.and_then(LocalEntityState::as_directory),
                remote.and_then(RemoteEntity::as_directory),
                base,
                deletion_verdicts,
            )
        }
        Some(base) if !bootstrap => plan_ongoing_file_action(
            path,
            local.and_then(LocalEntityState::as_file),
            remote.and_then(RemoteEntity::as_file),
            base,
        ),
        _ => plan_bootstrap_entity_action(path, local, remote),
    }
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
                })
                .or_else(|| {
                    plan_remote_directory_move(
                        &path,
                        local_entities,
                        remote_entities,
                        base_index,
                        base,
                    )
                })
                // #270: a `MoveRemote` renames the remote node to the *local* path's name, so an
                // unrepresentable destination would put a name on the remote that the listing can
                // only echo back lossily — the same fork, one move over. Dropping the pairing
                // hands both halves back to ordinary planning: the new path is reported
                // unsyncable, and the old one takes the approval-gated deletion path any
                // vanished local file takes. A `MoveLocal`'s destination comes from the remote map
                // and is UTF-8 by construction, so this never drops one. That leaves one
                // intentional case: an already-forked legacy pair (a base row on the real bytes,
                // its `proton_id` on the lossily-named node) plans a MoveLocal that renames the
                // local file onto the remote's name. The engine renaming a user's file is
                // deliberate here — it is what makes the file syncable again, under the only name
                // both sides can agree on, and the id match is what proves the pair.
                .filter(|action| {
                    action
                        .destination_path
                        .as_deref()
                        .is_none_or(is_representable_remotely)
                });
        if let Some(action) = action {
            suppressed_paths.insert(action.path.clone());
            if let Some(destination_path) = action.destination_path.clone() {
                suppressed_paths.insert(destination_path.clone());
                if action.entity_kind == EntityKind::Directory {
                    for (old_descendant, new_descendant) in directory_move_descendant_path_pairs(
                        &action.path,
                        &destination_path,
                        base_index,
                    ) {
                        suppressed_paths.insert(old_descendant);
                        suppressed_paths.insert(new_descendant);
                    }
                    // …and the descendants NO base row tracks (#12). A `MoveLocal` renames the
                    // directory on disk before the per-path actions run, so anything planned for a
                    // live descendant at the OLD path is planned against a local state that will
                    // not be there: the upload would first recreate the moved-away parent remotely
                    // (its parent is a move source, not a planned create) and then read a file that
                    // has moved. There is nothing to re-plan at the new path either — the pre-move
                    // scan does not know that key — so the whole subtree is left to the next pass,
                    // which sees it where it now is. The executor re-queues the destination so that
                    // pass cannot idle-skip.
                    for descendant in local_entities.keys() {
                        if !base_index.contains_key(descendant)
                            && descendant.starts_with(&action.path)
                            && descendant != &action.path
                        {
                            suppressed_paths.insert(descendant.clone());
                        }
                    }
                }
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
        EntityKind::File,
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
        EntityKind::File,
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

/// Detects a directory that was renamed or moved on the remote side and converges the
/// local side to match via a zero-mutation local filesystem rename, mirroring
/// `plan_remote_file_move`. Directories have no content hash, so the base record's
/// `proton_id` (see the backfill logic in `plan_ongoing_directory_action`) is the sole
/// and required matching key; a directory whose id was never backfilled correctly,
/// conservatively never matches here and keeps today's non-destructive recreate
/// fallback.
///
/// Local-to-remote directory rename detection (the inverse direction) is a deliberate
/// non-goal: a local directory has no stable identity of its own (unlike a file, which
/// can be matched by content hash), so detecting "this new local directory is the same
/// one that used to be at the old path" purely from local state is not reliable to infer
/// with the same rigor used everywhere else in this planner. See
/// `docs/rename-detection-design.md` for the full rationale.
fn plan_remote_directory_move(
    old_path: &Path,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
    base: &FileRecord,
) -> Option<PlannedAction> {
    let base_id = base_directory_id(base)?;
    local_entities.get(old_path)?.as_directory()?;
    if remote_entities.contains_key(old_path) {
        return None;
    }
    let new_path = unique_remote_directory_move_destination(
        old_path,
        base_id,
        local_entities,
        remote_entities,
        base_index,
    )?;
    Some(PlannedAction::move_local(
        old_path,
        &new_path,
        EntityKind::Directory,
        Some(base_id.to_owned()),
    ))
}

fn base_directory_id(base: &FileRecord) -> Option<&str> {
    if base.entity_kind == EntityKind::Directory {
        base.proton_id.as_deref()
    } else {
        None
    }
}

fn unique_remote_directory_move_destination(
    old_path: &Path,
    base_id: &str,
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Option<PathBuf> {
    let candidates: Vec<_> = remote_entities
        .iter()
        .filter_map(|(path, entity)| {
            let remote = entity.as_directory()?;
            if path == old_path
                || local_entities.contains_key(path)
                || base_index.contains_key(path)
                || remote.id.as_deref() != Some(base_id)
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

/// For every base-index path strictly nested under `old_path`, compute the path it
/// should move to once `old_path` itself moves to `new_path`, preserving the relative
/// suffix under the directory. Used both to suppress ordinary per-path planning for a
/// moved directory's descendants (`plan_file_path_transitions`) and to rewrite their
/// index rows at execution time (`Daemon::reconcile_blocking_inner`), so the two stay
/// perfectly consistent with each other.
pub(crate) fn directory_move_descendant_path_pairs(
    old_path: &Path,
    new_path: &Path,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Vec<(PathBuf, PathBuf)> {
    base_index
        .keys()
        .filter(|path| is_strict_descendant(old_path, path))
        .filter_map(|path| {
            let relative = path.strip_prefix(old_path).ok()?;
            Some((path.clone(), new_path.join(relative)))
        })
        .collect()
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

/// True when both sides are live and disagree about the kind — the only genuine type conflict,
/// since both claimants exist right now and neither can be adopted without losing the other.
fn is_live_kind_clash(local: Option<&LocalEntityState>, remote: Option<&RemoteEntity>) -> bool {
    match (
        local.map(LocalEntityState::kind),
        remote.map(remote_entity_kind),
    ) {
        (Some(local_kind), Some(remote_kind)) => local_kind != remote_kind,
        _ => false,
    }
}

/// True when the live side(s) do not clash with each other but the *base* row records a different
/// kind — a stale index row, not a conflict. Callers must rule out [`is_live_kind_clash`] first.
/// Comparing the sole live side against the stale base kind used to emit a `TypeConflict` the
/// daemon could only warn about, so the path never synced again (#47).
fn only_base_kind_is_stale(
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
    base: Option<&FileRecord>,
) -> bool {
    let Some(base_kind) = base.map(|record| record.entity_kind) else {
        return false;
    };
    local
        .map(LocalEntityState::kind)
        .is_some_and(|kind| kind != base_kind)
        || remote
            .map(remote_entity_kind)
            .is_some_and(|kind| kind != base_kind)
}

/// True once a local directory has already permanently claimed `path` against a
/// persistently clashing remote file: the base record already agrees with the
/// local directory's kind, so the remote file's continued presence here is a
/// tolerated, already-resolved state (see `plan_type_conflict_action`) rather
/// than a fresh type conflict to reprocess on every reconcile.
fn is_reconciled_directory_file_clash(
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
    base: Option<&FileRecord>,
) -> bool {
    matches!(local, Some(LocalEntityState::Directory(_)))
        && matches!(remote, Some(RemoteEntity::File(_)))
        && base.is_some_and(|record| record.entity_kind == EntityKind::Directory)
}

/// Plans the action for a path where the local and remote entity kinds
/// conflict. A local directory clashing with a same-named remote file (SD-09
/// in the E2E test-spec matrix) keeps the local directory and preserves the
/// clashing remote file's content as a separately tracked conflict sidecar
/// outside the directory, instead of silently discarding it. Every other kind
/// mismatch keeps the existing non-mutating skip behavior.
fn plan_type_conflict_action(
    path: &Path,
    local: Option<&LocalEntityState>,
    remote: Option<&RemoteEntity>,
    base: Option<&FileRecord>,
) -> PlannedAction {
    if let (Some(LocalEntityState::Directory(_)), Some(RemoteEntity::File(_))) = (local, remote) {
        return PlannedAction::type_conflict_with_sidecar(
            path,
            remote.and_then(RemoteEntity::remote_id),
        );
    }
    PlannedAction::new(
        path,
        SyncAction::TypeConflict,
        entity_kind_for_path(local, remote, base),
        remote.and_then(RemoteEntity::remote_id),
    )
}

/// Whether a relative path can survive a round trip through the remote side (#270).
///
/// The `proton-drive` CLI reports the remote tree as JSON, so every path in the remote map is a
/// UTF-8 string by construction. A local path that is not valid UTF-8 — legal on every Unix
/// filesystem, where a name is bytes — therefore comes back with U+FFFD in place of the offending
/// bytes, under a key that can never equal the one the local scan produced. That is not a CLI
/// limitation a better upload would fix: whatever the bytes do on the way out, the listing can
/// only ever echo a UTF-8 string back, so the engine has no way to match the uploaded copy to the
/// local file. Left ungated the planner saw two unrelated paths — the real one local-only
/// (`Upload`), the lossy one remote-only (`Download`) — and multiplied the pair every pass.
///
/// So such a path is *unsyncable*, and the planner says so (`SkipUnsupported`) instead of planning
/// transfers that cannot converge. The check is one-sided on purpose: it constrains the local
/// side, never the remote one. A remote name containing a genuine U+FFFD is indistinguishable
/// from a lossily-reported one, and refusing both would strand a real file — while allowing them
/// is what lets an already-forked pair settle, since the lossy copy downloads to a UTF-8 local
/// name and auto-links from then on.
pub(crate) fn is_representable_remotely(path: &Path) -> bool {
    path.to_str().is_some()
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

    // An unresolved conflict sidecar already exists on disk; leave the base record
    // alone until the user resolves it (editing or removing the sidecar marks the
    // record `Modified`, which re-enters the ordinary planning flow above on the next
    // reconcile). Without this early return, the very next reconcile would see local
    // unchanged / remote changed and plan an ordinary `Download` straight over the
    // original file, silently destroying the local content the sidecar protects.
    // Exception: when the conflicted file is gone on BOTH sides there is nothing left
    // to protect, and holding the record would leave a zombie row no user action can
    // clear (deleting the original file emits no sidecar event) — fall through to the
    // ordinary `(Missing, Missing)` -> `Purge` arm instead.
    if base.sync_status == SyncStatus::Conflict
        && !(local_delta == FileDelta::Missing && remote_delta == FileDelta::Missing)
    {
        return None;
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
        // Both sides diverged from the baseline. If they nonetheless reached
        // byte-identical content — two independent identical edits, or a transfer that
        // landed on disk but whose index checkpoint never committed (a crash or failure
        // between the side effect and its checkpoint; index writes still never precede
        // their side effect) — they already agree,
        // so adopt the shared content as the new baseline via AutoLink instead of
        // fabricating a spurious `.proton-cloud` sidecar and a permanently stuck Conflict
        // record. Mirrors the identical-content handling in `plan_bootstrap_file_action`.
        (FileDelta::Changed, FileDelta::Changed) => match (local, remote) {
            (Some(local), Some(remote))
                if remote.sha1_hash.as_deref() == Some(local.sha1_hash.as_str()) =>
            {
                Some(PlannedAction::new(
                    path,
                    SyncAction::AutoLink,
                    EntityKind::File,
                    remote_id(Some(remote), base),
                ))
            }
            _ => Some(PlannedAction::conflict(path, remote_id(remote, base))),
        },
        // Local was deleted while the remote was edited since the baseline. There is no
        // local content left to preserve, so a `.proton-cloud` sidecar would only strand
        // the remote's sole surviving copy under an odd name — and, with no local file to
        // upsert, the daemon could never record the resolution, re-downloading that
        // sidecar on every reconcile forever. Treat the remote edit as authoritative and
        // restore it at the original path; the delete/edit ambiguity resolves in favor of
        // the surviving content and converges in a single pass. A user who still wants it
        // gone can delete it again once the remote is unchanged, which then propagates
        // cleanly as `(Missing, Unchanged)` -> RemoteDelete.
        (FileDelta::Missing, FileDelta::Changed) => Some(PlannedAction::new(
            path,
            SyncAction::Download,
            EntityKind::File,
            remote_id(remote, base),
        )),
        // Remote is confirmed missing (not merely unknown), so there is nothing to
        // download for a conflict sidecar; `remote_id(remote, base)` would otherwise
        // fall back to the stale base id and guarantee every reconcile attempt fails
        // trying to download a file that is already gone. The local edit stays in place
        // and the sidecar is materialized from a COPY of it instead: the remote delete is
        // a real user action on another client, so it is never silently reverted by an
        // implicit re-upload, but the state still gets the ordinary sidecar exit (delete
        // the sidecar -> `Modified` -> `(Unchanged, Missing)` -> Upload) and the on-disk
        // artefact the GUI conflicts list walks for. Without it the path froze forever (#46).
        (FileDelta::Changed, FileDelta::Missing) => Some(PlannedAction::conflict_from_local_copy(
            path,
            base.proton_id.clone(),
        )),
        (FileDelta::Missing, FileDelta::Missing) => Some(PlannedAction::new(
            path,
            SyncAction::Purge,
            EntityKind::File,
            base.proton_id.clone(),
        )),
        // Remote file is present but its hash is unavailable – apply non-destructive
        // handling to avoid destroying local or remote data based on incomplete
        // information. Still backfill a missing/stale proton_id when the remote
        // listing exposes one (mirroring the directory-level auto-link behavior),
        // since discovering the id is safe even when the hash can't be compared yet.
        (FileDelta::Unchanged, FileDelta::Unchanged)
        | (FileDelta::Unchanged, FileDelta::Unknown) => {
            let linked_id = remote_id(remote, base);
            // An empty id is the reconstruction placeholder for "not yet backfilled"
            // (`reconstruct::remote_entity_from_record`), not a real remote id — linking
            // it would commit `proton_id = Some("")` to the index.
            if linked_id.as_deref().is_some_and(|id| !id.is_empty()) && linked_id != base.proton_id
            {
                Some(PlannedAction::new(
                    path,
                    SyncAction::AutoLink,
                    EntityKind::File,
                    linked_id,
                ))
            } else {
                None
            }
        }
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

fn plan_ongoing_directory_action(
    path: &Path,
    local: Option<&LocalDirectoryState>,
    remote: Option<&RemoteDirectory>,
    base: &FileRecord,
    deletion_verdicts: &HashMap<PathBuf, bool>,
) -> Option<PlannedAction> {
    match (local.is_some(), remote.is_some()) {
        // Both sides already agree the directory exists. Backfill the base record's
        // `proton_id` when the remote listing now exposes an id that the base record
        // does not already have recorded (this is the only time a locally-first-created
        // directory's id is ever discovered, since `ensure_directory` itself returns no
        // id). Once the ids already match, no further action is emitted.
        (true, true) => {
            let remote_id = remote.and_then(|directory| directory.id.clone());
            if remote_id.is_some() && remote_id != base.proton_id {
                Some(PlannedAction::new(
                    path,
                    SyncAction::AutoLink,
                    EntityKind::Directory,
                    remote_id,
                ))
            } else {
                None
            }
        }
        // Directory still exists locally but is gone remotely. Recreate it remotely
        // unless every tracked descendant independently proves the whole subtree was
        // cleanly removed remotely, in which case propagate the deletion locally.
        (true, false) => {
            if subtree_is_deletion_consistent(path, deletion_verdicts) {
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
            if subtree_is_deletion_consistent(path, deletion_verdicts) {
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

/// Reads the memoized verdict for whether `directory_path`'s whole subtree resolves to a
/// clean one-sided deletion (see [`compute_directory_deletion_verdicts`]). A missing entry
/// fails toward the non-destructive recreate behavior.
fn subtree_is_deletion_consistent(
    directory_path: &Path,
    deletion_verdicts: &HashMap<PathBuf, bool>,
) -> bool {
    deletion_verdicts
        .get(directory_path)
        .copied()
        .unwrap_or(false)
}

/// Precomputes, for every base-index directory, whether its whole subtree resolves to a
/// clean one-sided deletion: every descendant independently resolves to `RemoteDelete`,
/// `LocalDelete`, or `Purge` (any other resolution — upload, download, conflict,
/// auto-link, a directory recreate, an unsupported skip, or a path transition — fails the
/// proof so the caller falls back to the non-destructive recreate).
///
/// One bottom-up pass over the base index: entries are visited deepest-first and each one's own
/// resolution is folded into its nearest *tracked* ancestor, so every entry is planned exactly
/// once and a failure propagates up the ancestor chain of its own accord. The previous shape
/// re-scanned the whole base index per directory and re-planned every descendant once per
/// ancestor level — Θ(D·N) path-prefix checks on every reconcile, idle ones included (#48).
fn compute_directory_deletion_verdicts(
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
    suppressed_paths: &BTreeSet<PathBuf>,
) -> HashMap<PathBuf, bool> {
    if !any_base_directory_is_one_sided(local_entities, remote_entities, base_index) {
        // Nothing can read a verdict this pass, so proving anything is dead work. This is the
        // common case: an idle pass has no one-sided directory at all (#48).
        return HashMap::new();
    }

    // `subtree_clean[P]`: every base entry under `P` resolves to a one-sided delete/purge and no
    // live entity under it is untracked. Seeded optimistic, falsified by a failing descendant.
    let mut subtree_clean: HashMap<&Path, bool> = base_index
        .keys()
        .map(|path| (path.as_path(), true))
        .collect();

    // An entity live on either side with no base record (created since the last sync) resolves to
    // a bootstrap Upload/Download, and suppression would then drop that action as "covered" by
    // the recursive delete, destroying the only copy. Falsify its nearest tracked ancestor; every
    // ancestor above inherits it when that entry is folded in below.
    for path in local_entities.keys().chain(remote_entities.keys()) {
        if base_index.contains_key(path) {
            continue;
        }
        if let Some(ancestor) = nearest_tracked_ancestor(path, base_index) {
            subtree_clean.insert(ancestor, false);
        }
    }

    let mut entries: Vec<(&PathBuf, &FileRecord)> = base_index.iter().collect();
    // Deepest first: an entry always has more path components than its nearest tracked ancestor,
    // so every entry is final before the ancestor that folds it in is visited.
    entries.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));

    let bootstrap = base_index.is_empty();
    let mut verdicts: HashMap<PathBuf, bool> = HashMap::new();
    for (path, base) in entries {
        let clean = subtree_clean.get(path.as_path()).copied().unwrap_or(false);
        // Publish before planning: a one-sided directory's own resolution reads THIS verdict.
        if base.entity_kind == EntityKind::Directory {
            verdicts.insert(path.clone(), clean);
        }
        let resolves_to_deletion = !suppressed_paths.contains(path)
            && matches!(
                plan_entity_action(
                    path,
                    local_entities.get(path),
                    remote_entities.get(path),
                    Some(base),
                    bootstrap,
                    &verdicts,
                )
                .map(|planned| planned.action),
                Some(SyncAction::RemoteDelete | SyncAction::LocalDelete | SyncAction::Purge)
            );
        if !(clean && resolves_to_deletion)
            && let Some(ancestor) = nearest_tracked_ancestor(path, base_index)
        {
            subtree_clean.insert(ancestor, false);
        }
    }
    verdicts
}

/// True when some base directory is live on exactly one side — the only shape that reads a
/// deletion verdict, since `plan_ongoing_directory_action`'s one-sided arms are the sole callers
/// of [`subtree_is_deletion_consistent`]. It narrows the two sides exactly as those arms do, so
/// it can only over-approximate what is actually consumed, and an unconsulted verdict reads as
/// `false` — the non-destructive recreate. A third consumer would have to widen this gate.
fn any_base_directory_is_one_sided(
    local_entities: &HashMap<PathBuf, LocalEntityState>,
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> bool {
    base_index
        .iter()
        .filter(|(_, record)| record.entity_kind == EntityKind::Directory)
        .any(|(path, _)| {
            local_entities
                .get(path)
                .and_then(LocalEntityState::as_directory)
                .is_some()
                != remote_entities
                    .get(path)
                    .and_then(RemoteEntity::as_directory)
                    .is_some()
        })
}

/// The closest strict ancestor of `path` carrying a base record. Selective-sync filters can leave
/// gaps in the base index, so this walks past untracked intermediate directories instead of
/// assuming the immediate parent is tracked — a gap would otherwise detach a whole subtree from
/// the proof and let it be deleted unexamined.
fn nearest_tracked_ancestor<'a>(
    path: &Path,
    base_index: &'a HashMap<PathBuf, FileRecord>,
) -> Option<&'a Path> {
    path.ancestors().skip(1).find_map(|ancestor| {
        base_index
            .get_key_value(ancestor)
            .map(|(tracked, _)| tracked.as_path())
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
    pub(crate) fn new(
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
            sidecar_from_local_copy: false,
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
            sidecar_from_local_copy: false,
        }
    }

    /// Like `conflict`, but the sidecar is materialized by copying the surviving *local* file:
    /// the remote node is confirmed missing (not merely unknown), so there is nothing to
    /// download and `remote_id` only carries the dead base id forward for the index record.
    /// The sidecar is what gives this state an exit (delete it -> `Modified` -> upload) and the
    /// artefact the GUI conflicts list finds by walking the disk; without it the path silently
    /// stops syncing forever (#46).
    fn conflict_from_local_copy(path: &Path, remote_id: Option<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: None,
            action: SyncAction::Conflict,
            entity_kind: EntityKind::File,
            conflict_path: Some(conflict_copy_path(path)),
            remote_id,
            sidecar_from_local_copy: true,
        }
    }

    /// A local directory permanently keeps `path` against a same-named remote
    /// file (SD-09): `conflict_path` points at the sidecar location the
    /// clashing remote file's content is downloaded to instead of being
    /// silently discarded.
    fn type_conflict_with_sidecar(path: &Path, remote_id: Option<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: None,
            action: SyncAction::TypeConflict,
            entity_kind: EntityKind::Directory,
            conflict_path: Some(conflict_copy_path(path)),
            remote_id,
            sidecar_from_local_copy: false,
        }
    }

    fn move_local(
        path: &Path,
        destination_path: &Path,
        entity_kind: EntityKind,
        remote_id: Option<String>,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: Some(destination_path.to_path_buf()),
            action: SyncAction::MoveLocal,
            entity_kind,
            conflict_path: None,
            remote_id,
            sidecar_from_local_copy: false,
        }
    }

    fn move_remote(
        path: &Path,
        destination_path: Option<&Path>,
        entity_kind: EntityKind,
        remote_id: Option<String>,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            destination_path: destination_path.map(Path::to_path_buf),
            action: SyncAction::MoveRemote,
            entity_kind,
            conflict_path: None,
            remote_id,
            sidecar_from_local_copy: false,
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

    let Some(file_name) = path.file_name() else {
        return parent.join("proton-cloud");
    };

    if let Some(name) = file_name.to_str() {
        let stem = path.file_stem().and_then(|value| value.to_str());
        let extension = path.extension().and_then(|value| value.to_str());
        let renamed = match (stem, extension) {
            (Some(stem), Some(extension)) => format!("{stem}.proton-cloud.{extension}"),
            _ => format!("{name}.proton-cloud"),
        };
        return parent.join(renamed);
    }

    // `file_name` is not valid UTF-8: fall back to a byte-safe transform instead of
    // collapsing every non-UTF-8 name onto the same fixed "proton-cloud" literal,
    // which would make two different non-UTF-8-named conflicts collide onto the same
    // sidecar path.
    parent.join(conflict_copy_os_string(file_name))
}

/// Byte-safe fallback used by `conflict_copy_path` when `file_name` is not valid
/// UTF-8. Splices the same `.proton-cloud.`/`.proton-cloud` marker used by the UTF-8
/// fast path into the raw bytes, so distinct non-UTF-8 names never collapse onto the
/// same sidecar path.
fn conflict_copy_os_string(file_name: &OsStr) -> OsString {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let bytes = file_name.as_bytes();
    let mut result = Vec::new();
    match bytes.iter().rposition(|&byte| byte == b'.') {
        // A leading dot (e.g. ".env") is not treated as an extension separator here,
        // matching `Path::extension`'s own behavior for dotfiles.
        Some(index) if index > 0 => {
            result.extend_from_slice(&bytes[..index]);
            result.extend_from_slice(b".proton-cloud.");
            result.extend_from_slice(&bytes[index + 1..]);
        }
        _ => {
            result.extend_from_slice(bytes);
            result.extend_from_slice(b".proton-cloud");
        }
    }
    OsString::from_vec(result)
}

pub fn is_conflict_copy(path: &Path) -> bool {
    let Some(file_name) = path.file_name() else {
        return false;
    };
    // `to_string_lossy` is safe here: the ASCII marker below always survives lossy
    // conversion unchanged, since only genuinely invalid byte spans elsewhere in the
    // name are replaced with the Unicode replacement character.
    let name = file_name.to_string_lossy();
    name.contains(".proton-cloud.") || name.ends_with(".proton-cloud")
}

pub fn original_from_conflict_copy(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?;
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();

    if let Some(name) = file_name.to_str() {
        if let Some((stem, extension)) = name.rsplit_once(".proton-cloud.") {
            return Some(parent.join(format!("{stem}.{extension}")));
        }
        let stem = name.strip_suffix(".proton-cloud")?;
        return Some(parent.join(stem));
    }

    original_from_conflict_copy_os_string(file_name).map(|name| parent.join(name))
}

/// Byte-safe fallback used by `original_from_conflict_copy` when `file_name` is not
/// valid UTF-8. Mirrors `conflict_copy_os_string`'s marker placement in reverse so a
/// conflict sidecar for a non-UTF-8-named original file can still be recognized and
/// resolved.
fn original_from_conflict_copy_os_string(file_name: &OsStr) -> Option<OsString> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let bytes = file_name.as_bytes();
    const MID_MARKER: &[u8] = b".proton-cloud.";
    if let Some(index) = bytes
        .windows(MID_MARKER.len())
        .rposition(|window| window == MID_MARKER)
    {
        let mut result = Vec::new();
        result.extend_from_slice(&bytes[..index]);
        result.push(b'.');
        result.extend_from_slice(&bytes[index + MID_MARKER.len()..]);
        return Some(OsString::from_vec(result));
    }

    const SUFFIX_MARKER: &[u8] = b".proton-cloud";
    if bytes.ends_with(SUFFIX_MARKER) {
        return Some(OsString::from_vec(
            bytes[..bytes.len() - SUFFIX_MARKER.len()].to_vec(),
        ));
    }

    None
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

    /// A relative path holding a byte sequence that is not valid UTF-8, and the path the remote
    /// listing reports for the same file: the CLI speaks JSON, so the name comes back with U+FFFD
    /// where those bytes were. The two are different keys, which is the whole of #270.
    #[cfg(unix)]
    fn non_utf8_and_lossy_paths() -> (PathBuf, PathBuf) {
        use std::os::unix::ffi::OsStrExt;

        let real = PathBuf::from(OsStr::from_bytes(b"caf\xe9.txt"));
        let lossy = PathBuf::from(real.to_string_lossy().into_owned());
        assert_ne!(real, lossy, "the two keys must actually differ");
        (real, lossy)
    }

    #[cfg(unix)]
    fn local_at(path: &Path, hash: &str) -> LocalFileState {
        LocalFileState {
            relative_path: path.to_path_buf(),
            absolute_path: Path::new("/tmp").join(path),
            file_size: 1,
            mtime: 1,
            sha1_hash: hash.to_owned(),
        }
    }

    #[cfg(unix)]
    fn base_at(path: &Path, id: Option<&str>, hash: &str) -> FileRecord {
        FileRecord {
            file_path: path.to_path_buf(),
            ..base("placeholder", id, hash)
        }
    }

    // #270: the remote listing arrives as JSON, so a non-UTF-8 filename comes back lossy and the
    // planner saw two unrelated paths — the real one local-only (Upload) and the lossy one
    // remote-only (Download). Each pass then uploaded a second copy under the lossy name and
    // downloaded it back. The round trip is impossible by construction, not by CLI limitation:
    // whatever the upload does with the bytes, the listing can only ever echo a UTF-8 string, so
    // the engine could never match the result back to the local file. Such a path is unsyncable.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_filename_is_skipped_instead_of_forking_into_an_upload_and_a_download() {
        let (real, lossy) = non_utf8_and_lossy_paths();
        let mut local_files = HashMap::new();
        local_files.insert(real.clone(), local_at(&real, "hash-a"));

        let planned = plan_sync(&local_files, &HashMap::new(), &HashMap::new());

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].path, real);
        assert_eq!(
            planned[0].action,
            SyncAction::SkipUnsupported,
            "an unrepresentable path must not be uploaded: the copy it creates comes back under a \
             different key and forks"
        );

        // The lossy copy an earlier build already uploaded is an ordinary remote-only file: it
        // downloads and then auto-links, which is how a forked pair stops multiplying. No
        // heuristic hunts for U+FFFD in remote names — a name that genuinely contains one is
        // indistinguishable from a lossy one, and refusing both would strand a real file.
        let mut remote_files = HashMap::new();
        let lossy_name = lossy
            .to_str()
            .expect("a lossy path is UTF-8 by construction");
        remote_files.insert(
            lossy.clone(),
            remote(lossy_name, "remote-id", Some("hash-a")),
        );

        let planned = plan_sync(&local_files, &remote_files, &HashMap::new());

        let actions: Vec<_> = planned
            .iter()
            .map(|planned| (planned.path.clone(), planned.action))
            .collect();
        assert!(
            actions.contains(&(real.clone(), SyncAction::SkipUnsupported)),
            "unexpected plan: {actions:?}"
        );
        assert!(
            actions.contains(&(lossy, SyncAction::Download)),
            "unexpected plan: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|(_, action)| *action == SyncAction::Upload),
            "the fork's upload half must be gone: {actions:?}"
        );
    }

    // What the gate's placement buys. `compute_directory_deletion_verdicts` proves a subtree
    // through `plan_entity_action`, so gating inside that function — rather than at its
    // `plan_sync_entities` call site — is what makes the proof inherit the gate: `SkipUnsupported`
    // is not a delete, so the subtree never scores clean. Ungated, the tracked non-UTF-8 child
    // below reads as remotely deleted (its lossy remote key never matches), the proof passes, and
    // the recursive `LocalDelete` it authorises swallows the child's own action — deleting the only
    // copy of a file this engine has never been able to upload. This test is the guard on that
    // inheritance, and fails with exactly that `LocalDelete` of the parent when the gate is gone.
    #[cfg(unix)]
    #[test]
    fn a_directory_holding_a_non_utf8_file_is_never_proved_safe_to_delete() {
        let (real, _) = non_utf8_and_lossy_paths();
        let nested = PathBuf::from("docs").join(&real);
        let mut local_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        local_entities.insert(
            nested.clone(),
            LocalEntityState::File(local_at(&nested, "hash-a")),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("dir-id")),
        );
        base_index.insert(nested.clone(), base_at(&nested, Some("file-id"), "hash-a"));

        let planned = plan_sync_entities(&local_entities, &HashMap::new(), &base_index);

        assert!(
            !planned
                .iter()
                .any(|action| action.action == SyncAction::LocalDelete),
            "the local file is the only copy — nothing here may delete it: {planned:?}"
        );
        assert!(
            planned.iter().any(|action| action.path == nested
                && action.action == SyncAction::SkipUnsupported),
            "the unsyncable child must still be reported: {planned:?}"
        );
    }

    // A local rename onto an unrepresentable name used to pair into a `MoveRemote`, which would
    // rename the remote node to a name the next listing can only echo back lossily — the same fork
    // one move over. Dropping the pairing hands both halves to ordinary planning.
    #[cfg(unix)]
    #[test]
    fn a_rename_onto_a_non_utf8_name_is_not_moved_onto_the_remote() {
        let (real, _) = non_utf8_and_lossy_paths();
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(real.clone(), local_at(&real, "hash-a"));
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("hash-a")),
        );
        base_index.insert(
            PathBuf::from("notes.txt"),
            base("notes.txt", Some("remote-id"), "hash-a"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        let actions: Vec<_> = planned
            .iter()
            .map(|planned| (planned.path.clone(), planned.action))
            .collect();
        assert!(
            !actions
                .iter()
                .any(|(_, action)| *action == SyncAction::MoveRemote),
            "unexpected plan: {actions:?}"
        );
        assert!(
            actions.contains(&(real, SyncAction::SkipUnsupported)),
            "unexpected plan: {actions:?}"
        );
        assert!(
            actions.contains(&(PathBuf::from("notes.txt"), SyncAction::RemoteDelete)),
            "the vanished local file takes the ordinary, approval-gated deletion path: {actions:?}"
        );
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

    #[cfg(unix)]
    #[test]
    fn conflict_copy_naming_is_byte_safe_for_non_utf8_file_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let name_a = PathBuf::from(OsStr::from_bytes(b"fo\x80o.txt"));
        let name_b = PathBuf::from(OsStr::from_bytes(b"fo\x81o.txt"));
        assert_eq!(
            name_a.to_string_lossy(),
            name_b.to_string_lossy(),
            "test paths must actually collide under lossy UTF-8 conversion for this \
             test to be meaningful"
        );

        let copy_a = conflict_copy_path(&name_a);
        let copy_b = conflict_copy_path(&name_b);
        assert_ne!(
            copy_a, copy_b,
            "distinct non-UTF-8 names must not collapse onto the same conflict-copy path"
        );

        assert!(is_conflict_copy(&copy_a));
        assert!(is_conflict_copy(&copy_b));

        assert_eq!(original_from_conflict_copy(&copy_a), Some(name_a));
        assert_eq!(original_from_conflict_copy(&copy_b), Some(name_b));
    }

    #[test]
    fn conflict_status_record_is_never_replanned_as_download() {
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();

        local_files.insert(
            PathBuf::from("notes.txt"),
            local("notes.txt", "local-unchanged"),
        );
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "id-1", Some("remote-changed")),
        );
        let conflicted = FileRecord {
            sync_status: SyncStatus::Conflict,
            ..base("notes.txt", Some("id-1"), "local-unchanged")
        };
        base_index.insert(PathBuf::from("notes.txt"), conflicted);

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        assert!(
            planned
                .iter()
                .all(|action| action.path != Path::new("notes.txt")),
            "an unresolved conflict record must not be replanned until its sidecar is \
             resolved, and must never be silently overwritten by a Download"
        );
    }

    #[test]
    fn backfills_file_proton_id_once_remote_exposes_it() {
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();

        local_files.insert(PathBuf::from("linked.txt"), local("linked.txt", "same"));
        remote_files.insert(
            PathBuf::from("linked.txt"),
            remote("linked.txt", "id-newly-known", Some("same")),
        );
        base_index.insert(
            PathBuf::from("linked.txt"),
            base("linked.txt", None, "same"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        let action = planned
            .iter()
            .find(|action| action.path == Path::new("linked.txt"))
            .expect("a backfill action must be planned once the id becomes known");
        assert_eq!(action.action, SyncAction::AutoLink);
        assert_eq!(action.remote_id.as_deref(), Some("id-newly-known"));
    }

    #[test]
    fn ongoing_changed_changed_with_identical_content_autolinks_instead_of_conflicting() {
        // Base hash A; local and remote independently converged on identical content B.
        // This happens after two identical edits, or when a transfer lands on disk but
        // its index checkpoint never commits — a crash or failure between the side
        // effect and its checkpoint (leaving base at A).
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(PathBuf::from("notes.txt"), local("notes.txt", "hash-b"));
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "id-1", Some("hash-b")),
        );
        base_index.insert(
            PathBuf::from("notes.txt"),
            base("notes.txt", Some("id-1"), "hash-a"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);
        let action = planned
            .iter()
            .find(|action| action.path == Path::new("notes.txt"))
            .expect("an action must be planned");
        assert_eq!(
            action.action,
            SyncAction::AutoLink,
            "identical local and remote content must auto-link, not fabricate a conflict"
        );
        assert_eq!(action.remote_id.as_deref(), Some("id-1"));

        // Genuinely divergent content on both sides must still conflict.
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "id-1", Some("hash-c")),
        );
        let planned = plan_sync(&local_files, &remote_files, &base_index);
        let action = planned
            .iter()
            .find(|action| action.path == Path::new("notes.txt"))
            .expect("an action must be planned");
        assert_eq!(
            action.action,
            SyncAction::Conflict,
            "divergent local and remote content must still conflict"
        );
    }

    #[test]
    fn locally_deleted_file_edited_remotely_restores_the_remote_edit() {
        // notes.txt was synced at base A, then deleted locally while the remote copy was
        // edited to B. With no local content left to preserve, the remote edit is
        // authoritative and is restored via Download (not stranded in a sidecar that the
        // daemon could never record, which used to re-download every reconcile).
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "id-1", Some("hash-b")),
        );
        base_index.insert(
            PathBuf::from("notes.txt"),
            base("notes.txt", Some("id-1"), "hash-a"),
        );

        let planned = plan_sync(&HashMap::new(), &remote_files, &base_index);
        let action = planned
            .iter()
            .find(|action| action.path == Path::new("notes.txt"))
            .expect("an action must be planned");
        assert_eq!(action.action, SyncAction::Download);
        assert_eq!(action.remote_id.as_deref(), Some("id-1"));
        assert_eq!(
            action.conflict_path, None,
            "the resurrect path is a plain download, not a conflict sidecar"
        );
    }

    #[test]
    fn locally_edited_file_missing_remotely_conflicts_with_a_locally_copied_sidecar() {
        let mut local_files = HashMap::new();
        let mut base_index = HashMap::new();

        local_files.insert(
            PathBuf::from("edited-then-remote-removed.txt"),
            local("edited-then-remote-removed.txt", "new-local-hash"),
        );
        base_index.insert(
            PathBuf::from("edited-then-remote-removed.txt"),
            base("edited-then-remote-removed.txt", Some("id-1"), "old-hash"),
        );

        let planned = plan_sync(&local_files, &HashMap::new(), &base_index);

        let action = planned
            .iter()
            .find(|action| action.path == Path::new("edited-then-remote-removed.txt"))
            .expect("a conflict must be planned");
        assert_eq!(action.action, SyncAction::Conflict);
        assert_eq!(
            action.conflict_path.as_deref(),
            Some(Path::new("edited-then-remote-removed.proton-cloud.txt")),
            "the conflict must have a sidecar, or it has no exit and no user-visible artefact"
        );
        assert!(
            action.sidecar_from_local_copy,
            "a confirmed-missing remote must be copied from the local file, never downloaded"
        );
        assert_eq!(action.remote_id.as_deref(), Some("id-1"));
    }

    #[test]
    fn removing_the_sidecar_of_a_remote_deleted_conflict_plans_the_upload_exit() {
        // The conflict record carries the LOCAL hash (the daemon upserts `from_local`), so once
        // the sidecar removal marks it `Modified` the exit is the `(Unchanged, Missing)` arm.
        let mut local_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(
            PathBuf::from("edited.txt"),
            local("edited.txt", "local-hash"),
        );
        base_index.insert(
            PathBuf::from("edited.txt"),
            modified_base("edited.txt", Some("id-1"), "local-hash"),
        );

        let planned = plan_sync(&local_files, &HashMap::new(), &base_index);

        assert_eq!(planned.len(), 1, "{planned:?}");
        assert_eq!(planned[0].action, SyncAction::Upload);
        assert_eq!(planned[0].path, PathBuf::from("edited.txt"));
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
    fn backfills_directory_proton_id_once_remote_exposes_it() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        // "docs" was created locally-first (`ensure_directory` returns no id), so its
        // base record has `proton_id: None` even though the directory now genuinely
        // exists remotely with a real id once discovered on this reconcile's listing.
        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        base_index.insert(PathBuf::from("docs"), directory_base("docs", None));

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        let action = planned
            .iter()
            .find(|action| action.path == Path::new("docs"))
            .expect("docs directory should backfill its proton_id");
        assert_eq!(action.action, SyncAction::AutoLink);
        assert_eq!(action.entity_kind, EntityKind::Directory);
        assert_eq!(action.remote_id.as_deref(), Some("docs-id"));
    }

    #[test]
    fn does_not_re_backfill_directory_proton_id_once_already_linked() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            !planned
                .iter()
                .any(|action| action.path == Path::new("docs")),
            "an already-linked directory with agreeing ids should plan no action: {planned:?}"
        );
    }

    #[test]
    fn backfilling_one_directorys_id_never_touches_a_different_directorys_record() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        base_index.insert(PathBuf::from("docs"), directory_base("docs", None));

        local_entities.insert(PathBuf::from("archive"), local_directory("archive"));
        remote_entities.insert(
            PathBuf::from("archive"),
            remote_directory("archive", Some("archive-id")),
        );
        base_index.insert(
            PathBuf::from("archive"),
            directory_base("archive", Some("archive-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        let docs_action = planned
            .iter()
            .find(|action| action.path == Path::new("docs"))
            .expect("docs directory should backfill its own proton_id");
        assert_eq!(docs_action.remote_id.as_deref(), Some("docs-id"));
        assert!(
            !planned
                .iter()
                .any(|action| action.path == Path::new("archive")),
            "an unrelated, already-linked directory must never be touched by another \
             directory's backfill: {planned:?}"
        );
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
    fn untracked_remote_descendant_blocks_recursive_remote_directory_delete() {
        // Local `rm -rf docs` raced another device uploading a brand-new, never-synced
        // `docs/new.txt`. The recursive RemoteDelete would destroy the only copy, so the
        // verdict must fail and the untracked file's bootstrap Download must survive.
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/report.txt"),
            RemoteEntity::File(remote("docs/report.txt", "report-id", Some("same-hash"))),
        );
        remote_entities.insert(
            PathBuf::from("docs/new.txt"),
            RemoteEntity::File(remote("docs/new.txt", "new-id", Some("new-hash"))),
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

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::RemoteDelete),
            "an untracked descendant must veto the recursive directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/new.txt")
                    && action.action == SyncAction::Download),
            "the never-synced remote file must still be downloaded: {planned:?}"
        );
    }

    #[test]
    fn untracked_local_descendant_blocks_recursive_local_directory_delete() {
        // The remote side deleted `docs` wholesale, but the user has since created a
        // brand-new, never-synced `docs/new.txt` locally. The recursive LocalDelete
        // (`remove_dir_all`) would destroy it, and suppression would discard its
        // bootstrap Upload — the verdict must fail instead.
        let mut local_entities = HashMap::new();
        let remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        local_entities.insert(
            PathBuf::from("docs/report.txt"),
            LocalEntityState::File(local("docs/report.txt", "same-hash")),
        );
        local_entities.insert(
            PathBuf::from("docs/new.txt"),
            LocalEntityState::File(local("docs/new.txt", "new-hash")),
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

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::LocalDelete),
            "an untracked descendant must veto the recursive directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/new.txt")
                    && action.action == SyncAction::Upload),
            "the never-synced local file must still be uploaded: {planned:?}"
        );
    }

    #[test]
    fn a_stale_base_directory_kind_over_a_live_local_file_blocks_the_recursive_local_delete() {
        // The base row still records `docs/sub` as a directory, but it is now a never-uploaded
        // local FILE, and `docs` is gone remotely. Dispatching the subtree proof on the base kind
        // alone narrowed the live file away, resolved it to `Purge`, proved `docs` a clean
        // recursive `LocalDelete`, and suppression then dropped the file's own `Upload` — the only
        // copy (#282).
        let mut local_entities = HashMap::new();
        let remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        local_entities.insert(
            PathBuf::from("docs/sub"),
            LocalEntityState::File(local("docs/sub", "only-copy")),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/sub"),
            directory_base("docs/sub", Some("sub-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::LocalDelete),
            "a descendant whose live kind disagrees with its base row must veto the recursive \
             directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/sub")
                    && action.action == SyncAction::Upload),
            "the never-uploaded local file must still be uploaded: {planned:?}"
        );
    }

    #[test]
    fn a_stale_base_directory_kind_over_a_live_remote_file_blocks_the_recursive_remote_delete() {
        // Mirror of #282 on the remote side: the base row records `docs/sub` as a directory while
        // the remote now holds a never-downloaded FILE there, and `docs` is gone locally. The
        // recursive `RemoteDelete` would destroy it and suppression would drop its `Download`.
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/sub"),
            RemoteEntity::File(remote("docs/sub", "sub-id", Some("only-copy"))),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/sub"),
            directory_base("docs/sub", Some("sub-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::RemoteDelete),
            "a descendant whose live kind disagrees with its base row must veto the recursive \
             directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/sub")
                    && action.action == SyncAction::Download),
            "the never-downloaded remote file must still be downloaded: {planned:?}"
        );
    }

    #[test]
    fn a_stale_base_file_kind_over_a_live_local_directory_blocks_the_recursive_local_delete() {
        // The other half of the stale-kind class: the base row records `docs/sub` as a file while
        // it is now a local directory. `plan_ongoing_file_action` narrowed it away as missing on
        // both sides, so the subtree proof read `Purge` and deleted the live directory.
        let mut local_entities = HashMap::new();
        let remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        local_entities.insert(PathBuf::from("docs/sub"), local_directory("docs/sub"));
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/sub"),
            base("docs/sub", Some("sub-id"), "old-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::LocalDelete),
            "a descendant whose live kind disagrees with its base row must veto the recursive \
             directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/sub")
                    && action.action == SyncAction::CreateRemoteDirectory),
            "the live local directory must still be created remotely: {planned:?}"
        );
    }

    #[test]
    fn a_descendant_under_an_untracked_gap_still_blocks_the_recursive_directory_delete() {
        // Selective-sync filters drop base rows independently of the entries under them, so a
        // tracked descendant can sit under an untracked intermediate directory. The bottom-up
        // proof folds each entry into its nearest *tracked* ancestor: stopping at an absent
        // immediate parent would detach `docs/gap/report.txt` from the proof entirely and delete
        // the remote edit unexamined.
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("docs/gap/report.txt"),
            RemoteEntity::File(remote(
                "docs/gap/report.txt",
                "report-id",
                Some("changed-hash"),
            )),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs/gap/report.txt"),
            base("docs/gap/report.txt", Some("report-id"), "original-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::RemoteDelete),
            "a diverging descendant reached through an untracked gap must veto the recursive \
             directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/gap/report.txt")
                    && action.action == SyncAction::Download),
            "the remotely-edited descendant must still be restored: {planned:?}"
        );
    }

    #[test]
    fn a_nested_untracked_descendant_blocks_the_recursive_directory_delete() {
        // As `untracked_local_descendant_blocks_*`, but the never-synced file sits two levels down
        // under an equally untracked directory: the veto must reach `docs` from the whole chain.
        let mut local_entities = HashMap::new();
        let remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        local_entities.insert(PathBuf::from("docs/new"), local_directory("docs/new"));
        local_entities.insert(
            PathBuf::from("docs/new/deep.txt"),
            LocalEntityState::File(local("docs/new/deep.txt", "new-hash")),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            !planned.iter().any(|action| action.path == Path::new("docs")
                && action.action == SyncAction::LocalDelete),
            "a nested untracked descendant must veto the recursive directory delete: {planned:?}"
        );
        assert!(
            planned
                .iter()
                .any(|action| action.path == Path::new("docs/new/deep.txt")
                    && action.action == SyncAction::Upload),
            "the never-synced nested file must still be uploaded: {planned:?}"
        );
    }

    #[test]
    fn a_diverging_leaf_vetoes_every_directory_level_above_it() {
        // The fold carries a failure up one edge at a time, so a leaf three levels down must
        // still reach the top. Nothing in the chain may be deleted.
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        for dir in ["top", "top/mid", "top/mid/leaf"] {
            remote_entities.insert(PathBuf::from(dir), remote_directory(dir, Some(dir)));
            base_index.insert(PathBuf::from(dir), directory_base(dir, Some(dir)));
        }
        remote_entities.insert(
            PathBuf::from("top/mid/leaf/report.txt"),
            RemoteEntity::File(remote(
                "top/mid/leaf/report.txt",
                "report-id",
                Some("changed-hash"),
            )),
        );
        base_index.insert(
            PathBuf::from("top/mid/leaf/report.txt"),
            base(
                "top/mid/leaf/report.txt",
                Some("report-id"),
                "original-hash",
            ),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        for level in ["top", "top/mid", "top/mid/leaf"] {
            assert!(
                !planned.iter().any(|action| action.path == Path::new(level)
                    && matches!(
                        action.action,
                        SyncAction::RemoteDelete | SyncAction::LocalDelete
                    )),
                "{level} must not be deleted over a diverging leaf: {planned:?}"
            );
        }
    }

    #[test]
    fn a_pass_with_no_one_sided_directory_skips_the_deletion_proof() {
        // The gate that makes an idle pass cheap (#48): with every directory live on both sides
        // nothing can consume a verdict, so none are computed — and the plan is unchanged.
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("docs"), local_directory("docs"));
        remote_entities.insert(
            PathBuf::from("docs"),
            remote_directory("docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("docs-id")),
        );
        local_entities.insert(
            PathBuf::from("docs/report.txt"),
            LocalEntityState::File(local("docs/report.txt", "same-hash")),
        );
        remote_entities.insert(
            PathBuf::from("docs/report.txt"),
            RemoteEntity::File(remote("docs/report.txt", "report-id", Some("same-hash"))),
        );
        base_index.insert(
            PathBuf::from("docs/report.txt"),
            base("docs/report.txt", Some("report-id"), "same-hash"),
        );

        let verdicts = compute_directory_deletion_verdicts(
            &local_entities,
            &remote_entities,
            &base_index,
            &BTreeSet::new(),
        );

        assert!(
            verdicts.is_empty(),
            "no directory is live on exactly one side, so no verdict can be read: {verdicts:?}"
        );
        assert!(
            plan_sync_entities(&local_entities, &remote_entities, &base_index).is_empty(),
            "a fully converged tree still plans nothing"
        );
    }

    #[test]
    fn conflict_record_missing_on_both_sides_purges_instead_of_wedging() {
        // A sidecar-less conflict record whose file was then deleted locally (remote
        // already gone): without the Purge exit the record early-returns forever.
        let mut base_index = HashMap::new();
        base_index.insert(
            PathBuf::from("gone.txt"),
            FileRecord {
                sync_status: SyncStatus::Conflict,
                ..base("gone.txt", Some("id-1"), "old-hash")
            },
        );

        let planned = plan_sync(&HashMap::new(), &HashMap::new(), &base_index);

        assert_eq!(planned.len(), 1, "{planned:?}");
        assert_eq!(planned[0].path, PathBuf::from("gone.txt"));
        assert_eq!(planned[0].action, SyncAction::Purge);
    }

    #[test]
    fn conflict_record_with_surviving_local_edit_still_waits_for_resolution() {
        let mut local_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(PathBuf::from("edited.txt"), local("edited.txt", "new-hash"));
        base_index.insert(
            PathBuf::from("edited.txt"),
            FileRecord {
                sync_status: SyncStatus::Conflict,
                ..base("edited.txt", Some("id-1"), "old-hash")
            },
        );

        let planned = plan_sync(&local_files, &HashMap::new(), &base_index);

        assert!(
            planned.is_empty(),
            "an unresolved conflict with surviving local content must stay parked: {planned:?}"
        );
    }

    #[test]
    fn empty_reconstruction_placeholder_id_is_not_auto_linked() {
        // `reconstruct::remote_entity_from_record` materializes a not-yet-backfilled
        // record with an empty id; linking it would commit `proton_id = Some("")`.
        let mut local_files = HashMap::new();
        let mut remote_files = HashMap::new();
        let mut base_index = HashMap::new();
        local_files.insert(PathBuf::from("fresh.txt"), local("fresh.txt", "same-hash"));
        remote_files.insert(
            PathBuf::from("fresh.txt"),
            remote("fresh.txt", "", Some("same-hash")),
        );
        base_index.insert(
            PathBuf::from("fresh.txt"),
            base("fresh.txt", None, "same-hash"),
        );

        let planned = plan_sync(&local_files, &remote_files, &base_index);

        assert!(
            planned.is_empty(),
            "an empty placeholder id must not be auto-linked: {planned:?}"
        );
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
    fn deep_one_sided_directory_deletion_plans_in_bounded_time() {
        // A deeply nested directory chain deleted on one side must plan in polynomial
        // time. The previous mutual recursion was Θ(2^depth); at this depth it would not
        // finish in the age of the universe, so merely completing proves the fix.
        const DEPTH: usize = 60;
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        let mut path = PathBuf::new();
        for level in 0..DEPTH {
            path.push(format!("d{level}"));
            let key = path.to_str().expect("utf-8 path");
            let id = format!("id-{level}");
            remote_entities.insert(path.clone(), remote_directory(key, Some(&id)));
            base_index.insert(path.clone(), directory_base(key, Some(&id)));
        }

        // Locally the whole chain was removed while it still exists remotely.
        let planned = plan_sync_entities(&HashMap::new(), &remote_entities, &base_index);

        // The clean subtree deletion collapses to a single recursive RemoteDelete at the
        // top, with every descendant suppressed.
        assert_eq!(
            planned.len(),
            1,
            "descendants must be suppressed: {planned:?}"
        );
        assert_eq!(planned[0].path, PathBuf::from("d0"));
        assert_eq!(planned[0].action, SyncAction::RemoteDelete);
        assert_eq!(planned[0].entity_kind, EntityKind::Directory);
    }

    #[test]
    fn sibling_clean_and_diverging_subtrees_do_not_share_a_deletion_verdict() {
        // Under "top", the "gone" subtree was cleanly removed while the "kept" subtree has a
        // remotely-edited (diverging) descendant. The clean sibling must still delete, but
        // its `true` verdict must not leak to the diverging sibling or the shared parent,
        // which must both be recreated. Guards the per-path deletion-verdict memoization.
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        for dir in ["top", "top/gone", "top/kept"] {
            remote_entities.insert(PathBuf::from(dir), remote_directory(dir, Some(dir)));
            base_index.insert(PathBuf::from(dir), directory_base(dir, Some(dir)));
        }
        // Clean subtree: remote file unchanged from base -> RemoteDelete.
        remote_entities.insert(
            PathBuf::from("top/gone/clean.txt"),
            RemoteEntity::File(remote("top/gone/clean.txt", "clean-id", Some("same"))),
        );
        base_index.insert(
            PathBuf::from("top/gone/clean.txt"),
            base("top/gone/clean.txt", Some("clean-id"), "same"),
        );
        // Diverging subtree: remote file edited since base -> not a clean deletion.
        remote_entities.insert(
            PathBuf::from("top/kept/diverged.txt"),
            RemoteEntity::File(remote("top/kept/diverged.txt", "kept-id", Some("changed"))),
        );
        base_index.insert(
            PathBuf::from("top/kept/diverged.txt"),
            base("top/kept/diverged.txt", Some("kept-id"), "original"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);
        let action_for = |path: &str| {
            planned
                .iter()
                .find(|action| action.path == Path::new(path))
                .map(|action| action.action)
        };

        assert_eq!(
            action_for("top/gone"),
            Some(SyncAction::RemoteDelete),
            "the cleanly-removed sibling subtree must still delete: {planned:?}"
        );
        assert_eq!(
            action_for("top"),
            Some(SyncAction::CreateLocalDirectory),
            "the shared parent must be recreated (not deleted) because a sibling diverges: \
             {planned:?}"
        );
        assert!(
            !matches!(
                action_for("top/kept"),
                Some(SyncAction::RemoteDelete | SyncAction::LocalDelete)
            ),
            "the diverging sibling must not be deleted: {planned:?}"
        );
    }

    #[test]
    fn directory_deletion_falls_back_to_recreate_when_descendant_diverges() {
        let local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        // "docs" is gone locally, but its child file was independently modified on
        // the remote side after the last sync, so the subtree was not cleanly removed.
        // The directory must not be recursively deleted out from under the diverging
        // descendant; instead it is recreated locally and the remote edit is restored.
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
                    && action.action == SyncAction::Download),
            "the diverging descendant (locally deleted, remotely edited) must restore the \
             remote edit rather than be swept up in a subtree delete: {planned:?}"
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
        assert_eq!(planned[0].entity_kind, EntityKind::Directory);
        assert_eq!(planned[0].remote_id.as_deref(), Some("file-id"));
        assert_eq!(
            planned[0].conflict_path.as_deref(),
            Some(Path::new("same-name.proton-cloud")),
            "the clashing remote file must be downloadable as a sidecar outside the \
             kept local directory"
        );
    }

    #[test]
    fn one_sided_remote_file_over_a_stale_directory_record_downloads_it() {
        // #47: `docs` was a synced directory, deleted everywhere, and a remote FILE now holds the
        // name. Only the base row disagrees, so there is no conflict to report — adopt the file.
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        remote_entities.insert(
            PathBuf::from("docs"),
            RemoteEntity::File(remote("docs", "file-id", Some("hash"))),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("dir-id")),
        );

        let planned = plan_sync_entities(&HashMap::new(), &remote_entities, &base_index);

        assert_eq!(planned.len(), 1, "{planned:?}");
        assert_eq!(planned[0].path, PathBuf::from("docs"));
        assert_eq!(planned[0].action, SyncAction::Download);
        assert_eq!(planned[0].entity_kind, EntityKind::File);
        assert_eq!(planned[0].remote_id.as_deref(), Some("file-id"));
    }

    #[test]
    fn one_sided_local_file_over_a_stale_directory_record_uploads_it() {
        // The mirror of the above: the surviving side is local.
        let mut local_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(
            PathBuf::from("docs"),
            LocalEntityState::File(local("docs", "local-hash")),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("dir-id")),
        );

        let planned = plan_sync_entities(&local_entities, &HashMap::new(), &base_index);

        assert_eq!(planned.len(), 1, "{planned:?}");
        assert_eq!(planned[0].path, PathBuf::from("docs"));
        assert_eq!(planned[0].action, SyncAction::Upload);
        assert_eq!(planned[0].entity_kind, EntityKind::File);
    }

    #[test]
    fn one_sided_remote_directory_over_a_stale_file_record_creates_it_locally() {
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        remote_entities.insert(
            PathBuf::from("notes"),
            remote_directory("notes", Some("dir-id")),
        );
        base_index.insert(
            PathBuf::from("notes"),
            base("notes", Some("file-id"), "old-hash"),
        );

        let planned = plan_sync_entities(&HashMap::new(), &remote_entities, &base_index);

        assert_eq!(planned.len(), 1, "{planned:?}");
        assert_eq!(planned[0].action, SyncAction::CreateLocalDirectory);
        assert_eq!(planned[0].entity_kind, EntityKind::Directory);
        assert_eq!(planned[0].remote_id.as_deref(), Some("dir-id"));
    }

    #[test]
    fn both_sides_agreeing_on_a_new_kind_over_a_stale_record_adopt_it() {
        // Both live sides agree; only the base row is stale. Nothing clashes, so neither variant
        // may report a type conflict the daemon can do nothing about.
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        // Stale directory record, both sides now a file with identical content.
        local_entities.insert(
            PathBuf::from("docs"),
            LocalEntityState::File(local("docs", "same-hash")),
        );
        remote_entities.insert(
            PathBuf::from("docs"),
            RemoteEntity::File(remote("docs", "file-id", Some("same-hash"))),
        );
        base_index.insert(
            PathBuf::from("docs"),
            directory_base("docs", Some("dir-id")),
        );
        // Stale file record, both sides now a directory.
        local_entities.insert(PathBuf::from("notes"), local_directory("notes"));
        remote_entities.insert(
            PathBuf::from("notes"),
            remote_directory("notes", Some("new-dir-id")),
        );
        base_index.insert(
            PathBuf::from("notes"),
            base("notes", Some("old-file-id"), "old-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            planned
                .iter()
                .all(|action| action.action != SyncAction::TypeConflict),
            "agreeing live sides over a stale base kind are not a type conflict: {planned:?}"
        );
        let file = planned
            .iter()
            .find(|action| action.path == Path::new("docs"))
            .expect("the agreed file must be planned");
        assert_eq!(file.action, SyncAction::AutoLink);
        assert_eq!(file.entity_kind, EntityKind::File);
        assert_eq!(file.remote_id.as_deref(), Some("file-id"));
        let directory = planned
            .iter()
            .find(|action| action.path == Path::new("notes"))
            .expect("the agreed directory must be planned");
        assert_eq!(directory.action, SyncAction::AutoLink);
        assert_eq!(directory.entity_kind, EntityKind::Directory);
        assert_eq!(directory.remote_id.as_deref(), Some("new-dir-id"));
    }

    #[test]
    fn reconciled_directory_file_clash_is_not_replanned() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(PathBuf::from("same-name"), local_directory("same-name"));
        remote_entities.insert(
            PathBuf::from("same-name"),
            RemoteEntity::File(remote("same-name", "file-id", Some("hash"))),
        );
        base_index.insert(
            PathBuf::from("same-name"),
            directory_base("same-name", None),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            planned.is_empty(),
            "an already-resolved directory/file clash must not be replanned or \
             re-downloaded on every reconcile: {planned:?}"
        );
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
    fn local_file_rename_with_spaces_and_special_characters_plans_verified_remote_move() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();
        local_entities.insert(
            PathBuf::from("weird name (v2) [final] $$.txt"),
            LocalEntityState::File(local("weird name (v2) [final] $$.txt", "same-hash")),
        );
        remote_entities.insert(
            PathBuf::from("weird name (v1) & co's file!.txt"),
            RemoteEntity::File(remote(
                "weird name (v1) & co's file!.txt",
                "stable-id",
                Some("same-hash"),
            )),
        );
        base_index.insert(
            PathBuf::from("weird name (v1) & co's file!.txt"),
            base(
                "weird name (v1) & co's file!.txt",
                Some("stable-id"),
                "same-hash",
            ),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action, SyncAction::MoveRemote);
        assert_eq!(
            planned[0].path,
            PathBuf::from("weird name (v1) & co's file!.txt")
        );
        assert_eq!(
            planned[0].destination_path.as_deref(),
            Some(Path::new("weird name (v2) [final] $$.txt"))
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
    fn remote_directory_rename_plans_local_move_when_id_is_unique() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("old-docs"), local_directory("old-docs"));
        remote_entities.insert(
            PathBuf::from("new-docs"),
            remote_directory("new-docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("old-docs"),
            directory_base("old-docs", Some("docs-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(planned.len(), 1, "{planned:?}");
        assert_eq!(planned[0].action, SyncAction::MoveLocal);
        assert_eq!(planned[0].entity_kind, EntityKind::Directory);
        assert_eq!(planned[0].path, PathBuf::from("old-docs"));
        assert_eq!(
            planned[0].destination_path.as_deref(),
            Some(Path::new("new-docs"))
        );
        assert_eq!(planned[0].remote_id.as_deref(), Some("docs-id"));
    }

    #[test]
    fn remote_directory_move_suppresses_nested_file_and_subdirectory_from_independent_planning() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("old-docs"), local_directory("old-docs"));
        remote_entities.insert(
            PathBuf::from("new-docs"),
            remote_directory("new-docs", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("old-docs"),
            directory_base("old-docs", Some("docs-id")),
        );
        // A nested file and a nested subdirectory "along for the ride" inside the
        // moved directory: their base records still reference the OLD prefix, since
        // this reconcile's local/remote listings only reflect the top-level move.
        base_index.insert(
            PathBuf::from("old-docs/report.txt"),
            base("old-docs/report.txt", Some("report-id"), "same-hash"),
        );
        base_index.insert(
            PathBuf::from("old-docs/sub"),
            directory_base("old-docs/sub", Some("sub-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert_eq!(
            planned.len(),
            1,
            "the directory move should be the only planned action; descendants must be \
             suppressed rather than independently replanned: {planned:?}"
        );
        assert_eq!(planned[0].path, PathBuf::from("old-docs"));
        assert_eq!(planned[0].action, SyncAction::MoveLocal);
    }

    #[test]
    fn directory_move_is_not_inferred_without_a_backfilled_proton_id() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        local_entities.insert(PathBuf::from("old-docs"), local_directory("old-docs"));
        // A locally-modified descendant keeps the subtree from looking "cleanly
        // deleted", isolating this test to the move-inference decision itself rather
        // than the pre-existing empty-directory delete-propagation behavior.
        local_entities.insert(
            PathBuf::from("old-docs/report.txt"),
            LocalEntityState::File(local("old-docs/report.txt", "changed-locally")),
        );
        remote_entities.insert(
            PathBuf::from("new-docs"),
            remote_directory("new-docs", Some("docs-id")),
        );
        base_index.insert(PathBuf::from("old-docs"), directory_base("old-docs", None));
        base_index.insert(
            PathBuf::from("old-docs/report.txt"),
            base("old-docs/report.txt", Some("report-id"), "original-hash"),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            planned
                .iter()
                .all(|action| action.action != SyncAction::MoveLocal),
            "a directory without a backfilled proton_id must not infer a move: {planned:?}"
        );
        let directory_action = planned
            .iter()
            .find(|action| action.path == Path::new("old-docs"))
            .expect("old-docs directory should still be planned");
        assert_eq!(
            directory_action.action,
            SyncAction::CreateRemoteDirectory,
            "without a proton_id to match, the directory should keep the existing \
             non-destructive recreate fallback: {planned:?}"
        );
    }

    #[test]
    fn ambiguous_directory_move_candidates_are_not_inferred() {
        let mut local_entities = HashMap::new();
        let mut remote_entities = HashMap::new();
        let mut base_index = HashMap::new();

        // Two different directories share the same (corrupted/duplicated) id in this
        // remote listing; the move must not be inferred from ambiguous evidence.
        local_entities.insert(PathBuf::from("old-docs"), local_directory("old-docs"));
        remote_entities.insert(
            PathBuf::from("candidate-a"),
            remote_directory("candidate-a", Some("docs-id")),
        );
        remote_entities.insert(
            PathBuf::from("candidate-b"),
            remote_directory("candidate-b", Some("docs-id")),
        );
        base_index.insert(
            PathBuf::from("old-docs"),
            directory_base("old-docs", Some("docs-id")),
        );

        let planned = plan_sync_entities(&local_entities, &remote_entities, &base_index);

        assert!(
            planned
                .iter()
                .all(|action| action.action != SyncAction::MoveLocal),
            "ambiguous id matches must not be inferred as a directory move: {planned:?}"
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
