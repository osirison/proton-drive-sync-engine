//! Synchronous control-socket client with a mandatory client-side timeout.
//!
//! The daemon processes IPC on a single-threaded `select!` loop and deliberately does **not**
//! time-bound request *processing*, so a `status` poll issued while a reconcile is in flight can
//! block for the full duration of that reconcile. The daemon's own client library has no
//! client-side timeout either. This client therefore sets read/write timeouts itself and maps a
//! missing socket, a refused connection, or a timeout to [`IpcError::Unreachable`] — which the UI
//! must render as its own state, **never as zeroes**.

use crate::wire::{ControlCommand, ControlRequest, ControlResponse};
use std::path::Path;
use std::time::Duration;

/// Default client-side timeout for a control-socket round trip. Chosen above the daemon's 5 s
/// server-side I/O timeout so a slow-but-alive daemon is not misreported as unreachable, while a
/// truly stuck socket still fails fast enough for a 2 s poll cadence to recover.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);

/// Why a control-socket exchange failed.
#[derive(Debug)]
pub enum IpcError {
    /// The daemon could not be reached: the socket file is missing, the connection was refused,
    /// the peer closed early, or the request timed out. The UI must surface this as the
    /// "daemon unreachable" state (em-dash counters, empty ledger), not as zeroes.
    Unreachable(String),
    /// The daemon replied but the exchange could not be encoded/decoded — a protocol mismatch.
    /// Treated by [`crate::state::derive_state`] as unreachable, since the reply can't be trusted.
    Protocol(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Unreachable(m) => write!(f, "daemon unreachable: {m}"),
            IpcError::Protocol(m) => write!(f, "protocol error: {m}"),
        }
    }
}

impl std::error::Error for IpcError {}

/// Send one request and read one newline-delimited JSON reply, enforcing `timeout` on both the
/// write and the read.
#[cfg(unix)]
pub fn send_request(
    socket_path: &Path,
    request: &ControlRequest,
    timeout: Duration,
) -> Result<ControlResponse, IpcError> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(socket_path)
        .map_err(|e| IpcError::Unreachable(format!("connect {}: {e}", socket_path.display())))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| IpcError::Unreachable(e.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| IpcError::Unreachable(e.to_string()))?;

    let mut line = serde_json::to_string(request).map_err(|e| IpcError::Protocol(e.to_string()))?;
    line.push('\n');

    // `&UnixStream` implements both `Write` and `Read`, so one borrow covers the round trip.
    (&stream)
        .write_all(line.as_bytes())
        .map_err(|e| IpcError::Unreachable(format!("write: {e}")))?;
    (&stream)
        .flush()
        .map_err(|e| IpcError::Unreachable(format!("flush: {e}")))?;

    let mut reader = BufReader::new(&stream);
    let mut response_line = String::new();
    let n = reader
        .read_line(&mut response_line)
        .map_err(|e| IpcError::Unreachable(format!("read: {e}")))?;
    if n == 0 {
        return Err(IpcError::Unreachable(
            "daemon closed the connection without replying".into(),
        ));
    }
    serde_json::from_str(response_line.trim_end())
        .map_err(|e| IpcError::Protocol(format!("decode reply: {e}")))
}

/// Non-unix stub so the crate still type-checks off-unix (the daemon is unix-only in practice).
#[cfg(not(unix))]
pub fn send_request(
    _socket_path: &Path,
    _request: &ControlRequest,
    _timeout: Duration,
) -> Result<ControlResponse, IpcError> {
    Err(IpcError::Unreachable(
        "control socket is only supported on unix".into(),
    ))
}

/// The engine's own default control-socket path, delegated rather than reimplemented (#277).
///
/// The GUI used to keep a private copy that fell back to `<temp>/proton-sync.sock` when
/// `XDG_RUNTIME_DIR` was unset (SSH without `pam_systemd`, many containers), while the daemon
/// listened on the uid-namespaced `<temp>/proton-drive-sync-<uid>/proton-sync.sock` — so the GUI
/// rendered a healthy daemon as **unreachable**, and wrote a predictable name straight into
/// world-writable `/tmp`, the exact placement the engine's namespacing exists to avoid (#74).
/// Two places computing one path is how they drift; this is the one place.
///
/// Fallible for the engine's reason: that fallback **fails closed** rather than hand back a path
/// in attacker-plantable space. The caller must surface the failure as the unreachable state, not
/// substitute a guess.
pub fn default_socket_path() -> Result<std::path::PathBuf, String> {
    proton_drive_sync_engine::paths::default_socket_path()
        .map_err(|error| format!("cannot resolve the control socket path: {error}"))
}

/// Convenience wrapper for the argument-less commands (`status`, `pause`, `resume`, `syncnow`).
pub fn command(
    socket_path: &Path,
    command: ControlCommand,
    timeout: Duration,
) -> Result<ControlResponse, IpcError> {
    send_request(socket_path, &ControlRequest::new(command), timeout)
}

/// Convenience wrapper for `approve` / `deny`, which take a path (or `"all"`) argument.
/// `literal_path` mirrors [`ControlRequest::literal_path`]: pass `true` when the argument is a
/// row's actual path (so a file literally named `all` cannot be mistaken for the every-item
/// selector) and `false` for the explicit approve-all/deny-all form.
pub fn command_with_argument(
    socket_path: &Path,
    command: ControlCommand,
    argument: impl Into<String>,
    literal_path: bool,
    timeout: Duration,
) -> Result<ControlResponse, IpcError> {
    send_request(
        socket_path,
        &ControlRequest {
            argument: Some(argument.into()),
            literal_path,
            ..ControlRequest::new(command)
        },
        timeout,
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    // A canned status reply matching `ControlResponse`, served by a one-shot fake daemon.
    const CANNED_REPLY: &str = r#"{"status":"running","paused":false,"pending_changes":3,"message":"sync completed","last_sync_epoch_secs":1750000000,"last_error":null,"last_plan_summary":null,"last_successful_sync_summary":null,"status_history":[],"pending_deletions":[]}"#;

    fn spawn_one_shot_daemon(reply: &'static str) -> (std::path::PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                let mut out = reply.to_string();
                out.push('\n');
                let _ = (&stream).write_all(out.as_bytes());
            }
        });
        (path, dir)
    }

    #[test]
    fn round_trips_a_status_request_and_parses_the_reply() {
        let (path, _dir) = spawn_one_shot_daemon(CANNED_REPLY);
        let resp = command(&path, ControlCommand::Status, DEFAULT_TIMEOUT).unwrap();
        assert_eq!(resp.status, "running");
        assert_eq!(resp.pending_changes, 3);
        assert_eq!(resp.message, "sync completed");
        assert_eq!(resp.last_sync_epoch_secs, Some(1_750_000_000));
    }

    /// #277: the GUI's private copy of this default fell back to `<temp>/proton-sync.sock` while
    /// the engine's is uid-namespaced, so with `XDG_RUNTIME_DIR` unset the GUI polled a path the
    /// daemon never bound and drew a healthy daemon as unreachable. That literal must never be
    /// producible here again — in EITHER branch (with the runtime dir set the answer is under it).
    #[test]
    fn the_default_socket_path_is_never_the_unnamespaced_temp_one_the_gui_used_to_build() {
        let resolved = default_socket_path().expect("a default resolves on a test host");
        assert_ne!(
            resolved,
            std::env::temp_dir().join("proton-sync.sock"),
            "a predictable name straight in the shared temp dir is the placement #74 exists to \
             avoid, and the path the daemon never binds"
        );
        assert_eq!(
            resolved,
            proton_drive_sync_engine::paths::default_socket_path().expect("engine default"),
            "the GUI must not compute this path itself — one copy, or they drift again"
        );
    }

    #[test]
    fn a_missing_socket_is_unreachable_not_an_error_value() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.sock");
        let err = command(&missing, ControlCommand::Status, DEFAULT_TIMEOUT).unwrap_err();
        assert!(matches!(err, IpcError::Unreachable(_)), "got {err:?}");
    }

    #[test]
    fn serializes_the_argument_for_approve() {
        // Serve back whatever we like; we only assert the request the daemon received.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen_writer = seen.clone();
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                *seen_writer.lock().unwrap() = request_line;
                let _ = (&stream).write_all(format!("{CANNED_REPLY}\n").as_bytes());
            }
        });
        let _ = command_with_argument(
            &path,
            ControlCommand::Approve,
            "all",
            false,
            DEFAULT_TIMEOUT,
        );
        let request = seen.lock().unwrap().clone();
        assert!(
            request.contains("\"command\":\"approve\""),
            "req was {request}"
        );
        assert!(
            request.contains("\"argument\":\"all\""),
            "req was {request}"
        );
    }
}
