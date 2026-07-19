use crate::{AppResult, boxed_error};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtonNode {
    pub id: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub children: Vec<ProtonNode>,
    #[serde(default)]
    pub entries: Vec<ProtonNode>,
    #[serde(default)]
    pub files: Vec<ProtonNode>,
    pub active_revision: Option<ActiveRevision>,
    #[serde(rename = "type")]
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
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub sha1_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProtonDriveClient {
    executable: PathBuf,
}

pub trait ProtonClient: Send + Sync {
    fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>>;
    fn upload(&self, local_path: &Path, remote_root: &Path, relative_path: &Path) -> AppResult<()>;
    fn download(&self, remote_id: &str, destination: &Path) -> AppResult<()>;
    fn delete(&self, remote_id: &str) -> AppResult<()>;
}

impl ProtonDriveClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl ProtonClient for ProtonDriveClient {
    fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
        let output = Command::new(&self.executable)
            .args(["filesystem", "list", "--json"])
            .arg(remote_root)
            .output()?;
        if !output.status.success() {
            return Err(boxed_error(format!(
                "proton-drive list failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = String::from_utf8(output.stdout)?;
        parse_remote_files(&stdout, remote_root)
    }

    fn upload(&self, local_path: &Path, remote_root: &Path, relative_path: &Path) -> AppResult<()> {
        let remote_parent = relative_path
            .parent()
            .map(|parent| remote_root.join(parent))
            .unwrap_or_else(|| remote_root.to_path_buf());
        let output = Command::new(&self.executable)
            .arg("upload")
            .arg(local_path)
            .arg(remote_parent)
            .output()?;
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

    fn download(&self, remote_id: &str, destination: &Path) -> AppResult<()> {
        let output = Command::new(&self.executable)
            .args(["download", "--id", remote_id])
            .arg(destination)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive download failed for {remote_id}: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn delete(&self, remote_id: &str) -> AppResult<()> {
        let output = Command::new(&self.executable)
            .args(["delete", "--id", remote_id])
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(boxed_error(format!(
                "proton-drive delete failed for {remote_id}: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
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

    if !is_folder && let (Some(id), Some(name)) = (node.id.as_deref(), node.name.as_deref()) {
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
}
