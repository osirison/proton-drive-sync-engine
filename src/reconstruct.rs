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
    /// * `Ok(None)` — the node is not present in its parent listing (e.g. created then moved or
    ///   trashed before this pass ran); the reconstruction drops any stale location for it.
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
        if let Some(id) = &record.proton_id {
            uid_to_path.insert(id.clone(), path.clone());
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
            // Resolvable → remove it. Not tracked → nothing to remove; a full snapshot would not
            // track it either, so this is a safe skip, never a fallback.
            if let Some(path) = uid_to_path.remove(&uid) {
                remote.remove(&path);
            }
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
                // The node is not present in its parent listing — drop any stale location.
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
        ScanOptions::new(Path::new("/root"), &[], &[], &excludes).expect("scan options")
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
    fn a_node_absent_from_its_parent_drops_its_stale_location() {
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
}
