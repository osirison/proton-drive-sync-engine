//! Reconstructs the current remote entity map as **`base ⊕ delta`** — the last-known remote view
//! (the baseline `file_index`) overlaid with a volume-event delta. This is the O(changes)
//! alternative to re-walking the whole remote tree with [`crate::proton`], and it is what makes
//! event-driven reconcile hand the planner a *complete* map (which its move-detection and
//! directory-deletion verdicts require) without a full listing.
//!
//! The function is **pure**: all remote I/O (the targeted parent listing that resolves a
//! created/updated node's name + digest) is injected behind [`RemoteChangeResolver`], so the
//! reconstruction logic is unit-tested against fakes. When a change cannot be incorporated
//! without a full re-walk, it signals [`Reconstruction::FallbackToSnapshot`] rather than guessing.
//!
//! **Placement lives here, not in the resolver (#456).** The event stream is per volume, so a
//! delta carries every change on the volume, most of them in other folders. An event whose parent
//! this pair cannot place is *deferred* to the end of the delta and dropped as "outside this
//! folder" only when the pair's tree is fully named (see [`reconstruct_remote`]); otherwise the
//! pass walks, as it always did.
//!
//! See `docs/adr/0001-remote-change-detection-via-volume-events.md` and
//! `docs/adr/0006-shared-volume-event-scope.md`.

use crate::AppResult;
use crate::events::{RemoteChange, RemoteChangeKind, node_uid};
use crate::index::{EntityKind, FileRecord, ScanOptions};
use crate::proton::{RemoteDirectory, RemoteEntity, RemoteFile};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The two remote questions a reconstruction asks. Primitives only: *which* of them answers a
/// given event is [`reconstruct_remote`]'s decision, so it is tested without a daemon.
pub trait RemoteChangeResolver {
    /// The path the index holds for a composed node uid (`volumeId~nodeId`); `None` when no row
    /// carries it or two rows do (an ambiguous id places nothing). Reads the **unfiltered** index.
    fn indexed_path(&self, uid: &str) -> AppResult<Option<PathBuf>>;

    /// One non-recursive listing of a root-relative directory (`""` is the remote root), keyed by
    /// root-relative path. Memoized by the implementation for the duration of a pass. An error
    /// (the directory is gone, the listing is incomplete) makes the caller fall back to a snapshot.
    fn list(&self, relative_directory: &Path) -> AppResult<Rc<HashMap<PathBuf, RemoteEntity>>>;
}

/// Outcome of reconstructing the remote map from a delta.
pub enum Reconstruction {
    /// A complete remote entity map (`base ⊕ delta`), safe to hand to the planner.
    Complete {
        remote: HashMap<PathBuf, RemoteEntity>,
        /// Events dropped because they describe nodes outside this pair's folder.
        outside: usize,
    },
    /// A change could not be incorporated without a full re-walk; the caller must snapshot. The
    /// string is a human-readable reason for logging.
    FallbackToSnapshot(String),
}

/// What an event that could not be placed immediately is, until the delta is read to its end.
enum DeferredKind {
    /// A created/updated node whose parent this pair does not know. `tracked` is whether the
    /// node was in the pair's map when the event arrived: tracked means it moved *out*, untracked
    /// means it is somewhere else on the volume.
    Placement { tracked: bool },
    /// A removal of a node that is not in the pair's map.
    Removal,
}

struct Deferred {
    position: usize,
    uid: String,
    node_id: String,
    kind: DeferredKind,
}

/// Reconstructs the current remote entity map as `base_index ⊕ changes`.
///
/// `base_index` must be the **selective-sync-filtered** baseline (the same map the planner is
/// given), so excluded records are neither present as "remote" nor purged. `volume_id` bridges an
/// event's raw `LinkID` into the composed `proton_id` space via [`node_uid`]. `scan_options`
/// re-applies the include/exclude filter to newly-resolved nodes exactly as it applies to a full
/// listing.
///
/// `remote_root_uid` is the composed uid of the pair's own remote root, when known. Without it an
/// event whose parent is not in the index is placed by listing the root and anything the root
/// listing does not hold forces a snapshot (the behaviour before #456). With it:
///
/// * the root is a known parent (`""`), and an event about the root itself always forces a
///   snapshot — it is never skipped and never placed;
/// * an event whose parent is unknown is **deferred**, and dropped at the end of the delta only if
///   the pair's tree is fully named (condition S): every directory in the final map carries a
///   composed uid. Then the parent is neither the root nor any directory of the tree, so the node
///   is somewhere else on the volume. Otherwise the pass falls back to a snapshot, naming the
///   record that blocked it;
/// * a removal of a node the pair does not track is dropped at the end of the delta under the same
///   rule extended to files (every record carries a composed uid, no root uid needed): the removed
///   node could only have been one of the records that lack an id.
///
/// A directory placed by an `Updated` while untracked (moved in from elsewhere, or restored from
/// the trash) forces a snapshot, because volume events are per link and its subtree has none.
pub fn reconstruct_remote(
    base_index: &HashMap<PathBuf, FileRecord>,
    changes: &[RemoteChange],
    volume_id: &str,
    remote_root_uid: Option<&str>,
    scan_options: &ScanOptions,
    resolver: &dyn RemoteChangeResolver,
) -> Reconstruction {
    // The last-known remote view, materialized from the baseline index.
    let mut remote: HashMap<PathBuf, RemoteEntity> = HashMap::new();
    // Composed node uid (`proton_id`) -> its current path in `remote`. Seeded from the baseline so
    // a *removal* event resolves to a path even for a node not touched earlier in this same page,
    // and updated as changes are applied so within-page renames/creates stay consistent.
    let mut uid_to_path: HashMap<String, PathBuf> = HashMap::new();

    for (path, record) in base_index {
        // An empty id is the not-yet-backfilled placeholder, not a real uid — never seed it
        // (two fresh uploads would alias each other on the "" key).
        if let Some(id) = record.proton_id.as_deref().filter(|id| !id.is_empty())
            && let Some(previous) = uid_to_path.insert(id.to_owned(), path.clone())
        {
            // Two rows holding one id is a reachable transient state (e.g. a withheld
            // LocalDelete still pinning the old row while a Download committed the new
            // one). Which row wins here would be HashMap iteration order — replaying a
            // move event against the wrong winner silently drops the stale path and
            // advances the cursor past it. Only a full walk resolves this safely.
            return Reconstruction::FallbackToSnapshot(format!(
                "proton_id {id} is held by both {} and {}",
                previous.display(),
                path.display()
            ));
        }
        remote.insert(path.clone(), remote_entity_from_record(record));
    }

    // A root uid from another volume cannot be the parent of anything this stream describes.
    let root_uid = remote_root_uid.filter(|uid| is_composed_on(volume_id, uid));
    let mut deferred: Vec<Deferred> = Vec::new();
    // Position of the last event applied to each node during the first pass over the delta. A
    // deferred event that a later applied one overtook (a node moved out, then back in) is moot.
    let mut last_applied: HashMap<String, usize> = HashMap::new();

    for (position, change) in changes.iter().enumerate() {
        let uid = node_uid(volume_id, &change.node_id);

        // The root's own events: a rename, move or trash changes what the pair's path means, and
        // nothing about it can be placed or dropped. The bootstrap that follows learns the new
        // state (and, for a recreated root, its new uid).
        if root_uid == Some(uid.as_str()) {
            return Reconstruction::FallbackToSnapshot(format!(
                "the pair's own remote root (node {}) changed",
                change.node_id
            ));
        }

        // A hard delete OR a trashing (Updated + trashed) is a remote removal. Trashing arriving
        // as Updated (not Deleted) is the key event-stream subtlety; both mean "gone" here.
        let is_removal = matches!(change.kind, RemoteChangeKind::Deleted)
            || (matches!(change.kind, RemoteChangeKind::Updated) && change.trashed);

        if is_removal {
            if let Some(path) = uid_to_path.remove(&uid) {
                // Volume events are per-link: trashing/deleting a folder flips only the
                // folder's own link, with no events for its descendants. Cascade the removal,
                // or the map would claim the children still exist remotely and the planner
                // would *recreate* the folder the user just deleted. Sound under in-order
                // delivery: a child moved out beforehand was re-homed by its own earlier
                // event, so it no longer sits under `path`.
                remove_subtree(&mut remote, &mut uid_to_path, &path);
                last_applied.insert(uid, position);
            } else {
                // Not in the map: either a node elsewhere on the volume, or a record of this pair
                // that has no uid yet (just uploaded). Which one is only knowable once the whole
                // delta has been read — the record's own Created event may be later in it.
                deferred.push(Deferred {
                    position,
                    uid,
                    node_id: change.node_id.clone(),
                    kind: DeferredKind::Removal,
                });
            }
            continue;
        }

        // Created, or Updated (not trashed): find the node's parent, then its place in the
        // parent's listing.
        let parent_path = match locate_parent(change, volume_id, root_uid, &uid_to_path, resolver) {
            Ok(parent_path) => parent_path,
            Err(error) => {
                return Reconstruction::FallbackToSnapshot(format!(
                    "could not resolve remote change for node {}: {error}",
                    change.node_id
                ));
            }
        };

        let listing = match parent_path {
            Some(parent_path) => match resolver.list(&parent_path) {
                Ok(listing) => listing,
                Err(error) => {
                    return Reconstruction::FallbackToSnapshot(format!(
                        "could not resolve remote change for node {}: {error}",
                        change.node_id
                    ));
                }
            },
            None if root_uid.is_none() => {
                // No root uid, so nothing can say the node is outside: the root listing is the
                // only other place it can be, and if it is not there either the pair cannot place
                // it without a full walk. This is the pre-#456 rule, kept whole.
                match resolver.list(Path::new("")) {
                    Ok(listing) => {
                        if find_entity_by_uid(&listing, &uid).is_none() {
                            return Reconstruction::FallbackToSnapshot(format!(
                                "changed node {} is not under any indexed parent or the remote root",
                                change.node_id
                            ));
                        }
                        listing
                    }
                    Err(error) => {
                        return Reconstruction::FallbackToSnapshot(format!(
                            "could not resolve remote change for node {}: {error}",
                            change.node_id
                        ));
                    }
                }
            }
            None => {
                let tracked = uid_to_path.contains_key(&uid);
                // A tracked node is inside the tree, so an event that names *no* parent for it
                // cannot be a move out (a move out names the new parent). Reading it as one would
                // remove a file the user still has.
                if tracked && change.parent_id.is_none() {
                    return Reconstruction::FallbackToSnapshot(format!(
                        "event for tracked node {} names no parent",
                        change.node_id
                    ));
                }
                deferred.push(Deferred {
                    position,
                    uid,
                    node_id: change.node_id.clone(),
                    kind: DeferredKind::Placement { tracked },
                });
                continue;
            }
        };

        match find_entity_by_uid(&listing, &uid) {
            Some((path, entity)) => {
                let allowed = match &entity {
                    RemoteEntity::File(_) => scan_options.allows_relative_file(&path),
                    RemoteEntity::Directory(_) => scan_options.allows_relative_directory(&path),
                };
                let tracked = uid_to_path.contains_key(&uid);
                if allowed
                    && !tracked
                    && matches!(change.kind, RemoteChangeKind::Updated)
                    && matches!(entity, RemoteEntity::Directory(_))
                    && !is_unnamed_directory_record(&remote, &path, volume_id)
                {
                    // Moved in from elsewhere, or restored from the trash: volume events are per
                    // link, so nothing describes what is inside it. The map would hold an empty
                    // directory where the remote has a subtree. (A directory record that exists at
                    // this very path without a uid is the daemon's own folder being linked, not a
                    // newcomer.)
                    return Reconstruction::FallbackToSnapshot(format!(
                        "directory {} (node {}) appeared by an update, so its contents have no \
                         events of their own",
                        path.display(),
                        change.node_id
                    ));
                }
                // A rename/move leaves the node at a new path; drop the stale location first.
                if let Some(old_path) = uid_to_path.get(&uid).cloned()
                    && old_path != path
                {
                    remote.remove(&old_path);
                }
                if allowed {
                    uid_to_path.insert(uid.clone(), path.clone());
                    remote.insert(path, entity);
                } else {
                    // Excluded by selective sync: neither present as remote nor tracked, so it is
                    // never planned or purged.
                    uid_to_path.remove(&uid);
                }
                last_applied.insert(uid, position);
            }
            None => {
                // A *created* node missing from its parent listing is almost always the CLI
                // listing lagging the event stream (observed live: seconds behind), not a node
                // that vanished. Dropping it would advance the cursor past a create nothing
                // re-derives — the periodic full-tree resync is off by default and a restart
                // warm-starts from the cursor — so re-anchor with a full walk instead. This is
                // symmetric with the root-listing branch above, which already falls back here.
                // The bootstrap that follows captures a fresh cursor past this event, so a node
                // created then trashed before any listing saw it cannot loop (#30).
                if matches!(change.kind, RemoteChangeKind::Created) {
                    return Reconstruction::FallbackToSnapshot(format!(
                        "created node {} is not in its parent listing yet",
                        change.node_id
                    ));
                }
                // Updated: the node was already tracked, so absence is a real move/trash — drop
                // any stale location.
                if let Some(old_path) = uid_to_path.remove(&uid) {
                    remote.remove(&old_path);
                }
                last_applied.insert(uid, position);
            }
        }
    }

    let mut outside = 0;
    if !deferred.is_empty() {
        // Condition S, over the final map: which records carry a composed uid of this volume. The
        // paths are collected first so the moves out below can edit the map they were read from.
        let named: HashSet<PathBuf> = uid_to_path
            .iter()
            .filter(|(uid, _)| is_composed_on(volume_id, uid))
            .map(|(_, path)| path.clone())
            .collect();
        let unnamed_directory = remote
            .iter()
            .filter(|(path, entity)| {
                matches!(entity, RemoteEntity::Directory(_)) && !named.contains(*path)
            })
            .map(|(path, _)| path)
            .min()
            .cloned();
        let unnamed_record = remote
            .keys()
            .filter(|path| !named.contains(*path))
            .min()
            .cloned();
        let directories_named = root_uid.is_some() && unnamed_directory.is_none();

        for event in deferred {
            // A later event applied during the first pass is the node's final word.
            let overtaken = last_applied
                .get(&event.uid)
                .is_some_and(|applied| *applied > event.position);
            match event.kind {
                DeferredKind::Placement { tracked } => {
                    if overtaken {
                        continue;
                    }
                    if !directories_named {
                        return Reconstruction::FallbackToSnapshot(match &unnamed_directory {
                            Some(path) => format!(
                                "cannot tell whether node {} is outside this folder: {} has no \
                                 composed id",
                                event.node_id,
                                path.display()
                            ),
                            None => format!(
                                "cannot tell whether node {} is outside this folder: the folder's \
                                 own node is not known",
                                event.node_id
                            ),
                        });
                    }
                    if tracked {
                        // It named a parent the whole tree does not hold: it left the tree.
                        if let Some(path) = uid_to_path.remove(&event.uid) {
                            remove_subtree(&mut remote, &mut uid_to_path, &path);
                        }
                    } else {
                        outside += 1;
                    }
                }
                DeferredKind::Removal => {
                    if let Some(path) = &unnamed_record {
                        return Reconstruction::FallbackToSnapshot(format!(
                            "removal of untracked node {} while {} has no composed id",
                            event.node_id,
                            path.display()
                        ));
                    }
                    // Every record carries an id and none is this node: it was never synced here;
                    // a full snapshot would not list it either.
                    outside += 1;
                }
            }
        }
    }

    Reconstruction::Complete { remote, outside }
}

/// The root-relative path of the directory an event says its node sits in: the in-pass map first
/// (a folder created or moved earlier in this delta), then the pair's root, then the index.
/// `None` when the pair does not know the parent — including an event that names none.
fn locate_parent(
    change: &RemoteChange,
    volume_id: &str,
    root_uid: Option<&str>,
    uid_to_path: &HashMap<String, PathBuf>,
    resolver: &dyn RemoteChangeResolver,
) -> AppResult<Option<PathBuf>> {
    let Some(parent_id) = change.parent_id.as_deref() else {
        return Ok(None);
    };
    let parent_uid = node_uid(volume_id, parent_id);
    if let Some(path) = uid_to_path.get(&parent_uid) {
        return Ok(Some(path.clone()));
    }
    if root_uid == Some(parent_uid.as_str()) {
        return Ok(Some(PathBuf::new()));
    }
    resolver.indexed_path(&parent_uid)
}

/// Removes the entry at `path` and everything beneath it from both maps.
fn remove_subtree(
    remote: &mut HashMap<PathBuf, RemoteEntity>,
    uid_to_path: &mut HashMap<String, PathBuf>,
    path: &Path,
) {
    remote.remove(path);
    remote.retain(|key, _| !crate::sync::is_strict_descendant(path, key));
    uid_to_path.retain(|_, value| !crate::sync::is_strict_descendant(path, value));
}

/// The listed entry whose composed uid is `target_uid`, with its path.
fn find_entity_by_uid(
    listing: &HashMap<PathBuf, RemoteEntity>,
    target_uid: &str,
) -> Option<(PathBuf, RemoteEntity)> {
    listing
        .iter()
        .find(|(_, entity)| entity.remote_id().as_deref() == Some(target_uid))
        .map(|(path, entity)| (path.clone(), entity.clone()))
}

/// Whether `uid` is a composed `volumeId~nodeId` of this volume. A legacy raw id or an id of
/// another volume can never match an event of this stream, so it does not name its record.
fn is_composed_on(volume_id: &str, uid: &str) -> bool {
    uid.strip_prefix(volume_id)
        .is_some_and(|rest| rest.starts_with('~'))
}

/// Whether the map already holds a directory at `path` that no composed uid names — the folder
/// the daemon created (or a legacy-id one) that this event is the first to give an id.
fn is_unnamed_directory_record(
    remote: &HashMap<PathBuf, RemoteEntity>,
    path: &Path,
    volume_id: &str,
) -> bool {
    matches!(
        remote.get(path),
        Some(RemoteEntity::Directory(directory))
            if !directory
                .id
                .as_deref()
                .is_some_and(|id| is_composed_on(volume_id, id))
    )
}

/// Materializes a baseline [`FileRecord`] into the [`RemoteEntity`] the planner consumes. The
/// index records the remote state as of the last cursor, so a synced record *is* the last-known
/// remote node.
fn remote_entity_from_record(record: &FileRecord) -> RemoteEntity {
    let name = record
        .file_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_default();
    match record.entity_kind {
        EntityKind::Directory => RemoteEntity::Directory(RemoteDirectory {
            path: record.file_path.clone(),
            id: record.proton_id.clone(),
            name,
        }),
        EntityKind::File => RemoteEntity::File(RemoteFile {
            path: record.file_path.clone(),
            // A just-uploaded file has no `proton_id` until a full listing backfills it; an empty
            // id is harmless here because deletes/moves address the remote by path, and the
            // periodic safety resync backfills the real id.
            id: record.proton_id.clone().unwrap_or_default(),
            name,
            sha1_hash: record.sha1_hash.clone(),
            // A baseline record is a file the engine already synced, hence downloadable by
            // definition (unsupported Proton-native files never get a base record). This matches
            // what a full listing would report for the same file.
            downloadable: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boxed_error;
    use crate::index::SyncStatus;
    use std::cell::RefCell;

    const VOLUME: &str = "vol";
    /// The pair's own remote root, as the daemon would hand it in.
    const ROOT: Option<&str> = Some("vol~root");

    fn scan_options(excludes: &[&str]) -> ScanOptions {
        let excludes: Vec<String> = excludes.iter().map(|s| (*s).to_owned()).collect();
        ScanOptions::new(
            Path::new("/root"),
            &[],
            &[],
            &excludes,
            &crate::sync::ConflictNaming::default(),
        )
        .expect("scan options")
    }

    fn file_record(path: &str, sha1: &str, proton_id: Option<&str>) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::File,
            file_size: 1,
            mtime: 0,
            sha1_hash: Some(sha1.to_owned()),
            proton_id: proton_id.map(ToOwned::to_owned),
            sync_status: SyncStatus::Synced,
        }
    }

    fn directory_record(path: &str, proton_id: Option<&str>) -> FileRecord {
        FileRecord {
            entity_kind: EntityKind::Directory,
            sha1_hash: None,
            ..file_record(path, "unused", proton_id)
        }
    }

    fn remote_file(path: &str, id: &str, sha1: &str) -> RemoteEntity {
        RemoteEntity::File(RemoteFile {
            path: PathBuf::from(path),
            id: id.to_owned(),
            name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            sha1_hash: Some(sha1.to_owned()),
            downloadable: true,
        })
    }

    fn remote_directory(path: &str, id: &str) -> RemoteEntity {
        RemoteEntity::Directory(RemoteDirectory {
            path: PathBuf::from(path),
            id: Some(id.to_owned()),
            name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        })
    }

    fn entity_path(entity: &RemoteEntity) -> PathBuf {
        match entity {
            RemoteEntity::File(file) => file.path.clone(),
            RemoteEntity::Directory(directory) => directory.path.clone(),
        }
    }

    fn change(
        kind: RemoteChangeKind,
        node_id: &str,
        parent_id: Option<&str>,
        trashed: bool,
    ) -> RemoteChange {
        RemoteChange {
            kind,
            node_id: node_id.to_owned(),
            parent_id: parent_id.map(ToOwned::to_owned),
            trashed,
            shared: false,
            event_id: format!("evt-{node_id}"),
        }
    }

    fn created(node_id: &str, parent_id: &str) -> RemoteChange {
        change(RemoteChangeKind::Created, node_id, Some(parent_id), false)
    }

    fn updated(node_id: &str, parent_id: &str) -> RemoteChange {
        change(RemoteChangeKind::Updated, node_id, Some(parent_id), false)
    }

    fn deleted(node_id: &str) -> RemoteChange {
        change(RemoteChangeKind::Deleted, node_id, None, false)
    }

    fn trashed(node_id: &str, parent_id: &str) -> RemoteChange {
        change(RemoteChangeKind::Updated, node_id, Some(parent_id), true)
    }

    /// The two questions a pair can put to its remote and its index, answered from a map each.
    #[derive(Default)]
    struct FakeResolver {
        /// What `path_for_proton_id` would answer.
        indexed: HashMap<String, PathBuf>,
        /// The remote tree as it is now, by root-relative path.
        tree: HashMap<PathBuf, RemoteEntity>,
        /// Directories whose listing fails.
        unlistable: HashSet<PathBuf>,
        /// Every directory listed, in order.
        listed: RefCell<Vec<PathBuf>>,
    }

    impl FakeResolver {
        fn with_tree(entities: Vec<RemoteEntity>) -> Self {
            Self {
                tree: entities
                    .into_iter()
                    .map(|entity| (entity_path(&entity), entity))
                    .collect(),
                ..Default::default()
            }
        }

        fn indexed(mut self, uid: &str, path: &str) -> Self {
            self.indexed.insert(uid.to_owned(), PathBuf::from(path));
            self
        }

        fn listings(&self) -> Vec<PathBuf> {
            self.listed.borrow().clone()
        }
    }

    impl RemoteChangeResolver for FakeResolver {
        fn indexed_path(&self, uid: &str) -> AppResult<Option<PathBuf>> {
            Ok(self.indexed.get(uid).cloned())
        }

        fn list(&self, relative_directory: &Path) -> AppResult<Rc<HashMap<PathBuf, RemoteEntity>>> {
            self.listed
                .borrow_mut()
                .push(relative_directory.to_path_buf());
            if self.unlistable.contains(relative_directory) {
                return Err(boxed_error("targeted listing failed"));
            }
            Ok(Rc::new(
                self.tree
                    .iter()
                    .filter(|(path, _)| path.parent() == Some(relative_directory))
                    .map(|(path, entity)| (path.clone(), entity.clone()))
                    .collect(),
            ))
        }
    }

    fn reconstruct(
        base: &HashMap<PathBuf, FileRecord>,
        changes: &[RemoteChange],
        root_uid: Option<&str>,
        resolver: &FakeResolver,
    ) -> Reconstruction {
        reconstruct_remote(
            base,
            changes,
            VOLUME,
            root_uid,
            &scan_options(&[]),
            resolver,
        )
    }

    fn complete(reconstruction: Reconstruction) -> (HashMap<PathBuf, RemoteEntity>, usize) {
        match reconstruction {
            Reconstruction::Complete { remote, outside } => (remote, outside),
            Reconstruction::FallbackToSnapshot(reason) => {
                panic!("expected a complete reconstruction, got fallback: {reason}")
            }
        }
    }

    fn expect_fallback(outcome: Reconstruction, fragment: &str) {
        match outcome {
            Reconstruction::FallbackToSnapshot(reason) => assert!(
                reason.contains(fragment),
                "fallback reason should mention {fragment:?}: {reason}"
            ),
            Reconstruction::Complete { remote, .. } => {
                panic!(
                    "expected a snapshot fallback mentioning {fragment:?}, got a map of {:?}",
                    remote.keys().collect::<Vec<_>>()
                )
            }
        }
    }

    fn base_of(records: Vec<FileRecord>) -> HashMap<PathBuf, FileRecord> {
        records
            .into_iter()
            .map(|record| (record.file_path.clone(), record))
            .collect()
    }

    /// A folder `docs` holding `docs/a.txt`, every record carrying its composed id: the fully
    /// named tree every "skip" test starts from.
    fn named_tree() -> HashMap<PathBuf, FileRecord> {
        base_of(vec![
            directory_record("docs", Some("vol~docs")),
            file_record("docs/a.txt", "hash-a", Some("vol~a")),
        ])
    }

    // --- the behaviour before #456, kept ---------------------------------------------------

    #[test]
    fn unchanged_base_is_reproduced_verbatim_with_no_changes() {
        let base = base_of(vec![file_record("dir/a.txt", "hash-a", Some("vol~node-a"))]);
        let (map, outside) = complete(reconstruct(&base, &[], None, &FakeResolver::default()));
        assert_eq!(map.len(), 1);
        assert_eq!(outside, 0);
        assert_eq!(
            map[Path::new("dir/a.txt")].as_file().unwrap().sha1_hash,
            Some("hash-a".to_owned())
        );
    }

    #[test]
    fn a_created_node_is_added_via_its_indexed_parents_listing() {
        let base = base_of(vec![directory_record("dir", Some("vol~dir"))]);
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("dir", "vol~dir"),
            remote_file("dir/new.txt", "vol~node-new", "hash-new"),
        ])
        .indexed("vol~dir", "dir");
        let (map, _) = complete(reconstruct(
            &base,
            &[created("node-new", "dir")],
            None,
            &resolver,
        ));
        assert_eq!(
            map[Path::new("dir/new.txt")].as_file().unwrap().sha1_hash,
            Some("hash-new".to_owned())
        );
        assert_eq!(resolver.listings(), [PathBuf::from("dir")]);
    }

    #[test]
    fn an_updated_node_gets_a_fresh_digest() {
        let base = base_of(vec![file_record("a.txt", "old", Some("vol~node-a"))]);
        let resolver = FakeResolver::with_tree(vec![remote_file("a.txt", "vol~node-a", "new")]);
        let (map, _) = complete(reconstruct(
            &base,
            &[updated("node-a", "root")],
            None,
            &resolver,
        ));
        assert_eq!(
            map[Path::new("a.txt")].as_file().unwrap().sha1_hash,
            Some("new".to_owned())
        );
    }

    #[test]
    fn a_deleted_node_is_removed_from_the_map() {
        let base = base_of(vec![file_record("a.txt", "hash", Some("vol~node-a"))]);
        let (map, _) = complete(reconstruct(
            &base,
            &[deleted("node-a")],
            None,
            &FakeResolver::default(),
        ));
        assert!(
            map.is_empty(),
            "a deleted node must be absent so the planner can propagate it"
        );
    }

    #[test]
    fn a_trashed_node_arrives_as_updated_and_is_removed() {
        // Regression guard for the key subtlety: trashing is Updated + trashed, not Deleted.
        let base = base_of(vec![file_record("a.txt", "hash", Some("vol~node-a"))]);
        let (map, _) = complete(reconstruct(
            &base,
            &[trashed("node-a", "root")],
            None,
            &FakeResolver::default(),
        ));
        assert!(map.is_empty(), "a trashed node is a removal");
    }

    #[test]
    fn a_renamed_node_drops_its_stale_path() {
        let base = base_of(vec![file_record("old.txt", "hash", Some("vol~node-a"))]);
        let resolver = FakeResolver::with_tree(vec![remote_file("new.txt", "vol~node-a", "hash")]);
        let (map, _) = complete(reconstruct(
            &base,
            &[updated("node-a", "root")],
            None,
            &resolver,
        ));
        assert!(
            !map.contains_key(Path::new("old.txt")),
            "stale path must be dropped"
        );
        assert!(
            map.contains_key(Path::new("new.txt")),
            "new path must be present"
        );
    }

    #[test]
    fn an_unresolvable_change_signals_a_full_snapshot() {
        let resolver = FakeResolver {
            unlistable: HashSet::from([PathBuf::new()]),
            ..Default::default()
        };
        let outcome = reconstruct(
            &HashMap::new(),
            &[created("node-x", "root")],
            None,
            &resolver,
        );
        expect_fallback(outcome, "node-x");
    }

    #[test]
    fn an_excluded_created_node_is_never_added() {
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("secret", "vol~secret"),
            remote_file("secret/keys.txt", "vol~node-secret", "hash"),
        ])
        .indexed("vol~secret", "secret");
        let outcome = reconstruct_remote(
            &HashMap::new(),
            &[created("node-secret", "secret")],
            VOLUME,
            None,
            &scan_options(&["secret/**"]),
            &resolver,
        );
        let (map, _) = complete(outcome);
        assert!(
            map.is_empty(),
            "an excluded path in the delta must never be planned or tracked"
        );
    }

    #[test]
    fn a_delete_for_an_untracked_node_is_a_safe_skip() {
        let base = base_of(vec![file_record("keep.txt", "hash", Some("vol~node-keep"))]);
        let (map, outside) = complete(reconstruct(
            &base,
            &[deleted("unknown-node")],
            None,
            &FakeResolver::default(),
        ));
        assert_eq!(
            map.len(),
            1,
            "an untracked delete must not fall back or disturb the map"
        );
        assert!(map.contains_key(Path::new("keep.txt")));
        assert_eq!(outside, 1);
    }

    #[test]
    fn an_updated_node_absent_from_its_parent_drops_its_stale_location() {
        // Updated-only: the node was already tracked, so absence is a real move/trash. (A
        // *Created* absence is the lagging-listing case and forces a snapshot instead — see
        // `a_created_node_absent_from_its_parent_forces_a_snapshot`.)
        let base = base_of(vec![
            directory_record("docs", Some("vol~docs")),
            file_record("docs/a.txt", "hash", Some("vol~node-a")),
        ]);
        let resolver = FakeResolver::with_tree(vec![remote_directory("docs", "vol~docs")])
            .indexed("vol~docs", "docs");
        let (map, _) = complete(reconstruct(
            &base,
            &[updated("node-a", "docs")],
            None,
            &resolver,
        ));
        assert!(
            !map.contains_key(Path::new("docs/a.txt")),
            "a node no longer in its parent is dropped"
        );
    }

    #[test]
    fn a_created_node_absent_from_its_parent_forces_a_snapshot() {
        // #30: a create absent from its parent listing is the CLI listing lagging the event
        // stream, not a vanished node. Dropping it would advance the cursor past a create that
        // nothing re-derives (the periodic resync is off by default, and a restart warm-starts).
        let base = base_of(vec![directory_record("docs", Some("vol~docs"))]);
        let resolver = FakeResolver::with_tree(vec![remote_directory("docs", "vol~docs")])
            .indexed("vol~docs", "docs");
        let outcome = reconstruct(&base, &[created("node-new", "docs")], None, &resolver);
        expect_fallback(outcome, "node-new");
    }

    #[test]
    fn a_created_node_absent_from_its_parent_still_forces_a_snapshot_with_a_root_uid() {
        // #30 again, on the path #456 added: the parent is the root, which is now a known parent.
        let base = named_tree();
        let resolver = FakeResolver::default();
        let outcome = reconstruct(&base, &[created("node-new", "root")], ROOT, &resolver);
        expect_fallback(outcome, "not in its parent listing yet");
    }

    #[test]
    fn a_directory_removal_cascades_to_its_tracked_descendants() {
        // Volume events are per-link: a folder trash emits no events for descendants.
        // Without the cascade the map would claim `docs/a.txt` still exists remotely and
        // the planner would recreate the folder the user just deleted.
        let mut base = named_tree();
        base.extend(base_of(vec![file_record(
            "other.txt",
            "hash-o",
            Some("vol~node-o"),
        )]));
        let (map, _) = complete(reconstruct(
            &base,
            &[trashed("docs", "root")],
            None,
            &FakeResolver::default(),
        ));
        assert!(
            !map.contains_key(Path::new("docs")) && !map.contains_key(Path::new("docs/a.txt")),
            "the folder AND its descendants must be gone: {:?}",
            map.keys().collect::<Vec<_>>()
        );
        assert!(map.contains_key(Path::new("other.txt")));
    }

    #[test]
    fn an_untracked_removal_falls_back_when_a_baseline_record_lacks_a_composed_id() {
        // `b.txt` was just uploaded (no proton_id yet). A trash event for it cannot be
        // matched, but skipping would advance the cursor past the deletion forever — a
        // full snapshot is the only safe answer.
        let base = base_of(vec![file_record("b.txt", "hash-b", None)]);
        let outcome = reconstruct(
            &base,
            &[trashed("node-b", "root")],
            None,
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "node-b");
    }

    #[test]
    fn an_untracked_removal_falls_back_when_a_baseline_record_has_a_legacy_raw_id() {
        let base = base_of(vec![file_record("old.txt", "hash", Some("raw-legacy-id"))]);
        let outcome = reconstruct(&base, &[deleted("node-z")], None, &FakeResolver::default());
        expect_fallback(outcome, "node-z");
    }

    #[test]
    fn duplicate_baseline_proton_ids_force_a_snapshot() {
        // Reachable transient state: a withheld LocalDelete pins the old row while a
        // Download committed a new row with the same id. Seeding order would otherwise
        // decide which path a replayed move event resolves against.
        let base = base_of(vec![
            file_record("a.txt", "hash", Some("vol~node-dup")),
            file_record("b.txt", "hash", Some("vol~node-dup")),
        ]);
        let outcome = reconstruct(&base, &[], None, &FakeResolver::default());
        expect_fallback(outcome, "vol~node-dup");
    }

    #[test]
    fn empty_placeholder_ids_do_not_alias_each_other_in_seeding() {
        // Two just-uploaded records (id not yet backfilled) must not collide on "" —
        // with no removal events in the delta this reconstruction stays complete.
        let base = base_of(vec![
            file_record("x.txt", "hash-x", Some("")),
            file_record("y.txt", "hash-y", Some("")),
        ]);
        let (map, _) = complete(reconstruct(&base, &[], None, &FakeResolver::default()));
        assert_eq!(map.len(), 2);
    }

    // --- #456: an event about something else on the volume ------------------------------------

    #[test]
    fn a_foreign_create_is_skipped_when_every_directory_has_an_id() {
        let base = named_tree();
        let resolver = FakeResolver::default();
        let (map, outside) = complete(reconstruct(
            &base,
            &[created("elsewhere-file", "elsewhere-folder")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 1, "dropped, and counted");
        assert_eq!(map.len(), 2, "the pair's own map is untouched");
        assert!(
            resolver.listings().is_empty(),
            "a node in someone else's folder costs no listing: {:?}",
            resolver.listings()
        );
    }

    #[test]
    fn a_foreign_update_is_skipped_under_the_same_condition() {
        let (map, outside) = complete(reconstruct(
            &named_tree(),
            &[updated("elsewhere-file", "elsewhere-folder")],
            ROOT,
            &FakeResolver::default(),
        ));
        assert_eq!((map.len(), outside), (2, 1));
    }

    #[test]
    fn a_foreign_delete_is_skipped_when_every_record_has_an_id() {
        let (map, outside) = complete(reconstruct(
            &named_tree(),
            &[deleted("elsewhere-file"), trashed("elsewhere-2", "x")],
            ROOT,
            &FakeResolver::default(),
        ));
        assert_eq!((map.len(), outside), (2, 2));
    }

    #[test]
    fn a_foreign_event_costs_no_listing() {
        let resolver = FakeResolver::default();
        let (_, outside) = complete(reconstruct(
            &named_tree(),
            &[
                created("e1", "elsewhere"),
                updated("e2", "elsewhere"),
                deleted("e3"),
                created("e4", "another"),
            ],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 4);
        assert!(resolver.listings().is_empty());
    }

    #[test]
    fn a_foreign_event_that_precedes_the_daemons_own_folder_event_in_one_delta_is_still_skipped() {
        // The daemon created `docs` last pass: its record has no id yet. Its Created event is in
        // this delta, but AFTER a foreign event — so at the foreign event's position the tree is
        // not fully named. The decision waits for the end of the delta.
        let base = base_of(vec![directory_record("docs", None)]);
        let resolver = FakeResolver::with_tree(vec![remote_directory("docs", "vol~docs")]);
        let (map, outside) = complete(reconstruct(
            &base,
            &[created("foreign", "elsewhere"), created("docs", "root")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 1);
        assert_eq!(
            map[Path::new("docs")].remote_id().as_deref(),
            Some("vol~docs"),
            "and the folder was given its id"
        );
    }

    #[test]
    fn a_foreign_delete_that_precedes_the_daemons_own_file_event_is_still_skipped() {
        let base = base_of(vec![file_record("up.txt", "hash", None)]);
        let resolver = FakeResolver::with_tree(vec![remote_file("up.txt", "vol~up", "hash")]);
        let (map, outside) = complete(reconstruct(
            &base,
            &[deleted("foreign"), created("up", "root")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 1);
        assert_eq!(
            map[Path::new("up.txt")].remote_id().as_deref(),
            Some("vol~up")
        );
    }

    #[test]
    fn a_create_under_a_folder_created_in_the_same_delta_is_placed() {
        // The ADR's "second instance": `mkdir` then copy files in, one delta. The new folder has
        // no index row, so only the in-pass map can place its children.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("newdir", "vol~newdir"),
            remote_file("newdir/f.txt", "vol~f", "hash"),
        ]);
        let (map, outside) = complete(reconstruct(
            &HashMap::new(),
            &[created("newdir", "root"), created("f", "newdir")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 0);
        assert!(map.contains_key(Path::new("newdir")));
        assert!(map.contains_key(Path::new("newdir/f.txt")));
        assert_eq!(
            resolver.listings(),
            [PathBuf::new(), PathBuf::from("newdir")]
        );
    }

    #[test]
    fn a_create_under_a_folder_created_in_the_same_delta_is_placed_without_a_root_uid_too() {
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("newdir", "vol~newdir"),
            remote_file("newdir/f.txt", "vol~f", "hash"),
        ]);
        let (map, _) = complete(reconstruct(
            &HashMap::new(),
            &[created("newdir", "root"), created("f", "newdir")],
            None,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("newdir/f.txt")));
    }

    #[test]
    fn a_file_another_client_creates_inside_a_folder_the_daemon_just_created_is_placed() {
        // The hole case: `docs` is the daemon's own folder (no id yet), and someone else puts a
        // file in it. The folder's Created event comes first in the delta, so the file's parent
        // is known by then.
        let base = base_of(vec![directory_record("docs", None)]);
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/theirs.txt", "vol~theirs", "hash"),
        ]);
        let (map, outside) = complete(reconstruct(
            &base,
            &[created("docs", "root"), created("theirs", "docs")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 0);
        assert!(map.contains_key(Path::new("docs/theirs.txt")));
    }

    #[test]
    fn an_unknown_parent_while_a_directory_has_no_id_falls_back_and_names_it() {
        // The folder's own Created event has not arrived (the stream lags): the unknown parent
        // may be that folder. Skipping would advance the cursor past a create nothing re-derives.
        let base = base_of(vec![directory_record("docs", None)]);
        let outcome = reconstruct(
            &base,
            &[created("maybe-inside", "unknown-parent")],
            ROOT,
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "docs has no composed id");
    }

    #[test]
    fn an_unknown_parent_while_a_directory_has_a_legacy_raw_id_falls_back() {
        let base = base_of(vec![directory_record("docs", Some("raw-legacy-id"))]);
        let outcome = reconstruct(
            &base,
            &[created("maybe-inside", "unknown-parent")],
            ROOT,
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "docs has no composed id");
    }

    #[test]
    fn an_empty_placeholder_id_counts_as_missing_for_the_skip() {
        let base = base_of(vec![directory_record("d", Some(""))]);
        let outcome = reconstruct(
            &base,
            &[created("maybe-inside", "unknown-parent")],
            ROOT,
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "d has no composed id");
    }

    #[test]
    fn a_directory_of_another_volume_does_not_name_itself() {
        let base = base_of(vec![directory_record("docs", Some("other~docs"))]);
        let outcome = reconstruct(
            &base,
            &[created("maybe-inside", "unknown-parent")],
            ROOT,
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "docs has no composed id");
    }

    #[test]
    fn a_file_without_an_id_does_not_block_skipping_a_create() {
        // Files cannot be a parent. Only the removal arm needs them named.
        let base = base_of(vec![file_record("up.txt", "hash", None)]);
        let (_, outside) = complete(reconstruct(
            &base,
            &[created("elsewhere", "elsewhere-folder")],
            ROOT,
            &FakeResolver::default(),
        ));
        assert_eq!(outside, 1);
    }

    #[test]
    fn a_foreign_removal_names_the_record_that_has_no_id() {
        let base = base_of(vec![
            directory_record("docs", Some("vol~docs")),
            file_record("docs/up.txt", "hash", None),
        ]);
        let outcome = reconstruct(
            &base,
            &[deleted("maybe-the-upload")],
            ROOT,
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "docs/up.txt has no composed id");
    }

    #[test]
    fn without_a_root_uid_an_unplaceable_event_falls_back_as_before() {
        // No regression for a pair that has not learned its root: the root listing is consulted,
        // and a node in neither place forces the snapshot with the old reason.
        let resolver = FakeResolver::default();
        let outcome = reconstruct(
            &named_tree(),
            &[created("foreign", "elsewhere")],
            None,
            &resolver,
        );
        expect_fallback(outcome, "not under any indexed parent or the remote root");
        assert_eq!(
            resolver.listings(),
            [PathBuf::new()],
            "the root was listed, as it always was"
        );
    }

    #[test]
    fn without_a_root_uid_a_node_in_the_root_listing_is_still_placed() {
        let resolver = FakeResolver::with_tree(vec![remote_file("top.txt", "vol~top", "hash")]);
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[created("top", "whatever")],
            None,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("top.txt")));
    }

    #[test]
    fn a_root_uid_of_another_volume_is_ignored() {
        let resolver = FakeResolver::default();
        let outcome = reconstruct(
            &named_tree(),
            &[created("foreign", "elsewhere")],
            Some("other~root"),
            &resolver,
        );
        expect_fallback(outcome, "not under any indexed parent or the remote root");
    }

    #[test]
    fn the_pairs_own_root_event_is_never_skipped() {
        for event in [
            created("root", "its-parent"),
            updated("root", "its-parent"),
            deleted("root"),
            trashed("root", "its-parent"),
        ] {
            let outcome = reconstruct(&named_tree(), &[event], ROOT, &FakeResolver::default());
            expect_fallback(outcome, "own remote root");
        }
    }

    #[test]
    fn a_node_with_no_parent_id_is_skipped_only_when_the_tree_is_fully_named() {
        let event = change(RemoteChangeKind::Created, "volume-level", None, false);
        let (_, outside) = complete(reconstruct(
            &named_tree(),
            std::slice::from_ref(&event),
            ROOT,
            &FakeResolver::default(),
        ));
        assert_eq!(outside, 1);

        let unnamed = base_of(vec![directory_record("docs", None)]);
        expect_fallback(
            reconstruct(&unnamed, &[event], ROOT, &FakeResolver::default()),
            "docs has no composed id",
        );
    }

    #[test]
    fn an_event_for_a_tracked_node_that_names_no_parent_forces_a_snapshot() {
        // A tracked node is inside the tree, so "no parent" cannot be a move out of it: reading
        // it as one would remove a file the user still has.
        let event = change(RemoteChangeKind::Updated, "a", None, false);
        expect_fallback(
            reconstruct(&named_tree(), &[event], ROOT, &FakeResolver::default()),
            "names no parent",
        );
    }

    #[test]
    fn a_direct_child_of_the_root_is_placed_by_listing_the_root() {
        let resolver = FakeResolver::with_tree(vec![remote_file("top.txt", "vol~top", "hash")]);
        let (map, outside) = complete(reconstruct(
            &named_tree(),
            &[created("top", "root")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 0);
        assert!(map.contains_key(Path::new("top.txt")));
        assert_eq!(resolver.listings(), [PathBuf::new()]);
    }

    #[test]
    fn a_child_of_an_indexed_folder_is_placed_by_listing_that_folder() {
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/b.txt", "vol~b", "hash"),
        ])
        .indexed("vol~docs", "docs");
        // `docs` is in the base, so the in-pass map names it; the index is not even asked.
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[created("b", "docs")],
            ROOT,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("docs/b.txt")));
        assert_eq!(resolver.listings(), [PathBuf::from("docs")]);
    }

    #[test]
    fn a_tracked_node_moved_to_an_unknown_parent_is_removed_with_its_subtree() {
        let (map, outside) = complete(reconstruct(
            &named_tree(),
            &[updated("docs", "elsewhere-folder")],
            ROOT,
            &FakeResolver::default(),
        ));
        assert!(map.is_empty(), "the folder left the tree with its children");
        assert_eq!(
            outside, 0,
            "a move out is an applied change, not a foreign one"
        );
    }

    #[test]
    fn a_tracked_node_moved_out_is_kept_when_the_tree_is_not_fully_named() {
        // Its new parent could be the folder the daemon just made: it may not have left at all.
        let mut base = named_tree();
        base.extend(base_of(vec![directory_record("fresh", None)]));
        expect_fallback(
            reconstruct(
                &base,
                &[updated("a", "unknown-parent")],
                ROOT,
                &FakeResolver::default(),
            ),
            "fresh has no composed id",
        );
    }

    #[test]
    fn a_move_out_followed_by_a_move_back_in_one_delta_keeps_the_node() {
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/a.txt", "vol~a", "hash-a"),
        ]);
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[updated("a", "elsewhere-folder"), updated("a", "docs")],
            ROOT,
            &resolver,
        ));
        assert!(
            map.contains_key(Path::new("docs/a.txt")),
            "the later event is the node's final word"
        );
    }

    #[test]
    fn a_directory_moved_in_from_outside_falls_back() {
        // Per-link events: nothing describes what is inside it.
        let resolver = FakeResolver::with_tree(vec![remote_directory("incoming", "vol~incoming")]);
        expect_fallback(
            reconstruct(
                &named_tree(),
                &[updated("incoming", "root")],
                ROOT,
                &resolver,
            ),
            "incoming",
        );
    }

    #[test]
    fn a_directory_moved_in_from_outside_falls_back_without_a_root_uid_too() {
        let resolver = FakeResolver::with_tree(vec![remote_directory("incoming", "vol~incoming")]);
        expect_fallback(
            reconstruct(
                &named_tree(),
                &[updated("incoming", "root")],
                None,
                &resolver,
            ),
            "incoming",
        );
    }

    #[test]
    fn a_directory_restored_from_trash_falls_back() {
        // Trashed and restored in one delta: the trash cascaded over its subtree, and the
        // restore names nothing under it.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/a.txt", "vol~a", "hash-a"),
        ]);
        expect_fallback(
            reconstruct(
                &named_tree(),
                &[trashed("docs", "root"), updated("docs", "root")],
                ROOT,
                &resolver,
            ),
            "docs",
        );
    }

    #[test]
    fn a_created_directory_is_placed_as_before() {
        let resolver = FakeResolver::with_tree(vec![remote_directory("fresh", "vol~fresh")]);
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[created("fresh", "root")],
            ROOT,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("fresh")));
    }

    #[test]
    fn an_updated_directory_the_daemon_created_is_linked_not_treated_as_moved_in() {
        // The daemon's own `docs` has no id; its first event reaches the pair as an update.
        let base = base_of(vec![directory_record("docs", None)]);
        let resolver = FakeResolver::with_tree(vec![remote_directory("docs", "vol~docs")]);
        let (map, _) = complete(reconstruct(
            &base,
            &[updated("docs", "root")],
            ROOT,
            &resolver,
        ));
        assert_eq!(
            map[Path::new("docs")].remote_id().as_deref(),
            Some("vol~docs")
        );
    }

    #[test]
    fn a_file_moved_in_from_outside_is_placed() {
        let resolver = FakeResolver::with_tree(vec![remote_file("in.txt", "vol~in", "hash")]);
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[updated("in", "root")],
            ROOT,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("in.txt")));
    }

    #[test]
    fn a_create_under_an_excluded_directory_is_dropped_as_the_walk_would_not_list_it() {
        // `secret` is excluded and was never indexed: its children's events name an unknown
        // parent. A full walk would not list them either.
        let outcome = reconstruct_remote(
            &named_tree(),
            &[created("hidden", "secret")],
            VOLUME,
            ROOT,
            &scan_options(&["secret/**"]),
            &FakeResolver::default(),
        );
        let (map, outside) = complete(outcome);
        assert_eq!((map.len(), outside), (2, 1));
    }

    #[test]
    fn a_node_moved_into_an_excluded_directory_leaves_the_map() {
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("secret", "vol~secret"),
            remote_file("secret/a.txt", "vol~a", "hash-a"),
        ])
        .indexed("vol~secret", "secret");
        let outcome = reconstruct_remote(
            &named_tree(),
            &[updated("a", "secret")],
            VOLUME,
            ROOT,
            &scan_options(&["secret/**"]),
            &resolver,
        );
        let (map, _) = complete(outcome);
        assert!(!map.contains_key(Path::new("docs/a.txt")));
        assert!(!map.contains_key(Path::new("secret/a.txt")));
    }

    #[test]
    fn a_listing_that_fails_for_a_known_parent_forces_a_snapshot() {
        let resolver = FakeResolver {
            unlistable: HashSet::from([PathBuf::from("docs")]),
            ..Default::default()
        };
        expect_fallback(
            reconstruct(&named_tree(), &[created("b", "docs")], ROOT, &resolver),
            "could not resolve remote change for node b",
        );
    }
}
