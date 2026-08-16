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
//!
//! # Who spawns the `proton-drive` children (#323)
//!
//! **The daemon, whenever there is one.** The `proton-drive` CLI's SQLite cache and session store
//! are shared per user and not safe for concurrent use (#23), which is what
//! [`proton_drive_sync_engine::proton::CliGate`] (in-process) and the user-global lock in
//! `paths.rs` (across daemons) exist to prevent — and neither reaches a GUI process spawning its
//! own children. So the remote probe walks over the control socket, one
//! [`ControlCommand::List`](wire::ControlCommand::List) per directory, each a single child under
//! the daemon's one gate.
//!
//! The **walk itself is client-side, and that is the design rather than a compromise**: `List` may
//! run on the daemon's IPC task only because it is one CLI invocation under a bounded gate wait,
//! and a 64-listing walk is not that. The other daemon-side shape — ack plus a latch, computed on
//! the main loop, as `ControlCommand::Plan` does — queues behind whatever pass is running, which
//! can be half an hour, and this question is asked while a user waits on a folder picker. One
//! request per directory gets its answers in the *gaps* of a live pass, because the gate is held
//! for one child and not one pass.
//!
//! [`probe_remote_via_cli`] survives for the one case that has no daemon to ask — onboarding, which
//! is where four of the five `9a` frames are drawn — and for nothing else. The rule is
//! `run_dry_run`'s: the child **only** when nothing answers the socket at all.

use crate::ipc::{IpcError, send_request};
use crate::wire::{
    ControlCommand, ControlRequest, EntityKind, LIST_ENTRIES_MAX_LIMIT, ListingOutcome, wire_path,
};
use proton_drive_sync_engine::proton::{ProtonClient, ProtonDriveClient, RemoteEntity};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Directory listings one remote probe may spend. Each is a `proton-drive` subprocess, so this
/// bounds both the wall clock and the load placed on a CLI the daemon may also be using.
pub const MAX_REMOTE_LISTINGS: usize = 64;

/// How long one listing request may take, well above [`crate::ipc::DEFAULT_TIMEOUT`]'s 6 s.
///
/// That default is sized for a `status` poll answered from a published snapshot. A `list` is the
/// one verb that runs work: it may spend up to the daemon's `BROWSE_GATE_WAIT` waiting for the CLI
/// gate and then a whole `proton-drive` invocation, so 6 s would time out mid-walk on a busy
/// account — and a timeout maps to [`IpcError::Unreachable`], the one shape that means "no daemon".
pub const PROBE_LISTING_TIMEOUT: Duration = Duration::from_secs(30);

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

/// One directory's immediate children, in whatever path frame the walk is being run in.
pub struct ProbeListing {
    /// `(path, kind)` per child. A directory's path is fed straight back as the next listing's
    /// argument, so it must be in the same frame the walk was seeded in.
    pub entries: Vec<(PathBuf, EntityKind)>,
    /// The lister could not report every child of this directory — the control reply's own entry
    /// cap. The walk both counts fewer files and misses whole subtrees, so it must set
    /// [`FolderProbe::truncated`]: the counts are lower bounds from here down.
    pub truncated: bool,
}

/// Why one listing could not be answered. Two variants, because the walk treats them differently
/// and must not tell them apart by matching a sentence (#103).
#[derive(Debug)]
pub enum ProbeListingError {
    /// **Nothing was attempted**: a `proton-drive` child was already running and the daemon's
    /// bounded gate wait expired ([`ListingOutcome::Busy`]). Retryable as-is; it says nothing about
    /// the folder.
    Busy,
    /// The listing was attempted and failed. Display-only text.
    Failed(String),
    /// The request never completed at the transport — nothing answered the socket. Only the walk's
    /// *first* listing may produce this: after that, a daemon that was alive a moment ago may still
    /// be working, and reading its silence as absence is what would put a second CLI client beside
    /// it (#317's shape).
    Unreachable(IpcError),
}

/// The message a busy probe reports.
///
/// **Any `Busy` fails the whole probe**, rather than folding into `unreadable_directories`. A
/// partially-priced folder is a *wrong number* on the exact screen the number exists for — the
/// point of the step being that a mistake is noticed by size — and "the daemon is mid-transfer" is
/// both transient and the state in which hammering the CLI is the hazard being avoided.
const BUSY_MESSAGE: &str = "the sync daemon is busy with a transfer right now, so this folder could not be measured — \
     try again in a moment";

/// Files under a remote candidate folder, by a bounded breadth-first listing walk over an injected
/// lister.
///
/// [`FolderProbe::bytes`] is always `None` — see the module docs. Returns as soon as
/// [`MAX_REMOTE_LISTINGS`] listings have been spent, with `truncated` set.
///
/// `root` is the path the walk starts from, in the lister's own frame: the empty path for a lister
/// rooted at the candidate (the CLI adapter), or the candidate's absolute path for one that
/// addresses Proton Drive outright (the socket adapter). **One walk, two listers** — a second copy
/// of "what a probe counts" is the thing that would eventually disagree with the first.
pub fn probe_remote_from(
    root: &Path,
    mut list: impl FnMut(&Path) -> Result<ProbeListing, ProbeListingError>,
) -> Result<FolderProbe, ProbeListingError> {
    let mut probe = FolderProbe::default();
    let mut queue = VecDeque::from([root.to_path_buf()]);
    let mut listings = 0usize;
    // Seeded with the root, which is NOT a formality: a listing includes a wrapper node for the
    // directory it lists, so the root echoes itself back. An empty `seen` accepts that echo, queues
    // the root a second time, and counts every root-level file twice — a wrong number on the exact
    // screen the count is for. The queue guard alone stops the loop, not the double count.
    let mut seen: HashSet<PathBuf> = HashSet::from([root.to_path_buf()]);

    while let Some(directory) = queue.pop_front() {
        if listings >= MAX_REMOTE_LISTINGS {
            probe.truncated = true;
            break;
        }
        listings += 1;
        let listing = match list(&directory) {
            Ok(listing) => listing,
            // Not a partial count at any depth — see `BUSY_MESSAGE`.
            Err(error @ ProbeListingError::Busy) => return Err(error),
            Err(error) => {
                // The first listing failing means the candidate itself is unreachable — that is an
                // error about the folder the user picked, not a partial count to render. It is also
                // the only point at which "nothing answered the socket" may be reported as such.
                if listings == 1 {
                    return Err(error);
                }
                probe.unreadable_directories += 1;
                continue;
            }
        };
        probe.truncated |= listing.truncated;
        for (path, kind) in listing.entries {
            match kind {
                EntityKind::File => probe.files += 1,
                // `seen` guards against a listing that echoes its own directory back (the root
                // wrapper node), which would otherwise queue it forever.
                EntityKind::Directory => {
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

/// [`probe_remote_from`] over a [`ProtonClient`], walking in the frame relative to `remote_root`.
///
/// The lister spawns its own `proton-drive` children, so this is the **fallback** path — legitimate
/// only when nothing answers the control socket (see the module docs and [`probe_remote_via_cli`]).
pub fn probe_remote(client: &dyn ProtonClient, remote_root: &Path) -> Result<FolderProbe, String> {
    probe_remote_from(Path::new(""), |directory| {
        client
            .list_directory(remote_root, directory)
            .map(|entries| ProbeListing {
                entries: entries
                    .into_iter()
                    .map(|(path, entity)| {
                        let kind = match entity {
                            RemoteEntity::File(_) => EntityKind::File,
                            RemoteEntity::Directory(_) => EntityKind::Directory,
                        };
                        (path, kind)
                    })
                    .collect(),
                // A CLI listing is not windowed; only the control reply is.
                truncated: false,
            })
            .map_err(|error| ProbeListingError::Failed(error.to_string()))
    })
    .map_err(describe_probe_failure)
}

/// [`probe_remote`] against the real `proton-drive` CLI, spawning children **from this process**.
///
/// Here rather than in the Tauri layer to keep the facade rule intact: the command layer depends on
/// this crate and never on the engine, so the client type stays on this side of the boundary.
///
/// **Legitimate only when nothing answers the control socket** — onboarding, before any daemon
/// exists. Prefer [`probe_remote_via_daemon`], and see the module docs for why (#23/#323).
pub fn probe_remote_via_cli(proton_cli: &str, remote_path: &Path) -> Result<FolderProbe, String> {
    probe_remote(&ProtonDriveClient::new(proton_cli), remote_path)
}

/// [`probe_remote_from`] over the control socket: one [`ControlCommand::List`] per directory, each
/// a single `proton-drive` child behind the daemon's own gate (#323).
///
/// `candidate` is an **absolute** Proton Drive path — the selector frame the daemon added for
/// exactly this, because a candidate folder is outside every configured root by definition. The
/// reply answers in that same frame, so a listed directory is fed straight back as the next
/// request with no re-rooting.
///
/// The error is typed rather than a sentence so the caller can obey the one rule that matters here:
/// fall back to a `proton-drive` child **only** on [`ProbeListingError::Unreachable`], which this
/// returns only for a first listing that never reached the socket. A daemon that answered — even
/// with a failure, even undecodably — is a daemon, and spawning a child beside it is the hazard
/// this function exists to remove.
pub fn probe_remote_via_daemon(
    socket_path: &Path,
    candidate: &Path,
) -> Result<FolderProbe, ProbeListingError> {
    // A relative candidate would be resolved by the daemon against *its own* `remote_root`, so it
    // would measure a real folder — just not the one the user typed, and it would say so with a
    // confident number. Refused rather than resolved, because the whole point of the step is that a
    // wrong folder is noticed by its size.
    if !candidate.is_absolute() {
        return Err(ProbeListingError::Failed(format!(
            "'{}' is not a Proton Drive folder path; it must start with `/`, like /Drive/Photos",
            candidate.display()
        )));
    }
    probe_remote_from(candidate, |directory| {
        let request = ControlRequest {
            argument: Some(wire_path(directory).into_owned()),
            // The daemon's own maximum, so a wide folder is not silently under-counted; the reply
            // still says whether even that was enough, and `ProbeListing::truncated` carries it.
            limit: Some(LIST_ENTRIES_MAX_LIMIT),
            ..ControlRequest::new(ControlCommand::List)
        };
        let response = send_request(socket_path, &request, PROBE_LISTING_TIMEOUT)
            .map_err(ProbeListingError::Unreachable)?;
        match response.listing {
            Some(ListingOutcome::Listed {
                entries, truncated, ..
            }) => Ok(ProbeListing {
                entries: entries
                    .into_iter()
                    .map(|entry| (entry.path, entry.entity_kind))
                    .collect(),
                truncated,
            }),
            Some(ListingOutcome::Busy) => Err(ProbeListingError::Busy),
            Some(ListingOutcome::Failed { error }) => Err(ProbeListingError::Failed(error)),
            // A daemon this build does not understand, or one predating `list`'s absolute selector
            // and refusing it. Reported as the daemon's answer — never as absence, which is what
            // would send a second CLI client to walk beside it.
            Some(ListingOutcome::Unknown) | None => Err(ProbeListingError::Failed(
                "the sync daemon did not answer with a listing; it is probably older than this app"
                    .to_owned(),
            )),
        }
    })
}

/// The sentence a probe failure is shown as. One place, so the CLI and socket paths cannot end up
/// describing the same failure differently.
pub fn describe_probe_failure(error: ProbeListingError) -> String {
    match error {
        ProbeListingError::Busy => BUSY_MESSAGE.to_owned(),
        ProbeListingError::Failed(message) => message,
        ProbeListingError::Unreachable(error) => error.to_string(),
    }
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

    // ---------------------------------------------------------------------------------------
    // #323 — the same walk, driven over the control socket so the daemon owns every child.
    // ---------------------------------------------------------------------------------------

    /// A `ControlResponse` with nothing in it but the fields an older daemon has always sent, used
    /// as the base for the canned `list` replies below.
    const MINIMAL_REPLY: &str = r#"{"status":"running","paused":false,"pending_changes":0,"message":"remote listing","last_sync_epoch_secs":null,"last_error":null,"last_plan_summary":null,"last_successful_sync_summary":null,"status_history":[],"pending_deletions":[]}"#;

    fn reply(listing: Option<ListingOutcome>) -> String {
        let mut response: crate::wire::ControlResponse =
            serde_json::from_str(MINIMAL_REPLY).expect("minimal reply");
        response.listing = listing;
        serde_json::to_string(&response).expect("serialize")
    }

    fn entry(path: &str, kind: EntityKind) -> crate::wire::RemoteEntry {
        crate::wire::RemoteEntry {
            path: PathBuf::from(path),
            name: Path::new(path)
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned(),
            entity_kind: kind,
            sha1: None,
            downloadable: kind == EntityKind::File,
        }
    }

    fn listed(
        path: &str,
        entries: Vec<crate::wire::RemoteEntry>,
        truncated: bool,
    ) -> ListingOutcome {
        ListingOutcome::Listed {
            path: PathBuf::from(path),
            total: entries.len() + usize::from(truncated),
            truncated,
            entries,
        }
    }

    /// A fake daemon answering one canned `list` reply per selector, recording what it was asked.
    /// Unknown selectors get a `Failed` listing, which is what a real daemon says about a folder
    /// that is not there.
    fn spawn_listing_daemon(
        replies: std::collections::HashMap<String, String>,
    ) -> (
        PathBuf,
        tempfile::TempDir,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("control.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            while let Ok((stream, _)) = listener.accept() {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let request: ControlRequest =
                    serde_json::from_str(line.trim_end()).expect("decode request");
                let selector = request.argument.clone().unwrap_or_default();
                recorder.lock().expect("seen lock").push(selector.clone());
                let body = replies.get(&selector).cloned().unwrap_or_else(|| {
                    reply(Some(ListingOutcome::Failed {
                        error: format!("no such folder: {selector}"),
                    }))
                });
                let _ = (&stream).write_all(format!("{body}\n").as_bytes());
            }
        });
        (path, dir, seen)
    }

    #[test]
    fn a_socket_probe_walks_the_candidate_in_the_absolute_frame() {
        // The point of #323: the walk asks the daemon, which owns the one CLI gate — and it asks
        // in absolute paths, because a candidate folder is outside every configured root.
        let replies = std::collections::HashMap::from([
            (
                "/Drive/Candidate".to_owned(),
                reply(Some(listed(
                    "/Drive/Candidate",
                    vec![
                        entry("/Drive/Candidate/a.txt", EntityKind::File),
                        entry("/Drive/Candidate/sub", EntityKind::Directory),
                    ],
                    false,
                ))),
            ),
            (
                "/Drive/Candidate/sub".to_owned(),
                reply(Some(listed(
                    "/Drive/Candidate/sub",
                    vec![entry("/Drive/Candidate/sub/b.txt", EntityKind::File)],
                    false,
                ))),
            ),
        ]);
        let (socket, _dir, seen) = spawn_listing_daemon(replies);

        let probe = probe_remote_via_daemon(&socket, Path::new("/Drive/Candidate"))
            .expect("the probe must succeed");

        assert_eq!(probe.files, 2, "counted recursively");
        assert_eq!(
            probe.bytes, None,
            "a remote listing exposes no usable size, over the socket as over the CLI"
        );
        assert!(!probe.truncated);
        assert_eq!(
            seen.lock().expect("seen lock").as_slice(),
            ["/Drive/Candidate", "/Drive/Candidate/sub"],
            "one request per directory, each an absolute selector fed back from the last reply"
        );
    }

    #[test]
    fn a_relative_candidate_is_refused_rather_than_measured_against_the_daemons_own_root() {
        // The daemon reads a relative selector as `remote_root`-relative, so this would measure a
        // real folder and quote a confident number for the wrong one.
        let (socket, _dir, seen) = spawn_listing_daemon(std::collections::HashMap::new());
        let error = probe_remote_via_daemon(&socket, Path::new("Photos"))
            .expect_err("a relative candidate is not a Proton Drive path");
        assert!(
            matches!(&error, ProbeListingError::Failed(message) if message.contains("/Drive/")),
            "{error:?}"
        );
        assert!(
            seen.lock().expect("seen lock").is_empty(),
            "and it is refused before anything is asked of the daemon"
        );
    }

    #[test]
    fn a_windowed_listing_makes_the_probe_a_lower_bound() {
        // The daemon caps a reply's entries (`LIST_ENTRIES_MAX_LIMIT`). The CLI path has no such
        // cap, so this loss exists only over the socket — and a count that quietly dropped files
        // AND whole subtrees while reading as a total is the one thing this screen must not do.
        let replies = std::collections::HashMap::from([(
            "/Drive/Wide".to_owned(),
            reply(Some(listed(
                "/Drive/Wide",
                vec![entry("/Drive/Wide/a.txt", EntityKind::File)],
                true,
            ))),
        )]);
        let (socket, _dir, _seen) = spawn_listing_daemon(replies);

        let probe =
            probe_remote_via_daemon(&socket, Path::new("/Drive/Wide")).expect("probe succeeds");

        assert_eq!(probe.files, 1);
        assert!(
            probe.truncated,
            "a reply that held entries back makes every count below it a lower bound"
        );
    }

    #[test]
    fn a_busy_daemon_fails_the_whole_probe_rather_than_quoting_a_partial_count() {
        // `Busy` means nothing was attempted. Folding it into `unreadable_directories` would render
        // a confidently wrong number on the exact screen the number exists for.
        let replies = std::collections::HashMap::from([
            (
                "/Drive/Candidate".to_owned(),
                reply(Some(listed(
                    "/Drive/Candidate",
                    vec![
                        entry("/Drive/Candidate/a.txt", EntityKind::File),
                        entry("/Drive/Candidate/sub", EntityKind::Directory),
                    ],
                    false,
                ))),
            ),
            (
                "/Drive/Candidate/sub".to_owned(),
                reply(Some(ListingOutcome::Busy)),
            ),
        ]);
        let (socket, _dir, _seen) = spawn_listing_daemon(replies);

        let error = probe_remote_via_daemon(&socket, Path::new("/Drive/Candidate"))
            .expect_err("a busy listing must not be reported as a count");
        assert!(
            matches!(error, ProbeListingError::Busy),
            "and it must stay typed, so a caller never tells retry from gone by matching a \
             sentence: {error:?}"
        );
        assert!(describe_probe_failure(error).contains("try again"));
    }

    #[test]
    fn only_a_first_listing_that_never_reached_the_socket_is_reported_as_unreachable() {
        // The fallback rule, and the whole reason this error is typed: `Unreachable` is what sends
        // the GUI back to spawning its own `proton-drive` child, so it may only ever mean "there is
        // no daemon". A socket that dies mid-walk is a daemon that was alive a moment ago and may
        // still be working — starting a child beside it is #317's hazard on demand.
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nothing-here.sock");
        let error = probe_remote_via_daemon(&missing, Path::new("/Drive/Candidate"))
            .expect_err("nothing is listening");
        assert!(
            matches!(error, ProbeListingError::Unreachable(_)),
            "{error:?}"
        );

        // The same failure one level deeper is NOT unreachable: the root listed, so a daemon
        // answered, and the missing subfolder is counted as unreadable instead.
        let mut listings = 0usize;
        let probe = probe_remote_from(Path::new("/Drive/Candidate"), |directory| {
            listings += 1;
            if directory == Path::new("/Drive/Candidate") {
                Ok(ProbeListing {
                    entries: vec![
                        (PathBuf::from("/Drive/Candidate/a.txt"), EntityKind::File),
                        (PathBuf::from("/Drive/Candidate/sub"), EntityKind::Directory),
                    ],
                    truncated: false,
                })
            } else {
                Err(ProbeListingError::Unreachable(IpcError::Unreachable(
                    "socket died".to_owned(),
                )))
            }
        })
        .expect("a deeper failure is a partial count, not an absent daemon");
        assert_eq!(probe.files, 1);
        assert_eq!(probe.unreadable_directories, 1);
    }

    #[test]
    fn a_daemon_that_answers_without_a_listing_is_reported_and_never_read_as_absence() {
        // An older daemon knows `list` but refuses an absolute selector, and one newer than this
        // build may answer a state this build cannot name. Both ANSWERED, so neither may become
        // `Unreachable` — which is the only value that sends a second CLI client out.
        for listing in [
            None,
            Some(ListingOutcome::Unknown),
            Some(ListingOutcome::Failed {
                error: "unsafe remote path: /Drive/Candidate".to_owned(),
            }),
        ] {
            let replies = std::collections::HashMap::from([(
                "/Drive/Candidate".to_owned(),
                reply(listing.clone()),
            )]);
            let (socket, _dir, _seen) = spawn_listing_daemon(replies);
            let error = probe_remote_via_daemon(&socket, Path::new("/Drive/Candidate"))
                .expect_err("a daemon that listed nothing is not a probe result");
            assert!(
                matches!(error, ProbeListingError::Failed(_)),
                "{listing:?} answered, so it must not read as an absent daemon: {error:?}"
            );
        }
    }
}
