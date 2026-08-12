//! The GUI's own settings file (C6, #179).
//!
//! **Why a second file.** The daemon parses `proton-sync.toml` with `#[serde(deny_unknown_fields)]`
//! on `FileConfig` *and* its nested tables, so one key it does not know makes the daemon **fail to
//! start**. `notify_policy` is a GUI-local preference (IMPLEMENTATION-PLAN row 6) — it is never sent
//! to the daemon and changes nothing about what the daemon does — so it lives in `gui.toml` beside
//! the daemon's config rather than inside it.
//!
//! **Nothing here may change engine behaviour.** `11-notifications.md` is explicit about the third
//! card: *"'Never' must not change engine behaviour — deletions still wait for approval. Turning off
//! notifications is not consent."* That property holds by construction, because nothing in this
//! module is ever read by the daemon or passed to it.
//!
//! Edited in place with `toml_edit` for [`config_io`](crate::config_io)'s reason: a user's comments
//! and any key a later version adds survive a write from this one.

use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, value};

use crate::config_io::ConfigError;

/// When the app is allowed to interrupt you. Three cards, in the order `11a Settings` draws them.
///
/// The values are the wire form the webview sends and the file stores. An unknown or absent value
/// reads back as [`NotifyPolicy::OnlyWhenNeeded`] — the default the first card's badge names, and
/// the safe direction: a corrupt file must not silence the one event that can cost files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyPolicy {
    /// The four events. Default.
    #[default]
    OnlyWhenNeeded,
    /// Only the event that can cost you files; conflicts wait quietly in the app.
    OnlyPermanentDeletions,
    /// No banners at all. The tray glyph still changes and deletions still wait.
    Never,
}

impl NotifyPolicy {
    /// The stored/wire token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnlyWhenNeeded => "only_when_needed",
            Self::OnlyPermanentDeletions => "only_permanent_deletions",
            Self::Never => "never",
        }
    }

    /// Parse a stored/wire token. Unknown → `None`, so the caller decides between defaulting (a
    /// read) and refusing (a write).
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "only_when_needed" => Some(Self::OnlyWhenNeeded),
            "only_permanent_deletions" => Some(Self::OnlyPermanentDeletions),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// `gui.toml` beside the daemon's config. Takes the daemon config path so the two always share a
/// directory — the GUI owns that convention (`config_path.rs`).
pub fn gui_prefs_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|dir| dir.join("gui.toml"))
        .unwrap_or_else(|| PathBuf::from("gui.toml"))
}

/// Read the policy. **Never fails**: a missing, unreadable, unparseable or unknown value is the
/// default, because this file is a preference and not a source of truth about anyone's files.
pub fn load_notify_policy(path: &Path) -> NotifyPolicy {
    let Ok(text) = std::fs::read_to_string(path) else {
        return NotifyPolicy::default();
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return NotifyPolicy::default();
    };
    doc.get("notify_policy")
        .and_then(|item| item.as_str())
        .and_then(NotifyPolicy::parse)
        .unwrap_or_default()
}

/// Write the policy, preserving whatever else the file holds. Creates the directory and the file.
pub fn store_notify_policy(path: &Path, policy: NotifyPolicy) -> Result<(), ConfigError> {
    let mut doc = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        // A file that exists and does not parse is REPLACED rather than refused. It is the GUI's own
        // file with one key in it; refusing would leave the control permanently unable to save, and
        // there is nothing in here worth protecting the way a daemon config's comments are.
        .unwrap_or_default();
    doc["notify_policy"] = value(policy.as_str());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ConfigError::Io(e.to_string()))?;
    }
    std::fs::write(path, doc.to_string()).map_err(|e| ConfigError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_missing_file_is_the_default() {
        let dir = tempdir().unwrap();
        assert_eq!(
            load_notify_policy(&dir.path().join("gui.toml")),
            NotifyPolicy::OnlyWhenNeeded
        );
    }

    #[test]
    fn round_trips_every_policy() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        for policy in [
            NotifyPolicy::OnlyWhenNeeded,
            NotifyPolicy::OnlyPermanentDeletions,
            NotifyPolicy::Never,
        ] {
            store_notify_policy(&path, policy).unwrap();
            assert_eq!(load_notify_policy(&path), policy);
        }
    }

    #[test]
    fn a_write_keeps_the_rest_of_the_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(
            &path,
            "# mine\nsomething_else = 3\nnotify_policy = \"never\"\n",
        )
        .unwrap();
        store_notify_policy(&path, NotifyPolicy::OnlyWhenNeeded).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine"), "{text}");
        assert!(text.contains("something_else = 3"), "{text}");
        assert_eq!(load_notify_policy(&path), NotifyPolicy::OnlyWhenNeeded);
    }

    #[test]
    fn a_corrupt_file_reads_as_the_default_rather_than_as_silence() {
        // FAIL LOUD, NOT QUIET: the failure mode that matters is a broken file silently meaning
        // `never`, which would drop the one banner that can save someone's files.
        let dir = tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        std::fs::write(&path, "notify_policy = [ this is not toml").unwrap();
        assert_eq!(load_notify_policy(&path), NotifyPolicy::OnlyWhenNeeded);
        std::fs::write(&path, "notify_policy = \"whatever\"").unwrap();
        assert_eq!(load_notify_policy(&path), NotifyPolicy::OnlyWhenNeeded);
    }

    #[test]
    fn a_missing_directory_is_created() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("gui.toml");
        store_notify_policy(&path, NotifyPolicy::Never).unwrap();
        assert_eq!(load_notify_policy(&path), NotifyPolicy::Never);
    }

    #[test]
    fn the_prefs_file_sits_beside_the_daemon_config() {
        assert_eq!(
            gui_prefs_path(Path::new("/home/x/.config/proton-sync/proton-sync.toml")),
            PathBuf::from("/home/x/.config/proton-sync/gui.toml")
        );
    }
}
