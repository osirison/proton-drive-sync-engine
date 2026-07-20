use crate::{AppResult, boxed_error};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::warn;
use wait_timeout::ChildExt;

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_LIST_ATTEMPTS: usize = 2;
const EXECUTABLE_BUSY_SPAWN_ATTEMPTS: usize = 3;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);
/// How often `run_once` checks the shared cancellation flag while waiting for a
/// `proton-drive` child process to exit. Bounds how long a cooperative shutdown
/// takes to notice a cancellation request; it does not change the total timeout
/// budget enforced by `CommandPolicy::timeout`.
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtonNode {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub uid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub path: Option<String>,
    #[serde(default)]
    pub children: Vec<ProtonNode>,
    #[serde(default)]
    pub entries: Vec<ProtonNode>,
    #[serde(default)]
    pub files: Vec<ProtonNode>,
    #[serde(default, deserialize_with = "deserialize_optional_active_revision")]
    pub active_revision: Option<ActiveRevision>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub media_type: Option<String>,
    #[serde(
        rename = "type",
        default,
        deserialize_with = "deserialize_optional_string"
    )]
    pub kind: Option<String>,
    pub is_folder: Option<bool>,
    pub is_file: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRevision {
    pub claimed_digests: Option<ClaimedDigests>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimedDigests {
    #[serde(default, deserialize_with = "deserialize_optional_digest_string")]
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub sha1_hash: Option<String>,
    pub downloadable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDirectory {
    pub path: PathBuf,
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEntity {
    File(RemoteFile),
    Directory(RemoteDirectory),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteListingStatus {
    Found(HashMap<PathBuf, RemoteEntity>),
    RootMissing,
}

impl RemoteEntity {
    pub fn as_file(&self) -> Option<&RemoteFile> {
        match self {
            Self::File(file) => Some(file),
            Self::Directory(_) => None,
        }
    }

    pub fn as_directory(&self) -> Option<&RemoteDirectory> {
        match self {
            Self::File(_) => None,
            Self::Directory(directory) => Some(directory),
        }
    }

    pub fn remote_id(&self) -> Option<String> {
        match self {
            Self::File(file) => Some(file.id.clone()),
            Self::Directory(directory) => directory.id.clone(),
        }
    }
}

#[derive(Debug, Default)]
struct RemoteListing {
    files: HashMap<PathBuf, RemoteFile>,
    directories: HashMap<PathBuf, RemoteDirectory>,
}

impl RemoteListing {
    fn into_entities(self) -> HashMap<PathBuf, RemoteEntity> {
        self.directories
            .into_iter()
            .map(|(path, directory)| (path, RemoteEntity::Directory(directory)))
            .chain(
                self.files
                    .into_iter()
                    .map(|(path, file)| (path, RemoteEntity::File(file))),
            )
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ProtonDriveClient {
    executable: PathBuf,
    command_policy: CommandPolicy,
    cancel_flag: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandPolicy {
    pub timeout: Duration,
    pub list_attempts: usize,
}

pub trait ProtonClient: Send + Sync {
    fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>>;
    fn list_entities(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
        Ok(self
            .list(remote_root)?
            .into_iter()
            .map(|(path, file)| (path, RemoteEntity::File(file)))
            .collect())
    }
    fn list_entities_or_missing_root(&self, remote_root: &Path) -> AppResult<RemoteListingStatus> {
        Ok(RemoteListingStatus::Found(self.list_entities(remote_root)?))
    }
    fn ensure_root_directory(&self, remote_root: &Path) -> AppResult<()> {
        Err(boxed_error(format!(
            "creating remote root directories is not supported by this Proton client: {}",
            remote_root.display()
        )))
    }
    fn ensure_directory(&self, remote_root: &Path, relative_path: &Path) -> AppResult<()>;
    fn upload(&self, local_path: &Path, remote_root: &Path, relative_path: &Path) -> AppResult<()>;
    fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()>;
    fn delete(&self, remote_path: &Path) -> AppResult<()>;
    /// Renames and/or moves a remote entry from `old_relative_path` to
    /// `new_relative_path`, both resolved against `remote_root`. Implementations
    /// that cannot yet perform this safely should return an error; the default
    /// implementation does so.
    fn rename_or_move(
        &self,
        _remote_root: &Path,
        old_relative_path: &Path,
        new_relative_path: &Path,
    ) -> AppResult<()> {
        Err(boxed_error(format!(
            "rename/move is not supported by this Proton client ({} -> {})",
            old_relative_path.display(),
            new_relative_path.display()
        )))
    }
    /// Installs a shared cancellation flag that this client may poll to abort an
    /// in-flight command promptly instead of running to completion or timeout. The
    /// default implementation is a no-op, so test doubles are unaffected unless they
    /// choose to opt in.
    fn install_cancel_flag(&mut self, _cancel_flag: Arc<AtomicBool>) {}
}

impl ProtonDriveClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            command_policy: CommandPolicy::default(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_timeout(executable: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self::with_command_policy(
            executable,
            CommandPolicy::new(timeout, DEFAULT_LIST_ATTEMPTS),
        )
    }

    pub fn with_command_policy(
        executable: impl Into<PathBuf>,
        command_policy: CommandPolicy,
    ) -> Self {
        Self {
            executable: executable.into(),
            command_policy,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl CommandPolicy {
    pub fn new(timeout: Duration, list_attempts: usize) -> Self {
        Self {
            timeout,
            list_attempts: list_attempts.max(1),
        }
    }
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self::new(DEFAULT_COMMAND_TIMEOUT, DEFAULT_LIST_ATTEMPTS)
    }
}

impl ProtonClient for ProtonDriveClient {
    fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
        match self.list_entities_or_missing_root(remote_root)? {
            RemoteListingStatus::Found(entities) => Ok(entities
                .into_iter()
                .filter_map(|(path, entity)| entity.as_file().cloned().map(|file| (path, file)))
                .collect()),
            RemoteListingStatus::RootMissing => Err(boxed_error(format!(
                "remote root does not exist: {}",
                remote_root.display()
            ))),
        }
    }

    fn list_entities(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
        match self.list_entities_or_missing_root(remote_root)? {
            RemoteListingStatus::Found(entities) => Ok(entities),
            RemoteListingStatus::RootMissing => Err(boxed_error(format!(
                "remote root does not exist: {}",
                remote_root.display()
            ))),
        }
    }

    fn list_entities_or_missing_root(&self, remote_root: &Path) -> AppResult<RemoteListingStatus> {
        let mut files = HashMap::new();
        let mut directories = HashMap::new();
        let mut visited = BTreeSet::new();
        let mut pending = vec![PathBuf::new()];

        while let Some(relative_directory) = pending.pop() {
            if !visited.insert(relative_directory.clone()) {
                continue;
            }

            let remote_directory = if relative_directory.as_os_str().is_empty() {
                remote_root.to_path_buf()
            } else {
                remote_root.join(&relative_directory)
            };
            let output = self.run_proton_drive(
                "list",
                &[
                    OsString::from("filesystem"),
                    OsString::from("list"),
                    OsString::from("--json"),
                    remote_directory.as_os_str().to_os_string(),
                ],
                self.command_policy.list_attempts,
            )?;
            if !output.status.success() {
                if relative_directory.as_os_str().is_empty() && is_node_not_found(&output) {
                    return Ok(RemoteListingStatus::RootMissing);
                }
                return Err(boxed_error(format!(
                    "proton-drive list failed for {}: {}",
                    remote_directory.display(),
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
            let stdout = String::from_utf8(output.stdout)?;
            let listing = parse_remote_listing(&stdout, remote_root, &relative_directory)?;
            files.extend(listing.files);
            pending.extend(
                listing
                    .directories
                    .keys()
                    .filter(|folder| !visited.contains(*folder))
                    .cloned(),
            );
            directories.extend(listing.directories);
        }

        Ok(RemoteListingStatus::Found(
            RemoteListing { files, directories }.into_entities(),
        ))
    }

    fn ensure_root_directory(&self, remote_root: &Path) -> AppResult<()> {
        let remote_root = clean_remote_root_path(remote_root).ok_or_else(|| {
            boxed_error(format!(
                "unsafe remote root path: {}",
                remote_root.display()
            ))
        })?;
        if self.remote_path_exists(&remote_root)? {
            return Ok(());
        }
        let existing_parent = self
            .deepest_existing_remote_parent(&remote_root)?
            .ok_or_else(|| {
                boxed_error(format!(
                    "remote root parent does not exist for {}",
                    remote_root.display()
                ))
            })?;
        let relative_path = remote_root
            .strip_prefix(&existing_parent)
            .map_err(|error| {
                boxed_error(format!(
                    "failed to compute remote root path {} relative to existing parent {}: {error}",
                    remote_root.display(),
                    existing_parent.display()
                ))
            })?;
        self.create_missing_directory_components(&existing_parent, relative_path)
    }

    fn ensure_directory(&self, remote_root: &Path, relative_path: &Path) -> AppResult<()> {
        let relative_path = crate::validate_relative_path(relative_path).ok_or_else(|| {
            boxed_error(format!(
                "unsafe remote directory path: {}",
                relative_path.display()
            ))
        })?;
        self.create_missing_directory_components(remote_root, &relative_path)
    }

    fn upload(&self, local_path: &Path, remote_root: &Path, relative_path: &Path) -> AppResult<()> {
        let relative_path = crate::validate_relative_path(relative_path).ok_or_else(|| {
            boxed_error(format!(
                "unsafe remote upload path: {}",
                relative_path.display()
            ))
        })?;
        let remote_parent = relative_path
            .parent()
            .map(|parent| remote_root.join(parent))
            .unwrap_or_else(|| remote_root.to_path_buf());
        // `filesystem upload` prompts interactively for a conflict strategy whenever the
        // destination folder already contains a file with the same name (i.e. every time
        // the planner uploads a new revision of a previously synced file). With stdin
        // wired to `Stdio::null()` that prompt sees immediate EOF and the CLI silently
        // *skips* the file while still exiting 0 - the remote content is never updated,
        // yet the daemon would record the path as `SyncStatus::Synced`. Always pass an
        // explicit `--file-conflict-strategy replace` so a same-named upload creates a
        // new revision instead of hitting that prompt; this matches the planner's own
        // intent, since `SyncAction::Upload` is only ever chosen after the local content
        // has already been determined (via SHA-1 comparison) to be the version that
        // should win.
        let output = self.run_proton_drive(
            "upload",
            &[
                OsString::from("filesystem"),
                OsString::from("upload"),
                OsString::from("--file-conflict-strategy"),
                OsString::from("replace"),
                local_path.as_os_str().to_os_string(),
                remote_parent.as_os_str().to_os_string(),
            ],
            1,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive upload failed for {}: {}",
                local_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()> {
        let local_folder = destination.parent().ok_or_else(|| {
            boxed_error(format!(
                "download destination has no parent directory: {}",
                destination.display()
            ))
        })?;

        // `filesystem download <remotePath> <localFolder>` always names the downloaded
        // file after `remotePath`'s own basename inside `localFolder`; it does not
        // rename the result to `destination`'s basename. Most callers pass a
        // `destination` whose basename already matches the remote source (plain
        // `SyncAction::Download`), so this quirk is invisible there. But conflict
        // resolution intentionally downloads the remote's competing revision under a
        // *different* local name (the `.proton-cloud.` sidecar), specifically so the
        // caller's own conflicting file is left untouched. Downloading straight into
        // `local_folder` in that case would silently overwrite whatever file already
        // has the remote's basename there - for a conflict sidecar, that is exactly the
        // file this download is supposed to preserve. To make `download`'s contract
        // safe for every caller ("the content ends up at exactly `destination`, and
        // nothing else is touched"), always stage the download in a private scratch
        // directory first and move the single resulting entry into place with one
        // rename, rather than trusting the CLI to name it correctly on the first try.
        let scratch_dir = local_folder.join(format!(
            "{}{}-{}",
            crate::DOWNLOAD_SCRATCH_PREFIX,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&scratch_dir).map_err(|error| {
            boxed_error(format!(
                "failed to create scratch download directory {}: {error}",
                scratch_dir.display()
            ))
        })?;
        let _scratch_guard = ScratchDirGuard::new(&scratch_dir);

        let output = self.run_proton_drive(
            "download",
            &[
                OsString::from("filesystem"),
                OsString::from("download"),
                remote_path.as_os_str().to_os_string(),
                scratch_dir.as_os_str().to_os_string(),
            ],
            1,
        )?;
        if !output.status.success() {
            return Err(boxed_error(format!(
                "proton-drive download failed for {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let downloaded_path = single_entry_in_directory(&scratch_dir).map_err(|error| {
            boxed_error(format!(
                "download of {} did not produce exactly one file in the scratch \
                 directory {}: {error}",
                remote_path.display(),
                scratch_dir.display()
            ))
        })?;
        fs::rename(&downloaded_path, destination).map_err(|error| {
            boxed_error(format!(
                "downloaded {} to a scratch location but failed to move it to {}: {error}",
                remote_path.display(),
                destination.display()
            ))
        })
    }

    fn delete(&self, remote_path: &Path) -> AppResult<()> {
        let output = self.run_proton_drive(
            "trash",
            &[
                OsString::from("filesystem"),
                OsString::from("trash"),
                remote_path.as_os_str().to_os_string(),
            ],
            1,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive trash failed for {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn rename_or_move(
        &self,
        remote_root: &Path,
        old_relative_path: &Path,
        new_relative_path: &Path,
    ) -> AppResult<()> {
        let old_relative_path =
            crate::validate_relative_path(old_relative_path).ok_or_else(|| {
                boxed_error(format!(
                    "unsafe remote rename/move source path: {}",
                    old_relative_path.display()
                ))
            })?;
        let new_relative_path =
            crate::validate_relative_path(new_relative_path).ok_or_else(|| {
                boxed_error(format!(
                    "unsafe remote rename/move destination path: {}",
                    new_relative_path.display()
                ))
            })?;
        let old_remote_path = remote_root.join(&old_relative_path);
        let new_name = new_relative_path.file_name().ok_or_else(|| {
            boxed_error(format!(
                "rename/move destination has no file name: {}",
                new_relative_path.display()
            ))
        })?;
        let old_parent = old_relative_path.parent();
        let new_parent = new_relative_path.parent();

        if old_parent == new_parent {
            return self.rename_remote_entry(&old_remote_path, new_name);
        }

        let new_parent_remote = match new_parent {
            Some(parent) if !parent.as_os_str().is_empty() => remote_root.join(parent),
            _ => remote_root.to_path_buf(),
        };

        if old_relative_path.file_name() == Some(new_name) {
            return self.move_remote_entry(&old_remote_path, &new_parent_remote);
        }

        let old_name = old_relative_path.file_name().ok_or_else(|| {
            boxed_error(format!(
                "rename/move source has no file name: {}",
                old_relative_path.display()
            ))
        })?;
        self.move_remote_entry(&old_remote_path, &new_parent_remote)?;
        let moved_remote_path = new_parent_remote.join(old_name);
        self.rename_remote_entry(&moved_remote_path, new_name)
    }

    fn install_cancel_flag(&mut self, cancel_flag: Arc<AtomicBool>) {
        self.cancel_flag = cancel_flag;
    }
}

impl ProtonDriveClient {
    fn rename_remote_entry(&self, remote_path: &Path, new_name: &std::ffi::OsStr) -> AppResult<()> {
        let output = self.run_proton_drive(
            "rename",
            &[
                OsString::from("filesystem"),
                OsString::from("rename"),
                remote_path.as_os_str().to_os_string(),
                new_name.to_os_string(),
            ],
            1,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive rename failed for {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn move_remote_entry(&self, remote_path: &Path, target_parent: &Path) -> AppResult<()> {
        let output = self.run_proton_drive(
            "move",
            &[
                OsString::from("filesystem"),
                OsString::from("move"),
                remote_path.as_os_str().to_os_string(),
                target_parent.as_os_str().to_os_string(),
            ],
            1,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive move failed for {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn create_missing_directory_components(
        &self,
        remote_root: &Path,
        relative_path: &Path,
    ) -> AppResult<()> {
        let mut parent = remote_root.to_path_buf();

        for component in relative_path.components() {
            let component_name = component.as_os_str();
            let target = parent.join(component_name);

            if self.remote_path_exists(&target)? {
                parent = target;
                continue;
            }

            let output = self.run_proton_drive_quiet(
                "create-folder",
                &[
                    OsString::from("filesystem"),
                    OsString::from("create-folder"),
                    parent.as_os_str().to_os_string(),
                    component_name.to_os_string(),
                ],
                1,
            )?;
            if output.status.success() {
                parent = target;
                continue;
            }
            if self.remote_path_exists(&target)? {
                parent = target;
                continue;
            }
            return Err(boxed_error(format!(
                "proton-drive create-folder failed for {}: {}",
                target.display(),
                trimmed_stderr(&output)
            )));
        }

        Ok(())
    }

    fn remote_path_exists(&self, remote_path: &Path) -> AppResult<bool> {
        let output = self.run_proton_drive_quiet(
            "info",
            &[
                OsString::from("filesystem"),
                OsString::from("info"),
                remote_path.as_os_str().to_os_string(),
            ],
            1,
        )?;
        Ok(output.status.success())
    }

    fn deepest_existing_remote_parent(&self, remote_path: &Path) -> AppResult<Option<PathBuf>> {
        let mut candidate = remote_path.parent();
        while let Some(parent) = candidate {
            if parent.as_os_str().is_empty() {
                break;
            }
            if self.remote_path_exists(parent)? {
                return Ok(Some(parent.to_path_buf()));
            }
            candidate = parent.parent();
        }
        Ok(None)
    }

    fn run_proton_drive(
        &self,
        operation: &str,
        args: &[OsString],
        attempts: usize,
    ) -> AppResult<Output> {
        self.run_proton_drive_with_logging(operation, args, attempts, true)
    }

    fn run_proton_drive_quiet(
        &self,
        operation: &str,
        args: &[OsString],
        attempts: usize,
    ) -> AppResult<Output> {
        self.run_proton_drive_with_logging(operation, args, attempts, false)
    }

    fn run_proton_drive_with_logging(
        &self,
        operation: &str,
        args: &[OsString],
        attempts: usize,
        warn_on_unsuccessful_exit: bool,
    ) -> AppResult<Output> {
        let attempts = attempts.max(1);
        let mut last_error = None;
        for attempt in 1..=attempts {
            let output = match self.run_once(operation, args) {
                Ok(output) => output,
                Err(error) if attempt < attempts => {
                    warn!(
                        operation,
                        attempt,
                        attempts,
                        error = %error,
                        "retrying proton-drive command after error"
                    );
                    last_error = Some(error.to_string());
                    continue;
                }
                Err(error) => {
                    warn!(
                        operation,
                        attempt,
                        attempts,
                        error = %error,
                        "proton-drive command failed"
                    );
                    return Err(error);
                }
            };
            if output.status.success() || attempt == attempts {
                if !output.status.success() && warn_on_unsuccessful_exit {
                    warn!(
                        operation,
                        attempt,
                        attempts,
                        exit_status = ?output.status.code(),
                        stderr = %trimmed_stderr(&output),
                        "proton-drive command exited unsuccessfully"
                    );
                }
                return Ok(output);
            }
            let stderr = trimmed_stderr(&output);
            if warn_on_unsuccessful_exit {
                warn!(
                    operation,
                    attempt,
                    attempts,
                    exit_status = ?output.status.code(),
                    stderr = %stderr,
                    "retrying proton-drive command after unsuccessful exit"
                );
            }
            last_error = Some(format!("proton-drive {operation} failed: {stderr}"));
        }
        Err(boxed_error(last_error.unwrap_or_else(|| {
            format!("proton-drive {operation} failed")
        })))
    }

    fn run_once(&self, operation: &str, args: &[OsString]) -> AppResult<Output> {
        let mut child = self.spawn_once(args)?;
        let deadline = Instant::now() + self.command_policy.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                terminate_child_tree(&mut child);
                warn!(
                    operation,
                    timeout_ms = self.command_policy.timeout.as_millis(),
                    "proton-drive command timed out"
                );
                return Err(boxed_error(format!(
                    "proton-drive {operation} timed out after {}",
                    format_duration(self.command_policy.timeout)
                )));
            }
            let poll_interval = remaining.min(CANCELLATION_POLL_INTERVAL);
            if child.wait_timeout(poll_interval)?.is_some() {
                return Ok(child.wait_with_output()?);
            }
            if self.cancel_flag.load(Ordering::SeqCst) {
                terminate_child_tree(&mut child);
                warn!(
                    operation,
                    "proton-drive command cancelled before completion"
                );
                return Err(boxed_error(format!(
                    "proton-drive {operation} cancelled before completion"
                )));
            }
        }
    }

    fn spawn_once(&self, args: &[OsString]) -> AppResult<Child> {
        let mut busy_attempts = 0;
        loop {
            let mut command = Command::new(&self.executable);
            command
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // Put the child in its own process group (group id == its own pid) so
            // that a timed-out or cancelled command can be terminated as a whole
            // tree via `terminate_child_tree`. Without this, killing only the
            // directly spawned process (for example a shell wrapper) leaves any
            // grandchildren it forked (for example a `sleep` invoked mid-script)
            // running and still holding the piped stdout/stderr file descriptors
            // open, which blocks output collection until those orphans exit on
            // their own instead of returning promptly.
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            match command.spawn() {
                Ok(child) => return Ok(child),
                Err(error)
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && busy_attempts < EXECUTABLE_BUSY_SPAWN_ATTEMPTS =>
                {
                    busy_attempts += 1;
                    std::thread::sleep(EXECUTABLE_BUSY_RETRY_DELAY);
                }
                Err(error) => return Err(Box::new(error)),
            }
        }
    }
}

/// Terminates a spawned `proton-drive` command and every descendant it may have
/// forked, then reaps the direct child without reading its output.
///
/// `spawn_once` places the child in its own process group, so on Unix this signals
/// the whole group (`kill(-pid, SIGKILL)`) instead of only the directly tracked
/// process. This matters because shell-wrapped or multi-step commands can fork
/// grandchildren that inherit the piped stdout/stderr file descriptors; killing
/// only the direct child would leave those descendants running; reading output
/// via `wait_with_output` would then block until every holder of the pipe's
/// write end exits on its own, defeating the purpose of a prompt timeout or
/// cancellation. Using `wait` (not `wait_with_output`) here avoids that same
/// blocking read for the direct child's own pipes.
fn terminate_child_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        // SAFETY: `kill` only sends a signal to the process group headed by
        // `pid`; it does not dereference any pointers and cannot cause undefined
        // behavior regardless of whether that group still exists.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn trimmed_stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

fn is_node_not_found(output: &Output) -> bool {
    trimmed_stderr(output)
        .to_ascii_lowercase()
        .contains("node not found")
}

fn clean_remote_root_path(path: &Path) -> Option<PathBuf> {
    let mut clean_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => clean_path.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    if clean_path.as_os_str().is_empty() {
        None
    } else {
        Some(clean_path)
    }
}

/// Removes its scratch directory (recursively) when dropped, including on early
/// returns via `?`. Best-effort: a failure to clean up is silently ignored rather
/// than masking the caller's real result, since leftover scratch directories are
/// harmless clutter, not a correctness or safety problem.
struct ScratchDirGuard {
    path: PathBuf,
}

impl ScratchDirGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }
}

impl Drop for ScratchDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Returns the path of the single entry inside `directory`, failing if it contains
/// zero or more than one entry. Used to locate a just-downloaded file without
/// assuming the exact name the `proton-drive` CLI chose for it.
fn single_entry_in_directory(directory: &Path) -> AppResult<PathBuf> {
    let mut entries = fs::read_dir(directory)?;
    let first = entries.next().transpose()?;
    let second = entries.next().transpose()?;
    match (first, second) {
        (Some(entry), None) => Ok(entry.path()),
        (None, None) => Err(boxed_error(format!(
            "no entries were found in {}",
            directory.display()
        ))),
        _ => Err(boxed_error(format!(
            "expected exactly one entry in {}, found more than one",
            directory.display()
        ))),
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.as_millis().is_multiple_of(1000) {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn deserialize_optional_digest_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Ok(None),
    }
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        Some(Value::String(value)) => Ok(Some(value)),
        Some(Value::Object(object)) => Ok(object
            .get("value")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)),
        _ => Ok(None),
    }
}

fn deserialize_optional_active_revision<'de, D>(
    deserializer: D,
) -> Result<Option<ActiveRevision>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let revision = match value {
        Value::Object(mut object) => object.remove("value").unwrap_or(Value::Object(object)),
        other => other,
    };
    Ok(serde_json::from_value(revision).ok())
}

pub fn parse_remote_files(
    json: &str,
    remote_root: &Path,
) -> AppResult<HashMap<PathBuf, RemoteFile>> {
    Ok(parse_remote_listing(json, remote_root, Path::new(""))?.files)
}

pub fn parse_remote_entities(
    json: &str,
    remote_root: &Path,
) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
    Ok(parse_remote_listing(json, remote_root, Path::new(""))?.into_entities())
}

fn parse_remote_listing(
    json: &str,
    remote_root: &Path,
    parent_path: &Path,
) -> AppResult<RemoteListing> {
    let value: Value = serde_json::from_str(json)?;
    let mut listing = RemoteListing::default();
    collect_from_value(&value, parent_path, remote_root, &mut listing)?;
    Ok(listing)
}

fn collect_from_value(
    value: &Value,
    parent_path: &Path,
    remote_root: &Path,
    listing: &mut RemoteListing,
) -> AppResult<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_from_value(item, parent_path, remote_root, listing)?;
            }
        }
        Value::Object(object) => {
            // Deserialize once and let collect_node own all parent-path propagation.
            let node: ProtonNode = serde_json::from_value(Value::Object(object.clone()))?;
            collect_node(&node, parent_path, remote_root, listing)?;
        }
        _ => {}
    }
    Ok(())
}

fn collect_node(
    node: &ProtonNode,
    parent_path: &Path,
    remote_root: &Path,
    listing: &mut RemoteListing,
) -> AppResult<()> {
    let candidate_path = node
        .path
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| node.name.as_deref().map(|name| parent_path.join(name)));

    // If a path was provided but fails normalization (absolute path, `..` escape,
    // or root component), skip the node and all of its descendants.
    let relative_path = match candidate_path.as_deref() {
        Some(p) => match normalize_remote_path(p, remote_root) {
            Some(normalized) => normalized,
            None => return Ok(()),
        },
        None => PathBuf::new(),
    };

    let is_folder = node.is_folder.unwrap_or(false)
        || matches!(node.kind.as_deref(), Some("folder" | "directory"))
        || (!node.children.is_empty() || !node.entries.is_empty() || !node.files.is_empty());

    let id = node.id.as_deref().or(node.uid.as_deref());
    if is_folder && !relative_path.as_os_str().is_empty() {
        let name = node
            .name
            .clone()
            .or_else(|| {
                relative_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| relative_path.display().to_string());
        listing.directories.insert(
            relative_path.clone(),
            RemoteDirectory {
                path: relative_path.clone(),
                id: id.map(ToOwned::to_owned),
                name,
            },
        );
    }

    if !is_folder && let (Some(id), Some(name)) = (id, node.name.as_deref()) {
        listing.files.insert(
            relative_path.clone(),
            RemoteFile {
                path: relative_path.clone(),
                id: id.to_owned(),
                name: name.to_owned(),
                sha1_hash: node
                    .active_revision
                    .as_ref()
                    .and_then(|revision| revision.claimed_digests.as_ref())
                    .and_then(|digests| digests.sha1.clone()),
                downloadable: is_downloadable_media_type(node.media_type.as_deref()),
            },
        );
    }

    let next_parent = if relative_path.as_os_str().is_empty() {
        parent_path.to_path_buf()
    } else {
        relative_path
    };

    for child in &node.entries {
        collect_node(child, &next_parent, remote_root, listing)?;
    }
    for child in &node.children {
        collect_node(child, &next_parent, remote_root, listing)?;
    }
    for child in &node.files {
        collect_node(child, &next_parent, remote_root, listing)?;
    }

    Ok(())
}

fn is_downloadable_media_type(media_type: Option<&str>) -> bool {
    let Some(media_type) = media_type else {
        return true;
    };
    let normalized = media_type.to_ascii_lowercase();
    !matches!(
        normalized.as_str(),
        "application/vnd.proton.doc" | "application/vnd.proton.sheet"
    )
}

/// Normalize a Proton-reported path to a clean relative path within the sync
/// directory.  Returns `None` (and the calling node is silently skipped) when
/// the path:
///
/// * is absolute and does not start with `remote_root` or its basename,
/// * contains `..` (parent-directory) components, or
/// * contains any component (prefix, root dir) that could escape the local
///   sync directory when joined with `local_root`.
///
/// `CurDir` (`.`) components are stripped so the result only contains
/// `Normal` components and is safe to use as a `HashMap` key.
fn normalize_remote_path(path: &Path, remote_root: &Path) -> Option<PathBuf> {
    let relative = if let Ok(stripped) = path.strip_prefix(remote_root) {
        stripped.to_path_buf()
    } else if let Some(root_name) = remote_root.file_name()
        && let Ok(stripped) = path.strip_prefix(root_name)
    {
        stripped.to_path_buf()
    } else if path.is_relative() {
        path.to_path_buf()
    } else {
        // Absolute path that does not start with the expected remote root.
        return None;
    };

    crate::validate_relative_path(&relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use tempfile::tempdir;

    #[test]
    fn parses_nested_remote_files_and_sha1_values() {
        let json = r#"
        {
          "entries": [
            {
              "name": "Documents",
              "type": "folder",
              "entries": [
                {
                  "id": "file-1",
                  "name": "notes.txt",
                  "activeRevision": {
                    "claimedDigests": {
                      "sha1": "abc123"
                    }
                  }
                }
              ]
            },
            {
              "id": "file-2",
              "name": "root.txt",
              "path": "/Drive/root.txt",
              "activeRevision": {
                "claimedDigests": {
                  "sha1": "def456"
                }
              }
            }
          ]
        }
        "#;

        let files = parse_remote_files(json, Path::new("/Drive")).expect("parse remote files");

        let documents_file = files
            .get(Path::new("Documents/notes.txt"))
            .expect("nested document file");
        assert_eq!(documents_file.id, "file-1");
        assert_eq!(documents_file.sha1_hash.as_deref(), Some("abc123"));

        let root_file = files.get(Path::new("root.txt")).expect("root file");
        assert_eq!(root_file.id, "file-2");
        assert_eq!(root_file.sha1_hash.as_deref(), Some("def456"));
    }

    #[test]
    fn tolerates_missing_active_revisions() {
        let json = r#"[{"id":"file-1","name":"draft.txt"}]"#;

        let files = parse_remote_files(json, Path::new("/Drive")).expect("parse remote files");
        let file = files.get(Path::new("draft.txt")).expect("draft file");

        assert_eq!(file.sha1_hash, None);
    }

    #[test]
    fn tolerates_non_string_claimed_sha1_digest() {
        let json = r#"
                [
                    {
                        "id": "file-1",
                        "name": "budget.xlsx",
                        "activeRevision": {
                            "claimedDigests": {
                                "sha1": {
                                    "value": "1111111111111111111111111111111111111111"
                                }
                            }
                        }
                    }
                ]
                "#;

        let files = parse_remote_files(json, Path::new("/Drive")).expect("parse remote files");
        let file = files.get(Path::new("budget.xlsx")).expect("Excel file");

        assert_eq!(file.sha1_hash, None);
    }

    #[test]
    fn parses_wrapped_cli_metadata_fields() {
        let json = r#"
                [
                    {
                        "uid": "file-uid",
                        "name": {
                            "ok": true,
                            "value": "budget.xlsx"
                        },
                        "type": "file",
                        "activeRevision": {
                            "ok": true,
                            "value": {
                                "claimedDigests": {
                                    "sha1": "1111111111111111111111111111111111111111"
                                }
                            }
                        }
                    },
                    {
                        "uid": "folder-uid",
                        "name": {
                            "ok": true,
                            "value": "Reports"
                        },
                        "type": "folder"
                    }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/my-files/demo/")).expect("parse remote files");
        let file = files.get(Path::new("budget.xlsx")).expect("Excel file");

        assert_eq!(files.len(), 1);
        assert_eq!(file.id, "file-uid");
        assert_eq!(file.name, "budget.xlsx");
        assert_eq!(
            file.sha1_hash.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
    }

    #[test]
    fn parses_remote_directory_id_from_wrapped_cli_metadata() {
        let json = r#"
                [
                    {
                        "uid": "folder-uid",
                        "name": {
                            "ok": true,
                            "value": "Reports"
                        },
                        "type": "folder"
                    }
                ]
                "#;

        let entities = parse_remote_entities(json, Path::new("/my-files/demo/"))
            .expect("parse remote entities");
        let directory = entities
            .get(Path::new("Reports"))
            .and_then(RemoteEntity::as_directory)
            .expect("Reports directory entity");

        assert_eq!(directory.id.as_deref(), Some("folder-uid"));
        assert_eq!(directory.name, "Reports");
    }

    #[test]
    fn parses_remote_directory_with_no_id_as_none() {
        let json = r#"
                [
                    {
                        "name": "Documents",
                        "type": "folder"
                    }
                ]
                "#;

        let entities =
            parse_remote_entities(json, Path::new("/Drive")).expect("parse remote entities");
        let directory = entities
            .get(Path::new("Documents"))
            .and_then(RemoteEntity::as_directory)
            .expect("Documents directory entity");

        assert_eq!(directory.id, None);
    }

    #[test]
    fn marks_proton_native_sheets_as_not_downloadable() {
        let json = r#"
                [
                    {
                        "uid": "sheet-uid",
                        "name": {
                            "ok": true,
                            "value": "Untitled spreadsheet"
                        },
                        "type": "file",
                        "mediaType": "application/vnd.proton.sheet",
                        "activeRevision": {
                            "ok": true,
                            "value": {
                                "claimedDigests": {
                                    "sha1Verified": false
                                }
                            }
                        }
                    }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/my-files/demo/")).expect("parse remote files");
        let file = files
            .get(Path::new("Untitled spreadsheet"))
            .expect("Proton sheet");

        assert_eq!(file.id, "sheet-uid");
        assert_eq!(file.sha1_hash, None);
        assert!(!file.downloadable);
    }

    #[test]
    fn parses_filesystem_list_fixture() {
        let json = include_str!("../tests/fixtures/proton-drive-filesystem-list.json");

        let files = parse_remote_files(json, Path::new("/Drive/RemoteFolder"))
            .expect("parse filesystem list fixture");

        assert_eq!(files.len(), 3);
        assert_eq!(
            files
                .get(Path::new("Documents/notes.txt"))
                .expect("nested note")
                .sha1_hash
                .as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(
            files
                .get(Path::new("root.md"))
                .expect("root markdown file")
                .id,
            "file-root"
        );
        assert!(
            files
                .get(Path::new("Drafts/digest-missing.txt"))
                .expect("digest-missing file")
                .sha1_hash
                .is_none(),
            "fixture should preserve remote files whose digest is unavailable"
        );
        assert!(
            !files.contains_key(Path::new("passwd")),
            "fixture path outside the configured root must be skipped"
        );
    }

    #[test]
    fn nested_file_is_not_emitted_at_unqualified_root_path() {
        // Regression: the old collect_from_value walked entries/files/children before
        // calling collect_node, so nested files without an explicit `path` field were
        // inserted twice – once under the unqualified root name and once under the
        // correct parent-relative name.
        let json = r#"
        {
          "entries": [
            {
              "name": "Docs",
              "type": "folder",
              "entries": [
                {
                  "id": "nested-1",
                  "name": "nested.txt"
                }
              ]
            }
          ]
        }
        "#;

        let files = parse_remote_files(json, Path::new("/Drive")).expect("parse remote files");

        // The file must appear only under its qualified path.
        assert!(
            files.contains_key(Path::new("Docs/nested.txt")),
            "qualified path must be present"
        );
        // The bare, unqualified name must NOT appear as a separate entry.
        assert!(
            !files.contains_key(Path::new("nested.txt")),
            "unqualified root-level duplicate must not be emitted"
        );
    }

    #[test]
    fn normalize_remote_path_rejects_traversal_and_absolute_paths() {
        // Parent-directory component must be rejected.
        assert_eq!(
            normalize_remote_path(Path::new("../secret"), Path::new("/Drive")),
            None,
            "path with .. must be rejected"
        );

        // Absolute path that does not start with remote_root must be rejected.
        assert_eq!(
            normalize_remote_path(Path::new("/etc/passwd"), Path::new("/Drive")),
            None,
            "absolute path outside remote_root must be rejected"
        );

        // Nested .. disguised inside a longer path.
        assert_eq!(
            normalize_remote_path(Path::new("Documents/../../etc/passwd"), Path::new("/Drive")),
            None,
            "embedded .. must be rejected"
        );

        // A valid relative path must succeed.
        assert_eq!(
            normalize_remote_path(Path::new("Documents/notes.txt"), Path::new("/Drive")),
            Some(PathBuf::from("Documents/notes.txt")),
            "valid relative path must pass through"
        );

        // A valid absolute path rooted at remote_root must succeed.
        assert_eq!(
            normalize_remote_path(Path::new("/Drive/notes.txt"), Path::new("/Drive")),
            Some(PathBuf::from("notes.txt")),
            "absolute path under remote_root must be stripped correctly"
        );

        // CurDir (.) components must be stripped so the result is a canonical Normal-only path.
        assert_eq!(
            normalize_remote_path(Path::new("./Documents/notes.txt"), Path::new("/Drive")),
            Some(PathBuf::from("Documents/notes.txt")),
            "leading CurDir must be stripped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_times_out_when_cli_hangs() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
sleep 2
printf '{"entries":[]}\n'
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_millis(100), 1),
        );

        let started = Instant::now();
        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("hung proton-drive should time out");

        assert!(
            error.to_string().contains("timed out after 100ms"),
            "unexpected error: {error}"
        );
        // Regression guard: the timed-out command's process group must actually be
        // killed, not just the direct child. Previously `child.kill()` only reaped
        // the shell wrapper while an orphaned `sleep` grandchild kept the piped
        // stdout/stderr open, so `wait_with_output()` blocked until the full 2s
        // sleep elapsed even though the error already claimed a 100ms timeout.
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "timing out should not block on the fake CLI's full hang duration, took {:?}",
            started.elapsed()
        );
    }

    // Regression guard for the cooperative-cancellation polling loop in `run_once`:
    // a command that is never cancelled and completes well before its timeout must
    // behave exactly as before (no added latency from the new short polling interval).
    #[cfg(unix)]
    #[test]
    fn list_completes_normally_when_never_cancelled() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '{"entries":[]}\n'
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(5), 1),
        );

        let started = Instant::now();
        client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect("an uncancelled, quickly completing command should still succeed");

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "an immediately completing command must not be delayed by the cancellation \
             polling loop"
        );
    }

    // Proves the cooperative-cancellation feature this polling loop exists for: setting
    // the shared cancel flag while a command is genuinely blocked (the fake CLI sleeps
    // far longer than the poll interval) causes `run_once` to notice within about one
    // poll interval and kill the child, instead of waiting for the full configured
    // timeout or for the child to exit on its own.
    #[cfg(unix)]
    #[test]
    fn list_is_cancelled_promptly_instead_of_waiting_for_the_full_timeout() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
sleep 5
printf '{"entries":[]}\n'
"#,
        );
        let mut client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(30), 1),
        );
        let cancel_flag = Arc::new(AtomicBool::new(false));
        client.install_cancel_flag(Arc::clone(&cancel_flag));

        std::thread::spawn({
            let cancel_flag = Arc::clone(&cancel_flag);
            move || {
                std::thread::sleep(Duration::from_millis(150));
                cancel_flag.store(true, Ordering::SeqCst);
            }
        });

        let started = Instant::now();
        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("a cancelled command should return an error");
        let elapsed = started.elapsed();

        assert!(
            error.to_string().contains("cancelled before completion"),
            "unexpected error: {error}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "cancellation should be noticed within about one poll interval, not the full \
             30s configured timeout or the fake CLI's 5s sleep; elapsed: {elapsed:?}"
        );
    }

    // Regression test for a real bug found via live E2E testing: `spawn_once` did not
    // configure the child's stdin, so it inherited whatever stdin the calling process
    // happened to have. When that stdin was a live, interactive terminal, the real
    // proton-drive CLI (a Node.js process) kept an event-loop handle open waiting for
    // input that would never arrive, so the process never exited even though the
    // requested operation had already completed - causing every caller to hang until
    // `CommandPolicy`'s timeout forcibly killed the child. The fix is to always give the
    // child a closed stdin (`Stdio::null()`), since this is a non-interactive CLI wrapper
    // that never needs to read input from its caller.
    #[cfg(unix)]
    #[test]
    fn spawned_commands_do_not_inherit_an_open_stdin() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if read line; then
    echo "unexpected stdin data: $line" > "$0.stdin_result"
else
    echo "stdin closed" > "$0.stdin_result"
fi
printf '{"entries":[]}\n'
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(2), 1),
        );

        client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect("list should complete promptly regardless of the caller's own stdin");

        let stdin_result_path = PathBuf::from(format!("{}.stdin_result", executable.display()));
        assert_eq!(
            fs::read_to_string(&stdin_result_path).expect("recorded stdin observation"),
            "stdin closed\n",
            "the child process must always receive a closed stdin, never an inherited one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_stages_through_a_scratch_directory_and_moves_the_result_into_place() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
target_dir="$4"
printf 'downloaded content\n' > "$target_dir/budget.xlsx"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let destination = local_folder.join("budget.xlsx");

        client
            .download(Path::new("/my-files/demo/budget.xlsx"), &destination)
            .expect("download command");

        let recorded_args = fs::read_to_string(args_path(&executable)).expect("recorded args");
        let mut lines = recorded_args.lines();
        assert_eq!(lines.next(), Some("filesystem"));
        assert_eq!(lines.next(), Some("download"));
        assert_eq!(lines.next(), Some("/my-files/demo/budget.xlsx"));
        let scratch_dir = lines.next().expect("scratch directory argument");
        let scratch_prefix = local_folder.join(crate::DOWNLOAD_SCRATCH_PREFIX);
        assert!(
            scratch_dir.starts_with(scratch_prefix.to_str().expect("utf-8 scratch prefix")),
            "download should stage into a private scratch directory under the \
             destination's own parent folder, not the parent folder directly: {scratch_dir}"
        );
        assert!(
            lines.next().is_none(),
            "no further CLI arguments were expected"
        );

        assert_eq!(
            fs::read_to_string(&destination).expect("downloaded file at destination"),
            "downloaded content\n",
            "the downloaded content should end up at exactly the requested destination path"
        );
        assert!(
            !Path::new(scratch_dir).exists(),
            "the scratch directory should be cleaned up after a successful download"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_to_a_sidecar_name_does_not_clobber_a_file_matching_the_remote_basename() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
target_dir="$4"
printf 'remote content\n' > "$target_dir/notes.txt"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let local_folder = directory.path().join("run");
        fs::create_dir_all(&local_folder).expect("create local run folder");
        // A local file already exists with the same basename the remote source has.
        // This models conflict resolution: the caller deliberately requests a
        // *different* local name for the download (a `.proton-cloud.` sidecar)
        // specifically so this file is left untouched. Regression test for a bug
        // where `download()` trusted the CLI to name the result after `destination`,
        // when the real CLI always names it after the remote source instead - which
        // silently overwrote the very file conflict resolution was supposed to
        // preserve.
        let existing_local_path = local_folder.join("notes.txt");
        fs::write(&existing_local_path, "locally edited content\n").expect("write local edit");
        let sidecar_destination = local_folder.join("notes.proton-cloud.txt");

        client
            .download(Path::new("/my-files/run/notes.txt"), &sidecar_destination)
            .expect("download command");

        assert_eq!(
            fs::read_to_string(&existing_local_path).expect("read local file"),
            "locally edited content\n",
            "downloading a conflicting remote revision under a sidecar name must never \
             touch the local file that shares the remote's own basename"
        );
        assert_eq!(
            fs::read_to_string(&sidecar_destination).expect("read sidecar file"),
            "remote content\n",
            "the remote's content should land at exactly the requested sidecar destination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_fails_cleanly_when_the_cli_produces_no_file() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let destination = local_folder.join("budget.xlsx");

        let error = client
            .download(Path::new("/my-files/demo/budget.xlsx"), &destination)
            .expect_err("a CLI that produces no file should be reported as an error");

        assert!(
            error.to_string().contains("no entries were found"),
            "unexpected error: {error}"
        );
        assert!(
            !destination.exists(),
            "no file should appear at the destination when the download produced nothing"
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_recurses_into_remote_folders() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
    printf '%s\n' "$4" >> "$0.args"
    if [ "$4" = "/my-files/demo" ]; then
        cat <<'JSON'
[
    {
        "uid": "folder-id",
        "name": { "ok": true, "value": "cloud-folder" },
        "type": "folder"
    },
    {
        "uid": "root-id",
        "name": { "ok": true, "value": "root.txt" },
        "type": "file",
        "activeRevision": {
            "ok": true,
            "value": {
                "claimedDigests": {
                    "sha1": "1111111111111111111111111111111111111111"
                }
            }
        }
    }
]
JSON
        exit 0
    fi
    if [ "$4" = "/my-files/demo/cloud-folder" ]; then
        cat <<'JSON'
[
    {
        "uid": "nested-id",
        "name": { "ok": true, "value": "nested.txt" },
        "type": "file",
        "activeRevision": {
            "ok": true,
            "value": {
                "claimedDigests": {
                    "sha1": "2222222222222222222222222222222222222222"
                }
            }
        }
    }
]
JSON
        exit 0
    fi
fi
echo "unexpected args: $*" >&2
exit 64
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 2),
        );

        let files = client
            .list(Path::new("/my-files/demo"))
            .expect("recursive remote list");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded list paths"),
            "/my-files/demo\n/my-files/demo/cloud-folder\n"
        );
        assert_eq!(
            files
                .get(Path::new("root.txt"))
                .expect("root file")
                .sha1_hash
                .as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(
            files
                .get(Path::new("cloud-folder/nested.txt"))
                .expect("nested file")
                .sha1_hash
                .as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_directory_creates_missing_components_in_order() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "info" ]; then
  printf 'info:%s\n' "$3" >> "$0.args"
  exit 1
fi
if [ "$1" = "filesystem" ] && [ "$2" = "create-folder" ]; then
  printf 'create-folder:%s:%s\n' "$3" "$4" >> "$0.args"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 64
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .ensure_directory(
                Path::new("/my-files/demo"),
                Path::new("local-sub-directory/nested"),
            )
            .expect("ensure remote directory");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "info:/my-files/demo/local-sub-directory\n\
create-folder:/my-files/demo:local-sub-directory\n\
info:/my-files/demo/local-sub-directory/nested\n\
create-folder:/my-files/demo/local-sub-directory:nested\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_root_directory_creates_missing_components_below_existing_parent() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "info" ]; then
    printf 'info:%s\n' "$3" >> "$0.args"
    if [ "$3" = "/my-files" ]; then
        exit 0
    fi
    exit 1
fi
if [ "$1" = "filesystem" ] && [ "$2" = "create-folder" ]; then
    printf 'create-folder:%s:%s\n' "$3" "$4" >> "$0.args"
    exit 0
fi
echo "unexpected args: $*" >&2
exit 64
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .ensure_root_directory(Path::new("/my-files/demo/nested"))
            .expect("ensure missing remote root directory");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "info:/my-files/demo/nested\n\
info:/my-files/demo\n\
info:/my-files\n\
info:/my-files/demo\n\
create-folder:/my-files:demo\n\
info:/my-files/demo/nested\n\
create-folder:/my-files/demo:nested\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn delete_uses_filesystem_trash_command() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .delete(Path::new("/my-files/demo/removed.txt"))
            .expect("delete command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\ntrash\n/my-files/demo/removed.txt\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_renames_in_place_when_parent_is_unchanged() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("old-name.txt"),
                Path::new("new-name.txt"),
            )
            .expect("rename command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\nrename\n/my-files/demo/old-name.txt\nnew-name.txt\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_passes_spaces_and_special_characters_through_a_single_argument() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("weird name (v1) & co's file!.txt"),
                Path::new("weird name (v2) [final] $$ *.txt"),
            )
            .expect("rename command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\nrename\n/my-files/demo/weird name (v1) & co's file!.txt\nweird name (v2) [final] $$ *.txt\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_moves_when_only_parent_changes() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("old-folder/report.txt"),
                Path::new("new-folder/report.txt"),
            )
            .expect("move command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\nmove\n/my-files/demo/old-folder/report.txt\n/my-files/demo/new-folder\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_moves_to_root_when_new_parent_is_empty() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("nested/report.txt"),
                Path::new("report.txt"),
            )
            .expect("move command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\nmove\n/my-files/demo/nested/report.txt\n/my-files/demo\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_moves_then_renames_when_parent_and_name_change() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if [ "$2" = "move" ]; then
  printf 'move:%s:%s\n' "$3" "$4" >> "$0.args"
  exit 0
fi
if [ "$2" = "rename" ]; then
  printf 'rename:%s:%s\n' "$3" "$4" >> "$0.args"
  exit 0
fi
echo "unexpected args: $*" >&2
exit 64
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("old-folder/old-name.txt"),
                Path::new("new-folder/new-name.txt"),
            )
            .expect("move then rename command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "move:/my-files/demo/old-folder/old-name.txt:/my-files/demo/new-folder\n\
             rename:/my-files/demo/new-folder/old-name.txt:new-name.txt\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_reports_cli_failure() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
echo "boom" >&2
exit 1
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let error = client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("old-name.txt"),
                Path::new("new-name.txt"),
            )
            .expect_err("rename command should fail");

        assert!(error.to_string().contains("boom"));
    }

    #[cfg(unix)]
    #[test]
    fn list_retries_transient_cli_failure() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
counter="$0.attempts"
attempts=0
if [ -f "$counter" ]; then
  attempts=$(cat "$counter")
fi
attempts=$((attempts + 1))
printf '%s\n' "$attempts" > "$counter"
if [ "$attempts" -eq 1 ]; then
  echo "transient failure" >&2
  exit 75
fi
printf '{"entries":[]}\n'
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 2),
        );

        let files = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect("second list attempt should succeed");

        assert!(files.is_empty());
        assert_eq!(
            fs::read_to_string(attempt_counter_path(&executable)).expect("attempt counter"),
            "2\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn upload_passes_a_file_conflict_strategy_to_avoid_the_interactive_prompt() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        client
            .upload(
                Path::new("/tmp/local/notes.txt"),
                Path::new("/my-files/demo"),
                Path::new("notes.txt"),
            )
            .expect("upload command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\nupload\n--file-conflict-strategy\nreplace\n/tmp/local/notes.txt\n/my-files/demo/\n",
            "upload must always pass an explicit conflict strategy so revising an \
             already-synced file replaces its content instead of silently skipping it \
             behind a stdin-less interactive prompt (the real proton-drive CLI prompts \
             for a strategy whenever the destination already has a same-named file, and \
             defaults to skipping the file - while still exiting 0 - when that prompt \
             sees immediate EOF)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn upload_rejects_unsafe_relative_path() {
        let client = ProtonDriveClient::with_command_policy(
            PathBuf::from("/does/not/matter"),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let error = client
            .upload(
                Path::new("/tmp/local/notes.txt"),
                Path::new("/my-files/demo"),
                Path::new("../escape.txt"),
            )
            .expect_err("upload must reject a relative path that could escape remote_root");

        assert!(
            error.to_string().contains("unsafe remote upload path"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_or_move_rejects_unsafe_relative_paths() {
        let client = ProtonDriveClient::with_command_policy(
            PathBuf::from("/does/not/matter"),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let source_error = client
            .rename_or_move(
                Path::new("/Drive/RemoteFolder"),
                Path::new("../escape.txt"),
                Path::new("safe.txt"),
            )
            .expect_err("rename_or_move must reject an unsafe source path");
        assert!(
            source_error
                .to_string()
                .contains("unsafe remote rename/move source path"),
            "unexpected error: {source_error}"
        );

        let destination_error = client
            .rename_or_move(
                Path::new("/Drive/RemoteFolder"),
                Path::new("safe.txt"),
                Path::new("/absolute/escape.txt"),
            )
            .expect_err("rename_or_move must reject an unsafe destination path");
        assert!(
            destination_error
                .to_string()
                .contains("unsafe remote rename/move destination path"),
            "unexpected error: {destination_error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn upload_is_not_retried_after_cli_failure() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
counter="$0.attempts"
attempts=0
if [ -f "$counter" ]; then
  attempts=$(cat "$counter")
fi
attempts=$((attempts + 1))
printf '%s\n' "$attempts" > "$counter"
echo "upload failed" >&2
exit 75
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 3),
        );

        let error = client
            .upload(
                Path::new("/tmp/local.txt"),
                Path::new("/Drive/RemoteFolder"),
                Path::new("local.txt"),
            )
            .expect_err("failed upload should not be retried");

        assert!(
            error.to_string().contains("proton-drive upload failed"),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::read_to_string(attempt_counter_path(&executable)).expect("attempt counter"),
            "1\n"
        );
    }

    #[cfg(unix)]
    fn write_script(directory: &Path, name: &str, content: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, content).expect("write fake proton-drive");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("script permissions");
        path
    }

    #[cfg(unix)]
    fn attempt_counter_path(executable: &Path) -> PathBuf {
        PathBuf::from(format!("{}.attempts", executable.display()))
    }

    #[cfg(unix)]
    fn args_path(executable: &Path) -> PathBuf {
        PathBuf::from(format!("{}.args", executable.display()))
    }
}
