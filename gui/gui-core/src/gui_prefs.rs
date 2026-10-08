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
    edit_prefs(path, |doc| doc["notify_policy"] = value(policy.as_str()))
}

/// The folder pair the app was last showing (#102 phase 5a, ADR 0005 §6), by name. **Never fails**,
/// for the reason [`load_notify_policy`] never does: a missing, unreadable, unparseable,
/// non-string or empty value is `None`, and `None` means "the default pair".
///
/// This is a **remembered choice, not a fact about the daemon**: nothing here checks that the pair
/// still exists. [`crate::pairs::resolve_selection`] does that against what the daemon reports, on
/// every poll, and never writes the answer back — a name that is momentarily unknown (a daemon
/// restarting, a config being edited) must not erase a good preference.
///
/// It lives here and never in the daemon's own file: that file is `deny_unknown_fields`, so one key
/// it does not know stops the daemon from starting (see the module header).
pub fn load_selected_pair(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc = text.parse::<DocumentMut>().ok()?;
    doc.get("selected_pair")
        .and_then(|item| item.as_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Remember the pair the app is showing, preserving whatever else the file holds. Creates the
/// directory and the file, and replaces an unparseable file like [`store_notify_policy`] does.
///
/// An empty name is refused rather than stored: it would read back as `None`, so a write that
/// reports success would have remembered nothing.
pub fn store_selected_pair(path: &Path, name: &str) -> Result<(), ConfigError> {
    if name.is_empty() {
        return Err(ConfigError::Invalid(
            "a folder pair has a name; an empty one cannot be remembered".to_owned(),
        ));
    }
    edit_prefs(path, |doc| doc["selected_pair"] = value(name))
}

/// Read-modify-write of `gui.toml`, shared by every key so the two cannot differ on what happens to
/// a file that is missing, unparseable, or holds somebody else's keys.
fn edit_prefs(path: &Path, edit: impl FnOnce(&mut DocumentMut)) -> Result<(), ConfigError> {
    let mut doc = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        // A file that exists and does not parse is REPLACED rather than refused. It is the GUI's own
        // file with a couple of keys in it; refusing would leave the control permanently unable to
        // save, and there is nothing in here worth protecting the way a daemon config's comments are.
        .unwrap_or_default();
    edit(&mut doc);
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

    #[test]
    fn a_missing_or_broken_file_remembers_no_pair() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        assert_eq!(load_selected_pair(&path), None);
        std::fs::write(&path, "selected_pair = [ this is not toml").unwrap();
        assert_eq!(load_selected_pair(&path), None);
        // A value that is not a string, or is empty, is not a name.
        std::fs::write(&path, "selected_pair = 7\n").unwrap();
        assert_eq!(load_selected_pair(&path), None);
        std::fs::write(&path, "selected_pair = \"\"\n").unwrap();
        assert_eq!(load_selected_pair(&path), None);
    }

    #[test]
    fn the_selected_pair_round_trips_and_shares_the_file_with_the_notify_policy() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        store_notify_policy(&path, NotifyPolicy::Never).unwrap();
        store_selected_pair(&path, "photos").unwrap();
        // Writing one key must not disturb the other, in either order.
        assert_eq!(load_selected_pair(&path).as_deref(), Some("photos"));
        assert_eq!(load_notify_policy(&path), NotifyPolicy::Never);
        store_notify_policy(&path, NotifyPolicy::OnlyWhenNeeded).unwrap();
        assert_eq!(load_selected_pair(&path).as_deref(), Some("photos"));
        store_selected_pair(&path, "docs").unwrap();
        assert_eq!(load_selected_pair(&path).as_deref(), Some("docs"));
        assert_eq!(load_notify_policy(&path), NotifyPolicy::OnlyWhenNeeded);
    }

    #[test]
    fn storing_a_pair_keeps_the_rest_of_the_file_and_creates_what_is_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("gui.toml");
        store_selected_pair(&path, "photos").unwrap();
        assert_eq!(load_selected_pair(&path).as_deref(), Some("photos"));
        std::fs::write(&path, "# mine\nsomething_else = 3\n").unwrap();
        store_selected_pair(&path, "docs").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine"), "{text}");
        assert!(text.contains("something_else = 3"), "{text}");
        // An unparseable file is replaced, as the policy's writer does.
        std::fs::write(&path, "selected_pair = [ nope").unwrap();
        store_selected_pair(&path, "docs").unwrap();
        assert_eq!(load_selected_pair(&path).as_deref(), Some("docs"));
    }

    #[test]
    fn an_empty_name_is_refused_rather_than_remembered_as_nothing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gui.toml");
        assert!(store_selected_pair(&path, "").is_err());
        assert!(!path.exists(), "a refused write must not create the file");
    }
}
