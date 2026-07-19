use crate::index::{
    FileRecord, ScanOptions, SyncStatus, get_record, load_existing_index, load_index,
    local_file_state, mark_modified, open_database, purge_record, scan_local_files_with_options,
    upsert_record,
};
use crate::ipc::{
    ControlCommand, ControlRequest, ControlResponse, bind_listener, read_request, write_response,
};
use crate::proton::{ProtonClient, ProtonDriveClient, RemoteFile};
use crate::sync::{PlannedAction, SyncAction, original_from_conflict_copy, plan_sync};
use crate::{AppResult, boxed_error};
use fs2::FileExt;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    pub lockfile_path: PathBuf,
    pub scan_interval: Duration,
    pub proton_cli: PathBuf,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

pub struct Daemon<C: ProtonClient = ProtonDriveClient> {
    config: DaemonConfig,
    connection: Connection,
    proton: C,
    pending_changes: BTreeSet<PathBuf>,
    scan_options: ScanOptions,
    paused: bool,
    last_sync: Option<SystemTime>,
    _lock_guard: LockGuard,
}

pub fn preview_plan(config: &DaemonConfig) -> AppResult<Vec<PlannedAction>> {
    let client = ProtonDriveClient::new(config.proton_cli.clone());
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
    let local_files = scan_local_files_with_options(&config.local_root, &scan_options)?;
    let remote_files = filter_remote_files(proton.list(&config.remote_root)?, &scan_options);
    let base_index = load_existing_index(&config.db_path)?;
    let base_index = filter_base_index(base_index, &scan_options);
    let plan = plan_sync(&local_files, &remote_files, &base_index);
    info!(planned_actions = plan.len(), "dry-run sync plan computed");
    Ok(plan)
}

impl Daemon<ProtonDriveClient> {
    pub fn new(config: DaemonConfig) -> AppResult<Self> {
        let proton = ProtonDriveClient::new(config.proton_cli.clone());
        Self::with_client(config, proton)
    }
}

impl<C: ProtonClient> Daemon<C> {
    pub fn with_client(config: DaemonConfig, proton: C) -> AppResult<Self> {
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

        Ok(Self {
            config,
            connection,
            proton,
            pending_changes: BTreeSet::new(),
            scan_options,
            paused: false,
            last_sync: None,
            _lock_guard: lock_guard,
        })
    }

    pub async fn run(mut self) -> AppResult<()> {
        info!(
            local_root = %self.config.local_root.display(),
            remote_root = %self.config.remote_root.display(),
            socket_path = %self.config.socket_path.display(),
            "starting daemon"
        );
        let listener = bind_listener(&self.config.socket_path).await?;
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel();
        let mut watcher = build_watcher(watch_tx)?;
        watcher.watch(&self.config.local_root, RecursiveMode::Recursive)?;

        let mut interval = tokio::time::interval(self.config.scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;

        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                maybe_event = watch_rx.recv() => {
                    if let Some(event) = maybe_event {
                        self.handle_fs_event(event?)?;
                    }
                }
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    let (request, mut stream) = read_request(stream).await?;
                    debug!(command = ?request.command, "handling control request");
                    let response = self.handle_ipc_request(request).await?;
                    write_response(&mut stream, &response).await?;
                }
                _ = interval.tick() => {
                    self.reconcile_if_needed().await?;
                }
                _ = &mut shutdown => {
                    break;
                }
            }
        }

        if self.config.socket_path.exists() {
            fs::remove_file(&self.config.socket_path)?;
        }
        info!("daemon stopped");
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
                    if path.exists() && path.is_file() {
                        let local_state = local_file_state(&self.config.local_root, &path)?;
                        let existing = get_record(&self.connection, &relative_path)?;
                        let record = FileRecord::from_local(
                            relative_path.clone(),
                            &local_state,
                            existing.and_then(|record| record.proton_id),
                            SyncStatus::Modified,
                        );
                        upsert_record(&self.connection, &record)?;
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

    async fn handle_ipc_request(&mut self, request: ControlRequest) -> AppResult<ControlResponse> {
        let response = match request.command {
            ControlCommand::Status => self.status_response("daemon status"),
            ControlCommand::Pause => {
                self.paused = true;
                info!("sync paused");
                self.status_response("sync paused")
            }
            ControlCommand::Resume => {
                self.paused = false;
                info!("sync resumed");
                self.status_response("sync resumed")
            }
            ControlCommand::Syncnow => {
                if self.paused {
                    self.status_response("sync skipped because daemon is paused")
                } else {
                    self.reconcile().await?;
                    self.status_response("sync completed")
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
            last_sync_epoch_secs: self
                .last_sync
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
        }
    }

    async fn reconcile_if_needed(&mut self) -> AppResult<()> {
        if self.paused {
            return Ok(());
        }
        self.reconcile().await
    }

    async fn reconcile(&mut self) -> AppResult<()> {
        tokio::task::block_in_place(|| self.reconcile_blocking())
    }

    fn reconcile_blocking(&mut self) -> AppResult<()> {
        info!("starting reconciliation");
        let local_files =
            scan_local_files_with_options(&self.config.local_root, &self.scan_options)?;
        let remote_files = filter_remote_files(
            self.proton.list(&self.config.remote_root)?,
            &self.scan_options,
        );
        let base_index = filter_base_index(load_index(&self.connection)?, &self.scan_options);
        let plan = plan_sync(&local_files, &remote_files, &base_index);
        info!(planned_actions = plan.len(), "sync plan computed");

        for action in plan {
            debug!(path = %action.path.display(), action = ?action.action, "executing sync action");
            match action.action {
                SyncAction::Upload => {
                    if let Some(local) = local_files.get(&action.path) {
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
                        upsert_record(&self.connection, &record)?;
                    }
                }
                SyncAction::Download => {
                    if let Some(remote_id) = action.remote_id.as_deref()
                        && let Some(destination) =
                            safe_local_path(&self.config.local_root, &action.path)
                    {
                        ensure_parent_directory(&destination)?;
                        self.proton.download(remote_id, &destination)?;
                        let local_state = local_file_state(&self.config.local_root, &destination)?;
                        let record = FileRecord::from_local(
                            action.path.clone(),
                            &local_state,
                            Some(remote_id.to_owned()),
                            SyncStatus::Synced,
                        );
                        upsert_record(&self.connection, &record)?;
                    }
                }
                SyncAction::AutoLink => {
                    if let Some(local) = local_files.get(&action.path) {
                        let record = FileRecord::from_local(
                            action.path.clone(),
                            local,
                            action.remote_id.clone(),
                            SyncStatus::Synced,
                        );
                        upsert_record(&self.connection, &record)?;
                    }
                }
                SyncAction::Conflict => {
                    if let Some(remote_id) = action.remote_id.as_deref()
                        && let Some(conflict_path) = action.conflict_path.as_ref()
                        && let Some(destination) =
                            safe_local_path(&self.config.local_root, conflict_path)
                    {
                        ensure_parent_directory(&destination)?;
                        self.proton.download(remote_id, &destination)?;
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
                        upsert_record(&self.connection, &record)?;
                    }
                }
                SyncAction::RemoteDelete => {
                    if let Some(remote_id) = action.remote_id.as_deref() {
                        self.proton.delete(remote_id)?;
                    }
                    purge_record(&self.connection, &action.path)?;
                }
                SyncAction::LocalDelete => {
                    if let Some(destination) =
                        safe_local_path(&self.config.local_root, &action.path)
                        && destination.exists()
                    {
                        fs::remove_file(destination)?;
                    }
                    purge_record(&self.connection, &action.path)?;
                }
                SyncAction::Purge => {
                    purge_record(&self.connection, &action.path)?;
                }
            }
            self.pending_changes.remove(&action.path);
        }

        self.last_sync = Some(SystemTime::now());
        info!("reconciliation completed");
        Ok(())
    }
}

fn scan_options_from_config(config: &DaemonConfig) -> AppResult<ScanOptions> {
    ScanOptions::new(
        &config.local_root,
        std::slice::from_ref(&config.db_path),
        &config.include_patterns,
        &config.exclude_patterns,
    )
}

fn filter_remote_files(
    remote_files: HashMap<PathBuf, RemoteFile>,
    scan_options: &ScanOptions,
) -> HashMap<PathBuf, RemoteFile> {
    remote_files
        .into_iter()
        .filter(|(path, _)| scan_options.allows_relative_file(path))
        .collect()
}

fn filter_base_index(
    base_index: HashMap<PathBuf, FileRecord>,
    scan_options: &ScanOptions,
) -> HashMap<PathBuf, FileRecord> {
    base_index
        .into_iter()
        .filter(|(path, _)| scan_options.allows_relative_file(path))
        .collect()
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

/// Join `relative` onto `local_root` only when it is safe to do so.
///
/// Returns `None` (and the caller should skip the action) when `relative`
/// contains components that could escape `local_root`.  Delegates to
/// [`crate::validate_relative_path`] for consistent security semantics with
/// the remote-path normalization in `proton.rs`.
fn safe_local_path(local_root: &Path, relative: &Path) -> Option<PathBuf> {
    crate::validate_relative_path(relative).map(|safe| local_root.join(safe))
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
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedOperation {
        Upload {
            local_path: PathBuf,
            remote_root: PathBuf,
            relative_path: PathBuf,
        },
        Download {
            remote_id: String,
            destination: PathBuf,
        },
        Delete {
            remote_id: String,
        },
    }

    #[derive(Debug, Clone)]
    struct RecordingProtonClient {
        remote_files: HashMap<PathBuf, RemoteFile>,
        operations: Arc<Mutex<Vec<RecordedOperation>>>,
    }

    impl RecordingProtonClient {
        fn new(
            remote_files: HashMap<PathBuf, RemoteFile>,
        ) -> (Self, Arc<Mutex<Vec<RecordedOperation>>>) {
            let operations = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    remote_files,
                    operations: Arc::clone(&operations),
                },
                operations,
            )
        }
    }

    impl ProtonClient for RecordingProtonClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(self.remote_files.clone())
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

        fn download(&self, remote_id: &str, destination: &Path) -> AppResult<()> {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(destination, format!("downloaded:{remote_id}"))?;
            self.operations
                .lock()
                .expect("operations lock")
                .push(RecordedOperation::Download {
                    remote_id: remote_id.to_owned(),
                    destination: destination.to_path_buf(),
                });
            Ok(())
        }

        fn delete(&self, remote_id: &str) -> AppResult<()> {
            self.operations
                .lock()
                .expect("operations lock")
                .push(RecordedOperation::Delete {
                    remote_id: remote_id.to_owned(),
                });
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeProtonClient {
        remote_files: HashMap<PathBuf, RemoteFile>,
    }

    impl ProtonClient for FakeProtonClient {
        fn list(&self, _remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
            Ok(self.remote_files.clone())
        }

        fn upload(
            &self,
            _local_path: &Path,
            _remote_root: &Path,
            _relative_path: &Path,
        ) -> AppResult<()> {
            Err(boxed_error("unexpected upload in fake client"))
        }

        fn download(&self, _remote_id: &str, _destination: &Path) -> AppResult<()> {
            Err(boxed_error("unexpected download in fake client"))
        }

        fn delete(&self, _remote_id: &str) -> AppResult<()> {
            Err(boxed_error("unexpected delete in fake client"))
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
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
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
            "downloaded:remote-id"
        );
        assert!(operations.lock().expect("operations lock").contains(
            &RecordedOperation::Download {
                remote_id: "remote-id".to_owned(),
                destination,
            }
        ));
        let record = get_record(&daemon.connection, Path::new("remote-only.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.proton_id.as_deref(), Some("remote-id"));
        assert_eq!(record.sync_status, SyncStatus::Synced);
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
                    remote_id: "remote-id".to_owned(),
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
            "downloaded:remote-id"
        );
        assert!(operations.lock().expect("operations lock").contains(
            &RecordedOperation::Download {
                remote_id: "remote-id".to_owned(),
                destination: conflict_path,
            }
        ));
        let record = get_record(&daemon.connection, Path::new("notes.txt"))
            .expect("index lookup")
            .expect("index record");
        assert_eq!(record.sync_status, SyncStatus::Conflict);
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
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
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
        }
    }

    fn base_record(path: &str, proton_id: Option<&str>, sha1_hash: &str) -> FileRecord {
        FileRecord {
            file_path: PathBuf::from(path),
            file_size: 1,
            mtime: 1,
            sha1_hash: sha1_hash.to_owned(),
            proton_id: proton_id.map(ToOwned::to_owned),
            sync_status: SyncStatus::Synced,
        }
    }
}
