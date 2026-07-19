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

impl ProtonDriveClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn list(&self, remote_root: &Path) -> AppResult<HashMap<PathBuf, RemoteFile>> {
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

    pub fn upload(
        &self,
        local_path: &Path,
        remote_root: &Path,
        relative_path: &Path,
    ) -> AppResult<()> {
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

    pub fn download(&self, remote_id: &str, destination: &Path) -> AppResult<()> {
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

    pub fn delete(&self, remote_id: &str) -> AppResult<()> {
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
            if let Some(entries) = object.get("entries") {
                collect_from_value(entries, parent_path, remote_root, files)?;
            }
            if let Some(files_value) = object.get("files") {
                collect_from_value(files_value, parent_path, remote_root, files)?;
            }
            if let Some(children) = object.get("children") {
                collect_from_value(children, parent_path, remote_root, files)?;
            }

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

    let relative_path = candidate_path
        .as_deref()
        .map(|path| normalize_remote_path(path, remote_root))
        .unwrap_or_default();

    let is_folder = node.is_folder.unwrap_or(false)
        || matches!(node.kind.as_deref(), Some("folder" | "directory"))
        || (!node.children.is_empty() || !node.entries.is_empty() || !node.files.is_empty());

    if !is_folder
        && let (Some(id), Some(name)) = (node.id.as_deref(), node.name.as_deref())
    {
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

fn normalize_remote_path(path: &Path, remote_root: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix(remote_root) {
        return stripped.to_path_buf();
    }
    if let Some(root_name) = remote_root.file_name()
        && let Ok(stripped) = path.strip_prefix(root_name)
    {
        return stripped.to_path_buf();
    }
    path.to_path_buf()
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
}
