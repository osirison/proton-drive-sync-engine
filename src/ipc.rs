use crate::AppResult;
use crate::sync::PlanSummary;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ControlCommand {
    Status,
    Pause,
    Resume,
    Syncnow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlRequest {
    pub command: ControlCommand,
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

#[cfg(unix)]
pub async fn send_command(
    socket_path: &Path,
    command: ControlCommand,
) -> AppResult<ControlResponse> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let request = serde_json::to_vec(&ControlRequest { command })?;
    stream.write_all(&request).await?;
    stream.write_all(b"\n").await?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;
    Ok(serde_json::from_str(response.trim())?)
}

#[cfg(unix)]
pub async fn read_request(stream: UnixStream) -> AppResult<(ControlRequest, UnixStream)> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let request = serde_json::from_str(line.trim())?;
    Ok((request, reader.into_inner()))
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
}
