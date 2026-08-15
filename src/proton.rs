use crate::index::compute_sha1;
use crate::{AppResult, boxed_error};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
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
/// Minimum window `run_once` gives the output-pipe readers after the child has exited, used when
/// the command's own deadline has already expired at that moment. Normally the readers hit EOF the
/// instant the child exits; this only bites when something else (a forked grandchild that inherited
/// the pipe write ends) still holds the pipe open. Worst case `run_once` therefore returns within
/// `CommandPolicy::timeout` + this grace instead of blocking forever (issue #56).
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(1);

/// A CLI `{ok, value}`-wrapped string, keeping apart the three states a plain `Option<String>`
/// collapses into `None`:
///
/// * [`Self::Absent`] — the field is missing or `null`. Legitimate and common (`path` and `id`
///   are routinely not sent), so it keeps the historical silent-drop behaviour.
/// * [`Self::Undecodable`] — a value was **present** but carries no usable string (`{"ok": false,
///   ...}`, `{}`, `{"value": null}`, or a non-string scalar). The node exists remotely and this
///   listing cannot describe it, which is *not* the same as the node being absent (issue #59).
/// * [`Self::Decoded`] — a usable string, from either a bare string or the wrapper's `value`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum WrappedString {
    #[default]
    Absent,
    Undecodable,
    Decoded(String),
}

impl WrappedString {
    pub fn as_deref(&self) -> Option<&str> {
        match self {
            Self::Decoded(value) => Some(value.as_str()),
            Self::Absent | Self::Undecodable => None,
        }
    }

    fn is_undecodable(&self) -> bool {
        matches!(self, Self::Undecodable)
    }
}

impl<'de> Deserialize<'de> for WrappedString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<Value>::deserialize(deserializer)? {
            None | Some(Value::Null) => Self::Absent,
            Some(Value::String(value)) => Self::Decoded(value),
            Some(Value::Object(object)) => {
                if matches!(object.get("ok"), Some(Value::Bool(false))) {
                    Self::Undecodable
                } else {
                    match object.get("value").and_then(Value::as_str) {
                        Some(value) => Self::Decoded(value.to_owned()),
                        None => Self::Undecodable,
                    }
                }
            }
            // A present non-string scalar is a value we cannot read, not an absent one.
            Some(_) => Self::Undecodable,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtonNode {
    // Identity and locator fields carry the three-state wrapper: an undecodable one fails the
    // listing rather than silently dropping the node (#59). Non-structural metadata below stays
    // on the lenient `Option<String>` parse — an unreadable media type cannot lose a node.
    #[serde(default)]
    pub id: WrappedString,
    #[serde(default)]
    pub uid: WrappedString,
    #[serde(default)]
    pub name: WrappedString,
    #[serde(default)]
    pub path: WrappedString,
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

/// Stand-in for `Child::wait_timeout` (see [`ProtonDriveClient::wait_hook`]). A `waitpid` failure
/// cannot be provoked from outside the process, so this seam is what makes `run_once`'s
/// terminate-on-wait-failure exit (issue #57) reachable from a test.
type WaitHook = Arc<dyn Fn(&mut Child, Duration) -> io::Result<Option<ExitStatus>> + Send + Sync>;

#[derive(Clone)]
pub struct ProtonDriveClient {
    executable: PathBuf,
    command_policy: CommandPolicy,
    cancel_flag: Arc<AtomicBool>,
    progress_sink: Option<Arc<dyn ProgressSink>>,
    /// Test seam only; always `None` in production, where `run_once` waits via
    /// `wait_timeout::ChildExt`. See [`WaitHook`].
    wait_hook: Option<WaitHook>,
}

// Manual impl: `dyn ProgressSink` has no `Debug`, so the derive is no longer available.
impl std::fmt::Debug for ProtonDriveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtonDriveClient")
            .field("executable", &self.executable)
            .field("command_policy", &self.command_policy)
            .field("cancel_flag", &self.cancel_flag)
            .field("progress_sink", &self.progress_sink.is_some())
            .field("wait_hook", &self.wait_hook.is_some())
            .finish()
    }
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
    /// Lists a **single** remote directory (non-recursively) relative to `remote_root`, returning
    /// its immediate entries keyed by relative path. This is the O(1) targeted sibling of the
    /// O(folders) [`Self::list_entities_or_missing_root`] BFS, used by event-driven reconcile to
    /// resolve just the parent of a changed node instead of re-walking the whole tree. The
    /// default implementation errors; clients that can target a single directory override it.
    fn list_directory(
        &self,
        _remote_root: &Path,
        relative_directory: &Path,
    ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
        Err(boxed_error(format!(
            "single-directory listing is not supported by this Proton client: {}",
            relative_directory.display()
        )))
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
    /// Downloads several remote files in one operation, returning one result per request,
    /// aligned by index. The default implementation loops over [`Self::download`], so existing
    /// clients and test doubles behave exactly as before; clients that can fetch many files per
    /// invocation (the real CLI takes multiple `path...` arguments) override it. Callers must
    /// treat each element independently: some files may have landed at their destinations even
    /// when others — or the invocation as a whole — failed.
    fn download_many(&self, requests: &[DownloadRequest]) -> Vec<AppResult<()>> {
        requests
            .iter()
            .map(|request| self.download(&request.remote_path, &request.destination))
            .collect()
    }
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
    /// Installs a live-progress sink this client may notify from inside long operations
    /// (per-folder during the full-tree walk, per-transfer staging location). Display-only:
    /// implementations must never let a sink error or slow callback affect the operation.
    /// The default is a no-op, mirroring [`Self::install_cancel_flag`], so test doubles are
    /// unaffected unless they opt in.
    fn install_progress_sink(&mut self, _sink: Arc<dyn ProgressSink>) {}
}

/// One file in a batched download (see [`ProtonClient::download_many`]): fetch `remote_path`
/// and land its content at exactly `destination`. `expected_sha1` — the remote's claimed
/// digest, when the listing exposed one — lets a failed batch salvage the files that were
/// already fully staged, by verifying their content instead of trusting the CLI's exit status.
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub remote_path: PathBuf,
    pub destination: PathBuf,
    pub expected_sha1: Option<String>,
}

/// The CLI reported a missing node for an operation that covered this request — the TOCTOU of
/// issue #31: the node was in the listing this pass planned against, but is gone (trashed or
/// deleted by another client, or eventual-consistency lag) by the time its transfer ran. Typed so
/// the executor classifies it by downcast instead of re-matching stderr text: stderr is inspected
/// exactly once, here, where the CLI's output is in hand. Carries the message it replaces so
/// nothing is lost from the log.
///
/// **Not proof that `remote_path` itself is gone.** `download` covers one file and is exact, but
/// `download_many` runs one invocation over a whole batch and the CLI's note names no request, so
/// there every unstaged/unsalvaged file carries this type. Read it as "skip and replan", not as a
/// confirmed remote delete — nothing may be purged on it.
#[derive(Debug)]
pub struct NodeNotFound {
    pub remote_path: PathBuf,
    pub details: String,
}

impl std::fmt::Display for NodeNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl std::error::Error for NodeNotFound {}

/// True when `error` is a [`NodeNotFound`] — i.e. the CLI blamed a missing node rather than the
/// transfer itself, so this action is **skippable and replanned**, not failed. See
/// [`NodeNotFound`] for why that is weaker than "this request's node is gone" on the batched
/// path. Callers must not match message text: only an error constructed at a site that saw the
/// CLI's stderr classifies here.
pub fn is_node_not_found_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    error.downcast_ref::<NodeNotFound>().is_some()
}

/// Receiver for the concrete client's live progress callbacks (see
/// [`ProtonClient::install_progress_sink`]). The daemon publishes these into its status
/// snapshot so `proton-sync status` / the GUI can show what a long pass is doing. Callbacks
/// must be cheap and non-blocking — they run inline in the BFS walk and transfer paths.
pub trait ProgressSink: Send + Sync {
    /// One remote directory finished listing during the O(folders) full-tree walk.
    /// `folders_listed` counts directories completed so far, `directory` is root-relative
    /// (empty for the remote root itself).
    fn remote_folder_listed(&self, folders_listed: u64, directory: &Path);
    /// A download began staging into `scratch_dir`; the receiver may poll the directory's
    /// contents to observe bytes arriving while the CLI child runs.
    fn download_staging(&self, scratch_dir: &Path);
}

impl ProtonDriveClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            command_policy: CommandPolicy::default(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            progress_sink: None,
            wait_hook: None,
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
            progress_sink: None,
            wait_hook: None,
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

        // Sequential by design: these per-directory `list` calls must NOT be parallelized. The
        // `proton-drive` CLI is not concurrency-safe (shared SQLite → `SQLITE_BUSY`); see the
        // concurrency note on `run_proton_drive` and issues #23 / #17.
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
            if let Some(sink) = &self.progress_sink {
                sink.remote_folder_listed(visited.len() as u64, &relative_directory);
            }
        }

        Ok(RemoteListingStatus::Found(
            RemoteListing { files, directories }.into_entities(),
        ))
    }

    fn list_directory(
        &self,
        remote_root: &Path,
        relative_directory: &Path,
    ) -> AppResult<HashMap<PathBuf, RemoteEntity>> {
        let remote_directory = if relative_directory.as_os_str().is_empty() {
            remote_root.to_path_buf()
        } else {
            remote_root.join(relative_directory)
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
            return Err(boxed_error(format!(
                "proton-drive list failed for {}: {}",
                remote_directory.display(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = String::from_utf8(output.stdout)?;
        // Non-recursive: `parse_remote_listing` over a single directory's JSON yields that
        // directory's immediate entries, which is all the resolver needs to locate the changed
        // node among its siblings.
        Ok(parse_remote_listing(&stdout, remote_root, relative_directory)?.into_entities())
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
        let scratch_dir = download_scratch_dir(local_folder);
        fs::create_dir_all(&scratch_dir).map_err(|error| {
            boxed_error(format!(
                "failed to create scratch download directory {}: {error}",
                scratch_dir.display()
            ))
        })?;
        let _scratch_guard = ScratchDirGuard::new(&scratch_dir);
        if let Some(sink) = &self.progress_sink {
            sink.download_staging(&scratch_dir);
        }

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
            let message = format!(
                "proton-drive download failed for {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            // Classified here, where stderr is in hand: the node is gone, so the executor skips
            // this one action instead of failing the whole pass (#31).
            if is_node_not_found(&output) {
                return Err(Box::new(NodeNotFound {
                    remote_path: remote_path.to_path_buf(),
                    details: message,
                }));
            }
            return Err(boxed_error(message));
        }

        // The CLI exited 0, so if the scratch directory does not now hold exactly the downloaded
        // file, the CLI put it somewhere else (or silently skipped it) — e.g. it expands a
        // leading `~` in the destination argument while the daemon's own fs calls treat `~` as a
        // literal directory name. Surface the CLI's own output so the mismatch is diagnosable
        // from the error alone.
        let downloaded_path = single_entry_in_directory(&scratch_dir).map_err(|error| {
            boxed_error(format!(
                "download of {} reported success, but the scratch directory the daemon was \
                 watching ({}) did not end up holding exactly the one downloaded file: {error}; \
                 the CLI may have resolved the destination to a different path — proton-drive \
                 stdout: {}; stderr: {}",
                remote_path.display(),
                scratch_dir.display(),
                summarize_command_output(&output.stdout),
                summarize_command_output(&output.stderr),
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

    fn download_many(&self, requests: &[DownloadRequest]) -> Vec<AppResult<()>> {
        match batchable_download_folder(requests) {
            Some(local_folder) => self.run_download_batch(requests, &local_folder),
            None => requests
                .iter()
                .map(|request| self.download(&request.remote_path, &request.destination))
                .collect(),
        }
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
        let Err(rename_error) = self.rename_remote_entry(&moved_remote_path, new_name) else {
            return Ok(());
        };
        // The move succeeded but the rename did not, stranding the entity at
        // `new_parent/old_name`. Left there, the next reconcile can no longer recognize
        // this as a resumable rename (the source is gone from its old path), so it
        // re-uploads the new local name and re-downloads the moved entity, permanently
        // duplicating the file on both sides. Best-effort move it back to its original
        // parent so the operation is atomic-or-nothing and the next pass replans the
        // whole rename+move cleanly; surface both errors if even the rollback fails.
        let old_parent_remote = match old_parent {
            Some(parent) if !parent.as_os_str().is_empty() => remote_root.join(parent),
            _ => remote_root.to_path_buf(),
        };
        Err(
            match self.move_remote_entry(&moved_remote_path, &old_parent_remote) {
                Ok(()) => boxed_error(format!(
                    "proton-drive rename after move failed for {}: {rename_error}; rolled the \
                 move back to {} so the next reconcile can retry the rename cleanly",
                    moved_remote_path.display(),
                    old_remote_path.display()
                )),
                Err(rollback_error) => boxed_error(format!(
                    "proton-drive rename after move failed for {}: {rename_error}; additionally \
                 failed to roll the move back to {}: {rollback_error} — the entity is left \
                 at {} and may need manual cleanup",
                    moved_remote_path.display(),
                    old_parent_remote.display(),
                    moved_remote_path.display()
                )),
            },
        )
    }

    fn install_cancel_flag(&mut self, cancel_flag: Arc<AtomicBool>) {
        self.cancel_flag = cancel_flag;
    }

    fn install_progress_sink(&mut self, sink: Arc<dyn ProgressSink>) {
        self.progress_sink = Some(sink);
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

    /// Runs a single `proton-drive` subcommand (with `attempts` bounded retries).
    ///
    /// ## The CLI is not safe to run concurrently — issue #23
    ///
    /// The vendored `proton-drive` CLI keeps shared, writable SQLite cache stores
    /// (`~/.cache/proton-drive-cli`). Two overlapping invocations race on those stores and one
    /// aborts with `SQLITE_BUSY: database is locked` — a non-zero exit plus a `====` banner and JS
    /// stack trace printed to its output, which would then also fail our JSON parse.
    ///
    /// The engine does not trigger this itself. A **user-global single-instance lock**
    /// (`paths::default_global_lock_path`) admits at most one `proton-syncd` per user account, and
    /// within it reconcile is **single-flight** (serialized on `&mut self` via `block_in_place` in
    /// the daemon `select!` loop) while event polling shells `curl` (`session.rs`), not this CLI —
    /// so the engine drives at most one `proton-drive` process at a time. The only remaining way to
    /// reach `SQLITE_BUSY` is an *external* `proton-drive` process sharing this user's cache; on
    /// Linux — the platform this project targets, precisely because there is no Proton desktop
    /// client — that reduces to the user running `proton-drive` by hand while the daemon is live.
    /// Narrow; hence this note rather than in-engine retries. (The lock is keyed on `$XDG_STATE_HOME`,
    /// so the one way to defeat it is to deliberately point two daemons at different state homes —
    /// an explicit opt-in to the contention.)
    ///
    /// Defenses already in place, by command class:
    /// - **Read-only** listings (`list` / `list_directory`) pass `list_attempts` (> 1), so a
    ///   transient busy crash is retried and usually self-heals.
    /// - **Mutating** commands (upload/download/trash/rename/move/create-folder) pass `attempts = 1`
    ///   and are never auto-retried, so a busy crash mid-write fails the reconcile cleanly — the
    ///   failed action is never recorded (index writes happen only after their side effect
    ///   succeeded; completed actions keep their checkpoints) — and the remainder replans next
    ///   cycle.
    ///
    /// The real fix — one long-lived process owning the cache instead of many short-lived CLI
    /// spawns fighting over shared SQLite — is the SDK-sidecar tracked in #18. Until then, do **not**
    /// add engine-side parallelism across `proton-drive` invocations (this is also why #17,
    /// parallelizing the list BFS, stays deferred).
    /// Executes the batched form of [`ProtonClient::download_many`]: one `filesystem download`
    /// invocation naming every remote path, staged through a single private scratch directory
    /// and moved into place with one rename per file — the same "content lands at exactly
    /// `destination`, and nothing else is touched" contract as the single-file `download`.
    ///
    /// Results are per-file. After a successful CLI exit every request must have been staged
    /// under its remote basename; a missing entry (e.g. a file the CLI silently skipped) fails
    /// only that request. After a failed CLI exit (or timeout/cancellation) the batch still
    /// salvages every fully-staged file whose content matches its `expected_sha1`, so transfers
    /// that already completed survive the failure; unverifiable files fail with the batch error.
    fn run_download_batch(
        &self,
        requests: &[DownloadRequest],
        local_folder: &Path,
    ) -> Vec<AppResult<()>> {
        let scratch_dir = download_scratch_dir(local_folder);
        if let Err(error) = fs::create_dir_all(&scratch_dir) {
            let message = format!(
                "failed to create scratch download directory {}: {error}",
                scratch_dir.display()
            );
            return requests
                .iter()
                .map(|_| Err(boxed_error(message.clone())))
                .collect();
        }
        let _scratch_guard = ScratchDirGuard::new(&scratch_dir);
        if let Some(sink) = &self.progress_sink {
            sink.download_staging(&scratch_dir);
        }

        let mut args = Vec::with_capacity(requests.len() + 3);
        args.push(OsString::from("filesystem"));
        args.push(OsString::from("download"));
        args.extend(
            requests
                .iter()
                .map(|request| request.remote_path.as_os_str().to_os_string()),
        );
        args.push(scratch_dir.as_os_str().to_os_string());
        let timeout = batch_download_timeout(self.command_policy.timeout, requests.len());

        match self.run_proton_drive_with_timeout("download", &args, timeout) {
            Ok(output) if output.status.success() => requests
                .iter()
                .map(|request| promote_staged_download(&scratch_dir, request, &output))
                .collect(),
            Ok(output) => {
                let failure = format!(
                    "proton-drive download failed for a batch of {} files: {}",
                    requests.len(),
                    trimmed_stderr(&output)
                );
                salvage_staged_downloads(
                    &scratch_dir,
                    requests,
                    &failure,
                    is_node_not_found(&output),
                )
            }
            Err(error) => {
                let failure = format!(
                    "proton-drive download failed for a batch of {} files: {error}",
                    requests.len()
                );
                // A cancellation (shutdown/SIGTERM) must stay prompt: salvage hashes every
                // staged file, which could hold the exit for the full read of a large chunk.
                // Skip it — the scratch guard discards the staged files and the completed
                // transfers simply re-download next pass. The flag is latched once shutdown
                // begins, so this cannot misfire for an ordinary timeout or CLI failure.
                if self.cancel_flag.load(Ordering::SeqCst) {
                    return requests
                        .iter()
                        .map(|_| Err(boxed_error(failure.clone())))
                        .collect();
                }
                // No `Output` here (spawn error, timeout, cancellation), so there is no stderr to
                // classify: a transport failure is never a vanished node.
                salvage_staged_downloads(&scratch_dir, requests, &failure, false)
            }
        }
    }

    fn run_proton_drive(
        &self,
        operation: &str,
        args: &[OsString],
        attempts: usize,
    ) -> AppResult<Output> {
        self.run_proton_drive_with_logging(
            operation,
            args,
            attempts,
            true,
            self.command_policy.timeout,
        )
    }

    fn run_proton_drive_quiet(
        &self,
        operation: &str,
        args: &[OsString],
        attempts: usize,
    ) -> AppResult<Output> {
        self.run_proton_drive_with_logging(
            operation,
            args,
            attempts,
            false,
            self.command_policy.timeout,
        )
    }

    /// As [`Self::run_proton_drive`] with `attempts = 1`, but with an explicit per-invocation
    /// timeout. Used by batched downloads, where one invocation legitimately transfers many
    /// files and therefore deserves many times the single-command budget.
    fn run_proton_drive_with_timeout(
        &self,
        operation: &str,
        args: &[OsString],
        timeout: Duration,
    ) -> AppResult<Output> {
        self.run_proton_drive_with_logging(operation, args, 1, true, timeout)
    }

    fn run_proton_drive_with_logging(
        &self,
        operation: &str,
        args: &[OsString],
        attempts: usize,
        warn_on_unsuccessful_exit: bool,
        timeout: Duration,
    ) -> AppResult<Output> {
        let attempts = attempts.max(1);
        let mut last_error = None;
        for attempt in 1..=attempts {
            let output = match self.run_once(operation, args, timeout) {
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

    fn run_once(&self, operation: &str, args: &[OsString], timeout: Duration) -> AppResult<Output> {
        let mut child = self.spawn_once(args)?;
        // Drain stdout and stderr on separate threads *while the child runs*. A `proton-drive`
        // command whose output exceeds the OS pipe buffer (~64 KiB) fills that buffer and then
        // blocks on `write`, making no further progress, because nothing reads the pipe until the
        // child has exited — so the output is silently truncated to whatever was buffered and the
        // JSON listing fails to parse (issue #40). Reading concurrently keeps the pipe drained so
        // the full output is captured. `wait_with_output` can't be used here: it gives no way to
        // poll the cancellation flag / enforce the timeout while it reads.
        let stdout_reader = spawn_pipe_reader(child.stdout.take());
        let stderr_reader = spawn_pipe_reader(child.stderr.take());
        let deadline = Instant::now() + timeout;
        // Child lifecycle — `run_once` has exactly four exits, and each one is bounded:
        //   * timeout, cancellation, and wait failure leave a command that may still be running,
        //     so all three SIGKILL the whole process group (`terminate_child_tree`) and drop the
        //     reader handles *without* collecting them: a rogue grandchild holding the pipe must
        //     never delay a prompt return.
        //   * a child that exited is never killed — `wait_timeout` has already reaped the group
        //     leader, so `kill(-pid)` could now hit an unrelated, pid-reused group — its output is
        //     drained under a deadline instead (see the drain below).
        let status = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                terminate_child_tree(&mut child);
                warn!(
                    operation,
                    timeout_ms = timeout.as_millis(),
                    "proton-drive command timed out"
                );
                return Err(boxed_error(format!(
                    "proton-drive {operation} timed out after {}",
                    format_duration(timeout)
                )));
            }
            let poll_interval = remaining.min(CANCELLATION_POLL_INTERVAL);
            match self.wait_for_child(&mut child, poll_interval) {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                // Issue #57: a failed `waitpid` says nothing about the child, which keeps running.
                // Propagating the error bare would leave a *mutating* command (upload/trash) running
                // detached while the engine records the action as failed and replans it — the double
                // side effect the never-auto-retry convention exists to prevent. Kill first, then
                // report: the child has not been reaped here, so the process group is still ours.
                Err(error) => {
                    terminate_child_tree(&mut child);
                    warn!(
                        operation,
                        error = %error,
                        "failed to wait for the proton-drive command; terminated it"
                    );
                    return Err(boxed_error(format!(
                        "proton-drive {operation} could not be waited for ({error}); it was \
                         terminated and may or may not have completed remotely"
                    )));
                }
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
        };
        // Issue #56: a child's exit does NOT guarantee the readers are at EOF. A grandchild that
        // inherited the pipe write ends keeps them open, and collecting with no deadline then blocks
        // forever — past the cancellation poll (the loop is over), past `CommandPolicy::timeout`, and
        // with the reconcile running under `block_in_place` the whole daemon wedges until SIGKILL.
        // Bound the drain by the command's own deadline, with a floor for the case where it expired
        // at the moment of exit, and report expiry like a timeout. Output that arrives within the
        // command's budget still succeeds, so a CLI that legitimately hands its pipe to a grandchild
        // is unaffected; only the unbounded wait is gone.
        let drain_deadline = deadline.max(Instant::now() + PIPE_DRAIN_GRACE);
        let stdout =
            collect_pipe_output(stdout_reader, drain_deadline, timeout, operation, "stdout")?;
        let stderr =
            collect_pipe_output(stderr_reader, drain_deadline, timeout, operation, "stderr")?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// One timed wait on the child, routed through the optional [`WaitHook`] test seam.
    fn wait_for_child(
        &self,
        child: &mut Child,
        poll_interval: Duration,
    ) -> io::Result<Option<ExitStatus>> {
        match &self.wait_hook {
            Some(hook) => hook(child, poll_interval),
            None => child.wait_timeout(poll_interval),
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
                // A bare `proton-drive` (default `proton_cli`) is resolved via PATH. As a systemd
                // *user* service started at boot, the daemon inherits a minimal PATH that often
                // lacks the CLI's directory (e.g. ~/.local/bin) until the desktop session imports
                // the shell environment, so early passes fail to spawn it with a raw, opaque "No
                // such file or directory (os error 2)". Translate that into an actionable message —
                // but tailor the hint: a bare name is a PATH lookup (surface PATH + the
                // absolute-path escape hatch), whereas a configured absolute/relative path is not,
                // so pointing at PATH there would only mislead (e.g. a typo'd `proton_cli`).
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let is_bare_name = self
                        .executable
                        .parent()
                        .is_none_or(|parent| parent.as_os_str().is_empty());
                    let hint = if is_bare_name {
                        format!(
                            "It is resolved via PATH. Install proton-drive and make sure it is on \
                             PATH, or set `proton_cli` (config file) / `--proton-cli` to its \
                             absolute path — as a systemd user service the CLI's directory may not \
                             be on PATH early in the boot session. PATH={}",
                            std::env::var("PATH").unwrap_or_default()
                        )
                    } else {
                        "Check that this configured `proton_cli` / `--proton-cli` path exists and \
                         is executable."
                            .to_owned()
                    };
                    return Err(boxed_error(format!(
                        "could not run the proton-drive CLI `{}`: {error}. {hint}",
                        self.executable.display()
                    )));
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

/// A pipe-reader thread's one-shot result channel. Deliberately not a `JoinHandle`: joining a
/// thread cannot be given a deadline, and this read is only guaranteed to finish when *every*
/// holder of the pipe's write end is gone — not merely the direct child (issue #56).
type PipeReader = Receiver<io::Result<Vec<u8>>>;

/// Spawns a thread that reads a child pipe to EOF and reports its full contents on a channel.
/// Draining the pipe concurrently with the running child is what keeps a large (> OS pipe buffer)
/// `proton-drive` output from blocking and being truncated (issue #40). Returns `None` when the
/// handle is absent (`Child::stdout`/`stderr` are `Option`; always present for a piped child).
///
/// The thread is detached: when the receiver is dropped (every non-success exit of `run_once`, plus
/// a drain that expired) the send fails harmlessly and the thread ends once its read does.
fn spawn_pipe_reader<R: Read + Send + 'static>(reader: Option<R>) -> Option<PipeReader> {
    reader.map(|mut reader| {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let result = reader.read_to_end(&mut buffer).map(|_| buffer);
            let _ = sender.send(result);
        });
        receiver
    })
}

/// Collects a [`spawn_pipe_reader`] thread's output, waiting no later than `deadline`, and surfaces
/// a read error, a stalled pipe, or a dead reader thread as an `AppResult`. Only called once the
/// child has exited; the timeout/cancel/wait-failure paths drop the reader without collecting.
///
/// Expiry is not a lost cause the caller can paper over: the output would be truncated mid-JSON, so
/// it is an error, worded like the timeout it effectively is. Nothing is killed on expiry — the
/// group leader has already been reaped and its pid may have been reused. The message reports the
/// drain wait actually served, not `timeout`: `deadline` is floored by `PIPE_DRAIN_GRACE`, so the
/// two differ whenever the command expired at the instant of exit. `timeout` is carried alongside
/// it so an operator can still correlate the failure with the command budget.
///
/// A `None` reader is an invariant violation (`spawn_once` always sets stdout/stderr to
/// `Stdio::piped()`, so `child.stdout`/`stderr` are always present) and is reported as an error
/// rather than silently yielding empty output — the latter would only resurface as a confusing
/// downstream JSON parse failure.
fn collect_pipe_output(
    reader: Option<PipeReader>,
    deadline: Instant,
    timeout: Duration,
    operation: &str,
    stream: &str,
) -> AppResult<Vec<u8>> {
    let reader = reader.ok_or_else(|| {
        boxed_error(
            "proton-drive child output pipe was not captured (expected a piped stdout/stderr)",
        )
    })?;
    let started = Instant::now();
    match reader.recv_timeout(deadline.saturating_duration_since(started)) {
        Ok(result) => Ok(result?),
        Err(RecvTimeoutError::Timeout) => {
            let waited = started.elapsed();
            warn!(
                operation,
                stream,
                waited_ms = waited.as_millis(),
                timeout_ms = timeout.as_millis(),
                "proton-drive command exited but its output pipe stayed open"
            );
            Err(boxed_error(format!(
                "proton-drive {operation} exited but its {stream} was still held open after {} \
                 (a forked grandchild is keeping the pipe alive; the command's own budget was {}), \
                 so its output could not be collected",
                format_duration(waited),
                format_duration(timeout)
            )))
        }
        // The thread dropped its sender without sending: it panicked mid-read.
        Err(RecvTimeoutError::Disconnected) => {
            Err(boxed_error("proton-drive output reader thread panicked"))
        }
    }
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

/// Builds the private per-invocation scratch directory path under `local_folder` used to
/// stage downloads before their final rename into place (see the naming-contract note in
/// [`ProtonClient::download`]'s concrete impl for why staging is mandatory).
fn download_scratch_dir(local_folder: &Path) -> PathBuf {
    local_folder.join(format!(
        "{}{}-{}",
        crate::DOWNLOAD_SCRATCH_PREFIX,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    ))
}

/// The preconditions for [`ProtonClient::download_many`]'s single-invocation form: at least
/// two files, one shared destination directory (the CLI takes exactly one `localFolder`), and
/// each destination named exactly after its remote basename with no duplicates — the CLI names
/// every staged file after its remote source, so that name is the only handle for mapping
/// staged entries back to requests. Returns the shared destination directory when the
/// preconditions hold; the caller falls back to per-file downloads otherwise.
fn batchable_download_folder(requests: &[DownloadRequest]) -> Option<PathBuf> {
    if requests.len() < 2 {
        return None;
    }
    let local_folder = requests.first()?.destination.parent()?;
    let mut folded_names = BTreeSet::new();
    for request in requests {
        if request.destination.parent() != Some(local_folder) {
            return None;
        }
        let destination_name = request.destination.file_name()?;
        if request.remote_path.file_name() != Some(destination_name) {
            return None;
        }
        // The uniqueness check is case-folded, not byte-exact: Proton Drive is
        // case-sensitive, but the local filesystem may not be, and two staged siblings
        // differing only by case would collapse onto one file there. Such a batch falls
        // back to per-file downloads, which keep the per-file staging contract.
        if !folded_names.insert(destination_name.to_string_lossy().to_lowercase()) {
            return None;
        }
    }
    Some(local_folder.to_path_buf())
}

/// Every file in a batch gets the configured per-command budget: one invocation transferring
/// N files may legitimately take N times as long as one. Deliberately unclamped — a cap lower
/// than what a chunk genuinely needs would make that chunk permanently un-downloadable; a hung
/// CLI child is instead bounded by cancellation (shutdown kills it within the poll interval).
fn batch_download_timeout(single_command_timeout: Duration, files: usize) -> Duration {
    single_command_timeout.saturating_mul(u32::try_from(files).unwrap_or(u32::MAX))
}

/// Locates a request's staged file inside the batch scratch directory: the CLI names every
/// download after its remote source, and the batch preconditions pinned each destination
/// basename to exactly that name.
fn staged_download_path(scratch_dir: &Path, request: &DownloadRequest) -> AppResult<PathBuf> {
    let name = request.destination.file_name().ok_or_else(|| {
        boxed_error(format!(
            "download destination has no file name: {}",
            request.destination.display()
        ))
    })?;
    Ok(scratch_dir.join(name))
}

/// Moves one request's staged file into place after a successful batch invocation. The CLI
/// exited 0, so a missing staged entry means it silently skipped that file (e.g. a
/// Proton-native document) or named it unexpectedly — that request fails with the CLI's own
/// output attached, without affecting its siblings.
fn promote_staged_download(
    scratch_dir: &Path,
    request: &DownloadRequest,
    output: &Output,
) -> AppResult<()> {
    let staged = staged_download_path(scratch_dir, request)?;
    if !is_regular_file(&staged) {
        let message = format!(
            "batch download reported success, but {} was not staged in {} — the CLI may have \
             skipped it or named it unexpectedly; proton-drive stdout: {}; stderr: {}",
            request.remote_path.display(),
            scratch_dir.display(),
            summarize_command_output(&output.stdout),
            summarize_command_output(&output.stderr),
        );
        // Exit 0, a node-not-found note somewhere in stderr, and this file missing from staging.
        // The check reads the whole invocation's stderr and the note names no request, so this is
        // the BATCH's signal, not this file's: a sibling the CLI skipped for another reason (a
        // Proton-native document) is classified alongside the vanished node. Skipping is still
        // the safer read — nothing is recorded, the cursor is held, the action replans — and the
        // misread lasts only while a vanished node keeps the note in stderr. Without the note the
        // miss is unexplained and stays fatal (#31).
        if is_node_not_found(output) {
            return Err(Box::new(NodeNotFound {
                remote_path: request.remote_path.clone(),
                details: format!("{message}; the note names no file, so it cannot be attributed"),
            }));
        }
        return Err(boxed_error(message));
    }
    move_staged_download(&staged, request)
}

/// Salvages a failed batch: any request whose staged file is provably complete — its content
/// hashes to the remote's claimed SHA-1 — is still moved into place and reported `Ok`, so an
/// error late in a large batch does not discard the transfers that already finished. A partial
/// file can never match its digest, and without a claimed digest there is nothing safe to
/// verify against, so every other request fails with the batch error.
///
/// `node_not_found` says the batch's stderr reported a missing node. One CLI invocation covers
/// many files and names no single one, so the signal cannot be attributed: every unsalvaged
/// request is typed [`NodeNotFound`], which makes the executor skip them (recording nothing,
/// holding the event cursor) instead of failing the pass. Their downloads simply replan next
/// pass, when the vanished node is no longer listed.
fn salvage_staged_downloads(
    scratch_dir: &Path,
    requests: &[DownloadRequest],
    failure: &str,
    node_not_found: bool,
) -> Vec<AppResult<()>> {
    let unsalvaged = |request: &DownloadRequest| -> Box<dyn std::error::Error + Send + Sync> {
        if node_not_found {
            Box::new(NodeNotFound {
                remote_path: request.remote_path.clone(),
                details: format!(
                    "{failure}; one invocation covers the whole batch, so the missing node cannot \
                     be attributed to a single file"
                ),
            })
        } else {
            boxed_error(failure.to_owned())
        }
    };
    requests
        .iter()
        .map(|request| {
            let Some(expected_sha1) = request.expected_sha1.as_deref() else {
                return Err(unsalvaged(request));
            };
            let staged = staged_download_path(scratch_dir, request)?;
            let verified = is_regular_file(&staged)
                && compute_sha1(&staged).is_ok_and(|hash| hash == expected_sha1);
            if !verified {
                return Err(unsalvaged(request));
            }
            move_staged_download(&staged, request)
        })
        .collect()
}

fn move_staged_download(staged: &Path, request: &DownloadRequest) -> AppResult<()> {
    fs::rename(staged, &request.destination).map_err(|error| {
        boxed_error(format!(
            "downloaded {} to a scratch location but failed to move it to {}: {error}",
            request.remote_path.display(),
            request.destination.display()
        ))
    })
}

/// `true` only for an actual regular file at `path` — not a symlink to one (`symlink_metadata`
/// does not follow), not a directory, not absent. Batch staging only ever deals in regular
/// files the CLI wrote, so anything else is treated as "not staged".
fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
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

/// Renders captured CLI output for inclusion in an error message: lossy UTF-8, trimmed, capped
/// to its final characters (the end is where a CLI's summary and error lines land), and
/// `"(empty)"` when there was none — so the surrounding message never reads as truncated itself.
fn summarize_command_output(bytes: &[u8]) -> String {
    const MAX_CHARS: usize = 600;
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(empty)".to_owned();
    }
    let total_chars = trimmed.chars().count();
    if total_chars <= MAX_CHARS {
        return trimmed.to_owned();
    }
    let tail: String = trimmed.chars().skip(total_chars - MAX_CHARS).collect();
    format!("(first {} chars omitted) …{tail}", total_chars - MAX_CHARS)
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
        // Normalize hex digests to lowercase at the parse boundary: every comparison
        // downstream — the planner's SHA-1 equality checks and the batch-download salvage
        // verification — matches against `compute_sha1`'s lowercase output, so an
        // uppercase digest from the CLI would otherwise read as "content differs" forever.
        Some(Value::String(value)) => Ok(Some(value.to_ascii_lowercase())),
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
    // `true`: the top-level nodes of this listing are the CLI's root wrapper level.
    collect_from_value(&value, parent_path, remote_root, &mut listing, true)?;
    Ok(listing)
}

fn collect_from_value(
    value: &Value,
    parent_path: &Path,
    remote_root: &Path,
    listing: &mut RemoteListing,
    is_root: bool,
) -> AppResult<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                // A top-level array does not descend a level: its items are still root nodes.
                collect_from_value(item, parent_path, remote_root, listing, is_root)?;
            }
        }
        Value::Object(object) => {
            // Deserialize once and let collect_node own all parent-path propagation.
            let node: ProtonNode = serde_json::from_value(Value::Object(object.clone()))?;
            collect_node(&node, parent_path, remote_root, listing, is_root)?;
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
    is_root: bool,
) -> AppResult<()> {
    // #59: a wrapped value that is PRESENT but undecodable means the node exists remotely and
    // this listing cannot describe it. Dropping it silently is indistinguishable from a deletion
    // to the planner (an omitted node reads as `Missing` -> LocalDelete), so a node whose
    // identity or locator is unusable *because* it failed to decode fails the whole listing:
    // `list`/`list_entities_or_missing_root` abort the pass, and `list_directory` makes
    // `TargetedResolver` error, which the caller turns into `Reconstruction::FallbackToSnapshot`.
    // Either way the planner never sees a map that is missing a node still present remotely.
    let id = node.id.as_deref().or(node.uid.as_deref());
    if id.is_none() && (node.id.is_undecodable() || node.uid.is_undecodable()) {
        // Never placeholder an unidentifiable node: `find_entity_by_uid` would not match it, the
        // targeted resolver would report it absent from its parent, and the reconstruction would
        // drop its location — #59 again, through the incremental path.
        return Err(incomplete_listing_error("id", parent_path));
    }

    let candidate_path = node
        .path
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| node.name.as_deref().map(|name| parent_path.join(name)));

    if candidate_path.is_none() && (node.path.is_undecodable() || node.name.is_undecodable()) {
        // Unplaceable: neither locator decoded, so no placeholder can be positioned either.
        // (A node with *both* locators merely absent is a structural container — see below —
        // and stays a silent, byte-identical pass-through.)
        return Err(incomplete_listing_error("name/path", parent_path));
    }

    // The root-basename strip must collapse only the genuine depth-0 wrapper node,
    // never a descendant that merely shares the root's basename (issue #54). A
    // non-empty `parent_path` means this call is resolving beneath some directory
    // (a re-parented child, or a targeted subdirectory listing), so it is never the
    // root wrapper even at the listing's top level.
    let is_root_wrapper = is_root && parent_path.as_os_str().is_empty();

    // If a path was provided but fails normalization (absolute path, `..` escape,
    // or root component), skip the node and all of its descendants.
    let relative_path = match candidate_path.as_deref() {
        Some(p) => match normalize_remote_path(p, remote_root, is_root_wrapper) {
            Some(normalized) => normalized,
            None => return Ok(()),
        },
        None => PathBuf::new(),
    };

    let is_folder = node.is_folder.unwrap_or(false)
        || matches!(node.kind.as_deref(), Some("folder" | "directory"))
        || (!node.children.is_empty() || !node.entries.is_empty() || !node.files.is_empty());

    if is_folder && !relative_path.as_os_str().is_empty() {
        let name = node
            .name
            .as_deref()
            .map(ToOwned::to_owned)
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

    if !is_folder {
        if relative_path.as_os_str().is_empty() {
            // #72: an empty relative path IS the remote root, never a file within it. Keyed
            // under "" it passes every downstream filter and resolves to the roots themselves,
            // so the planner would download the remote root onto the local root and fail every
            // pass. The directory branch above has always rejected it. Expected for the depth-0
            // wrapper of a folder listed without a type marker; anything else is malformed.
            if !is_root_wrapper {
                warn!(
                    remote_id = ?id,
                    "skipping remote file entry that resolves to the remote root itself"
                );
            }
        } else if let Some(id) = id {
            // #59: an undecodable (not absent) locator that another locator still placed. Keep
            // the node in the map — never read as a deletion — but inert: no digest and not
            // downloadable, which routes it into the planner's existing `Unsupported` arm
            // instead of a blind download of a node the CLI could not decode.
            let degraded = node.name.is_undecodable() || node.path.is_undecodable();
            let name = node.name.as_deref().map(ToOwned::to_owned).or_else(|| {
                degraded
                    .then(|| {
                        relative_path
                            .file_name()
                            .and_then(|value| value.to_str())
                            .map(ToOwned::to_owned)
                    })
                    .flatten()
            });
            if let Some(name) = name {
                listing.files.insert(
                    relative_path.clone(),
                    RemoteFile {
                        path: relative_path.clone(),
                        id: id.to_owned(),
                        name,
                        sha1_hash: (!degraded)
                            .then(|| {
                                node.active_revision
                                    .as_ref()
                                    .and_then(|revision| revision.claimed_digests.as_ref())
                                    .and_then(|digests| digests.sha1.clone())
                            })
                            .flatten(),
                        downloadable: !degraded
                            && is_downloadable_media_type(node.media_type.as_deref()),
                    },
                );
            }
        }
    }

    let next_parent = if relative_path.as_os_str().is_empty() {
        parent_path.to_path_buf()
    } else {
        relative_path
    };

    // A nameless structural container (the CLI wraps its listing in an outer
    // `{"entries": [...]}` object with no name/path) does not itself occupy the
    // wrapper level — it passes `is_root` straight through to the real nodes it
    // holds. Any node that carries its own identity, on the other hand, consumes
    // the root level: its descendants are never the wrapper, so a child that
    // shares the root's basename keeps its qualified relative path (issue #54).
    let child_is_root = is_root && candidate_path.is_none();
    for child in &node.entries {
        collect_node(child, &next_parent, remote_root, listing, child_is_root)?;
    }
    for child in &node.children {
        collect_node(child, &next_parent, remote_root, listing, child_is_root)?;
    }
    for child in &node.files {
        collect_node(child, &next_parent, remote_root, listing, child_is_root)?;
    }

    Ok(())
}

/// The listing describes a node it cannot identify or place (#59). Erroring is the conservative
/// outcome: an incomplete listing must never reach the planner, because a node missing from the
/// remote map is indistinguishable from one that was deleted remotely.
fn incomplete_listing_error(
    field: &str,
    parent_path: &Path,
) -> Box<dyn std::error::Error + Send + Sync> {
    let parent = if parent_path.as_os_str().is_empty() {
        "the remote root".to_owned()
    } else {
        parent_path.display().to_string()
    };
    boxed_error(format!(
        "remote listing is incomplete: a node under {parent} has an undecodable {field}; \
         refusing to plan against a listing in which it would read as a deletion"
    ))
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
///
/// `is_root_wrapper` gates the bare root-basename strip: the CLI's tree output
/// wraps a listing in a single node named after the listed folder, which must
/// collapse to `""`. That strip is applied ONLY to that depth-0 wrapper — a
/// genuine descendant that merely shares the root's basename (e.g. a subfolder
/// `Documents` under root `/Drive/Documents`) must survive as a normal relative
/// path rather than aliasing to the root and vanishing (issue #54).
fn normalize_remote_path(
    path: &Path,
    remote_root: &Path,
    is_root_wrapper: bool,
) -> Option<PathBuf> {
    let relative = if let Ok(stripped) = path.strip_prefix(remote_root) {
        stripped.to_path_buf()
    } else if is_root_wrapper
        && let Some(root_name) = remote_root.file_name()
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
    fn an_explicitly_failed_wrapper_is_undecodable_even_with_a_string_value() {
        let value: WrappedString = serde_json::from_str(r#"{"ok": false, "value": "node-id"}"#)
            .expect("the wrapper shape should deserialize");

        assert_eq!(value, WrappedString::Undecodable);
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
    fn root_named_subfolder_survives_and_is_not_reparented_to_root() {
        // Regression for issue #54: the CLI wraps a listing in a depth-0 node named
        // after the listed folder, which must collapse to "". The old strip fired at
        // every depth, so a genuine subfolder that happens to share the root's basename
        // ("Documents" under root "/Drive/Documents") also collapsed to "" — its whole
        // subtree vanished and its child re-parented to the root, colliding with real
        // root entries. The subtree must instead survive under its qualified path.
        let json = r#"
        {
          "entries": [
            {
              "name": "Documents",
              "type": "folder",
              "entries": [
                {
                  "name": "Documents",
                  "type": "folder",
                  "entries": [
                    {
                      "id": "file-inner",
                      "name": "notes.txt",
                      "activeRevision": {
                        "claimedDigests": {
                          "sha1": "3333333333333333333333333333333333333333"
                        }
                      }
                    }
                  ]
                }
              ]
            }
          ]
        }
        "#;

        let remote_root = Path::new("/Drive/Documents");

        // The nested subfolder must be recorded as a directory (so the BFS walk queues
        // and recurses into it) at its qualified path, not collapsed to the root.
        let entities = parse_remote_entities(json, remote_root).expect("parse remote entities");
        let subfolder = entities
            .get(Path::new("Documents"))
            .and_then(RemoteEntity::as_directory)
            .expect("root-named subfolder must survive as a directory entity");
        assert_eq!(subfolder.name, "Documents");

        // The child file must live under the subfolder, not aliased to the root.
        let files = parse_remote_files(json, remote_root).expect("parse remote files");
        let nested = files
            .get(Path::new("Documents/notes.txt"))
            .expect("child of the root-named subfolder must keep its qualified path");
        assert_eq!(nested.id, "file-inner");
        assert_eq!(
            nested.sha1_hash.as_deref(),
            Some("3333333333333333333333333333333333333333")
        );

        // It must NOT be re-parented to the root (the key-collision variant).
        assert!(
            !files.contains_key(Path::new("notes.txt")),
            "child must not re-parent to the root and collide with real root entries"
        );
        // And the subtree must not have vanished into the empty root path.
        assert_eq!(files.len(), 1, "exactly the one nested file must be mapped");
    }

    #[test]
    fn normalize_remote_path_rejects_traversal_and_absolute_paths() {
        // Parent-directory component must be rejected.
        assert_eq!(
            normalize_remote_path(Path::new("../secret"), Path::new("/Drive"), true),
            None,
            "path with .. must be rejected"
        );

        // Absolute path that does not start with remote_root must be rejected.
        assert_eq!(
            normalize_remote_path(Path::new("/etc/passwd"), Path::new("/Drive"), true),
            None,
            "absolute path outside remote_root must be rejected"
        );

        // Nested .. disguised inside a longer path.
        assert_eq!(
            normalize_remote_path(
                Path::new("Documents/../../etc/passwd"),
                Path::new("/Drive"),
                true
            ),
            None,
            "embedded .. must be rejected"
        );

        // A valid relative path must succeed.
        assert_eq!(
            normalize_remote_path(Path::new("Documents/notes.txt"), Path::new("/Drive"), true),
            Some(PathBuf::from("Documents/notes.txt")),
            "valid relative path must pass through"
        );

        // A valid absolute path rooted at remote_root must succeed.
        assert_eq!(
            normalize_remote_path(Path::new("/Drive/notes.txt"), Path::new("/Drive"), true),
            Some(PathBuf::from("notes.txt")),
            "absolute path under remote_root must be stripped correctly"
        );

        // CurDir (.) components must be stripped so the result is a canonical Normal-only path.
        assert_eq!(
            normalize_remote_path(
                Path::new("./Documents/notes.txt"),
                Path::new("/Drive"),
                true
            ),
            Some(PathBuf::from("Documents/notes.txt")),
            "leading CurDir must be stripped"
        );

        // A node NOT at the root-wrapper level that merely shares the root basename must
        // keep its name verbatim instead of being stripped to "" (issue #54).
        assert_eq!(
            normalize_remote_path(Path::new("Drive"), Path::new("/Drive"), false),
            Some(PathBuf::from("Drive")),
            "root-named descendant must survive when not the root wrapper"
        );
        // The same name AT the root-wrapper level still collapses (the CLI's wrapper node).
        assert_eq!(
            normalize_remote_path(Path::new("Drive"), Path::new("/Drive"), true),
            Some(PathBuf::new()),
            "root wrapper node still collapses to the empty path"
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

    #[test]
    fn missing_bare_name_cli_points_at_path_and_the_absolute_path_escape_hatch() {
        // The boot-time PATH race: a bare `proton_cli` (default) is a PATH lookup, and the
        // daemon's systemd user-service PATH lacks the CLI's directory. Surface an actionable
        // message that names the executable, PATH, and the absolute-path escape hatch — not a raw
        // os-error-2.
        let client = ProtonDriveClient::with_command_policy(
            PathBuf::from("definitely-not-proton-drive-xyz"),
            CommandPolicy::new(Duration::from_secs(5), 1),
        );

        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("a missing CLI binary must fail")
            .to_string();

        assert!(
            error.contains("could not run the proton-drive CLI"),
            "error should name the CLI: {error}"
        );
        assert!(
            error.contains("PATH"),
            "a bare-name lookup failure should surface PATH: {error}"
        );
        assert!(
            error.contains("proton_cli"),
            "error should point at the proton_cli/--proton-cli escape hatch: {error}"
        );
    }

    #[test]
    fn missing_configured_path_cli_does_not_mislead_about_path() {
        // A configured absolute/relative `proton_cli` that doesn't resolve is NOT a PATH lookup
        // (e.g. a typo'd path). The hint must point at the configured value, not tell the user to
        // put it on PATH.
        let client = ProtonDriveClient::with_command_policy(
            PathBuf::from("/nonexistent/definitely-not-proton-drive"),
            CommandPolicy::new(Duration::from_secs(5), 1),
        );

        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("a missing CLI binary must fail")
            .to_string();

        assert!(
            error.contains("could not run the proton-drive CLI"),
            "error should name the CLI: {error}"
        );
        assert!(
            error.contains("proton_cli"),
            "error should reference the configured proton_cli value: {error}"
        );
        assert!(
            !error.contains("PATH"),
            "a configured-path failure must not mislead about PATH: {error}"
        );
    }

    // Regression guard for issue #40: a `filesystem list` output larger than the OS pipe buffer
    // (~64 KiB) must be captured in full. `run_once` used to read the child's stdout only *after*
    // the child exited, so a large listing filled the pipe, the child blocked on `write`, and the
    // output was truncated mid-JSON — corrupting the parse (and, once the child wedged, timing out).
    // Draining stdout concurrently on a thread fixes it. The fake CLI emits ~105 KiB of valid JSON.
    #[cfg(unix)]
    #[test]
    fn list_captures_output_larger_than_the_pipe_buffer() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '['
i=0
while [ "$i" -lt 800 ]; do
    if [ "$i" -gt 0 ]; then printf ','; fi
    printf '{"name":"file-%04d.txt","uid":"vol~node-%04d","activeRevision":{"claimedDigests":{"sha1":"da39a3ee5e6b4b0d3255bfef95601890afd80709"}}}' "$i" "$i"
    i=$((i + 1))
done
printf ']\n'
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(10), 1),
        );

        let listing = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect("a listing larger than the pipe buffer must be captured and parsed in full");

        assert_eq!(
            listing.len(),
            800,
            "every node from the >64 KiB listing must be present (truncation would drop the tail)"
        );
        assert!(
            listing.contains_key(Path::new("file-0799.txt")),
            "the final node must survive — the tail is exactly what truncation loses"
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

    // Regression guard for issue #56: `run_once`'s success path used to join the output-pipe reader
    // threads with no deadline, assuming a child's exit means EOF is imminent. It does not: the
    // grandchild backgrounded here inherits the pipe write ends and holds them open long after the
    // direct child exits, so the old join blocked for the grandchild's whole lifetime — past
    // `CommandPolicy::timeout`, past the cancellation poll, wedging the daemon (which reconciles
    // under `block_in_place`) until SIGKILL. The fake CLI prints a complete, valid listing before
    // exiting, so pre-fix this returns `Ok` after ~30s and both assertions below fail.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_the_output_pipe_cannot_block_the_success_path() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
sleep 30 &
printf '{"entries":[]}\n'
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_millis(400), 1),
        );

        let started = Instant::now();
        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("a pipe still held open after the child exits must not block forever");
        let elapsed = started.elapsed();

        assert!(
            error.to_string().contains("still held open"),
            "unexpected error: {error}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the drain must end at the command deadline plus the grace, not when the grandchild \
             exits 30s later; elapsed: {elapsed:?}"
        );
    }

    // The bounded drain itself, without a child: a reader that never reports is exactly what a pipe
    // held open by a grandchild looks like to `run_once`. Deterministic — the sender is alive and
    // simply never sends, so the only way out is the deadline.
    #[test]
    fn a_pipe_reader_that_never_reports_expires_at_the_drain_deadline() {
        // Binding (not `_`) keeps the sender alive for the call: dropping it is the panicked-reader
        // case, not the stalled-pipe case.
        let (_sender, receiver) = mpsc::channel::<io::Result<Vec<u8>>>();

        let started = Instant::now();
        let error = collect_pipe_output(
            Some(receiver),
            Instant::now() + Duration::from_millis(50),
            Duration::from_millis(50),
            "list",
            "stdout",
        )
        .expect_err("a reader that never reaches EOF must not block its caller");

        assert!(
            error.to_string().contains("stdout"),
            "the error must name the stalled stream: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "collection must return at the deadline; elapsed: {:?}",
            started.elapsed()
        );
    }

    // A reader thread that panics drops its sender without sending. That must keep reporting a
    // panicked reader (the pre-mpsc `join()` behaviour), not be confused with a stalled pipe.
    #[test]
    fn a_dead_pipe_reader_is_still_reported_as_a_panicked_reader() {
        let (sender, receiver) = mpsc::channel::<io::Result<Vec<u8>>>();
        drop(sender);

        let error = collect_pipe_output(
            Some(receiver),
            Instant::now() + Duration::from_secs(30),
            Duration::from_secs(30),
            "list",
            "stdout",
        )
        .expect_err("a reader thread that died without reporting must surface as an error");

        assert!(
            error.to_string().contains("panicked"),
            "unexpected error: {error}"
        );
    }

    // Regression guard for issue #57: `run_once` used to propagate a `wait_timeout` failure with a
    // bare `?`, unlike the timeout and cancellation exits, leaving the command running detached. For
    // a mutating call (upload/trash) the side effect then proceeds *after* the engine has recorded
    // the action as failed and replanned it — the double side effect the never-auto-retry convention
    // exists to prevent — and the child is never reaped. A real `waitpid` failure cannot be provoked
    // from outside the process, so the wait is injected through the `wait_hook` seam; the wiring
    // (not just an extracted helper) is what is under test. Deterministic: the failure is injected
    // on the first poll that observes the child's pid file, which the child renames into place
    // atomically, so no timing assumption beyond "the child eventually starts".
    #[cfg(unix)]
    #[test]
    fn a_failed_wait_terminates_the_command_instead_of_leaving_it_running() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
echo $$ > "$0.pid.tmp"
mv "$0.pid.tmp" "$0.pid"
sleep 30
"#,
        );
        let pid_path = PathBuf::from(format!("{}.pid", executable.display()));
        let mut client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(30), 1),
        );
        client.wait_hook = Some(Arc::new({
            let pid_path = pid_path.clone();
            move |child: &mut Child, poll_interval: Duration| {
                let outcome = child.wait_timeout(poll_interval);
                if pid_path.exists() {
                    return Err(io::Error::other("injected waitpid failure"));
                }
                outcome
            }
        }));

        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("a failed wait must fail the command");

        assert!(
            error.to_string().contains("could not be waited for"),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("may or may not have completed"),
            "the error must state that the remote outcome is unknown: {error}"
        );
        let pid: libc::pid_t = fs::read_to_string(&pid_path)
            .expect("the fake CLI records its pid")
            .trim()
            .parse()
            .expect("pid");
        // No polling: `terminate_child_tree` SIGKILLs the group and reaps the direct child before
        // `run_once` returns, so the pid is already gone. Pre-fix the child is still sleeping here.
        // SAFETY: signal 0 only probes for the process's existence; it dereferences nothing.
        let probe = unsafe { libc::kill(pid, 0) };
        assert_eq!(
            probe, -1,
            "the command must not still be running after a failed wait (pid {pid})"
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH),
            "the child must be gone and reaped, not merely unsignalable"
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

    fn batch_request(
        local_folder: &Path,
        name: &str,
        expected_sha1: Option<&str>,
    ) -> DownloadRequest {
        DownloadRequest {
            remote_path: PathBuf::from(format!("/my-files/demo/{name}")),
            destination: local_folder.join(name),
            expected_sha1: expected_sha1.map(ToOwned::to_owned),
        }
    }

    /// SHA-1 of `content`, computed by hashing a scratch file — the same code path the salvage
    /// verification uses.
    fn sha1_of_content(directory: &Path, content: &str) -> String {
        let probe = directory.join("sha1-probe");
        fs::write(&probe, content).expect("write probe file");
        compute_sha1(&probe).expect("hash probe file")
    }

    #[cfg(unix)]
    #[test]
    fn download_many_uses_one_invocation_and_places_every_file() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
for a; do last="$a"; done
printf 'alpha content\n' > "$last/alpha.txt"
printf 'beta content\n' > "$last/beta.txt"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let requests = vec![
            batch_request(&local_folder, "alpha.txt", None),
            batch_request(&local_folder, "beta.txt", None),
        ];

        let results = client.download_many(&requests);

        assert!(
            results.iter().all(Result::is_ok),
            "both files should succeed: {results:?}"
        );
        let recorded_args = fs::read_to_string(args_path(&executable)).expect("recorded args");
        let mut lines = recorded_args.lines();
        assert_eq!(lines.next(), Some("filesystem"));
        assert_eq!(lines.next(), Some("download"));
        assert_eq!(lines.next(), Some("/my-files/demo/alpha.txt"));
        assert_eq!(lines.next(), Some("/my-files/demo/beta.txt"));
        let scratch_dir = lines.next().expect("scratch directory argument");
        let scratch_prefix = local_folder.join(crate::DOWNLOAD_SCRATCH_PREFIX);
        assert!(
            scratch_dir.starts_with(scratch_prefix.to_str().expect("utf-8 scratch prefix")),
            "the batch should stage into one private scratch directory under the shared \
             destination folder: {scratch_dir}"
        );
        assert!(
            lines.next().is_none(),
            "exactly one CLI invocation expected"
        );
        assert_eq!(
            fs::read_to_string(local_folder.join("alpha.txt")).expect("alpha at destination"),
            "alpha content\n"
        );
        assert_eq!(
            fs::read_to_string(local_folder.join("beta.txt")).expect("beta at destination"),
            "beta content\n"
        );
        assert!(
            !Path::new(scratch_dir).exists(),
            "the scratch directory should be cleaned up after the batch"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_many_reports_an_unstaged_file_as_a_per_item_error() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
for a; do last="$a"; done
printf 'alpha content\n' > "$last/alpha.txt"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let requests = vec![
            batch_request(&local_folder, "alpha.txt", None),
            batch_request(&local_folder, "beta.txt", None),
        ];

        let results = client.download_many(&requests);

        assert!(results[0].is_ok(), "the staged file succeeds: {results:?}");
        let error = results[1].as_ref().expect_err("the skipped file must fail");
        assert!(
            error.to_string().contains("was not staged"),
            "the per-item error must name the silent skip: {error}"
        );
        assert!(
            local_folder.join("alpha.txt").is_file(),
            "the staged sibling still lands at its destination"
        );
        assert!(
            !local_folder.join("beta.txt").exists(),
            "nothing may appear at the skipped file's destination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_many_salvages_digest_verified_files_when_the_cli_fails() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
for a; do last="$a"; done
printf 'salvaged content\n' > "$last/alpha.txt"
printf 'partial' > "$last/beta.txt"
printf 'gamma content\n' > "$last/gamma.txt"
echo "network dropped" >&2
exit 1
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let alpha_sha1 = sha1_of_content(directory.path(), "salvaged content\n");
        let beta_sha1 = sha1_of_content(directory.path(), "full beta content\n");
        let requests = vec![
            batch_request(&local_folder, "alpha.txt", Some(&alpha_sha1)),
            batch_request(&local_folder, "beta.txt", Some(&beta_sha1)),
            // Fully staged by the fake CLI, but the remote exposed no claimed digest — there
            // is nothing safe to verify against, so it must NOT be salvaged.
            batch_request(&local_folder, "gamma.txt", None),
        ];

        let results = client.download_many(&requests);

        assert!(
            results[0].is_ok(),
            "a fully-staged, digest-verified file must be salvaged: {results:?}"
        );
        assert_eq!(
            fs::read_to_string(local_folder.join("alpha.txt")).expect("salvaged file"),
            "salvaged content\n"
        );
        let error = results[1]
            .as_ref()
            .expect_err("the partially-staged file must fail");
        assert!(
            error.to_string().contains("network dropped"),
            "the per-item error must carry the CLI's own failure: {error}"
        );
        assert!(
            !local_folder.join("beta.txt").exists(),
            "a partial file whose digest cannot verify must never reach its destination"
        );
        assert!(
            results[2].is_err(),
            "a file without a claimed digest must never be salvaged: {results:?}"
        );
        assert!(
            !local_folder.join("gamma.txt").exists(),
            "an unverifiable staged file must never reach its destination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_many_with_case_colliding_basenames_falls_back_to_per_file_downloads() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" >> "$0.args"
printf -- '--\n' >> "$0.args"
for a; do last="$a"; done
name=$(basename "$3")
printf 'fallback content\n' > "$last/$name"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        // Legal siblings on case-sensitive Proton Drive, but they would collapse onto one
        // staged file on a case-insensitive local filesystem — the batch preconditions must
        // reject them so each file keeps its own per-file staging.
        let requests = vec![
            batch_request(&local_folder, "Readme.txt", None),
            batch_request(&local_folder, "readme.txt", None),
        ];

        let results = client.download_many(&requests);

        assert!(
            results.iter().all(Result::is_ok),
            "both fallback downloads should succeed: {results:?}"
        );
        let recorded_args = fs::read_to_string(args_path(&executable)).expect("recorded args");
        assert_eq!(
            recorded_args.matches("--\n").count(),
            2,
            "case-fold-equal basenames must disable the single-invocation batch: {recorded_args}"
        );
    }

    #[test]
    fn batch_download_timeout_scales_per_file_and_saturates() {
        assert_eq!(
            batch_download_timeout(Duration::from_secs(60), 25),
            Duration::from_secs(1500),
            "every file in a batch keeps the full single-command budget"
        );
        assert_eq!(
            batch_download_timeout(Duration::from_secs(1), 1),
            Duration::from_secs(1)
        );
        assert_eq!(
            batch_download_timeout(Duration::MAX, 2),
            Duration::MAX,
            "scaling must saturate instead of overflowing"
        );
    }

    #[test]
    fn claimed_sha1_digests_are_normalized_to_lowercase_at_parse_time() {
        let json = r#"
                [
                    {
                        "uid": "upper-uid",
                        "name": "shouty.txt",
                        "type": "file",
                        "mediaType": "text/plain",
                        "activeRevision": {
                            "ok": true,
                            "value": {
                                "claimedDigests": {
                                    "sha1": "ABCDEF0123456789ABCDEF0123456789ABCDEF01"
                                }
                            }
                        }
                    }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/my-files/demo/")).expect("parse remote files");
        let file = files.get(Path::new("shouty.txt")).expect("parsed file");

        assert_eq!(
            file.sha1_hash.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01"),
            "digests must be lowercased at the parse boundary so every downstream comparison \
             (planner equality, salvage verification) matches compute_sha1's lowercase output"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_many_with_mixed_destination_folders_falls_back_to_per_file_downloads() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
printf '%s\n' "$@" >> "$0.args"
printf -- '--\n' >> "$0.args"
for a; do last="$a"; done
name=$(basename "$3")
printf 'fallback content\n' > "$last/$name"
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let folder_a = directory.path().join("a");
        let folder_b = directory.path().join("b");
        fs::create_dir_all(&folder_a).expect("folder a");
        fs::create_dir_all(&folder_b).expect("folder b");
        let requests = vec![
            batch_request(&folder_a, "one.txt", None),
            DownloadRequest {
                remote_path: PathBuf::from("/my-files/demo/two.txt"),
                destination: folder_b.join("two.txt"),
                expected_sha1: None,
            },
        ];

        let results = client.download_many(&requests);

        assert!(
            results.iter().all(Result::is_ok),
            "both fallback downloads should succeed: {results:?}"
        );
        let recorded_args = fs::read_to_string(args_path(&executable)).expect("recorded args");
        assert_eq!(
            recorded_args.matches("--\n").count(),
            2,
            "destinations in different folders violate the batch preconditions, so each file \
             must download in its own invocation: {recorded_args}"
        );
        assert!(folder_a.join("one.txt").is_file());
        assert!(folder_b.join("two.txt").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn download_fails_cleanly_when_the_cli_produces_no_file() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
echo "Saved file to /somewhere/else/budget.xlsx"
echo "one stray warning" >&2
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

        let message = error.to_string();
        assert!(
            message.contains("no entries were found"),
            "unexpected error: {message}"
        );
        // The CLI exited 0 yet delivered nothing where the daemon looked, so the only clue to
        // where the file actually went is the CLI's own output — the error must carry it.
        assert!(
            message.contains("Saved file to /somewhere/else/budget.xlsx"),
            "the error must include the CLI's stdout: {message}"
        );
        assert!(
            message.contains("one stray warning"),
            "the error must include the CLI's stderr: {message}"
        );
        assert!(
            !destination.exists(),
            "no file should appear at the destination when the download produced nothing"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_classifies_a_node_not_found_exit_as_a_vanished_node() {
        // #31: trashed between the listing and the transfer. Typed so the executor skips this
        // one action instead of failing the pass.
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
echo "Error: Node not found" >&2
exit 1
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(1), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");

        let error = client
            .download(
                Path::new("/my-files/demo/gone.txt"),
                &local_folder.join("gone.txt"),
            )
            .expect_err("a missing node must still be an error");

        assert!(
            is_node_not_found_error(error.as_ref()),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("Node not found"),
            "the classified error must keep the CLI's own message: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_leaves_an_ordinary_failure_unclassified() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
echo "Error: network dropped" >&2
exit 1
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(1), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");

        let error = client
            .download(
                Path::new("/my-files/demo/here.txt"),
                &local_folder.join("here.txt"),
            )
            .expect_err("a failing CLI must be an error");

        assert!(
            !is_node_not_found_error(error.as_ref()),
            "only a missing node may be skippable; everything else fails the pass: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_many_classifies_every_unstaged_file_when_stderr_reports_a_missing_node() {
        // Exit 0, one file staged, one announced as missing, one unstaged for a reason the CLI
        // never explains (a Proton-native document). The check reads the whole invocation's
        // stderr, so BOTH unstaged files classify — the note names no request and attribution is
        // impossible. Pinned deliberately: skipping is the safer read of an unattributable
        // signal, and it costs at most a replan.
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
for a; do last="$a"; done
printf 'alpha content\n' > "$last/alpha.txt"
echo "beta.txt: Node not found" >&2
exit 0
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let requests = vec![
            batch_request(&local_folder, "alpha.txt", None),
            batch_request(&local_folder, "beta.txt", None),
            batch_request(&local_folder, "gamma.txt", None),
        ];

        let results = client.download_many(&requests);

        assert!(results[0].is_ok(), "the staged file succeeds: {results:?}");
        for index in [1, 2] {
            let error = results[index]
                .as_ref()
                .expect_err("an unstaged file must fail");
            assert!(
                is_node_not_found_error(error.as_ref()),
                "unexpected error: {error}"
            );
            assert!(
                error.to_string().contains("cannot be attributed"),
                "the message must admit the note names no file: {error}"
            );
        }
        assert!(
            local_folder.join("alpha.txt").is_file(),
            "the staged sibling still lands at its destination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_batch_reporting_a_missing_node_classifies_its_unsalvaged_files() {
        // One invocation covers the whole batch and names no single file, so every unsalvaged
        // request is classified — the executor skips them (recording nothing, holding the
        // cursor) rather than failing the pass. Digest-verified files still salvage as `Ok`.
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
for a; do last="$a"; done
printf 'salvaged content\n' > "$last/alpha.txt"
echo "Error: Node not found" >&2
exit 1
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(2), 1),
        );
        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let alpha_sha1 = sha1_of_content(directory.path(), "salvaged content\n");
        let requests = vec![
            batch_request(&local_folder, "alpha.txt", Some(&alpha_sha1)),
            batch_request(&local_folder, "beta.txt", None),
        ];

        let results = client.download_many(&requests);

        assert!(
            results[0].is_ok(),
            "a digest-verified file still salvages: {results:?}"
        );
        let error = results[1].as_ref().expect_err("the unsalvaged file fails");
        assert!(
            is_node_not_found_error(error.as_ref()),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("cannot be attributed"),
            "the message must admit the attribution is unknown: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_reports_its_staging_directory_to_the_progress_sink() {
        struct RecordingSink {
            staging: std::sync::Mutex<Option<(PathBuf, bool)>>,
        }
        impl ProgressSink for RecordingSink {
            fn remote_folder_listed(&self, _folders_listed: u64, _directory: &Path) {}
            fn download_staging(&self, scratch_dir: &Path) {
                *self.staging.lock().expect("staging lock") =
                    Some((scratch_dir.to_path_buf(), scratch_dir.is_dir()));
            }
        }

        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
target_dir="$4"
printf 'downloaded content\n' > "$target_dir/budget.xlsx"
exit 0
"#,
        );
        let mut client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(1), 1),
        );
        let sink = Arc::new(RecordingSink {
            staging: std::sync::Mutex::new(None),
        });
        client.install_progress_sink(Arc::clone(&sink) as Arc<dyn ProgressSink>);

        let local_folder = directory.path().join("demo");
        fs::create_dir_all(&local_folder).expect("create local demo folder");
        let destination = local_folder.join("budget.xlsx");
        client
            .download(Path::new("/my-files/demo/budget.xlsx"), &destination)
            .expect("download command");

        let (scratch_dir, existed_when_reported) = sink
            .staging
            .lock()
            .expect("staging lock")
            .clone()
            .expect("the download must report its staging directory");
        assert!(
            scratch_dir.starts_with(&local_folder),
            "staging directory should live under the destination's parent: {}",
            scratch_dir.display()
        );
        assert!(
            existed_when_reported,
            "the staging directory must already exist when reported, so the receiver \
             can immediately poll it for bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_reports_walk_progress_to_the_sink() {
        struct CountingSink {
            calls: std::sync::Mutex<Vec<(u64, PathBuf)>>,
        }
        impl ProgressSink for CountingSink {
            fn remote_folder_listed(&self, folders_listed: u64, directory: &Path) {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push((folders_listed, directory.to_path_buf()));
            }
            fn download_staging(&self, _scratch_dir: &Path) {}
        }

        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if [ "$4" = "/my-files/demo" ]; then
    cat <<'JSON'
[
    {
        "uid": "folder-id",
        "name": { "ok": true, "value": "cloud-folder" },
        "type": "folder"
    }
]
JSON
else
    printf '[]\n'
fi
exit 0
"#,
        );
        let mut client = ProtonDriveClient::with_command_policy(
            executable,
            CommandPolicy::new(Duration::from_secs(1), 1),
        );
        let sink = Arc::new(CountingSink {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        client.install_progress_sink(Arc::clone(&sink) as Arc<dyn ProgressSink>);

        client
            .list_entities(Path::new("/my-files/demo"))
            .expect("recursive list");

        let calls = sink.calls.lock().expect("calls lock").clone();
        assert_eq!(
            calls,
            vec![(1, PathBuf::new()), (2, PathBuf::from("cloud-folder")),],
            "each finished directory should be reported with a running count"
        );
    }

    #[test]
    fn summarize_command_output_handles_empty_and_oversized_output() {
        assert_eq!(summarize_command_output(b""), "(empty)");
        assert_eq!(summarize_command_output(b"  \n"), "(empty)");
        assert_eq!(summarize_command_output(b" saved to /x \n"), "saved to /x");

        let long = "a".repeat(700);
        let summarized = summarize_command_output(long.as_bytes());
        assert!(
            summarized.starts_with("(first 100 chars omitted) …"),
            "unexpected prefix: {summarized}"
        );
        assert!(summarized.ends_with(&"a".repeat(600)));
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

    // Regression guard for issue #54 through the production `client.list()` BFS path
    // (the parse-entry test above only covers an empty `parent_path`). Remote root
    // `/Drive/Documents` contains a subfolder also named `Documents`: the root wrapper
    // must collapse, the root-named subfolder must survive and be BFS-queued, and — the
    // subtlety this test locks down — the basename strip must NOT fire again when its
    // child is resolved under the non-empty `parent_path` "Documents", or the child
    // would be re-parented to the bare root name and collide with real root entries.
    #[cfg(unix)]
    #[test]
    fn list_keeps_a_root_named_subfolders_child_under_its_qualified_path() {
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
if [ "$1" = "filesystem" ] && [ "$2" = "list" ] && [ "$3" = "--json" ]; then
    printf '%s\n' "$4" >> "$0.args"
    if [ "$4" = "/Drive/Documents" ]; then
        cat <<'JSON'
[
    {
        "name": { "ok": true, "value": "Documents" },
        "type": "folder",
        "entries": [
            {
                "uid": "sub-id",
                "name": { "ok": true, "value": "Documents" },
                "type": "folder"
            }
        ]
    }
]
JSON
        exit 0
    fi
    if [ "$4" = "/Drive/Documents/Documents" ]; then
        cat <<'JSON'
[
    {
        "uid": "notes-id",
        "name": { "ok": true, "value": "notes.txt" },
        "type": "file",
        "activeRevision": {
            "ok": true,
            "value": {
                "claimedDigests": {
                    "sha1": "4444444444444444444444444444444444444444"
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
            .list(Path::new("/Drive/Documents"))
            .expect("list a root-named subfolder");

        // The wrapper collapsed, the root-named subfolder survived and was BFS-queued
        // (its directory got listed), proving it did not vanish or re-parent to root.
        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded list paths"),
            "/Drive/Documents\n/Drive/Documents/Documents\n"
        );
        assert_eq!(
            files
                .get(Path::new("Documents/notes.txt"))
                .expect("child under the root-named subfolder")
                .sha1_hash
                .as_deref(),
            Some("4444444444444444444444444444444444444444")
        );
        // The basename strip must NOT have fired at the non-root parent_path.
        assert!(
            !files.contains_key(Path::new("notes.txt")),
            "child must not re-parent to the bare root name"
        );
        assert_eq!(files.len(), 1, "exactly the one nested file must be mapped");
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
    fn rename_or_move_rolls_the_move_back_when_the_rename_fails() {
        // When both the parent and the name change, rename_or_move moves first, then
        // renames. If the rename fails after the move succeeded, the entity is stranded
        // at new-folder/old-name.txt. Without a rollback the next reconcile would
        // duplicate the file on both sides, so rename_or_move must move it back to its
        // original parent and report the failure.
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
  echo "rename boom" >&2
  exit 1
fi
echo "unexpected args: $*" >&2
exit 64
"#,
        );
        let client = ProtonDriveClient::with_command_policy(
            executable.clone(),
            CommandPolicy::new(Duration::from_secs(1), 1),
        );

        let error = client
            .rename_or_move(
                Path::new("/my-files/demo"),
                Path::new("old-folder/old-name.txt"),
                Path::new("new-folder/new-name.txt"),
            )
            .expect_err("a failed rename after a successful move must return an error");

        assert!(
            error.to_string().contains("rename boom") && error.to_string().contains("rolled the"),
            "the error must surface the rename failure and note the rollback: {error}"
        );
        // The move ran, the rename failed, then the entity was moved back to its original
        // parent (old-folder) rather than left stranded under new-folder.
        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "move:/my-files/demo/old-folder/old-name.txt:/my-files/demo/new-folder\n\
             rename:/my-files/demo/new-folder/old-name.txt:new-name.txt\n\
             move:/my-files/demo/new-folder/old-name.txt:/my-files/demo/old-folder\n"
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

    // --- degraded / malformed remote node guards (#72 empty path, #59 undecodable wrapper) ---

    #[test]
    fn a_file_entry_that_resolves_to_the_remote_root_is_not_listed() {
        // #72: a file node named `.`, or one whose path IS the remote root, normalizes to the
        // empty relative path. Keyed under "" it passes every downstream filter and makes the
        // planner download the remote root onto the local root, failing every pass. The
        // directory branch has always rejected the empty path; the file branch must too.
        let json = r#"
                [
                    { "uid": "dot-file", "name": ".", "type": "file" },
                    {
                        "uid": "root-shaped-file",
                        "name": "RemoteFolder",
                        "path": "/Drive/RemoteFolder",
                        "type": "file"
                    },
                    { "uid": "real-file", "name": "keep.txt", "type": "file" }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/Drive/RemoteFolder")).expect("parse remote files");

        assert!(
            !files.contains_key(Path::new("")),
            "a remote entry resolving to the remote root must never be listed as a file: {files:?}"
        );
        assert!(files.contains_key(Path::new("keep.txt")));
    }

    #[test]
    fn an_undecodable_name_fails_the_listing_instead_of_dropping_the_node() {
        // #59: `{"ok": false, ...}` is a value that is PRESENT but unreadable. Dropping the node
        // is indistinguishable from a deletion to the planner, which would then plan a
        // LocalDelete for content that still exists remotely.
        let json = r#"
                [
                    {
                        "uid": "degraded",
                        "name": { "ok": false, "error": "cannot decrypt node name" },
                        "type": "file"
                    },
                    { "uid": "sibling", "name": "keep.txt", "type": "file" }
                ]
                "#;

        let error = parse_remote_files(json, Path::new("/Drive/RemoteFolder"))
            .expect_err("an undecodable name must fail the listing");

        assert!(
            error.to_string().contains("remote listing is incomplete"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_undecodable_folder_name_fails_the_listing_instead_of_dropping_its_subtree() {
        // A name-less folder is worse than a name-less file: the BFS only queues
        // `listing.directories`, so the whole subtree disappears from the remote map.
        let json = r#"
                [
                    {
                        "uid": "degraded-folder",
                        "name": { "ok": false },
                        "type": "folder",
                        "entries": [
                            { "uid": "buried", "name": "notes.txt", "type": "file" }
                        ]
                    }
                ]
                "#;

        let error = parse_remote_entities(json, Path::new("/Drive/RemoteFolder"))
            .expect_err("an undecodable folder name must fail the listing");

        assert!(
            error.to_string().contains("remote listing is incomplete"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_undecodable_node_id_fails_the_listing() {
        // Identity, not just location: a placeholder whose id cannot match makes
        // `find_entity_by_uid` report the node as absent from its parent, and the
        // reconstruction then drops its location — #59 through the incremental path.
        let json = r#"
                [
                    {
                        "uid": { "ok": false },
                        "name": "notes.txt",
                        "type": "file"
                    }
                ]
                "#;

        let error = parse_remote_files(json, Path::new("/Drive/RemoteFolder"))
            .expect_err("an undecodable node id must fail the listing");

        assert!(
            error.to_string().contains("remote listing is incomplete"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_non_string_name_fails_the_listing() {
        // A present-but-unreadable value need not be the `{ok, value}` wrapper.
        let json = r#"[ { "uid": "weird", "name": 42, "type": "file" } ]"#;

        let error = parse_remote_files(json, Path::new("/Drive/RemoteFolder"))
            .expect_err("a non-string name must fail the listing");

        assert!(
            error.to_string().contains("remote listing is incomplete"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_undecodable_name_with_a_decodable_path_is_listed_inert() {
        // The locator survived, so the node keeps its place in the remote map — never read as a
        // deletion — but stays inert: no digest and not downloadable, which routes it into the
        // planner's existing `Unsupported` arm instead of a blind download.
        let json = r#"
                [
                    {
                        "uid": "degraded",
                        "name": { "ok": false },
                        "path": "/Drive/RemoteFolder/sub/notes.txt",
                        "type": "file",
                        "activeRevision": {
                            "claimedDigests": {
                                "sha1": "1111111111111111111111111111111111111111"
                            }
                        }
                    }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/Drive/RemoteFolder")).expect("parse remote files");

        let file = files
            .get(Path::new("sub/notes.txt"))
            .expect("a degraded node with a usable path stays in the map");
        assert_eq!(file.name, "notes.txt");
        assert_eq!(file.sha1_hash, None);
        assert!(!file.downloadable);
    }

    #[test]
    fn an_undecodable_folder_name_with_a_decodable_path_stays_queueable() {
        // The BFS only descends into `listing.directories`, so a degraded folder that a locator
        // still places must land there under its real path — otherwise its whole subtree is
        // missing from the remote map even though the listing itself succeeded.
        let json = r#"
                [
                    {
                        "uid": "folder-id",
                        "name": { "ok": false },
                        "path": "/Drive/RemoteFolder/Reports",
                        "type": "folder"
                    }
                ]
                "#;

        let entities = parse_remote_entities(json, Path::new("/Drive/RemoteFolder"))
            .expect("parse remote entities");

        let directory = entities
            .get(Path::new("Reports"))
            .and_then(RemoteEntity::as_directory)
            .expect("a degraded folder with a usable path stays queueable");
        assert_eq!(directory.name, "Reports");
        assert_eq!(directory.id.as_deref(), Some("folder-id"));
    }

    #[test]
    fn an_absent_or_null_field_keeps_todays_silent_drop() {
        // The guard's blind spot, pinned deliberately: a field that is missing entirely or null
        // is indistinguishable from "the CLI does not send this field", which is legitimate and
        // common (`path` and `id` are routinely absent). Only a PRESENT-but-unreadable value
        // fails the listing. If the real CLI reports an undecryptable name by omitting the field
        // or sending null, #59's shape survives this fix — unverifiable offline; the `#[ignore]`d
        // `live_wrapped_value_shapes_for_the_undecodable_node_guard` in `tests/proton_live.rs`
        // reports which shapes a real account emits.
        let json = r#"
                [
                    { "uid": "no-name", "type": "file" },
                    { "uid": "null-name", "name": null, "type": "file" }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/Drive/RemoteFolder")).expect("parse remote files");

        assert!(files.is_empty(), "unexpected files: {files:?}");
    }

    #[test]
    fn an_undecodable_media_type_does_not_fail_the_listing() {
        // Scope check: only the identity and locator wrappers gate the listing. A node whose
        // non-structural metadata fails to decode is still fully placeable.
        let json = r#"
                [
                    {
                        "uid": "file-id",
                        "name": "notes.txt",
                        "type": "file",
                        "mediaType": { "ok": false }
                    }
                ]
                "#;

        let files =
            parse_remote_files(json, Path::new("/Drive/RemoteFolder")).expect("parse remote files");

        let file = files.get(Path::new("notes.txt")).expect("listed file");
        assert!(file.downloadable);
    }

    #[cfg(unix)]
    #[test]
    fn list_fails_when_a_listed_directory_contains_an_undecodable_node() {
        // The guard must reach the BFS driver (and therefore the daemon), not just the pure
        // parser: an incomplete listing aborts the pass instead of handing the planner a map
        // that is missing a node which still exists remotely.
        let directory = tempdir().expect("tempdir");
        let executable = write_script(
            directory.path(),
            "fake-proton-drive",
            r#"#!/bin/sh
cat <<'JSON'
[
    { "uid": "degraded", "name": { "ok": false }, "type": "file" }
]
JSON
exit 0
"#,
        );
        let client = ProtonDriveClient::new(executable);

        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("an incomplete listing must not be returned as a complete map");

        assert!(
            error.to_string().contains("remote listing is incomplete"),
            "unexpected error: {error}"
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
