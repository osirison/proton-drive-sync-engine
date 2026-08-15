//! What a **candidate** folder holds, before anything is configured (G25, #240).
//!
//! `9a Folders` quotes a price for each side of the pair the user is about to choose — the point of
//! the step being that a wrong folder is noticed by its size, not by its name. That is four numbers,
//! and **they are not equally obtainable**:
//!
//! | side | files | bytes |
//! | --- | --- | --- |
//! | local | yes — a metadata walk | yes — the same walk's `stat` |
//! | remote | yes — a bounded listing walk | **no** |
//!
//! # Why the remote byte total is absent rather than approximate
//!
//! A remote listing exposes no usable file size. The CLI's `totalStorageSize` reads `0` on a large
//! minority of perfectly healthy files (873 of 5424 on a live account, including a 132 MB archive
//! that downloads byte-perfect), so it is a reporting quirk, not a size — which is why
//! [`proton_drive_sync_engine::proton::RemoteFile`] does not carry one at all. Summing it would
//! quote a confident number that is wrong by an arbitrary amount on the exact folder the user is
//! deciding about. [`FolderProbe::bytes`] is therefore `Option`, and the remote side always answers
//! `None` — "not obtainable", never `0`.
//!
//! # Why this is not [`crate::index_read`]'s job, or G7's
//!
//! G7 (#207) totals a *configured* pair from the index. This runs before any index exists: the
//! candidate is outside every root and has never been scanned. Nothing here reads the index.
//!
//! # Bound
//!
//! The local walk is disk-local and runs to completion. The remote walk is a network walk against a
//! path the user has not committed to, so it is capped at [`MAX_REMOTE_LISTINGS`] directory
//! listings; hitting the cap sets [`FolderProbe::truncated`] and the counts become lower bounds. A
//! truncated probe must be rendered as "at least N", never as N.

use proton_drive_sync_engine::proton::{ProtonClient, ProtonDriveClient, RemoteEntity};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Directory listings one remote probe may spend. Each is a `proton-drive` subprocess, so this
/// bounds both the wall clock and the load placed on a CLI the daemon may also be using.
pub const MAX_REMOTE_LISTINGS: usize = 64;

/// One side's answer to "how much is under here".
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct FolderProbe {
    pub files: u64,
    /// Total bytes, or `None` when the side cannot report sizes — **always** `None` for a remote
    /// probe (see the module docs). `None` means unknown and must never render as `0 bytes`.
    pub bytes: Option<u64>,
    /// The walk stopped at its bound, so `files`/`bytes` are lower bounds, not totals.
    pub truncated: bool,
    /// Directories that could not be read (permissions, races, a listing the CLI could not decode).
    /// Non-zero means the counts are incomplete for a reason other than the bound.
    pub unreadable_directories: u64,
}

/// Files and bytes under a local candidate folder.
///
/// Metadata only — `read_dir` plus one `stat` per file, no hashing — because this answers a
/// question asked while the user waits, about a folder that may never be synced.
///
/// Symlinks are counted as neither files nor directories, matching the engine (which does not sync
/// them); a directory reached twice through links is walked once, so a link loop terminates and a
/// byte total is never doubled.
pub fn probe_local(root: &Path) -> Result<FolderProbe, String> {
    if !root.is_dir() {
        return Err(format!("not a folder: {}", root.display()));
    }
    let mut probe = FolderProbe {
        bytes: Some(0),
        ..FolderProbe::default()
    };
    let mut seen = HashSet::new();
    visit_local(root, &mut probe, &mut seen);
    Ok(probe)
}

fn visit_local(directory: &Path, probe: &mut FolderProbe, seen: &mut HashSet<PathBuf>) {
    match std::fs::canonicalize(directory) {
        Ok(canonical) => {
            if !seen.insert(canonical) {
                return;
            }
        }
        Err(_) => {
            probe.unreadable_directories += 1;
            return;
        }
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        probe.unreadable_directories += 1;
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            probe.unreadable_directories += 1;
            continue;
        };
        // Does NOT follow the link, so a symlink is a symlink rather than whatever it points at.
        let Ok(file_type) = entry.file_type() else {
            probe.unreadable_directories += 1;
            continue;
        };
        if file_type.is_dir() {
            visit_local(&entry.path(), probe, seen);
        } else if file_type.is_file() {
            probe.files += 1;
            // A file that cannot be stat'd is still a file: count it, but stop claiming a total.
            // Dropping to `None` for the whole side is deliberate — a byte total silently missing
            // one file is worse than one honestly marked unknown.
            match entry.metadata() {
                Ok(metadata) => {
                    probe.bytes = probe
                        .bytes
                        .map(|total| total.saturating_add(metadata.len()));
                }
                Err(_) => probe.bytes = None,
            }
        }
    }
}

/// Files under a remote candidate folder, by a bounded breadth-first listing walk.
///
/// [`FolderProbe::bytes`] is always `None` — see the module docs. Returns as soon as
/// [`MAX_REMOTE_LISTINGS`] listings have been spent, with `truncated` set.
///
/// `remote_root` is the path the walk starts from; every listing is resolved relative to it, which
/// is what lets one probe cover a subtree without re-rooting the client.
pub fn probe_remote(client: &dyn ProtonClient, remote_root: &Path) -> Result<FolderProbe, String> {
    let mut probe = FolderProbe::default();
    let mut queue = VecDeque::from([PathBuf::new()]);
    let mut listings = 0usize;
    // Seeded with the root, which is NOT a formality: a listing includes a wrapper node for the
    // directory it lists, so the root echoes itself back. An empty `seen` accepts that echo, queues
    // the root a second time, and counts every root-level file twice — a wrong number on the exact
    // screen the count is for. The queue guard alone stops the loop, not the double count.
    let mut seen: HashSet<PathBuf> = HashSet::from([PathBuf::new()]);

    while let Some(directory) = queue.pop_front() {
        if listings >= MAX_REMOTE_LISTINGS {
            probe.truncated = true;
            break;
        }
        listings += 1;
        let entries = match client.list_directory(remote_root, &directory) {
            Ok(entries) => entries,
            Err(error) => {
                // The first listing failing means the candidate itself is unreachable — that is an
                // error about the folder the user picked, not a partial count to render.
                if listings == 1 {
                    return Err(error.to_string());
                }
                probe.unreadable_directories += 1;
                continue;
            }
        };
        for (path, entity) in entries {
            match entity {
                RemoteEntity::File(_) => probe.files += 1,
                // `seen` guards against a listing that echoes its own directory back (the root
                // wrapper node), which would otherwise queue it forever.
                RemoteEntity::Directory(_) => {
                    if seen.insert(path.clone()) {
                        queue.push_back(path);
                    }
                }
            }
        }
    }
    if !queue.is_empty() {
        probe.truncated = true;
    }
    Ok(probe)
}

/// [`probe_remote`] against the real `proton-drive` CLI.
///
/// Here rather than in the Tauri layer to keep the facade rule intact: the command layer depends on
/// this crate and never on the engine, so the client type stays on this side of the boundary.
pub fn probe_remote_via_cli(proton_cli: &str, remote_path: &Path) -> Result<FolderProbe, String> {
    probe_remote(&ProtonDriveClient::new(proton_cli), remote_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proton_drive_sync_engine::proton::{RemoteDirectory, RemoteFile};
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn a_local_probe_counts_files_and_bytes_and_ignores_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("a.txt"), b"hello").expect("write");
        fs::create_dir(dir.path().join("sub")).expect("mkdir");
        fs::write(dir.path().join("sub/b.txt"), b"world!").expect("write");

        let probe = probe_local(dir.path()).expect("probe");
        assert_eq!(probe.files, 2, "directories are not files");
        assert_eq!(probe.bytes, Some(11));
        assert!(!probe.truncated);
    }

    #[test]
    fn a_local_probe_is_empty_but_not_unknown_for_an_empty_folder() {
        // The zero/unknown distinction this module exists to keep: an empty folder really is 0
        // bytes, and must not answer `None` the way the remote side does.
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe_local(dir.path()).expect("probe");
        assert_eq!(probe.files, 0);
        assert_eq!(probe.bytes, Some(0));
    }

    #[test]
    fn a_local_probe_rejects_a_path_that_is_not_a_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.txt");
        fs::write(&file, b"x").expect("write");
        assert!(probe_local(&file).is_err());
        assert!(probe_local(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn a_local_probe_walks_a_symlink_loop_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("sub")).expect("mkdir");
        fs::write(dir.path().join("sub/a.txt"), b"12345").expect("write");
        std::os::unix::fs::symlink(dir.path(), dir.path().join("sub/loop")).expect("symlink");

        let probe = probe_local(dir.path()).expect("probe");
        assert_eq!(probe.files, 1, "the loop must not multiply the count");
        assert_eq!(probe.bytes, Some(5));
    }

    /// Answers one canned listing per relative directory.
    struct FakeClient {
        listings: HashMap<PathBuf, HashMap<PathBuf, RemoteEntity>>,
    }

    impl ProtonClient for FakeClient {
        fn list(
            &self,
            _root: &Path,
        ) -> Result<HashMap<PathBuf, RemoteFile>, Box<dyn std::error::Error + Send + Sync>>
        {
            unimplemented!("a probe never takes the recursive walk")
        }

        fn list_directory(
            &self,
            _root: &Path,
            relative: &Path,
        ) -> Result<HashMap<PathBuf, RemoteEntity>, Box<dyn std::error::Error + Send + Sync>>
        {
            self.listings.get(relative).cloned().ok_or_else(
                || -> Box<dyn std::error::Error + Send + Sync> {
                    format!("no such directory: {}", relative.display()).into()
                },
            )
        }

        fn ensure_directory(
            &self,
            _root: &Path,
            _relative: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            unimplemented!()
        }

        fn upload(
            &self,
            _local: &Path,
            _root: &Path,
            _relative: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            unimplemented!()
        }

        fn download(
            &self,
            _remote: &Path,
            _destination: &Path,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            unimplemented!()
        }

        fn delete(&self, _remote: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            unimplemented!()
        }
    }

    fn file(name: &str) -> RemoteEntity {
        RemoteEntity::File(RemoteFile {
            path: PathBuf::from(name),
            id: name.to_owned(),
            name: name.to_owned(),
            sha1_hash: None,
            downloadable: true,
        })
    }

    fn directory(name: &str) -> RemoteEntity {
        RemoteEntity::Directory(RemoteDirectory {
            path: PathBuf::from(name),
            id: Some(name.to_owned()),
            name: name.to_owned(),
        })
    }

    #[test]
    fn a_remote_probe_counts_files_recursively_and_never_reports_bytes() {
        let listings = HashMap::from([
            (
                PathBuf::new(),
                HashMap::from([
                    (PathBuf::from("a.txt"), file("a.txt")),
                    (PathBuf::from("sub"), directory("sub")),
                    // The listing's own wrapper node, echoing the directory being listed. Present
                    // here because it is present in the real reply, and because accepting it once
                    // made the walk re-list the root and double every file above it.
                    (PathBuf::new(), directory("")),
                ]),
            ),
            (
                PathBuf::from("sub"),
                HashMap::from([(PathBuf::from("sub/b.txt"), file("sub/b.txt"))]),
            ),
        ]);
        let probe = probe_remote(&FakeClient { listings }, Path::new("/Drive/X")).expect("probe");
        assert_eq!(probe.files, 2);
        assert_eq!(
            probe.bytes, None,
            "a remote listing exposes no usable size; None is the answer, never Some(0)"
        );
        assert!(!probe.truncated);
    }

    #[test]
    fn a_remote_probe_stops_at_its_bound_and_says_so() {
        // Each directory holds one file and one deeper directory, so the walk would never end.
        let mut listings = HashMap::new();
        let mut here = PathBuf::new();
        for depth in 0..(MAX_REMOTE_LISTINGS + 10) {
            let child = here.join(format!("d{depth}"));
            listings.insert(
                here.clone(),
                HashMap::from([
                    (here.join("f.txt"), file("f.txt")),
                    (child.clone(), directory(child.to_str().expect("utf8"))),
                ]),
            );
            here = child;
        }
        let probe = probe_remote(&FakeClient { listings }, Path::new("/Drive/X")).expect("probe");
        assert!(probe.truncated, "hitting the bound must be reported");
        assert_eq!(
            probe.files, MAX_REMOTE_LISTINGS as u64,
            "one file per listing spent, and the count is a lower bound"
        );
    }

    #[test]
    fn a_remote_probe_fails_when_the_candidate_itself_is_unreachable() {
        // A first-listing failure is about the folder the user picked, so it is an error rather
        // than a confident `0 files`.
        let probe = probe_remote(
            &FakeClient {
                listings: HashMap::new(),
            },
            Path::new("/Drive/Nope"),
        );
        assert!(probe.is_err());
    }
}
