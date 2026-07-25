//! The GUI's derived daemon state — the single enum that drives the pill, arcs, headline,
//! sub-line, buttons, banner, stat values, ledger contents and footer (design §6).
//!
//! The daemon's own `status` string is only `"running"` or `"paused"`; everything else the UI
//! needs is *derived* here from the reply (or its absence), so the derivation lives in one place
//! and can't disagree across screens.

use crate::ipc::IpcError;
use crate::wire::ControlResponse;

/// The six reachable UI states (design §6). `Running` vs `Idle` is derived from `pending_changes`
/// (the daemon does not distinguish "syncing" from "up to date" in its status string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DaemonState {
    /// Reconciling / has outstanding work (`pending_changes > 0`). Design: "Syncing N of M".
    Running,
    /// Reachable, paused=false, nothing pending. Design: "Everything is up to date".
    Idle,
    /// The daemon reports `paused = true`.
    Paused,
    /// The last error looks like a Proton sign-in expiry (E6 workaround: pattern-match until the
    /// daemon classifies auth state itself). Design: "Proton sign-in expired".
    AuthExpired,
    /// The socket could not be reached, or the reply could not be trusted. Counters must render as
    /// em-dashes and the ledger as explicitly empty — never zeroes.
    Unreachable,
    /// Reachable but nothing has ever synced (no `last_sync`, empty history). Design: first run.
    FirstRun,
}

impl DaemonState {
    /// `true` when the UI must blank counters to em-dashes rather than show a value.
    pub fn counters_unknown(self) -> bool {
        matches!(self, DaemonState::Unreachable | DaemonState::FirstRun)
    }
}

/// Heuristic auth-expiry detector (E6 workaround). Deliberately conservative: it matches the
/// vocabulary Proton/HTTP auth failures actually use, and avoids broad tokens like bare "auth"
/// that appear in unrelated words. Replaced by daemon-side classification when E6 lands.
pub fn looks_like_auth_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "unauthor",    // unauthorized / unauthorised
        "authenticat", // authentication / failed to authenticate
        "authoriz",    // authorization
        "401",
        "sign in",
        "sign-in",
        "signed out",
        "session expired",
        "session has expired",
        "not logged in",
        "logged out",
        "re-authenticate",
        "reauthenticate",
        "expired token",
        "token expired",
        "invalid token",
        "invalid session",
        "credential",
    ];
    NEEDLES.iter().any(|needle| m.contains(needle))
}

/// Derive the UI state from a status round trip. Pass `Ok(&response)` on success or `Err(&error)`
/// when the socket call failed.
pub fn derive_state(reply: Result<&ControlResponse, &IpcError>) -> DaemonState {
    let response = match reply {
        // A protocol error means the daemon is there but its reply can't be trusted; fall back to
        // unreachable rather than rendering possibly-wrong numbers.
        Err(_) => return DaemonState::Unreachable,
        Ok(response) => response,
    };

    if response.paused {
        return DaemonState::Paused;
    }
    if let Some(error) = &response.last_error {
        if looks_like_auth_error(error) {
            return DaemonState::AuthExpired;
        }
    }
    if response.last_sync_epoch_secs.is_none() && response.status_history.is_empty() {
        return DaemonState::FirstRun;
    }
    if response.pending_changes > 0 {
        DaemonState::Running
    } else {
        DaemonState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::ControlResponse;

    fn response() -> ControlResponse {
        ControlResponse {
            status: "running".into(),
            paused: false,
            pending_changes: 0,
            message: String::new(),
            last_sync_epoch_secs: Some(1),
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: vec![],
            pending_deletions: vec![],
        }
    }

    #[test]
    fn unreachable_wins_over_everything() {
        let err = IpcError::Unreachable("no socket".into());
        assert_eq!(derive_state(Err(&err)), DaemonState::Unreachable);
        let err = IpcError::Protocol("bad json".into());
        assert_eq!(derive_state(Err(&err)), DaemonState::Unreachable);
    }

    #[test]
    fn paused_beats_pending_and_auth() {
        let mut r = response();
        r.paused = true;
        r.pending_changes = 9;
        r.last_error = Some("401 unauthorized".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::Paused);
    }

    #[test]
    fn auth_error_is_detected() {
        let mut r = response();
        r.last_error = Some("proton-drive: request failed: 401 Unauthorized".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::AuthExpired);
    }

    #[test]
    fn first_run_when_never_synced() {
        let mut r = response();
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        assert_eq!(derive_state(Ok(&r)), DaemonState::FirstRun);
    }

    #[test]
    fn running_vs_idle_from_pending_changes() {
        let mut r = response();
        r.pending_changes = 2;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Running);
        r.pending_changes = 0;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Idle);
    }

    #[test]
    fn auth_matcher_avoids_false_positives() {
        assert!(looks_like_auth_error("401 Unauthorized"));
        assert!(looks_like_auth_error(
            "Proton session expired, please sign in"
        ));
        assert!(!looks_like_auth_error("sync completed"));
        assert!(!looks_like_auth_error("uploaded 12 files"));
        assert!(!looks_like_auth_error("author.txt could not be read"));
    }
}
