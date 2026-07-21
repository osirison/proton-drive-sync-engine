//! Concrete, environment-facing implementations of the [`crate::events`] injectable seams for
//! the **current CLI-session-reuse approach** (ADR 0001): the engine borrows the logged-in
//! `proton-drive` CLI's Proton session instead of owning an independent one.
//!
//! This is the pragmatic, proven-working plumbing that unblocks wiring detection into the
//! daemon. Its two deliberate limitations follow from *not* owning the session:
//!
//! * **No independent refresh.** [`CliKeyringSession::refresh`] re-reads the CLI's (possibly
//!   rotated) token from the keyring rather than driving a token refresh itself — the CLI
//!   stays the session owner. If the CLI has not refreshed, the retry simply `401`s again and
//!   the caller skips that pass. The daemon is therefore only as fresh as the CLI keeps the
//!   session; a future independent (browser-forked) session provider will replace this behind
//!   the same [`SessionProvider`] trait.
//! * **Local desktop assumptions.** Reading the keyring shells `secret-tool` and needs the
//!   Secret Service reachable (`DBUS_SESSION_BUS_ADDRESS` set, keyring unlocked).
//!
//! [`CurlHttpTransport`] is an independent, swappable [`HttpTransport`] — a dependency-free
//! HTTP `GET` via `curl`, matching the crate's existing shell-out style — used with any
//! session provider, not just the reuse one.

use crate::events::{HttpResponse, HttpTransport, SessionProvider};
use crate::{AppResult, boxed_error};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Keyring coordinates of the `proton-drive` CLI's persisted session. The CLI stores it via
/// Bun's secret API in the Secret Service under this service/account pair.
const KEYRING_SERVICE: &str = "ch.proton.drive/drive-sdk-cli";
const KEYRING_ACCOUNT: &str = "auth-session";

/// The subset of the CLI's session entry that authenticating an events request needs. The
/// entry also holds key-unlock material (`userKeyPassword`/`cachePassword`); detection is
/// auth-only, so those are intentionally ignored here.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionTokens {
    uid: String,
    access_token: String,
}

/// A [`SessionProvider`] that reuses the `proton-drive` CLI's session from the OS keyring.
pub struct CliKeyringSession {
    secret_tool: PathBuf,
    tokens: Mutex<SessionTokens>,
}

impl CliKeyringSession {
    /// Reads the current CLI session from the keyring using `secret-tool` on `PATH`.
    pub fn from_cli_keyring() -> AppResult<Self> {
        Self::with_secret_tool("secret-tool")
    }

    /// Reads the current CLI session using a specific `secret-tool` executable.
    pub fn with_secret_tool(secret_tool: impl Into<PathBuf>) -> AppResult<Self> {
        let secret_tool = secret_tool.into();
        let tokens = read_keyring_session(&secret_tool)?;
        Ok(Self {
            secret_tool,
            tokens: Mutex::new(tokens),
        })
    }
}

impl SessionProvider for CliKeyringSession {
    fn auth_headers(&self) -> AppResult<Vec<(String, String)>> {
        let tokens = self.tokens.lock().expect("session tokens mutex");
        Ok(vec![
            ("x-pm-uid".to_owned(), tokens.uid.clone()),
            (
                "Authorization".to_owned(),
                format!("Bearer {}", tokens.access_token),
            ),
        ])
    }

    fn refresh(&self) -> AppResult<()> {
        // Reuse model: we do not own the session, so "refresh" means re-read the CLI's current
        // (possibly rotated) token rather than performing a token refresh ourselves.
        let fresh = read_keyring_session(&self.secret_tool)?;
        *self.tokens.lock().expect("session tokens mutex") = fresh;
        Ok(())
    }
}

fn read_keyring_session(secret_tool: &Path) -> AppResult<SessionTokens> {
    let output = Command::new(secret_tool)
        .args([
            "lookup",
            "service",
            KEYRING_SERVICE,
            "account",
            KEYRING_ACCOUNT,
        ])
        .output()
        .map_err(|error| boxed_error(format!("failed to run secret-tool: {error}")))?;
    if !output.status.success() {
        return Err(boxed_error(
            "secret-tool could not read the proton-drive CLI session; is the CLI logged in, the \
             desktop keyring unlocked, and DBUS_SESSION_BUS_ADDRESS set?",
        ));
    }
    let secret = String::from_utf8(output.stdout)
        .map_err(|error| boxed_error(format!("keyring session was not valid UTF-8: {error}")))?;
    parse_session_secret(secret.trim())
}

/// Extracts the session `uid` + `accessToken` from the CLI's stored secret JSON.
fn parse_session_secret(json: &str) -> AppResult<SessionTokens> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| boxed_error(format!("keyring session was not valid JSON: {error}")))?;
    let session = value
        .get("session")
        .ok_or_else(|| boxed_error("keyring session entry has no `session` object"))?;
    let uid = session
        .get("uid")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| boxed_error("keyring session is missing `session.uid`"))?
        .to_owned();
    let access_token = session
        .get("accessToken")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| boxed_error("keyring session is missing `session.accessToken`"))?
        .to_owned();
    Ok(SessionTokens { uid, access_token })
}

/// A dependency-free [`HttpTransport`] that performs a `GET` by shelling `curl`. Time-bounded
/// so a network hang cannot stall a caller (e.g. the daemon's reconcile loop).
pub struct CurlHttpTransport {
    curl: PathBuf,
    timeout_secs: u64,
}

impl CurlHttpTransport {
    /// A transport using `curl` on `PATH` with a 30s per-request timeout.
    pub fn new() -> Self {
        Self {
            curl: PathBuf::from("curl"),
            timeout_secs: 30,
        }
    }

    /// Overrides the per-request timeout (seconds).
    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs.max(1);
        self
    }
}

impl Default for CurlHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for CurlHttpTransport {
    fn get(&self, url: &str, headers: &[(String, String)]) -> AppResult<HttpResponse> {
        let mut command = Command::new(&self.curl);
        command
            .arg("-s")
            .arg("--max-time")
            .arg(self.timeout_secs.to_string())
            .arg("-w")
            .arg("\n%{http_code}");
        for (key, value) in headers {
            command.arg("-H").arg(format!("{key}: {value}"));
        }
        command.arg(url);
        let output = command
            .output()
            .map_err(|error| boxed_error(format!("failed to run curl: {error}")))?;
        let combined = String::from_utf8_lossy(&output.stdout);
        // `-w '\n%{http_code}'` appends the status on its own trailing line.
        let (body, status) = combined
            .rsplit_once('\n')
            .ok_or_else(|| boxed_error("curl produced no status line"))?;
        let status = status.trim().parse::<u16>().map_err(|error| {
            boxed_error(format!("curl returned an unparseable status: {error}"))
        })?;
        Ok(HttpResponse {
            status,
            body: body.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uid_and_access_token_from_the_session_secret() {
        let secret = r#"{
            "cachePassword": "ignored",
            "userKeyPassword": "ignored",
            "session": { "uid": "uid-123", "accessToken": "tok-456", "refreshToken": "r" },
            "telemetryEnabled": true
        }"#;
        let tokens = parse_session_secret(secret).expect("parse session secret");
        assert_eq!(tokens.uid, "uid-123");
        assert_eq!(tokens.access_token, "tok-456");
    }

    #[test]
    fn rejects_a_secret_missing_the_session_object() {
        let error = parse_session_secret(r#"{"telemetryEnabled":true}"#)
            .expect_err("missing session must error");
        assert!(error.to_string().contains("no `session` object"));
    }

    #[test]
    fn rejects_a_secret_missing_the_access_token() {
        let error = parse_session_secret(r#"{"session":{"uid":"u"}}"#)
            .expect_err("missing access token must error");
        assert!(error.to_string().contains("session.accessToken"));
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_session_secret("not json").is_err());
    }
}
