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
}

#[cfg(unix)]
pub async fn bind_listener(socket_path: &Path) -> AppResult<UnixListener> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
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
}
