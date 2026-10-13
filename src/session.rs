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

use crate::events::{HttpResponse, HttpTransport, SessionProvider, TransientFetchError};
use crate::{AppResult, boxed_error};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Keyring coordinates of the `proton-drive` CLI's persisted session. The CLI stores it via
/// Bun's secret API in the Secret Service under this service/account pair.
const KEYRING_SERVICE: &str = "ch.proton.drive/drive-sdk-cli";
const KEYRING_ACCOUNT: &str = "auth-session";

/// The arguments of `secret-tool` that look the CLI's session entry up. The one spelling of the
/// coordinates: [`read_keyring_session`] reads the secret with it and the `doctor` sign-in check
/// asks only whether the entry exists (stdout discarded).
pub(crate) fn keyring_lookup_args() -> [&'static str; 5] {
    [
        "lookup",
        "service",
        KEYRING_SERVICE,
        "account",
        KEYRING_ACCOUNT,
    ]
}

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
        .args(keyring_lookup_args())
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

/// How long curl may spend **making the connection** (name lookup, TCP connect and the TLS
/// handshake together), before `--max-time` counts the whole request. A healthy connection takes
/// well under a second, so ten seconds is slack, and a dead one — packets going nowhere — now costs
/// ten seconds of the daemon's one loop instead of the full request timeout. It does not shorten a
/// connection that was made and then stalls: that is still [`REQUEST_TIMEOUT_SECS`].
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// The whole-request timeout. Unchanged at thirty seconds: the journal records the timeout firing
/// but not which phase it fired in (curl ran silent, so its stderr was empty — it now runs with
/// `-S`), so it gives no evidence that a shorter limit is safe for a slow page that was going to
/// arrive.
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// What a curl exit code says about **whether asking again is expected to work** — `Some(what
/// happened)` for the codes that mean the request got no answer, `None` for every other code.
///
/// In, and why: a name that would not resolve (6) and a connection that would not open (7) are a
/// network or a resolver that is down right now; a handshake that failed (35) and a transfer cut
/// off — an empty reply (52), a failed send (55) or receive (56), a transfer closed early (18), an
/// HTTP/2 stream the peer reset (92) — are a connection that dropped; and a timeout (28) is the
/// request that got nothing back in time, the code the journal records most. None of them tells us
/// anything about the feed, so none is evidence that the cursor or the tree is wrong.
///
/// Out, and why: bad usage or build (1 unsupported protocol, 2 init failed, 3 malformed URL, 4, 53
/// and 54 missing features), which are the same on every attempt; a proxy that cannot be resolved
/// (5), a configuration state rather than a moment; a failed write to our own pipe (23) or read of
/// our own input (26), which are this machine; too many redirects (47), the server's answer; and
/// every certificate and cipher refusal (51, 58, 59, 60, 77, 82, 83, 90, 91), which stay until a
/// person changes something — they fall back to the walk, which the CLI can still do. A process
/// killed by a signal has no exit code at all and is not classified either.
fn transient_curl_exit(code: i32) -> Option<&'static str> {
    Some(match code {
        6 => "could not resolve host",
        7 => "could not connect",
        18 => "transfer closed early",
        28 => "timed out",
        35 => "TLS handshake failed",
        52 => "empty reply",
        55 => "failed sending",
        56 => "failed receiving",
        92 => "HTTP/2 stream reset",
        _ => return None,
    })
}

/// A dependency-free [`HttpTransport`] that performs a `GET` by shelling `curl`. Time-bounded
/// so a network hang cannot stall a caller (e.g. the daemon's reconcile loop).
///
/// A failure that says nothing about the feed — see [`transient_curl_exit`], and an HTTP status of
/// `000` — is returned as a [`TransientFetchError`]; any other failure is a plain error.
pub struct CurlHttpTransport {
    curl: PathBuf,
    timeout_secs: u64,
    connect_timeout_secs: u64,
}

impl CurlHttpTransport {
    /// A transport using `curl` on `PATH` with a 30s per-request timeout and a 10s connect
    /// timeout.
    pub fn new() -> Self {
        Self::with_curl("curl")
    }

    /// A transport using a specific `curl` executable (tests inject a fake here).
    pub fn with_curl(curl: impl Into<PathBuf>) -> Self {
        Self {
            curl: curl.into(),
            timeout_secs: REQUEST_TIMEOUT_SECS,
            connect_timeout_secs: CONNECT_TIMEOUT_SECS,
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
        // Headers carry the bearer token, so they must never appear on the argv (visible to
        // every local process via /proc/*/cmdline). `-H @-` (curl >= 7.55.0) reads the header
        // lines from stdin instead; only the URL and fixed flags stay on the command line.
        //
        // `-s` keeps the progress meter off and `-S` puts the error message back, so a failure
        // says which timeout fired ("Connection timed out", "Resolving timed out", "Operation
        // timed out after 30001 ms with 0 bytes received") instead of leaving the log with only an
        // exit code.
        let mut child = Command::new(&self.curl)
            .arg("-s")
            .arg("-S")
            .arg("--connect-timeout")
            .arg(self.connect_timeout_secs.to_string())
            .arg("--max-time")
            .arg(self.timeout_secs.to_string())
            .arg("-w")
            .arg("\n%{http_code}")
            .arg("-H")
            .arg("@-")
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| boxed_error(format!("failed to run curl: {error}")))?;
        {
            let mut stdin = child.stdin.take().expect("curl stdin is piped");
            let mut header_lines = String::new();
            for (key, value) in headers {
                header_lines.push_str(key);
                header_lines.push_str(": ");
                header_lines.push_str(value);
                header_lines.push('\n');
            }
            if let Err(error) = stdin.write_all(header_lines.as_bytes()) {
                // A broken pipe means curl exited before reading its stdin (e.g. a bad URL);
                // fall through so its exit status / stderr is reported as the real failure.
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(boxed_error(format!(
                        "failed to write curl headers: {error}"
                    )));
                }
            }
            // Dropping the handle closes stdin so curl sees EOF on the header stream.
        }
        let output = child
            .wait_with_output()
            .map_err(|error| boxed_error(format!("failed to run curl: {error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = format!(
                "curl failed ({status}): {stderr}",
                status = output.status,
                stderr = stderr.trim(),
            );
            // A killed process has no exit code and is not classified.
            let transient = output
                .status
                .code()
                .and_then(|code| transient_curl_exit(code).map(|what| (code, what)));
            return Err(match transient {
                Some((code, what)) => Box::new(TransientFetchError::new(
                    format!("curl exit {code} ({what})"),
                    detail,
                )),
                None => boxed_error(detail),
            });
        }
        let combined = String::from_utf8_lossy(&output.stdout);
        // `-w '\n%{http_code}'` appends the status on its own trailing line.
        let (body, status) = combined
            .rsplit_once('\n')
            .ok_or_else(|| boxed_error("curl produced no status line"))?;
        let status = status.trim().parse::<u16>().map_err(|error| {
            boxed_error(format!("curl returned an unparseable status: {error}"))
        })?;
        if status == 0 {
            // curl writes `000` when no HTTP response was received; that is a transport
            // failure, not a response the caller should interpret — and, like a timeout, says
            // nothing about the feed.
            return Err(Box::new(TransientFetchError::new(
                "no response received",
                "curl reported HTTP status 000 (no response received); treating as a \
                 transport failure",
            )));
        }
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
    fn the_keyring_lookup_arguments_are_pinned() {
        assert_eq!(
            keyring_lookup_args(),
            [
                "lookup",
                "service",
                "ch.proton.drive/drive-sdk-cli",
                "account",
                "auth-session"
            ]
        );
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_session_secret("not json").is_err());
    }

    #[cfg(unix)]
    fn write_script(directory: &Path, name: &str, content: &str) -> PathBuf {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join(name);
        fs::write(&path, content).expect("write fake curl");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }

    /// Executing a just-written script can race a concurrent test's `fork` window (the child
    /// briefly inherits the write fd before `exec` closes it) and fail with `ETXTBSY`; retry
    /// briefly — the window closes as soon as the sibling execs.
    #[cfg(unix)]
    fn get_via_fake_curl(
        transport: &CurlHttpTransport,
        url: &str,
        headers: &[(String, String)],
    ) -> AppResult<HttpResponse> {
        for _ in 0..50 {
            match transport.get(url, headers) {
                Err(error) if error.to_string().contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                outcome => return outcome,
            }
        }
        transport.get(url, headers)
    }

    /// Issue #55 regression guard: the Authorization header (bearer token) must travel over
    /// curl's stdin (`-H @-`), never on the argv where /proc/*/cmdline exposes it.
    #[cfg(unix)]
    #[test]
    fn headers_are_fed_via_stdin_and_never_appear_on_the_argv() {
        let directory = tempfile::tempdir().expect("tempdir");
        let argv_capture = directory.path().join("argv.txt");
        let stdin_capture = directory.path().join("stdin.txt");
        let script = write_script(
            directory.path(),
            "fake-curl",
            &format!(
                r#"#!/bin/sh
printf '%s\n' "$@" > "{argv}"
cat > "{stdin}"
printf 'response-body\n200'
"#,
                argv = argv_capture.display(),
                stdin = stdin_capture.display(),
            ),
        );
        let transport = CurlHttpTransport::with_curl(&script);
        let headers = vec![
            ("x-pm-uid".to_owned(), "uid-1".to_owned()),
            ("Authorization".to_owned(), "Bearer secret-token".to_owned()),
        ];

        let response = get_via_fake_curl(&transport, "https://example.invalid/events", &headers)
            .expect("fake curl get");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "response-body");

        let stdin_seen = std::fs::read_to_string(&stdin_capture).expect("stdin capture");
        assert!(
            stdin_seen.contains("Authorization: Bearer secret-token"),
            "auth header must arrive via stdin, got: {stdin_seen}"
        );
        assert!(
            stdin_seen.contains("x-pm-uid: uid-1"),
            "all headers must arrive via stdin, got: {stdin_seen}"
        );

        let argv_seen = std::fs::read_to_string(&argv_capture).expect("argv capture");
        assert!(
            !argv_seen.contains("secret-token"),
            "the bearer token must never be on the argv, got: {argv_seen}"
        );
        assert!(
            argv_seen.lines().any(|line| line == "@-"),
            "curl must be told to read headers from stdin, got: {argv_seen}"
        );
        assert!(
            argv_seen
                .lines()
                .any(|line| line == "https://example.invalid/events"),
            "the URL stays on the argv, got: {argv_seen}"
        );
    }

    /// Issue #58 regression guard: a nonzero curl exit is a transport error, not a response.
    #[cfg(unix)]
    #[test]
    fn a_nonzero_curl_exit_is_reported_with_its_code_and_stderr() {
        let directory = tempfile::tempdir().expect("tempdir");
        let script = write_script(
            directory.path(),
            "fake-curl",
            r#"#!/bin/sh
cat > /dev/null
echo 'simulated: could not resolve host' >&2
exit 7
"#,
        );
        let transport = CurlHttpTransport::with_curl(&script);

        let error = get_via_fake_curl(&transport, "https://example.invalid/events", &[])
            .expect_err("nonzero curl exit must error");
        let message = error.to_string();
        assert!(
            message.contains('7'),
            "error must mention curl's exit code, got: {message}"
        );
        assert!(
            message.contains("could not resolve host"),
            "error must include curl's stderr, got: {message}"
        );
    }

    /// curl is asked for a connect timeout beside the whole-request one, and for its error message
    /// (`-S`): a dead connection must cost ten seconds of the daemon's loop rather than thirty, and
    /// the journal's "curl failed (exit status: 28):" must say which timeout it was.
    #[cfg(unix)]
    #[test]
    fn curl_gets_a_connect_timeout_a_request_timeout_and_its_error_message() {
        let directory = tempfile::tempdir().expect("tempdir");
        let argv_capture = directory.path().join("argv.txt");
        let script = write_script(
            directory.path(),
            "fake-curl",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{argv}\"\ncat > /dev/null\nprintf 'b\\n200'\n",
                argv = argv_capture.display(),
            ),
        );
        let transport = CurlHttpTransport::with_curl(&script);

        get_via_fake_curl(&transport, "https://example.invalid/events", &[]).expect("get");

        let argv: Vec<String> = std::fs::read_to_string(&argv_capture)
            .expect("argv capture")
            .lines()
            .map(str::to_owned)
            .collect();
        let value_after = |flag: &str| {
            let at = argv.iter().position(|arg| arg == flag);
            at.and_then(|at| argv.get(at + 1)).cloned()
        };
        assert_eq!(value_after("--connect-timeout").as_deref(), Some("10"));
        assert_eq!(value_after("--max-time").as_deref(), Some("30"));
        assert!(argv.iter().any(|arg| arg == "-S"), "{argv:?}");
        assert!(argv.iter().any(|arg| arg == "-s"), "{argv:?}");
    }

    /// The set of curl exit codes that mean "no answer, ask again", pinned both ways: a code added
    /// to the set sends a failure the daemon used to answer with a full-tree walk to a wait instead,
    /// and a code taken out does the reverse.
    #[test]
    fn exactly_these_curl_exit_codes_are_transient() {
        let transient: Vec<i32> = (0..=255)
            .filter(|code| transient_curl_exit(*code).is_some())
            .collect();
        assert_eq!(transient, [6, 7, 18, 28, 35, 52, 55, 56, 92]);
        for code in [
            0, 1, 2, 3, 4, 5, 22, 23, 26, 47, 51, 53, 54, 58, 59, 60, 77, 82, 83, 90, 91, 126, 127,
        ] {
            assert_eq!(transient_curl_exit(code), None, "curl exit {code}");
        }
    }

    /// Through the real `get`: each exit code in the set is a typed transient error whose message is
    /// what it was before the type existed; any other code is a plain error.
    #[cfg(unix)]
    #[test]
    fn a_curl_exit_in_the_set_is_typed_transient_and_any_other_is_not() {
        use crate::events::transient_fetch_cause;

        let directory = tempfile::tempdir().expect("tempdir");
        for (code, transient) in [
            (6, true),
            (7, true),
            (18, true),
            (28, true),
            (35, true),
            (52, true),
            (55, true),
            (56, true),
            (92, true),
            (1, false),
            (3, false),
            (5, false),
            (22, false),
            (47, false),
            (60, false),
            (127, false),
        ] {
            let script = write_script(
                directory.path(),
                &format!("fake-curl-{code}"),
                &format!(
                    "#!/bin/sh\ncat > /dev/null\necho 'curl: ({code}) stand-in' >&2\nexit {code}\n"
                ),
            );
            let transport = CurlHttpTransport::with_curl(&script);
            let error = get_via_fake_curl(&transport, "https://example.invalid/events", &[])
                .expect_err("a nonzero exit must error");
            assert_eq!(
                transient_fetch_cause(error.as_ref()).is_some(),
                transient,
                "curl exit {code}: {error}"
            );
            assert!(
                error.to_string().starts_with(&format!(
                    "curl failed (exit status: {code}): curl: ({code})"
                )),
                "the message is the one the journal has always shown: {error}"
            );
            if transient {
                assert!(
                    transient_fetch_cause(error.as_ref())
                        .expect("typed")
                        .starts_with(&format!("curl exit {code} (")),
                    "{error}"
                );
            }
        }
    }

    /// A curl killed by a signal has no exit code; nothing can be said about it, so it stays a plain
    /// error and the daemon answers it as it always did.
    #[cfg(unix)]
    #[test]
    fn a_curl_killed_by_a_signal_is_not_classified() {
        let directory = tempfile::tempdir().expect("tempdir");
        let script = write_script(
            directory.path(),
            "fake-curl",
            "#!/bin/sh\ncat > /dev/null\nkill -KILL $$\n",
        );
        let transport = CurlHttpTransport::with_curl(&script);

        let error = get_via_fake_curl(&transport, "https://example.invalid/events", &[])
            .expect_err("a killed curl must error");

        assert!(
            !crate::events::is_transient_fetch_error(error.as_ref()),
            "{error}"
        );
    }

    /// Issue #58 regression guard: a `%{http_code}` of 000 means no response was received;
    /// it must surface as a transport error, never as `HttpResponse { status: 0 }`.
    #[cfg(unix)]
    #[test]
    fn an_http_code_of_zero_is_a_transport_error() {
        let directory = tempfile::tempdir().expect("tempdir");
        let script = write_script(
            directory.path(),
            "fake-curl",
            r#"#!/bin/sh
cat > /dev/null
printf '\n000'
"#,
        );
        let transport = CurlHttpTransport::with_curl(&script);

        let error = get_via_fake_curl(&transport, "https://example.invalid/events", &[])
            .expect_err("http_code 000 must error");
        let message = error.to_string();
        assert!(
            message.contains("000"),
            "error must mention the 000 status, got: {message}"
        );
        assert_eq!(
            crate::events::transient_fetch_cause(error.as_ref()),
            Some("no response received"),
            "no response at all says nothing about the feed: asking again is expected to work"
        );
    }
}
