use crate::{AppResult, boxed_error};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use tracing::warn;
use wait_timeout::ChildExt;

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_LIST_ATTEMPTS: usize = 2;

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

#[derive(Debug, Clone)]
pub struct ProtonDriveClient {
    executable: PathBuf,
    command_policy: CommandPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandPolicy {
    pub timeout: Duration,
    pub list_attempts: usize,
}

pub trait ProtonClient: Send + Sync {
    fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>>;
    fn ensure_directory(&self, remote_root: &Path, relative_path: &Path) -> AppResult<()>;
    fn upload(&self, local_path: &Path, remote_root: &Path, relative_path: &Path) -> AppResult<()>;
    fn download(&self, remote_path: &Path, destination: &Path) -> AppResult<()>;
    fn delete(&self, remote_path: &Path) -> AppResult<()>;
}

impl ProtonDriveClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            command_policy: CommandPolicy::default(),
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
        let output = self.run_proton_drive(
            "list",
            &[
                OsString::from("filesystem"),
                OsString::from("list"),
                OsString::from("--json"),
                remote_root.as_os_str().to_os_string(),
            ],
            self.command_policy.list_attempts,
        )?;
        if !output.status.success() {
            return Err(boxed_error(format!(
                "proton-drive list failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = String::from_utf8(output.stdout)?;
        parse_remote_files(&stdout, remote_root)
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
        let remote_parent = relative_path
            .parent()
            .map(|parent| remote_root.join(parent))
            .unwrap_or_else(|| remote_root.to_path_buf());
        let output = self.run_proton_drive(
            "upload",
            &[
                OsString::from("filesystem"),
                OsString::from("upload"),
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
        let output = self.run_proton_drive(
            "download",
            &[
                OsString::from("filesystem"),
                OsString::from("download"),
                remote_path.as_os_str().to_os_string(),
                local_folder.as_os_str().to_os_string(),
            ],
            1,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive download failed for {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr)
            )))
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
}

impl ProtonDriveClient {
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
        let mut child = Command::new(&self.executable)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if child.wait_timeout(self.command_policy.timeout)?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        let _ = child.kill();
        let _ = child.wait_with_output();
        warn!(
            operation,
            timeout_ms = self.command_policy.timeout.as_millis(),
            "proton-drive command timed out"
        );
        Err(boxed_error(format!(
            "proton-drive {operation} timed out after {}",
            format_duration(self.command_policy.timeout)
        )))
    }
}

fn trimmed_stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
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
    let value: Value = serde_json::from_str(json)?;
    let mut files = HashMap::new();
    collect_from_value(&value, Path::new(""), remote_root, &mut files)?;
    Ok(files)
}

fn collect_from_value(
    value: &Value,
    parent_path: &Path,
    remote_root: &Path,
    files: &mut HashMap<PathBuf, RemoteFile>,
) -> AppResult<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_from_value(item, parent_path, remote_root, files)?;
            }
        }
        Value::Object(object) => {
            // Deserialize once and let collect_node own all parent-path propagation.
            let node: ProtonNode = serde_json::from_value(Value::Object(object.clone()))?;
            collect_node(&node, parent_path, remote_root, files)?;
        }
        _ => {}
    }
    Ok(())
}

fn collect_node(
    node: &ProtonNode,
    parent_path: &Path,
    remote_root: &Path,
    files: &mut HashMap<PathBuf, RemoteFile>,
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
    if !is_folder && let (Some(id), Some(name)) = (id, node.name.as_deref()) {
        files.insert(
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
        collect_node(child, &next_parent, remote_root, files)?;
    }
    for child in &node.children {
        collect_node(child, &next_parent, remote_root, files)?;
    }
    for child in &node.files {
        collect_node(child, &next_parent, remote_root, files)?;
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

        let error = client
            .list(Path::new("/Drive/RemoteFolder"))
            .expect_err("hung proton-drive should time out");

        assert!(
            error.to_string().contains("timed out after 100ms"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_uses_filesystem_path_command() {
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
            .download(
                Path::new("/my-files/demo/budget.xlsx"),
                Path::new("/tmp/proton-sync-demo/budget.xlsx"),
            )
            .expect("download command");

        assert_eq!(
            fs::read_to_string(args_path(&executable)).expect("recorded args"),
            "filesystem\ndownload\n/my-files/demo/budget.xlsx\n/tmp/proton-sync-demo\n"
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
