use crate::AppResult;
use crate::index::EntityKind;
use crate::sync::{DeleteDirection, PlanSummary, SyncAction, UnsyncableItem};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ControlCommand {
    Status,
    Pause,
    Resume,
    Syncnow,
    /// Force the daemon's next reconcile to a full-tree walk instead of a warm start / incremental
    /// pass (e.g. to self-heal suspected drift). Like `Syncnow`, it also schedules that pass. Wire
    /// value `"resync"`; an older daemon that predates this variant rejects it as an unknown
    /// command (the client is simply newer than the daemon).
    Resync,
    /// Approve pending deletions so they apply on the next sync. The `argument` on the request
    /// selects the target: a relative path, or `"all"` for every currently-pending deletion.
    Approve,
    /// Revoke a prior approval (before it has applied). Same `argument` selector as `Approve`.
    Deny,
    /// **Refuse** a withheld deletion and put the two sides back in step: purge the baseline
    /// `file_index` record for the target (and its whole subtree), so the surviving side stops
    /// looking like a deletion and is adopted back onto the other side by the bootstrap arm — an
    /// upload for `direction: local`, a download for `direction: remote` (#224). Same `argument`
    /// selector as `Approve`, and like it, only a *currently pending* deletion can be named.
    ///
    /// Distinct from [`Self::Deny`], which revokes an approval and leaves the deletion withheld and
    /// re-derived for ever. Wire value `"keep"`; an older daemon rejects it as an unknown command.
    Keep,
    /// Ask the daemon to exit gracefully (same clean path as SIGTERM). Lets a UI restart the
    /// daemon regardless of how it was launched (systemd unit or direct spawn).
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlRequest {
    pub command: ControlCommand,
    /// Optional argument for commands that need one (`Approve`/`Deny`). `#[serde(default)]` keeps
    /// the wire shape backward-compatible: older clients that omit it still parse.
    #[serde(default)]
    pub argument: Option<String>,
    /// When `true`, `argument` is always a **literal relative path** — the daemon must not give
    /// the reserved word `"all"` its every-pending-item meaning. This lets a client target a
    /// file literally named `all` (any letter case) without mass-approving. `#[serde(default)]`
    /// keeps wire compat: legacy clients omit it and retain the historical case-insensitive
    /// `"all"` interpretation.
    #[serde(default)]
    pub literal_path: bool,
    /// Which deletion at `argument` an `Approve` authorizes when **nothing pending matches it** —
    /// the pre-pass approval the Plan screen's typed-`DELETE` gate needs (#227). A plan can name a
    /// deletion before any pass has withheld it, and a path alone does not say which of the two
    /// deletions at it is meant, so an approval with no direction authorizes nothing (the #298 rule:
    /// an ambiguous selector authorizes nothing). Ignored when the selector *does* match a pending
    /// item — that item's own direction is the authority — and ignored entirely by every other
    /// command. `#[serde(default)]` keeps both directions of the wire compatible.
    #[serde(default)]
    pub direction: Option<DeleteDirection>,
}

/// The wire form of a path, and the form an `approve`/`deny` selector is matched against. One
/// definition for every wire this engine publishes — see [`crate::wire_path`], which also states
/// the rule a client may act on a lossy path under.
pub use crate::wire_path;

/// One withheld deletion surfaced to the user for review. `path` + `direction` identify it for an
/// `approve`; `fingerprint` (a file's baseline SHA-1 or a directory's `proton_id`) is what the
/// approval is pinned to, so it cannot later authorize a different deletion at the same path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingDeletion {
    /// Lossy on the wire (see [`crate::lossy_path`]); the daemon keeps the real path in this
    /// field.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    pub direction: DeleteDirection,
    pub entity_kind: EntityKind,
    pub fingerprint: String,
    /// When **this pass** derived the withheld action — the age of the pass, not of the deletion.
    /// The gate stamps `now` on everything it withholds and a pass cannot idle-skip while anything
    /// is pending, so this refreshes every ~30s for as long as the item waits. Read
    /// [`Self::first_seen_epoch_secs`] for "when did this happen"; this one answers "how fresh is
    /// this reply". Kept under its original name because it is a required field on the wire and an
    /// older client would fail to parse a reply without it (#225).
    pub detected_epoch_secs: u64,
    /// When this deletion was **first** withheld, carried across passes and restarts in
    /// `index::withheld_deletions` and re-stamped only when the fingerprint changes (a different
    /// deletion at the same path). This is the field a UI ages "deleted on Proton 22m ago" from.
    /// `#[serde(default)]` for replies from an older daemon — and `0` there means *unknown*, so a
    /// client must treat it as "no age to show" rather than as the epoch.
    #[serde(default)]
    pub first_seen_epoch_secs: u64,
    /// Files beneath a directory deletion, and their total size — what the subtree would cost you
    /// (#208). Counted from the baseline index at gate time, files only (a directory's own
    /// `file_size` is not a subtree total), so it is what the engine would actually delete: the
    /// baseline is already selective-sync filtered. `None` for a file (its own size is a lookup the
    /// client already has) and `None` from an older daemon — never `0`, which is a real answer for
    /// an empty folder.
    #[serde(default)]
    pub subtree_files: Option<u64>,
    #[serde(default)]
    pub subtree_bytes: Option<u64>,
}

/// One action whose side effect failed during the last pass (#136). The executor keeps going past
/// it — the rest of the plan still runs — so a pass can end with some items failed and everything
/// else synced. That is a **third** pass outcome, not a flavour of either other one: it is reported
/// as itself here and in [`crate::daemon::PassOutcome`], never folded into success or failure.
///
/// Reported, never acted on: the failed action is not recorded in the index and simply re-plans
/// next pass. This is not a retry queue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedItem {
    /// Lossy on the wire (see [`crate::lossy_path`]); the daemon keeps the real path in this
    /// field.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    pub action: SyncAction,
    /// The failure. At most [`FAILED_ITEM_ERROR_LIMIT`] bytes, ellipsis included — these ride on
    /// every status reply.
    pub error: String,
}

/// Hard cap on a [`FailedItem::error`] string in bytes, *inclusive* of the truncation ellipsis: a
/// CLI failure can carry kilobytes of stderr.
pub const FAILED_ITEM_ERROR_LIMIT: usize = 500;

/// The daemon's resolved folder pair + index location, surfaced over IPC so a UI can reflect the
/// *live* configuration no matter how the daemon was launched (config file, flags, or defaults).
/// Without this, a client that guesses at a config path renders placeholders against a healthy
/// daemon whose roots it cannot know.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunningConfigInfo {
    #[serde(with = "crate::lossy_path")]
    pub local_root: PathBuf,
    #[serde(with = "crate::lossy_path")]
    pub remote_root: PathBuf,
    #[serde(with = "crate::lossy_path")]
    pub db_path: PathBuf,
}

/// Live "what is the daemon doing right now", surfaced while `syncing` is true so clients can
/// render more than a spinner during a long pass (a multi-minute remote walk, a multi-GB
/// transfer). Purely informational display data: every field is best-effort, absence means
/// "unknown or not applicable", and nothing here participates in any sync decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncActivity {
    /// Coarse machine-readable step: `scanning-local`, `listing-remote`, `fetching-events`,
    /// `executing`, or `committing`. Clients should render an unrecognized token verbatim
    /// rather than fail, so new phases can be added without a lockstep upgrade.
    pub phase: String,
    /// Human-readable fragment locating the phase: the folder currently being listed, the file
    /// currently being scanned, or the action currently executing.
    #[serde(default)]
    pub detail: Option<String>,
    /// Remote folders listed so far during a `listing-remote` walk.
    #[serde(default)]
    pub folders_listed: Option<u64>,
    /// Local files visited so far during a `scanning-local` pass.
    #[serde(default)]
    pub files_scanned: Option<u64>,
    /// 1-based position of the currently executing action within the plan (`executing`).
    #[serde(default)]
    pub action_index: Option<u64>,
    /// Total number of planned actions this pass (`executing`).
    #[serde(default)]
    pub action_total: Option<u64>,
    /// The in-flight file transfer, when the executing action is an upload or download.
    #[serde(default)]
    pub transfer: Option<TransferActivity>,
    /// When this phase began (unix seconds), for elapsed-time rendering.
    #[serde(default)]
    pub since_epoch_secs: Option<u64>,
}

/// A file transfer in flight. For downloads, `bytes_done` is sampled live from the staging
/// scratch directory each time a status reply is built, so a client polling status watches the
/// number grow while the CLI child is still running.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferActivity {
    /// `"upload"` or `"download"`.
    pub direction: String,
    /// Root-relative path of the file being transferred.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    /// Total size in bytes when known (uploads: the local file's size; downloads: unknown —
    /// the remote listing carries no size).
    #[serde(default)]
    pub bytes_total: Option<u64>,
    /// Bytes transferred so far when observable (downloads only; see the type docs).
    #[serde(default)]
    pub bytes_done: Option<u64>,
    /// When this transfer began (unix seconds).
    pub started_epoch_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlResponse {
    pub status: String,
    pub paused: bool,
    /// `true` while a reconcile pass is actually in flight. Distinct from `status == "running"`,
    /// which only means "not paused". `#[serde(default)]` keeps replies from older daemons
    /// parseable.
    #[serde(default)]
    pub syncing: bool,
    /// Count of completed reconcile attempts (success or failure) since the daemon started.
    /// A client that scheduled a sync can poll until this advances past the value in its ack
    /// (and `syncing` is false again) to know *its* pass finished. `#[serde(default)]` for
    /// replies from older daemons.
    #[serde(default)]
    pub reconcile_seq: u64,
    pub pending_changes: usize,
    pub message: String,
    pub last_sync_epoch_secs: Option<u64>,
    pub last_error: Option<String>,
    pub last_plan_summary: Option<PlanSummary>,
    pub last_successful_sync_summary: Option<PlanSummary>,
    pub status_history: Vec<StatusHistoryEntry>,
    /// Deletions currently withheld by the delete-approval guard, awaiting the user's approval.
    /// `#[serde(default)]` so a response from an older daemon (without the field) still parses.
    #[serde(default)]
    pub pending_deletions: Vec<PendingDeletion>,
    /// The live resolved configuration (see [`RunningConfigInfo`]). `#[serde(default)]` keeps
    /// replies from older daemons parseable.
    #[serde(default)]
    pub config: Option<RunningConfigInfo>,
    /// What the daemon is doing right now (see [`SyncActivity`]); `None` when idle or from an
    /// older daemon (`#[serde(default)]` keeps both directions of the wire compatible).
    #[serde(default)]
    pub activity: Option<SyncActivity>,
    /// Items whose action failed during the last pass, **bounded** to the first
    /// `daemon::FAILED_ITEMS_REPORTED` in plan order (#136). `failed_item_count` is the untruncated
    /// total, so `failed_items.len() <= failed_item_count` always. A non-zero count is the wire form
    /// of the partial-failure pass outcome: some items failed, everything else synced.
    /// `#[serde(default)]` for replies from older daemons.
    #[serde(default)]
    pub failed_items: Vec<FailedItem>,
    /// How many items failed in the last pass, counting past the bound on `failed_items`.
    #[serde(default)]
    pub failed_item_count: usize,
    /// Entities the engine cannot sync and never will without user action (see
    /// [`UnsyncableItem`]) — a skipped entity used to be nothing but an anonymous
    /// `skipped_unsupported` counter, so a cloud-only file could sit unsynced for months with
    /// nothing naming it (#295). Display-only: like [`SyncActivity`], nothing here participates in
    /// any sync decision — the planner re-derives every skip from ground truth. `#[serde(default)]`
    /// so a reply from an older daemon still parses.
    #[serde(default)]
    pub unsyncable: Vec<UnsyncableItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusHistoryEntry {
    pub epoch_secs: u64,
    pub message: String,
    pub last_error: Option<String>,
    pub plan_summary: Option<PlanSummary>,
    pub successful_sync_summary: Option<PlanSummary>,
    /// How many items failed in this pass (#136); `0` for a clean or wholly-failed pass, so a
    /// history reader can tell a partial pass from either. `#[serde(default)]` for entries written
    /// by an older daemon (the sidecar is read back across upgrades).
    #[serde(default)]
    pub failed_item_count: usize,
}

#[cfg(unix)]
pub async fn bind_listener(socket_path: &Path) -> AppResult<UnixListener> {
    use std::os::unix::fs::FileTypeExt;

    // Use `symlink_metadata` (not `exists`/`metadata`) so a symlink at `socket_path` is
    // classified by its own type rather than transparently followed - and only ever
    // remove a path that is actually a leftover Unix socket from a previous run. If a
    // misconfigured `--socket-path` points at a regular file or symlink, deleting it
    // unconditionally would destroy user data; refuse instead.
    match std::fs::symlink_metadata(socket_path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            std::fs::remove_file(socket_path)?;
        }
        Ok(_) => {
            return Err(crate::boxed_error(format!(
                "refusing to bind control socket: {} already exists and is not a socket",
                socket_path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // #62: `bind` creates the socket with `0777 & ~umask`, and a chmod at the published path lands
    // only afterwards — with a permissive umask another local user can connect in that window and
    // issue any control command (`approve --all` authorises every withheld deletion). Bind inside a
    // freshly created owner-only directory instead: there the socket is unreachable to anyone else
    // whatever its own mode, so the loose-mode window is not observable, and the rename into place
    // preserves the binding (the inode is what is bound; clients resolve the published name to it).
    // This is the "land via `fs::rename` from a staging dir" shape the path-safety invariant
    // already requires for a write to a predictable name. Deliberately NOT a process-wide
    // `libc::umask` flip (the fix the issue proposed): this runs on a multithreaded runtime
    // alongside the startup sidecar/DB writes, so it would trade this race for a worse one.
    let staging = StagingDir::create(socket_path)?;
    let staged_socket = staging.socket_path();
    // The staged path is LONGER than the published one, so a configured socket that fits
    // `sun_path` on its own can overflow once staged. `bind`'s own error ("path must be shorter
    // than SUN_LEN") would name a path the user never configured and give no hint that staging is
    // why, so check here and say all three things: which socket, how long staged, what the limit
    // is. Binding in place instead is not an option — that is the vulnerability.
    let capacity = sun_path_capacity();
    if staged_socket.as_os_str().len() >= capacity {
        return Err(crate::boxed_error(format!(
            "failed to bind control socket {}: it is bound in a private staging directory and \
             renamed into place (so it is never briefly world-connectable), and that staged path \
             is {} bytes — past this platform's {}-byte limit for a Unix socket path. Use a \
             shorter socket_path, or one in a shorter directory.",
            socket_path.display(),
            staged_socket.as_os_str().len(),
            capacity - 1
        )));
    }
    // Errors name the PUBLISHED path: the staging directory is an implementation detail, and a
    // `sun_path`-too-long or permission failure is about the socket the user configured.
    let listener = UnixListener::bind(&staged_socket).map_err(|error| {
        crate::boxed_error(format!(
            "failed to bind control socket {}: {error}",
            socket_path.display()
        ))
    })?;
    std::fs::set_permissions(&staged_socket, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&staged_socket, socket_path).map_err(|error| {
        crate::boxed_error(format!(
            "failed to publish control socket at {}: {error}",
            socket_path.display()
        ))
    })?;
    Ok(listener)
}

/// Maximum `.psb-<pid>-<n>` names tried before giving up. Two things leave one behind: a SIGKILL
/// that skips [`Drop`], and — by design — a `Drop` that ran and *refused*, because it removes only
/// an empty directory and another local user planted something inside (see the `Drop` impl). Either
/// way the loop steps **past** a taken name rather than clearing it, so a leftover costs one attempt
/// and a start fails only if a reused pid meets this many of them.
#[cfg(unix)]
const STAGING_DIR_ATTEMPTS: u64 = 16;
/// Fixed, short staged socket name. Both this and the staging directory name are charged to the
/// platform's `sockaddr_un.sun_path` budget on top of the published path, so neither carries
/// anything beyond what uniqueness needs. **Nothing validates that budget ahead of `bind`** —
/// `paths::default_socket_path` just joins, and the XDG default happens to land far inside it — so
/// [`bind_listener`] measures the staged path itself rather than assuming it fits.
#[cfg(unix)]
const STAGED_SOCKET_NAME: &str = "s";

/// Bytes the platform gives a Unix socket path, NUL terminator included. Read off the platform's
/// own `sockaddr_un` rather than hardcoded: it is 108 on Linux and 104 on some BSDs.
#[cfg(unix)]
fn sun_path_capacity() -> usize {
    std::mem::size_of::<libc::sockaddr_un>() - std::mem::offset_of!(libc::sockaddr_un, sun_path)
}

/// The private directory the control socket is bound in before being renamed to its published
/// path (see [`bind_listener`]). Cleaned up on drop, so every failure path cleans up — but only
/// ever its own, empty self (see the `Drop` impl).
#[cfg(unix)]
struct StagingDir {
    directory: PathBuf,
}

#[cfg(unix)]
impl StagingDir {
    /// Created as a sibling of `socket_path` so the rename is same-directory (hence atomic and
    /// never cross-device). `DirBuilder::mode(0o700)` is umask-proof: mkdir's umask can only
    /// *clear* permission bits, never add them, so the directory is owner-only from the instant it
    /// exists, and `mkdir` never follows a symlink planted at the name — a leftover is retried
    /// past, never removed. Together with the non-recursive [`Drop`], nothing here can delete
    /// anything this process did not put there.
    fn create(socket_path: &Path) -> AppResult<Self> {
        use std::os::unix::fs::DirBuilderExt;

        let parent = socket_path.parent().ok_or_else(|| {
            crate::boxed_error(format!(
                "refusing to bind control socket: {} has no parent directory",
                socket_path.display()
            ))
        })?;
        // Unique per process *and* per call: one process binds more than one listener over its
        // lifetime (and many in the test suite), and a reused name would collide.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        let mut last_error = None;
        for _ in 0..STAGING_DIR_ATTEMPTS {
            let directory = parent.join(format!(
                ".psb-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match builder.create(&directory) {
                Ok(()) => return Ok(Self { directory }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(error);
                }
                // Names the SOCKET as well as the staging directory: the commonest cause is a
                // parent that does not exist, which used to surface as a plain bind failure on
                // the configured path and must stay as recognisable.
                Err(error) => {
                    return Err(crate::boxed_error(format!(
                        "failed to bind control socket {}: could not create its staging directory \
                         {}: {error}",
                        socket_path.display(),
                        directory.display()
                    )));
                }
            }
        }
        Err(crate::boxed_error(format!(
            "failed to bind control socket {}: {} staging directory names under {} were already \
             taken (last: {})",
            socket_path.display(),
            STAGING_DIR_ATTEMPTS,
            parent.display(),
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown".to_owned())
        )))
    }

    fn socket_path(&self) -> PathBuf {
        self.directory.join(STAGED_SOCKET_NAME)
    }
}

#[cfg(unix)]
impl Drop for StagingDir {
    /// Deliberately **not** `remove_dir_all`. In the very deployment #62 is about — a socket path
    /// under a directory other local users can write — an attacker can remove this name after
    /// `create` made it and leave their own tree at it, and a recursive delete would then take
    /// that tree. `remove_dir` refuses a non-empty directory (`ENOTEMPTY`) and refuses a symlink
    /// (`ENOTDIR`), so the only thing it can ever remove is the empty directory this process
    /// created. The staged socket is normally already renamed away by now; removing it first
    /// covers the path where the bind succeeded and the rename did not.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.socket_path());
        let _ = std::fs::remove_dir(&self.directory);
    }
}

/// Sends a no-argument control command. Thin wrapper over [`send_request`] kept for the commands
/// that carry no argument (`status`/`pause`/`resume`/`syncnow`).
#[cfg(unix)]
pub async fn send_command(
    socket_path: &Path,
    command: ControlCommand,
) -> AppResult<ControlResponse> {
    send_request(
        socket_path,
        ControlRequest {
            command,
            argument: None,
            literal_path: false,
            direction: None,
        },
    )
    .await
}

/// Sends a full control request (used by commands that carry an `argument`, e.g. `approve <path>`).
#[cfg(unix)]
pub async fn send_request(
    socket_path: &Path,
    request: ControlRequest,
) -> AppResult<ControlResponse> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let request = serde_json::to_vec(&request)?;
    stream.write_all(&request).await?;
    stream.write_all(b"\n").await?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;
    Ok(serde_json::from_str(response.trim())?)
}

/// Upper bound on the bytes read while parsing a single control request. A control
/// request is a short JSON line, so capping the read keeps a client that streams bytes
/// without ever sending a newline from growing the read buffer without bound. Reaching
/// the cap yields an incomplete line that fails to parse, dropping the connection.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

#[cfg(unix)]
pub async fn read_request(stream: UnixStream) -> AppResult<(ControlRequest, UnixStream)> {
    let mut reader = BufReader::new(stream).take(MAX_REQUEST_BYTES);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let request = serde_json::from_str(line.trim())?;
    Ok((request, reader.into_inner().into_inner()))
}

#[cfg(unix)]
pub async fn write_response(stream: &mut UnixStream, response: &ControlResponse) -> AppResult<()> {
    stream
        .write_all(format!("{}\n", serde_json::to_string(response)?).as_bytes())
        .await?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::sync::UnsyncableReason;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn control_request_without_literal_path_parses_as_legacy() {
        // A request from an older client omits `literal_path`; it must parse with the flag off,
        // preserving the historical case-insensitive "all" interpretation for that client.
        let legacy = r#"{"command":"approve","argument":"all"}"#;
        let request: ControlRequest =
            serde_json::from_str(legacy).expect("legacy request must parse");
        assert_eq!(request.argument.as_deref(), Some("all"));
        assert!(!request.literal_path);
    }

    #[test]
    fn a_pending_deletion_from_an_older_daemon_still_parses() {
        // The lifecycle fields (#225 first-seen, #208 subtree totals) are additions to a shape an
        // older daemon already emits, so they default rather than fail the whole reply. `0` and
        // `None` are the honest readings of "this daemon does not know", which is why a client
        // must not age anything from a zero.
        let legacy = r#"{
            "path": "photos/2019",
            "direction": "local",
            "entity_kind": "directory",
            "fingerprint": "vol~node",
            "detected_epoch_secs": 42
        }"#;
        let pending: PendingDeletion =
            serde_json::from_str(legacy).expect("legacy pending deletion must parse");
        assert_eq!(pending.detected_epoch_secs, 42);
        assert_eq!(pending.first_seen_epoch_secs, 0);
        assert_eq!((pending.subtree_files, pending.subtree_bytes), (None, None));
    }

    #[test]
    fn a_keep_request_names_a_command_an_older_client_never_sent() {
        // Wire-visible names, pinned: `keep` is the refusal (#224) and `direction` rides along for
        // `approve`'s pre-pass form (#227). Both are additive — an older client omits `direction`
        // and never sends `keep`.
        let json = serde_json::to_string(&ControlRequest {
            command: ControlCommand::Keep,
            argument: Some("photos/2019".to_owned()),
            literal_path: true,
            direction: None,
        })
        .expect("serialize");
        assert!(json.contains(r#""command":"keep""#), "{json}");

        let approve: ControlRequest = serde_json::from_str(
            r#"{"command":"approve","argument":"a.txt","direction":"remote"}"#,
        )
        .expect("parse");
        assert_eq!(approve.direction, Some(DeleteDirection::Remote));
    }

    #[test]
    fn control_response_without_activity_still_parses() {
        // A reply from an older daemon carries no `activity` field; a newer client must parse
        // it (as None) rather than error — the same guarantee the other `#[serde(default)]`
        // fields give.
        let legacy = r#"{
            "status": "running",
            "paused": false,
            "pending_changes": 0,
            "message": "daemon status",
            "last_sync_epoch_secs": null,
            "last_error": null,
            "last_plan_summary": null,
            "last_successful_sync_summary": null,
            "status_history": []
        }"#;
        let response: ControlResponse =
            serde_json::from_str(legacy).expect("legacy reply must parse");
        assert!(response.activity.is_none());
        assert!(!response.syncing);
    }

    #[test]
    fn sync_activity_round_trips_and_partial_json_defaults() {
        let activity = SyncActivity {
            phase: "executing".to_owned(),
            detail: Some("downloading a/b.bin".to_owned()),
            folders_listed: None,
            files_scanned: None,
            action_index: Some(3),
            action_total: Some(10),
            transfer: Some(TransferActivity {
                direction: "download".to_owned(),
                path: PathBuf::from("a/b.bin"),
                bytes_total: None,
                bytes_done: Some(1024),
                started_epoch_secs: 5,
            }),
            since_epoch_secs: Some(4),
        };
        let json = serde_json::to_string(&activity).expect("serialize");
        let back: SyncActivity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, activity);

        // A minimal activity (just the phase) parses, every optional field defaulting — so
        // phases can grow fields without breaking older clients.
        let minimal: SyncActivity =
            serde_json::from_str(r#"{"phase": "listing-remote"}"#).expect("minimal activity");
        assert_eq!(minimal.phase, "listing-remote");
        assert!(minimal.transfer.is_none());
        assert!(minimal.folders_listed.is_none());
    }

    /// A path whose bytes are not valid UTF-8 (the engine supports them: `index_key` is a BLOB).
    fn non_utf8_path(suffix: &[u8]) -> PathBuf {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = b"bad-\xff".to_vec();
        bytes.extend_from_slice(suffix);
        PathBuf::from(OsStr::from_bytes(&bytes))
    }

    fn response_with_non_utf8_paths() -> ControlResponse {
        ControlResponse {
            status: "running".to_owned(),
            paused: false,
            syncing: true,
            reconcile_seq: 7,
            pending_changes: 0,
            message: "daemon status".to_owned(),
            last_sync_epoch_secs: None,
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: Vec::new(),
            pending_deletions: vec![PendingDeletion {
                path: non_utf8_path(b".txt"),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: "hash".to_owned(),
                detected_epoch_secs: 1,
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
            }],
            failed_items: vec![FailedItem {
                path: non_utf8_path(b"-failed.txt"),
                action: SyncAction::Upload,
                error: "upload failed".to_owned(),
            }],
            failed_item_count: 1,
            config: Some(RunningConfigInfo {
                local_root: non_utf8_path(b"-root"),
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                db_path: non_utf8_path(b"-root/.sync/sync_index.db"),
            }),
            activity: Some(SyncActivity {
                phase: "executing".to_owned(),
                detail: None,
                folders_listed: None,
                files_scanned: None,
                action_index: Some(1),
                action_total: Some(1),
                transfer: Some(TransferActivity {
                    direction: "download".to_owned(),
                    path: non_utf8_path(b".bin"),
                    bytes_total: None,
                    bytes_done: None,
                    started_epoch_secs: 2,
                }),
                since_epoch_secs: None,
            }),
            unsyncable: vec![UnsyncableItem {
                path: non_utf8_path(b"-doc"),
                entity_kind: EntityKind::File,
                reason: UnsyncableReason::RemoteNotDownloadable,
                first_seen_epoch_secs: 3,
            }],
        }
    }

    #[test]
    fn a_non_utf8_path_anywhere_still_serializes_the_whole_response() {
        // #61: `impl Serialize for Path` ERRORS on non-UTF-8, and every PathBuf here rides on
        // every reply — one such path used to fail `write_response` for status/pause/approve
        // alike, locking the control plane out until the pending item cleared.
        let response = response_with_non_utf8_paths();
        let json =
            serde_json::to_string(&response).expect("a non-UTF-8 path must not fail a reply");
        assert!(
            json.contains('\u{fffd}'),
            "the wire form must be the lossy rendering: {json}"
        );

        let back: ControlResponse = serde_json::from_str(&json).expect("reply parses back");
        assert_eq!(
            back.pending_deletions[0].path,
            PathBuf::from(&*wire_path(&response.pending_deletions[0].path)),
            "a client sees exactly the lossy form the daemon published"
        );
        assert_eq!(
            back.failed_items[0].path,
            PathBuf::from(&*wire_path(&response.failed_items[0].path)),
            "a failed item's path is lossy on the wire like every other path"
        );
        assert_eq!(
            back.config.expect("config").local_root,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-root")))
        );
        // #295's list rides the same reply, so it is bound by the same rule: a non-UTF-8 path is
        // exactly the kind of entity that lands on it.
        assert_eq!(
            back.unsyncable[0].path,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-doc")))
        );
    }

    #[test]
    fn wire_path_is_the_form_both_ends_match_on() {
        // The approve/deny selector a client can send is the lossy string it received; a daemon
        // that compared it against the real PathBuf could never match one (#61).
        let real = non_utf8_path(b".txt");
        assert_ne!(real, PathBuf::from(&*wire_path(&real)));
        assert_eq!(wire_path(&real), real.to_string_lossy());
        // UTF-8 paths are untouched, so nothing about the ordinary case changes.
        assert_eq!(wire_path(Path::new("a/b.txt")), "a/b.txt");
    }

    #[tokio::test]
    async fn the_socket_is_never_created_in_place_at_the_published_path() {
        // #62: `bind` creates the socket with `0777 & ~umask` and the chmod lands after, so a
        // permissive umask leaves a window where any local user can connect and issue commands.
        // The window is not observable by reading the mode afterwards (both orderings end at
        // 0600), so watch the parent directory instead: the socket must ARRIVE at the published
        // path already configured (a rename), never be CREATED there.
        use notify::{EventKind, RecursiveMode, Watcher};

        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .expect("watcher");
        watcher
            .watch(directory.path(), RecursiveMode::NonRecursive)
            .expect("watch the socket's parent directory");

        let listener = bind_listener(&socket_path).await.expect("bind listener");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut arrivals = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(event)) => {
                    if event.paths.iter().any(|path| path == &socket_path) {
                        arrivals.push(event.kind);
                    }
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    if !arrivals.is_empty() {
                        break;
                    }
                }
            }
        }
        drop(watcher);
        drop(listener);

        assert!(
            !arrivals.is_empty(),
            "no filesystem event for the socket path was observed — the test would pass vacuously"
        );
        assert!(
            !arrivals
                .iter()
                .any(|kind| matches!(kind, EventKind::Create(_))),
            "the socket must be renamed into place, not created there with the process umask: \
             {arrivals:?}"
        );
    }

    #[tokio::test]
    async fn a_socket_path_that_fits_only_unstaged_is_refused_with_the_reason() {
        // Copilot review: staging makes the bound path LONGER than the configured one, so a socket
        // path that fits `sun_path` on its own can overflow once staged. `bind`'s own error names
        // the staged path — which the user never configured — and never mentions staging, so the
        // check is here. Binding in place instead would be the vulnerability #62 is about.
        let directory = tempdir().expect("tempdir");
        let capacity = sun_path_capacity();
        let name = "s.sock";
        // The longest published path that still fits unstaged: any staging suffix overflows it,
        // whatever this run's pid length happens to be.
        let pad = (capacity - 1)
            .checked_sub(directory.path().as_os_str().len() + 2 + name.len())
            .expect("the temp dir must leave room to pad up to the limit");
        let padded = directory.path().join("p".repeat(pad));
        std::fs::create_dir(&padded).expect("padding directory");
        let socket_path = padded.join(name);
        assert_eq!(
            socket_path.as_os_str().len(),
            capacity - 1,
            "the published path must sit exactly at the limit, or the test proves nothing"
        );

        let error = bind_listener(&socket_path)
            .await
            .expect_err("a staged path past the limit must be refused");

        let message = error.to_string();
        assert!(
            message.contains("limit for a Unix socket path")
                && message.contains(&socket_path.display().to_string()),
            "the error must name the configured socket and the limit: {message}"
        );
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&padded)
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the refused bind must leave nothing behind: {leftovers:?}"
        );
    }

    #[test]
    fn dropping_a_staging_dir_never_deletes_a_tree_it_did_not_create() {
        // Copilot review, and it holds: `remove_dir_all` on a REAL directory deletes its whole
        // tree (only a symlink is left unfollowed). In the group-writable parent #62 is about, an
        // attacker can drop this name after `create` made it and leave their own tree at it, so a
        // recursive cleanup would take that tree. `remove_dir` refuses a non-empty directory.
        let directory = tempdir().expect("tempdir");
        let staging =
            StagingDir::create(&directory.path().join("daemon.sock")).expect("staging dir");
        let planted = staging.directory.join("attacker-tree");
        std::fs::create_dir(&planted).expect("plant a directory");
        std::fs::write(planted.join("data"), b"not ours").expect("plant a file");
        let staging_path = staging.directory.clone();

        drop(staging);

        assert!(
            planted.join("data").exists(),
            "cleanup must not recurse into content this process did not put there"
        );
        assert!(staging_path.is_dir());
        std::fs::remove_dir_all(&staging_path).expect("test cleanup");
    }

    #[tokio::test]
    async fn bind_listener_leaves_no_staging_directory_behind() {
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");

        let listener = bind_listener(&socket_path).await.expect("bind listener");
        let leftovers: Vec<PathBuf> = std::fs::read_dir(directory.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path != &socket_path)
            .collect();
        drop(listener);

        assert!(
            leftovers.is_empty(),
            "the private staging directory must be removed: {leftovers:?}"
        );
    }

    #[tokio::test]
    async fn a_socket_bound_through_staging_answers_at_the_published_path() {
        // The rename must preserve the binding: a client connecting to the published path
        // reaches the listener that was bound inside the staging directory.
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");
        let listener = bind_listener(&socket_path).await.expect("bind listener");

        let connect = UnixStream::connect(&socket_path).await;
        assert!(
            connect.is_ok(),
            "the renamed socket must still accept connections: {connect:?}"
        );
        drop(listener);
    }

    #[tokio::test]
    async fn bind_listener_restricts_socket_permissions() {
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");

        let listener = bind_listener(&socket_path).await.expect("bind listener");
        let mode = std::fs::metadata(&socket_path)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;

        drop(listener);
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn bind_listener_refuses_to_replace_a_regular_file() {
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");
        std::fs::write(&socket_path, b"not a socket").expect("write regular file");

        let error = bind_listener(&socket_path)
            .await
            .expect_err("bind_listener must refuse to delete a non-socket file");

        assert!(
            error
                .to_string()
                .contains("already exists and is not a socket"),
            "unexpected error: {error}"
        );
        assert_eq!(
            std::fs::read(&socket_path).expect("file preserved"),
            b"not a socket",
            "the pre-existing file must not be deleted"
        );
    }

    #[tokio::test]
    async fn bind_listener_replaces_a_stale_socket() {
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");
        {
            let stale_listener = bind_listener(&socket_path)
                .await
                .expect("bind stale socket");
            drop(stale_listener);
        }
        // The socket file itself still exists on disk after the listener is dropped;
        // binding again must recognize it as a socket and replace it cleanly.
        assert!(socket_path.exists());

        let listener = bind_listener(&socket_path)
            .await
            .expect("rebinding over a stale socket must succeed");
        drop(listener);
    }

    #[tokio::test]
    async fn read_request_rejects_an_unterminated_oversized_line() {
        let (mut client, server) = UnixStream::pair().expect("socket pair");
        // Flood the connection with more than the cap and never send a newline or close,
        // mimicking a client that streams bytes to grow the read buffer without bound.
        let writer = tokio::spawn(async move {
            let junk = vec![b'x'; MAX_REQUEST_BYTES as usize + 1024];
            let _ = client.write_all(&junk).await;
            std::future::pending::<()>().await;
        });

        let result = read_request(server).await;
        writer.abort();

        assert!(
            result.is_err(),
            "an over-length request with no newline must be rejected, not read unbounded"
        );
    }
}
