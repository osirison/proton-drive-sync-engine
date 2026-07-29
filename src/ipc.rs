use crate::AppResult;
use crate::index::EntityKind;
use crate::sync::{DeleteDirection, PlanSummary};
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
    /// Approve pending deletions so they apply on the next sync. The `argument` on the request
    /// selects the target: a relative path, or `"all"` for every currently-pending deletion.
    Approve,
    /// Revoke a prior approval (before it has applied). Same `argument` selector as `Approve`.
    Deny,
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
}

/// One withheld deletion surfaced to the user for review. `path` + `direction` identify it for an
/// `approve`; `fingerprint` (a file's baseline SHA-1 or a directory's `proton_id`) is what the
/// approval is pinned to, so it cannot later authorize a different deletion at the same path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingDeletion {
    pub path: PathBuf,
    pub direction: DeleteDirection,
    pub entity_kind: EntityKind,
    pub fingerprint: String,
    pub detected_epoch_secs: u64,
}

/// The daemon's resolved folder pair + index location, surfaced over IPC so a UI can reflect the
/// *live* configuration no matter how the daemon was launched (config file, flags, or defaults).
/// Without this, a client that guesses at a config path renders placeholders against a healthy
/// daemon whose roots it cannot know.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunningConfigInfo {
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusHistoryEntry {
    pub epoch_secs: u64,
    pub message: String,
    pub last_error: Option<String>,
    pub plan_summary: Option<PlanSummary>,
    pub successful_sync_summary: Option<PlanSummary>,
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
    let listener = UnixListener::bind(socket_path)?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
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
    use tempfile::tempdir;

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
