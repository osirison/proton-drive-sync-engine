use crate::events::{EventSource, EventsClient, RemoteChange, node_uid, volume_id_from_proton_id};
use crate::index::{
    EntityKind, FileRecord, LocalEntityState, LocalFileState, ScanOptions, SyncStatus,
    load_event_cursor, load_existing_index, load_index, local_directory_state, local_file_state,
    mark_modified, open_database, path_for_proton_id, purge_record,
    scan_local_entities_reusing_hashes, store_event_cursor, upsert_record,
};
use crate::ipc::{
    ControlCommand, ControlRequest, ControlResponse, StatusHistoryEntry, bind_listener,
    read_request, write_response,
};
use crate::proton::{
    CommandPolicy, ProtonClient, ProtonDriveClient, RemoteEntity, RemoteListingStatus,
};
use crate::reconstruct::{Reconstruction, RemoteChangeResolver, reconstruct_remote};
use crate::session::{CliKeyringSession, CurlHttpTransport};
use crate::sync::{
    PlanSummary, PlannedAction, SyncAction, directory_move_descendant_path_pairs,
    is_strict_descendant, original_from_conflict_copy, plan_sync_entities,
};
use crate::{AppResult, boxed_error};
use fs2::FileExt;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const STATUS_HISTORY_LIMIT: usize = 20;
/// Time budget for a single control-connection read or write. Bounds how long a stalled
/// or idle client can occupy the daemon's single-threaded event loop before it is
/// dropped, so a silent client cannot indefinitely block reconciles, filesystem events,
/// or graceful shutdown.
const IPC_IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the daemon polls the volume event stream when `events_driven` is enabled. Matches
/// the ~30s cadence Proton's own client uses (ADR 0001). Only the incremental (O(changes)) path
/// runs this often; full-tree snapshots stay on `scan_interval` and the periodic safety resync.
const EVENTS_POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    pub lockfile_path: PathBuf,
    pub scan_interval: Duration,
    pub proton_cli: PathBuf,
    pub proton_timeout: Duration,
    pub proton_list_attempts: usize,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    /// Opt-in: detect remote changes from Proton's volume event stream (O(changes)) instead of
    /// re-walking the whole remote tree (O(folders)) every pass. Default `false` keeps today's
    /// full-scan behavior byte-for-byte. See `docs/adr/0001-*`.
    pub events_driven: bool,
    /// When event-driven, force a full-tree reconvergence snapshot every N incremental passes.
    /// Bounds any completeness gap inherited from a missed snapshot item or the reuse-session
    /// staleness window, and backfills `proton_id` for just-uploaded nodes. Clamped to `>= 1`.
    pub events_full_scan_every: u64,
}

pub struct Daemon<C: ProtonClient = ProtonDriveClient> {
    config: DaemonConfig,
    connection: Connection,
    proton: C,
    pending_changes: BTreeSet<PathBuf>,
    scan_options: ScanOptions,
    paused: bool,
    last_sync: Option<SystemTime>,
    last_error: Option<String>,
    last_plan_summary: Option<PlanSummary>,
    last_successful_sync_summary: Option<PlanSummary>,
    status_history_path: PathBuf,
    metrics_path: PathBuf,
    status_history: Vec<StatusHistoryEntry>,
    ipc_io_timeout: Duration,
    /// Remote change detection via the volume event stream. `None` when `events_driven` is off
    /// (or the session could not be read), in which case every reconcile is a full-tree snapshot
    /// exactly as before this feature.
    event_source: Option<Box<dyn EventSource>>,
    /// Number of successful incremental (event-driven) passes since the last full-tree snapshot.
    /// Drives the mandatory periodic safety resync (`events_full_scan_every`).
    incremental_passes_since_full_scan: u64,
    _lock_guard: LockGuard,
}

#[derive(Debug, Serialize)]
struct MetricsSnapshot {
    generated_epoch_secs: u64,
    status: String,
    paused: bool,
    pending_changes: usize,
    last_sync_epoch_secs: Option<u64>,
    last_error: Option<String>,
    last_plan_summary: Option<PlanSummary>,
    last_successful_sync_summary: Option<PlanSummary>,
    status_history_entries: usize,
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
        let lock_guard = LockGuard::acquire(&config.lockfile_path)?;
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

        let daemon = Self {
            config,
            connection,
            proton,
            pending_changes: BTreeSet::new(),
            scan_options,
            paused: false,
            last_sync: None,
            last_error: None,
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history_path,
            metrics_path,
            status_history,
            ipc_io_timeout: IPC_IO_TIMEOUT,
            event_source,
            incremental_passes_since_full_scan: 0,
            _lock_guard: lock_guard,
        };
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
        // Runs independently of the select! loop below so it can still observe a
        // shutdown signal and flip the flag while the main task is blocked inside
        // `reconcile()`'s synchronous `block_in_place` call, letting an in-flight
        // proton-drive command be cancelled promptly instead of only being noticed
        // once that blocking call returns control to this task.
        tokio::spawn(async move {
            shutdown_signal().await;
            cancel_flag.store(true, Ordering::SeqCst);
        });

        let listener = bind_listener(&self.config.socket_path).await?;
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel();
        let mut watcher = build_watcher(watch_tx)?;
        watcher.watch(&self.config.local_root, RecursiveMode::Recursive)?;

        let mut interval = tokio::time::interval(self.config.scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;

        // A faster poll cadence for event-driven mode: an incremental pass is O(changes) and
        // usually idle, so polling the stream often keeps remote-change latency low without the
        // cost of a full-tree walk. The arm is gated on `events_driven`, so with the feature off
        // it never fires and the loop behaves exactly as before. Using an interval arm (rather
        // than a separate event-fetching task) keeps the single owner of `event_source` and the
        // SQLite connection inside the loop, avoiding shared-state hazards.
        let mut events_poll = tokio::time::interval(EVENTS_POLL_INTERVAL);
        events_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        events_poll.tick().await;

        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                maybe_event = watch_rx.recv() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            let outcome =
                                tokio::task::block_in_place(|| self.handle_fs_event(event));
                            if let Err(error) = outcome {
                                warn!(%error, "failed to process filesystem event");
                            }
                        }
                        Some(Err(error)) => {
                            warn!(%error, "filesystem watcher reported an error");
                        }
                        None => break,
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            if let Err(error) = self.handle_ipc_stream(stream).await {
                                warn!(%error, "failed to handle control connection");
                            }
                        }
                        Err(error) => {
                            warn!(%error, "failed to accept control connection");
                        }
                    }
                }
                _ = interval.tick() => {
                    self.reconcile_if_needed().await?;
                }
                _ = events_poll.tick(), if self.config.events_driven => {
                    self.reconcile_if_needed().await?;
                }
                _ = &mut shutdown => {
                    break;
                }
            }
        }

        remove_control_socket(&self.config.socket_path);
        info!("daemon stopped");
        Ok(())
    }

    async fn handle_ipc_stream(&mut self, stream: UnixStream) -> AppResult<()> {
        // Time-bound the request read so a client that connects but never sends a
        // complete request line cannot park the single-threaded select! loop (and with it
        // periodic scans, filesystem events, and graceful shutdown) indefinitely. The
        // read is also length-bounded in `read_request`. Request *processing* below is
        // deliberately not time-bounded: a `Syncnow` triggers a full reconcile that may
        // legitimately take a long time.
        let (request, mut stream) =
            match tokio::time::timeout(self.ipc_io_timeout, read_request(stream)).await {
                Ok(result) => result?,
                Err(_elapsed) => {
                    warn!(
                        timeout_secs = self.ipc_io_timeout.as_secs(),
                        "control connection did not send a request within the timeout; dropping it"
                    );
                    return Ok(());
                }
            };
        debug!(command = ?request.command, "handling control request");
        let response = self.handle_ipc_request(request).await?;
        // Time-bound the response write too, so a client that sends a valid request then
        // never reads cannot wedge the loop on a full send buffer.
        match tokio::time::timeout(self.ipc_io_timeout, write_response(&mut stream, &response))
            .await
        {
            Ok(result) => result?,
            Err(_elapsed) => warn!(
                timeout_secs = self.ipc_io_timeout.as_secs(),
                "control client did not read the response within the timeout; dropping it"
            ),
        }
        Ok(())
    }

    fn handle_fs_event(&mut self, event: Event) -> AppResult<()> {
        for path in event.paths {
            if path.is_dir() {
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
                    mark_modified(&self.connection, &relative_path)?;
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

    async fn handle_ipc_request(&mut self, request: ControlRequest) -> AppResult<ControlResponse> {
        let response = match request.command {
            ControlCommand::Status => self.status_response("daemon status"),
            ControlCommand::Pause => {
                self.paused = true;
                info!("sync paused");
                self.write_metrics_snapshot()?;
                self.status_response("sync paused")
            }
            ControlCommand::Resume => {
                self.paused = false;
                info!("sync resumed");
                self.write_metrics_snapshot()?;
                self.status_response("sync resumed")
            }
            ControlCommand::Syncnow => {
                if self.paused {
                    self.status_response("sync skipped because daemon is paused")
                } else {
                    match self.reconcile().await {
                        Ok(()) => self.status_response("sync completed"),
                        Err(error) => {
                            error!(%error, "manual reconciliation failed");
                            self.status_response(&format!("sync failed: {error}"))
                        }
                    }
                }
            }
        };
        Ok(response)
    }

    fn status_response(&self, message: &str) -> ControlResponse {
        ControlResponse {
            status: if self.paused {
                "paused".to_owned()
            } else {
                "running".to_owned()
            },
            paused: self.paused,
            pending_changes: self.pending_changes.len(),
            message: message.to_owned(),
            last_sync_epoch_secs: self.last_sync_epoch_secs(),
            last_error: self.last_error.clone(),
            last_plan_summary: self.last_plan_summary.clone(),
            last_successful_sync_summary: self.last_successful_sync_summary.clone(),
            status_history: self.status_history.clone(),
        }
    }

    async fn reconcile_if_needed(&mut self) -> AppResult<()> {
        if self.paused {
            return Ok(());
        }
        if let Err(error) = self.reconcile().await {
            error!(%error, "scheduled reconciliation failed");
        }
        Ok(())
    }

    async fn reconcile(&mut self) -> AppResult<()> {
        tokio::task::block_in_place(|| self.reconcile_blocking())
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
        result
    }

    fn reconcile_blocking_inner(&mut self) -> AppResult<()> {
        info!("starting reconciliation");
        // Load the baseline before scanning so the scan can reuse each unchanged file's
        // recorded SHA-1 (matching size + mtime) instead of re-hashing the whole tree.
        let base_records = load_index(&self.connection)?;

        // Event-driven steady state: attempt an incremental pass (O(changes)) before resorting to
        // a full-tree snapshot (O(folders)). Any doubt — no cursor, a server refresh, an events
        // error, or an unresolvable node — falls through to the snapshot below, which is exactly
        // today's behavior. When `events_driven` is off this predicate is always false.
        if self.should_try_incremental(&base_records) {
            match self.try_incremental_reconcile(&base_records)? {
                IncrementalOutcome::Committed | IncrementalOutcome::Idle => return Ok(()),
                IncrementalOutcome::Fallback(reason) => {
                    info!(%reason, "event-driven pass fell back to a full-tree snapshot");
                }
            }
        }

        self.bootstrap_reconcile(base_records)
    }

    /// Whether an incremental (event-stream) pass may be attempted this cycle. Requires the
    /// feature on with a usable event source, a volume id derivable from a previously-stored
    /// composed `proton_id`, a stored cursor to replay from, and that the mandatory periodic
    /// safety resync is not currently due.
    fn should_try_incremental(&self, base_records: &HashMap<PathBuf, FileRecord>) -> bool {
        if !self.config.events_driven || self.event_source.is_none() {
            return false;
        }
        if self.incremental_passes_since_full_scan >= self.config.events_full_scan_every {
            return false;
        }
        let Some(volume) = derive_volume_id(base_records) else {
            return false;
        };
        matches!(load_event_cursor(&self.connection, volume), Ok(Some(_)))
    }

    /// One incremental pass: fetch the event delta, reconstruct the complete remote map as
    /// `base ⊕ delta`, plan, execute, and advance the cursor inside the post-side-effects commit.
    /// Returns [`IncrementalOutcome::Fallback`] (without committing) whenever the delta cannot be
    /// turned into a complete map, so the caller re-bootstraps.
    fn try_incremental_reconcile(
        &mut self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> AppResult<IncrementalOutcome> {
        let volume = derive_volume_id(base_records)
            .expect("should_try_incremental guarantees a derivable volume id")
            .to_owned();
        let cursor = match load_event_cursor(&self.connection, &volume)? {
            Some(cursor) => cursor,
            None => return Ok(IncrementalOutcome::Fallback("no stored cursor".to_owned())),
        };

        // Fetch the delta (paginating) *before* the local scan so a fully idle cycle does no work.
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

        // Idle: no remote changes and no pending local changes → advance the cursor and skip the
        // local stat-walk and planning entirely.
        if delta.changes.is_empty() && self.pending_changes.is_empty() {
            if delta.latest_event_id != cursor.last_event_id {
                store_event_cursor(
                    &self.connection,
                    &volume,
                    &delta.latest_event_id,
                    current_epoch_secs() as i64,
                )?;
            }
            self.incremental_passes_since_full_scan += 1;
            info!("event-driven pass idle; no remote or local changes");
            return Ok(IncrementalOutcome::Idle);
        }

        let local_entities = scan_local_entities_reusing_hashes(
            &self.config.local_root,
            &self.scan_options,
            base_records,
        )?;
        let local_files = local_files_from_entities(&local_entities);
        let base_index = filter_base_index(base_records.clone(), &self.scan_options);

        let remote_entities = {
            let resolver = TargetedResolver {
                proton: &self.proton,
                connection: &self.connection,
                remote_root: &self.config.remote_root,
                volume_id: &volume,
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
        self.incremental_passes_since_full_scan += 1;
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

    /// Full-tree snapshot reconcile — the original behavior, plus (when event-driven) capturing
    /// and persisting the replay cursor `C0`. Resets the periodic-resync counter.
    fn bootstrap_reconcile(&mut self, base_records: HashMap<PathBuf, FileRecord>) -> AppResult<()> {
        // Market-data recovery: capture the cursor *before* the snapshot when the volume is
        // already known, so a change landing during the walk is re-delivered (idempotently) by
        // the next incremental pass. On the first-ever bootstrap the volume is only known after
        // the walk, and the mandatory periodic resync bounds that one-time gap.
        let pre_snapshot_cursor = self.capture_pre_snapshot_cursor(&base_records);

        let local_entities = scan_local_entities_reusing_hashes(
            &self.config.local_root,
            &self.scan_options,
            &base_records,
        )?;
        let local_files = local_files_from_entities(&local_entities);
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
        Ok(())
    }

    /// Reads the current latest cursor before a snapshot, when event-driven and the volume is
    /// already derivable from the baseline. Best-effort: a failure just defers cursor capture to
    /// after the snapshot.
    fn capture_pre_snapshot_cursor(
        &self,
        base_records: &HashMap<PathBuf, FileRecord>,
    ) -> Option<CursorUpdate> {
        if !self.config.events_driven {
            return None;
        }
        let source = self.event_source.as_ref()?;
        let volume = derive_volume_id(base_records)?.to_owned();
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
    /// effects, then commits the resulting index mutations — and, when `cursor_update` is set, the
    /// advanced event cursor — in a **single** post-success transaction (the commit-after-side-
    /// effects invariant: a mid-plan failure leaves both the index and the cursor unadvanced).
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
        info!(
            planned_actions = plan_summary.total,
            uploads = plan_summary.uploads,
            downloads = plan_summary.downloads,
            conflicts = plan_summary.conflicts,
            skipped_unsupported = plan_summary.skipped_unsupported,
            destructive_actions = plan_summary.destructive_actions,
            "sync plan computed"
        );

        let mut index_mutations = Vec::new();
        let planned_remote_directories: BTreeSet<PathBuf> = plan
            .iter()
            .filter(|action| action.action == SyncAction::CreateRemoteDirectory)
            .map(|action| action.path.clone())
            .collect();

        for action in &plan {
            debug!(path = %action.path.display(), action = ?action.action, "executing sync action");
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
                        self.proton.upload(
                            &local.absolute_path,
                            &self.config.remote_root,
                            &action.path,
                        )?;
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
                    let Some(destination) = safe_local_path(&self.config.local_root, &action.path)
                    else {
                        warn!(
                            path = %action.path.display(),
                            "skipping download: local destination escapes the sync root \
                             (e.g. through a symlinked directory)"
                        );
                        continue;
                    };
                    ensure_parent_directory(&destination)?;
                    self.proton.download(&remote_path, &destination)?;
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
                        continue;
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
                        && let Some(source) = safe_local_path(&self.config.local_root, &action.path)
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
                                if let Some(descendant_record) = base_index.get(&old_descendant) {
                                    index_mutations
                                        .push(IndexMutation::Purge(old_descendant.clone()));
                                    index_mutations.push(IndexMutation::Upsert(FileRecord {
                                        file_path: new_descendant,
                                        ..descendant_record.clone()
                                    }));
                                }
                            }
                        } else {
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
                    if action.remote_id.is_some()
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
                }
                SyncAction::LocalDelete => {
                    let Some(destination) = safe_local_path(&self.config.local_root, &action.path)
                    else {
                        warn!(
                            path = %action.path.display(),
                            "skipping local delete: path escapes the sync root \
                             (e.g. through a symlinked directory)"
                        );
                        continue;
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

        let transaction = self.connection.transaction()?;
        for mutation in &index_mutations {
            mutation.apply(&transaction)?;
        }
        // Advance the event cursor in the SAME transaction as the index mutations: it must move
        // only after every side effect of this plan has succeeded, so a mid-plan failure (which
        // returns early above, before this commit) replays the same events next pass rather than
        // silently skipping them. Reprocessing events is idempotent; skipping them loses changes.
        if let Some(cursor_update) = &cursor_update {
            store_event_cursor(
                &transaction,
                &cursor_update.scope_id,
                &cursor_update.last_event_id,
                current_epoch_secs() as i64,
            )?;
        }
        transaction.commit()?;
        // `pending_changes` only drives status reporting - planning always performs a
        // fresh full scan regardless of its contents - so it is safe, simpler, and
        // leak-free to clear it unconditionally after every successful reconcile
        // rather than removing entries one at a time keyed on the paths that happened
        // to appear in this pass's plan. Per-path removal could miss paths that were
        // inserted by a filesystem event but never produced a corresponding planned
        // action (for example a misclassified directory removal, a non-regular-file
        // event, or a path whose plan outcome was `None`), leaking them until the
        // process restarted.
        self.pending_changes.clear();

        self.last_sync = Some(SystemTime::now());
        self.last_successful_sync_summary = Some(plan_summary);
        info!("reconciliation completed");
        Ok(())
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
        MetricsSnapshot {
            generated_epoch_secs: current_epoch_secs(),
            status: if self.paused {
                "paused".to_owned()
            } else {
                "running".to_owned()
            },
            paused: self.paused,
            pending_changes: self.pending_changes.len(),
            last_sync_epoch_secs: self.last_sync_epoch_secs(),
            last_error: self.last_error.clone(),
            last_plan_summary: self.last_plan_summary.clone(),
            last_successful_sync_summary: self.last_successful_sync_summary.clone(),
            status_history_entries: self.status_history.len(),
        }
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

fn scan_options_from_config(config: &DaemonConfig) -> AppResult<ScanOptions> {
    let ignored_paths = vec![
        config.db_path.clone(),
        status_history_path(&config.db_path),
        metrics_path(&config.db_path),
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
    ensure_parent_directory(path)?;
    fs::write(path, serde_json::to_vec_pretty(history)?)?;
    Ok(())
}

fn write_metrics_snapshot(path: &Path, metrics: &MetricsSnapshot) -> AppResult<()> {
    ensure_parent_directory(path)?;
    fs::write(path, serde_json::to_vec_pretty(metrics)?)?;
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
}

impl<C: ProtonClient> RemoteChangeResolver for TargetedResolver<'_, C> {
    fn resolve(&self, change: &RemoteChange) -> AppResult<Option<(PathBuf, RemoteEntity)>> {
        let target_uid = node_uid(self.volume_id, &change.node_id);

        // Prefer listing the event's parent directory when it is indexed (the common nested case).
        if let Some(parent_id) = change.parent_id.as_deref() {
            let parent_uid = node_uid(self.volume_id, parent_id);
            if let Some(parent_path) = path_for_proton_id(self.connection, &parent_uid)? {
                let listing = self.proton.list_directory(self.remote_root, &parent_path)?;
                // Absent from its stated parent → let the reconstruction drop any stale location.
                return Ok(find_entity_by_uid(listing, &target_uid));
            }
        }

        // The parent is not indexed (e.g. a top-level node whose parent is the remote root, which
        // has no index record). Fall back to listing the root; if the node is not there either we
        // cannot place it without a full walk, so signal a snapshot.
        let root_listing = self
            .proton
            .list_directory(self.remote_root, Path::new(""))?;
        match find_entity_by_uid(root_listing, &target_uid) {
            Some(resolved) => Ok(Some(resolved)),
            None => Err(boxed_error(format!(
                "changed node {} is not under any indexed parent or the remote root",
                change.node_id
            ))),
        }
    }
}

fn find_entity_by_uid(
    listing: HashMap<PathBuf, RemoteEntity>,
    target_uid: &str,
) -> Option<(PathBuf, RemoteEntity)> {
    listing
        .into_iter()
        .find(|(_, entity)| entity.remote_id().as_deref() == Some(target_uid))
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
/// contains components that could escape `local_root`.  Delegates to
/// [`crate::validate_relative_path`] for consistent security semantics with
/// the remote-path normalization in `proton.rs`.
fn safe_local_path(local_root: &Path, relative: &Path) -> Option<PathBuf> {
    let destination = local_root.join(crate::validate_relative_path(relative)?);
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

fn safe_remote_path(remote_root: &Path, relative: &Path) -> Option<PathBuf> {
    crate::validate_relative_path(relative).map(|safe| remote_root.join(safe))
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

struct LockGuard {
    path: PathBuf,
    _file: File,
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
        Ok(Self {
            path: path.to_path_buf(),
            _file: file,
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self._file.unlock();
        let _ = fs::remove_file(&self.path);
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
            scan_interval: Duration::from_secs(300),
            proton_cli: PathBuf::from("proton-drive"),
            proton_timeout: Duration::from_secs(60),
            proton_list_attempts: 2,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            events_driven: false,
            events_full_scan_every: 20,
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
    fn reconcile_does_not_commit_directory_move_when_later_action_fails() {
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
        assert!(
            get_record(&daemon.connection, Path::new("old-docs"))
                .expect("old directory index lookup")
                .is_some(),
            "a successful directory move must not be committed when a later action fails"
        );
        assert!(
            get_record(&daemon.connection, Path::new("old-docs/report.txt"))
                .expect("old descendant index lookup")
                .is_some(),
            "descendant rewrites queued by an earlier action must not be committed either"
        );
        assert!(
            get_record(&daemon.connection, Path::new("new-docs"))
                .expect("new directory index lookup")
                .is_none(),
            "no new index row should appear until the whole reconcile succeeds"
        );
    }

    #[test]
    fn reconcile_does_not_commit_index_when_later_action_fails() {
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
                .is_none(),
            "a successful early action must not be committed when a later action fails"
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

    #[tokio::test]
    async fn idle_ipc_client_is_dropped_after_the_io_timeout_instead_of_blocking() {
        // A client that connects to the control socket but never sends a request line
        // must not park the daemon's single-threaded event loop forever (which would
        // freeze reconciles, filesystem events, and graceful shutdown). handle_ipc_stream
        // must return after the IO timeout rather than hang.
        let directory = tempdir().expect("tempdir");
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let (client, _) = RecordingProtonClient::new(HashMap::new());
        let mut daemon = Daemon::with_client(test_config(directory.path(), &local_root), client)
            .expect("daemon");
        daemon.ipc_io_timeout = Duration::from_millis(50);

        let (control_client, server) = UnixStream::pair().expect("socket pair");

        // The outer timeout is a generous test-only guard: if the handler regressed to
        // blocking forever it fails here instead of hanging the suite.
        let outcome =
            tokio::time::timeout(Duration::from_secs(5), daemon.handle_ipc_stream(server))
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
            !lock_path.exists(),
            "released lock guard should remove stale lockfile"
        );
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

    fn test_config(directory: &Path, local_root: &Path) -> DaemonConfig {
        DaemonConfig {
            local_root: local_root.to_path_buf(),
            remote_root: PathBuf::from("/Drive/RemoteFolder"),
            db_path: directory.join("sync_index.db"),
            socket_path: directory.join("daemon.sock"),
            lockfile_path: directory.join("daemon.lock"),
            scan_interval: Duration::from_secs(300),
            proton_cli: PathBuf::from("proton-drive"),
            proton_timeout: Duration::from_secs(60),
            proton_list_attempts: 2,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            events_driven: false,
            events_full_scan_every: 20,
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
    }

    impl EventFakeClient {
        fn new(remote_entities: HashMap<PathBuf, RemoteEntity>) -> Self {
            Self {
                remote_entities,
                full_walks: Arc::new(AtomicUsize::new(0)),
                directory_lists: Arc::new(AtomicUsize::new(0)),
                failed_uploads: BTreeSet::new(),
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
            if self.failed_uploads.contains(relative_path) {
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
        // Already at the resync threshold → this pass must snapshot, not go incremental.
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
}
