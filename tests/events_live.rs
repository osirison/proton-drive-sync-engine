//! Live end-to-end check for the detection core (`proton_drive_sync_engine::events`) against
//! the real Proton Drive volume events API.
//!
//! `#[ignore]` and read-only. It drives the actual [`EventsClient`] through two **temporary
//! harness** implementations — a curl-backed [`HttpTransport`] and a session provider that
//! *reuses the logged-in CLI's session* from the OS keyring. Neither is the production design
//! (the real client owns an independent forked session and a native HTTP transport); they
//! exist only to exercise the client end-to-end today.
//!
//! Run (needs the `proton-drive` CLI logged in, the desktop keyring unlocked, and the volume
//! id, e.g. from `~/.local/share/proton-drive-cli/events.json`):
//!
//! ```bash
//! PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
//!   cargo test --test events_live -- --ignored --nocapture
//! ```
#![cfg(unix)]

use proton_drive_sync_engine::AppResult;
use proton_drive_sync_engine::boxed_error;
use proton_drive_sync_engine::events::{
    EventsClient, HttpResponse, HttpTransport, SessionProvider,
};
use std::process::Command;

/// Reused CLI session (harness only). Cannot refresh — a reused access token is owned by the
/// CLI, so `refresh` deliberately fails rather than fighting the CLI over token rotation.
struct ReusedCliSession {
    uid: String,
    access_token: String,
}

impl ReusedCliSession {
    fn from_cli_keyring() -> AppResult<Self> {
        let output = Command::new("secret-tool")
            .args([
                "lookup",
                "service",
                "ch.proton.drive/drive-sdk-cli",
                "account",
                "auth-session",
            ])
            .output()?;
        if !output.status.success() {
            return Err(boxed_error(
                "secret-tool could not read the CLI session (is the desktop keyring unlocked \
                 and DBUS_SESSION_BUS_ADDRESS set?)",
            ));
        }
        let secret = String::from_utf8(output.stdout)?;
        let value: serde_json::Value = serde_json::from_str(secret.trim())?;
        let uid = value["session"]["uid"]
            .as_str()
            .ok_or_else(|| boxed_error("session.uid missing from keyring entry"))?
            .to_owned();
        let access_token = value["session"]["accessToken"]
            .as_str()
            .ok_or_else(|| boxed_error("session.accessToken missing from keyring entry"))?
            .to_owned();
        Ok(Self { uid, access_token })
    }
}

impl SessionProvider for ReusedCliSession {
    fn auth_headers(&self) -> AppResult<Vec<(String, String)>> {
        Ok(vec![
            ("x-pm-uid".to_owned(), self.uid.clone()),
            (
                "Authorization".to_owned(),
                format!("Bearer {}", self.access_token),
            ),
        ])
    }

    fn refresh(&self) -> AppResult<()> {
        Err(boxed_error(
            "the reuse harness cannot refresh a token it does not own",
        ))
    }
}

/// Dep-free transport that shells `curl`, matching the crate's existing shell-out style.
struct CurlTransport;

impl HttpTransport for CurlTransport {
    fn get(&self, url: &str, headers: &[(String, String)]) -> AppResult<HttpResponse> {
        let mut command = Command::new("curl");
        command
            .arg("-s")
            .arg("--max-time")
            .arg("30")
            .arg("-w")
            .arg("\n%{http_code}");
        for (key, value) in headers {
            command.arg("-H").arg(format!("{key}: {value}"));
        }
        command.arg(url);
        let output = command.output()?;
        let combined = String::from_utf8_lossy(&output.stdout);
        let (body, status) = combined
            .rsplit_once('\n')
            .ok_or_else(|| boxed_error("curl produced no status line"))?;
        Ok(HttpResponse {
            status: status.trim().parse()?,
            body: body.to_owned(),
        })
    }
}

#[test]
#[ignore = "requires a logged-in proton-drive CLI, an unlocked keyring, and PROTON_SYNC_EVENTS_VOLUME"]
fn live_detection_core_reads_the_volume_event_stream() {
    let volume = std::env::var("PROTON_SYNC_EVENTS_VOLUME")
        .expect("set PROTON_SYNC_EVENTS_VOLUME to the drive volume id");
    let session = ReusedCliSession::from_cli_keyring().expect("read reused CLI session");
    let client = EventsClient::new(CurlTransport, session, "cli-drive@0.5.0");

    // 1. The bootstrap call: fetch the current cursor.
    let cursor = client
        .latest_cursor(&volume)
        .expect("latest_cursor should authenticate and return a cursor");
    assert!(!cursor.is_empty(), "cursor must be non-empty");

    // 2. Fetch the delta since that cursor. A just-fetched cursor usually yields zero changes,
    //    but the response must still parse into a valid page whose cursor is present.
    let page = client
        .events_since(&volume, &cursor)
        .expect("events_since should return a parseable page");
    assert!(
        !page.latest_event_id.is_empty(),
        "the page must carry the next cursor"
    );

    eprintln!(
        "live events OK: changes={} more={} refresh={} next_cursor={}",
        page.changes.len(),
        page.more,
        page.refresh,
        page.latest_event_id
    );
}
