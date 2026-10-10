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

use crate::events::{RemoteChange, RemoteChangeKind, node_uid};
use crate::index::{EntityKind, FileRecord, ScanOptions};
use crate::proton::{RemoteDirectory, RemoteEntity, RemoteFile};
use crate::{AppResult, boxed_error};
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
    /// A created/updated node that is not in the pair's map, whose event names a parent the pair
    /// does not know. (A node that *is* in the map and names one is a walk at once: it moved out of
    /// the tree or into a folder the pair cannot see into, and the event cannot say which.)
    Placement,
    /// A removal of a node that is not in the pair's map.
    Removal,
}

struct Deferred {
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
/// listing does not hold forces a snapshot (the behaviour before #456; the in-pass map does not
/// place a child either, because it places a node from the end-state listing and can hold a path
/// the delta later takes away, but a parent it has placed or moved forces a snapshot, because the
/// index's path for it is stale). **It is treated as unknown whenever the scan options carry include
/// patterns**: an include rule leaves the folders between the root and a matching file without
/// rows, so the index no longer holds the tree's folders and "not any folder I know" says nothing.
/// With it known:
///
/// * the root is a known parent (`""`), and an event about the root itself always forces a
///   snapshot — it is never skipped and never placed;
/// * an event for a node the pair does **not** hold, whose parent is unknown, is **deferred**, and
///   dropped at the end of the delta only if the pair's tree is fully named and fully held
///   (condition S): every directory in the final map carries a composed uid, no record of the
///   baseline carries an id that is not one of this volume's (a raw id of an older index, another
///   volume's id; the upload window's `None` and `""` do not count), **and** every
///   record sits directly in the root or in a held folder (a baseline record is checked against the
///   baseline's directory records, a record of the final map against the final map's). Then the
///   parent is neither the root nor any directory of the tree, so the node is somewhere else on the
///   volume. A record under a folder with no row (an index written before folders were rows, or a
///   folder an exclude rule has since hidden) could be sitting in exactly the folder the event
///   names, and a record under an older id could be the node itself, moved out. Otherwise the pass
///   falls back to a snapshot, naming the record that blocked it;
/// * an event that names **no** parent is never deferred: nothing says where its node is;
/// * an event for a node the pair **does** hold that names an unknown parent forces a snapshot at
///   once. It left the tree or went into a folder the pair cannot see into (no row, nothing
///   recorded beneath it), the event cannot say which, and reading it as "left" removed a file the
///   user still has;
/// * a removal of a node the pair does not track is dropped at the end of the delta under the same
///   rule extended to files (every record carries a composed uid, no root uid needed): the removed
///   node could only have been one of the records that lack an id.
///
/// A directory placed by an `Updated` while untracked (moved in from elsewhere, or restored from
/// the trash) forces a snapshot, because volume events are per link and its subtree has none. So
/// does an event naming a parent that an earlier event of the same delta took out of the map: the
/// index still holds that parent's old path, which may by now belong to another folder.
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

    // A root uid from another volume cannot be the parent of anything this stream describes, and
    // under include rules the index does not hold the tree's folders (see above).
    let root_uid = remote_root_uid
        .filter(|uid| is_composed_on(volume_id, uid))
        .filter(|_| !scan_options.has_include_patterns());
    let mut deferred: Vec<Deferred> = Vec::new();
    // Every uid taken out of `uid_to_path` by an event of this delta. The index still holds such a
    // node's old path, and the path may belong to another folder by now.
    let mut taken_out: HashSet<String> = HashSet::new();

    for change in changes {
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
                remove_subtree(&mut remote, &mut uid_to_path, &mut taken_out, &path);
                taken_out.insert(uid);
            } else {
                // Not in the map: either a node elsewhere on the volume, or a record of this pair
                // that has no uid yet (just uploaded). Which one is only knowable once the whole
                // delta has been read — the record's own Created event may be later in it.
                deferred.push(Deferred {
                    node_id: change.node_id.clone(),
                    kind: DeferredKind::Removal,
                });
            }
            continue;
        }

        // Created, or Updated (not trashed): find the node's parent, then its place in the
        // parent's listing.
        let parent_path = match locate_parent(
            change,
            volume_id,
            root_uid,
            &uid_to_path,
            &taken_out,
            resolver,
        ) {
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
                // No parent at all says nothing about where the node is: it may be a node restored
                // from the trash into this very tree. Only a parent that is not ours can say
                // "somewhere else".
                if change.parent_id.is_none() {
                    return Reconstruction::FallbackToSnapshot(format!(
                        "event for node {} names no parent",
                        change.node_id
                    ));
                }
                // A node the pair holds that names a parent it does not hold either left the tree
                // or went into a folder it cannot see into (no row, nothing recorded beneath it).
                // The event cannot say which, and removing it on the first reading planned the
                // deletion of a file that was still there. "Holds" includes a node an earlier
                // event of this delta took out of the map (an update the end-state listing could
                // not find where it said): its next event is the one that says where it went.
                if uid_to_path.contains_key(&uid) || taken_out.contains(&uid) {
                    return Reconstruction::FallbackToSnapshot(format!(
                        "node {} is in this folder and names a parent it does not hold",
                        change.node_id
                    ));
                }
                deferred.push(Deferred {
                    node_id: change.node_id.clone(),
                    kind: DeferredKind::Placement,
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
                    uid_to_path.insert(uid, path.clone());
                    remote.insert(path, entity);
                } else if uid_to_path.remove(&uid).is_some() {
                    // Excluded by selective sync: neither present as remote nor tracked, so it is
                    // never planned or purged.
                    taken_out.insert(uid);
                }
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
                // any stale location, and what was beneath it (events are per link, so nothing
                // else will say that a folder's children went with it).
                if let Some(old_path) = uid_to_path.remove(&uid) {
                    remove_subtree(&mut remote, &mut uid_to_path, &mut taken_out, &old_path);
                    taken_out.insert(uid);
                }
            }
        }
    }

    let mut outside = 0;
    if !deferred.is_empty() {
        // Condition S, over the final map: which records carry a composed uid of this volume.
        let named: HashSet<PathBuf> = uid_to_path
            .iter()
            .filter(|(uid, _)| is_composed_on(volume_id, uid))
            .map(|(_, path)| path.clone())
            .collect();
        // A record of the baseline under a real id that no event of this stream can carry (a raw id
        // of an older index, another volume's id) may be the node an event is about: moved out,
        // or removed. Read from the **baseline**, not the final map: a different node may have
        // taken its path since, which would hide it, and nothing in the delta says what became of
        // the node itself. A walk rewrites such an id, so this costs a walk once.
        let legacy_in_base = base_index
            .iter()
            .filter(|(_, record)| {
                record
                    .proton_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty() && !is_composed_on(volume_id, id))
            })
            .map(|(path, _)| path)
            .min()
            .cloned();
        // A record with no id yet (the daemon uploaded it, or made the folder, last pass) is named
        // by its own `Created` event, which an event of a node that left cannot precede. So the
        // final map is the question: once that event is in the delta the record is named. A
        // directory is the unknown parent itself, so one without an id blocks a placement.
        let unnamed_directory = remote
            .iter()
            .filter(|(path, entity)| {
                matches!(entity, RemoteEntity::Directory(_)) && !named.contains(*path)
            })
            .map(|(path, _)| path)
            .min()
            .cloned();
        // What stops the removal of a node the pair does not hold from being dropped: a record that
        // carries no composed id of this volume, since the removed node may be that record. A pair
        // that knows its root reads the final map, where a record named by its own `Created` event
        // in this delta is named; one that does not decides as before #456, from the baseline
        // alone, and walks for any record without an id whatever the delta later names.
        let unnamed_record = if root_uid.is_some() {
            legacy_in_base.clone().or_else(|| {
                remote
                    .keys()
                    .filter(|path| !named.contains(*path))
                    .min()
                    .cloned()
            })
        } else {
            base_index
                .iter()
                .filter(|(_, record)| {
                    !record
                        .proton_id
                        .as_deref()
                        .is_some_and(|id| is_composed_on(volume_id, id))
                })
                .map(|(path, _)| path)
                .min()
                .cloned()
        };
        // A record that sits in a folder no record holds: the tree has a folder this pair cannot
        // name, and an event about a node it does not hold may be about that folder. Read only by a
        // pair that knows its root (an include rule makes it unknown, and then the index never held
        // the tree's folders to begin with), so a deferred placement, which exists only then,
        // always has it.
        let outside_any_folder =
            root_uid.and_then(|_| first_record_outside_any_folder(base_index, &remote));
        // The first condition S fails on, if any, for a placement.
        let placement_blocker = if let Some(path) = unnamed_directory.or(legacy_in_base) {
            Some(PlacementBlocker::Unnamed(path))
        } else {
            outside_any_folder
                .clone()
                .map(PlacementBlocker::RecordOutsideAnyFolder)
        };

        for event in deferred {
            match event.kind {
                DeferredKind::Placement => {
                    if let Some(blocker) = &placement_blocker {
                        return Reconstruction::FallbackToSnapshot(format!(
                            "cannot tell whether node {} is outside this folder: {}",
                            event.node_id,
                            blocker.describe()
                        ));
                    }
                    // The parent is not the root and not any folder of the tree, and the node was
                    // not held: it is somewhere else on the volume.
                    outside += 1;
                }
                DeferredKind::Removal => {
                    if let Some(path) = &unnamed_record {
                        return Reconstruction::FallbackToSnapshot(format!(
                            "removal of untracked node {} while {} has no composed id",
                            event.node_id,
                            path.display()
                        ));
                    }
                    // The node may be a folder with no row, trashed with the pair's records beneath
                    // it: the event is for its own link alone, and nothing in the delta says which
                    // records were under it.
                    if let Some(path) = &outside_any_folder {
                        return Reconstruction::FallbackToSnapshot(format!(
                            "removal of untracked node {} while {} is in a folder the index has no \
                             record of",
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
/// `None` when the pair does not know the parent — including an event that names none. An error
/// when the parent was taken out of the map by an earlier event of this delta: the index still
/// holds its old path, and that path may belong to another folder by now.
///
/// The in-pass map is trusted to *place* a child only when the root uid is known. It places a node
/// from the **end-state** listing, so it can hold a path that a later event of the delta takes
/// away; a pair that does not know its root keeps the order of the code before #456, the index and
/// then the root listing. It still asks the map whether the index can be trusted: a parent the
/// delta placed or moved is not where the index says, and the index's path may by now belong to
/// another folder, so that child walks.
fn locate_parent(
    change: &RemoteChange,
    volume_id: &str,
    root_uid: Option<&str>,
    uid_to_path: &HashMap<String, PathBuf>,
    taken_out: &HashSet<String>,
    resolver: &dyn RemoteChangeResolver,
) -> AppResult<Option<PathBuf>> {
    let Some(parent_id) = change.parent_id.as_deref() else {
        return Ok(None);
    };
    let parent_uid = node_uid(volume_id, parent_id);
    if let Some(in_pass) = uid_to_path.get(&parent_uid) {
        if root_uid.is_some() {
            return Ok(Some(in_pass.clone()));
        }
        let indexed = resolver.indexed_path(&parent_uid)?;
        if indexed.as_ref() != Some(in_pass) {
            return Err(boxed_error(format!(
                "its parent {parent_id} was placed or moved earlier in this delta, so the \
                 index does not say where it is"
            )));
        }
        return Ok(indexed);
    }
    if root_uid == Some(parent_uid.as_str()) {
        return Ok(Some(PathBuf::new()));
    }
    if taken_out.contains(&parent_uid) {
        return Err(boxed_error(format!(
            "its parent {parent_id} was taken out earlier in this delta"
        )));
    }
    resolver.indexed_path(&parent_uid)
}

/// Removes the entry at `path` and everything beneath it from both maps, recording the uids that
/// leave the second.
fn remove_subtree(
    remote: &mut HashMap<PathBuf, RemoteEntity>,
    uid_to_path: &mut HashMap<String, PathBuf>,
    taken_out: &mut HashSet<String>,
    path: &Path,
) {
    remote.remove(path);
    remote.retain(|key, _| !crate::sync::is_strict_descendant(path, key));
    uid_to_path.retain(|uid, value| {
        let beneath = crate::sync::is_strict_descendant(path, value);
        if beneath {
            taken_out.insert(uid.clone());
        }
        !beneath
    });
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

/// What stops an event with an unknown parent from being read as "somewhere else on the volume".
enum PlacementBlocker {
    /// A directory of the map has no composed uid, or a file's id is not one of this volume: the
    /// unknown parent may be that directory, the node that file.
    Unnamed(PathBuf),
    /// A record sits in a folder the map does not hold, so the tree has a folder this pair cannot
    /// name and the unknown parent may be it.
    RecordOutsideAnyFolder(PathBuf),
}

impl PlacementBlocker {
    fn describe(&self) -> String {
        match self {
            Self::Unnamed(path) => format!("{} has no composed id", path.display()),
            Self::RecordOutsideAnyFolder(path) => format!(
                "{} is in a folder the index has no record of",
                path.display()
            ),
        }
    }
}

/// The first (by path) record that sits in a folder no record holds, over the baseline **and** the
/// final map. The baseline is read too because a removal in this delta (a trash, a delete) takes a
/// record out of the map, and with it the evidence that the tree has a folder this pair cannot
/// name.
fn first_record_outside_any_folder(
    base_index: &HashMap<PathBuf, FileRecord>,
    remote: &HashMap<PathBuf, RemoteEntity>,
) -> Option<PathBuf> {
    let in_base = base_index.keys().filter(|path| {
        !parent_is_held(path, |parent| {
            base_index
                .get(parent)
                .is_some_and(|record| matches!(record.entity_kind, EntityKind::Directory))
        })
    });
    let in_map = remote.keys().filter(|path| {
        !parent_is_held(path, |parent| {
            matches!(remote.get(parent), Some(RemoteEntity::Directory(_)))
        })
    });
    in_base.chain(in_map).min().cloned()
}

/// Whether the folder `path` sits in is the pair's root, or a directory according to `is_directory`.
fn parent_is_held(path: &Path, is_directory: impl Fn(&Path) -> bool) -> bool {
    match path.parent() {
        None => true,
        Some(parent) if parent.as_os_str().is_empty() => true,
        Some(parent) => is_directory(parent),
    }
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
mod model_tests;

#[cfg(test)]
mod tests {
    use super::*;
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

    fn scan_options_including(includes: &[&str]) -> ScanOptions {
        let includes: Vec<String> = includes.iter().map(|s| (*s).to_owned()).collect();
        ScanOptions::new(
            Path::new("/root"),
            &[],
            &includes,
            &[],
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
    fn without_a_root_uid_a_create_under_a_folder_created_in_the_same_delta_walks_as_before() {
        // Round 2 of the review of #461: a pair that does not know its root decides as the code
        // did before #456, and that code asked the index, never the in-pass map. The in-pass map
        // places a node from the END-STATE listing, so it can hold a path the delta later takes
        // away (next test). It is asked only whether the index can be trusted, and for a folder
        // the delta placed it cannot: the index has never heard of it.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("newdir", "vol~newdir"),
            remote_file("newdir/f.txt", "vol~f", "hash"),
        ]);
        expect_fallback(
            reconstruct(
                &HashMap::new(),
                &[created("newdir", "root"), created("f", "newdir")],
                None,
                &resolver,
            ),
            "placed or moved earlier in this delta",
        );
    }

    /// The review's repro: the folder `n7` is made outside the tree and moved in after `n3`, which
    /// held the name `a`, is trashed; a file is made in it while it is still outside. The listing is
    /// the end state, so the first event already places `n7` at `a`.
    fn a_folder_that_takes_a_trashed_ones_name() -> (HashMap<PathBuf, FileRecord>, FakeResolver) {
        let base = base_of(vec![directory_record("a", Some("vol~n3"))]);
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("a", "vol~n7"),
            remote_file("a/f.txt", "vol~n8", "hash"),
        ]);
        (base, resolver)
    }

    fn the_folder_is_made_outside_then_moved_in() -> Vec<RemoteChange> {
        vec![
            created("n7", "top"),
            created("n8", "n7"),
            trashed("n3", "root"),
            updated("n7", "root"),
        ]
    }

    #[test]
    fn without_a_root_uid_a_parent_placed_in_this_delta_is_not_trusted_to_place_a_child() {
        // The first version of the fix answered Complete without `a/f.txt`: the second event was
        // placed through the in-pass map, the trash of `n3` then removed `a` with everything under
        // it, and the last event re-added only the folder. Before #456 the second event fell back.
        let (base, resolver) = a_folder_that_takes_a_trashed_ones_name();
        expect_fallback(
            reconstruct(
                &base,
                &the_folder_is_made_outside_then_moved_in(),
                None,
                &resolver,
            ),
            "placed or moved earlier in this delta",
        );
    }

    #[test]
    fn without_a_root_uid_a_parent_moved_earlier_in_the_delta_forces_a_snapshot() {
        // Found by the model test under an include rule, and older than #456 (the code before it
        // asked the index alone): `docs` moves under `more`, and a file is then moved into it. The
        // index still says `docs` is at `docs`, and the end-state listing there holds nothing, so
        // the file was "absent from its parent" and dropped without a word.
        let mut base = named_tree();
        base.extend(base_of(vec![directory_record("more", Some("vol~more"))]));
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("more", "vol~more"),
            remote_directory("more/docs", "vol~docs"),
            remote_file("more/docs/a.txt", "vol~a", "hash-a"),
            remote_file("more/docs/n5.txt", "vol~n5", "hash-5"),
        ])
        .indexed("vol~docs", "docs")
        .indexed("vol~more", "more");
        expect_fallback(
            reconstruct(
                &base,
                &[updated("docs", "more"), updated("n5", "docs")],
                None,
                &resolver,
            ),
            "placed or moved earlier in this delta",
        );
    }

    #[test]
    fn without_a_root_uid_a_parent_the_delta_did_not_touch_is_asked_of_the_index_as_before() {
        // The control: where the in-pass map and the index agree nothing changes.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/a.txt", "vol~a", "hash-a"),
            remote_file("docs/b.txt", "vol~b", "hash-b"),
        ])
        .indexed("vol~docs", "docs");
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[created("b", "docs")],
            None,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("docs/b.txt")));
        assert_eq!(resolver.listings(), [PathBuf::from("docs")]);
    }

    #[test]
    fn include_rules_leave_the_root_unknown_so_the_in_pass_map_places_nothing_either() {
        let (base, resolver) = a_folder_that_takes_a_trashed_ones_name();
        expect_fallback(
            reconstruct_remote(
                &base,
                &the_folder_is_made_outside_then_moved_in(),
                VOLUME,
                ROOT,
                &scan_options_including(&["**/a", "**/*.txt"]),
                &resolver,
            ),
            "placed or moved earlier in this delta",
        );
    }

    #[test]
    fn with_a_root_uid_the_same_history_is_not_a_wrong_map() {
        // Known root: the first event names a parent the pair does not hold, so it is deferred and
        // the folder is placed only by its last event, which is a directory appearing by an update.
        let (base, resolver) = a_folder_that_takes_a_trashed_ones_name();
        expect_fallback(
            reconstruct(
                &base,
                &the_folder_is_made_outside_then_moved_in(),
                ROOT,
                &resolver,
            ),
            "appeared by an update",
        );
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
    fn a_node_whose_event_names_no_parent_is_never_skipped() {
        // Round 2 of the review of #461: "no parent" says nothing about where the node is. A node
        // restored from the trash can arrive that way and be inside the tree, so it was read as
        // somewhere else and never downloaded. Skipping needs a parent that is not ours, not none.
        for kind in [RemoteChangeKind::Created, RemoteChangeKind::Updated] {
            let event = change(kind, "restored", None, false);
            expect_fallback(
                reconstruct(
                    &named_tree(),
                    std::slice::from_ref(&event),
                    ROOT,
                    &FakeResolver::default(),
                ),
                "names no parent",
            );
        }
    }

    #[test]
    fn a_node_with_no_parent_id_is_not_placed_from_a_guess() {
        // The harm, end to end: the file is back in `docs`, the event does not say so.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/a.txt", "vol~a", "hash-a"),
            remote_file("docs/back.txt", "vol~back", "hash-back"),
        ]);
        let event = change(RemoteChangeKind::Updated, "back", None, false);
        expect_fallback(
            reconstruct(&named_tree(), &[event], ROOT, &resolver),
            "names no parent",
        );
        assert!(
            resolver.listings().is_empty(),
            "nothing is placed from a guess: {:?}",
            resolver.listings()
        );
    }

    #[test]
    fn a_removal_names_no_parent_and_is_still_skipped_when_the_tree_is_fully_named() {
        // A removal never carries one, and needs none: it is decided by the ids alone.
        let (_, outside) = complete(reconstruct(
            &named_tree(),
            &[deleted("elsewhere")],
            ROOT,
            &FakeResolver::default(),
        ));
        assert_eq!(outside, 1);
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
    fn a_tracked_node_that_names_an_unknown_parent_forces_a_snapshot() {
        // Round 2 of the review of #461. Such an event is a move out of the tree, or a move into a
        // folder this pair has no row for (empty, nothing recorded beneath it), and nothing in the
        // delta says which. Read as "out" the node was removed from the map, and the planner saw a
        // file the user still has as deleted remotely. A walk is the only answer that is right in
        // both cases, so the pair pays one for a real move out.
        for (event, label) in [
            (updated("docs", "elsewhere-folder"), "a folder"),
            (updated("a", "elsewhere-folder"), "a file"),
        ] {
            let outcome = reconstruct(&named_tree(), &[event], ROOT, &FakeResolver::default());
            assert!(
                matches!(&outcome, Reconstruction::FallbackToSnapshot(reason)
                    if reason.contains("names a parent it does not hold")),
                "{label} moved out must walk"
            );
        }
    }

    #[test]
    fn a_tracked_file_moved_into_a_folder_with_no_row_forces_a_snapshot() {
        // The destructive form: `empty` is on the remote and in the tree, holds nothing the index
        // records and has no row. Its listing would have the file; the map without a walk would not.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_directory("empty", "vol~empty"),
            remote_file("empty/a.txt", "vol~a", "hash-a"),
        ]);
        expect_fallback(
            reconstruct(&named_tree(), &[updated("a", "empty")], ROOT, &resolver),
            "names a parent it does not hold",
        );
    }

    #[test]
    fn a_node_that_stays_in_the_tree_is_still_placed_by_its_known_parent() {
        // The control for the two above: a move between folders the pair holds is not a walk.
        let mut base = named_tree();
        base.extend(base_of(vec![directory_record("more", Some("vol~more"))]));
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_directory("more", "vol~more"),
            remote_file("more/a.txt", "vol~a", "hash-a"),
        ]);
        let (map, outside) = complete(reconstruct(&base, &[updated("a", "more")], ROOT, &resolver));
        assert_eq!(outside, 0);
        assert!(map.contains_key(Path::new("more/a.txt")));
        assert!(!map.contains_key(Path::new("docs/a.txt")));
    }

    /// A history to hand `reconstruct`: its label, the baseline, the events and the remote.
    type History = (
        &'static str,
        HashMap<PathBuf, FileRecord>,
        Vec<RemoteChange>,
        FakeResolver,
    );

    #[test]
    fn the_histories_the_old_move_out_removal_had_to_get_right_now_walk() {
        // Before round 2 a tracked node's move out was applied when its event was read, and these
        // histories pinned that order: a node that takes the path a moved-out node had, a rename
        // into that path, a move out and back. Nothing is removed on that reading any more, so the
        // class cannot be wrong; each of them is a snapshot.
        let folder_and_child = || {
            base_of(vec![
                directory_record("D", Some("vol~d-node")),
                file_record("D/g", "hash-g", Some("vol~g")),
            ])
        };
        let cases: Vec<History> = vec![
            (
                "another folder takes the name",
                folder_and_child(),
                vec![updated("d-node", "elsewhere"), created("e-node", "root")],
                FakeResolver::with_tree(vec![remote_directory("D", "vol~e-node")]),
            ),
            (
                "a rename into the name",
                base_of(vec![
                    directory_record("D", Some("vol~d-node")),
                    file_record("f.txt", "hash-f", Some("vol~f")),
                ]),
                vec![updated("d-node", "elsewhere"), updated("f", "root")],
                FakeResolver::with_tree(vec![remote_file("D", "vol~f", "hash-f")]),
            ),
            (
                "a file out and back",
                named_tree(),
                vec![updated("a", "elsewhere-folder"), updated("a", "docs")],
                FakeResolver::with_tree(vec![
                    remote_directory("docs", "vol~docs"),
                    remote_file("docs/a.txt", "vol~a", "hash-a"),
                ]),
            ),
            (
                "a folder out and back",
                named_tree(),
                vec![updated("docs", "elsewhere-folder"), updated("docs", "root")],
                FakeResolver::with_tree(vec![
                    remote_directory("docs", "vol~docs"),
                    remote_file("docs/a.txt", "vol~a", "hash-a"),
                ]),
            ),
        ];
        for (label, base, events, resolver) in cases {
            assert!(
                matches!(
                    reconstruct(&base, &events, ROOT, &resolver),
                    Reconstruction::FallbackToSnapshot(_)
                ),
                "{label}: expected a snapshot"
            );
        }
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

    // --- the review of #461: what "outside" may not assume ---------------------------------------

    /// With an include rule `docs` is not a sync entity: `docs/x.md` is synced and `docs` has no row.
    fn a_file_synced_under_a_folder_with_no_row() -> HashMap<PathBuf, FileRecord> {
        base_of(vec![file_record("docs/x.md", "hash-x", Some("vol~x"))])
    }

    fn docs_on_the_remote() -> FakeResolver {
        FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/x.md", "vol~x", "hash-x2"),
            remote_file("docs/b.md", "vol~b", "hash-b"),
        ])
    }

    #[test]
    fn a_file_created_in_a_folder_the_include_rules_give_no_row_is_not_read_as_outside() {
        let options = scan_options_including(&["**/*.md"]);
        assert!(
            !options.allows_relative_directory(Path::new("docs")),
            "precondition: the include rule makes docs a non-entity"
        );
        expect_fallback(
            reconstruct_remote(
                &a_file_synced_under_a_folder_with_no_row(),
                &[created("b", "docs")],
                VOLUME,
                ROOT,
                &options,
                &docs_on_the_remote(),
            ),
            "not under any indexed parent or the remote root",
        );
    }

    #[test]
    fn an_edit_of_a_synced_file_in_a_folder_the_include_rules_give_no_row_is_not_a_move_out() {
        expect_fallback(
            reconstruct_remote(
                &a_file_synced_under_a_folder_with_no_row(),
                &[updated("x", "docs")],
                VOLUME,
                ROOT,
                &scan_options_including(&["**/*.md"]),
                &docs_on_the_remote(),
            ),
            "not under any indexed parent or the remote root",
        );
    }

    #[test]
    fn include_rules_make_the_root_uid_unknown_so_nothing_is_skipped() {
        // Even a pair whose every record is named: the folders in between are not rows, so no
        // statement about "the whole tree" can be made.
        let resolver = FakeResolver::default();
        expect_fallback(
            reconstruct_remote(
                &base_of(vec![file_record("top.md", "hash", Some("vol~top"))]),
                &[created("foreign", "elsewhere")],
                VOLUME,
                ROOT,
                &scan_options_including(&["**/*.md"]),
                &resolver,
            ),
            "not under any indexed parent or the remote root",
        );
        assert_eq!(
            resolver.listings(),
            [PathBuf::new()],
            "the root was listed, as before #456"
        );
    }

    #[test]
    fn include_rules_still_place_a_direct_child_of_the_root_by_listing_the_root() {
        let resolver = FakeResolver::with_tree(vec![remote_file("top.md", "vol~top", "hash-2")]);
        let (map, outside) = complete(reconstruct_remote(
            &base_of(vec![file_record("top.md", "hash", Some("vol~top"))]),
            &[updated("top", "root")],
            VOLUME,
            ROOT,
            &scan_options_including(&["**/*.md"]),
            &resolver,
        ));
        assert_eq!(outside, 0);
        assert_eq!(
            map[Path::new("top.md")].as_file().unwrap().sha1_hash,
            Some("hash-2".to_owned())
        );
    }

    #[test]
    fn a_record_in_a_folder_the_index_does_not_hold_stops_the_skip() {
        // The same shape without any rule: an index with a file row and no folder row (written
        // before folders were rows). The unknown parent may be that folder.
        expect_fallback(
            reconstruct(
                &a_file_synced_under_a_folder_with_no_row(),
                &[created("b", "docs")],
                ROOT,
                &docs_on_the_remote(),
            ),
            "docs/x.md",
        );
    }

    #[test]
    fn a_record_two_levels_below_a_missing_folder_row_stops_the_skip() {
        let base = base_of(vec![
            directory_record("a", Some("vol~a")),
            file_record("a/b/c.txt", "hash-c", Some("vol~c")),
        ]);
        expect_fallback(
            reconstruct(
                &base,
                &[created("foreign", "elsewhere")],
                ROOT,
                &FakeResolver::default(),
            ),
            "a/b/c.txt",
        );
    }

    #[test]
    fn an_edit_of_a_record_in_a_folder_the_index_does_not_hold_is_not_a_move_out() {
        expect_fallback(
            reconstruct(
                &a_file_synced_under_a_folder_with_no_row(),
                &[updated("x", "docs")],
                ROOT,
                &docs_on_the_remote(),
            ),
            "names a parent it does not hold",
        );
    }

    #[test]
    fn a_record_whose_folder_row_exists_does_not_stop_the_skip() {
        // The control for the three above: the same file with its folder row.
        let (map, outside) = complete(reconstruct(
            &named_tree(),
            &[created("foreign", "elsewhere")],
            ROOT,
            &FakeResolver::default(),
        ));
        assert_eq!((map.len(), outside), (2, 1));
    }

    #[test]
    fn a_record_removed_by_the_delta_still_stops_the_skip_in_its_folder() {
        // The evidence that the tree has a folder the pair cannot name is a record beneath it, and
        // the delta can remove that record: `docs/x.md` is trashed, and a create names `docs`.
        // Read from the final map alone the tree looks fully held and the create is "elsewhere".
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_file("docs/b.md", "vol~b", "hash-b"),
        ]);
        expect_fallback(
            reconstruct(
                &a_file_synced_under_a_folder_with_no_row(),
                &[trashed("x", "docs"), created("b", "docs")],
                ROOT,
                &resolver,
            ),
            "docs/x.md is in a folder the index has no record of",
        );
    }

    #[test]
    fn a_foreign_removal_walks_when_a_record_sits_in_a_folder_the_index_does_not_hold() {
        // The removal of a node the pair does not hold may be the folder that has no row, with the
        // pair's records beneath it. Only a pair that knows its root can say the rest of the tree is
        // named, so it is the one that reads this evidence.
        expect_fallback(
            reconstruct(
                &a_file_synced_under_a_folder_with_no_row(),
                &[deleted("foreign")],
                ROOT,
                &FakeResolver::default(),
            ),
            "docs/x.md is in a folder the index has no record of",
        );
    }

    #[test]
    fn a_foreign_removal_is_still_dropped_by_a_pair_that_does_not_know_its_root() {
        // The pre-#456 rule, kept whole: every record carries an id and none is this node.
        let (map, outside) = complete(reconstruct(
            &a_file_synced_under_a_folder_with_no_row(),
            &[deleted("foreign")],
            None,
            &FakeResolver::default(),
        ));
        assert_eq!((map.len(), outside), (1, 1));
        let (_, outside) = complete(reconstruct_remote(
            &a_file_synced_under_a_folder_with_no_row(),
            &[deleted("foreign")],
            VOLUME,
            ROOT,
            &scan_options_including(&["**/*.md"]),
            &FakeResolver::default(),
        ));
        assert_eq!(outside, 1, "an include rule makes the root unknown");
    }

    /// Review of #461, round 3: `b` has no row, `b/a.txt` is a record beneath it, and the daemon's
    /// own upload `c.txt` has no id until its `Created` event, which is in the delta. That event
    /// names the one record without an id, so the removal of `b` read as "every record is named, so
    /// this was never synced here" and `b/a.txt` stayed in the map.
    fn a_folder_with_no_row_and_the_daemons_own_upload() -> (
        HashMap<PathBuf, FileRecord>,
        FakeResolver,
        Vec<RemoteChange>,
    ) {
        (
            base_of(vec![
                file_record("b/a.txt", "hash-a", Some("vol~a")),
                file_record("c.txt", "hash-c", None),
            ]),
            FakeResolver::with_tree(vec![remote_file("c.txt", "vol~c", "hash-c")]),
            vec![created("c", "root"), trashed("b", "root")],
        )
    }

    #[test]
    fn a_trashed_folder_with_no_row_walks_even_when_its_own_event_names_the_last_unnamed_record() {
        let (base, resolver, events) = a_folder_with_no_row_and_the_daemons_own_upload();
        expect_fallback(
            reconstruct(&base, &events, ROOT, &resolver),
            "b/a.txt is in a folder the index has no record of",
        );
    }

    #[test]
    fn a_pair_that_does_not_know_its_root_decides_a_removal_as_before_456() {
        // The same delta, read by a pair without a root uid: a record without an id walks, as it
        // always did, whatever a later event of the delta names. (Before this was pinned, such a
        // pair dropped the trash of `b` here because `c.txt`'s own `Created` event is in the delta,
        // a walk the old code made.)
        let (base, resolver, events) = a_folder_with_no_row_and_the_daemons_own_upload();
        expect_fallback(
            reconstruct(&base, &events, None, &resolver),
            "removal of untracked node b while c.txt has no composed id",
        );
    }

    #[test]
    fn a_trashed_folder_with_a_row_is_removed_with_its_records_as_before() {
        // The control: the same delta with the folder's row.
        let base = base_of(vec![
            directory_record("b", Some("vol~b")),
            file_record("b/a.txt", "hash-a", Some("vol~a")),
            file_record("c.txt", "hash-c", None),
        ]);
        let resolver = FakeResolver::with_tree(vec![remote_file("c.txt", "vol~c", "hash-c")]);
        let (map, outside) = complete(reconstruct(
            &base,
            &[created("c", "root"), trashed("b", "root")],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 0);
        let mut paths: Vec<_> = map.keys().cloned().collect();
        paths.sort();
        assert_eq!(paths, [PathBuf::from("c.txt")]);
    }

    #[test]
    fn a_folder_the_delta_renamed_leaves_its_children_unheld_and_a_foreign_event_walks() {
        // The map half of the "record outside any folder" check: `docs` is renamed `e`, which
        // moves its own entry and leaves `docs/a.txt` in the map under a folder that is no longer
        // there (events are per link; the rename of a folder with descendants is a limit older than
        // #456). The baseline still holds `docs`, so only the map shows it.
        let resolver = FakeResolver::with_tree(vec![remote_directory("e", "vol~docs")]);
        expect_fallback(
            reconstruct(
                &named_tree(),
                &[updated("docs", "root"), created("foreign", "elsewhere")],
                ROOT,
                &resolver,
            ),
            "docs/a.txt is in a folder the index has no record of",
        );
    }

    // --- round 2 of the review of #461 ---------------------------------------------------------

    #[test]
    fn a_parent_trashed_earlier_in_the_delta_is_not_resolved_through_its_stale_index_path() {
        // `c` is trashed and a different folder takes the name; then an event names the trashed `c`
        // as its parent. The index still says `c` is at `c`, which is now somebody else's folder,
        // so the listing there does not hold `b`, and "an updated node absent from its parent" was
        // read as a real move and dropped only `b` itself. `b` is under a trashed folder: the pair
        // cannot say from the delta where it is, so it walks. (The seed that found this,
        // 207852601, reached it through a tracked move out, which now walks earlier; this is the
        // same stale lookup through a removal.)
        let base = base_of(vec![
            directory_record("b", Some("vol~b")),
            file_record("b/f.txt", "hash-f", Some("vol~f")),
            directory_record("c", Some("vol~c")),
        ]);
        let resolver = FakeResolver::with_tree(vec![remote_directory("c", "vol~c2")])
            .indexed("vol~b", "b")
            .indexed("vol~c", "c");
        expect_fallback(
            reconstruct(
                &base,
                &[
                    trashed("c", "root"),
                    created("c2", "root"),
                    updated("b", "c"),
                ],
                ROOT,
                &resolver,
            ),
            "taken out earlier in this delta",
        );
    }

    #[test]
    fn a_parent_that_a_removal_took_with_it_is_not_resolved_through_the_index_either() {
        // `docs` goes with its children when it is trashed: a child's later event names a parent
        // that is no longer in the tree.
        let resolver = FakeResolver::with_tree(vec![remote_directory("docs", "vol~docs2")])
            .indexed("vol~docs", "docs")
            .indexed("vol~sub", "docs/sub");
        let mut base = named_tree();
        base.extend(base_of(vec![directory_record("docs/sub", Some("vol~sub"))]));
        expect_fallback(
            reconstruct(
                &base,
                &[trashed("docs", "root"), updated("x", "sub")],
                ROOT,
                &resolver,
            ),
            "taken out earlier in this delta",
        );
    }

    #[test]
    fn a_parent_moved_into_an_excluded_folder_is_not_resolved_through_the_index_either() {
        // `docs` goes into `secret`, which an exclude rule hides: the node leaves the map like
        // any other. An event that names it as a parent afterwards (a file inside it, edited) would
        // be looked up by the index's old path for `docs`, which is a root-level folder that is not
        // there any more; the pair walks instead of reading the miss.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("secret", "vol~secret"),
            remote_directory("secret/docs", "vol~docs"),
            remote_file("secret/docs/a.txt", "vol~a", "hash-2"),
        ])
        .indexed("vol~secret", "secret")
        .indexed("vol~docs", "docs");
        let outcome = reconstruct_remote(
            &named_tree(),
            &[updated("docs", "secret"), updated("a", "docs")],
            VOLUME,
            ROOT,
            &scan_options(&["secret/**"]),
            &resolver,
        );
        expect_fallback(outcome, "taken out earlier in this delta");
    }

    #[test]
    fn a_directory_absent_from_its_parent_takes_its_subtree_with_it() {
        // A lone `Updated` for `docs` whose listing no longer holds it (it moved after the delta
        // ends): dropping only the folder left `docs/a.txt` in the map as a file of a folder that
        // is not there.
        let mut base = named_tree();
        base.extend(base_of(vec![file_record(
            "keep.txt",
            "hash-k",
            Some("vol~keep"),
        )]));
        let resolver = FakeResolver::with_tree(vec![remote_file("keep.txt", "vol~keep", "hash-k")]);
        let (map, _) = complete(reconstruct(
            &base,
            &[updated("docs", "root")],
            ROOT,
            &resolver,
        ));
        let mut paths: Vec<_> = map.keys().cloned().collect();
        paths.sort();
        assert_eq!(paths, [PathBuf::from("keep.txt")]);
    }

    #[test]
    fn the_counterexample_of_seed_207852601_walks() {
        // Found by the review's fuzz (no rules, 12 operations, nodes that retake a name, direct
        // deletes), n1 being the pair's root: the folder `c` (n11) moves out, another folder `c`
        // (n13) is made, `b` (n7, with the child `b/a`) moves under the old one, and a folder `b`
        // (n14) is made in its place. The old code answered Complete with `b/a` still in the map
        // under the new `b`: the move of `b` was looked up through `c`'s stale path in the index,
        // found nothing there, and dropped `b` alone.
        let base = base_of(vec![
            directory_record("a", Some("vol~n8")),
            directory_record("b", Some("vol~n7")),
            directory_record("b/a", Some("vol~n12")),
            directory_record("c", Some("vol~n11")),
        ]);
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("a", "vol~n8"),
            remote_directory("b", "vol~n14"),
            remote_directory("c", "vol~n13"),
        ])
        .indexed("vol~n8", "a")
        .indexed("vol~n7", "b")
        .indexed("vol~n12", "b/a")
        .indexed("vol~n11", "c");
        expect_fallback(
            reconstruct_remote(
                &base,
                &[
                    trashed("n6", "n3"),
                    updated("n11", "n4"),
                    deleted("n6"),
                    created("n13", "n1"),
                    updated("n7", "n11"),
                    created("n14", "n1"),
                    created("n15", "n0"),
                ],
                VOLUME,
                Some("vol~n1"),
                &scan_options(&[]),
                &resolver,
            ),
            "names a parent it does not hold",
        );
    }

    #[test]
    fn a_file_record_with_a_legacy_or_foreign_id_that_moved_out_is_not_left_behind() {
        // Its id never matches an event of this volume, so the pair holds it under no uid and an
        // `Updated` for it looks like news about somebody else's node. Dropped as outside, the
        // record stayed in the map for ever.
        for id in ["raw-legacy-id", "other~old"] {
            let base = base_of(vec![
                directory_record("docs", Some("vol~docs")),
                file_record("docs/old.txt", "hash", Some(id)),
            ]);
            expect_fallback(
                reconstruct(
                    &base,
                    &[updated("old", "elsewhere-folder")],
                    ROOT,
                    &FakeResolver::default(),
                ),
                "docs/old.txt has no composed id",
            );
        }
    }

    #[test]
    fn a_node_an_absent_update_took_out_and_that_then_names_an_unknown_parent_walks() {
        // Found by the model test. The first event is read against the end-state listing, which no
        // longer has the file in `docs` (it moved after), so it is taken out of the map. Its second
        // event names `empty`, a folder with no row: the node is still ours, and reading the
        // event as news about someone else's node lost `empty/a.txt`.
        let resolver = FakeResolver::with_tree(vec![
            remote_directory("docs", "vol~docs"),
            remote_directory("empty", "vol~empty"),
            remote_file("empty/a.txt", "vol~a", "hash-a"),
        ]);
        expect_fallback(
            reconstruct(
                &named_tree(),
                &[updated("a", "docs"), updated("a", "empty")],
                ROOT,
                &resolver,
            ),
            "names a parent it does not hold",
        );
    }

    #[test]
    fn a_legacy_record_whose_path_another_node_takes_still_stops_the_skip() {
        // Found by the model test: the folder `a` is held under a raw id, it is trashed, and a
        // file takes the name in the same delta. The listing overwrites the record, so the final
        // map has no record without an id and the trash of `a` looked like news about someone
        // else's node, leaving `a/b` behind. The baseline still has the record.
        let base = base_of(vec![
            directory_record("a", Some("n4")),
            file_record("a/b", "hash-b", Some("vol~b")),
        ]);
        let resolver = FakeResolver::with_tree(vec![remote_file("a", "vol~n6", "hash")]);
        expect_fallback(
            reconstruct(
                &base,
                &[trashed("n4", "root"), created("n6", "root")],
                ROOT,
                &resolver,
            ),
            "a has no composed id",
        );
    }

    #[test]
    fn a_file_record_in_the_upload_window_does_not_stop_the_skip() {
        // `None` and `""` are "not named yet": the daemon uploaded the file last pass and its own
        // `Created` event is still to come (or is in this delta, below). A walk for every foreign
        // event until it arrives would be a cost nothing needs.
        for id in [None, Some("")] {
            let base = base_of(vec![file_record("up.txt", "hash", id)]);
            let (_, outside) = complete(reconstruct(
                &base,
                &[updated("elsewhere", "elsewhere-folder")],
                ROOT,
                &FakeResolver::default(),
            ));
            assert_eq!(outside, 1, "id {id:?}");
        }
    }

    #[test]
    fn foreign_events_around_the_daemons_own_upload_in_one_delta_are_still_skipped() {
        let base = base_of(vec![file_record("up.txt", "hash", None)]);
        let resolver = FakeResolver::with_tree(vec![remote_file("up.txt", "vol~up", "hash")]);
        let (map, outside) = complete(reconstruct(
            &base,
            &[
                updated("before", "elsewhere"),
                created("up", "root"),
                updated("after", "elsewhere"),
            ],
            ROOT,
            &resolver,
        ));
        assert_eq!(outside, 2);
        assert_eq!(
            map[Path::new("up.txt")].remote_id().as_deref(),
            Some("vol~up")
        );
    }

    #[test]
    fn a_node_seen_elsewhere_and_then_moved_in_is_placed() {
        // A deferred event is decided like any other even when a later one places the node (the
        // check that skipped it did nothing but save a walk when the tree was not fully named, and
        // was removed in round 2). With the tree fully named, nothing walks.
        let resolver = FakeResolver::with_tree(vec![remote_file("in.txt", "vol~in", "hash")]);
        let (map, _) = complete(reconstruct(
            &named_tree(),
            &[updated("in", "elsewhere-folder"), updated("in", "root")],
            ROOT,
            &resolver,
        ));
        assert!(map.contains_key(Path::new("in.txt")));
    }
}
