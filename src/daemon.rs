use crate::index::{
    FileRecord, SyncStatus, get_record, load_index, local_file_state, mark_modified, open_database,
    purge_record, scan_local_files, upsert_record,
};
use crate::ipc::{
    ControlCommand, ControlRequest, ControlResponse, bind_listener, read_request, write_response,
};
use crate::proton::ProtonDriveClient;
use crate::sync::{SyncAction, original_from_conflict_copy, plan_sync};
use crate::{AppResult, boxed_error};
use fs2::FileExt;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    pub lockfile_path: PathBuf,
    pub scan_interval: Duration,
    pub proton_cli: PathBuf,
}

pub struct Daemon {
    config: DaemonConfig,
    connection: Connection,
    proton: ProtonDriveClient,
    pending_changes: BTreeSet<PathBuf>,
    paused: bool,
    last_sync: Option<SystemTime>,
    _lock_guard: LockGuard,
}

impl Daemon {
    pub fn new(config: DaemonConfig) -> AppResult<Self> {
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
        let proton = ProtonDriveClient::new(config.proton_cli.clone());

        Ok(Self {
            config,
            connection,
            proton,
            pending_changes: BTreeSet::new(),
            paused: false,
            last_sync: None,
            _lock_guard: lock_guard,
        })
    }

    pub async fn run(mut self) -> AppResult<()> {
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
                self.status_response("sync paused")
            }
            ControlCommand::Resume => {
                self.paused = false;
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
        let local_files = scan_local_files(&self.config.local_root)?;
        let remote_files = self.proton.list(&self.config.remote_root)?;
        let base_index = load_index(&self.connection)?;
        let plan = plan_sync(&local_files, &remote_files, &base_index);

        for action in plan {
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
        Ok(())
    }
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
    use tempfile::tempdir;

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
}
