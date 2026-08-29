use crate::ancestor::{LineSummary, MAX_SUMMARY_BYTES};
use crate::dirconfig::{DirectoryConfigResolver, EffectiveSettings};
use crate::events::{EventSource, EventsClient, RemoteChange, node_uid, volume_id_from_proton_id};
use crate::index::{
    EntityKind, EventCursor, FileEvent, FileRecord, HistoryRetention, IndexTotals,
    LocalEntityState, LocalFileState, LocalScan, PassKind, PassOutcomeKind, ScanOptions,
    SyncStatus, UnsyncableEntry, WithheldDeletion, begin_pass, byte_totals_since,
    delete_delete_approval, file_events, finish_pass, get_record, index_totals, insert_file_events,
    last_full_sweep, load_event_cursor, load_existing_index, load_index, load_sole_event_cursor,
    load_unsyncable_items, load_warm_start_count, load_withheld_deletions, local_directory_state,
    local_file_state, mark_modified, matching_delete_approval, open_database, path_for_proton_id,
    prune_agreed_summaries, prune_history, purge_record, purge_subtree_records, recent_passes,
    replace_unsyncable_items, replace_withheld_deletions, reset_index_state, scan_local_tree,
    store_agreed_summary, store_event_cursor, store_warm_start_count, upsert_delete_approval,
    upsert_record,
};
use crate::ipc::{
    ACTIVITY_EVENTS_DEFAULT_LIMIT, ACTIVITY_EVENTS_MAX_LIMIT, ApplyOutcome, AuthState,
    ControlCommand, ControlResponse, FAILED_ITEM_ERROR_LIMIT, FailedItem,
    LIST_ENTRIES_DEFAULT_LIMIT, LIST_ENTRIES_MAX_LIMIT, ListingOutcome, LocalDisposal,
    PASS_HISTORY_REPORTED, PLAN_ACTIONS_DEFAULT_LIMIT, PLAN_ACTIONS_MAX_LIMIT, PLAN_PASS_KIND,
    PassHistory, PassProgress, PendingDeletion, PlanOutcome, RemoteEntry, ReviewedPlan,
    RunningConfigInfo, SELECTOR_DISPLAY_LIMIT, StatusHistoryEntry, SyncActivity,
    TRANSFERS_REPORTED, TransferActivity, bind_listener, read_request, write_response,
};
use crate::proton::{
    AuthFailure, CommandPolicy, DownloadRequest, ProgressSink, ProtonClient, ProtonDriveClient,
    RemoteEntity, RemoteListingStatus, is_auth_failure_error, is_cli_busy_error,
    is_node_not_found_error,
};
use chrono::{Local, TimeZone};

use crate::reconstruct::{Reconstruction, RemoteChangeResolver, reconstruct_remote};
use crate::schedule::FullScanSchedule;
use crate::session::{CliKeyringSession, CurlHttpTransport};
use crate::sync::{
    ConflictNaming, DeleteDirection, DryRunReport, PlanSummary, PlannedAction, SyncAction,
    TransferDirection, UnsyncableItem, UnsyncableOrigin, directory_move_descendant_path_pairs,
    is_strict_descendant, plan_sync_entities_with_stats, plan_token, unsyncable_items,
};
use crate::trash::{LocalDeleteMode, dispose};
use crate::{AppResult, boxed_error};
use fs2::FileExt;
use indicatif::{ProgressBar, ProgressStyle};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::btree_map::Entry;
use std::fs::{self, File, OpenOptions};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub(crate) const STATUS_HISTORY_LIMIT: usize = 20;
/// Time budget for a single control-connection read or write. Control connections are served
/// concurrently on their own task, so a stalled client cannot block the daemon — the timeout
/// just stops silent clients from accumulating parked connection tasks forever.
const IPC_IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the read-only `list` verb (#99) may wait for the `proton::CliGate` before answering
/// [`ListingOutcome::Busy`]. Long enough to slip between the short listings of a full-tree walk —
/// which is the point of gating per invocation rather than per pass — and short enough to stay well
/// inside a client's own round-trip budget (`proton-sync`'s is 10s), so a refusal arrives as an
/// answer rather than as a timeout.
const BROWSE_GATE_WAIT: Duration = Duration::from_secs(2);
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

/// What the scheduled-sweep timer is parked on when there is no schedule, or when one cannot be
/// resolved to an instant at all. **The arm is disabled in both cases** — it is gated on the
/// resolved deadline, not on the key being present — so this deadline exists only because a pinned
/// `sleep` needs one, and is never reached. A day rather than `Duration::MAX`, which overflows
/// tokio's timer wheel, and rather than something short, which would wake the loop for nothing.
const NO_SCHEDULE_PARK: Duration = Duration::from_secs(86_400);
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
    /// The one **user-facing** full-sweep trigger (#193, G4): a wall-clock moment at which the next
    /// pass is a full walk. `None` — the default, and what every config written before this key
    /// says — means no scheduled sweep at all.
    ///
    /// It does not replace, wrap or arbitrate with the two self-heal nets beside it
    /// (`events_full_scan_every`, `warm_start_full_walk_every`): all three mean the identical thing
    /// to a pass, so they compose through one idempotent latch and there is no precedence rule.
    /// See [`crate::schedule`] for that reasoning and for why a missed run is skipped, not caught up.
    pub full_scan_schedule: Option<FullScanSchedule>,
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
    /// What a local deletion does to the entity: [`LocalDeleteMode::Trash`] (the default) moves it
    /// to the desktop trash, [`LocalDeleteMode::Permanent`] removes it from disk. A **different
    /// question** from `delete_approval_local`, which decides only whether the deletion waits for a
    /// person: this one decides what happens once it goes ahead, and is what the wire's
    /// [`crate::ipc::LocalDisposal`] and every warning attached to a deletion are derived from.
    pub local_delete_mode: LocalDeleteMode,
    /// Startup-reconcile tuning (warm start; see [`WarmStartConfig`]).
    pub warm_start: WarmStartConfig,
    /// How conflict sidecars are named (`conflict_suffix`, default `proton-cloud`). One value for
    /// the planner that writes them and the scanner/watcher that must not sync them — see
    /// [`crate::sync::ConflictNaming`], including what changing it does to sidecars already on disk.
    pub conflict_naming: ConflictNaming,
    /// The resolved `tracing` filter directive (`--log-level` > `RUST_LOG` > file > `info`; see
    /// [`crate::config::resolve_log_filter`]). Applied by the binary at startup — the daemon holds
    /// it so the value that is running is the value the config resolved, not a second env read.
    pub log_filter: String,
}

impl DaemonConfig {
    /// The **one** projection of the resolved config onto the two scopes ADR 0005 §2 classifies
    /// every key into — the same split `crate::config::KeyScope` makes machine-checked at parse
    /// time, now expressed in what the runtime holds.
    ///
    /// Consuming, not borrowing, and that is the point: after this call there is exactly one copy of
    /// `local_root`, inside the [`PairConfig`] the pair's runtime owns. A second copy left on the
    /// daemon is how "which root is this pass about" comes to have two answers.
    ///
    /// One projection, both callers: the daemon's constructor and the one-shot `--dry-run` preview
    /// (which clones its borrowed config to reach it). A second projection would be a second
    /// classification, free to drift from `KeyScope` and from this one.
    ///
    /// **Phase 3/4 changes the shape, not this rule:** `resolve_runtime_config` grows a
    /// `Vec<PairConfig>` and this becomes `(ProcessConfig, Vec<PairConfig>)`. Until then
    /// `config::refuse_unsupported_pair_count` is what makes the singular half correct.
    ///
    /// Four `KeyScope::Daemon` keys are deliberately **spent** here rather than carried: `proton_cli`,
    /// `proton_timeout` and `proton_list_attempts` are consumed constructing the one shared client
    /// (`command_policy_from_config`), and `log_filter` by the binary's tracing setup — all three
    /// times before a daemon exists. Being daemon-wide is precisely *why* they can be spent at
    /// construction: there is one client and one filter for the process, so nothing later needs to
    /// ask which pair's they were.
    fn into_parts(self) -> (ProcessConfig, PairConfig) {
        let Self {
            local_root,
            remote_root,
            db_path,
            socket_path,
            lockfile_path,
            global_lock_path,
            scan_interval,
            full_scan_schedule,
            proton_cli: _,
            proton_timeout: _,
            proton_list_attempts: _,
            download_batch_size,
            include_patterns,
            exclude_patterns,
            events_driven,
            events_full_scan_every,
            delete_approval_remote,
            delete_approval_local,
            local_delete_mode,
            warm_start,
            conflict_naming,
            log_filter: _,
        } = self;
        (
            ProcessConfig {
                socket_path,
                global_lock_path,
            },
            PairConfig {
                local_root,
                remote_root,
                db_path,
                lockfile_path,
                scan_interval,
                full_scan_schedule,
                download_batch_size,
                include_patterns,
                exclude_patterns,
                events_driven,
                events_full_scan_every,
                delete_approval_remote,
                delete_approval_local,
                local_delete_mode,
                warm_start,
                conflict_naming,
            },
        )
    }
}

/// The `crate::config::KeyScope::Daemon` keys a **running** daemon still holds: the control socket
/// it serves and the user-global lock it holds for its whole life. Both describe the process rather
/// than a tree, and neither could be per-pair — one socket must be locatable without knowing a root
/// (`paths.rs`), and the global lock is what makes "one `proton-syncd` per user" true (#23).
///
/// The other four daemon-wide keys are spent at construction — see [`DaemonConfig::into_parts`].
#[derive(Debug, Clone)]
struct ProcessConfig {
    socket_path: PathBuf,
    global_lock_path: PathBuf,
}

/// The keys that describe **one tree**: what to sync, what to skip, how often, how a pass behaves,
/// what a deletion needs (`crate::config::KeyScope::Pair`). Field docs live on [`DaemonConfig`],
/// which is still the shape a config file and the CLI flags resolve to.
#[derive(Debug, Clone)]
struct PairConfig {
    local_root: PathBuf,
    remote_root: PathBuf,
    db_path: PathBuf,
    lockfile_path: PathBuf,
    scan_interval: Duration,
    /// See [`DaemonConfig::full_scan_schedule`]. Per-pair because it describes a tree, exactly as
    /// `scan_interval` above it does — phase 4's due queue will arm one timer per pair.
    full_scan_schedule: Option<FullScanSchedule>,
    download_batch_size: usize,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
    events_driven: bool,
    events_full_scan_every: u64,
    delete_approval_remote: bool,
    delete_approval_local: bool,
    local_delete_mode: LocalDeleteMode,
    warm_start: WarmStartConfig,
    conflict_naming: ConflictNaming,
}

/// One folder pair's runtime: everything a pass over that pair decides from and writes back.
///
/// **What is not here is the point** (#102 phase 2, ADR 0005 §1). The `proton-drive` client, its
/// `CommandPolicy`, the [`crate::proton::CliGate`] inside it, the cancel flag and the progress sink
/// stay on the daemon *by force*: one client is one gate, so a pair holding its own would be #23
/// reintroduced inside one process. Neither is the Proton session verdict here — one user, one
/// session (`ControlShared::auth`).
///
/// The daemon holds one of these per pair. Today that is exactly one, because
/// `config::refuse_unsupported_pair_count` refuses more; phase 4 lifts that and the scheduler picks
/// which pair's runtime the next pass gets.
struct PairRuntime {
    /// What describes this tree (see [`PairConfig`]).
    config: PairConfig,
    /// This pair's index — the baseline, the cursor, the approvals, the history. One database per
    /// pair, already: it lives in `<local_root>/.sync/sync_index.db`, which is why multi-pair needs
    /// no schema migration (ADR 0005 §3). The control plane's second connection to the same file is
    /// built beside it in [`Daemon::run`].
    connection: Connection,
    /// Selective-sync filters + ignore rules + this pair's conflict-sidecar naming. Derived from
    /// [`Self::config`] at construction, and the *same* value the planner, the scanner and the
    /// watcher read — one spelling on the writing side and another on the reading side means the
    /// engine uploads the sidecar it just wrote.
    scan_options: ScanOptions,
    status_history_path: PathBuf,
    metrics_path: PathBuf,
    /// Relative paths under this pair's root that the watcher has queued for the next pass.
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
    ///
    /// Per-pair, but a watcher **error** carries no path: an inotify overflow means events were lost
    /// *somewhere*, so once there are N pairs it must be set on every one of them (ADR 0005 §5).
    force_local_rescan: bool,
    last_sync: Option<SystemTime>,
    last_error: Option<String>,
    last_plan_summary: Option<PlanSummary>,
    last_successful_sync_summary: Option<PlanSummary>,
    status_history: Vec<StatusHistoryEntry>,
    /// Number of successful incremental (event-driven) passes since the last full-tree snapshot.
    /// Drives the opt-in periodic safety resync (`events_full_scan_every`, disabled by default).
    incremental_passes_since_full_scan: u64,
    /// `true` until the first reconcile of this process succeeds. The first pass is special —
    /// `notify` has replayed nothing, so it must full-scan the local tree (either via a bootstrap
    /// or a warm start's forced local scan) to catch edits made while the daemon was down. Cleared
    /// only on success, so a failed first pass retries as a first pass (keeping the "startup
    /// snapshots first" floor sticky across failures).
    ///
    /// Per-pair, and **every** pair must get such a pass before it may take the idle fast-path: the
    /// flag is about one tree's local scan, so a pair that inherited another's cleared flag would
    /// strand an edit made while the daemon was down.
    is_first_reconcile: bool,
    /// Consecutive warm starts since the last full-tree walk, **persisted across restarts** (loaded
    /// from / stored to the `warm_start_state` table). Drives the every-N-warm-starts self-healing
    /// full walk (`warm_start.full_walk_every`). Distinct from the in-run
    /// `incremental_passes_since_full_scan`.
    warm_starts_since_full_walk: u64,
    /// The last reported reason **this pair** could not resolve a volume + cursor, so a standing
    /// decline is logged once instead of every pass (and re-logged if the cause changes). `None`
    /// while the scope resolves. Diagnostic only — see `PairPass::resolve_event_scope`.
    ///
    /// Per-pair because the causes are: the volume comes from this pair's baseline, the cursor from
    /// this pair's database. The process-wide causes latch on [`Daemon::session_declined`], which
    /// documents why one shared latch cannot serve both.
    event_scope_declined: Option<String>,
    /// Deletions withheld by the delete-approval guard on the most recent reconcile, awaiting the
    /// user's approval. Recomputed from ground truth every pass, so it always reflects the current
    /// plan; surfaced over IPC (`proton-sync pending`) and in the metrics sidecar.
    pending_deletions: Vec<PendingDeletion>,
    /// Items whose action failed during the most recent pass, bounded to
    /// [`FAILED_ITEMS_REPORTED`] in plan order (#136). Reset at the start of every pass's
    /// execution; `last_failed_item_count` keeps the untruncated total.
    last_failed_items: Vec<FailedItem>,
    /// Untruncated count behind `last_failed_items`.
    last_failed_item_count: usize,
    /// Entities the engine cannot sync, standing across passes and restarts (see
    /// `PairPass::record_unsyncable` for why it is not simply recomputed, and
    /// `index::load_unsyncable_items` for why it is persisted). Display-only: surfaced over IPC and
    /// in the metrics sidecar, read by no sync decision.
    unsyncable: Vec<UnsyncableItem>,
    /// Cached corpus size (#207), republished on every `publish_status` so a status reply never
    /// queries SQLite. `None` until the first refresh.
    index_totals: Option<IndexTotals>,
    /// Whether [`Self::index_totals`] may be out of date. Set when a pass plans any action (only a
    /// planned action mutates the index, so an empty plan cannot change the totals — that is what
    /// keeps an idle event-driven pass free of the aggregate query), and cleared by a refresh.
    ///
    /// Deliberately **not** gated on the pass succeeding: checkpoint commits are durable even when
    /// a later action fails the pass (ADR 0003), so a failed pass can still have changed the index,
    /// and waiting for a clean one would serve a stale count indefinitely.
    index_totals_stale: bool,
    /// Paths already named at `warn` this run, so a permanent condition is reported once per
    /// process instead of once per pass — a pass-rate log on a condition that never changes is
    /// noise, and noise is why the old `debug!` line went unread for five months (#295). Pruned
    /// when a path leaves the list, so a recurrence speaks again.
    reported_unsyncable: HashSet<PathBuf>,
    /// The pass currently in flight (see [`PassLog`]): its history rows, its rollup, and the clock
    /// behind `duration_ms`. Replaced at the top of every pass.
    pass_log: PassLog,
    /// Pass-level history + today's byte totals, re-read from the index after each pass and
    /// published on every status reply. Cached here so a polling client never queries SQLite.
    pass_history: Option<PassHistory>,
    /// What the pass currently in flight was asked to do (#100/#192). Taken from the shared plan
    /// slot at the top of each pass and reset by `reconcile_blocking` as it seals the apply's
    /// verdict, so it describes exactly one pass and never leaks into the next.
    pass_intent: PassIntent,
    /// The verdict `PairPass::execute_plan_and_commit` reached about this pass's apply, when it got
    /// far enough to reach one. `None` means the pass ended before deciding, and
    /// `reconcile_blocking` seals a `Failed` — which is what stops an apply whose remote listing
    /// blew up from leaving its client polling for ever.
    apply_report: Option<ApplyOutcome>,
    /// Per-root instance lock; released on drop. Held for the daemon's whole lifetime. Per-pair
    /// because it stops two daemons syncing the *same* root — the user-global lock (on the daemon)
    /// is what stops a second daemon at all.
    _lock_guard: LockGuard,
}

/// **One pass, one pair** — the receiver the whole reconcile family runs on.
///
/// A pass needs two things: the pair it is about, and the process-wide pieces every pair shares
/// (the one client, the control plane, the cancel flag, the one event-stream session). This borrows
/// exactly those from the daemon, which is what makes "a pass cannot touch another pair" structural
/// rather than a rule each of forty methods has to remember: [`Self::pair`] is the *only* pair in
/// scope here, because `pairs` itself is not.
///
/// Built by [`Daemon::pass`], which is where phase 4's scheduler will *choose* the pair. Nothing in
/// the pass family chooses one, so lifting `N > 1` changes that one function, not these bodies.
///
/// A borrowed view rather than a `&mut PairRuntime` parameter threaded through every signature
/// because `self.method(&mut self.pairs[0])` is rejected by the borrow checker — `&mut self` and
/// `&mut self.pairs[0]` overlap. Destructuring the daemon's fields once, in `pass()`, is the split
/// the compiler proves disjoint.
struct PairPass<'a, C: ProtonClient> {
    /// The pair this pass is about.
    pair: &'a mut PairRuntime,
    /// The one `proton-drive` client, hence the one [`crate::proton::CliGate`] (#23).
    proton: &'a Arc<C>,
    shared: &'a Arc<ControlShared>,
    cancel_flag: &'a Arc<AtomicBool>,
    /// The volume-event session. One per process: it is a *session*, and the volume is a per-call
    /// argument (ADR 0005 §5), which is why the cursor is per-pair but this is not.
    event_source: &'a mut Option<Box<dyn EventSource>>,
    event_source_factory: &'a mut Box<dyn FnMut() -> Option<Box<dyn EventSource>> + Send>,
    /// See [`Daemon::session_declined`]. The pair's own half of the same message family is
    /// `PairRuntime::event_scope_declined`.
    session_declined: &'a mut Option<SessionDecline>,
    /// Copied rather than borrowed: a pass only ever *reports* this cadence (in the degraded-session
    /// message), never changes it.
    events_poll_interval: Duration,
}

pub struct Daemon<C: ProtonClient = ProtonDriveClient> {
    /// What describes the process (see [`ProcessConfig`]).
    process: ProcessConfig,
    /// The folder pairs this daemon syncs, in config order (see [`PairRuntime`]).
    ///
    /// **Length 1 today**, because `config::refuse_unsupported_pair_count` refuses more until the
    /// runtime can serialize their passes. It is a `Vec` rather than one pair so that lifting the
    /// refusal is a change to *who picks* — [`Self::pass`] and the run loop's cadence — rather than
    /// a change to what a pass is.
    pairs: Vec<PairRuntime>,
    /// The `proton-drive` client. Behind an `Arc` because the control-socket task holds it too:
    /// the read-only `list` verb (#99) runs a CLI invocation from that task, and it must be the
    /// *same* client instance as the main loop's — one client is one `proton::CliGate`, and a
    /// second instance would be a second gate, i.e. no serialization at all (#23).
    proton: Arc<C>,
    /// State shared with the concurrently-running control-socket server task (see
    /// [`ControlShared`]). The daemon core is the only writer of the snapshot; `paused` is
    /// written by both sides (IPC `pause`/`resume` flips it, the core reads it).
    shared: Arc<ControlShared>,
    ipc_io_timeout: Duration,
    /// How long the read-only `list` verb may wait for the CLI gate ([`BROWSE_GATE_WAIT`]); a field
    /// for the same reason [`Self::ipc_io_timeout`] is one — a test needs a budget it can outlast.
    browse_gate_wait: Duration,
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
    /// The **process-wide** half of the "event-driven detection unavailable" message family: why this
    /// daemon cannot stream events *at all*, latched so a standing cause is logged once instead of
    /// every pass (and re-logged when it changes). `None` while nothing is wrong with the session.
    /// Diagnostic only.
    ///
    /// It is one latch per *scope*, not one latch overall (#102 phase 2, ADR 0005 §1). The two causes
    /// here — no CLI session, and a signed-out account — are facts about the user; a missing volume
    /// or an unreadable cursor is a fact about one tree and latches on that pair
    /// (`PairRuntime::event_scope_declined`). Sharing one latch across pairs would make two pairs
    /// declining for two volumes overwrite each other's reason and log again every pass, for ever.
    ///
    /// **At most one of the two latches speaks per pass, and that is structural rather than
    /// arranged**: `note_degraded_session_if_needed` reports only when there is *no* event source,
    /// and every path that resolves a scope (`warm_start_eligible`, `should_try_incremental`,
    /// `try_incremental_reconcile`) requires one — so a pass either has no session, or has one and
    /// may then decline on the volume/cursor. The auth cause is the exception by design: it is
    /// evidence from the pass's *outcome*, so it can follow a scope decline in the same pass, which is
    /// exactly the case a shared latch used to mask by overwriting.
    session_declined: Option<SessionDecline>,
    /// Shared with the `proton-drive` client and flipped by a shutdown signal or the IPC
    /// `shutdown` command. The executor polls it: a pass that keeps going past item failures must
    /// still stop dead on shutdown, or it would spawn (and immediately kill) one CLI child per
    /// remaining action. Owned here so `run` can install the same flag on the client.
    cancel_flag: Arc<AtomicBool>,
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
    /// Items whose action failed in the last pass (#136); bounded like the IPC reply's, with
    /// `failed_item_count` the untruncated total. `#[serde(default)]` so a sidecar written by an
    /// older daemon still deserializes.
    #[serde(default)]
    pub failed_items: Vec<FailedItem>,
    #[serde(default)]
    pub failed_item_count: usize,
    /// Entities that cannot be synced (see [`UnsyncableItem`]). `#[serde(default)]` so a sidecar
    /// written by an older daemon still parses.
    #[serde(default)]
    pub unsyncable: Vec<UnsyncableItem>,
    /// Corpus size (#207), mirrored from the status snapshot so the sidecar answers it too — the
    /// GUI falls back to the sidecars when the socket is down. `#[serde(default)]` so a sidecar
    /// written by an older daemon still parses.
    #[serde(default)]
    pub index_totals: Option<IndexTotals>,
}

/// State shared between the daemon core and the control-socket server task so control requests
/// are answered instantly from the latest published snapshot — even while a reconcile is
/// blocking the main loop. This is what keeps the CLI and GUI responsive during a long sync.
///
/// **Split along the one line that is not a judgement call** (#102 phase 2, ADR 0005 §1): what
/// describes *a tree* is a [`PairShared`] block, and what describes *this user's Proton session* is
/// [`Self::auth`]. The block is singular here because the daemon runs exactly one pair; phase 3
/// replaces it with `pairs: Vec<PairShared>` plus the wire's `pair` selector, at which point every
/// `self.pair` below becomes a lookup and [`Self::response`] answers for the *selected* pair. The
/// wire is unchanged by that: [`crate::ipc::ControlResponse`]'s per-pair fields stay flattened at
/// the top level (three legacy-JSON floor tests pin it), so what phase 3 adds is a second way to
/// address them, never a second shape.
struct ControlShared {
    /// What the daemon believes about the Proton session (#103), as an [`AuthState`] discriminant.
    ///
    /// **Per-user, not per-pair, and the only field here that is.** Every pair reaches Proton
    /// through one CLI session, so a sign-out is a fact about the process (ADR 0005 §1). An atomic
    /// slot of its own, deliberately **not** a `snapshot` field: the snapshot is published wholesale
    /// by the daemon core and only ever read by the IPC task, whereas this has two writers. The core
    /// writes it from each pass's outcome, and the IPC task writes it from the `list` verb — a browse
    /// refused for an expired session is the fastest auth evidence the daemon can get, and making it
    /// wait for the next pass would be the whole point of designing #99 and #103 together thrown
    /// away.
    auth: AtomicU8,
    /// Everything the control plane publishes about one folder pair.
    pair: PairShared,
}

/// The per-pair half of [`ControlShared`]: one folder pair's latches, counters and published
/// snapshot. One block per pair once phase 3 lands; exactly one today.
struct PairShared {
    /// Whether syncing is paused. Written by the IPC task (`pause`/`resume`) and read by the
    /// daemon core before each reconcile, so a pause takes effect from the next pass.
    paused: AtomicBool,
    /// `true` while a reconcile pass is in flight (drives the `syncing` status).
    syncing: AtomicBool,
    /// `true` while the in-flight pass is a **plan-only** one (#100/#209).
    ///
    /// It exists because a plan pass claims [`Self::syncing`] — it must, or `activity` is gated off
    /// every status reply and #209's progress line has nothing to read — while deliberately not
    /// bumping [`Self::reconcile_seq`]. Without this discriminator a `syncnow` issued during a
    /// rehearsal would be told "already in progress" and its client (`watch_syncnow`) would wait
    /// for `reconcile_seq + 2`, when the pass it is queued behind contributes 0 — so it would sit
    /// until some later unrelated pass reached that number and then report *that* pass's outcome.
    plan_pass: AtomicBool,
    /// Count of completed reconcile attempts since startup (see `ControlResponse::reconcile_seq`).
    reconcile_seq: AtomicU64,
    /// Set by the IPC `resync` command to force the daemon's next reconcile to a full-tree walk
    /// (rather than a warm start / incremental pass). The daemon core consumes it with a `swap`
    /// at the top of each pass. Written by the IPC task, read-and-cleared by the core.
    force_full_walk: AtomicBool,
    /// Set by the IPC `reset_index` command to discard the learned state before the next pass.
    /// Same shape as [`Self::force_full_walk`] and for the same reason: the truncation must happen
    /// on the main loop between passes, never from the IPC task under an in-flight reconcile.
    /// Written by the IPC task, read-and-cleared by the core.
    reset_index: AtomicBool,
    /// The daemon core's most recently published status. The IPC task only ever reads it.
    snapshot: StdMutex<StatusSnapshot>,
    /// Live activity for the in-flight pass (see [`SyncActivity`]). Written from inside the
    /// blocking reconcile (phase changes, per-folder walk progress, per-file scan progress,
    /// per-action execution) and read by the IPC task on every status reply, so clients see
    /// motion while the main task is blocked. Separate from `snapshot` because it churns far
    /// more often than a `publish_status` and needs none of the snapshot's contents.
    activity: StdMutex<ActivityState>,
    /// The reviewed plan and its request lifecycle (#100/#192/#209). Its own mutex, and **one**
    /// mutex for the whole plane rather than an atomic per counter: booking a request, sealing it,
    /// and replacing the stored plan are one transition each, and splitting them across atomics is
    /// how a client comes to read a stale `Computed` as the answer to the request it just made.
    plan: StdMutex<PlanSlot>,
}

/// Everything the plan verbs decide from, under one lock.
///
/// The two counter pairs are the whole synchronisation contract (see [`PlanOutcome`]): a request
/// books `requested`, the pass that answers it sets `completed` to the value it started at, and a
/// client waits until `completed >= the number its ack returned`. `requested > completed` therefore
/// *is* "a pass is scheduled or running" — one fact derived from the counters rather than a third
/// `pending` flag that could disagree with them.
#[derive(Default)]
struct PlanSlot {
    /// Plan requests booked, and plan requests answered.
    requested: u64,
    completed: u64,
    /// The latest computed plan; replaced wholesale (one stored plan at a time, latest wins).
    stored: Option<StoredPlan>,
    /// Why the plan pass that answered `completed` failed, when it did. Mutually exclusive with a
    /// same-generation `stored`, because a pass either produced a plan or did not.
    error: Option<String>,
    /// Apply requests booked, apply requests answered, and the last verdict.
    apply_requested: u64,
    apply_completed: u64,
    apply: Option<ApplyOutcome>,
    /// The apply the next pass must carry out, booked by the IPC task and taken by the core at the
    /// top of the pass. Held here rather than in a separate slot so booking the request and
    /// queueing the work it describes are one transition under one lock — a queued apply whose
    /// counter was not booked (or the reverse) is a client waiting on a verdict that never comes.
    queued_apply: Option<PassIntent>,
}

/// One computed plan, held for exactly as long as it is the latest one.
///
/// Kept in memory only. A plan is a statement about *right now* — both trees as they were minutes
/// ago — so it must not outlive the process that observed them: a token persisted across a restart
/// would authorise an apply against a tree nobody looked at. A client holding a token from a
/// previous run gets [`ApplyOutcome::Stale`], which is the correct answer.
#[derive(Clone)]
struct StoredPlan {
    token: String,
    computed_epoch_secs: u64,
    summary: PlanSummary,
    actions: Vec<PlannedAction>,
    cannot_sync: Vec<UnsyncableEntry>,
    /// The pair's disposal mode when this plan was computed — what its `LocalDelete` rows would do.
    local_disposal: LocalDisposal,
}

impl StoredPlan {
    /// This plan as a wire outcome, with its rows bounded.
    ///
    /// **Destructive rows come first and can never be truncated out.** The window is a rendering,
    /// and the rendering a user decides from: `files_at_risk` is safety copy, so a plan whose
    /// deletions fell off the end of the window would read as safe. Everything past the destructive
    /// rows is plan order, truncated at the cap. Truncating at all is only defensible because
    /// applying names the *token*, never the rows — the daemon executes the whole plan either way.
    fn outcome(&self, plan_seq: u64, limit: Option<usize>) -> PlanOutcome {
        let limit = limit
            .unwrap_or(PLAN_ACTIONS_DEFAULT_LIMIT)
            .clamp(1, PLAN_ACTIONS_MAX_LIMIT);
        let total = self.actions.len();
        let (destructive, rest): (Vec<&PlannedAction>, Vec<&PlannedAction>) = self
            .actions
            .iter()
            .partition(|action| action.action.is_destructive());
        let actions: Vec<PlannedAction> = destructive
            .into_iter()
            // `chain` after the partition, not `take(limit)` over the whole thing: the cap bounds
            // the reply, and a plan with more destructive rows than the cap is a plan whose every
            // dangerous row still has to be named.
            .chain(rest.into_iter().take(limit))
            .cloned()
            .collect();
        PlanOutcome::Computed(Box::new(ReviewedPlan {
            plan_seq,
            token: self.token.clone(),
            computed_epoch_secs: self.computed_epoch_secs,
            summary: self.summary.clone(),
            truncated: total > actions.len(),
            total,
            actions,
            cannot_sync: self.cannot_sync.clone(),
            local_disposal: self.local_disposal,
        }))
    }
}

/// What an in-flight pass has been asked to do with the plan it is about to compute.
///
/// A plan-only pass is deliberately **absent** from this enum: it does not run through
/// `reconcile_blocking` at all (see [`PairPass::plan_only_blocking`]), which is what makes its
/// inertness structural rather than a list of things each arm remembers not to do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PassIntent {
    /// An ordinary reconcile: plan and execute.
    Sync,
    /// Execute the plan the user reviewed, if this pass's own plan is still that plan (#100).
    Apply {
        apply_seq: u64,
        token: String,
        /// #192: drop the destructive rows, even the approved ones.
        skip_destructive: bool,
    },
}

/// [`ControlShared::activity`]'s contents: the wire-visible activity plus the internal staging
/// location of an in-flight download, kept out of the wire type so status replies can sample
/// bytes-so-far without leaking scratch paths to clients.
#[derive(Default)]
struct ActivityState {
    current: Option<SyncActivity>,
    download_scratch: Option<PathBuf>,
    /// The in-flight pass (#213). Held HERE rather than inside `current` because `begin_activity`
    /// replaces `current` on every phase change: a pass block stored there would restart its own
    /// clock three times per pass, which is the bug. Stamped onto every activity as it is built,
    /// so there is exactly one copy and one writer.
    pass: Option<PassProgress>,
}

/// Everything a status reply needs beyond the atomics above. Published by the daemon core via
/// [`PairPass::publish_status`] whenever its state changes.
#[derive(Clone)]
struct StatusSnapshot {
    pending_changes: usize,
    last_sync_epoch_secs: Option<u64>,
    last_error: Option<String>,
    last_plan_summary: Option<PlanSummary>,
    last_successful_sync_summary: Option<PlanSummary>,
    status_history: Vec<StatusHistoryEntry>,
    pending_deletions: Vec<PendingDeletion>,
    failed_items: Vec<FailedItem>,
    failed_item_count: usize,
    unsyncable: Vec<UnsyncableItem>,
    /// Cached corpus size (#207). Cached *here*, in the snapshot the IPC task reads, so answering
    /// `status` never touches SQLite — the whole point of publishing a snapshot is that status stays
    /// instant while a reconcile blocks the main task, and a per-reply `COUNT(*)`/`SUM()` would
    /// undo that (cf. #48, a per-pass Θ(dirs × index) walk that was too expensive).
    index_totals: Option<IndexTotals>,
    config: RunningConfigInfo,
    /// Pass-level history + today's byte totals, re-read from the index once per pass. Queried
    /// there rather than per status reply so a 2s GUI poll never hits SQLite.
    history: Option<PassHistory>,
}

impl ControlShared {
    fn new(config: RunningConfigInfo) -> Self {
        Self {
            auth: AtomicU8::new(auth_discriminant(AuthState::Unknown)),
            pair: PairShared::new(config),
        }
    }
}

impl PairShared {
    fn new(config: RunningConfigInfo) -> Self {
        Self {
            paused: AtomicBool::new(false),
            syncing: AtomicBool::new(false),
            plan_pass: AtomicBool::new(false),
            reconcile_seq: AtomicU64::new(0),
            force_full_walk: AtomicBool::new(false),
            reset_index: AtomicBool::new(false),
            snapshot: StdMutex::new(StatusSnapshot {
                pending_changes: 0,
                last_sync_epoch_secs: None,
                last_error: None,
                last_plan_summary: None,
                last_successful_sync_summary: None,
                status_history: Vec::new(),
                pending_deletions: Vec::new(),
                failed_items: Vec::new(),
                failed_item_count: 0,
                unsyncable: Vec::new(),
                index_totals: None,
                config,
                history: None,
            }),
            activity: StdMutex::new(ActivityState::default()),
            plan: StdMutex::new(PlanSlot::default()),
        }
    }
}

impl ControlShared {
    fn is_paused(&self) -> bool {
        self.pair.paused.load(Ordering::SeqCst)
    }

    fn plan_slot(&self) -> std::sync::MutexGuard<'_, PlanSlot> {
        self.pair.plan.lock().expect("plan slot lock")
    }

    /// Books a plan request and returns its number. The IPC task calls this *before* replying, so a
    /// client polling straight after its ack can never read the previous plan as its own answer.
    fn book_plan_request(&self) -> u64 {
        let mut slot = self.plan_slot();
        slot.requested += 1;
        slot.requested
    }

    /// Un-books the most recent plan request. For the one case where the ack was built and the pass
    /// then could not be scheduled (the daemon is shutting down): without this the counter stays
    /// ahead of `completed` for ever and every later poller reads `Computing` on a pass that will
    /// never run.
    fn unbook_plan_request(&self) {
        let mut slot = self.plan_slot();
        slot.requested = slot.requested.saturating_sub(1);
    }

    /// The generation a plan pass starting now will answer, or `None` when everything booked has
    /// already been answered (a duplicate `PlanNow` for a request an earlier pass covered — two
    /// clicks on `Check again` cost one remote walk, not two).
    fn claim_plan_pass(&self) -> Option<u64> {
        let slot = self.plan_slot();
        (slot.requested > slot.completed).then_some(slot.requested)
    }

    /// Seals a plan pass: publishes its result and marks every request up to `generation` answered.
    /// **Every** exit from a plan pass calls this, success or failure — a booked request that is
    /// never sealed is a poller that never stops.
    fn seal_plan_pass(&self, generation: u64, result: Result<StoredPlan, String>) {
        let mut slot = self.plan_slot();
        match result {
            Ok(plan) => {
                slot.stored = Some(plan);
                slot.error = None;
            }
            Err(error) => slot.error = Some(error),
        }
        slot.completed = slot.completed.max(generation);
    }

    /// Replaces the stored plan without booking a generation — what a **diverged apply** does with
    /// the plan it just computed (#100). The client asked to apply, not to plan, so nothing is
    /// answered here; the new plan is simply what a re-review will find.
    fn replace_stored_plan(&self, plan: StoredPlan) {
        let mut slot = self.plan_slot();
        slot.stored = Some(plan);
        slot.error = None;
    }

    /// Books an apply against `token`, or refuses it as stale. One stored plan at a time and latest
    /// wins, so a token that is not the current plan's authorises nothing — never a silent apply of
    /// something the user never saw.
    fn book_apply_request(&self, token: &str, skip_destructive: bool) -> Option<u64> {
        let mut slot = self.plan_slot();
        let current = slot.stored.as_ref()?;
        if current.token != token {
            return None;
        }
        slot.apply_requested += 1;
        slot.apply = None;
        slot.queued_apply = Some(PassIntent::Apply {
            apply_seq: slot.apply_requested,
            token: token.to_owned(),
            skip_destructive,
        });
        Some(slot.apply_requested)
    }

    /// Un-books an apply whose ack was built but whose pass could not be scheduled (the daemon is
    /// shutting down). Drops the queued work with the counter, so the two cannot disagree.
    fn unbook_apply_request(&self) {
        let mut slot = self.plan_slot();
        slot.apply_requested = slot.apply_requested.saturating_sub(1);
        slot.queued_apply = None;
    }

    /// What the pass starting now was asked to do. Consumed exactly once — a `take`, like every
    /// other latch the core reads at the top of a pass — so an apply is attempted by one pass and
    /// no later one inherits it.
    fn take_apply_request(&self) -> PassIntent {
        self.plan_slot()
            .queued_apply
            .take()
            .unwrap_or(PassIntent::Sync)
    }

    /// Seals an apply pass with its verdict. Same rule as [`Self::seal_plan_pass`].
    fn seal_apply_pass(&self, outcome: ApplyOutcome) {
        let mut slot = self.plan_slot();
        let generation = match &outcome {
            ApplyOutcome::Applied { apply_seq, .. }
            | ApplyOutcome::Diverged { apply_seq }
            | ApplyOutcome::Failed { apply_seq, .. }
            | ApplyOutcome::Scheduled { apply_seq } => *apply_seq,
            ApplyOutcome::Stale | ApplyOutcome::Paused | ApplyOutcome::Unknown => return,
        };
        slot.apply_completed = slot.apply_completed.max(generation);
        slot.apply = Some(outcome);
    }

    /// The current answer to [`ControlCommand::PlanResult`]: what the plan plane has for the newest
    /// request it has been asked about.
    fn plan_outcome(&self, limit: Option<usize>) -> PlanOutcome {
        let slot = self.plan_slot();
        if slot.requested > slot.completed {
            return PlanOutcome::Computing {
                plan_seq: slot.completed,
            };
        }
        if let Some(error) = &slot.error {
            return PlanOutcome::Failed {
                plan_seq: slot.completed,
                error: error.clone(),
            };
        }
        match &slot.stored {
            Some(plan) => plan.outcome(slot.completed, limit),
            None => PlanOutcome::Absent,
        }
    }

    fn apply_outcome(&self) -> Option<ApplyOutcome> {
        self.plan_slot().apply.clone()
    }

    fn auth_state(&self) -> AuthState {
        auth_from_discriminant(self.auth.load(Ordering::SeqCst))
    }

    /// Records a verdict about the Proton session, returning `true` when it actually changed.
    ///
    /// Both writers call this and neither may pass [`AuthState::Unknown`]: this is where evidence
    /// lands, and "I learned nothing" is the absence of a call, not a call. A caller with an
    /// unclassified failure in hand must therefore leave the state alone rather than reset it —
    /// which is the point, because the alternative reads a timeout as proof of being signed in.
    fn record_auth_state(&self, state: AuthState) -> bool {
        debug_assert!(
            state != AuthState::Unknown,
            "`Unknown` is the absence of evidence; do not publish it as a verdict"
        );
        let previous = self.auth.swap(auth_discriminant(state), Ordering::SeqCst);
        auth_from_discriminant(previous) != state
    }

    /// Starts a new activity phase, replacing whatever was current (and dropping any stale
    /// download staging location from the previous action).
    fn begin_activity(&self, mut activity: SyncActivity) {
        let mut state = self.pair.activity.lock().expect("activity lock");
        activity.pass = state.pass.clone();
        state.current = Some(activity);
        state.download_scratch = None;
    }

    /// Opens the pass block every subsequent activity carries (#213).
    fn begin_pass_progress(&self, started_epoch_secs: u64, kind: PassKind) {
        self.begin_pass_progress_named(started_epoch_secs, kind.as_str());
    }

    /// [`Self::begin_pass_progress`] for the one pass that has no [`PassKind`]: a plan-only pass
    /// writes no `sync_passes` row, so it has no *stored* kind — but the live block must still say
    /// what is running, or a Plan screen reading `files_scanned` cannot tell its own rehearsal from
    /// a sync pass that happened to start at the same moment (#209). `PassProgress::kind` is a
    /// documented open token, so [`PLAN_PASS_KIND`] degrades to "rendered verbatim" on any client
    /// that does not know it.
    fn begin_plan_progress(&self, started_epoch_secs: u64) {
        self.begin_pass_progress_named(started_epoch_secs, PLAN_PASS_KIND);
    }

    fn begin_pass_progress_named(&self, started_epoch_secs: u64, kind: &str) {
        let mut state = self.pair.activity.lock().expect("activity lock");
        state.pass = Some(PassProgress {
            started_epoch_secs,
            changes: None,
            kind: kind.to_owned(),
            uploaded_files: 0,
            downloaded_files: 0,
            uploaded_bytes: 0,
            downloaded_bytes: 0,
        });
        Self::restamp_pass(&mut state);
    }

    /// Corrects the pass block once the daemon knows which strategy it is running, or how many
    /// changes the plan holds. No-op before `begin_pass_progress`.
    fn update_pass_progress(&self, update: impl FnOnce(&mut PassProgress)) {
        let mut state = self.pair.activity.lock().expect("activity lock");
        if let Some(pass) = state.pass.as_mut() {
            update(pass);
        }
        Self::restamp_pass(&mut state);
    }

    /// Copies the pass block onto the current activity, so a change is visible without waiting for
    /// the next phase.
    fn restamp_pass(state: &mut ActivityState) {
        let pass = state.pass.clone();
        if let Some(current) = state.current.as_mut() {
            current.pass = pass;
        }
    }

    /// Applies `update` to the current activity, creating one with `phase` first if none is
    /// current or the phase changed (which also resets `since_epoch_secs` to now, so elapsed
    /// times are per-phase). Used by the high-frequency walk/scan callbacks.
    fn update_activity(&self, phase: &str, update: impl FnOnce(&mut SyncActivity)) {
        let mut state = self.pair.activity.lock().expect("activity lock");
        let same_phase = matches!(&state.current, Some(current) if current.phase == phase);
        if !same_phase {
            let pass = state.pass.clone();
            state.current = Some(SyncActivity {
                pass,
                ..new_activity(phase)
            });
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
        let mut state = self.pair.activity.lock().expect("activity lock");
        state.download_scratch = Some(scratch_dir.to_path_buf());
    }

    /// Clears all live activity (the pass is over; `syncing` is about to read false).
    fn clear_activity(&self) {
        let mut state = self.pair.activity.lock().expect("activity lock");
        *state = ActivityState::default();
    }

    /// Snapshot of the current activity for a status reply. Purely in-memory — a download's
    /// live `bytes_done` is filled in separately by [`Self::response_with_sampled_activity`],
    /// so building a plain reply never touches the filesystem.
    ///
    /// The legacy `transfer` mirror is derived here rather than stored, so it cannot disagree with
    /// the list it mirrors (#211).
    fn activity_for_response(&self) -> Option<SyncActivity> {
        let mut activity = self
            .pair
            .activity
            .lock()
            .expect("activity lock")
            .current
            .clone();
        if let Some(activity) = activity.as_mut() {
            activity.derive_legacy_transfer();
        }
        activity
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
            let state = self.pair.activity.lock().expect("activity lock");
            (state.current.clone(), state.download_scratch.clone())
        };
        response.activity = current;
        if let Some(activity) = &mut response.activity {
            // Into the LIST first, then derive the mirror from it. The other order publishes a
            // mirror one sample stale, and writing the sample into the mirror instead would leave
            // every #211-aware client reading a list that never grows.
            if let Some(scratch_dir) = scratch
                && let Some(transfer) = activity
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.is_active() && transfer.direction == "download")
            {
                transfer.bytes_done = staged_bytes(&scratch_dir).await;
            }
            activity.derive_legacy_transfer();
        }
        response
    }

    fn is_syncing(&self) -> bool {
        self.pair.syncing.load(Ordering::SeqCst)
    }

    /// Whether a pass that will bump [`Self::reconcile_seq`] is in flight — i.e. a real reconcile,
    /// not a rehearsal. The one question the `syncnow`/`resync` acks must ask, because their
    /// clients count passes by that sequence.
    fn a_counted_pass_is_running(&self) -> bool {
        self.is_syncing() && !self.pair.plan_pass.load(Ordering::SeqCst)
    }

    fn response(&self, message: &str) -> ControlResponse {
        let paused = self.is_paused();
        let syncing = self.is_syncing();
        let snapshot = self
            .pair
            .snapshot
            .lock()
            .expect("control snapshot lock")
            .clone();
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
            reconcile_seq: self.pair.reconcile_seq.load(Ordering::SeqCst),
            pending_changes: snapshot.pending_changes,
            message: message.to_owned(),
            last_sync_epoch_secs: snapshot.last_sync_epoch_secs,
            last_error: snapshot.last_error,
            last_plan_summary: snapshot.last_plan_summary,
            last_successful_sync_summary: snapshot.last_successful_sync_summary,
            status_history: snapshot.status_history,
            pending_deletions: snapshot.pending_deletions,
            failed_items: snapshot.failed_items,
            failed_item_count: snapshot.failed_item_count,
            history: snapshot.history.clone(),
            // Only an `activity` request answers with the per-file feed; every other reply omits
            // it rather than paying for the query.
            file_history: None,
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
            unsyncable: snapshot.unsyncable,
            // A clone of an `Option<IndexTotals>` (two `u64`s, `Copy`) — the reply path never
            // queries the index. See `StatusSnapshot::index_totals`.
            index_totals: snapshot.index_totals,
            // Only a `list` reply carries a listing; every other one omits it rather than
            // implying an empty folder.
            listing: None,
            // Only the plan-family replies carry these. A reviewed plan can be tens of thousands of
            // rows and the GUI polls `status` every two seconds, so putting them on every reply
            // would make the cheap verb the expensive one.
            plan: None,
            apply: None,
            auth: self.auth_state(),
        }
    }

    fn metrics(&self) -> MetricsSnapshot {
        let paused = self.is_paused();
        let snapshot = self
            .pair
            .snapshot
            .lock()
            .expect("control snapshot lock")
            .clone();
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
            failed_items: snapshot.failed_items,
            failed_item_count: snapshot.failed_item_count,
            // Deliberately NOT mirrored into the sidecar: the pass history is already durable in
            // the index's own tables, and a second copy on disk is a second thing to keep in step.
            // The sidecar keeps counts (`status_history_entries`), not lists.
            unsyncable: snapshot.unsyncable,
            index_totals: snapshot.index_totals,
        }
    }
}

/// [`ControlShared::auth`] holds an [`AuthState`] as a `u8`, since there is no atomic for the enum
/// itself. The two functions below are the only mapping; keeping them adjacent (and total, with no
/// fall-through that invents a verdict) is what stops a stray discriminant from reading as
/// "signed in".
fn auth_discriminant(state: AuthState) -> u8 {
    match state {
        AuthState::Unknown => 0,
        AuthState::SignedIn => 1,
        AuthState::SignedOut => 2,
    }
}

fn auth_from_discriminant(value: u8) -> AuthState {
    match value {
        1 => AuthState::SignedIn,
        2 => AuthState::SignedOut,
        // Everything else, `0` included: no verdict. A value this function cannot name is exactly
        // the case that must not become an all-clear.
        _ => AuthState::Unknown,
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
        transfers: Vec::new(),
        transfers_remaining: None,
        // Derived from `transfers` on the way out (`SyncActivity::derive_legacy_transfer`); the
        // core never sets it.
        transfer: None,
        since_epoch_secs: Some(current_epoch_secs()),
        // Stamped by `begin_activity` from the shared pass block, never built here: the pass
        // outlives the phase (#213).
        pass: None,
    }
}

/// This pass's transfers and where they sit in the plan, so the queue behind the row in flight is a
/// slice rather than a forward scan (#211): a plan with 100k downloads followed by 50k deletes
/// would otherwise re-scan its own tail once per action.
///
/// One borrow rather than three more arguments on two already-long executor signatures — and
/// `positions.len()` is the pass's transfer total, so nothing counts that a second time.
struct TransferQueue<'a> {
    plan: &'a [PlannedAction],
    /// Indices into `plan` of every `Upload`/`Download`, in plan order.
    positions: &'a [usize],
    local_files: &'a HashMap<PathBuf, LocalFileState>,
}

impl TransferQueue<'_> {
    fn total(&self) -> usize {
        self.positions.len()
    }

    /// The transfer queue as the wire carries it: the rows in flight, then the next planned
    /// transfers, capped at [`TRANSFERS_REPORTED`] — plus how many transfers this pass still has
    /// left, counted past the cap.
    ///
    /// `started` is how many transfers the pass has begun (the spinner's `transfer_index` after its
    /// increment), so `positions[started..]` is exactly what has not been reached yet and the
    /// remainder is that plus the rows still running. Counted from this cursor and never from the
    /// committed per-direction counters: those fold over a different action set (a conflict sidecar
    /// is a download that is not a `Download`), so subtracting one from the other would drift.
    ///
    /// A queued upload knows its size — the scan measured it — and a queued download does not, for
    /// the same reason an active one does not.
    fn window(
        &self,
        started: usize,
        active: Vec<TransferActivity>,
    ) -> (Vec<TransferActivity>, u64) {
        // A row's weight is the number of files it covers, not 1: a batched download is one row over
        // a whole chunk, and counting it as one transfer would under-report the remainder by the
        // rest of its batch. Same arithmetic the client uses to size `+n more`.
        let in_flight: u64 = active.iter().map(|row| row.files.unwrap_or(1)).sum();
        let remaining = (self.total() - started.min(self.total())) as u64 + in_flight;
        let mut window = active;
        for position in self.positions.iter().skip(started) {
            if window.len() >= TRANSFERS_REPORTED {
                break;
            }
            let action = &self.plan[*position];
            let (direction, bytes_total) = match action.action {
                SyncAction::Upload => (
                    "upload",
                    self.local_files
                        .get(&action.path)
                        .map(|local| local.file_size),
                ),
                _ => ("download", None),
            };
            window.push(TransferActivity::queued(
                direction,
                action.path.clone(),
                bytes_total,
            ));
        }
        (window, remaining)
    }
}

/// An `executing` activity, which **always carries its transfer window**.
///
/// The one constructor for the phase, so the window cannot be left off. Publishing the phase and
/// then filling the window in a second `update_activity` is two lock acquisitions with an
/// observable state between them: a status poll landing there reads `executing` with an empty list
/// and `transfers_remaining: None`, which is the value that field documents as *not executing* —
/// and blinks the transfer rows for the length of a CLI spawn. Both publish sites go through here.
fn executing_activity(
    detail: String,
    action_index: usize,
    action_total: u64,
    window: (Vec<TransferActivity>, u64),
) -> SyncActivity {
    let (transfers, remaining) = window;
    SyncActivity {
        detail: Some(detail),
        action_index: Some(action_index as u64 + 1),
        action_total: Some(action_total),
        transfers,
        transfers_remaining: Some(remaining),
        ..new_activity(PHASE_EXECUTING)
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
    /// Run a plan-only pass (#100/#192/#209). On the core for the same reason `SyncNow` is: it is a
    /// full local walk plus an O(folders) remote walk, which must never run on the IPC task.
    PlanNow,
    Shutdown,
}

/// The `--dry-run` report. Returns the whole [`DryRunReport`] rather than the bare plan because
/// `summary.matched_files` is a planner fact no plan row carries (#242).
pub fn preview_plan(config: &DaemonConfig) -> AppResult<DryRunReport> {
    let client = ProtonDriveClient::with_command_policy(
        config.proton_cli.clone(),
        command_policy_from_config(config),
    );
    preview_plan_with_client(config, &client)
}

pub fn preview_plan_with_client(
    config: &DaemonConfig,
    proton: &impl ProtonClient,
) -> AppResult<DryRunReport> {
    // Through the *same* projection the daemon's constructor uses, so a preview and a real pass can
    // never disagree about which keys describe the tree (ADR 0005 §2). A preview rehearses ONE pair
    // — the default one, since this path predates the selector `--pair` will add.
    let (_, pair_config) = config.clone().into_parts();
    let scan_options = scan_options_from_config(&pair_config)?;
    let base_records = load_existing_index(&pair_config.db_path)?;
    build_plan_report(
        &pair_config,
        proton,
        &scan_options,
        base_records,
        None,
        |_phase| {},
    )
}

/// Scan → list → plan, and **the one definition of what a rehearsal is**.
///
/// Both plan producers run it: the one-shot child (`preview_plan_with_client`, which has no daemon
/// and reads the index off disk) and the daemon's own plan verb (#100/#209, which has an open
/// connection and a live activity surface). They differ only in where the baseline comes from and
/// in whether anything is watching, which is exactly what the two injected seams cover — a second
/// copy of this body would be two answers to "what would change", and the plan token would then
/// depend on which one you asked.
///
/// **A full-tree remote walk, always.** A rehearsal is reviewed and then authorised, so it is built
/// against ground truth rather than against `reconstruct_remote(base ⊕ delta)`; that is also what
/// keeps it structurally incapable of consuming the event delta, which is the one thing a plan pass
/// must never do (advancing the cursor past changes nothing applied would orphan them).
fn build_plan_report(
    config: &PairConfig,
    proton: &impl ProtonClient,
    scan_options: &ScanOptions,
    base_records: HashMap<PathBuf, FileRecord>,
    observer: Option<crate::index::ScanObserver<'_>>,
    mut phase: impl FnMut(&'static str),
) -> AppResult<DryRunReport> {
    info!(
        local_root = %config.local_root.display(),
        remote_root = %config.remote_root.display(),
        "building dry-run sync plan"
    );
    phase(PHASE_SCANNING_LOCAL);
    // `scan_local_tree`, not one of the entity-only entry points: the walk's own drops are the
    // half #315 needs, and taking both from one walk is what makes them describe one moment.
    let local_scan = scan_local_tree(&config.local_root, scan_options, &base_records, observer)?;
    phase(PHASE_LISTING_REMOTE);
    let (remote_entities, remote_root_missing) =
        load_remote_entities(proton, &config.remote_root, scan_options)?;
    let mut base_index = filter_base_index(base_records, scan_options);
    if remote_root_missing {
        base_index.clear();
    }
    let mut outcome = plan_sync_entities_with_stats(
        &local_scan.entities,
        &remote_entities,
        &base_index,
        &config.conflict_naming,
    );
    prepend_remote_root_creation_if_missing(&mut outcome.actions, remote_root_missing);
    info!(
        planned_actions = outcome.actions.len(),
        matched_files = outcome.matched_files,
        cannot_sync = local_scan.unsyncable.len(),
        "dry-run sync plan computed"
    );
    Ok(DryRunReport::from_outcome(outcome, local_scan.unsyncable))
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
        // The one split (see [`DaemonConfig::into_parts`]): from here on, "the local root" is the
        // *pair's* local root and nothing on the daemon holds a second copy of it.
        let (process, pair_config) = config.into_parts();
        fs::create_dir_all(&pair_config.local_root)?;
        if let Some(parent) = pair_config.db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = process.socket_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        // Ensure the lockfile's directory (the `.sync` state dir by default) exists before
        // acquiring the lock, so a first-ever run on a fresh root does not fail here.
        if let Some(parent) = pair_config.lockfile_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let lock_guard = LockGuard::acquire(&pair_config.lockfile_path)?;
        // Then the user-global lock, so a second daemon started for a *different* root (its per-root
        // lock above would succeed) still cannot run: every daemon shells the same `proton-drive`
        // CLI, whose shared SQLite cache/session store is not concurrency-safe (#23). A crashed
        // daemon leaves only an unlocked file behind, which `acquire` reuses — restart still works.
        let global_lock_guard = LockGuard::acquire(&process.global_lock_path).map_err(|error| {
            boxed_error(format!(
                "cannot start: {error}. Only one proton-syncd may run per user account — every \
                 daemon shells the same proton-drive CLI, whose SQLite cache and session store are \
                 not safe for concurrent use (#23). Stop the other daemon first."
            ))
        })?;
        let connection = open_database(&pair_config.db_path)?;
        let scan_options = scan_options_from_config(&pair_config)?;
        let status_history_path = status_history_path(&pair_config.db_path);
        let metrics_path = metrics_path(&pair_config.db_path);
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
        // Standing across restarts, because an incremental pass cannot re-derive it (see
        // `record_unsyncable`). A read failure degrades to empty: the next full-tree walk rebuilds
        // the list, and nothing but display depends on it.
        let unsyncable = load_unsyncable_items(&connection).unwrap_or_else(|error| {
            warn!(%error, "ignoring an unreadable unsyncable-items list");
            Vec::new()
        });
        // Captured by value so the closure re-reads the keyring at runtime without holding the
        // config. Only the feature flag is needed — `CliKeyringSession::from_cli_keyring` sources
        // the session from the keyring itself. Quiet on failure (unlike the startup
        // `build_event_source`, which warns once): this runs every degraded pass.
        let events_driven = pair_config.events_driven;
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
            local_root: pair_config.local_root.clone(),
            remote_root: pair_config.remote_root.clone(),
            db_path: pair_config.db_path.clone(),
        }));
        let mut daemon = Self {
            process,
            pairs: vec![PairRuntime {
                config: pair_config,
                connection,
                scan_options,
                status_history_path,
                metrics_path,
                pending_changes: BTreeSet::new(),
                authored_writes: HashSet::new(),
                force_local_rescan: false,
                last_sync: None,
                last_error: None,
                last_plan_summary: None,
                last_successful_sync_summary: None,
                status_history,
                incremental_passes_since_full_scan,
                is_first_reconcile: true,
                warm_starts_since_full_walk,
                event_scope_declined: None,
                pending_deletions: Vec::new(),
                last_failed_items: Vec::new(),
                last_failed_item_count: 0,
                unsyncable,
                index_totals: None,
                // Stale from the start: the index already has contents at boot (a warm start
                // replays against them), so the first refresh must run rather than publish `None`
                // until the first pass that happens to plan something.
                index_totals_stale: true,
                reported_unsyncable: HashSet::new(),
                pass_log: PassLog::new(current_epoch_secs()),
                pass_history: None,
                pass_intent: PassIntent::Sync,
                apply_report: None,
                _lock_guard: lock_guard,
            }],
            proton: Arc::new(proton),
            shared,
            ipc_io_timeout: IPC_IO_TIMEOUT,
            browse_gate_wait: BROWSE_GATE_WAIT,
            events_poll_interval: EVENTS_POLL_INTERVAL,
            event_source,
            event_source_factory,
            session_declined: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            _global_lock_guard: global_lock_guard,
        };
        // Populate the published history before the first pass, so a client polling a
        // just-started daemon already sees the last full sweep from a previous run.
        daemon.pass().refresh_pass_history();
        daemon.pass().refresh_index_totals();
        daemon.pass().publish_status();
        daemon.pass().write_metrics_snapshot()?;
        Ok(daemon)
    }

    /// `C: 'static` because the control-socket task holds an `Arc<C>` for the `list` verb (#99),
    /// and a spawned task may outlive this frame.
    pub async fn run(mut self) -> AppResult<()>
    where
        C: 'static,
    {
        info!(
            local_root = %self.pair().config.local_root.display(),
            remote_root = %self.pair().config.remote_root.display(),
            socket_path = %self.process.socket_path.display(),
            "starting daemon"
        );
        let cancel_flag = Arc::clone(&self.cancel_flag);
        {
            // The only `&mut` the client ever needs, and it is taken *before* the control-socket
            // task below is handed its own `Arc` clone — so the reference count is still one and
            // this cannot fail. (`Daemon::run` consumes `self`, and nothing between construction
            // and here clones the client.)
            let proton = Arc::get_mut(&mut self.proton).expect(
                "the proton client is not shared until the control-socket task is spawned below",
            );
            proton.install_cancel_flag(Arc::clone(&cancel_flag));
            // Live-progress plumbing: the client reports per-folder walk progress and download
            // staging locations straight into `ControlShared`, so status replies stay alive while
            // this task is blocked inside a reconcile.
            proton.install_progress_sink(Arc::new(SharedProgressSink {
                shared: Arc::clone(&self.shared),
            }));
        }
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

        let listener = bind_listener(&self.process.socket_path).await?;
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
        // THREE per-pair values reach the control plane, and each becomes a lookup in phase 3 rather
        // than a second copy: the approvals connection (one per pair — 2N handles, no schema change,
        // ADR 0005 §3), the metrics sidecar path, and the `list` verb's relative frame. They are
        // pair 0's here because there is one pair; the plane itself is per-process and stays so.
        let approvals_connection = open_database(&self.pair().config.db_path)?;
        let ipc_task = tokio::spawn(serve_control_socket(
            listener,
            Arc::new(ControlPlane {
                shared: Arc::clone(&self.shared),
                approvals: tokio::sync::Mutex::new(approvals_connection),
                loop_tx,
                io_timeout: self.ipc_io_timeout,
                metrics_path: self.pair().metrics_path.clone(),
                cancel_flag: Arc::clone(&cancel_flag),
                browse: BrowseContext {
                    // The SAME client the main loop uses, not a second one built from the same
                    // config: one client is one `proton::CliGate`, and two gates serialize
                    // nothing.
                    proton: Arc::clone(&self.proton),
                    remote_root: self.pair().config.remote_root.clone(),
                    gate_wait: self.browse_gate_wait,
                },
            }),
        ));
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel();
        let mut watcher = build_watcher(watch_tx)?;
        // One watcher, N watched roots once phase 4 lands (`watch` is called per root) — and
        // `handle_fs_event` is where an event is routed back to its owning pair.
        watcher.watch(&self.pair().config.local_root, RecursiveMode::Recursive)?;

        // Per-pair cadence: phase 4 replaces this single interval with the due queue, since two
        // pairs may want different `scan_interval`s and their passes must still be serialized.
        let mut interval = tokio::time::interval(self.pair().config.scan_interval);
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

        // The scheduled full sweep (#193). Created ONCE and `reset` after each firing rather than
        // rebuilt in the arm: a `sleep` constructed inside `select!` is a new deadline every time
        // any other arm wakes the loop, so a busy watcher would push the sweep back for ever and
        // the bug would only show on a machine with file activity.
        //
        // A daemon with no schedule still needs a future to pin, so it gets a far one and the
        // precondition keeps it from ever being polled. It is deliberately not `Duration::MAX`,
        // which overflows tokio's timer.
        let mut sweep_due = self.next_full_sweep_in();
        let mut full_sweep =
            std::pin::pin!(tokio::time::sleep(sweep_due.unwrap_or(NO_SCHEDULE_PARK)));

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
                                let pending_before = self.pair().pending_changes.len();
                                let outcome =
                                    tokio::task::block_in_place(|| self.handle_fs_event(event));
                                if let Err(error) = outcome {
                                    warn!(%error, "failed to process filesystem event");
                                }
                                if self.pair().pending_changes.len() != pending_before {
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
                            LoopCommand::PlanNow => self.plan_now().await,
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
                        if self.pair().config.events_driven && self.event_source.is_some() => {
                        self.reconcile_if_needed().await?;
                    }
                    // #193. Latch first, then reconcile, then re-arm from a fresh clock read — so a
                    // pass that runs long, or is skipped by a pause, cannot make the next deadline
                    // drift by however long it took. The latch itself survives a pause for the
                    // reason `resync` does: it describes the next pass, not this moment.
                    // GATED ON A RESOLVED DEADLINE, not on the key being set. A schedule that
                    // parses but resolves to no instant parks on the 24h sentinel, and gating on
                    // `full_scan_schedule.is_some()` would have let that sentinel fire — a daily
                    // sweep from a daemon this file documents as inert.
                    _ = full_sweep.as_mut(), if sweep_due.is_some() => {
                        self.latch_scheduled_full_sweep();
                        self.reconcile_if_needed().await?;
                        // Re-armed from a fresh clock read AFTER the pass, so a long sweep cannot
                        // make the next deadline drift by however long it took.
                        sweep_due = self.next_full_sweep_in();
                        full_sweep.as_mut().reset(
                            tokio::time::Instant::now() + sweep_due.unwrap_or(NO_SCHEDULE_PARK),
                        );
                    }
                    _ = &mut shutdown => {
                        break;
                    }
                }
            }
        }

        ipc_task.abort();
        remove_control_socket(&self.process.socket_path);
        info!("daemon stopped");
        Ok(())
    }

    /// How long until the next scheduled full sweep, or `None` when nothing is scheduled.
    ///
    /// **The whole run-loop arm is this call plus a latch**, deliberately: the arithmetic is
    /// [`crate::schedule`]'s and pure, the daylight-saving choice is `resolve_local`'s and pure, and
    /// what is left here is reading the clock. Nothing about *when* a sweep happens is decided in
    /// the `select!`, which is what makes it testable without wall clock.
    ///
    /// `None` for a schedule whose instant cannot be resolved at all — see `resolve_local`'s bound —
    /// and for one whose next eight occurrences have all already passed in real time. The run loop
    /// gates its arm on this answer rather than on the key being set, so `None` really is the same
    /// inert state a daemon with no schedule is in.
    fn next_full_sweep_in(&self) -> Option<Duration> {
        let schedule = self.pair().config.full_scan_schedule?;
        let now = Local::now();
        let mut cursor = now.naive_local();
        // AN OCCURRENCE THAT IS LATER ON THE CLOCK CAN BE EARLIER IN REAL TIME, and this loop is
        // the whole reason there is no "fire immediately" fallback here.
        //
        // `next_due` is strictly after `cursor` in NAIVE time, which is what a schedule is written
        // in. The map from naive to instant is not monotonic across a daylight-saving fold: during
        // the repeated hour, `Local::now()` may already be past the earlier mapping of a naive time
        // that is still in the clock's future. Subtracting then gives a negative duration.
        //
        // The obvious fallback — clamp to zero and let it fire — is a **spin**: the timer arm
        // re-arms by calling this again, the mapping is still non-monotonic, and it recomputes the
        // same zero for the rest of the window. Each turn latches `force_full_walk` and runs an
        // O(folders) sweep; with the daemon paused, `reconcile_if_needed` returns at once and it is
        // a bare 100%-CPU loop. Found by adversarial review, latent rather than live only because
        // `chrono 0.4.45` orders its ambiguous fields the opposite way to its own documentation —
        // so a chrono upgrade would have switched it on.
        //
        // Instead: if an occurrence has already passed in real time, ask for the one after it. The
        // bound is a backstop rather than a limit the arithmetic needs — one fold can swallow at
        // most one occurrence — and answering `None` there is the same "not armed" state as an
        // unresolvable schedule, which the caller checks rather than assumes.
        for _ in 0..8 {
            let due = schedule.next_due(cursor);
            let instant =
                crate::schedule::resolve_local(due, |naive| Local.from_local_datetime(&naive))?;
            if let Ok(delay) = (instant - now).to_std() {
                return Some(delay);
            }
            cursor = due;
        }
        None
    }

    /// A scheduled moment arrived: the next pass is a full sweep.
    ///
    /// **It sets the same latch `resync` sets, and that is the whole design** (#193). All three
    /// full-sweep triggers mean the identical thing to a pass, so a schedule that fires while a
    /// safety net has already latched one is one sweep — because the latch is a `bool`, not because
    /// anything arbitrates. That is why the maintainer decision's "does not queue a second, does
    /// not suppress one either" costs no code: reusing the seam is what delivers it.
    fn latch_scheduled_full_sweep(&self) {
        info!("scheduled full sweep is due; the next pass will walk the whole tree");
        self.shared
            .pair
            .force_full_walk
            .store(true, Ordering::SeqCst);
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
        // The error carries no path, so with N pairs this must set the flag on *every* one of them:
        // an inotify overflow means events were lost somewhere, and that is the only fail-safe
        // reading available (ADR 0005 §5).
        self.pair_mut().force_local_rescan = true;
    }

    /// Test-only convenience wrapper over the free [`apply_approval_command`], which production
    /// code runs on the IPC task with its own connection and the published pending list.
    #[cfg(test)]
    fn apply_approval_command(&self, selector: Option<&str>, approve: bool) -> AppResult<String> {
        apply_approval_command(
            &self.pair().connection,
            &self.pair().pending_deletions,
            selector,
            false,
            approve,
            None,
        )
    }

    /// Test-only convenience wrapper over the free [`apply_keep_command`].
    #[cfg(test)]
    fn apply_keep_command(&self, selector: Option<&str>) -> AppResult<(String, bool)> {
        apply_keep_command(
            &self.pair().connection,
            &self.pair().pending_deletions,
            selector,
            true,
        )
    }

    /// The pair a pass is about, and the **one** place that choice is made.
    ///
    /// Phase 2 has exactly one pair, so this is `pairs[0]`. Phase 4 replaces the choice with the due
    /// queue (explicit requests ahead of timer-due pairs, earliest `next_due` among the rest) and
    /// nothing else about a pass changes — which is the whole point of the borrowed view.
    fn pass(&mut self) -> PairPass<'_, C> {
        // Destructured rather than `&mut self.pairs[0]` beside `self.shared`: field-by-field is what
        // the borrow checker proves disjoint.
        //
        // **Exhaustive, with no `..`**, for the same reason [`DaemonConfig::into_parts`] is: adding a
        // field to `Daemon` should make the compiler ask "does a pass need this?" rather than
        // silently answer no. The `_` bindings are the recorded answers — the socket and the global
        // lock describe the process's own lifetime, and the three cadences belong to the run loop
        // that schedules passes rather than to a pass.
        let Self {
            pairs,
            proton,
            shared,
            cancel_flag,
            event_source,
            event_source_factory,
            session_declined,
            events_poll_interval,
            process: _,
            ipc_io_timeout: _,
            browse_gate_wait: _,
            _global_lock_guard: _,
        } = self;
        PairPass {
            pair: &mut pairs[0],
            proton,
            shared,
            cancel_flag,
            event_source,
            event_source_factory,
            session_declined,
            events_poll_interval: *events_poll_interval,
        }
    }

    /// The pair itself, for the loop-level reads that are not a pass (the watched root, the
    /// cadence, the sidecar paths). Same singular assumption as [`Self::pass`].
    fn pair(&self) -> &PairRuntime {
        &self.pairs[0]
    }

    fn pair_mut(&mut self) -> &mut PairRuntime {
        &mut self.pairs[0]
    }

    /// One reconcile pass. Phase 4 changes which pair `pass()` hands over, not this call.
    fn reconcile_blocking(&mut self) -> AppResult<PassOutcome> {
        self.pass().reconcile_blocking()
    }

    /// Routes a filesystem event to the pair that owns it.
    ///
    /// One watcher with N watched roots once phase 4 lands, so this is where an absolute path is
    /// matched to its pair by longest prefix — unambiguous because ADR 0005 §2 rule 4 forbids
    /// nesting. Routing must happen *before* the pair's filters run, or an event under pair A's root
    /// would be tested against pair B's globs.
    fn handle_fs_event(&mut self, event: Event) -> AppResult<()> {
        self.pass().handle_fs_event(event)
    }

    fn publish_status(&mut self) {
        self.pass().publish_status();
    }

    /// Test-only convenience: publish and build a status reply exactly as the IPC task would.
    /// Production replies are built by the IPC task itself from [`ControlShared`].
    #[cfg(test)]
    fn status_response(&mut self, message: &str) -> ControlResponse {
        self.publish_status();
        self.shared.response(message)
    }

    fn is_paused(&self) -> bool {
        self.shared.is_paused()
    }

    async fn reconcile_if_needed(&mut self) -> AppResult<()> {
        if self.is_paused() {
            self.discard_queued_apply_for_pause();
            return Ok(());
        }
        match self.reconcile().await {
            // Not silence: `execute_plan_and_commit` already warned per item and once with the
            // total, and the outcome is on every status reply. Logging it again here would be a
            // second, differently-worded report of the same pass.
            Ok(PassOutcome::Clean | PassOutcome::Partial { .. }) => {}
            Err(error) => error!(%error, "scheduled reconciliation failed"),
        }
        Ok(())
    }

    async fn reconcile(&mut self) -> AppResult<PassOutcome> {
        // Flag the pass for status replies before blocking this task; the IPC task keeps
        // serving (and reporting `syncing`) for the whole duration.
        self.shared.pair.syncing.store(true, Ordering::SeqCst);
        let result = tokio::task::block_in_place(|| self.reconcile_blocking());
        self.shared.pair.syncing.store(false, Ordering::SeqCst);
        // The pass is over either way; never let a stale "downloading X" outlive it.
        self.shared.clear_activity();
        result
    }

    /// Drops an apply that was booked and then overtaken by a pause, and seals it as failed.
    ///
    /// **A latch is the wrong shape for a destructive request.** `resync` and `reset_index` survive
    /// a pause because they are latches for what the *next* pass should do, and the next pass is
    /// the user's own resume. An apply is different in two ways: it performs deletions, and it
    /// authorises a plan the user reviewed minutes ago. Letting it sit through a pause would fire a
    /// destructive side effect on some later interval tick, hours after the user paused precisely
    /// to stop things happening — and `reconcile_if_needed`'s early return means nothing would seal
    /// it in the meantime either.
    ///
    /// Sealed as `Failed` rather than silently dropped: a booked request that nothing answers is a
    /// client polling for ever, and "the apply did not happen" is exactly what `Failed` says. The
    /// plan itself is untouched, so resuming and applying the same token again just works.
    fn discard_queued_apply_for_pause(&mut self) {
        let PassIntent::Apply { apply_seq, .. } = self.shared.take_apply_request() else {
            return;
        };
        info!("syncing was paused before the scheduled apply ran; it was not applied");
        self.shared.seal_apply_pass(ApplyOutcome::Failed {
            apply_seq,
            error: "syncing was paused before the apply ran; nothing was applied".to_owned(),
        });
    }

    /// One plan-only pass (#100/#192/#209): the rehearsal, run on the main loop so the existing
    /// [`SyncActivity`] describes it and no second progress channel has to exist.
    ///
    /// **This pass observes and consumes NOTHING**, which is the single most dangerous thing about
    /// it: it runs the same scan and the same walk as a real pass, so anything it consumed would be
    /// consumed *without* the side effects that justify consuming it. The whole family, and the one
    /// reason behind all of it — a rehearsal that spent evidence would leave the real pass unable to
    /// re-derive what it was rehearsing:
    ///
    /// * **the event cursor** — never read and never written here; the pass does not even resolve an
    ///   event scope, because it always full-walks the remote rather than replaying a delta
    ///   (`build_plan_report`). An incremental plan *would* consume the delta to reconstruct its
    ///   map, and advancing the cursor past changes nothing applied orphans them for ever;
    /// * **`pending_changes` / `pending_deletions`** — not drained: only a pass that acted on them
    ///   may clear them;
    /// * **`is_first_reconcile` / `force_local_rescan`** — not cleared: the local scan a plan pass
    ///   runs proves nothing was *synced*, which is the floor those flags hold;
    /// * **the warm-start counter** — not touched: it counts full walks that produced a baseline;
    /// * **`last_sync` / `last_successful_sync_summary` / `last_plan_summary`** — not set: nothing
    ///   synced, and a rehearsal's plan is not the last pass's plan;
    /// * **`sync_passes` / `sync_events`** — nothing written: history is written behind a side
    ///   effect, and there was none;
    /// * **delete approvals and `withheld_deletions`** — never consulted: the gate is
    ///   execution-time, and nothing here executes;
    /// * **the standing `unsyncable` list** — not merged into: this pass's drops ride on the plan
    ///   itself as `cannot_sync` (#315), so nothing display-only is mutated either.
    ///
    /// It does not go through [`Self::reconcile_blocking`] at all, which is what makes that list
    /// structural rather than nine things each branch has to remember not to do.
    async fn plan_now(&mut self) {
        // Coalesced: two clicks on `Check again` while one walk is running cost one walk, because
        // the pass that is already booked answers both requests.
        let Some(generation) = self.shared.claim_plan_pass() else {
            return;
        };
        // `syncing` gates `activity` onto status replies, so a plan pass must claim it or #209's
        // progress line has nothing to read. The pause check lives at the ack (as `syncnow`'s
        // does); a pause landing after it lets this inert pass finish rather than orphan a booked
        // request that nothing would ever seal. **Do not add one here to match
        // `discard_queued_apply_for_pause`**: cancelling an apply IS a seal, returning from here is
        // not, and the GUI's plan poller has no bail-out to rescue it. Guard:
        // `a_plan_pass_booked_before_a_pause_still_runs_and_seals`.
        // `plan_pass` BEFORE `syncing`, and cleared after it, so no reply can ever observe
        // `syncing` without the flag that says which kind of pass it is.
        self.shared.pair.plan_pass.store(true, Ordering::SeqCst);
        self.shared.pair.syncing.store(true, Ordering::SeqCst);
        self.shared.begin_plan_progress(current_epoch_secs());
        let result = tokio::task::block_in_place(|| self.pass().plan_only_blocking());
        self.shared.pair.syncing.store(false, Ordering::SeqCst);
        self.shared.pair.plan_pass.store(false, Ordering::SeqCst);
        self.shared.clear_activity();
        // #103's rule, unchanged: a pass that reached Proton is evidence of a session, a classified
        // auth failure is evidence against one, and anything else moves nothing.
        match &result {
            Ok(_) => self.pass().note_auth_evidence(AuthState::SignedIn),
            Err(error) if is_auth_failure_error(error.as_ref()) => {
                self.pass().note_auth_evidence(AuthState::SignedOut)
            }
            Err(_) => {}
        }
        let sealed = result.map_err(|error| {
            warn!(%error, "plan pass failed");
            error.to_string()
        });
        self.shared.seal_plan_pass(generation, sealed);
    }
}

impl<C: ProtonClient> PairPass<'_, C> {
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
                    && !self.pair.config.conflict_naming.is_conflict_copy(&path)
                    && let Ok(relative_path) = path.strip_prefix(&self.pair.config.local_root)
                    && !relative_path.as_os_str().is_empty()
                    && self
                        .pair
                        .scan_options
                        .allows_relative_directory(relative_path)
                {
                    self.pair
                        .pending_changes
                        .insert(relative_path.to_path_buf());
                }
                continue;
            }
            if self.pair.config.conflict_naming.is_conflict_copy(&path) {
                if matches!(event.kind, EventKind::Remove(_))
                    && let Some(original) = self
                        .pair
                        .config
                        .conflict_naming
                        .original_from_conflict_copy(&path)
                    && let Ok(relative_path) = original.strip_prefix(&self.pair.config.local_root)
                    && self.pair.scan_options.allows_relative_file(relative_path)
                {
                    mark_modified(&self.pair.connection, relative_path)?;
                    self.pair
                        .pending_changes
                        .insert(relative_path.to_path_buf());
                }
                continue;
            }

            let relative_path = match path.strip_prefix(&self.pair.config.local_root) {
                Ok(relative_path) => relative_path.to_path_buf(),
                Err(_) => continue,
            };

            if !self.pair.scan_options.allows_relative_file(&relative_path) {
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
                    if !self.pair.authored_writes.contains(&relative_path) {
                        mark_modified(&self.pair.connection, &relative_path)?;
                    }
                    self.pair.pending_changes.insert(relative_path);
                }
                EventKind::Remove(_) => {
                    self.pair.pending_changes.insert(relative_path);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Publishes this pair's current state to [`ControlShared`], from which the IPC task answers
    /// every control request. Called whenever the state it copies changes.
    ///
    /// Everything below the `auth` line of that struct is per-pair, so with N pairs this publishes
    /// into *this* pair's block and the reply's flattened top-level fields describe whichever pair
    /// the request selected (ADR 0005 §4). Phase 2 has one pair, so it publishes as it always did.
    fn publish_status(&self) {
        let snapshot = StatusSnapshot {
            pending_changes: self.pair.pending_changes.len(),
            last_sync_epoch_secs: self.last_sync_epoch_secs(),
            last_error: self.pair.last_error.clone(),
            last_plan_summary: self.pair.last_plan_summary.clone(),
            last_successful_sync_summary: self.pair.last_successful_sync_summary.clone(),
            status_history: self.pair.status_history.clone(),
            pending_deletions: self.pair.pending_deletions.clone(),
            failed_items: self.pair.last_failed_items.clone(),
            failed_item_count: self.pair.last_failed_item_count,
            unsyncable: self.pair.unsyncable.clone(),
            history: self.pair.pass_history.clone(),
            index_totals: self.pair.index_totals,
            config: RunningConfigInfo {
                local_root: self.pair.config.local_root.clone(),
                remote_root: self.pair.config.remote_root.clone(),
                db_path: self.pair.config.db_path.clone(),
            },
        };
        *self
            .shared
            .pair
            .snapshot
            .lock()
            .expect("control snapshot lock") = snapshot;
    }

    /// Recomputes the cached corpus size (#207) if a pass may have changed it, then clears the
    /// stale flag. No-op when nothing planned since the last refresh, so the steady-state idle
    /// event-driven pass costs no query at all.
    ///
    /// A read failure leaves the previous value in place and clears the flag: the totals are
    /// display-only, so a stale number beats both a spurious `None` (which a client renders as "no
    /// answer") and a retry loop on a connection the pass itself is using.
    fn refresh_index_totals(&mut self) {
        if !self.pair.index_totals_stale {
            return;
        }
        self.pair.index_totals_stale = false;
        match index_totals(&self.pair.connection) {
            Ok(totals) => self.pair.index_totals = Some(totals),
            Err(error) => warn!(%error, "failed to read index totals"),
        }
    }

    /// One-line `last_error` for a partial pass (#136): terse on purpose — it rides on every
    /// status reply, and the per-item detail is in `failed_items`.
    fn failure_summary(&self, failed: usize) -> String {
        match self.pair.last_failed_items.first() {
            Some(first) => format!(
                "{failed} item(s) failed to sync (first: {})",
                first.path.display()
            ),
            None => format!("{failed} item(s) failed to sync"),
        }
    }

    /// The blocking half of [`Self::plan_now`] — see its docs for the inertness family.
    fn plan_only_blocking(&mut self) -> AppResult<StoredPlan> {
        info!("computing a plan-only pass");
        let base_records = load_index(&self.pair.connection)?;
        let local_root = self.pair.config.local_root.clone();
        let shared = Arc::clone(self.shared);
        let observer = |files_seen: u64, path: &Path| {
            let display = path
                .strip_prefix(&local_root)
                .unwrap_or(path)
                .display()
                .to_string();
            shared.update_activity(PHASE_SCANNING_LOCAL, |activity| {
                activity.files_scanned = Some(files_seen);
                activity.detail = Some(display);
            });
        };
        let report = build_plan_report(
            &self.pair.config,
            self.proton.as_ref(),
            &self.pair.scan_options,
            base_records,
            Some(&observer),
            |phase| self.shared.begin_activity(new_activity(phase)),
        )?;
        Ok(StoredPlan {
            token: report.token(),
            computed_epoch_secs: current_epoch_secs(),
            summary: report.summary,
            actions: report.plan,
            cannot_sync: report.cannot_sync,
            local_disposal: LocalDisposal::of(
                DeleteDirection::Local,
                self.pair.config.local_delete_mode,
            ),
        })
    }

    fn reconcile_blocking(&mut self) -> AppResult<PassOutcome> {
        // Open the pass as a unit (#213): one start time and one clock, both surviving every
        // phase change below. The kind is corrected by whichever branch `reconcile_blocking_inner`
        // takes.
        self.pair.pass_log = PassLog::new(current_epoch_secs());
        self.shared.begin_pass_progress(
            self.pair.pass_log.started_epoch_secs,
            self.pair.pass_log.kind,
        );
        let result = self.reconcile_blocking_inner();
        // What this pass proved about the session (#103), decided before the message below so the
        // reply that carries the message already carries the verdict.
        //
        // Three inputs, and the third one moves nothing. A pass that completed — cleanly or with
        // failed items — reached Proton: a bootstrap listed the tree, an incremental fetched the
        // event delta, and a partial pass landed at least one action by definition (the breaker
        // trips only when *nothing* succeeded). A pass that failed on a **classified** auth failure
        // is a sign-out. A pass that failed any other way — a timeout, a missing binary, a full
        // disk — says nothing either way and must leave the state alone: this is precisely the
        // fall-through that would otherwise read "not recognised" as "signed in and fine".
        match &result {
            Ok(PassOutcome::Clean | PassOutcome::Partial { .. }) => {
                self.note_auth_evidence(AuthState::SignedIn);
            }
            Err(error) if is_auth_failure_error(error.as_ref()) => {
                self.note_auth_evidence(AuthState::SignedOut);
            }
            Err(_) => {}
        }
        // Three outcomes, three arms, no fall-through: a pass that landed most of its plan and
        // failed some of it is neither of the other two, and absorbing it into either is how a
        // failed pass came to render as "everything is up to date" (#246). The history record
        // takes its outcome from the same three arms, so it can never disagree with the message.
        let message = match &result {
            Ok(PassOutcome::Clean) => {
                self.pair.last_error = None;
                self.finalize_pass_record(PassOutcomeKind::Clean, 0, None);
                "sync completed".to_owned()
            }
            // `last_error` carries the summary so every surface that already reads it — the CLI
            // headline, the GUI's `derive_state`, the tray — reports a partial pass as a problem
            // rather than silently as a success. `failed_items` carries the detail.
            Ok(PassOutcome::Partial { failed }) => {
                let failed = *failed;
                let summary = self.failure_summary(failed);
                self.finalize_pass_record(PassOutcomeKind::Partial, failed, Some(&summary));
                self.pair.last_error = Some(summary);
                format!("sync completed with {failed} failed item(s)")
            }
            Err(error) => {
                let error = error.to_string();
                // Reachable by a shutdown mid-plan too, which is why `interrupted` never is: every
                // graceful ending seals the row here.
                self.finalize_pass_record(PassOutcomeKind::Failed, 0, Some(&error));
                self.pair.last_error = Some(error);
                "sync failed".to_owned()
            }
        };
        // Seal this pass's apply verdict (#100), before the reply that carries it is publishable.
        // Every exit seals: a pass that never reached the comparison — a failed scan, a failed
        // listing — would otherwise leave its client polling a request nothing will ever answer.
        self.seal_apply_outcome(&result);
        // Before `record_status_history`, which snapshots into the metrics sidecar: refreshing
        // after it would publish this pass's history beside the previous pass's corpus size.
        // Runs on every outcome, not just `Ok` — see `index_totals_stale`.
        self.refresh_index_totals();
        self.record_status_history(&message);
        // The attempt is complete (recorded either way): bump the sequence a waiting client
        // watches, then publish the final state of this pass.
        self.shared
            .pair
            .reconcile_seq
            .fetch_add(1, Ordering::SeqCst);
        self.publish_status();
        result
    }

    /// Publishes the verdict for this pass's apply request, and resets the intent so it can never
    /// describe the next pass. A no-op for an ordinary sync.
    ///
    /// [`Self::execute_plan_and_commit`] reaches the real verdicts (applied with its counts,
    /// diverged); this fills in the one it cannot — a pass that failed before deciding — from the
    /// pass's own outcome. A pass that somehow ended `Ok` without reaching the comparison is
    /// reported as failed too, deliberately: "the apply did not happen and I cannot say it applied"
    /// is the honest answer, and an arm that fell through to `Applied` would be #246's shape on the
    /// one surface where it authorises a deletion.
    fn seal_apply_outcome(&mut self, result: &AppResult<PassOutcome>) {
        let intent = std::mem::replace(&mut self.pair.pass_intent, PassIntent::Sync);
        let PassIntent::Apply { apply_seq, .. } = intent else {
            return;
        };
        let outcome = self
            .pair
            .apply_report
            .take()
            .unwrap_or_else(|| ApplyOutcome::Failed {
                apply_seq,
                error: match result {
                    Err(error) => truncate_error(&error.to_string()),
                    Ok(_) => "the pass ended without applying the reviewed plan".to_owned(),
                },
            });
        self.shared.seal_apply_pass(outcome);
    }

    fn reconcile_blocking_inner(&mut self) -> AppResult<PassOutcome> {
        info!("starting reconciliation");
        // What this pass was asked to do (#100). Taken once, here, so every downstream branch —
        // warm start, incremental, bootstrap — reaches the same single check in
        // `execute_plan_and_commit`, whichever remote map it ends up planning against.
        self.pair.pass_intent = self.shared.take_apply_request();
        self.pair.apply_report = None;
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
        self.pair.authored_writes.clear();
        // This pass's failures start empty (#136), and they are cleared HERE rather than in the
        // executor so a pass that dies before planning — a failed scan, a failed listing — cannot
        // publish the previous pass's items beside its own "sync failed".
        self.pair.last_failed_items.clear();
        self.pair.last_failed_item_count = 0;
        // A runtime `resync` forces this pass to a full-tree walk, overriding both the warm start
        // and the steady-state incremental path. Consume it exactly once (a `swap`), so a later
        // pass is not also forced. (The startup `--full-walk` flag is separate — see below.)
        let resync_requested = self
            .shared
            .pair
            .force_full_walk
            .swap(false, Ordering::SeqCst);

        // `reset_index` is applied BEFORE the baseline is loaded, or this pass would plan against
        // the very rows it just erased. Consumed with the same `swap`-once discipline as the
        // resync latch, and on the main loop — never from the IPC task, which holds its own
        // connection to this database.
        if self.shared.pair.reset_index.swap(false, Ordering::SeqCst) {
            // RE-LATCH ON FAILURE. `reset-index` is a typed confirmation the user already gave and
            // the IPC reply already promised; a truncation that fails (a locked database, a disk
            // error) must not consume it, or the request evaporates and the daemon carries on with
            // the state the user asked to discard.
            if let Err(error) = reset_index_state(&self.pair.connection) {
                self.shared.pair.reset_index.store(true, Ordering::SeqCst);
                return Err(error);
            }
            // A reset makes this a first pass in every sense that matters: an empty baseline is
            // the bootstrap condition, and the cleared cursor has to be re-seeded by a full walk
            // before any later pass may replay from it. `is_first_reconcile` also restores the
            // "always full-scan the local tree" floor the empty baseline now depends on.
            self.pair.is_first_reconcile = true;
            self.pair.force_local_rescan = true;
            self.pair.incremental_passes_since_full_scan = 0;
            warn!("index reset: baseline, event cursors and delete approvals discarded");
        }

        // Load the baseline before scanning so the scan can reuse each unchanged file's
        // recorded SHA-1 (matching size + mtime) instead of re-hashing the whole tree.
        let base_records = load_index(&self.pair.connection)?;

        // The first reconcile after boot is special: `notify` has replayed nothing, so it must
        // full-scan the local tree. `first_reconcile` either full-walks or warm-starts (both do
        // that local scan). Clear `is_first_reconcile` only on success, so a failed first pass
        // retries as a first pass — keeping the "startup snapshots first" floor sticky across
        // failures (a failed pass must not let the next one drop into the steady-state idle
        // fast-path, which skips the local scan and would strand an offline edit).
        if self.pair.is_first_reconcile {
            // `--full-walk` (a process-lifetime config flag, not the consumed atomic) stays in
            // force across a failed first pass, so the requested full walk still happens on retry.
            let force_bootstrap = resync_requested || self.pair.config.warm_start.force_full_walk;
            let result = self.first_reconcile(base_records, force_bootstrap);
            // CLEAN only — a partial pass is not a success (#136). The re-queued failed paths
            // would keep the next pass non-idle anyway, but the "startup snapshots first" floor is
            // the wrong place to start relying on that: a pass that did not land everything leaves
            // the next one with the same job the first pass exists to do.
            if matches!(result, Ok(PassOutcome::Clean)) {
                self.pair.is_first_reconcile = false;
                // Both first-pass branches full-scan the local tree, so any pending
                // watcher-error rescan is satisfied here.
                self.pair.force_local_rescan = false;
            }
            return result;
        }

        // Event-driven steady state: attempt an incremental pass (O(changes)) before resorting to
        // a full-tree snapshot (O(folders)). Any doubt — no cursor, a server refresh, an events
        // error, or an unresolvable node — falls through to the snapshot below, which is exactly
        // today's behavior. When `events_driven` is off this predicate is always false.
        if !resync_requested && self.should_try_incremental(&base_records) {
            self.set_pass_kind(PassKind::Incremental);
            // A watcher error dropped events (#51): this pass must stat-walk the local tree
            // instead of taking the idle fast-path, which only knows about `pending_changes`.
            //
            // An **apply** forces it for a different reason (#100): the idle fast-path returns
            // before `execute_plan_and_commit`, so an apply landing on an idle cycle would never
            // reach the token comparison — it would neither execute nor report, and its client
            // would poll for ever. A pass that was asked to apply always plans.
            let force_local_scan = self.pair.force_local_rescan
                || matches!(self.pair.pass_intent, PassIntent::Apply { .. });
            match self.try_incremental_reconcile(&base_records, force_local_scan)? {
                IncrementalOutcome::Committed(outcome) => {
                    // Same rule as the first-pass floor above: only a clean pass discharges a
                    // pending rescan, because only a clean pass acted on everything it scanned.
                    if outcome == PassOutcome::Clean {
                        self.pair.force_local_rescan = false;
                    }
                    return Ok(outcome);
                }
                IncrementalOutcome::Idle => {
                    self.pair.force_local_rescan = false;
                    return Ok(PassOutcome::Clean);
                }
                IncrementalOutcome::Fallback(reason) => {
                    info!(%reason, "event-driven pass fell back to a full-tree snapshot");
                }
            }
        }

        // A snapshot always full-scans the local tree, so it too clears the pending rescan.
        let result = self.bootstrap_reconcile(base_records);
        if matches!(result, Ok(PassOutcome::Clean)) {
            self.pair.force_local_rescan = false;
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
    ) -> AppResult<PassOutcome> {
        if !force_bootstrap && self.warm_start_eligible(&base_records) {
            self.set_pass_kind(PassKind::WarmStart);
            // `true` = force the local stat-walk even if the event delta is empty: on a fresh boot
            // `pending_changes` is empty, so the idle fast-path would otherwise skip the scan and
            // strand a file edited while the daemon was down.
            match self.try_incremental_reconcile(&base_records, true)? {
                // The warm start itself worked either way — it replayed the cursor instead of
                // walking the tree — so it counts toward the every-N-warm-starts full walk even
                // when items inside it failed.
                IncrementalOutcome::Committed(outcome) => {
                    self.record_successful_warm_start();
                    info!("warm start completed; skipped the full-tree remote walk");
                    return Ok(outcome);
                }
                IncrementalOutcome::Idle => {
                    self.record_successful_warm_start();
                    info!("warm start completed; skipped the full-tree remote walk");
                    return Ok(PassOutcome::Clean);
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
        let warm = &self.pair.config.warm_start;
        if !warm.enabled || !self.pair.config.events_driven || self.event_source.is_none() {
            return false;
        }
        // Self-healing full walk every N warm starts (across restarts). `0` maps to `u64::MAX`.
        if self.pair.warm_starts_since_full_walk >= effective_full_scan_every(warm.full_walk_every)
        {
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
        let max_age = self.pair.config.warm_start.max_cursor_age;
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
        let floor = effective_full_scan_every(self.pair.config.warm_start.full_walk_every);
        self.pair.warm_starts_since_full_walk = self
            .pair
            .warm_starts_since_full_walk
            .saturating_add(1)
            .min(floor);
        if let Err(error) =
            store_warm_start_count(&self.pair.connection, self.pair.warm_starts_since_full_walk)
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
        if !self.pair.config.events_driven || self.event_source.is_some() {
            return;
        }
        if let Some(source) = (self.event_source_factory)() {
            info!("reused CLI session became readable; resuming event-driven change detection");
            *self.event_source = Some(source);
            // On a *mid-life* reacquisition (not the first pass), force this pass to full-walk by
            // reseeding the resync floor. Steady-state incremental has no cursor-age gate, so
            // without this it would replay the cursor persisted by a previous process — which the
            // degraded snapshots never advanced past — and miss everything since. A full walk
            // captures a fresh cursor to stream from. On the *first* pass this reseed is skipped:
            // `first_reconcile` already decides warm-start-vs-bootstrap with its own cursor-age
            // gate (which subsumes this concern), and reseeding here would also risk overflowing
            // the counter when a warm start increments it past the `u64::MAX` sentinel.
            if !self.pair.is_first_reconcile {
                self.pair.incremental_passes_since_full_scan =
                    effective_full_scan_every(self.pair.config.events_full_scan_every);
            }
        }
    }

    /// Reports the one cause [`Self::resolve_event_scope`] can never see: `events_driven` is on but
    /// there is no event source (locked keyring, headless host, CLI not logged in), so the scope is
    /// never even consulted. Every pass is then a full-tree snapshot and the fast events-poll arm
    /// is gated off, so both those snapshots and the per-pass session retry ride `scan_interval`
    /// (#50).
    ///
    /// The **process** latch, not the pair's, because the cause is the process's: there is one CLI
    /// session for the user. It stays one message family with one reason per cause — see
    /// [`Daemon::session_declined`] for why the two latches cannot both speak in one pass.
    ///
    /// **Two `KeyScope::Pair` values are visible from here and neither may reach the latch**, or the
    /// split this cause belongs to is undone for it (guard:
    /// `the_process_session_cause_is_neither_cleared_nor_worded_by_one_pairs_config`):
    ///
    /// * **`events_driven` may not gate the clear.** A live [`Self::event_source`] is the process
    ///   fact that ends this cause, and it is the only thing that clears it. A pair that has merely
    ///   opted out of event-driven detection knows nothing about the keyring, so clearing on its
    ///   behalf would wipe a cause that is still true for every other pair — which would then
    ///   re-log it on its next pass, for ever, which is precisely what the latch exists to stop.
    /// * **`scan_interval` may not be in the reason.** [`note_session_declined`] decides log-or-stay
    ///   -silent by comparing the *reason string*, so a per-pair number in a process-wide reason
    ///   makes two pairs overwrite each other and both log every pass. The typed
    ///   [`SessionDeclineCause`] does not save this: it governs clearing, never the log decision.
    ///   Only the process-wide poll cadence is named; each pair's own interval is its config's.
    ///
    /// Clearing is per-cause for the same reason it is in `note_auth_evidence`, from the other
    /// side: a readable keyring says nothing about a signed-out account.
    fn note_degraded_session_if_needed(&mut self) {
        if self.event_source.is_some() {
            clear_session_decline(self.session_declined, SessionDeclineCause::NoSession);
            return;
        }
        if !self.pair.config.events_driven {
            return;
        }
        note_session_declined(
            self.session_declined,
            SessionDeclineCause::NoSession,
            format!(
                "no usable proton-drive CLI session (locked keyring, headless host, or the CLI is \
                 not logged in); the {}s event poll is off, so full-tree snapshots and the session \
                 retry ride each pair's own scan interval instead",
                self.events_poll_interval.as_secs(),
            ),
        );
    }

    /// Whether an incremental (event-stream) pass may be attempted this cycle. Requires the
    /// feature on with a usable event source, a resolvable event scope (a volume id **and** a
    /// stored cursor to replay from — see [`Self::resolve_event_scope`]), and that the opt-in
    /// periodic safety resync (disabled by default) is not currently due.
    fn should_try_incremental(&mut self, base_records: &HashMap<PathBuf, FileRecord>) -> bool {
        if !self.pair.config.events_driven || self.event_source.is_none() {
            return false;
        }
        // Periodic safety resync (opt-in). `events_full_scan_every == 0` maps to `u64::MAX` here,
        // a threshold the counter cannot reach in any realistic runtime after the startup floor
        // resets it to 0 (that would take 2^64 passes) — so a disabled resync leaves the daemon
        // event-driven until restart or an event-stream fallback.
        if self.pair.incremental_passes_since_full_scan
            >= effective_full_scan_every(self.pair.config.events_full_scan_every)
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
            // Both failures decline the same way here; only the bootstrap cursor cares which.
            Err(error) => {
                self.note_event_scope_declined(error.into_reason());
                return None;
            }
        };
        match load_event_cursor(&self.pair.connection, &volume) {
            Ok(Some(cursor)) => {
                self.pair.event_scope_declined = None;
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

    /// The volume id this root's event stream is scoped to, or why it cannot be determined. Derived
    /// from any baseline composed `proton_id` first (free, no I/O).
    fn volume_id_for_scope(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> Result<String, VolumeScopeError> {
        if let Some(volume) = derive_volume_id(base_records) {
            return Ok(volume.to_owned());
        }
        // No baseline composed id: a brand-new sync that has recorded nothing, or a remote that is
        // entirely Proton-native (unsupported files get no index row). The sole stored cursor's
        // scope id *is* the volume a previous bootstrap anchored, so the gate can still engage
        // (#32 — it used to stay on full-tree walks forever here).
        match load_sole_event_cursor(&self.pair.connection) {
            Ok(Some(cursor)) => Ok(cursor.scope_id),
            Ok(None) => Err(VolumeScopeError::Unnameable(
                "no event volume: no indexed node carries a composed proton_id, and no single \
                 stored cursor names one"
                    .to_owned(),
            )),
            Err(error) => Err(VolumeScopeError::Unreadable(format!(
                "no event volume: the stored event cursor is unreadable: {error}"
            ))),
        }
    }

    /// Records what this pass proved about the Proton session (#103), and reports a sign-out
    /// through the **same** one-reason-per-cause message family rather than opening a second
    /// channel.
    ///
    /// "Why is this daemon not doing what I expect" has one message family: one line per cause, said
    /// once, re-said when the cause changes. An expired session is a cause in it — and a real one,
    /// since the events fetch and the CLI share that session, so a sign-out degrades detection
    /// exactly as a locked keyring does. Two surfaces independently deciding why the daemon is
    /// degraded would disagree.
    ///
    /// The **process** latch, because a session is per-user: one sign-out is not N sign-outs, and
    /// with N pairs reporting it per pair would say the same thing N times.
    ///
    /// Recovery clears the latch **only for this cause**: a working session says nothing about a
    /// missing volume or an unreadable cursor, and erasing their line would re-report it next pass as
    /// if it were new.
    fn note_auth_evidence(&mut self, state: AuthState) {
        publish_auth_evidence(self.shared, state);
        match state {
            AuthState::SignedOut => note_session_declined(
                self.session_declined,
                SessionDeclineCause::SignedOut,
                AUTH_DECLINE_REASON.to_owned(),
            ),
            AuthState::SignedIn => {
                clear_session_decline(self.session_declined, SessionDeclineCause::SignedOut);
            }
            // Unreachable by contract — `record_auth_state` asserts it — and deliberately inert
            // rather than "reset to unknown": absence of evidence is not evidence.
            AuthState::Unknown => {}
        }
    }

    /// Reports why **this pair** cannot replay from the event stream, once per distinct cause. Info
    /// level: this is the only signal distinguishing "still full-walking because it cannot stream"
    /// from the other causes of repeated full syncs.
    ///
    /// The reasons that reach here are this pair's *standing* ones — no volume, no/unreadable cursor
    /// — and they are latched per pair for a concrete reason: with one shared latch, two pairs
    /// declining for two volumes would each see the other's string, log again, and never settle. The
    /// process-wide causes (no session, signed out) latch on the daemon instead
    /// ([`Daemon::session_declined`]).
    ///
    /// A pass that started streaming and then gave up logs the separate per-pass line
    /// ("event-driven pass fell back to a full-tree snapshot"), which is deliberately a different
    /// message.
    fn note_event_scope_declined(&mut self, reason: String) {
        if self.pair.event_scope_declined.as_deref() != Some(reason.as_str()) {
            info!(%reason, "event-driven detection unavailable; using full-tree walks");
            self.pair.event_scope_declined = Some(reason);
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
            && self.pair.pending_changes.is_empty()
            && self.pair.pending_deletions.is_empty()
        {
            if delta.latest_event_id != cursor.last_event_id {
                store_event_cursor(
                    &self.pair.connection,
                    &volume,
                    &delta.latest_event_id,
                    current_epoch_secs() as i64,
                )?;
            }
            self.pair.incremental_passes_since_full_scan = self
                .pair
                .incremental_passes_since_full_scan
                .saturating_add(1);
            info!("event-driven pass idle; no remote or local changes");
            return Ok(IncrementalOutcome::Idle);
        }

        let local_scan = self.scan_local_entities_reporting_progress(base_records)?;
        let local_files = local_files_from_entities(&local_scan.entities);
        let base_index = filter_base_index(base_records.clone(), &self.pair.scan_options);

        let remote_entities = {
            // Built here and dropped at the end of this block: its listing memo lives exactly as
            // long as the pass.
            let resolver = TargetedResolver {
                proton: self.proton.as_ref(),
                connection: &self.pair.connection,
                remote_root: &self.pair.config.remote_root,
                volume_id: &volume,
                listings: RefCell::new(HashMap::new()),
            };
            match reconstruct_remote(
                &base_index,
                &delta.changes,
                &volume,
                &self.pair.scan_options,
                &resolver,
            ) {
                Reconstruction::Complete(map) => map,
                Reconstruction::FallbackToSnapshot(reason) => {
                    return Ok(IncrementalOutcome::Fallback(reason));
                }
            }
        };

        let outcome = self.execute_plan_and_commit(
            &local_scan,
            &local_files,
            &remote_entities,
            &base_index,
            RemoteMapShape {
                remote_root_missing: false,
                full_snapshot: false,
            },
            Some(CursorUpdate {
                scope_id: volume,
                last_event_id: delta.latest_event_id,
            }),
        )?;
        self.pair.incremental_passes_since_full_scan = self
            .pair
            .incremental_passes_since_full_scan
            .saturating_add(1);
        Ok(IncrementalOutcome::Committed(outcome))
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

    /// This pair's local scan, surfacing per-file progress into [`ControlShared`] so a slow
    /// pass over large files (SHA-1 hashing is the cost) reads as alive in status replies.
    ///
    /// Returns the whole [`LocalScan`], entries dropped as unsyncable included (#232). Every caller
    /// walks the **entire** tree — there is no partial local scan — which is what lets
    /// [`Self::record_unsyncable`] read the absence of a path from `scan.unsyncable` as evidence.
    fn scan_local_entities_reporting_progress(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> AppResult<LocalScan> {
        self.shared
            .begin_activity(new_activity(PHASE_SCANNING_LOCAL));
        let observer = |files_seen: u64, path: &Path| {
            let display = path
                .strip_prefix(&self.pair.config.local_root)
                .unwrap_or(path)
                .display()
                .to_string();
            self.shared
                .update_activity(PHASE_SCANNING_LOCAL, |activity| {
                    activity.files_scanned = Some(files_seen);
                    activity.detail = Some(display);
                });
        };
        scan_local_tree(
            &self.pair.config.local_root,
            &self.pair.scan_options,
            base_records,
            Some(&observer),
        )
    }

    /// Full-tree snapshot reconcile — the original behavior, plus (when event-driven) capturing
    /// and persisting the replay cursor `C0`. Resets the periodic-resync counter.
    fn bootstrap_reconcile(
        &mut self,
        base_records: HashMap<PathBuf, FileRecord>,
    ) -> AppResult<PassOutcome> {
        // Set here rather than at the call sites: a warm start that falls through to a snapshot IS
        // a full sweep, and `Full sweep now` asks when the last full-tree walk ran (#238).
        self.set_pass_kind(PassKind::FullSweep);
        // The cursor asserts "every remote change up to this event has been applied", which is only
        // true of a cursor read *before* the walk: a change landing mid-walk in an already-listed
        // folder is missed by this snapshot, so anchoring after the walk skips it in the delta too
        // (#294). Read it up front — including on a first-ever bootstrap, which names its volume
        // from one targeted listing of the remote root rather than from the snapshot (#303). When
        // any of that fails, this pass persists no cursor rather than a false one; only a remote
        // root that names no volume at all still anchors after the walk, and there the post-walk
        // read has nothing to name a volume with either. See `resolve_bootstrap_cursor_update`.
        let pre_snapshot_cursor = self.capture_pre_snapshot_cursor(&base_records);

        let local_scan = self.scan_local_entities_reporting_progress(&base_records)?;
        let local_files = local_files_from_entities(&local_scan.entities);
        // The client's progress sink updates the per-folder count/path from inside the walk;
        // this just flips the phase so status shows "listing remote" the moment it starts.
        self.shared
            .begin_activity(new_activity(PHASE_LISTING_REMOTE));
        let (remote_entities, remote_root_missing) = load_remote_entities(
            self.proton.as_ref(),
            &self.pair.config.remote_root,
            &self.pair.scan_options,
        )?;
        let mut base_index = filter_base_index(base_records, &self.pair.scan_options);
        if remote_root_missing {
            base_index.clear();
        }

        let cursor_update = self.resolve_bootstrap_cursor_update(
            pre_snapshot_cursor,
            &remote_entities,
            remote_root_missing,
        );

        let outcome = self.execute_plan_and_commit(
            &local_scan,
            &local_files,
            &remote_entities,
            &base_index,
            RemoteMapShape {
                remote_root_missing,
                full_snapshot: true,
            },
            cursor_update,
        )?;
        self.pair.incremental_passes_since_full_scan = 0;
        // A full walk is the self-healing event warm starts count toward: reset the across-restart
        // warm-start floor so the every-N cadence restarts from this fresh baseline. Best-effort —
        // see `record_successful_warm_start`.
        if self.pair.warm_starts_since_full_walk != 0 {
            self.pair.warm_starts_since_full_walk = 0;
            if let Err(error) = store_warm_start_count(&self.pair.connection, 0) {
                warn!(%error, "failed to reset the warm-start counter after a full walk");
            }
        }
        Ok(outcome)
    }

    /// Reads the current latest cursor before a snapshot, when event-driven and the volume can be
    /// named *before* the walk. The distinction the return type carries is the point: "nothing
    /// names a volume" and "a lookup failed" are different answers, and only the former may be
    /// repaired after the walk (#294).
    fn capture_pre_snapshot_cursor(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> PreSnapshotCursor {
        if !self.pair.config.events_driven {
            return PreSnapshotCursor::Unavailable;
        }
        // Every remaining arm needs an event source, and the one below also spends a CLI
        // invocation: a degraded session (events on, no source) already pays a full snapshot per
        // `scan_interval` tick and must not also pay a listing whose answer it could not use.
        let Some(source) = self.event_source.as_ref() else {
            return PreSnapshotCursor::Unavailable;
        };
        let volume = match self.volume_id_for_scope(base_records) {
            Ok(volume) => volume,
            // Nothing *stored* names a volume: the first-ever bootstrap, with no prior cursor to
            // lose. Ask the remote instead (#303) — one targeted listing of the remote root, the
            // O(1) call the event resolver already makes, whose nodes carry the composed
            // `volumeId~nodeId`. Only when that too names nothing does the pass fall back to
            // today's post-walk anchor.
            Err(VolumeScopeError::Unnameable(_)) => match self.volume_id_from_remote_root() {
                Some(volume) => volume,
                None => return PreSnapshotCursor::VolumeUnknown,
            },
            // The lookup itself failed, so a stored cursor may well exist and simply be invisible
            // right now; anchoring after the walk would overwrite it with a post-walk value.
            Err(VolumeScopeError::Unreadable(reason)) => {
                warn!(%reason, "cannot name the event volume before the snapshot; this pass persists no cursor");
                return PreSnapshotCursor::Unavailable;
            }
        };
        match source.latest_cursor(&volume) {
            Ok(last_event_id) => PreSnapshotCursor::Captured(CursorUpdate {
                scope_id: volume,
                last_event_id,
            }),
            Err(error) => {
                // Typically an unhealthy events endpoint — the same condition that forced this
                // snapshot in the first place, so this arm fires exactly when a fabricated cursor
                // would skip the most. Reached from *both* volume sources, deliberately: once the
                // volume is named, a post-walk retry would re-open the very window this read
                // exists to close, so a first-ever bootstrap whose cursor read fails persists
                // nothing and re-derives next pass like every other pass does.
                warn!(%error, "could not capture the pre-snapshot events cursor; this pass persists no cursor, so a change made during the walk is re-derived next pass instead of being skipped forever");
                PreSnapshotCursor::Unavailable
            }
        }
    }

    /// Names the event volume from the remote itself, for the one case nothing stored can name it:
    /// a first-ever bootstrap, whose baseline is empty and whose cursor row does not exist yet
    /// (#303). One **targeted** listing of the remote root — `list_directory`, the O(1) call the
    /// event resolver uses, not the O(folders) walk — is enough, because every listed node carries
    /// the composed `volumeId~nodeId` that [`derive_volume_id_from_entities`] splits.
    ///
    /// `None` for every failure, and a `None` costs only the pre-#303 post-walk anchor: this must
    /// never add a way for a first sync to fail. Two causes reach it — the listing errored, or it
    /// named no volume: an **empty** remote root (whose own wrapper node the listing drops, so a
    /// childless root yields nothing), or nodes carrying only legacy uncomposed ids. The post-walk
    /// derive reads the same ids, so it is equally empty-handed on the second cause and on the
    /// first — a walk that listed no node can have missed no change to one.
    ///
    /// Note the derivation itself is not a second definition of "what volume is this scope": it is
    /// [`derive_volume_id_from_entities`], the same function the post-walk arm calls, over a
    /// smaller map.
    fn volume_id_from_remote_root(&self) -> Option<String> {
        match self
            .proton
            .list_directory(&self.pair.config.remote_root, Path::new(""))
        {
            Ok(entities) => {
                let volume = derive_volume_id_from_entities(&entities);
                if volume.is_none() {
                    debug!(
                        "the remote root listing names no event volume; this bootstrap anchors its cursor after the walk"
                    );
                }
                volume
            }
            Err(error) => {
                debug!(%error, "could not list the remote root to name the event volume; this bootstrap anchors its cursor after the walk");
                None
            }
        }
    }

    /// Chooses the cursor to persist with a bootstrap. `Captured` → persist it. `Unavailable` →
    /// persist **nothing**: a lookup failed while prior state may exist, and a cursor read *after*
    /// the walk would assert that every change made during it had been applied when the snapshot
    /// never saw it (#294). `VolumeUnknown` → nothing could name the volume before the walk, not
    /// even the remote root listing [`Self::capture_pre_snapshot_cursor`] tries (#303); anchor from
    /// a post-walk read, or the event path could never engage at all. This arm keeps the mid-walk
    /// window #303 closed everywhere else, and it is *not* claimed to be safe — it is the
    /// deliberate degrade, chosen because the alternative is failing a first sync over a listing.
    /// What narrows it is that its commonest cause is an **empty** remote root — the listing drops
    /// the root's own wrapper node, so a childless root names nothing — and a walk that listed no
    /// node can have missed no change to one.
    ///
    /// This is one of three places that can suppress the cursor, but they are one decision, not
    /// three that can disagree: all of them only ever force the single `cursor_update` toward
    /// `None`, and the sole store site is the final commit in [`Self::execute_plan_and_commit`]
    /// (see the withheld-delete and vanished-node holds there).
    fn resolve_bootstrap_cursor_update(
        &self,
        pre_snapshot_cursor: PreSnapshotCursor,
        remote_entities: &HashMap<PathBuf, RemoteEntity>,
        remote_root_missing: bool,
    ) -> Option<CursorUpdate> {
        if !self.pair.config.events_driven || remote_root_missing {
            return None;
        }
        match pre_snapshot_cursor {
            PreSnapshotCursor::Captured(cursor) => return Some(cursor),
            // Persist nothing, and deliberately do not *clear* a stored cursor either: an older
            // cursor is the safer replay point (re-delivered events reconstruct idempotently;
            // skipped ones are lost outright). The next pass then either streams from that older
            // cursor or, when none exists, declines the scope and bootstraps again — costly, but it
            // self-heals on the first pass whose pre-snapshot read succeeds.
            PreSnapshotCursor::Unavailable => return None,
            PreSnapshotCursor::VolumeUnknown => {}
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

    /// Folds this pass's evidence into the standing unsyncable list, names each new entry at `warn`
    /// exactly once per run, and persists the result (#295, #232).
    ///
    /// Two producers feed it, and they are two different observations:
    ///
    /// * the plan's `SkipUnsupported` rows — a remote node the CLI cannot download as bytes, or a
    ///   local name that is not valid UTF-8;
    /// * `local_unsyncable`, the entries the local stat-walk **dropped** (a socket, symlink, FIFO
    ///   or device node). These never reach the planner at all: nothing records them in
    ///   `file_index`, and the planner is only ever handed entities the scan kept.
    ///
    /// A merge rather than a replacement, because a skipped entity is **never recorded in
    /// `file_index`**: it is absent from the baseline `reconstruct_remote` overlays, so an
    /// incremental pass does not plan it at all and its `skipped_unsupported` count is 0. Replacing
    /// the list every pass would therefore blank it on the cadence the daemon normally runs at
    /// (event-driven by default, warm start on boot), which is the invisibility being fixed. The
    /// three clauses:
    ///
    /// * everything this pass observed is added (first-seen preserved for one already listed);
    /// * a path this pass planned some *other* action for is dropped — it became syncable, and any
    ///   pass can prove that — **unless this pass also observed it as unsyncable**, which happens
    ///   when a synced file is replaced by a socket: the planner sees the file gone and plans a
    ///   delete while the scan sees the socket. Direct observation wins, and the path is reported
    ///   as both, because it *is* both;
    /// * a path this pass did **not** observe is dropped only when this pass's evidence covers
    ///   where that reason comes from — see [`UnsyncableOrigin`]. Local reasons prune here
    ///   unconditionally: every caller of this function ran the whole local stat-walk first (both
    ///   `bootstrap_reconcile` and `try_incremental_reconcile` scan before planning, and the
    ///   events-mode idle fast-path returns before either), so its silence about a local path is
    ///   evidence. Remote and unknown reasons need a `full_snapshot`, since an incremental pass's
    ///   remote map is `base ⊕ delta` over a baseline no skip is in.
    ///
    /// So a Proton Docs file deleted remotely between full walks lingers in the list until the next
    /// one (`proton-sync resync` forces it), while a deleted socket goes on the next pass that
    /// plans anything at all. Display-only data either way: a stale row misleads nobody into a side
    /// effect, and the planner re-derives every skip from ground truth regardless.
    fn record_unsyncable(
        &mut self,
        plan: &[PlannedAction],
        local_unsyncable: &[UnsyncableEntry],
        full_snapshot: bool,
    ) {
        // `destination_path` counts as much as `path`: a move's destination is a path this pass
        // planned a real action ONTO, so a stale entry there is contradicted by the same evidence.
        let superseded: HashSet<&Path> = plan
            .iter()
            .filter(|action| action.action != SyncAction::SkipUnsupported)
            .flat_map(|action| {
                std::iter::once(action.path.as_path()).chain(action.destination_path.as_deref())
            })
            .collect();
        let now = current_epoch_secs();
        let mut observed = unsyncable_items(plan, now);
        observed.extend(local_unsyncable.iter().map(|entry| UnsyncableItem {
            path: entry.relative_path.clone(),
            // The engine knows two kinds and none of these is a directory it would descend into —
            // a symlink to a folder is not followed either — so every local drop is filed as a
            // file. The `reason` is what says *what* it really is.
            entity_kind: EntityKind::File,
            reason: entry.reason.clone(),
            first_seen_epoch_secs: now,
        }));
        let rederived: HashSet<&Path> = observed.iter().map(|item| item.path.as_path()).collect();
        let mut merged: BTreeMap<PathBuf, UnsyncableItem> = self
            .pair
            .unsyncable
            .iter()
            .filter(|item| {
                if rederived.contains(item.path.as_path()) {
                    // Observed again this pass. Kept for its first-seen epoch; the `or_insert`
                    // below is what preserves it.
                    return true;
                }
                if superseded.contains(item.path.as_path()) {
                    return false;
                }
                !pass_proves_absence_of(item.reason.origin(), full_snapshot)
            })
            .map(|item| (item.path.clone(), item.clone()))
            .collect();
        for item in observed {
            // A path already listed keeps the epoch it was FIRST seen at — but only while it is
            // the SAME fact. A socket replaced by a symlink under one name is one path and two
            // different problems, and carrying the old row would go on reporting `local_socket`
            // for a socket that is gone, indefinitely, because the path never leaves the list to
            // be re-added. So the reason (and the kind) always come from this pass's observation,
            // and only the age is carried.
            //
            // The age is carried across a *changed* reason rather than re-stamped, which is where
            // this parts company with `withheld_deletions`: that table pins to a fingerprint
            // because an approval must authorise exactly what the user saw, and a changed
            // fingerprint makes the approval inert. Nothing here authorises anything. The question
            // this epoch answers is "how long has this path been unsyncable", and a path that was
            // a socket in March and is a symlink today has been unsyncable since March.
            match merged.entry(item.path.clone()) {
                Entry::Occupied(mut slot) => {
                    let first_seen_epoch_secs = slot.get().first_seen_epoch_secs;
                    slot.insert(UnsyncableItem {
                        first_seen_epoch_secs,
                        ..item
                    });
                }
                Entry::Vacant(slot) => {
                    slot.insert(item);
                }
            }
        }
        let merged: Vec<UnsyncableItem> = merged.into_values().collect();

        for item in &merged {
            if self.pair.reported_unsyncable.insert(item.path.clone()) {
                warn!(
                    path = %item.path.display(),
                    reason = item.reason.as_str(),
                    "cannot sync this entity: {}. It is skipped on every pass and will not \
                     transfer in either direction until that changes",
                    item.reason.describe()
                );
            }
        }
        self.pair
            .reported_unsyncable
            .retain(|path| merged.iter().any(|item| &item.path == path));

        if merged == self.pair.unsyncable {
            return;
        }
        self.pair.unsyncable = merged;
        if let Err(error) = replace_unsyncable_items(&self.pair.connection, &self.pair.unsyncable) {
            warn!(%error, "failed to persist the unsyncable-items list");
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
    ///
    /// A failing action does **not** end the pass (#136): its own mutations are discarded, it is
    /// recorded as a [`FailedItem`], and execution continues with its successors — otherwise one
    /// poisoned file starves every action ordered after it, on every pass, forever. Such a pass
    /// returns [`PassOutcome::Partial`]. This is not a retry: the action is attempted once per
    /// pass, exactly as before, and is never re-attempted within one.
    fn execute_plan_and_commit(
        &mut self,
        local_scan: &LocalScan,
        local_files: &HashMap<PathBuf, LocalFileState>,
        remote_entities: &HashMap<PathBuf, RemoteEntity>,
        base_index: &HashMap<PathBuf, FileRecord>,
        remote_map: RemoteMapShape,
        cursor_update: Option<CursorUpdate>,
    ) -> AppResult<PassOutcome> {
        let local_entities = &local_scan.entities;
        let remote_root_missing = remote_map.remote_root_missing;
        let outcome = plan_sync_entities_with_stats(
            local_entities,
            remote_entities,
            base_index,
            &self.pair.config.conflict_naming,
        );
        let matched_files = outcome.matched_files;
        let mut plan = outcome.actions;
        prepend_remote_root_creation_if_missing(&mut plan, remote_root_missing);
        // #100's comparison, and it happens BEFORE this method mutates anything about the pass —
        // no published summary, no stale-totals flag, no unsyncable merge, no delete gate. A
        // diverged apply must leave the daemon exactly as it found it, because the user's next act
        // is to review the new plan, not to inherit half a pass.
        let skip_destructive = match &self.pair.pass_intent {
            PassIntent::Sync => false,
            PassIntent::Apply {
                apply_seq,
                token,
                skip_destructive,
            } => {
                let apply_seq = *apply_seq;
                let skip_destructive = *skip_destructive;
                let fresh = plan_token(&plan);
                if fresh != *token {
                    let summary = PlanSummary::from_outcome(&plan, matched_files);
                    info!(
                        reviewed = %token,
                        fresh = %fresh,
                        "apply refused: the plan changed since it was reviewed"
                    );
                    // Publish what a re-review will find, then refuse. The client learns *which*
                    // refusal this was from the typed `Diverged`, never from the message.
                    self.shared.replace_stored_plan(StoredPlan {
                        token: fresh,
                        computed_epoch_secs: current_epoch_secs(),
                        summary,
                        actions: plan,
                        cannot_sync: local_scan.unsyncable.clone(),
                        local_disposal: LocalDisposal::of(
                            DeleteDirection::Local,
                            self.pair.config.local_delete_mode,
                        ),
                    });
                    self.pair.apply_report = Some(ApplyOutcome::Diverged { apply_seq });
                    // An `Err`, not a clean pass that happened to do nothing: `PassOutcome::Clean`
                    // sets `last_sync` and `last_successful_sync_summary`, which would draw
                    // "everything is up to date" over a plan still waiting to be applied — #246's
                    // shape. Failing also leaves `is_first_reconcile` and `force_local_rescan`
                    // exactly where they were, which is right: this pass consumed nothing.
                    return Err(boxed_error(
                        "apply refused: the plan changed since it was reviewed, so nothing was \
                         applied. Review the new plan and apply that.",
                    ));
                }
                skip_destructive
            }
        };
        let plan_summary = PlanSummary::from_outcome(&plan, matched_files);
        // Only a planned action mutates the index, so an empty plan provably cannot move the
        // totals — that is what keeps an idle pass free of the aggregate query.
        self.pair.index_totals_stale |= !plan.is_empty();
        self.pair.last_plan_summary = Some(plan_summary.clone());
        // The pass's change count, established the moment a plan exists (#213). Counts actions
        // with a side effect, so an `AutoLink`-heavy adoption pass does not read as thousands of
        // changes. Until this point the count is honestly `None` — the client omits the clause
        // rather than printing a `0` it would have to take back.
        let planned_changes = plan
            .iter()
            .filter(|action| action_performs_side_effects(&action.action))
            .count() as u64;
        self.pair.pass_log.planned_changes = Some(planned_changes);
        self.shared
            .update_pass_progress(|pass| pass.changes = Some(planned_changes));
        self.record_unsyncable(&plan, &local_scan.unsyncable, remote_map.proves_absence());

        // Delete-approval gate: decide, per destructive action, whether to execute it now or
        // withhold it pending the user's approval (see `decide_delete_gate`). This filters the
        // plan at execution time; the planner itself stays pure.
        let DeleteGate {
            withheld_paths,
            pending,
            consumed_approvals: approved_deletes,
            withheld: _,
        } = self.decide_delete_gate(&plan, base_index)?;
        // A withheld deletion originates from ground truth (a remote-delete event, or a missing
        // local file). If any deletion is withheld this pass, do NOT advance the event cursor:
        // otherwise a withheld `LocalDelete`'s originating event would fall out of future deltas
        // (reconstruct overlays only the *new* delta onto the surviving baseline) and the pending
        // item would vanish from the queue until the next full-tree resync. Holding the cursor
        // keeps every pending deletion re-derived from ground truth each pass, so an approval
        // applies promptly and the queue never goes stale. The cursor resumes advancing the first
        // pass with nothing withheld.
        let mut cursor_update = if withheld_paths.is_empty() {
            cursor_update
        } else {
            None
        };
        self.pair.pending_deletions = pending;
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
            blocked_awaiting_approval = self.pair.pending_deletions.len(),
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
        let transfer_positions: Vec<usize> = plan
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                matches!(action.action, SyncAction::Upload | SyncAction::Download)
            })
            .map(|(position, _)| position)
            .collect();
        let transfer_queue = TransferQueue {
            plan: &plan,
            positions: &transfer_positions,
            local_files,
        };
        let mut transfer_index = 0usize;
        let action_total = plan.len() as u64;
        let mut pending_approval_consumptions: Vec<(PathBuf, DeleteDirection)> = Vec::new();

        // Nodes skipped because the remote said they are gone (#31). Counted, never recorded:
        // see the cursor hold after the loop.
        let mut vanished_nodes = 0usize;
        // Destructive rows dropped by a filtered apply (#192). Same shape as the two above, and
        // for the same reason: an action the pass declined to perform holds the cursor.
        let mut skipped_destructive = 0usize;
        // Per-item failures this pass, and the paths to re-queue for the next one (#136).
        let mut failures = PassFailures::default();

        let mut action_number = 0usize;
        while action_number < plan.len() {
            // Shutdown beats failure tolerance: the cancel flag makes every remaining action fail
            // instantly (after spawning and killing a CLI child), so continuing past failures
            // would turn a prompt exit into thousands of pointless round trips. Stop dead — the
            // checkpoints already landed, and the remainder re-plans on the next start.
            if self.cancel_flag.load(Ordering::SeqCst) {
                return Err(boxed_error(
                    "reconciliation cancelled: the daemon is shutting down",
                ));
            }
            let action = &plan[action_number];
            // #192, and it is a skip at the executor rather than a filter over the plan.
            //
            // Filtering the plan would have taken three lifecycles with it: `decide_delete_gate`
            // would see no destructive rows, so `replace_withheld_deletions` would wipe the
            // first-seen ages every withheld deletion is dated from (#225) and re-stamp them as new
            // next pass, `pending_deletions` would blank while the deletions are still real (the
            // Deletions screen goes empty), and `record_unsyncable`'s superseded set would shift.
            // Skipping here leaves every one of those computed from the full plan, exactly as an
            // ordinary pass computes them.
            //
            // Dropped even when a standing approval exists: the user asked for THIS run without
            // them, which is a different question from "may this deletion ever apply". The approval
            // is consumed inside the delete arms, so skipping the arm leaves it standing and
            // unspent — and the deletion simply re-plans next pass, exactly like a withheld one.
            if skip_destructive && action.action.is_destructive() {
                debug!(
                    path = %action.path.display(),
                    action = ?action.action,
                    "skipping a destructive action: this apply was asked to run the plan without them"
                );
                skipped_destructive += 1;
                action_number += 1;
                continue;
            }
            // A run of two-plus consecutive planned downloads executes as chunked multi-file
            // CLI invocations instead of one subprocess per file (grouped by destination
            // directory — see `execute_download_run`); a run of one takes the single-file arm
            // below. `download_batch_size = 1` disables batching entirely.
            if self.pair.config.download_batch_size > 1 && action.action == SyncAction::Download {
                let run_length = plan[action_number..]
                    .iter()
                    .take_while(|action| action.action == SyncAction::Download)
                    .count();
                if run_length > 1 {
                    vanished_nodes += self.execute_download_run(
                        &plan[action_number..action_number + run_length],
                        action_number,
                        action_total,
                        &transfer_queue,
                        &mut transfer_index,
                        remote_entities,
                        interactive_progress,
                        &mut index_mutations,
                        &mut pending_approval_consumptions,
                        &mut failures,
                    )?;
                    action_number += run_length;
                    continue;
                }
            }
            let checkpoint_after = action_performs_side_effects(&action.action);
            debug!(path = %action.path.display(), action = ?action.action, "executing sync action");
            // The queue is published on EVERY action, not only on the transfer arms below. A
            // delete or a folder creation between two uploads is still a pass with transfers left,
            // and `begin_activity` replaces the whole activity — so publishing only from the
            // transfer arms would blank the list for the duration of that action, and a client
            // drawing an empty list as "nothing is moving" would say it mid-upload. `transfers`
            // holds the queued rows here and gains an active one when a transfer actually starts.
            self.shared.begin_activity(executing_activity(
                // Root-level actions have an empty relative path; skip it rather than render
                // a trailing space ("creating remote folder ").
                if action.path.as_os_str().is_empty() {
                    activity_verb(&action.action).to_owned()
                } else {
                    format!(
                        "{} {}",
                        activity_verb(&action.action),
                        action.path.display()
                    )
                },
                action_number,
                action_total,
                transfer_queue.window(transfer_index, Vec::new()),
            ));
            // Everything this action may mutate, so a failure can roll back to exactly here: no
            // arm records anything before its own side effect lands, but truncating makes "the
            // failed action is never recorded" structural rather than a property of each arm.
            let mutations_before = index_mutations.len();
            let consumptions_before = pending_approval_consumptions.len();
            let events_before = self.pair.pass_log.pending.len();
            // A closure, not a labeled block, so `?` inside an arm ends THIS action rather than the
            // pass (#136) — `break 'action` (the old loop `continue`) is now `return Ok(())`.
            let mut execute_action = || -> AppResult<()> {
                match action.action {
                    SyncAction::Upload => {
                        if let Some(local) = local_files.get(&action.path) {
                            if let Some(parent) = action.path.parent()
                                && !parent.as_os_str().is_empty()
                                && !planned_remote_directories.contains(parent)
                            {
                                self.proton
                                    .ensure_directory(&self.pair.config.remote_root, parent)?;
                            }
                            transfer_index += 1;
                            let (transfers, remaining) = transfer_queue.window(
                                transfer_index,
                                vec![TransferActivity::active(
                                    "upload",
                                    action.path.clone(),
                                    Some(local.file_size),
                                    current_epoch_secs(),
                                )],
                            );
                            self.shared.update_activity(PHASE_EXECUTING, |activity| {
                                activity.transfers = transfers;
                                activity.transfers_remaining = Some(remaining);
                            });
                            let spinner = begin_transfer_spinner(
                                interactive_progress,
                                transfer_index,
                                transfer_queue.total(),
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
                                &self.pair.config.remote_root,
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
                            self.pair.pass_log.note(
                                SyncAction::Upload,
                                &action.path,
                                Some(local.file_size),
                            );
                        }
                    }
                    SyncAction::Download => {
                        let remote_id = action.remote_id.as_deref().ok_or_else(|| {
                            boxed_error(format!(
                                "planned download for {} is missing a remote id",
                                action.path.display()
                            ))
                        })?;
                        let remote_path =
                            safe_remote_path(&self.pair.config.remote_root, &action.path)
                                .ok_or_else(|| {
                                    boxed_error(format!(
                                        "planned download for {} has an unsafe remote path",
                                        action.path.display()
                                    ))
                                })?;
                        let Some(destination) =
                            safe_local_path(&self.pair.config.local_root, &action.path)
                        else {
                            warn!(
                                path = %action.path.display(),
                                "skipping download: local destination escapes the sync root \
                                 (e.g. through a symlinked directory)"
                            );
                            return Ok(());
                        };
                        ensure_parent_directory(&destination)?;
                        transfer_index += 1;
                        // `bytes_total` stays unknown (the remote listing carries no size), but
                        // `bytes_done` is sampled live from the staging directory the client
                        // reports via the progress sink, so status still shows a growing count.
                        let (transfers, remaining) = transfer_queue.window(
                            transfer_index,
                            vec![TransferActivity::active(
                                "download",
                                action.path.clone(),
                                None,
                                current_epoch_secs(),
                            )],
                        );
                        self.shared.update_activity(PHASE_EXECUTING, |activity| {
                            activity.transfers = transfers;
                            activity.transfers_remaining = Some(remaining);
                        });
                        let spinner = begin_transfer_spinner(
                            interactive_progress,
                            transfer_index,
                            transfer_queue.total(),
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
                        if !skip_if_node_vanished(result, &action.path)? {
                            vanished_nodes += 1;
                            return Ok(());
                        }
                        // The write we just landed will echo back through the watcher; record it so
                        // `handle_fs_event` does not flip this fresh `Synced` record to `Modified`
                        // (issue #49). Downloads always target a regular file.
                        self.pair.authored_writes.insert(action.path.clone());
                        let local_state =
                            local_file_state(&self.pair.config.local_root, &destination)?;
                        let record = FileRecord::from_local(
                            action.path.clone(),
                            &local_state,
                            Some(remote_id.to_owned()),
                            SyncStatus::Synced,
                        );
                        index_mutations.push(IndexMutation::Upsert(record));
                        self.pair.pass_log.note(
                            SyncAction::Download,
                            &action.path,
                            Some(local_state.file_size),
                        );
                    }
                    SyncAction::CreateRemoteDirectory => {
                        if action.path.as_os_str().is_empty() {
                            self.proton
                                .ensure_root_directory(&self.pair.config.remote_root)?;
                            return Ok(());
                        }
                        if let Some(LocalEntityState::Directory(local)) =
                            local_entities.get(&action.path)
                        {
                            self.proton
                                .ensure_directory(&self.pair.config.remote_root, &action.path)?;
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
                            self.pair.pass_log.note(
                                SyncAction::CreateRemoteDirectory,
                                &action.path,
                                None,
                            );
                        }
                    }
                    SyncAction::CreateLocalDirectory => {
                        if let Some(destination) =
                            safe_local_path(&self.pair.config.local_root, &action.path)
                        {
                            fs::create_dir_all(&destination)?;
                            let local_state =
                                local_directory_state(&self.pair.config.local_root, &destination)?;
                            let record = FileRecord::from_local_directory(
                                action.path.clone(),
                                &local_state,
                                action.remote_id.clone(),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                            self.pair.pass_log.note(
                                SyncAction::CreateLocalDirectory,
                                &action.path,
                                None,
                            );
                        }
                    }
                    SyncAction::MoveLocal => {
                        if let Some(destination_path) = action.destination_path.as_ref()
                            && let Some(source) =
                                safe_local_path(&self.pair.config.local_root, &action.path)
                            && let Some(destination) =
                                safe_local_path(&self.pair.config.local_root, destination_path)
                        {
                            ensure_parent_directory(&destination)?;
                            fs::rename(&source, &destination)?;
                            self.pair.pass_log.note_move(
                                SyncAction::MoveLocal,
                                &action.path,
                                destination_path,
                            );
                            if action.entity_kind == EntityKind::Directory {
                                // A directory rename relocates untracked descendants that the
                                // planner deliberately left for the next pass (#12), and inotify
                                // emits nothing for the children of a renamed directory — so
                                // without this hint an event-driven pass with an empty delta would
                                // idle-skip the local scan and never discover them.
                                failures.rescan_next_pass.insert(destination_path.clone());
                                let local_state = local_directory_state(
                                    &self.pair.config.local_root,
                                    &destination,
                                )?;
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
                                self.pair.authored_writes.insert(destination_path.clone());
                                let local_state =
                                    local_file_state(&self.pair.config.local_root, &destination)?;
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
                                    .ensure_directory(&self.pair.config.remote_root, parent)?;
                            }
                            self.proton.rename_or_move(
                                &self.pair.config.remote_root,
                                &action.path,
                                destination_path,
                            )?;
                            self.pair.pass_log.note_move(
                                SyncAction::MoveRemote,
                                &action.path,
                                destination_path,
                            );
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
                        // Bytes only when the sidecar came off the network; a local copy moves
                        // none, and #191's totals count network traffic.
                        let mut sidecar_bytes = None;
                        if action.sidecar_from_local_copy {
                            // The remote node is confirmed gone, so the sidecar is a copy of the
                            // surviving local file (#46). Recording the conflict without the
                            // sidecar is the frozen state this action exists to end, so anything
                            // that stops the copy skips the index write too and re-plans.
                            let (Some(conflict_path), Some(local)) =
                                (action.conflict_path.as_ref(), local_files.get(&action.path))
                            else {
                                return Ok(());
                            };
                            let Some(destination) =
                                safe_local_path(&self.pair.config.local_root, conflict_path)
                            else {
                                warn!(
                                    path = %action.path.display(),
                                    "skipping conflict: the sidecar destination escapes the sync \
                                     root (e.g. through a symlinked directory)"
                                );
                                return Ok(());
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
                                safe_remote_path(&self.pair.config.remote_root, &action.path)
                            && let Some(conflict_path) = action.conflict_path.as_ref()
                            && let Some(destination) =
                                safe_local_path(&self.pair.config.local_root, conflict_path)
                        {
                            ensure_parent_directory(&destination)?;
                            let result = self.proton.download(&remote_path, &destination);
                            // Skipping the sidecar means skipping the `Conflict` record below
                            // too: a conflict with no sidecar has no exit and is invisible to the
                            // GUI's conflicts list (#46). Next pass the node is no longer listed,
                            // so the planner reaches `(Changed, Missing)` and copies the local
                            // file into the sidecar instead.
                            if !skip_if_node_vanished(result, &action.path)? {
                                vanished_nodes += 1;
                                return Ok(());
                            }
                            sidecar_bytes = fs::symlink_metadata(&destination)
                                .ok()
                                .filter(|metadata| metadata.is_file())
                                .map(|metadata| metadata.len());
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
                            // Recorded beside the index mutation, not beside the sidecar write:
                            // the event this logs is the conflict itself (`Both sides had changed`
                            // in `7a File lookup`), which is what the record makes true.
                            self.pair.pass_log.note(
                                SyncAction::Conflict,
                                &action.path,
                                sidecar_bytes,
                            );
                        }
                    }
                    SyncAction::TypeConflict => {
                        let mut sidecar_bytes = None;
                        if action.entity_kind == EntityKind::Directory
                            && let Some(conflict_path) = action.conflict_path.as_ref()
                            && let Some(LocalEntityState::Directory(local_directory)) =
                                local_entities.get(&action.path)
                        {
                            if action.remote_id.is_some()
                                && let Some(remote_path) =
                                    safe_remote_path(&self.pair.config.remote_root, &action.path)
                                && let Some(destination) =
                                    safe_local_path(&self.pair.config.local_root, conflict_path)
                            {
                                ensure_parent_directory(&destination)?;
                                let result = self.proton.download(&remote_path, &destination);
                                // Same whole-action skip as the `Conflict` arm: without its
                                // sidecar the type clash has no exit, so the directory record is
                                // not written either and the pass re-plans from ground truth.
                                if !skip_if_node_vanished(result, &action.path)? {
                                    vanished_nodes += 1;
                                    return Ok(());
                                }
                                let local_state =
                                    local_file_state(&self.pair.config.local_root, &destination)?;
                                let sidecar_record = FileRecord::from_local(
                                    conflict_path.clone(),
                                    &local_state,
                                    action.remote_id.clone(),
                                    SyncStatus::Conflict,
                                );
                                index_mutations.push(IndexMutation::Upsert(sidecar_record));
                                sidecar_bytes = Some(local_state.file_size);
                            }
                            let directory_record = FileRecord::from_local_directory(
                                action.path.clone(),
                                local_directory,
                                None,
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(directory_record));
                            self.pair.pass_log.note(
                                SyncAction::TypeConflict,
                                &action.path,
                                sidecar_bytes,
                            );
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
                            return Ok(());
                        }
                        action.remote_id.as_deref().ok_or_else(|| {
                            boxed_error(format!(
                                "planned remote delete for {} is missing a remote id",
                                action.path.display()
                            ))
                        })?;
                        let remote_path =
                            safe_remote_path(&self.pair.config.remote_root, &action.path)
                                .ok_or_else(|| {
                                    boxed_error(format!(
                                        "planned remote delete for {} has an unsafe remote path",
                                        action.path.display()
                                    ))
                                })?;
                        self.proton.delete(&remote_path)?;
                        self.pair
                            .pass_log
                            .note(SyncAction::RemoteDelete, &action.path, None);
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
                            return Ok(());
                        }
                        let Some(destination) =
                            safe_local_path(&self.pair.config.local_root, &action.path)
                        else {
                            warn!(
                                path = %action.path.display(),
                                "skipping local delete: path escapes the sync root \
                                 (e.g. through a symlinked directory)"
                            );
                            return Ok(());
                        };
                        if destination.exists() {
                            // The ONE disposal (`crate::trash`). A failed trash move propagates as
                            // this action's `Err` and is caught by the #136 path below: the entity
                            // stays on disk, nothing is purged, the cursor is held and the deletion
                            // re-plans. It must never fall back to unlinking — see `trash::dispose`.
                            dispose(
                                self.pair.config.local_delete_mode,
                                &destination,
                                action.entity_kind,
                            )?;
                        }
                        self.pair
                            .pass_log
                            .note(SyncAction::LocalDelete, &action.path, None);
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
                        // Per-pass trace only. The user-facing report is `record_unsyncable`
                        // above: one `warn` per path per run, plus the standing list on the
                        // status payload.
                        debug!(
                            path = %action.path.display(),
                            remote_id = ?action.remote_id,
                            reason = action.unsyncable_reason().as_str(),
                            "skipping an entity that cannot be synced"
                        );
                    }
                }
                Ok(())
            };
            let action_result = execute_action();
            if let Err(error) = action_result {
                // A shutdown makes every remaining action fail the same way; that is not this
                // action's fault and the pass must end now, not record 6000 items.
                if self.cancel_flag.load(Ordering::SeqCst) {
                    return Err(error);
                }
                // Discard whatever this action queued: its side effect did not land, so its index
                // write must not either (the commit-after-side-effects invariant). It re-plans next
                // pass, from ground truth, exactly as a pass-fatal failure used to.
                index_mutations.truncate(mutations_before);
                pending_approval_consumptions.truncate(consumptions_before);
                self.pair.pass_log.truncate(events_before);
                failures.record(action, error.as_ref());
                if failures.breaker_tripped() {
                    return Err(failures.abandoned(Some(&error.to_string())));
                }
            } else {
                failures.note_success();
            }
            if checkpoint_after {
                // Land this action's outcome durably before moving on: a later failure — or a
                // shutdown mid-pass — can no longer discard work that already happened. Index-only
                // actions (AutoLink/Purge) accumulate instead, so an adoption-heavy pass stays a
                // few large transactions rather than thousands of per-row fsyncs.
                commit_checkpoint(
                    &mut self.pair.connection,
                    &mut index_mutations,
                    &mut pending_approval_consumptions,
                    &mut self.pair.pass_log,
                    self.shared,
                    &self.pair.config.local_root,
                )?;
            }
            action_number += 1;
        }

        // ONE cursor policy, four causes. A withheld delete (above), a node that vanished mid-pass,
        // an item whose action failed, and a destructive row a filtered apply dropped are all the
        // same statement: this pass did not apply everything the remote delta describes. Advancing
        // past it would drop the originating event from every future delta — `reconstruct_remote`
        // overlays only the *new* delta onto the surviving baseline — so the unapplied change would
        // never be re-derived. Held, the next pass re-derives it from ground truth, which also keeps
        // that pass non-idle.
        if skipped_destructive > 0 {
            info!(
                skipped = skipped_destructive,
                "ran the reviewed plan without its destructive actions; holding the event cursor so \
                 the next pass re-derives them from ground truth"
            );
            cursor_update = None;
        }
        if vanished_nodes > 0 {
            warn!(
                skipped = vanished_nodes,
                "skipped node(s) that vanished remotely mid-pass; holding the event cursor so the \
                 next pass re-derives them from ground truth"
            );
            cursor_update = None;
        }
        if failures.count > 0 {
            warn!(
                failed = failures.count,
                "action(s) failed; the rest of the plan still ran, and the event cursor is held so \
                 the next pass re-derives them from ground truth"
            );
            cursor_update = None;
        }

        self.shared.begin_activity(new_activity(PHASE_COMMITTING));
        // Read before the transaction opens, exactly as `commit_checkpoint` does — and this second
        // site is not optional (#217): an ADOPTION is index-only, so a whole first sync of an
        // already-matching tree accumulates its `Synced` records here and never touches a
        // checkpoint. Capturing only in `commit_checkpoint` would leave every adopted file without
        // an ancestor, which is the commonest way a file becomes agreed at all.
        let summaries = collect_agreed_summaries(&index_mutations, &self.pair.config.local_root);
        let transaction = self.pair.connection.transaction()?;
        // Whatever accumulated since the last checkpoint (trailing index-only mutations, plus any
        // approval consumption not yet flushed) commits here together with the cursor.
        for mutation in &index_mutations {
            mutation.apply(&transaction)?;
        }
        for (path, digest, summary) in &summaries {
            store_agreed_summary(&transaction, path, digest, summary, current_epoch_secs())?;
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
        // Whatever the last checkpoint did not carry (an index-only tail, or the events of an
        // action that needs no checkpoint of its own).
        let pass_id = self.pair.pass_log.write_pending(&transaction)?;
        transaction.commit()?;
        self.pair.pass_log.note_committed(pass_id, self.shared);
        // `pending_changes` is a wake-up/status hint, not a plan input, and every pass that
        // reaches this commit ran the local stat-walk (the events-mode idle fast-path returns
        // before `execute_plan_and_commit`), so clearing the whole set here cannot lose work: the
        // scan already observed every queued path. An event that arrived *during* the pass is
        // still sitting in the watcher channel — the loop drains it only after this blocking
        // reconcile returns — so it re-inserts itself afterwards. Clearing unconditionally rather
        // than per planned path also avoids leaking entries that produced no action (a directory
        // event, a misclassified removal, a non-regular-file event, a `None` plan outcome).
        self.pair.pending_changes.clear();
        // …except what this pass left undone. A failed item is local-side work no remote event
        // describes (a failed upload has none), so the held cursor alone would not re-derive it:
        // with an empty delta the next event-driven pass would take the idle fast-path, skip the
        // local scan entirely, and never re-plan it. Re-queueing keeps that pass non-idle. Same for
        // a path a directory move relocated out from under its own plan (#12).
        self.pair
            .pending_changes
            .append(&mut failures.rescan_next_pass);

        let outcome = failures.outcome();
        self.pair.last_failed_items = std::mem::take(&mut failures.items);
        self.pair.last_failed_item_count = failures.count;
        // #100: the apply reached a verdict. Recorded here rather than by `seal_apply_outcome`
        // because only this frame knows what actually ran — and recorded on a partial pass too,
        // since "some of what you authorised failed" is an applied plan with failures, not a
        // different verdict (#136's third outcome, reported as itself).
        if let PassIntent::Apply { apply_seq, .. } = self.pair.pass_intent {
            self.pair.apply_report = Some(ApplyOutcome::Applied {
                apply_seq,
                // `PassLog::changed` — side effects that actually **committed**, not actions the
                // loop got through. A withheld deletion, a vanished node and a `SkipUnsupported`
                // all return `Ok` from the action closure, so counting successful closures would
                // report a withheld deletion as executed. This is the fold that already builds the
                // history rollup, so the apply's count and the pass's row can never disagree.
                executed: self.pair.pass_log.changed,
                skipped_destructive,
                failed: failures.count,
            });
        }
        match outcome {
            PassOutcome::Clean => {
                self.pair.last_sync = Some(SystemTime::now());
                self.pair.last_successful_sync_summary = Some(plan_summary);
                info!("reconciliation completed");
            }
            // Deliberately NOT a sync for `last_sync` / `last_successful_sync_summary` purposes:
            // both mean "the last pass that landed everything", and this one did not. The pass is
            // reported as itself instead — through the returned outcome, `failed_items`, and the
            // status-history message.
            PassOutcome::Partial { failed } => {
                info!(
                    failed,
                    planned_actions = plan_summary.total,
                    "reconciliation completed with failed items"
                );
            }
        }
        Ok(outcome)
    }

    /// Executes one run of consecutive planned `Download` actions as chunked multi-file CLI
    /// invocations ([`ProtonClient::download_many`]) instead of one subprocess per file. The
    /// run is segmented into *consecutive* same-destination-directory groups (a batch shares
    /// one `localFolder`) — never merging same-directory files across an intervening
    /// subdirectory's downloads, so execution order is exactly plan order — and each segment
    /// is split into chunks of at most `download_batch_size`. Every chunk is
    /// checkpoint-committed as soon as it lands, so a failure — or a daemon shutdown — never
    /// discards transfers that already completed. A failed file is recorded into `failures` and
    /// costs only itself: the rest of its chunk still lands, and the run continues (#136).
    /// Returns how many files were skipped because their remote node has vanished (#31) — those
    /// neither fail the pass nor get recorded.
    #[allow(clippy::too_many_arguments)]
    fn execute_download_run(
        &mut self,
        run: &[PlannedAction],
        first_action_number: usize,
        action_total: u64,
        transfer_queue: &TransferQueue<'_>,
        transfer_index: &mut usize,
        remote_entities: &HashMap<PathBuf, RemoteEntity>,
        interactive_progress: bool,
        index_mutations: &mut Vec<IndexMutation>,
        pending_approval_consumptions: &mut Vec<(PathBuf, DeleteDirection)>,
        failures: &mut PassFailures,
    ) -> AppResult<usize> {
        let mut groups: Vec<(PathBuf, Vec<PreparedDownload<'_>>)> = Vec::new();
        for (offset, action) in run.iter().enumerate() {
            // A malformed plan entry is that item's failure, not the run's: it can no more starve
            // its successors than a failed transfer can.
            let Some(remote_id) = action.remote_id.as_deref() else {
                failures.record(
                    action,
                    boxed_error(format!(
                        "planned download for {} is missing a remote id",
                        action.path.display()
                    ))
                    .as_ref(),
                );
                continue;
            };
            let Some(remote_path) = safe_remote_path(&self.pair.config.remote_root, &action.path)
            else {
                failures.record(
                    action,
                    boxed_error(format!(
                        "planned download for {} has an unsafe remote path",
                        action.path.display()
                    ))
                    .as_ref(),
                );
                continue;
            };
            let Some(destination) = safe_local_path(&self.pair.config.local_root, &action.path)
            else {
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
        let batch_size = self.pair.config.download_batch_size.max(1);
        let mut vanished_nodes = 0usize;
        for (parent, members) in &groups {
            if let Some(first) = members.first()
                && let Err(error) = ensure_parent_directory(&first.request.destination)
            {
                // This group's destination directory cannot be made (a file squatting on the name,
                // a permissions problem). That is a failure of THIS group's files, not of the pass:
                // the single-file arm fails per item here, so failing the pass would make the same
                // local-FS problem behave differently at `download_batch_size = 1` — and a
                // persistent one would starve every download ordered after it (#136).
                for member in members {
                    failures.record(member.action, error.as_ref());
                }
                continue;
            }
            for chunk in members.chunks(batch_size) {
                // Same shutdown rule as the per-action loop: a batched run is one iteration there,
                // so without this a cancelled 1000-file run would keep issuing doomed batches.
                if self.cancel_flag.load(Ordering::SeqCst) {
                    return Err(boxed_error(
                        "reconciliation cancelled: the daemon is shutting down",
                    ));
                }
                vanished_nodes += self.execute_download_chunk(
                    parent,
                    chunk,
                    action_total,
                    transfer_queue,
                    transfer_index,
                    interactive_progress,
                    index_mutations,
                    pending_approval_consumptions,
                    failures,
                )?;
            }
        }
        Ok(vanished_nodes)
    }

    /// Executes one chunk of prepared downloads via a single [`ProtonClient::download_many`]
    /// call, records every landed file, and checkpoint-commits — completed transfers in a failing
    /// chunk stay durable. Every file that failed is recorded into `failures` individually and
    /// costs only itself (#136). A file whose remote node has vanished (#31) is not a failure at
    /// all: it is skipped (nothing recorded) and counted into the returned tally.
    #[allow(clippy::too_many_arguments)]
    fn execute_download_chunk(
        &mut self,
        parent: &Path,
        chunk: &[PreparedDownload<'_>],
        action_total: u64,
        transfer_queue: &TransferQueue<'_>,
        transfer_index: &mut usize,
        interactive_progress: bool,
        index_mutations: &mut Vec<IndexMutation>,
        pending_approval_consumptions: &mut Vec<(PathBuf, DeleteDirection)>,
        failures: &mut PassFailures,
    ) -> AppResult<usize> {
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
        // `bytes_done` is sampled live from the staging directory the client reports via the
        // progress sink; for a chunk it grows across the whole batch. The path names the
        // directory being filled rather than a single file (`display_parent`, so a root-level
        // chunk renders as "." instead of an empty string in `proton-sync status`).
        //
        // ONE ROW FOR THE WHOLE CHUNK, carrying `files`, rather than one row per file: the batch is
        // a single CLI invocation into a single staging directory, so its bytes-so-far is one
        // measurement of the group and no division of it across N rows would be true. A row that
        // covers many files says so, so a client cannot read the folder name as a filename (#211).
        let chunk_row = TransferActivity {
            files: Some(chunk.len() as u64),
            ..TransferActivity::active(
                "download",
                display_parent.to_path_buf(),
                None,
                current_epoch_secs(),
            )
        };
        self.shared.begin_activity(executing_activity(
            format!(
                "downloading {} files in {}",
                chunk.len(),
                display_parent.display()
            ),
            last_action_number,
            action_total,
            transfer_queue.window(*transfer_index, vec![chunk_row]),
        ));
        let verb = format!("downloading {} files from", chunk.len());
        let spinner = begin_transfer_spinner(
            interactive_progress,
            first_transfer_ordinal,
            transfer_queue.total(),
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

        let mut vanished_nodes = 0usize;
        let mut chunk_failures = 0usize;
        for (prepared, result) in chunk.iter().zip(results) {
            match result {
                // A stat/hash failure on a landed file (e.g. deleted out from under us the
                // instant after promotion) is that ITEM's failure, never a `?` out of the
                // loop — the chunk's other survivors must still reach the checkpoint below.
                Ok(()) => {
                    match local_file_state(
                        &self.pair.config.local_root,
                        &prepared.request.destination,
                    ) {
                        Ok(local_state) => {
                            // Suppress this landed download's watcher echo so it cannot flip the
                            // fresh `Synced` record to `Modified` (issue #49); same rationale as
                            // the single-file `Download` arm.
                            self.pair
                                .authored_writes
                                .insert(prepared.action.path.clone());
                            let record = FileRecord::from_local(
                                prepared.action.path.clone(),
                                &local_state,
                                Some(prepared.remote_id.clone()),
                                SyncStatus::Synced,
                            );
                            index_mutations.push(IndexMutation::Upsert(record));
                            self.pair.pass_log.note(
                                SyncAction::Download,
                                &prepared.action.path,
                                Some(local_state.file_size),
                            );
                            debug!(
                                target: TRANSFER_LOG_TARGET,
                                path = %prepared.action.path.display(),
                                "downloaded file from Proton Drive (batched)"
                            );
                        }
                        Err(error) => {
                            chunk_failures += 1;
                            failures.record(
                                prepared.action,
                                boxed_error(format!(
                                    "downloaded but could not be recorded: {error}"
                                ))
                                .as_ref(),
                            );
                        }
                    }
                }
                // The node is gone since the listing (#31): skip this file alone — no record, no
                // failure — so the chunk's other files still land and the pass continues.
                Err(error) if is_node_not_found_error(error.as_ref()) => {
                    warn!(
                        path = %prepared.action.path.display(),
                        %error,
                        "skipping batched download: the remote node is gone since the listing"
                    );
                    vanished_nodes += 1;
                }
                Err(error) => {
                    chunk_failures += 1;
                    failures.record(prepared.action, error.as_ref());
                }
            }
        }
        // Land this chunk's completed downloads durably: a failing chunk's survivors are recorded
        // and never re-transferred.
        commit_checkpoint(
            &mut self.pair.connection,
            index_mutations,
            pending_approval_consumptions,
            &mut self.pair.pass_log,
            self.shared,
            &self.pair.config.local_root,
        )?;
        // A chunk is many actions; one landed file in it clears the breaker's consecutive run
        // exactly as a successful single action would. A vanished node landed nothing.
        if chunk_failures + vanished_nodes < chunk.len() {
            failures.note_success();
        }
        if failures.breaker_tripped() {
            return Err(failures.abandoned(None));
        }
        Ok(vanished_nodes)
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
            &self.pair.config.local_root,
            EffectiveSettings {
                require_remote_delete_approval: self.pair.config.delete_approval_remote,
                require_local_delete_approval: self.pair.config.delete_approval_local,
            },
        );
        let mut gate = DeleteGate::default();
        let now = current_epoch_secs();
        // The ages carried over from earlier passes, keyed exactly as the table is. A read failure
        // degrades to "no history": every item then reads as first seen now, which is the same
        // answer this surface gave before the table existed.
        let previous: HashMap<(PathBuf, DeleteDirection), WithheldDeletion> =
            load_withheld_deletions(&self.pair.connection)
                .unwrap_or_else(|error| {
                    warn!(%error, "failed to read the withheld-deletion ages; reporting them as new");
                    Vec::new()
                })
                .into_iter()
                .map(|item| ((item.path.clone(), item.direction), item))
                .collect();
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
            if matching_delete_approval(
                &self.pair.connection,
                &action.path,
                direction,
                &fingerprint,
            )? {
                gate.consumed_approvals
                    .push((action.path.clone(), direction));
            } else {
                // The stored age applies only while the fingerprint still matches: a different
                // fingerprint at the same path is a different deletion, and inheriting the old
                // one's age would date it from something that already resolved.
                let first_seen = previous
                    .get(&(action.path.clone(), direction))
                    .filter(|stored| stored.fingerprint == fingerprint)
                    .map_or(now, |stored| stored.first_seen_epoch_secs);
                let (subtree_files, subtree_bytes) = if is_directory {
                    let (files, bytes) = subtree_totals(base_index, &action.path);
                    (Some(files), Some(bytes))
                } else {
                    (None, None)
                };
                gate.withheld_paths.insert(action.path.clone());
                gate.pending.push(PendingDeletion {
                    path: action.path.clone(),
                    direction,
                    entity_kind: action.entity_kind,
                    fingerprint: fingerprint.clone(),
                    detected_epoch_secs: now,
                    first_seen_epoch_secs: first_seen,
                    subtree_files,
                    subtree_bytes,
                    // What this daemon will really do, not what the config file currently says: a
                    // save needs a restart, so the file leads the running daemon and a client
                    // reading it would drop the warnings early (see `LocalDisposal`).
                    disposal: LocalDisposal::of(direction, self.pair.config.local_delete_mode),
                });
                gate.withheld.push(WithheldDeletion {
                    path: action.path.clone(),
                    direction,
                    fingerprint,
                    first_seen_epoch_secs: first_seen,
                });
            }
        }
        // The gate's output IS the complete withheld set for this pass, so this replace is also what
        // clears the table on the first pass that withholds nothing. Skipped when nothing moved —
        // the steady state of a queue nobody has answered is the SAME set every ~30s, and an
        // unchanged set is a DELETE+INSERT+fsync saying nothing (`record_unsyncable` guards the
        // identical shape). Display-only either way, hence warn rather than fail: a pass must not
        // die because an age could not be filed.
        let unchanged = gate.withheld.len() == previous.len()
            && gate
                .withheld
                .iter()
                .all(|item| previous.get(&(item.path.clone(), item.direction)) == Some(item));
        if !unchanged
            && let Err(error) = replace_withheld_deletions(&self.pair.connection, &gate.withheld)
        {
            warn!(%error, "failed to persist the withheld-deletion ages");
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
    /// Seals this pass's history row and re-reads the published summary.
    ///
    /// Called from `reconcile_blocking`'s three-arm outcome match — after every side effect, and
    /// after `execute_plan_and_commit`'s own final commit. An idle pass writes nothing at all
    /// (see [`PassLog::is_notable`]). Errors are logged, never propagated: this is display data,
    /// and a full disk must not turn a successful sync into a failed one.
    fn finalize_pass_record(
        &mut self,
        outcome: PassOutcomeKind,
        failed: usize,
        error: Option<&str>,
    ) {
        if let Err(problem) = self.write_pass_record(outcome, failed, error) {
            warn!(error = %problem, "failed to record this pass in the sync history");
        }
        self.refresh_pass_history();
    }

    fn write_pass_record(
        &mut self,
        outcome: PassOutcomeKind,
        failed: usize,
        error: Option<&str>,
    ) -> AppResult<()> {
        if !self.pair.pass_log.is_notable(outcome) {
            return Ok(());
        }
        let id = match self.pair.pass_log.id {
            Some(id) => id,
            // A notable pass that committed no event (a clean full sweep, or a pass that failed
            // before planning) opens its row here instead.
            None => begin_pass(
                &self.pair.connection,
                self.pair.pass_log.started_epoch_secs,
                self.pair.pass_log.kind,
            )?,
        };
        self.pair.pass_log.id = Some(id);
        finish_pass(
            &self.pair.connection,
            id,
            self.pair.pass_log.started.elapsed().as_millis() as u64,
            outcome,
            self.pair.pass_log.changed,
            failed,
            self.pair.pass_log.bytes_uploaded,
            self.pair.pass_log.bytes_downloaded,
            error,
        )?;
        // Only a pass that wrote rows pays for the prune.
        prune_history(
            &self.pair.connection,
            current_epoch_secs(),
            HistoryRetention::default(),
        )?;
        // The ancestor summaries are bounded here too, on the same "only a pass that wrote rows
        // pays" rule (#217). Evicting one costs the say-less card and nothing else, which is what
        // makes any bound defensible on a standing per-file table.
        prune_agreed_summaries(&self.pair.connection)?;
        Ok(())
    }

    /// Re-reads the three published history queries. Run after **every** pass, idle ones included:
    /// `today`'s window moves at local midnight, and an idle pass is often the only thing running
    /// when it does.
    fn refresh_pass_history(&mut self) {
        match self.load_pass_history() {
            Ok(history) => self.pair.pass_history = Some(history),
            Err(error) => warn!(%error, "failed to read the sync history"),
        }
    }

    fn load_pass_history(&self) -> AppResult<PassHistory> {
        Ok(PassHistory {
            recent: recent_passes(&self.pair.connection, PASS_HISTORY_REPORTED)?,
            last_full_sweep: last_full_sweep(&self.pair.connection)?,
            today: byte_totals_since(&self.pair.connection, local_day_start(current_epoch_secs()))?,
        })
    }

    /// Records the strategy this pass is running, on both the durable row and the live pass block.
    /// One setter, so the two can never disagree.
    fn set_pass_kind(&mut self, kind: PassKind) {
        self.pair.pass_log.kind = kind;
        self.shared
            .update_pass_progress(|pass| pass.kind = kind.as_str().to_owned());
    }

    fn record_status_history(&mut self, message: &str) {
        let entry = StatusHistoryEntry {
            epoch_secs: current_epoch_secs(),
            message: message.to_owned(),
            last_error: self.pair.last_error.clone(),
            plan_summary: self.pair.last_plan_summary.clone(),
            successful_sync_summary: self.pair.last_successful_sync_summary.clone(),
            failed_item_count: self.pair.last_failed_item_count,
        };
        self.pair.status_history.push(entry);
        if self.pair.status_history.len() > STATUS_HISTORY_LIMIT {
            let remove_count = self.pair.status_history.len() - STATUS_HISTORY_LIMIT;
            self.pair.status_history.drain(0..remove_count);
        }
        if let Err(error) =
            write_status_history(&self.pair.status_history_path, &self.pair.status_history)
        {
            warn!(
                path = %self.pair.status_history_path.display(),
                error = %error,
                "failed to persist daemon status history"
            );
        }
        if let Err(error) = self.write_metrics_snapshot() {
            warn!(
                path = %self.pair.metrics_path.display(),
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
        write_metrics_snapshot(&self.pair.metrics_path, &self.metrics_snapshot())
    }

    fn last_sync_epoch_secs(&self) -> Option<u64> {
        self.pair
            .last_sync
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
    direction: Option<DeleteDirection>,
) -> AppResult<String> {
    let Some(selector) = selector else {
        return Ok(
            "no target: pass a relative path, or \"all\" to act on every pending deletion"
                .to_owned(),
        );
    };
    let Selection { target, matches } = select_pending(pending_deletions, selector, literal_path);

    if matches.is_empty() {
        return Ok(match target {
            // Nothing pending answers to this path. For an `approve` that is not necessarily a
            // typo: a plan can name a deletion no pass has withheld yet, which is the Plan
            // screen's typed-`DELETE` gate (#227). Falls through to a pre-approval, which is
            // stricter than this path — it names the direction and pins to the index itself.
            Some(path) if approve => {
                return pre_approve_planned_deletion(connection, path, direction);
            }
            Some(path) => format!("no pending deletion matches '{}'", truncate_selector(path)),
            // NEVER a pre-approval: "all" means every currently-pending item, never everything
            // for ever, and there is nothing to enumerate ahead of a pass.
            None => "no deletions are pending approval".to_owned(),
        });
    }
    // Fail closed on the destructive side only: a `deny` over-revoking is the safe direction.
    if approve && let Some(message) = ambiguous_selector_message(&matches, target, "approve") {
        return Ok(message);
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

/// Records an approval for a deletion this daemon has **planned but not yet withheld** (#227) — the
/// Plan screen authorises its plan's deletion before the pass runs, and by construction nothing is
/// pending at that moment (an action becomes pending only when `decide_delete_gate` withholds it).
/// `decide_delete_gate` needs no change to consume it: the approval it looks for is
/// `(path, direction, fingerprint)`, and this writes exactly that row.
///
/// Three things make it as safe as approving a pending item, and each of them refuses rather than
/// guesses:
///
/// * **The direction must be named.** A path alone does not say which of the two deletions at it is
///   authorised, and an ambiguous selector authorises nothing (#298, one step earlier in time).
/// * **The path is validated before it is stored.** This is the first place `approve` writes a path
///   the daemon did not itself publish, so it clears `validate_relative_path_non_empty` — the plain
///   form maps `""`/`.` to the root, and an approval for the root is an approval for everything.
/// * **The fingerprint comes from the index, not from the client.** It is derived exactly as
///   `delete_fingerprint` derives it from a baseline record, so the pass matches it or the approval
///   is inert. A path with no baseline record (or a record carrying neither digest nor remote id)
///   has no fingerprint to pin to and is refused: `delete_fingerprint`'s remaining fallback is the
///   *action's* remote id, which cannot be known before the plan exists, so a row written here
///   could never match and would be a standing authorisation that only looks like one.
fn pre_approve_planned_deletion(
    connection: &Connection,
    selector: &str,
    direction: Option<DeleteDirection>,
) -> AppResult<String> {
    // Bounded once, here, and every sentence below interpolates the bounded form: the selector is
    // the client's own bytes and a reply is read into memory whole (#313). The *matching* above and
    // the validation below still use the full `selector`.
    let shown = truncate_selector(selector);
    let Some(direction) = direction else {
        return Ok(format!(
            "no pending deletion matches '{shown}'; to approve a deletion that is planned but \
             not yet withheld, name its direction (`--direction local` to delete it here, \
             `--direction remote` to delete it on Proton Drive)"
        ));
    };
    let Some(path) = crate::validate_relative_path_non_empty(Path::new(selector)) else {
        return Ok(format!(
            "'{shown}' is not a valid relative path inside the sync root"
        ));
    };
    let Some(fingerprint) = get_record(connection, &path)?.and_then(|record| {
        record
            .sha1_hash
            .clone()
            .or_else(|| record.proton_id.clone())
    }) else {
        return Ok(format!(
            "'{shown}' has no synced record to pin an approval to; nothing was approved — \
             approve it from the deletions queue once the daemon has planned it"
        ));
    };
    upsert_delete_approval(
        connection,
        &path,
        direction,
        &fingerprint,
        current_epoch_secs() as i64,
    )?;
    Ok(format!(
        "approved 1 planned deletion(s); it applies when the daemon plans it, and only while \
         '{shown}' still matches what you approved"
    ))
}

/// Refuses withheld deletions: purge the baseline record for each (and its whole subtree) so the
/// surviving side is adopted back onto the other one, and revoke any standing approval for it.
///
/// **Why a purge is the whole primitive.** A withheld deletion is a triangle — one side has the
/// entity, the other does not, and the baseline says both used to. Remove the baseline row and the
/// remaining side is simply new: `plan_ongoing_action` never sees a deletion again, the bootstrap
/// arm uploads (`direction: local`) or downloads (`direction: remote`) it, and the queue empties
/// because the planner stops deriving the action rather than because anything remembers a refusal.
///
/// **This is not an index write ahead of a side effect.** It records nothing that happened; it
/// *forgets* state so the next pass re-derives from ground truth, the same class as the approval
/// writes that already run on this connection.
///
/// Returns the reply and whether anything changed, because a refusal that did change something
/// needs a pass — and a full-walk one (see the caller).
fn apply_keep_command(
    connection: &Connection,
    pending_deletions: &[PendingDeletion],
    selector: Option<&str>,
    literal_path: bool,
) -> AppResult<(String, bool)> {
    let Some(selector) = selector else {
        return Ok((
            "no target: pass a relative path, or \"all\" to keep every pending deletion".to_owned(),
            false,
        ));
    };
    let Selection { target, matches } = select_pending(pending_deletions, selector, literal_path);
    if matches.is_empty() {
        return Ok((
            match target {
                Some(path) => format!("no pending deletion matches '{}'", truncate_selector(path)),
                None => "no deletions are pending approval".to_owned(),
            },
            false,
        ));
    }
    // Fail closed exactly as `approve` does, and for a reason `deny` does not share: `deny` only
    // revokes — its effect is that nothing happens — while keeping MUTATES the index and puts
    // content back on a side the user may have deliberately cleared, on rows they provably cannot
    // tell apart.
    if let Some(message) = ambiguous_selector_message(&matches, target, "keep") {
        return Ok((message, false));
    }

    let transaction = connection.unchecked_transaction()?;
    let mut kept = 0usize;
    let mut stale = 0usize;
    for pending in &matches {
        // Pinned like an approval is. The published queue can be a pass stale, so re-derive the
        // fingerprint from the index and refuse when it has moved: a refusal that no longer names
        // what the user saw is inert, not "close enough".
        let current = get_record(&transaction, &pending.path)?.and_then(|record| {
            record
                .sha1_hash
                .clone()
                .or_else(|| record.proton_id.clone())
        });
        if current.as_deref() != Some(pending.fingerprint.as_str()) {
            stale += 1;
            continue;
        }
        // EVERY purged path, in BOTH directions, and that is the same argument the root's own
        // approval makes one level up: a purged record leaves any approval at that path pointing at
        // nothing the user can see. It is reachable — a descendant can carry an approval from an
        // earlier pass that queued it individually, or from a pre-pass `approve --direction`
        // (#227) — and it is not inert, because the fingerprint an approval is pinned to is the
        // content's own: re-adopt the file unchanged and the stale approval matches the new record
        // exactly, so the next deletion of that path would execute without ever being shown.
        // Over-revoking is the safe direction here, as it is for `deny`.
        for purged in purge_subtree_records(&transaction, &pending.path)? {
            for direction in [DeleteDirection::Local, DeleteDirection::Remote] {
                delete_delete_approval(&transaction, &purged, direction)?;
            }
        }
        kept += 1;
    }
    transaction.commit()?;

    if kept == 0 {
        return Ok((
            format!(
                "nothing was kept: {stale} pending deletion(s) no longer match what was shown; \
                 check `proton-sync pending` and try again"
            ),
            false,
        ));
    }
    let stale_note = if stale > 0 {
        format!("; {stale} no longer matched what was shown and were left alone")
    } else {
        String::new()
    };
    Ok((
        format!(
            "kept {kept} pending deletion(s); the other side is put back on the next sync{stale_note}"
        ),
        true,
    ))
}

/// Which pending deletions a selector names. Shared by `approve`/`deny` and `keep` so one rule
/// decides what a selector means; a second copy is what eventually disagrees.
struct Selection<'a> {
    /// `None` for the explicit `"all"` form (every pending item); `Some` for a literal path.
    target: Option<&'a str>,
    matches: Vec<&'a PendingDeletion>,
}

fn select_pending<'a>(
    pending_deletions: &'a [PendingDeletion],
    selector: &'a str,
    literal_path: bool,
) -> Selection<'a> {
    // A literal-path request never gets the "all" interpretation.
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
    Selection { target, matches }
}

/// The reply for a targeted selector that names more than one pending deletion, or `None` when it
/// names at most one.
///
/// The planner emits at most one delete per path, so this happens only when two real paths differ
/// solely in bytes the lossy wire replaced (#61). Acting on both would touch an item the user never
/// picked out — and they cannot pick it out, the rows render identically.
///
/// Names `--all`, NOT the wire's `"all"` selector: this branch is only reachable from a `<PATH>`
/// argument, which the CLI always sends with `literal_path` set — so `proton-sync approve all`
/// would be read as a path literally named `all` and do nothing. Telling the user to type the one
/// thing that provably cannot work is worse than saying nothing.
fn ambiguous_selector_message(
    matches: &[&PendingDeletion],
    target: Option<&str>,
    command: &str,
) -> Option<String> {
    let target = target?;
    if matches.len() < 2 {
        return None;
    }
    // The client's own selector, not a path the daemon published — #313's issue body reads this
    // arm the other way round. Bounded like every other echo of it (the *matching* above is
    // untouched and still compares the full wire form).
    let target = truncate_selector(target);
    let past_tense = if command == "keep" {
        "kept"
    } else {
        "approved"
    };
    Some(format!(
        "{} pending deletions render as '{target}' and cannot be told apart on the wire (their \
         paths are not valid UTF-8); nothing was {past_tense} — {command} every pending deletion \
         instead with `proton-sync {command} --all`",
        matches.len(),
    ))
}

/// What the control task needs to answer the read-only `list` verb (#99) without going near the
/// daemon core: the daemon's **own** client (so every `proton-drive` invocation in this process
/// shares one `proton::CliGate`) and the remote root its selectors resolve against.
struct BrowseContext<C: ProtonClient> {
    proton: Arc<C>,
    remote_root: PathBuf,
    /// How long a browse may wait for the CLI gate before answering `busy`. A field rather than
    /// the constant so tests can drive it at a cadence they can observe, like `ipc_io_timeout`.
    gate_wait: Duration,
}

/// Everything a control connection is served from, in one place: the published snapshot, the IPC
/// task's own database handle, the channel to the daemon core, and the `list` verb's context.
/// Built once in [`Daemon::run`] and shared by every connection task.
struct ControlPlane<C: ProtonClient> {
    shared: Arc<ControlShared>,
    /// The IPC task's *second* connection to the index — the core owns the first. Both set a busy
    /// timeout, so a rare same-database collision waits rather than failing.
    approvals: tokio::sync::Mutex<Connection>,
    loop_tx: mpsc::UnboundedSender<LoopCommand>,
    io_timeout: Duration,
    metrics_path: PathBuf,
    cancel_flag: Arc<AtomicBool>,
    browse: BrowseContext<C>,
}

/// Accept loop for the control socket, run on its own task so control requests are served while
/// the daemon core is blocked in a reconcile. Each connection is handled on a further spawned
/// task, so one stalled client cannot delay the others. Aborted by `run()` on shutdown.
async fn serve_control_socket<C: ProtonClient + 'static>(
    listener: UnixListener,
    plane: Arc<ControlPlane<C>>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let plane = Arc::clone(&plane);
                tokio::spawn(async move {
                    if let Err(error) = handle_control_connection(stream, &plane).await {
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
async fn handle_control_connection<C: ProtonClient + 'static>(
    stream: UnixStream,
    plane: &ControlPlane<C>,
) -> AppResult<()> {
    let ControlPlane {
        shared,
        approvals,
        loop_tx,
        io_timeout,
        metrics_path,
        cancel_flag,
        browse,
    } = plane;
    let io_timeout = *io_timeout;
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
            shared.pair.paused.store(true, Ordering::SeqCst);
            info!("sync paused");
            persist_metrics_best_effort(shared, metrics_path);
            shared.response("sync paused")
        }
        ControlCommand::Resume => {
            shared.pair.paused.store(false, Ordering::SeqCst);
            info!("sync resumed");
            persist_metrics_best_effort(shared, metrics_path);
            shared.response("sync resumed")
        }
        ControlCommand::Syncnow => {
            if shared.is_paused() {
                shared.response("sync skipped because daemon is paused")
            } else {
                // `a_counted_pass_is_running`, not `is_syncing`: a rehearsal claims `syncing`
                // but bumps no `reconcile_seq`, so telling a client "another pass will follow"
                // would make it wait for a count the queued sync cannot reach alone.
                let message = if shared.a_counted_pass_is_running() {
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
            shared.pair.force_full_walk.store(true, Ordering::SeqCst);
            if shared.is_paused() {
                shared.response("full resync queued; it will run when syncing resumes")
            } else {
                let message = if shared.a_counted_pass_is_running() {
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
        ControlCommand::ResetIndex => {
            // Latched, not applied here: this task holds a *second* connection to the same
            // database and a reconcile may be mid-checkpoint on the first one. Order matters —
            // latch the reset before scheduling the pass that consumes it, and latch the full walk
            // with it so a cleared cursor cannot be mistaken for a warm-start opportunity.
            shared.pair.reset_index.store(true, Ordering::SeqCst);
            shared.pair.force_full_walk.store(true, Ordering::SeqCst);
            if shared.is_paused() {
                shared.response("index reset queued; it will run when syncing resumes")
            } else if loop_tx.send(LoopCommand::SyncNow).is_ok() {
                shared.response("index reset scheduled; the next pass rebuilds from scratch")
            } else {
                shared.response("daemon is shutting down; index reset not scheduled")
            }
        }
        ControlCommand::Approve | ControlCommand::Deny => {
            let approve = request.command == ControlCommand::Approve;
            let pending = shared
                .pair
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
                request.direction,
            )?;
            drop(connection);
            if approve {
                info!(argument = ?request.argument, "delete approval recorded");
            } else {
                info!(argument = ?request.argument, "delete approval revoked");
            }
            shared.response(&message)
        }
        ControlCommand::Activity => {
            // Answered from the index on this task, like `approve`/`deny` — a bounded, indexed
            // read over the history tables, not daemon work.
            let connection = approvals.lock().await;
            let outcome = query_file_history(&connection, &request);
            drop(connection);
            match outcome {
                Ok(history) => {
                    let mut response = shared.response("sync activity");
                    response.file_history = Some(history);
                    response
                }
                // Reported in the reply rather than dropping the connection: history is display
                // data, and a client that asked for it must be told why it is missing.
                Err(error) => {
                    warn!(%error, "failed to read the per-file sync history");
                    let mut response = shared.response("sync activity unavailable");
                    response.last_error = Some(error.to_string());
                    response
                }
            }
        }
        ControlCommand::Keep => {
            let pending = shared
                .pair
                .snapshot
                .lock()
                .expect("control snapshot lock")
                .pending_deletions
                .clone();
            let connection = approvals.lock().await;
            let (message, changed) = apply_keep_command(
                &connection,
                &pending,
                request.argument.as_deref(),
                request.literal_path,
            )?;
            drop(connection);
            // Nothing purged (no match, an ambiguous selector, a stale fingerprint) means nothing
            // to put back, and no reason to spend a full walk on it.
            let message = if !changed {
                message
            } else {
                info!(argument = ?request.argument, "withheld deletion refused");
                // A FULL WALK, not just a pass, and this is what makes the `remote` direction work
                // at all. An incremental pass builds its remote map as
                // `reconstruct_remote(base ⊕ delta)`, so the baseline IS the remote view: the
                // record this refusal just purged is gone from every reconstructed map, and no
                // remote event will ever re-derive it (the deletion happened HERE, so the volume
                // stream never mentioned it). Without this latch, "bring it back to this computer"
                // downloads nothing, ever, until something else forces a full walk — and the
                // periodic resync is off by default. Latched for both directions: one rule, and it
                // costs one full walk on a rare user click.
                shared.pair.force_full_walk.store(true, Ordering::SeqCst);
                if shared.is_paused() {
                    format!("{message} (queued: syncing is paused)")
                } else if loop_tx.send(LoopCommand::SyncNow).is_ok() {
                    message
                } else {
                    format!("{message} (the daemon is shutting down; no sync was scheduled)")
                }
            };
            shared.response(&message)
        }
        ControlCommand::List => {
            // The one control command that runs work on this task, and the only place the
            // "answer from the snapshot, never by running work here" contract bends. Two
            // properties keep that bend bounded, and neither is the connection's own IO timeout
            // (which covers the read and the write, not the work between them):
            //
            //   * the wait for the CLI gate is capped at `gate_wait`, so a browse behind a
            //     multi-GB transfer answers `busy` rather than parking the connection;
            //   * the gate then admits ONE listing at a time, so however often a client retries,
            //     at most one request is ever running the CLI and every other one is refused
            //     within `gate_wait` — there is no pile-up of blocking tasks.
            //
            // The listing itself is bounded only by the CLI's own command timeout, which can
            // outlast a client's patience; that costs a dropped reply, not a stuck daemon.
            let outcome =
                browse_remote_directory(browse, shared, request.argument.as_deref(), request.limit)
                    .await;
            let message = match &outcome {
                ListingOutcome::Listed { .. } => "remote listing",
                ListingOutcome::Busy => "remote listing unavailable: the proton-drive CLI is busy",
                ListingOutcome::Failed { .. } | ListingOutcome::Unknown => "remote listing failed",
            };
            let mut response = shared.response(message);
            response.listing = Some(outcome);
            response
        }
        ControlCommand::Plan => {
            // `Syncnow`-shaped, and deliberately not `List`-shaped: the work is a full local
            // stat-walk plus an O(folders) remote walk, which must never run on this task. Ack now,
            // compute on the core. Pause semantics mirror `syncnow`'s exactly — a paused daemon
            // runs no passes, and a plan is a pass.
            if shared.is_paused() {
                let mut response = shared.response("plan skipped because daemon is paused");
                response.plan = Some(PlanOutcome::Paused);
                response
            } else {
                // Booked BEFORE the reply is built, so a client polling straight after its ack
                // reads `Computing` rather than the previous plan as its own answer.
                let plan_seq = shared.book_plan_request();
                if loop_tx.send(LoopCommand::PlanNow).is_ok() {
                    let mut response = shared.response("plan scheduled");
                    response.plan = Some(PlanOutcome::Scheduled { plan_seq });
                    response
                } else {
                    // Un-book it: a request nothing will ever seal leaves every later poller
                    // reading `Computing` for ever.
                    shared.unbook_plan_request();
                    let mut response =
                        shared.response("daemon is shutting down; plan not scheduled");
                    response.plan = Some(PlanOutcome::Failed {
                        plan_seq,
                        error: "the daemon is shutting down".to_owned(),
                    });
                    response
                }
            }
        }
        ControlCommand::PlanResult => {
            // A query, answered straight from the published slot — no work on this task, exactly
            // like `status`. Carries the last apply's verdict too: a client that asked to apply
            // learns "applied" from here, and on a divergence finds the new plan in the same reply.
            let mut response = shared.response("plan result");
            response.plan = Some(shared.plan_outcome(request.limit));
            response.apply = shared.apply_outcome();
            response
        }
        ControlCommand::Apply => {
            let outcome = schedule_apply(
                shared,
                loop_tx,
                request.argument.as_deref(),
                request.skip_destructive,
            );
            let message = match &outcome {
                ApplyOutcome::Scheduled { .. } => "apply scheduled",
                ApplyOutcome::Stale => {
                    "that plan is no longer the current one; review the plan again and apply that"
                }
                ApplyOutcome::Paused => "apply skipped because daemon is paused",
                ApplyOutcome::Failed { .. } => "daemon is shutting down; apply not scheduled",
                // Unreachable from `schedule_apply`, which never returns a verdict — those are the
                // pass's to write. Named rather than folded into an existing arm so a new verdict
                // has to be given a sentence rather than inherit one.
                ApplyOutcome::Applied { .. }
                | ApplyOutcome::Diverged { .. }
                | ApplyOutcome::Unknown => "apply",
            };
            let mut response = shared.response(message);
            response.apply = Some(outcome);
            response
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

/// Books a [`ControlCommand::Apply`] and schedules the pass that carries it out (#100/#192).
///
/// **Nothing is executed here, and nothing is even decided here.** This task only checks that the
/// token names the plan the daemon currently holds — one stored plan at a time, latest wins — and
/// hands the work to the core. The real comparison happens inside the pass, against the plan it
/// freshly computes, because a token matching the *stored* plan says only that no newer plan has
/// been computed, not that the tree still looks like it did.
fn schedule_apply(
    shared: &ControlShared,
    loop_tx: &mpsc::UnboundedSender<LoopCommand>,
    token: Option<&str>,
    skip_destructive: bool,
) -> ApplyOutcome {
    // A missing token is a stale token: an apply must name what it authorises, and "whatever you
    // have" is exactly the review/apply divergence window #100 closes.
    let Some(token) = token else {
        return ApplyOutcome::Stale;
    };
    if shared.is_paused() {
        return ApplyOutcome::Paused;
    }
    let Some(apply_seq) = shared.book_apply_request(token, skip_destructive) else {
        return ApplyOutcome::Stale;
    };
    if loop_tx.send(LoopCommand::SyncNow).is_ok() {
        info!(skip_destructive, "apply scheduled for a reviewed plan");
        ApplyOutcome::Scheduled { apply_seq }
    } else {
        shared.unbook_apply_request();
        ApplyOutcome::Failed {
            apply_seq,
            error: "the daemon is shutting down".to_owned(),
        }
    }
}

/// Which directory a [`ControlCommand::List`] selector names — the two frames the verb accepts, so
/// that "what does this path mean" is decided once rather than at each use (#99/#323).
enum BrowseTarget {
    /// The historical selector: a path relative to the daemon's configured `remote_root`, empty for
    /// the root itself.
    UnderRoot(PathBuf),
    /// An absolute location on Proton Drive, outside every configured root. The folder probe's
    /// question — "how much is under this candidate?" — is asked before any pair exists, so no
    /// relative form can express it.
    Absolute(PathBuf),
}

impl BrowseTarget {
    /// `(root, relative)` for [`ProtonClient::browse_directory`]. An absolute target is its own
    /// root with an empty relative part, which is the same shape the remote-root browse already
    /// takes — so the CLI call, the wrapper-node drop and the parent filter below are one code
    /// path, not two.
    fn request<'a>(&'a self, remote_root: &'a Path) -> (&'a Path, &'a Path) {
        match self {
            Self::UnderRoot(relative) => (remote_root, relative.as_path()),
            Self::Absolute(absolute) => (absolute.as_path(), Path::new("")),
        }
    }

    /// One listed key, moved into the frame the *selector* was written in.
    ///
    /// **The reply always speaks the request's own frame**, which is the rule that keeps the
    /// extension from being an overload: entries under a relative selector stay relative to
    /// `remote_root` exactly as before, and entries under an absolute one come back absolute — so a
    /// client can feed any entry straight back as the next selector, and never has to know which
    /// root a path it was handed is relative to. (Relative-to-the-*requested-directory* was the
    /// other option and is worse: for the absolute case it would duplicate `RemoteEntry::name`.)
    fn entry_path(&self, key: PathBuf) -> PathBuf {
        match self {
            Self::UnderRoot(_) => key,
            Self::Absolute(absolute) => absolute.join(key),
        }
    }

    /// The directory this reply echoes back, in that same frame.
    fn echo(&self) -> PathBuf {
        match self {
            Self::UnderRoot(relative) => relative.clone(),
            Self::Absolute(absolute) => absolute.clone(),
        }
    }
}

/// Answers a [`ControlCommand::List`] request: one non-recursive remote directory listing (#99).
///
/// The selector is an **external path**, so it is validated before it reaches the CLI — and *which*
/// guard it takes is decided by the selector itself, because the verb accepts two frames:
///
/// * **Relative** (the default): [`crate::validate_relative_path`], not the non-empty form the
///   path-safety ladder demands of a side effect. The empty path is a legitimate *value* here: it
///   is the remote root, a remote browser's landing page. Joining it onto the root promotes
///   nothing, because listing a directory is a read; the rule the non-empty form exists for (#72)
///   is about a per-entry download or delete silently becoming a whole-root one.
/// * **Absolute** (#323): [`crate::validate_absolute_remote_path`], whose doc states exactly what
///   an absolute selector may and may not reach. It names a folder outside every configured root,
///   which is the folder probe's question — asked before a pair is chosen, so `remote_root` does
///   not exist yet to be relative to. Absolute selectors were previously *refused* by the relative
///   guard, so no client can have been relying on the old meaning: this is a pure extension.
///
/// Both frames are one CLI invocation under one bounded gate wait. A caller that wants a whole
/// subtree walks it by issuing one request per directory, which is the shape that keeps this verb
/// on the IPC task at all — see [`ControlCommand::List`].
///
/// Every failure is reported as a [`ListingOutcome`] rather than by dropping the connection: a
/// client that asked for a listing must be told which of "retry", "gone" and "broken" it got, and
/// must never have to read that out of prose.
async fn browse_remote_directory<C: ProtonClient + 'static>(
    browse: &BrowseContext<C>,
    shared: &ControlShared,
    selector: Option<&str>,
    limit: Option<usize>,
) -> ListingOutcome {
    let selector = selector.unwrap_or("");
    let validated = if Path::new(selector).is_absolute() {
        crate::validate_absolute_remote_path(Path::new(selector)).map(BrowseTarget::Absolute)
    } else {
        crate::validate_relative_path(Path::new(selector)).map(BrowseTarget::UnderRoot)
    };
    let Some(target) = validated else {
        return ListingOutcome::Failed {
            // Bounded like the other two failure arms, and here it is the *client's* bytes being
            // echoed: a control request may carry up to `MAX_REQUEST_BYTES`, so a 64 KiB selector
            // that fails validation would otherwise put 64 KiB of someone else's string on the
            // wire. The one rule covers all three arms rather than the two that happened to
            // originate remotely (Copilot review). The bound is on the *selector* rather than on
            // the finished sentence (#313), so the prefix naming the failure always survives.
            error: format!("unsafe remote path: {}", truncate_selector(selector)),
        };
    };
    let proton = Arc::clone(&browse.proton);
    let gate_wait = browse.gate_wait;
    // What the CLI is asked for, in the CLI's own terms: an absolute target is its own root with an
    // empty relative part. `relative` is what the reply's entry keys are relative to, and is used
    // again by the parent filter below.
    let (root, relative) = target.request(&browse.remote_root);
    let (root, relative) = (root.to_path_buf(), relative.to_path_buf());
    let listed_relative = relative.clone();
    // `spawn_blocking`: the listing is a synchronous subprocess. Running it inline would block a
    // runtime worker, which is exactly what would stop the *other* control connections being
    // served — the property this whole task layout exists for.
    let listed = tokio::task::spawn_blocking(move || {
        proton.browse_directory(&root, &listed_relative, gate_wait)
    })
    .await;

    let entities = match listed {
        Ok(Ok(entities)) => {
            // Success evidence, and the freshest there is: something reached Proton just now.
            publish_auth_evidence(shared, AuthState::SignedIn);
            entities
        }
        Ok(Err(error)) if is_cli_busy_error(error.as_ref()) => {
            // Nothing was attempted, so this is evidence of nothing. Leaving the auth state alone
            // here is the same rule the unclassified-failure arm below follows.
            debug!(%error, "remote listing skipped: the proton-drive CLI is busy");
            return ListingOutcome::Busy;
        }
        Ok(Err(error)) => {
            // Per-request, not per-pass: a user asked for this, so one line per request is
            // information rather than the pass-rate noise `note_event_scope_declined` guards
            // against.
            // The selector's own frame, not `relative` — which is empty for an absolute target and
            // would log every failed probe as a failure at the remote root.
            warn!(path = %target.echo().display(), %error, "remote listing failed");
            if is_auth_failure_error(error.as_ref()) {
                publish_auth_evidence(shared, AuthState::SignedOut);
            }
            return ListingOutcome::Failed {
                // Bounded like every other error this engine puts on the wire (#136's
                // `FailedItem::error`): a failing CLI invocation can carry kilobytes of stderr,
                // and the reply is read into memory whole. The log line above kept it in full.
                error: truncate_error(&error.to_string()),
            };
        }
        Err(join_error) => {
            warn!(%join_error, "the remote-listing task failed");
            return ListingOutcome::Failed {
                error: truncate_error(&format!("remote listing task failed: {join_error}")),
            };
        }
    };

    let mut entries: Vec<RemoteEntry> = entities
        .into_iter()
        // "In this folder" is exactly "one level down from it", and saying it that way covers two
        // things with one rule. The CLI wraps a listing in the directory's own node, so the
        // requested directory comes back keyed by its own path — its parent is not `relative`, so
        // it drops, and browsing the root does not report the root inside itself. And the listing
        // is supposed to be non-recursive: if the CLI ever nests a second level, a grandchild's
        // parent is not `relative` either, so a browse cannot silently flatten a subtree into what
        // reads as one folder's contents.
        .filter(|(path, _)| path.parent() == Some(relative.as_path()))
        // Into the selector's own frame, and only here: everything above works in the frame the
        // CLI answered in (relative to whatever root was listed), so the reframing is one map, not
        // a rule each branch has to remember. A relative request is the identity.
        .map(|(path, entity)| (target.entry_path(path), entity))
        .map(|(path, entity)| match entity {
            RemoteEntity::File(file) => RemoteEntry {
                path,
                name: file.name,
                entity_kind: EntityKind::File,
                sha1: file.sha1_hash,
                downloadable: file.downloadable,
            },
            RemoteEntity::Directory(directory) => RemoteEntry {
                path,
                name: directory.name,
                entity_kind: EntityKind::Directory,
                sha1: None,
                // A directory is not a download; `false` here says nothing about its contents.
                downloadable: false,
            },
        })
        .collect();
    // Directories first, then by name: a stable order a browser can render without sorting, and
    // one a test can assert on. `HashMap` iteration order is otherwise arbitrary.
    entries.sort_by(|left, right| {
        (left.entity_kind == EntityKind::File)
            .cmp(&(right.entity_kind == EntityKind::File))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.path.cmp(&right.path))
    });
    let total = entries.len();
    let limit = limit
        .unwrap_or(LIST_ENTRIES_DEFAULT_LIMIT)
        .clamp(1, LIST_ENTRIES_MAX_LIMIT);
    entries.truncate(limit);
    ListingOutcome::Listed {
        path: target.echo(),
        truncated: total > entries.len(),
        total,
        entries,
    }
}

/// Publishes an auth verdict and logs the transition once; `true` when it changed.
///
/// Only ever called with real evidence — something reached Proton, or a failure the engine
/// *classified* as an auth failure (see [`AuthState`]). Both writers go through here: the control
/// task calls it directly, and the daemon core wraps it in [`PairPass::note_auth_evidence`] to route
/// a sign-out through the once-per-cause latch as well, which needs a `&mut` the IPC task does not
/// have.
fn publish_auth_evidence(shared: &ControlShared, state: AuthState) -> bool {
    let changed = shared.record_auth_state(state);
    if changed {
        info!(auth = state.as_str(), "proton sign-in state changed");
    }
    changed
}

/// The standing reason reported when the Proton session is the thing that is wrong.
const AUTH_DECLINE_REASON: &str = "the proton-drive CLI session is signed out or expired; run `proton-drive login` \
     (the daemon reuses that CLI's session)";

/// A latched process-wide decline: which cause, and the line it was reported with.
///
/// The cause is carried beside the reason so a recovery can clear **its own** cause without a string
/// comparison. There are two of them and they recover independently — a keyring that becomes readable
/// says nothing about a signed-out account — so "clear if the stored reason equals mine" would either
/// need one canonical spelling per cause (which the degraded-session line, formatted with two
/// intervals, cannot have) or would erase the other cause's line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionDecline {
    cause: SessionDeclineCause,
    reason: String,
}

/// The two ways the *process* can fail to stream events. Both are facts about this user's Proton
/// session, which is why they latch on the daemon and not on a pair (see
/// [`Daemon::session_declined`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionDeclineCause {
    /// `events_driven` is on but no event source could be built: a locked keyring, a headless host,
    /// or a `proton-drive` CLI that is not logged in.
    NoSession,
    /// A pass or a `list` failed on a *classified* auth failure (#103).
    SignedOut,
}

/// Latches a process-wide decline and logs it once, re-logging only when the line changes. Same
/// contract as `PairPass::note_event_scope_declined`, at the other scope.
fn note_session_declined(
    latch: &mut Option<SessionDecline>,
    cause: SessionDeclineCause,
    reason: String,
) {
    if latch.as_ref().map(|held| held.reason.as_str()) != Some(reason.as_str()) {
        info!(%reason, "event-driven detection unavailable; using full-tree walks");
        *latch = Some(SessionDecline { cause, reason });
    }
}

/// Clears the latch **only if it is holding `cause`**. The whole point of the typed cause: a
/// recovery must not erase a line that describes something else, or that something else re-reports
/// next pass as if it were new.
fn clear_session_decline(latch: &mut Option<SessionDecline>, cause: SessionDeclineCause) {
    if latch.as_ref().is_some_and(|held| held.cause == cause) {
        *latch = None;
    }
}

/// Answers a [`ControlCommand::Activity`] request. The selector is a **relative path** and is
/// validated like every other externally-sourced path before it reaches a query — it is only ever
/// a lookup key here, never joined onto a root, but an unvalidated path has no business being
/// stored or matched either.
fn query_file_history(
    connection: &Connection,
    request: &crate::ipc::ControlRequest,
) -> AppResult<crate::index::FileHistory> {
    // `0` and `None` are the same request: everything still retained.
    let since = match request.window_secs.unwrap_or(0) {
        0 => 0,
        window => current_epoch_secs().saturating_sub(window),
    };
    let path = match request.argument.as_deref() {
        Some(selector) => Some(resolve_history_path(connection, selector, since)?),
        None => None,
    };
    let limit = request
        .limit
        .unwrap_or(ACTIVITY_EVENTS_DEFAULT_LIMIT)
        .clamp(1, ACTIVITY_EVENTS_MAX_LIMIT);
    file_events(connection, path.as_deref(), since, limit)
}

/// Turns an `activity` selector into the stored path to query.
///
/// A **dispatch on the selector, not a fallback chain**: a selector free of U+FFFD is used as the
/// stored key directly, and only one containing U+FFFD is matched against the **wire rendering** of
/// the paths in the window. So a UTF-8 selector that matches nothing reports an empty window rather
/// than falling through to the scan — which is right, since its own bytes *are* the key and a
/// rendering scan could only return the same row or the wrong one.
///
/// The U+FFFD branch exists because a non-UTF-8 path reaches the user only as `to_string_lossy`:
/// the string they copy off `status` or `activity` no longer carries the bytes the BLOB key was
/// built from, so a byte-exact lookup would answer "nothing moved" for a file that moved. A file
/// legitimately *named* with U+FFFD takes that branch too and still resolves, just via the scan.
///
/// Same rule `apply_approval_command` follows for `approve`/`deny` (#61): a wire path is a
/// rendering, and a rendering that maps to more than one stored path resolves to none of them.
///
/// Read-only, so an ambiguous selector is an error rather than a silent pick — a wrong feed is a
/// wrong answer even when nothing is destroyed.
fn resolve_history_path(
    connection: &Connection,
    selector: &str,
    since_epoch_secs: u64,
) -> AppResult<PathBuf> {
    // Both messages below reach the wire as `ControlResponse.last_error`, so the selector they
    // echo is bounded like every other rendering of the client's own bytes (#313).
    let validated =
        crate::validate_relative_path_non_empty(Path::new(selector)).ok_or_else(|| {
            boxed_error(format!(
                "unsafe history path: {}",
                truncate_selector(selector)
            ))
        })?;
    // A selector that is already a stored key needs no scan. `distinct_event_paths` is bounded by
    // retention, but this keeps the common case a point query.
    if !selector.contains(char::REPLACEMENT_CHARACTER) {
        return Ok(validated);
    }
    // No dedup: the query is `SELECT DISTINCT`, so these differ byte-wise by construction — and
    // two of them rendering alike is precisely the ambiguity the count below refuses.
    let mut matches: Vec<PathBuf> =
        crate::index::distinct_event_paths(connection, since_epoch_secs)?
            .into_iter()
            .filter(|stored| crate::wire_path(stored) == crate::wire_path(&validated))
            .collect();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        // Nothing matched: hand back the validated form so the query runs and reports an empty
        // window, which is the truthful answer for a path with no recorded events.
        0 => Ok(validated),
        count => Err(boxed_error(format!(
            "{} names {count} different recorded paths that render identically; the \
             history cannot say which one is meant",
            truncate_selector(selector)
        ))),
    }
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
/// [`PairPass::decide_delete_gate`]). `withheld_paths` are the destructive actions the execution loop
/// must skip; `pending` is the user-facing view of those; `consumed_approvals` are the approvals to
/// delete once the deletions they authorized have actually run.
#[derive(Default)]
struct DeleteGate {
    withheld_paths: HashSet<PathBuf>,
    pending: Vec<PendingDeletion>,
    consumed_approvals: Vec<(PathBuf, DeleteDirection)>,
    /// The same items as `pending`, in the shape the `withheld_deletions` table stores — the ages
    /// this pass carried forward, ready to be written back (see [`WithheldDeletion`]).
    withheld: Vec<WithheldDeletion>,
}

/// Files beneath `root` in the baseline index, and their total size (#208). Files only: a
/// directory record's `file_size` is its own, never a subtree total, and counting directories would
/// make "1,204 files" answer a question nobody asked. Component-wise containment, and `root` itself
/// is excluded — it is the folder being deleted, not something inside it.
fn subtree_totals(base_index: &HashMap<PathBuf, FileRecord>, root: &Path) -> (u64, u64) {
    base_index
        .iter()
        .filter(|(path, record)| {
            record.entity_kind == EntityKind::File && is_strict_descendant(root, path)
        })
        .fold((0u64, 0u64), |(files, bytes), (_, record)| {
            (files + 1, bytes.saturating_add(record.file_size))
        })
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

/// What this pass's remote map is. Both facts are about the *map*, which is why they travel
/// together rather than as two loose booleans.
#[derive(Debug, Clone, Copy)]
struct RemoteMapShape {
    /// The remote root itself is missing: the map is empty and the baseline was cleared, so the
    /// map describes nothing about the remote tree.
    remote_root_missing: bool,
    /// The map came from a full-tree walk rather than an event-driven reconstruction.
    full_snapshot: bool,
}

impl RemoteMapShape {
    /// Whether a path's **absence** from this map is evidence. Only a full-tree walk that actually
    /// found the root proves it; a reconstruction can only ever prove presence, since a node with
    /// no index record is not in the baseline it overlays (see [`PairPass::record_unsyncable`]).
    fn proves_absence(self) -> bool {
        self.full_snapshot && !self.remote_root_missing
    }
}

/// Whether a pass may drop a standing [`UnsyncableItem`] with this origin **by absence** — i.e.
/// whether this pass looked everywhere the reason could have come from (#232).
///
/// `full_snapshot` here is [`RemoteMapShape::proves_absence`]: a full-tree walk that found its
/// root. It deliberately does not gate the local arm. A missing remote root voids the *remote*
/// evidence and nothing else — the local stat-walk still ran, and it is the whole tree either way,
/// because every caller of [`PairPass::record_unsyncable`] scans locally before it plans.
///
/// Exhaustive by variant, no `_`: a new origin has to answer this question rather than inherit an
/// arm's guess.
fn pass_proves_absence_of(origin: UnsyncableOrigin, full_snapshot: bool) -> bool {
    match origin {
        UnsyncableOrigin::Local => true,
        UnsyncableOrigin::Remote | UnsyncableOrigin::Unknown => full_snapshot,
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

/// Classifies a finished single-file download. `Ok(false)` is the TOCTOU of issue #31: the node
/// was in the listing this plan was built from but is gone now (trashed by another client, or
/// listing lag). The caller then skips the action **whole** — no side effect, no index mutation —
/// and the next pass re-derives it from ground truth. Every other failure still fails the pass.
fn skip_if_node_vanished(result: AppResult<()>, path: &Path) -> AppResult<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if is_node_not_found_error(error.as_ref()) => {
            warn!(
                path = %path.display(),
                %error,
                "skipping download: the remote node is gone since the listing"
            );
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Durably commits everything accumulated since the previous checkpoint — the incremental half
/// of the commit-after-side-effects scheme (see `execute_plan_and_commit`). An index write
/// still never precedes its side effect; a checkpoint only makes *completed* work survive a
/// later failure or shutdown. The event cursor is deliberately not part of any checkpoint (it
/// advances only in the final commit of a fully-successful pass). Approval consumptions ride
/// in the same transaction as the deletion's own index purge.
/// The agreed-version summaries an about-to-commit checkpoint should record (#217).
///
/// One entry per `Upsert` of a `Synced` **file** record whose content can be summarised at all. A
/// file that is not valid UTF-8, is past [`crate::ancestor::MAX_SUMMARY_BYTES`] or has more lines
/// than [`crate::ancestor::MAX_SUMMARY_LINES`] yields nothing, and a conflict on it later draws the
/// say-less card — which is #347's own headline case, the 4 GB video whose card claimed a line had
/// been added to it.
///
/// **Nothing here returns an error.** Every failure is a file that cannot be read or is not text,
/// and the cost of each is one sentence the card does not draw. Letting any of it fail a pass would
/// trade a working sync for a decoration.
fn collect_agreed_summaries(
    index_mutations: &[IndexMutation],
    local_root: &Path,
) -> Vec<(PathBuf, String, LineSummary)> {
    let mut summaries = Vec::new();
    for mutation in index_mutations {
        let IndexMutation::Upsert(record) = mutation else {
            continue;
        };
        // ONLY `Synced`. A `Conflict` record's digest is the local file's, not the agreed one — so
        // capturing there would file the diverged version under the ancestor's name and make every
        // sentence on the card wrong in the confident direction.
        if record.sync_status != SyncStatus::Synced || record.entity_kind != EntityKind::File {
            continue;
        }
        let Some(digest) = record.sha1_hash.as_deref() else {
            continue;
        };
        let Some(absolute) = safe_local_path(local_root, &record.file_path) else {
            continue;
        };
        // `read_to_string` is the UTF-8 test and the read in one: a binary file fails it, which is
        // the answer rather than a problem. The size cap is checked first so a huge file is not
        // read into memory to be discarded.
        let readable = fs::symlink_metadata(&absolute)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() <= MAX_SUMMARY_BYTES as u64)
            .and_then(|_| fs::read_to_string(&absolute).ok());
        let Some(text) = readable else {
            continue;
        };
        if let Some(summary) = LineSummary::of(&text) {
            summaries.push((record.file_path.clone(), digest.to_owned(), summary));
        }
    }
    summaries
}

fn commit_checkpoint(
    connection: &mut Connection,
    index_mutations: &mut Vec<IndexMutation>,
    pending_approval_consumptions: &mut Vec<(PathBuf, DeleteDirection)>,
    pass_log: &mut PassLog,
    shared: &ControlShared,
    local_root: &Path,
) -> AppResult<()> {
    if index_mutations.is_empty()
        && pending_approval_consumptions.is_empty()
        && pass_log.pending.is_empty()
    {
        return Ok(());
    }
    // THE ANCESTOR CAPTURE, and this is the only moment it can happen (#217). The agreed content
    // exists on disk between the sync that agreed it and the first edit after; by the time a
    // conflict is planned the local file has moved and `SyncAction::Conflict`'s own arm has already
    // replaced the baseline digest with the current one. So a file becoming agreed is the capture
    // point, which is exactly what an `Upsert` of a `Synced` record is.
    //
    // READ BEFORE THE TRANSACTION OPENS, deliberately: a batched download commits up to
    // `download_batch_size` records at once, and holding a write transaction open across that many
    // file reads would put the database's busy window in the hands of the disk. Failing to read one
    // costs the say-less card and nothing else, so nothing here can fail the pass.
    let summaries = collect_agreed_summaries(index_mutations, local_root);

    let transaction = connection.transaction()?;
    for mutation in index_mutations.iter() {
        mutation.apply(&transaction)?;
    }
    // In the SAME transaction as the record it describes: a summary that outlived a rolled-back
    // upsert would answer for a version the index never agreed on.
    for (path, digest, summary) in &summaries {
        store_agreed_summary(&transaction, path, digest, summary, current_epoch_secs())?;
    }
    for (path, direction) in pending_approval_consumptions.iter() {
        delete_delete_approval(&transaction, path, *direction)?;
    }
    // History rides in the same transaction as the mutations it describes, so a shutdown between
    // the two can never leave an index row whose history row vanished (or the reverse).
    let pass_id = pass_log.write_pending(&transaction)?;
    transaction.commit()?;
    index_mutations.clear();
    pending_approval_consumptions.clear();
    // Fold the rollup only now: before the commit these events might still have been rolled back.
    pass_log.note_committed(pass_id, shared);
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

fn scan_options_from_config(config: &PairConfig) -> AppResult<ScanOptions> {
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
        &config.conflict_naming,
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

/// Start of the current **local** calendar day, in unix seconds.
///
/// `386 MB sent · 1.1 GB received today` is a calendar day, not a rolling 24 hours: at 00:30 the
/// two differ by everything that happened yesterday evening. Nothing in `std` knows where the
/// boundary is and the engine takes no date-time dependency (it shells `curl` rather than link an
/// HTTP crate), so this asks libc — which the crate already uses for `geteuid` and `kill`.
fn local_day_start(now_epoch_secs: u64) -> u64 {
    // `try_from`, not `as`: `time_t` is `i32` on 32-bit targets, where a silent truncation would
    // hand `localtime_r` a wrapped instant and put the day boundary somewhere else entirely.
    // Everything here degrades to "window unknown" rather than a confident wrong span.
    let Ok(time) = libc::time_t::try_from(now_epoch_secs) else {
        return now_epoch_secs;
    };
    let mut broken_down: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `time` and `broken_down` are live locals for the duration of the call, and
    // `localtime_r` is the thread-safe form (it writes only into the buffer it is handed).
    if unsafe { libc::localtime_r(&time, &mut broken_down) }.is_null() {
        // The boundary is unknown; an empty window reports nothing rather than a confident total
        // over the wrong span.
        return now_epoch_secs;
    }
    // `mktime`, NOT `now - (h*3600 + m*60 + s)`. On a DST transition day the offset at local
    // midnight differs from the offset now, so the subtraction lands an hour off: after a
    // spring-forward the day is 23 hours long, and 12:00 minus twelve hours is 01:00 local, not
    // midnight. `mktime` applies the zone's own rules for that calendar day; `tm_isdst = -1` tells
    // it to work out the offset rather than trusting the flag `localtime_r` left behind.
    broken_down.tm_hour = 0;
    broken_down.tm_min = 0;
    broken_down.tm_sec = 0;
    broken_down.tm_isdst = -1;
    // SAFETY: `broken_down` is a live local; `mktime` reads and normalizes it in place.
    let midnight = unsafe { libc::mktime(&mut broken_down) };
    // `-1` is `mktime`'s error return. A midnight later than `now` means the zone has no 00:00 on
    // this date and normalization moved forward past it — rare, and still not a reason to publish
    // a window that has not started.
    if midnight < 0 || midnight as u64 > now_epoch_secs {
        return now_epoch_secs;
    }
    midnight as u64
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
    /// Changes were planned, executed, and committed. Carries the pass's own outcome, because a
    /// committed pass is not necessarily a clean one (#136).
    Committed(PassOutcome),
    /// Nothing to do (no remote or local changes); the cursor was advanced without side effects.
    Idle,
    /// The delta could not be turned into a complete map; the caller must full-tree snapshot. The
    /// string is a human-readable reason for logging.
    Fallback(String),
}

/// How a reconcile pass that ran to the end came out. `Err` from the same call is the fourth
/// possibility and a different thing entirely: the pass itself could not proceed (a scan, a remote
/// listing, a commit).
///
/// [`PassOutcome::Partial`] is a **state of its own** (#136), not a flavour of either other one:
/// the plan ran to completion, some items' side effects failed. It is enumerated at every branch
/// on pass outcome rather than falling through one — a trailing arm silently absorbing an
/// undrawn state is how a failed pass came to render as "everything is up to date" (#246).
/// `#[must_use]` so no caller can drop the distinction by ignoring the value.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOutcome {
    /// Every planned action landed (or was deliberately skipped: withheld, unsupported, vanished).
    Clean,
    /// `failed` actions failed; the rest of the plan still executed and committed.
    Partial { failed: usize },
}

/// How many failed items a status reply carries (the count is untruncated). Enough to see the
/// shape of a failure without unbounded replies.
pub const FAILED_ITEMS_REPORTED: usize = 20;

/// Consecutive failures that abandon a pass in which **nothing** has succeeded yet. Not a
/// quarantine (#136 D1): it is the difference between "this item is poisoned" and "this daemon
/// cannot talk to Proton right now" — an expired session or a missing CLI otherwise costs one
/// doomed round trip per planned action, every pass. Gated on zero successes so it can never
/// starve anything: a pass that has accomplished nothing loses nothing by stopping.
const CONSECUTIVE_FAILURE_LIMIT: usize = 20;

/// A pass's history rows: the in-flight event buffer plus the rollup folded from every batch that
/// actually committed (#213/#229/#238/#230/#190/#191).
///
/// The buffer mirrors `index_mutations` exactly — an action that fails truncates it back, a
/// checkpoint drains it — so a history row can never describe a side effect that did not land, and
/// it commits in the same transaction as the index mutation it accompanies (ADR 0003). The rollup
/// is folded only *after* a transaction commits, never from the live buffer: a pass whose last
/// action failed would otherwise count events the rollback discarded.
struct PassLog {
    started_epoch_secs: u64,
    started: Instant,
    /// The strategy this pass ran. Starts at [`PassKind::Incremental`] and is corrected by whichever
    /// branch is taken; a pass that dies before choosing one keeps the default, which asserts only
    /// that no full-tree walk happened — true, since none did.
    kind: PassKind,
    /// The `sync_passes` row, opened by the first checkpoint that committed an event. `None` while
    /// the pass has landed nothing, which is what keeps an idle pass out of the log entirely.
    id: Option<i64>,
    /// Events awaiting the next commit.
    pending: Vec<FileEvent>,
    /// Side effects that landed **and committed** — the pass row's `changed`. Distinct from the
    /// live `planned_changes` below, which is what the plan asked for.
    changed: usize,
    bytes_uploaded: u64,
    bytes_downloaded: u64,
    /// Transfers that landed and committed, per direction — the live `44 sent · 115 received`
    /// (#243). Folded in [`Self::note_committed`] beside the byte sums, off the same match arms, so
    /// a count and a byte sum can never describe different sets of side effects.
    files_uploaded: u64,
    files_downloaded: u64,
    /// Side-effecting actions the plan holds, for the live pass block (#213). `None` until the
    /// plan exists.
    planned_changes: Option<u64>,
}

impl PassLog {
    fn new(now_epoch_secs: u64) -> Self {
        Self {
            started_epoch_secs: now_epoch_secs,
            started: Instant::now(),
            kind: PassKind::Incremental,
            id: None,
            pending: Vec::new(),
            changed: 0,
            bytes_uploaded: 0,
            bytes_downloaded: 0,
            files_uploaded: 0,
            files_downloaded: 0,
            planned_changes: None,
        }
    }

    /// Buffers a side effect that just landed at `path`. `bytes` is set only when bytes actually
    /// crossed the network (see [`SyncAction::transfer_direction`]).
    fn note(&mut self, action: SyncAction, path: &Path, bytes: Option<u64>) {
        self.pending.push(FileEvent {
            path: path.to_path_buf(),
            source_path: None,
            action,
            bytes,
            epoch_secs: current_epoch_secs(),
            pass_id: self.id.unwrap_or_default(),
        });
    }

    /// Buffers a move. Recorded at its **destination**, so a later per-path lookup on the file's
    /// current path finds the move that brought it there (#190) rather than missing its own event.
    fn note_move(&mut self, action: SyncAction, from: &Path, to: &Path) {
        self.pending.push(FileEvent {
            path: to.to_path_buf(),
            source_path: Some(from.to_path_buf()),
            action,
            bytes: None,
            epoch_secs: current_epoch_secs(),
            pass_id: self.id.unwrap_or_default(),
        });
    }

    /// Discards everything buffered since `mark` — the failed action's rollback point.
    fn truncate(&mut self, mark: usize) {
        self.pending.truncate(mark);
    }

    /// Writes the buffered events (opening the pass row on first use) **inside the caller's
    /// transaction**. Returns the row id for [`Self::note_committed`], which the caller calls only
    /// once that transaction has committed.
    fn write_pending(&self, connection: &Connection) -> AppResult<Option<i64>> {
        if self.pending.is_empty() {
            return Ok(self.id);
        }
        let id = match self.id {
            Some(id) => id,
            None => begin_pass(connection, self.started_epoch_secs, self.kind)?,
        };
        insert_file_events(connection, id, &self.pending)?;
        Ok(Some(id))
    }

    /// Folds the just-committed batch into the rollup and adopts the row id, then republishes the
    /// live per-direction progress (#243/#98).
    ///
    /// The publish is *inside* the fold rather than beside it at each caller, so the counters a
    /// status reply shows are the rollup itself and cannot be a caller that forgot to push them.
    fn note_committed(&mut self, id: Option<i64>, shared: &ControlShared) {
        self.id = id;
        for event in self.pending.drain(..) {
            self.changed += 1;
            // The per-direction FILE counts fold off exactly these arms, not off the direction
            // alone: an action that crossed the network with no amount to report is in neither
            // sum, and counting it here would publish `115 received` beside 0 received bytes. The
            // one such case today is a conflict sidecar written from the local copy — direction
            // `Down`, nothing fetched.
            match (event.action.transfer_direction(), event.bytes) {
                (Some(TransferDirection::Up), Some(bytes)) => {
                    self.bytes_uploaded = self.bytes_uploaded.saturating_add(bytes);
                    self.files_uploaded = self.files_uploaded.saturating_add(1);
                }
                (Some(TransferDirection::Down), Some(bytes)) => {
                    self.bytes_downloaded = self.bytes_downloaded.saturating_add(bytes);
                    self.files_downloaded = self.files_downloaded.saturating_add(1);
                }
                // An action that moves no bytes, or one that moved an unknown number.
                (Some(_), None) | (None, _) => {}
            }
        }
        shared.update_pass_progress(|pass| {
            pass.uploaded_files = self.files_uploaded;
            pass.downloaded_files = self.files_downloaded;
            pass.uploaded_bytes = self.bytes_uploaded;
            pass.downloaded_bytes = self.bytes_downloaded;
        });
    }

    /// Whether this pass earns a durable row. An **idle pass writes nothing**: with events-driven
    /// detection on by default the daemon polls every 30s, so recording every pass identically
    /// would evict each interesting row within minutes and leave a log of ~2900 "nothing happened"
    /// entries a day. Nothing is lost — "the daemon is alive and passing" is already answered by
    /// `reconcile_seq` and `last_sync_epoch_secs` on the live status reply.
    ///
    /// A full sweep is recorded even when it changed nothing, because `Full sweep now` asks
    /// exactly that: when did the last one run, and was anything out of step (#238).
    fn is_notable(&self, outcome: PassOutcomeKind) -> bool {
        self.changed > 0 || self.kind == PassKind::FullSweep || outcome != PassOutcomeKind::Clean
    }
}

/// A pass's item failures (#136), plus what the next pass must be told to look at again.
#[derive(Default)]
struct PassFailures {
    /// Bounded to [`FAILED_ITEMS_REPORTED`], in plan order.
    items: Vec<FailedItem>,
    /// Untruncated total.
    count: usize,
    /// Failures since the last success, for [`CONSECUTIVE_FAILURE_LIMIT`].
    consecutive: usize,
    /// Whether any action succeeded this pass.
    any_success: bool,
    /// Whether any failure this pass was classified by `proton.rs` as an auth failure. Recorded
    /// here because `record` is the last place the *typed* error exists — a `FailedItem` keeps only
    /// the message, and re-matching that message downstream is the second-classifier bug shape
    /// (#103). Read only by [`Self::abandoned`], so a systemic sign-out is reported as one.
    auth_failure: bool,
    /// Paths to re-queue into `pending_changes` once the pass clears it.
    rescan_next_pass: BTreeSet<PathBuf>,
}

impl PassFailures {
    fn record(
        &mut self,
        action: &PlannedAction,
        error: &(dyn std::error::Error + Send + Sync + 'static),
    ) {
        // Named at `warn` only while the reported list has room, then dropped to `debug`: a pass
        // that fails thousands of actions must not bury its own summary line (and everything else
        // in the journal) under thousands of copies of it. The count is always exact — the
        // post-loop line reports the total, and `failed_item_count` rides on every status reply.
        if self.items.len() < FAILED_ITEMS_REPORTED {
            warn!(
                path = %action.path.display(),
                action = ?action.action,
                %error,
                "sync action failed; continuing with the rest of the plan"
            );
            self.items.push(FailedItem {
                path: action.path.clone(),
                action: action.action,
                error: truncate_error(&error.to_string()),
            });
        } else {
            debug!(
                path = %action.path.display(),
                action = ?action.action,
                %error,
                "sync action failed (past the reported limit); continuing with the rest of the plan"
            );
        }
        self.count += 1;
        self.consecutive += 1;
        self.auth_failure |= is_auth_failure_error(error);
        self.rescan_next_pass.insert(action.path.clone());
    }

    fn note_success(&mut self) {
        self.any_success = true;
        self.consecutive = 0;
    }

    /// The error that abandons a systemically failing pass — **one** constructor for both trip
    /// sites, which is also the only way its auth classification can be in one place. `last` is the
    /// most recent failure's message where the caller has it (the single-action site); the batched
    /// site does not, since one chunk covers many files.
    ///
    /// Typed [`AuthFailure`] when any failure this pass was one: the breaker's whole premise is
    /// "this is not a per-item problem", and an expired session is the named example in its own
    /// doc comment — so the pass-level reader gets the same verdict a single failed listing gives.
    fn abandoned(&self, last: Option<&str>) -> Box<dyn std::error::Error + Send + Sync> {
        let last = last
            .map(|last| format!(" (last: {last})"))
            .unwrap_or_default();
        let details = format!(
            "reconciliation abandoned: {} actions in a row failed and nothing in this pass \
             succeeded{last}; the failure looks systemic rather than per-item, so the rest of the \
             plan is not attempted",
            self.consecutive
        );
        if self.auth_failure {
            Box::new(AuthFailure { details })
        } else {
            boxed_error(details)
        }
    }

    fn breaker_tripped(&self) -> bool {
        !self.any_success && self.consecutive >= CONSECUTIVE_FAILURE_LIMIT
    }

    fn outcome(&self) -> PassOutcome {
        match self.count {
            0 => PassOutcome::Clean,
            failed => PassOutcome::Partial { failed },
        }
    }
}

/// Bounds an error string to [`FAILED_ITEM_ERROR_LIMIT`] **bytes, inclusive of the ellipsis**, cut
/// on a char boundary — so the documented cap is the real one and no consumer has to budget for an
/// overshoot.
fn truncate_error(error: &str) -> String {
    truncate_for_display(error, FAILED_ITEM_ERROR_LIMIT)
}

/// Bounds a **client selector** to [`SELECTOR_DISPLAY_LIMIT`] before it is rendered back into a
/// reply message (#313).
///
/// **Display only, and never on the matching path.** `approve`/`deny`/`keep` match a selector in its
/// wire form and refuse one that names more than one pending deletion (#61); truncating before that
/// comparison would make two different paths compare equal, which is the opposite of what those
/// rules are for. So this is called on the way *out*, on the value being interpolated into a
/// sentence, and on nothing else.
fn truncate_selector(selector: &str) -> String {
    truncate_for_display(selector, SELECTOR_DISPLAY_LIMIT)
}

/// The one body behind every bounded string this engine puts in a reply: cut to `limit` **bytes
/// inclusive of the ellipsis**, on a char boundary, so the documented cap is the real one.
///
/// One body rather than a truncation per call site — three had accumulated by #313, and a fourth
/// would have been a fourth chance to get the inclusive-of-the-ellipsis arithmetic wrong.
///
/// **The cap holds for every `limit`, including one too small to hold the ellipsis.** Both named
/// limits are far above it, but this is the shared primitive now, and a caller has to be able to
/// trust the sentence above rather than a second one qualifying it — so when the marker will not
/// fit it is the *marker* that gives way, not the bound. Such a result cannot say it was cut, which
/// is the honest answer for a budget that leaves no room to say anything (Copilot review).
fn truncate_for_display(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let (budget, marker) = match limit.checked_sub(ELLIPSIS.len()) {
        Some(budget) => (budget, ELLIPSIS),
        None => (limit, ""),
    };
    let end = (0..=budget)
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0);
    format!("{}{marker}", &text[..end])
}

const ELLIPSIS: &str = "…";

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

/// Why [`PairPass::volume_id_for_scope`] could not name the event volume. Callers that only log
/// take [`Self::into_reason`]; the bootstrap cursor is the one caller the distinction matters to.
enum VolumeScopeError {
    /// Nothing names a volume: no baseline composed `proton_id` and no sole stored cursor row.
    Unnameable(String),
    /// The stored cursor could not be read, so a volume may exist and be invisible right now.
    Unreadable(String),
}

impl VolumeScopeError {
    fn into_reason(self) -> String {
        match self {
            Self::Unnameable(reason) | Self::Unreadable(reason) => reason,
        }
    }
}

/// What a bootstrap learned about the event cursor *before* it started walking (#294). The two
/// `None`-shaped outcomes are deliberately distinct: only one of them may be repaired by a
/// post-walk read.
enum PreSnapshotCursor {
    /// Read before the walk: safe to persist — every event after it still replays next pass.
    Captured(CursorUpdate),
    /// No volume nameable yet (no baseline composed `proton_id`, no stored cursor), i.e. the
    /// first-ever bootstrap. Nothing prior exists to lose, and a post-walk read is the only way
    /// the event path can ever engage.
    VolumeUnknown,
    /// No cursor may be anchored from this pass: the feature is off, there is no event source, or
    /// a lookup failed (the volume could not be read, or `latest_cursor` did) while prior state
    /// may exist. That last case is the dangerous one — a post-walk value would claim mid-walk
    /// changes were applied, and could overwrite a stored cursor that is merely unreadable now.
    Unavailable,
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

/// What looking at the user-global single-instance lock could tell about a running daemon (#317).
///
/// **Three states, and the third is not a synonym for either other one** — the rule
/// [`crate::ipc::AuthState`] already follows. A probe that could not answer must not be read as
/// "nothing is running": the whole point of the caller is to warn, and a fall-through arm meaning
/// "fine" is the #246 shape.
#[derive(Debug)]
pub enum GlobalLockProbe {
    /// Something holds the lock, so a `proton-syncd` is running for this user.
    Held,
    /// Nothing holds it. Includes "the lockfile does not exist at all", which is what a user who
    /// has never started a daemon has — and which the probe must not create.
    Free,
    /// The lock could not be looked at (its path could not be resolved, it could not be opened, the
    /// lock call failed for some reason other than contention). Evidence of **neither** state.
    Unknown(String),
}

/// Is a daemon holding `path`?
///
/// The caller passes [`DaemonConfig::global_lock_path`] — the field a daemon's own
/// `LockGuard::acquire` takes — rather than re-resolving `paths::default_global_lock_path()` here.
/// A second resolution is a second thing to keep in step with the first, and this one would answer
/// about a different file the moment the config layer stopped taking the default.
///
/// **`flock` has no query operation, so "check without acquiring" is not literally satisfiable.**
/// What this does instead is the closest thing that has the same effect: take a **shared** lock
/// non-blockingly and drop it in the next statement. Three properties make that the right reading
/// of #317's requirement (never hold the lock, never refuse a dry run because a daemon is up):
///
/// * **It never waits and never holds.** A held lock answers immediately from `EWOULDBLOCK`; a free
///   one is released before this function returns. Acquiring the *exclusive* lock for the duration
///   of the preview — the obvious alternative — would block a daemon from starting for as long as
///   the walk runs, which is worse than the hazard being warned about.
/// * **Shared, not exclusive**, so two dry runs probing at once do not refuse each other. It is the
///   weakest lock that still conflicts with the daemon's exclusive one.
/// * **It never creates the file.** A dry run must leave no state behind, and a missing lockfile is
///   itself an answer (nothing has ever locked it). `LockGuard::acquire` creates; this does not.
///
/// The residual, stated rather than hidden: a daemon whose own `LockGuard::acquire` lands inside the
/// few microseconds this holds the shared lock sees it held and refuses to start, with its usual
/// message, and starting again works. That window did not exist before, and it is the price of
/// there being no `flock` query.
pub fn probe_daemon_lock(path: &Path) -> GlobalLockProbe {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return GlobalLockProbe::Free,
        Err(error) => {
            return GlobalLockProbe::Unknown(format!(
                "cannot read the lockfile {}: {error}",
                path.display()
            ));
        }
    };
    // Called through `fs2::FileExt` by name, not as `file.try_lock_shared()`: std grew an
    // *inherent* `File::try_lock_shared` whose error is a `TryLockError`, and an inherent method
    // wins method resolution — so the unqualified spelling would silently pick a different API from
    // the `try_lock_exclusive` two functions up, and this file would hold two lock vocabularies.
    match FileExt::try_lock_shared(&file) {
        Ok(()) => {
            // Dropping `file` would release it too; unlocking here says so at the point it matters.
            let _ = file.unlock();
            GlobalLockProbe::Free
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => GlobalLockProbe::Held,
        Err(error) => GlobalLockProbe::Unknown(format!(
            "cannot test the lockfile {}: {error}",
            path.display()
        )),
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
    use crate::proton::{NodeNotFound, RemoteDirectory, RemoteFile};
    use crate::sync::UnsyncableReason;
    use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};
    use sha1::{Digest, Sha1};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    /// The `list`-verb context a control connection is served with, built from a test daemon
    /// exactly as `run` builds it — same client instance, hence the same `CliGate`.
    fn browse_context<C: ProtonClient>(daemon: &Daemon<C>) -> BrowseContext<C> {
        BrowseContext {
            proton: Arc::clone(&daemon.proton),
            remote_root: daemon.pair().config.remote_root.clone(),
            gate_wait: daemon.browse_gate_wait,
        }
    }

    // ---------------------------------------------------------------------------------------
    // #99 (the read-only `list` verb) and #103 (daemon-side auth classification).
    // ---------------------------------------------------------------------------------------

    /// A daemon whose remote holds `notes.txt`, `all/` and `all/inside.txt`, with `failure`
    /// governing how a listing fails.
    fn browsing_daemon(
        directory: &Path,
        local_root: &Path,
        failure: ListingFailure,
    ) -> Daemon<RecordingProtonClient> {
        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("notes.txt"),
            RemoteEntity::File(RemoteFile {
                path: PathBuf::from("/Drive/RemoteFolder/notes.txt"),
                id: "file-notes".to_owned(),
                name: "notes.txt".to_owned(),
                sha1_hash: Some("abc".to_owned()),
                downloadable: true,
            }),
        );
        remote_entities.insert(
            PathBuf::from("design.gdoc"),
            RemoteEntity::File(RemoteFile {
                path: PathBuf::from("/Drive/RemoteFolder/design.gdoc"),
                id: "file-doc".to_owned(),
                name: "design.gdoc".to_owned(),
                sha1_hash: None,
                downloadable: false,
            }),
        );
        // A folder literally named `all`: the reserved-word trap the approve/deny selector has,
        // and which a read-only verb must not inherit.
        remote_entities.insert(
            PathBuf::from("all"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("/Drive/RemoteFolder/all"),
                id: Some("dir-all".to_owned()),
                name: "all".to_owned(),
            }),
        );
        remote_entities.insert(
            PathBuf::from("all/inside.txt"),
            RemoteEntity::File(RemoteFile {
                path: PathBuf::from("/Drive/RemoteFolder/all/inside.txt"),
                id: "file-inside".to_owned(),
                name: "inside.txt".to_owned(),
                sha1_hash: Some("def".to_owned()),
                downloadable: true,
            }),
        );
        let (mut client, _operations) = RecordingProtonClient::new(HashMap::new());
        client.remote_entities = remote_entities;
        client.listing_failure = failure;
        Daemon::with_client(test_config(directory, local_root), client).expect("daemon")
    }

    fn browse(
        daemon: &Daemon<RecordingProtonClient>,
        selector: Option<&str>,
        limit: Option<usize>,
    ) -> ListingOutcome {
        let context = browse_context(daemon);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(browse_remote_directory(
                &context,
                &daemon.shared,
                selector,
                limit,
            ))
    }

    fn listed_names(outcome: &ListingOutcome) -> Vec<String> {
        match outcome {
            ListingOutcome::Listed { entries, .. } => {
                entries.iter().map(|entry| entry.name.clone()).collect()
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn the_list_verb_answers_a_folders_contents_and_not_the_folder_itself() {
        // #99. The empty selector is the remote root — the legitimate `Some("")` case the plain
        // path guard exists for — and the CLI's own wrapper node must not come back as an entry,
        // or every folder would appear to contain itself.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::None);

        let root = browse(&daemon, None, None);
        // Directories first, then by name.
        assert_eq!(
            listed_names(&root),
            vec!["all", "design.gdoc", "notes.txt"],
            "the root's immediate children, and nothing from inside `all/`"
        );
        let ListingOutcome::Listed {
            path,
            total,
            truncated,
            entries,
        } = &root
        else {
            panic!("expected a listing");
        };
        assert_eq!(
            path,
            Path::new(""),
            "the root echoes back as the empty path"
        );
        assert_eq!((*total, *truncated), (3, false));
        // A Proton-native node is named as present-but-unfetchable rather than as an ordinary
        // file that simply never arrives.
        let doc = entries
            .iter()
            .find(|entry| entry.name == "design.gdoc")
            .expect("the doc");
        assert!(!doc.downloadable);
        assert_eq!(doc.sha1, None);

        // …and a subfolder lists its own contents, with itself dropped.
        let inside = browse(&daemon, Some("all"), None);
        assert_eq!(listed_names(&inside), vec!["inside.txt"]);
    }

    /// A `ProtonClient` whose single-directory listing hands back the WHOLE tree, however deep —
    /// what a CLI that nested a second level of `entries` would produce. The browse must still
    /// answer one folder's contents.
    #[derive(Debug)]
    struct OverdeepListingClient {
        entities: HashMap<PathBuf, RemoteEntity>,
    }

    impl ProtonClient for OverdeepListingClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(HashMap::new())
        }

        fn list_directory(
            &self,
            _remote_root: &Path,
            _relative_directory: &Path,
        ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            Ok(self.entities.clone())
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected ensure directory"))
        }

        fn upload(&self, _local: &Path, _root: &Path, _relative: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected upload"))
        }

        fn download(&self, _remote_path: &Path, _destination: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected download"))
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected delete"))
        }
    }

    /// A client that keys its remote by **absolute** path and answers a listing the way the real
    /// one does: the children of `root/relative`, keyed *relative to `root`*, plus the CLI's own
    /// wrapper node for the directory it was asked about.
    ///
    /// `RecordingProtonClient` cannot stand in here: it ignores `remote_root` entirely, so it
    /// answers an absolute selector and a relative one identically — which is precisely the
    /// distinction #323 introduces.
    /// The `(root, relative)` pairs a listing client was asked for.
    type ListingRequests = Arc<Mutex<Vec<(PathBuf, PathBuf)>>>;

    #[derive(Debug)]
    struct AbsoluteListingClient {
        entities: HashMap<PathBuf, RemoteEntity>,
        seen: ListingRequests,
    }

    impl AbsoluteListingClient {
        fn new(paths: &[(&str, bool)]) -> (Self, ListingRequests) {
            let entities = paths
                .iter()
                .map(|(path, is_dir)| {
                    let name = Path::new(path)
                        .file_name()
                        .expect("name")
                        .to_string_lossy()
                        .into_owned();
                    let entity = if *is_dir {
                        RemoteEntity::Directory(RemoteDirectory {
                            path: PathBuf::from(path),
                            id: Some(format!("dir-{name}")),
                            name,
                        })
                    } else {
                        RemoteEntity::File(RemoteFile {
                            path: PathBuf::from(path),
                            id: format!("file-{name}"),
                            name,
                            sha1_hash: Some("aa".to_owned()),
                            downloadable: true,
                        })
                    };
                    (PathBuf::from(path), entity)
                })
                .collect();
            let seen = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    entities,
                    seen: Arc::clone(&seen),
                },
                seen,
            )
        }
    }

    impl ProtonClient for AbsoluteListingClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(HashMap::new())
        }

        fn list_directory(
            &self,
            remote_root: &Path,
            relative_directory: &Path,
        ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            self.seen
                .lock()
                .expect("seen lock")
                .push((remote_root.to_path_buf(), relative_directory.to_path_buf()));
            let directory = if relative_directory.as_os_str().is_empty() {
                remote_root.to_path_buf()
            } else {
                remote_root.join(relative_directory)
            };
            let mut listing: HashMap<PathBuf, RemoteEntity> = self
                .entities
                .iter()
                .filter(|(absolute, _)| absolute.parent() == Some(directory.as_path()))
                .filter_map(|(absolute, entity)| {
                    Some((
                        absolute.strip_prefix(remote_root).ok()?.to_path_buf(),
                        entity.clone(),
                    ))
                })
                .collect();
            // The wrapper node the CLI includes for the directory being listed.
            if let Some(entity) = self.entities.get(&directory) {
                listing.insert(relative_directory.to_path_buf(), entity.clone());
            }
            Ok(listing)
        }

        fn ensure_directory(&self, _remote_root: &Path, _relative_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected ensure directory"))
        }

        fn upload(&self, _local: &Path, _root: &Path, _relative: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected upload"))
        }

        fn download(&self, _remote_path: &Path, _destination: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected download"))
        }

        fn delete(&self, _remote_path: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected delete"))
        }
    }

    fn absolute_browsing_daemon(
        directory: &Path,
        local_root: &Path,
    ) -> (Daemon<AbsoluteListingClient>, ListingRequests) {
        let (client, seen) = AbsoluteListingClient::new(&[
            ("/Drive/Candidate", true),
            ("/Drive/Candidate/a.txt", false),
            ("/Drive/Candidate/sub", true),
            ("/Drive/Candidate/sub/b.txt", false),
            // Inside the daemon's own configured root, which the absolute selector must not be
            // resolved against.
            ("/Drive/RemoteFolder/mine.txt", false),
        ]);
        let daemon =
            Daemon::with_client(test_config(directory, local_root), client).expect("daemon");
        (daemon, seen)
    }

    fn browse_absolute(
        daemon: &Daemon<AbsoluteListingClient>,
        selector: Option<&str>,
    ) -> ListingOutcome {
        let context = BrowseContext {
            proton: Arc::clone(&daemon.proton),
            remote_root: daemon.pair().config.remote_root.clone(),
            gate_wait: daemon.browse_gate_wait,
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(browse_remote_directory(
                &context,
                &daemon.shared,
                selector,
                None,
            ))
    }

    /// #323: the folder probe's question is about a path **outside every configured root**, asked
    /// before a pair exists, so the selector has to be absolute — and the reply has to answer in
    /// that same frame, or a walk cannot use an entry as its next selector.
    #[test]
    fn an_absolute_selector_lists_a_folder_outside_the_configured_root() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (daemon, seen) = absolute_browsing_daemon(directory.path(), &local_root);
        assert_eq!(
            daemon.pair().config.remote_root,
            PathBuf::from("/Drive/RemoteFolder"),
            "the candidate below is deliberately not under this"
        );

        let outcome = browse_absolute(&daemon, Some("/Drive/Candidate"));
        let ListingOutcome::Listed {
            path,
            entries,
            total,
            truncated,
        } = &outcome
        else {
            panic!("expected a listing, got {outcome:?}");
        };
        assert_eq!(
            path,
            Path::new("/Drive/Candidate"),
            "the echo answers in the frame the request used"
        );
        assert_eq!((*total, *truncated), (2, false));
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![
                PathBuf::from("/Drive/Candidate/sub"),
                PathBuf::from("/Drive/Candidate/a.txt"),
            ],
            "entries come back absolute — not relative to remote_root (which they are not under) \
             and not bare names (which would duplicate `name`)"
        );
        // The CLI was asked for the candidate as its own root, so nothing was resolved against
        // `remote_root`, and the folder's own wrapper node did not come back as an entry.
        assert_eq!(
            seen.lock().expect("seen lock").as_slice(),
            [(PathBuf::from("/Drive/Candidate"), PathBuf::new())]
        );

        // And an entry can be fed straight back, which is what makes a client-side walk possible.
        let deeper = browse_absolute(&daemon, Some("/Drive/Candidate/sub"));
        assert_eq!(
            listed_names(&deeper),
            vec!["b.txt"],
            "the walk's next step, got {deeper:?}"
        );
    }

    /// The new boundary, guarded where it enters (#323). An absolute selector is validated by
    /// `validate_absolute_remote_path`, so `..` is refused rather than resolved — and the refusal
    /// happens before any CLI child is spawned, not after one has listed the wrong folder.
    #[test]
    fn an_absolute_selector_that_escapes_is_refused_before_anything_is_listed() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (daemon, seen) = absolute_browsing_daemon(directory.path(), &local_root);

        for selector in [
            "/Drive/Candidate/..",
            "/Drive/../Drive/Candidate",
            "/Drive/./../x",
        ] {
            let outcome = browse_absolute(&daemon, Some(selector));
            let ListingOutcome::Failed { error } = &outcome else {
                panic!("{selector} must be refused, got {outcome:?}");
            };
            assert!(error.starts_with("unsafe remote path:"), "{error:?}");
        }
        assert!(
            seen.lock().expect("seen lock").is_empty(),
            "a refused selector must never reach the CLI"
        );
    }

    #[test]
    fn a_browse_answers_one_level_even_when_the_listing_hands_back_a_whole_subtree() {
        // "In this folder" is one level down, and the daemon says so rather than trusting the
        // listing to be non-recursive. A CLI that nested a second level would otherwise flatten a
        // subtree into what a browser draws as one folder's contents — and the folder's own
        // wrapper node would appear inside itself.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut entities = HashMap::new();
        for path in [
            "photos",
            "photos/beach.jpg",
            "photos/2019",
            "photos/2019/ski.jpg",
        ] {
            let name = Path::new(path)
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            entities.insert(
                PathBuf::from(path),
                if path.ends_with(".jpg") {
                    RemoteEntity::File(RemoteFile {
                        path: PathBuf::from("/Drive/RemoteFolder").join(path),
                        id: format!("id-{name}"),
                        name,
                        sha1_hash: Some("abc".to_owned()),
                        downloadable: true,
                    })
                } else {
                    RemoteEntity::Directory(RemoteDirectory {
                        path: PathBuf::from("/Drive/RemoteFolder").join(path),
                        id: Some(format!("id-{name}")),
                        name,
                    })
                },
            );
        }
        let daemon = Daemon::with_client(
            test_config(directory.path(), &local_root),
            OverdeepListingClient { entities },
        )
        .expect("daemon");

        let context = browse_context(&daemon);
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(browse_remote_directory(
                &context,
                &daemon.shared,
                Some("photos"),
                None,
            ));

        let ListingOutcome::Listed { entries, total, .. } = outcome else {
            panic!("expected a listing");
        };
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["2019", "beach.jpg"],
            "neither the folder itself nor anything below `2019/`"
        );
        assert_eq!(
            total, 2,
            "the total counts what the folder holds, not the subtree"
        );
    }

    #[test]
    fn a_folder_named_all_is_listed_rather_than_treated_as_a_sentinel() {
        // The reserved-word trap: `"all"` means "every pending deletion" to approve/deny/keep, and
        // a read-only verb must not grow a second meaning for it. Browsing `all` lists the folder.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::None);

        assert_eq!(
            listed_names(&browse(&daemon, Some("all"), None)),
            vec!["inside.txt"]
        );
        assert_eq!(
            listed_names(&browse(&daemon, Some("All"), None)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_list_selector_that_escapes_the_root_is_refused_before_it_is_joined() {
        // The path-safety boundary. A listing is a read, so the EMPTY path is legitimate here —
        // but `..` never is, and the refusal must happen before anything is joined onto the root.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::None);

        for selector in ["../etc", "all/../..", "sub/../../escape"] {
            match browse(&daemon, Some(selector), None) {
                ListingOutcome::Failed { error } => {
                    assert!(error.contains("unsafe remote path"), "{selector}: {error}");
                }
                other => panic!("{selector} must be refused, got {other:?}"),
            }
        }
        assert_eq!(
            daemon.shared.auth_state(),
            AuthState::Unknown,
            "a refused selector never reached Proton, so it proves nothing about the session"
        );

        // `/etc` WAS IN THAT LIST AND IS DELIBERATELY NOT ANY MORE (#323). A leading `/` now
        // selects the absolute frame, where the selector names a location on **Proton Drive** —
        // `proton-drive filesystem list /etc` asks about a remote node, and nothing on this machine
        // is reachable through a verb that only ever hands paths to that CLI. Its own escapes go
        // through `validate_absolute_remote_path` instead, guarded by
        // `an_absolute_selector_that_escapes_is_refused_before_anything_is_listed`.
        assert!(
            matches!(
                browse(&daemon, Some("/etc"), None),
                ListingOutcome::Listed { .. }
            ),
            "an absolute selector is a remote location, not an escape"
        );
    }

    #[test]
    fn a_listing_is_capped_by_the_daemon_and_says_how_many_it_held_back() {
        // The reply is one JSON line read into memory whole, so the bound is the daemon's. A
        // truncated listing must say so rather than read as a short folder.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::None);

        let ListingOutcome::Listed {
            entries,
            total,
            truncated,
            ..
        } = browse(&daemon, None, Some(1))
        else {
            panic!("expected a listing");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(total, 3, "the total is untruncated");
        assert!(truncated);

        // A limit past the hard cap clamps to it rather than being honoured.
        let ListingOutcome::Listed { entries, .. } = browse(&daemon, None, Some(usize::MAX)) else {
            panic!("expected a listing");
        };
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn a_busy_cli_answers_busy_and_moves_no_sign_in_verdict() {
        // The concurrency decision's user-visible half (#99): a browse that could not get past the
        // CLI gate reports a retryable state — NOT a failure, and not an empty folder. Nothing ran,
        // so it is evidence of nothing about the session either.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::Busy);
        daemon.shared.record_auth_state(AuthState::SignedIn);

        assert_eq!(browse(&daemon, None, None), ListingOutcome::Busy);
        assert_eq!(
            daemon.shared.auth_state(),
            AuthState::SignedIn,
            "a busy gate says nothing about the session"
        );
    }

    #[test]
    fn a_listing_refused_for_the_session_reports_signed_out_on_the_very_same_reply() {
        // #99 and #103 designed together: a read-only verb is where an auth failure surfaces
        // first, and the client that asked is the one that needs to know. The verdict rides on the
        // same reply, so a UI does not wait for the next pass to learn it is signed out.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::Auth);

        match browse(&daemon, None, None) {
            ListingOutcome::Failed { error } => assert!(error.contains("401")),
            other => panic!("expected a failed listing, got {other:?}"),
        }
        assert_eq!(daemon.shared.auth_state(), AuthState::SignedOut);
        assert_eq!(
            daemon.shared.response("x").auth,
            AuthState::SignedOut,
            "and it is on the wire, not just in memory"
        );
    }

    #[test]
    fn an_unclassified_listing_failure_leaves_the_sign_in_verdict_exactly_where_it_was() {
        // THE fall-through guard (#246's shape, one layer over). A timeout is not a sign-out and
        // is not a sign-in: an "unknown error" arm that resolved either way would turn every
        // unrecognised failure into a verdict the daemon has no evidence for.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::Other);

        assert_eq!(daemon.shared.auth_state(), AuthState::Unknown);
        assert!(matches!(
            browse(&daemon, None, None),
            ListingOutcome::Failed { .. }
        ));
        assert_eq!(
            daemon.shared.auth_state(),
            AuthState::Unknown,
            "an unclassified failure must not read as signed in OR as signed out"
        );

        // The same from the other starting point: a known-bad session is not repaired by a
        // timeout, and a known-good one is not condemned by one.
        daemon.shared.record_auth_state(AuthState::SignedOut);
        assert!(matches!(
            browse(&daemon, None, None),
            ListingOutcome::Failed { .. }
        ));
        assert_eq!(daemon.shared.auth_state(), AuthState::SignedOut);
    }

    #[test]
    fn a_failed_listings_error_is_bounded_like_every_other_error_on_the_wire() {
        // A `proton-drive` failure can carry kilobytes of stderr, and the reply is one JSON line
        // read into memory whole — the same reason `FailedItem::error` is capped (#136).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::Verbose);

        let ListingOutcome::Failed { error } = browse(&daemon, None, None) else {
            panic!("expected a failed listing");
        };

        assert!(
            error.len() <= FAILED_ITEM_ERROR_LIMIT,
            "the wire error must be bounded, got {} bytes",
            error.len()
        );
        assert!(
            error.ends_with('…'),
            "and it must say it was cut rather than look complete: {error:?}"
        );

        // The *refused selector* arm is bound by the same rule, and it is the one where the bytes
        // are the client's own: a control request may carry up to `ipc::MAX_REQUEST_BYTES`, so an
        // unbounded echo here would put 64 KiB of somebody else's string on the wire.
        let huge = format!("../{}", "a".repeat(64 * 1024));
        let ListingOutcome::Failed { error } = browse(&daemon, Some(&huge), None) else {
            panic!("expected a refused selector");
        };
        assert!(
            error.len() <= FAILED_ITEM_ERROR_LIMIT,
            "a refused selector must not echo itself unbounded, got {} bytes",
            error.len()
        );
        assert!(error.starts_with("unsafe remote path:"), "{error:?}");
    }

    #[test]
    fn a_successful_listing_is_the_evidence_that_clears_a_stale_sign_out() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let daemon = browsing_daemon(directory.path(), &local_root, ListingFailure::None);
        daemon.shared.record_auth_state(AuthState::SignedOut);

        assert!(matches!(
            browse(&daemon, None, None),
            ListingOutcome::Listed { .. }
        ));
        assert_eq!(daemon.shared.auth_state(), AuthState::SignedIn);
    }

    #[test]
    fn a_successful_pass_reports_the_session_as_signed_in() {
        // The daemon core's half of the same verdict: a pass that completed reached Proton.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        assert_eq!(daemon.shared.auth_state(), AuthState::Unknown);

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert_eq!(daemon.shared.auth_state(), AuthState::SignedIn);
    }

    #[test]
    fn a_pass_that_failed_on_the_session_reports_it_once_through_the_existing_latch() {
        // #103's reporting rule: an expired session is another cause in the ONE
        // "why is this daemon degraded" family, said once and re-said only when the cause changes —
        // not a second channel that can disagree with it. It latches on the PROCESS (#102 phase 2),
        // because a session is per-user, while a missing volume is a fact about one tree.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.pass().note_auth_evidence(AuthState::SignedOut);
        assert_eq!(daemon.shared.auth_state(), AuthState::SignedOut);
        assert_eq!(
            daemon
                .session_declined
                .as_ref()
                .map(|declined| declined.reason.as_str()),
            Some(AUTH_DECLINE_REASON),
            "the sign-out is reported through the session latch, not beside it"
        );
        // The reason is a log line and a status string, so it must read as one sentence. The
        // source spells it across two lines with a `\` continuation, which strips the newline AND
        // the next line's indentation — but that is invisible at the call site and easy to
        // "fix" into a real run of spaces (Copilot review read it as one). Assert the rendered
        // form rather than trusting the escape.
        assert!(
            !AUTH_DECLINE_REASON.contains("  ") && !AUTH_DECLINE_REASON.contains('\n'),
            "the reason must render as one line with no whitespace run: {AUTH_DECLINE_REASON:?}"
        );

        // Recovery clears it, because it is OUR cause…
        daemon.pass().note_auth_evidence(AuthState::SignedIn);
        assert_eq!(daemon.session_declined, None);

        // …and only ours, in BOTH directions of the split. A working session says nothing about a
        // missing volume (the pair's latch) …
        daemon
            .pass()
            .note_event_scope_declined("no event volume: nothing carries a proton_id".to_owned());
        daemon.pass().note_auth_evidence(AuthState::SignedIn);
        assert_eq!(
            daemon.pair().event_scope_declined.as_deref(),
            Some("no event volume: nothing carries a proton_id")
        );

        // … nor about a locked keyring, which shares the *same* latch as the sign-out and is the
        // case a string comparison could not tell apart: the degraded-session line is formatted
        // with two intervals, so it has no canonical spelling to compare against. A sign-in must
        // leave it standing.
        daemon.pair_mut().config.events_driven = true;
        daemon.event_source = None;
        daemon.pass().note_degraded_session_if_needed();
        let degraded = daemon.session_declined.clone();
        assert!(
            degraded
                .as_ref()
                .is_some_and(|declined| declined.reason.contains("no usable proton-drive CLI")),
            "a degraded session must latch its own cause: {degraded:?}"
        );
        daemon.pass().note_auth_evidence(AuthState::SignedIn);
        assert_eq!(
            daemon.session_declined, degraded,
            "a sign-in must not erase the keyring's line and let it re-report as new next pass"
        );
    }

    #[test]
    fn a_systemically_auth_failing_pass_abandons_with_a_classified_error() {
        // The breaker's own doc names an expired session as its motivating case, so the error it
        // raises must carry that classification through to the pass-level reader — otherwise a
        // whole pass of 401s reads as an unexplained failure while a single failed listing does
        // not.
        let mut failures = PassFailures::default();
        let action = PlannedAction::new(
            Path::new("a.txt"),
            SyncAction::Upload,
            EntityKind::File,
            None,
        );
        let auth = AuthFailure {
            details: "proton-drive upload failed: 401 Unauthorized".to_owned(),
        };
        failures.record(&action, &auth);
        let abandoned = failures.abandoned(Some("401 Unauthorized"));
        assert!(is_auth_failure_error(abandoned.as_ref()));
        assert!(abandoned.to_string().contains("reconciliation abandoned"));

        // …and a pass that failed for other reasons is not retyped into a session verdict.
        let mut ordinary = PassFailures::default();
        ordinary.record(&action, &*boxed_error("ENOSPC"));
        assert!(!is_auth_failure_error(ordinary.abandoned(None).as_ref()));
    }

    /// Unwraps a pass and pins it CLEAN. Every test that is not specifically about item failures
    /// says this, so a partial pass (#136) can never be waved through as "it returned Ok" — the
    /// shape of bug that once let a failed pass render as success (#246).
    trait ExpectCleanPass {
        fn expect_clean(self, context: &str);
        /// The counterpart: the pass ran to the end with exactly `failed` item failures.
        fn expect_partial(self, failed: usize, context: &str);
    }

    impl ExpectCleanPass for AppResult<PassOutcome> {
        #[track_caller]
        fn expect_clean(self, context: &str) {
            match self.unwrap_or_else(|error| panic!("{context}: {error}")) {
                PassOutcome::Clean => {}
                PassOutcome::Partial { failed } => {
                    panic!("{context}: expected a clean pass, got {failed} failed item(s)")
                }
            }
        }

        #[track_caller]
        fn expect_partial(self, failed: usize, context: &str) {
            assert_eq!(
                self.unwrap_or_else(|error| panic!("{context}: {error}")),
                PassOutcome::Partial { failed },
                "{context}"
            );
        }
    }

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
        /// Absolute remote paths whose download fails with a typed [`NodeNotFound`] — models a
        /// node trashed between the listing and the transfer (#31). Honoured by both the
        /// single-file and the batched path.
        vanished_downloads: BTreeSet<PathBuf>,
        /// How `list_directory` fails, for the `list` verb's three outcomes. A field rather than a
        /// boxed error because the client is `Clone` — and because the point of the test is the
        /// *type* the daemon classifies on, which an enum names better than a message.
        listing_failure: ListingFailure,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    enum ListingFailure {
        #[default]
        None,
        /// The CLI gate refused an interactive listing: nothing ran.
        Busy,
        /// The CLI blamed the Proton session.
        Auth,
        /// Anything else — the arm that must move no auth verdict at all.
        Other,
        /// A failure whose message is far past the wire's error bound.
        Verbose,
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
                    vanished_downloads: BTreeSet::new(),
                    listing_failure: ListingFailure::None,
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
                    vanished_downloads: BTreeSet::new(),
                    listing_failure: ListingFailure::None,
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
                    vanished_downloads: BTreeSet::new(),
                    listing_failure: ListingFailure::None,
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
                    vanished_downloads: BTreeSet::new(),
                    listing_failure: ListingFailure::None,
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

        /// Models the real client closely enough for the `list` verb to be worth testing: the
        /// directory's immediate children **plus the directory itself**, because the CLI wraps a
        /// listing in its own node and the daemon's browse is what has to drop it again.
        fn list_directory(
            &self,
            _remote_root: &Path,
            relative_directory: &Path,
        ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
            match self.listing_failure {
                ListingFailure::None => {}
                ListingFailure::Busy => {
                    return Err(Box::new(crate::proton::CliBusy {
                        operation: "list".to_owned(),
                        waited: Duration::from_millis(1),
                    }));
                }
                ListingFailure::Auth => {
                    return Err(Box::new(AuthFailure {
                        details: "proton-drive list failed: 401 Unauthorized".to_owned(),
                    }));
                }
                ListingFailure::Other => {
                    return Err(boxed_error("proton-drive list timed out after 60s"));
                }
                ListingFailure::Verbose => {
                    return Err(boxed_error("x".repeat(FAILED_ITEM_ERROR_LIMIT * 4)));
                }
            }
            let mut listing: HashMap<PathBuf, RemoteEntity> = self
                .remote_entities
                .iter()
                .filter(|(path, _)| path.parent().unwrap_or(Path::new("")) == relative_directory)
                .map(|(path, entity)| (path.clone(), entity.clone()))
                .collect();
            if let Some(wrapper) = self.remote_entities.get(relative_directory) {
                listing.insert(relative_directory.to_path_buf(), wrapper.clone());
            }
            Ok(listing)
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
            // The real client hands `local_path` to the CLI, which cannot read a file that is not
            // there. Modelling that is what makes a stale pre-move source path observable (#12).
            if !local_path.exists() {
                return Err(boxed_error(format!(
                    "upload source is missing: {}",
                    local_path.display()
                )));
            }
            Ok(())
        }

        fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()> {
            if self.vanished_downloads.contains(remote_path) {
                self.operations.lock().expect("operations lock").push(
                    RecordedOperation::Download {
                        remote_path: remote_path.to_path_buf(),
                        destination: destination.to_path_buf(),
                    },
                );
                return Err(Box::new(NodeNotFound {
                    remote_path: remote_path.to_path_buf(),
                    details: format!(
                        "proton-drive download failed for {}: Node not found",
                        remote_path.display()
                    ),
                }));
            }
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
                    if self.vanished_downloads.contains(&request.remote_path) {
                        return Err(Box::new(NodeNotFound {
                            remote_path: request.remote_path.clone(),
                            details: format!(
                                "proton-drive download failed for {}: Node not found",
                                request.remote_path.display()
                            ),
                        })
                            as Box<dyn std::error::Error + Send + Sync>);
                    }
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
            full_scan_schedule: None,
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
            local_delete_mode: LocalDeleteMode::default(),
            warm_start: WarmStartConfig::default(),
            conflict_naming: ConflictNaming::default(),
            log_filter: "info".to_owned(),
        };

        let plan = preview_plan_with_client(&config, &FakeProtonClient { remote_files })
            .expect("preview plan")
            .plan;

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

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        assert_eq!(daemon.pair().last_error, None);
        assert_eq!(
            daemon
                .pair()
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
            &daemon.pair().connection,
            &base_record("stable.txt", Some("stale-remote-id"), hash.as_str()),
        )
        .expect("base index record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
                .pair()
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

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("local-only.txt"))
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

        daemon.reconcile_blocking().expect_clean("reconcile");

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
            &daemon.pair().connection,
            Path::new("local-sub-directory/subdirectory-file.txt"),
        )
        .expect("index lookup")
        .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Synced);
        let directory_record =
            get_record(&daemon.pair().connection, Path::new("local-sub-directory"))
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

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![RecordedOperation::EnsureDirectory {
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                relative_path: PathBuf::from("empty-local-dir"),
            }]
        );
        let record = get_record(&daemon.pair().connection, Path::new("empty-local-dir"))
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

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote directory bootstrap should not upload, download, or delete file content"
        );
        assert!(
            local_root.join("empty-remote-dir").is_dir(),
            "remote-only directory should be created locally"
        );
        let record = get_record(&daemon.pair().connection, Path::new("empty-remote-dir"))
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

        daemon.reconcile_blocking().expect_clean("reconcile");

        let conflict_path = local_root.join("same-name.proton-cloud");
        assert_eq!(
            fs::read_to_string(&conflict_path).expect("conflict sidecar"),
            "downloaded:/Drive/RemoteFolder/same-name"
        );
        assert!(
            local_root.join("same-name").is_dir(),
            "the local directory must win the clash and stay in place"
        );
        let directory_record = get_record(&daemon.pair().connection, Path::new("same-name"))
            .expect("directory index lookup")
            .expect("directory index record");
        assert_eq!(directory_record.entity_kind, EntityKind::Directory);
        assert_eq!(directory_record.sync_status, SyncStatus::Synced);
        assert_eq!(directory_record.proton_id, None);
        let sidecar_record = get_record(
            &daemon.pair().connection,
            Path::new("same-name.proton-cloud"),
        )
        .expect("sidecar index lookup")
        .expect("sidecar index record");
        assert_eq!(sidecar_record.entity_kind, EntityKind::File);
        assert_eq!(sidecar_record.sync_status, SyncStatus::Conflict);
        assert_eq!(sidecar_record.proton_id.as_deref(), Some("remote-file-id"));

        daemon.reconcile_blocking().expect_clean("second reconcile");

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
            &daemon.pair().connection,
            &base_record("old-name.txt", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote rename convergence should not upload, download, or delete"
        );
        assert!(!old_path.exists(), "old local path should be moved away");
        assert!(local_root.join("new-name.txt").is_file());
        assert!(
            get_record(&daemon.pair().connection, Path::new("old-name.txt"))
                .expect("old index lookup")
                .is_none(),
            "old path should be purged from the index"
        );
        let record = get_record(&daemon.pair().connection, Path::new("new-name.txt"))
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
            &daemon.pair().connection,
            &base_record("old-name.txt", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
            get_record(&daemon.pair().connection, Path::new("old-name.txt"))
                .expect("old index lookup")
                .is_none(),
            "old path should be purged from the index"
        );
        let record = get_record(&daemon.pair().connection, Path::new("new-name.txt"))
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
            &daemon.pair().connection,
            &base_record("claude_export.zip", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "auto-link must not upload, download, or delete content"
        );
        let record = get_record(&daemon.pair().connection, Path::new("shared.txt"))
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("remote-only.txt"))
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
            .expect_clean("reconcile skips the unsafe entry rather than erroring");

        assert!(
            !outside.join("secret.txt").exists(),
            "remote content must not be written outside the sync root through the symlink"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("sub/secret.txt"))
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

        daemon.reconcile_blocking().expect_partial(
            1,
            "an empty-path download must be refused, not executed — as that item's failure",
        );

        assert!(
            daemon.pair().last_failed_items[0]
                .error
                .contains("unsafe remote path"),
            "unexpected failure: {:?}",
            daemon.pair().last_failed_items
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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

        daemon.reconcile_blocking().expect_clean("reconcile");

        let destination = local_root.join("nested/remote-only.txt");
        assert_eq!(
            fs::read_to_string(&destination).expect("downloaded file"),
            "downloaded:/Drive/RemoteFolder/nested/remote-only.txt"
        );
        let record = get_record(
            &daemon.pair().connection,
            Path::new("nested/remote-only.txt"),
        )
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
            &daemon.pair().connection,
            &base_record(
                "removed-remotely.txt",
                Some("remote-id"),
                base_hash.as_str(),
            ),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            !local_path.exists(),
            "remote deletion should remove the unchanged local file"
        );
        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "remote deletion reconciliation must not call Proton mutations"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("removed-remotely.txt"))
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
            &daemon.pair().connection,
            &base_record("removed.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .contains(&RecordedOperation::Delete {
                    remote_path: PathBuf::from("/Drive/RemoteFolder/removed.txt"),
                })
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("removed.txt"))
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            local_path.exists(),
            "the guard must not delete the local file without approval"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_some(),
            "the base record must survive so the delete re-plans next pass"
        );
        assert!(operations.lock().expect("ops").is_empty());
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        assert_eq!(
            daemon.pair().pending_deletions[0].direction,
            DeleteDirection::Local
        );
        assert_eq!(
            daemon.pair().pending_deletions[0].path,
            PathBuf::from("keep.txt")
        );
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
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
                disposal: LocalDisposal::Permanent,
            },
            PendingDeletion {
                path: PathBuf::from("other.txt"),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: "fp-other".to_owned(),
                detected_epoch_secs: 1,
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
                disposal: LocalDisposal::Permanent,
            },
        ];

        let message = apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some("All"),
            true,
            true,
            None,
        )
        .expect("literal-path approve");
        assert!(
            message.contains("approved 1"),
            "a literal-path selector must approve only the file named \"All\": {message}"
        );
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .len(),
            1,
        );

        let message = apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some("All"),
            false,
            true,
            None,
        )
        .expect("legacy approve");
        assert!(
            message.contains("approved 2"),
            "without the literal flag the same selector keeps the every-item meaning: {message}"
        );
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
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
            first_seen_epoch_secs: 1,
            subtree_files: None,
            subtree_bytes: None,
            disposal: LocalDisposal::Permanent,
        }];
        let selector = crate::ipc::wire_path(&real_path);
        assert_ne!(
            PathBuf::from(&*selector),
            real_path,
            "the wire form is lossy"
        );

        let message = apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some(&selector),
            true,
            true,
            None,
        )
        .expect("approve by wire form");

        assert!(
            message.contains("approved 1"),
            "the lossy selector a client can send must match: {message}"
        );
        let approvals =
            crate::index::load_delete_approvals(&daemon.pair().connection).expect("approvals");
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
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
                disposal: LocalDisposal::Permanent,
            })
            .collect();
        let selector = crate::ipc::wire_path(&pending[0].path);
        assert_eq!(selector, crate::ipc::wire_path(&pending[1].path));

        let message = apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some(&selector),
            true,
            true,
            None,
        )
        .expect("ambiguous approve");

        assert!(
            message.contains("cannot be told apart"),
            "an ambiguous selector must explain itself: {message}"
        );
        // The way out it offers must be one that WORKS from here: this branch is only reached with
        // `literal_path` set, where the wire's `"all"` selector is read as a path named `all`.
        assert!(
            message.contains("proton-sync approve --all"),
            "the message must name the flag, not the reserved word a literal-path request cannot \
             use: {message}"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("approvals")
                .is_empty(),
            "an ambiguous approve must authorize nothing"
        );

        // "all" is the deliberate every-item form and still works, as does a deny.
        apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some("all"),
            false,
            true,
            None,
        )
        .expect("approve all");
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("approvals")
                .len(),
            2
        );
        apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some(&selector),
            true,
            false,
            None,
        )
        .expect("ambiguous deny");
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("approvals")
                .is_empty(),
            "a deny is not gated: revoking more than asked is the safe direction"
        );
    }

    /// #313: every arm that renders the caller's selector back into a reply bounds it, and
    /// **nothing on the matching path is bounded** — a display cap that reached the comparison
    /// would make two different long paths compare equal, which is the opposite of the #61 rule
    /// that a wire path is a rendering and an ambiguous selector authorizes nothing.
    #[test]
    fn an_oversized_selector_is_bounded_where_it_is_echoed_and_nowhere_else() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        // A control request may carry up to `ipc::MAX_REQUEST_BYTES`, and nothing narrower bounds
        // `argument` — so this is what a client can actually send.
        let huge = "a".repeat(64 * 1024);
        let bounded = |message: &str| {
            assert!(
                message.len() <= SELECTOR_DISPLAY_LIMIT + 400,
                "an echoed selector must not carry the client's 64 KiB back: {} bytes",
                message.len()
            );
        };

        bounded(
            &apply_approval_command(
                &daemon.pair().connection,
                &[],
                Some(&huge),
                true,
                false,
                None,
            )
            .expect("deny with no match"),
        );
        bounded(
            &apply_keep_command(&daemon.pair().connection, &[], Some(&huge), true)
                .expect("keep with no match")
                .0,
        );
        // The pre-approval arm (#227), reached by an `approve` nothing pending matches.
        bounded(
            &apply_approval_command(
                &daemon.pair().connection,
                &[],
                Some(&huge),
                true,
                true,
                None,
            )
            .expect("pre-approve without a direction"),
        );
        bounded(
            &apply_approval_command(
                &daemon.pair().connection,
                &[],
                Some(&huge),
                true,
                true,
                Some(DeleteDirection::Remote),
            )
            .expect("pre-approve with no index record"),
        );

        // The ambiguous arm renders the *client's* selector too (the issue body reads it as the
        // daemon's own paths; it is not).
        let long_prefix = "b".repeat(SELECTOR_DISPLAY_LIMIT * 2);
        let ambiguous: Vec<PendingDeletion> = [0xffu8, 0xfeu8]
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                let mut bytes = long_prefix.clone().into_bytes();
                bytes.push(*byte);
                bytes.extend_from_slice(b".txt");
                PendingDeletion {
                    path: PathBuf::from(OsStr::from_bytes(&bytes)),
                    direction: DeleteDirection::Remote,
                    entity_kind: EntityKind::File,
                    fingerprint: format!("fp-{index}"),
                    detected_epoch_secs: 1,
                    first_seen_epoch_secs: 1,
                    subtree_files: None,
                    subtree_bytes: None,
                    disposal: LocalDisposal::Permanent,
                }
            })
            .collect();
        let lossy = crate::ipc::wire_path(&ambiguous[0].path).into_owned();
        assert_eq!(lossy, crate::ipc::wire_path(&ambiguous[1].path));
        let message = apply_approval_command(
            &daemon.pair().connection,
            &ambiguous,
            Some(&lossy),
            true,
            true,
            None,
        )
        .expect("ambiguous approve");
        assert!(message.contains("cannot be told apart"), "{message}");
        bounded(&message);

        // And the load-bearing half, which is what a display cap would destroy if it ever reached
        // the comparison: two pending deletions that differ ONLY beyond the cap. A truncating
        // matcher renders them one selector, so the targeted approve either refuses both as
        // ambiguous or authorizes a deletion nobody picked. Neither is acceptable, and neither is
        // visible from a test that approves a single long path (which matches its own truncation).
        let shared_prefix = "c".repeat(SELECTOR_DISPLAY_LIMIT * 2);
        let pending: Vec<PendingDeletion> = ["one.txt", "two.txt"]
            .iter()
            .enumerate()
            .map(|(index, leaf)| PendingDeletion {
                path: PathBuf::from(format!("{shared_prefix}/{leaf}")),
                direction: DeleteDirection::Remote,
                entity_kind: EntityKind::File,
                fingerprint: format!("fp-long-{index}"),
                detected_epoch_secs: 1,
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
                disposal: LocalDisposal::Permanent,
            })
            .collect();
        let selector = crate::ipc::wire_path(&pending[1].path).into_owned();
        let message = apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some(&selector),
            true,
            true,
            None,
        )
        .expect("approve a long path");
        assert!(
            message.contains("approved 1"),
            "a path longer than the display cap must still be approvable, and unambiguously: \
             {message}"
        );
        let approved: Vec<PathBuf> = crate::index::load_delete_approvals(&daemon.pair().connection)
            .expect("approvals")
            .into_iter()
            .map(|approval| approval.path)
            .collect();
        assert_eq!(
            approved,
            vec![pending[1].path.clone()],
            "exactly the one named, pinned to the whole path rather than the shortened rendering"
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
            first_seen_epoch_secs: 1,
            subtree_files: None,
            subtree_bytes: None,
            disposal: LocalDisposal::Permanent,
        }];

        let message = apply_approval_command(
            &daemon.pair().connection,
            &pending,
            Some("nested/folder/"),
            true,
            true,
            None,
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");
        daemon.reconcile_blocking().expect_clean("reconcile");
        assert_eq!(
            daemon.pair().pending_deletions.len(),
            1,
            "a deletion is pending"
        );

        // No selector → no-op: nothing is approved even though something is pending.
        let message = daemon
            .apply_approval_command(None, true)
            .expect("no-selector approve");
        assert!(
            message.contains("no target"),
            "unexpected message: {message}"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
            "a missing selector must not approve any deletion"
        );

        // Explicit "all" is the deliberate way to approve everything pending.
        daemon
            .apply_approval_command(Some("all"), true)
            .expect("approve all");
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
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
            &daemon.pair().connection,
            &base_record("removed.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("first reconcile");
        assert!(
            operations.lock().expect("ops").is_empty(),
            "the remote delete must be withheld pending approval"
        );
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        let pending = daemon.pair().pending_deletions[0].clone();
        assert_eq!(pending.direction, DeleteDirection::Remote);

        // Approve exactly this deletion (path + direction + fingerprint), then reconcile again.
        upsert_delete_approval(
            &daemon.pair().connection,
            &pending.path,
            pending.direction,
            &pending.fingerprint,
            1,
        )
        .expect("approve");

        daemon.reconcile_blocking().expect_clean("second reconcile");
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
            get_record(&daemon.pair().connection, Path::new("removed.txt"))
                .expect("lookup")
                .is_none(),
            "the applied delete purges the index record"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
            "the approval is consumed once the delete has applied"
        );
        assert!(daemon.pair().pending_deletions.is_empty());
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            !local_path.exists(),
            "a subtree opted out of the guard must delete without approval"
        );
        assert!(daemon.pair().pending_deletions.is_empty());
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            local_path.exists() && daemon.pair().pending_deletions.len() == 1,
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
            &daemon.pair().connection,
            &base_record("a.txt", Some("vol~na"), base.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("incremental reconcile");

        assert_eq!(full_walks.load(Ordering::SeqCst), 0, "stayed incremental");
        assert!(
            local_root.join("a.txt").exists(),
            "the withheld local delete must not touch the file"
        );
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "the cursor must NOT advance while a destructive action is withheld, so the pending \
             delete keeps re-deriving from ground truth every pass"
        );
    }

    // --- the withheld-deletion lifecycle: first-seen, subtree totals, keep, pre-approval ------

    /// A daemon whose local root holds `keep.txt`, with a matching baseline record and an empty
    /// remote — the shape that plans a withheld `LocalDelete`. Returns the daemon and the file.
    fn daemon_withholding_a_local_delete(
        directory: &Path,
        local_root: &Path,
    ) -> (Daemon<RecordingProtonClient>, PathBuf) {
        let local_path = local_root.join("keep.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon =
            Daemon::with_client(guarded_config(directory, local_root), client).expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");
        (daemon, local_path)
    }

    #[test]
    fn a_withheld_deletion_keeps_the_age_it_was_first_seen_at_across_restarts() {
        // #225. `detected_epoch_secs` is the age of the PASS — the gate stamps `now` on everything
        // it withholds, and a pass cannot idle-skip while anything is pending — so a three-day-old
        // deletion reported an age of seconds. The stored first-seen is the fact that survives.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (mut daemon, _) = daemon_withholding_a_local_delete(directory.path(), &local_root);

        daemon.reconcile_blocking().expect_clean("first reconcile");
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        let fingerprint = daemon.pair().pending_deletions[0].fingerprint.clone();

        // Age the stored row to three days ago and start a FRESH daemon over the same database:
        // the queue survives a restart, so its age has to as well.
        const THREE_DAYS_AGO: u64 = 1_700_000_000;
        replace_withheld_deletions(
            &daemon.pair().connection,
            &[WithheldDeletion {
                path: PathBuf::from("keep.txt"),
                direction: DeleteDirection::Local,
                fingerprint,
                first_seen_epoch_secs: THREE_DAYS_AGO,
            }],
        )
        .expect("age the stored row");
        drop(daemon);

        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("restarted daemon");
        daemon.reconcile_blocking().expect_clean("second reconcile");

        let pending = &daemon.pair().pending_deletions[0];
        assert_eq!(
            pending.first_seen_epoch_secs, THREE_DAYS_AGO,
            "the first-seen time must be carried forward, not re-stamped"
        );
        assert!(
            pending.detected_epoch_secs > THREE_DAYS_AGO,
            "detected_epoch_secs is still this pass's own clock: {pending:?}"
        );
    }

    #[test]
    fn a_different_deletion_at_the_same_path_re_stamps_its_age() {
        // The carryover is keyed on the fingerprint as well as the path: a new deletion at a path
        // that already had one is a different thing, and dating it from the old one would age it
        // from something that already resolved.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (mut daemon, _) = daemon_withholding_a_local_delete(directory.path(), &local_root);
        replace_withheld_deletions(
            &daemon.pair().connection,
            &[WithheldDeletion {
                path: PathBuf::from("keep.txt"),
                direction: DeleteDirection::Local,
                fingerprint: "a-fingerprint-from-some-earlier-deletion".to_owned(),
                first_seen_epoch_secs: 1_700_000_000,
            }],
        )
        .expect("seed a stale row");

        daemon.reconcile_blocking().expect_clean("reconcile");

        let pending = &daemon.pair().pending_deletions[0];
        assert_eq!(
            pending.first_seen_epoch_secs, pending.detected_epoch_secs,
            "a fingerprint that does not match the stored row starts the clock again"
        );
    }

    #[test]
    fn a_withheld_directory_deletion_reports_what_the_subtree_costs() {
        // #208. The confirmation names what you would lose, and a directory's own `file_size` in
        // `file_index` is not a subtree total. Files only, strict descendants only.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::create_dir(local_root.join("photos")).expect("local directory");
        fs::write(local_root.join("photos/a.jpg"), b"aaaa").expect("child a");
        fs::write(local_root.join("photos/b.jpg"), b"bbbbbb").expect("child b");
        fs::write(local_root.join("outside.txt"), b"z").expect("sibling");

        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &directory_record("photos", Some("dir-id")),
        )
        .expect("directory record");
        // A sibling OUTSIDE the folder, and the folder's own record, must not be counted.
        for (path, bytes) in [
            ("photos/a.jpg", 4u64),
            ("photos/b.jpg", 6u64),
            ("outside.txt", 1u64),
        ] {
            let hash = crate::index::compute_sha1(&local_root.join(path)).expect("hash");
            let mut record = base_record(path, Some(path), hash.as_str());
            record.file_size = bytes;
            upsert_record(&daemon.pair().connection, &record).expect("record");
        }

        daemon.reconcile_blocking().expect_clean("reconcile");

        let folder = daemon
            .pair()
            .pending_deletions
            .iter()
            .find(|pending| pending.path == Path::new("photos"))
            .expect("the folder deletion is withheld");
        assert_eq!(folder.subtree_files, Some(2));
        assert_eq!(folder.subtree_bytes, Some(10));
        // A file's own size is a lookup the client already has, so it reports no subtree at all
        // rather than a total of one.
        let file = daemon
            .pair()
            .pending_deletions
            .iter()
            .find(|pending| pending.path == Path::new("outside.txt"))
            .expect("the file deletion is withheld");
        assert_eq!((file.subtree_files, file.subtree_bytes), (None, None));
    }

    #[test]
    fn keeping_a_withheld_local_delete_puts_the_file_back_on_proton() {
        // #224. `Keep it — put it back on Proton Drive`: purging the baseline record turns the
        // surviving local file from "a deletion nobody approved" into a file the engine has never
        // seen, which the bootstrap arm uploads.
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("first reconcile");
        assert_eq!(daemon.pair().pending_deletions.len(), 1);

        let (message, changed) = daemon
            .apply_keep_command(Some("keep.txt"))
            .expect("keep the deletion");
        assert!(changed, "the refusal purged something: {message}");
        assert!(message.contains("kept 1"), "{message}");
        assert!(
            get_record(&daemon.pair().connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_none(),
            "the baseline record is what made this a deletion; keeping it removes that"
        );

        daemon.reconcile_blocking().expect_clean("second reconcile");
        assert!(
            operations
                .lock()
                .expect("ops")
                .iter()
                .any(|operation| matches!(
                    operation,
                    RecordedOperation::Upload { relative_path, .. }
                        if relative_path == Path::new("keep.txt")
                )),
            "the kept file must be uploaded back: {:?}",
            operations.lock().expect("ops")
        );
        assert!(
            daemon.pair().pending_deletions.is_empty(),
            "the queue empties because the planner stops deriving the deletion"
        );
    }

    #[test]
    fn keeping_a_folder_purges_its_whole_subtree() {
        // A directory deletion is planned RECURSIVELY — one action for the folder, every
        // descendant suppressed — so purging the folder alone would re-adopt it while each
        // surviving child record went on deriving its own withheld delete.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::create_dir(local_root.join("photos")).expect("local directory");
        fs::write(local_root.join("photos/a.jpg"), b"aaaa").expect("child");
        // `photosx` shares a byte prefix with `photos` and is NOT under it.
        fs::write(local_root.join("photosx"), b"z").expect("prefix sibling");

        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &directory_record("photos", Some("dir-id")),
        )
        .expect("directory record");
        for path in ["photos/a.jpg", "photosx"] {
            let hash = crate::index::compute_sha1(&local_root.join(path)).expect("hash");
            upsert_record(
                &daemon.pair().connection,
                &base_record(path, Some(path), hash.as_str()),
            )
            .expect("record");
        }

        daemon.reconcile_blocking().expect_clean("reconcile");
        let (_, changed) = daemon
            .apply_keep_command(Some("photos"))
            .expect("keep the folder");

        assert!(changed);
        for path in ["photos", "photos/a.jpg"] {
            assert!(
                get_record(&daemon.pair().connection, Path::new(path))
                    .expect("lookup")
                    .is_none(),
                "{path} must be purged with the subtree"
            );
        }
        assert!(
            get_record(&daemon.pair().connection, Path::new("photosx"))
                .expect("lookup")
                .is_some(),
            "containment is component-wise: photosx is not under photos"
        );
    }

    #[test]
    fn keeping_a_folder_revokes_the_approvals_under_it_too() {
        // Copilot, PR #309. A descendant can carry a standing approval the folder card never showed
        // — an earlier pass that queued it individually, or a pre-pass `approve --direction` — and
        // the purge alone would leave it pointing at a record that no longer exists. It is not
        // inert: the fingerprint is the content's own, so re-adopting the file unchanged makes the
        // stale approval match the NEW record exactly, and the next deletion of that path would
        // execute without ever being shown.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::create_dir(local_root.join("photos")).expect("local directory");
        fs::write(local_root.join("photos/a.jpg"), b"aaaa").expect("child");

        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &directory_record("photos", Some("dir-id")),
        )
        .expect("directory record");
        let child_hash =
            crate::index::compute_sha1(&local_root.join("photos/a.jpg")).expect("hash");
        upsert_record(
            &daemon.pair().connection,
            &base_record("photos/a.jpg", Some("child-id"), child_hash.as_str()),
        )
        .expect("child record");
        upsert_delete_approval(
            &daemon.pair().connection,
            Path::new("photos/a.jpg"),
            DeleteDirection::Local,
            child_hash.as_str(),
            1,
        )
        .expect("a standing approval for the descendant");

        daemon.reconcile_blocking().expect_clean("reconcile");
        let (_, changed) = daemon
            .apply_keep_command(Some("photos"))
            .expect("keep the folder");

        assert!(changed);
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
            "an approval under a kept folder must go with the record it was pinned to"
        );
    }

    #[test]
    fn a_keep_whose_fingerprint_moved_is_inert() {
        // The published queue can be a pass stale, and a refusal is pinned exactly as an approval
        // is: it names one exact thing, or it does nothing.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (mut daemon, _) = daemon_withholding_a_local_delete(directory.path(), &local_root);
        daemon.reconcile_blocking().expect_clean("reconcile");

        // The user saw one thing; the index now says another.
        daemon.pair_mut().pending_deletions[0].fingerprint = "what-the-user-saw".to_owned();
        let (message, changed) = daemon
            .apply_keep_command(Some("keep.txt"))
            .expect("keep the deletion");

        assert!(!changed, "{message}");
        assert!(message.contains("nothing was kept"), "{message}");
        assert!(
            get_record(&daemon.pair().connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_some(),
            "a stale refusal must not purge anything"
        );
    }

    #[test]
    fn an_ambiguous_keep_selector_keeps_nothing() {
        // The same rule `approve` fails closed on (#298/#61), for a reason `deny` does not share:
        // keeping MUTATES the index and puts content back on a side the user may have cleared.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(guarded_config(directory.path(), &local_root), client)
            .expect("daemon");
        let pending: Vec<PendingDeletion> = [&b"bad-\xff.txt"[..], &b"bad-\xfe.txt"[..]]
            .iter()
            .map(|bytes| PendingDeletion {
                path: PathBuf::from(OsStr::from_bytes(bytes)),
                direction: DeleteDirection::Local,
                entity_kind: EntityKind::File,
                fingerprint: "fp".to_owned(),
                detected_epoch_secs: 1,
                first_seen_epoch_secs: 1,
                subtree_files: None,
                subtree_bytes: None,
                disposal: LocalDisposal::Permanent,
            })
            .collect();
        let selector = crate::ipc::wire_path(&pending[0].path).into_owned();
        assert_eq!(selector, crate::ipc::wire_path(&pending[1].path));

        let (message, changed) =
            apply_keep_command(&daemon.pair().connection, &pending, Some(&selector), true)
                .expect("ambiguous keep");

        assert!(!changed);
        assert!(
            message.contains("nothing was kept") && message.contains("proton-sync keep --all"),
            "an ambiguous selector must keep nothing and name the deliberate form: {message}"
        );
    }

    #[test]
    fn a_planned_deletion_can_be_approved_before_any_pass_withholds_it() {
        // #227. The Plan screen authorises the plan's deletion BEFORE the pass runs, when nothing
        // is pending by construction. The gate needs no change to honour it: the row this writes
        // is the `(path, direction, fingerprint)` triple it already looks for.
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        // Nothing pending yet — no pass has run at all.
        let message = apply_approval_command(
            &daemon.pair().connection,
            &[],
            Some("keep.txt"),
            true,
            true,
            Some(DeleteDirection::Local),
        )
        .expect("pre-approve");
        assert!(message.contains("approved 1"), "{message}");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            !local_path.exists(),
            "the pre-approved local delete must apply on the pass that plans it"
        );
        assert!(
            daemon.pair().pending_deletions.is_empty(),
            "nothing was withheld, so nothing is pending"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
            "the approval is consumed in the same checkpoint as the delete's purge"
        );
    }

    #[test]
    fn a_pre_approval_with_no_direction_authorises_nothing() {
        // A path alone does not say WHICH of the two deletions at it is meant, and an ambiguous
        // selector authorises nothing (#298, one step earlier in time).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (daemon, _) = daemon_withholding_a_local_delete(directory.path(), &local_root);

        let message = apply_approval_command(
            &daemon.pair().connection,
            &[],
            Some("keep.txt"),
            true,
            true,
            None,
        )
        .expect("pre-approve without a direction");

        assert!(message.contains("name its direction"), "{message}");
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
            "nothing may be recorded from a selector that does not name a deletion"
        );
    }

    #[test]
    fn a_pre_approval_is_refused_for_a_path_with_nothing_to_pin_to() {
        // The fingerprint comes from the INDEX, not from the client. With no record there is
        // nothing to pin to, and `delete_fingerprint`'s remaining fallback is the *action's*
        // remote id — unknowable before the plan exists — so a row written here could never match.
        // Also the first place `approve` would store a client-supplied path, hence the guard.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (daemon, _) = daemon_withholding_a_local_delete(directory.path(), &local_root);

        for (selector, expected) in [
            ("never-synced.txt", "no synced record"),
            ("../escape.txt", "not a valid relative path"),
            (".", "not a valid relative path"),
        ] {
            let message = apply_approval_command(
                &daemon.pair().connection,
                &[],
                Some(selector),
                true,
                true,
                Some(DeleteDirection::Local),
            )
            .expect("pre-approve");
            assert!(
                message.contains(expected),
                "'{selector}' should be refused with '{expected}': {message}"
            );
        }
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
        );
    }

    #[test]
    fn the_agreed_version_is_captured_when_it_is_agreed_and_answers_the_conflict_later() {
        // #217/#347. The whole feature in one pass sequence, because the timing IS the feature: the
        // ancestor exists only between the sync that agreed it and the next edit, and by the time a
        // conflict is planned the local file has moved AND `SyncAction::Conflict` has replaced the
        // baseline digest with the current one. A capture at conflict time would file the diverged
        // version under the ancestor's name.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let agreed = "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n";
        let agreed_hash = sha1_bytes(agreed.as_bytes());
        fs::write(local_root.join("todo.txt"), agreed).expect("write agreed");

        // A remote file with the same digest: the pass adopts it, which is a file becoming agreed.
        let client = RecordingProtonClient::new(HashMap::from([(
            PathBuf::from("todo.txt"),
            RemoteFile {
                path: PathBuf::from("/Drive/RemoteFolder/todo.txt"),
                id: "file-todo".to_owned(),
                name: "todo.txt".to_owned(),
                sha1_hash: Some(agreed_hash.clone()),
                downloadable: true,
            },
        )]))
        .0;
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        daemon.reconcile_blocking().expect_clean("adopt");

        let summary = crate::index::load_agreed_summary(
            &daemon.pair().connection,
            Path::new("todo.txt"),
            &agreed_hash,
        )
        .expect("load")
        .expect("the agreed version was summarised when it became agreed");
        assert_eq!(summary.line_count(), 4);

        // Now both sides move. The local edit changes line 2 back; the summary must still answer
        // for the version they agreed on, under ITS digest, not the new one.
        let mine = "# Todo\n- buy milk\n- call Alice\n- ship v1\n";
        fs::write(local_root.join("todo.txt"), mine).expect("local edit");

        let facts = crate::ancestor::compare_to_ancestor(
            &summary,
            &crate::ancestor::LineSummary::of(mine).expect("current"),
        )
        .expect("inside the cutoff");
        assert_eq!(
            facts,
            crate::ancestor::VersionFacts {
                added: 0,
                changed: 1,
                removed: 0
            },
            "you changed a line — which is only answerable because the ancestor was captured"
        );

        // And the record the conflict itself writes does NOT become an ancestor: a `Conflict` row
        // carries the local file's digest, so summarising it would answer the card with the very
        // version it is asking about.
        let mine_hash = sha1_bytes(mine.as_bytes());
        assert!(
            crate::index::load_agreed_summary(
                &daemon.pair().connection,
                Path::new("todo.txt"),
                &mine_hash,
            )
            .expect("load")
            .is_none(),
            "the diverged version must not be filed as an ancestor"
        );
    }

    #[test]
    fn a_file_that_cannot_be_summarised_costs_a_sentence_and_never_a_pass() {
        // #347's headline case. A binary file is adopted exactly as a text one is; it simply has no
        // ancestor summary, and the card then draws only what it can compute. The pass is CLEAN —
        // nothing about a decoration may fail a sync.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let bytes = [0u8, 159, 146, 150, b'\n', 0, 1, 2];
        let hash = sha1_bytes(&bytes);
        fs::write(local_root.join("clip.bin"), bytes).expect("write binary");

        let client = RecordingProtonClient::new(HashMap::from([(
            PathBuf::from("clip.bin"),
            RemoteFile {
                path: PathBuf::from("/Drive/RemoteFolder/clip.bin"),
                id: "file-clip".to_owned(),
                name: "clip.bin".to_owned(),
                sha1_hash: Some(hash.clone()),
                downloadable: true,
            },
        )]))
        .0;
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        daemon
            .reconcile_blocking()
            .expect_clean("adopt a binary file");

        assert!(
            crate::index::load_agreed_summary(
                &daemon.pair().connection,
                Path::new("clip.bin"),
                &hash,
            )
            .expect("load")
            .is_none(),
            "a file that is not text has no line summary, and that is ordinary"
        );
    }

    #[test]
    fn a_kept_remote_direction_deletion_comes_back_only_on_a_full_walk() {
        // THE REASON `Keep` LATCHES `force_full_walk`. An incremental pass builds its remote map as
        // `reconstruct_remote(base ⊕ delta)`, so the baseline IS the remote view: once the refusal
        // purges the record, the surviving remote file is in no reconstructed map, and no event
        // will ever re-derive it — the deletion happened HERE, so the volume stream never mentioned
        // it. Without the latch `bring it back to this computer` downloads nothing, ever (the
        // periodic resync is off by default and a restart warm-starts from the cursor).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let base = sha1_bytes(b"base");

        // Local copy absent, remote node still present, synced baseline → RemoteDelete.
        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("a.txt"),
            remote_file_entity("a.txt", "vol~na", base.as_str()),
        )]));
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                delete_approval_local: true,
                delete_approval_remote: true,
                ..event_config(directory.path(), &local_root)
            },
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![one_page("cursor-1", vec![]); 3],
            ))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("a.txt", Some("vol~na"), base.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;
        // The local deletion is what this pass has to notice, and no remote event announces it —
        // the same clause a dropped watcher event takes (#51).
        daemon.pair_mut().force_local_rescan = true;

        daemon.reconcile_blocking().expect_clean("first reconcile");
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        assert_eq!(
            daemon.pair().pending_deletions[0].direction,
            DeleteDirection::Remote
        );

        let (_, changed) = daemon.apply_keep_command(Some("a.txt")).expect("keep");
        assert!(changed);

        // An incremental pass cannot see the survivor at all: this is the failure the latch exists
        // to prevent, asserted rather than assumed.
        daemon.reconcile_blocking().expect_clean("incremental pass");
        assert_eq!(full_walks.load(Ordering::SeqCst), 0, "stayed incremental");
        assert!(
            !local_root.join("a.txt").exists(),
            "an incremental pass has no remote map that holds the kept file"
        );

        // The latch the `keep` control command sets alongside the purge.
        daemon
            .shared
            .pair
            .force_full_walk
            .store(true, Ordering::SeqCst);
        daemon.reconcile_blocking().expect_clean("full-walk pass");

        assert_eq!(full_walks.load(Ordering::SeqCst), 1);
        assert!(
            local_root.join("a.txt").exists(),
            "the full walk sees the surviving remote file and downloads it back"
        );
    }

    #[test]
    fn a_due_schedule_makes_the_next_pass_a_full_sweep_and_two_triggers_are_still_one() {
        // #193. The arm is `latch_scheduled_full_sweep` + `reconcile_if_needed` + a re-arm, so this
        // drives the latch directly rather than the `select!` — the timing is `schedule.rs`'s and
        // pure, and a test that waited on wall clock would prove nothing this does not.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");

        let client = EventFakeClient::new(HashMap::new());
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                full_scan_schedule: Some("weekly sun 03:00".parse().expect("schedule")),
                ..event_config(directory.path(), &local_root)
            },
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![one_page("cursor-1", vec![]); 4],
            ))),
        )
        .expect("daemon");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;

        // Steady state: a pass with no trigger stays incremental. Asserted, or the sweep below
        // would be indistinguishable from the daemon full-walking anyway.
        daemon
            .reconcile_blocking()
            .expect_clean("steady-state pass");
        assert_eq!(full_walks.load(Ordering::SeqCst), 0);

        daemon.latch_scheduled_full_sweep();
        daemon.reconcile_blocking().expect_clean("scheduled sweep");
        assert_eq!(full_walks.load(Ordering::SeqCst), 1);

        // Consumed exactly once, like every other user of this latch: the pass AFTER a scheduled
        // sweep is incremental again, or one schedule would turn the daemon into a full-walking one
        // for ever.
        daemon
            .reconcile_blocking()
            .expect_clean("pass after the sweep");
        assert_eq!(full_walks.load(Ordering::SeqCst), 1);

        // THE "does not queue a second, does not suppress one either" RULE, and it is structural
        // rather than arranged: the latch is a `bool`, so a schedule firing while a safety net has
        // already latched a sweep is one sweep and neither trigger is lost. Nothing arbitrates,
        // which is why there is no precedence rule to get wrong.
        daemon
            .shared
            .pair
            .force_full_walk
            .store(true, Ordering::SeqCst);
        daemon.latch_scheduled_full_sweep();
        daemon
            .reconcile_blocking()
            .expect_clean("two triggers, one sweep");
        assert_eq!(full_walks.load(Ordering::SeqCst), 2);
        daemon
            .reconcile_blocking()
            .expect_clean("and nothing left over");
        assert_eq!(full_walks.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_daemon_with_no_schedule_arms_no_sweep_timer() {
        // The `None` half, which is what every config written before this key resolves to and must
        // keep resolving to: no schedule, no deadline, and the run loop's arm never armed.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        assert!(daemon.pair().config.full_scan_schedule.is_none());
        assert!(daemon.next_full_sweep_in().is_none());

        // With one set, the deadline is real, in the future, and inside the longest a weekly
        // schedule can be away — the property the run loop depends on, since a negative or absurd
        // delay is a timer that fires immediately and for ever.
        daemon.pair_mut().config.full_scan_schedule =
            Some("weekly sun 03:00".parse().expect("schedule"));
        let delay = daemon
            .next_full_sweep_in()
            .expect("a scheduled sweep has a deadline");
        assert!(
            delay <= Duration::from_secs(7 * 24 * 3600 + 3600),
            "got {delay:?}"
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
            &daemon.pair().connection,
            &base_record("a.txt", Some("vol~nx"), base_hash.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        // Pass 1 — the id takeover. Incremental: Download(b.txt) commits a row with proton_id
        // vol~nx; LocalDelete(a.txt) is withheld, so a.txt keeps its vol~nx row → duplicate id.
        daemon
            .reconcile_blocking()
            .expect_clean("pass 1 (incremental takeover)");
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
        assert_eq!(
            daemon.pair().pending_deletions.len(),
            1,
            "LocalDelete(a.txt) held"
        );
        assert_eq!(
            daemon.pair().pending_deletions[0].direction,
            DeleteDirection::Local
        );
        // Both rows now hold the same composed id — the transient duplicate. (A naive #71(c) would
        // already have cleared a.txt's id here, failing this assertion.)
        assert_eq!(
            get_record(&daemon.pair().connection, Path::new("a.txt"))
                .expect("lookup a.txt")
                .expect("a.txt row")
                .proton_id
                .as_deref(),
            Some("vol~nx"),
            "the withheld local delete keeps a.txt pinned to vol~nx"
        );
        assert_eq!(
            get_record(&daemon.pair().connection, Path::new("b.txt"))
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
            .expect_clean("pass 2 (fallback re-derives the withheld delete)");
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
            daemon.pair().pending_deletions.len(),
            1,
            "the LocalDelete must still be withheld, not silently dropped"
        );
        assert_eq!(
            daemon.pair().pending_deletions[0].direction,
            DeleteDirection::Local
        );
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;
        // The watcher would have observed the local deletion, so the first pass is not idle.
        daemon
            .pair_mut()
            .pending_changes
            .insert(PathBuf::from("keep.txt"));

        // Pass 1: withhold.
        daemon
            .reconcile_blocking()
            .expect_clean("first incremental reconcile");
        assert_eq!(full_walks.load(Ordering::SeqCst), 0, "stayed incremental");
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        let pending = daemon.pair().pending_deletions[0].clone();
        assert_eq!(pending.direction, DeleteDirection::Remote);

        // Pass 2: empty delta and `pending_changes` now cleared, but still pending → must re-derive
        // (not idle-skip), so the queue stays fresh.
        daemon
            .reconcile_blocking()
            .expect_clean("second incremental reconcile");
        assert_eq!(
            daemon.pair().pending_deletions.len(),
            1,
            "an empty-delta pass must re-derive the still-pending remote delete"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_some(),
            "nothing is deleted while unapproved"
        );

        // Approve, then reconcile once more with the same empty delta: it must apply now.
        upsert_delete_approval(
            &daemon.pair().connection,
            &pending.path,
            pending.direction,
            &pending.fingerprint,
            1,
        )
        .expect("approve");

        daemon
            .reconcile_blocking()
            .expect_clean("third incremental reconcile");
        assert!(
            get_record(&daemon.pair().connection, Path::new("keep.txt"))
                .expect("lookup")
                .is_none(),
            "an approved remote delete must apply on the next incremental pass, not wait for a \
             periodic bootstrap"
        );
        assert!(daemon.pair().pending_deletions.is_empty());
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
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
            &daemon.pair().connection,
            &base_record("gone.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "both-side deletion should not mutate local or remote files"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("gone.txt"))
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
            &daemon.pair().connection,
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
            &daemon.pair().connection,
            &base_record("docs/report.txt", Some("report-id"), "same-hash"),
        )
        .expect("file base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![RecordedOperation::Delete {
                remote_path: PathBuf::from("/Drive/RemoteFolder/docs"),
            }],
            "the whole subtree should be removed with a single recursive delete call"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs"))
                .expect("index lookup")
                .is_none(),
            "recursive remote delete should purge the directory's own index record"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/report.txt"))
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
            &daemon.pair().connection,
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
            &daemon.pair().connection,
            &base_record("docs/report.txt", Some("report-id"), base_hash.as_str()),
        )
        .expect("file base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "recursive local delete reconciliation must not call Proton mutations"
        );
        assert!(
            !local_root.join("docs").exists(),
            "the whole local directory subtree should be removed"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs"))
                .expect("index lookup")
                .is_none(),
            "recursive local delete should purge the directory's own index record"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/report.txt"))
                .expect("index lookup")
                .is_none(),
            "recursive local delete should purge descendant index records too"
        );
    }

    #[test]
    fn a_stale_base_kind_does_not_let_a_directory_delete_remove_a_live_local_file() {
        // #282 end to end, with the delete-approval guard OFF (test_config) so nothing but the
        // plan itself stands between the file and `remove_dir_all`. The base row still calls
        // `docs/sub` a directory; it is now a never-uploaded FILE and `docs` is gone remotely.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("docs")).expect("local docs directory");
        let file_path = local_root.join("docs").join("sub");
        fs::write(&file_path, b"the only copy").expect("local file");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &directory_record("docs", Some("docs-id")),
        )
        .expect("directory base record");
        upsert_record(
            &daemon.pair().connection,
            &directory_record("docs/sub", Some("sub-id")),
        )
        .expect("stale directory base record over a live file");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            file_path.is_file(),
            "the live local file must survive: a stale base kind must not prove its parent a \
             clean recursive delete"
        );
        assert_eq!(
            fs::read(&file_path).expect("surviving file"),
            b"the only copy",
            "the surviving file must be untouched"
        );
        let recorded = operations.lock().expect("operations lock").clone();
        assert!(
            recorded.contains(&RecordedOperation::Upload {
                local_path: file_path.clone(),
                remote_root: PathBuf::from("/Drive/RemoteFolder"),
                relative_path: PathBuf::from("docs/sub"),
            }),
            "the never-uploaded file must be uploaded instead: {recorded:?}"
        );
        let record = get_record(&daemon.pair().connection, Path::new("docs/sub"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            record.entity_kind,
            EntityKind::File,
            "the upload's upsert must replace the stale directory row"
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
            &daemon.pair().connection,
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
            &daemon.pair().connection,
            &base_record("old-docs/report.txt", Some("report-id"), file_hash.as_str()),
        )
        .expect("nested file base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
            get_record(&daemon.pair().connection, Path::new("old-docs"))
                .expect("old directory index lookup")
                .is_none(),
            "old directory path should be purged from the index"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("old-docs/report.txt"))
                .expect("old descendant index lookup")
                .is_none(),
            "old descendant path should be purged from the index"
        );
        let directory_record = get_record(&daemon.pair().connection, Path::new("new-docs"))
            .expect("new directory index lookup")
            .expect("new directory index record");
        assert_eq!(directory_record.entity_kind, EntityKind::Directory);
        assert_eq!(directory_record.proton_id.as_deref(), Some("docs-id"));
        let descendant_record =
            get_record(&daemon.pair().connection, Path::new("new-docs/report.txt"))
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
    fn a_remote_directory_move_never_acts_on_an_untracked_descendant_at_its_pre_move_path() {
        // #12. `old-docs` was renamed to `new-docs` remotely, and the local `old-docs` also holds
        // `fresh.txt`, which no base row tracks. The MoveLocal runs first and physically relocates
        // `fresh.txt`, so anything planned for it at the OLD path is planned against a local state
        // that no longer exists: the upload would recreate a spurious remote `old-docs` (its parent
        // is a move source, not a planned create) and then read a vanished file.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("old-docs")).expect("local old-docs directory");
        let file_path = local_root.join("old-docs").join("report.txt");
        fs::write(&file_path, b"same content").expect("nested local file");
        let file_hash = crate::index::compute_sha1(&file_path).expect("nested file hash");
        fs::write(local_root.join("old-docs").join("fresh.txt"), b"brand new")
            .expect("untracked nested local file");
        // Two levels down, so the suppression is proved for a whole subtree and not just the
        // moved directory's immediate children.
        fs::create_dir(local_root.join("old-docs").join("nested")).expect("untracked directory");
        fs::write(local_root.join("old-docs/nested/deep.txt"), b"deeper").expect("deep file");
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
            &daemon.pair().connection,
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
            &daemon.pair().connection,
            &base_record("old-docs/report.txt", Some("report-id"), file_hash.as_str()),
        )
        .expect("nested file base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        let recorded = operations.lock().expect("operations lock").clone();
        assert!(
            !recorded.iter().any(|operation| matches!(
                operation,
                RecordedOperation::EnsureDirectory { relative_path, .. }
                    if relative_path == Path::new("old-docs")
            )),
            "the moved-away parent must never be recreated remotely: {recorded:?}"
        );
        assert!(
            !recorded.iter().any(|operation| matches!(
                operation,
                RecordedOperation::Upload { relative_path, .. }
                    if relative_path.starts_with("old-docs")
            )),
            "nothing in the moved subtree may be uploaded from its pre-move path: {recorded:?}"
        );
        assert!(
            !recorded.iter().any(|operation| matches!(
                operation,
                RecordedOperation::EnsureDirectory { relative_path, .. }
                    if relative_path.starts_with("old-docs")
            )),
            "nor may any of its directories be created remotely there: {recorded:?}"
        );
        assert!(
            local_root.join("new-docs").join("fresh.txt").is_file(),
            "the untracked file rides along with the directory rename"
        );

        // Re-derived from ground truth on the next pass, now at its post-move path.
        daemon.reconcile_blocking().expect_clean("second reconcile");

        let recorded = operations.lock().expect("operations lock").clone();
        for relative in ["new-docs/fresh.txt", "new-docs/nested/deep.txt"] {
            assert!(
                recorded.contains(&RecordedOperation::Upload {
                    local_path: local_root.join(relative),
                    remote_root: PathBuf::from("/Drive/RemoteFolder"),
                    relative_path: PathBuf::from(relative),
                }),
                "{relative} uploads from its post-move path: {recorded:?}"
            );
        }
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
            &daemon.pair().connection,
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
            &daemon.pair().connection,
            &base_record("old-docs/report.txt", Some("report-id"), file_hash.as_str()),
        )
        .expect("nested file base record");

        daemon
            .reconcile_blocking()
            .expect_partial(1, "the later upload fails as an item, not as the pass");

        assert_eq!(
            daemon.pair().last_failed_items[0].path,
            PathBuf::from("will-fail.txt")
        );
        assert!(
            daemon.pair().last_failed_items[0]
                .error
                .contains("upload failed for will-fail.txt"),
            "unexpected failure: {:?}",
            daemon.pair().last_failed_items
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
            get_record(&daemon.pair().connection, Path::new("old-docs"))
                .expect("old directory index lookup")
                .is_none(),
            "a completed directory move must be checkpoint-committed despite the later failure"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("old-docs/report.txt"))
                .expect("old descendant index lookup")
                .is_none(),
            "descendant rewrites commit in the same checkpoint as their directory move"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("new-docs"))
                .expect("new directory index lookup")
                .is_some(),
            "the moved directory's new index row lands with the move's own checkpoint"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("new-docs/report.txt"))
                .expect("new descendant index lookup")
                .is_some(),
            "the moved descendant's new index row lands with the move's own checkpoint"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("will-fail.txt"))
                .expect("failed upload index lookup")
                .is_none(),
            "the failed action itself must never be recorded"
        );
        assert!(
            daemon.pair().last_sync.is_none(),
            "a partial pass must not count as a successful sync even with checkpoints committed"
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

        daemon
            .reconcile_blocking()
            .expect_partial(1, "the second upload fails as an item, not as the pass");

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
            get_record(&daemon.pair().connection, Path::new("first.txt"))
                .expect("first index lookup")
                .is_some(),
            "a completed upload must be checkpoint-committed even when a later action fails"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("second.txt"))
                .expect("second index lookup")
                .is_none(),
            "failed action must not be committed"
        );
        assert!(
            daemon.pair().last_sync.is_none(),
            "a partial pass must not advance last_sync"
        );
        assert_eq!(
            daemon.pair().last_error.as_deref(),
            Some("1 item(s) failed to sync (first: second.txt)"),
            "a partial pass still sets last_error, so every surface that reads it reports the \
             pass as a problem instead of drawing 'up to date' over it (#246)"
        );
        assert_eq!(
            daemon
                .pair()
                .last_plan_summary
                .as_ref()
                .map(|summary| summary.total),
            Some(2)
        );
        assert!(daemon.pair().last_successful_sync_summary.is_none());
        let status = daemon.status_response("daemon status");
        assert_eq!(
            status.last_error.as_deref(),
            Some("1 item(s) failed to sync (first: second.txt)")
        );
        assert_eq!(status.failed_item_count, 1);
        assert_eq!(
            status.failed_items,
            vec![FailedItem {
                path: PathBuf::from("second.txt"),
                action: SyncAction::Upload,
                error: "upload failed for second.txt".to_owned(),
            }],
            "the partial-failure state is reported as itself, per item"
        );
        assert_eq!(
            status
                .last_plan_summary
                .as_ref()
                .map(|summary| summary.total),
            Some(2)
        );
        assert!(status.last_successful_sync_summary.is_none());
        assert_eq!(
            status
                .status_history
                .last()
                .map(|entry| (entry.message.as_str(), entry.failed_item_count)),
            Some(("sync completed with 1 failed item(s)", 1)),
            "the history entry names the third state rather than either of the other two"
        );
    }

    #[test]
    fn a_local_deletion_goes_to_the_trash_by_default_and_still_purges_its_record() {
        // The default path. The file must leave the sync root (so the mirror is correct), be
        // findable afterwards (so the removed warnings are honest), and the baseline row must go
        // (so the deletion is not re-planned for ever).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let trash = directory.path().join("trash");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("removed-remotely.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(
            DaemonConfig {
                local_delete_mode: LocalDeleteMode::Trash,
                ..test_config(directory.path(), &local_root)
            },
            client,
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record(
                "removed-remotely.txt",
                Some("remote-id"),
                base_hash.as_str(),
            ),
        )
        .expect("base record");

        let _hook = crate::trash::test_hook::install_fake_trash(trash.clone());
        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(!local_path.exists(), "the file must leave the sync root");
        assert_eq!(
            fs::read(trash.join("removed-remotely.txt")).expect("read the trashed copy"),
            b"base content",
            "and must be recoverable, with its bytes intact"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("removed-remotely.txt"))
                .expect("index lookup")
                .is_none(),
            "a trashed deletion purges its baseline row exactly as an unlinked one does"
        );
        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "mirroring a remote deletion must not call Proton mutations"
        );
    }

    #[test]
    fn permanent_mode_unlinks_and_leaves_nothing_in_the_trash() {
        // The pre-change behaviour, asserted rather than assumed. `!path.exists()` alone cannot
        // tell the two modes apart — a trashed file also stops existing at its path — so the
        // discriminating assertion is that the trash was never asked.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let trash = directory.path().join("trash");
        fs::create_dir(&local_root).expect("local root");
        let local_path = local_root.join("gone-for-good.txt");
        fs::write(&local_path, b"base content").expect("local file");
        let base_hash = crate::index::compute_sha1(&local_path).expect("base hash");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(
            DaemonConfig {
                local_delete_mode: LocalDeleteMode::Permanent,
                ..test_config(directory.path(), &local_root)
            },
            client,
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("gone-for-good.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        let _hook = crate::trash::test_hook::install_fake_trash(trash.clone());
        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(!local_path.exists());
        assert!(
            !trash.join("gone-for-good.txt").exists(),
            "permanent mode must not quietly become recoverable"
        );
        assert_eq!(
            crate::trash::test_hook::calls(),
            vec![(
                LocalDeleteMode::Permanent,
                local_path,
                crate::index::EntityKind::File
            )],
            "the disposal is reached, and reached with the mode the pair configured"
        );
    }

    #[test]
    fn a_trashed_directory_deletion_moves_the_tree_and_purges_its_descendants() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let trash = directory.path().join("trash");
        fs::create_dir(&local_root).expect("local root");
        fs::create_dir_all(local_root.join("photos/2019")).expect("subtree");
        let inner = local_root.join("photos/2019/one.jpg");
        fs::write(&inner, b"one").expect("nested file");
        let inner_hash = crate::index::compute_sha1(&inner).expect("hash");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(
            DaemonConfig {
                local_delete_mode: LocalDeleteMode::Trash,
                ..test_config(directory.path(), &local_root)
            },
            client,
        )
        .expect("daemon");
        for record in [
            FileRecord {
                entity_kind: crate::index::EntityKind::Directory,
                ..base_record("photos", Some("remote-photos"), "")
            },
            FileRecord {
                entity_kind: crate::index::EntityKind::Directory,
                ..base_record("photos/2019", Some("remote-2019"), "")
            },
            base_record(
                "photos/2019/one.jpg",
                Some("remote-one"),
                inner_hash.as_str(),
            ),
        ] {
            upsert_record(&daemon.pair().connection, &record).expect("base record");
        }

        let _hook = crate::trash::test_hook::install_fake_trash(trash.clone());
        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            !local_root.join("photos").exists(),
            "the tree leaves the root"
        );
        assert_eq!(
            fs::read(trash.join("photos/2019/one.jpg")).expect("read the nested trashed file"),
            b"one",
            "the subtree travels INSIDE the folder — one move, not a flattened per-file sweep"
        );
        for path in ["photos", "photos/2019", "photos/2019/one.jpg"] {
            assert!(
                get_record(&daemon.pair().connection, Path::new(path))
                    .expect("index lookup")
                    .is_none(),
                "{path} must be purged with the directory that held it"
            );
        }
    }

    #[test]
    fn a_failing_trash_move_keeps_the_file_and_does_not_unlink_it_instead() {
        // DESIGN D4, AND THE LOAD-BEARING ONE. Every warning this change removes rests on the
        // failure mode being "the file is still there". A fallback to `remove_file` would make the
        // trash a courtesy and the permanent removal the real behaviour — with a one-click
        // approval in front of it, because the card would be drawn as recoverable.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let doomed = local_root.join("doomed.txt");
        fs::write(&doomed, b"must survive").expect("local file");
        let doomed_hash = crate::index::compute_sha1(&doomed).expect("hash");
        // A successor in the same plan, so "the rest of the pass still runs" is observable.
        fs::write(local_root.join("fresh.txt"), b"fresh").expect("local file");
        let (client, operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(
            DaemonConfig {
                local_delete_mode: LocalDeleteMode::Trash,
                ..test_config(directory.path(), &local_root)
            },
            client,
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("doomed.txt", Some("remote-id"), doomed_hash.as_str()),
        )
        .expect("base record");

        let _hook = crate::trash::test_hook::install_failing_trash();
        daemon.reconcile_blocking().expect_partial(
            1,
            "a trash that refuses is one failed item, not a failed pass",
        );

        assert!(
            doomed.exists(),
            "the entity must survive a failed trash move"
        );
        assert_eq!(fs::read(&doomed).expect("read"), b"must survive");
        assert!(
            get_record(&daemon.pair().connection, Path::new("doomed.txt"))
                .expect("index lookup")
                .is_some(),
            "the baseline row must survive too, or the deletion is never re-planned"
        );
        assert_eq!(daemon.pair().last_failed_item_count, 1);
        assert_eq!(
            daemon.pair().last_failed_items[0].path,
            PathBuf::from("doomed.txt")
        );
        assert!(
            daemon
                .pair()
                .pending_changes
                .contains(Path::new("doomed.txt")),
            "the failed path is re-queued so an events-driven pass cannot idle-skip it"
        );
        let uploaded: Vec<PathBuf> = operations
            .lock()
            .expect("operations lock")
            .iter()
            .filter_map(|operation| match operation {
                RecordedOperation::Upload { relative_path, .. } => Some(relative_path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            uploaded,
            vec![PathBuf::from("fresh.txt")],
            "the actions ordered after the failed deletion must still run"
        );
    }

    #[test]
    fn a_failing_trash_move_holds_the_event_cursor() {
        // The other half of D4: the deletion has to come back. An advanced cursor would drop its
        // event out of the next delta, and an incremental pass's remote map IS
        // `reconstruct_remote(base ⊕ delta)` — so nothing would ever re-derive it. The file would
        // sit on disk, un-mirrored, with the deletion neither done nor pending.
        //
        // The same rule a WITHHELD deletion gets (`a_withheld_local_delete_holds_the_event_cursor`)
        // and a vanished node gets: one policy, now three causes.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let base = sha1_bytes(b"base");
        let doomed = local_root.join("a.txt");
        fs::write(&doomed, b"base").expect("local file");

        // A remote delete event for the baseline node → the planner derives a LocalDelete. The
        // guard is OFF here, unlike the withheld-deletion test: this one is about a deletion that
        // was allowed to go ahead and could not.
        let client = EventFakeClient::new(HashMap::new());
        let full_walks = Arc::clone(&client.full_walks);
        let page = one_page(
            "cursor-1",
            vec![change(RemoteChangeKind::Deleted, "na", None, false)],
        );
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                local_delete_mode: LocalDeleteMode::Trash,
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
            &daemon.pair().connection,
            &base_record("a.txt", Some("vol~na"), base.as_str()),
        )
        .expect("seed base record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;

        let _hook = crate::trash::test_hook::install_failing_trash();
        daemon
            .reconcile_blocking()
            .expect_partial(1, "the refused trash move is one failed item");

        assert_eq!(full_walks.load(Ordering::SeqCst), 0, "stayed incremental");
        assert!(
            doomed.exists(),
            "the entity survives a trash that refused it"
        );
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "the cursor must NOT advance past a deletion that did not land"
        );
    }

    #[test]
    fn a_failing_action_no_longer_starves_the_actions_ordered_after_it() {
        // #136. The failing file sorts FIRST, which is exactly the incident shape: a fail-fast
        // executor attempted 0 of the remaining 2 uploads, every pass, forever.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        for name in ["a-poisoned.txt", "b.txt", "c.txt"] {
            fs::write(local_root.join(name), name.as_bytes()).expect("local file");
        }
        let (client, operations) = RecordingProtonClient::with_failed_uploads(
            HashMap::new(),
            BTreeSet::from([PathBuf::from("a-poisoned.txt")]),
        );
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_partial(1, "one poisoned item, two good ones");

        let uploaded: Vec<PathBuf> = operations
            .lock()
            .expect("operations lock")
            .iter()
            .filter_map(|operation| match operation {
                RecordedOperation::Upload { relative_path, .. } => Some(relative_path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            uploaded,
            vec![
                PathBuf::from("a-poisoned.txt"),
                PathBuf::from("b.txt"),
                PathBuf::from("c.txt"),
            ],
            "every successor of the failing action must still be attempted, in plan order"
        );
        for name in ["b.txt", "c.txt"] {
            assert!(
                get_record(&daemon.pair().connection, Path::new(name))
                    .expect("index lookup")
                    .is_some(),
                "{name} must be checkpoint-committed despite the earlier failure"
            );
        }
        assert!(
            get_record(&daemon.pair().connection, Path::new("a-poisoned.txt"))
                .expect("index lookup")
                .is_none(),
            "the failed action itself is never recorded (it re-plans next pass)"
        );
        assert_eq!(daemon.pair().last_failed_item_count, 1);
        assert_eq!(
            daemon.pair().last_failed_items[0].path,
            PathBuf::from("a-poisoned.txt")
        );
        assert!(
            daemon
                .pair()
                .pending_changes
                .contains(Path::new("a-poisoned.txt")),
            "the failed path is re-queued for the next pass"
        );
    }

    #[test]
    fn a_systemically_failing_pass_is_abandoned_rather_than_ground_through() {
        // The breaker: nothing has succeeded and the failures keep coming (an expired session, a
        // missing CLI), so the remaining actions are doomed round trips rather than per-item
        // problems. Distinct from #136's poisoned file, which has successes around it.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let doomed = CONSECUTIVE_FAILURE_LIMIT + 5;
        let mut failing = BTreeSet::new();
        for index in 0..doomed {
            let name = format!("file-{index:03}.txt");
            fs::write(local_root.join(&name), b"x").expect("local file");
            failing.insert(PathBuf::from(&name));
        }
        let (client, operations) =
            RecordingProtonClient::with_failed_uploads(HashMap::new(), failing);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        let error = daemon
            .reconcile_blocking()
            .expect_err("a pass that accomplishes nothing must be abandoned");

        assert!(
            error.to_string().contains("looks systemic"),
            "unexpected error: {error}"
        );
        let attempts = operations.lock().expect("operations lock").len();
        assert_eq!(
            attempts, CONSECUTIVE_FAILURE_LIMIT,
            "the breaker must stop at the limit rather than attempt all {doomed}"
        );
    }

    #[test]
    fn a_shutdown_stops_the_pass_instead_of_failing_every_remaining_action() {
        // Shutdown beats failure tolerance: the cancel flag makes every in-flight command fail, so
        // continuing past those failures would spawn and kill one CLI child per remaining action.
        struct CancellingClient {
            cancel: Arc<AtomicBool>,
            uploads: Arc<Mutex<Vec<PathBuf>>>,
        }
        impl ProtonClient for CancellingClient {
            fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
                Ok(HashMap::new())
            }
            fn ensure_directory(&self, _root: &Path, _relative: &Path) -> AppResult<()> {
                Ok(())
            }
            fn upload(&self, _local: &Path, _root: &Path, relative: &Path) -> AppResult<()> {
                self.uploads
                    .lock()
                    .expect("uploads lock")
                    .push(relative.to_path_buf());
                // The shutdown lands during this transfer, which is how the real client reports a
                // cancellation: the command it was running fails.
                self.cancel.store(true, Ordering::SeqCst);
                Err(boxed_error(
                    "proton-drive upload cancelled before completion",
                ))
            }
            fn download(&self, _remote: &Path, _destination: &Path) -> AppResult<()> {
                Ok(())
            }
            fn delete(&self, _remote: &Path) -> AppResult<()> {
                Ok(())
            }
        }

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(local_root.join(name), b"x").expect("local file");
        }
        let uploads = Arc::new(Mutex::new(Vec::new()));
        let mut daemon = Daemon::with_client(
            test_config(directory.path(), &local_root),
            CancellingClient {
                cancel: Arc::new(AtomicBool::new(false)),
                uploads: Arc::clone(&uploads),
            },
        )
        .expect("daemon");
        daemon.cancel_flag = Arc::clone(&daemon.proton.cancel);

        let error = daemon
            .reconcile_blocking()
            .expect_err("a cancelled pass is a failed pass, not a partial one");

        assert!(
            error.to_string().contains("cancelled"),
            "unexpected error: {error}"
        );
        assert_eq!(
            uploads.lock().expect("uploads lock").len(),
            1,
            "the pass must stop dead on the cancellation, not attempt the remaining actions"
        );
        assert_eq!(
            daemon.pair().last_failed_item_count,
            0,
            "a cancellation is not the item's failure and is not reported as one"
        );
    }

    #[test]
    fn a_reported_failure_is_bounded_in_both_count_and_message_length() {
        assert_eq!(truncate_error("short"), "short");
        let long = "e".repeat(FAILED_ITEM_ERROR_LIMIT * 3);
        let truncated = truncate_error(&long);
        assert!(
            truncated.len() <= FAILED_ITEM_ERROR_LIMIT,
            "the cap includes the ellipsis, so the documented bound is the real one"
        );
        assert!(truncated.ends_with('…'));
        // A multi-byte char straddling the limit must not panic or split.
        let multibyte = "é".repeat(FAILED_ITEM_ERROR_LIMIT);
        let truncated = truncate_error(&multibyte);
        assert!(truncated.len() <= FAILED_ITEM_ERROR_LIMIT);
        assert!(truncated.ends_with('…'));

        // The cap is the promise, at EVERY limit — including one too small to hold the ellipsis,
        // where the marker gives way rather than the bound. Unreachable from the two named limits,
        // but this is the shared primitive and its doc states the rule without qualification.
        for limit in 0..=ELLIPSIS.len() {
            let cut = truncate_for_display("éàü-and-more", limit);
            assert!(
                cut.len() <= limit,
                "limit {limit} produced {} bytes: {cut:?}",
                cut.len()
            );
        }
        assert_eq!(truncate_for_display("abcdef", 2), "ab");
        assert_eq!(truncate_for_display("abcdef", 4), "a…");

        let mut failures = PassFailures::default();
        for index in 0..(FAILED_ITEMS_REPORTED + 7) {
            let action = PlannedAction::new(
                &PathBuf::from(format!("file-{index}.txt")),
                SyncAction::Upload,
                EntityKind::File,
                None,
            );
            failures.record(&action, boxed_error("boom").as_ref());
        }
        assert_eq!(failures.items.len(), FAILED_ITEMS_REPORTED);
        assert_eq!(failures.count, FAILED_ITEMS_REPORTED + 7);
        assert_eq!(
            failures.outcome(),
            PassOutcome::Partial {
                failed: FAILED_ITEMS_REPORTED + 7
            }
        );
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

        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");

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
                get_record(&daemon.pair().connection, Path::new(path))
                    .expect("index lookup")
                    .is_some(),
                "batched download must record {path} in the index"
            );
        }
        assert!(daemon.pair().last_sync.is_some(), "the pass succeeded");
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
                &daemon.pair().connection,
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

        daemon
            .reconcile_blocking()
            .expect_clean("ongoing reconcile");

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
    fn an_unmakeable_destination_directory_fails_only_its_own_group_of_downloads() {
        // The batched path's own `ensure_parent_directory` used to be pass-fatal, so the same
        // local-FS problem behaved differently at `download_batch_size = 1` (where it is the
        // item's failure) — and a persistent one starved every download ordered after it (#136).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        // A regular file squatting where `docs/` must be: its group's mkdir cannot succeed.
        fs::write(local_root.join("docs"), b"not a directory").expect("squatter");

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
            ("media/d.txt", "id-d", "hd"),
        ] {
            remote_entities.insert(
                PathBuf::from(path),
                RemoteEntity::File(remote(path, id, Some(hash))),
            );
        }
        let (client, _operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let config = DaemonConfig {
            download_batch_size: 2,
            ..test_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client(config, client).expect("daemon");

        daemon.reconcile_blocking().expect_partial(
            2,
            "both files under the unmakeable directory fail, and only those",
        );

        assert!(
            local_root.join("media/d.txt").is_file(),
            "a later group's download must still land"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("media/d.txt"))
                .expect("index lookup")
                .is_some(),
            "and must be recorded"
        );
        for path in ["docs/a.txt", "docs/b.txt"] {
            assert!(
                get_record(&daemon.pair().connection, Path::new(path))
                    .expect("index lookup")
                    .is_none(),
                "{path} never landed, so it must not be recorded"
            );
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

        daemon
            .reconcile_blocking()
            .expect_partial(1, "a failed file in the batch costs only itself");
        assert_eq!(
            daemon.pair().last_failed_items[0].path,
            PathBuf::from("docs/b.txt")
        );

        // The chunk's survivor was checkpoint-committed before the pass failed...
        assert!(
            local_root.join("docs/a.txt").is_file(),
            "the successfully downloaded file stays on disk"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/a.txt"))
                .expect("survivor index lookup")
                .is_some(),
            "a completed download in a failing chunk must be checkpoint-committed"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs"))
                .expect("directory index lookup")
                .is_some(),
            "the earlier CreateLocalDirectory checkpoint must also survive"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/b.txt"))
                .expect("failed index lookup")
                .is_none(),
            "the failed file must never be recorded"
        );
        // ...but the pass-level outcomes are those of a pass that did not land everything: no
        // cursor, no last_sync.
        assert!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("load cursor")
                .is_none(),
            "a partial pass must never advance the event cursor, checkpoints notwithstanding"
        );
        assert!(daemon.pair().last_sync.is_none());
    }

    #[test]
    fn a_vanished_node_skips_its_download_and_the_rest_of_the_pass_still_runs() {
        // Issue #31: the node was in the listing this pass planned against, but another client
        // trashed it before the transfer ran. Only that action is skipped; every action ordered
        // after it still executes and commits, and the vanished node is NOT recorded.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("z.txt"), b"local only").expect("local file");

        let mut remote_entities = HashMap::new();
        // `a.txt` sorts first, so the download of `b.txt` and the upload of `z.txt` are both
        // ordered AFTER the vanished node.
        remote_entities.insert(
            PathBuf::from("a.txt"),
            RemoteEntity::File(remote("a.txt", "vol~na", Some("ha"))),
        );
        remote_entities.insert(
            PathBuf::from("b.txt"),
            RemoteEntity::File(remote("b.txt", "vol~nb", Some("hb"))),
        );
        let (mut client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        client.vanished_downloads = BTreeSet::from([PathBuf::from("/Drive/RemoteFolder/a.txt")]);
        let config = DaemonConfig {
            // 1 disables batching, so this drives the single-file `Download` arm.
            download_batch_size: 1,
            ..event_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-1"))),
        )
        .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_clean("a vanished node must not fail the whole pass");

        assert!(
            operations
                .lock()
                .expect("operations lock")
                .iter()
                .any(|operation| matches!(
                    operation,
                    RecordedOperation::Download { remote_path, .. }
                        if remote_path == Path::new("/Drive/RemoteFolder/a.txt")
                )),
            "the vanished node's download was still attempted"
        );
        assert!(
            !local_root.join("a.txt").exists(),
            "nothing may be written for the vanished node"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("a.txt"))
                .expect("vanished index lookup")
                .is_none(),
            "a skipped node must never be recorded as if it had synced"
        );
        assert!(
            local_root.join("b.txt").is_file()
                && get_record(&daemon.pair().connection, Path::new("b.txt"))
                    .expect("survivor index lookup")
                    .is_some(),
            "the download ordered after the vanished node still ran and committed"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("z.txt"))
                .expect("upload index lookup")
                .is_some(),
            "the upload ordered after the vanished node still ran and committed"
        );
        // The disappearance is unconfirmed (it may equally be a transient listing error), so the
        // cursor is held exactly as it is for a withheld delete: next pass re-derives from
        // ground truth.
        assert!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("load cursor")
                .is_none(),
            "a pass that skipped a vanished node must not advance the event cursor"
        );
        assert!(
            daemon.pair().last_sync.is_some(),
            "everything executable executed, so the pass succeeded"
        );
    }

    #[test]
    fn a_vanished_node_in_a_batch_does_not_fail_the_rest_of_its_chunk() {
        // Issue #31, batched path: one trashed node among 3 in a single `download_many` chunk
        // must not cost the other 2 — they land, are checkpoint-committed, and the pass survives.
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
        for (path, id, hash) in [
            ("docs/a.txt", "vol~na", "ha"),
            ("docs/b.txt", "vol~nb", "hb"),
            ("docs/c.txt", "vol~nc", "hc"),
        ] {
            remote_entities.insert(
                PathBuf::from(path),
                RemoteEntity::File(remote(path, id, Some(hash))),
            );
        }
        let (mut client, operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        client.vanished_downloads =
            BTreeSet::from([PathBuf::from("/Drive/RemoteFolder/docs/b.txt")]);
        let config = DaemonConfig {
            download_batch_size: 3,
            ..event_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            client,
            Some(Box::new(FakeEventSource::new("cursor-1"))),
        )
        .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_clean("a vanished node in a batch must not fail the pass");

        assert_eq!(
            *operations.lock().expect("operations lock"),
            vec![RecordedOperation::DownloadBatch {
                remote_paths: vec![
                    PathBuf::from("/Drive/RemoteFolder/docs/a.txt"),
                    PathBuf::from("/Drive/RemoteFolder/docs/b.txt"),
                    PathBuf::from("/Drive/RemoteFolder/docs/c.txt"),
                ],
            }],
            "the whole chunk is still issued as one invocation"
        );
        for path in ["docs/a.txt", "docs/c.txt"] {
            assert!(
                local_root.join(path).is_file(),
                "the chunk's survivor {path} must still land"
            );
            assert!(
                get_record(&daemon.pair().connection, Path::new(path))
                    .expect("survivor index lookup")
                    .is_some(),
                "the chunk's survivor {path} must be checkpoint-committed"
            );
        }
        assert!(
            !local_root.join("docs/b.txt").exists(),
            "nothing may be written for the vanished node"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/b.txt"))
                .expect("vanished index lookup")
                .is_none(),
            "a skipped node must never be recorded as if it had synced"
        );
        assert!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("load cursor")
                .is_none(),
            "a pass that skipped a vanished node must not advance the event cursor"
        );
        assert!(daemon.pair().last_sync.is_some(), "the pass succeeded");
    }

    #[test]
    fn a_vanished_conflict_sidecar_skips_the_whole_action_including_its_record() {
        // Issue #31 at the OTHER download site: the conflicting remote revision is trashed before
        // the sidecar download runs. The `Conflict` record must be skipped along with the
        // download — recording a conflict whose sidecar was never written leaves it with no exit
        // and invisible to the GUI's disk-walking conflicts list (#46). Next pass the node is no
        // longer listed, so the planner reaches `(Changed, Missing)` and copies the local file in.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("notes.txt"), b"local changed").expect("local file");
        // Sorts after `notes.txt`, so its upload is ordered after the skipped conflict.
        fs::write(local_root.join("z.txt"), b"local only").expect("local file");

        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("notes.txt"),
            RemoteEntity::File(remote("notes.txt", "vol~n1", Some("remote-changed"))),
        );
        let (mut client, _operations) =
            RecordingProtonClient::with_remote_entities(remote_entities);
        client.vanished_downloads =
            BTreeSet::from([PathBuf::from("/Drive/RemoteFolder/notes.txt")]);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-1"))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("notes.txt", Some("vol~n1"), "base-hash"),
        )
        .expect("base record");

        daemon
            .reconcile_blocking()
            .expect_clean("a vanished conflict revision must not fail the whole pass");

        assert!(
            !local_root.join("notes.proton-cloud.txt").exists(),
            "no sidecar may be written for the vanished revision"
        );
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("the base record is untouched, not removed");
        assert_eq!(
            record.sync_status,
            SyncStatus::Synced,
            "a conflict with no sidecar has no exit, so the record is not written either"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("z.txt"))
                .expect("upload index lookup")
                .is_some(),
            "the upload ordered after the skipped conflict still ran and committed"
        );
        assert!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("load cursor")
                .is_none(),
            "a pass that skipped a vanished node must not advance the event cursor"
        );
        assert!(daemon.pair().last_sync.is_some(), "the pass succeeded");
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

        daemon
            .reconcile_blocking()
            .expect_partial(1, "an unrecordable batch item fails alone");
        assert!(
            daemon.pair().last_failed_items[0]
                .error
                .contains("could not be recorded"),
            "unexpected failure: {:?}",
            daemon.pair().last_failed_items
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/a.txt"))
                .expect("survivor index lookup")
                .is_some(),
            "the chunk's other survivor must still be checkpoint-committed"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("docs/b.txt"))
                .expect("failed index lookup")
                .is_none(),
            "the unrecordable item must not be recorded"
        );
        assert!(daemon.pair().last_sync.is_none());
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
            &daemon.pair().connection,
            &base_record("removed.txt", Some("removed-id"), "same-hash"),
        )
        .expect("base record");

        // Pass 1: the remote delete is withheld pending approval; the upload then fails as an
        // item. The pending deletion is still published.
        daemon
            .reconcile_blocking()
            .expect_partial(1, "the failing upload is this pass's one failed item");
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        let pending = daemon.pair().pending_deletions[0].clone();
        upsert_delete_approval(
            &daemon.pair().connection,
            &pending.path,
            pending.direction,
            &pending.fingerprint,
            1,
        )
        .expect("approve");

        // Pass 2: the approved delete executes and checkpoints — its index purge and the
        // approval consumption commit together — and the later upload still fails as an item.
        daemon
            .reconcile_blocking()
            .expect_partial(1, "the failing upload still fails as an item");
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
            get_record(&daemon.pair().connection, Path::new("removed.txt"))
                .expect("record lookup")
                .is_none(),
            "the executed delete's index purge must be checkpoint-committed despite the failure"
        );
        assert!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("load approvals")
                .is_empty(),
            "the consumed approval must not survive the partial pass"
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

        assert_eq!(daemon.pair().last_error.as_deref(), Some("list failed"));
        assert!(daemon.pair().last_sync.is_none());
        assert!(daemon.pair().last_plan_summary.is_none());
        assert!(daemon.pair().last_successful_sync_summary.is_none());
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
        let mut daemon = Daemon::with_client(config, client).expect("daemon");

        let status = daemon.status_response("daemon status");
        let info = status
            .config
            .expect("status must report the running config");
        assert_eq!(info.local_root, local_root);
        assert_eq!(info.remote_root, expected_remote);
        assert_eq!(info.db_path, expected_db);
    }

    #[test]
    fn the_status_reply_carries_index_totals_without_querying_the_index() {
        // #207. Two claims, and the second is the load-bearing one:
        //   1. a pass publishes the corpus size, files only;
        //   2. the reply is served from the published snapshot, so it is unaffected by the index
        //      the daemon actually holds — which is what "O(1) status" MEANS here. Emptying the
        //      table and re-asking proves the reply never reaches SQLite: a per-reply
        //      COUNT(*)/SUM() would now answer 0.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("a.txt"), b"12345").expect("write");
        fs::write(local_root.join("b.txt"), b"1234567890").expect("write");
        fs::create_dir(local_root.join("sub")).expect("mkdir");
        fs::write(local_root.join("sub/c.txt"), b"xyz").expect("write");

        let config = test_config(directory.path(), &local_root);
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(config, client).expect("daemon");
        let _ = daemon.reconcile_blocking().expect("reconcile");

        let totals = daemon
            .status_response("daemon status")
            .index_totals
            .expect("a pass must publish the corpus size");
        assert_eq!(
            totals.files, 3,
            "three files; `sub/` is a row in file_index but is not a file"
        );
        assert_eq!(totals.bytes, 18);

        daemon
            .pair()
            .connection
            .execute("DELETE FROM file_index", [])
            .expect("empty the index");
        assert_eq!(
            daemon.status_response("daemon status").index_totals,
            Some(totals),
            "the reply is a snapshot read, not a query"
        );
    }

    #[test]
    fn an_idle_pass_does_not_recompute_the_index_totals() {
        // The cache invariant: only a planned action can mutate the index, so an empty plan must
        // leave the totals alone. Proven by planting a sentinel and running an idle pass — a pass
        // that recomputed would overwrite it.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("a.txt"), b"12345").expect("write");

        // The remote already holds the same bytes, so the first pass adopts it (`AutoLink`) and the
        // second has nothing at all to plan — a genuine idle pass, which is the steady state under
        // event-driven reconcile and therefore the case that must stay free of the query.
        let remote_files = HashMap::from([(
            PathBuf::from("a.txt"),
            RemoteFile {
                path: PathBuf::from("a.txt"),
                id: "remote-a".to_owned(),
                name: "a.txt".to_owned(),
                sha1_hash: Some(
                    crate::index::compute_sha1(&local_root.join("a.txt")).expect("sha1"),
                ),
                downloadable: true,
            },
        )]);

        let config = test_config(directory.path(), &local_root);
        let (client, _) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(config, client).expect("daemon");
        let _ = daemon.reconcile_blocking().expect("first pass");
        assert_eq!(
            daemon.pair().index_totals,
            Some(IndexTotals { files: 1, bytes: 5 })
        );

        let sentinel = IndexTotals {
            files: 42,
            bytes: 42,
        };
        daemon.pair_mut().index_totals = Some(sentinel);
        let _ = daemon.reconcile_blocking().expect("idle pass");
        assert_eq!(
            daemon.pair().index_totals,
            Some(sentinel),
            "an idle pass plans nothing, so it must not spend the aggregate query"
        );
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

            daemon.reconcile_blocking().expect_clean("reconcile");

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
        let mut daemon = Daemon::with_client(config, client).expect("restarted daemon");

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
        .expect("preview plan")
        .plan;

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
        .expect("preview plan")
        .plan;

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

        daemon.reconcile_blocking().expect_clean("reconcile");

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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        assert!(
            daemon
                .pair()
                .config
                .conflict_naming
                .is_conflict_copy(&sidecar_path)
        );
        assert_eq!(
            daemon
                .pair()
                .config
                .conflict_naming
                .original_from_conflict_copy(&sidecar_path),
            Some(local_path.clone())
        );
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));

        let operations_after_first = operations.lock().expect("operations lock").len();
        daemon.reconcile_blocking().expect_clean("second reconcile");
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("first reconcile");

        let sidecar_path = local_root.join("notes.proton-cloud.txt");
        fs::remove_file(&sidecar_path).expect("resolve by removing the sidecar");
        daemon
            .handle_fs_event(
                Event::new(EventKind::Remove(RemoveKind::File)).add_path(sidecar_path.clone()),
            )
            .expect("handle sidecar remove event");

        daemon.reconcile_blocking().expect_clean("second reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("first reconcile");

        fs::write(&local_path, b"second local edit").expect("edit the original again");
        daemon
            .handle_fs_event(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
                    .add_path(local_path.clone()),
            )
            .expect("handle local edit event");

        daemon.reconcile_blocking().expect_clean("second reconcile");

        assert_eq!(
            fs::read(local_root.join("notes.proton-cloud.txt")).expect("sidecar"),
            b"local edit",
            "the existing sidecar must survive a re-planned conflict untouched"
        );
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
        daemon.reconcile_blocking().expect_clean("second reconcile");
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert_eq!(
            fs::read(&sidecar_path).expect("sidecar"),
            b"the user's own copy",
            "an existing sidecar file must never be overwritten"
        );
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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
            &daemon.pair().connection,
            &directory_record("docs", Some("dir-id")),
        )
        .expect("stale directory record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert_eq!(
            fs::read(local_root.join("docs")).expect("downloaded file"),
            content,
            "the surviving remote file must be downloaded over the stale directory record"
        );
        let record = get_record(&daemon.pair().connection, Path::new("docs"))
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
        daemon.reconcile_blocking().expect_clean("second reconcile");
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "stale-base-hash"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            operations.lock().expect("operations lock").is_empty(),
            "identical local and remote content must auto-link without any network \
             transfer or sidecar download"
        );
        assert!(
            !local_root.join("notes.proton-cloud.txt").exists(),
            "no spurious conflict sidecar must be created when both sides already agree"
        );
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), "base-hash-a"),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("first reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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

        daemon.reconcile_blocking().expect_clean("second reconcile");

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
            daemon.pair().pending_changes.is_empty(),
            "downloaded conflict sidecars must not be queued as local creations"
        );
        assert!(
            get_record(
                &daemon.pair().connection,
                Path::new("notes.proton-cloud.txt")
            )
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
        shared.pair.syncing.store(true, Ordering::SeqCst);
        shared.pair.paused.store(true, Ordering::SeqCst);
        let mid_pass = shared.response("m");
        assert_eq!(mid_pass.status, "syncing");
        assert!(mid_pass.paused);
        assert!(mid_pass.syncing);

        shared.pair.syncing.store(false, Ordering::SeqCst);
        assert_eq!(shared.response("m").status, "paused");

        shared.pair.paused.store(false, Ordering::SeqCst);
        assert_eq!(shared.response("m").status, "running");
    }

    fn activity_test_shared() -> ControlShared {
        ControlShared::new(RunningConfigInfo {
            local_root: PathBuf::from("/local"),
            remote_root: PathBuf::from("/remote"),
            db_path: PathBuf::from("/db"),
        })
    }

    /// A planned action, for the window tests below. Only the fields the window reads.
    fn planned(action: SyncAction, path: &str) -> PlannedAction {
        PlannedAction::new(Path::new(path), action, EntityKind::File, None)
    }

    #[test]
    fn the_transfer_window_is_bounded_and_the_remainder_counts_past_it() {
        // Eight transfers with a delete in the middle, so the window has to skip a non-transfer
        // action rather than assume the plan is transfers all the way down.
        let mut plan: Vec<PlannedAction> = (0..4)
            .map(|i| planned(SyncAction::Upload, &format!("up/{i}.txt")))
            .collect();
        plan.push(planned(SyncAction::LocalDelete, "gone.txt"));
        plan.extend((0..4).map(|i| planned(SyncAction::Download, &format!("down/{i}.txt"))));
        let positions: Vec<usize> = plan
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                matches!(action.action, SyncAction::Upload | SyncAction::Download)
            })
            .map(|(position, _)| position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 5, 6, 7, 8]);

        let mut local_files = HashMap::new();
        local_files.insert(
            PathBuf::from("up/1.txt"),
            LocalFileState {
                relative_path: PathBuf::from("up/1.txt"),
                absolute_path: PathBuf::from("/local/up/1.txt"),
                file_size: 4096,
                mtime: 0,
                sha1_hash: "x".to_owned(),
            },
        );
        let queue = TransferQueue {
            plan: &plan,
            positions: &positions,
            local_files: &local_files,
        };
        assert_eq!(queue.total(), 8);

        // One transfer started: itself, then five queued rows fill the six-row window, and the
        // remainder counts the two the window cannot name plus the one running.
        let active = TransferActivity::active("upload", PathBuf::from("up/0.txt"), Some(10), 9);
        let (window, remaining) = queue.window(1, vec![active]);
        assert_eq!(window.len(), TRANSFERS_REPORTED);
        assert_eq!(remaining, 8, "7 not yet started + 1 in flight");
        assert!(window[0].is_active());
        assert!(window[1..].iter().all(|row| !row.is_active()));
        assert_eq!(
            window
                .iter()
                .map(|row| row.path.display().to_string())
                .collect::<Vec<_>>(),
            vec![
                "up/0.txt",
                "up/1.txt",
                "up/2.txt",
                "up/3.txt",
                "down/0.txt",
                "down/1.txt"
            ],
            "plan order, with the delete stepped over rather than counted"
        );
        // A queued UPLOAD carries the size the scan measured; a queued download cannot.
        assert_eq!(window[1].bytes_total, Some(4096));
        assert_eq!(window[4].bytes_total, None);
        assert_eq!(window[4].direction, "download");
        assert_eq!(
            window[1].started_epoch_secs, None,
            "a queued row has not started"
        );

        // A batched row stands for its whole chunk, so the remainder counts its files and not the
        // single row: 3 not yet started + 5 in flight.
        let chunk = TransferActivity {
            files: Some(5),
            ..TransferActivity::active("download", PathBuf::from("down"), None, 9)
        };
        let (_, remaining) = queue.window(5, vec![chunk]);
        assert_eq!(remaining, 8);

        // The last transfer running: nothing queued, and the remainder is just it.
        let last = TransferActivity::active("download", PathBuf::from("down/3.txt"), None, 9);
        let (window, remaining) = queue.window(8, vec![last]);
        assert_eq!(window.len(), 1);
        assert_eq!(remaining, 1);

        // Between two transfers — a delete executing — the window is queued rows only. It is NOT
        // empty, which is the whole point: an empty list mid-pass would read as "nothing is
        // moving".
        let (window, remaining) = queue.window(4, Vec::new());
        assert_eq!(window.len(), 4);
        assert!(window.iter().all(|row| !row.is_active()));
        assert_eq!(remaining, 4);
    }

    #[test]
    fn an_executing_activity_always_carries_its_transfer_window() {
        // Found by review, on the batched-download path: the phase was published first and the
        // window filled by a second `update_activity`, so a status poll landing between the two
        // lock acquisitions read `executing` with an empty list and `transfers_remaining: None` —
        // the value that field documents as "not executing at all" — and blinked the rows.
        //
        // `executing_activity` is now the only way to build the phase, so the window rides in the
        // same publish. This asserts the property that makes it worth having: every state a client
        // can observe during `executing` says how many transfers are left.
        let plan = vec![
            planned(SyncAction::Upload, "a.txt"),
            planned(SyncAction::LocalDelete, "b.txt"),
        ];
        let positions = vec![0usize];
        let local_files = HashMap::new();
        let queue = TransferQueue {
            plan: &plan,
            positions: &positions,
            local_files: &local_files,
        };

        let shared = activity_test_shared();
        shared.pair.syncing.store(true, Ordering::SeqCst);
        for (index, window) in [
            (0, queue.window(0, Vec::new())),
            (
                0,
                queue.window(
                    1,
                    vec![TransferActivity::active(
                        "upload",
                        PathBuf::from("a.txt"),
                        None,
                        1,
                    )],
                ),
            ),
            // The batched arm: one row standing for a whole chunk.
            (
                1,
                queue.window(
                    1,
                    vec![TransferActivity {
                        files: Some(4),
                        ..TransferActivity::active("download", PathBuf::from("d"), None, 1)
                    }],
                ),
            ),
        ] {
            shared.begin_activity(executing_activity("x".to_owned(), index, 2, window));
            let activity = shared.activity_for_response().expect("activity");
            assert_eq!(activity.phase, PHASE_EXECUTING);
            assert!(
                activity.transfers_remaining.is_some(),
                "an executing activity that says `None` is indistinguishable from an idle one"
            );
            // …and the other half of the same rule: a positive remainder always names rows.
            if activity.transfers_remaining > Some(0) {
                assert!(!activity.transfers.is_empty());
            }
        }
    }

    #[test]
    fn per_direction_progress_folds_only_what_actually_crossed_the_network() {
        let shared = activity_test_shared();
        shared.begin_pass_progress(100, PassKind::Incremental);
        shared.begin_activity(new_activity(PHASE_EXECUTING));
        let mut log = PassLog::new(100);

        log.note(SyncAction::Upload, Path::new("a.txt"), Some(30));
        log.note(SyncAction::Download, Path::new("b.txt"), Some(70));
        // A conflict sidecar fetched from the remote IS a download.
        log.note(SyncAction::Conflict, Path::new("c.txt"), Some(5));
        // …one written from the local copy crossed nothing, and is in neither count. Reporting it
        // as received would print a file count beside zero bytes for it.
        log.note(SyncAction::Conflict, Path::new("d.txt"), None);
        // Actions that move no bytes at all are in neither, direction or not.
        log.note(SyncAction::LocalDelete, Path::new("e.txt"), None);
        log.note(SyncAction::CreateLocalDirectory, Path::new("f"), None);
        log.note_committed(None, &shared);

        let pass = shared
            .activity_for_response()
            .and_then(|activity| activity.pass)
            .expect("the pass block carries the counters");
        assert_eq!(pass.uploaded_files, 1);
        assert_eq!(pass.downloaded_files, 2);
        assert_eq!(pass.uploaded_bytes, 30);
        assert_eq!(pass.downloaded_bytes, 75);
        // The counts and the sums are one fold over one set of events, so they cannot disagree
        // about which side effects happened.
        assert_eq!(pass.uploaded_files + pass.downloaded_files, 3);
        assert_eq!(
            log.changed, 6,
            "every landed action still counts as changed"
        );
    }

    #[tokio::test]
    async fn the_legacy_mirror_is_derived_after_the_sample_never_beside_it() {
        // The ordering rule at `SyncActivity::derive_legacy_transfer`: a status reply samples the
        // staging directory into the LIST, then derives the mirror. Deriving first would publish a
        // mirror one sample stale; sampling the mirror instead would starve the list every #211
        // client reads.
        let shared = activity_test_shared();
        shared.pair.syncing.store(true, Ordering::SeqCst);
        let scratch = tempdir().expect("scratch dir");
        fs::write(scratch.path().join("part.bin"), vec![0u8; 4096]).expect("staged bytes");

        shared.begin_activity(SyncActivity {
            transfers: vec![
                TransferActivity::active("download", PathBuf::from("a/b.bin"), None, 1),
                TransferActivity::queued("upload", PathBuf::from("c/d.txt"), Some(12)),
            ],
            transfers_remaining: Some(2),
            ..new_activity(PHASE_EXECUTING)
        });
        shared.note_download_scratch(scratch.path());

        let activity = shared
            .response_with_sampled_activity("m")
            .await
            .activity
            .expect("activity");
        assert_eq!(
            activity.transfers[0].bytes_done,
            Some(4096),
            "the sample lands on the active row of the list"
        );
        assert_eq!(
            activity.transfer.expect("mirror").bytes_done,
            Some(4096),
            "and the mirror is derived from that row, after it"
        );
        assert_eq!(activity.transfers[1].bytes_done, None);
    }

    #[test]
    fn progress_sink_walk_updates_surface_in_status_replies() {
        let shared = Arc::new(activity_test_shared());
        // Replies only carry activity while a pass is in flight (`syncing`), matching how the
        // daemon core brackets every reconcile.
        shared.pair.syncing.store(true, Ordering::SeqCst);
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
        shared.pair.syncing.store(false, Ordering::SeqCst);
        assert!(shared.response("m").activity.is_none());
    }

    #[tokio::test]
    async fn download_bytes_are_sampled_live_from_the_staging_directory() {
        let shared = activity_test_shared();
        shared.pair.syncing.store(true, Ordering::SeqCst);
        let scratch = tempdir().expect("scratch dir");
        fs::write(scratch.path().join("partial.bin"), vec![0u8; 2048]).expect("partial file");

        shared.begin_activity(SyncActivity {
            transfers: vec![TransferActivity::active(
                "download",
                PathBuf::from("a/b.bin"),
                None,
                0,
            )],
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
            transfers: vec![TransferActivity::active(
                "upload",
                PathBuf::from("c/d.bin"),
                Some(10),
                0,
            )],
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
        let (loop_tx, _loop_rx) = mpsc::unbounded_channel();
        let plane = ControlPlane {
            shared: Arc::clone(&daemon.shared),
            approvals: tokio::sync::Mutex::new(
                open_database(&daemon.pair().config.db_path).expect("second connection"),
            ),
            loop_tx,
            io_timeout: Duration::from_millis(50),
            metrics_path: daemon.pair().metrics_path.clone(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            browse: browse_context(&daemon),
        };

        let (control_client, server) = UnixStream::pair().expect("socket pair");

        // The outer timeout is a generous test-only guard: if the handler regressed to
        // blocking forever it fails here instead of hanging the suite.
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            handle_control_connection(server, &plane),
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
        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(
            &daemon.pair().connection,
            Path::new("created-while-running.txt"),
        )
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
            &daemon.pair().connection,
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

        let queued_record = get_record(
            &daemon.pair().connection,
            Path::new("edited-while-running.txt"),
        )
        .expect("queued index lookup")
        .expect("queued index record");
        assert_eq!(queued_record.sha1_hash, Some(base_hash));
        assert_eq!(queued_record.sync_status, SyncStatus::Modified);

        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(
            &daemon.pair().connection,
            Path::new("edited-while-running.txt"),
        )
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
            &daemon.pair().connection,
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
        daemon
            .pair_mut()
            .pending_changes
            .insert(PathBuf::from("stable.txt"));

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert!(
            daemon.pair().pending_changes.is_empty(),
            "pending_changes must not leak entries for paths that plan to no action: {:?}",
            daemon.pair().pending_changes
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
            &daemon.pair().connection,
            &base_record("notes.txt", Some("remote-id"), base_hash.as_str()),
        )
        .expect("base record");

        // Pass: local Unchanged vs base, remote Changed → the daemon downloads and commits Synced.
        daemon.reconcile_blocking().expect_clean("reconcile");
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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

        let after = get_record(&daemon.pair().connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(
            after.sync_status,
            SyncStatus::Synced,
            "the daemon's own download echo must not flip the record to Modified (issue #49)"
        );
        assert!(
            daemon
                .pair()
                .pending_changes
                .contains(Path::new("notes.txt")),
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
            &daemon.pair().connection,
            &base_record("b.txt", Some("id-b"), b_hash.as_str()),
        )
        .expect("b base record");

        daemon.reconcile_blocking().expect_clean("reconcile");
        assert!(
            daemon.pair().authored_writes.contains(Path::new("a.txt")),
            "the downloaded file must be recorded as an authored write, so the guard is exercised"
        );
        assert!(
            !daemon.pair().authored_writes.contains(Path::new("b.txt")),
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

        let b_record = get_record(&daemon.pair().connection, Path::new("b.txt"))
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
            &daemon1.pair().connection,
            &base_record("doc.txt", Some("id-doc"), base_hash.as_str()),
        )
        .expect("base record");

        daemon1
            .reconcile_blocking()
            .expect_clean("pass 1 reconcile");
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
        let record_after_echo = get_record(&daemon1.pair().connection, Path::new("doc.txt"))
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

        daemon2
            .reconcile_blocking()
            .expect_clean("pass 2 reconcile");

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
        upsert_record(&daemon.pair().connection, &record).expect("conflict record");

        daemon
            .handle_fs_event(Event::new(EventKind::Remove(RemoveKind::File)).add_path(sidecar_path))
            .expect("handle sidecar remove event");

        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Modified);
        assert!(
            daemon
                .pair()
                .pending_changes
                .contains(Path::new("notes.txt")),
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
        upsert_record(&daemon.pair().connection, &record).expect("conflict record");

        daemon
            .handle_fs_event(Event::new(EventKind::Remove(RemoveKind::File)).add_path(sidecar_path))
            .expect("handle sidecar remove event");
        daemon.reconcile_blocking().expect_clean("reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("notes.txt"))
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

    /// #317: `--dry-run` returns before `Daemon::new`, so it can neither take nor be refused by the
    /// user-global lock — it must therefore be able to *look* at it. The look answers all three
    /// states, leaves no file behind, and above all leaves no lock behind.
    #[test]
    fn the_global_lock_probe_answers_without_creating_or_keeping_anything() {
        let directory = tempdir().expect("tempdir");
        // Deliberately under a directory that does not exist either: `LockGuard::acquire` would
        // create both, and a dry run must leave nothing behind on a machine that has never run a
        // daemon.
        let lock_path = directory.path().join("state").join("single-instance.lock");

        assert!(
            matches!(probe_daemon_lock(&lock_path), GlobalLockProbe::Free),
            "a lockfile that has never existed means nothing is holding it"
        );
        assert!(
            !lock_path.exists() && !lock_path.parent().expect("parent").exists(),
            "and looking must not create it"
        );

        let guard = LockGuard::acquire(&lock_path).expect("a daemon takes the lock");
        assert!(
            matches!(probe_daemon_lock(&lock_path), GlobalLockProbe::Held),
            "a live daemon's lock must be visible, or the warning never fires"
        );
        // The property the whole design turns on: probing a *held* lock must not have waited for
        // it, and probing a free one must not have kept it. If the probe held anything, this
        // acquire — the daemon's own — would fail.
        drop(guard);
        assert!(matches!(
            probe_daemon_lock(&lock_path),
            GlobalLockProbe::Free
        ));
        let guard = LockGuard::acquire(&lock_path)
            .expect("a probe that kept the lock would refuse the next daemon start");
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
            full_scan_schedule: None,
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
            // PERMANENT IN THE SHARED FIXTURE, where production defaults to `Trash`. Every
            // pre-existing daemon test that deletes locally was written against the unlink, and a
            // fixture-wide flip would silently change what each of them exercises. Tests about the
            // disposal itself set the mode explicitly and install `trash::test_hook`; `dispose`
            // panics on an un-hooked `Trash` in the lib test binary, so this cannot be got wrong by
            // forgetting rather than by deciding.
            local_delete_mode: LocalDeleteMode::Permanent,
            // Warm start disabled in the shared fixtures: existing event tests drive the first
            // reconcile as a steady-state incremental/bootstrap pass. Dedicated warm-start tests
            // enable it explicitly. (`test_config` sets `events_driven: false`, so warm start would
            // be ineligible here anyway; being explicit keeps the intent obvious.)
            warm_start: WarmStartConfig {
                enabled: false,
                ..WarmStartConfig::default()
            },
            conflict_naming: ConflictNaming::default(),
            log_filter: "info".to_owned(),
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
        /// A remote that keeps changing *while the walk runs*: the full-tree walk overwrites this
        /// shared cursor (a [`FakeEventSource::latest_handle`]) with the second value. A cursor
        /// read before the walk therefore reads the first value and one read after reads the
        /// second, which is what makes "when was the cursor read" observable (#303/#294).
        cursor_advanced_by_the_walk: Option<(Arc<Mutex<String>>, String)>,
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
                cursor_advanced_by_the_walk: None,
            }
        }

        /// Makes every full-tree walk set `handle` to `advanced`, simulating a remote change that
        /// lands mid-walk.
        fn advancing_cursor_on_walk(mut self, handle: Arc<Mutex<String>>, advanced: &str) -> Self {
            self.cursor_advanced_by_the_walk = Some((handle, advanced.to_owned()));
            self
        }

        fn advance_cursor_for_walk(&self) {
            if let Some((handle, advanced)) = &self.cursor_advanced_by_the_walk {
                *handle.lock().expect("shared latest cursor lock") = advanced.clone();
            }
        }
    }

    impl ProtonClient for EventFakeClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            self.full_walks.fetch_add(1, Ordering::SeqCst);
            self.advance_cursor_for_walk();
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
            self.advance_cursor_for_walk();
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
        /// Shared so a test can watch it move: [`Self::latest_handle`] hands the same cell to a
        /// client that advances it mid-walk, which is how "before or after the walk" becomes an
        /// assertable difference rather than a claim about call order.
        latest: Arc<Mutex<String>>,
        /// Scripted `latest_cursor` answers, consumed front-first; `None` = the read fails. Once
        /// drained, every call answers `latest` — so an unhealthy endpoint that recovers is one
        /// script (`AppResult` is not `Clone`, hence the queue rather than a stored result).
        scripted_latest: Mutex<Vec<Option<String>>>,
        fail_since: bool,
    }

    impl FakeEventSource {
        fn new(latest: &str) -> Self {
            Self {
                pages: Mutex::new(Vec::new()),
                latest: Arc::new(Mutex::new(latest.to_owned())),
                scripted_latest: Mutex::new(Vec::new()),
                fail_since: false,
            }
        }

        fn with_pages(latest: &str, pages: Vec<VolumeEventPage>) -> Self {
            Self {
                pages: Mutex::new(pages),
                latest: Arc::new(Mutex::new(latest.to_owned())),
                scripted_latest: Mutex::new(Vec::new()),
                fail_since: false,
            }
        }

        /// `scripted` answers the first `latest_cursor` calls (`None` = failure); later calls fall
        /// through to `latest`.
        fn with_scripted_latest(latest: &str, scripted: Vec<Option<String>>) -> Self {
            Self {
                pages: Mutex::new(Vec::new()),
                latest: Arc::new(Mutex::new(latest.to_owned())),
                scripted_latest: Mutex::new(scripted),
                fail_since: false,
            }
        }

        fn failing() -> Self {
            Self {
                pages: Mutex::new(Vec::new()),
                latest: Arc::new(Mutex::new("c0".to_owned())),
                scripted_latest: Mutex::new(Vec::new()),
                fail_since: true,
            }
        }

        /// The cell `latest_cursor` answers from, for a client that moves the remote mid-walk.
        fn latest_handle(&self) -> Arc<Mutex<String>> {
            Arc::clone(&self.latest)
        }
    }

    impl EventSource for FakeEventSource {
        fn latest_cursor(&self, _volume_id: &str) -> AppResult<String> {
            let mut scripted = self.scripted_latest.lock().expect("scripted latest lock");
            if scripted.is_empty() {
                return Ok(self.latest.lock().expect("latest cursor lock").clone());
            }
            scripted
                .remove(0)
                .ok_or_else(|| boxed_error("latest cursor unauthorized"))
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

    #[test]
    fn an_unsyncable_entity_is_named_persisted_and_outlives_the_passes_that_cannot_replan_it() {
        // #295: a Proton-native remote file plans `SkipUnsupported` every pass and was reported
        // only as an anonymous counter. It must be named, and it must SURVIVE — a skip is never
        // recorded in `file_index`, so it is absent from the baseline an incremental pass
        // reconstructs the remote from, and a per-pass list would blank on the daemon's normal
        // cadence (event-driven, warm start on boot).
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut remote_files = HashMap::new();
        remote_files.insert(
            PathBuf::from("Unsorted/Networth"),
            RemoteFile {
                downloadable: false,
                ..remote("Unsorted/Networth", "vol~node", None)
            },
        );
        let (client, _ops) = RecordingProtonClient::new(remote_files);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");

        assert_eq!(daemon.pair().unsyncable.len(), 1, "the skip must be named");
        assert_eq!(
            daemon.pair().unsyncable[0].path,
            PathBuf::from("Unsorted/Networth")
        );
        assert_eq!(
            daemon.pair().unsyncable[0].reason,
            UnsyncableReason::RemoteNotDownloadable,
            "a node the CLI cannot fetch as bytes — NOT the missing claimed digest #295 blamed"
        );
        assert_eq!(
            daemon.status_response("status").unsyncable,
            daemon.pair().unsyncable,
            "and it rides the status payload a client reads"
        );
        let first_seen = daemon.pair().unsyncable[0].first_seen_epoch_secs;
        assert_eq!(
            load_unsyncable_items(&daemon.pair().connection).expect("reload"),
            daemon.pair().unsyncable,
            "persisted, so a restart that warm-starts does not re-hide it"
        );

        // An incremental pass cannot re-derive the node at all: an empty plan must not drop it,
        // and the first-seen epoch must not restart.
        daemon.pass().record_unsyncable(&[], &[], false);
        assert_eq!(
            daemon.pair().unsyncable.len(),
            1,
            "an incremental pass only adds"
        );
        assert_eq!(
            daemon.pair().unsyncable[0].first_seen_epoch_secs,
            first_seen
        );

        // A full-tree walk is the only pass whose silence is evidence.
        daemon.pass().record_unsyncable(&[], &[], true);
        assert!(
            daemon.pair().unsyncable.is_empty(),
            "a full snapshot that no longer plans the skip clears it"
        );
        assert!(
            load_unsyncable_items(&daemon.pair().connection)
                .expect("reload")
                .is_empty(),
            "and the cleared list is persisted too"
        );
    }

    #[test]
    fn a_path_that_becomes_syncable_leaves_the_unsyncable_list_on_any_pass() {
        // The one thing an incremental pass CAN prove: it planned a real action for the path, so
        // whatever made it unsyncable is gone. Waiting for a full walk would leave a stale row
        // contradicting a transfer the user just watched happen.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _ops) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.pass().record_unsyncable(
            &[PlannedAction::skip_unsupported(
                Path::new("doc"),
                EntityKind::File,
                Some("vol~node".to_owned()),
                UnsyncableReason::RemoteNotDownloadable,
            )],
            &[],
            true,
        );
        assert_eq!(daemon.pair().unsyncable.len(), 1);

        daemon.pass().record_unsyncable(
            &[PlannedAction::new(
                Path::new("doc"),
                SyncAction::Download,
                EntityKind::File,
                Some("vol~node".to_owned()),
            )],
            &[],
            false,
        );
        assert!(daemon.pair().unsyncable.is_empty());

        // A move's DESTINATION is proof of the same kind: the pass planned a real action onto that
        // path, so an entry sitting there is already contradicted.
        daemon.pass().record_unsyncable(
            &[PlannedAction::skip_unsupported(
                Path::new("doc"),
                EntityKind::File,
                None,
                UnsyncableReason::RemoteNotDownloadable,
            )],
            &[],
            true,
        );
        assert_eq!(daemon.pair().unsyncable.len(), 1);
        daemon.pass().record_unsyncable(
            &[PlannedAction {
                destination_path: Some(PathBuf::from("doc")),
                ..PlannedAction::new(
                    Path::new("elsewhere"),
                    SyncAction::MoveLocal,
                    EntityKind::File,
                    None,
                )
            }],
            &[],
            false,
        );
        assert!(daemon.pair().unsyncable.is_empty());
    }

    #[test]
    fn a_standing_unsyncable_entity_is_reported_once_per_run_not_once_per_pass() {
        // A permanent condition logged every pass is noise, and noise is why the pre-existing
        // per-pass `debug!` line went unread for five months (#295). The dedup set is the guard;
        // a path that leaves the list and comes back speaks again.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _ops) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        let skip = || {
            vec![PlannedAction::skip_unsupported(
                Path::new("doc"),
                EntityKind::File,
                None,
                UnsyncableReason::RemoteNotDownloadable,
            )]
        };

        daemon.pass().record_unsyncable(&skip(), &[], true);
        daemon.pass().record_unsyncable(&skip(), &[], true);
        assert_eq!(
            daemon.pair().reported_unsyncable.len(),
            1,
            "the second pass must not re-report the same standing path"
        );

        daemon.pass().record_unsyncable(&[], &[], true);
        assert!(
            daemon.pair().reported_unsyncable.is_empty(),
            "a path that leaves the list is un-reported, so a recurrence is announced again"
        );
        daemon.pass().record_unsyncable(&skip(), &[], true);
        assert_eq!(daemon.pair().reported_unsyncable.len(), 1);
    }

    /// A daemon over an empty local root, for the `record_unsyncable` merge tests.
    fn unsyncable_test_daemon(
        directory: &tempfile::TempDir,
    ) -> (
        PathBuf,
        Daemon<RecordingProtonClient>,
        Arc<Mutex<Vec<RecordedOperation>>>,
    ) {
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, ops) = RecordingProtonClient::new(HashMap::new());
        let daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        (local_root, daemon, ops)
    }

    fn socket_entry(path: &str) -> UnsyncableEntry {
        UnsyncableEntry {
            relative_path: PathBuf::from(path),
            reason: UnsyncableReason::LocalSocket,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_local_socket_reaches_the_standing_list_and_the_status_reply() {
        // #232's whole point: the scan drops it, so nothing downstream can name it — the list has
        // to be fed from the walk itself, not from the plan.
        let directory = tempdir().expect("tempdir");
        let (local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);
        let _listener = std::os::unix::net::UnixListener::bind(local_root.join("session.sock"))
            .expect("socket");

        daemon.reconcile_blocking().expect_clean("bootstrap");

        assert_eq!(
            daemon.pair().unsyncable.len(),
            1,
            "{:?}",
            daemon.pair().unsyncable
        );
        assert_eq!(
            daemon.pair().unsyncable[0].path,
            PathBuf::from("session.sock")
        );
        assert_eq!(
            daemon.pair().unsyncable[0].reason,
            UnsyncableReason::LocalSocket
        );
        assert_eq!(
            daemon.pair().unsyncable[0].entity_kind,
            EntityKind::File,
            "the engine knows two kinds and this is not a directory it would descend into"
        );
        assert_eq!(
            daemon.status_response("status").unsyncable,
            daemon.pair().unsyncable,
            "and it rides the status payload a client reads"
        );
        assert_eq!(
            load_unsyncable_items(&daemon.pair().connection).expect("reload"),
            daemon.pair().unsyncable,
            "persisted, so a restart that warm-starts does not re-hide it"
        );
    }

    #[test]
    fn a_local_reason_prunes_on_any_pass_while_a_remote_one_waits_for_a_full_sweep() {
        // The asymmetry, stated as a test. A local skip is observed by the stat-walk, which every
        // pass reaching the executor runs over the WHOLE tree — so its silence is evidence. A
        // remote skip is absent from the baseline an incremental pass reconstructs from, so only a
        // full-tree walk's silence is. Without the split, a deleted socket would sit on the list
        // until a full sweep — which, with the periodic resync off by default and restarts warm-
        // starting, is close to never.
        let directory = tempdir().expect("tempdir");
        let (_local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);
        let remote_skip = vec![PlannedAction::skip_unsupported(
            Path::new("Networth"),
            EntityKind::File,
            None,
            UnsyncableReason::RemoteNotDownloadable,
        )];

        daemon
            .pass()
            .record_unsyncable(&remote_skip, &[socket_entry("session.sock")], true);
        assert_eq!(daemon.pair().unsyncable.len(), 2);

        // An incremental pass that re-derives neither: it walked the local tree and did not find
        // the socket, but its remote map cannot speak about the Docs node at all.
        daemon.pass().record_unsyncable(&[], &[], false);
        assert_eq!(
            daemon
                .pair()
                .unsyncable
                .iter()
                .map(|item| item.path.display().to_string())
                .collect::<Vec<_>>(),
            vec!["Networth".to_owned()],
            "the socket goes, the remote skip stays: {:?}",
            daemon.pair().unsyncable
        );

        daemon.pass().record_unsyncable(&[], &[], true);
        assert!(
            daemon.pair().unsyncable.is_empty(),
            "{:?}",
            daemon.pair().unsyncable
        );
    }

    #[test]
    fn an_unknown_reason_from_a_newer_build_waits_for_the_pass_that_walks_both_sides() {
        // A token this build cannot place could have come from either producer, so the only pass
        // whose silence is evidence is the one that walked both. Dropping it on an incremental
        // pass would blank a newer daemon's report every 30 seconds after a downgrade.
        let directory = tempdir().expect("tempdir");
        let (_local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);
        daemon.pair_mut().unsyncable = vec![UnsyncableItem {
            path: PathBuf::from("mystery"),
            entity_kind: EntityKind::File,
            reason: UnsyncableReason::Other("local_wormhole".to_owned()),
            first_seen_epoch_secs: 1,
        }];

        daemon.pass().record_unsyncable(&[], &[], false);
        assert_eq!(
            daemon.pair().unsyncable.len(),
            1,
            "an incremental pass keeps it"
        );

        daemon.pass().record_unsyncable(&[], &[], true);
        assert!(daemon.pair().unsyncable.is_empty(), "a full sweep drops it");
    }

    #[test]
    fn a_path_whose_kind_changes_reports_the_new_kind_and_keeps_its_age() {
        // A socket replaced by a symlink under one name is one path and two different problems.
        // The path never leaves the list to be re-added, so a carried row would go on reporting
        // `local_socket` for a socket that is gone — indefinitely, and on the one surface whose
        // whole job is saying what the thing is. (Copilot, PR #316, suppressed.)
        let directory = tempdir().expect("tempdir");
        let (_local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);

        daemon
            .pass()
            .record_unsyncable(&[], &[socket_entry("thing")], false);
        daemon.pair_mut().unsyncable[0].first_seen_epoch_secs -= 600;
        let first_seen = daemon.pair().unsyncable[0].first_seen_epoch_secs;

        daemon.pass().record_unsyncable(
            &[],
            &[UnsyncableEntry {
                relative_path: PathBuf::from("thing"),
                reason: UnsyncableReason::LocalSymlink,
            }],
            false,
        );

        assert_eq!(
            daemon.pair().unsyncable.len(),
            1,
            "{:?}",
            daemon.pair().unsyncable
        );
        assert_eq!(
            daemon.pair().unsyncable[0].reason,
            UnsyncableReason::LocalSymlink,
            "the reason must be this pass's observation, not the one it replaced"
        );
        assert_eq!(
            daemon.pair().unsyncable[0].first_seen_epoch_secs,
            first_seen,
            "the age is about the PATH being unsyncable, and it has been since then"
        );
    }

    #[test]
    fn a_local_entry_keeps_the_epoch_it_was_first_seen_at() {
        let directory = tempdir().expect("tempdir");
        let (_local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);

        daemon
            .pass()
            .record_unsyncable(&[], &[socket_entry("session.sock")], false);
        let first_seen = daemon.pair().unsyncable[0].first_seen_epoch_secs;
        daemon.pair_mut().unsyncable[0].first_seen_epoch_secs = first_seen.saturating_sub(600);
        let expected = daemon.pair().unsyncable[0].first_seen_epoch_secs;

        daemon
            .pass()
            .record_unsyncable(&[], &[socket_entry("session.sock")], true);
        assert_eq!(
            daemon.pair().unsyncable[0].first_seen_epoch_secs,
            expected,
            "'how long has this been stuck' must not restart every pass"
        );
    }

    #[test]
    fn a_path_the_scan_still_reports_survives_a_delete_planned_for_the_file_it_replaced() {
        // Replace a synced file with a socket and one pass holds two true statements about one
        // path: the planner sees the file gone and plans a delete, the scan sees the socket. The
        // supersession rule means "this path became syncable", which is not what happened, so
        // direct observation this pass outranks it. Visible in the FIRST-SEEN epoch: dropping the
        // carried entry and re-adding the observed one restarts the age, so "stuck since Tuesday"
        // would read as "stuck since just now" on every pass the delete is re-planned — and while
        // the deletion is withheld awaiting approval, that is every pass.
        let directory = tempdir().expect("tempdir");
        let (_local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);

        daemon
            .pass()
            .record_unsyncable(&[], &[socket_entry("session.sock")], true);
        daemon.pair_mut().unsyncable[0].first_seen_epoch_secs -= 600;
        let first_seen = daemon.pair().unsyncable[0].first_seen_epoch_secs;

        daemon.pass().record_unsyncable(
            &[PlannedAction::new(
                Path::new("session.sock"),
                SyncAction::RemoteDelete,
                EntityKind::File,
                None,
            )],
            &[socket_entry("session.sock")],
            true,
        );

        assert_eq!(
            daemon.pair().unsyncable.len(),
            1,
            "{:?}",
            daemon.pair().unsyncable
        );
        assert_eq!(
            daemon.pair().unsyncable[0].reason,
            UnsyncableReason::LocalSocket
        );
        assert_eq!(
            daemon.pair().unsyncable[0].first_seen_epoch_secs,
            first_seen,
            "the entry was carried, not re-created"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_deleted_socket_leaves_the_list_on_the_next_pass_that_plans_anything() {
        // End to end over a real filesystem, because the freshness argument is about which passes
        // run the stat-walk — not about what `record_unsyncable` does when handed a list.
        let directory = tempdir().expect("tempdir");
        let (local_root, mut daemon, _ops) = unsyncable_test_daemon(&directory);
        let socket_path = local_root.join("session.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("socket");

        daemon.reconcile_blocking().expect_clean("bootstrap");
        assert_eq!(daemon.pair().unsyncable.len(), 1);

        drop(listener);
        fs::remove_file(&socket_path).expect("remove socket");
        daemon.reconcile_blocking().expect_clean("second pass");

        assert!(
            daemon.pair().unsyncable.is_empty(),
            "the walk no longer finds it: {:?}",
            daemon.pair().unsyncable
        );
        assert!(
            load_unsyncable_items(&daemon.pair().connection)
                .expect("reload")
                .is_empty(),
            "and the shorter list is persisted"
        );
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
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        // Keyring now readable: the factory yields a source.
        daemon.event_source_factory = Box::new(|| Some(Box::new(FakeEventSource::new("cursor-0"))));
        daemon.pass().reacquire_event_source_if_needed();
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
            daemon.pair().incremental_passes_since_full_scan,
            effective_full_scan_every(daemon.pair().config.events_full_scan_every),
            "mid-life reacquisition must force a snapshot floor"
        );

        // Idempotent: a working source is never rebuilt (the factory must not be called again).
        daemon.event_source_factory = Box::new(|| panic!("must not rebuild an existing source"));
        daemon.pass().reacquire_event_source_if_needed();
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
        daemon.pass().reacquire_event_source_if_needed();
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
        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "bootstrap performs one full walk"
        );
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("cursor persisted by bootstrap");
        assert_eq!(cursor.last_event_id, "cursor-0");
        assert_eq!(daemon.pair().incremental_passes_since_full_scan, 0);

        // Second pass: stored cursor + derivable volume + no changes → idle incremental, no walk.
        daemon
            .reconcile_blocking()
            .expect_clean("idle incremental reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "an idle incremental pass must not re-walk the whole tree"
        );
        assert_eq!(daemon.pair().incremental_passes_since_full_scan, 1);
    }

    /// #294: the pre-snapshot cursor read fails (unhealthy events endpoint — the same condition
    /// that forces the snapshot) and the endpoint then recovers *during* the walk. The old code
    /// read the cursor after the walk and banked it, skipping every change made while walking.
    /// Nothing may be persisted here; the next pass finds no cursor, declines the scope, and
    /// bootstraps again — which is also the recovery path, so the loop terminates on the first
    /// pass whose pre-snapshot read succeeds.
    #[test]
    fn a_failed_pre_snapshot_cursor_read_persists_no_cursor_and_heals_on_the_next_pass() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");
        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]));
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            // First `latest_cursor` call fails; every later call answers "cursor-live".
            Some(Box::new(FakeEventSource::with_scripted_latest(
                "cursor-live",
                vec![None],
            ))),
        )
        .expect("daemon");
        // A baseline names the volume, so the cursor *is* readable before the walk — this is the
        // "prior state exists" case, not a first-ever bootstrap. No cursor stored yet.
        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed record");

        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the first pass bootstraps"
        );
        assert!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("load cursor")
                .is_none(),
            "a cursor read after the walk asserts mid-walk changes were applied; persist none"
        );

        // Second pass: no cursor → the scope declines → bootstrap again, and this time the
        // pre-snapshot read succeeds, so the cursor it banks predates that walk.
        daemon.reconcile_blocking().expect_clean("second bootstrap");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            2,
            "without a cursor the next pass must full-walk"
        );
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("cursor persisted once the endpoint answered");
        assert_eq!(cursor.last_event_id, "cursor-live");

        // Third pass: streaming resumed, so the repeated bootstrapping stops.
        daemon.reconcile_blocking().expect_clean("idle incremental");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            2,
            "a recovered cursor ends the bootstrap-every-pass loop"
        );
        assert_eq!(daemon.pair().incremental_passes_since_full_scan, 1);
    }

    /// #294, the sub-case with a cursor already stored: persisting nothing must also not *clear*
    /// it. The older cursor is the safer replay point (re-delivered events reconstruct
    /// idempotently; skipped ones are lost), so the next pass streams from it rather than
    /// bootstrapping again.
    #[test]
    fn a_failed_pre_snapshot_cursor_read_leaves_the_stored_cursor_untouched() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");
        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]));
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_scripted_latest(
                "cursor-new",
                vec![None],
            ))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-old", 1).expect("seed cursor");
        // Warm start is off in this fixture, so the first pass bootstraps with a cursor already
        // stored — exactly the shape of an events-fetch fallback on a long-running daemon.
        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");

        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("the stored cursor must survive");
        assert_eq!(
            cursor.last_event_id, "cursor-old",
            "the pass must neither fabricate a post-walk cursor nor clear the stored one"
        );
        assert_eq!(cursor.updated_at, 1, "the stored row is untouched");
        assert_eq!(full_walks.load(Ordering::SeqCst), 1);

        // The surviving cursor keeps the next pass incremental: it replays from before the walk
        // (over-replay is idempotent), so a failed capture costs no extra full walk here.
        daemon.reconcile_blocking().expect_clean("idle incremental");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the older cursor is still a usable replay point"
        );
    }

    /// #294, third door: the volume lookup itself fails. `volume_id_for_scope` falls back to the
    /// sole stored cursor row when no baseline record carries a composed id, so an unreadable row
    /// is *not* "first-ever bootstrap" — a cursor exists, it just cannot be seen this pass. Anchor
    /// after the walk and it is overwritten with a post-walk value.
    #[test]
    fn an_unreadable_cursor_row_is_not_treated_as_a_first_ever_bootstrap() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");
        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]));
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-live"))),
        )
        .expect("daemon");
        // No composed id in the baseline (an all-Proton-native remote, #32), so the volume can only
        // come from the cursor row — whose `updated_at` is text, making the row unreadable while
        // leaving it fully writable.
        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", None, keep.as_str()),
        )
        .expect("seed record");
        daemon
            .pair()
            .connection
            .execute(
                "INSERT INTO remote_event_cursor (scope_id, last_event_id, updated_at)
                 VALUES ('vol', 'cursor-old', 'corrupt')",
                [],
            )
            .expect("seed corrupt cursor");
        assert!(
            load_sole_event_cursor(&daemon.pair().connection).is_err(),
            "precondition: the cursor row is unreadable"
        );

        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");

        let stored: String = daemon
            .pair()
            .connection
            .query_row(
                "SELECT last_event_id FROM remote_event_cursor WHERE scope_id = 'vol'",
                [],
                |row| row.get(0),
            )
            .expect("cursor row still present");
        assert_eq!(
            stored, "cursor-old",
            "an unreadable row must not be overwritten by a post-walk cursor"
        );
    }

    /// Seeds a first-ever-bootstrap shape: an empty baseline, no stored cursor, and a remote whose
    /// single top-level file matches local (so the pass is clean and reaches the final commit).
    /// The event source answers `cursor-before-walk` until the full-tree walk runs, which moves it
    /// to `cursor-after-walk` — so the persisted value *names when it was read*.
    fn first_bootstrap_fixture(
        directory: &tempfile::TempDir,
    ) -> (EventFakeClient, FakeEventSource) {
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");
        let source = FakeEventSource::new("cursor-before-walk");
        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]))
        .advancing_cursor_on_walk(source.latest_handle(), "cursor-after-walk");
        (client, source)
    }

    /// #303: the first-ever bootstrap was the one arm still reading its cursor *after* the walk,
    /// because nothing stored named a volume yet. A change landing mid-walk in an already-listed
    /// folder is missed by the snapshot and then skipped by the delta — and since ADR 0004 made
    /// restarts warm-start from the cursor, with the periodic resync off by default, nothing
    /// re-derives it. The volume is nameable up front from one targeted listing of the remote root.
    #[test]
    fn a_first_ever_bootstrap_anchors_its_cursor_before_the_walk() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let (client, source) = first_bootstrap_fixture(&directory);
        let full_walks = Arc::clone(&client.full_walks);
        let directory_lists = Arc::clone(&client.directory_lists);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(source)),
        )
        .expect("daemon");
        assert!(
            load_index(&daemon.pair().connection)
                .expect("load index")
                .is_empty(),
            "precondition: nothing indexed, so no baseline composed id names the volume"
        );
        assert!(
            load_sole_event_cursor(&daemon.pair().connection)
                .expect("load sole cursor")
                .is_none(),
            "precondition: no stored cursor names the volume either"
        );

        daemon
            .reconcile_blocking()
            .expect_clean("first-ever bootstrap");

        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("the first-ever bootstrap must anchor a cursor");
        assert_eq!(
            cursor.last_event_id, "cursor-before-walk",
            "the anchored cursor must predate the walk, or every change made during it is skipped"
        );
        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            1,
            "naming the volume costs exactly one targeted listing, not a second walk"
        );
        assert_eq!(full_walks.load(Ordering::SeqCst), 1);
    }

    /// The listing is an addition to a path that must never gain a way to fail: when it errors, the
    /// bootstrap degrades to the pre-#303 post-walk anchor rather than aborting a first sync.
    #[test]
    fn a_failed_remote_root_listing_degrades_to_the_post_walk_anchor() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let (mut client, source) = first_bootstrap_fixture(&directory);
        client.fail_directory_lists = true;
        let full_walks = Arc::clone(&client.full_walks);
        let directory_lists = Arc::clone(&client.directory_lists);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(source)),
        )
        .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_clean("first-ever bootstrap with an unlistable root");

        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("a failed listing must not cost the anchor entirely");
        assert_eq!(
            cursor.last_event_id, "cursor-after-walk",
            "exactly the pre-#303 behaviour: unable to name the volume up front, anchor after"
        );
        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            1,
            "the listing is attempted once and never retried"
        );
        assert_eq!(full_walks.load(Ordering::SeqCst), 1);
    }

    /// Once the volume *is* named, a failed cursor read is the #294 decision, not a reason to retry
    /// after the walk: a post-walk retry here would re-open the window through a side door. Persist
    /// nothing and re-derive next pass.
    #[test]
    fn a_first_ever_bootstrap_whose_cursor_read_fails_persists_no_cursor() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let (client, _source) = first_bootstrap_fixture(&directory);
        let directory_lists = Arc::clone(&client.directory_lists);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            // The pre-walk read fails; a post-walk retry would answer "cursor-live".
            Some(Box::new(FakeEventSource::with_scripted_latest(
                "cursor-live",
                vec![None],
            ))),
        )
        .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_clean("first-ever bootstrap with an unhealthy events endpoint");

        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            1,
            "precondition: the volume was named, so the failure is the cursor read"
        );
        assert!(
            load_sole_event_cursor(&daemon.pair().connection)
                .expect("load sole cursor")
                .is_none(),
            "a named volume whose cursor read failed must persist nothing, not anchor after the walk"
        );
    }

    /// The extra listing is confined to the arm that needs it. A degraded session (events on, no
    /// source) already pays a full snapshot per `scan_interval` tick and must not also pay a
    /// listing whose answer it could not use; with events off, nothing reads a cursor at all.
    #[test]
    fn a_bootstrap_that_cannot_use_a_cursor_never_lists_the_remote_root() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let (client, _source) = first_bootstrap_fixture(&directory);
        let degraded_lists = Arc::clone(&client.directory_lists);
        // `with_client` injects event_source = None: events on, session unavailable.
        let mut degraded = Daemon::with_client(event_config(directory.path(), &local_root), client)
            .expect("degraded daemon");
        // The default factory reads the real keyring; pin it shut so the session stays degraded.
        degraded.event_source_factory = Box::new(|| None);
        degraded
            .reconcile_blocking()
            .expect_clean("degraded bootstrap");
        assert_eq!(
            degraded_lists.load(Ordering::SeqCst),
            0,
            "a session that cannot stream events must not spend a listing naming their volume"
        );

        let other = tempdir().expect("tempdir");
        let other_local = other.path().join("local");
        let (client, source) = first_bootstrap_fixture(&other);
        let events_off_lists = Arc::clone(&client.directory_lists);
        let mut events_off = Daemon::with_client_and_event_source(
            DaemonConfig {
                events_driven: false,
                ..event_config(other.path(), &other_local)
            },
            client,
            Some(Box::new(source)),
        )
        .expect("events-off daemon");
        events_off
            .reconcile_blocking()
            .expect_clean("events-off bootstrap");
        assert_eq!(
            events_off_lists.load(Ordering::SeqCst),
            0,
            "the snapshot-only path stays byte-identical to the pre-events behaviour"
        );
    }

    /// The other half of the confinement: a bootstrap whose volume the *baseline* already names
    /// spends no listing and — the part that matters — keeps anchoring before the walk.
    #[test]
    fn a_bootstrap_that_can_already_name_its_volume_lists_nothing_extra() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        let (client, source) = first_bootstrap_fixture(&directory);
        let directory_lists = Arc::clone(&client.directory_lists);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(source)),
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), sha1_bytes(b"keep").as_str()),
        )
        .expect("seed record");

        daemon.reconcile_blocking().expect_clean("bootstrap");

        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            0,
            "a baseline composed id already names the volume; the listing must not become a \
             per-bootstrap cost"
        );
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("cursor anchored");
        assert_eq!(cursor.last_event_id, "cursor-before-walk");
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
            &daemon.pair().connection,
            &directory_record("dir", Some("vol~ndir")),
        )
        .expect("seed dir record");
        upsert_record(
            &daemon.pair().connection,
            &base_record("dir/a.txt", Some("vol~na"), old.as_str()),
        )
        .expect("seed file record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams.
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("incremental reconcile");

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
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
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
        // The stream offers a NEW cursor with no changes in it, so the hold below is observable:
        // without it this pass would persist `cursor-1`.
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::with_pages(
                "cursor-1",
                vec![one_page("cursor-1", Vec::new())],
            ))),
        )
        .expect("daemon");

        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams.
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;
        // Make the pass non-idle so it scans + plans the failing upload.
        daemon
            .pair_mut()
            .pending_changes
            .insert(PathBuf::from("new.txt"));

        daemon.reconcile_blocking().expect_partial(
            1,
            "the failed upload is an item failure, not a pass failure",
        );

        // Neither the index nor the cursor advanced; the events replay next pass.
        assert!(
            get_record(&daemon.pair().connection, Path::new("new.txt"))
                .expect("get record")
                .is_none(),
            "the failed file must not be recorded (no partial commit)"
        );
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("load cursor")
            .expect("cursor present");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "cursor must NOT advance while an item failed"
        );
        assert!(
            daemon.pair().pending_changes.contains(Path::new("new.txt")),
            "the failed path is re-queued: no remote event describes a failed upload, so the \
             held cursor alone would let the next pass idle-skip it"
        );
        assert_eq!(
            daemon.pair().incremental_passes_since_full_scan,
            1,
            "a partial pass is still an incremental pass for the periodic-resync counter"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "the incremental path took no full walk"
        );
    }

    #[test]
    fn a_partial_pass_re_queues_its_failed_paths_so_the_next_event_driven_pass_retries_them() {
        // #136's starvation, one layer down: with the periodic resync off by default, the ONLY
        // thing that makes the next event-driven pass look at a failed upload again is the
        // re-queue. A failed upload produces no remote event, so the held cursor cannot help: an
        // empty delta plus an empty `pending_changes` takes the idle fast-path, which skips the
        // local scan entirely.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let keep = sha1_bytes(b"keep");
        fs::write(local_root.join("keep.txt"), b"keep").expect("keep file");
        fs::write(local_root.join("new.txt"), b"new").expect("new file");

        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("keep.txt"),
            remote_file_entity("keep.txt", "vol~nk", keep.as_str()),
        )]));
        // Fails once, then clears itself: pass 2's upload succeeds, so the record it writes is
        // proof that pass 2 planned and executed rather than idle-skipping.
        let fail_next_upload = Arc::clone(&client.fail_next_upload);
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;
        daemon
            .pair_mut()
            .pending_changes
            .insert(PathBuf::from("new.txt"));

        fail_next_upload.store(true, Ordering::SeqCst);
        daemon
            .reconcile_blocking()
            .expect_partial(1, "pass 1's upload fails");

        // Pass 2 sees the same empty delta. Nothing else can wake it: no watcher runs in this
        // test, and the previous pass cleared `pending_changes` before re-queueing.
        daemon
            .reconcile_blocking()
            .expect_clean("pass 2 retries the failed upload from ground truth");

        assert!(
            get_record(&daemon.pair().connection, Path::new("new.txt"))
                .expect("get record")
                .is_some(),
            "the re-queued path must be re-planned and uploaded by the next event-driven pass"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "and it must do so incrementally — no full walk rescued it"
        );
        assert!(
            daemon.pair().pending_changes.is_empty(),
            "a clean pass clears the queue again"
        );
    }

    #[test]
    fn an_untracked_descendant_of_a_moved_directory_uploads_on_the_next_event_driven_pass() {
        // #12's other half. The move pass is CLEAN, so the cursor advances and the next pass has
        // an empty delta; inotify emits nothing for the children of a renamed directory, so the
        // re-queued destination is the only reason that pass looks at `fresh.txt` at all.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir_all(local_root.join("old-docs")).expect("local old-docs directory");
        let report = local_root.join("old-docs").join("report.txt");
        fs::write(&report, b"same content").expect("nested local file");
        let report_hash = crate::index::compute_sha1(&report).expect("nested file hash");
        fs::write(local_root.join("old-docs").join("fresh.txt"), b"brand new")
            .expect("untracked nested local file");

        let client = EventFakeClient::new(HashMap::from([(
            PathBuf::from("new-docs"),
            RemoteEntity::Directory(RemoteDirectory {
                path: PathBuf::from("new-docs"),
                id: Some("vol~nd".to_owned()),
                name: "new-docs".to_owned(),
            }),
        )]));
        let full_walks = Arc::clone(&client.full_walks);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            client,
            Some(Box::new(FakeEventSource::new("cursor-1"))),
        )
        .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &FileRecord {
                file_path: PathBuf::from("old-docs"),
                entity_kind: EntityKind::Directory,
                file_size: 0,
                mtime: 1,
                sha1_hash: None,
                proton_id: Some("vol~nd".to_owned()),
                sync_status: SyncStatus::Synced,
            },
        )
        .expect("directory base record");
        upsert_record(
            &daemon.pair().connection,
            &base_record("old-docs/report.txt", Some("vol~nr"), report_hash.as_str()),
        )
        .expect("nested file base record");

        // Pass 1 is the startup snapshot: it converges the directory move and captures a cursor.
        daemon
            .reconcile_blocking()
            .expect_clean("the move pass plans nothing for the untracked descendant");
        assert!(
            get_record(&daemon.pair().connection, Path::new("new-docs/fresh.txt"))
                .expect("get record")
                .is_none(),
            "the untracked descendant is deliberately left for the next pass"
        );

        // Pass 2 streams: empty delta, so only the re-queued destination keeps it non-idle.
        daemon
            .reconcile_blocking()
            .expect_clean("the next pass uploads it from its post-move path");

        let record = get_record(&daemon.pair().connection, Path::new("new-docs/fresh.txt"))
            .expect("get record")
            .expect("the untracked descendant is finally recorded");
        assert_eq!(record.entity_kind, EntityKind::File);
        assert!(
            get_record(&daemon.pair().connection, Path::new("old-docs/fresh.txt"))
                .expect("get record")
                .is_none(),
            "nothing may ever be recorded at the pre-move path"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "only the startup snapshot walked the tree; the second pass stayed incremental"
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
        daemon.pair_mut().scan_options =
            scan_options_from_config(&daemon.pair().config).expect("scan options");

        upsert_record(
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        // The excluded parent is not indexed, forcing resolution via the root listing.
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams.
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("incremental reconcile");

        assert!(
            !local_root.join("secret/x.txt").exists(),
            "an excluded remote file must never be downloaded"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("secret/x.txt"))
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams
        // (and then falls back to a snapshot for the reason under test, not the startup floor).
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("reconcile falls back cleanly");

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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Simulate the mandatory startup bootstrap having already run, so this pass streams
        // (and then falls back to a snapshot for the reason under test, not the startup floor).
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        // Steady state, not the first pass after boot: bypass the `first_reconcile` warm-start /
        // bootstrap branch so this test drives the ongoing incremental path directly.
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("reconcile falls back cleanly");

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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("an incomplete listing falls back cleanly");

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
            get_record(&daemon.pair().connection, Path::new("keep.txt"))
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        // Steady state (not the first pass), already at the resync threshold → this pass must
        // snapshot via the periodic-resync counter, not go incremental.
        daemon.pair_mut().is_first_reconcile = false;
        daemon.pair_mut().incremental_passes_since_full_scan = 1;

        daemon
            .reconcile_blocking()
            .expect_clean("forced resync reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "reaching events_full_scan_every must force a full-tree snapshot"
        );
        assert_eq!(
            daemon.pair().incremental_passes_since_full_scan,
            0,
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
        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the first reconcile after startup must still full-scan"
        );
        assert_eq!(daemon.pair().incremental_passes_since_full_scan, 0);

        // Far more idle passes than the *old* default (20) would have resynced at — none may walk.
        for _ in 0..50 {
            daemon
                .reconcile_blocking()
                .expect_clean("idle incremental reconcile");
        }
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "with the periodic resync disabled, only the startup snapshot ever walks the whole tree"
        );
        assert_eq!(
            daemon.pair().incremental_passes_since_full_scan,
            50,
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
        daemon.reconcile_blocking().expect_clean("startup snapshot");
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
        let created = daemon.pair().config.local_root.join("photos");
        fs::create_dir(&created).expect("empty local directory");

        daemon
            .handle_fs_event(Event::new(EventKind::Create(CreateKind::Folder)).add_path(created))
            .expect("handle directory create event");
        assert!(
            daemon.pair().pending_changes.contains(Path::new("photos")),
            "a directory create must queue the path so the pass is not idle"
        );

        daemon
            .reconcile_blocking()
            .expect_clean("incremental reconcile after the directory create");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the directory must be mirrored by the incremental pass, not by a full-tree walk"
        );
        let summary = daemon
            .pair()
            .last_successful_sync_summary
            .as_ref()
            .expect("successful pass summary");
        assert_eq!(
            summary.remote_directories_created, 1,
            "the empty directory must be created remotely"
        );
        let record = get_record(&daemon.pair().connection, Path::new("photos"))
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
        let edited = daemon.pair().config.local_root.join("a.txt");
        let contents = b"edited while the watcher was deaf";
        fs::write(&edited, contents).expect("local edit");
        // Deliberately no `handle_fs_event`: this is the event the overflow dropped.
        assert!(daemon.pair().pending_changes.is_empty());

        daemon.note_watch_error(&notify::Error::generic("inotify queue overflow"));
        daemon
            .reconcile_blocking()
            .expect_clean("incremental reconcile after the watcher error");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the forced rescan is local-only; the remote stays on the event stream"
        );
        let summary = daemon
            .pair()
            .last_successful_sync_summary
            .as_ref()
            .expect("successful pass summary");
        assert_eq!(
            summary.uploads, 1,
            "the edit whose watcher event was lost must still upload"
        );
        let record = get_record(&daemon.pair().connection, Path::new("a.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sha1_hash, Some(sha1_bytes(contents)));
        assert!(
            !daemon.pair().force_local_rescan,
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

        daemon
            .reconcile_blocking()
            .expect_clean("degraded first pass");
        let session_cause = daemon.session_declined.clone();
        let reported = session_cause
            .as_ref()
            .map(|declined| declined.reason.as_str())
            .expect("a degraded pass must report why it is full-walking");
        assert!(
            reported.contains("no usable proton-drive CLI session")
                && reported.contains("scan interval"),
            "the session cause must name itself and the cadence it implies: {reported}"
        );
        assert_eq!(
            daemon.pair().event_scope_declined,
            None,
            "a pass with no event source never consults the scope, so the pair's latch must stay \
             empty rather than double-report the session's condition"
        );

        // Same cause next pass → the recorded reason is unchanged, so it is logged once.
        daemon
            .reconcile_blocking()
            .expect_clean("degraded second pass");
        assert_eq!(daemon.session_declined, session_cause);

        // Session recovers: the session cause is cleared (so a later regression re-reports rather
        // than being swallowed by a stale latch) and the *next* cause — the scope, which no degraded
        // pass could anchor a cursor for — is reported in its own words on the pair's latch.
        daemon.event_source = Some(Box::new(FakeEventSource::new("cursor-0")));
        daemon.reconcile_blocking().expect_clean("recovered pass");
        assert_eq!(
            daemon.session_declined, None,
            "the session cause is gone, and only its own recovery may clear it"
        );
        let scope_cause = daemon.pair().event_scope_declined.clone();
        assert!(
            scope_cause
                .as_deref()
                .is_some_and(|reason| reason.contains("no stored event cursor")),
            "a scope decline must follow the session cause, not compete with it: {scope_cause:?}"
        );

        // That pass anchored a cursor, so the scope now resolves: the record clears, and a later
        // regression of either cause reports again instead of being swallowed by a stale latch.
        daemon.reconcile_blocking().expect_clean("incremental pass");
        assert_eq!(daemon.pair().event_scope_declined, None);
        assert_eq!(daemon.session_declined, None);
    }

    #[test]
    fn the_process_session_cause_is_neither_cleared_nor_worded_by_one_pairs_config() {
        // #102 phase 2. `note_degraded_session_if_needed` writes the PROCESS latch while reading two
        // `KeyScope::Pair` values, so both have to be kept away from what that latch does — or the
        // split is undone for this cause the moment there is a second pair:
        //
        // * `events_driven` gating the *clear* means a pair that opted out of event-driven detection
        //   wipes a keyring cause that is still true, and the degraded pair re-logs it next pass, for
        //   ever — the exact failure the split exists to prevent.
        // * `scan_interval` inside the *reason* means two pairs with different intervals produce two
        //   different strings, and `note_session_declined` decides log-or-silent by comparing the
        //   reason — so both log every pass. The typed cause does not save this: it governs
        //   clearing, never the log decision.
        //
        // Driven by mutating the pair's config between calls, which is exactly what a second pair's
        // pass would present to this method: it reads nothing else about the pair.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut daemon = Daemon::with_client_and_event_source(
            DaemonConfig {
                scan_interval: Duration::from_secs(300),
                ..event_config(directory.path(), &local_root)
            },
            EventFakeClient::new(HashMap::new()),
            None,
        )
        .expect("daemon");
        daemon.event_source_factory = Box::new(|| None);

        daemon.pass().note_degraded_session_if_needed();
        let latched = daemon.session_declined.clone();
        let reason = latched
            .as_ref()
            .map(|declined| declined.reason.as_str())
            .expect("a degraded session latches its cause");
        assert!(
            !reason.contains("300"),
            "a process-wide reason must not carry one pair's scan interval: {reason}"
        );

        // A pair that does not want event-driven detection says nothing about the keyring.
        daemon.pair_mut().config.events_driven = false;
        daemon.pass().note_degraded_session_if_needed();
        assert_eq!(
            daemon.session_declined, latched,
            "a per-pair opt-out must not clear a process-wide cause"
        );

        // A different pair's cadence must not re-word the reason, because re-wording is re-logging.
        daemon.pair_mut().config.events_driven = true;
        daemon.pair_mut().config.scan_interval = Duration::from_secs(4321);
        daemon.pass().note_degraded_session_if_needed();
        assert_eq!(
            daemon.session_declined, latched,
            "the reason is a process fact, so two pairs' cadences cannot make it differ"
        );

        // And the process fact that ends the cause — a live session — does clear it.
        daemon.event_source = Some(Box::new(FakeEventSource::new("cursor-0")));
        daemon.pass().note_degraded_session_if_needed();
        assert_eq!(daemon.session_declined, None);
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
            &daemon.pair().connection,
            &directory_record("dir", Some("vol~ndir")),
        )
        .expect("seed dir record");
        for (name, digest) in ["a", "b", "c"].iter().zip(&digests) {
            upsert_record(
                &daemon.pair().connection,
                &base_record(
                    &format!("dir/{name}.txt"),
                    Some(&format!("vol~n{name}")),
                    digest,
                ),
            )
            .expect("seed file record");
        }
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;

        daemon
            .reconcile_blocking()
            .expect_clean("first incremental pass");
        assert_eq!(
            directory_lists.load(Ordering::SeqCst),
            1,
            "two events in one folder must share a single targeted listing"
        );

        daemon
            .reconcile_blocking()
            .expect_clean("second incremental pass");
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
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon.pair_mut().incremental_passes_since_full_scan = 0;
        daemon.pair_mut().is_first_reconcile = false;

        let base_records = load_index(&daemon.pair().connection).expect("load index");
        assert!(
            daemon.pass().should_try_incremental(&base_records),
            "a stored cursor alone must be enough to engage the event-driven gate"
        );
        daemon
            .reconcile_blocking()
            .expect_clean("incremental reconcile");

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
            daemon.pair().event_scope_declined.is_none(),
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
        daemon.pair_mut().is_first_reconcile = false;

        let base = HashMap::new();
        assert!(!daemon.pass().should_try_incremental(&base));
        let reported = daemon.pair().event_scope_declined.clone();
        assert!(
            reported
                .as_deref()
                .is_some_and(|reason| reason.contains("volume")),
            "the decline must name the volume as the missing input: {reported:?}"
        );
        // Same cause next pass → recorded reason is unchanged, so it is logged once.
        assert!(!daemon.pass().should_try_incremental(&base));
        assert_eq!(daemon.pair().event_scope_declined, reported);
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
            &daemon.pair().connection,
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
            &daemon.pair().connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        daemon
            .reconcile_blocking()
            .expect_clean("warm-start reconcile");

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
        let record = get_record(&daemon.pair().connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(edited),
            "the forced local scan still catches and uploads the offline edit"
        );
        assert_eq!(
            daemon.pair().warm_starts_since_full_walk,
            1,
            "a completed warm start advances the across-restart counter"
        );
        assert_eq!(
            load_warm_start_count(&daemon.pair().connection).expect("load count"),
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
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1)
            .expect("seed stale cursor");

        daemon
            .reconcile_blocking()
            .expect_clean("startup reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "a stale cursor must force a full walk, not a warm start"
        );
        let record = get_record(&daemon.pair().connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(edited),
            "the offline edit still syncs on the bootstrap's local scan"
        );
        assert_eq!(
            daemon.pair().warm_starts_since_full_walk,
            0,
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
            &daemon.pair().connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        // Pass 1: the warm start scans, plans the upload, and the upload fails → a partial pass.
        fail_next_upload.store(true, Ordering::SeqCst);
        daemon
            .reconcile_blocking()
            .expect_partial(1, "the first pass's upload fails");
        assert!(
            daemon.pair().is_first_reconcile,
            "a first pass that did not land everything must stay a first pass"
        );
        let record = get_record(&daemon.pair().connection, Path::new("keep.txt"))
            .expect("get record")
            .expect("keep.txt still recorded");
        assert_eq!(
            record.sha1_hash,
            Some(old),
            "the failed action committed nothing; the record keeps the old digest"
        );

        // Pass 2: still a first pass → warm-starts again, forcing the local scan; the upload now
        // succeeds and the edit lands. It never bootstrapped and never idle-skipped.
        daemon.reconcile_blocking().expect_clean("retry reconcile");
        assert!(
            !daemon.pair().is_first_reconcile,
            "a successful pass finally clears the first-pass flag"
        );
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "neither pass walked the whole remote tree"
        );
        let record = get_record(&daemon.pair().connection, Path::new("keep.txt"))
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
        daemon.pair_mut().config.warm_start.full_walk_every = 3;
        store_event_cursor(
            &daemon.pair().connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");
        // The machine has already warm-started up to the floor across prior boots.
        daemon.pair_mut().warm_starts_since_full_walk = 3;
        store_warm_start_count(&daemon.pair().connection, 3).expect("seed count");

        daemon.reconcile_blocking().expect_clean("reconcile");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "reaching the warm-start floor forces a full walk"
        );
        assert_eq!(
            daemon.pair().warm_starts_since_full_walk,
            0,
            "a full walk resets the warm-start floor"
        );
        assert_eq!(
            load_warm_start_count(&daemon.pair().connection).expect("load"),
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
        daemon.pair_mut().config.warm_start.force_full_walk = true;
        store_event_cursor(
            &daemon.pair().connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        daemon.reconcile_blocking().expect_clean("reconcile");

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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(
            &daemon.pair().connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");

        // First pass warm-starts (no walk).
        daemon
            .reconcile_blocking()
            .expect_clean("warm-start first pass");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            0,
            "the first pass warm-started, no walk yet"
        );

        // A resync request → the next pass full-walks.
        daemon
            .shared
            .pair
            .force_full_walk
            .store(true, Ordering::SeqCst);
        daemon
            .reconcile_blocking()
            .expect_clean("forced resync pass");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the resync request forces exactly one full walk"
        );

        // The latch was consumed: the following pass is incremental again.
        daemon
            .reconcile_blocking()
            .expect_clean("back to steady state");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "the force-full-walk latch is consumed after one pass"
        );
    }

    #[test]
    fn a_reset_index_request_discards_the_learned_state_and_rebuilds_by_adoption() {
        // G23/#237's *Reset the index*. Three claims at once: the baseline, cursor and approvals
        // are gone; the next pass full-walks (an empty index is the bootstrap condition, and the
        // cleared cursor must not be mistaken for a warm-start opportunity); and the user's files
        // are untouched — an already-agreeing pair is re-adopted, not re-transferred.
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
            &daemon.pair().connection,
            &base_record("keep.txt", Some("vol~nk"), keep.as_str()),
        )
        .expect("seed keep record");
        store_event_cursor(
            &daemon.pair().connection,
            "vol",
            "cursor-0",
            current_epoch_secs() as i64,
        )
        .expect("seed fresh cursor");
        upsert_delete_approval(
            &daemon.pair().connection,
            Path::new("keep.txt"),
            DeleteDirection::Local,
            "fingerprint",
            current_epoch_secs() as i64,
        )
        .expect("seed approval");

        daemon
            .reconcile_blocking()
            .expect_clean("warm-start first pass");
        assert_eq!(full_walks.load(Ordering::SeqCst), 0);
        daemon.pair_mut().is_first_reconcile = false;

        // Only the reset latch — NOT `force_full_walk`, which the IPC handler also sets. The full
        // walk below therefore proves the reset's own doing: a cleared cursor leaves nothing to
        // warm-start from, so the pass has to walk.
        daemon.shared.pair.reset_index.store(true, Ordering::SeqCst);
        daemon.reconcile_blocking().expect_clean("reset pass");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "a reset pass rebuilds from a full walk, never from the cursor it just cleared"
        );
        // The cursor was cleared and then RE-SEEDED by this pass, which is the point: a reset must
        // not leave the daemon permanently cursorless and full-walking. That the truncation itself
        // happens is pinned at the layer that does it —
        // `index::reset_index_state_clears_every_table_the_daemon_decides_from`.
        assert!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("cursor query")
                .is_some(),
            "a successful reset pass re-seeds the cursor from the walk it just did"
        );
        assert!(
            !matching_delete_approval(
                &daemon.pair().connection,
                Path::new("keep.txt"),
                DeleteDirection::Local,
                "fingerprint",
            )
            .expect("approval query"),
            "a standing approval is pinned to a baseline the reset erased"
        );
        assert!(
            local_root.join("keep.txt").exists(),
            "resetting the index must not touch the user's files"
        );
        let record = get_record(&daemon.pair().connection, Path::new("keep.txt"))
            .expect("record query")
            .expect("the rebuilt baseline re-adopts the already-agreeing pair");
        assert_eq!(record.sha1_hash.as_deref(), Some(keep.as_str()));
        assert!(
            !daemon.shared.pair.reset_index.load(Ordering::SeqCst),
            "the reset latch is consumed exactly once"
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
        daemon
            .reconcile_blocking()
            .expect_clean("bootstrap reconcile");
        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            1,
            "bootstrap performs exactly one full-tree walk"
        );
        assert_eq!(
            daemon.pair().incremental_passes_since_full_scan,
            0,
            "bootstrap resets the resync counter"
        );
        assert_eq!(
            derive_volume_id(&load_index(&daemon.pair().connection).expect("load index")),
            Some(volume.as_str()),
            "the seed gave the base index a composed proton_id so the incremental gate can engage"
        );
        let bootstrap_cursor = load_event_cursor(&daemon.pair().connection, &volume)
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
            daemon
                .reconcile_blocking()
                .expect_clean("incremental reconcile");
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
        let final_counter = daemon.pair().incremental_passes_since_full_scan;
        let final_cursor = load_event_cursor(&daemon.pair().connection, &volume)
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

    // ---- pass and path history (#213 / #229 / #238 / #230 / #190 / #191) ---------------------

    fn recorded_passes(daemon: &Daemon<impl ProtonClient>) -> Vec<crate::index::PassRecord> {
        recent_passes(&daemon.pair().connection, 100).expect("recent passes")
    }

    fn recorded_events(daemon: &Daemon<impl ProtonClient>) -> Vec<FileEvent> {
        file_events(&daemon.pair().connection, None, 0, 100)
            .expect("file events")
            .events
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_action_records_no_history_row_even_when_its_first_step_landed() {
        // `PassLog::truncate` on the failure path, which nothing else pins — deleting the call
        // leaves the rest of the suite green, because every *other* arm queues its row after the
        // last thing that can fail. `MoveLocal` does not: it renames, THEN notes the move, THEN
        // stats/hashes the destination to build the record, and that last step can fail. The
        // action is reported failed and its index mutation rolled back, so it replans from ground
        // truth next pass — and the history must not claim a move the index does not show.
        //
        // The failure is engineered without a seam, from the one asymmetry between the scan and the
        // executor: the scan reuses a base record's SHA-1 when size and mtime match (so it never
        // opens the file), while `local_file_state` in the executor passes an EMPTY known map and
        // therefore always re-hashes. An unreadable file is invisible to the first and fatal to the
        // second. `fs::rename` needs no permission on the file itself, so the move still lands.
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let old_path = local_root.join("old-name.txt");
        fs::write(&old_path, b"same content").expect("old local file");
        let hash = crate::index::compute_sha1(&old_path).expect("old hash");
        // Built from the file's real size + mtime so the scan's quick-check hits and the walk never
        // opens it; a mismatch here would fail the pass in the scan instead, proving nothing.
        let state = crate::index::local_file_state(&local_root, &old_path).expect("local state");
        let base = FileRecord::from_local(
            PathBuf::from("old-name.txt"),
            &state,
            Some("stable-id".to_owned()),
            SyncStatus::Synced,
        );

        let mut permissions = fs::metadata(&old_path).expect("metadata").permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&old_path, permissions).expect("make the file unreadable");
        if fs::read(&old_path).is_ok() {
            // Running as root (or on a filesystem that ignores the mode): the engineered failure is
            // unavailable, and a test that silently proves nothing is worse than one that says so.
            eprintln!(
                "skipping: this user can read a 0o000 file, so the post-rename hash cannot be \
                 made to fail"
            );
            return;
        }

        let mut remote_entities = HashMap::new();
        remote_entities.insert(
            PathBuf::from("new-name.txt"),
            RemoteEntity::File(remote("new-name.txt", "stable-id", Some(hash.as_str()))),
        );
        let (client, _operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(&daemon.pair().connection, &base).expect("base record");

        daemon
            .reconcile_blocking()
            .expect_partial(1, "the move's record cannot be built");

        // The rename itself landed — this is the case the truncate exists for.
        assert!(
            local_root.join("new-name.txt").is_file(),
            "precondition: the rename ran before the step that failed"
        );
        // …and the index still shows the old path, because the action's mutations were rolled back.
        assert!(
            get_record(&daemon.pair().connection, Path::new("new-name.txt"))
                .expect("new index lookup")
                .is_none(),
            "a failed action records nothing"
        );

        let events = recorded_events(&daemon);
        assert!(
            events.is_empty(),
            "the failed action's queued history row must be discarded with its index mutation: \
             {events:?}"
        );
        let passes = recorded_passes(&daemon);
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].outcome, "partial");
        assert_eq!(passes[0].failed, 1);
        assert_eq!(
            passes[0].changed, 0,
            "the rollup counts committed side effects, and none committed"
        );
    }

    #[test]
    fn a_full_sweep_that_changed_nothing_is_still_recorded() {
        // `Full sweep now` asks two things a changeless sweep is the answer to: when did the last
        // one run, and was anything out of step (#238). A pass row is written for every full
        // sweep even though the same pass, run incrementally, would write none.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect_clean("empty sweep");

        let passes = recorded_passes(&daemon);
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].kind, "full-sweep");
        assert_eq!(passes[0].outcome, "clean");
        assert_eq!(passes[0].changed, 0, "nothing was out of step");
        assert_eq!(passes[0].failed, 0);
        assert!(recorded_events(&daemon).is_empty(), "nothing moved");
        // And it is reachable as the answer to "when was the last full sweep".
        assert_eq!(
            last_full_sweep(&daemon.pair().connection)
                .expect("sweep")
                .map(|pass| pass.id),
            Some(passes[0].id)
        );
    }

    #[test]
    fn idle_passes_record_nothing_at_all() {
        // The load-bearing retention decision. With events-driven detection on by default the
        // daemon runs a pass every 30s — ~2900 a day — and recording each one identically would
        // evict every interesting row within minutes. "The daemon is alive" is already answered by
        // `reconcile_seq` and `last_sync_epoch_secs` on the live status reply.
        let directory = tempdir().expect("tempdir");
        let (mut daemon, _walks) = steady_state_event_daemon(&directory);
        // The shipped default: no periodic safety resync, so nothing after the startup snapshot
        // full-sweeps and every pass below is a genuinely idle incremental one.
        daemon.pair_mut().config.events_full_scan_every = 0;
        let after_bootstrap = recorded_passes(&daemon).len();

        for _ in 0..50 {
            daemon.reconcile_blocking().expect_clean("idle pass");
        }

        let passes = recorded_passes(&daemon);
        assert_eq!(
            passes.len(),
            after_bootstrap,
            "fifty idle passes must add no rows"
        );
        assert!(
            passes.iter().all(|pass| pass.kind == "full-sweep"),
            "only the startup sweep is recorded: {passes:?}"
        );
        // The daemon still reports that it is passing — that is a different surface.
        assert!(daemon.shared.pair.reconcile_seq.load(Ordering::SeqCst) >= 50);
    }

    #[test]
    fn a_pass_that_moved_something_records_it_with_its_bytes() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("note.txt"), b"0123456789").expect("local file");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon.reconcile_blocking().expect_clean("upload pass");

        let passes = recorded_passes(&daemon);
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].changed, 1);
        assert_eq!(passes[0].bytes_uploaded, 10);
        assert_eq!(passes[0].bytes_downloaded, 0);

        let events = recorded_events(&daemon);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, PathBuf::from("note.txt"));
        assert_eq!(events[0].action, SyncAction::Upload);
        assert_eq!(events[0].bytes, Some(10));
        assert_eq!(events[0].pass_id, passes[0].id, "the event names its pass");

        // #191 reads the rollup, over the window the pass started in.
        let totals = byte_totals_since(&daemon.pair().connection, 0).expect("totals");
        assert_eq!(totals.uploaded_bytes, 10);
        // …and it is what the status reply publishes.
        let status = daemon.status_response("daemon status");
        let history = status.history.expect("published history");
        assert_eq!(history.today.uploaded_bytes, 10);
        assert_eq!(history.recent.len(), 1);
    }

    #[test]
    fn a_partial_pass_is_recorded_as_partial_and_only_the_landed_action_has_an_event() {
        // #304's third outcome is a state of its own: the row says `partial`, carries the failed
        // count, and counts only what actually landed. A trailing arm folding it into either other
        // outcome is #246.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("first.txt"), b"first").expect("first");
        fs::write(local_root.join("second.txt"), b"second").expect("second");
        let (client, _operations) = RecordingProtonClient::with_failed_uploads(
            HashMap::new(),
            BTreeSet::from([PathBuf::from("second.txt")]),
        );
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_partial(1, "the second upload fails as an item");

        let passes = recorded_passes(&daemon);
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].outcome, "partial");
        assert_eq!(passes[0].failed, 1);
        assert_eq!(passes[0].changed, 1, "only the first upload landed");
        assert_eq!(passes[0].bytes_uploaded, 5);
        assert!(passes[0].error.is_some(), "a partial pass says what failed");

        // The failed action's history row is discarded with its index mutation — a history row
        // never describes a side effect that did not happen.
        let events = recorded_events(&daemon);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, PathBuf::from("first.txt"));
    }

    #[test]
    fn a_failed_pass_is_recorded_with_its_error() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let mut daemon = Daemon::with_client(
            test_config(directory.path(), &local_root),
            FailingListProtonClient,
        )
        .expect("daemon");

        daemon
            .reconcile_blocking()
            .expect_err("the remote listing fails");

        let passes = recorded_passes(&daemon);
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].outcome, "failed");
        assert_eq!(passes[0].changed, 0);
        assert!(
            passes[0]
                .error
                .as_deref()
                .is_some_and(|error| error.contains("list failed")),
            "the failure is named on the row: {:?}",
            passes[0].error
        );
        // `interrupted` is only ever reachable by a crash: every graceful ending, failures
        // included, seals the row.
        assert!(
            recorded_passes(&daemon)
                .iter()
                .all(|p| p.outcome != "interrupted")
        );
    }

    #[test]
    fn a_files_own_history_survives_it_being_moved() {
        // #190 looks history up by the path a file has NOW. Recording the move at its destination
        // is what keeps the query from missing the very event that moved it.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let new_path = local_root.join("new-name.txt");
        fs::write(&new_path, b"same content").expect("new local file");
        let hash = crate::index::compute_sha1(&new_path).expect("new hash");
        let remote_entities = HashMap::from([(
            PathBuf::from("old-name.txt"),
            RemoteEntity::File(remote("old-name.txt", "stable-id", Some(hash.as_str()))),
        )]);
        let (client, _operations) = RecordingProtonClient::with_remote_entities(remote_entities);
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        upsert_record(
            &daemon.pair().connection,
            &base_record("old-name.txt", Some("stable-id"), hash.as_str()),
        )
        .expect("base record");

        daemon.reconcile_blocking().expect_clean("rename pass");

        let at_destination = file_events(
            &daemon.pair().connection,
            Some(Path::new("new-name.txt")),
            0,
            10,
        )
        .expect("history for the new path");
        assert_eq!(at_destination.events.len(), 1);
        assert_eq!(at_destination.events[0].action, SyncAction::MoveRemote);
        assert_eq!(
            at_destination.events[0].source_path,
            Some(PathBuf::from("old-name.txt")),
            "where it came from is still on the row"
        );
        // A per-path query offers no byte total: the rollup is per pass and holds no paths, so
        // the only number it could give is the window's whole traffic — which would read as this
        // file's own.
        assert!(at_destination.totals.is_none());
        // Looking it up at the path it LEFT finds nothing, which is the point.
        assert_eq!(
            file_events(
                &daemon.pair().connection,
                Some(Path::new("old-name.txt")),
                0,
                10
            )
            .expect("history for the old path")
            .total,
            0
        );
    }

    #[test]
    fn the_live_pass_block_outlives_every_phase_change() {
        // #213: `SyncActivity::since_epoch_secs` restarts on each phase, so a client rendering it
        // as the pass's elapsed time counts backwards three times per pass. The pass block is
        // stamped onto every activity from one place and never reset by a phase.
        let shared = ControlShared::new(RunningConfigInfo {
            local_root: PathBuf::from("/local"),
            remote_root: PathBuf::from("/Drive/RemoteFolder"),
            db_path: PathBuf::from("/local/.sync/sync_index.db"),
        });
        shared.begin_pass_progress(1_000, PassKind::WarmStart);
        shared.begin_activity(new_activity(PHASE_SCANNING_LOCAL));
        shared.update_pass_progress(|pass| pass.changes = Some(3));
        shared.begin_activity(new_activity(PHASE_EXECUTING));
        // A phase started by the high-frequency walk callback keeps it too.
        shared.update_activity(PHASE_COMMITTING, |activity| {
            activity.folders_listed = Some(1)
        });

        let activity = shared.activity_for_response().expect("activity");
        assert_eq!(activity.phase, PHASE_COMMITTING);
        let pass = activity.pass.expect("pass block");
        assert_eq!(pass.started_epoch_secs, 1_000);
        assert_eq!(pass.changes, Some(3));
        assert_eq!(pass.kind, "warm-start");

        // The pass ending clears it, so a stale pass can never outlive its own reconcile.
        shared.clear_activity();
        assert!(shared.activity_for_response().is_none());
    }

    #[test]
    fn the_pass_kind_reaches_the_live_block_and_the_row_together() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _operations) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        daemon.shared.begin_pass_progress(1, PassKind::Incremental);
        daemon.pass().set_pass_kind(PassKind::FullSweep);

        assert_eq!(daemon.pair().pass_log.kind, PassKind::FullSweep);
        daemon.shared.begin_activity(new_activity(PHASE_EXECUTING));
        assert_eq!(
            daemon
                .shared
                .activity_for_response()
                .and_then(|activity| activity.pass)
                .map(|pass| pass.kind),
            Some("full-sweep".to_owned()),
            "one setter, so the row and the live block cannot disagree"
        );
    }

    #[test]
    fn a_non_utf8_path_is_queryable_through_its_wire_form() {
        // #61's rule, on the read side: a non-UTF-8 path reaches the user only as
        // `to_string_lossy`, so the string copied off the screen is not the stored key. A
        // byte-exact lookup answered "nothing moved" for a file that had moved — the same
        // "unknown rendered as zero" failure, one surface over.
        use std::os::unix::ffi::OsStrExt;
        let connection = Connection::open_in_memory().expect("db");
        crate::index::initialize_schema(&connection).expect("schema");
        let stored = PathBuf::from(std::ffi::OsStr::from_bytes(b"dir/re\xffport.txt"));
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        insert_file_events(
            &connection,
            pass,
            &[FileEvent {
                path: stored.clone(),
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(7),
                epoch_secs: 5,
                pass_id: pass,
            }],
        )
        .expect("insert");

        let selector = crate::wire_path(&stored).into_owned();
        assert!(
            selector.contains(char::REPLACEMENT_CHARACTER),
            "the fixture must actually be lossy on the wire"
        );
        let resolved = resolve_history_path(&connection, &selector, 0).expect("resolve");
        assert_eq!(resolved, stored, "the wire form found the stored key");
        let found = file_events(&connection, Some(&resolved), 0, 10).expect("events");
        assert_eq!(found.total, 1);

        // A UTF-8 selector still takes the point-query path, untouched.
        assert_eq!(
            resolve_history_path(&connection, "docs/spec.md", 0).expect("utf8"),
            PathBuf::from("docs/spec.md")
        );
        // And the guard still refuses a path that escapes the root.
        assert!(resolve_history_path(&connection, "../etc/passwd", 0).is_err());
    }

    #[test]
    fn an_ambiguous_lossy_history_selector_resolves_to_neither() {
        // Two different stored paths, one rendering. Read-only or not, picking one arbitrarily
        // would answer a question the selector did not ask.
        use std::os::unix::ffi::OsStrExt;
        let connection = Connection::open_in_memory().expect("db");
        crate::index::initialize_schema(&connection).expect("schema");
        let first = PathBuf::from(std::ffi::OsStr::from_bytes(b"re\xffport.txt"));
        let second = PathBuf::from(std::ffi::OsStr::from_bytes(b"re\xfeport.txt"));
        let pass = begin_pass(&connection, 0, PassKind::Incremental).expect("begin");
        let events: Vec<FileEvent> = [first.clone(), second.clone()]
            .into_iter()
            .map(|path| FileEvent {
                path,
                source_path: None,
                action: SyncAction::Upload,
                bytes: Some(1),
                epoch_secs: 5,
                pass_id: pass,
            })
            .collect();
        insert_file_events(&connection, pass, &events).expect("insert");

        let selector = crate::wire_path(&first).into_owned();
        assert_eq!(
            selector,
            crate::wire_path(&second),
            "one rendering, two paths"
        );
        let error = resolve_history_path(&connection, &selector, 0)
            .expect_err("an ambiguous selector must not resolve");
        assert!(
            error.to_string().contains("cannot say which one"),
            "the reason names the ambiguity: {error}"
        );
    }

    #[test]
    fn todays_window_starts_at_local_midnight() {
        let now = current_epoch_secs();
        let start = local_day_start(now);
        assert!(start <= now, "the day started before now");
        // 26h, not 24h: a fall-back DST day is 25 hours long, and pinning this at exactly one day
        // would make the test fail once a year on a correct answer.
        assert!(
            now - start < 26 * 3600,
            "and no more than one calendar day ago: {start} vs {now}"
        );
        // The boundary is a whole minute past the epoch in every timezone offset in use.
        assert_eq!(start % 60, 0);
    }

    #[test]
    fn the_day_boundary_survives_a_dst_transition() {
        // The bug `mktime` replaces: `now - (h*3600 + m*60 + s)` assumes the offset at midnight is
        // the offset now, so it lands an hour off on both transition days. Driven over a whole
        // year of local noons, which crosses whatever transitions this machine's zone has.
        let mut previous = None;
        for day in 0..366u64 {
            // 2024-01-01T12:00:00Z plus one day at a time.
            let noonish = 1_704_110_400 + day * 86_400;
            let start = local_day_start(noonish);
            assert!(start <= noonish, "day {day}: boundary is not in the future");
            assert!(
                noonish - start < 26 * 3600,
                "day {day}: boundary more than a long day back ({start} vs {noonish})"
            );
            // Consecutive days must land on distinct, increasing boundaries — the failure the old
            // arithmetic produced was two days sharing one, or a boundary going backwards.
            if let Some(previous) = previous {
                assert!(
                    start > previous,
                    "day {day}: boundary did not advance ({previous} -> {start})"
                );
            }

            previous = Some(start);
        }
    }

    // ---- the plan verb, the token, and the filtered apply (#100 / #192 / #209) -----------------

    /// Runs one plan-only pass exactly as `LoopCommand::PlanNow` does. Multi-thread, because
    /// `plan_now` blocks the pass with `block_in_place`.
    fn run_plan_pass<C: ProtonClient>(daemon: &mut Daemon<C>) {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(daemon.plan_now());
    }

    fn computed_plan(shared: &ControlShared) -> ReviewedPlan {
        match shared.plan_outcome(None) {
            PlanOutcome::Computed(plan) => *plan,
            other => panic!("expected a computed plan, got {other:?}"),
        }
    }

    /// A daemon with a live event source, a stored cursor, and one file on each side that differ —
    /// so a real pass would plan, execute, drain and advance. Everything a plan pass must leave
    /// alone is set up here so the test can prove it did.
    fn plan_verb_daemon(directory: &tempfile::TempDir) -> Daemon<EventFakeClient> {
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("local-only.txt"), b"local").expect("local file");
        let remote_entities = HashMap::from([(
            PathBuf::from("remote-only.txt"),
            remote_file_entity("remote-only.txt", "vol~nr", sha1_bytes(b"remote").as_str()),
        )]);
        let mut daemon = Daemon::with_client_and_event_source(
            event_config(directory.path(), &local_root),
            EventFakeClient::new(remote_entities),
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        // TEST LANDMINE: `with_client_and_event_source` installs the default factory, which reads
        // the REAL OS keyring whenever `events_driven` is on. Pin it, or a machine with an
        // unlocked keyring silently acquires a live source and the test passes for another reason.
        daemon.event_source_factory = Box::new(|| None);
        daemon
    }

    /// The whole inertness family of the plan verb, in one place because it is one rule: a
    /// rehearsal that spent evidence would leave the real pass unable to re-derive what it
    /// rehearsed. One assertion per bullet of `plan_now`'s doc.
    #[test]
    fn a_plan_pass_observes_and_consumes_nothing() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");
        daemon
            .pair_mut()
            .pending_changes
            .insert(PathBuf::from("local-only.txt"));
        daemon.pair_mut().pending_deletions.push(PendingDeletion {
            path: PathBuf::from("gone.txt"),
            direction: DeleteDirection::Local,
            entity_kind: EntityKind::File,
            fingerprint: "hash".to_owned(),
            detected_epoch_secs: 1,
            first_seen_epoch_secs: 1,
            subtree_files: None,
            subtree_bytes: None,
            disposal: LocalDisposal::Permanent,
        });
        daemon.pair_mut().warm_starts_since_full_walk = 3;
        daemon.pair_mut().force_local_rescan = true;
        let seq_before = daemon.shared.pair.reconcile_seq.load(Ordering::SeqCst);
        let plan_seq = daemon.shared.book_plan_request();

        run_plan_pass(&mut daemon);

        let plan = computed_plan(&daemon.shared);
        assert_eq!(
            plan.plan_seq, plan_seq,
            "the pass answers the request booked"
        );
        assert_eq!(
            plan.total, 2,
            "one upload and one download: {:?}",
            plan.actions
        );
        assert!(!plan.token.is_empty());

        // The event cursor: never read, never written. A plan pass full-walks the remote precisely
        // so it cannot consume a delta.
        let cursor = load_event_cursor(&daemon.pair().connection, "vol")
            .expect("cursor read")
            .expect("cursor row");
        assert_eq!(
            cursor.last_event_id, "cursor-0",
            "a rehearsal must not advance the event cursor"
        );
        // Nothing drained, nothing cleared, no counter moved.
        assert_eq!(daemon.pair().pending_changes.len(), 1);
        assert_eq!(daemon.pair().pending_deletions.len(), 1);
        assert!(
            daemon.pair().is_first_reconcile,
            "a rehearsal is not a first pass"
        );
        assert!(
            daemon.pair().force_local_rescan,
            "a rehearsal is not a rescan"
        );
        assert_eq!(daemon.pair().warm_starts_since_full_walk, 3);
        assert_eq!(
            load_warm_start_count(&daemon.pair().connection).expect("counter"),
            0,
            "the persisted counter is untouched too"
        );
        // Nothing synced, so nothing may say a sync happened.
        assert!(daemon.pair().last_sync.is_none());
        assert!(daemon.pair().last_successful_sync_summary.is_none());
        assert!(daemon.pair().last_plan_summary.is_none());
        // No history: history is written behind a side effect, and there was none.
        assert!(recorded_passes(&daemon).is_empty());
        assert!(recorded_events(&daemon).is_empty());
        // No index rows, either direction.
        assert!(
            get_record(&daemon.pair().connection, Path::new("local-only.txt"))
                .expect("lookup")
                .is_none()
        );
        // And nothing was transferred: the local tree is exactly as it was.
        assert!(
            !daemon
                .pair()
                .config
                .local_root
                .join("remote-only.txt")
                .exists()
        );
        // `reconcile_seq` deliberately does not move: a `syncnow` watcher polling it must not be
        // satisfied by a pass that synced nothing.
        assert_eq!(
            daemon.shared.pair.reconcile_seq.load(Ordering::SeqCst),
            seq_before
        );
    }

    /// #209: the plan pass runs on the main loop, so the existing activity surface describes it —
    /// and says which pass it is, so a Plan screen cannot read a coincidental sync's progress as
    /// its own rehearsal's.
    #[test]
    fn a_plan_pass_is_visible_as_a_plan_on_the_activity_surface() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        daemon.shared.begin_plan_progress(42);
        daemon.shared.pair.syncing.store(true, Ordering::SeqCst);
        daemon
            .shared
            .begin_activity(new_activity(PHASE_SCANNING_LOCAL));
        daemon
            .shared
            .update_activity(PHASE_SCANNING_LOCAL, |activity| {
                activity.files_scanned = Some(8_431);
            });

        let response = daemon.status_response("daemon status");
        let activity = response
            .activity
            .expect("activity while a pass is in flight");
        assert_eq!(activity.phase, PHASE_SCANNING_LOCAL);
        assert_eq!(activity.files_scanned, Some(8_431));
        let pass = activity.pass.expect("pass block");
        assert_eq!(
            pass.kind, PLAN_PASS_KIND,
            "the live block must name the rehearsal, not a sync kind"
        );
        assert_eq!(pass.started_epoch_secs, 42);
        // The denominator #209 wants is the corpus size, which already rides on every reply (#207).
        assert!(response.index_totals.is_some());
    }

    /// Two `Check again` clicks while one walk is running cost one walk.
    #[test]
    fn a_second_plan_request_while_one_is_running_is_answered_by_that_pass() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        let first = daemon.shared.book_plan_request();
        let second = daemon.shared.book_plan_request();
        assert_eq!((first, second), (1, 2));

        run_plan_pass(&mut daemon);
        assert_eq!(
            computed_plan(&daemon.shared).plan_seq,
            2,
            "one pass answers every request booked before it started"
        );
        // The queued duplicate finds nothing left to do.
        assert!(daemon.shared.claim_plan_pass().is_none());
        let walks_before = daemon.proton.full_walks.load(Ordering::SeqCst);
        run_plan_pass(&mut daemon);
        assert_eq!(
            daemon.proton.full_walks.load(Ordering::SeqCst),
            walks_before,
            "a duplicate PlanNow must not spend a second remote walk"
        );
    }

    #[test]
    fn a_plan_result_before_any_plan_is_absent_rather_than_an_empty_plan() {
        let directory = tempdir().expect("tempdir");
        let daemon = plan_verb_daemon(&directory);
        assert_eq!(daemon.shared.plan_outcome(None), PlanOutcome::Absent);
        assert!(daemon.shared.apply_outcome().is_none());
    }

    /// #100 happy path: the token names the plan, the pass re-plans, the plans agree, and the work
    /// runs. Not a replay — the rows executed are the ones this pass computed.
    #[test]
    fn applying_a_reviewed_plan_executes_it() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let token = computed_plan(&daemon.shared).token;

        let apply_seq = daemon
            .shared
            .book_apply_request(&token, false)
            .expect("the current plan's token must be accepted");
        daemon
            .reconcile_blocking()
            .expect_clean("an apply of an unchanged plan runs cleanly");

        match daemon.shared.apply_outcome().expect("verdict") {
            ApplyOutcome::Applied {
                apply_seq: sealed,
                executed,
                skipped_destructive,
                failed,
            } => {
                assert_eq!(sealed, apply_seq);
                assert_eq!((skipped_destructive, failed), (0, 0));
                assert_eq!(executed, 2, "one upload and one download");
            }
            other => panic!("expected an applied verdict, got {other:?}"),
        }
        assert!(
            daemon
                .pair()
                .config
                .local_root
                .join("remote-only.txt")
                .exists(),
            "the reviewed download must have landed"
        );
    }

    /// #100's whole point: the plan moved between review and apply, so **nothing runs**, and the
    /// fresh plan is published for re-review rather than silently applied.
    #[test]
    fn an_apply_whose_plan_moved_executes_nothing_and_publishes_the_new_plan() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let reviewed = computed_plan(&daemon.shared);

        // The tree moves under the reviewed plan: a second local file appears.
        fs::write(daemon.pair().config.local_root.join("appeared.txt"), b"new").expect("new file");

        let apply_seq = daemon
            .shared
            .book_apply_request(&reviewed.token, false)
            .expect("the token is still the stored one");
        let outcome = daemon.reconcile_blocking();
        assert!(
            outcome.is_err(),
            "a diverged apply must not report a clean pass: `Clean` sets `last_sync`, which draws \
             'everything is up to date' over a plan still waiting"
        );

        match daemon.shared.apply_outcome().expect("verdict") {
            ApplyOutcome::Diverged { apply_seq: sealed } => assert_eq!(sealed, apply_seq),
            other => panic!("expected a divergence, got {other:?}"),
        }
        // Nothing ran…
        assert!(
            !daemon
                .pair()
                .config
                .local_root
                .join("remote-only.txt")
                .exists()
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("local-only.txt"))
                .expect("lookup")
                .is_none()
        );
        // …and the pass consumed nothing it would need again.
        assert!(daemon.pair().is_first_reconcile);
        assert!(daemon.pair().last_sync.is_none());
        // The new plan is what a re-review finds, under a new token.
        let fresh = computed_plan(&daemon.shared);
        assert_ne!(fresh.token, reviewed.token);
        assert_eq!(fresh.total, 3, "the appeared file is in the new plan");
        // And the stale token now authorises nothing.
        assert!(
            daemon
                .shared
                .book_apply_request(&reviewed.token, false)
                .is_none(),
            "one stored plan at a time, latest wins"
        );
    }

    #[test]
    fn an_apply_naming_no_plan_at_all_is_refused_without_scheduling_anything() {
        let directory = tempdir().expect("tempdir");
        let daemon = plan_verb_daemon(&directory);
        let (loop_tx, mut loop_rx) = mpsc::unbounded_channel();

        for token in [None, Some("1:never-computed")] {
            let outcome = schedule_apply(&daemon.shared, &loop_tx, token, false);
            assert_eq!(outcome, ApplyOutcome::Stale, "token: {token:?}");
        }
        assert!(
            loop_rx.try_recv().is_err(),
            "a refused apply must schedule no pass at all"
        );
    }

    #[test]
    fn a_paused_daemon_applies_nothing() {
        let directory = tempdir().expect("tempdir");
        let daemon = plan_verb_daemon(&directory);
        daemon.shared.pair.paused.store(true, Ordering::SeqCst);
        let (loop_tx, mut loop_rx) = mpsc::unbounded_channel();
        // Same pause semantics as `syncnow`, whose refusal this mirrors: a paused daemon runs no
        // passes, and a plan is a pass.
        assert_eq!(
            schedule_apply(&daemon.shared, &loop_tx, Some("1:whatever"), false),
            ApplyOutcome::Paused
        );
        assert!(loop_rx.try_recv().is_err());
    }

    /// #192, and the reason it is a skip at the executor rather than a filter over the plan: the
    /// deletion is dropped for this run, but everything the gate computes from the full plan —
    /// the pending list, and the first-seen ages behind it — is exactly what an ordinary pass
    /// computes.
    #[test]
    fn a_filtered_apply_drops_the_deletion_and_leaves_the_approval_standing() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("doomed.txt"), b"doomed").expect("local file");
        fs::write(local_root.join("fresh.txt"), b"fresh").expect("local file");
        // `doomed.txt` is synced and gone from the remote → a LocalDelete; `fresh.txt` is new →
        // an Upload. The guard is ON, and the deletion is pre-approved, so an ordinary apply would
        // delete it.
        let config = DaemonConfig {
            delete_approval_local: true,
            ..event_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            EventFakeClient::new(HashMap::new()),
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        daemon.event_source_factory = Box::new(|| None);
        let doomed = sha1_bytes(b"doomed");
        upsert_record(
            &daemon.pair().connection,
            &base_record("doomed.txt", Some("vol~nd"), doomed.as_str()),
        )
        .expect("seed record");
        upsert_delete_approval(
            &daemon.pair().connection,
            Path::new("doomed.txt"),
            DeleteDirection::Local,
            &doomed,
            1,
        )
        .expect("seed approval");
        store_event_cursor(&daemon.pair().connection, "vol", "cursor-0", 1).expect("seed cursor");

        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let reviewed = computed_plan(&daemon.shared);
        assert_eq!(reviewed.summary.destructive_actions, 1);

        daemon
            .shared
            .book_apply_request(&reviewed.token, true)
            .expect("token accepted");
        daemon
            .reconcile_blocking()
            .expect_clean("a filtered apply runs its non-destructive rows cleanly");

        match daemon.shared.apply_outcome().expect("verdict") {
            ApplyOutcome::Applied {
                skipped_destructive,
                failed,
                ..
            } => {
                assert_eq!(skipped_destructive, 1);
                assert_eq!(failed, 0);
            }
            other => panic!("expected an applied verdict, got {other:?}"),
        }
        // The deletion did not happen even though an approval authorised it: the user asked for
        // THIS run without it, which is a different question.
        assert!(
            local_root.join("doomed.txt").exists(),
            "a filtered apply must drop the deletion even with a standing approval"
        );
        assert!(
            get_record(&daemon.pair().connection, Path::new("doomed.txt"))
                .expect("lookup")
                .is_some(),
            "the skipped deletion's index row must survive, so it re-plans"
        );
        // The approval is left standing and unspent — it authorises a future pass, not this one.
        assert_eq!(
            crate::index::load_delete_approvals(&daemon.pair().connection)
                .expect("approvals")
                .len(),
            1
        );
        // …and the cursor is held, exactly as a withheld deletion holds it: the dropped action must
        // keep re-deriving from ground truth.
        assert_eq!(
            load_event_cursor(&daemon.pair().connection, "vol")
                .expect("cursor read")
                .expect("cursor row")
                .last_event_id,
            "cursor-0",
            "a dropped destructive row holds the event cursor"
        );
    }

    /// Filtering the plan *before* the gate would blank the pending list and wipe the #225
    /// first-seen ages. Skipping at the executor leaves both exactly as an ordinary pass computes
    /// them.
    #[test]
    fn a_filtered_apply_still_reports_the_deletions_it_skipped_as_pending() {
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        fs::write(local_root.join("doomed.txt"), b"doomed").expect("local file");
        let config = DaemonConfig {
            delete_approval_local: true,
            ..event_config(directory.path(), &local_root)
        };
        let mut daemon = Daemon::with_client_and_event_source(
            config,
            EventFakeClient::new(HashMap::new()),
            Some(Box::new(FakeEventSource::new("cursor-0"))),
        )
        .expect("daemon");
        daemon.event_source_factory = Box::new(|| None);
        upsert_record(
            &daemon.pair().connection,
            &base_record("doomed.txt", Some("vol~nd"), sha1_bytes(b"doomed").as_str()),
        )
        .expect("seed record");

        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let reviewed = computed_plan(&daemon.shared);
        daemon
            .shared
            .book_apply_request(&reviewed.token, true)
            .expect("token accepted");
        daemon
            .reconcile_blocking()
            .expect_clean("a filtered apply of a wholly destructive plan is still a clean pass");

        assert_eq!(
            daemon.pair().pending_deletions.len(),
            1,
            "the gate still ran over the full plan, so the Deletions screen is unchanged"
        );
        let ages = load_withheld_deletions(&daemon.pair().connection).expect("ages");
        assert_eq!(ages.len(), 1, "the first-seen ages are not wiped");
        assert_eq!(ages[0].path, PathBuf::from("doomed.txt"));
    }

    /// A destructive row must never fall off the reply window: `files_at_risk` is safety copy, and
    /// a plan whose deletions were truncated away reads as safe.
    #[test]
    fn the_plan_window_never_truncates_a_destructive_row() {
        let stored = StoredPlan {
            token: "1:abc".to_owned(),
            computed_epoch_secs: 1,
            summary: PlanSummary::default(),
            local_disposal: LocalDisposal::Permanent,
            actions: (0..10)
                .map(|index| {
                    PlannedAction::new(
                        Path::new(&format!("upload-{index}.txt")),
                        SyncAction::Upload,
                        EntityKind::File,
                        None,
                    )
                })
                .chain(std::iter::once(PlannedAction::new(
                    Path::new("doomed.txt"),
                    SyncAction::LocalDelete,
                    EntityKind::File,
                    None,
                )))
                .collect(),
            cannot_sync: Vec::new(),
        };

        let PlanOutcome::Computed(window) = stored.outcome(1, Some(2)) else {
            panic!("expected a computed plan");
        };
        assert_eq!(window.total, 11);
        assert!(window.truncated);
        assert!(
            window
                .actions
                .iter()
                .any(|action| action.action == SyncAction::LocalDelete),
            "the deletion must survive a window that truncated nine uploads: {:?}",
            window.actions
        );
        assert_eq!(window.actions.len(), 3, "one deletion plus the capped rest");
    }

    /// The reply's real bound, pinned because the doc states it: **`destructive + limit`**, not
    /// `limit`. A cap that reached the destructive rows would be the one truncation this window
    /// exists to prevent, so a plan with more deletions than the cap sends all of them.
    #[test]
    fn the_action_window_bounds_the_ordinary_rows_and_never_the_destructive_ones() {
        let stored = StoredPlan {
            token: "1:abc".to_owned(),
            computed_epoch_secs: 1,
            summary: PlanSummary::default(),
            local_disposal: LocalDisposal::Permanent,
            actions: (0..40)
                .map(|index| {
                    PlannedAction::new(
                        Path::new(&format!("doomed-{index}.txt")),
                        SyncAction::LocalDelete,
                        EntityKind::File,
                        None,
                    )
                })
                .chain((0..40).map(|index| {
                    PlannedAction::new(
                        Path::new(&format!("upload-{index}.txt")),
                        SyncAction::Upload,
                        EntityKind::File,
                        None,
                    )
                }))
                .collect(),
            cannot_sync: Vec::new(),
        };

        let PlanOutcome::Computed(window) = stored.outcome(1, Some(5)) else {
            panic!("expected a computed plan");
        };
        assert_eq!(
            window.actions.len(),
            45,
            "forty deletions the cap may not reach, plus five of the rest"
        );
        assert!(
            window.actions.len() > 5,
            "the limit bounds the tail, not the reply — a client sizing a buffer reads the sum"
        );
        assert_eq!(
            window
                .actions
                .iter()
                .filter(|action| action.action.is_destructive())
                .count(),
            40,
            "every destructive row survives whatever the cap"
        );
        assert!(window.truncated);
        assert_eq!(window.total, 80);
    }

    /// An apply that dies before the comparison must still seal a verdict, or its client polls a
    /// request nothing will ever answer.
    #[test]
    fn an_apply_that_fails_before_deciding_still_seals_a_verdict() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let token = computed_plan(&daemon.shared).token;
        let apply_seq = daemon
            .shared
            .book_apply_request(&token, false)
            .expect("token accepted");
        // The local scan now fails, so the pass never reaches the token comparison.
        fs::remove_dir_all(&daemon.pair().config.local_root).expect("remove the local root");

        assert!(daemon.reconcile_blocking().is_err());
        match daemon.shared.apply_outcome().expect("verdict") {
            ApplyOutcome::Failed {
                apply_seq: sealed, ..
            } => assert_eq!(sealed, apply_seq),
            other => panic!("expected a failed verdict, got {other:?}"),
        }
        // And the intent is gone: the next pass is an ordinary sync, not a second attempt.
        assert_eq!(daemon.pair().pass_intent, PassIntent::Sync);
    }

    /// A rehearsal claims `syncing` but bumps no `reconcile_seq`, so a `syncnow` issued during one
    /// must be told "scheduled", not "already in progress" — its client counts passes by that
    /// sequence, and would otherwise wait for a number the queued sync cannot reach alone.
    #[test]
    fn a_syncnow_during_a_rehearsal_is_told_it_is_scheduled_not_queued_behind_a_pass() {
        let directory = tempdir().expect("tempdir");
        let daemon = plan_verb_daemon(&directory);

        // Idle: nothing is running.
        assert!(!daemon.shared.a_counted_pass_is_running());

        // A rehearsal in flight. It MUST claim `syncing` (or `activity` is gated off every status
        // reply and #209's progress line has nothing to read) …
        daemon.shared.pair.plan_pass.store(true, Ordering::SeqCst);
        daemon.shared.pair.syncing.store(true, Ordering::SeqCst);
        assert!(
            daemon.shared.is_syncing(),
            "a rehearsal must still publish its activity"
        );
        // … and must NOT read as a pass a `syncnow` can be queued behind.
        assert!(!daemon.shared.a_counted_pass_is_running());
        assert_eq!(daemon.shared.response("x").status, "syncing");

        // A real pass is both.
        daemon.shared.pair.plan_pass.store(false, Ordering::SeqCst);
        assert!(daemon.shared.a_counted_pass_is_running());

        // And the flag is cleared inside `syncing`'s window, so no reply can see `syncing` without
        // knowing which kind of pass it is.
        daemon.shared.pair.syncing.store(false, Ordering::SeqCst);
        assert!(!daemon.shared.a_counted_pass_is_running());
    }

    /// An apply overtaken by a pause is cancelled, not latched: a destructive request must not fire
    /// on some later interval tick, hours after the user paused to stop things happening.
    #[test]
    fn a_pause_cancels_an_apply_it_overtook_rather_than_latching_it() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let plan = computed_plan(&daemon.shared);
        let apply_seq = daemon
            .shared
            .book_apply_request(&plan.token, false)
            .expect("token accepted");
        // The pause lands after the ack, so the loop reaches its paused early-return.
        daemon.shared.pair.paused.store(true, Ordering::SeqCst);

        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(daemon.reconcile_if_needed())
            .expect("a paused tick is not an error");

        // Cancelled and sealed, so the client stops polling…
        match daemon.shared.apply_outcome().expect("verdict") {
            ApplyOutcome::Failed {
                apply_seq: sealed, ..
            } => assert_eq!(sealed, apply_seq),
            other => panic!("expected a cancelled apply, got {other:?}"),
        }
        assert!(
            !daemon
                .pair()
                .config
                .local_root
                .join("remote-only.txt")
                .exists()
        );

        // …and resuming does NOT fire it: the intent is gone, so the next pass is an ordinary sync.
        daemon.shared.pair.paused.store(false, Ordering::SeqCst);
        assert_eq!(daemon.shared.take_apply_request(), PassIntent::Sync);
        // The plan itself survives, so the same token applies again once syncing resumes.
        assert!(
            daemon
                .shared
                .book_apply_request(&plan.token, false)
                .is_some()
        );
    }

    /// The **other half of that asymmetry**, and the reason it is not an inconsistency to be fixed:
    /// a pause cancels a booked *apply*, but a booked *plan* still runs to completion.
    ///
    /// `LoopCommand::PlanNow` goes straight to [`Daemon::plan_now`], which deliberately has no
    /// pause check — the check lives at the ack (`ControlCommand::Plan` → `PlanOutcome::Paused`),
    /// so a pause landing *after* the ack finds a request already booked, and only the pass can
    /// seal it. **Adding a pause guard to `plan_now` as an obvious consistency fix would orphan
    /// that request**: `plan_outcome` answers `Computing` for ever while `requested > completed`,
    /// and the GUI's plan poller (`plan_through_daemon`) has neither a paused bail-out nor a
    /// timeout — it would hang a `spawn_blocking` thread with the Plan screen stuck on `Checking…`.
    /// The apply may be cancelled precisely because cancelling it *is* a seal.
    ///
    /// Safe because the pass is **inert**: it runs through the pause and still syncs nothing, which
    /// is what `a_plan_pass_observes_and_consumes_nothing` proves in full.
    #[test]
    fn a_plan_pass_booked_before_a_pause_still_runs_and_seals() {
        let directory = tempdir().expect("tempdir");
        let mut daemon = plan_verb_daemon(&directory);
        let plan_seq = daemon.shared.book_plan_request();
        // The pause lands after the ack booked the request — the one window the ack cannot cover.
        daemon.shared.pair.paused.store(true, Ordering::SeqCst);

        run_plan_pass(&mut daemon);

        // Sealed: a poller that acked before the pause is answered rather than left on `Computing`.
        let plan = computed_plan(&daemon.shared);
        assert!(
            plan.plan_seq >= plan_seq,
            "the pass must answer the request booked before the pause ({} < {plan_seq})",
            plan.plan_seq
        );
        // …and the rehearsal really was one: it planned work and performed none of it, so running a
        // plan through a pause never moves a byte.
        assert!(
            plan.total > 0,
            "precondition: the fixture's two sides differ, so there is something to plan"
        );
        assert!(
            !daemon
                .pair()
                .config
                .local_root
                .join("remote-only.txt")
                .exists()
        );
    }

    /// The events-mode idle fast-path returns before `execute_plan_and_commit`, so an apply that
    /// landed on an idle cycle would neither execute nor report — and its client would poll for
    /// ever. A pass asked to apply always plans.
    #[test]
    fn an_apply_landing_on_an_idle_cycle_still_reaches_the_comparison() {
        let directory = tempdir().expect("tempdir");
        let (mut daemon, full_walks) = steady_state_event_daemon(&directory);
        daemon.event_source_factory = Box::new(|| None);
        daemon.shared.book_plan_request();
        run_plan_pass(&mut daemon);
        let plan = computed_plan(&daemon.shared);
        assert_eq!(plan.total, 0, "precondition: both sides already match");
        let walks_after_planning = full_walks.load(Ordering::SeqCst);

        let apply_seq = daemon
            .shared
            .book_apply_request(&plan.token, false)
            .expect("token accepted");
        daemon
            .reconcile_blocking()
            .expect_clean("an apply of an empty plan is a clean pass");

        assert_eq!(
            full_walks.load(Ordering::SeqCst),
            walks_after_planning,
            "the apply still runs as an incremental pass; it is only the idle SKIP that is suppressed"
        );
        match daemon.shared.apply_outcome().expect("verdict") {
            ApplyOutcome::Applied {
                apply_seq: sealed,
                executed,
                ..
            } => {
                assert_eq!(sealed, apply_seq);
                assert_eq!(executed, 0, "there was nothing to do, and it said so");
            }
            other => panic!("an idle apply must still report a verdict, got {other:?}"),
        }
    }
}
