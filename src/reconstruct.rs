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
//! See `docs/adr/0001-remote-change-detection-via-volume-events.md`.

use crate::AppResult;
use crate::events::{RemoteChange, RemoteChangeKind, node_uid};
use crate::index::{EntityKind, FileRecord, ScanOptions};
use crate::proton::{RemoteDirectory, RemoteEntity, RemoteFile};
use std::collections::HashMap;
use std::path::PathBuf;

/// Targeted resolution of a created/updated remote node to its current `(relative path, entity)`.
///
/// The concrete daemon implementation lists just the node's parent directory (an O(1) call) and
/// reads the changed node's decrypted name + digest from it; tests inject a fake.
pub trait RemoteChangeResolver {
    /// Resolve a created/updated node.
    ///
    /// * `Ok(Some((path, entity)))` — the node's current location and metadata.
    /// * `Ok(None)` — the node is not present in its parent listing. For an `Updated` node (already
    ///   tracked) that is a real move/trash and the reconstruction drops its stale location; for a
    ///   `Created` node it means the listing lags the event stream, and the reconstruction falls
    ///   back to a full snapshot rather than lose the create (#30).
    /// * `Err(_)` — resolution failed hard (parent unknown, listing failed); the caller falls back
    ///   to a full snapshot rather than plan against an incomplete map.
    fn resolve(&self, change: &RemoteChange) -> AppResult<Option<(PathBuf, RemoteEntity)>>;
}

/// Outcome of reconstructing the remote map from a delta.
pub enum Reconstruction {
    /// A complete remote entity map (`base ⊕ delta`), safe to hand to the planner.
    Complete(HashMap<PathBuf, RemoteEntity>),
    /// A change could not be incorporated without a full re-walk; the caller must snapshot. The
    /// string is a human-readable reason for logging.
    FallbackToSnapshot(String),
}

/// Reconstructs the current remote entity map as `base_index ⊕ changes`.
///
/// `base_index` must be the **selective-sync-filtered** baseline (the same map the planner is
/// given), so excluded records are neither present as "remote" nor purged. `volume_id` bridges an
/// event's raw `LinkID` into the composed `proton_id` space via [`node_uid`]. `scan_options`
/// re-applies the include/exclude filter to newly-resolved nodes exactly as it applies to a full
/// listing.
pub fn reconstruct_remote(
    base_index: &HashMap<PathBuf, FileRecord>,
    changes: &[RemoteChange],
    volume_id: &str,
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

    for change in changes {
        let uid = node_uid(volume_id, &change.node_id);
        // A hard delete OR a trashing (Updated + trashed) is a remote removal. Trashing arriving
        // as Updated (not Deleted) is the key event-stream subtlety; both mean "gone" here.
        let is_removal = matches!(change.kind, RemoteChangeKind::Deleted)
            || (matches!(change.kind, RemoteChangeKind::Updated) && change.trashed);

        if is_removal {
            if let Some(path) = uid_to_path.remove(&uid) {
                remote.remove(&path);
                // Volume events are per-link: trashing/deleting a folder flips only the
                // folder's own link, with no events for its descendants. Cascade the removal,
                // or the map would claim the children still exist remotely and the planner
                // would *recreate* the folder the user just deleted. Sound under in-order
                // delivery: a child moved out beforehand was re-homed by its own earlier
                // event, so it no longer sits under `path`.
                remote.retain(|key, _| !crate::sync::is_strict_descendant(&path, key));
                uid_to_path.retain(|_, value| !crate::sync::is_strict_descendant(&path, value));
            } else if base_index.values().any(|record| {
                record
                    .proton_id
                    .as_deref()
                    .is_none_or(|id| !id.contains('~'))
            }) {
                // Unresolvable removal while some baseline record has no composed id (the
                // just-uploaded window, or a legacy raw id): the removed node may BE one of
                // those records, and a full snapshot would notice its absence where this
                // reconstruction cannot. Skipping would advance the cursor past the event
                // forever.
                return Reconstruction::FallbackToSnapshot(format!(
                    "removal of untracked node {} while baseline records lack composed ids",
                    change.node_id
                ));
            }
            // Otherwise: every baseline record is id-tracked and none matched, so the node
            // was never synced here — nothing to remove; a full snapshot would not track it
            // either. Safe skip.
            continue;
        }

        // Created, or Updated (not trashed): resolve the node's current path + metadata.
        match resolver.resolve(change) {
            Ok(Some((path, entity))) => {
                // A rename/move leaves the node at a new path; drop the stale location first.
                if let Some(old_path) = uid_to_path.get(&uid).cloned()
                    && old_path != path
                {
                    remote.remove(&old_path);
                }
                let allowed = match &entity {
                    RemoteEntity::File(_) => scan_options.allows_relative_file(&path),
                    RemoteEntity::Directory(_) => scan_options.allows_relative_directory(&path),
                };
                if allowed {
                    uid_to_path.insert(uid, path.clone());
                    remote.insert(path, entity);
                } else {
                    // Excluded by selective sync: neither present as remote nor tracked, so it is
                    // never planned or purged.
                    uid_to_path.remove(&uid);
                }
            }
            Ok(None) => {
                // A *created* node missing from its parent listing is almost always the CLI
                // listing lagging the event stream (observed live: seconds behind), not a node
                // that vanished. Dropping it would advance the cursor past a create nothing
                // re-derives — the periodic full-tree resync is off by default and a restart
                // warm-starts from the cursor — so re-anchor with a full walk instead. This is
                // symmetric with the resolver's root-listing branch, which already errs here.
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
            }
            Err(error) => {
                return Reconstruction::FallbackToSnapshot(format!(
                    "could not resolve remote change for node {}: {error}",
                    change.node_id
                ));
            }
        }
    }

    Reconstruction::Complete(remote)
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
    use std::collections::HashSet;
    use std::path::Path;

    const VOLUME: &str = "vol";

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

    fn change(kind: RemoteChangeKind, node_id: &str, trashed: bool) -> RemoteChange {
        RemoteChange {
            kind,
            node_id: node_id.to_owned(),
            parent_id: Some("root".to_owned()),
            trashed,
            shared: false,
            event_id: format!("evt-{node_id}"),
        }
    }

    #[derive(Default)]
    struct FakeResolver {
        resolved: HashMap<String, (PathBuf, RemoteEntity)>,
        absent: HashSet<String>,
        hard_fail: HashSet<String>,
    }

    impl RemoteChangeResolver for FakeResolver {
        fn resolve(&self, change: &RemoteChange) -> AppResult<Option<(PathBuf, RemoteEntity)>> {
            if self.hard_fail.contains(&change.node_id) {
                return Err(boxed_error("targeted listing failed"));
            }
            if self.absent.contains(&change.node_id) {
                return Ok(None);
            }
            Ok(self.resolved.get(&change.node_id).cloned())
        }
    }

    fn complete(reconstruction: Reconstruction) -> HashMap<PathBuf, RemoteEntity> {
        match reconstruction {
            Reconstruction::Complete(map) => map,
            Reconstruction::FallbackToSnapshot(reason) => {
                panic!("expected a complete reconstruction, got fallback: {reason}")
            }
        }
    }

    #[test]
    fn unchanged_base_is_reproduced_verbatim_with_no_changes() {
        let base = HashMap::from([(
            PathBuf::from("dir/a.txt"),
            file_record("dir/a.txt", "hash-a", Some("vol~node-a")),
        )]);
        let map = complete(reconstruct_remote(
            &base,
            &[],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        ));
        assert_eq!(map.len(), 1);
        assert_eq!(
            map[Path::new("dir/a.txt")].as_file().unwrap().sha1_hash,
            Some("hash-a".to_owned())
        );
    }

    #[test]
    fn a_created_node_is_added_via_the_resolver() {
        let base = HashMap::new();
        let resolver = FakeResolver {
            resolved: HashMap::from([(
                "node-new".to_owned(),
                (
                    PathBuf::from("dir/new.txt"),
                    remote_file("dir/new.txt", "vol~node-new", "hash-new"),
                ),
            )]),
            ..Default::default()
        };
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Created, "node-new", false)],
            VOLUME,
            &scan_options(&[]),
            &resolver,
        ));
        assert_eq!(
            map[Path::new("dir/new.txt")].as_file().unwrap().sha1_hash,
            Some("hash-new".to_owned())
        );
    }

    #[test]
    fn an_updated_node_gets_a_fresh_digest() {
        let base = HashMap::from([(
            PathBuf::from("a.txt"),
            file_record("a.txt", "old", Some("vol~node-a")),
        )]);
        let resolver = FakeResolver {
            resolved: HashMap::from([(
                "node-a".to_owned(),
                (
                    PathBuf::from("a.txt"),
                    remote_file("a.txt", "vol~node-a", "new"),
                ),
            )]),
            ..Default::default()
        };
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Updated, "node-a", false)],
            VOLUME,
            &scan_options(&[]),
            &resolver,
        ));
        assert_eq!(
            map[Path::new("a.txt")].as_file().unwrap().sha1_hash,
            Some("new".to_owned())
        );
    }

    #[test]
    fn a_deleted_node_is_removed_from_the_map() {
        let base = HashMap::from([(
            PathBuf::from("a.txt"),
            file_record("a.txt", "hash", Some("vol~node-a")),
        )]);
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Deleted, "node-a", false)],
            VOLUME,
            &scan_options(&[]),
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
        let base = HashMap::from([(
            PathBuf::from("a.txt"),
            file_record("a.txt", "hash", Some("vol~node-a")),
        )]);
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Updated, "node-a", true)],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        ));
        assert!(map.is_empty(), "a trashed node is a removal");
    }

    #[test]
    fn a_renamed_node_drops_its_stale_path() {
        let base = HashMap::from([(
            PathBuf::from("old.txt"),
            file_record("old.txt", "hash", Some("vol~node-a")),
        )]);
        let resolver = FakeResolver {
            resolved: HashMap::from([(
                "node-a".to_owned(),
                (
                    PathBuf::from("new.txt"),
                    remote_file("new.txt", "vol~node-a", "hash"),
                ),
            )]),
            ..Default::default()
        };
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Updated, "node-a", false)],
            VOLUME,
            &scan_options(&[]),
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
            hard_fail: HashSet::from(["node-x".to_owned()]),
            ..Default::default()
        };
        let outcome = reconstruct_remote(
            &HashMap::new(),
            &[change(RemoteChangeKind::Created, "node-x", false)],
            VOLUME,
            &scan_options(&[]),
            &resolver,
        );
        match outcome {
            Reconstruction::FallbackToSnapshot(reason) => {
                assert!(
                    reason.contains("node-x"),
                    "reason should name the node: {reason}"
                )
            }
            Reconstruction::Complete(_) => panic!("an unresolvable change must force a snapshot"),
        }
    }

    #[test]
    fn an_excluded_created_node_is_never_added() {
        let resolver = FakeResolver {
            resolved: HashMap::from([(
                "node-secret".to_owned(),
                (
                    PathBuf::from("secret/keys.txt"),
                    remote_file("secret/keys.txt", "vol~node-secret", "hash"),
                ),
            )]),
            ..Default::default()
        };
        let map = complete(reconstruct_remote(
            &HashMap::new(),
            &[change(RemoteChangeKind::Created, "node-secret", false)],
            VOLUME,
            &scan_options(&["secret/**"]),
            &resolver,
        ));
        assert!(
            map.is_empty(),
            "an excluded path in the delta must never be planned or tracked"
        );
    }

    #[test]
    fn a_delete_for_an_untracked_node_is_a_safe_skip() {
        let base = HashMap::from([(
            PathBuf::from("keep.txt"),
            file_record("keep.txt", "hash", Some("vol~node-keep")),
        )]);
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Deleted, "unknown-node", false)],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        ));
        assert_eq!(
            map.len(),
            1,
            "an untracked delete must not fall back or disturb the map"
        );
        assert!(map.contains_key(Path::new("keep.txt")));
    }

    #[test]
    fn an_updated_node_absent_from_its_parent_drops_its_stale_location() {
        // Updated-only: the node was already tracked, so absence is a real move/trash. (A
        // *Created* absence is the lagging-listing case and forces a snapshot instead — see
        // `a_created_node_absent_from_its_parent_forces_a_snapshot`.)
        let base = HashMap::from([(
            PathBuf::from("a.txt"),
            file_record("a.txt", "hash", Some("vol~node-a")),
        )]);
        let resolver = FakeResolver {
            absent: HashSet::from(["node-a".to_owned()]),
            ..Default::default()
        };
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Updated, "node-a", false)],
            VOLUME,
            &scan_options(&[]),
            &resolver,
        ));
        assert!(map.is_empty(), "a node no longer in its parent is dropped");
    }

    #[test]
    fn a_created_node_absent_from_its_parent_forces_a_snapshot() {
        // #30: a create absent from its parent listing is the CLI listing lagging the event
        // stream, not a vanished node. Dropping it would advance the cursor past a create that
        // nothing re-derives (the periodic resync is off by default, and a restart warm-starts).
        let resolver = FakeResolver {
            absent: HashSet::from(["node-new".to_owned()]),
            ..Default::default()
        };
        let outcome = reconstruct_remote(
            &HashMap::new(),
            &[change(RemoteChangeKind::Created, "node-new", false)],
            VOLUME,
            &scan_options(&[]),
            &resolver,
        );
        expect_fallback(outcome, "node-new");
    }

    fn directory_record(path: &str, proton_id: Option<&str>) -> FileRecord {
        FileRecord {
            entity_kind: EntityKind::Directory,
            sha1_hash: None,
            ..file_record(path, "unused", proton_id)
        }
    }

    fn expect_fallback(outcome: Reconstruction, fragment: &str) {
        match outcome {
            Reconstruction::FallbackToSnapshot(reason) => assert!(
                reason.contains(fragment),
                "fallback reason should mention {fragment:?}: {reason}"
            ),
            Reconstruction::Complete(_) => {
                panic!("expected a snapshot fallback mentioning {fragment:?}")
            }
        }
    }

    #[test]
    fn a_directory_removal_cascades_to_its_tracked_descendants() {
        // Volume events are per-link: a folder trash emits no events for descendants.
        // Without the cascade the map would claim `docs/a.txt` still exists remotely and
        // the planner would recreate the folder the user just deleted.
        let base = HashMap::from([
            (
                PathBuf::from("docs"),
                directory_record("docs", Some("vol~node-docs")),
            ),
            (
                PathBuf::from("docs/a.txt"),
                file_record("docs/a.txt", "hash-a", Some("vol~node-a")),
            ),
            (
                PathBuf::from("other.txt"),
                file_record("other.txt", "hash-o", Some("vol~node-o")),
            ),
        ]);
        let map = complete(reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Updated, "node-docs", true)],
            VOLUME,
            &scan_options(&[]),
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
        let base = HashMap::from([(PathBuf::from("b.txt"), file_record("b.txt", "hash-b", None))]);
        let outcome = reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Updated, "node-b", true)],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "node-b");
    }

    #[test]
    fn an_untracked_removal_falls_back_when_a_baseline_record_has_a_legacy_raw_id() {
        let base = HashMap::from([(
            PathBuf::from("old.txt"),
            file_record("old.txt", "hash", Some("raw-legacy-id")),
        )]);
        let outcome = reconstruct_remote(
            &base,
            &[change(RemoteChangeKind::Deleted, "node-z", false)],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "node-z");
    }

    #[test]
    fn duplicate_baseline_proton_ids_force_a_snapshot() {
        // Reachable transient state: a withheld LocalDelete pins the old row while a
        // Download committed a new row with the same id. Seeding order would otherwise
        // decide which path a replayed move event resolves against.
        let base = HashMap::from([
            (
                PathBuf::from("a.txt"),
                file_record("a.txt", "hash", Some("vol~node-dup")),
            ),
            (
                PathBuf::from("b.txt"),
                file_record("b.txt", "hash", Some("vol~node-dup")),
            ),
        ]);
        let outcome = reconstruct_remote(
            &base,
            &[],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        );
        expect_fallback(outcome, "vol~node-dup");
    }

    #[test]
    fn empty_placeholder_ids_do_not_alias_each_other_in_seeding() {
        // Two just-uploaded records (id not yet backfilled) must not collide on "" —
        // with no removal events in the delta this reconstruction stays complete.
        let base = HashMap::from([
            (
                PathBuf::from("x.txt"),
                file_record("x.txt", "hash-x", Some("")),
            ),
            (
                PathBuf::from("y.txt"),
                file_record("y.txt", "hash-y", Some("")),
            ),
        ]);
        let map = complete(reconstruct_remote(
            &base,
            &[],
            VOLUME,
            &scan_options(&[]),
            &FakeResolver::default(),
        ));
        assert_eq!(map.len(), 2);
    }
}
