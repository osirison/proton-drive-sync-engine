//! Read/write the daemon's TOML config, safely.
//!
//! Two hard constraints drive this module:
//! 1. The daemon parses its config with `#[serde(deny_unknown_fields)]` (on `FileConfig` *and* the
//!    nested `[delete_approval]` table), so a single stray key makes the daemon **fail to start**.
//! 2. A user's config carries comments and daemon-only keys the Settings UI does not expose
//!    (`db_path`, `events_full_scan_every`, …) that must survive a save.
//!
//! So we edit the document **in place** with `toml_edit` (comments + untouched keys preserved) and,
//! before writing, validate the whole rendered document against the daemon's own `FileConfig`
//! parser — refusing to write anything the daemon would reject. The include/exclude selective-sync
//! globs use the **bare** keys `include` / `exclude` (not `*_patterns`).

use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};

/// Why a config read/validate/write failed.
#[derive(Debug)]
pub enum ConfigError {
    Io(String),
    /// The file on disk is not valid TOML.
    Parse(String),
    /// The edited document would be rejected by the daemon's own parser (unknown key, wrong type).
    /// The message is the daemon parser's own error, surfaced verbatim to the user.
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(m) => write!(f, "config i/o error: {m}"),
            ConfigError::Parse(m) => write!(f, "config is not valid TOML: {m}"),
            ConfigError::Invalid(m) => write!(f, "config would be rejected by the daemon: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// An in-memory, edit-in-place view of a config file. Getters read known keys; setters mutate only
/// the targeted key, leaving comments and every other key untouched.
pub struct ConfigDoc {
    doc: DocumentMut,
}

impl ConfigDoc {
    /// Load a config file. A missing file yields an empty document (the daemon has no canonical
    /// default path; the caller owns/discovers the path).
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(ConfigError::Io(e.to_string())),
        };
        Self::from_toml_str(&text)
    }

    /// Parse a document from a TOML string.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(Self { doc })
    }

    /// Render the current document back to TOML (comments and layout preserved).
    pub fn to_toml_string(&self) -> String {
        self.doc.to_string()
    }

    // ---- getters (top-level scalars) ----
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.doc.get(key).and_then(Item::as_str).map(str::to_string)
    }
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.doc.get(key).and_then(Item::as_integer)
    }
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.doc.get(key).and_then(Item::as_bool)
    }
    pub fn get_string_array(&self, key: &str) -> Vec<String> {
        self.doc
            .get(key)
            .and_then(Item::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- setters (edit in place) ----
    pub fn set_str(&mut self, key: &str, v: &str) {
        self.doc[key] = value(v);
    }
    pub fn set_int(&mut self, key: &str, v: i64) {
        self.doc[key] = value(v);
    }
    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.doc[key] = value(v);
    }
    pub fn set_string_array(&mut self, key: &str, items: &[String]) {
        let mut array = Array::new();
        for item in items {
            array.push(item.as_str());
        }
        self.doc[key] = value(array);
    }
    /// Remove a top-level key entirely (e.g. clearing an override so the daemon default applies).
    pub fn remove(&mut self, key: &str) {
        self.doc.remove(key);
    }

    // ---- the nested `[delete_approval]` table (`remote` / `local` booleans) ----
    pub fn get_delete_approval(&self, direction: &str) -> Option<bool> {
        self.doc
            .get("delete_approval")
            .and_then(Item::as_table)
            .and_then(|t| t.get(direction))
            .and_then(Item::as_bool)
    }
    pub fn set_delete_approval(&mut self, direction: &str, enabled: bool) {
        if self
            .doc
            .get("delete_approval")
            .and_then(Item::as_table)
            .is_none()
        {
            self.doc
                .insert("delete_approval", Item::Table(Table::new()));
        }
        if let Some(table) = self.doc["delete_approval"].as_table_mut() {
            table.insert(direction, value(enabled));
        }
    }

    /// Validate the current document against the daemon's own `FileConfig` parser (which enforces
    /// `deny_unknown_fields` and field types). Returns `Invalid` if the daemon would reject it.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let rendered = self.to_toml_string();
        toml::from_str::<proton_drive_sync_engine::config::FileConfig>(&rendered)
            .map(|_| ())
            .map_err(|e| ConfigError::Invalid(e.to_string()))
    }

    /// Validate, then write the document atomically with mode `0600`. Never writes a config the
    /// daemon would refuse to start on.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        write_atomic_0600(path, self.to_toml_string().as_bytes())
            .map_err(|e| ConfigError::Io(e.to_string()))
    }
}

#[cfg(unix)]
fn write_atomic_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)?;
    }
    let base = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let tmp = path.with_file_name(format!(".{base}.tmp"));

    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
    }
    // Enforce 0600 even if the temp file pre-existed with looser perms.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)
}

#[cfg(not(unix))]
fn write_atomic_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# my sync config
local_root = "/home/u/ProtonDrive"
remote_root = "/Drive/RemoteFolder"

# keep the events poller lively
events_driven = true
events_full_scan_every = 10   # daemon-only key the UI does not expose

exclude = ["*.tmp"]
"#;

    #[test]
    fn editing_preserves_comments_and_daemon_only_keys() {
        let mut doc = ConfigDoc::from_toml_str(SAMPLE).unwrap();
        doc.set_string_array("exclude", &["*.tmp".into(), "node_modules/".into()]);
        doc.set_int("scan_interval_secs", 120);

        let rendered = doc.to_toml_string();
        assert!(
            rendered.contains("# my sync config"),
            "top comment lost:\n{rendered}"
        );
        assert!(
            rendered.contains("# keep the events poller lively"),
            "comment lost"
        );
        assert!(
            rendered.contains("events_full_scan_every = 10"),
            "daemon-only key lost"
        );
        assert!(rendered.contains("node_modules/"), "exclude not updated");
        assert!(
            rendered.contains("scan_interval_secs = 120"),
            "new key not added"
        );

        // And it still round-trips through the daemon's parser.
        doc.validate()
            .expect("edited config must satisfy the daemon parser");
    }

    #[test]
    fn validate_rejects_unknown_keys_so_the_daemon_cannot_be_bricked() {
        let doc = ConfigDoc::from_toml_str("local_root = \"/x\"\nfrobnicate = 1\n").unwrap();
        let err = doc.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)), "got {err:?}");
    }

    #[test]
    fn delete_approval_nested_table_round_trips() {
        let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        assert_eq!(doc.get_delete_approval("remote"), None);
        doc.set_delete_approval("remote", false);
        doc.set_delete_approval("local", true);
        assert_eq!(doc.get_delete_approval("remote"), Some(false));
        assert_eq!(doc.get_delete_approval("local"), Some(true));
        doc.validate()
            .expect("nested delete_approval must satisfy the daemon parser");
    }

    #[test]
    fn save_writes_0600_and_is_readable_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/proton-sync.toml");
        let mut doc = ConfigDoc::from_toml_str(SAMPLE).unwrap();
        doc.set_bool("events_driven", false);
        doc.save(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config must be written 0600");
        let reread = ConfigDoc::load(&path).unwrap();
        assert_eq!(reread.get_bool("events_driven"), Some(false));
        assert_eq!(
            reread.get_str("local_root").as_deref(),
            Some("/home/u/ProtonDrive")
        );
    }

    #[test]
    fn missing_file_loads_as_empty_document() {
        let dir = tempfile::tempdir().unwrap();
        let doc = ConfigDoc::load(&dir.path().join("does-not-exist.toml")).unwrap();
        assert_eq!(doc.get_str("local_root"), None);
    }
}
