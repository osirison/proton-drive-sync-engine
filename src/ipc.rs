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
    /// [`ControlCommand::Activity`] only: cap on returned rows. `None` uses
    /// [`ACTIVITY_EVENTS_DEFAULT_LIMIT`]; the reply's `total` is untruncated either way.
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
}

/// Default and hard cap on the rows one [`ControlCommand::Activity`] reply carries. A control
/// reply is read into memory whole, so the limit is the daemon's to enforce, not the client's.
pub const ACTIVITY_EVENTS_DEFAULT_LIMIT: usize = 50;
pub const ACTIVITY_EVENTS_MAX_LIMIT: usize = 500;

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
        }
    }
}

/// The wire form of a path, and the form an `approve`/`deny` selector is matched against. One
/// definition for every wire this engine publishes — see [`crate::wire_path`], which also states
/// the rule a client may act on a lossy path under.
pub use crate::wire_path;

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
    /// queued behind them, counted past the window above. `None` outside `executing`, and from a
    /// daemon predating the field.
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
        self.transfers.iter().filter(|transfer| !transfer.is_active())
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
