use crate::dirconfig::{DirectoryConfigResolver, EffectiveSettings};
use crate::events::{EventSource, EventsClient, RemoteChange, node_uid, volume_id_from_proton_id};
use crate::index::{
    EntityKind, EventCursor, FileRecord, LocalEntityState, LocalFileState, ScanOptions, SyncStatus,
    delete_delete_approval, load_event_cursor, load_existing_index, load_index,
    load_sole_event_cursor, load_warm_start_count, local_directory_state, local_file_state,
    mark_modified, matching_delete_approval, open_database, path_for_proton_id, purge_record,
    scan_local_entities_observed, scan_local_entities_reusing_hashes, store_event_cursor,
    store_warm_start_count, upsert_delete_approval, upsert_record,
};
use crate::ipc::{
    ControlCommand, ControlResponse, PendingDeletion, RunningConfigInfo, StatusHistoryEntry,
    SyncActivity, TransferActivity, bind_listener, read_request, write_response,
};
use crate::proton::{
    CommandPolicy, DownloadRequest, ProgressSink, ProtonClient, ProtonDriveClient, RemoteEntity,
    RemoteListingStatus,
};
use crate::reconstruct::{Reconstruction, RemoteChangeResolver, reconstruct_remote};
use crate::session::{CliKeyringSession, CurlHttpTransport};
use crate::sync::{
    DeleteDirection, PlanSummary, PlannedAction, SyncAction, directory_move_descendant_path_pairs,
    is_strict_descendant, original_from_conflict_copy, plan_sync_entities,
};
use crate::{AppResult, boxed_error};
use fs2::FileExt;
use indicatif::{ProgressBar, ProgressStyle};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const STATUS_HISTORY_LIMIT: usize = 20;
/// Time budget for a single control-connection read or write. Control connections are served
/// concurrently on their own task, so a stalled client cannot block the daemon — the timeout
/// just stops silent clients from accumulating parked connection tasks forever.
const IPC_IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the daemon polls the volume event stream while event-driven detection is **live** —
/// `events_driven` on *and* an event source built. Matches the ~30s cadence Proton's own client
/// uses (ADR 0001). Only the incremental (O(changes)) path runs at this cadence: the select arm is
/// gated on `event_source.is_some()`, so a daemon degraded to snapshots (unreadable CLI session)
/// does not turn this fast poll into a full-tree walk every 30s (#50) — it reconciles on
/// `scan_interval` instead, which is also when it retries the session.
///
/// This interval is **not** a snapshot cadence, and neither is `scan_interval` in events mode
/// (#52): a `scan_interval` tick is just another incremental — usually idle — pass. The only
/// remote-tree walks in events mode are a startup bootstrap (when warm start is ineligible), an
/// event-stream fallback (no cursor / no volume / fetch error / server refresh / unresolvable
/// node / incomplete remote listing), `proton-sync resync` / `--full-walk`, the opt-in periodic
/// `events_full_scan_every`, the across-restart `warm_start_full_walk_every`, and the degraded
/// session above. A warm start instead replays the cursor and scans the local tree only.
const EVENTS_POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Dedicated `tracing` target for the per-file upload/download log lines, so operators can filter
/// or silence the (potentially high-volume, filename-bearing) transfer trace independently of the
/// daemon's other info-level lifecycle logs — e.g. `RUST_LOG=proton_drive_sync_engine::transfer=warn`
/// to suppress them, or `=debug` on just this target to isolate them.
const TRANSFER_LOG_TARGET: &str = "proton_drive_sync_engine::transfer";

/// Default number of consecutive warm starts (event-driven reconciles on the first pass after
/// boot) before the next boot forces a self-healing full-tree walk. `0` disables the periodic
/// full walk entirely (via the same `u64::MAX` sentinel as `events_full_scan_every`), leaving the
/// daemon warm-starting indefinitely — bounded only by the cursor-age gate and the event-stream
/// fallbacks. Heals across **reboots**, not within a long-running process (that is what the
/// in-run `events_full_scan_every` is for).
pub const DEFAULT_WARM_START_FULL_WALK_EVERY: u64 = 30;

/// Default maximum age of the persisted event cursor for a warm start to be attempted. Older than
/// this and the first pass full-walks instead, so a boot after long downtime cannot warm-start
/// against a cursor that may be past the server's event-retention window (which we cannot verify
/// from here). `0` disables the age gate. Note this measures the last cursor *advance*, not the
/// last successful pass — an idle volume left untouched past this window then rebooted takes an
/// unnecessary (but always safe) full walk.
pub const DEFAULT_WARM_START_MAX_CURSOR_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Startup-reconcile tuning: whether the first pass after boot may **warm-start** (an event-driven
/// reconcile that swaps the O(folders) remote walk for an O(changes) cursor replay while still
/// doing the cheap full local stat-walk to catch edits made while the daemon was down), plus the
/// two safety bounds on it and the one-shot `--full-walk` override. See `docs/adr/0004-*`.
#[derive(Debug, Clone)]
pub struct WarmStartConfig {
    /// Master switch. When `false`, the first pass after boot always full-walks (the pre-warm-start
    /// behavior, byte-for-byte).
    pub enabled: bool,
    /// Force a full walk instead of a warm start every N warm starts (across restarts). `0`
    /// disables the periodic full walk (mapped to `u64::MAX` by [`effective_full_scan_every`]).
    pub full_walk_every: u64,
    /// Warm-start only if the persisted event cursor is at most this old; otherwise full-walk.
    /// `Duration::ZERO` disables the age gate.
    pub max_cursor_age: Duration,
    /// One-shot: force this process's first pass to full-walk regardless of eligibility (the
    /// `--full-walk` startup flag). Sticky across a failed first pass so the requested full walk
    /// still happens on retry; irrelevant once the first pass succeeds.
    pub force_full_walk: bool,
}

impl Default for WarmStartConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            full_walk_every: DEFAULT_WARM_START_FULL_WALK_EVERY,
            max_cursor_age: Duration::from_secs(DEFAULT_WARM_START_MAX_CURSOR_AGE_SECS),
            force_full_walk: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    /// Per-root instance lock (`<local_root>/.sync/proton-sync.lock` by default): stops two daemons
    /// syncing the *same* root.
    pub lockfile_path: PathBuf,
    /// User-global single-instance lock (`paths::default_global_lock_path`): stops a *second*
    /// daemon anywhere for this user, because they all shell the same `proton-drive` CLI whose
    /// shared SQLite cache/session store is not concurrency-safe (`SQLITE_BUSY`; #23).
    pub global_lock_path: PathBuf,
    /// Timer cadence for an automatic reconcile. What a tick *does* depends on the mode:
    /// - events off → a full-tree snapshot (the historical meaning of this option).
    /// - events on with a live event source → another incremental (usually idle) pass, exactly
    ///   like an [`EVENTS_POLL_INTERVAL`] tick. It does **not** force a snapshot (#52): that would
    ///   reinstate the periodic full walk `events_full_scan_every = 0` disables by default.
    /// - events on with an unusable session → the snapshot cadence *and* the session-retry
    ///   cadence, because the fast event poll is gated off while degraded (#50).
    pub scan_interval: Duration,
    pub proton_cli: PathBuf,
    pub proton_timeout: Duration,
    pub proton_list_attempts: usize,
    /// Maximum planned downloads bundled into one `proton-drive filesystem download`
    /// invocation (the CLI accepts multiple remote paths per call). Consecutive planned
    /// downloads are grouped by destination directory and chunked to this size; every chunk is
    /// checkpoint-committed on landing. `1` disables batching (one subprocess per file, the
    /// pre-batching behavior).
    pub download_batch_size: usize,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    /// Detect remote changes from Proton's volume event stream (O(changes)) instead of re-walking
    /// the whole remote tree (O(folders)) every pass. **Default `true`** (`src/config.rs`); opt out
    /// with `--no-events-driven` / `events_driven = false` for the byte-identical snapshot-only
    /// path. On with an unusable CLI session the daemon degrades to snapshots on `scan_interval`
    /// (see [`EVENTS_POLL_INTERVAL`]). See `docs/adr/0001-*`.
    pub events_driven: bool,
    /// When event-driven, force a full-tree reconvergence snapshot every N incremental passes.
    /// Bounds any completeness gap inherited from a missed snapshot item or the reuse-session
    /// staleness window, and backfills `proton_id` for just-uploaded nodes. Clamped to `>= 1`.
    pub events_full_scan_every: u64,
    /// Daemon-wide default for the directional delete-approval guard — the root of the
    /// per-directory `.proton-sync.toml` inheritance chain (see `crate::dirconfig`). When `true`, a
    /// deletion propagating to Proton Drive (`RemoteDelete`) is withheld pending user approval.
    pub delete_approval_remote: bool,
    /// As `delete_approval_remote`, but for a deletion propagating to the local disk
    /// (`LocalDelete`) — the remote-trash-deletes-your-local-file direction.
    pub delete_approval_local: bool,
    /// Startup-reconcile tuning (warm start; see [`WarmStartConfig`]).
    pub warm_start: WarmStartConfig,
}

pub struct Daemon<C: ProtonClient = ProtonDriveClient> {
    config: DaemonConfig,
    connection: Connection,
    proton: C,
    pending_changes: BTreeSet<PathBuf>,
    /// Relative paths the daemon itself wrote into the watched tree during the current pass
    /// (`Download` destinations and `MoveLocal` *file* destinations). The `notify` watcher echoes
    /// those writes straight back as `Create`/`Modify` events; without this the echo would flip the
    /// just-committed `Synced` record to `Modified`, and a `Modified` record whose remote then
    /// changes plans a stale `Upload` that reverts the newer remote edit (or resurrects a remote
    /// delete) — issue #49. `handle_fs_event` suppresses `mark_modified` for a path in this set (but
    /// still queues it in `pending_changes`); it is cleared at the top of every pass, so the
    /// suppression lasts exactly one echo window and a later genuine user edit is never affected.
    authored_writes: HashSet<PathBuf>,
    /// Set when the filesystem watcher reported an error (typically an inotify queue overflow =
    /// events were dropped), so `pending_changes` under-reports and the events-mode idle
    /// fast-path must not skip the local stat-walk — the lost events would otherwise never be
    /// re-derived (#51). Forces `force_local_scan` on the next incremental pass; cleared only when
    /// a pass succeeds, so a failed pass keeps the rescan pending.
    force_local_rescan: bool,
    scan_options: ScanOptions,
    /// State shared with the concurrently-running control-socket server task (see
    /// [`ControlShared`]). The daemon core is the only writer of the snapshot; `paused` is
    /// written by both sides (IPC `pause`/`resume` flips it, the core reads it).
    shared: Arc<ControlShared>,
    last_sync: Option<SystemTime>,
    last_error: Option<String>,
    last_plan_summary: Option<PlanSummary>,
    last_successful_sync_summary: Option<PlanSummary>,
    status_history_path: PathBuf,
    metrics_path: PathBuf,
    status_history: Vec<StatusHistoryEntry>,
    ipc_io_timeout: Duration,
    /// Poll cadence of the events select arm ([`EVENTS_POLL_INTERVAL`]); a field so tests can drive
    /// the run loop at a cadence they can observe, exactly like [`Self::ipc_io_timeout`].
    events_poll_interval: Duration,
    /// Remote change detection via the volume event stream. `None` when `events_driven` is off
    /// (or the session could not be read), in which case every reconcile is a full-tree snapshot
    /// exactly as before this feature.
    event_source: Option<Box<dyn EventSource>>,
    /// Rebuilds [`Self::event_source`] on demand. Invoked before each reconcile while degraded
    /// (`events_driven` on but no source) so a daemon that started before the desktop keyring was
    /// unlocked — the common case for a systemd user service launched at boot — resumes O(changes)
    /// event-driven detection without a manual restart once the keyring becomes readable. Boxed so
    /// tests can inject a fake that flips from `None` to `Some`.
    event_source_factory: Box<dyn FnMut() -> Option<Box<dyn EventSource>> + Send>,
    /// Number of successful incremental (event-driven) passes since the last full-tree snapshot.
    /// Drives the opt-in periodic safety resync (`events_full_scan_every`, disabled by default).
    incremental_passes_since_full_scan: u64,
    /// `true` until the first reconcile of this process succeeds. The first pass is special —
    /// `notify` has replayed nothing, so it must full-scan the local tree (either via a bootstrap
    /// or a warm start's forced local scan) to catch edits made while the daemon was down. Cleared
    /// only on success, so a failed first pass retries as a first pass (keeping the "startup
    /// snapshots first" floor sticky across failures).
    is_first_reconcile: bool,
    /// Consecutive warm starts since the last full-tree walk, **persisted across restarts** (loaded
    /// from / stored to the `warm_start_state` table). Drives the every-N-warm-starts self-healing
    /// full walk (`warm_start.full_walk_every`). Distinct from the in-run
    /// `incremental_passes_since_full_scan`.
    warm_starts_since_full_walk: u64,
    /// The last reported reason the event-driven path could not resolve a volume + cursor, so a
    /// standing decline is logged once instead of every pass (and re-logged if the cause changes).
    /// `None` while the scope resolves. Diagnostic only — see `resolve_event_scope`.
    event_scope_declined: Option<String>,
    /// Deletions withheld by the delete-approval guard on the most recent reconcile, awaiting the
    /// user's approval. Recomputed from ground truth every pass, so it always reflects the current
    /// plan; surfaced over IPC (`proton-sync pending`) and in the metrics sidecar.
    pending_deletions: Vec<PendingDeletion>,
    /// Per-root instance lock; released on drop. Held for the daemon's whole lifetime.
    _lock_guard: LockGuard,
    /// User-global single-instance lock; released on drop. Held for the daemon's whole lifetime so
    /// no second daemon for this user can start and race the shared `proton-drive` cache (#23).
    _global_lock_guard: LockGuard,
}

/// On-disk snapshot the daemon writes to `<db>.metrics.json` at startup and after each sync.
/// Public + `Deserialize` so the desktop GUI can read it as a first-class live-state source
/// (preferred over live SQLite reads, which race the daemon's non-WAL writer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub generated_epoch_secs: u64,
    pub status: String,
    pub paused: bool,
    pub pending_changes: usize,
    pub last_sync_epoch_secs: Option<u64>,
    pub last_error: Option<String>,
    pub last_plan_summary: Option<PlanSummary>,
    pub last_successful_sync_summary: Option<PlanSummary>,
    pub status_history_entries: usize,
    pub pending_deletions: Vec<PendingDeletion>,
}

/// State shared between the daemon core and the control-socket server task so control requests
/// are answered instantly from the latest published snapshot — even while a reconcile is
/// blocking the main loop. This is what keeps the CLI and GUI responsive during a long sync.
struct ControlShared {
    /// Whether syncing is paused. Written by the IPC task (`pause`/`resume`) and read by the
    /// daemon core before each reconcile, so a pause takes effect from the next pass.
    paused: AtomicBool,
    /// `true` while a reconcile pass is in flight (drives the `syncing` status).
    syncing: AtomicBool,
    /// Count of completed reconcile attempts since startup (see `ControlResponse::reconcile_seq`).
    reconcile_seq: AtomicU64,
    /// Set by the IPC `resync` command to force the daemon's next reconcile to a full-tree walk
    /// (rather than a warm start / incremental pass). The daemon core consumes it with a `swap`
    /// at the top of each pass. Written by the IPC task, read-and-cleared by the core.
    force_full_walk: AtomicBool,
    /// The daemon core's most recently published status. The IPC task only ever reads it.
    snapshot: StdMutex<StatusSnapshot>,
    /// Live activity for the in-flight pass (see [`SyncActivity`]). Written from inside the
    /// blocking reconcile (phase changes, per-folder walk progress, per-file scan progress,
    /// per-action execution) and read by the IPC task on every status reply, so clients see
    /// motion while the main task is blocked. Separate from `snapshot` because it churns far
    /// more often than a `publish_status` and needs none of the snapshot's contents.
    activity: StdMutex<ActivityState>,
}

/// [`ControlShared::activity`]'s contents: the wire-visible activity plus the internal staging
/// location of an in-flight download, kept out of the wire type so status replies can sample
/// bytes-so-far without leaking scratch paths to clients.
#[derive(Default)]
struct ActivityState {
    current: Option<SyncActivity>,
    download_scratch: Option<PathBuf>,
}

/// Everything a status reply needs beyond the atomics above. Published by the daemon core via
/// [`Daemon::publish_status`] whenever its state changes.
#[derive(Clone)]
struct StatusSnapshot {
    pending_changes: usize,
    last_sync_epoch_secs: Option<u64>,
    last_error: Option<String>,
    last_plan_summary: Option<PlanSummary>,
    last_successful_sync_summary: Option<PlanSummary>,
    status_history: Vec<StatusHistoryEntry>,
    pending_deletions: Vec<PendingDeletion>,
    config: RunningConfigInfo,
}

impl ControlShared {
    fn new(config: RunningConfigInfo) -> Self {
        Self {
            paused: AtomicBool::new(false),
            syncing: AtomicBool::new(false),
            reconcile_seq: AtomicU64::new(0),
            force_full_walk: AtomicBool::new(false),
            snapshot: StdMutex::new(StatusSnapshot {
                pending_changes: 0,
                last_sync_epoch_secs: None,
                last_error: None,
                last_plan_summary: None,
                last_successful_sync_summary: None,
                status_history: Vec::new(),
                pending_deletions: Vec::new(),
                config,
            }),
            activity: StdMutex::new(ActivityState::default()),
        }
    }

    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Starts a new activity phase, replacing whatever was current (and dropping any stale
    /// download staging location from the previous action).
    fn begin_activity(&self, activity: SyncActivity) {
        let mut state = self.activity.lock().expect("activity lock");
        state.current = Some(activity);
        state.download_scratch = None;
    }

    /// Applies `update` to the current activity, creating one with `phase` first if none is
    /// current or the phase changed (which also resets `since_epoch_secs` to now, so elapsed
    /// times are per-phase). Used by the high-frequency walk/scan callbacks.
    fn update_activity(&self, phase: &str, update: impl FnOnce(&mut SyncActivity)) {
        let mut state = self.activity.lock().expect("activity lock");
        let same_phase = matches!(&state.current, Some(current) if current.phase == phase);
        if !same_phase {
            state.current = Some(new_activity(phase));
        }
        let current = state
            .current
            .as_mut()
            .expect("activity was just ensured above");
        update(current);
    }

    /// Records the staging directory of the in-flight download so status replies can sample
    /// bytes-so-far while the CLI child is still running.
    fn note_download_scratch(&self, scratch_dir: &Path) {
        let mut state = self.activity.lock().expect("activity lock");
        state.download_scratch = Some(scratch_dir.to_path_buf());
    }

    /// Clears all live activity (the pass is over; `syncing` is about to read false).
    fn clear_activity(&self) {
        let mut state = self.activity.lock().expect("activity lock");
        *state = ActivityState::default();
    }

    /// Snapshot of the current activity for a status reply. Purely in-memory — a download's
    /// live `bytes_done` is filled in separately by [`Self::response_with_sampled_activity`],
    /// so building a plain reply never touches the filesystem.
    fn activity_for_response(&self) -> Option<SyncActivity> {
        self.activity.lock().expect("activity lock").current.clone()
    }

    /// [`Self::response`] plus the one live measurement a reply can carry: a download's
    /// bytes-so-far, sampled from its staging directory with async fs calls. Only the `status`
    /// command pays for this — the IPC handler's "answer from the snapshot, never by running
    /// work on this task" contract stays intact for everything else, and the sampling itself
    /// never blocks the async runtime.
    async fn response_with_sampled_activity(&self, message: &str) -> ControlResponse {
        let mut response = self.response(message);
        if !response.syncing {
            return response;
        }
        // Take the activity and the staging directory under ONE lock acquisition, so the
        // sampled bytes always belong to the transfer being reported. Re-reading them
        // separately (or reusing `response`'s own activity, cloned under an earlier
        // acquisition) could pair action X's transfer with action Y's staging directory when
        // a poll lands exactly on an action boundary.
        let (current, scratch) = {
            let state = self.activity.lock().expect("activity lock");
            (state.current.clone(), state.download_scratch.clone())
        };
        response.activity = current;
        if let Some(activity) = &mut response.activity
            && let Some(transfer) = &mut activity.transfer
            && transfer.direction == "download"
            && let Some(scratch_dir) = scratch
        {
            transfer.bytes_done = staged_bytes(&scratch_dir).await;
        }
        response
    }

    fn is_syncing(&self) -> bool {
        self.syncing.load(Ordering::SeqCst)
    }

    fn response(&self, message: &str) -> ControlResponse {
        let paused = self.is_paused();
        let syncing = self.is_syncing();
        let snapshot = self.snapshot.lock().expect("control snapshot lock").clone();
        ControlResponse {
            // The string reports live *activity*, so an in-flight pass stays "syncing" even
            // when a pause was just accepted mid-pass (the pass still runs to completion);
            // the `paused` boolean carries the standing request either way.
            status: if syncing {
                "syncing"
            } else if paused {
                "paused"
            } else {
                "running"
            }
            .to_owned(),
            paused,
            syncing,
            reconcile_seq: self.reconcile_seq.load(Ordering::SeqCst),
            pending_changes: snapshot.pending_changes,
            message: message.to_owned(),
            last_sync_epoch_secs: snapshot.last_sync_epoch_secs,
            last_error: snapshot.last_error,
            last_plan_summary: snapshot.last_plan_summary,
            last_successful_sync_summary: snapshot.last_successful_sync_summary,
            status_history: snapshot.status_history,
            pending_deletions: snapshot.pending_deletions,
            config: Some(snapshot.config),
            // Gated on `syncing` read above: `syncing` and the activity slot are updated
            // independently at pass end, so without the gate a reply issued between
            // `syncing.store(false)` and `clear_activity()` could pair `syncing: false`
            // with a stale "downloading X".
            activity: if syncing {
                self.activity_for_response()
            } else {
                None
            },
        }
    }

    fn metrics(&self) -> MetricsSnapshot {
        let paused = self.is_paused();
        let snapshot = self.snapshot.lock().expect("control snapshot lock").clone();
        MetricsSnapshot {
            generated_epoch_secs: current_epoch_secs(),
            status: if paused { "paused" } else { "running" }.to_owned(),
            paused,
            pending_changes: snapshot.pending_changes,
            last_sync_epoch_secs: snapshot.last_sync_epoch_secs,
            last_error: snapshot.last_error,
            last_plan_summary: snapshot.last_plan_summary,
            last_successful_sync_summary: snapshot.last_successful_sync_summary,
            status_history_entries: snapshot.status_history.len(),
            pending_deletions: snapshot.pending_deletions,
        }
    }
}

/// Activity phase tokens (see [`SyncActivity::phase`]). Wire-visible: renamed values break
/// older clients' phase-specific rendering (they fall back to showing the raw token).
const PHASE_SCANNING_LOCAL: &str = "scanning-local";
const PHASE_LISTING_REMOTE: &str = "listing-remote";
const PHASE_FETCHING_EVENTS: &str = "fetching-events";
const PHASE_EXECUTING: &str = "executing";
const PHASE_COMMITTING: &str = "committing";

/// A blank [`SyncActivity`] for `phase`, stamped with the current time.
fn new_activity(phase: &str) -> SyncActivity {
    SyncActivity {
        phase: phase.to_owned(),
        detail: None,
        folders_listed: None,
        files_scanned: None,
        action_index: None,
        action_total: None,
        transfer: None,
        since_epoch_secs: Some(current_epoch_secs()),
    }
}

/// Human verb for an executing action's activity line (`"downloading a/b.txt"`).
fn activity_verb(action: &SyncAction) -> &'static str {
    match action {
        SyncAction::Upload => "uploading",
        SyncAction::Download => "downloading",
        SyncAction::CreateRemoteDirectory => "creating remote folder",
        SyncAction::CreateLocalDirectory => "creating local folder",
        SyncAction::MoveLocal => "moving locally",
        SyncAction::MoveRemote => "moving remotely",
        SyncAction::AutoLink => "linking",
        SyncAction::Conflict => "resolving conflict",
        SyncAction::TypeConflict => "resolving type conflict",
        SyncAction::RemoteDelete => "deleting remotely",
        SyncAction::LocalDelete => "deleting locally",
        SyncAction::Purge => "clearing index entry",
        SyncAction::SkipUnsupported => "skipping unsupported",
    }
}

/// Sum of the file sizes currently inside a download staging directory (the CLI downloads a
/// single file into it, so this is that file's bytes-so-far). Async so the IPC task never
/// blocks on filesystem calls against a directory under active write load. Best-effort
/// display data: any error — the directory already renamed away, a race with the move — is
/// just `None`.
async fn staged_bytes(scratch_dir: &Path) -> Option<u64> {
    let mut entries = tokio::fs::read_dir(scratch_dir).await.ok()?;
    let mut total = 0u64;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(metadata) = entry.metadata().await
            && metadata.is_file()
        {
            total += metadata.len();
        }
    }
    Some(total)
}

/// The daemon's [`ProgressSink`]: forwards the concrete client's live callbacks into
/// [`ControlShared`] so every status reply reflects them. Installed in [`Daemon::run`].
struct SharedProgressSink {
    shared: Arc<ControlShared>,
}

impl ProgressSink for SharedProgressSink {
    fn remote_folder_listed(&self, folders_listed: u64, directory: &Path) {
        self.shared
            .update_activity(PHASE_LISTING_REMOTE, |activity| {
                activity.folders_listed = Some(folders_listed);
                activity.detail = Some(if directory.as_os_str().is_empty() {
                    "/".to_owned()
                } else {
                    directory.display().to_string()
                });
            });
    }

    fn download_staging(&self, scratch_dir: &Path) {
        self.shared.note_download_scratch(scratch_dir);
    }
}

/// Work the control-socket task hands to the daemon's main loop — the two commands that must run
/// on the core (everything else is answered directly from [`ControlShared`]).
enum LoopCommand {
    SyncNow,
    Shutdown,
}

pub fn preview_plan(config: &DaemonConfig) -> AppResult<Vec<PlannedAction>> {
    let client = ProtonDriveClient::with_command_policy(
        config.proton_cli.clone(),
        command_policy_from_config(config),
    );
    preview_plan_with_client(config, &client)
}

pub fn preview_plan_with_client(
    config: &DaemonConfig,
    proton: &impl ProtonClient,
) -> AppResult<Vec<PlannedAction>> {
    info!(
        local_root = %config.local_root.display(),
        remote_root = %config.remote_root.display(),
        "building dry-run sync plan"
    );
    let scan_options = scan_options_from_config(config)?;
    let base_records = load_existing_index(&config.db_path)?;
    let local_entities =
        scan_local_entities_reusing_hashes(&config.local_root, &scan_options, &base_records)?;
    let (remote_entities, remote_root_missing) =
        load_remote_entities(proton, &config.remote_root, &scan_options)?;
    let mut base_index = filter_base_index(base_records, &scan_options);
    if remote_root_missing {
        base_index.clear();
    }
    let mut plan = plan_sync_entities(&local_entities, &remote_entities, &base_index);
    prepend_remote_root_creation_if_missing(&mut plan, remote_root_missing);
    info!(planned_actions = plan.len(), "dry-run sync plan computed");
    Ok(plan)
}

impl Daemon<ProtonDriveClient> {
    pub fn new(config: DaemonConfig) -> AppResult<Self> {
        let proton = ProtonDriveClient::with_command_policy(
            config.proton_cli.clone(),
            command_policy_from_config(&config),
        );
        let event_source = build_event_source(&config);
        Self::with_client_and_event_source(config, proton, event_source)
    }
}

impl<C: ProtonClient> Daemon<C> {
    pub fn with_client(config: DaemonConfig, proton: C) -> AppResult<Self> {
        Self::with_client_and_event_source(config, proton, None)
    }

    pub fn with_client_and_event_source(
        config: DaemonConfig,
        proton: C,
        event_source: Option<Box<dyn EventSource>>,
    ) -> AppResult<Self> {
        fs::create_dir_all(&config.local_root)?;
        if let Some(parent) = config.db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = config.socket_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        // Ensure the lockfile's directory (the `.sync` state dir by default) exists before
        // acquiring the lock, so a first-ever run on a fresh root does not fail here.
        if let Some(parent) = config.lockfile_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let lock_guard = LockGuard::acquire(&config.lockfile_path)?;
        // Then the user-global lock, so a second daemon started for a *different* root (its per-root
        // lock above would succeed) still cannot run: every daemon shells the same `proton-drive`
        // CLI, whose shared SQLite cache/session store is not concurrency-safe (#23). A crashed
        // daemon leaves only an unlocked file behind, which `acquire` reuses — restart still works.
        let global_lock_guard = LockGuard::acquire(&config.global_lock_path).map_err(|error| {
            boxed_error(format!(
                "cannot start: {error}. Only one proton-syncd may run per user account — every \
                 daemon shells the same proton-drive CLI, whose SQLite cache and session store are \
                 not safe for concurrent use (#23). Stop the other daemon first."
            ))
        })?;
        let connection = open_database(&config.db_path)?;
        let scan_options = scan_options_from_config(&config)?;
        let status_history_path = status_history_path(&config.db_path);
        let metrics_path = metrics_path(&config.db_path);
        let status_history = load_status_history(&status_history_path).unwrap_or_else(|error| {
            warn!(
                path = %status_history_path.display(),
                error = %error,
                "ignoring unreadable daemon status history"
            );
            Vec::new()
        });

        // The first reconcile after startup is handled explicitly by `first_reconcile` (via the
        // `is_first_reconcile` flag), which either full-walks or warm-starts — both full-scan the
        // local tree, so a file edited while the daemon was down (an empty `pending_changes`, since
        // `notify` never replays pre-existing files) is always caught. The in-run resync counter
        // therefore starts at 0 and only ever gates the *periodic* in-run resync
        // (`events_full_scan_every`), never the startup pass.
        let incremental_passes_since_full_scan = 0;
        // Persisted across restarts: how many warm starts have happened since the last full walk,
        // driving the every-N-warm-starts self-healing full walk. A read failure degrades to 0
        // (worst case: one extra warm start before the next full walk), never blocks startup.
        let warm_starts_since_full_walk =
            load_warm_start_count(&connection).unwrap_or_else(|error| {
                warn!(%error, "ignoring unreadable warm-start counter; treating it as zero");
                0
            });
        // Captured by value so the closure re-reads the keyring at runtime without holding the
        // config. Only the feature flag is needed — `CliKeyringSession::from_cli_keyring` sources
        // the session from the keyring itself. Quiet on failure (unlike the startup
        // `build_event_source`, which warns once): this runs every degraded pass.
        let events_driven = config.events_driven;
        let event_source_factory: Box<dyn FnMut() -> Option<Box<dyn EventSource>> + Send> =
            Box::new(move || {
                if !events_driven {
                    return None;
                }
                match CliKeyringSession::from_cli_keyring() {
                    Ok(session) => Some(Box::new(EventsClient::new(
                        CurlHttpTransport::new(),
                        session,
                        EVENTS_APP_VERSION,
                    )) as Box<dyn EventSource>),
                    Err(_) => None,
                }
            });
        let shared = Arc::new(ControlShared::new(RunningConfigInfo {
            local_root: config.local_root.clone(),
            remote_root: config.remote_root.clone(),
            db_path: config.db_path.clone(),
        }));
        let daemon = Self {
            config,
            connection,
            proton,
            pending_changes: BTreeSet::new(),
            authored_writes: HashSet::new(),
            force_local_rescan: false,
            scan_options,
            shared,
            last_sync: None,
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history_path,
            metrics_path,
            status_history,
            ipc_io_timeout: IPC_IO_TIMEOUT,
            events_poll_interval: EVENTS_POLL_INTERVAL,
            event_source,
            event_source_factory,
            incremental_passes_since_full_scan,
            is_first_reconcile: true,
            warm_starts_since_full_walk,
            event_scope_declined: None,
            pending_deletions: Vec::new(),
            _lock_guard: lock_guard,
            _global_lock_guard: global_lock_guard,
        };
        daemon.publish_status();
        daemon.write_metrics_snapshot()?;
        Ok(daemon)
    }

    pub async fn run(mut self) -> AppResult<()> {
        info!(
            local_root = %self.config.local_root.display(),
            remote_root = %self.config.remote_root.display(),
            socket_path = %self.config.socket_path.display(),
            "starting daemon"
        );
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.proton.install_cancel_flag(Arc::clone(&cancel_flag));
        // Live-progress plumbing: the client reports per-folder walk progress and download
        // staging locations straight into `ControlShared`, so status replies stay alive while
        // this task is blocked inside a reconcile.
        self.proton
            .install_progress_sink(Arc::new(SharedProgressSink {
                shared: Arc::clone(&self.shared),
            }));
        // Runs independently of the select! loop below so it can still observe a
        // shutdown signal and flip the flag while the main task is blocked inside
        // `reconcile()`'s synchronous `block_in_place` call, letting an in-flight
        // proton-drive command be cancelled promptly instead of only being noticed
        // once that blocking call returns control to this task.
        let signal_cancel_flag = Arc::clone(&cancel_flag);
        tokio::spawn(async move {
            shutdown_signal().await;
            signal_cancel_flag.store(true, Ordering::SeqCst);
        });

        let listener = bind_listener(&self.config.socket_path).await?;
        // Serve the control socket on its own task so a status poll (or pause/approve/…) is
        // answered instantly even while this task is blocked inside a reconcile. The task gets
        // its own SQLite connection for approval writes (both connections set a busy timeout,
        // so a rare same-database collision waits instead of failing); `syncnow`/`shutdown` are
        // forwarded to the loop below over `loop_rx`. The cancel flag gives an IPC `shutdown`
        // the same teeth as a signal: an in-flight proton-drive command is killed rather than
        // holding up the exit (the commit-after-side-effects invariant makes that safe — the
        // interrupted pass keeps only the checkpoints of actions that fully completed and
        // replans the remainder from ground truth on next start).
        let (loop_tx, mut loop_rx) = mpsc::unbounded_channel();
        let approvals_connection = open_database(&self.config.db_path)?;
        let ipc_task = tokio::spawn(serve_control_socket(
            listener,
            Arc::clone(&self.shared),
            approvals_connection,
            loop_tx,
            self.ipc_io_timeout,
            self.metrics_path.clone(),
            Arc::clone(&cancel_flag),
        ));
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel();
        let mut watcher = build_watcher(watch_tx)?;
        watcher.watch(&self.config.local_root, RecursiveMode::Recursive)?;

        let mut interval = tokio::time::interval(self.config.scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;

        // A faster poll cadence for event-driven mode: an incremental pass is O(changes) and
        // usually idle, so polling the stream often keeps remote-change latency low without the
        // cost of a full-tree walk. The arm is gated on `events_driven` *and* a live event source,
        // so with the feature off — or degraded to snapshots — it never fires and the loop behaves
        // exactly as before. Using an interval arm (rather than a separate event-fetching task)
        // keeps the single owner of `event_source` and the SQLite connection inside the loop,
        // avoiding shared-state hazards.
        let mut events_poll = tokio::time::interval(self.events_poll_interval);
        events_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        events_poll.tick().await;

        // Why this daemon may behave like an events-off one is reported by
        // `note_degraded_session_if_needed`, from inside every pass — one message family with one
        // reason per cause (see `note_event_scope_declined`), not a second line here saying
        // something adjacent.

        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);

        // Reconcile once immediately on startup so a fresh sync (or one restarted after
        // downtime) converges right away instead of waiting a full `scan_interval` for the first
        // periodic tick. Both interval arms above consumed their immediate first tick, and
        // filesystem-watch events only accumulate `pending_changes` (they never trigger a
        // reconcile), so the periodic tick is otherwise the sole automatic sync trigger — without
        // this, a freshly started daemon downloads/uploads nothing until `scan_interval` (default
        // 5 minutes) elapses. This is also the "first reconcile after startup" the constructor
        // seeds to be a full-tree snapshot.
        //
        // Run it inside a `biased` select against `shutdown` so the signal future is *polled and
        // registered before* this (synchronous, `block_in_place`) reconcile starts. A SIGINT that
        // arrives while the first reconcile is stuck is flipped into the cancel flag by the task
        // spawned above (unblocking the reconcile); the loop below then re-polls this same
        // `shutdown` future and latches that already-delivered signal to exit cleanly. Without the
        // pre-registration, a signal delivered during the very first reconcile would be missed and
        // the daemon would fail to shut down. `reconcile_if_needed` respects `paused` and
        // logs-then-swallows reconcile errors, so a flaky first sync retries at the interval
        // instead of aborting startup.
        let mut shutdown_during_startup = false;
        tokio::select! {
            biased;
            _ = &mut shutdown => shutdown_during_startup = true,
            result = self.reconcile_if_needed() => result?,
        }

        if !shutdown_during_startup {
            loop {
                tokio::select! {
                    maybe_event = watch_rx.recv() => {
                        match maybe_event {
                            Some(Ok(event)) => {
                                let pending_before = self.pending_changes.len();
                                let outcome =
                                    tokio::task::block_in_place(|| self.handle_fs_event(event));
                                if let Err(error) = outcome {
                                    warn!(%error, "failed to process filesystem event");
                                }
                                if self.pending_changes.len() != pending_before {
                                    self.publish_status();
                                }
                            }
                            Some(Err(error)) => {
                                self.note_watch_error(&error);
                            }
                            None => break,
                        }
                    }
                    Some(command) = loop_rx.recv() => {
                        match command {
                            LoopCommand::SyncNow => self.reconcile_if_needed().await?,
                            LoopCommand::Shutdown => {
                                info!("shutting down on control request");
                                break;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        self.reconcile_if_needed().await?;
                    }
                    // #50: `events_driven` alone is not enough — without a source every pass is a
                    // full-tree walk, and this fast cadence would run one every 30s forever.
                    _ = events_poll.tick(),
                        if self.config.events_driven && self.event_source.is_some() => {
                        self.reconcile_if_needed().await?;
                    }
                    _ = &mut shutdown => {
                        break;
                    }
                }
            }
        }

        ipc_task.abort();
        remove_control_socket(&self.config.socket_path);
        info!("daemon stopped");
        Ok(())
    }

    /// The filesystem watcher failed. On Linux that is usually an inotify queue overflow, which
    /// means events were **dropped**: the files they described are absent from `pending_changes`
    /// and, in events mode, produce no remote event either — so the idle fast-path would skip the
    /// local stat-walk and strand them (#51). Force the next pass to scan the local tree.
    /// The whole run-loop arm is this call, so the loop cannot drift from what the tests drive.
    fn note_watch_error(&mut self, error: &notify::Error) {
        warn!(
            %error,
            "filesystem watcher reported an error; forcing a local rescan on the next pass \
             because events may have been dropped"
        );
        self.force_local_rescan = true;
    }

    fn handle_fs_event(&mut self, event: Event) -> AppResult<()> {
        for path in event.paths {
            if path.is_dir() {
                // #51: an empty `mkdir` emits a directory event and nothing else. Dropping it left
                // `pending_changes` empty, so every events-mode pass idle-skipped planning and the
                // folder was never mirrored. Queue the path so the pass plans (it re-scans and
                // emits `CreateRemoteDirectory`); never `mark_modified` — a directory has no
                // file-content record semantics. The empty relative path is the watched root
                // itself, not a syncable entity.
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
                    && !crate::sync::is_conflict_copy(&path)
                    && let Ok(relative_path) = path.strip_prefix(&self.config.local_root)
                    && !relative_path.as_os_str().is_empty()
                    && self.scan_options.allows_relative_directory(relative_path)
                {
                    self.pending_changes.insert(relative_path.to_path_buf());
                }
                continue;
            }
            if crate::sync::is_conflict_copy(&path) {
                if matches!(event.kind, EventKind::Remove(_))
                    && let Some(original) = original_from_conflict_copy(&path)
                    && let Ok(relative_path) = original.strip_prefix(&self.config.local_root)
                    && self.scan_options.allows_relative_file(relative_path)
                {
                    mark_modified(&self.connection, relative_path)?;
                    self.pending_changes.insert(relative_path.to_path_buf());
                }
                continue;
            }

            let relative_path = match path.strip_prefix(&self.config.local_root) {
                Ok(relative_path) => relative_path.to_path_buf(),
                Err(_) => continue,
            };

            if !self.scan_options.allows_relative_file(&relative_path) {
                continue;
            }

            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    // A brand-new file with no existing base-index record does not need
                    // to be hashed and upserted here: the next full reconcile's
                    // bootstrap planning already discovers and uploads it correctly
                    // from the local scan alone. `mark_modified` is a targeted
                    // `UPDATE ... WHERE file_path = ?` that is a safe no-op when no row
                    // exists yet, so it is always safe to call unconditionally without
                    // first checking whether the path exists, is a regular file, or
                    // already has a record - this also avoids synchronously hashing
                    // potentially large files on every raw filesystem event.
                    //
                    // Exception: skip `mark_modified` when this is the watcher echoing a
                    // write the daemon *itself* just made (a Download destination or a
                    // `MoveLocal` file destination this pass). Flipping that fresh `Synced`
                    // record to `Modified` would make the next pass plan a stale `Upload`
                    // over a newer remote edit — reverting it, or resurrecting a remote
                    // delete (issue #49). Still queue it in `pending_changes` so the path is
                    // re-examined: a genuine user edit landing in the same window is caught
                    // regardless, because planning re-scans and detects it from the content
                    // delta even without the `Modified` flag.
                    if !self.authored_writes.contains(&relative_path) {
                        mark_modified(&self.connection, &relative_path)?;
                    }
                    self.pending_changes.insert(relative_path);
                }
                EventKind::Remove(_) => {
                    self.pending_changes.insert(relative_path);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Publishes the daemon core's current state to [`ControlShared`], from which the IPC task
    /// answers every control request. Called whenever the state it copies changes.
    fn publish_status(&self) {
        let snapshot = StatusSnapshot {
            pending_changes: self.pending_changes.len(),
            last_sync_epoch_secs: self.last_sync_epoch_secs(),
            last_error: self.last_error.clone(),
            last_plan_summary: self.last_plan_summary.clone(),
            last_successful_sync_summary: self.last_successful_sync_summary.clone(),
            status_history: self.status_history.clone(),
            pending_deletions: self.pending_deletions.clone(),
            config: RunningConfigInfo {
                local_root: self.config.local_root.clone(),
                remote_root: self.config.remote_root.clone(),
                db_path: self.config.db_path.clone(),
            },
        };
        *self.shared.snapshot.lock().expect("control snapshot lock") = snapshot;
    }

    /// Test-only convenience: publish and build a status reply exactly as the IPC task would.
    /// Production replies are built by the IPC task itself from [`ControlShared`].
    #[cfg(test)]
    fn status_response(&self, message: &str) -> ControlResponse {
        self.publish_status();
        self.shared.response(message)
    }

    fn is_paused(&self) -> bool {
        self.shared.is_paused()
    }

    async fn reconcile_if_needed(&mut self) -> AppResult<()> {
        if self.is_paused() {
            return Ok(());
        }
        if let Err(error) = self.reconcile().await {
            error!(%error, "scheduled reconciliation failed");
        }
        Ok(())
    }

    async fn reconcile(&mut self) -> AppResult<()> {
        // Flag the pass for status replies before blocking this task; the IPC task keeps
        // serving (and reporting `syncing`) for the whole duration.
        self.shared.syncing.store(true, Ordering::SeqCst);
        let result = tokio::task::block_in_place(|| self.reconcile_blocking());
        self.shared.syncing.store(false, Ordering::SeqCst);
        // The pass is over either way; never let a stale "downloading X" outlive it.
        self.shared.clear_activity();
        result
    }

    fn reconcile_blocking(&mut self) -> AppResult<()> {
        let result = self.reconcile_blocking_inner();
        let message = match &result {
            Ok(()) => {
                self.last_error = None;
                "sync completed"
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                "sync failed"
            }
        };
        self.record_status_history(message);
        // The attempt is complete (recorded either way): bump the sequence a waiting client
        // watches, then publish the final state of this pass.
        self.shared.reconcile_seq.fetch_add(1, Ordering::SeqCst);
        self.publish_status();
        result
    }

    fn reconcile_blocking_inner(&mut self) -> AppResult<()> {
        info!("starting reconciliation");
        // Recover event-driven detection if it was disabled at startup because the keyring was
        // still locked (the boot race). No-op once a source exists or when the feature is off.
        self.reacquire_event_source_if_needed();
        // Still no source → report that cause through the same once-per-cause latch the scope
        // reasons use, so an operator reading "keeps doing full syncs" gets exactly one line
        // naming exactly one cause.
        self.note_degraded_session_if_needed();
        // Start each pass with an empty authored-writes set: it only needs to survive from a
        // download/move to the watcher echo of that same write (which drains after this pass
        // returns from `block_in_place`, before the next pass runs). Clearing here bounds the
        // `mark_modified` suppression in `handle_fs_event` to a single echo window, so a genuine
        // user edit that arrives in any later pass is never mistaken for the daemon's own echo.
        self.authored_writes.clear();
        // Load the baseline before scanning so the scan can reuse each unchanged file's
        // recorded SHA-1 (matching size + mtime) instead of re-hashing the whole tree.
        let base_records = load_index(&self.connection)?;

        // A runtime `resync` forces this pass to a full-tree walk, overriding both the warm start
        // and the steady-state incremental path. Consume it exactly once (a `swap`), so a later
        // pass is not also forced. (The startup `--full-walk` flag is separate — see below.)
        let resync_requested = self.shared.force_full_walk.swap(false, Ordering::SeqCst);

        // The first reconcile after boot is special: `notify` has replayed nothing, so it must
        // full-scan the local tree. `first_reconcile` either full-walks or warm-starts (both do
        // that local scan). Clear `is_first_reconcile` only on success, so a failed first pass
        // retries as a first pass — keeping the "startup snapshots first" floor sticky across
        // failures (a failed pass must not let the next one drop into the steady-state idle
        // fast-path, which skips the local scan and would strand an offline edit).
        if self.is_first_reconcile {
            // `--full-walk` (a process-lifetime config flag, not the consumed atomic) stays in
            // force across a failed first pass, so the requested full walk still happens on retry.
            let force_bootstrap = resync_requested || self.config.warm_start.force_full_walk;
            let result = self.first_reconcile(base_records, force_bootstrap);
            if result.is_ok() {
                self.is_first_reconcile = false;
                // Both first-pass branches full-scan the local tree, so any pending
                // watcher-error rescan is satisfied here.
                self.force_local_rescan = false;
            }
            return result;
        }

        // Event-driven steady state: attempt an incremental pass (O(changes)) before resorting to
        // a full-tree snapshot (O(folders)). Any doubt — no cursor, a server refresh, an events
        // error, or an unresolvable node — falls through to the snapshot below, which is exactly
        // today's behavior. When `events_driven` is off this predicate is always false.
        if !resync_requested && self.should_try_incremental(&base_records) {
            // A watcher error dropped events (#51): this pass must stat-walk the local tree
            // instead of taking the idle fast-path, which only knows about `pending_changes`.
            match self.try_incremental_reconcile(&base_records, self.force_local_rescan)? {
                IncrementalOutcome::Committed | IncrementalOutcome::Idle => {
                    self.force_local_rescan = false;
                    return Ok(());
                }
                IncrementalOutcome::Fallback(reason) => {
                    info!(%reason, "event-driven pass fell back to a full-tree snapshot");
                }
            }
        }

        // A snapshot always full-scans the local tree, so it too clears the pending rescan.
        let result = self.bootstrap_reconcile(base_records);
        if result.is_ok() {
            self.force_local_rescan = false;
        }
        result
    }

    /// The first reconcile after this process booted. Warm-starts when eligible (an event-driven
    /// reconcile that keeps the cheap full local stat-walk but replays the remote from the
    /// persisted cursor instead of the O(folders) walk), otherwise full-walks. A warm start that
    /// cannot complete falls through to a bootstrap. Both paths full-scan the local tree, so an
    /// edit made while the daemon was down is always caught.
    fn first_reconcile(
        &mut self,
        base_records: HashMap<PathBuf, FileRecord>,
        force_bootstrap: bool,
    ) -> AppResult<()> {
        if !force_bootstrap && self.warm_start_eligible(&base_records) {
            // `true` = force the local stat-walk even if the event delta is empty: on a fresh boot
            // `pending_changes` is empty, so the idle fast-path would otherwise skip the scan and
            // strand a file edited while the daemon was down.
            match self.try_incremental_reconcile(&base_records, true)? {
                IncrementalOutcome::Committed | IncrementalOutcome::Idle => {
                    self.record_successful_warm_start();
                    info!("warm start completed; skipped the full-tree remote walk");
                    return Ok(());
                }
                IncrementalOutcome::Fallback(reason) => {
                    info!(%reason, "warm start fell back to a full-tree snapshot");
                }
            }
        }
        self.bootstrap_reconcile(base_records)
    }

    /// Whether the first pass after boot may warm-start: the feature is enabled with a usable event
    /// source, the every-N-warm-starts full-walk floor is not yet due, a volume and a stored cursor
    /// exist, and that cursor is fresh enough (unlike steady-state incremental, the first pass adds
    /// the cursor-age gate — it may be replaying a cursor persisted by a previous process across an
    /// unknown amount of downtime).
    fn warm_start_eligible(&mut self, base_records: &HashMap<PathBuf, FileRecord>) -> bool {
        let warm = &self.config.warm_start;
        if !warm.enabled || !self.config.events_driven || self.event_source.is_none() {
            return false;
        }
        // Self-healing full walk every N warm starts (across restarts). `0` maps to `u64::MAX`.
        if self.warm_starts_since_full_walk >= effective_full_scan_every(warm.full_walk_every) {
            return false;
        }
        let Some((_, cursor)) = self.resolve_event_scope(base_records) else {
            return false;
        };
        self.cursor_is_fresh(&cursor)
    }

    /// Whether the persisted cursor is recent enough to warm-start against. Guards the one thing we
    /// cannot verify from here — that the server signals a refresh (rather than silently truncating)
    /// for a cursor past its event-retention window. A `Duration::ZERO` max age disables the gate; a
    /// future `updated_at` (clock skew) reads as stale so we take the safe full walk.
    fn cursor_is_fresh(&self, cursor: &EventCursor) -> bool {
        let max_age = self.config.warm_start.max_cursor_age;
        if max_age.is_zero() {
            return true;
        }
        let age = current_epoch_secs() as i64 - cursor.updated_at;
        age >= 0 && (age as u64) <= max_age.as_secs()
    }

    /// Records one more successful warm start (persisting the across-restart counter). Best-effort:
    /// this is a heuristic that only affects *when* the next self-healing full walk fires, so a
    /// write failure logs and is swallowed rather than failing an otherwise-successful sync.
    fn record_successful_warm_start(&mut self) {
        // Cap at the floor: the counter is only ever compared against
        // `effective_full_scan_every(full_walk_every)`, and once it reaches that a bootstrap resets
        // it — so there is no reason to climb past it. This keeps the value small (the in-memory and
        // persisted counts always agree, and never approach the signed-column range) except in the
        // "floor disabled" case, where the cap is `u64::MAX` and `store_warm_start_count` saturates.
        let floor = effective_full_scan_every(self.config.warm_start.full_walk_every);
        self.warm_starts_since_full_walk = self
            .warm_starts_since_full_walk
            .saturating_add(1)
            .min(floor);
        if let Err(error) =
            store_warm_start_count(&self.connection, self.warm_starts_since_full_walk)
        {
            warn!(%error, "failed to persist the warm-start counter; it may reset on restart");
        }
    }

    /// Re-attempt building [`Self::event_source`] when event-driven detection is enabled but no
    /// source exists — typically because the desktop keyring was still locked when the daemon
    /// started at boot, so `build_event_source` degraded to `None` for the process lifetime. Lets
    /// the daemon resume O(changes) event-driven detection without a manual restart once the
    /// keyring is unlocked. No-op when the feature is off or a source already exists (so a working
    /// source is never rebuilt, and the keyring is only re-read while actually degraded).
    fn reacquire_event_source_if_needed(&mut self) {
        if !self.config.events_driven || self.event_source.is_some() {
            return;
        }
        if let Some(source) = (self.event_source_factory)() {
            info!("reused CLI session became readable; resuming event-driven change detection");
            self.event_source = Some(source);
            // On a *mid-life* reacquisition (not the first pass), force this pass to full-walk by
            // reseeding the resync floor. Steady-state incremental has no cursor-age gate, so
            // without this it would replay the cursor persisted by a previous process — which the
            // degraded snapshots never advanced past — and miss everything since. A full walk
            // captures a fresh cursor to stream from. On the *first* pass this reseed is skipped:
            // `first_reconcile` already decides warm-start-vs-bootstrap with its own cursor-age
            // gate (which subsumes this concern), and reseeding here would also risk overflowing
            // the counter when a warm start increments it past the `u64::MAX` sentinel.
            if !self.is_first_reconcile {
                self.incremental_passes_since_full_scan =
                    effective_full_scan_every(self.config.events_full_scan_every);
            }
        }
    }

    /// Reports the one cause [`Self::resolve_event_scope`] can never see: `events_driven` is on but
    /// there is no event source (locked keyring, headless host, CLI not logged in), so the scope is
    /// never even consulted. Every pass is then a full-tree snapshot and the fast events-poll arm
    /// is gated off, so both those snapshots and the per-pass session retry ride `scan_interval`
    /// (#50). Routed through the scope latch on purpose: "event-driven detection unavailable" stays
    /// **one** message family with one reason per cause, and a later scope decline — or a recovery,
    /// which clears the latch — re-reports correctly.
    fn note_degraded_session_if_needed(&mut self) {
        if !self.config.events_driven || self.event_source.is_some() {
            return;
        }
        self.note_event_scope_declined(format!(
            "no usable proton-drive CLI session (locked keyring, headless host, or the CLI is not \
             logged in); the {}s event poll is off, so full-tree snapshots and the session retry \
             both run on the {}s scan interval",
            self.events_poll_interval.as_secs(),
            self.config.scan_interval.as_secs(),
        ));
    }

    /// Whether an incremental (event-stream) pass may be attempted this cycle. Requires the
    /// feature on with a usable event source, a resolvable event scope (a volume id **and** a
    /// stored cursor to replay from — see [`Self::resolve_event_scope`]), and that the opt-in
    /// periodic safety resync (disabled by default) is not currently due.
    fn should_try_incremental(&mut self, base_records: &HashMap<PathBuf, FileRecord>) -> bool {
        if !self.config.events_driven || self.event_source.is_none() {
            return false;
        }
        // Periodic safety resync (opt-in). `events_full_scan_every == 0` maps to `u64::MAX` here,
        // a threshold the counter cannot reach in any realistic runtime after the startup floor
        // resets it to 0 (that would take 2^64 passes) — so a disabled resync leaves the daemon
        // event-driven until restart or an event-stream fallback.
        if self.incremental_passes_since_full_scan
            >= effective_full_scan_every(self.config.events_full_scan_every)
        {
            return false;
        }
        self.resolve_event_scope(base_records).is_some()
    }

    /// The event scope an incremental pass replays from: `(volume id, its stored cursor)`.
    ///
    /// `None` unless a **real cursor** exists, so no caller can engage the event-driven path
    /// without one. Every `None` records a reason and logs it once (a standing decline used to be
    /// silent, and "the daemon keeps doing full syncs" is told apart only by the log line);
    /// resolving again clears the record so a later regression re-reports.
    fn resolve_event_scope(
        &mut self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> Option<(String, EventCursor)> {
        let volume = match self.volume_id_for_scope(base_records) {
            Ok(volume) => volume,
            Err(reason) => {
                self.note_event_scope_declined(reason);
                return None;
            }
        };
        match load_event_cursor(&self.connection, &volume) {
            Ok(Some(cursor)) => {
                self.event_scope_declined = None;
                Some((volume, cursor))
            }
            Ok(None) => {
                self.note_event_scope_declined(format!(
                    "no stored event cursor for volume {volume}"
                ));
                None
            }
            Err(error) => {
                self.note_event_scope_declined(format!(
                    "the stored event cursor for volume {volume} is unreadable: {error}"
                ));
                None
            }
        }
    }

    /// The volume id this root's event stream is scoped to, or a human-readable reason it cannot be
    /// determined. Derived from any baseline composed `proton_id` first (free, no I/O).
    fn volume_id_for_scope(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> Result<String, String> {
        if let Some(volume) = derive_volume_id(base_records) {
            return Ok(volume.to_owned());
        }
        // No baseline composed id: a brand-new sync that has recorded nothing, or a remote that is
        // entirely Proton-native (unsupported files get no index row). The sole stored cursor's
        // scope id *is* the volume a previous bootstrap anchored, so the gate can still engage
        // (#32 — it used to stay on full-tree walks forever here).
        match load_sole_event_cursor(&self.connection) {
            Ok(Some(cursor)) => Ok(cursor.scope_id),
            Ok(None) => Err(
                "no event volume: no indexed node carries a composed proton_id, and no single \
                 stored cursor names one"
                    .to_owned(),
            ),
            Err(error) => Err(format!(
                "no event volume: the stored event cursor is unreadable: {error}"
            )),
        }
    }

    /// Reports why the event-driven path is unavailable, once per distinct cause. Info level: this
    /// is the only signal distinguishing "still full-walking because it cannot stream" from the
    /// other causes of repeated full syncs.
    ///
    /// The reasons that reach here are the *standing* ones — no session
    /// ([`Self::note_degraded_session_if_needed`]), no volume, no/unreadable cursor. A pass that
    /// started streaming and then gave up logs the separate per-pass line ("event-driven pass fell
    /// back to a full-tree snapshot"), which is deliberately a different message.
    fn note_event_scope_declined(&mut self, reason: String) {
        if self.event_scope_declined.as_deref() != Some(reason.as_str()) {
            info!(%reason, "event-driven detection unavailable; using full-tree walks");
            self.event_scope_declined = Some(reason);
        }
    }

    /// One incremental pass: fetch the event delta, reconstruct the complete remote map as
    /// `base ⊕ delta`, plan, execute, and advance the cursor inside the post-side-effects commit.
    /// Returns [`IncrementalOutcome::Fallback`] (without committing) whenever the delta cannot be
    /// turned into a complete map, so the caller re-bootstraps.
    ///
    /// `force_local_scan` skips the idle fast-path so the local stat-walk always runs. Two callers
    /// set it: the warm start (first pass after boot — `pending_changes` is empty on a fresh
    /// process, so the fast-path would strand a file edited while the daemon was down), and a
    /// steady-state pass after a watcher error dropped events (#51, `force_local_rescan`).
    /// Otherwise steady-state passes leave it `false` to keep the O(1) idle poll cheap.
    fn try_incremental_reconcile(
        &mut self,
        base_records: &HashMap<PathBuf, FileRecord>,
        force_local_scan: bool,
    ) -> AppResult<IncrementalOutcome> {
        let Some((volume, cursor)) = self.resolve_event_scope(base_records) else {
            return Ok(IncrementalOutcome::Fallback(
                "no event volume or stored cursor".to_owned(),
            ));
        };

        // Fetch the delta (paginating) *before* the local scan so a fully idle cycle does no work.
        self.shared
            .begin_activity(new_activity(PHASE_FETCHING_EVENTS));
        let delta = match self.fetch_event_delta(&volume, &cursor.last_event_id) {
            Ok(delta) => delta,
            Err(error) => {
                return Ok(IncrementalOutcome::Fallback(format!(
                    "events fetch failed: {error}"
                )));
            }
        };
        if delta.refresh {
            return Ok(IncrementalOutcome::Fallback(
                "server requested a full refresh".to_owned(),
            ));
        }

        // Idle: no remote changes, no pending local changes, and nothing withheld awaiting approval
        // → advance the cursor and skip the local stat-walk and planning entirely. The pending-
        // deletions guard matters for a withheld `RemoteDelete` (a *local* deletion): it is tracked
        // via `pending_changes`, which the prior pass cleared on commit, and it produces no remote
        // event — so without this clause the next pass would idle-skip planning and never re-derive
        // it, leaving an approved delete unapplied until the periodic bootstrap. Forcing a plan here
        // re-derives it from ground truth every pass (and recomputes the pending list), so an
        // approval applies on the very next reconcile; once resolved, the pass goes idle again. (A
        // withheld `LocalDelete` is already non-idle: the held cursor keeps its event in the delta.)
        // `force_local_scan` (a warm start) suppresses this fast-path: even with an empty delta it
        // must run the local stat-walk to catch offline edits `pending_changes` cannot know about.
        if !force_local_scan
            && delta.changes.is_empty()
            && self.pending_changes.is_empty()
            && self.pending_deletions.is_empty()
        {
            if delta.latest_event_id != cursor.last_event_id {
                store_event_cursor(
                    &self.connection,
                    &volume,
                    &delta.latest_event_id,
                    current_epoch_secs() as i64,
                )?;
            }
            self.incremental_passes_since_full_scan =
                self.incremental_passes_since_full_scan.saturating_add(1);
            info!("event-driven pass idle; no remote or local changes");
            return Ok(IncrementalOutcome::Idle);
        }

        let local_entities = self.scan_local_entities_reporting_progress(base_records)?;
        let local_files = local_files_from_entities(&local_entities);
        let base_index = filter_base_index(base_records.clone(), &self.scan_options);

        let remote_entities = {
            // Built here and dropped at the end of this block: its listing memo lives exactly as
            // long as the pass.
            let resolver = TargetedResolver {
                proton: &self.proton,
                connection: &self.connection,
                remote_root: &self.config.remote_root,
                volume_id: &volume,
                listings: RefCell::new(HashMap::new()),
            };
            match reconstruct_remote(
                &base_index,
                &delta.changes,
                &volume,
                &self.scan_options,
                &resolver,
            ) {
                Reconstruction::Complete(map) => map,
                Reconstruction::FallbackToSnapshot(reason) => {
                    return Ok(IncrementalOutcome::Fallback(reason));
                }
            }
        };

        self.execute_plan_and_commit(
            &local_entities,
            &local_files,
            &remote_entities,
            &base_index,
            false,
            Some(CursorUpdate {
                scope_id: volume,
                last_event_id: delta.latest_event_id,
            }),
        )?;
        self.incremental_passes_since_full_scan =
            self.incremental_passes_since_full_scan.saturating_add(1);
        Ok(IncrementalOutcome::Committed)
    }

    /// Fetches and concatenates the volume event delta from `from_cursor`, following `more`
    /// pagination. Surfaces a `refresh` signal and the final cursor to persist.
    fn fetch_event_delta(&self, volume: &str, from_cursor: &str) -> AppResult<EventDelta> {
        // A generous bound purely to stop a misbehaving stream from looping forever; real deltas
        // paginate in a handful of pages.
        const MAX_PAGES: usize = 10_000;
        let source = self
            .event_source
            .as_ref()
            .expect("should_try_incremental guarantees an event source");
        let mut changes = Vec::new();
        let mut cursor = from_cursor.to_owned();
        for _ in 0..MAX_PAGES {
            let page = source.events_since(volume, &cursor)?;
            if page.refresh {
                return Ok(EventDelta {
                    changes,
                    latest_event_id: page.latest_event_id,
                    refresh: true,
                });
            }
            changes.extend(page.changes);
            if !page.more {
                return Ok(EventDelta {
                    changes,
                    latest_event_id: page.latest_event_id,
                    refresh: false,
                });
            }
            cursor = page.latest_event_id;
        }
        Err(boxed_error(
            "volume event stream did not finish paginating within the page limit",
        ))
    }

    /// The daemon's local scan, surfacing per-file progress into [`ControlShared`] so a slow
    /// pass over large files (SHA-1 hashing is the cost) reads as alive in status replies.
    fn scan_local_entities_reporting_progress(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> AppResult<HashMap<PathBuf, LocalEntityState>> {
        self.shared
            .begin_activity(new_activity(PHASE_SCANNING_LOCAL));
        let observer = |files_seen: u64, path: &Path| {
            let display = path
                .strip_prefix(&self.config.local_root)
                .unwrap_or(path)
                .display()
                .to_string();
            self.shared
                .update_activity(PHASE_SCANNING_LOCAL, |activity| {
                    activity.files_scanned = Some(files_seen);
                    activity.detail = Some(display);
                });
        };
        scan_local_entities_observed(
            &self.config.local_root,
            &self.scan_options,
            base_records,
            Some(&observer),
        )
    }

    /// Full-tree snapshot reconcile — the original behavior, plus (when event-driven) capturing
    /// and persisting the replay cursor `C0`. Resets the periodic-resync counter.
    fn bootstrap_reconcile(&mut self, base_records: HashMap<PathBuf, FileRecord>) -> AppResult<()> {
        // Market-data recovery: capture the cursor *before* the snapshot when the volume is
        // already known, so a change landing during the walk is re-delivered (idempotently) by
        // the next incremental pass. On the first-ever bootstrap the volume is only known after
        // the walk, so a change landing in an already-listed folder mid-walk is missed by this
        // snapshot *and* precedes the post-walk cursor — a one-time gap that heals on the next
        // process restart (the startup floor re-snapshots) or, if the opt-in periodic resync is
        // enabled (`events_full_scan_every = N`), within N passes. With the default (0) it stays
        // until restart; enable the periodic resync if that bound matters for your deployment.
        let pre_snapshot_cursor = self.capture_pre_snapshot_cursor(&base_records);

        let local_entities = self.scan_local_entities_reporting_progress(&base_records)?;
        let local_files = local_files_from_entities(&local_entities);
        // The client's progress sink updates the per-folder count/path from inside the walk;
        // this just flips the phase so status shows "listing remote" the moment it starts.
        self.shared
            .begin_activity(new_activity(PHASE_LISTING_REMOTE));
        let (remote_entities, remote_root_missing) =
            load_remote_entities(&self.proton, &self.config.remote_root, &self.scan_options)?;
        let mut base_index = filter_base_index(base_records, &self.scan_options);
        if remote_root_missing {
            base_index.clear();
        }

        let cursor_update = self.resolve_bootstrap_cursor_update(
            pre_snapshot_cursor,
            &remote_entities,
            remote_root_missing,
        );

        self.execute_plan_and_commit(
            &local_entities,
            &local_files,
            &remote_entities,
            &base_index,
            remote_root_missing,
            cursor_update,
        )?;
        self.incremental_passes_since_full_scan = 0;
        // A full walk is the self-healing event warm starts count toward: reset the across-restart
        // warm-start floor so the every-N cadence restarts from this fresh baseline. Best-effort —
        // see `record_successful_warm_start`.
        if self.warm_starts_since_full_walk != 0 {
            self.warm_starts_since_full_walk = 0;
            if let Err(error) = store_warm_start_count(&self.connection, 0) {
                warn!(%error, "failed to reset the warm-start counter after a full walk");
            }
        }
        Ok(())
    }

    /// Reads the current latest cursor before a snapshot, when event-driven and the volume is
    /// already known (from the baseline or a previously stored cursor). Best-effort: a failure just
    /// defers cursor capture to after the snapshot.
    fn capture_pre_snapshot_cursor(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> Option<CursorUpdate> {
        if !self.config.events_driven {
            return None;
        }
        let source = self.event_source.as_ref()?;
        let volume = self.volume_id_for_scope(base_records).ok()?;
        match source.latest_cursor(&volume) {
            Ok(last_event_id) => Some(CursorUpdate {
                scope_id: volume,
                last_event_id,
            }),
            Err(error) => {
                warn!(%error, "could not capture the pre-snapshot events cursor; deriving it after the snapshot instead");
                None
            }
        }
    }

    /// Chooses the cursor to persist with a bootstrap: the pre-snapshot one if captured, else
    /// (first-ever bootstrap) the volume derived from the fresh snapshot with its latest cursor
    /// read now. Returns `None` when event-driven is off or nothing can be anchored yet.
    fn resolve_bootstrap_cursor_update(
        &self,
        pre_snapshot_cursor: Option<CursorUpdate>,
        remote_entities: &HashMap<PathBuf, RemoteEntity>,
        remote_root_missing: bool,
    ) -> Option<CursorUpdate> {
        if !self.config.events_driven || remote_root_missing {
            return None;
        }
        if let Some(cursor) = pre_snapshot_cursor {
            return Some(cursor);
        }
        let source = self.event_source.as_ref()?;
        let volume = derive_volume_id_from_entities(remote_entities)?;
        match source.latest_cursor(&volume) {
            Ok(last_event_id) => Some(CursorUpdate {
                scope_id: volume,
                last_event_id,
            }),
            Err(error) => {
                warn!(%error, "could not capture the post-snapshot events cursor; incremental sync stays off until the next successful bootstrap");
                None
            }
        }
    }

    /// Plans against the given complete remote map, executes every action performing all side
    /// effects, and commits the resulting index mutations **incrementally**: after every action
    /// whose side effect succeeded (and after every batched download chunk) the mutations
    /// accumulated so far are checkpoint-committed (`commit_checkpoint`), so a mid-plan failure —
    /// or a daemon shutdown — keeps everything already done durable, and only the failed action
    /// and its unexecuted successors re-plan next pass. The commit-after-side-effects invariant
    /// is unchanged in the direction that matters: an index write still never precedes its side
    /// effect. The event cursor is the deliberate exception — it advances only in the final
    /// commit of a fully-successful pass, because it asserts "every remote change up to this
    /// event has been applied", which holds only when the whole plan landed.
    fn execute_plan_and_commit(
        &mut self,
        local_entities: &HashMap<PathBuf, LocalEntityState>,
        local_files: &HashMap<PathBuf, LocalFileState>,
        remote_entities: &HashMap<PathBuf, RemoteEntity>,
        base_index: &HashMap<PathBuf, FileRecord>,
        remote_root_missing: bool,
        cursor_update: Option<CursorUpdate>,
    ) -> AppResult<()> {
        let mut plan = plan_sync_entities(local_entities, remote_entities, base_index);
        prepend_remote_root_creation_if_missing(&mut plan, remote_root_missing);
        let plan_summary = PlanSummary::from_plan(&plan);
        self.last_plan_summary = Some(plan_summary.clone());

        // Delete-approval gate: decide, per destructive action, whether to execute it now or
        // withhold it pending the user's approval (see `decide_delete_gate`). This filters the
        // plan at execution time; the planner itself stays pure.
        let DeleteGate {
            withheld_paths,
            pending,
            consumed_approvals: approved_deletes,
        } = self.decide_delete_gate(&plan, base_index)?;
        // A withheld deletion originates from ground truth (a remote-delete event, or a missing
        // local file). If any deletion is withheld this pass, do NOT advance the event cursor:
        // otherwise a withheld `LocalDelete`'s originating event would fall out of future deltas
        // (reconstruct overlays only the *new* delta onto the surviving baseline) and the pending
        // item would vanish from the queue until the next full-tree resync. Holding the cursor
        // keeps every pending deletion re-derived from ground truth each pass, so an approval
        // applies promptly and the queue never goes stale. The cursor resumes advancing the first
        // pass with nothing withheld.
        let cursor_update = if withheld_paths.is_empty() {
            cursor_update
        } else {
            None
        };
        self.pending_deletions = pending;
        // Publish now — not just at pass end — so a status poll issued during the transfers
        // below already reports this pass's plan and pending deletions.
        self.publish_status();

        info!(
            planned_actions = plan_summary.total,
            uploads = plan_summary.uploads,
            downloads = plan_summary.downloads,
            conflicts = plan_summary.conflicts,
            skipped_unsupported = plan_summary.skipped_unsupported,
            destructive_actions = plan_summary.destructive_actions,
            blocked_awaiting_approval = self.pending_deletions.len(),
            "sync plan computed"
        );

        let mut index_mutations = Vec::new();
        let planned_remote_directories: BTreeSet<PathBuf> = plan
            .iter()
            .filter(|action| action.action == SyncAction::CreateRemoteDirectory)
            .map(|action| action.path.clone())
            .collect();

        // Foreground progress: show a live per-file spinner when stderr is a terminal, counting
        // `[i/N]` through the transfers in this plan. Under systemd/journald stderr is not a
        // terminal, so `begin_transfer_spinner` returns `None` and each arm falls back to its
        // `info!` trace line instead (no progress-bar escape codes in the journal).
        let interactive_progress = std::io::stderr().is_terminal();
        let transfer_total = plan
            .iter()
            .filter(|action| matches!(action.action, SyncAction::Upload | SyncAction::Download))
            .count();
        let mut transfer_index = 0usize;
        let action_total = plan.len() as u64;
        let mut pending_approval_consumptions: Vec<(PathBuf, DeleteDirection)> = Vec::new();

        let mut action_number = 0usize;
        while action_number < plan.len() {
            let action = &plan[action_number];
            // A run of two-plus consecutive planned downloads executes as chunked multi-file
            // CLI invocations instead of one subprocess per file (grouped by destination
            // directory — see `execute_download_run`); a run of one takes the single-file arm
            // below. `download_batch_size = 1` disables batching entirely.
            if self.config.download_batch_size > 1 && action.action == SyncAction::Download {
                let run_length = plan[action_number..]
                    .iter()
                    .take_while(|action| action.action == SyncAction::Download)
                    .count();
                if run_length > 1 {
                    self.execute_download_run(
                        &plan[action_number..action_number + run_length],
                        action_number,
                        action_total,
                        transfer_total,
                        &mut transfer_index,
                        remote_entities,
                        interactive_progress,
                        &mut index_mutations,
                        &mut pending_approval_consumptions,
                    )?;
                    action_number += run_length;
                    continue;
                }
            }
            let checkpoint_after = action_performs_side_effects(&action.action);
            debug!(path = %action.path.display(), action = ?action.action, "executing sync action");
            self.shared.begin_activity(SyncActivity {
                // Root-level actions have an empty relative path; skip it rather than render
                // a trailing space ("creating remote folder ").
                detail: Some(if action.path.as_os_str().is_empty() {
                    activity_verb(&action.action).to_owned()
                } else {
                    format!(
                        "{} {}",
                        activity_verb(&action.action),
                        action.path.display()
                    )
                }),
                action_index: Some(action_number as u64 + 1),
                action_total: Some(action_total),
                ..new_activity(PHASE_EXECUTING)
            });
            // Labeled so an arm can bail out of just this action (`break 'action`, the old
            // loop `continue`) while still reaching the per-action checkpoint below.
            'action: {
                match action.action {
                    SyncAction::Upload => {
                        if let Some(local) = local_files.get(&action.path) {
                            if let Some(parent) = action.path.parent()
                                && !parent.as_os_str().is_empty()
                                && !planned_remote_directories.contains(parent)
                            {
                                self.proton
                                    .ensure_directory(&self.config.remote_root, parent)?;
                            }
                            transfer_index += 1;
                            self.shared.update_activity(PHASE_EXECUTING, |activity| {
                                activity.transfer = Some(TransferActivity {
                                    direction: "upload".to_owned(),
                                    path: action.path.clone(),
                                    bytes_total: Some(local.file_size),
                                    bytes_done: None,
                                    started_epoch_secs: current_epoch_secs(),
                                });
                            });
                            let spinner = begin_transfer_spinner(
                                interactive_progress,
                                transfer_index,
                                transfer_total,
                                "uploading",
                                &action.path,
                                Some(local.file_size),
                            );
                            if spinner.is_none() {
                                info!(
                                    target: TRANSFER_LOG_TARGET,
                                    path = %action.path.display(),
                                    size_bytes = local.file_size,
                                    "uploading file to Proton Drive"
                                );
                            }
                            let result = self.proton.upload(
                                &local.absolute_path,
                                &self.config.remote_root,
                                &action.path,
                            );
                            finish_transfer_spinner(spinner);
                            result?;
                            let record = FileRecord::from_local(
                                action.path.clone(),
                                local,
                                action.remote_id.clone().or_else(|| {
                                    base_index
                                        .get(&action.path)
                                        .and_then(|record| record.proton_id.clone())
                                }),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                    }
                    SyncAction::Download => {
                        let remote_id = action.remote_id.as_deref().ok_or_else(|| {
                            boxed_error(format!(
                                "planned download for {} is missing a remote id",
                                action.path.display()
                            ))
                        })?;
                        let remote_path = safe_remote_path(&self.config.remote_root, &action.path)
                            .ok_or_else(|| {
                                boxed_error(format!(
                                    "planned download for {} has an unsafe remote path",
                                    action.path.display()
                                ))
                            })?;
                        let Some(destination) =
                            safe_local_path(&self.config.local_root, &action.path)
                        else {
                            warn!(
                                path = %action.path.display(),
                                "skipping download: local destination escapes the sync root \
                                 (e.g. through a symlinked directory)"
                            );
                            break 'action;
                        };
                        ensure_parent_directory(&destination)?;
                        transfer_index += 1;
                        // `bytes_total` stays unknown (the remote listing carries no size), but
                        // `bytes_done` is sampled live from the staging directory the client
                        // reports via the progress sink, so status still shows a growing count.
                        self.shared.update_activity(PHASE_EXECUTING, |activity| {
                            activity.transfer = Some(TransferActivity {
                                direction: "download".to_owned(),
                                path: action.path.clone(),
                                bytes_total: None,
                                bytes_done: None,
                                started_epoch_secs: current_epoch_secs(),
                            });
                        });
                        let spinner = begin_transfer_spinner(
                            interactive_progress,
                            transfer_index,
                            transfer_total,
                            "downloading",
                            &action.path,
                            None,
                        );
                        if spinner.is_none() {
                            info!(
                                target: TRANSFER_LOG_TARGET,
                                path = %action.path.display(),
                                remote_id,
                                "downloading file from Proton Drive"
                            );
                        }
                        let result = self.proton.download(&remote_path, &destination);
                        finish_transfer_spinner(spinner);
                        result?;
                        // The write we just landed will echo back through the watcher; record it so
                        // `handle_fs_event` does not flip this fresh `Synced` record to `Modified`
                        // (issue #49). Downloads always target a regular file.
                        self.authored_writes.insert(action.path.clone());
                        let local_state = local_file_state(&self.config.local_root, &destination)?;
                        let record = FileRecord::from_local(
                            action.path.clone(),
                            &local_state,
                            Some(remote_id.to_owned()),
                            SyncStatus::Synced,
                        );
                        index_mutations.push(IndexMutation::Upsert(record));
                    }
                    SyncAction::CreateRemoteDirectory => {
                        if action.path.as_os_str().is_empty() {
                            self.proton
                                .ensure_root_directory(&self.config.remote_root)?;
                            break 'action;
                        }
                        if let Some(LocalEntityState::Directory(local)) =
                            local_entities.get(&action.path)
                        {
                            self.proton
                                .ensure_directory(&self.config.remote_root, &action.path)?;
                            let record = FileRecord::from_local_directory(
                                action.path.clone(),
                                local,
                                action.remote_id.clone().or_else(|| {
                                    base_index
                                        .get(&action.path)
                                        .and_then(|record| record.proton_id.clone())
                                }),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                    }
                    SyncAction::CreateLocalDirectory => {
                        if let Some(destination) =
                            safe_local_path(&self.config.local_root, &action.path)
                        {
                            fs::create_dir_all(&destination)?;
                            let local_state =
                                local_directory_state(&self.config.local_root, &destination)?;
                            let record = FileRecord::from_local_directory(
                                action.path.clone(),
                                &local_state,
                                action.remote_id.clone(),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                    }
                    SyncAction::MoveLocal => {
                        if let Some(destination_path) = action.destination_path.as_ref()
                            && let Some(source) =
                                safe_local_path(&self.config.local_root, &action.path)
                            && let Some(destination) =
                                safe_local_path(&self.config.local_root, destination_path)
                        {
                            ensure_parent_directory(&destination)?;
                            fs::rename(&source, &destination)?;
                            if action.entity_kind == EntityKind::Directory {
                                let local_state =
                                    local_directory_state(&self.config.local_root, &destination)?;
                                let record = FileRecord::from_local_directory(
                                    destination_path.clone(),
                                    &local_state,
                                    action.remote_id.clone(),
                                    SyncStatus::Synced,
                                );
                                index_mutations.push(IndexMutation::Purge(action.path.clone()));
                                index_mutations.push(IndexMutation::Upsert(record));
                                for (old_descendant, new_descendant) in
                                    directory_move_descendant_path_pairs(
                                        &action.path,
                                        destination_path,
                                        base_index,
                                    )
                                {
                                    if let Some(descendant_record) = base_index.get(&old_descendant)
                                    {
                                        index_mutations
                                            .push(IndexMutation::Purge(old_descendant.clone()));
                                        index_mutations.push(IndexMutation::Upsert(FileRecord {
                                            file_path: new_descendant,
                                            ..descendant_record.clone()
                                        }));
                                    }
                                }
                            } else {
                                // The renamed-into-place file echoes through the watcher; record
                                // its destination so `handle_fs_event` does not flip the fresh
                                // `Synced` record to `Modified` (issue #49). Directory moves are
                                // already ignored by the `path.is_dir()` guard at the top of
                                // `handle_fs_event`, so only the file branch records.
                                self.authored_writes.insert(destination_path.clone());
                                let local_state =
                                    local_file_state(&self.config.local_root, &destination)?;
                                let record = FileRecord::from_local(
                                    destination_path.clone(),
                                    &local_state,
                                    action.remote_id.clone(),
                                    SyncStatus::Synced,
                                );
                                index_mutations.push(IndexMutation::Purge(action.path.clone()));
                                index_mutations.push(IndexMutation::Upsert(record));
                            }
                        }
                    }
                    SyncAction::MoveRemote => {
                        if let Some(destination_path) = action.destination_path.as_ref()
                            && let Some(local) = local_files.get(destination_path)
                        {
                            // The destination's parent folder may not exist on the remote yet:
                            // a move is a transition action prepended ahead of the
                            // `CreateRemoteDirectory` that would make it, so `rename_or_move`
                            // would fail with "Node not found" (the poison-pill loop of #141).
                            // Ensure it first (idempotent mkdir-p; the later create degrades to a
                            // no-op) — mirroring the `Upload` and `MoveLocal` arms, which already
                            // ensure their destination parent. Unconditional, not gated on
                            // `planned_remote_directories`: unlike an upload (planned after its
                            // parent's create), a move runs *before* that create.
                            if let Some(parent) = destination_path.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                self.proton
                                    .ensure_directory(&self.config.remote_root, parent)?;
                            }
                            self.proton.rename_or_move(
                                &self.config.remote_root,
                                &action.path,
                                destination_path,
                            )?;
                            let record = FileRecord::from_local(
                                destination_path.clone(),
                                local,
                                action.remote_id.clone(),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Purge(action.path.clone()));
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                    }
                    SyncAction::AutoLink => match local_entities.get(&action.path) {
                        Some(LocalEntityState::File(local)) => {
                            let record = FileRecord::from_local(
                                action.path.clone(),
                                local,
                                action.remote_id.clone(),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                        Some(LocalEntityState::Directory(local)) => {
                            let record = FileRecord::from_local_directory(
                                action.path.clone(),
                                local,
                                action.remote_id.clone(),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                        None => {}
                    },
                    SyncAction::Conflict => {
                        if action.sidecar_from_local_copy {
                            // The remote node is confirmed gone, so the sidecar is a copy of the
                            // surviving local file (#46). Recording the conflict without the
                            // sidecar is the frozen state this action exists to end, so anything
                            // that stops the copy skips the index write too and re-plans.
                            let (Some(conflict_path), Some(local)) =
                                (action.conflict_path.as_ref(), local_files.get(&action.path))
                            else {
                                break 'action;
                            };
                            let Some(destination) =
                                safe_local_path(&self.config.local_root, conflict_path)
                            else {
                                warn!(
                                    path = %action.path.display(),
                                    "skipping conflict: the sidecar destination escapes the sync \
                                     root (e.g. through a symlinked directory)"
                                );
                                break 'action;
                            };
                            // Never clobber or write *through* whatever is already at the sidecar
                            // path. `symlink_metadata` classifies a symlink as itself, so a
                            // dangling one reads as present here — `Path::exists()` follows the
                            // link and answers "absent", after which `fs::copy` would push the
                            // local file's bytes through it, outside the sync root or over
                            // another file inside it. `local_write_escapes_root` cannot catch
                            // that either: it canonicalizes the deepest *existing* ancestor, and
                            // a dangling link has none.
                            match fs::symlink_metadata(&destination) {
                                Ok(metadata) => {
                                    // Occupied. A regular file is an unresolved sidecar from an
                                    // earlier pass (or the user's own copy) and is left alone;
                                    // anything else is the user's object and is never replaced.
                                    // The conflict is still recorded below either way, so the
                                    // pass converges instead of re-planning this action forever —
                                    // and removing the squatter emits the same sidecar-removal
                                    // event that drives the ordinary exit.
                                    if !metadata.is_file() {
                                        warn!(
                                            path = %destination.display(),
                                            "conflict sidecar not written: the path is already \
                                             taken by a symlink or directory, which is never \
                                             replaced"
                                        );
                                    }
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                    ensure_parent_directory(&destination)?;
                                    copy_into_new_file(&local.absolute_path, &destination)?;
                                }
                                Err(error) => return Err(error.into()),
                            }
                        } else if action.remote_id.is_some()
                            && let Some(remote_path) =
                                safe_remote_path(&self.config.remote_root, &action.path)
                            && let Some(conflict_path) = action.conflict_path.as_ref()
                            && let Some(destination) =
                                safe_local_path(&self.config.local_root, conflict_path)
                        {
                            ensure_parent_directory(&destination)?;
                            self.proton.download(&remote_path, &destination)?;
                        }
                        if let Some(local) = local_files.get(&action.path) {
                            let record = FileRecord::from_local(
                                action.path.clone(),
                                local,
                                action.remote_id.clone().or_else(|| {
                                    base_index
                                        .get(&action.path)
                                        .and_then(|record| record.proton_id.clone())
                                }),
                                SyncStatus::Conflict,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                        }
                    }
                    SyncAction::TypeConflict => {
                        if action.entity_kind == EntityKind::Directory
                            && let Some(conflict_path) = action.conflict_path.as_ref()
                            && let Some(LocalEntityState::Directory(local_directory)) =
                                local_entities.get(&action.path)
                        {
                            if action.remote_id.is_some()
                                && let Some(remote_path) =
                                    safe_remote_path(&self.config.remote_root, &action.path)
                                && let Some(destination) =
                                    safe_local_path(&self.config.local_root, conflict_path)
                            {
                                ensure_parent_directory(&destination)?;
                                self.proton.download(&remote_path, &destination)?;
                                let local_state =
                                    local_file_state(&self.config.local_root, &destination)?;
                                let sidecar_record = FileRecord::from_local(
                                    conflict_path.clone(),
                                    &local_state,
                                    action.remote_id.clone(),
                                    SyncStatus::Conflict,
                                );
                                index_mutations.push(IndexMutation::Upsert(sidecar_record));
                            }
                            let directory_record = FileRecord::from_local_directory(
                                action.path.clone(),
                                local_directory,
                                None,
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(directory_record));
                        } else {
                            warn!(
                                path = %action.path.display(),
                                "skipping sync action because local and remote entity types differ"
                            );
                        }
                    }
                    SyncAction::RemoteDelete => {
                        if withheld_paths.contains(&action.path) {
                            // Guard on here and not yet approved: skip the deletion AND its index
                            // mutation, so nothing is lost and it re-plans (still pending) next pass.
                            break 'action;
                        }
                        action.remote_id.as_deref().ok_or_else(|| {
                            boxed_error(format!(
                                "planned remote delete for {} is missing a remote id",
                                action.path.display()
                            ))
                        })?;
                        let remote_path = safe_remote_path(&self.config.remote_root, &action.path)
                            .ok_or_else(|| {
                                boxed_error(format!(
                                    "planned remote delete for {} has an unsafe remote path",
                                    action.path.display()
                                ))
                            })?;
                        self.proton.delete(&remote_path)?;
                        index_mutations.push(IndexMutation::Purge(action.path.clone()));
                        if action.entity_kind == EntityKind::Directory {
                            for descendant in descendant_index_paths(&action.path, base_index) {
                                index_mutations.push(IndexMutation::Purge(descendant));
                            }
                        }
                        record_approval_consumption(
                            &approved_deletes,
                            &action.path,
                            DeleteDirection::Remote,
                            &mut pending_approval_consumptions,
                        );
                    }
                    SyncAction::LocalDelete => {
                        if withheld_paths.contains(&action.path) {
                            // Guard on here and not yet approved: skip the deletion AND its index
                            // mutation, so nothing is lost and it re-plans (still pending) next pass.
                            break 'action;
                        }
                        let Some(destination) =
                            safe_local_path(&self.config.local_root, &action.path)
                        else {
                            warn!(
                                path = %action.path.display(),
                                "skipping local delete: path escapes the sync root \
                                 (e.g. through a symlinked directory)"
                            );
                            break 'action;
                        };
                        if destination.exists() {
                            if action.entity_kind == EntityKind::Directory {
                                fs::remove_dir_all(&destination)?;
                            } else {
                                fs::remove_file(&destination)?;
                            }
                        }
                        index_mutations.push(IndexMutation::Purge(action.path.clone()));
                        if action.entity_kind == EntityKind::Directory {
                            for descendant in descendant_index_paths(&action.path, base_index) {
                                index_mutations.push(IndexMutation::Purge(descendant));
                            }
                        }
                        record_approval_consumption(
                            &approved_deletes,
                            &action.path,
                            DeleteDirection::Local,
                            &mut pending_approval_consumptions,
                        );
                    }
                    SyncAction::Purge => {
                        index_mutations.push(IndexMutation::Purge(action.path.clone()));
                    }
                    SyncAction::SkipUnsupported => {
                        debug!(
                            path = %action.path.display(),
                            remote_id = ?action.remote_id,
                            "skipping Proton-native file that proton-drive cannot download"
                        );
                    }
                }
            }
            if checkpoint_after {
                // Land this action's outcome durably before moving on: a later failure — or a
                // shutdown mid-pass — can no longer discard work that already happened. Index-only
                // actions (AutoLink/Purge) accumulate instead, so an adoption-heavy pass stays a
                // few large transactions rather than thousands of per-row fsyncs.
                commit_checkpoint(
                    &mut self.connection,
                    &mut index_mutations,
                    &mut pending_approval_consumptions,
                )?;
            }
            action_number += 1;
        }

        self.shared.begin_activity(new_activity(PHASE_COMMITTING));
        let transaction = self.connection.transaction()?;
        // Whatever accumulated since the last checkpoint (trailing index-only mutations, plus any
        // approval consumption not yet flushed) commits here together with the cursor.
        for mutation in &index_mutations {
            mutation.apply(&transaction)?;
        }
        // Advance the event cursor ONLY in this final, whole-pass-succeeded transaction — never in
        // a mid-pass checkpoint: the cursor asserts "every remote change up to this event has been
        // applied", so a mid-plan failure must replay the same events next pass rather than
        // silently skipping them. Reprocessing events is idempotent; skipping them loses changes.
        if let Some(cursor_update) = &cursor_update {
            store_event_cursor(
                &transaction,
                &cursor_update.scope_id,
                &cursor_update.last_event_id,
                current_epoch_secs() as i64,
            )?;
        }
        for (path, direction) in &pending_approval_consumptions {
            delete_delete_approval(&transaction, path, *direction)?;
        }
        transaction.commit()?;
        // `pending_changes` is a wake-up/status hint, not a plan input, and every pass that
        // reaches this commit ran the local stat-walk (the events-mode idle fast-path returns
        // before `execute_plan_and_commit`), so clearing the whole set here cannot lose work: the
        // scan already observed every queued path. An event that arrived *during* the pass is
        // still sitting in the watcher channel — the loop drains it only after this blocking
        // reconcile returns — so it re-inserts itself afterwards. Clearing unconditionally rather
        // than per planned path also avoids leaking entries that produced no action (a directory
        // event, a misclassified removal, a non-regular-file event, a `None` plan outcome).
        self.pending_changes.clear();

        self.last_sync = Some(SystemTime::now());
        self.last_successful_sync_summary = Some(plan_summary);
        info!("reconciliation completed");
        Ok(())
    }

    /// Executes one run of consecutive planned `Download` actions as chunked multi-file CLI
    /// invocations ([`ProtonClient::download_many`]) instead of one subprocess per file. The
    /// run is segmented into *consecutive* same-destination-directory groups (a batch shares
    /// one `localFolder`) — never merging same-directory files across an intervening
    /// subdirectory's downloads, so execution order is exactly plan order — and each segment
    /// is split into chunks of at most `download_batch_size`. Every chunk is
    /// checkpoint-committed as soon as it lands, so a failure — or a daemon shutdown — never
    /// discards transfers that already completed; the first failed file aborts the pass after
    /// its chunk's survivors are committed.
    #[allow(clippy::too_many_arguments)]
    fn execute_download_run(
        &mut self,
        run: &[PlannedAction],
        first_action_number: usize,
        action_total: u64,
        transfer_total: usize,
        transfer_index: &mut usize,
        remote_entities: &HashMap<PathBuf, RemoteEntity>,
        interactive_progress: bool,
        index_mutations: &mut Vec<IndexMutation>,
        pending_approval_consumptions: &mut Vec<(PathBuf, DeleteDirection)>,
    ) -> AppResult<()> {
        let mut groups: Vec<(PathBuf, Vec<PreparedDownload<'_>>)> = Vec::new();
        for (offset, action) in run.iter().enumerate() {
            let remote_id = action.remote_id.as_deref().ok_or_else(|| {
                boxed_error(format!(
                    "planned download for {} is missing a remote id",
                    action.path.display()
                ))
            })?;
            let remote_path =
                safe_remote_path(&self.config.remote_root, &action.path).ok_or_else(|| {
                    boxed_error(format!(
                        "planned download for {} has an unsafe remote path",
                        action.path.display()
                    ))
                })?;
            let Some(destination) = safe_local_path(&self.config.local_root, &action.path) else {
                warn!(
                    path = %action.path.display(),
                    "skipping download: local destination escapes the sync root \
                     (e.g. through a symlinked directory)"
                );
                continue;
            };
            // The remote's claimed digest rides along so a failed chunk can salvage the files
            // that were already fully staged (verified by content, not by exit status).
            let expected_sha1 = remote_entities
                .get(&action.path)
                .and_then(|entity| entity.as_file())
                .and_then(|file| file.sha1_hash.clone());
            let parent = action
                .path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
            let prepared = PreparedDownload {
                action,
                action_number: first_action_number + offset,
                remote_id: remote_id.to_owned(),
                request: DownloadRequest {
                    remote_path,
                    destination,
                    expected_sha1,
                },
            };
            // Segment, don't bucket: extend the current group only while the parent is
            // unchanged. A same-parent file appearing again after an intervening
            // subdirectory's downloads starts a NEW group, so execution order stays
            // exactly plan order (progress indices stay monotonic and a failure aborts at
            // the same point the per-file path would have).
            match groups.last_mut() {
                Some((group, members)) if *group == parent => members.push(prepared),
                _ => groups.push((parent, vec![prepared])),
            }
        }
        let batch_size = self.config.download_batch_size.max(1);
        for (parent, members) in &groups {
            if let Some(first) = members.first() {
                ensure_parent_directory(&first.request.destination)?;
            }
            for chunk in members.chunks(batch_size) {
                self.execute_download_chunk(
                    parent,
                    chunk,
                    action_total,
                    transfer_total,
                    transfer_index,
                    interactive_progress,
                    index_mutations,
                    pending_approval_consumptions,
                )?;
            }
        }
        Ok(())
    }

    /// Executes one chunk of prepared downloads via a single [`ProtonClient::download_many`]
    /// call, records every landed file, checkpoint-commits, and only then fails the pass if any
    /// file in the chunk failed — completed transfers in a failing chunk stay durable.
    #[allow(clippy::too_many_arguments)]
    fn execute_download_chunk(
        &mut self,
        parent: &Path,
        chunk: &[PreparedDownload<'_>],
        action_total: u64,
        transfer_total: usize,
        transfer_index: &mut usize,
        interactive_progress: bool,
        index_mutations: &mut Vec<IndexMutation>,
        pending_approval_consumptions: &mut Vec<(PathBuf, DeleteDirection)>,
    ) -> AppResult<()> {
        let display_parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let first_transfer_ordinal = *transfer_index + 1;
        *transfer_index += chunk.len();
        let last_action_number = chunk
            .last()
            .map(|prepared| prepared.action_number)
            .unwrap_or(0);
        self.shared.begin_activity(SyncActivity {
            detail: Some(format!(
                "downloading {} files in {}",
                chunk.len(),
                display_parent.display()
            )),
            action_index: Some(last_action_number as u64 + 1),
            action_total: Some(action_total),
            ..new_activity(PHASE_EXECUTING)
        });
        // `bytes_done` is sampled live from the staging directory the client reports via the
        // progress sink; for a chunk it grows across the whole batch. The path names the
        // directory being filled rather than a single file (`display_parent`, so a root-level
        // chunk renders as "." instead of an empty string in `proton-sync status`).
        self.shared.update_activity(PHASE_EXECUTING, |activity| {
            activity.transfer = Some(TransferActivity {
                direction: "download".to_owned(),
                path: display_parent.to_path_buf(),
                bytes_total: None,
                bytes_done: None,
                started_epoch_secs: current_epoch_secs(),
            });
        });
        let verb = format!("downloading {} files from", chunk.len());
        let spinner = begin_transfer_spinner(
            interactive_progress,
            first_transfer_ordinal,
            transfer_total,
            &verb,
            display_parent,
            None,
        );
        if spinner.is_none() {
            info!(
                target: TRANSFER_LOG_TARGET,
                directory = %display_parent.display(),
                files = chunk.len(),
                "downloading batch of files from Proton Drive"
            );
        }
        let requests: Vec<DownloadRequest> = chunk
            .iter()
            .map(|prepared| prepared.request.clone())
            .collect();
        let results = self.proton.download_many(&requests);
        finish_transfer_spinner(spinner);

        let mut first_failure: Option<(PathBuf, String)> = None;
        let record_item_failure =
            |first_failure: &mut Option<(PathBuf, String)>, path: &Path, error: String| {
                warn!(path = %path.display(), error = %error, "batched download failed for file");
                if first_failure.is_none() {
                    *first_failure = Some((path.to_path_buf(), error));
                }
            };
        for (prepared, result) in chunk.iter().zip(results) {
            match result {
                // A stat/hash failure on a landed file (e.g. deleted out from under us the
                // instant after promotion) is that ITEM's failure, never a `?` out of the
                // loop — the chunk's other survivors must still reach the checkpoint below.
                Ok(()) => {
                    match local_file_state(&self.config.local_root, &prepared.request.destination) {
                        Ok(local_state) => {
                            // Suppress this landed download's watcher echo so it cannot flip the
                            // fresh `Synced` record to `Modified` (issue #49); same rationale as
                            // the single-file `Download` arm.
                            self.authored_writes.insert(prepared.action.path.clone());
                            let record = FileRecord::from_local(
                                prepared.action.path.clone(),
                                &local_state,
                                Some(prepared.remote_id.clone()),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                            debug!(
                                target: TRANSFER_LOG_TARGET,
                                path = %prepared.action.path.display(),
                                "downloaded file from Proton Drive (batched)"
                            );
                        }
                        Err(error) => record_item_failure(
                            &mut first_failure,
                            &prepared.action.path,
                            format!("downloaded but could not be recorded: {error}"),
                        ),
                    }
                }
                Err(error) => record_item_failure(
                    &mut first_failure,
                    &prepared.action.path,
                    error.to_string(),
                ),
            }
        }
        // Land this chunk's completed downloads durably before deciding the pass's fate: even
        // when the chunk failed, its survivors are recorded and never re-transferred.
        commit_checkpoint(
            &mut self.connection,
            index_mutations,
            pending_approval_consumptions,
        )?;
        match first_failure {
            None => Ok(()),
            Some((path, error)) => Err(boxed_error(format!(
                "download failed for {}: {error}",
                path.display()
            ))),
        }
    }

    /// Decides, for every destructive action in `plan`, whether to execute it now or withhold it
    /// pending the user's approval. A directory-scoped [`DirectoryConfigResolver`] (rooted at the
    /// daemon-wide default) says whether the direction is guarded at each path; a guarded deletion
    /// executes only if the user has a standing approval matching its exact `fingerprint`, which is
    /// then queued for consumption. Everything else is withheld and reported as pending. `Purge`
    /// (index-only, no data loss) has no delete direction and is never gated.
    fn decide_delete_gate(
        &self,
        plan: &[PlannedAction],
        base_index: &HashMap<PathBuf, FileRecord>,
    ) -> AppResult<DeleteGate> {
        let mut resolver = DirectoryConfigResolver::new(
            &self.config.local_root,
            EffectiveSettings {
                require_remote_delete_approval: self.config.delete_approval_remote,
                require_local_delete_approval: self.config.delete_approval_local,
            },
        );
        let mut gate = DeleteGate::default();
        let now = current_epoch_secs();
        for action in plan {
            let Some(direction) = action.action.delete_direction() else {
                continue;
            };
            let is_directory = action.entity_kind == EntityKind::Directory;
            if !resolver
                .resolve(&action.path, is_directory)
                .requires_approval(direction)
            {
                continue; // guard off for this path/direction → execute normally
            }
            let fingerprint = delete_fingerprint(action, base_index);
            if matching_delete_approval(&self.connection, &action.path, direction, &fingerprint)? {
                gate.consumed_approvals
                    .push((action.path.clone(), direction));
            } else {
                gate.withheld_paths.insert(action.path.clone());
                gate.pending.push(PendingDeletion {
                    path: action.path.clone(),
                    direction,
                    entity_kind: action.entity_kind,
                    fingerprint,
                    detected_epoch_secs: now,
                });
            }
        }
        Ok(gate)
    }

    /// Applies an `approve`/`deny` control command against the currently-pending deletions. The
    /// `selector` must be an explicit relative path, or the literal `"all"` for every pending item.
    /// A missing selector is rejected as a no-op: acting on "all" must be a deliberate choice, so an
    /// accidentally-omitted argument from any IPC client can never approve every deletion at once.
    /// Approving records a standing approval keyed to the pending item's exact fingerprint (so it
    /// authorizes only that deletion); denying revokes any such approval. Only paths that are
    /// *currently pending* are acted on, so a bogus argument is a harmless no-op and no unvalidated
    /// path is ever stored.
    /// Test-only convenience wrapper over the free [`apply_approval_command`], which production
    /// code runs on the IPC task with its own connection and the published pending list.
    #[cfg(test)]
    fn apply_approval_command(&self, selector: Option<&str>, approve: bool) -> AppResult<String> {
        apply_approval_command(
            &self.connection,
            &self.pending_deletions,
            selector,
            false,
            approve,
        )
    }

    fn record_status_history(&mut self, message: &str) {
        let entry = StatusHistoryEntry {
            epoch_secs: current_epoch_secs(),
            message: message.to_owned(),
            last_error: self.last_error.clone(),
            plan_summary: self.last_plan_summary.clone(),
            successful_sync_summary: self.last_successful_sync_summary.clone(),
        };
        self.status_history.push(entry);
        if self.status_history.len() > STATUS_HISTORY_LIMIT {
            let remove_count = self.status_history.len() - STATUS_HISTORY_LIMIT;
            self.status_history.drain(0..remove_count);
        }
        if let Err(error) = write_status_history(&self.status_history_path, &self.status_history) {
            warn!(
                path = %self.status_history_path.display(),
                error = %error,
                "failed to persist daemon status history"
            );
        }
        if let Err(error) = self.write_metrics_snapshot() {
            warn!(
                path = %self.metrics_path.display(),
                error = %error,
                "failed to persist daemon metrics snapshot"
            );
        }
    }

    fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.publish_status();
        self.shared.metrics()
    }

    fn write_metrics_snapshot(&self) -> AppResult<()> {
        write_metrics_snapshot(&self.metrics_path, &self.metrics_snapshot())
    }

    fn last_sync_epoch_secs(&self) -> Option<u64> {
        self.last_sync
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
    }
}

/// Applies an `approve`/`deny` control command against the currently-pending deletions. The
/// `selector` must be an explicit relative path, or the literal `"all"` for every pending item.
/// A missing selector is rejected as a no-op: acting on "all" must be a deliberate choice, so an
/// accidentally-omitted argument from any IPC client can never approve every deletion at once.
/// When `literal_path` is set (see [`ControlRequest::literal_path`]), the selector is always a
/// path — a pending deletion literally named `all` can then be targeted without the reserved
/// word swallowing it into the every-item meaning.
/// Approving records a standing approval keyed to the pending item's exact fingerprint (so it
/// authorizes only that deletion); denying revokes any such approval. Only paths that are
/// *currently pending* are acted on, so a bogus argument is a harmless no-op and no unvalidated
/// path is ever stored.
fn apply_approval_command(
    connection: &Connection,
    pending_deletions: &[PendingDeletion],
    selector: Option<&str>,
    literal_path: bool,
    approve: bool,
) -> AppResult<String> {
    let Some(selector) = selector else {
        return Ok(
            "no target: pass a relative path, or \"all\" to act on every pending deletion"
                .to_owned(),
        );
    };
    // `None` here means the explicit "all" selector (every pending item); a plain path filters
    // to that one item. A literal-path request never gets the "all" interpretation.
    let target = if literal_path {
        Some(selector)
    } else {
        Some(selector).filter(|value| !value.eq_ignore_ascii_case("all"))
    };
    let matches: Vec<&PendingDeletion> = pending_deletions
        .iter()
        .filter(|pending| match target {
            // Compared in the wire form, not against the real `PathBuf`: a client can only ever
            // have seen the lossy rendering the daemon published (#61), so an exact comparison
            // would make every non-UTF-8 path permanently unapprovable — exactly the paths whose
            // withheld deletion motivated the lossy wire. The approval itself is still recorded
            // against the real path below. Still compared as *paths*, not strings, so the
            // component-wise leniency of the previous `PathBuf` comparison (a shell-completed
            // trailing slash on a directory, a `./` prefix) survives.
            Some(selector) => {
                Path::new(&*crate::ipc::wire_path(&pending.path)) == Path::new(selector)
            }
            None => true,
        })
        .collect();

    if matches.is_empty() {
        return Ok(match target {
            Some(path) => format!("no pending deletion matches '{path}'"),
            None => "no deletions are pending approval".to_owned(),
        });
    }
    // The planner emits at most one delete per path, so a targeted selector matches more than one
    // row only when two real paths differ solely in bytes the lossy wire replaced (#61). Authorising
    // both from one ambiguous selector would delete a file the user never picked out — and they
    // cannot pick it out, the rows render identically. Fail closed on the destructive side only:
    // a `deny` over-revoking is the safe direction, and `--all` remains the deliberate every-item
    // form.
    if approve && target.is_some() && matches.len() > 1 {
        return Ok(format!(
            "{} pending deletions render as '{}' and cannot be told apart on the wire (their \
             paths are not valid UTF-8); nothing was approved — use \"all\" to approve every \
             pending deletion",
            matches.len(),
            target.unwrap_or_default()
        ));
    }

    let now = current_epoch_secs() as i64;
    for pending in &matches {
        if approve {
            upsert_delete_approval(
                connection,
                &pending.path,
                pending.direction,
                &pending.fingerprint,
                now,
            )?;
        } else {
            delete_delete_approval(connection, &pending.path, pending.direction)?;
        }
    }

    let verb = if approve { "approved" } else { "denied" };
    Ok(format!(
        "{verb} {} pending deletion(s); run `proton-sync syncnow` to apply now",
        matches.len()
    ))
}

/// Accept loop for the control socket, run on its own task so control requests are served while
/// the daemon core is blocked in a reconcile. Each connection is handled on a further spawned
/// task, so one stalled client cannot delay the others. Aborted by `run()` on shutdown.
async fn serve_control_socket(
    listener: UnixListener,
    shared: Arc<ControlShared>,
    approvals_connection: Connection,
    loop_tx: mpsc::UnboundedSender<LoopCommand>,
    io_timeout: Duration,
    metrics_path: PathBuf,
    cancel_flag: Arc<AtomicBool>,
) {
    let approvals = Arc::new(tokio::sync::Mutex::new(approvals_connection));
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let shared = Arc::clone(&shared);
                let approvals = Arc::clone(&approvals);
                let loop_tx = loop_tx.clone();
                let metrics_path = metrics_path.clone();
                let cancel_flag = Arc::clone(&cancel_flag);
                tokio::spawn(async move {
                    let outcome = handle_control_connection(
                        stream,
                        &shared,
                        &approvals,
                        &loop_tx,
                        io_timeout,
                        &metrics_path,
                        &cancel_flag,
                    )
                    .await;
                    if let Err(error) = outcome {
                        warn!(%error, "failed to handle control connection");
                    }
                });
            }
            Err(error) => warn!(%error, "failed to accept control connection"),
        }
    }
}

/// Serves one control connection: read a request (time-bounded), answer it from [`ControlShared`]
/// — never by running work on this task — and write the reply (time-bounded). `syncnow` and
/// `shutdown` are acknowledged immediately and forwarded to the daemon core over `loop_tx`; a
/// client that wants to observe the requested sync finishing polls `status` until
/// `reconcile_seq` advances past the value in its ack and `syncing` is false again.
async fn handle_control_connection(
    stream: UnixStream,
    shared: &ControlShared,
    approvals: &tokio::sync::Mutex<Connection>,
    loop_tx: &mpsc::UnboundedSender<LoopCommand>,
    io_timeout: Duration,
    metrics_path: &Path,
    cancel_flag: &AtomicBool,
) -> AppResult<()> {
    // Time-bound the request read (it is also length-bounded in `read_request`) so silent
    // clients do not accumulate parked connection tasks.
    let (request, mut stream) = match tokio::time::timeout(io_timeout, read_request(stream)).await {
        Ok(result) => result?,
        Err(_elapsed) => {
            warn!(
                timeout_secs = io_timeout.as_secs(),
                "control connection did not send a request within the timeout; dropping it"
            );
            return Ok(());
        }
    };
    debug!(command = ?request.command, "handling control request");
    let response = match request.command {
        ControlCommand::Status => shared.response_with_sampled_activity("daemon status").await,
        ControlCommand::Pause => {
            shared.paused.store(true, Ordering::SeqCst);
            info!("sync paused");
            persist_metrics_best_effort(shared, metrics_path);
            shared.response("sync paused")
        }
        ControlCommand::Resume => {
            shared.paused.store(false, Ordering::SeqCst);
            info!("sync resumed");
            persist_metrics_best_effort(shared, metrics_path);
            shared.response("sync resumed")
        }
        ControlCommand::Syncnow => {
            if shared.is_paused() {
                shared.response("sync skipped because daemon is paused")
            } else {
                let message = if shared.is_syncing() {
                    "sync already in progress; another pass will follow"
                } else {
                    "sync scheduled"
                };
                if loop_tx.send(LoopCommand::SyncNow).is_ok() {
                    shared.response(message)
                } else {
                    shared.response("daemon is shutting down; sync not scheduled")
                }
            }
        }
        ControlCommand::Resync => {
            // Latch the full-walk request first so it survives even if the daemon is paused (it
            // will apply on the next pass after resume), then schedule a pass via the same path as
            // `syncnow`. The core consumes the latch with a `swap` at the top of that pass.
            shared.force_full_walk.store(true, Ordering::SeqCst);
            if shared.is_paused() {
                shared.response("full resync queued; it will run when syncing resumes")
            } else {
                let message = if shared.is_syncing() {
                    "full resync scheduled; it will run after the current pass"
                } else {
                    "full resync scheduled"
                };
                if loop_tx.send(LoopCommand::SyncNow).is_ok() {
                    shared.response(message)
                } else {
                    shared.response("daemon is shutting down; resync not scheduled")
                }
            }
        }
        ControlCommand::Approve | ControlCommand::Deny => {
            let approve = request.command == ControlCommand::Approve;
            let pending = shared
                .snapshot
                .lock()
                .expect("control snapshot lock")
                .pending_deletions
                .clone();
            let connection = approvals.lock().await;
            let message = apply_approval_command(
                &connection,
                &pending,
                request.argument.as_deref(),
                request.literal_path,
                approve,
            )?;
            drop(connection);
            if approve {
                info!(argument = ?request.argument, "delete approval recorded");
            } else {
                info!(argument = ?request.argument, "delete approval revoked");
            }
            shared.response(&message)
        }
        ControlCommand::Shutdown => {
            info!("shutdown requested over control socket");
            // Same teeth as a delivered signal: cancel any in-flight proton-drive command so
            // the daemon core returns from its reconcile promptly and observes the command
            // below, instead of finishing a potentially long transfer first.
            cancel_flag.store(true, Ordering::SeqCst);
            let _ = loop_tx.send(LoopCommand::Shutdown);
            shared.response("shutting down")
        }
    };
    // Time-bound the response write too, so a client that sends a valid request then never
    // reads cannot park this task on a full send buffer.
    match tokio::time::timeout(io_timeout, write_response(&mut stream, &response)).await {
        Ok(result) => result?,
        Err(_elapsed) => warn!(
            timeout_secs = io_timeout.as_secs(),
            "control client did not read the response within the timeout; dropping it"
        ),
    }
    Ok(())
}

/// Writes the metrics sidecar from the shared control state, logging (not failing) on error —
/// the IPC task must keep serving even if the sidecar path is briefly unwritable.
fn persist_metrics_best_effort(shared: &ControlShared, metrics_path: &Path) {
    if let Err(error) = write_metrics_snapshot(metrics_path, &shared.metrics()) {
        warn!(
            path = %metrics_path.display(),
            %error,
            "failed to persist daemon metrics snapshot"
        );
    }
}

/// The outcome of applying the delete-approval guard to a plan (see
/// [`Daemon::decide_delete_gate`]). `withheld_paths` are the destructive actions the execution loop
/// must skip; `pending` is the user-facing view of those; `consumed_approvals` are the approvals to
/// delete once the deletions they authorized have actually run.
#[derive(Default)]
struct DeleteGate {
    withheld_paths: HashSet<PathBuf>,
    pending: Vec<PendingDeletion>,
    consumed_approvals: Vec<(PathBuf, DeleteDirection)>,
}

/// The stable identity of the entity a deletion would remove, used to pin an approval to exactly
/// what the user saw: a file's last-synced SHA-1, else a directory's `proton_id`, else the action's
/// remote id. If the entity changes before the delete applies, the fingerprint no longer matches
/// and any earlier approval is inert.
fn delete_fingerprint(action: &PlannedAction, base_index: &HashMap<PathBuf, FileRecord>) -> String {
    base_index
        .get(&action.path)
        .and_then(|record| {
            record
                .sha1_hash
                .clone()
                .or_else(|| record.proton_id.clone())
        })
        .or_else(|| action.remote_id.clone())
        .unwrap_or_default()
}

enum IndexMutation {
    Upsert(FileRecord),
    Purge(PathBuf),
}

impl IndexMutation {
    fn apply(&self, connection: &Connection) -> AppResult<()> {
        match self {
            Self::Upsert(record) => upsert_record(connection, record),
            Self::Purge(path) => purge_record(connection, path),
        }
    }
}

/// One planned download prepared for batched execution: boundary-validated paths plus the
/// remote's claimed digest (for salvage verification when a chunk fails midway).
struct PreparedDownload<'plan> {
    action: &'plan PlannedAction,
    /// Position in the overall plan, for `action_index` progress reporting.
    action_number: usize,
    remote_id: String,
    request: DownloadRequest,
}

/// Whether executing this action performs a side effect (a CLI call or a local filesystem
/// change). These checkpoint-commit immediately after completing; index-only actions
/// (`AutoLink`/`Purge`) and no-ops accumulate until the next checkpoint or the final commit,
/// so an adoption-heavy pass (thousands of AutoLinks) stays a few large transactions instead
/// of thousands of per-row fsyncs.
fn action_performs_side_effects(action: &SyncAction) -> bool {
    !matches!(
        action,
        SyncAction::AutoLink | SyncAction::Purge | SyncAction::SkipUnsupported
    )
}

/// Durably commits everything accumulated since the previous checkpoint — the incremental half
/// of the commit-after-side-effects scheme (see `execute_plan_and_commit`). An index write
/// still never precedes its side effect; a checkpoint only makes *completed* work survive a
/// later failure or shutdown. The event cursor is deliberately not part of any checkpoint (it
/// advances only in the final commit of a fully-successful pass). Approval consumptions ride
/// in the same transaction as the deletion's own index purge.
fn commit_checkpoint(
    connection: &mut Connection,
    index_mutations: &mut Vec<IndexMutation>,
    pending_approval_consumptions: &mut Vec<(PathBuf, DeleteDirection)>,
) -> AppResult<()> {
    if index_mutations.is_empty() && pending_approval_consumptions.is_empty() {
        return Ok(());
    }
    let transaction = connection.transaction()?;
    for mutation in index_mutations.iter() {
        mutation.apply(&transaction)?;
    }
    for (path, direction) in pending_approval_consumptions.iter() {
        delete_delete_approval(&transaction, path, *direction)?;
    }
    transaction.commit()?;
    index_mutations.clear();
    pending_approval_consumptions.clear();
    Ok(())
}

/// Queues the consumption of the standing delete approval matching a deletion that just
/// executed, so the next checkpoint removes the approval in the same transaction as the index
/// purge it authorized. No-op when the delete ran with the guard off (nothing was approved).
fn record_approval_consumption(
    approved: &[(PathBuf, DeleteDirection)],
    path: &Path,
    direction: DeleteDirection,
    pending: &mut Vec<(PathBuf, DeleteDirection)>,
) {
    if approved.iter().any(|(approved_path, approved_direction)| {
        approved_path == path && *approved_direction == direction
    }) {
        pending.push((path.to_path_buf(), direction));
    }
}

fn scan_options_from_config(config: &DaemonConfig) -> AppResult<ScanOptions> {
    // The default state paths all live under `<local_root>/.sync`, which `ScanOptions` ignores as a
    // subtree. These explicit entries additionally cover a state path relocated *out* of `.sync`
    // via `--db-path`/`--lockfile-path` but still inside the sync root, so it is never planned for
    // upload. (A path outside the root normalizes to `None` and is simply dropped.)
    let ignored_paths = vec![
        config.db_path.clone(),
        status_history_path(&config.db_path),
        metrics_path(&config.db_path),
        config.lockfile_path.clone(),
    ];
    ScanOptions::new(
        &config.local_root,
        &ignored_paths,
        &config.include_patterns,
        &config.exclude_patterns,
    )
}

fn status_history_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("status.json")
}

fn metrics_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("metrics.json")
}

fn load_status_history(path: &Path) -> AppResult<Vec<StatusHistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let history = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&history)?)
}

fn write_status_history(path: &Path, history: &[StatusHistoryEntry]) -> AppResult<()> {
    write_atomically(path, &serde_json::to_vec_pretty(history)?)
}

fn write_metrics_snapshot(path: &Path, metrics: &MetricsSnapshot) -> AppResult<()> {
    write_atomically(path, &serde_json::to_vec_pretty(metrics)?)
}

/// Write `bytes` to `path` atomically: write a sibling temp file, fsync it, then rename over the
/// destination. Rename is atomic within a filesystem, so a concurrent reader — e.g. a GUI polling
/// `<db>.metrics.json` / `<db>.status.json`, or the CLI — never observes a partially written file.
fn write_atomically(path: &Path, bytes: &[u8]) -> AppResult<()> {
    use std::io::{ErrorKind, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_NONCE: AtomicU64 = AtomicU64::new(0);
    ensure_parent_directory(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("sidecar");
    // Open a fresh sibling temp with `create_new` (O_EXCL): it fails rather than following or
    // truncating a pre-existing path — e.g. an attacker-planted symlink in a writable sidecar
    // directory — and the unpredictable, per-attempt suffix avoids colliding with a concurrent
    // writer. Then fsync and atomically rename over the destination.
    let (mut file, tmp) = loop {
        let nonce = TMP_NONCE.fetch_add(1, Ordering::Relaxed)
            ^ SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or_default();
        let candidate =
            path.with_file_name(format!(".{file_name}.{}.{nonce:x}.tmp", std::process::id()));
        let mut open_opts = fs::OpenOptions::new();
        open_opts.write(true).create_new(true);
        // The sidecars carry local paths, errors, and pending deletions; write them owner-only
        // (0600), like the control socket, so they don't leak if the sidecar dir is readable by
        // other users. The final file inherits the temp's mode through the rename.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_opts.mode(0o600);
        }
        match open_opts.open(&candidate) {
            Ok(file) => break (file, candidate),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error.into());
    }
    Ok(())
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn filter_remote_entities(
    remote_entities: HashMap<PathBuf, RemoteEntity>,
    scan_options: &ScanOptions,
) -> HashMap<PathBuf, RemoteEntity> {
    remote_entities
        .into_iter()
        .filter(|(path, entity)| match entity {
            RemoteEntity::File(_) => scan_options.allows_relative_file(path),
            RemoteEntity::Directory(_) => scan_options.allows_relative_directory(path),
        })
        .collect()
}

fn load_remote_entities(
    proton: &impl ProtonClient,
    remote_root: &Path,
    scan_options: &ScanOptions,
) -> AppResult<(HashMap<PathBuf, RemoteEntity>, bool)> {
    match proton.list_entities_or_missing_root(remote_root)? {
        RemoteListingStatus::Found(remote_entities) => {
            Ok((filter_remote_entities(remote_entities, scan_options), false))
        }
        RemoteListingStatus::RootMissing => Ok((HashMap::new(), true)),
    }
}

fn prepend_remote_root_creation_if_missing(plan: &mut Vec<PlannedAction>, root_missing: bool) {
    if root_missing {
        plan.insert(
            0,
            PlannedAction::new(
                Path::new(""),
                SyncAction::CreateRemoteDirectory,
                EntityKind::Directory,
                None,
            ),
        );
    }
}

fn local_files_from_entities(
    local_entities: &HashMap<PathBuf, LocalEntityState>,
) -> HashMap<PathBuf, crate::index::LocalFileState> {
    local_entities
        .iter()
        .filter_map(|(path, entity)| match entity {
            LocalEntityState::File(file) => Some((path.clone(), file.clone())),
            LocalEntityState::Directory(_) => None,
        })
        .collect()
}

fn filter_base_index(
    base_index: HashMap<PathBuf, FileRecord>,
    scan_options: &ScanOptions,
) -> HashMap<PathBuf, FileRecord> {
    base_index
        .into_iter()
        .filter(|(path, record)| match record.entity_kind {
            EntityKind::File => scan_options.allows_relative_file(path),
            EntityKind::Directory => scan_options.allows_relative_directory(path),
        })
        .collect()
}

fn command_policy_from_config(config: &DaemonConfig) -> CommandPolicy {
    CommandPolicy::new(config.proton_timeout, config.proton_list_attempts)
}

/// `x-pm-appversion` sent on events requests. Matches the value the live detection harness uses
/// (`tests/events_live.rs`); Proton validates it.
const EVENTS_APP_VERSION: &str = "cli-drive@0.5.0";

/// Builds the real [`EventSource`] (an [`EventsClient`] over `curl` + the reused CLI keyring
/// session) when `events_driven` is on. Returns `None` — falling back to full-tree snapshots —
/// when the feature is off or the CLI session cannot be read, so a missing/locked keyring
/// degrades gracefully instead of failing daemon startup.
fn build_event_source(config: &DaemonConfig) -> Option<Box<dyn EventSource>> {
    if !config.events_driven {
        return None;
    }
    match CliKeyringSession::from_cli_keyring() {
        Ok(session) => Some(Box::new(EventsClient::new(
            CurlHttpTransport::new(),
            session,
            EVENTS_APP_VERSION,
        ))),
        Err(error) => {
            warn!(
                %error,
                "events_driven is enabled but the reused CLI session could not be read; \
                 falling back to full-tree remote scans"
            );
            None
        }
    }
}

/// Result of an attempted incremental (event-driven) reconcile pass.
enum IncrementalOutcome {
    /// Changes were planned, executed, and committed with the cursor advanced.
    Committed,
    /// Nothing to do (no remote or local changes); the cursor was advanced without side effects.
    Idle,
    /// The delta could not be turned into a complete map; the caller must full-tree snapshot. The
    /// string is a human-readable reason for logging.
    Fallback(String),
}

/// A concatenated, paginated volume event delta plus the cursor to persist afterwards.
struct EventDelta {
    changes: Vec<RemoteChange>,
    latest_event_id: String,
    /// The server asked the client to discard its cursor and reconverge with a full scan.
    refresh: bool,
}

/// The event cursor to persist alongside a reconcile's index mutations, in the same transaction.
struct CursorUpdate {
    scope_id: String,
    last_event_id: String,
}

/// Resolves a created/updated node to its current `(relative path, entity)` by listing just the
/// node's parent directory (an O(1) call), the targeted alternative to a full-tree walk.
struct TargetedResolver<'a, C: ProtonClient> {
    proton: &'a C,
    connection: &'a Connection,
    remote_root: &'a Path,
    volume_id: &'a str,
    /// Parent listings already fetched **during this pass**, keyed by composed parent uid
    /// ([`ROOT_LISTING_KEY`] for the remote root). N events in one folder — a bulk copy, or N
    /// revisions of one file — then cost one `proton-drive` subprocess instead of N (#70).
    ///
    /// Deliberately pass-scoped: the resolver is built inside `try_incremental_reconcile` and
    /// dropped with it, so a listing can never outlive the pass that read it. Resolution reads
    /// *current* remote state, so a listing carried into a later pass would plan against a folder
    /// that has since changed.
    listings: RefCell<HashMap<String, Rc<HashMap<PathBuf, RemoteEntity>>>>,
}

/// Memo key for the remote-root listing. Never collides with a composed uid (`volumeId~nodeId`).
const ROOT_LISTING_KEY: &str = "";

impl<C: ProtonClient> TargetedResolver<'_, C> {
    /// This pass's listing of `relative_directory`, fetched once and memoized under `key`.
    fn listing(
        &self,
        key: &str,
        relative_directory: &Path,
    ) -> AppResult<Rc<HashMap<PathBuf, RemoteEntity>>> {
        // Read the memo through a borrow that ends before the (possibly long) CLI call below.
        let cached = self.listings.borrow().get(key).cloned();
        if let Some(listing) = cached {
            return Ok(listing);
        }
        let listing = Rc::new(
            self.proton
                .list_directory(self.remote_root, relative_directory)?,
        );
        self.listings
            .borrow_mut()
            .insert(key.to_owned(), Rc::clone(&listing));
        Ok(listing)
    }
}

impl<C: ProtonClient> RemoteChangeResolver for TargetedResolver<'_, C> {
    fn resolve(&self, change: &RemoteChange) -> AppResult<Option<(PathBuf, RemoteEntity)>> {
        let target_uid = node_uid(self.volume_id, &change.node_id);

        // Prefer listing the event's parent directory when it is indexed (the common nested case).
        if let Some(parent_id) = change.parent_id.as_deref() {
            let parent_uid = node_uid(self.volume_id, parent_id);
            if let Some(parent_path) = path_for_proton_id(self.connection, &parent_uid)? {
                let listing = self.listing(&parent_uid, &parent_path)?;
                // Absent from its stated parent → the reconstruction drops any stale location
                // (an update) or re-anchors with a full walk (a create whose listing lags).
                return Ok(find_entity_by_uid(&listing, &target_uid));
            }
        }

        // The parent is not indexed (e.g. a top-level node whose parent is the remote root, which
        // has no index record). Fall back to listing the root; if the node is not there either we
        // cannot place it without a full walk, so signal a snapshot.
        let root_listing = self.listing(ROOT_LISTING_KEY, Path::new(""))?;
        match find_entity_by_uid(&root_listing, &target_uid) {
            Some(resolved) => Ok(Some(resolved)),
            None => Err(boxed_error(format!(
                "changed node {} is not under any indexed parent or the remote root",
                change.node_id
            ))),
        }
    }
}

fn find_entity_by_uid(
    listing: &HashMap<PathBuf, RemoteEntity>,
    target_uid: &str,
) -> Option<(PathBuf, RemoteEntity)> {
    listing
        .iter()
        .find(|(_, entity)| entity.remote_id().as_deref() == Some(target_uid))
        .map(|(path, entity)| (path.clone(), entity.clone()))
}

/// The periodic full-scan cadence to compare the pass counter against, translating the
/// user-facing "disabled" sentinel into a threshold no realistic runtime can reach. `0` (the
/// default) disables the periodic safety resync — after the startup floor the daemon stays purely
/// event-driven; any positive `N` reinstates a full walk every `N` incremental passes. Because the
/// counter only ever increments while it is *below* this value (i.e. while `should_try_incremental`
/// was true), it never overflows even when seeded at `u64::MAX`.
fn effective_full_scan_every(configured: u64) -> u64 {
    if configured == 0 {
        u64::MAX
    } else {
        configured
    }
}

/// Derives the volume id from any baseline record carrying a composed `proton_id`
/// (`volumeId~nodeId`). `None` when nothing has been synced with a composed id yet.
fn derive_volume_id(base_records: &HashMap<PathBuf, FileRecord>) -> Option<&str> {
    base_records
        .values()
        .filter_map(|record| record.proton_id.as_deref())
        .find_map(volume_id_from_proton_id)
}

/// Derives the volume id from a fresh snapshot's remote entities (used on the first-ever bootstrap
/// when no baseline composed id exists yet).
fn derive_volume_id_from_entities(
    remote_entities: &HashMap<PathBuf, RemoteEntity>,
) -> Option<String> {
    remote_entities.values().find_map(|entity| {
        let id = match entity {
            RemoteEntity::File(file) => Some(file.id.as_str()),
            RemoteEntity::Directory(directory) => directory.id.as_deref(),
        }?;
        volume_id_from_proton_id(id).map(ToOwned::to_owned)
    })
}

fn build_watcher(
    watch_tx: mpsc::UnboundedSender<notify::Result<Event>>,
) -> AppResult<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |event| {
        let _ = watch_tx.send(event);
    })?;
    Ok(watcher)
}

fn ensure_parent_directory(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Copies `source` to `destination`, which must not exist. The destination is opened with
/// `create_new` (`O_EXCL`), which never follows a symlink and fails on any existing object, so a
/// planted or stale link at a predictable path cannot redirect the write — and, being one atomic
/// syscall, it also closes the window between a prior check and this write. An existing object is
/// therefore not an error here: the caller has already decided that an occupied path is left
/// alone, and losing the race means the path is occupied. A partially written file is removed
/// rather than left behind as the artefact of an operation that then failed.
fn copy_into_new_file(source: &Path, destination: &Path) -> AppResult<()> {
    let mut reader = File::open(source)?;
    let mut writer = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            warn!(
                path = %destination.display(),
                "not writing file: something appeared at the path first, and it is never replaced"
            );
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = std::io::copy(&mut reader, &mut writer) {
        drop(writer);
        let _ = fs::remove_file(destination);
        return Err(error.into());
    }
    Ok(())
}

/// Starts a foreground progress spinner for an in-flight file transfer, `[index/total] verb path
/// (size)` with elapsed time, but only when `interactive` (stderr is a terminal). Under
/// systemd/journald it returns `None` and the caller falls back to a structured `info!` line, so
/// the journal never sees progress-bar escape codes. The `proton-drive` CLI exposes no byte
/// counter while a transfer runs, so this is an indeterminate spinner (elapsed), not a percentage.
fn begin_transfer_spinner(
    interactive: bool,
    index: usize,
    total: usize,
    verb: &str,
    path: &Path,
    size_bytes: Option<u64>,
) -> Option<ProgressBar> {
    if !interactive {
        return None;
    }
    let size = size_bytes
        .map(|bytes| format!(" ({})", indicatif::HumanBytes(bytes)))
        .unwrap_or_default();
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg} [{elapsed}]")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    spinner.set_message(format!("[{index}/{total}] {verb} {}{size}", path.display()));
    spinner.enable_steady_tick(Duration::from_millis(120));
    Some(spinner)
}

/// Clears a transfer spinner once its transfer finishes (or fails). A no-op when there was no
/// spinner (headless runs). Called before propagating any transfer error so the spinner never
/// lingers on screen after a failed upload/download.
fn finish_transfer_spinner(spinner: Option<ProgressBar>) {
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
}

/// Removes the control socket on shutdown, but only when the path is still an actual Unix
/// socket. Uses `symlink_metadata` (not `exists()`) so a regular file or symlink swapped in
/// at runtime — by misconfiguration or a malicious replacement — is left in place with a
/// warning rather than deleted, mirroring the safety check in `ipc::bind_listener`.
/// Best-effort: a cleanup failure never fails the daemon's exit.
fn remove_control_socket(socket_path: &Path) {
    use std::os::unix::fs::FileTypeExt;

    match fs::symlink_metadata(socket_path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            if let Err(error) = fs::remove_file(socket_path) {
                warn!(
                    path = %socket_path.display(),
                    %error,
                    "failed to remove control socket on shutdown"
                );
            }
        }
        Ok(_) => warn!(
            path = %socket_path.display(),
            "control socket path is not a socket at shutdown; leaving it in place"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(
            path = %socket_path.display(),
            %error,
            "failed to inspect control socket on shutdown"
        ),
    }
}

/// Join `relative` onto `local_root` only when it is safe to do so.
///
/// Returns `None` (and the caller should skip the action) when `relative`
/// contains components that could escape `local_root`, or is the root itself.
/// Delegates to [`crate::validate_relative_path_non_empty`] for consistent security semantics
/// with the remote-path normalization in `proton.rs`: an empty relative path resolves to
/// `local_root`, turning a per-entry download or delete into a whole-root one (#72).
fn safe_local_path(local_root: &Path, relative: &Path) -> Option<PathBuf> {
    let destination = local_root.join(crate::validate_relative_path_non_empty(relative)?);
    if local_write_escapes_root(local_root, &destination) {
        return None;
    }
    Some(destination)
}

/// Returns true when writing to `destination` (already lexically validated and joined onto
/// `local_root`) would actually land outside `local_root` because a pre-existing intermediate
/// component is a symlink pointing elsewhere.
///
/// `validate_relative_path` is purely lexical and cannot see this: a directory symlink such
/// as `sub -> /outside` inside the sync root would otherwise let a remote entry `sub/foo` be
/// created or downloaded straight through the symlink to `/outside/foo` (the scanner already
/// skips symlinks, so the local side never balances it). This resolves the deepest existing
/// ancestor of `destination` — which follows every symlink along the way — and requires it to
/// stay within the canonicalized `local_root`. Fails closed: if `local_root` or the ancestor
/// cannot be canonicalized, the write is treated as escaping.
fn local_write_escapes_root(local_root: &Path, destination: &Path) -> bool {
    let Ok(canonical_root) = fs::canonicalize(local_root) else {
        return true;
    };
    let mut ancestor = destination;
    loop {
        match fs::canonicalize(ancestor) {
            Ok(canonical) => return !canonical.starts_with(&canonical_root),
            Err(_) => match ancestor.parent() {
                Some(parent) => ancestor = parent,
                None => return true,
            },
        }
    }
}

/// The remote-side counterpart of [`safe_local_path`]: rejects the empty path too, so a planned
/// action can never address the remote root itself (#72). `CreateRemoteDirectory` handles the
/// root through `ensure_root_directory` before reaching here.
fn safe_remote_path(remote_root: &Path, relative: &Path) -> Option<PathBuf> {
    crate::validate_relative_path_non_empty(relative).map(|safe| remote_root.join(safe))
}

/// Returns every base-index path strictly nested under `directory_path`, used to purge
/// the whole subtree from the index in one commit when a directory is recursively deleted.
fn descendant_index_paths(
    directory_path: &Path,
    base_index: &HashMap<PathBuf, FileRecord>,
) -> Vec<PathBuf> {
    base_index
        .keys()
        .filter(|path| is_strict_descendant(directory_path, path))
        .cloned()
        .collect()
}

/// Advisory single-instance lock. The lockfile is **never unlinked** (#13): `flock` binds to the
/// inode, not the path, so removing the file on drop lets a daemon that adopted the same inode in
/// the drop window keep running while a later start creates a *fresh* inode and locks it
/// independently — two live daemons on one root, or (via the user-global lock) one per root, both
/// racing the shared `proton-drive` SQLite cache (#23). Dropping the guard releases the lock and
/// leaves an empty file behind; `acquire` reuses it, so a leftover file never blocks a restart.
struct LockGuard {
    file: File,
}

impl LockGuard {
    fn acquire(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| {
                boxed_error(format!(
                    "failed to open lockfile {}: {error}",
                    path.display()
                ))
            })?;
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                boxed_error(format!(
                    "daemon already running; lockfile is locked at {}",
                    path.display()
                ))
            } else {
                boxed_error(format!(
                    "failed to lock lockfile {}: {error}",
                    path.display()
                ))
            }
        })?;
        Ok(Self { file })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Release the flock only — never unlink; see the type comment (#13).
        let _ = self.file.unlock();
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{RemoteChangeKind, VolumeEventPage};
    use crate::index::EntityKind;
    use crate::index::get_record;
    use crate::proton::{RemoteDirectory, RemoteFile};
    use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};
    use sha1::{Digest, Sha1};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn sidecar_write_is_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("sync_index.status.json");
        write_atomically(&path, b"{\"ok\":true}").expect("write sidecar");
        let mode = fs::metadata(&path)
            .expect("sidecar metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "status/metrics sidecars must be owner-only (they carry local paths, errors, and \
             pending deletions)"
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedOperation {
        EnsureRootDirectory {
            remote_root: PathBuf,
        },
        EnsureDirectory {
            remote_root: PathBuf,
            relative_path: PathBuf,
        },
        Upload {
            local_path: PathBuf,
            remote_root: PathBuf,
            relative_path: PathBuf,
        },
        Download {
            remote_path: PathBuf,
            destination: PathBuf,
        },
        DownloadBatch {
            remote_paths: Vec<PathBuf>,
        },
        Delete {
            remote_path: PathBuf,
        },
        RenameOrMove {
            remote_root: PathBuf,
            old_relative_path: PathBuf,
            new_relative_path: PathBuf,
        },
    }

    #[derive(Debug, Clone)]
    struct RecordingProtonClient {
        remote_files: HashMap<PathBuf, RemoteFile>,
        remote_entities: HashMap<PathBuf, RemoteEntity>,
        remote_contents: HashMap<PathBuf, Vec<u8>>,
        operations: Arc<Mutex<Vec<RecordedOperation>>>,
        failed_uploads: BTreeSet<PathBuf>,
        /// Absolute remote paths whose per-item result in `download_many` fails (the batch
        /// itself still runs, mirroring the real client's partial-failure contract).
        failed_batch_downloads: BTreeSet<PathBuf>,
        /// Absolute remote paths for which `download_many` reports `Ok` WITHOUT writing the
        /// destination file — models a landed file vanishing before the daemon can stat it.
        unstaged_batch_downloads: BTreeSet<PathBuf>,
    }

    impl RecordingProtonClient {
        fn new(
            remote_files: HashMap<PathBuf, RemoteFile>,
        ) -> (Self, Arc<Mutex<Vec<RecordedOperation>>>) {
            let operations = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    remote_entities: remote_entities_from_files(&remote_files),
                    remote_files,
                    remote_contents: HashMap::new(),
                    operations: Arc::clone(&operations),
                    failed_uploads: BTreeSet::new(),
                    failed_batch_downloads: BTreeSet::new(),
                    unstaged_batch_downloads: BTreeSet::new(),
                },
                operations,
            )
        }

        fn with_remote_contents(
            remote_files: HashMap<PathBuf, RemoteFile>,
            remote_contents: HashMap<PathBuf, Vec<u8>>,
        ) -> (Self, Arc<Mutex<Vec<RecordedOperation>>>) {
            let operations = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    remote_entities: remote_entities_from_files(&remote_files),
                    remote_files,
                    remote_contents,
                    operations: Arc::clone(&operations),
                    failed_uploads: BTreeSet::new(),
                    failed_batch_downloads: BTreeSet::new(),
                    unstaged_batch_downloads: BTreeSet::new(),
                },
                operations,
            )
        }

        fn with_failed_uploads(
            remote_files: HashMap<PathBuf, RemoteFile>,
            failed_uploads: BTreeSet<PathBuf>,
        ) -> (Self, Arc<Mutex<Vec<RecordedOperation>>>) {
            let operations = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    remote_entities: remote_entities_from_files(&remote_files),
                    remote_files,
                    remote_contents: HashMap::new(),
                    operations: Arc::clone(&operations),
                    failed_uploads,
                    failed_batch_downloads: BTreeSet::new(),
                    unstaged_batch_downloads: BTreeSet::new(),
                },
                operations,
            )
        }

        fn with_remote_entities(
            remote_entities: HashMap<PathBuf, RemoteEntity>,
        ) -> (Self, Arc<Mutex<Vec<RecordedOperation>>>) {
            let remote_files = remote_entities
                .iter()
                .filter_map(|(path, entity)| match entity {
                    RemoteEntity::File(file) => Some((path.clone(), file.clone())),
                    RemoteEntity::Directory(_) => None,
                })
                .collect();
            let operations = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    remote_files,
                    remote_entities,
                    remote_contents: HashMap::new(),
                    operations: Arc::clone(&operations),
                    failed_uploads: BTreeSet::new(),
                    failed_batch_downloads: BTreeSet::new(),
                    unstaged_batch_downloads: BTreeSet::new(),
                },
                operations,
            )
        }
    }

    impl ProtonClient for RecordingProtonClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(self.remote_files.clone())
        }

        fn list_entities(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            Ok(self.remote_entities.clone())
        }

        fn ensure_directory(&self, remote_root: &Path, relative_path: &Path) -> AppResult<()> {
            self.operations.lock().expect("operations lock").push(
                RecordedOperation::EnsureDirectory {
                    remote_root: remote_root.to_path_buf(),
                    relative_path: relative_path.to_path_buf(),
                },
            );
            Ok(())
        }

        fn upload(
            &self,
            local_path: &Path,
            remote_root: &Path,
            relative_path: &Path,
        ) -> AppResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(RecordedOperation::Upload {
                    local_path: local_path.to_path_buf(),
                    remote_root: remote_root.to_path_buf(),
                    relative_path: relative_path.to_path_buf(),
                });
            if self.failed_uploads.contains(relative_path) {
                return Err(boxed_error(format!(
                    "upload failed for {}",
                    relative_path.display()
                )));
            }
            Ok(())
        }

        fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()> {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = self
                .remote_contents
                .get(remote_path)
                .cloned()
                .unwrap_or_else(|| format!("downloaded:{}", remote_path.display()).into_bytes());
            fs::write(destination, content)?;
            self.operations
                .lock()
                .expect("operations lock")
                .push(RecordedOperation::Download {
                    remote_path: remote_path.to_path_buf(),
                    destination: destination.to_path_buf(),
                });
            Ok(())
        }

        fn download_many(&self, requests: &[DownloadRequest]) -> Vec<AppResult<()>> {
            self.operations.lock().expect("operations lock").push(
                RecordedOperation::DownloadBatch {
                    remote_paths: requests
                        .iter()
                        .map(|request| request.remote_path.clone())
                        .collect(),
                },
            );
            requests
                .iter()
                .map(|request| {
                    if self.failed_batch_downloads.contains(&request.remote_path) {
                        return Err(boxed_error(format!(
                            "download failed for {}",
                            request.remote_path.display()
                        )));
                    }
                    if self.unstaged_batch_downloads.contains(&request.remote_path) {
                        return Ok(());
                    }
                    if let Some(parent) = request.destination.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let content = self
                        .remote_contents
                        .get(&request.remote_path)
                        .cloned()
                        .unwrap_or_else(|| {
                            format!("downloaded:{}", request.remote_path.display()).into_bytes()
                        });
                    fs::write(&request.destination, content)?;
                    Ok(())
                })
                .collect()
        }

        fn delete(&self, remote_path: &Path) -> AppResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(RecordedOperation::Delete {
                    remote_path: remote_path.to_path_buf(),
                });
            Ok(())
        }

        fn rename_or_move(
            &self,
            remote_root: &Path,
            old_relative_path: &Path,
            new_relative_path: &Path,
        ) -> AppResult<()> {
            self.operations.lock().expect("operations lock").push(
                RecordedOperation::RenameOrMove {
                    remote_root: remote_root.to_path_buf(),
                    old_relative_path: old_relative_path.to_path_buf(),
                    new_relative_path: new_relative_path.to_path_buf(),
                },
            );
            Ok(())
        }
    }

    fn remote_entities_from_files(
        remote_files: &HashMap<PathBuf, RemoteFile>,
    ) -> HashMap<PathBuf, RemoteEntity> {
        remote_files
            .iter()
            .map(|(path, file)| (path.clone(), RemoteEntity::File(file.clone())))
            .collect()
    }

    #[derive(Debug)]
    struct FakeProtonClient {
        remote_files: HashMap<PathBuf, RemoteFile>,
    }

    #[derive(Debug)]
    struct ParentCheckingDownloadClient {
        remote_files: HashMap<PathBuf, RemoteFile>,
    }

    #[derive(Debug)]
    struct FailingListProtonClient;

    #[derive(Debug, Clone)]
    struct MissingRootProtonClient {
        operations: Arc<Mutex<Vec<RecordedOperation>>>,
    }

    impl ProtonClient for FakeProtonClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(self.remote_files.clone())
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected ensure directory in fake client"))
        }

        fn upload(
            &self,
            _local_path: &Path,
            _remote_root: &Path,
            _relative_path: &Path,
        ) -> AppResult<()> {
            Err(boxed_error("unexpected upload in fake client"))
        }

        fn download(&self, _remote_path: &Path, _destination: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected download in fake client"))
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected delete in fake client"))
        }
    }

    impl ProtonClient for ParentCheckingDownloadClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(self.remote_files.clone())
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Err(boxed_error(
                "unexpected ensure directory in parent-checking client",
            ))
        }

        fn upload(
            &self,
            _local_path: &Path,
            _remote_root: &Path,
            _relative_path: &Path,
        ) -> AppResult<()> {
            Err(boxed_error("unexpected upload in parent-checking client"))
        }

        fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()> {
            let parent = destination
                .parent()
                .ok_or_else(|| boxed_error("download destination should have a parent"))?;
            if !parent.is_dir() {
                return Err(boxed_error(format!(
                    "download parent was not created: {}",
                    parent.display()
                )));
            }
            fs::write(destination, format!("downloaded:{}", remote_path.display()))?;
            Ok(())
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected delete in parent-checking client"))
        }
    }

    impl ProtonClient for FailingListProtonClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Err(boxed_error("list failed"))
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Err(boxed_error(
                "unexpected ensure directory in failing list client",
            ))
        }

        fn upload(
            &self,
            _local_path: &Path,
            _remote_root: &Path,
            _relative_path: &Path,
        ) -> AppResult<()> {
            Err(boxed_error("unexpected upload in failing list client"))
        }

        fn download(&self, _remote_path: &Path, _destination: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected download in failing list client"))
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected delete in failing list client"))
        }
    }

    impl ProtonClient for MissingRootProtonClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Err(boxed_error("list should use missing-root status"))
        }

        fn list_entities_or_missing_root(
            &self,
            _remote_root: &Path,
        ) -> AppResult<RemoteListingStatus> {
            Ok(RemoteListingStatus::RootMissing)
        }

        fn ensure_root_directory(&self, remote_root: &Path) -> AppResult<()> {
            self.operations.lock().expect("operations lock").push(
                RecordedOperation::EnsureRootDirectory {
                    remote_root: remote_root.to_path_buf(),
                },
            );
            Ok(())
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Err(boxed_error(
                "unexpected ensure directory in missing-root client",
            ))
        }

        fn upload(
            &self,
            local_path: &Path,
            remote_root: &Path,
            relative_path: &Path,
        ) -> AppResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(RecordedOperation::Upload {
                    local_path: local_path.to_path_buf(),
                    remote_root: remote_root.to_path_buf(),
                    relative_path: relative_path.to_path_buf(),
                });
            Ok(())
        }

        fn download(&self, _remote_path: &Path, _destination: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected download in missing-root client"))
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected delete in missing-root client"))
        }
    }

    #[test]
    fn preview_plan_uses_injected_proton_client() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("local.txt"), b"local").expect("local file");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("remote.txt"),
            RemoteFile {
                path: PathBuf::from("remote.txt"),
                id: "remote-id".to_owned(),
                name: "remote.txt".to_owned(),
                sha1_hash: Some("remote-hash".to_owned()),
                downloadable: true,
            },
        );
        let config = DaemonConfig {
            local_root,
            remote_root: PathBuf::from("/Drive/RemoteFolder"),
            db_path: directory.path().join("missing.db"),
            socket_path: directory.path().join("daemon.sock"),
            lockfile_path: directory.path().join("daemon.lock"),
            global_lock_path: directory.path().join("single-instance.lock"),
            scan_interval: Duration::from_secs(300),
            proton_cli: PathBuf::from("proton-drive"),
            proton_timeout: Duration::from_secs(60),
            proton_list_attempts: 2,
            download_batch_size: 1,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            events_driven: false,
            events_full_scan_every: 20,
            delete_approval_remote: false,
            delete_approval_local: false,
            warm_start: WarmStartConfig::default(),
        };

        let plan = preview_plan_with_client(&config, &FakeProtonClient { remote_files })
            .expect("preview plan");

        assert!(
            plan.iter()
                .any(|action| action.path == Path::new("local.txt")
                    && action.action == SyncAction::Upload)
        );
        assert!(
            plan.iter()
                .any(|action| action.path == Path::new("remote.txt")
                    && action.action == SyncAction::Download
                    && action.remote_id.as_deref() == Some("remote-id"))
        );
    }

    #[test]
    fn reconcile_creates_missing_remote_root_before_uploading() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("local-only.txt");
        fs::write(&local_path, b"local").expect("local file");
        let operations = Arc::new(Mutex::new(Vec::new()));
        let client = MissingRootProtonClient {
            operations: Arc::clone(&operations),
        };
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        let operations = operations.lock().expect("operations lock");
        assert_eq!(
            operations.first(),
            Some(&RecordedOperation::EnsureRootDirectory {
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
            }),
            "remote root must be created before any upload runs"
        );
        assert!(
            operations.contains(&RecordedOperation::Upload {
                local_path,
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                relative_path: PathBuf::from("local-only.txt"),
            }),
            "local files should still upload after creating the missing remote root"
        );
        assert_eq!(daemon.last_error, None);
        assert_eq!(
            daemon
                .last_successful_sync_summary
                .as_ref()
                .map(|summary| summary.remote_directories_created),
            Some(1)
        );
    }

    #[test]
    fn missing_remote_root_bootstraps_from_local_even_with_existing_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("stable.txt");
        fs::write(&local_path, b"stable").expect("local file");
        let operations = Arc::new(Mutex::new(Vec::new()));
        let client = MissingRootProtonClient {
            operations: Arc::clone(&operations),
        };
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let hash = sha1_bytes(b"stable");
        upsert_record(
            &daemon.connection,
            &base_record("stable.txt", Some("stale-remote-id"), hash.as_str()),
        )
        .expect("base index record");

        daemon.reconcile_blocking().expect("reconcile");

        let operations = operations.lock().expect("operations lock");
        assert!(
            operations.contains(&RecordedOperation::Upload {
                local_path: local_path.clone(),
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                relative_path: PathBuf::from("stable.txt"),
            }),
            "missing configured root should bootstrap from local files instead of deleting them"
        );
        assert!(
            local_path.exists(),
            "local data must not be deleted just because the configured remote root is missing"
        );
        assert_eq!(
            daemon
                .last_successful_sync_summary
                .as_ref()
                .map(|summary| summary.local_deletes),
            Some(0)
        );
    }

    #[test]
    fn reconcile_uploads_local_only_file_and_records_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("local-only.txt");
        fs::write(&local_path, b"local").expect("local file");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Upload {
                    local_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("local-only.txt"),
                })
        );
        let record = get_record(&daemon.connection, Path::new("local-only.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_uploads_nested_local_file_after_ensuring_remote_parent_directory() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("local-sub-directory")).expect("local nested root");
        let local_path = local_root
            .join("local-sub-directory")
            .join("subdirectory-file.txt");
        fs::write(&local_path, b"local").expect("nested local file");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![
                RecordedOperation::EnsureDirectory {
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("local-sub-directory"),
                },
                RecordedOperation::Upload {
                    local_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("local-sub-directory/subdirectory-file.txt"),
                },
            ]
        );
        let record = get_record(
            &daemon.connection,
            Path::new("local-sub-directory/subdirectory-file.txt"),
        )
        .expect("index lookup")
        .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Synced);
        let directory_record = get_record(&daemon.connection, Path::new("local-sub-directory"))
            .expect("directory index lookup")
            .expect("directory index record");
        assert_eq!(directory_record.entity_kind, EntityKind::Directory);
        assert_eq!(directory_record.sha1_hash, None);
        assert_eq!(directory_record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_creates_remote_empty_directory_and_records_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("empty-local-dir")).expect("local empty directory");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![RecordedOperation::EnsureDirectory {
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                relative_path: PathBuf::from("empty-local-dir"),
            }]
        );
        let record = get_record(&daemon.connection, Path::new("empty-local-dir"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.entity_kind, EntityKind::Directory);
        assert_eq!(record.sha1_hash, None);
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_creates_local_empty_directory_from_remote_entity() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("empty-remote-dir"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("empty-remote-dir"),
                id: Some("remote-dir-id".to_owned()),
                name: "empty-remote-dir".to_owned(),
            }),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote directory bootstrap should not upload, download, or delete file content"
        );
        assert!(
            local_root.join("empty-remote-dir").is_dir(),
            "remote-only directory should be created locally"
        );
        let record = get_record(&daemon.connection, Path::new("empty-remote-dir"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.entity_kind, EntityKind::Directory);
        assert_eq!(record.proton_id.as_deref(), Some("remote-dir-id"));
        assert_eq!(record.sha1_hash, None);
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_resolves_directory_file_type_conflict_with_a_sidecar_download() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("same-name")).expect("local directory");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("same-name"),
            RemoteEntity::File(remote("same-name", "remote-file-id", Some("hash"))),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        let conflict_path = local_root.join("same-name.proton-cloud");
        assert_eq!(
            fs::read_to_string(&conflict_path).expect("conflict sidecar"),
            "downloaded:/Drive/RemoteFolder/same-name"
        );
        assert!(
            local_root.join("same-name").is_dir(),
            "the local directory must win the clash and stay in place"
        );
        let directory_record = get_record(&daemon.connection, Path::new("same-name"))
            .expect("directory index lookup")
            .expect("directory index record");
        assert_eq!(directory_record.entity_kind, EntityKind::Directory);
        assert_eq!(directory_record.sync_status, SyncStatus::Synced);
        assert_eq!(directory_record.proton_id, None);
        let sidecar_record = get_record(&daemon.connection, Path::new("same-name.proton-cloud"))
            .expect("sidecar index lookup")
            .expect("sidecar index record");
        assert_eq!(sidecar_record.entity_kind, EntityKind::File);
        assert_eq!(sidecar_record.sync_status, SyncStatus::Conflict);
        assert_eq!(sidecar_record.proton_id.as_deref(), Some("remote-file-id"));

        daemon.reconcile_blocking().expect("second reconcile");

        assert_eq!(
            operations.lock().expect("operations lock").len(),
            1,
            "an already-resolved directory/file clash must not re-download the sidecar \
             on a subsequent reconcile"
        );
    }

    #[test]
    fn reconcile_applies_verified_remote_rename_as_local_move() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let old_path = local_root.join("old-name.txt");
        fs::write(&old_path, b"same content").expect("old local file");
        let hash = crate::index::compute_sha1(&old_path).expect("old hash");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("new-name.txt"),
            RemoteEntity::File(remote("new-name.txt", "stable-id", Some(hash.as_str()))),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("old-name.txt", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote rename convergence should not upload, download, or delete"
        );
        assert!(!old_path.exists(), "old local path should be moved away");
        assert!(local_root.join("new-name.txt").is_file());
        assert!(
            get_record(&daemon.connection, Path::new("old-name.txt"))
                .expect("old index lookup")
                .is_none(),
            "old path should be purged from the index"
        );
        let record = get_record(&daemon.connection, Path::new("new-name.txt"))
            .expect("new index lookup")
            .expect("new index record");
        assert_eq!(record.proton_id.as_deref(), Some("stable-id"));
        assert_eq!(record.sha1_hash.as_deref(), Some(hash.as_str()));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_executes_verified_local_rename_as_remote_move() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let new_path = local_root.join("new-name.txt");
        fs::write(&new_path, b"same content").expect("new local file");
        let hash = crate::index::compute_sha1(&new_path).expect("new hash");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("old-name.txt"),
            RemoteEntity::File(remote("old-name.txt", "stable-id", Some(hash.as_str()))),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("old-name.txt", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            operations.lock().expect("operations lock").as_slice(),
            [RecordedOperation::RenameOrMove {
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                old_relative_path: PathBuf::from("old-name.txt"),
                new_relative_path: PathBuf::from("new-name.txt"),
            }],
            "verified local rename should execute exactly one remote rename/move"
        );
        assert!(new_path.is_file());
        assert!(
            get_record(&daemon.connection, Path::new("old-name.txt"))
                .expect("old index lookup")
                .is_none(),
            "old path should be purged from the index"
        );
        let record = get_record(&daemon.connection, Path::new("new-name.txt"))
            .expect("new index lookup")
            .expect("new index record");
        assert_eq!(record.proton_id.as_deref(), Some("stable-id"));
        assert_eq!(record.sha1_hash.as_deref(), Some(hash.as_str()));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn remote_move_into_a_new_directory_creates_the_destination_parent_first() {
        // Regression for #141: a local file moved into a brand-new subfolder replays as a
        // remote move whose destination parent does not exist on the remote yet. Because the
        // move is a transition action prepended ahead of the folder's `CreateRemoteDirectory`,
        // the executor must ensure that parent *itself* before the move — otherwise
        // `rename_or_move` fails with "Node not found" every pass and the plan never makes
        // progress (the observed poison-pill loop). The `MoveLocal` and `Upload` arms already
        // ensure their destination parent; this proves `MoveRemote` now does too.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let new_dir = local_root.join("AI History");
        fs::create_dir(&new_dir).expect("new local dir");
        let new_path = new_dir.join("claude_export.zip");
        fs::write(&new_path, b"same content").expect("moved local file");
        let hash = crate::index::compute_sha1(&new_path).expect("hash");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("claude_export.zip"),
            RemoteEntity::File(remote(
                "claude_export.zip",
                "stable-id",
                Some(hash.as_str()),
            )),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("claude_export.zip", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        let ops = operations.lock().expect("operations lock");
        let move_index = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    RecordedOperation::RenameOrMove { old_relative_path, new_relative_path, .. }
                        if old_relative_path == Path::new("claude_export.zip")
                            && new_relative_path == Path::new("AI History/claude_export.zip")
                )
            })
            .expect("the remote move must be executed");
        let ensure_index = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    RecordedOperation::EnsureDirectory { relative_path, .. }
                        if relative_path == Path::new("AI History")
                )
            })
            .expect("the destination parent folder must be ensured on the remote");
        assert!(
            ensure_index < move_index,
            "the destination folder must be created before the move: ops={ops:?}"
        );
    }

    #[test]
    fn reconcile_autolinks_matching_bootstrap_file_without_network_transfer() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("shared.txt");
        fs::write(&local_path, b"same content").expect("local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("shared.txt"),
            remote("shared.txt", "remote-id", Some(local_hash.as_str())),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "auto-link must not upload, download, or delete content"
        );
        let record = get_record(&daemon.connection, Path::new("shared.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));
        assert_eq!(record.sha1_hash, Some(local_hash));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_uploads_modified_local_file_and_updates_index_hash() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local hash b").expect("local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("base-hash")),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Upload {
                    local_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("notes.txt"),
                })
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));
        assert_eq!(record.sha1_hash, Some(local_hash));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_downloads_remote_only_file_and_records_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("remote-only.txt"),
            remote("remote-only.txt", "remote-id", Some("remote-hash")),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        let destination = local_root.join("remote-only.txt");
        assert_eq!(
            fs::read_to_string(&destination).expect("downloaded file"),
            "downloaded:/Drive/RemoteFolder/remote-only.txt"
        );
        assert!(operations.lock().expect("operations lock").contains(
            &RecordedOperation::Download {
                remote_path: PathBuf::from("/Drive/RemoteFolder/remote-only.txt"),
                destination,
            }
        ));
        let record = get_record(&daemon.connection, Path::new("remote-only.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[cfg(unix)]
    #[test]
    fn download_through_a_symlinked_directory_is_refused_not_written_outside_root() {
        // local_root contains a pre-existing directory symlink `sub -> outside` (a common
        // user setup). A remote file under `sub/` must not be written through the symlink to
        // a location outside the sync root, and must not be recorded as synced.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let outside = directory.path().join("outside");
        fs::create_dir(&local_root).expect("local root");
        fs::create_dir(&outside).expect("outside dir");
        std::os::unix::fs::symlink(&outside, local_root.join("sub")).expect("directory symlink");

        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("sub/secret.txt"),
            remote("sub/secret.txt", "remote-id", Some("hash")),
        );
        let mut remote_contents = HashMap::new();
        remote_contents.insert(
            PathBuf::from("/Drive/RemoteFolder/sub/secret.txt"),
            b"exfiltrated".to_vec(),
        );
        let (client, operations) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect("reconcile skips the unsafe entry rather than erroring");

        assert!(
            !outside.join("secret.txt").exists(),
            "remote content must not be written outside the sync root through the symlink"
        );
        assert!(
            get_record(&daemon.connection, Path::new("sub/secret.txt"))
                .expect("index lookup")
                .is_none(),
            "a refused download must not be recorded as synced"
        );
        assert!(
            !operations
                .lock()
                .expect("operations lock")
                .iter()
                .any(|op| matches!(op, RecordedOperation::Download { .. })),
            "no download should run for a path that escapes the sync root"
        );
    }

    #[test]
    fn safe_paths_reject_the_empty_relative_path() {
        // #72: `validate_relative_path` maps "" and "." to Some("") — load-bearing for the
        // remote root itself (the listing's root wrapper node, and `CreateRemoteDirectory`'s
        // empty-path arm, which never reaches these helpers). Every path-keyed *side effect*
        // must refuse it: joined onto a root it resolves to the root, turning a per-file
        // download or delete into a whole-root one.
        let root = Path::new("/tmp/sync-root");
        assert_eq!(safe_remote_path(root, Path::new("")), None);
        assert_eq!(safe_remote_path(root, Path::new(".")), None);
        assert_eq!(safe_local_path(root, Path::new("")), None);
        assert_eq!(safe_local_path(root, Path::new(".")), None);
        assert_eq!(
            safe_remote_path(root, Path::new("notes.txt")),
            Some(root.join("notes.txt"))
        );
    }

    #[test]
    fn reconcile_never_downloads_a_remote_entry_that_resolves_to_the_sync_roots() {
        // #72, second layer: even if an empty-path entry reached the planner, the executor must
        // refuse it rather than run a download whose destination is the local root itself.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_files = HashMap::new();
        remote_files.insert(PathBuf::from(""), remote("", "root-id", Some("root-hash")));
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        let error = daemon
            .reconcile_blocking()
            .expect_err("an empty-path download must be refused, not executed");

        assert!(
            error.to_string().contains("unsafe remote path"),
            "unexpected error: {error}"
        );
        assert!(
            !operations
                .lock()
                .expect("operations lock")
                .iter()
                .any(|op| matches!(
                    op,
                    RecordedOperation::Download { .. } | RecordedOperation::DownloadBatch { .. }
                )),
            "no download may run for an entry that resolves to the sync roots"
        );
        assert!(
            local_root
                .read_dir()
                .expect("local root listing")
                .next()
                .is_none(),
            "the local root must be untouched"
        );
    }

    #[test]
    fn reconcile_downloads_modified_remote_file_and_updates_index_hash() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let remote_content = b"remote hash b".to_vec();
        let remote_hash = sha1_bytes(&remote_content);
        let remote_path = PathBuf::from("/Drive/RemoteFolder/notes.txt");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some(remote_hash.as_str())),
        );
        let remote_contents = HashMap::from([(remote_path.clone(), remote_content.clone())]);
        let (client, operations) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            fs::read(&local_path).expect("downloaded local file"),
            remote_content
        );
        assert!(operations.lock().expect("operations lock").contains(
            &RecordedOperation::Download {
                remote_path,
                destination: local_path,
            }
        ));
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));
        assert_eq!(record.sha1_hash, Some(remote_hash));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_downloads_nested_remote_file_and_creates_parent_directories() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("nested/remote-only.txt"),
            remote("nested/remote-only.txt", "remote-id", Some("remote-hash")),
        );
        let client = ParentCheckingDownloadClient { remote_files };
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        let destination = local_root.join("nested/remote-only.txt");
        assert_eq!(
            fs::read_to_string(&destination).expect("downloaded file"),
            "downloaded:/Drive/RemoteFolder/nested/remote-only.txt"
        );
        let record = get_record(&daemon.connection, Path::new("nested/remote-only.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_deletes_local_file_when_remote_synced_file_was_removed() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("removed-remotely.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record(
                "removed-remotely.txt",
                Some("remote-id"),
                base_hash.as_str(),
            ),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            !local_path.exists(),
            "remote deletion should remove the unchanged local file"
        );
        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote deletion reconciliation must not call Proton mutations"
        );
        assert!(
            get_record(&daemon.connection, Path::new("removed-remotely.txt"))
                .expect("index lookup")
                .is_none(),
            "local delete should purge the index record"
        );
    }

    #[test]
    fn reconcile_deletes_remote_file_when_local_synced_file_was_removed() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("removed.txt"),
            remote("removed.txt", "remote-id", Some("base-hash")),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("removed.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Delete {
                    remote_path: PathBuf::from("/Drive/RemoteFolder/removed.txt"),
                })
        );
        assert!(
            get_record(&daemon.connection, Path::new("removed.txt"))
                .expect("index lookup")
                .is_none(),
            "remote delete should purge the index record"
        );
    }

    // --- delete-approval guard --------------------------------------------------------------

    /// `test_config` with the guard ON for both directions (production default).
    fn guarded_config(directory: &Path, local_root: &Path) -> DaemonConfig {
        DaemonConfig {
            delete_approval_remote: true,
            delete_approval_local: true,
            ..test_config(directory, local_root)
        }
    }

    #[test]
    fn guard_withholds_a_local_delete_and_preserves_the_file_and_index() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("keep.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            local_path.exists(),
            "the guard must not delete the local file without approval"
        );
        assert!(
            get_record(&daemon.connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_some(),
            "the base record must survive so the delete re-plans next pass"
        );
        assert!(operations.lock().expect("ops").is_empty());
        assert_eq!(daemon.pending_deletions.len(), 1);
        assert_eq!(
            daemon.pending_deletions[0].direction,
            DeleteDirection::Local
        );
        assert_eq!(daemon.pending_deletions[0].path, PathBuf::from("keep.txt"));
    }

    #[test]
    fn a_literal_path_selector_never_gets_the_reserved_all_meaning() {
        // The wire reserves "all" (any letter case) as the every-item selector — but a request
        // flagged `literal_path` is targeting a row by its actual path, so a pending deletion
        // for a file literally named "All" must be approvable alone. Without the flag the same
        // selector keeps its historical case-insensitive every-item meaning (legacy clients).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let pending = vec![
            PendingDeletion {
                path: PathBuf::from("All"),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: "fp-all".to_owned(),
                detected_epoch_secs: 1,
            },
            PendingDeletion {
                path: PathBuf::from("other.txt"),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: "fp-other".to_owned(),
                detected_epoch_secs: 1,
            },
        ];

        let message = apply_approval_command(&daemon.connection, &pending, Some("All"), true, true)
            .expect("literal-path approve");
        assert!(
            message.contains("approved 1"),
            "a literal-path selector must approve only the file named \"All\": {message}"
        );
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .len(),
            1,
        );

        let message =
            apply_approval_command(&daemon.connection, &pending, Some("All"), false, true)
                .expect("legacy approve");
        assert!(
            message.contains("approved 2"),
            "without the literal flag the same selector keeps the every-item meaning: {message}"
        );
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .len(),
            2,
        );
    }

    #[test]
    fn a_non_utf8_pending_deletion_is_approvable_through_its_wire_form() {
        // #61: a non-UTF-8 path cannot survive JSON, so a client only ever sees (and can only ever
        // send back) `to_string_lossy`. Matching the selector against the real `PathBuf` would make
        // exactly those paths permanently unapprovable — and they are the ones that motivated the
        // lossy wire. The approval must still be recorded against the REAL path, or the
        // execution-time gate would never find it.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let real_path = PathBuf::from(OsStr::from_bytes(b"bad-\xffdir/file.txt"));
        let pending = vec![PendingDeletion {
            path: real_path.clone(),
            direction: DeleteDirection::Remote,
            entity_kind: EntityKind::File,
            fingerprint: "fp".to_owned(),
            detected_epoch_secs: 1,
        }];
        let selector = crate::ipc::wire_path(&real_path);
        assert_ne!(
            PathBuf::from(&*selector),
            real_path,
            "the wire form is lossy"
        );

        let message =
            apply_approval_command(&daemon.connection, &pending, Some(&selector), true, true)
                .expect("approve by wire form");

        assert!(
            message.contains("approved 1"),
            "the lossy selector a client can send must match: {message}"
        );
        let approvals = crate::index::load_delete_approvals(&daemon.connection).expect("approvals");
        assert_eq!(approvals.len(), 1);
        assert_eq!(
            approvals[0].path, real_path,
            "the approval must be pinned to the real path the guard checks, not the lossy one"
        );
    }

    #[test]
    fn an_ambiguous_lossy_selector_approves_nothing() {
        // Two real paths differing only in the bytes `to_string_lossy` replaces render as ONE
        // selector, so a targeted approve cannot say which was meant — and the `pending` list the
        // user read from shows them identically. Approving both would delete a file nobody picked.
        // `deny` is not gated: over-revoking is the safe direction.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let pending: Vec<PendingDeletion> = [&b"bad-\xff.txt"[..], &b"bad-\xfe.txt"[..]]
            .iter()
            .enumerate()
            .map(|(index, bytes)| PendingDeletion {
                path: PathBuf::from(OsStr::from_bytes(bytes)),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: format!("fp-{index}"),
                detected_epoch_secs: 1,
            })
            .collect();
        let selector = crate::ipc::wire_path(&pending[0].path);
        assert_eq!(selector, crate::ipc::wire_path(&pending[1].path));

        let message =
            apply_approval_command(&daemon.connection, &pending, Some(&selector), true, true)
                .expect("ambiguous approve");

        assert!(
            message.contains("cannot be told apart"),
            "an ambiguous selector must explain itself: {message}"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("approvals")
                .is_empty(),
            "an ambiguous approve must authorize nothing"
        );

        // "all" is the deliberate every-item form and still works, as does a deny.
        apply_approval_command(&daemon.connection, &pending, Some("all"), false, true)
            .expect("approve all");
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("approvals")
                .len(),
            2
        );
        apply_approval_command(&daemon.connection, &pending, Some(&selector), true, false)
            .expect("ambiguous deny");
        assert!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("approvals")
                .is_empty(),
            "a deny is not gated: revoking more than asked is the safe direction"
        );
    }

    #[test]
    fn a_selector_with_a_trailing_slash_still_matches_a_pending_directory() {
        // Shell completion appends `/` to a directory, and pending deletions include directories.
        // The comparison is component-wise (`Path`), not string equality, so that still matches —
        // as it did before the selector moved to the wire form (#61).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let pending = vec![PendingDeletion {
            path: PathBuf::from("nested/folder"),
            direction: DeleteDirection::Remote,
            entity_kind: EntityKind::Directory,
            fingerprint: "fp-dir".to_owned(),
            detected_epoch_secs: 1,
        }];

        let message = apply_approval_command(
            &daemon.connection,
            &pending,
            Some("nested/folder/"),
            true,
            true,
        )
        .expect("approve by completed path");

        assert!(
            message.contains("approved 1"),
            "a trailing slash must not make a pending directory unapprovable: {message}"
        );
    }

    #[test]
    fn approve_without_a_selector_is_a_no_op_and_never_approves_everything() {
        // A missing argument (e.g. from a non-CLI IPC client) must not approve all pending
        // deletions; only an explicit "all" does.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("keep.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");
        daemon.reconcile_blocking().expect("reconcile");
        assert_eq!(daemon.pending_deletions.len(), 1, "a deletion is pending");

        // No selector → no-op: nothing is approved even though something is pending.
        let message = daemon
            .apply_approval_command(None, true)
            .expect("no-selector approve");
        assert!(
            message.contains("no target"),
            "unexpected message: {message}"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .is_empty(),
            "a missing selector must not approve any deletion"
        );

        // Explicit "all" is the deliberate way to approve everything pending.
        daemon
            .apply_approval_command(Some("all"), true)
            .expect("approve all");
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .len(),
            1,
            "an explicit \"all\" approves the pending deletion"
        );
    }

    #[test]
    fn guard_withholds_a_remote_delete_until_approved_then_applies_it() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        // Local file gone, remote still present, synced baseline → RemoteDelete.
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("removed.txt"),
            remote("removed.txt", "remote-id", Some("base-hash")),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("removed.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("first reconcile");
        assert!(
            operations.lock().expect("ops").is_empty(),
            "the remote delete must be withheld pending approval"
        );
        assert_eq!(daemon.pending_deletions.len(), 1);
        let pending = daemon.pending_deletions[0].clone();
        assert_eq!(pending.direction, DeleteDirection::Remote);

        // Approve exactly this deletion (path + direction + fingerprint), then reconcile again.
        upsert_delete_approval(
            &daemon.connection,
            &pending.path,
            pending.direction,
            &pending.fingerprint,
            1,
        )
        .expect("approve");

        daemon.reconcile_blocking().expect("second reconcile");
        assert!(
            operations
                .lock()
                .expect("ops")
                .contains(&RecordedOperation::Delete {
                    remote_path: PathBuf::from("/Drive/RemoteFolder/removed.txt"),
                }),
            "an approved remote delete must apply on the next reconcile"
        );
        assert!(
            get_record(&daemon.connection, Path::new("removed.txt"))
                .expect("lookup")
                .is_none(),
            "the applied delete purges the index record"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .is_empty(),
            "the approval is consumed once the delete has applied"
        );
        assert!(daemon.pending_deletions.is_empty());
    }

    #[test]
    fn a_directory_config_opts_a_subtree_out_of_the_guard() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("keep.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        // A root config disables the local-delete guard for the whole tree. It is ignored by the
        // scanner, so it is never itself planned for upload.
        fs::write(
            local_root.join(".proton-sync.toml"),
            "[delete_approval]\nlocal = false\n",
        )
        .expect("directory config");

        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            !local_path.exists(),
            "a subtree opted out of the guard must delete without approval"
        );
        assert!(daemon.pending_deletions.is_empty());
    }

    #[test]
    fn a_malformed_directory_config_keeps_the_guard_on() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("keep.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        fs::write(local_root.join(".proton-sync.toml"), "not = = valid toml")
            .expect("directory config");

        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            local_path.exists() && daemon.pending_deletions.len() == 1,
            "a malformed directory config must fail safe and keep the delete withheld"
        );
    }

    #[test]
    fn a_withheld_local_delete_holds_the_event_cursor() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let base = sha1_bytes(b"base");
        fs::write(local_root.join("a.txt"), b"base").expect("local file");

        // A remote delete event for the baseline node → the planner derives a LocalDelete.
        let client = EventFakeClient::new(HashMap::new());
        let full_walks = Arc::clone(&client.full_walks);
        let page = one_page(
            "cursor-1",
            vec![change(RemoteChangeKind::Deleted, "na", None, false)],
        );
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                delete_approval_local: true,
                delete_approval_remote: true,
                ..event_config(directory.path(), &local_root)
            },
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![page],
            ))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("a.txt", Some("vol~na"), base.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        daemon.reconcile_blocking().expect("incremental reconcile");

        assert_eq!(full_walks.load(Ordering::SeqCst), 0, "stayed incremental");
        assert!(
            local_root.join("a.txt").exists(),
            "the withheld local delete must not touch the file"
        );
        assert_eq!(daemon.pending_deletions.len(), 1);
        let cursor = load_event_cursor(&daemon.connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "the cursor must NOT advance while a destructive action is withheld, so the pending \
             delete keeps re-deriving from ground truth every pass"
        );
    }

    #[test]
    fn a_rename_edit_duplicate_proton_id_does_not_drop_the_withheld_local_delete() {
        // Issue #71(c) regression guard — spans the id-takeover pass AND the pass after it.
        //
        // A remote rename+edit (a.txt -> b.txt, content ALSO changed) fails move detection (the
        // digest no longer equals the base hash), so the planner falls back to Download(b.txt) +
        // LocalDelete(a.txt). With the local delete-approval guard on (the default), the
        // LocalDelete is WITHHELD while the Download commits a fresh b.txt row carrying the SAME
        // composed proton_id (vol~nx) as the still-present a.txt row — the transient DUPLICATE
        // proton_id issue #71 describes.
        //
        // This locks in the correct current behavior. On the next pass, with the move event still
        // in the delta (the cursor is held), the duplicate forces reconstruct's snapshot fallback
        // (issue #71(b)); the snapshot lists the real remote (a.txt gone, b.txt present) and
        // re-derives LocalDelete(a.txt) from ground truth, it stays withheld, a.txt survives, and
        // the cursor does NOT advance. A naive #71(c) "clear the id from the other row on
        // takeover" would erase the duplicate, let reconstruction complete with a PHANTOM
        // remote-present a.txt, drop the withheld delete, and advance the cursor past the move — a
        // worse data-integrity regression. See the comment in `index::upsert_record`.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let base_hash = sha1_bytes(b"base");
        // The fake client's `download` writes b"downloaded"; make the remote claim that exact
        // digest so b.txt is a clean no-op once it lands (no confounding re-download later).
        let downloaded_hash = sha1_bytes(b"downloaded");
        fs::write(local_root.join("a.txt"), b"base").expect("local a.txt");

        // Remote AFTER the rename+edit: node nx now lives at b.txt (a.txt is gone remotely). This
        // map backs both the targeted directory listing (the incremental resolver) and the full
        // walk (the snapshot fallback).
        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("b.txt"),
            remote_file_entity("b.txt", "vol~nx", downloaded_hash.as_str()),
        )]));
        let full_walks = Arc::clone(&client.full_walks);

        // The move arrives as Updated(node nx, not trashed). Script it on BOTH passes: the held
        // cursor means the real stream re-delivers it from cursor-0 until a pass advances past it
        // (which must not happen while the delete is withheld).
        let pages = vec![
            one_page(
                "cursor-1",
                vec![change(RemoteChangeKind::Updated, "nx", None, false)],
            ),
            one_page(
                "cursor-1",
                vec![change(RemoteChangeKind::Updated, "nx", None, false)],
            ),
        ];
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                delete_approval_local: true,
                delete_approval_remote: true,
                ..event_config(directory.path(), &local_root)
            },
            client,
            Some(Box::new(FakeEventSource::with_pages("cursor-1", pages))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("a.txt", Some("vol~nx"), base_hash.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        // Pass 1 — the id takeover. Incremental: Download(b.txt) commits a row with proton_id
        // vol~nx; LocalDelete(a.txt) is withheld, so a.txt keeps its vol~nx row → duplicate id.
        daemon
            .reconcile_blocking()
            .expect("pass 1 (incremental takeover)");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "pass 1 stays incremental (no full walk)"
        );
        assert!(local_root.join("b.txt").exists(), "b.txt was downloaded");
        assert!(
            local_root.join("a.txt").exists(),
            "the withheld local delete leaves a.txt in place"
        );
        assert_eq!(daemon.pending_deletions.len(), 1, "LocalDelete(a.txt) held");
        assert_eq!(
            daemon.pending_deletions[0].direction,
            DeleteDirection::Local
        );
        // Both rows now hold the same composed id — the transient duplicate. (A naive #71(c) would
        // already have cleared a.txt's id here, failing this assertion.)
        assert_eq!(
            get_record(&daemon.connection, Path::new("a.txt"))
                .expect("lookup a.txt")
                .expect("a.txt row")
                .proton_id
                .as_deref(),
            Some("vol~nx"),
            "the withheld local delete keeps a.txt pinned to vol~nx"
        );
        assert_eq!(
            get_record(&daemon.connection, Path::new("b.txt"))
                .expect("lookup b.txt")
                .expect("b.txt row")
                .proton_id
                .as_deref(),
            Some("vol~nx"),
            "the fresh download commits b.txt with the same vol~nx"
        );

        // Pass 2 — the pass after takeover, move event still in the delta. The duplicate proton_id
        // forces exactly one snapshot fallback (#71(b)); the snapshot re-derives the withheld
        // LocalDelete, a.txt survives, and the cursor is held.
        daemon
            .reconcile_blocking()
            .expect("pass 2 (fallback re-derives the withheld delete)");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the duplicate proton_id forces exactly one snapshot fallback"
        );
        assert!(
            local_root.join("a.txt").exists(),
            "a.txt must survive: a naive #71(c) id-clear would strand a phantom remote entry and \
             drop the delete"
        );
        assert_eq!(
            daemon.pending_deletions.len(),
            1,
            "the LocalDelete must still be withheld, not silently dropped"
        );
        assert_eq!(
            daemon.pending_deletions[0].direction,
            DeleteDirection::Local
        );
        let cursor = load_event_cursor(&daemon.connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "the cursor must NOT advance across the id takeover while the delete is withheld"
        );
    }

    #[test]
    fn an_approved_remote_delete_applies_on_the_next_incremental_pass() {
        // Regression (event-driven / default mode): a withheld RemoteDelete is *local*-origin —
        // tracked via `pending_changes` (cleared on commit) and generating no remote event — so an
        // empty-delta incremental pass must not idle-skip planning while it is pending. Otherwise
        // `approve` + `syncnow` would not apply it until the periodic bootstrap.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        // No local file: it was synced, then deleted locally → RemoteDelete.

        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]));
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                delete_approval_local: true,
                delete_approval_remote: true,
                ..event_config(directory.path(), &local_root)
            },
            client,
            // Empty pages on every fetch: a local deletion generates no remote event.
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;
        // The watcher would have observed the local deletion, so the first pass is not idle.
        daemon.pending_changes.insert(PathBuf::from("keep.txt"));

        // Pass 1: withhold.
        daemon
            .reconcile_blocking()
            .expect("first incremental reconcile");
        assert_eq!(full_walks.load(Ordering::SeqCst), 0, "stayed incremental");
        assert_eq!(daemon.pending_deletions.len(), 1);
        let pending = daemon.pending_deletions[0].clone();
        assert_eq!(pending.direction, DeleteDirection::Remote);

        // Pass 2: empty delta and `pending_changes` now cleared, but still pending → must re-derive
        // (not idle-skip), so the queue stays fresh.
        daemon
            .reconcile_blocking()
            .expect("second incremental reconcile");
        assert_eq!(
            daemon.pending_deletions.len(),
            1,
            "an empty-delta pass must re-derive the still-pending remote delete"
        );
        assert!(
            get_record(&daemon.connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_some(),
            "nothing is deleted while unapproved"
        );

        // Approve, then reconcile once more with the same empty delta: it must apply now.
        upsert_delete_approval(
            &daemon.connection,
            &pending.path,
            pending.direction,
            &pending.fingerprint,
            1,
        )
        .expect("approve");

        daemon
            .reconcile_blocking()
            .expect("third incremental reconcile");
        assert!(
            get_record(&daemon.connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_none(),
            "an approved remote delete must apply on the next incremental pass, not wait for a \
             periodic bootstrap"
        );
        assert!(daemon.pending_deletions.is_empty());
        assert!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .is_empty(),
            "the approval is consumed once applied"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "no full walk was needed"
        );
    }

    #[test]
    fn reconcile_purges_index_when_both_sides_deleted() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("gone.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "both-side deletion should not mutate local or remote files"
        );
        assert!(
            get_record(&daemon.connection, Path::new("gone.txt"))
                .expect("index lookup")
                .is_none(),
            "ghost state resolution should purge the index record"
        );
    }

    #[test]
    fn reconcile_recursively_deletes_remote_directory_and_purges_descendant_index_rows() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("docs"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("docs"),
                id: Some("docs-id".to_owned()),
                name: "docs".to_owned(),
            }),
        );
        remote_entities.insert(
            PathBuf::from("docs/report.txt"),
            RemoteEntity::File(remote("docs/report.txt", "report-id", Some("same-hash"))),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &FileRecord {
                file_path: PathBuf::from("docs"),
                entity_kind: EntityKind::Directory,
                file_size: 0,
                mtime: 1,
                sha1_hash: None,
                proton_id: Some("docs-id".to_owned()),
                sync_status: SyncStatus::Synced,
            },
        )
        .expect("directory base record");
        upsert_record(
            &daemon.connection,
            &base_record("docs/report.txt", Some("report-id"), "same-hash"),
        )
        .expect("file base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![RecordedOperation::Delete {
                remote_path: PathBuf::from("/Drive/RemoteFolder/docs"),
            }],
            "the whole subtree should be removed with a single recursive delete call"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs"))
                .expect("index lookup")
                .is_none(),
            "recursive remote delete should purge the directory's own index record"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs/report.txt"))
                .expect("index lookup")
                .is_none(),
            "recursive remote delete should purge descendant index records too"
        );
    }

    #[test]
    fn reconcile_recursively_deletes_local_directory_and_purges_descendant_index_rows() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("docs")).expect("local docs directory");
        let file_path = local_root.join("docs").join("report.txt");
        fs::write(&file_path, b"same content").expect("local file");
        let base_hash = crate::index::compute_sha1(&file_path).expect("base hash");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &FileRecord {
                file_path: PathBuf::from("docs"),
                entity_kind: EntityKind::Directory,
                file_size: 0,
                mtime: 1,
                sha1_hash: None,
                proton_id: Some("docs-id".to_owned()),
                sync_status: SyncStatus::Synced,
            },
        )
        .expect("directory base record");
        upsert_record(
            &daemon.connection,
            &base_record("docs/report.txt", Some("report-id"), base_hash.as_str()),
        )
        .expect("file base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "recursive local delete reconciliation must not call Proton mutations"
        );
        assert!(
            !local_root.join("docs").exists(),
            "the whole local directory subtree should be removed"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs"))
                .expect("index lookup")
                .is_none(),
            "recursive local delete should purge the directory's own index record"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs/report.txt"))
                .expect("index lookup")
                .is_none(),
            "recursive local delete should purge descendant index records too"
        );
    }

    #[test]
    fn reconcile_moves_local_directory_and_rewrites_descendant_index_rows() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("old-docs")).expect("local old-docs directory");
        let file_path = local_root.join("old-docs").join("report.txt");
        fs::write(&file_path, b"same content").expect("nested local file");
        let file_hash = crate::index::compute_sha1(&file_path).expect("nested file hash");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("new-docs"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("new-docs"),
                id: Some("docs-id".to_owned()),
                name: "new-docs".to_owned(),
            }),
        );
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &FileRecord {
                file_path: PathBuf::from("old-docs"),
                entity_kind: EntityKind::Directory,
                file_size: 0,
                mtime: 1,
                sha1_hash: None,
                proton_id: Some("docs-id".to_owned()),
                sync_status: SyncStatus::Synced,
            },
        )
        .expect("directory base record");
        upsert_record(
            &daemon.connection,
            &base_record("old-docs/report.txt", Some("report-id"), file_hash.as_str()),
        )
        .expect("nested file base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote directory rename convergence should not call any Proton mutation"
        );
        assert!(
            !local_root.join("old-docs").exists(),
            "old local directory path should be moved away"
        );
        assert!(local_root.join("new-docs").is_dir());
        assert!(local_root.join("new-docs").join("report.txt").is_file());
        assert!(
            get_record(&daemon.connection, Path::new("old-docs"))
                .expect("old directory index lookup")
                .is_none(),
            "old directory path should be purged from the index"
        );
        assert!(
            get_record(&daemon.connection, Path::new("old-docs/report.txt"))
                .expect("old descendant index lookup")
                .is_none(),
            "old descendant path should be purged from the index"
        );
        let directory_record = get_record(&daemon.connection, Path::new("new-docs"))
            .expect("new directory index lookup")
            .expect("new directory index record");
        assert_eq!(directory_record.entity_kind, EntityKind::Directory);
        assert_eq!(directory_record.proton_id.as_deref(), Some("docs-id"));
        let descendant_record = get_record(&daemon.connection, Path::new("new-docs/report.txt"))
            .expect("new descendant index lookup")
            .expect("new descendant index record");
        assert_eq!(descendant_record.proton_id.as_deref(), Some("report-id"));
        assert_eq!(
            descendant_record.sha1_hash.as_deref(),
            Some(file_hash.as_str())
        );
        assert_eq!(descendant_record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn reconcile_checkpoints_directory_move_when_later_action_fails() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("old-docs")).expect("local old-docs directory");
        let file_path = local_root.join("old-docs").join("report.txt");
        fs::write(&file_path, b"same content").expect("nested local file");
        let file_hash = crate::index::compute_sha1(&file_path).expect("nested file hash");
        fs::write(local_root.join("will-fail.txt"), b"fails").expect("failing upload file");
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("new-docs"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("new-docs"),
                id: Some("docs-id".to_owned()),
                name: "new-docs".to_owned(),
            }),
        );
        let (mut client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        client.failed_uploads = BTreeSet::from([PathBuf::from("will-fail.txt")]);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &FileRecord {
                file_path: PathBuf::from("old-docs"),
                entity_kind: EntityKind::Directory,
                file_size: 0,
                mtime: 1,
                sha1_hash: None,
                proton_id: Some("docs-id".to_owned()),
                sync_status: SyncStatus::Synced,
            },
        )
        .expect("directory base record");
        upsert_record(
            &daemon.connection,
            &base_record("old-docs/report.txt", Some("report-id"), file_hash.as_str()),
        )
        .expect("nested file base record");

        let error = daemon
            .reconcile_blocking()
            .expect_err("later upload should fail");

        assert!(
            error
                .to_string()
                .contains("upload failed for will-fail.txt"),
            "unexpected error: {error}"
        );
        assert!(
            operations
                .lock()
                .expect("operations lock")
                .iter()
                .any(|operation| matches!(operation, RecordedOperation::Upload { .. })),
            "the later upload attempt should still have been made"
        );
        assert!(
            !local_root.join("old-docs").exists(),
            "the directory rename side effect happens before the later failure"
        );
        assert!(local_root.join("new-docs").join("report.txt").is_file());
        // The move completed on disk before the later upload failed, so its checkpoint commit
        // must have recorded it — the index reflects what actually happened, and the completed
        // move is never re-derived (or worse, re-executed) on the retry pass.
        assert!(
            get_record(&daemon.connection, Path::new("old-docs"))
                .expect("old directory index lookup")
                .is_none(),
            "a completed directory move must be checkpoint-committed despite the later failure"
        );
        assert!(
            get_record(&daemon.connection, Path::new("old-docs/report.txt"))
                .expect("old descendant index lookup")
                .is_none(),
            "descendant rewrites commit in the same checkpoint as their directory move"
        );
        assert!(
            get_record(&daemon.connection, Path::new("new-docs"))
                .expect("new directory index lookup")
                .is_some(),
            "the moved directory's new index row lands with the move's own checkpoint"
        );
        assert!(
            get_record(&daemon.connection, Path::new("new-docs/report.txt"))
                .expect("new descendant index lookup")
                .is_some(),
            "the moved descendant's new index row lands with the move's own checkpoint"
        );
        assert!(
            get_record(&daemon.connection, Path::new("will-fail.txt"))
                .expect("failed upload index lookup")
                .is_none(),
            "the failed action itself must never be recorded"
        );
        assert!(
            daemon.last_sync.is_none(),
            "a failed pass must not count as a successful sync even with checkpoints committed"
        );
    }

    #[test]
    fn reconcile_checkpoints_completed_actions_when_later_action_fails() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let first_path = local_root.join("first.txt");
        let second_path = local_root.join("second.txt");
        fs::write(&first_path, b"first").expect("first file");
        fs::write(&second_path, b"second").expect("second file");
        let (client, operations) = RecordingProtonClient::with_failed_uploads(
            HashMap::new(),
            BTreeSet::from([PathBuf::from("second.txt")]),
        );
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        let error = daemon
            .reconcile_blocking()
            .expect_err("second upload should fail");

        assert!(
            error.to_string().contains("upload failed for second.txt"),
            "unexpected error: {error}"
        );
        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![
                RecordedOperation::Upload {
                    local_path: first_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("first.txt"),
                },
                RecordedOperation::Upload {
                    local_path: second_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("second.txt"),
                },
            ]
        );
        assert!(
            get_record(&daemon.connection, Path::new("first.txt"))
                .expect("first index lookup")
                .is_some(),
            "a completed upload must be checkpoint-committed even when a later action fails"
        );
        assert!(
            get_record(&daemon.connection, Path::new("second.txt"))
                .expect("second index lookup")
                .is_none(),
            "failed action must not be committed"
        );
        assert!(
            daemon.last_sync.is_none(),
            "failed reconciliation must not advance last_sync"
        );
        assert_eq!(
            daemon.last_error.as_deref(),
            Some("upload failed for second.txt")
        );
        assert_eq!(
            daemon
                .last_plan_summary
                .as_ref()
                .map(|summary| summary.total),
            Some(2)
        );
        assert!(daemon.last_successful_sync_summary.is_none());
        let status = daemon.status_response("daemon status");
        assert_eq!(
            status.last_error.as_deref(),
            Some("upload failed for second.txt")
        );
        assert_eq!(
            status
                .last_plan_summary
                .as_ref()
                .map(|summary| summary.total),
            Some(2)
        );
        assert!(status.last_successful_sync_summary.is_none());
    }

    #[test]
    fn consecutive_downloads_batch_by_directory_and_chunk_size() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let mut remote_entities = HashMap::new();
        for name in ["docs", "media"] {
            remote_entities.insert(
                PathBuf::from(name),
                RemoteEntity::Directory(RemoteDirectory {
                    path: PathBuf::from(name),
                    id: Some(format!("{name}-id")),
                    name: name.to_owned(),
                }),
            );
        }
        for (path, id, hash) in [
            ("docs/a.txt", "id-a", "ha"),
            ("docs/b.txt", "id-b", "hb"),
            ("docs/c.txt", "id-c", "hc"),
            ("media/d.txt", "id-d", "hd"),
        ] {
            remote_entities.insert(
                PathBuf::from(path),
                RemoteEntity::File(remote(path, id, Some(hash))),
            );
        }
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let config = DaemonConfig {
            download_batch_size: 2,
            ..test_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client(config, client).expect("daemon");

        daemon.reconcile_blocking().expect("bootstrap reconcile");

        // The three consecutive `docs/` downloads form one run: grouped under their shared
        // parent and chunked to the batch size (2 + 1). The `media/` download is separated
        // from them by media's own CreateLocalDirectory action, so its run has length one and
        // takes the plain single-file path.
        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![
                RecordedOperation::DownloadBatch {
                    remote_paths: vec![
                        PathBuf::from("/Drive/RemoteFolder/docs/a.txt"),
                        PathBuf::from("/Drive/RemoteFolder/docs/b.txt"),
                    ],
                },
                RecordedOperation::DownloadBatch {
                    remote_paths: vec![PathBuf::from("/Drive/RemoteFolder/docs/c.txt")],
                },
                RecordedOperation::Download {
                    remote_path: PathBuf::from("/Drive/RemoteFolder/media/d.txt"),
                    destination: local_root.join("media/d.txt"),
                },
            ]
        );
        for path in ["docs/a.txt", "docs/b.txt", "docs/c.txt", "media/d.txt"] {
            assert!(
                local_root.join(path).is_file(),
                "batched download must land {path} at its destination"
            );
            assert!(
                get_record(&daemon.connection, Path::new(path))
                    .expect("index lookup")
                    .is_some(),
                "batched download must record {path} in the index"
            );
        }
        assert!(daemon.last_sync.is_some(), "the pass succeeded");
    }

    #[test]
    fn same_parent_downloads_separated_by_a_subtree_never_merge_across_it() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        // The directories already exist locally AND carry base records, so the ongoing
        // planner emits no directory actions — leaving docs/a.txt, docs/sub/x.txt and
        // docs/z.txt as one uninterrupted run of downloads with interleaved parents.
        fs::create_dir_all(local_root.join("docs/sub")).expect("local directories");

        let mut remote_entities = HashMap::new();
        for (path, id) in [("docs", "dir-docs"), ("docs/sub", "dir-sub")] {
            remote_entities.insert(
                PathBuf::from(path),
                RemoteEntity::Directory(RemoteDirectory {
                    path: PathBuf::from(path),
                    id: Some(id.to_owned()),
                    name: Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path)
                        .to_owned(),
                }),
            );
        }
        for (path, id, hash) in [
            ("docs/a.txt", "id-a", "ha"),
            ("docs/sub/x.txt", "id-x", "hx"),
            ("docs/z.txt", "id-z", "hz"),
        ] {
            remote_entities.insert(
                PathBuf::from(path),
                RemoteEntity::File(remote(path, id, Some(hash))),
            );
        }
        let (client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let config = DaemonConfig {
            download_batch_size: 10,
            ..test_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client(config, client).expect("daemon");
        for (path, id) in [("docs", "dir-docs"), ("docs/sub", "dir-sub")] {
            upsert_record(
                &daemon.connection,
                &FileRecord {
                    file_path: PathBuf::from(path),
                    entity_kind: EntityKind::Directory,
                    file_size: 0,
                    mtime: 1,
                    sha1_hash: None,
                    proton_id: Some(id.to_owned()),
                    sync_status: SyncStatus::Synced,
                },
            )
            .expect("directory base record");
        }

        daemon.reconcile_blocking().expect("ongoing reconcile");

        // docs/a.txt and docs/z.txt share a parent but are separated in plan order by
        // docs/sub/x.txt; merging them into one batch would reorder execution relative to
        // the plan, so each contiguous same-parent segment must batch on its own.
        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![
                RecordedOperation::DownloadBatch {
                    remote_paths: vec![PathBuf::from("/Drive/RemoteFolder/docs/a.txt")],
                },
                RecordedOperation::DownloadBatch {
                    remote_paths: vec![PathBuf::from("/Drive/RemoteFolder/docs/sub/x.txt")],
                },
                RecordedOperation::DownloadBatch {
                    remote_paths: vec![PathBuf::from("/Drive/RemoteFolder/docs/z.txt")],
                },
            ],
            "same-parent downloads must not merge across an intervening subtree"
        );
        for path in ["docs/a.txt", "docs/sub/x.txt", "docs/z.txt"] {
            assert!(local_root.join(path).is_file(), "{path} must land");
        }
    }

    #[test]
    fn a_failed_batch_download_checkpoints_survivors_without_advancing_the_cursor() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("docs"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("docs"),
                id: Some("vol~nd".to_owned()),
                name: "docs".to_owned(),
            }),
        );
        remote_entities.insert(
            PathBuf::from("docs/a.txt"),
            RemoteEntity::File(remote("docs/a.txt", "vol~na", Some("ha"))),
        );
        remote_entities.insert(
            PathBuf::from("docs/b.txt"),
            RemoteEntity::File(remote("docs/b.txt", "vol~nb", Some("hb"))),
        );
        let (mut client, _operations) =
            RecordingProtonClient::with_remote_entities(remote_entities);
        client.failed_batch_downloads =
            BTreeSet::from([PathBuf::from("/Drive/RemoteFolder/docs/b.txt")]);
        let config = DaemonConfig {
            download_batch_size: 2,
            ..event_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-1"))),
        )
        .expect("daemon");

        let error = daemon
            .reconcile_blocking()
            .expect_err("a failed file in the batch must fail the pass");
        assert!(
            error.to_string().contains("download failed for docs/b.txt"),
            "unexpected error: {error}"
        );

        // The chunk's survivor was checkpoint-committed before the pass failed...
        assert!(
            local_root.join("docs/a.txt").is_file(),
            "the successfully downloaded file stays on disk"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs/a.txt"))
                .expect("survivor index lookup")
                .is_some(),
            "a completed download in a failing chunk must be checkpoint-committed"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs"))
                .expect("directory index lookup")
                .is_some(),
            "the earlier CreateLocalDirectory checkpoint must also survive"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs/b.txt"))
                .expect("failed index lookup")
                .is_none(),
            "the failed file must never be recorded"
        );
        // ...but the pass-level outcomes are still those of a failure: no cursor, no last_sync.
        assert!(
            load_event_cursor(&daemon.connection, "vol")
                .expect("load cursor")
                .is_none(),
            "a failed pass must never advance the event cursor, checkpoints notwithstanding"
        );
        assert!(daemon.last_sync.is_none());
    }

    #[test]
    fn a_batch_item_that_cannot_be_recorded_fails_alone_and_the_survivors_commit() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("docs"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("docs"),
                id: Some("docs-id".to_owned()),
                name: "docs".to_owned(),
            }),
        );
        remote_entities.insert(
            PathBuf::from("docs/a.txt"),
            RemoteEntity::File(remote("docs/a.txt", "id-a", Some("ha"))),
        );
        remote_entities.insert(
            PathBuf::from("docs/b.txt"),
            RemoteEntity::File(remote("docs/b.txt", "id-b", Some("hb"))),
        );
        let (mut client, _operations) =
            RecordingProtonClient::with_remote_entities(remote_entities);
        // The client reports success for b.txt but never writes it, so the daemon's stat of
        // the landed file fails — that must be b.txt's OWN failure, not a `?` that discards
        // the chunk's other survivor.
        client.unstaged_batch_downloads =
            BTreeSet::from([PathBuf::from("/Drive/RemoteFolder/docs/b.txt")]);
        let config = DaemonConfig {
            download_batch_size: 2,
            ..test_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client(config, client).expect("daemon");

        let error = daemon
            .reconcile_blocking()
            .expect_err("an unrecordable batch item must fail the pass");
        assert!(
            error.to_string().contains("could not be recorded"),
            "unexpected error: {error}"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs/a.txt"))
                .expect("survivor index lookup")
                .is_some(),
            "the chunk's other survivor must still be checkpoint-committed"
        );
        assert!(
            get_record(&daemon.connection, Path::new("docs/b.txt"))
                .expect("failed index lookup")
                .is_none(),
            "the unrecordable item must not be recorded"
        );
        assert!(daemon.last_sync.is_none());
    }

    #[test]
    fn an_executed_deletes_approval_is_consumed_even_when_a_later_action_fails() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        // A local-only file whose upload fails, ordered after the delete ("removed.txt" <
        // "zz-fail.txt"), so every pass ends in an error after the delete has executed.
        fs::write(local_root.join("zz-fail.txt"), b"fails").expect("failing upload file");

        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("removed.txt"),
            remote("removed.txt", "removed-id", Some("same-hash")),
        );
        let (mut client, operations) = RecordingProtonClient::new(remote_files);
        client.failed_uploads = BTreeSet::from([PathBuf::from("zz-fail.txt")]);
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("removed.txt", Some("removed-id"), "same-hash"),
        )
        .expect("base record");

        // Pass 1: the remote delete is withheld pending approval; the pass then fails on the
        // upload. The pending deletion is still published.
        daemon
            .reconcile_blocking()
            .expect_err("the failing upload fails the pass");
        assert_eq!(daemon.pending_deletions.len(), 1);
        let pending = daemon.pending_deletions[0].clone();
        upsert_delete_approval(
            &daemon.connection,
            &pending.path,
            pending.direction,
            &pending.fingerprint,
            1,
        )
        .expect("approve");

        // Pass 2: the approved delete executes and checkpoints — its index purge and the
        // approval consumption commit together — before the later upload fails the pass again.
        daemon
            .reconcile_blocking()
            .expect_err("the failing upload still fails the pass");
        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Delete {
                    remote_path: PathBuf::from("/Drive/RemoteFolder/removed.txt"),
                }),
            "the approved delete must have executed"
        );
        assert!(
            get_record(&daemon.connection, Path::new("removed.txt"))
                .expect("record lookup")
                .is_none(),
            "the executed delete's index purge must be checkpoint-committed despite the failure"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.connection)
                .expect("load approvals")
                .is_empty(),
            "the consumed approval must not survive the failed pass"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scheduled_reconcile_failure_is_reported_without_stopping_daemon() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut daemon = Daemon::with_client(
            test_config(directory.path(), &local_root),
            FailingListProtonClient,
        )
        .expect("daemon");

        daemon
            .reconcile_if_needed()
            .await
            .expect("scheduled failure should not stop daemon");

        assert_eq!(daemon.last_error.as_deref(), Some("list failed"));
        assert!(daemon.last_sync.is_none());
        assert!(daemon.last_plan_summary.is_none());
        assert!(daemon.last_successful_sync_summary.is_none());
        let status = daemon.status_response("daemon status");
        assert_eq!(status.status, "running");
        assert_eq!(status.last_error.as_deref(), Some("list failed"));
        assert_eq!(status.status_history.len(), 1);
        assert_eq!(status.status_history[0].message, "sync failed");
        assert_eq!(
            status.status_history[0].last_error.as_deref(),
            Some("list failed")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_reconciles_once_on_startup_before_the_first_interval_tick() {
        // Regression: a fresh daemon must converge immediately when `run()` starts, not sit
        // idle until the first periodic `scan_interval` tick. The event loop consumes the
        // interval's immediate first tick, and filesystem-watch events only accumulate
        // `pending_changes` (they never trigger a reconcile), so the periodic tick is the sole
        // automatic sync trigger. Without a startup reconcile, a fresh sync from a populated
        // remote downloads *nothing* until `scan_interval` (default 5 minutes) elapses.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("remote-hash")),
        );
        let client = ParentCheckingDownloadClient { remote_files };

        // A scan interval far longer than the poll window below, so if the file ever appears
        // locally only the startup reconcile — never a periodic tick — could have produced it.
        let config = DaemonConfig {
            scan_interval: Duration::from_secs(3600),
            ..test_config(directory.path(), &local_root)
        };
        let daemon = Daemon::with_client(config, client).expect("daemon");

        let downloaded = local_root.join("notes.txt");
        let handle = tokio::spawn(daemon.run());

        // Poll for the startup reconcile to land the file. Generous bound; in practice the
        // reconcile completes within milliseconds of `run()` starting.
        let mut appeared = false;
        for _ in 0..100 {
            if downloaded.exists() {
                appeared = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Abort and then await the handle so the daemon task is fully torn down (its DB
        // connection, lockfile, and watcher on `directory` released) before the assertions and
        // the tempdir cleanup at end of scope — otherwise the aborted task could still be
        // unwinding, leaking resources and making the suite flaky. The task is idle in its
        // `select!` loop by now (the download already landed), so the abort takes effect
        // promptly; the `JoinError::Cancelled` from the abort is expected and ignored.
        handle.abort();
        let _ = handle.await;

        assert!(
            appeared,
            "a fresh daemon must reconcile on startup and download the remote file, not wait \
             for the first scan-interval tick"
        );
        assert_eq!(
            fs::read_to_string(&downloaded).expect("downloaded file"),
            "downloaded:/Drive/RemoteFolder/notes.txt"
        );
    }

    #[test]
    fn status_response_reports_the_resolved_running_config() {
        // A UI client can only reflect the daemon's real folder pair if the status reply carries
        // it — the daemon may have been launched with flags and no config file the client knows.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let config = test_config(directory.path(), &local_root);
        let expected_remote = config.remote_root.clone();
        let expected_db = config.db_path.clone();
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(config, client).expect("daemon");

        let status = daemon.status_response("daemon status");
        let info = status
            .config
            .expect("status must report the running config");
        assert_eq!(info.local_root, local_root);
        assert_eq!(info.remote_root, expected_remote);
        assert_eq!(info.db_path, expected_db);
    }

    #[test]
    fn daemon_loads_persisted_status_history_on_restart() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let config = test_config(directory.path(), &local_root);
        {
            let (client, _) = RecordingProtonClient::new(HashMap::new());
            let mut daemon = Daemon::with_client(config.clone(), client).expect("daemon");

            daemon.reconcile_blocking().expect("reconcile");

            let status = daemon.status_response("daemon status");
            assert_eq!(status.status_history.len(), 1);
            assert_eq!(status.status_history[0].message, "sync completed");
            assert_eq!(
                status.status_history[0]
                    .successful_sync_summary
                    .as_ref()
                    .map(|summary| summary.total),
                Some(0)
            );
        }

        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(config, client).expect("restarted daemon");

        let status = daemon.status_response("daemon status");
        assert_eq!(status.status_history.len(), 1);
        assert_eq!(status.status_history[0].message, "sync completed");
        assert!(status.status_history[0].last_error.is_none());
    }

    #[test]
    fn daemon_state_files_are_not_planned_for_sync() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let config = DaemonConfig {
            db_path: local_root.join("sync_index.db"),
            ..test_config(directory.path(), &local_root)
        };
        fs::write(status_history_path(&config.db_path), "[]").expect("status history file");
        fs::write(metrics_path(&config.db_path), "{}").expect("metrics file");

        let plan = preview_plan_with_client(
            &config,
            &FakeProtonClient {
                remote_files: HashMap::new(),
            },
        )
        .expect("preview plan");

        assert!(
            plan.is_empty(),
            "daemon state files must be ignored by sync planning: {plan:?}"
        );
    }

    #[test]
    fn leftover_download_scratch_directory_is_not_planned_for_upload() {
        // A crash-orphaned download scratch directory inside the synced root must never
        // be planned for upload; otherwise the junk directory and its partial file would
        // be pushed to the remote and propagated to every other client.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let scratch_dir = local_root.join(format!("{}1234-9876", crate::DOWNLOAD_SCRATCH_PREFIX));
        fs::create_dir(&scratch_dir).expect("scratch dir");
        fs::write(scratch_dir.join("budget.xlsx"), b"partial download").expect("partial file");
        let config = test_config(directory.path(), &local_root);

        let plan = preview_plan_with_client(
            &config,
            &FakeProtonClient {
                remote_files: HashMap::new(),
            },
        )
        .expect("preview plan");

        assert!(
            plan.is_empty(),
            "an orphaned download scratch directory must never be planned: {plan:?}"
        );
    }

    #[test]
    fn daemon_writes_metrics_snapshot_after_reconcile() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let config = test_config(directory.path(), &local_root);
        let metrics_path = metrics_path(&config.db_path);
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(config, client).expect("daemon");

        daemon.reconcile_blocking().expect("reconcile");

        let metrics = fs::read_to_string(metrics_path).expect("metrics snapshot");
        let metrics: serde_json::Value = serde_json::from_str(&metrics).expect("metrics JSON");
        assert_eq!(metrics["status"], "running");
        assert_eq!(metrics["paused"], false);
        assert!(metrics["last_sync_epoch_secs"].as_u64().is_some());
        assert!(metrics["last_error"].is_null());
        assert_eq!(metrics["last_plan_summary"]["total"].as_u64(), Some(0));
        assert_eq!(
            metrics["last_successful_sync_summary"]["total"].as_u64(),
            Some(0)
        );
        assert_eq!(metrics["status_history_entries"].as_u64(), Some(1));
    }

    #[test]
    fn reconcile_downloads_conflict_sidecar_and_marks_index_conflict() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("notes.txt"), b"local changed").expect("local file");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("remote-changed")),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        let conflict_path = local_root.join("notes.proton-cloud.txt");
        assert_eq!(
            fs::read_to_string(&conflict_path).expect("conflict sidecar"),
            "downloaded:/Drive/RemoteFolder/notes.txt"
        );
        assert!(operations.lock().expect("operations lock").contains(
            &RecordedOperation::Download {
                remote_path: PathBuf::from("/Drive/RemoteFolder/notes.txt"),
                destination: conflict_path,
            }
        ));
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
    }

    #[test]
    fn reconcile_copies_the_local_edit_into_a_sidecar_when_the_remote_is_deleted() {
        // (Changed, Missing), issues #46/#15: the remote node is confirmed gone, so nothing may be
        // downloaded; the local edit is preserved and the sidecar is materialized by COPYING it,
        // which is the artefact the GUI conflicts list walks for and the handle for the exit.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local edit").expect("local file");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .iter()
                .all(|operation| !matches!(
                    operation,
                    RecordedOperation::Download { .. } | RecordedOperation::DownloadBatch { .. }
                )),
            "a confirmed-missing remote must never be downloaded for a sidecar"
        );
        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        assert_eq!(
            fs::read(&sidecar_path).expect("sidecar"),
            b"local edit",
            "the sidecar must be a copy of the surviving local file"
        );
        assert_eq!(
            fs::read(&local_path).expect("local file"),
            b"local edit",
            "the local edit must be preserved untouched"
        );
        // The two predicates `gui_core::conflicts::scan_conflicts` walks the disk with: the sidecar
        // is only in the GUI's conflicts list if both hold for what the daemon actually wrote.
        assert!(crate::sync::is_conflict_copy(&sidecar_path));
        assert_eq!(
            crate::sync::original_from_conflict_copy(&sidecar_path),
            Some(local_path.clone())
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));

        let operations_after_first = operations.lock().expect("operations lock").len();
        daemon.reconcile_blocking().expect("second reconcile");
        assert_eq!(
            operations.lock().expect("operations lock").len(),
            operations_after_first,
            "a parked conflict must not re-run its side effects every pass"
        );
    }

    #[test]
    fn deleting_the_copied_sidecar_uploads_the_preserved_local_edit() {
        // The exit for the (Changed, Missing) conflict: removing the sidecar re-arms the record and
        // the next pass uploads the local edit instead of resurrecting the sidecar forever (#46b).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local edit").expect("local file");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("first reconcile");

        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        fs::remove_file(&sidecar_path).expect("resolve by removing the sidecar");
        daemon
            .handle_fs_event(
                Event::new(EventKind::Remove(RemoveKind::File)).add_path(sidecar_path.clone()),
            )
            .expect("handle sidecar remove event");

        daemon.reconcile_blocking().expect("second reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Upload {
                    local_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("notes.txt"),
                }),
            "deleting the sidecar must resolve the conflict by uploading the local edit"
        );
        assert!(
            !sidecar_path.exists(),
            "the resolved sidecar must not be resurrected"
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn a_replanned_remote_deleted_conflict_never_overwrites_the_existing_sidecar() {
        // Editing the original (instead of resolving) marks the record `Modified`, which escapes
        // the Conflict early-return and re-plans the same conflict. That re-plan must leave the
        // user's sidecar alone: it is the only copy of the content the conflict was raised over,
        // and clobbering it with newer local bytes would silently destroy the resolution choice.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local edit").expect("local file");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("first reconcile");

        fs::write(&local_path, b"second local edit").expect("edit the original again");
        daemon
            .handle_fs_event(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
                    .add_path(local_path.clone()),
            )
            .expect("handle local edit event");

        daemon.reconcile_blocking().expect("second reconcile");

        assert_eq!(
            fs::read(local_root.join("notes.proton-cloud.txt")).expect("sidecar"),
            b"local edit",
            "the existing sidecar must survive a re-planned conflict untouched"
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            record.sync_status,
            SyncStatus::Conflict,
            "the re-plan re-parks the record, and the sidecar is still its exit"
        );
        assert_eq!(
            record.sha1_hash.as_deref(),
            Some(sha1_bytes(b"second local edit").as_str()),
            "the record must track the newest local content"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_sidecar_path_never_receives_the_preserved_local_file() {
        // The sidecar path is predictable (`notes.proton-cloud.txt` beside `notes.txt`), so
        // anything already sitting there — a hostile plant or a stale link the user made — must
        // not be followed. A BROKEN symlink is the dangerous shape: `Path::exists()` follows it
        // and reports "absent", and `local_write_escapes_root` canonicalizes the deepest EXISTING
        // ancestor, which a dangling link does not have, so neither guard sees it.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let outside = directory.path().join("outside");
        fs::create_dir(&outside).expect("outside dir");
        let target = outside.join("target.txt");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local edit").expect("local file");
        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        std::os::unix::fs::symlink(&target, &sidecar_path).expect("plant a broken symlink");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            !target.exists(),
            "the preserved local edit must never be written through a symlink at the sidecar path"
        );
        assert!(
            fs::symlink_metadata(&sidecar_path)
                .expect("sidecar path")
                .is_symlink(),
            "the user's own object at the path must be left exactly as it was"
        );
        assert_eq!(
            fs::read_link(&sidecar_path).expect("symlink target"),
            target,
            "the symlink must not be retargeted or replaced"
        );
        assert_eq!(
            fs::read(&local_path).expect("local file"),
            b"local edit",
            "the local edit must still be preserved"
        );
        // Converges rather than wedging: the conflict is recorded, so the record parks instead of
        // re-planning this action with a warning on every pass.
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
        daemon.reconcile_blocking().expect("second reconcile");
        assert!(
            !target.exists(),
            "a later pass must not write through it either"
        );
    }

    #[test]
    fn a_pre_existing_sidecar_file_is_left_alone_and_the_conflict_is_still_recorded() {
        // The user (or an interrupted earlier pass) already put a file at the sidecar path: it is
        // never overwritten, and the conflict is still recorded so the state keeps its exit.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local edit").expect("local file");
        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        fs::write(&sidecar_path, b"the user's own copy").expect("pre-existing sidecar");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            fs::read(&sidecar_path).expect("sidecar"),
            b"the user's own copy",
            "an existing sidecar file must never be overwritten"
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
    }

    #[test]
    fn reconcile_replaces_a_stale_directory_record_with_the_new_remote_file() {
        // #47: `docs` was a synced directory, deleted on both sides; a remote FILE now holds the
        // name. Only the BASE kind is stale (no live clash), so the pass must adopt the surviving
        // remote file instead of warning about a type conflict forever.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let content = b"the new remote file".to_vec();
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("docs"),
            remote(
                "docs",
                "remote-file-id",
                Some(sha1_bytes(&content).as_str()),
            ),
        );
        let mut remote_contents = HashMap::new();
        remote_contents.insert(PathBuf::from("/Drive/RemoteFolder/docs"), content.clone());
        let (client, operations) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &directory_record("docs", Some("dir-id")),
        )
        .expect("stale directory record");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            fs::read(local_root.join("docs")).expect("downloaded file"),
            content,
            "the surviving remote file must be downloaded over the stale directory record"
        );
        let record = get_record(&daemon.connection, Path::new("docs"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            record.entity_kind,
            EntityKind::File,
            "the stale directory kind must be replaced, not kept"
        );
        assert_eq!(record.sync_status, SyncStatus::Synced);
        assert_eq!(record.proton_id.as_deref(), Some("remote-file-id"));

        let operations_after_first = operations.lock().expect("operations lock").len();
        daemon.reconcile_blocking().expect("second reconcile");
        assert_eq!(
            operations.lock().expect("operations lock").len(),
            operations_after_first,
            "the adopted file must converge instead of re-downloading every pass"
        );
    }

    #[test]
    fn reconcile_autolinks_identical_local_and_remote_content_instead_of_conflicting() {
        // Reproduces the state left after a partial reconcile rolls back its deferred
        // index commit (see reconcile_does_not_commit_index_when_later_action_fails):
        // the baseline is stale at A while local and remote already agree on content B.
        // The next reconcile must recover cleanly by auto-linking, not fabricate a
        // spurious conflict sidecar and a permanently stuck Conflict record.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"converged content").expect("local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some(local_hash.as_str())),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "stale-base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "identical local and remote content must auto-link without any network \
             transfer or sidecar download"
        );
        assert!(
            !local_root.join("notes.proton-cloud.txt").exists(),
            "no spurious conflict sidecar must be created when both sides already agree"
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sha1_hash, Some(local_hash));
        assert_eq!(
            record.sync_status,
            SyncStatus::Synced,
            "the converged baseline must be recorded as Synced, not stuck in Conflict"
        );
    }

    #[test]
    fn reconcile_restores_remotely_edited_file_deleted_locally_and_then_converges() {
        // A synced file is deleted locally while the remote copy is edited. The first
        // reconcile must restore the remote edit at the original path and record it, and
        // the second reconcile must be a no-op instead of re-downloading forever.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let remote_content = b"remotely edited content".to_vec();
        // The sha1 the daemon will record for the restored bytes; the fake remote must
        // advertise the same hash so the second reconcile sees the file as converged.
        let hash_probe = directory.path().join("hash-probe");
        fs::write(&hash_probe, &remote_content).expect("hash probe");
        let remote_hash = crate::index::compute_sha1(&hash_probe).expect("remote hash");
        fs::remove_file(&hash_probe).ok();

        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some(remote_hash.as_str())),
        );
        let mut remote_contents = HashMap::new();
        remote_contents.insert(
            PathBuf::from("/Drive/RemoteFolder/notes.txt"),
            remote_content.clone(),
        );
        let (client, operations) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        // Baseline records notes.txt as previously synced at a stale hash; the file is
        // absent on disk (deleted locally).
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash-a"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect("first reconcile");

        let restored = local_root.join("notes.txt");
        assert_eq!(
            fs::read(&restored).expect("restored file"),
            remote_content,
            "the remote edit must be restored at the original path"
        );
        assert!(
            !local_root.join("notes.proton-cloud.txt").exists(),
            "restoring must not create a conflict sidecar"
        );
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Synced);
        assert_eq!(record.sha1_hash.as_deref(), Some(remote_hash.as_str()));
        let downloads = |ops: &Arc<Mutex<Vec<RecordedOperation>>>| {
            ops.lock()
                .expect("operations lock")
                .iter()
                .filter(|op| matches!(op, RecordedOperation::Download { .. }))
                .count()
        };
        assert_eq!(
            downloads(&operations),
            1,
            "the first reconcile restores the remote edit with exactly one download"
        );

        daemon.reconcile_blocking().expect("second reconcile");

        assert_eq!(
            downloads(&operations),
            1,
            "a restored, converged file must not be re-downloaded on the next reconcile"
        );
    }

    #[test]
    fn watcher_ignores_conflict_sidecar_create_events() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        fs::write(&sidecar_path, b"remote conflict copy").expect("conflict sidecar");
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon
            .handle_fs_event(
                Event::new(EventKind::Create(CreateKind::File)).add_path(sidecar_path.clone()),
            )
            .expect("handle sidecar create event");

        assert!(
            daemon.pending_changes.is_empty(),
            "downloaded conflict sidecars must not be queued as local creations"
        );
        assert!(
            get_record(&daemon.connection, Path::new("notes.proton-cloud.txt"))
                .expect("sidecar index lookup")
                .is_none(),
            "downloaded conflict sidecars must not be indexed as sync data"
        );
    }

    #[test]
    fn status_string_reports_syncing_even_while_a_pause_is_requested() {
        // A pause accepted mid-pass does not stop the in-flight pass; the status string must
        // keep reporting the live activity ("syncing") while the `paused` boolean carries the
        // standing request. Once the pass ends, the string becomes "paused".
        let shared = ControlShared::new(RunningConfigInfo {
            local_root: PathBuf::from("/local"),
            remote_root: PathBuf::from("/remote"),
            db_path: PathBuf::from("/db"),
        });
        shared.syncing.store(true, Ordering::SeqCst);
        shared.paused.store(true, Ordering::SeqCst);
        let mid_pass = shared.response("m");
        assert_eq!(mid_pass.status, "syncing");
        assert!(mid_pass.paused);
        assert!(mid_pass.syncing);

        shared.syncing.store(false, Ordering::SeqCst);
        assert_eq!(shared.response("m").status, "paused");

        shared.paused.store(false, Ordering::SeqCst);
        assert_eq!(shared.response("m").status, "running");
    }

    fn activity_test_shared() -> ControlShared {
        ControlShared::new(RunningConfigInfo {
            local_root: PathBuf::from("/local"),
            remote_root: PathBuf::from("/remote"),
            db_path: PathBuf::from("/db"),
        })
    }

    #[test]
    fn progress_sink_walk_updates_surface_in_status_replies() {
        let shared = Arc::new(activity_test_shared());
        // Replies only carry activity while a pass is in flight (`syncing`), matching how the
        // daemon core brackets every reconcile.
        shared.syncing.store(true, Ordering::SeqCst);
        let sink = SharedProgressSink {
            shared: Arc::clone(&shared),
        };

        sink.remote_folder_listed(3, Path::new("Companies/Acme"));
        let activity = shared.response("m").activity.expect("walk activity");
        assert_eq!(activity.phase, PHASE_LISTING_REMOTE);
        assert_eq!(activity.folders_listed, Some(3));
        assert_eq!(activity.detail.as_deref(), Some("Companies/Acme"));
        let first_since = activity.since_epoch_secs;

        // Further folders update the same activity (phase start time stays put)…
        sink.remote_folder_listed(4, Path::new(""));
        let activity = shared.response("m").activity.expect("walk activity");
        assert_eq!(activity.folders_listed, Some(4));
        assert_eq!(
            activity.detail.as_deref(),
            Some("/"),
            "the remote root itself renders as \"/\", not an empty string"
        );
        assert_eq!(
            activity.since_epoch_secs, first_since,
            "elapsed time is per-phase, not per-folder"
        );

        // …and the end of the pass clears everything.
        shared.clear_activity();
        assert!(shared.response("m").activity.is_none());

        // Even a not-yet-cleared activity is withheld once `syncing` is false: the two are
        // updated independently at pass end, and a reply between the two stores must never
        // pair `syncing: false` with a stale "downloading X".
        sink.remote_folder_listed(9, Path::new("stale"));
        shared.syncing.store(false, Ordering::SeqCst);
        assert!(shared.response("m").activity.is_none());
    }

    #[tokio::test]
    async fn download_bytes_are_sampled_live_from_the_staging_directory() {
        let shared = activity_test_shared();
        shared.syncing.store(true, Ordering::SeqCst);
        let scratch = tempdir().expect("scratch dir");
        fs::write(scratch.path().join("partial.bin"), vec![0u8; 2048]).expect("partial file");

        shared.begin_activity(SyncActivity {
            transfer: Some(TransferActivity {
                direction: "download".to_owned(),
                path: PathBuf::from("a/b.bin"),
                bytes_total: None,
                bytes_done: None,
                started_epoch_secs: 0,
            }),
            ..new_activity(PHASE_EXECUTING)
        });
        shared.note_download_scratch(scratch.path());

        // The plain (in-memory) reply never touches the filesystem…
        let transfer = shared
            .response("m")
            .activity
            .expect("activity")
            .transfer
            .expect("transfer");
        assert_eq!(
            transfer.bytes_done, None,
            "a plain reply must not sample the filesystem"
        );

        // …while the status reply samples the staging directory at reply time.
        let transfer = shared
            .response_with_sampled_activity("m")
            .await
            .activity
            .expect("activity")
            .transfer
            .expect("transfer");
        assert_eq!(transfer.bytes_done, Some(2048));

        // More bytes staged → the next status reply sees the larger number, with no
        // intervening daemon-side update: sampling happens at reply time.
        fs::write(scratch.path().join("partial.bin"), vec![0u8; 6144]).expect("grow file");
        let transfer = shared
            .response_with_sampled_activity("m")
            .await
            .activity
            .expect("activity")
            .transfer
            .expect("transfer");
        assert_eq!(transfer.bytes_done, Some(6144));

        // Beginning the next action drops the stale staging dir: a following upload must
        // never report the previous download's bytes.
        shared.begin_activity(SyncActivity {
            transfer: Some(TransferActivity {
                direction: "upload".to_owned(),
                path: PathBuf::from("c/d.bin"),
                bytes_total: Some(10),
                bytes_done: None,
                started_epoch_secs: 0,
            }),
            ..new_activity(PHASE_EXECUTING)
        });
        let transfer = shared
            .response_with_sampled_activity("m")
            .await
            .activity
            .expect("activity")
            .transfer
            .expect("transfer");
        assert_eq!(
            transfer.bytes_done, None,
            "an upload's progress is unobservable and must not inherit a stale scratch dir"
        );
    }

    #[tokio::test]
    async fn idle_ipc_client_is_dropped_after_the_io_timeout_instead_of_blocking() {
        // A client that connects to the control socket but never sends a request line must not
        // park its connection task forever. handle_control_connection must return after the IO
        // timeout rather than hang.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let approvals = tokio::sync::Mutex::new(
            open_database(&daemon.config.db_path).expect("second connection"),
        );
        let (loop_tx, _loop_rx) = mpsc::unbounded_channel();

        let (control_client, server) = UnixStream::pair().expect("socket pair");

        // The outer timeout is a generous test-only guard: if the handler regressed to
        // blocking forever it fails here instead of hanging the suite.
        let cancel_flag = AtomicBool::new(false);
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            handle_control_connection(
                server,
                &daemon.shared,
                &approvals,
                &loop_tx,
                Duration::from_millis(50),
                &daemon.metrics_path,
                &cancel_flag,
            ),
        )
        .await
        .expect("an idle control connection must be dropped after the IO timeout");
        drop(control_client);

        outcome.expect("dropping an idle control connection is a clean, non-error outcome");
    }

    #[test]
    fn shutdown_socket_removal_only_deletes_actual_sockets() {
        let directory = tempdir().expect("tempdir");

        // A real Unix socket left on disk is removed.
        let socket_path = directory.path().join("daemon.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("bind socket");
        drop(listener);
        assert!(
            socket_path.exists(),
            "binding leaves the socket file on disk"
        );
        remove_control_socket(&socket_path);
        assert!(
            !socket_path.exists(),
            "a real control socket must be removed on shutdown"
        );

        // A non-socket file swapped in at the socket path is left untouched.
        let regular = directory.path().join("not-a-socket");
        fs::write(&regular, b"important").expect("write regular file");
        remove_control_socket(&regular);
        assert!(
            regular.exists(),
            "a non-socket path must never be deleted on shutdown"
        );
        assert_eq!(
            fs::read(&regular).expect("regular file preserved"),
            b"important"
        );

        // A missing path is a clean no-op.
        remove_control_socket(&directory.path().join("missing.sock"));
    }

    #[test]
    fn watcher_queued_new_file_is_uploaded_on_reconcile() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("created-while-running.txt");
        fs::write(&local_path, b"new local content").expect("local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon
            .handle_fs_event(
                Event::new(EventKind::Create(CreateKind::File)).add_path(local_path.clone()),
            )
            .expect("handle create event");
        daemon.reconcile_blocking().expect("reconcile");

        assert!(local_path.exists(), "new local file must not be deleted");
        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Upload {
                    local_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("created-while-running.txt"),
                })
        );
        let record = get_record(&daemon.connection, Path::new("created-while-running.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sha1_hash, Some(local_hash));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn watcher_preserves_base_hash_until_existing_local_edit_uploads() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("edited-while-running.txt");
        fs::write(&local_path, b"base content").expect("base local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("edited-while-running.txt"),
            remote(
                "edited-while-running.txt",
                "remote-id",
                Some(base_hash.as_str()),
            ),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record(
                "edited-while-running.txt",
                Some("remote-id"),
                base_hash.as_str(),
            ),
        )
        .expect("base record");

        fs::write(&local_path, b"local content b").expect("modify local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        daemon
            .handle_fs_event(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
                    .add_path(local_path.clone()),
            )
            .expect("handle modify event");

        let queued_record = get_record(&daemon.connection, Path::new("edited-while-running.txt"))
            .expect("queued index lookup")
            .expect("queued index record");
        assert_eq!(queued_record.sha1_hash, Some(base_hash));
        assert_eq!(queued_record.sync_status, SyncStatus::Modified);

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Upload {
                    local_path,
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from("edited-while-running.txt"),
                })
        );
        let record = get_record(&daemon.connection, Path::new("edited-while-running.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sha1_hash, Some(local_hash));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn pending_change_that_plans_no_action_is_still_cleared_after_reconcile() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("stable.txt");
        fs::write(&local_path, b"stable content").expect("local file");
        let hash = crate::index::compute_sha1(&local_path).expect("hash");

        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("stable.txt"),
            remote("stable.txt", "id-1", Some(hash.as_str())),
        );
        let (client, _) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("stable.txt", Some("id-1"), hash.as_str()),
        )
        .expect("base record");

        // Simulate a spurious filesystem event (for example a metadata-only touch)
        // that queues a path in `pending_changes` even though the file never actually
        // diverges from the synced base on either side. Because both sides are
        // unchanged, planning produces no action at all for this path, so it can
        // never appear in a plan-keyed "completed paths" list - only an unconditional
        // clear after a successful commit reliably drops it instead of leaking it
        // forever.
        daemon.pending_changes.insert(PathBuf::from("stable.txt"));

        daemon.reconcile_blocking().expect("reconcile");

        assert!(
            daemon.pending_changes.is_empty(),
            "pending_changes must not leak entries for paths that plan to no action: {:?}",
            daemon.pending_changes
        );
    }

    #[test]
    fn authored_download_echo_does_not_flip_synced_record_to_modified() {
        // Issue #49 (T1): the daemon's own download write echoes back through the `notify`
        // watcher. That echo must NOT flip the just-committed `Synced` record to `Modified`
        // (a `Modified` record whose remote later changes plans a stale `Upload` that reverts
        // the newer remote edit). The echo must still register a pending change so the path is
        // re-examined next pass.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");

        let remote_content = b"remote content v1".to_vec();
        let remote_hash = sha1_bytes(&remote_content);
        let remote_path = PathBuf::from("/Drive/RemoteFolder/notes.txt");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some(remote_hash.as_str())),
        );
        let remote_contents = HashMap::from([(remote_path, remote_content.clone())]);
        let (client, _) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("notes.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        // Pass: local Unchanged vs base, remote Changed → the daemon downloads and commits Synced.
        daemon.reconcile_blocking().expect("reconcile");
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            record.sync_status,
            SyncStatus::Synced,
            "the download must commit a Synced record"
        );
        assert_eq!(record.sha1_hash, Some(remote_hash));

        // The daemon's own write echoes back through the watcher.
        daemon
            .handle_fs_event(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
                    .add_path(local_path.clone()),
            )
            .expect("handle echo event");

        let after = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            after.sync_status,
            SyncStatus::Synced,
            "the daemon's own download echo must not flip the record to Modified (issue #49)"
        );
        assert!(
            daemon.pending_changes.contains(Path::new("notes.txt")),
            "the echo must still register a pending change so the path is re-examined next pass"
        );
    }

    #[test]
    fn unauthored_write_still_marks_record_modified_even_with_a_pending_authored_write() {
        // Issue #49 (T2): the suppression must key on the path, not merely on "the set is
        // non-empty". A Create/Modify for a file the daemon did NOT author still flips its record
        // to `Modified` as before, even while an unrelated authored download sits in
        // `authored_writes`.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        // File A: remote-only this pass, so the daemon downloads it → it lands in `authored_writes`.
        let a_content = b"remote a".to_vec();
        let a_hash = sha1_bytes(&a_content);
        let a_remote_path = PathBuf::from("/Drive/RemoteFolder/a.txt");
        // File B: a stable, already-synced local file the daemon never writes.
        let b_path = local_root.join("b.txt");
        fs::write(&b_path, b"stable b").expect("b file");
        let b_hash = crate::index::compute_sha1(&b_path).expect("b hash");

        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("a.txt"),
            remote("a.txt", "id-a", Some(a_hash.as_str())),
        );
        remote_files.insert(
            PathBuf::from("b.txt"),
            remote("b.txt", "id-b", Some(b_hash.as_str())),
        );
        let remote_contents = HashMap::from([(a_remote_path, a_content)]);
        let (client, _) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("b.txt", Some("id-b"), b_hash.as_str()),
        )
        .expect("b base record");

        daemon.reconcile_blocking().expect("reconcile");
        assert!(
            daemon.authored_writes.contains(Path::new("a.txt")),
            "the downloaded file must be recorded as an authored write, so the guard is exercised"
        );
        assert!(
            !daemon.authored_writes.contains(Path::new("b.txt")),
            "the daemon never wrote B, so it must not be an authored write"
        );

        // A Modify for B — which the daemon did not author — must still mark it Modified, even
        // though `authored_writes` is non-empty (it holds A).
        daemon
            .handle_fs_event(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
                    .add_path(b_path.clone()),
            )
            .expect("handle modify event");

        let b_record = get_record(&daemon.connection, Path::new("b.txt"))
            .expect("index lookup")
            .expect("b record");
        assert_eq!(
            b_record.sync_status,
            SyncStatus::Modified,
            "a genuine (non-authored) edit must still flip the record to Modified"
        );
    }

    #[test]
    fn authored_echo_prevents_stale_upload_reverting_a_newer_remote_edit() {
        // Issue #49 (T3): end-to-end payoff. Pass 1 downloads a file (commits Synced) and its
        // write echoes through the watcher. The remote is then edited again. Because the echo did
        // NOT flip the record to `Modified`, pass 2 correctly plans a `Download` of the newer
        // remote content instead of an `Upload` that would revert it. The two passes use two
        // daemon instances on the same db + local_root so the payoff is carried by the persisted
        // record (the second daemon's fresh client models the remote having changed meanwhile).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("doc.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");

        let remote_path = PathBuf::from("/Drive/RemoteFolder/doc.txt");
        let content_v1 = b"remote content v1".to_vec();
        let hash_v1 = sha1_bytes(&content_v1);
        let content_v2 = b"remote content v2 (newer)".to_vec();
        let hash_v2 = sha1_bytes(&content_v2);

        // --- Pass 1: daemon1 downloads v1, then its write echoes back through the watcher. ---
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("doc.txt"),
            remote("doc.txt", "id-doc", Some(hash_v1.as_str())),
        );
        let remote_contents = HashMap::from([(remote_path.clone(), content_v1.clone())]);
        let (client, _) =
            RecordingProtonClient::with_remote_contents(remote_files, remote_contents);
        let mut daemon1 = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon1");
        upsert_record(
            &daemon1.connection,
            &base_record("doc.txt", Some("id-doc"), base_hash.as_str()),
        )
        .expect("base record");

        daemon1.reconcile_blocking().expect("pass 1 reconcile");
        assert_eq!(
            fs::read(&local_path).expect("local after download"),
            content_v1,
            "pass 1 must download v1 into the local tree"
        );
        // The daemon's own download echoes through the watcher.
        daemon1
            .handle_fs_event(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
                    .add_path(local_path.clone()),
            )
            .expect("handle echo event");
        let record_after_echo = get_record(&daemon1.connection, Path::new("doc.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            record_after_echo.sync_status,
            SyncStatus::Synced,
            "the download echo must leave the record Synced, not Modified (issue #49)"
        );
        drop(daemon1); // release the lock guards so daemon2 can open the same root + db

        // --- Pass 2: the remote is edited to v2; a fresh daemon reconciles the same db + tree. ---
        let mut remote_files_v2 = HashMap::new();
        remote_files_v2.insert(
            PathBuf::from("doc.txt"),
            remote("doc.txt", "id-doc", Some(hash_v2.as_str())),
        );
        let remote_contents_v2 = HashMap::from([(remote_path.clone(), content_v2.clone())]);
        let (client2, operations2) =
            RecordingProtonClient::with_remote_contents(remote_files_v2, remote_contents_v2);
        let mut daemon2 = Daemon::with_client(test_config(directory.path(), &local_root), client2)
            .expect("daemon2");

        daemon2.reconcile_blocking().expect("pass 2 reconcile");

        // The newer remote edit wins: v2 is downloaded, and the stale local copy is never uploaded.
        assert_eq!(
            fs::read(&local_path).expect("local after pass 2"),
            content_v2,
            "the newer remote content must win — a reverting upload of the stale local copy is the bug"
        );
        let ops = operations2.lock().expect("operations lock");
        assert!(
            ops.contains(&RecordedOperation::Download {
                remote_path: remote_path.clone(),
                destination: local_path.clone(),
            }),
            "pass 2 must download the newer remote content: {ops:?}"
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, RecordedOperation::Upload { .. })),
            "pass 2 must not upload (revert) the stale local copy: {ops:?}"
        );
    }

    #[test]
    fn watcher_marks_original_modified_when_conflict_sidecar_is_removed() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"local resolved content").expect("local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let mut record = base_record("notes.txt", Some("remote-id"), local_hash.as_str());
        record.sync_status = SyncStatus::Conflict;
        upsert_record(&daemon.connection, &record).expect("conflict record");

        daemon
            .handle_fs_event(Event::new(EventKind::Remove(RemoveKind::File)).add_path(sidecar_path))
            .expect("handle sidecar remove event");

        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Modified);
        assert!(
            daemon.pending_changes.contains(Path::new("notes.txt")),
            "deleting a conflict sidecar should queue the original for resolution"
        );
    }

    #[test]
    fn deleting_conflict_sidecar_uploads_local_resolution() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("notes.txt");
        fs::write(&local_path, b"resolved local content").expect("local file");
        let local_hash = crate::index::compute_sha1(&local_path).expect("local hash");
        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("notes.txt"),
            remote("notes.txt", "remote-id", Some("remote-conflict-hash")),
        );
        let (client, operations) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let mut record = base_record("notes.txt", Some("remote-id"), local_hash.as_str());
        record.sync_status = SyncStatus::Conflict;
        upsert_record(&daemon.connection, &record).expect("conflict record");

        daemon
            .handle_fs_event(Event::new(EventKind::Remove(RemoveKind::File)).add_path(sidecar_path))
            .expect("handle sidecar remove event");
        daemon.reconcile_blocking().expect("reconcile");

        let operations = operations.lock().expect("operations lock");
        assert!(operations.contains(&RecordedOperation::Upload {
            local_path,
            remote_root: PathBuf::from("/Drive/RemoteFolder"),
            relative_path: PathBuf::from("notes.txt"),
        }));
        assert!(
            operations
                .iter()
                .all(|operation| !matches!(operation, RecordedOperation::Download { .. })),
            "local conflict resolution must not download over the user's chosen local copy"
        );
        drop(operations);
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sha1_hash, Some(local_hash));
        assert_eq!(record.sync_status, SyncStatus::Synced);
    }

    #[test]
    fn lock_guard_reuses_stale_lockfile() {
        let directory = tempdir().expect("tempdir");
        let lock_path = directory.path().join("daemon.lock");
        File::create(&lock_path).expect("stale lockfile");

        let guard = LockGuard::acquire(&lock_path).expect("acquire stale lockfile");

        drop(guard);
        assert!(
            lock_path.exists(),
            "a released guard must LEAVE the lockfile in place (#13): unlinking it lets a later \
             start lock a fresh inode independently"
        );
        LockGuard::acquire(&lock_path).expect("the leftover file must be reusable by a restart");
    }

    #[test]
    fn a_dropped_lock_guard_does_not_unlink_the_lockfile_another_guard_now_holds() {
        // #13, the flock-over-unlink race, made deterministic. Daemon A is inside its drop
        // window: it has released the flock but has not finished dropping. Daemon B opens the
        // SAME inode and wins the lock. If A's drop unlinks the path, a third start C creates a
        // FRESH inode whose flock is independent of B's — two live daemons.
        let directory = tempdir().expect("tempdir");
        let lock_path = directory.path().join("daemon.lock");

        let daemon_a = LockGuard::acquire(&lock_path).expect("A locks");
        daemon_a
            .file
            .unlock()
            .expect("A releases the flock (drop step 1)");
        let daemon_b = LockGuard::acquire(&lock_path).expect("B adopts the inode in A's window");
        drop(daemon_a); // drop step 2: must not remove the path

        assert!(
            lock_path.exists(),
            "A's drop must not unlink the lockfile B is holding"
        );
        assert!(
            LockGuard::acquire(&lock_path).is_err(),
            "a third daemon must contend on the same inode B holds and be refused"
        );
        drop(daemon_b);
    }

    #[test]
    fn lock_guard_rejects_second_live_instance() {
        let directory = tempdir().expect("tempdir");
        let lock_path = directory.path().join("daemon.lock");
        let guard = LockGuard::acquire(&lock_path).expect("first lock");

        let second = LockGuard::acquire(&lock_path);

        assert!(second.is_err(), "second live daemon must not acquire lock");
        drop(guard);
    }

    #[test]
    fn second_daemon_is_rejected_by_the_user_global_lock_even_across_roots() {
        // Two daemons for the SAME user but DIFFERENT roots: their per-root locks differ (both
        // would succeed), yet the second must still be refused because they share the proton-drive
        // CLI's SQLite cache/session store (#23). Only the global_lock_path is shared here, so it
        // is provably the discriminator — everything else (root, socket, per-root lock, db) differs.
        let session = tempdir().expect("tempdir");
        let shared_global = session.path().join("single-instance.lock");

        let make = |name: &str| -> (DaemonConfig, PathBuf) {
            let state_dir = session.path().join(format!("state-{name}"));
            let local_root = session.path().join(format!("root-{name}"));
            fs::create_dir_all(&state_dir).expect("state dir");
            fs::create_dir_all(&local_root).expect("local root");
            let mut config = test_config(&state_dir, &local_root);
            config.global_lock_path = shared_global.clone();
            (config, local_root)
        };

        let (config_a, _root_a) = make("a");
        let (config_b, _root_b) = make("b");

        let (client_a, _) = RecordingProtonClient::new(HashMap::new());
        let daemon_a = Daemon::with_client(config_a, client_a).expect("first daemon starts");

        let (client_b, _) = RecordingProtonClient::new(HashMap::new());
        let second = Daemon::with_client(config_b, client_b);
        assert!(
            second.is_err(),
            "a second daemon for this user must be rejected even with a different root/socket"
        );
        drop(daemon_a);
    }

    #[test]
    fn a_stale_global_lock_file_does_not_block_startup() {
        // A SIGKILLed daemon leaves its global lock FILE behind but the OS releases the flock. The
        // stale, unlocked file must not block a restart (mirrors `lock_guard_reuses_stale_lockfile`
        // for the per-root lock).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("root");
        fs::create_dir_all(&local_root).expect("local root");
        let mut config = test_config(directory.path(), &local_root);
        config.global_lock_path = directory.path().join("single-instance.lock");
        fs::write(&config.global_lock_path, b"").expect("stale global lock file");

        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(config, client);
        assert!(
            daemon.is_ok(),
            "a stale (unlocked) global lock file must not block startup"
        );
    }

    #[test]
    fn a_stopped_daemon_leaves_both_lockfiles_in_place_and_a_restart_reuses_them() {
        // #13 at the daemon level: BOTH guards (per-root and user-global) share one Drop, so
        // neither path may be unlinked on shutdown. Stable inodes are what make the next start
        // contend on the same flock instead of creating an independent one.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("root");
        fs::create_dir_all(&local_root).expect("local root");
        let config = test_config(directory.path(), &local_root);
        let lockfile_path = config.lockfile_path.clone();
        let global_lock_path = config.global_lock_path.clone();

        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(config, client).expect("first daemon starts");
        drop(daemon);

        assert!(lockfile_path.exists(), "per-root lockfile must persist");
        assert!(global_lock_path.exists(), "global lockfile must persist");

        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let restarted = Daemon::with_client(test_config(directory.path(), &local_root), client);
        assert!(
            restarted.is_ok(),
            "the leftover (unlocked) lockfiles must not block a restart"
        );
    }

    fn test_config(directory: &Path, local_root: &Path) -> DaemonConfig {
        DaemonConfig {
            local_root: local_root.to_path_buf(),
            remote_root: PathBuf::from("/Drive/RemoteFolder"),
            db_path: directory.join("sync_index.db"),
            socket_path: directory.join("daemon.sock"),
            lockfile_path: directory.join("daemon.lock"),
            global_lock_path: directory.join("single-instance.lock"),
            scan_interval: Duration::from_secs(300),
            proton_cli: PathBuf::from("proton-drive"),
            proton_timeout: Duration::from_secs(60),
            proton_list_attempts: 2,
            // Batching off by default in the general fixture so existing per-file download
            // expectations hold; the dedicated batching tests opt in explicitly.
            download_batch_size: 1,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            events_driven: false,
            events_full_scan_every: 20,
            // Default the general test fixture to guard-OFF so existing delete-propagation tests
            // keep asserting the delete actually happens. The dedicated gate tests below build a
            // config with the guard on explicitly.
            delete_approval_remote: false,
            delete_approval_local: false,
            // Warm start disabled in the shared fixtures: existing event tests drive the first
            // reconcile as a steady-state incremental/bootstrap pass. Dedicated warm-start tests
            // enable it explicitly. (`test_config` sets `events_driven: false`, so warm start would
            // be ineligible here anyway; being explicit keeps the intent obvious.)
            warm_start: WarmStartConfig {
                enabled: false,
                ..WarmStartConfig::default()
            },
        }
    }

    fn remote(path: &str, id: &str, sha1_hash: Option<&str>) -> RemoteFile {
        RemoteFile {
            path: PathBuf::from(path),
            id: id.to_owned(),
            name: Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_owned(),
            sha1_hash: sha1_hash.map(ToOwned::to_owned),
            downloadable: true,
        }
    }

    fn base_record(path: &str, proton_id: Option<&str>, sha1_hash: &str) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::File,
            file_size: 1,
            mtime: 1,
            sha1_hash: Some(sha1_hash.to_owned()),
            proton_id: proton_id.map(ToOwned::to_owned),
            sync_status: SyncStatus::Synced,
        }
    }

    fn sha1_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    // --- event-driven (incremental) reconcile tests ---------------------------------------

    /// A Proton client that counts full-tree walks vs targeted single-directory lists, so tests
    /// can prove an incremental pass never re-walks the whole tree.
    #[derive(Clone)]
    struct EventFakeClient {
        remote_entities: HashMap<PathBuf, RemoteEntity>,
        full_walks: Arc<AtomicUsize>,
        directory_lists: Arc<AtomicUsize>,
        failed_uploads: BTreeSet<PathBuf>,
        /// When `true`, the *next* upload fails and the flag clears itself — so a caller can make a
        /// single pass fail and have the retry succeed (used to prove a failed first pass retries
        /// as a first pass rather than idle-skipping the local scan).
        fail_next_upload: Arc<AtomicBool>,
        /// When `true`, every targeted single-directory listing fails the way `proton::collect_node`
        /// fails an incomplete listing (a node present remotely that this listing cannot describe).
        fail_directory_lists: bool,
    }

    impl EventFakeClient {
        fn new(remote_entities: HashMap<PathBuf, RemoteEntity>) -> Self {
            Self {
                remote_entities,
                full_walks: Arc::new(AtomicUsize::new(0)),
                directory_lists: Arc::new(AtomicUsize::new(0)),
                failed_uploads: BTreeSet::new(),
                fail_next_upload: Arc::new(AtomicBool::new(false)),
                fail_directory_lists: false,
            }
        }
    }

    impl ProtonClient for EventFakeClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            self.full_walks.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .remote_entities
                .iter()
                .filter_map(|(path, entity)| {
                    entity.as_file().cloned().map(|file| (path.clone(), file))
                })
                .collect())
        }

        fn list_entities_or_missing_root(
            &self,
            _remote_root: &Path,
        ) -> AppResult<RemoteListingStatus> {
            self.full_walks.fetch_add(1, Ordering::SeqCst);
            Ok(RemoteListingStatus::Found(self.remote_entities.clone()))
        }

        fn list_directory(
            &self,
            _remote_root: &Path,
            relative_directory: &Path,
        ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            self.directory_lists.fetch_add(1, Ordering::SeqCst);
            if self.fail_directory_lists {
                return Err(boxed_error(
                    "remote listing is incomplete: a node under the remote root has an \
                     undecodable name/path",
                ));
            }
            Ok(self
                .remote_entities
                .iter()
                .filter(|(path, _)| path.parent() == Some(relative_directory))
                .map(|(path, entity)| (path.clone(), entity.clone()))
                .collect())
        }

        fn ensure_root_directory(&self, _remote_root: &Path) -> AppResult<()> {
            Ok(())
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Ok(())
        }

        fn upload(
            &self,
            _local_path: &Path,
            _remote_root: &Path,
            relative_path: &Path,
        ) -> AppResult<()> {
            if self.fail_next_upload.swap(false, Ordering::SeqCst)
                || self.failed_uploads.contains(relative_path)
            {
                return Err(boxed_error(format!(
                    "upload failed for {}",
                    relative_path.display()
                )));
            }
            Ok(())
        }

        fn download(&self, _remote_path: &Path, destination: &Path) -> AppResult<()> {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(destination, b"downloaded")?;
            Ok(())
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Ok(())
        }

        fn rename_or_move(&self, _r: &Path, _o: &Path, _n: &Path) -> AppResult<()> {
            Ok(())
        }
    }

    /// A scriptable [`EventSource`]: `events_since` replays a queue of pages (an empty queue means
    /// "no changes"), `latest_cursor` returns a fixed cursor. Optionally fails to exercise the
    /// events-error fallback.
    struct FakeEventSource {
        pages: Mutex<Vec<VolumeEventPage>>,
        latest: String,
        fail_since: bool,
    }

    impl FakeEventSource {
        fn new(latest: &str) -> Self {
            Self {
                pages: Mutex::new(Vec::new()),
                latest: latest.to_owned(),
                fail_since: false,
            }
        }

        fn with_pages(latest: &str, pages: Vec<VolumeEventPage>) -> Self {
            Self {
                pages: Mutex::new(pages),
                latest: latest.to_owned(),
                fail_since: false,
            }
        }

        fn failing() -> Self {
            Self {
                pages: Mutex::new(Vec::new()),
                latest: "c0".to_owned(),
                fail_since: true,
            }
        }
    }

    impl EventSource for FakeEventSource {
        fn latest_cursor(&self, _volume_id: &str) -> AppResult<String> {
            Ok(self.latest.clone())
        }

        fn events_since(&self, _volume_id: &str, cursor: &str) -> AppResult<VolumeEventPage> {
            if self.fail_since {
                return Err(boxed_error("events fetch boom"));
            }
            let mut pages = self.pages.lock().expect("pages lock");
            if pages.is_empty() {
                Ok(VolumeEventPage {
                    latest_event_id: cursor.to_owned(),
                    more: false,
                    refresh: false,
                    changes: Vec::new(),
                })
            } else {
                Ok(pages.remove(0))
            }
        }
    }

    fn event_config(directory: &Path, local_root: &Path) -> DaemonConfig {
        DaemonConfig {
            events_driven: true,
            events_full_scan_every: 20,
            ..test_config(directory, local_root)
        }
    }

    #[test]
    fn event_source_is_reacquired_when_the_keyring_becomes_readable() {
        // Boot race: the daemon started with the keyring locked, so `event_source` is None and it
        // is stuck on full-tree snapshots. Once the keyring is unlocked, the next pass must resume
        // event-driven detection without a restart.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _ops) = RecordingProtonClient::new(HashMap::new());
        // `with_client` injects event_source = None, mirroring a keyring-locked startup.
        let mut daemon = Daemon::with_client(event_config(directory.path(), &local_root), client)
            .expect("daemon");
        assert!(
            daemon.event_source.is_none(),
            "precondition: no event source at a keyring-locked startup"
        );
        // Simulate the degraded window: the snapshot passes that ran while degraded reset the
        // startup-snapshot floor to 0, so without a reseed the next pass would go incremental.
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        // Keyring now readable: the factory yields a source.
        daemon.event_source_factory = Box::new(|| Some(Box::new(FakeEventSource::new("cursor-0"))));
        daemon.reacquire_event_source_if_needed();
        assert!(
            daemon.event_source.is_some(),
            "event-driven detection should resume once the session is readable"
        );
        // Mid-life reacquisition (not the first pass) must reseed the resync floor so this pass
        // full-scans (capturing a fresh cursor) instead of streaming against the stale persisted
        // cursor — steady-state incremental has no cursor-age gate to catch that staleness, so the
        // reseed is what protects it. (On the *first* pass the reseed is skipped; `first_reconcile`
        // applies its own cursor-age gate there instead.)
        assert_eq!(
            daemon.incremental_passes_since_full_scan,
            effective_full_scan_every(daemon.config.events_full_scan_every),
            "mid-life reacquisition must force a snapshot floor"
        );

        // Idempotent: a working source is never rebuilt (the factory must not be called again).
        daemon.event_source_factory = Box::new(|| panic!("must not rebuild an existing source"));
        daemon.reacquire_event_source_if_needed();
    }

    #[test]
    fn event_source_is_not_reacquired_when_events_driven_is_off() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _ops) = RecordingProtonClient::new(HashMap::new());
        // events_driven defaults to false in `test_config`.
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        daemon.event_source_factory =
            Box::new(|| panic!("must not build a source when the feature is off"));
        daemon.reacquire_event_source_if_needed();
        assert!(daemon.event_source.is_none());
    }

    fn remote_dir(path: &str, id: &str) -> RemoteEntity {
        RemoteEntity::Directory(RemoteDirectory {
            path: PathBuf::from(path),
            id: Some(id.to_owned()),
            name: Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_owned(),
        })
    }

    fn remote_file_entity(path: &str, id: &str, sha1_hash: &str) -> RemoteEntity {
        RemoteEntity::File(remote(path, id, Some(sha1_hash)))
    }

    fn change(
        kind: RemoteChangeKind,
        node_id: &str,
        parent_id: Option<&str>,
        trashed: bool,
    ) -> RemoteChange {
        RemoteChange {
            kind,
            node_id: node_id.to_owned(),
            parent_id: parent_id.map(ToOwned::to_owned),
            trashed,
            shared: false,
            event_id: format!("evt-{node_id}"),
        }
    }

    fn one_page(latest: &str, changes: Vec<RemoteChange>) -> VolumeEventPage {
        VolumeEventPage {
            latest_event_id: latest.to_owned(),
            more: false,
            refresh: false,
            changes,
        }
    }

    #[test]
    fn bootstrap_captures_cursor_then_idle_incremental_does_no_full_walk() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let remote_entities = HashMap::from([(
            PathBuf::from("a.txt"),
            remote_file_entity("a.txt", "vol~na", "h"),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");

        // First pass: no baseline → bootstrap snapshot, which captures and stores the cursor.
        daemon.reconcile_blocking().expect("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "bootstrap performs one full walk"
        );
        let cursor = load_event_cursor(&daemon.connection, "vol")
            .expect("load cursor")
            .expect("cursor persisted by bootstrap");
        assert_eq!(cursor.last_event_id, "cursor-0");
        assert_eq!(daemon.incremental_passes_since_full_scan, 0);

        // Second pass: stored cursor + derivable volume + no changes → idle incremental, no walk.
        daemon
            .reconcile_blocking()
            .expect("idle incremental reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "an idle incremental pass must not re-walk the whole tree"
        );
        assert_eq!(daemon.incremental_passes_since_full_scan, 1);
    }

    #[test]
    fn single_remote_change_plans_one_action_with_zero_full_walks() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("dir")).expect("local dir");
        let old = sha1_bytes(b"old");
        fs::write(local_root.join("dir/a.txt"), b"old").expect("local file");

        // Remote reflects the *new* content; the fake resolves it via a targeted parent list.
        let remote_entities = HashMap::from([
            (PathBuf::from("dir"), remote_dir("dir", "vol~ndir")),
            (
                PathBuf::from("dir/a.txt"),
                remote_file_entity("dir/a.txt", "vol~na", &sha1_bytes(b"new")),
            ),
        ]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let directory_lists = Arc::clone(&client.directory_lists);
        let page = one_page(
            "cursor-1",
            vec![change(RemoteChangeKind::Updated, "na", Some("ndir"), false)],
        );
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![page],
            ))),
        )
        .expect("daemon");

        // Seed a synced baseline + stored cursor so the very first pass is incremental.
        upsert_record(
            &daemon.connection,
            &directory_record("dir", Some("vol~ndir")),
        )
        .expect("seed dir record");
        upsert_record(
            &daemon.connection,
            &base_record("dir/a.txt", Some("vol~na"), old.as_str()),
        )
        .expect("seed file record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams.
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        daemon.reconcile_blocking().expect("incremental reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "a single nested change must be planned without any full-tree walk"
        );
        assert!(
            directory_lists.load(Ordering::SeqCst) >= 1,
            "the parent was listed"
        );
        // The change was applied (download overwrote the local file) and the cursor advanced.
        assert_eq!(
            fs::read(local_root.join("dir/a.txt")).expect("read local"),
            b"downloaded"
        );
        let cursor = load_event_cursor(&daemon.connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-1",
            "cursor advanced after success"
        );
    }

    #[test]
    fn mid_plan_failure_leaves_index_and_cursor_unadvanced() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        // A synced baseline file (so the volume is derivable) plus a new local file that will fail
        // to upload.
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");
        fs::write(local_root.join("new.txt"), b"new").expect("new file");

        let mut client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]));
        client.failed_uploads = BTreeSet::from([PathBuf::from("new.txt")]);
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams.
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;
        // Make the pass non-idle so it scans + plans the failing upload.
        daemon.pending_changes.insert(PathBuf::from("new.txt"));

        let result = daemon.reconcile_blocking();
        assert!(result.is_err(), "a failed upload must fail the reconcile");

        // Neither the index nor the cursor advanced; the events replay next pass.
        assert!(
            get_record(&daemon.connection, Path::new("new.txt"))
                .expect("get record")
                .is_none(),
            "the failed file must not be recorded (no partial commit)"
        );
        let cursor = load_event_cursor(&daemon.connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "cursor must NOT advance on failure"
        );
        assert_eq!(daemon.incremental_passes_since_full_scan, 0);
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "the incremental path took no full walk"
        );
    }

    #[test]
    fn excluded_path_in_delta_is_never_downloaded_or_recorded() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");

        // A created remote node under an excluded path; resolvable via the root listing.
        let remote_entities = HashMap::from([
            (
                PathBuf::from("keep.txt"),
                remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
            ),
            (
                PathBuf::from("secret/x.txt"),
                remote_file_entity("secret/x.txt", "vol~ns", "h"),
            ),
            (PathBuf::from("secret"), remote_dir("secret", "vol~nsec")),
        ]);
        let client = EventFakeClient::new(remote_entities);
        let page = one_page(
            "cursor-1",
            vec![change(RemoteChangeKind::Created, "ns", Some("nsec"), false)],
        );
        let mut config = event_config(directory.path(), &local_root);
        config.exclude_patterns = vec!["secret/**".to_owned()];
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![page],
            ))),
        )
        .expect("daemon");
        // Rebuild scan options so the exclude pattern actually applies (the daemon caches them).
        daemon.scan_options = scan_options_from_config(&daemon.config).expect("scan options");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        // The excluded parent is not indexed, forcing resolution via the root listing.
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams.
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        daemon.reconcile_blocking().expect("incremental reconcile");

        assert!(
            !local_root.join("secret/x.txt").exists(),
            "an excluded remote file must never be downloaded"
        );
        assert!(
            get_record(&daemon.connection, Path::new("secret/x.txt"))
                .expect("get record")
                .is_none(),
            "an excluded path must never be recorded"
        );
    }

    #[test]
    fn server_refresh_falls_back_to_a_full_snapshot() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");

        let remote_entities = HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let refresh_page = VolumeEventPage {
            latest_event_id: "cursor-1".to_owned(),
            more: false,
            refresh: true,
            changes: Vec::new(),
        };
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![refresh_page],
            ))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams
        // (and then falls back to a snapshot for the reason under test, not the startup floor).
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect("reconcile falls back cleanly");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "a server refresh signal must trigger a full-tree snapshot"
        );
    }

    #[test]
    fn events_fetch_error_falls_back_to_a_full_snapshot() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");

        let remote_entities = HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::failing())),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams
        // (and then falls back to a snapshot for the reason under test, not the startup floor).
        daemon.incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect("reconcile falls back cleanly");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "an events fetch error must trigger a full-tree snapshot"
        );
    }

    #[test]
    fn an_incomplete_directory_listing_falls_back_instead_of_planning_a_deletion() {
        // `list_directory` can now fail (#59's incomplete-listing guard). That error must travel
        // resolver -> `Reconstruction::FallbackToSnapshot` -> a full-tree snapshot, and must never
        // become a plan: the whole point of failing the listing is that a node missing from it is
        // indistinguishable from a deleted one. The pass-scoped listing memo (#70) sits on that
        // path — it caches the `Rc` only after a successful call, so a failure is never memoized.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");

        let remote_entities = HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]);
        let mut client = EventFakeClient::new(remote_entities);
        client.fail_directory_lists = true;
        let full_walks = Arc::clone(&client.full_walks);
        let directory_lists = Arc::clone(&client.directory_lists);
        // An update to a node whose parent is not indexed sends the resolver to the root listing.
        let page = one_page(
            "cursor-1",
            vec![change(RemoteChangeKind::Updated, "nk", None, false)],
        );
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![page],
            ))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.incremental_passes_since_full_scan = 0;
        daemon.is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect("an incomplete listing falls back cleanly");

        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            1,
            "the failed listing must abandon the incremental pass, not be retried"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "an incomplete targeted listing must fall back to a full-tree snapshot"
        );
        assert!(
            local_root.join("keep.txt").exists(),
            "an incomplete listing must never delete local content"
        );
        assert!(
            get_record(&daemon.connection, Path::new("keep.txt"))
                .expect("index lookup")
                .is_some(),
            "an incomplete listing must not purge the index record either"
        );
    }

    #[test]
    fn periodic_safety_resync_forces_a_full_snapshot() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");

        let remote_entities = HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let mut config = event_config(directory.path(), &local_root);
        config.events_full_scan_every = 1;
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Steady state (not the first pass), already at the resync threshold → this pass must
        // snapshot via the periodic-resync counter, not go incremental.
        daemon.is_first_reconcile = false;
        daemon.incremental_passes_since_full_scan = 1;

        daemon
            .reconcile_blocking()
            .expect("forced resync reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "reaching events_full_scan_every must force a full-tree snapshot"
        );
        assert_eq!(
            daemon.incremental_passes_since_full_scan, 0,
            "the resync counter resets after a full snapshot"
        );
    }

    #[test]
    fn disabled_periodic_resync_never_full_scans_after_the_startup_snapshot() {
        // The shipped default: `events_full_scan_every == 0` disables the periodic safety resync,
        // so after the one mandatory startup snapshot every subsequent pass stays event-driven no
        // matter how many passes elapse. Regression for the "daemon keeps doing full syncs" report.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let remote_entities = HashMap::from([(
            PathBuf::from("a.txt"),
            remote_file_entity("a.txt", "vol~na", "h"),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let mut config = event_config(directory.path(), &local_root);
        config.events_full_scan_every = 0; // periodic resync disabled (the shipped default)
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");

        // The first pass after boot still full-scans exactly once even with the resync disabled:
        // `first_reconcile` bootstraps (there is no stored cursor/baseline to warm-start from here),
        // then hands off to the event-driven steady state, which stays incremental forever.
        daemon.reconcile_blocking().expect("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the first reconcile after startup must still full-scan"
        );
        assert_eq!(daemon.incremental_passes_since_full_scan, 0);

        // Far more idle passes than the *old* default (20) would have resynced at — none may walk.
        for _ in 0..50 {
            daemon
                .reconcile_blocking()
                .expect("idle incremental reconcile");
        }
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "with the periodic resync disabled, only the startup snapshot ever walks the whole tree"
        );
        assert_eq!(
            daemon.incremental_passes_since_full_scan, 50,
            "the pass counter keeps climbing without ever tripping a resync"
        );
    }

    /// Bootstraps an event-driven daemon into the steady state the idle gate lives in: `local/a.txt`
    /// already matching remote `a.txt` (so the startup snapshot only adopts it), which leaves a base
    /// record carrying the composed `proton_id` the volume is derived from plus a stored cursor to
    /// replay from. Returns the daemon and its full-walk counter, both past the one startup walk.
    fn steady_state_event_daemon(
        directory: &tempfile::TempDir,
    ) -> (Daemon<EventFakeClient>, Arc<AtomicUsize>) {
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("a.txt"), b"a").expect("local file");
        let remote_entities = HashMap::from([(
            PathBuf::from("a.txt"),
            remote_file_entity("a.txt", "vol~na", sha1_bytes(b"a").as_str()),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        daemon.reconcile_blocking().expect("startup snapshot");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "precondition: the first pass after boot full-walks exactly once"
        );
        (daemon, full_walks)
    }

    #[test]
    fn an_empty_directory_create_is_mirrored_by_the_next_event_driven_pass() {
        // #51: `mkdir photos` (empty) emits ONLY a directory event, which the watcher handler
        // dropped — so `pending_changes` stayed empty, every events-mode pass took the idle
        // fast-path, and the folder never reached Proton Drive. Nothing healed it either: the
        // periodic resync is off by default (PR #138) and a restart warm-starts from the cursor
        // rather than bootstrapping (PR #160).
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks) = steady_state_event_daemon(&directory);
        let created = daemon.config.local_root.join("photos");
        fs::create_dir(&created).expect("empty local directory");

        daemon
            .handle_fs_event(Event::new(EventKind::Create(CreateKind::Folder)).add_path(created))
            .expect("handle directory create event");
        assert!(
            daemon.pending_changes.contains(Path::new("photos")),
            "a directory create must queue the path so the pass is not idle"
        );

        daemon
            .reconcile_blocking()
            .expect("incremental reconcile after the directory create");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the directory must be mirrored by the incremental pass, not by a full-tree walk"
        );
        let summary = daemon
            .last_successful_sync_summary
            .as_ref()
            .expect("successful pass summary");
        assert_eq!(
            summary.remote_directories_created, 1,
            "the empty directory must be created remotely"
        );
        let record = get_record(&daemon.connection, Path::new("photos"))
            .expect("index lookup")
            .expect("the created directory must be recorded");
        assert_eq!(record.entity_kind, EntityKind::Directory);
    }

    #[test]
    fn a_watcher_error_forces_the_next_event_driven_pass_to_scan_locally() {
        // #51: an inotify queue overflow drops events, so `pending_changes` under-reports. A local
        // edit produces no remote event either, so an idle-skipping pass strands it — and with the
        // periodic resync off by default there is no later full walk to re-derive it from.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks) = steady_state_event_daemon(&directory);
        let edited = daemon.config.local_root.join("a.txt");
        let contents = b"edited while the watcher was deaf";
        fs::write(&edited, contents).expect("local edit");
        // Deliberately no `handle_fs_event`: this is the event the overflow dropped.
        assert!(daemon.pending_changes.is_empty());

        daemon.note_watch_error(&notify::Error::generic("inotify queue overflow"));
        daemon
            .reconcile_blocking()
            .expect("incremental reconcile after the watcher error");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the forced rescan is local-only; the remote stays on the event stream"
        );
        let summary = daemon
            .last_successful_sync_summary
            .as_ref()
            .expect("successful pass summary");
        assert_eq!(
            summary.uploads, 1,
            "the edit whose watcher event was lost must still upload"
        );
        let record = get_record(&daemon.connection, Path::new("a.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sha1_hash, Some(sha1_bytes(contents)));
        assert!(
            !daemon.force_local_rescan,
            "a successful pass clears the pending rescan"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_degraded_session_does_not_full_walk_on_the_events_poll_cadence() {
        // #50: with `events_driven` on but no usable CLI session, every pass is a full-tree walk —
        // and the poll arm, gated only on `events_driven`, ran one every EVENTS_POLL_INTERVAL
        // (30s) forever: 10x the configured scan interval, in the one configuration where the fast
        // cadence buys nothing.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let client = EventFakeClient::new(HashMap::new());
        let full_walks = Arc::clone(&client.full_walks);
        let config = DaemonConfig {
            // Far longer than the window below, so inside it only the startup reconcile and the
            // events poll can fire.
            scan_interval: Duration::from_secs(3600),
            ..event_config(directory.path(), &local_root)
        };
        // `with_client` injects no event source — a keyring-locked / headless startup.
        let mut daemon = Daemon::with_client(config, client).expect("daemon");
        // The real factory reads this machine's keyring; pin it degraded so the test measures the
        // degraded cadence rather than the developer's desktop session.
        daemon.event_source_factory = Box::new(|| None);
        daemon.events_poll_interval = Duration::from_millis(20);

        let handle = tokio::spawn(daemon.run());
        // Wait for the startup reconcile, then leave the loop running for ~30 poll ticks.
        for _ in 0..100 {
            if full_walks.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(600)).await;
        handle.abort();
        let _ = handle.await;

        let walks = full_walks.load(Ordering::SeqCst);
        assert_eq!(
            walks, 1,
            "a degraded session must not walk the whole remote tree on the event-poll cadence: \
             {walks} full-tree walks in ~30 ticks"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_degraded_session_still_reconciles_on_the_scan_interval() {
        // The other half of #50's gate: degraded must mean "snapshot cadence", not "no cadence".
        // Each of these passes also re-attempts the session (`reacquire_event_source_if_needed`),
        // so keyring recovery still happens without a restart — just at the scan interval.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let client = EventFakeClient::new(HashMap::new());
        let full_walks = Arc::clone(&client.full_walks);
        let config = DaemonConfig {
            scan_interval: Duration::from_millis(150),
            ..event_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client(config, client).expect("daemon");
        daemon.event_source_factory = Box::new(|| None);
        daemon.events_poll_interval = Duration::from_millis(15);

        let handle = tokio::spawn(daemon.run());
        tokio::time::sleep(Duration::from_millis(900)).await;
        handle.abort();
        let _ = handle.await;

        let walks = full_walks.load(Ordering::SeqCst);
        assert!(
            (2..=20).contains(&walks),
            "a degraded daemon must keep reconciling on its ~150ms scan interval (a handful of \
             walks) and not on the ~15ms event poll (dozens): saw {walks}"
        );
    }

    #[test]
    fn a_degraded_session_reports_that_cause_once_without_masking_the_scope_causes() {
        // "Keeps doing full syncs" is told apart only by the log line, so the causes must form one
        // message family with one reason each: no session (invisible to `resolve_event_scope`,
        // which is never reached without a source) vs no volume / no cursor. Neither may
        // double-report the other's condition, and neither may be silent.
        // Driven through `reconcile_blocking` (not the reporter directly) so the wiring is part of
        // what is asserted.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        // One remote file whose digest matches what the fake's download writes, so the tree
        // converges after the first pass and later passes are about the reporting, not transfers.
        let remote_entities = HashMap::from([(
            PathBuf::from("a.txt"),
            remote_file_entity("a.txt", "vol~na", sha1_bytes(b"downloaded").as_str()),
        )]);
        // `with_client...(None)` = no event source: a keyring-locked / headless startup.
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            EventFakeClient::new(remote_entities),
            None,
        )
        .expect("daemon");
        // `reacquire_event_source_if_needed` runs the real factory at the start of every pass and
        // would read this machine's keyring; pin it degraded so the test measures the degraded
        // daemon and not the developer's desktop session.
        daemon.event_source_factory = Box::new(|| None);

        daemon.reconcile_blocking().expect("degraded first pass");
        let session_cause = daemon.event_scope_declined.clone();
        let reported = session_cause
            .as_deref()
            .expect("a degraded pass must report why it is full-walking");
        assert!(
            reported.contains("no usable proton-drive CLI session")
                && reported.contains("scan interval"),
            "the session cause must name itself and the cadence it implies: {reported}"
        );

        // Same cause next pass → the recorded reason is unchanged, so it is logged once.
        daemon.reconcile_blocking().expect("degraded second pass");
        assert_eq!(daemon.event_scope_declined, session_cause);

        // Session recovers: the session reporter goes quiet and the *next* cause — the scope, which
        // no degraded pass could anchor a cursor for — is reported in its own words, not masked.
        daemon.event_source = Some(Box::new(FakeEventSource::new("cursor-0")));
        daemon.reconcile_blocking().expect("recovered pass");
        let scope_cause = daemon.event_scope_declined.clone();
        assert!(
            scope_cause
                .as_deref()
                .is_some_and(|reason| reason.contains("no stored event cursor")),
            "a scope decline must replace the session cause, not compete with it: {scope_cause:?}"
        );

        // That pass anchored a cursor, so the scope now resolves: the record clears, and a later
        // regression of either cause reports again instead of being swallowed by a stale latch.
        daemon.reconcile_blocking().expect("incremental pass");
        assert_eq!(daemon.event_scope_declined, None);
    }

    #[test]
    fn changes_sharing_a_parent_take_one_targeted_listing_per_pass() {
        // #70: N events in one folder must collapse to ONE `list_directory` subprocess, and the
        // memo must die with the pass (resolution reads *current* remote state, so reusing a
        // listing across passes would plan against a stale folder).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("dir")).expect("local dir");
        let digests: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                let path = local_root.join("dir").join(format!("{name}.txt"));
                fs::write(&path, name.as_bytes()).expect("local file");
                sha1_bytes(name.as_bytes())
            })
            .collect();

        // Remote matches local everywhere, so the pass plans no actions and every
        // `list_directory` call counted below comes from targeted event resolution.
        let mut remote_entities =
            HashMap::from([(PathBuf::from("dir"), remote_dir("dir", "vol~ndir"))]);
        for (name, digest) in ["a", "b", "c"].iter().zip(&digests) {
            remote_entities.insert(
                PathBuf::from(format!("dir/{name}.txt")),
                remote_file_entity(&format!("dir/{name}.txt"), &format!("vol~n{name}"), digest),
            );
        }
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let directory_lists = Arc::clone(&client.directory_lists);
        let first = one_page(
            "cursor-1",
            vec![
                change(RemoteChangeKind::Updated, "na", Some("ndir"), false),
                change(RemoteChangeKind::Updated, "nb", Some("ndir"), false),
            ],
        );
        let second = one_page(
            "cursor-2",
            vec![change(RemoteChangeKind::Updated, "nc", Some("ndir"), false)],
        );
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-2",
                vec![first, second],
            ))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.connection,
            &directory_record("dir", Some("vol~ndir")),
        )
        .expect("seed dir record");
        for (name, digest) in ["a", "b", "c"].iter().zip(&digests) {
            upsert_record(
                &daemon.connection,
                &base_record(
                    &format!("dir/{name}.txt"),
                    Some(&format!("vol~n{name}")),
                    digest,
                ),
            )
            .expect("seed file record");
        }
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.incremental_passes_since_full_scan = 0;
        daemon.is_first_reconcile = false;

        daemon.reconcile_blocking().expect("first incremental pass");
        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            1,
            "two events in one folder must share a single targeted listing"
        );

        daemon
            .reconcile_blocking()
            .expect("second incremental pass");
        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            2,
            "the memo must not survive the pass: a later pass re-lists the same parent"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "no fallback walk may inflate or deflate the counts"
        );
    }

    #[test]
    fn the_gate_derives_the_volume_from_the_stored_cursor_when_the_index_has_none() {
        // #32: an all-Proton-native remote records no `proton_id`, so the volume cannot come from
        // the base index — but the bootstrap still anchored a cursor, whose scope id *is* the
        // volume. Without this the daemon full-walks every pass forever.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let remote_entities = HashMap::from([(
            PathBuf::from("new.txt"),
            remote_file_entity("new.txt", "vol~nn", "h"),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let page = one_page(
            "cursor-1",
            vec![change(RemoteChangeKind::Created, "nn", None, false)],
        );
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![page],
            ))),
        )
        .expect("daemon");

        // No base records at all — only the cursor the bootstrap stored.
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.incremental_passes_since_full_scan = 0;
        daemon.is_first_reconcile = false;

        assert!(
            daemon.should_try_incremental(&load_index(&daemon.connection).expect("load index")),
            "a stored cursor alone must be enough to engage the event-driven gate"
        );
        daemon.reconcile_blocking().expect("incremental reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "the pass must stream from the cursor instead of walking the whole tree"
        );
        assert!(
            local_root.join("new.txt").exists(),
            "the created remote file must be downloaded by the incremental pass"
        );
        assert!(
            daemon.event_scope_declined.is_none(),
            "nothing was declined, so nothing should be reported"
        );
    }

    #[test]
    fn a_gate_with_no_cursor_and_no_volume_reports_why_once() {
        // "Keeps doing full syncs" is diagnosed by the log line, so the decline must be stated —
        // and stated once, not every pass.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            EventFakeClient::new(HashMap::new()),
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        daemon.is_first_reconcile = false;

        let base = HashMap::new();
        assert!(!daemon.should_try_incremental(&base));
        let reported = daemon.event_scope_declined.clone();
        assert!(
            reported
                .as_deref()
                .is_some_and(|reason| reason.contains("volume")),
            "the decline must name the volume as the missing input: {reported:?}"
        );
        // Same cause next pass → recorded reason is unchanged, so it is logged once.
        assert!(!daemon.should_try_incremental(&base));
        assert_eq!(daemon.event_scope_declined, reported);
    }

    /// Seeds `local/keep.txt` edited to `edited`, a baseline record + remote entity at `old`, and a
    /// warm-start-enabled event daemon. Returns the daemon, the `full_walks` counter, and the two
    /// digests, so each first-pass test only has to set the cursor freshness / floor it exercises.
    fn warm_start_fixture(
        directory: &tempfile::TempDir,
    ) -> (
        Daemon<EventFakeClient>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        String,
        String,
    ) {
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let old = sha1_bytes(b"keep");
        let edited = sha1_bytes(b"edited while the daemon was down");
        fs::write(
            local_root.join("keep.txt"),
            b"edited while the daemon was down",
        )
        .expect("local file");
        let remote_entities = HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", old.as_str()),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let directory_lists = Arc::clone(&client.directory_lists);
        let mut config = event_config(directory.path(), &local_root);
        // The shared fixtures default warm start OFF; the warm-start tests opt in explicitly.
        config.warm_start.enabled = true;
        let daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), old.as_str()),
        )
        .expect("seed keep record");
        (daemon, full_walks, directory_lists, old, edited)
    }

    #[test]
    fn first_reconcile_warm_starts_on_a_fresh_cursor() {
        // The common restart: a recent, still-valid cursor. The first pass replays the remote from
        // the cursor (no O(folders) walk, no targeted directory lists on an empty delta) while its
        // forced local stat-walk still catches a file edited while the daemon was down.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks, directory_lists, _old, edited) =
            warm_start_fixture(&directory);
        store_event_cursor(
            &daemon.connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        daemon.reconcile_blocking().expect("warm-start reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "a warm start must not walk the whole remote tree"
        );
        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            0,
            "an empty delta resolves no directories either"
        );
        let record = get_record(&daemon.connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(edited),
            "the forced local scan still catches and uploads the offline edit"
        );
        assert_eq!(
            daemon.warm_starts_since_full_walk, 1,
            "a completed warm start advances the across-restart counter"
        );
        assert_eq!(
            load_warm_start_count(&daemon.connection).expect("load count"),
            1,
            "and persists it so the floor survives a restart"
        );
    }

    #[test]
    fn first_reconcile_bootstraps_when_the_cursor_is_stale() {
        // A boot after long downtime: the persisted cursor may be past the server's event-retention
        // window. The cursor-age gate rejects it, so the first pass full-walks (safe) instead of
        // warm-starting against a cursor whose delta the server might silently truncate. The
        // offline edit is still caught — by the bootstrap's own full local scan.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks, _lists, _old, edited) = warm_start_fixture(&directory);
        // updated_at = 1 (1970): far past the 7-day default age gate.
        store_event_cursor(&daemon.connection, "vol", "cursor-0", 1).expect("seed stale cursor");

        daemon.reconcile_blocking().expect("startup reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "a stale cursor must force a full walk, not a warm start"
        );
        let record = get_record(&daemon.connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(edited),
            "the offline edit still syncs on the bootstrap's local scan"
        );
        assert_eq!(
            daemon.warm_starts_since_full_walk, 0,
            "no warm start happened, so the counter stays at zero"
        );
    }

    #[test]
    fn a_failed_first_pass_retries_as_a_first_pass_and_still_scans_locally() {
        // Regression: `is_first_reconcile` clears only on success. If a warm start's upload fails,
        // the next pass must retry as a *first* pass (forcing the local scan) rather than dropping
        // into the steady-state idle fast-path — which, with an empty delta and an empty
        // `pending_changes` on a fresh boot, would skip the scan and strand the offline edit.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks, _lists, old, edited) = warm_start_fixture(&directory);
        let fail_next_upload = Arc::clone(&daemon.proton.fail_next_upload);
        store_event_cursor(
            &daemon.connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        // Pass 1: the warm start scans, plans the upload, and the upload fails → the pass errors.
        fail_next_upload.store(true, Ordering::SeqCst);
        assert!(
            daemon.reconcile_blocking().is_err(),
            "the first pass must fail when its upload fails"
        );
        assert!(
            daemon.is_first_reconcile,
            "a failed first pass must stay a first pass"
        );
        let record = get_record(&daemon.connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(old),
            "the failed pass committed nothing; the record keeps the old digest"
        );

        // Pass 2: still a first pass → warm-starts again, forcing the local scan; the upload now
        // succeeds and the edit lands. It never bootstrapped and never idle-skipped.
        daemon.reconcile_blocking().expect("retry reconcile");
        assert!(
            !daemon.is_first_reconcile,
            "a successful pass finally clears the first-pass flag"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "neither pass walked the whole remote tree"
        );
        let record = get_record(&daemon.connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(edited),
            "the retry's forced local scan caught and uploaded the offline edit"
        );
    }

    #[test]
    fn the_warm_start_floor_forces_a_bootstrap_and_resets_the_counter() {
        // Even with a fresh cursor, once `warm_starts_since_full_walk` reaches the configured floor
        // the first pass full-walks (the self-healing cadence across reboots); the walk resets the
        // persisted counter so the cadence restarts.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks, _lists, _old, _edited) = warm_start_fixture(&directory);
        daemon.config.warm_start.full_walk_every = 3;
        store_event_cursor(
            &daemon.connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");
        // The machine has already warm-started up to the floor across prior boots.
        daemon.warm_starts_since_full_walk = 3;
        store_warm_start_count(&daemon.connection, 3).expect("seed count");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "reaching the warm-start floor forces a full walk"
        );
        assert_eq!(
            daemon.warm_starts_since_full_walk, 0,
            "a full walk resets the warm-start floor"
        );
        assert_eq!(
            load_warm_start_count(&daemon.connection).expect("load"),
            0,
            "and persists the reset"
        );
    }

    #[test]
    fn force_full_walk_config_bootstraps_the_first_pass_despite_a_fresh_cursor() {
        // The `--full-walk` startup flag: a full walk this boot even when a warm start would be
        // eligible.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks, _lists, _old, _edited) = warm_start_fixture(&directory);
        daemon.config.warm_start.force_full_walk = true;
        store_event_cursor(
            &daemon.connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        daemon.reconcile_blocking().expect("reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "--full-walk must force a bootstrap on the first pass"
        );
    }

    #[test]
    fn a_resync_request_forces_one_full_walk_then_clears() {
        // `proton-sync resync` latches `force_full_walk`; the next pass consumes it with a full
        // walk, and the pass after that returns to the incremental steady state. Uses an
        // already-converged tree (local == remote == baseline) so every pass is a clean no-op and
        // only the walk count varies.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("local file");
        let remote_entities = HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]);
        let client = EventFakeClient::new(remote_entities);
        let full_walks = Arc::clone(&client.full_walks);
        let mut config = event_config(directory.path(), &local_root);
        config.warm_start.enabled = true;
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(
            &daemon.connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        // First pass warm-starts (no walk).
        daemon.reconcile_blocking().expect("warm-start first pass");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "the first pass warm-started, no walk yet"
        );

        // A resync request → the next pass full-walks.
        daemon.shared.force_full_walk.store(true, Ordering::SeqCst);
        daemon.reconcile_blocking().expect("forced resync pass");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the resync request forces exactly one full walk"
        );

        // The latch was consumed: the following pass is incremental again.
        daemon.reconcile_blocking().expect("back to steady state");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the force-full-walk latch is consumed after one pass"
        );
    }

    fn directory_record(path: &str, proton_id: Option<&str>) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            entity_kind: EntityKind::Directory,
            file_size: 0,
            mtime: 1,
            sha1_hash: None,
            proton_id: proton_id.map(ToOwned::to_owned),
            sync_status: SyncStatus::Synced,
        }
    }

    /// Wraps a real [`ProtonDriveClient`], delegating every call but counting the two listing
    /// shapes so a live test can prove *how* the remote was read: `full_walks` is the O(folders)
    /// bootstrap/snapshot walk, `directory_lists` is the O(1) targeted resolution the incremental
    /// path uses. Exactly one full walk across a whole run means the create was found purely by the
    /// event stream, with no fallback snapshot.
    struct WalkCountingClient {
        inner: ProtonDriveClient,
        full_walks: Arc<AtomicUsize>,
        directory_lists: Arc<AtomicUsize>,
    }

    impl ProtonClient for WalkCountingClient {
        fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            self.inner.list(remote_root)
        }
        fn list_entities(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            self.inner.list_entities(remote_root)
        }
        fn list_entities_or_missing_root(
            &self,
            remote_root: &Path,
        ) -> AppResult<RemoteListingStatus> {
            self.full_walks.fetch_add(1, Ordering::SeqCst);
            self.inner.list_entities_or_missing_root(remote_root)
        }
        fn list_directory(
            &self,
            remote_root: &Path,
            relative_directory: &Path,
        ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            self.directory_lists.fetch_add(1, Ordering::SeqCst);
            self.inner.list_directory(remote_root, relative_directory)
        }
        fn ensure_root_directory(&self, remote_root: &Path) -> AppResult<()> {
            self.inner.ensure_root_directory(remote_root)
        }
        fn ensure_directory(&self, remote_root: &Path, relative_path: &Path) -> AppResult<()> {
            self.inner.ensure_directory(remote_root, relative_path)
        }
        fn upload(
            &self,
            local_path: &Path,
            remote_root: &Path,
            relative_path: &Path,
        ) -> AppResult<()> {
            self.inner.upload(local_path, remote_root, relative_path)
        }
        fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()> {
            self.inner.download(remote_path, destination)
        }
        fn delete(&self, remote_path: &Path) -> AppResult<()> {
            self.inner.delete(remote_path)
        }
        fn rename_or_move(
            &self,
            remote_root: &Path,
            old_relative_path: &Path,
            new_relative_path: &Path,
        ) -> AppResult<()> {
            self.inner
                .rename_or_move(remote_root, old_relative_path, new_relative_path)
        }
        fn install_cancel_flag(&mut self, cancel_flag: Arc<AtomicBool>) {
            self.inner.install_cancel_flag(cancel_flag);
        }
    }

    /// **Phase 4 — flag-on live e2e.** Drives a real [`Daemon`] (real CLI client + real event
    /// source) through the market-data recovery shape end to end: bootstrap ("get truth" — one
    /// full-tree snapshot that captures the replay cursor), then a *remote* create the daemon must
    /// discover from the volume event stream alone and download via the O(1) targeted path — with
    /// **zero** further full walks. Also characterizes the listing-lag race the resolver is exposed
    /// to (a just-created node briefly absent from its parent listing): if the create only lands
    /// after a fallback snapshot, `full_walks` climbs past 1 and this test says so.
    ///
    /// ```bash
    /// PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
    /// PROTON_SYNC_LIVE_REMOTE_ROOT=/my-files/<disposable-folder> \
    /// PROTON_SYNC_LIVE_WRITE=1 \
    ///   cargo test --lib daemon::tests::live_event_driven_reconcile -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "live e2e: real CLI + keyring; set PROTON_SYNC_EVENTS_VOLUME, a disposable PROTON_SYNC_LIVE_REMOTE_ROOT, and PROTON_SYNC_LIVE_WRITE=1"]
    fn live_event_driven_reconcile_downloads_a_remote_create_without_a_full_walk() {
        if std::env::var("PROTON_SYNC_LIVE_WRITE").ok().as_deref() != Some("1") {
            eprintln!(
                "skipping live e2e: set PROTON_SYNC_LIVE_WRITE=1 (uploads then trashes a probe)"
            );
            return;
        }
        // Surface the daemon's own tracing (fallback reasons, plan summaries) under --nocapture.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_test_writer()
            .try_init();
        let volume =
            std::env::var("PROTON_SYNC_EVENTS_VOLUME").expect("set PROTON_SYNC_EVENTS_VOLUME");
        let remote_root = PathBuf::from(
            std::env::var("PROTON_SYNC_LIVE_REMOTE_ROOT")
                .expect("set PROTON_SYNC_LIVE_REMOTE_ROOT"),
        );

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let mut config = test_config(directory.path(), &local_root);
        config.remote_root = remote_root.clone();
        config.events_driven = true;
        // Keep the periodic resync out of reach so a downloaded create can only be explained by the
        // incremental (event-stream) path, never a forced full walk.
        config.events_full_scan_every = 1000;

        let full_walks = Arc::new(AtomicUsize::new(0));
        let directory_lists = Arc::new(AtomicUsize::new(0));
        let client = WalkCountingClient {
            inner: ProtonDriveClient::new(config.proton_cli.clone()),
            full_walks: Arc::clone(&full_walks),
            directory_lists: Arc::clone(&directory_lists),
        };
        // A separate, UNCOUNTED client used only to poll for propagation, so waiting on the
        // eventually-consistent listing never inflates the targeted-list counter the assertions
        // rely on.
        let watcher = ProtonDriveClient::new(config.proton_cli.clone());
        let event_source =
            build_event_source(&config).expect("build a real event source from the CLI keyring");
        let mut daemon = Daemon::with_client_and_event_source(config, client, Some(event_source))
            .expect("daemon");

        // Per-run unique names keep the test hermetic: a prior run's node that is trashed but still
        // briefly listed (server lag) can never collide with this run's.
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let seed_rel = PathBuf::from(format!("proton-sync-e2e-seed-{unique}.txt"));
        let probe_rel = PathBuf::from(format!("proton-sync-e2e-probe-{unique}.txt"));
        let seed_scratch = std::env::temp_dir().join(seed_rel.file_name().unwrap());
        let probe_scratch = std::env::temp_dir().join(probe_rel.file_name().unwrap());

        // Phase A0 — seed a SUPPORTED (downloadable) file so the bootstrap indexes a record carrying
        // a composed proton_id. Event-driven mode derives the volume from *base* records, so without
        // one the gate never engages and every pass silently full-walks (safe, but not what we are
        // exercising). Wait until it is listable so the bootstrap snapshot includes it.
        fs::write(&seed_scratch, b"e2e seed payload").expect("write seed scratch");
        daemon
            .proton
            .upload(&seed_scratch, &remote_root, &seed_rel)
            .expect("upload the seed");
        assert!(
            wait_until_listed(&watcher, &remote_root, &seed_rel, Duration::from_secs(60)),
            "the seed file must become listable before bootstrap"
        );

        // Phase A — bootstrap ("get truth"): the startup floor forces exactly one full-tree
        // snapshot, which downloads + records the seed (base now carries a proton_id → the volume is
        // derivable), captures the replay cursor, and resets the resync counter.
        daemon.reconcile_blocking().expect("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "bootstrap performs exactly one full-tree walk"
        );
        assert_eq!(
            daemon.incremental_passes_since_full_scan, 0,
            "bootstrap resets the resync counter"
        );
        assert_eq!(
            derive_volume_id(&load_index(&daemon.connection).expect("load index")),
            Some(volume.as_str()),
            "the seed gave the base index a composed proton_id so the incremental gate can engage"
        );
        let bootstrap_cursor = load_event_cursor(&daemon.connection, &volume)
            .expect("load cursor")
            .expect("bootstrap captured a cursor")
            .last_event_id;
        eprintln!("bootstrap OK: one full walk, cursor = {bootstrap_cursor}");

        // Phase B — make a REMOTE create the daemon can only learn about from the event stream.
        fs::write(&probe_scratch, b"event-driven e2e probe payload").expect("write probe scratch");
        daemon
            .proton
            .upload(&probe_scratch, &remote_root, &probe_rel)
            .expect("upload the probe");
        // Readiness gate (raw, uncounted): wait until the probe is listable so the targeted resolve
        // cannot hit the listing-lag race. This isolates the wiring proof (does the real
        // event→resolve→download chain work) from propagation timing (which this test must not gate
        // on against an eventually-consistent service).
        assert!(
            wait_until_listed(&watcher, &remote_root, &probe_rel, Duration::from_secs(60)),
            "the probe must become listable within the propagation window"
        );

        // Phase C — stream: reconcile until the probe is downloaded. Event delivery can still lag the
        // listing by a beat, so an early pass may be idle; none of these passes is a full walk.
        let local_probe = local_root.join(&probe_rel);
        let dir_lists_before = directory_lists.load(Ordering::SeqCst);
        let mut picked_up_on_pass = None;
        for pass in 1..=10 {
            daemon.reconcile_blocking().expect("incremental reconcile");
            if local_probe.exists() {
                picked_up_on_pass = Some(pass);
                break;
            }
            std::thread::sleep(Duration::from_secs(3));
        }

        // Snapshot the observations, then clean up BOTH nodes BEFORE asserting so a failure never
        // leaves the account dirty.
        let walks = full_walks.load(Ordering::SeqCst);
        let dir_lists_during = directory_lists.load(Ordering::SeqCst) - dir_lists_before;
        let final_counter = daemon.incremental_passes_since_full_scan;
        let final_cursor = load_event_cursor(&daemon.connection, &volume)
            .ok()
            .flatten()
            .map(|cursor| cursor.last_event_id);
        let _ = daemon.proton.delete(&remote_root.join(&probe_rel));
        let _ = daemon.proton.delete(&remote_root.join(&seed_rel));
        let _ = fs::remove_file(&probe_scratch);
        let _ = fs::remove_file(&seed_scratch);

        let pass = picked_up_on_pass
            .expect("the remote-created probe must be downloaded by an incremental pass");
        assert!(
            dir_lists_during >= 1,
            "the incremental phase must resolve the create via a targeted directory listing"
        );
        assert_eq!(
            walks, 1,
            "the create must be found via the event stream with NO fallback full walk"
        );
        assert_ne!(
            final_cursor.as_deref(),
            Some(bootstrap_cursor.as_str()),
            "the cursor must advance past the create event"
        );
        eprintln!(
            "live e2e OK: remote create downloaded on incremental pass {pass}; \
             full_walks={walks}, targeted_lists_during_stream={dir_lists_during}, \
             resync_counter={final_counter}, cursor {bootstrap_cursor} -> {final_cursor:?}"
        );
    }

    /// Polls a remote directory (raw, uncounted) until `relative` appears in the root listing or the
    /// timeout elapses. Used by the live e2e to wait out the CLI's eventual-consistency lag without
    /// disturbing the targeted-list counter under test.
    fn wait_until_listed(
        client: &ProtonDriveClient,
        remote_root: &Path,
        relative: &Path,
        timeout: Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Ok(listing) = client.list_directory(remote_root, Path::new(""))
                && listing.contains_key(relative)
            {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}
