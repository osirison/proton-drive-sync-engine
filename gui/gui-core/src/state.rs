//! The GUI's derived daemon state — the single enum that drives the pill, arcs, headline,
//! sub-line, buttons, banner, stat values, ledger contents and footer (design §6).
//!
//! The daemon's own `status` string is only `"running"` or `"paused"`; everything else the UI
//! needs is *derived* here from the reply (or its absence), so the derivation lives in one place
//! and can't disagree across screens.

use crate::ipc::IpcError;
use crate::wire::{AuthState, ControlResponse, PairSummary};

/// The eight reachable UI states (design §6). `Running` is primarily the daemon's own `syncing`
/// flag (a reconcile pass is in flight); `pending_changes > 0` is kept as a secondary signal so
/// replies from older daemons (whose `syncing` deserializes to `false`) still derive usefully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DaemonState {
    /// Reconciling / has outstanding work (`pending_changes > 0`). Design: "Syncing N of M".
    Running,
    /// Reachable, paused=false, nothing pending, and a pass has finished for this folder. Design:
    /// "Everything is up to date". **Never for a folder with no finished pass** — see `Queued`.
    Idle,
    /// Two folders or more, and this one has not finished a pass since the daemon started (a daemon
    /// starts every folder with no last sync; only a finished pass sets it), is not running one, and
    /// is not paused or failed. Its turn is coming: passes are serialized, so either another folder's
    /// pass is running (`PairState::waiting_for` names it) or the daemon has not reached this one
    /// yet. The daemon starts it by itself when it does — every folder is due at start-up, and one
    /// resumed from a pause is reached at its next turn — so waiting never needs the person.
    ///
    /// This is the state `Idle` used to be for it: a summary has no history, so a folder waiting
    /// behind a 25 minute pass read `up to date` beside one that had never copied a file.
    Queued,
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
    /// **Only for the one folder** (or a daemon that lists none): it is the first-folder wizard's
    /// state, and with two folders or more the same facts are `Queued`.
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

/// Everything [`derive`] reads, as plain values — the one input both ways of knowing a pair's state
/// are reduced to (#102 phase 5a-2, brief section 4.3).
///
/// There are two ways to know a pair. A **full reply** (`ControlResponse`) describes the pair it is
/// about, and carries its status history. A **summary** (`PairSummary`, one entry of the reply's
/// `pairs`) is all the app knows about every *other* pair, and it carries no history. Two functions
/// deriving a state from those two would be sixty lines of ordered rules, each paid for by a bug
/// (#103, #246, #311), written twice — so there is one [`derive`] and two adapters
/// ([`PairFacts::from`] a reply, [`facts_of`] a summary) that fill this in.
#[derive(Debug, Clone, Copy)]
pub struct PairFacts<'a> {
    pub paused: bool,
    pub syncing: bool,
    pub last_error: Option<&'a str>,
    pub pending_changes: usize,
    pub last_sync: Option<u64>,
    /// Whether the pair's status history is empty — **`None` is "not known"**, which is what a
    /// summary has. `FirstRun` is entered only on `Some(true)`, so a pair known by its summary alone
    /// can never be called first-run: the wrong answer there would be a wizard (or "nothing synced
    /// yet") drawn over an established pair after a daemon restart, when `last_sync` is also `None`
    /// until the next pass succeeds. The safe direction is to draw `Idle`/`Running`/`Failed` for it.
    pub history_empty: Option<bool>,
    /// Daemon-wide: the session is per user, so every pair's facts carry the one verdict.
    pub auth: AuthState,
    /// What the other folders are doing. The one fact about the rest of the app a folder's state
    /// depends on: whether it has had its turn only means something when there are other folders
    /// to take it.
    pub peers: Peers<'a>,
}

/// What the folders other than this one are doing, as the reply that lists them says.
///
/// Passes are serialized (one gate, one pass at a time, ADR 0005 §5 — `ControlShared::active_pair`
/// in the engine reads at most one `syncing`), so a folder that has not had its turn is either
/// waiting for the one pass that is running or about to be popped; `Busy` names the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peers<'a> {
    /// No other folder: one folder, or a reply that lists none (a daemon older than the selector).
    /// State derives exactly as it did before folders.
    Alone,
    /// Other folders, and none of them is running a pass right now.
    Resting,
    /// Another folder is running a pass right now.
    Busy(&'a str),
}

/// The peers of the folder named `me` in `pairs`. `me` is `None` when the reply names no folder
/// (an unresolved selector), in which case nobody is excluded.
fn peers_of<'a>(pairs: &'a [PairSummary], me: Option<&str>) -> Peers<'a> {
    if pairs.len() < 2 {
        return Peers::Alone;
    }
    pairs
        .iter()
        .find(|pair| pair.syncing && Some(pair.name.as_str()) != me)
        .map_or(Peers::Resting, |pair| Peers::Busy(&pair.name))
}

impl<'a> PairFacts<'a> {
    /// The folder this one is waiting for: the one running a pass, when there is one.
    pub fn waiting_for(&self) -> Option<&'a str> {
        match self.peers {
            Peers::Busy(name) => Some(name),
            Peers::Alone | Peers::Resting => None,
        }
    }
}

impl<'a> From<&'a ControlResponse> for PairFacts<'a> {
    /// A full reply: history is known, so `FirstRun` is reachable (for a folder that is alone).
    fn from(response: &'a ControlResponse) -> Self {
        Self {
            paused: response.paused,
            syncing: response.syncing,
            last_error: response.last_error.as_deref(),
            pending_changes: response.pending_changes,
            last_sync: response.last_sync_epoch_secs,
            history_empty: Some(response.status_history.is_empty()),
            auth: response.auth,
            peers: peers_of(&response.pairs, response.pair.as_deref()),
        }
    }
}

/// A pair known only by its summary. `auth` is the daemon-wide verdict from the reply the summary
/// arrived in (a summary has none of its own), and `pairs` is that reply's list, which is where its
/// peers are read.
///
/// **`history_empty` is `None`, not `Some(true)`** (`a_summary_can_never_derive_first_run`), and
/// **`last_error` is carried** (`an_unavailable_pair_derives_failed_from_its_summary`): a pair whose
/// folder is missing publishes its reason there (phase 4b), and dropping it would draw an unplugged
/// drive as `Idle`.
pub fn facts_of<'a>(
    summary: &'a PairSummary,
    auth: AuthState,
    pairs: &'a [PairSummary],
) -> PairFacts<'a> {
    PairFacts {
        paused: summary.paused,
        syncing: summary.syncing,
        last_error: summary.last_error.as_deref(),
        pending_changes: summary.pending_changes,
        last_sync: summary.last_sync_epoch_secs,
        history_empty: None,
        auth,
        peers: peers_of(pairs, Some(summary.name.as_str())),
    }
}

/// One pair's derived state, by name — what the status payload carries so the webview, the tray and
/// the selector all read one answer per pair instead of deriving their own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PairState {
    pub name: String,
    pub state: DaemonState,
    /// How bad `state` is when the tray has to show ONE thing for several folders: [`severity`],
    /// carried so the webview can order folders worst-first and pick the worst without a rank table
    /// of its own (two places computing the same thing is how they come to disagree).
    pub rank: u8,
    /// For a `Queued` folder: the folder whose pass it is waiting for. Absent when nothing is running
    /// (the daemon has not reached it yet) and for every other state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
}

/// How bad a state is when folders disagree and the tray can show only one glyph — **higher is worse**
/// (#102 phase 5d, ADR 0005, brief section 4.3). Highest first:
///
/// | rank | state | why it sits there |
/// |---|---|---|
/// | 7 | `Unreachable` | the control socket: process-wide, there are no per-pair answers to rank |
/// | 6 | `AuthExpired` | the session is per user, so every unpaused pair says it at once |
/// | 5 | `Failed` | one folder's last pass failed (or its folder is gone); never hidden behind a healthy one |
/// | 4 | `FirstRun` | only for a folder that is alone, so never beside others |
/// | 3 | `Running` | something is moving; a folder the person paused does not outrank it |
/// | 2 | `Queued` | a folder that has not had its turn: below `Running`, because the pass that is moving is the news; above `Paused`, because it is about to move and a pause is the person's own doing. It wears the syncing glyph, as `Running` does (something is syncing or about to start), and ranks below every state that asks something of the person |
/// | 1 | `Paused` | above `Idle`, because an all-clear glyph over a folder that is not syncing is the lie to avoid; below `Queued`, because the person did it on purpose and the title names which |
/// | 0 | `Idle` | only when every folder is |
///
/// An exhaustive `match` with no `_` arm: a new `DaemonState` cannot be added without answering where
/// it ranks, which is the same guarantee `ConfigKey::scope` gives the engine.
pub fn severity(state: DaemonState) -> u8 {
    match state {
        DaemonState::Unreachable => 7,
        DaemonState::AuthExpired => 6,
        DaemonState::Failed => 5,
        DaemonState::FirstRun => 4,
        DaemonState::Running => 3,
        DaemonState::Queued => 2,
        DaemonState::Paused => 1,
        DaemonState::Idle => 0,
    }
}

/// The one state the tray's glyph shows for folders in `states` — the worst by [`severity`]. A reply
/// that lists no folders (a daemon older than the selector, or no reply at all) has nothing to rank,
/// and `described` — the state derived from the reply itself — stands.
pub fn aggregate_state(described: DaemonState, states: &[PairState]) -> DaemonState {
    states
        .iter()
        .map(|pair| pair.state)
        .max_by_key(|state| severity(*state))
        .unwrap_or(described)
}

/// The state of every pair a reply lists.
///
/// **The pair the reply is about gets `described`, the state derived from the full reply**: its
/// history is known, so it is the only one that can be `FirstRun` (and only when it is alone). Every
/// other pair is derived from its summary ([`facts_of`]), which is the safe direction (see
/// [`PairFacts::history_empty`]). A reply that lists no pairs (a daemon older than the selector)
/// lists none here.
pub fn pair_states(response: &ControlResponse, described: DaemonState) -> Vec<PairState> {
    response
        .pairs
        .iter()
        .map(|summary| {
            let facts = facts_of(summary, response.auth, &response.pairs);
            let state = if response.pair.as_deref() == Some(summary.name.as_str()) {
                described
            } else {
                derive(&facts)
            };
            PairState {
                name: summary.name.clone(),
                state,
                rank: severity(state),
                waiting_for: waiting_for_of(state, &facts),
            }
        })
        .collect()
}

/// The folder a pair in `state` is waiting for: set only for `Queued`, and only while another folder
/// is running a pass. `None` for a `Queued` folder means nothing is running yet — the daemon has not
/// reached it — which is a different sentence, not a missing one.
pub fn waiting_for_of(state: DaemonState, facts: &PairFacts<'_>) -> Option<String> {
    (state == DaemonState::Queued)
        .then(|| facts.waiting_for())
        .flatten()
        .map(str::to_owned)
}

/// [`waiting_for_of`] for the pair a full reply describes — what the status payload carries beside
/// its `state`.
pub fn described_waiting_for(response: &ControlResponse, state: DaemonState) -> Option<String> {
    waiting_for_of(state, &PairFacts::from(response))
}

/// Derive the UI state from a status round trip. Pass `Ok(&response)` on success or `Err(&error)`
/// when the socket call failed.
pub fn derive_state(reply: Result<&ControlResponse, &IpcError>) -> DaemonState {
    match reply {
        // A protocol error means the daemon is there but its reply can't be trusted; fall back to
        // unreachable rather than rendering possibly-wrong numbers.
        Err(_) => DaemonState::Unreachable,
        Ok(response) => derive(&PairFacts::from(response)),
    }
}

/// The ordered rules, over facts. **The only place a pair's state is decided.**
pub fn derive(facts: &PairFacts<'_>) -> DaemonState {
    if facts.paused {
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
    match facts.auth {
        AuthState::SignedOut => return DaemonState::AuthExpired,
        AuthState::SignedIn => {}
        AuthState::Unknown => {
            if let Some(error) = facts.last_error
                && looks_like_auth_error(error)
            {
                return DaemonState::AuthExpired;
            }
        }
    }
    if facts.syncing {
        return DaemonState::Running;
    }
    // `Some(true)` and no other value: see `PairFacts::history_empty`. And ONLY FOR A FOLDER THAT IS
    // ALONE: this is the first-folder wizard's state, and beside other folders the same facts are a
    // folder waiting for its turn (`Queued`, below). Before that it was the only way a never-synced
    // folder was not `Idle`, and it needed a full reply — so the folders the window was not about
    // read `up to date` while the one it was about read `nothing synced yet`.
    if facts.peers == Peers::Alone && facts.last_sync.is_none() && facts.history_empty == Some(true)
    {
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
    if facts.last_error.is_some() {
        return DaemonState::Failed;
    }
    // A FOLDER WITH NO FINISHED PASS IS NOT SETTLED, and not syncing either (#455 and the live report).
    // `last_sync` is set by a pass that finished and by nothing else, and a daemon starts every folder
    // without one, so `None` here means this folder has not had its turn in this run — whether it has
    // never synced or synced last week. Passes are serialized, so it is waiting for the pass that is
    // running (`Busy`) or for the daemon to reach it (`Resting`).
    //
    // After `Failed`, so an unavailable folder or a failed first pass keeps saying so; before the
    // queue, because `pending_changes` is the watcher's count and a folder that has had no pass is
    // not syncing it. Only beside other folders: alone, there is no other folder's pass to wait for,
    // and the answer stays what it was before folders existed (a one-folder app sees nothing new).
    if facts.peers != Peers::Alone && facts.last_sync.is_none() {
        return DaemonState::Queued;
    }
    if facts.pending_changes > 0 {
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
            pause_unsaved: None,
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
            pair: Some("default".to_owned()),
            pairs: vec![],
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
        assert_eq!(name(DaemonState::Queued), "\"queued\"");
    }

    // ---- one derivation, two adapters (#102 phase 5a-2) ----

    /// A summary built from the numbers a full reply carries — what the daemon's `pairs` entry for
    /// that same pair would say.
    fn summary_of(r: &ControlResponse) -> PairSummary {
        PairSummary {
            name: "p".to_owned(),
            local_root: "/l".into(),
            remote_root: "/r".into(),
            db_path: "/d".into(),
            paused: r.paused,
            syncing: r.syncing,
            reconcile_seq: r.reconcile_seq,
            last_sync_epoch_secs: r.last_sync_epoch_secs,
            last_error: r.last_error.clone(),
            pending_changes: r.pending_changes,
            pending_deletions: 0,
        }
    }

    /// Every combination of the inputs `derive` reads, with the error text varied across the kinds
    /// that route differently (none, auth-shaped, plain) — 2·2·3·3·2·2·3 = 432 replies.
    fn corpus() -> Vec<ControlResponse> {
        let mut out = Vec::new();
        for paused in [false, true] {
            for syncing in [false, true] {
                for error in [
                    None,
                    Some("401 Unauthorized"),
                    Some("proton-drive list failed: timed out"),
                ] {
                    for pending in [0usize, 3] {
                        for last_sync in [None, Some(1_750_000_000u64)] {
                            for history_empty in [true, false] {
                                for auth in [
                                    AuthState::Unknown,
                                    AuthState::SignedIn,
                                    AuthState::SignedOut,
                                ] {
                                    let mut r = response();
                                    r.paused = paused;
                                    r.syncing = syncing;
                                    r.last_error = error.map(str::to_owned);
                                    r.pending_changes = pending;
                                    r.last_sync_epoch_secs = last_sync;
                                    r.status_history = if history_empty {
                                        vec![]
                                    } else {
                                        vec![crate::wire::StatusHistoryEntry {
                                            epoch_secs: 1,
                                            message: "sync completed".into(),
                                            last_error: None,
                                            plan_summary: None,
                                            successful_sync_summary: None,
                                            failed_item_count: 0,
                                        }]
                                    };
                                    r.auth = auth;
                                    out.push(r);
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Acceptance 2: the reply entry point is the shared derivation over the reply's facts, for every
    /// reply there is — and the corpus really reaches every state, or this would pass trivially.
    #[test]
    fn a_full_reply_derives_the_same_state_through_either_entry_point() {
        let mut reached = std::collections::HashSet::new();
        for r in corpus() {
            let via_facts = derive(&PairFacts::from(&r));
            assert_eq!(derive_state(Ok(&r)), via_facts);
            reached.insert(format!("{via_facts:?}"));
        }
        for state in [
            "Running",
            "Idle",
            "Paused",
            "AuthExpired",
            "Failed",
            "FirstRun",
        ] {
            assert!(reached.contains(state), "the corpus never derives {state}");
        }
    }

    /// The other half of the brief's property: a pair known by its summary alone derives the same
    /// state as the full reply for the same numbers — **except** `FirstRun`, which a summary can
    /// never be (it has no history), and which lands on the state the reply would have had without
    /// that one rule.
    #[test]
    fn a_summary_derives_what_the_reply_would_except_first_run() {
        let mut first_runs = 0;
        for r in corpus() {
            let full = derive_state(Ok(&r));
            let from_summary = derive(&facts_of(&summary_of(&r), r.auth, &[]));
            if full == DaemonState::FirstRun {
                first_runs += 1;
                assert_ne!(from_summary, DaemonState::FirstRun);
            } else {
                assert_eq!(from_summary, full, "{r:?}");
            }
        }
        assert!(first_runs > 0, "the corpus never reaches first run");
    }

    /// `a_summary_can_never_derive_first_run`. The setting where it matters: a daemon that has just
    /// restarted. `last_sync_epoch_secs` is `None` for EVERY pair until its first pass succeeds, so
    /// "no last sync" is true of a pair that has synced for a year. The reply for the selected pair
    /// can tell (its history is on disk); a summary cannot, and must not guess.
    #[test]
    fn a_summary_can_never_derive_first_run() {
        let mut r = response();
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        let summary = summary_of(&r);
        let facts = facts_of(&summary, AuthState::SignedIn, &[]);
        assert_eq!(
            facts.history_empty, None,
            "a summary has no history to read"
        );
        assert_ne!(derive(&facts), DaemonState::FirstRun);
        assert_eq!(derive(&facts), DaemonState::Idle);
        // The same numbers as a full reply ARE first run: the difference is only what is known.
        assert_eq!(derive_state(Ok(&r)), DaemonState::FirstRun);
        // And nothing else a summary says reaches it either.
        for pending in [0, 5] {
            for error in [None, Some("x".to_owned())] {
                let mut s = summary.clone();
                s.pending_changes = pending;
                s.last_error = error;
                assert_ne!(
                    derive(&facts_of(&s, AuthState::Unknown, &[])),
                    DaemonState::FirstRun
                );
            }
        }
    }

    /// `an_unavailable_pair_derives_failed_from_its_summary` (maintainer decision M1b). A pair whose
    /// folder is missing (an unplugged drive) is never synced, publishes its reason as `last_error`
    /// and carries its last successful sync forward. Built from that summary it is `Failed` — not
    /// `Idle`, which would be a green tick over a folder nothing is syncing.
    #[test]
    fn an_unavailable_pair_derives_failed_from_its_summary() {
        let summary = PairSummary {
            name: "drive".to_owned(),
            local_root: "/mnt/usb/Sync".into(),
            remote_root: "/Drive/Usb".into(),
            db_path: "/mnt/usb/Sync/.sync/sync_index.db".into(),
            paused: false,
            syncing: false,
            reconcile_seq: 3,
            last_sync_epoch_secs: Some(1_750_000_000),
            last_error: Some("the sync folder /mnt/usb/Sync is not available".to_owned()),
            pending_changes: 0,
            pending_deletions: 0,
        };
        assert_eq!(
            derive(&facts_of(&summary, AuthState::SignedIn, &[])),
            DaemonState::Failed
        );
        // With no carried last sync too (a pair that was unavailable from the first boot): still
        // not first run — a summary cannot be — so still `Failed`.
        let mut never = summary.clone();
        never.last_sync_epoch_secs = None;
        assert_eq!(
            derive(&facts_of(&never, AuthState::SignedIn, &[])),
            DaemonState::Failed
        );
    }

    /// The pair a reply is about is derived from the reply; the rest from their summaries.
    #[test]
    fn the_described_pair_gets_the_full_state_and_the_others_their_summaries() {
        let mut r = response();
        r.pair = Some("a".to_owned());
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        let mut a = summary_of(&r);
        a.name = "a".to_owned();
        // `b` has never synced as far as its summary says, but a summary cannot call that first run.
        let mut b = a.clone();
        b.name = "b".to_owned();
        let mut c = a.clone();
        c.name = "c".to_owned();
        c.paused = true;
        r.pairs = vec![a, b, c];
        // Beside other folders the wizard's state does not exist: the same facts are a folder that has
        // not had its turn, for the described folder and for the one known by its summary alike.
        let described = derive_state(Ok(&r));
        assert_eq!(described, DaemonState::Queued);
        let states = pair_states(&r, described);
        let named = |name: &str| states.iter().find(|s| s.name == name).unwrap().state;
        assert_eq!(named("a"), DaemonState::Queued);
        assert_eq!(named("b"), DaemonState::Queued);
        assert_eq!(named("c"), DaemonState::Paused);
        // A legacy-shaped reply lists nothing, and the same facts are then the wizard's.
        r.pairs.clear();
        assert!(pair_states(&r, described).is_empty());
        assert_eq!(derive_state(Ok(&r)), DaemonState::FirstRun);
    }

    /// The brief's table (section 4.3), worst first, written out here as its own object: the property
    /// below compares `aggregate_state` against THIS, so a rank that was reordered in the function and
    /// not here is a failure, not a pair of agreeing mistakes.
    const WORST_FIRST: [DaemonState; 8] = [
        DaemonState::Unreachable,
        DaemonState::AuthExpired,
        DaemonState::Failed,
        DaemonState::FirstRun,
        DaemonState::Running,
        DaemonState::Queued,
        DaemonState::Paused,
        DaemonState::Idle,
    ];

    fn listed(states: &[DaemonState]) -> Vec<PairState> {
        states
            .iter()
            .enumerate()
            .map(|(at, state)| PairState {
                name: format!("p{at}"),
                state: *state,
                rank: severity(*state),
                waiting_for: None,
            })
            .collect()
    }

    /// `worst_state_wins`, as a property over EVERY combination of up to four folders rather than the
    /// handful somebody thought of. The reference is the first state of `WORST_FIRST` that any folder
    /// is in, so the mixed cases the brief names fall out of it: a paused and an idle folder show
    /// paused, a paused and a syncing one show syncing, a failed one beats anything.
    #[test]
    fn worst_state_wins() {
        let mut checked = 0;
        for size in 1..=4usize {
            for code in 0..WORST_FIRST.len().pow(size as u32) {
                let combo: Vec<DaemonState> = (0..size)
                    .map(|at| {
                        WORST_FIRST[(code / WORST_FIRST.len().pow(at as u32)) % WORST_FIRST.len()]
                    })
                    .collect();
                let expected = *WORST_FIRST
                    .iter()
                    .find(|candidate| combo.contains(candidate))
                    .expect("a non-empty combination has a worst state");
                assert_eq!(
                    aggregate_state(DaemonState::Idle, &listed(&combo)),
                    expected,
                    "{combo:?}"
                );
                checked += 1;
            }
        }
        // 8 + 64 + 512 + 4096: the loop really did walk them all.
        assert_eq!(checked, 4680);
    }

    /// The three cases the brief calls out by name, so a reader sees the rule without decoding the
    /// property: they are the ones a glyph reader will actually meet.
    #[test]
    fn the_mixed_cases_the_brief_names() {
        use DaemonState::*;
        let of = |states: &[DaemonState]| aggregate_state(Idle, &listed(states));
        assert_eq!(
            of(&[Paused, Idle]),
            Paused,
            "paused + idle -> the paused glyph"
        );
        assert_eq!(
            of(&[Paused, Running]),
            Running,
            "paused + syncing -> the syncing glyph"
        );
        assert_eq!(
            of(&[Failed, Idle]),
            Failed,
            "failed + anything -> the offline glyph"
        );
        assert_eq!(of(&[Failed, Running, Paused]), Failed);
        assert_eq!(of(&[AuthExpired, Failed]), AuthExpired);
    }

    /// A reply that lists no folder has nothing to rank: the state derived from the reply stands.
    #[test]
    fn a_reply_that_lists_no_folders_is_ranked_by_what_it_says_itself() {
        assert_eq!(
            aggregate_state(DaemonState::Paused, &[]),
            DaemonState::Paused
        );
        assert_eq!(
            aggregate_state(DaemonState::Unreachable, &[]),
            DaemonState::Unreachable
        );
    }

    /// The rank rides on every `PairState` a reply produces, so the webview orders folders by what
    /// Rust decided.
    #[test]
    fn every_pair_state_carries_the_rank_of_its_state() {
        let mut r = response();
        r.pair = Some("a".to_owned());
        let mut a = summary_of(&r);
        a.name = "a".to_owned();
        let mut b = a.clone();
        b.name = "b".to_owned();
        b.paused = true;
        r.pairs = vec![a, b];
        let described = derive_state(Ok(&r));
        let states = pair_states(&r, described);
        assert_eq!(states.len(), 2);
        for pair in &states {
            assert_eq!(pair.rank, severity(pair.state), "{pair:?}");
        }
        assert!(states[1].rank > states[0].rank, "paused outranks idle");
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

    // ---- a folder that has not had its turn (#455 and the live report) ----

    /// A summary as the daemon lists it: `last_sync` set unless the folder has not completed a pass
    /// in this run (a daemon starts every folder at `None`, and only a finished pass sets it).
    fn folder(name: &str, syncing: bool, last_sync: Option<u64>) -> PairSummary {
        PairSummary {
            name: name.to_owned(),
            local_root: format!("/l/{name}").into(),
            remote_root: format!("/Drive/{name}").into(),
            db_path: format!("/l/{name}/.sync/i.db").into(),
            paused: false,
            syncing,
            reconcile_seq: 0,
            last_sync_epoch_secs: last_sync,
            last_error: None,
            pending_changes: 0,
            pending_deletions: 0,
        }
    }

    /// The live report, as the daemon sent it: the default folder is mid-pass (a 25 minute full
    /// walk), the other two have not had a turn. `photos` is the folder the window shows, so its
    /// reply is the full one, with an empty history.
    fn three_folders() -> ControlResponse {
        let mut r = response();
        r.pair = Some("photos".to_owned());
        r.last_sync_epoch_secs = None;
        r.status_history = vec![];
        r.pairs = vec![
            folder("documents", true, None),
            folder("photos", false, None),
            folder("videos", false, None),
        ];
        r
    }

    #[test]
    fn a_folder_that_has_not_synced_is_never_up_to_date_beside_one_that_is_syncing() {
        let r = three_folders();
        let described = derive_state(Ok(&r));
        let states = pair_states(&r, described);
        let state_of = |name: &str| states.iter().find(|s| s.name == name).unwrap().state;
        assert_eq!(state_of("documents"), DaemonState::Running);
        for name in ["photos", "videos"] {
            assert_ne!(state_of(name), DaemonState::Idle, "{name} reads up to date");
        }
        assert_ne!(described, DaemonState::Idle);
    }

    #[test]
    fn the_wire_form_of_a_waiting_folder_says_it_is_waiting_and_for_whom() {
        let r = three_folders();
        let described = derive_state(Ok(&r));
        let wire = serde_json::to_value(pair_states(&r, described)).expect("serializes");
        for at in [1, 2] {
            assert_eq!(wire[at]["state"], "queued", "{wire}");
            assert_eq!(wire[at]["waiting_for"], "documents", "{wire}");
        }
        assert!(
            wire[0].get("waiting_for").is_none(),
            "a folder that is not waiting names nothing: {wire}"
        );
    }

    fn state_in(r: &ControlResponse, name: &str) -> PairState {
        pair_states(r, derive_state(Ok(r)))
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} is not listed"))
    }

    #[test]
    fn a_folder_waits_for_the_one_that_is_running_a_pass_and_says_which() {
        let r = three_folders();
        let photos = state_in(&r, "photos");
        assert_eq!(photos.state, DaemonState::Queued);
        assert_eq!(photos.waiting_for.as_deref(), Some("documents"));
        // The described folder and a folder known by its summary say the same thing.
        let videos = state_in(&r, "videos");
        assert_eq!(videos.state, DaemonState::Queued);
        assert_eq!(videos.waiting_for.as_deref(), Some("documents"));
        assert_eq!(derive_state(Ok(&r)), DaemonState::Queued);
        assert_eq!(
            described_waiting_for(&r, DaemonState::Queued).as_deref(),
            Some("documents")
        );
        // The folder that is running is not waiting for anything, least of all itself.
        let documents = state_in(&r, "documents");
        assert_eq!(documents.state, DaemonState::Running);
        assert_eq!(documents.waiting_for, None);
    }

    #[test]
    fn with_nothing_running_a_folder_that_has_not_had_its_turn_is_starting_and_names_nobody() {
        // The daemon has just started: every folder is due and none has been popped yet.
        let mut r = three_folders();
        r.pairs[0].syncing = false;
        for name in ["documents", "photos", "videos"] {
            let state = state_in(&r, name);
            assert_eq!(state.state, DaemonState::Queued, "{name}");
            assert_eq!(state.waiting_for, None, "{name} has nobody to wait for");
        }
        assert_eq!(described_waiting_for(&r, DaemonState::Queued), None);
    }

    /// WHY a folder that HAS synced and waits behind another may stay `up to date`: it finished a pass
    /// in this run with no error, nothing the daemon has said since contradicts that, and its last
    /// sync is on the hero. Calling every settled folder `waiting` for as long as another folder's
    /// pass runs would flip the whole list on each pass and say nothing true about the files.
    #[test]
    fn a_folder_that_has_finished_a_pass_stays_up_to_date_behind_another_folders_pass() {
        let mut r = three_folders();
        r.pairs[1].last_sync_epoch_secs = Some(1_750_000_000);
        r.pairs[2].last_sync_epoch_secs = Some(1_750_000_000);
        r.last_sync_epoch_secs = Some(1_750_000_000);
        let states = pair_states(&r, derive_state(Ok(&r)));
        let word = |name: &str| states.iter().find(|s| s.name == name).unwrap();
        assert_eq!(word("photos").state, DaemonState::Idle);
        assert_eq!(word("videos").state, DaemonState::Idle);
        assert_eq!(word("photos").waiting_for, None);
    }

    #[test]
    fn a_folder_that_was_never_synced_keeps_paused_failed_signed_out_and_syncing_beside_others() {
        // Each of these outranked everything below it before, and must go on doing so.
        let mut r = three_folders();
        r.pairs[2].paused = true;
        assert_eq!(state_in(&r, "videos").state, DaemonState::Paused);

        let mut r = three_folders();
        r.pairs[2].last_error = Some("the sync folder is not available".to_owned());
        assert_eq!(state_in(&r, "videos").state, DaemonState::Failed);

        let mut r = three_folders();
        r.auth = AuthState::SignedOut;
        assert_eq!(state_in(&r, "videos").state, DaemonState::AuthExpired);
        assert_eq!(state_in(&r, "photos").state, DaemonState::AuthExpired);

        let mut r = three_folders();
        r.pairs[2].syncing = true;
        r.pairs[0].syncing = false;
        assert_eq!(state_in(&r, "videos").state, DaemonState::Running);
        assert_eq!(state_in(&r, "videos").waiting_for, None);
        // ... and the others now wait for it.
        assert_eq!(
            state_in(&r, "photos").waiting_for.as_deref(),
            Some("videos")
        );
    }

    #[test]
    fn a_failed_first_pass_on_the_described_folder_is_failed_and_not_nothing_synced_yet() {
        // Beside other folders a never-synced folder's error used to be hidden by the wizard's state.
        let mut r = three_folders();
        r.last_error = Some("proton-drive list failed: timed out".to_owned());
        r.pairs[1].last_error = r.last_error.clone();
        assert_eq!(derive_state(Ok(&r)), DaemonState::Failed);
    }

    #[test]
    fn a_queue_of_watcher_changes_does_not_make_a_folder_without_a_pass_syncing() {
        // `pending_changes` is the watcher's count. A folder that has not been popped is not syncing
        // them, and `Syncing 3 changes` over it is the same untruth `Idle` was.
        let mut r = three_folders();
        r.pending_changes = 3;
        r.pairs[2].pending_changes = 3;
        assert_eq!(state_in(&r, "videos").state, DaemonState::Queued);
        assert_eq!(derive_state(Ok(&r)), DaemonState::Queued);
    }

    /// Acceptance: one folder is byte for byte what it was. For every reply the corpus reaches,
    /// listing the folder as the only one changes nothing, and nothing is ever `Queued`.
    #[test]
    fn one_folder_derives_exactly_as_before_and_is_never_queued() {
        for r in corpus() {
            let before = derive_state(Ok(&r));
            assert_ne!(before, DaemonState::Queued, "{r:?}");
            let mut one = r.clone();
            let mut only = summary_of(&one);
            only.name = "default".to_owned();
            one.pair = Some("default".to_owned());
            one.pairs = vec![only];
            assert_eq!(derive_state(Ok(&one)), before, "{r:?}");
            let states = pair_states(&one, before);
            assert_eq!(states[0].state, before);
            assert_eq!(states[0].waiting_for, None);
        }
    }

    /// The property behind the whole change: beside other folders, no folder without a finished pass
    /// is ever `Idle` — whatever the others are doing — and a folder with one is never `Queued`.
    #[test]
    fn beside_other_folders_up_to_date_needs_a_finished_pass() {
        let mut checked = 0;
        for mine in 0..16u32 {
            for other in 0..16u32 {
                let flags =
                    |bits: u32| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
                let (syncing, paused, errored, synced) = flags(mine);
                let (o_syncing, o_paused, o_errored, o_synced) = flags(other);
                let build = |name: &str, s: bool, p: bool, e: bool, done: bool| {
                    let mut f = folder(name, s, done.then_some(1_750_000_000));
                    f.paused = p;
                    f.last_error = e.then(|| "the remote listing timed out".to_owned());
                    f
                };
                let mut r = response();
                r.pair = Some("mine".to_owned());
                r.pairs = vec![
                    build("other", o_syncing, o_paused, o_errored, o_synced),
                    build("mine", syncing, paused, errored, synced),
                ];
                let me = &r.pairs[1];
                r.paused = me.paused;
                r.syncing = me.syncing;
                r.last_error = me.last_error.clone();
                r.last_sync_epoch_secs = me.last_sync_epoch_secs;
                r.status_history = vec![];
                let state = state_in(&r, "mine").state;
                assert_eq!(derive_state(Ok(&r)), state, "reply and list agree");
                if !synced {
                    assert_ne!(state, DaemonState::Idle, "mine={mine} other={other}");
                }
                if synced {
                    assert_ne!(state, DaemonState::Queued, "mine={mine} other={other}");
                }
                if state == DaemonState::Queued {
                    assert!(!syncing && !paused && !errored && !synced);
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 256);
    }

    /// The peers of a folder are the OTHER folders. A folder that is running a pass is not waiting for
    /// itself, and its peers are at rest when nothing else runs — this is what `waiting_for` reads.
    #[test]
    fn a_folder_is_not_its_own_peer() {
        let r = three_folders();
        let documents = &r.pairs[0];
        assert!(
            documents.syncing,
            "the premise: documents is the one running"
        );
        assert_eq!(
            facts_of(documents, r.auth, &r.pairs).peers,
            Peers::Resting,
            "nothing else is running"
        );
        assert_eq!(
            facts_of(&r.pairs[1], r.auth, &r.pairs).peers,
            Peers::Busy("documents")
        );
        assert_eq!(facts_of(documents, r.auth, &[]).peers, Peers::Alone);
    }

    #[test]
    fn a_reply_that_names_no_folder_excludes_nobody_from_its_peers() {
        // An unresolved selector answers `pair: None` with `pairs` populated.
        let mut r = three_folders();
        r.pair = None;
        r.pairs[0].syncing = false;
        r.pairs[1].syncing = true;
        let facts = PairFacts::from(&r);
        assert_eq!(facts.peers, Peers::Busy("photos"));
    }

    #[test]
    fn queued_ranks_between_running_and_paused() {
        assert!(severity(DaemonState::Running) > severity(DaemonState::Queued));
        assert!(severity(DaemonState::Queued) > severity(DaemonState::Paused));
        // Beside a running folder the glyph is the running one, whichever is listed first.
        for states in [
            [DaemonState::Queued, DaemonState::Running],
            [DaemonState::Running, DaemonState::Queued],
        ] {
            assert_eq!(
                aggregate_state(DaemonState::Idle, &listed(&states)),
                DaemonState::Running
            );
        }
        assert!(!DaemonState::Queued.counters_unknown());
    }

    /// Waiting asks nothing of the person, so it never hides a state that does (or one that is
    /// wrong): beside any of these the glyph is theirs, whichever folder is listed first.
    #[test]
    fn a_waiting_folder_never_outranks_a_state_that_needs_the_person() {
        for worse in [
            DaemonState::FirstRun,
            DaemonState::Failed,
            DaemonState::AuthExpired,
            DaemonState::Unreachable,
        ] {
            assert!(severity(worse) > severity(DaemonState::Queued), "{worse:?}");
            for states in [[DaemonState::Queued, worse], [worse, DaemonState::Queued]] {
                assert_eq!(
                    aggregate_state(DaemonState::Idle, &listed(&states)),
                    worse,
                    "{states:?}"
                );
            }
        }
    }
}
