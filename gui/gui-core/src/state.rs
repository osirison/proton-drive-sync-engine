//! The GUI's derived daemon state — the single enum that drives the pill, arcs, headline,
//! sub-line, buttons, banner, stat values, ledger contents and footer (design §6).
//!
//! The daemon's own `status` string is only `"running"` or `"paused"`; everything else the UI
//! needs is *derived* here from the reply (or its absence), so the derivation lives in one place
//! and can't disagree across screens.

use crate::ipc::IpcError;
use crate::wire::{AuthState, ControlResponse};

/// The seven reachable UI states (design §6). `Running` is primarily the daemon's own `syncing`
/// flag (a reconcile pass is in flight); `pending_changes > 0` is kept as a secondary signal so
/// replies from older daemons (whose `syncing` deserializes to `false`) still derive usefully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DaemonState {
    /// Reconciling / has outstanding work (`pending_changes > 0`). Design: "Syncing N of M".
    Running,
    /// Reachable, paused=false, nothing pending. Design: "Everything is up to date".
    Idle,
    /// The daemon reports `paused = true`.
    Paused,
    /// The Proton session is gone and the user has to sign in again. Design: "Proton sign-in
    /// expired". Reached from the daemon's own [`AuthState::SignedOut`] verdict (#103), or — only
    /// while that verdict is [`AuthState::Unknown`] — from [`looks_like_auth_error`].
    AuthExpired,
    /// The daemon is reachable and its last pass FAILED for some other reason — a remote list that
    /// timed out, a `proton-drive` binary that is not on `PATH`, a transfer that errored.
    ///
    /// This state exists because its absence was a false all-clear (#246): every branch below is a
    /// state the daemon is *in*, and a daemon whose last pass failed is in none of them, so it fell
    /// through to `Idle` and every surface drew `Everything is up to date` over a sync that did not
    /// happen. `reconcile_blocking` records the reason and the reply carries it; nothing read it.
    ///
    /// The reply itself is trustworthy — the counters are the daemon's own and are NOT blanked.
    Failed,
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

/// Heuristic auth-expiry detector. Deliberately conservative: it matches the vocabulary
/// Proton/HTTP auth failures actually use, and avoids broad tokens like bare "auth" that appear in
/// unrelated words.
///
/// **No longer the answer — the fallback for one state (#103/#311).** The daemon classifies the
/// CLI's stderr once, in `proton.rs`, and publishes an [`AuthState`] on every reply; this runs only
/// when that verdict is [`AuthState::Unknown`], which a reply from a daemon predating #103
/// deserializes to. It is kept rather than deleted for the same reason `transfers_remaining: None`
/// means "older daemon" rather than "nothing left": this app has no version floor against the
/// daemon it talks to, so the state a missing field lands in must still be readable.
///
/// It is a **matcher over a sentence**, so it is wrong in both directions and neither is
/// hypothetical: `last_error` is written by every failing pass, and a message quoting a filename
/// like `credentials.txt` trips it, while an auth failure phrased any other way does not. That is
/// precisely why it must not run once the daemon has said something — see [`derive_state`].
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
    // THE DAEMON'S VERDICT, AND THE NEEDLE LIST ONLY WHERE IT HAS NONE (#103/#311).
    //
    // Three states, and the third is not a synonym for either other one (`ipc::AuthState`), so it
    // is matched exhaustively rather than left to a fall-through — a trailing arm meaning "fine"
    // is #246's shape, and this is the field a fall-through would be worst on:
    //
    //   · `SignedOut` is the verdict, and it is read WITHOUT consulting `last_error`. The `list`
    //     verb is a second writer, so an expired session is published the moment an interactive
    //     request hits it — before any pass has failed and while `last_error` is still `None`.
    //     That case is invisible to the needle list by construction.
    //   · `SignedIn` SUPPRESSES the needle list rather than merely outranking it. Something reached
    //     Proton successfully, so an auth-shaped `last_error` — which outlives its pass, being
    //     cleared only by a success — is a failure of some other kind, and this is the false
    //     positive daemon-side classification exists to remove. It falls through to `Failed` below.
    //   · `Unknown` is no verdict at all (a daemon older than #103, or one whose only failures were
    //     of another kind), so the pre-#103 heuristic answers, exactly as it did before.
    match response.auth {
        AuthState::SignedOut => return DaemonState::AuthExpired,
        AuthState::SignedIn => {}
        AuthState::Unknown => {
            if let Some(error) = &response.last_error
                && looks_like_auth_error(error)
            {
                return DaemonState::AuthExpired;
            }
        }
    }
    if response.syncing {
        return DaemonState::Running;
    }
    if response.last_sync_epoch_secs.is_none() && response.status_history.is_empty() {
        return DaemonState::FirstRun;
    }
    // AFTER `syncing` and `FirstRun`, BEFORE the queue and the settled fall-through, and every one
    // of those three placements is load-bearing (#246):
    //
    //   · after `syncing`, because a retry already in flight is the newer fact — `last_error` is
    //     only cleared when a pass SUCCEEDS (`reconcile_blocking`), so it outlives the failure it
    //     describes and would otherwise pin a working daemon to its last bad pass;
    //   · after `FirstRun`, so a machine that has never synced still gets the onboarding takeover.
    //     Reachable in practice only if the history sidecar is missing, since `record_status_history`
    //     runs on both arms of a pass — but the wizard is the better answer when both could apply;
    //   · before `pending_changes`, because a failure with a queue behind it is still a failure. The
    //     queue is why it matters, not a reason to call it `Running`.
    if response.last_error.is_some() {
        return DaemonState::Failed;
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
            syncing: false,
            reconcile_seq: 0,
            pending_changes: 0,
            message: String::new(),
            last_sync_epoch_secs: Some(1),
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: vec![],
            pending_deletions: vec![],
            failed_items: vec![],
            failed_item_count: 0,
            config: None,
            activity: None,
            unsyncable: vec![],
            history: None,
            file_history: None,
            index_totals: None,
            listing: None,
            plan: None,
            apply: None,
            auth: Default::default(),
        }
    }

    /// EVERY variant of [`IpcError`], including #335's new `NotListening`. The finer split exists so
    /// the restart can *decide* on the daemon's presence; what this screen *draws* must not move
    /// with it, because a reply that could not be trusted is still no numbers to show.
    #[test]
    fn unreachable_wins_over_everything() {
        let err = IpcError::Unreachable("timed out".into());
        assert_eq!(derive_state(Err(&err)), DaemonState::Unreachable);
        let err = IpcError::Protocol("bad json".into());
        assert_eq!(derive_state(Err(&err)), DaemonState::Unreachable);
        let err = IpcError::NotListening("no socket".into());
        assert_eq!(derive_state(Err(&err)), DaemonState::Unreachable);
    }

    #[test]
    fn paused_beats_pending_and_auth() {
        let mut r = response();
        r.paused = true;
        r.pending_changes = 9;
        r.last_error = Some("401 unauthorized".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::Paused);
        // And beats the daemon's own verdict, not just the heuristic: `paused` is a thing the user
        // did and the only state with a `Resume` in it.
        r.last_error = None;
        r.auth = AuthState::SignedOut;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Paused);
    }

    #[test]
    fn auth_error_is_detected() {
        // `response()` leaves `auth` at its default `Unknown`, which is the ONE state the needle
        // list still answers in (#311) — a daemon predating #103 sends no field at all.
        let mut r = response();
        assert_eq!(r.auth, AuthState::Unknown);
        r.last_error = Some("proton-drive: request failed: 401 Unauthorized".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::AuthExpired);
    }

    #[test]
    fn a_signed_out_verdict_needs_no_failed_pass_behind_it() {
        // #103/#311, and the case the needle list CANNOT see: `auth` has two writers, and the
        // `list` verb is the one an expired session hits first. It publishes `signed-out` without
        // any pass having failed, so `last_error` is still `None` and there is no sentence to
        // match. Before this the GUI drew `Everything is up to date` over a signed-out daemon
        // until the next scheduled pass happened to fail.
        let mut r = response();
        r.auth = AuthState::SignedOut;
        r.last_error = None;
        assert_eq!(derive_state(Ok(&r)), DaemonState::AuthExpired);
    }

    #[test]
    fn a_signed_in_verdict_suppresses_the_needle_list_rather_than_outranking_it() {
        // The false positive the daemon's classification exists to remove. `last_error` is cleared
        // only by a SUCCESSFUL pass, so it outlives its failure; a message merely SHAPED like an
        // auth error, on a daemon that has since reached Proton, is a failure of some other kind.
        // It must land on `Failed` — the honest state — and not on the sign-in takeover, whose
        // whole content is a button that fixes nothing here.
        let mut r = response();
        r.auth = AuthState::SignedIn;
        r.last_error = Some("could not read credentials.txt: permission denied".into());
        assert!(
            looks_like_auth_error(r.last_error.as_deref().unwrap()),
            "the point of this test is a message the heuristic DOES match"
        );
        assert_eq!(derive_state(Ok(&r)), DaemonState::Failed);
    }

    #[test]
    fn a_signed_in_daemon_with_nothing_wrong_is_still_idle() {
        // The other half of suppression: skipping the needle list must not skip everything after
        // it. A healthy signed-in daemon keeps deriving exactly as before.
        let mut r = response();
        r.auth = AuthState::SignedIn;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Idle);
        r.pending_changes = 2;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Running);
    }

    #[test]
    fn an_unknown_verdict_is_not_a_verdict_in_either_direction() {
        // `Unknown` means the daemon has learned nothing — not signed in, and not a problem. With
        // no error to match it must derive as if the field were not there at all.
        let mut r = response();
        r.auth = AuthState::Unknown;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Idle);
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        assert_eq!(derive_state(Ok(&r)), DaemonState::FirstRun);
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
    fn syncing_flag_means_running_even_with_no_pending_changes() {
        // A download-only pass has pending_changes == 0; the daemon's own `syncing` flag is what
        // marks it as actively reconciling.
        let mut r = response();
        r.syncing = true;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Running);

        // And it wins over first-run emptiness: the first-ever startup sync shows as syncing,
        // not "nothing has synced yet".
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        assert_eq!(derive_state(Ok(&r)), DaemonState::Running);
    }

    #[test]
    fn a_failed_pass_is_not_idle() {
        // #246. The bug: every branch is a state the daemon is IN, a failed pass is none of them,
        // and the fall-through drew `Everything is up to date` over it.
        let mut r = response();
        r.last_error =
            Some("proton-drive list failed: No such file or directory (os error 2)".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::Failed);
        assert_ne!(derive_state(Ok(&r)), DaemonState::Idle);
        // Counters stay KNOWN, unlike unreachable and first-run: the reply is the daemon's own.
        assert!(!DaemonState::Failed.counters_unknown());
    }

    #[test]
    fn a_partial_pass_is_not_idle_either() {
        // #136 adds a THIRD pass outcome: most of the plan landed, some items failed. The GUI has
        // no drawn state for it, so it must land on the nearest honest one — never on the
        // fall-through. The daemon makes that work by setting `last_error` on a partial pass too;
        // this pins that the mapping holds, because a partial pass reaching `Idle` would draw
        // `Everything is up to date` over failed items.
        let mut r = response();
        r.failed_item_count = 3;
        r.last_error = Some("3 item(s) failed to sync (first: docs/a.txt)".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::Failed);
        assert_ne!(derive_state(Ok(&r)), DaemonState::Idle);
    }

    #[test]
    fn a_retry_in_flight_outranks_the_failure_it_is_retrying() {
        // `last_error` is cleared only by a SUCCESSFUL pass, so it is still set while the next one
        // runs. Reading it there would pin a working daemon to its last bad pass.
        let mut r = response();
        r.last_error = Some("remote list timed out".into());
        r.syncing = true;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Running);
    }

    #[test]
    fn a_queue_behind_a_failure_does_not_make_it_running() {
        // The other order — `pending_changes` first — reads a failed pass with work waiting as
        // `Syncing 4 changes`, which is the same false all-clear one word further on.
        let mut r = response();
        r.last_error = Some("upload failed: disk quota exceeded".into());
        r.pending_changes = 4;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Failed);
    }

    #[test]
    fn paused_and_auth_still_outrank_a_failure() {
        let mut r = response();
        r.last_error = Some("remote list timed out".into());
        r.paused = true;
        assert_eq!(derive_state(Ok(&r)), DaemonState::Paused);
        r.paused = false;
        // An auth-shaped failure is a failure too; the specific state wins because it has a
        // specific sentence and a specific menu.
        r.last_error = Some("401 Unauthorized".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::AuthExpired);
    }

    #[test]
    fn a_machine_that_has_never_synced_still_reaches_the_wizard() {
        // `FirstRun` is checked first so the onboarding takeover survives a daemon that has both
        // never synced and just failed.
        let mut r = response();
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        r.last_error = Some("proton-drive: command not found".into());
        assert_eq!(derive_state(Ok(&r)), DaemonState::FirstRun);
    }

    #[test]
    fn the_wire_name_is_what_the_webview_switches_on() {
        // The webview keys on this string in five places (the two tray tables, `chipFor`,
        // `heroStateOf`, the onboarding latch) and `trayMenu` THROWS on a key it does not know, so
        // the serialized name is an interface and not an implementation detail. `tray-view.test.js`
        // derives its state list from this enum by lowering the first letter, which is only correct
        // while `rename_all = "camelCase"` agrees — this is where the two meet.
        let name = |state: DaemonState| serde_json::to_string(&state).expect("serializes");
        assert_eq!(name(DaemonState::Failed), "\"failed\"");
        assert_eq!(name(DaemonState::AuthExpired), "\"authExpired\"");
        assert_eq!(name(DaemonState::FirstRun), "\"firstRun\"");
        assert_eq!(name(DaemonState::Idle), "\"idle\"");
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
