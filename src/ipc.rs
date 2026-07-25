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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlResponse {
    pub status: String,
    pub paused: bool,
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
