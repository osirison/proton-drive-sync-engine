use crate::AppResult;
use crate::index::{EntityKind, IndexTotals};
use crate::sync::{DeleteDirection, PlanSummary, SyncAction, UnsyncableItem};
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
    /// Force the daemon's next reconcile to a full-tree walk instead of a warm start / incremental
    /// pass (e.g. to self-heal suspected drift). Like `Syncnow`, it also schedules that pass. Wire
    /// value `"resync"`; an older daemon that predates this variant rejects it as an unknown
    /// command (the client is simply newer than the daemon).
    Resync,
    /// Discard the daemon's learned state — the baseline index, the event cursors, the warm-start
    /// counter and the standing delete approvals — and reconcile from scratch (G23/#237's *Reset
    /// the index*). Latched like [`Self::Resync`] and applied by the main loop between passes, so
    /// nothing is truncated under an in-flight reconcile; the database FILE is never removed (see
    /// `index::reset_index_state`). Wire value `"reset_index"`; an older daemon rejects it as an
    /// unknown command.
    #[serde(rename = "reset_index")]
    ResetIndex,
    /// Approve pending deletions so they apply on the next sync. The `argument` on the request
    /// selects the target: a relative path, or `"all"` for every currently-pending deletion.
    Approve,
    /// Revoke a prior approval (before it has applied). Same `argument` selector as `Approve`.
    Deny,
    /// **Refuse** a withheld deletion and put the two sides back in step: purge the baseline
    /// `file_index` record for the target (and its whole subtree), so the surviving side stops
    /// looking like a deletion and is adopted back onto the other side by the bootstrap arm — an
    /// upload for `direction: local`, a download for `direction: remote` (#224). Same `argument`
    /// selector as `Approve`, and like it, only a *currently pending* deletion can be named.
    ///
    /// Distinct from [`Self::Deny`], which revokes an approval and leaves the deletion withheld and
    /// re-derived for ever. Wire value `"keep"`; an older daemon rejects it as an unknown command.
    Keep,
    /// Ask the daemon to exit gracefully (same clean path as SIGTERM). Lets a UI restart the
    /// daemon regardless of how it was launched (systemd unit or direct spawn).
    Shutdown,
    /// Query the per-file history log: what moved and when (#230), or one path's own history
    /// (#190) when `argument` names it. `window_secs` and `limit` bound the answer. Wire value
    /// `"activity"`; an older daemon rejects it as an unknown command. The pass-level history
    /// (#229/#238) and today's byte totals (#191) ride on every `Status` reply instead — the
    /// screens that draw them are already polling it.
    Activity,
    /// List one **remote** directory, non-recursively (#99). `argument` names it relative to the
    /// daemon's configured `remote_root`; absent or empty means the root itself, which is the
    /// legitimate landing page of a remote browser and the reason this selector is validated with
    /// `validate_relative_path` rather than its non-empty sibling — a listing reads, it does not
    /// join a path onto a root to produce a side effect.
    ///
    /// **`literal_path` has no meaning here.** There is no reserved word: a folder named `all`
    /// lists like any other. The `Approve`/`Deny`/`Keep` selector is the only place `"all"` means
    /// anything, and a read-only verb must not grow a second sentinel.
    ///
    /// Read-only and answered on the IPC task, but unlike every other verb it *does* run work —
    /// one `proton-drive` invocation, behind the process's CLI gate. See
    /// [`crate::proton::CliGate`] for why that gate exists and why this verb may answer
    /// [`ListingOutcome::Busy`] instead of waiting. Wire value `"list"`; an older daemon rejects it
    /// as an unknown command.
    List,
    /// Compute a **plan-only pass** — the rehearsal the Plan screen reviews — on the daemon's own
    /// client, behind the one CLI gate (#100/#192/#209, and the on-demand half of #317).
    ///
    /// **`Syncnow`-shaped, deliberately not `List`-shaped.** A plan is a full local stat-walk plus
    /// an O(folders) remote walk that can run half an hour; it must never run on the IPC task. So
    /// this is an immediate ack plus a latch, and the pass runs on the daemon's main loop — which
    /// is also what makes the existing [`SyncActivity`] describe it, closing #209's progress line
    /// with no second progress channel. Its pause semantics mirror `syncnow`'s exactly: a paused
    /// daemon schedules nothing and answers [`PlanOutcome::Paused`].
    ///
    /// **The pass observes and consumes nothing.** It persists no event cursor, drains no pending
    /// changes, clears no first-pass or rescan flag, moves no warm-start counter, writes no history
    /// row, consumes no delete approval and re-stamps no withheld deletion — one rule with one
    /// reason: a rehearsal that consumed evidence would leave the real pass unable to re-derive it.
    ///
    /// The ack carries the request number to wait for; read the answer with [`Self::PlanResult`].
    /// Wire value `"plan"`; an older daemon rejects it as an unknown command.
    Plan,
    /// Read the last computed plan (and the last apply's verdict) — a query, answered straight from
    /// the published snapshot like `status`, never by running work. The sibling of [`Self::Plan`]
    /// exactly as `status` is the sibling of `syncnow`.
    ///
    /// Wire value `"plan_result"`; an older daemon rejects it as an unknown command.
    #[serde(rename = "plan_result")]
    PlanResult,
    /// Execute a plan the user reviewed, named by its token in `argument` (#100).
    ///
    /// **Re-plan-and-compare, never replay.** The daemon runs a *normal* pass, canonicalises the
    /// plan it freshly computes, and executes it only if its token equals the one presented. It
    /// never executes frozen rows: `sync::compute_directory_deletion_verdicts` proves a recursive
    /// delete against the actions that will *really* be planned (#282), so a verdict proved against
    /// a tree that has since moved would authorise a deletion nothing re-derived — and an upload of
    /// since-changed content would ship under a review of different bytes. On divergence nothing is
    /// executed and the fresh plan is published with a new token, which is the Plan screen's
    /// re-confirm workaround made atomic instead of racy.
    ///
    /// With `skip_destructive` set this is #192's filtered apply: the reviewed plan minus its
    /// destructive rows.
    ///
    /// **Not a latch.** `Resync` and `ResetIndex` survive a pause because they describe what the
    /// next pass should do; an apply performs deletions under a review that is already minutes old,
    /// so a pause that overtakes it *cancels* it ([`ApplyOutcome::Failed`]) rather than firing it
    /// on some later tick. The plan is untouched, so resuming and applying the same token works.
    ///
    /// Wire value `"apply"`; an older daemon rejects it as an unknown command.
    Apply,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlRequest {
    pub command: ControlCommand,
    /// Optional argument for commands that need one (`Approve`/`Deny`). `#[serde(default)]` keeps
    /// the wire shape backward-compatible: older clients that omit it still parse.
    #[serde(default)]
    pub argument: Option<String>,
    /// When `true`, `argument` is always a **literal relative path** — the daemon must not give
    /// the reserved word `"all"` its every-pending-item meaning. This lets a client target a
    /// file literally named `all` (any letter case) without mass-approving. `#[serde(default)]`
    /// keeps wire compat: legacy clients omit it and retain the historical case-insensitive
    /// `"all"` interpretation.
    #[serde(default)]
    pub literal_path: bool,
    /// [`ControlCommand::Activity`] only: how far back to look, in seconds. `None` (and `0`) mean
    /// "everything still retained" — which is what a per-path history wants, since the row it is
    /// after ("First brought to this computer") may be months old. `#[serde(default)]` for wire
    /// compat.
    #[serde(default)]
    pub window_secs: Option<u64>,
    /// Cap on returned rows. [`ControlCommand::Activity`] uses
    /// [`ACTIVITY_EVENTS_DEFAULT_LIMIT`] when it is `None`, [`ControlCommand::List`] uses
    /// [`LIST_ENTRIES_DEFAULT_LIMIT`]; both report an untruncated `total` either way, and both
    /// clamp to their own maximum because a control reply is read into memory whole.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Which deletion at `argument` an `Approve` authorizes when **nothing pending matches it** —
    /// the pre-pass approval the Plan screen's typed-`DELETE` gate needs (#227). A plan can name a
    /// deletion before any pass has withheld it, and a path alone does not say which of the two
    /// deletions at it is meant, so an approval with no direction authorizes nothing (the #298 rule:
    /// an ambiguous selector authorizes nothing). Ignored when the selector *does* match a pending
    /// item — that item's own direction is the authority — and ignored entirely by every other
    /// command. `#[serde(default)]` keeps both directions of the wire compatible.
    #[serde(default)]
    pub direction: Option<DeleteDirection>,
    /// [`ControlCommand::Apply`] only: run the reviewed plan **minus its destructive rows** (#192's
    /// `Run it without the deletion`).
    ///
    /// The membership test is [`crate::sync::SyncAction::is_destructive`] — the same one
    /// [`PlanSummary::destructive_actions`] counts with — so the rows dropped are exactly the rows
    /// the user was shown as destructive.
    ///
    /// **This is not the approval gate, and it overrides it.** A destructive row is dropped even
    /// when a standing approval would have authorised it: the user asked for *this run* without
    /// them, and an approval answers a different question ("may this deletion ever apply"). The
    /// approval is left standing and unconsumed, and the deletion re-plans next pass.
    /// `#[serde(default)]` keeps both directions of the wire compatible, and every other command
    /// ignores it.
    #[serde(default)]
    pub skip_destructive: bool,
}

/// Default and hard cap on the rows one [`ControlCommand::Activity`] reply carries. A control
/// reply is read into memory whole, so the limit is the daemon's to enforce, not the client's.
pub const ACTIVITY_EVENTS_DEFAULT_LIMIT: usize = 50;
pub const ACTIVITY_EVENTS_MAX_LIMIT: usize = 500;

/// Default and hard cap on the entries one [`ControlCommand::List`] reply carries, for the same
/// reason [`ACTIVITY_EVENTS_DEFAULT_LIMIT`] exists: the whole reply is one JSON line read into
/// memory, so the bound is the daemon's to enforce and not the client's to be trusted with. A
/// remote folder can hold tens of thousands of nodes.
pub const LIST_ENTRIES_DEFAULT_LIMIT: usize = 500;
pub const LIST_ENTRIES_MAX_LIMIT: usize = 5_000;

impl ControlRequest {
    /// A request carrying nothing but its command. Every optional field is a later addition, so
    /// building one this way keeps a caller from having to name fields its command ignores.
    pub fn new(command: ControlCommand) -> Self {
        Self {
            command,
            argument: None,
            literal_path: false,
            window_secs: None,
            limit: None,
            direction: None,
            skip_destructive: false,
        }
    }
}

/// The wire form of a path, and the form an `approve`/`deny` selector is matched against. One
/// definition for every wire this engine publishes — see [`crate::wire_path`], which also states
/// the rule a client may act on a lossy path under.
pub use crate::wire_path;

/// One entry of a [`ControlCommand::List`] reply: a node that exists on Proton right now.
///
/// **Remote ground truth, not sync state.** Nothing here is filtered by the selective-sync globs
/// and nothing is read from the baseline index: the verb answers "what is on Proton under this
/// folder", which is a different question from "what would this daemon sync". A client that wants
/// the second question has `status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteEntry {
    /// Path relative to the daemon's `remote_root`. Lossy on the wire like every other path this
    /// engine publishes (see [`crate::lossy_path`]), so it is a **rendering**: a client must not
    /// feed it back as a selector for anything destructive.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    /// The node's own name, as the remote reports it.
    pub name: String,
    pub entity_kind: EntityKind,
    /// The remote's claimed SHA-1, when it exposes one. `None` for a directory, and for a file
    /// that has no byte content to digest — a Proton-native Docs/Sheets node — which is also the
    /// commonest reason `downloadable` is `false`.
    #[serde(default)]
    pub sha1: Option<String>,
    /// Whether the engine could fetch this node's bytes. `false` marks a node the sync planner
    /// treats as unsupported, so a browser can show it as present-but-not-syncable rather than
    /// implying it will arrive.
    pub downloadable: bool,
}

/// The answer to a [`ControlCommand::List`] request, and the reason the reply carries a typed
/// state rather than prose in `message`: a client deciding "retry in a moment" versus "this folder
/// is gone" by pattern-matching a sentence is exactly the bug #103 exists to remove, and it would
/// be no better for having been introduced by #99.
///
/// Internally tagged with `state`, and an unrecognized tag parses as [`Self::Unknown`] rather than
/// failing the whole reply — the rule [`crate::sync::UnsyncableReason`] already follows, so a newer
/// daemon can add a state without breaking an older client's parse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ListingOutcome {
    /// The directory was listed. `entries` is capped (see [`LIST_ENTRIES_MAX_LIMIT`]); `total` is
    /// the untruncated count, so `truncated` is `total > entries.len()`.
    Listed {
        /// The listed directory, relative to `remote_root` — empty for the root itself. Echoed
        /// back so a reply can be matched to the request that asked for it.
        #[serde(with = "crate::lossy_path")]
        path: PathBuf,
        entries: Vec<RemoteEntry>,
        total: usize,
        truncated: bool,
    },
    /// A `proton-drive` invocation was already running and did not finish within this request's
    /// budget, so **nothing was attempted**. Retryable as-is; it says nothing about the remote.
    /// See [`crate::proton::CliGate`].
    Busy,
    /// The listing was attempted and failed. Carries the failure for display only — a client must
    /// not classify it by matching this text. If the cause was the Proton session, the reply's
    /// `auth` field already says so.
    Failed { error: String },
    /// A state added by a newer daemon. Render it as "unavailable"; never as an empty folder.
    #[serde(other)]
    Unknown,
}

/// Default and hard cap on the plan rows one reply carries **beyond its destructive ones**. Same
/// reason as [`LIST_ENTRIES_DEFAULT_LIMIT`]: the reply is one JSON line read into memory whole, and
/// a first sync of a large folder plans tens of thousands of rows.
///
/// **It is not a bound on the reply, and the difference is deliberate.** A reply carries *every*
/// destructive row plus at most this many of the rest, so its worst case is
/// `summary.destructive_actions + limit` — a plan that deletes 12,000 files sends 12,000 rows
/// whatever the cap. That is the trade this window is built around: `files_at_risk` is safety copy
/// and a plan whose deletions fell off the end reads as safe, so the deletions are the one thing a
/// cap may not reach. A client sizing a buffer must read the sum, not this constant.
///
/// **Truncating the rest is safe only because of the token.** A client applies a plan by naming its
/// token, never by sending rows back, so the window is a *rendering* and the daemon still executes
/// the whole plan either way.
pub const PLAN_ACTIONS_DEFAULT_LIMIT: usize = 500;
pub const PLAN_ACTIONS_MAX_LIMIT: usize = 5_000;

/// The answer to a [`ControlCommand::Plan`] or [`ControlCommand::PlanResult`] request.
///
/// Typed, internally tagged, and `#[serde(other)]`-terminated, following [`ListingOutcome`]: a
/// client must never tell "still working" from "it failed" by matching a sentence (#103).
///
/// **`plan_seq` is the whole synchronisation contract.** [`Self::Scheduled`] carries the request
/// number the ack booked; every other variant carries the number the daemon has *answered*. A
/// client waits until `plan_seq >= its ack's plan_seq`, exactly as `syncnow` waits on
/// `reconcile_seq` — but on its own counter, because a plan pass deliberately does **not** bump
/// `reconcile_seq`: doing so would satisfy a concurrent `syncnow` watcher's target early and make
/// it report a sync that never ran.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum PlanOutcome {
    /// Ack for [`ControlCommand::Plan`]: a plan-only pass is scheduled. Wait for `plan_seq`.
    Scheduled { plan_seq: u64 },
    /// A plan pass is scheduled or running. Retryable as-is.
    ///
    /// **`plan_seq` is the last generation *answered*, not the one in flight** — the only number
    /// the plane can state, since the request being computed has no answer yet. It is therefore
    /// strictly below every outstanding request, and a client must read this arm as "not yet, keep
    /// polling" and **never** compare it against its own ack's `plan_seq`: that comparison would
    /// resolve a wait with the *previous* plan, which is the exact confusion booking a generation
    /// before the ack is built exists to prevent. Only [`Self::Computed`] and [`Self::Failed`]
    /// carry a generation that answers anything.
    Computing { plan_seq: u64 },
    /// The reviewed plan. Boxed because it is two orders of magnitude larger than every other arm,
    /// and this enum rides on a reply that mostly carries the small ones.
    Computed(Box<ReviewedPlan>),
    /// The plan pass failed. Display-only text; a client must not classify it by matching it.
    Failed { plan_seq: u64, error: String },
    /// Syncing is paused, so nothing was scheduled — the same answer `syncnow` gives, for the same
    /// reason: a paused daemon runs no passes, and a plan is a pass.
    Paused,
    /// Nothing has been planned in this daemon run. Not an empty plan.
    Absent,
    /// A state added by a newer daemon. Render it as "unavailable"; never as an empty plan.
    #[serde(other)]
    Unknown,
}

/// The payload of [`PlanOutcome::Computed`]: one plan, as the user reviews it.
///
/// Flattened onto the outcome by serde's internally-tagged representation, so on the wire this is
/// `{"state":"computed", "token":…, "summary":…, …}` — one object, not a nested one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewedPlan {
    pub plan_seq: u64,
    /// The plan's identity (see [`crate::sync::plan_token`]) — what an `apply` is pinned to.
    pub token: String,
    pub computed_epoch_secs: u64,
    pub summary: PlanSummary,
    /// A window over the plan: **every** destructive row, then as many of the rest as the request's
    /// limit allows (see [`PLAN_ACTIONS_MAX_LIMIT`], which bounds only that second part — this
    /// vector can be longer). `total` is the untruncated plan length, and `truncated` says whether
    /// anything was held back.
    pub actions: Vec<crate::sync::PlannedAction>,
    pub total: usize,
    pub truncated: bool,
    /// What the plan's own local walk **dropped** as unsyncable (#315/#232). Deliberately not
    /// spelled `unsyncable`, which on [`ControlResponse`] means the persistent merged store — this
    /// is one observation, with no age and nothing merged into it.
    #[serde(default)]
    pub cannot_sync: Vec<crate::index::UnsyncableEntry>,
}

/// The answer to a [`ControlCommand::Apply`] request — the ack states, then the verdict the pass
/// leaves behind for the next [`ControlCommand::PlanResult`] to collect.
///
/// Same rules as [`PlanOutcome`], and the same counter discipline on `apply_seq`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ApplyOutcome {
    /// Ack: the token names the current plan, and a pass is scheduled to re-plan, compare and
    /// execute. Wait for `apply_seq`.
    Scheduled { apply_seq: u64 },
    /// The token names no current plan — superseded by a newer one, or left over from a previous
    /// daemon run. **Nothing was scheduled**: one stored plan at a time, latest wins, and a stale
    /// token is refused rather than silently upgraded into a fresh apply of something unreviewed.
    Stale,
    /// Syncing is paused, so nothing was scheduled (as `syncnow`).
    Paused,
    /// The pass executed the reviewed plan. `skipped_destructive` is #192's dropped-row count —
    /// `0` on an ordinary apply — and `failed` is the partial-pass count (#136).
    Applied {
        apply_seq: u64,
        executed: usize,
        skipped_destructive: usize,
        failed: usize,
    },
    /// The freshly computed plan differed from the reviewed one, so **nothing was executed**. The
    /// same reply's `plan` field carries the new plan and its new token: re-review, then apply that.
    Diverged { apply_seq: u64 },
    /// The pass failed before it could decide. Display-only text.
    Failed { apply_seq: u64, error: String },
    /// A state added by a newer daemon. Never render it as "applied".
    #[serde(other)]
    Unknown,
}

/// What the daemon currently believes about the Proton session (#103), so a UI stops deciding it
/// by matching error strings.
///
/// **Three states, and the third is not a synonym for either other one.** Only *evidence of
/// success* moves this to [`Self::SignedIn`], and only a failure the engine *classified* as an auth
/// failure moves it to [`Self::SignedOut`]; an unclassified failure — a timeout, a missing binary,
/// a disk error — moves it nowhere. A classifier whose fall-through arm reported "signed in and
/// fine" would turn every unrecognized failure into a false all-clear, which is the shape that has
/// shipped here before (#246).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthState {
    /// Nothing has yet proved or disproved the session — a freshly started daemon, or one whose
    /// only failures were of some other kind. Not "signed in", and not "signed out".
    #[default]
    Unknown,
    /// Something reached Proton successfully.
    SignedIn,
    /// A `proton-drive` invocation was refused for the session (see
    /// [`crate::proton::AuthFailure`]). The user has to sign in again.
    SignedOut,
}

impl AuthState {
    /// The wire token. Hand-written (like [`crate::sync::UnsyncableReason`]'s) so the mapping is
    /// visible in one place and an unknown token can degrade to [`Self::Unknown`] instead of
    /// failing an older client's whole reply.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::SignedIn => "signed-in",
            Self::SignedOut => "signed-out",
        }
    }
}

impl Serialize for AuthState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuthState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let token = String::deserialize(deserializer)?;
        Ok(match token.as_str() {
            "signed-in" => Self::SignedIn,
            "signed-out" => Self::SignedOut,
            // Including the literal `"unknown"`, and anything a newer daemon invents: an
            // unrecognized verdict is not a verdict.
            _ => Self::Unknown,
        })
    }
}

/// One withheld deletion surfaced to the user for review. `path` + `direction` identify it for an
/// `approve`; `fingerprint` (a file's baseline SHA-1 or a directory's `proton_id`) is what the
/// approval is pinned to, so it cannot later authorize a different deletion at the same path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingDeletion {
    /// Lossy on the wire (see [`crate::lossy_path`]); the daemon keeps the real path in this
    /// field.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    pub direction: DeleteDirection,
    pub entity_kind: EntityKind,
    pub fingerprint: String,
    /// When **this pass** derived the withheld action — the age of the pass, not of the deletion.
    /// The gate stamps `now` on everything it withholds and a pass cannot idle-skip while anything
    /// is pending, so this refreshes every ~30s for as long as the item waits. Read
    /// [`Self::first_seen_epoch_secs`] for "when did this happen"; this one answers "how fresh is
    /// this reply". Kept under its original name because it is a required field on the wire and an
    /// older client would fail to parse a reply without it (#225).
    pub detected_epoch_secs: u64,
    /// When this deletion was **first** withheld, carried across passes and restarts in
    /// `index::withheld_deletions` and re-stamped only when the fingerprint changes (a different
    /// deletion at the same path). This is the field a UI ages "deleted on Proton 22m ago" from.
    /// `#[serde(default)]` for replies from an older daemon — and `0` there means *unknown*, so a
    /// client must treat it as "no age to show" rather than as the epoch.
    #[serde(default)]
    pub first_seen_epoch_secs: u64,
    /// Files beneath a directory deletion, and their total size — what the subtree would cost you
    /// (#208). Counted from the baseline index at gate time, files only (a directory's own
    /// `file_size` is not a subtree total), so it is what the engine would actually delete: the
    /// baseline is already selective-sync filtered. `None` for a file (its own size is a lookup the
    /// client already has) and `None` from an older daemon — never `0`, which is a real answer for
    /// an empty folder.
    #[serde(default)]
    pub subtree_files: Option<u64>,
    #[serde(default)]
    pub subtree_bytes: Option<u64>,
}

/// One action whose side effect failed during the last pass (#136). The executor keeps going past
/// it — the rest of the plan still runs — so a pass can end with some items failed and everything
/// else synced. That is a **third** pass outcome, not a flavour of either other one: it is reported
/// as itself here and in [`crate::daemon::PassOutcome`], never folded into success or failure.
///
/// Reported, never acted on: the failed action is not recorded in the index and simply re-plans
/// next pass. This is not a retry queue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedItem {
    /// Lossy on the wire (see [`crate::lossy_path`]); the daemon keeps the real path in this
    /// field.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    pub action: SyncAction,
    /// The failure. At most [`FAILED_ITEM_ERROR_LIMIT`] bytes, ellipsis included — these ride on
    /// every status reply.
    pub error: String,
}

/// Hard cap on a [`FailedItem::error`] string in bytes, *inclusive* of the truncation ellipsis: a
/// CLI failure can carry kilobytes of stderr.
pub const FAILED_ITEM_ERROR_LIMIT: usize = 500;

/// The daemon's resolved folder pair + index location, surfaced over IPC so a UI can reflect the
/// *live* configuration no matter how the daemon was launched (config file, flags, or defaults).
/// Without this, a client that guesses at a config path renders placeholders against a healthy
/// daemon whose roots it cannot know.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunningConfigInfo {
    #[serde(with = "crate::lossy_path")]
    pub local_root: PathBuf,
    #[serde(with = "crate::lossy_path")]
    pub remote_root: PathBuf,
    #[serde(with = "crate::lossy_path")]
    pub db_path: PathBuf,
}

/// Live "what is the daemon doing right now", surfaced while `syncing` is true so clients can
/// render more than a spinner during a long pass (a multi-minute remote walk, a multi-GB
/// transfer). Purely informational display data: every field is best-effort, absence means
/// "unknown or not applicable", and nothing here participates in any sync decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncActivity {
    /// Coarse machine-readable step: `scanning-local`, `listing-remote`, `fetching-events`,
    /// `executing`, or `committing`. Clients should render an unrecognized token verbatim
    /// rather than fail, so new phases can be added without a lockstep upgrade.
    pub phase: String,
    /// Human-readable fragment locating the phase: the folder currently being listed, the file
    /// currently being scanned, or the action currently executing.
    #[serde(default)]
    pub detail: Option<String>,
    /// Remote folders listed so far during a `listing-remote` walk.
    #[serde(default)]
    pub folders_listed: Option<u64>,
    /// Local files visited so far during a `scanning-local` pass.
    #[serde(default)]
    pub files_scanned: Option<u64>,
    /// 1-based position of the currently executing action within the plan (`executing`).
    #[serde(default)]
    pub action_index: Option<u64>,
    /// Total number of planned actions this pass (`executing`).
    #[serde(default)]
    pub action_total: Option<u64>,
    /// The transfer queue as a bounded window (#211): every transfer in flight, then the next
    /// planned ones as [`TRANSFER_STATE_QUEUED`] rows, capped at [`TRANSFERS_REPORTED`].
    ///
    /// **Bounded, not truncated-and-silent**: [`Self::transfers_remaining`] carries the untruncated
    /// figure, so a client draws `+n more` from the daemon's count rather than from the length of a
    /// list it was handed.
    #[serde(default)]
    pub transfers: Vec<TransferActivity>,
    /// How many transfer actions this pass has left — the in-flight ones plus every one still
    /// queued behind them, counted past the window above.
    ///
    /// **This is what tells an empty [`Self::transfers`] apart from three different things**, and
    /// an empty list is not one of them on its own:
    ///
    /// - `None` — nothing is known about transfers: the pass is not executing yet (it is scanning,
    ///   walking or committing), or the reply came from a daemon predating #211. A client must NOT
    ///   render this as "nothing is moving"; it is unknown, not zero.
    /// - `Some(0)` — the pass is executing and has no transfers left. This is the only reading that
    ///   licenses "nothing is transferring".
    /// - `Some(n)` with rows — the window, and `n` counts past it.
    ///
    /// `Some(n > 0)` with an *empty* list cannot occur: the window always names as much of the
    /// remainder as it can hold, and it is republished on every action rather than only on the
    /// transfer ones, so a delete between two uploads does not blank it.
    ///
    /// Owned by the executor's own queue cursor and **never** derived from
    /// [`PassProgress::uploaded_files`]/[`PassProgress::downloaded_files`]: those count what
    /// *committed* over a different action set (a conflict sidecar is a download that is not a
    /// `Download`), so the subtraction would drift and, on a conflict-heavy pass, go negative.
    #[serde(default)]
    pub transfers_remaining: Option<u64>,
    /// The in-flight file transfer — **the first active row of [`Self::transfers`], derived**, and
    /// kept only so a client built before #211 still renders something.
    ///
    /// Written in exactly one place, [`Self::derive_legacy_transfer`], immediately before a reply
    /// goes out. The daemon core never assigns it: two fields set from two places about the same
    /// transfer is how they start disagreeing.
    #[serde(default)]
    pub transfer: Option<TransferActivity>,
    /// When this **phase** began (unix seconds). Reset on every phase change, so it is not the
    /// pass's start — read [`PassProgress::started_epoch_secs`] for that (#213).
    #[serde(default)]
    pub since_epoch_secs: Option<u64>,
    /// The pass as a unit: one start time and one change count that survive every phase change
    /// (#213). Carried across `begin_activity` rather than reset with the phase, so an elapsed
    /// time rendered from it climbs monotonically instead of jumping back to zero three times.
    #[serde(default)]
    pub pass: Option<PassProgress>,
}

/// The in-flight pass, as opposed to the phase it is currently in (#213).
///
/// Everything here survives a phase change. That is not a convention — it is why the per-direction
/// progress of #243 and #98 lives here and not on [`SyncActivity`]: `begin_activity` replaces the
/// current activity once per *action*, so a counter stored there would reset a few hundred times a
/// pass, which is the exact bug the pass block was added to fix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassProgress {
    /// When the pass started (unix seconds) — stable for its whole duration.
    pub started_epoch_secs: u64,
    /// Side-effecting actions this pass **planned**. `None` until the plan exists (the scan and
    /// remote walk run first), which a client renders as an omitted clause — never as `0`, per
    /// `14-behaviour-and-state.md`'s "a null summary means unknown, not zero". Distinct from
    /// [`crate::index::PassRecord::changed`], which is what a finished pass actually landed.
    #[serde(default)]
    pub changes: Option<u64>,
    /// Open token, [`crate::index::PassKind::as_str`] — render an unrecognized one verbatim.
    pub kind: String,
    /// Transfers that have landed **and committed** this pass, per direction (#243). Counts, not
    /// bytes: `44 sent · 115 received` is two action counters, and the bytes below are a different
    /// question that happens to be folded from the same events.
    ///
    /// Both pairs come from one fold — `daemon::PassLog::note_committed`, the same one that builds
    /// the history rollup — so a count and a byte sum here always describe the same set of side
    /// effects, and the direction is `SyncAction::transfer_direction` rather than a second
    /// derivation of it. An action that moved no bytes over the network (a conflict sidecar copied
    /// from the local file) is in neither.
    ///
    /// Invariant a client may rely on: `uploaded_files + downloaded_files <= action_index`, since a
    /// pass has at least as many actions as it has transfers.
    #[serde(default)]
    pub uploaded_files: u64,
    #[serde(default)]
    pub downloaded_files: u64,
    /// Bytes that have landed this pass, per direction (#98). Named to match
    /// [`crate::index::ByteTotals`], which answers the same question over a finished window.
    ///
    /// This is the byte figure #98 can honestly have. A **per-file** percentage is unreachable —
    /// see [`TransferActivity`] — but a pass-level one is not: `uploaded_bytes` against
    /// [`crate::sync::PlanSummary::upload_bytes`] is a real fraction of a real total. There is no
    /// download counterpart, because nothing knows what a download will weigh until it lands.
    #[serde(default)]
    pub uploaded_bytes: u64,
    #[serde(default)]
    pub downloaded_bytes: u64,
}

/// The pass-level history that rides on every status reply: enough to draw `Last 20 passes`
/// (#229), `Last one 2 days ago` (#238) and today's byte totals (#191) without a second round
/// trip. All three are queries over the one history schema in [`crate::index`] — see its module
/// note for why a byte total never comes from the per-file rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassHistory {
    /// The most recent recorded passes, newest first, capped at [`PASS_HISTORY_REPORTED`].
    /// **Notable passes only** — an idle pass records nothing (see `daemon::PassLog`), so twenty
    /// rows here are twenty passes that did something rather than ten minutes of polling.
    pub recent: Vec<crate::index::PassRecord>,
    /// The last full-tree walk, however long ago. `None` before this daemon has ever run one.
    #[serde(default)]
    pub last_full_sweep: Option<crate::index::PassRecord>,
    /// Bytes moved per direction since local midnight.
    pub today: crate::index::ByteTotals,
}

/// How many pass records a status reply carries. Matches the twenty bars `6a Activity passes`
/// draws.
pub const PASS_HISTORY_REPORTED: usize = 20;

/// [`PassProgress::kind`] for a plan-only pass (#209) — the rehearsal, not a sync.
///
/// Deliberately **not** an `index::PassKind` variant: a plan pass writes no `sync_passes` row, so
/// it has no stored kind, and inventing one would put a value in that column's vocabulary that no
/// row can ever hold. `kind` is an open token precisely so a live-only value can live here, and a
/// client that does not know this one renders it verbatim. What it buys: a screen reading
/// `files_scanned` can tell "my rehearsal is running" from "a sync started".
pub const PLAN_PASS_KIND: &str = "plan";

/// One row of the transfer queue: a file moving now, or one planned and not started.
///
/// **A PER-FILE PERCENTAGE IS UNREACHABLE HERE, BY CONSTRUCTION — NOT UNIMPLEMENTED (#98).**
/// `bytes_total` and `bytes_done` are never both present on the same transfer, because the two
/// directions can see opposite halves of the question:
///
/// | direction  | `bytes_total`                                | `bytes_done`                        |
/// | ---------- | -------------------------------------------- | ----------------------------------- |
/// | `upload`   | the local file's size, from the scan          | none — the CLI reports no progress  |
/// | `download` | none — a remote listing carries no file size  | sampled live from the staging dir   |
///
/// The remote JSON does carry a `totalStorageSize`, and it is **not** a file size: it reads `0` on
/// 873 of 5424 healthy files on a real account (those files download fine), and it plausibly
/// describes the encrypted footprint rather than the plaintext byte count. `proton.rs` deliberately
/// does not parse it, and nothing may make it a `bytes_total`.
///
/// So a client draws no track at all rather than a fraction it had to invent — `null` progress
/// meaning *no track*, which is a different thing from `0`, which reads as stalled. The figure that
/// *is* honest is per pass, not per file: see [`PassProgress::uploaded_bytes`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferActivity {
    /// `"upload"` or `"download"`.
    pub direction: String,
    /// Root-relative path of the file being transferred — or, when `files` is set, of the folder
    /// the batch is filling.
    #[serde(with = "crate::lossy_path")]
    pub path: PathBuf,
    /// Total size in bytes when known (uploads: the local file's size; downloads: unknown —
    /// the remote listing carries no size).
    #[serde(default)]
    pub bytes_total: Option<u64>,
    /// Bytes transferred so far when observable (downloads only; see the type docs).
    #[serde(default)]
    pub bytes_done: Option<u64>,
    /// [`TRANSFER_STATE_ACTIVE`] or [`TRANSFER_STATE_QUEUED`]. Open token — a client renders an
    /// unrecognized one verbatim rather than failing.
    ///
    /// A `String` rather than a tagged enum like [`ListingOutcome`], deliberately: this is the same
    /// kind of thing as [`SyncActivity::phase`] on the same record — a label with no payload of its
    /// own — and it follows that field's rule. A tagged enum earns its keep when the arms carry
    /// *different data* and a client would otherwise pattern-match prose to tell them apart.
    ///
    /// Defaults to *active* when absent, which is what a daemon predating #211 meant: it reported
    /// the in-flight transfer and nothing else.
    #[serde(default = "transfer_state_active")]
    pub state: String,
    /// How many files this row covers, when it covers more than one: a batched download
    /// (`download_many`) is a single CLI invocation over a whole folder's chunk, so the files in it
    /// are in flight together and share one staging directory. `None` means one file, and `path` is
    /// that file.
    ///
    /// The batch is one row rather than N because its `bytes_done` is one measurement of one
    /// directory: split across N rows it would have to be divided, and no division of it is true.
    #[serde(default)]
    pub files: Option<u64>,
    /// When this transfer began (unix seconds). `None` on a queued row — it has not begun.
    ///
    /// Stamped when the **executor reaches** the transfer, which is not quite when the child starts
    /// moving bytes: since #99 every `proton-drive` invocation waits on [`crate::proton::CliGate`],
    /// so a browse request already in flight can sit between the two. The clock a client renders
    /// from this is therefore "how long this transfer has been the daemon's current job", which is
    /// the honest reading — and nothing derives a *rate* from it, which is the figure that gap
    /// would falsify.
    ///
    /// A row built by [`Self::active`] always carries it, and that is load-bearing: the derived
    /// [`SyncActivity::transfer`] mirror is taken from active rows only, and a client predating
    /// #211 deserializes that field into a non-optional `u64`, so a `null` there would fail its
    /// whole reply.
    #[serde(default)]
    pub started_epoch_secs: Option<u64>,
}

/// [`TransferActivity::state`] for a transfer the daemon is running now.
pub const TRANSFER_STATE_ACTIVE: &str = "active";
/// [`TransferActivity::state`] for a planned transfer that has not started.
pub const TRANSFER_STATE_QUEUED: &str = "queued";

fn transfer_state_active() -> String {
    TRANSFER_STATE_ACTIVE.to_owned()
}

/// How many transfer rows a status reply carries (#211). The design's own cap — "cap at ~6 visible
/// with `+n more` in mono if exceeded" — so the window is the display and anything past it is a
/// number, not a row. Named here because the wire is where the bound is decided; the GUI slices to
/// its own cap as well, and draws `+n more` from
/// [`SyncActivity::transfers_remaining`] rather than from this length.
pub const TRANSFERS_REPORTED: usize = 6;

impl TransferActivity {
    /// A transfer that is running now. The only constructor that stamps a start time, which is what
    /// makes "every active row has one" true by construction rather than by review.
    pub fn active(direction: &str, path: PathBuf, bytes_total: Option<u64>, now: u64) -> Self {
        Self {
            direction: direction.to_owned(),
            path,
            bytes_total,
            bytes_done: None,
            state: transfer_state_active(),
            files: None,
            started_epoch_secs: Some(now),
        }
    }

    /// A planned transfer that has not started. No start time, no bytes done.
    pub fn queued(direction: &str, path: PathBuf, bytes_total: Option<u64>) -> Self {
        Self {
            direction: direction.to_owned(),
            path,
            bytes_total,
            bytes_done: None,
            state: TRANSFER_STATE_QUEUED.to_owned(),
            files: None,
            started_epoch_secs: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.state == TRANSFER_STATE_ACTIVE
    }
}

impl SyncActivity {
    /// The transfer running right now: the first active row of [`Self::transfers`], or — from a
    /// daemon predating #211, which sent only the singular field — the mirror.
    ///
    /// One definition, so no client has to decide which of the two fields to believe.
    pub fn active_transfer(&self) -> Option<&TransferActivity> {
        self.transfers
            .iter()
            .find(|transfer| transfer.is_active())
            .or(self.transfer.as_ref())
    }

    /// The rows waiting behind it, in plan order, as far as the window reaches.
    pub fn queued_transfers(&self) -> impl Iterator<Item = &TransferActivity> {
        self.transfers
            .iter()
            .filter(|transfer| !transfer.is_active())
    }

    /// How many transfers this pass still has that the window does not name — the `+n more` figure.
    ///
    /// `transfers_remaining` counts the running rows too, and a batched row stands for its whole
    /// chunk, so the tail is the remainder minus the files the window already accounts for. The
    /// same arithmetic the GUI's `hiddenTransfers` does, and the reason both are written down: a
    /// count is a claim.
    pub fn transfers_past_the_window(&self) -> u64 {
        let shown: u64 = self
            .transfers
            .iter()
            .map(|transfer| transfer.files.unwrap_or(1))
            .sum();
        self.transfers_remaining
            .unwrap_or(shown)
            .saturating_sub(shown)
    }

    /// Recomputes the legacy [`Self::transfer`] mirror from [`Self::transfers`].
    ///
    /// Called once, on the way out of the process, and nowhere else. Callers that measure something
    /// onto a transfer (the download staging sample) must write it into the **list** and then call
    /// this — deriving first would publish a mirror that is one measurement stale, and writing to
    /// the mirror instead would leave the list, which is what every #211-aware client reads, short
    /// of the number.
    pub fn derive_legacy_transfer(&mut self) {
        self.transfer = self
            .transfers
            .iter()
            .find(|transfer| transfer.is_active())
            .cloned();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlResponse {
    pub status: String,
    pub paused: bool,
    /// `true` while a reconcile pass is actually in flight. Distinct from `status == "running"`,
    /// which only means "not paused". `#[serde(default)]` keeps replies from older daemons
    /// parseable.
    #[serde(default)]
    pub syncing: bool,
    /// Count of completed reconcile attempts (success or failure) since the daemon started.
    /// A client that scheduled a sync can poll until this advances past the value in its ack
    /// (and `syncing` is false again) to know *its* pass finished. `#[serde(default)]` for
    /// replies from older daemons.
    #[serde(default)]
    pub reconcile_seq: u64,
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
    /// The live resolved configuration (see [`RunningConfigInfo`]). `#[serde(default)]` keeps
    /// replies from older daemons parseable.
    #[serde(default)]
    pub config: Option<RunningConfigInfo>,
    /// What the daemon is doing right now (see [`SyncActivity`]); `None` when idle or from an
    /// older daemon (`#[serde(default)]` keeps both directions of the wire compatible).
    #[serde(default)]
    pub activity: Option<SyncActivity>,
    /// Items whose action failed during the last pass, **bounded** to the first
    /// `daemon::FAILED_ITEMS_REPORTED` in plan order (#136). `failed_item_count` is the untruncated
    /// total, so `failed_items.len() <= failed_item_count` always. A non-zero count is the wire form
    /// of the partial-failure pass outcome: some items failed, everything else synced.
    /// `#[serde(default)]` for replies from older daemons.
    #[serde(default)]
    pub failed_items: Vec<FailedItem>,
    /// How many items failed in the last pass, counting past the bound on `failed_items`.
    #[serde(default)]
    pub failed_item_count: usize,
    /// Entities the engine cannot sync and never will without user action (see
    /// [`UnsyncableItem`]) — a skipped entity used to be nothing but an anonymous
    /// `skipped_unsupported` counter, so a cloud-only file could sit unsynced for months with
    /// nothing naming it (#295). Display-only: like [`SyncActivity`], nothing here participates in
    /// any sync decision — the planner re-derives every skip from ground truth. `#[serde(default)]`
    /// so a reply from an older daemon still parses.
    #[serde(default)]
    pub unsyncable: Vec<UnsyncableItem>,
    /// Pass-level history (see [`PassHistory`]). `None` means **an older daemon, or this one could
    /// not read its history** — `Daemon::with_client` publishes it at construction, before the
    /// first pass, and a daemon with no recorded passes publishes `Some` with empty fields. So
    /// "nothing has run yet" is `Some`, and `None` is never a transient a client should wait out:
    /// `refresh_pass_history` reaching its `Err` arm warns and leaves this `None`, so a client that
    /// renders it as "waiting for the first pass" would say that forever over an unreadable
    /// database. `#[serde(default)]` keeps both wire directions compatible.
    #[serde(default)]
    pub history: Option<PassHistory>,
    /// The answer to a [`ControlCommand::Activity`] request, and `None` on every other reply.
    #[serde(default)]
    pub file_history: Option<crate::index::FileHistory>,
    /// How many files the index tracks and how many bytes they come to (#207) — the *corpus* size,
    /// as distinct from `pending_changes`, which is work outstanding.
    ///
    /// Counts **files only**: `file_index` stores directories as rows too, so a count including
    /// them would describe no set a user recognises.
    ///
    /// Read straight from the published snapshot, so a status reply stays O(1) however large the
    /// index grows; the daemon recomputes it at most once per pass, and only after a pass whose
    /// plan could have mutated the index. `None` = not computed yet, or a reply from a daemon
    /// predating the field — distinct from `Some(IndexTotals { files: 0, bytes: 0 })`, which is a
    /// genuinely empty index. `#[serde(default)]` keeps both directions of the wire compatible.
    #[serde(default)]
    pub index_totals: Option<IndexTotals>,
    /// The answer to a [`ControlCommand::List`] request, and `None` on every other reply.
    #[serde(default)]
    pub listing: Option<ListingOutcome>,
    /// The answer to a [`ControlCommand::Plan`] / [`ControlCommand::PlanResult`] request, and
    /// `None` on every other reply — including `status`, deliberately: a reviewed plan can be tens
    /// of thousands of rows and the GUI polls status every two seconds.
    #[serde(default)]
    pub plan: Option<PlanOutcome>,
    /// The answer to a [`ControlCommand::Apply`] request, and — on a
    /// [`ControlCommand::PlanResult`] reply — the verdict the last apply pass left behind. `None`
    /// on every other reply.
    #[serde(default)]
    pub apply: Option<ApplyOutcome>,
    /// What the daemon believes about the Proton session (see [`AuthState`]). Rides on **every**
    /// reply, not just `status`: a `list` that is refused for an expired session is the fastest
    /// evidence there is, and the client that asked for it is the one that most needs to know.
    /// `#[serde(default)]` gives a reply from an older daemon [`AuthState::Unknown`], which is the
    /// honest reading — that daemon classified nothing.
    #[serde(default)]
    pub auth: AuthState,
}

/// One completed reconcile **attempt**, including the idle ones — a rolling debug trail of the
/// last twenty passes (`daemon::STATUS_HISTORY_LIMIT`), persisted to `<db>.status.json`.
///
/// Deliberately carries no duration, kind or outcome, though #229/#238 proposed it as their home:
/// with event-driven detection on by default the daemon runs a pass every 30s and records every
/// one here, so twenty entries are about ten minutes of wall clock. That window can answer neither
/// "how long did each of the last 20 passes take" (they are twenty idle polls) nor "when was the
/// last full sweep" (two days is four thousand entries ago). Both live on [`PassHistory`], which is
/// backed by the durable, idle-filtered `sync_passes` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusHistoryEntry {
    pub epoch_secs: u64,
    pub message: String,
    pub last_error: Option<String>,
    pub plan_summary: Option<PlanSummary>,
    pub successful_sync_summary: Option<PlanSummary>,
    /// How many items failed in this pass (#136); `0` for a clean or wholly-failed pass, so a
    /// history reader can tell a partial pass from either. `#[serde(default)]` for entries written
    /// by an older daemon (the sidecar is read back across upgrades).
    #[serde(default)]
    pub failed_item_count: usize,
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
    // #62: `bind` creates the socket with `0777 & ~umask`, and a chmod at the published path lands
    // only afterwards — with a permissive umask another local user can connect in that window and
    // issue any control command (`approve --all` authorises every withheld deletion). Bind inside a
    // freshly created owner-only directory instead: there the socket is unreachable to anyone else
    // whatever its own mode, so the loose-mode window is not observable, and the rename into place
    // preserves the binding (the inode is what is bound; clients resolve the published name to it).
    // This is the "land via `fs::rename` from a staging dir" shape the path-safety invariant
    // already requires for a write to a predictable name. Deliberately NOT a process-wide
    // `libc::umask` flip (the fix the issue proposed): this runs on a multithreaded runtime
    // alongside the startup sidecar/DB writes, so it would trade this race for a worse one.
    let staging = StagingDir::create(socket_path)?;
    let staged_socket = staging.socket_path();
    // The staged path is LONGER than the published one, so a configured socket that fits
    // `sun_path` on its own can overflow once staged. `bind`'s own error ("path must be shorter
    // than SUN_LEN") would name a path the user never configured and give no hint that staging is
    // why, so check here and say all three things: which socket, how long staged, what the limit
    // is. Binding in place instead is not an option — that is the vulnerability.
    let capacity = sun_path_capacity();
    if staged_socket.as_os_str().len() >= capacity {
        return Err(crate::boxed_error(format!(
            "failed to bind control socket {}: it is bound in a private staging directory and \
             renamed into place (so it is never briefly world-connectable), and that staged path \
             is {} bytes — past this platform's {}-byte limit for a Unix socket path. Use a \
             shorter socket_path, or one in a shorter directory.",
            socket_path.display(),
            staged_socket.as_os_str().len(),
            capacity - 1
        )));
    }
    // Errors name the PUBLISHED path: the staging directory is an implementation detail, and a
    // `sun_path`-too-long or permission failure is about the socket the user configured.
    let listener = UnixListener::bind(&staged_socket).map_err(|error| {
        crate::boxed_error(format!(
            "failed to bind control socket {}: {error}",
            socket_path.display()
        ))
    })?;
    std::fs::set_permissions(&staged_socket, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&staged_socket, socket_path).map_err(|error| {
        crate::boxed_error(format!(
            "failed to publish control socket at {}: {error}",
            socket_path.display()
        ))
    })?;
    Ok(listener)
}

/// Maximum `.psb-<pid>-<n>` names tried before giving up. Two things leave one behind: a SIGKILL
/// that skips [`Drop`], and — by design — a `Drop` that ran and *refused*, because it removes only
/// an empty directory and another local user planted something inside (see the `Drop` impl). Either
/// way the loop steps **past** a taken name rather than clearing it, so a leftover costs one attempt
/// and a start fails only if a reused pid meets this many of them.
#[cfg(unix)]
const STAGING_DIR_ATTEMPTS: u64 = 16;
/// Fixed, short staged socket name. Both this and the staging directory name are charged to the
/// platform's `sockaddr_un.sun_path` budget on top of the published path, so neither carries
/// anything beyond what uniqueness needs. **Nothing validates that budget ahead of `bind`** —
/// `paths::default_socket_path` just joins, and the XDG default happens to land far inside it — so
/// [`bind_listener`] measures the staged path itself rather than assuming it fits.
#[cfg(unix)]
const STAGED_SOCKET_NAME: &str = "s";

/// Bytes the platform gives a Unix socket path, NUL terminator included. Read off the platform's
/// own `sockaddr_un` rather than hardcoded: it is 108 on Linux and 104 on some BSDs.
#[cfg(unix)]
fn sun_path_capacity() -> usize {
    std::mem::size_of::<libc::sockaddr_un>() - std::mem::offset_of!(libc::sockaddr_un, sun_path)
}

/// The private directory the control socket is bound in before being renamed to its published
/// path (see [`bind_listener`]). Cleaned up on drop, so every failure path cleans up — but only
/// ever its own, empty self (see the `Drop` impl).
#[cfg(unix)]
struct StagingDir {
    directory: PathBuf,
}

#[cfg(unix)]
impl StagingDir {
    /// Created as a sibling of `socket_path` so the rename is same-directory (hence atomic and
    /// never cross-device). `DirBuilder::mode(0o700)` is umask-proof: mkdir's umask can only
    /// *clear* permission bits, never add them, so the directory is owner-only from the instant it
    /// exists, and `mkdir` never follows a symlink planted at the name — a leftover is retried
    /// past, never removed. Together with the non-recursive [`Drop`], nothing here can delete
    /// anything this process did not put there.
    fn create(socket_path: &Path) -> AppResult<Self> {
        use std::os::unix::fs::DirBuilderExt;

        let parent = socket_path.parent().ok_or_else(|| {
            crate::boxed_error(format!(
                "refusing to bind control socket: {} has no parent directory",
                socket_path.display()
            ))
        })?;
        // Unique per process *and* per call: one process binds more than one listener over its
        // lifetime (and many in the test suite), and a reused name would collide.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        let mut last_error = None;
        for _ in 0..STAGING_DIR_ATTEMPTS {
            let directory = parent.join(format!(
                ".psb-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match builder.create(&directory) {
                Ok(()) => return Ok(Self { directory }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_error = Some(error);
                }
                // Names the SOCKET as well as the staging directory: the commonest cause is a
                // parent that does not exist, which used to surface as a plain bind failure on
                // the configured path and must stay as recognisable.
                Err(error) => {
                    return Err(crate::boxed_error(format!(
                        "failed to bind control socket {}: could not create its staging directory \
                         {}: {error}",
                        socket_path.display(),
                        directory.display()
                    )));
                }
            }
        }
        Err(crate::boxed_error(format!(
            "failed to bind control socket {}: {} staging directory names under {} were already \
             taken (last: {})",
            socket_path.display(),
            STAGING_DIR_ATTEMPTS,
            parent.display(),
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown".to_owned())
        )))
    }

    fn socket_path(&self) -> PathBuf {
        self.directory.join(STAGED_SOCKET_NAME)
    }
}

#[cfg(unix)]
impl Drop for StagingDir {
    /// Deliberately **not** `remove_dir_all`. In the very deployment #62 is about — a socket path
    /// under a directory other local users can write — an attacker can remove this name after
    /// `create` made it and leave their own tree at it, and a recursive delete would then take
    /// that tree. `remove_dir` refuses a non-empty directory (`ENOTEMPTY`) and refuses a symlink
    /// (`ENOTDIR`), so the only thing it can ever remove is the empty directory this process
    /// created. The staged socket is normally already renamed away by now; removing it first
    /// covers the path where the bind succeeded and the rename did not.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.socket_path());
        let _ = std::fs::remove_dir(&self.directory);
    }
}

/// Sends a no-argument control command. Thin wrapper over [`send_request`] kept for the commands
/// that carry no argument (`status`/`pause`/`resume`/`syncnow`).
#[cfg(unix)]
pub async fn send_command(
    socket_path: &Path,
    command: ControlCommand,
) -> AppResult<ControlResponse> {
    send_request(socket_path, ControlRequest::new(command)).await
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
    use crate::sync::UnsyncableReason;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn the_reset_index_command_has_a_stable_wire_value() {
        // The container is `rename_all = "lowercase"`, which would spell this `resetindex`. The
        // explicit rename is the wire contract, and an older daemon rejecting it as unknown is the
        // documented behaviour — so the string must not drift.
        let request = ControlRequest::new(ControlCommand::ResetIndex);
        let encoded = serde_json::to_string(&request).expect("encode");
        assert!(
            encoded.contains(r#""command":"reset_index""#),
            "unexpected wire form: {encoded}"
        );
        let decoded: ControlRequest =
            serde_json::from_str(r#"{"command":"reset_index"}"#).expect("decode");
        assert_eq!(decoded.command, ControlCommand::ResetIndex);
    }

    #[test]
    fn control_request_without_literal_path_parses_as_legacy() {
        // A request from an older client omits `literal_path`; it must parse with the flag off,
        // preserving the historical case-insensitive "all" interpretation for that client.
        let legacy = r#"{"command":"approve","argument":"all"}"#;
        let request: ControlRequest =
            serde_json::from_str(legacy).expect("legacy request must parse");
        assert_eq!(request.argument.as_deref(), Some("all"));
        assert!(!request.literal_path);
    }

    #[test]
    fn a_pending_deletion_from_an_older_daemon_still_parses() {
        // The lifecycle fields (#225 first-seen, #208 subtree totals) are additions to a shape an
        // older daemon already emits, so they default rather than fail the whole reply. `0` and
        // `None` are the honest readings of "this daemon does not know", which is why a client
        // must not age anything from a zero.
        let legacy = r#"{
            "path": "photos/2019",
            "direction": "local",
            "entity_kind": "directory",
            "fingerprint": "vol~node",
            "detected_epoch_secs": 42
        }"#;
        let pending: PendingDeletion =
            serde_json::from_str(legacy).expect("legacy pending deletion must parse");
        assert_eq!(pending.detected_epoch_secs, 42);
        assert_eq!(pending.first_seen_epoch_secs, 0);
        assert_eq!((pending.subtree_files, pending.subtree_bytes), (None, None));
    }

    #[test]
    fn a_keep_request_names_a_command_an_older_client_never_sent() {
        // Wire-visible names, pinned: `keep` is the refusal (#224) and `direction` rides along for
        // `approve`'s pre-pass form (#227). Both are additive — an older client omits `direction`
        // and never sends `keep`.
        let json = serde_json::to_string(&ControlRequest {
            argument: Some("photos/2019".to_owned()),
            literal_path: true,
            ..ControlRequest::new(ControlCommand::Keep)
        })
        .expect("serialize");
        assert!(json.contains(r#""command":"keep""#), "{json}");

        let approve: ControlRequest = serde_json::from_str(
            r#"{"command":"approve","argument":"a.txt","direction":"remote"}"#,
        )
        .expect("parse");
        assert_eq!(approve.direction, Some(DeleteDirection::Remote));
    }

    #[test]
    fn the_list_command_has_a_stable_wire_value_and_carries_a_plain_path() {
        // Wire-visible name, pinned. An older daemon rejecting `list` as an unknown command is the
        // documented behaviour (as for `resync`/`keep`/`activity`), so the string must not drift.
        let request = ControlRequest {
            argument: Some("photos/2019".to_owned()),
            limit: Some(20),
            ..ControlRequest::new(ControlCommand::List)
        };
        let encoded = serde_json::to_string(&request).expect("encode");
        assert!(encoded.contains(r#""command":"list""#), "{encoded}");

        let decoded: ControlRequest =
            serde_json::from_str(r#"{"command":"list","argument":"a/b"}"#).expect("decode");
        assert_eq!(decoded.command, ControlCommand::List);
        assert_eq!(decoded.argument.as_deref(), Some("a/b"));
        // No reserved word, so no `literal_path` to set: a folder named `all` is just a folder.
        assert!(!decoded.literal_path);
    }

    #[test]
    fn the_plan_family_commands_have_stable_wire_values() {
        // Wire-visible names, pinned. `plan_result` needs the explicit rename for the same reason
        // `reset_index` does — the container is `rename_all = "lowercase"`, which would spell it
        // `planresult`.
        for (command, wire) in [
            (ControlCommand::Plan, r#""command":"plan""#),
            (ControlCommand::PlanResult, r#""command":"plan_result""#),
            (ControlCommand::Apply, r#""command":"apply""#),
        ] {
            let encoded =
                serde_json::to_string(&ControlRequest::new(command.clone())).expect("encode");
            assert!(encoded.contains(wire), "{encoded}");
        }
        let decoded: ControlRequest = serde_json::from_str(
            r#"{"command":"apply","argument":"1:abc","skip_destructive":true}"#,
        )
        .expect("decode");
        assert_eq!(decoded.command, ControlCommand::Apply);
        assert_eq!(decoded.argument.as_deref(), Some("1:abc"));
        assert!(decoded.skip_destructive);
        // An older client omits the flag, and omitting it means "run the whole plan".
        let legacy: ControlRequest =
            serde_json::from_str(r#"{"command":"apply","argument":"1:abc"}"#).expect("decode");
        assert!(!legacy.skip_destructive);
    }

    #[test]
    fn a_plan_outcome_round_trips_and_an_unknown_state_is_not_an_empty_plan() {
        let computed = PlanOutcome::Computed(Box::new(ReviewedPlan {
            plan_seq: 3,
            token: "1:abc".to_owned(),
            computed_epoch_secs: 42,
            summary: PlanSummary::default(),
            actions: Vec::new(),
            total: 0,
            truncated: false,
            cannot_sync: Vec::new(),
        }));
        let json = serde_json::to_string(&computed).expect("serialize");
        assert!(json.contains(r#""state":"computed""#), "{json}");
        assert_eq!(
            serde_json::from_str::<PlanOutcome>(&json).expect("round trip"),
            computed
        );
        assert_eq!(
            serde_json::to_string(&PlanOutcome::Paused).expect("serialize"),
            r#"{"state":"paused"}"#
        );

        // A state a newer daemon invented degrades rather than failing the reply — and critically
        // does NOT land on `computed` with no rows, which a client would draw as "nothing to do".
        let future: PlanOutcome =
            serde_json::from_str(r#"{"state":"queued-behind-a-sync","eta":30}"#)
                .expect("an unknown state must parse");
        assert_eq!(future, PlanOutcome::Unknown);
    }

    #[test]
    fn an_apply_outcome_round_trips_and_an_unknown_state_is_never_applied() {
        let applied = ApplyOutcome::Applied {
            apply_seq: 2,
            executed: 7,
            skipped_destructive: 1,
            failed: 0,
        };
        let json = serde_json::to_string(&applied).expect("serialize");
        assert!(json.contains(r#""state":"applied""#), "{json}");
        assert_eq!(
            serde_json::from_str::<ApplyOutcome>(&json).expect("round trip"),
            applied
        );
        assert_eq!(
            serde_json::to_string(&ApplyOutcome::Stale).expect("serialize"),
            r#"{"state":"stale"}"#
        );
        let future: ApplyOutcome =
            serde_json::from_str(r#"{"state":"queued"}"#).expect("an unknown state must parse");
        assert_eq!(future, ApplyOutcome::Unknown);
        assert!(
            !matches!(future, ApplyOutcome::Applied { .. }),
            "an unrecognised verdict must never read as a completed apply"
        );
    }

    #[test]
    fn a_status_reply_carries_no_plan_and_an_older_daemons_reply_still_parses() {
        // The plan rides only on the plan-family replies: a reviewed plan can be tens of thousands
        // of rows and the GUI polls `status` every two seconds.
        let legacy = r#"{
            "status": "running",
            "paused": false,
            "pending_changes": 0,
            "message": "daemon status",
            "last_sync_epoch_secs": null,
            "last_error": null,
            "last_plan_summary": null,
            "last_successful_sync_summary": null,
            "status_history": []
        }"#;
        let response: ControlResponse =
            serde_json::from_str(legacy).expect("legacy reply must parse");
        assert!(response.plan.is_none());
        assert!(response.apply.is_none());
    }

    #[test]
    fn a_listing_outcome_round_trips_and_an_unknown_state_is_not_an_empty_folder() {
        let listed = ListingOutcome::Listed {
            path: PathBuf::from("photos"),
            entries: vec![RemoteEntry {
                path: PathBuf::from("photos/a.jpg"),
                name: "a.jpg".to_owned(),
                entity_kind: EntityKind::File,
                sha1: Some("abc".to_owned()),
                downloadable: true,
            }],
            total: 1,
            truncated: false,
        };
        let json = serde_json::to_string(&listed).expect("serialize");
        assert!(json.contains(r#""state":"listed""#), "{json}");
        assert_eq!(
            serde_json::from_str::<ListingOutcome>(&json).expect("round trip"),
            listed
        );
        assert_eq!(
            serde_json::to_string(&ListingOutcome::Busy).expect("serialize"),
            r#"{"state":"busy"}"#
        );

        // A state a newer daemon invented must degrade, not fail the whole reply — and must not
        // land on `Listed` with no entries, which a client would draw as an empty folder.
        let future: ListingOutcome =
            serde_json::from_str(r#"{"state":"rate-limited","retry_after":30}"#)
                .expect("an unknown state must parse");
        assert_eq!(future, ListingOutcome::Unknown);
    }

    #[test]
    fn an_auth_state_is_an_open_token_and_an_unknown_one_is_no_verdict() {
        // #103: the wire form is a hand-written token so an unrecognised value degrades to
        // `Unknown` instead of failing an older client's whole reply — and, critically, instead of
        // defaulting to "signed in and fine".
        for state in [
            AuthState::Unknown,
            AuthState::SignedIn,
            AuthState::SignedOut,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            assert_eq!(json, format!("\"{}\"", state.as_str()));
            assert_eq!(
                serde_json::from_str::<AuthState>(&json).expect("round trip"),
                state
            );
        }
        assert_eq!(
            serde_json::from_str::<AuthState>(r#""needs-2fa""#).expect("unknown token"),
            AuthState::Unknown,
            "an unrecognised verdict is not a verdict"
        );
        assert_eq!(AuthState::default(), AuthState::Unknown);
    }

    #[test]
    fn a_reply_from_a_daemon_that_predates_both_fields_parses_as_no_listing_and_no_verdict() {
        // Both are additive. An older daemon classified nothing, so `Unknown` is the honest
        // reading of its silence — and it sends no listing at all, which is not an empty folder.
        let legacy = r#"{
            "status": "running",
            "paused": false,
            "pending_changes": 0,
            "message": "daemon status",
            "last_sync_epoch_secs": null,
            "last_error": null,
            "last_plan_summary": null,
            "last_successful_sync_summary": null,
            "status_history": []
        }"#;
        let response: ControlResponse =
            serde_json::from_str(legacy).expect("legacy reply must parse");
        assert!(response.listing.is_none());
        assert_eq!(response.auth, AuthState::Unknown);
    }

    #[test]
    fn control_response_without_activity_still_parses() {
        // A reply from an older daemon carries no `activity` field; a newer client must parse
        // it (as None) rather than error — the same guarantee the other `#[serde(default)]`
        // fields give.
        let legacy = r#"{
            "status": "running",
            "paused": false,
            "pending_changes": 0,
            "message": "daemon status",
            "last_sync_epoch_secs": null,
            "last_error": null,
            "last_plan_summary": null,
            "last_successful_sync_summary": null,
            "status_history": []
        }"#;
        let response: ControlResponse =
            serde_json::from_str(legacy).expect("legacy reply must parse");
        assert!(response.activity.is_none());
        assert!(!response.syncing);
    }

    #[test]
    fn sync_activity_round_trips_and_partial_json_defaults() {
        let activity = SyncActivity {
            phase: "executing".to_owned(),
            detail: Some("downloading a/b.bin".to_owned()),
            folders_listed: None,
            files_scanned: None,
            action_index: Some(3),
            action_total: Some(10),
            transfers: vec![
                TransferActivity {
                    bytes_done: Some(1024),
                    ..TransferActivity::active("download", PathBuf::from("a/b.bin"), None, 5)
                },
                TransferActivity::queued("upload", PathBuf::from("c/d.bin"), Some(64)),
            ],
            transfers_remaining: Some(9),
            transfer: Some(TransferActivity {
                bytes_done: Some(1024),
                ..TransferActivity::active("download", PathBuf::from("a/b.bin"), None, 5)
            }),
            since_epoch_secs: Some(4),
            pass: Some(PassProgress {
                started_epoch_secs: 2,
                changes: Some(3),
                kind: "incremental".to_owned(),
                uploaded_files: 4,
                downloaded_files: 7,
                uploaded_bytes: 900,
                downloaded_bytes: 12,
            }),
        };
        let json = serde_json::to_string(&activity).expect("serialize");
        let back: SyncActivity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, activity);

        // A minimal activity (just the phase) parses, every optional field defaulting — so
        // phases can grow fields without breaking older clients.
        let minimal: SyncActivity =
            serde_json::from_str(r#"{"phase": "listing-remote"}"#).expect("minimal activity");
        assert_eq!(minimal.phase, "listing-remote");
        assert!(minimal.transfer.is_none());
        assert!(minimal.folders_listed.is_none());
        assert!(minimal.pass.is_none());
    }

    #[test]
    fn the_pass_block_is_the_pass_not_the_phase() {
        // #213: `since_epoch_secs` restarts on every phase change, which is why a client rendering
        // it as the pass's elapsed time counts backwards three times. The pass block does not.
        let json = r#"{"phase":"committing","since_epoch_secs":900,
                       "pass":{"started_epoch_secs":100,"kind":"full-sweep"}}"#;
        let activity: SyncActivity = serde_json::from_str(json).expect("activity with a pass");
        let pass = activity.pass.expect("pass block");
        assert_eq!(pass.started_epoch_secs, 100);
        assert!(activity.since_epoch_secs.unwrap() > pass.started_epoch_secs);
        // Unknown is not zero: a client omits the clause rather than printing "0 changes".
        assert_eq!(pass.changes, None);
        // …and neither are the per-direction counters, which an older daemon simply does not send.
        assert_eq!(pass.uploaded_files, 0);
        assert_eq!(pass.downloaded_bytes, 0);
    }

    #[test]
    fn a_reply_from_a_daemon_predating_the_transfer_list_still_parses() {
        // The wire-compat direction that matters for a new client: an older daemon sends the
        // singular `transfer` with a bare `started_epoch_secs`, no `state`, and no list at all.
        let json = r#"{"phase":"executing","action_index":3,"action_total":10,
                       "transfer":{"direction":"upload","path":"a/b.bin",
                                   "bytes_total":64,"started_epoch_secs":7}}"#;
        let activity: SyncActivity = serde_json::from_str(json).expect("legacy activity");
        assert!(activity.transfers.is_empty());
        assert_eq!(
            activity.transfers_remaining, None,
            "unknown, and a client must not read it as `nothing is moving`"
        );
        // A row with no `state` is active: that is what the field meant before it existed.
        let transfer = activity
            .active_transfer()
            .expect("the mirror is the in-flight transfer for a client that knows both shapes");
        assert!(transfer.is_active());
        assert_eq!(transfer.started_epoch_secs, Some(7));
        assert_eq!(transfer.files, None);
    }

    #[test]
    fn the_window_is_named_rows_plus_a_count_that_outlives_them() {
        // `+n more` is the daemon's figure, not `transfers.len()`: the window is capped and a
        // batched row stands for its whole chunk. 25 in flight + 2 named + 91 unnamed = 118.
        let mut activity = SyncActivity {
            phase: "executing".to_owned(),
            transfers: vec![
                TransferActivity {
                    files: Some(25),
                    ..TransferActivity::active("download", PathBuf::from("photos/2024"), None, 1)
                },
                TransferActivity::queued("upload", PathBuf::from("a.txt"), Some(10)),
                TransferActivity::queued("download", PathBuf::from("b.txt"), None),
            ],
            transfers_remaining: Some(118),
            ..blank()
        };
        assert_eq!(activity.transfers_past_the_window(), 91);
        assert_eq!(activity.queued_transfers().count(), 2);

        // The mirror is the first ACTIVE row, never simply the first row, and it is derived rather
        // than stored — the only writer is this call.
        assert_eq!(activity.transfer, None);
        activity.derive_legacy_transfer();
        let mirror = activity.transfer.as_ref().expect("mirror");
        assert_eq!(mirror.path, PathBuf::from("photos/2024"));
        assert!(
            mirror.started_epoch_secs.is_some(),
            "an older client deserializes this into a non-optional u64, so a null would fail its \
             whole reply"
        );

        // A window holding only queued rows has no mirror — and that is not "nothing is moving",
        // which `transfers_remaining` is what answers.
        let mut queued_only = SyncActivity {
            phase: "executing".to_owned(),
            transfers: vec![TransferActivity::queued(
                "upload",
                PathBuf::from("c.txt"),
                None,
            )],
            transfers_remaining: Some(1),
            ..blank()
        };
        queued_only.derive_legacy_transfer();
        assert_eq!(queued_only.transfer, None);
        assert_eq!(queued_only.active_transfer(), None);
        assert_eq!(queued_only.transfers_past_the_window(), 0);
    }

    fn blank() -> SyncActivity {
        SyncActivity {
            phase: String::new(),
            detail: None,
            folders_listed: None,
            files_scanned: None,
            action_index: None,
            action_total: None,
            transfers: Vec::new(),
            transfers_remaining: None,
            transfer: None,
            since_epoch_secs: None,
            pass: None,
        }
    }

    /// A path whose bytes are not valid UTF-8 (the engine supports them: `index_key` is a BLOB).
    fn non_utf8_path(suffix: &[u8]) -> PathBuf {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = b"bad-\xff".to_vec();
        bytes.extend_from_slice(suffix);
        PathBuf::from(OsStr::from_bytes(&bytes))
    }

    fn response_with_non_utf8_paths() -> ControlResponse {
        ControlResponse {
            status: "running".to_owned(),
            paused: false,
            syncing: true,
            reconcile_seq: 7,
            pending_changes: 0,
            message: "daemon status".to_owned(),
            last_sync_epoch_secs: None,
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: Vec::new(),
            pending_deletions: vec![PendingDeletion {
                path: non_utf8_path(b".txt"),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: "hash".to_owned(),
                detected_epoch_secs: 1,
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
            }],
            failed_items: vec![FailedItem {
                path: non_utf8_path(b"-failed.txt"),
                action: SyncAction::Upload,
                error: "upload failed".to_owned(),
            }],
            failed_item_count: 1,
            config: Some(RunningConfigInfo {
                local_root: non_utf8_path(b"-root"),
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                db_path: non_utf8_path(b"-root/.sync/sync_index.db"),
            }),
            activity: Some(SyncActivity {
                phase: "executing".to_owned(),
                detail: None,
                folders_listed: None,
                files_scanned: None,
                action_index: Some(1),
                action_total: Some(1),
                transfers: vec![
                    TransferActivity::active("download", non_utf8_path(b".bin"), None, 2),
                    TransferActivity::queued("upload", non_utf8_path(b"-queued.bin"), Some(3)),
                ],
                transfers_remaining: Some(2),
                transfer: Some(TransferActivity::active(
                    "download",
                    non_utf8_path(b".bin"),
                    None,
                    2,
                )),
                since_epoch_secs: None,
                pass: Some(PassProgress {
                    started_epoch_secs: 1,
                    changes: Some(2),
                    kind: "warm-start".to_owned(),
                    uploaded_files: 0,
                    downloaded_files: 1,
                    uploaded_bytes: 0,
                    downloaded_bytes: 8,
                }),
            }),
            unsyncable: vec![UnsyncableItem {
                path: non_utf8_path(b"-doc"),
                entity_kind: EntityKind::File,
                reason: UnsyncableReason::RemoteNotDownloadable,
                first_seen_epoch_secs: 3,
            }],
            history: Some(PassHistory {
                recent: vec![pass_record()],
                last_full_sweep: Some(pass_record()),
                today: crate::index::ByteTotals {
                    since_epoch_secs: 0,
                    uploaded_bytes: 10,
                    downloaded_bytes: 20,
                },
            }),
            // Both of a `FileEvent`'s paths are lossy wire paths, so they belong in this fixture
            // too — a new PathBuf field that skips it is exactly how #61 happened.
            file_history: Some(crate::index::FileHistory {
                events: vec![crate::index::FileEvent {
                    path: non_utf8_path(b"-moved"),
                    source_path: Some(non_utf8_path(b"-original")),
                    action: SyncAction::MoveLocal,
                    bytes: None,
                    epoch_secs: 4,
                    pass_id: 1,
                }],
                total: 1,
                files: 1,
                totals: Some(crate::index::ByteTotals {
                    since_epoch_secs: 0,
                    uploaded_bytes: 0,
                    downloaded_bytes: 0,
                }),
            }),
            index_totals: Some(IndexTotals {
                files: 12_480,
                bytes: 41_200_000_000,
            }),
            // A listing is all paths, so it belongs in this fixture more than anything else on the
            // reply does — a browse of a folder holding a non-UTF-8 name must not fail the reply.
            listing: Some(ListingOutcome::Listed {
                path: non_utf8_path(b"-folder"),
                entries: vec![RemoteEntry {
                    path: non_utf8_path(b"-folder/bad.bin"),
                    name: "bad.bin".to_owned(),
                    entity_kind: EntityKind::File,
                    sha1: None,
                    downloadable: true,
                }],
                total: 1,
                truncated: false,
            }),
            // A plan is nothing but paths, and #270 guarantees a non-UTF-8 one is *planned* rather
            // than dropped — so a reviewed plan holding one must not fail the reply that carries it
            // (the #300 rule, one layer up).
            plan: Some(PlanOutcome::Computed(Box::new(ReviewedPlan {
                plan_seq: 1,
                token: "1:abc".to_owned(),
                computed_epoch_secs: 5,
                summary: PlanSummary::default(),
                actions: vec![crate::sync::PlannedAction {
                    path: non_utf8_path(b"-planned.txt"),
                    destination_path: Some(non_utf8_path(b"-moved-to.txt")),
                    action: SyncAction::MoveLocal,
                    entity_kind: EntityKind::File,
                    conflict_path: None,
                    remote_id: None,
                    sidecar_from_local_copy: false,
                    skip_reason: None,
                    size_bytes: None,
                }],
                total: 1,
                truncated: false,
                cannot_sync: vec![crate::index::UnsyncableEntry {
                    relative_path: non_utf8_path(b"-socket"),
                    reason: UnsyncableReason::LocalSocket,
                }],
            }))),
            apply: Some(ApplyOutcome::Diverged { apply_seq: 1 }),
            auth: AuthState::SignedOut,
        }
    }

    fn pass_record() -> crate::index::PassRecord {
        crate::index::PassRecord {
            id: 1,
            started_epoch_secs: 100,
            duration_ms: 250,
            kind: "full-sweep".to_owned(),
            outcome: "clean".to_owned(),
            changed: 0,
            failed: 0,
            bytes_uploaded: 0,
            bytes_downloaded: 0,
            error: None,
        }
    }

    #[test]
    fn a_non_utf8_path_anywhere_still_serializes_the_whole_response() {
        // #61: `impl Serialize for Path` ERRORS on non-UTF-8, and every PathBuf here rides on
        // every reply — one such path used to fail `write_response` for status/pause/approve
        // alike, locking the control plane out until the pending item cleared.
        let response = response_with_non_utf8_paths();
        let json =
            serde_json::to_string(&response).expect("a non-UTF-8 path must not fail a reply");
        assert!(
            json.contains('\u{fffd}'),
            "the wire form must be the lossy rendering: {json}"
        );

        let back: ControlResponse = serde_json::from_str(&json).expect("reply parses back");
        assert_eq!(
            back.pending_deletions[0].path,
            PathBuf::from(&*wire_path(&response.pending_deletions[0].path)),
            "a client sees exactly the lossy form the daemon published"
        );
        assert_eq!(
            back.failed_items[0].path,
            PathBuf::from(&*wire_path(&response.failed_items[0].path)),
            "a failed item's path is lossy on the wire like every other path"
        );
        assert_eq!(
            back.config.expect("config").local_root,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-root")))
        );
        // #295's list rides the same reply, so it is bound by the same rule: a non-UTF-8 path is
        // exactly the kind of entity that lands on it.
        assert_eq!(
            back.unsyncable[0].path,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-doc")))
        );
        // A history event carries TWO paths, and a move's source is the one a per-path feed is
        // most likely to hold a non-UTF-8 value for.
        let event = &back.file_history.expect("file history").events[0];
        assert_eq!(
            event.path,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-moved")))
        );
        assert_eq!(
            event.source_path,
            Some(PathBuf::from(&*wire_path(&non_utf8_path(b"-original"))))
        );
        // The reviewed plan is the newest wire surface made of paths, and both halves of it — the
        // rows and the walk's own drops (#315) — go through the same rendering.
        let PlanOutcome::Computed(plan) = back.plan.expect("plan") else {
            panic!("the plan outcome must survive the round trip as `computed`");
        };
        let (actions, cannot_sync) = (&plan.actions, &plan.cannot_sync);
        assert_eq!(
            actions[0].path,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-planned.txt")))
        );
        assert_eq!(
            actions[0].destination_path,
            Some(PathBuf::from(&*wire_path(&non_utf8_path(b"-moved-to.txt"))))
        );
        assert_eq!(
            cannot_sync[0].relative_path,
            PathBuf::from(&*wire_path(&non_utf8_path(b"-socket")))
        );
    }

    #[test]
    fn wire_path_is_the_form_both_ends_match_on() {
        // The approve/deny selector a client can send is the lossy string it received; a daemon
        // that compared it against the real PathBuf could never match one (#61).
        let real = non_utf8_path(b".txt");
        assert_ne!(real, PathBuf::from(&*wire_path(&real)));
        assert_eq!(wire_path(&real), real.to_string_lossy());
        // UTF-8 paths are untouched, so nothing about the ordinary case changes.
        assert_eq!(wire_path(Path::new("a/b.txt")), "a/b.txt");
    }

    #[tokio::test]
    async fn the_socket_is_never_created_in_place_at_the_published_path() {
        // #62: `bind` creates the socket with `0777 & ~umask` and the chmod lands after, so a
        // permissive umask leaves a window where any local user can connect and issue commands.
        // The window is not observable by reading the mode afterwards (both orderings end at
        // 0600), so watch the parent directory instead: the socket must ARRIVE at the published
        // path already configured (a rename), never be CREATED there.
        use notify::{EventKind, RecursiveMode, Watcher};

        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .expect("watcher");
        watcher
            .watch(directory.path(), RecursiveMode::NonRecursive)
            .expect("watch the socket's parent directory");

        let listener = bind_listener(&socket_path).await.expect("bind listener");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut arrivals = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(event)) => {
                    if event.paths.iter().any(|path| path == &socket_path) {
                        arrivals.push(event.kind);
                    }
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    if !arrivals.is_empty() {
                        break;
                    }
                }
            }
        }
        drop(watcher);
        drop(listener);

        assert!(
            !arrivals.is_empty(),
            "no filesystem event for the socket path was observed — the test would pass vacuously"
        );
        assert!(
            !arrivals
                .iter()
                .any(|kind| matches!(kind, EventKind::Create(_))),
            "the socket must be renamed into place, not created there with the process umask: \
             {arrivals:?}"
        );
    }

    #[tokio::test]
    async fn a_socket_path_that_fits_only_unstaged_is_refused_with_the_reason() {
        // Copilot review: staging makes the bound path LONGER than the configured one, so a socket
        // path that fits `sun_path` on its own can overflow once staged. `bind`'s own error names
        // the staged path — which the user never configured — and never mentions staging, so the
        // check is here. Binding in place instead would be the vulnerability #62 is about.
        let directory = tempdir().expect("tempdir");
        let capacity = sun_path_capacity();
        let name = "s.sock";
        // The longest published path that still fits unstaged: any staging suffix overflows it,
        // whatever this run's pid length happens to be.
        let pad = (capacity - 1)
            .checked_sub(directory.path().as_os_str().len() + 2 + name.len())
            .expect("the temp dir must leave room to pad up to the limit");
        let padded = directory.path().join("p".repeat(pad));
        std::fs::create_dir(&padded).expect("padding directory");
        let socket_path = padded.join(name);
        assert_eq!(
            socket_path.as_os_str().len(),
            capacity - 1,
            "the published path must sit exactly at the limit, or the test proves nothing"
        );

        let error = bind_listener(&socket_path)
            .await
            .expect_err("a staged path past the limit must be refused");

        let message = error.to_string();
        assert!(
            message.contains("limit for a Unix socket path")
                && message.contains(&socket_path.display().to_string()),
            "the error must name the configured socket and the limit: {message}"
        );
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&padded)
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the refused bind must leave nothing behind: {leftovers:?}"
        );
    }

    #[test]
    fn dropping_a_staging_dir_never_deletes_a_tree_it_did_not_create() {
        // Copilot review, and it holds: `remove_dir_all` on a REAL directory deletes its whole
        // tree (only a symlink is left unfollowed). In the group-writable parent #62 is about, an
        // attacker can drop this name after `create` made it and leave their own tree at it, so a
        // recursive cleanup would take that tree. `remove_dir` refuses a non-empty directory.
        let directory = tempdir().expect("tempdir");
        let staging =
            StagingDir::create(&directory.path().join("daemon.sock")).expect("staging dir");
        let planted = staging.directory.join("attacker-tree");
        std::fs::create_dir(&planted).expect("plant a directory");
        std::fs::write(planted.join("data"), b"not ours").expect("plant a file");
        let staging_path = staging.directory.clone();

        drop(staging);

        assert!(
            planted.join("data").exists(),
            "cleanup must not recurse into content this process did not put there"
        );
        assert!(staging_path.is_dir());
        std::fs::remove_dir_all(&staging_path).expect("test cleanup");
    }

    #[tokio::test]
    async fn bind_listener_leaves_no_staging_directory_behind() {
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");

        let listener = bind_listener(&socket_path).await.expect("bind listener");
        let leftovers: Vec<PathBuf> = std::fs::read_dir(directory.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path != &socket_path)
            .collect();
        drop(listener);

        assert!(
            leftovers.is_empty(),
            "the private staging directory must be removed: {leftovers:?}"
        );
    }

    #[tokio::test]
    async fn a_socket_bound_through_staging_answers_at_the_published_path() {
        // The rename must preserve the binding: a client connecting to the published path
        // reaches the listener that was bound inside the staging directory.
        let directory = tempdir().expect("tempdir");
        let socket_path = directory.path().join("daemon.sock");
        let listener = bind_listener(&socket_path).await.expect("bind listener");

        let connect = UnixStream::connect(&socket_path).await;
        assert!(
            connect.is_ok(),
            "the renamed socket must still accept connections: {connect:?}"
        );
        drop(listener);
    }

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
