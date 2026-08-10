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

/// What Settings → Deletions calls `deletion_policy`, expressed over the two keys the daemon
/// actually has (C1, #174).
///
/// `delete_approval.remote` gates the **recoverable** direction — a file leaving this computer goes
/// to Proton's Trash and can be pulled back. `delete_approval.local` gates the **permanent** one —
/// a file removed from disk is gone. So the tab's three cards are three points on those two
/// booleans, and no new config key is needed:
///
/// | card                             | `remote` | `local` |
/// | -------------------------------- | -------- | ------- |
/// | Ask me every time *(recommended)* | `true`   | `true`  |
/// | Only ask about permanent ones     | `false`  | `true`  |
/// | Never ask                         | `false`  | `false` |
///
/// Two booleans have four states and the tab draws three, which is why
/// [`DeletionPolicy::OnlyRecoverable`] exists. A hand-edited config can hold `remote = true,
/// local = false` — ask about the recoverable deletions, let the permanent ones through — and the
/// UI must be able to say so. Folding it into the nearest card would mean the next save silently
/// rewrote a setting the user never touched, which is the one thing [`ConfigDoc`] is built not to
/// do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionPolicy {
    /// Every deletion waits for a person. The daemon's default, and what an empty config means.
    AskEveryTime,
    /// Recoverable deletions go through; permanent ones wait.
    OnlyPermanent,
    /// Nothing waits.
    Never,
    /// Permanent deletions go through; recoverable ones wait. **No radio card draws this** — it is
    /// only reachable by hand-editing the config, and is preserved rather than coerced.
    OnlyRecoverable,
}

impl DeletionPolicy {
    /// The policy a `(remote, local)` pair expresses. Total: every combination has a name.
    pub fn from_directions(remote: bool, local: bool) -> Self {
        match (remote, local) {
            (true, true) => Self::AskEveryTime,
            (false, true) => Self::OnlyPermanent,
            (false, false) => Self::Never,
            (true, false) => Self::OnlyRecoverable,
        }
    }

    /// The `(remote, local)` pair this policy writes. Inverse of [`Self::from_directions`].
    pub fn directions(self) -> (bool, bool) {
        match self {
            Self::AskEveryTime => (true, true),
            Self::OnlyPermanent => (false, true),
            Self::Never => (false, false),
            Self::OnlyRecoverable => (true, false),
        }
    }

    /// Whether a radio card in `8a Deletions tab` represents this policy. `false` for
    /// [`Self::OnlyRecoverable`], which the tab has no control for.
    pub fn is_drawn(self) -> bool {
        !matches!(self, Self::OnlyRecoverable)
    }
}

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

    /// The deletion policy the config currently expresses — see [`DeletionPolicy`].
    ///
    /// An unset direction reads as `true`, which is the daemon's own default
    /// (`config.rs`: `file.remote.unwrap_or(true)`), so an empty config is `AskEveryTime` rather
    /// than an unknown.
    pub fn get_deletion_policy(&self) -> DeletionPolicy {
        DeletionPolicy::from_directions(
            self.get_delete_approval("remote").unwrap_or(true),
            self.get_delete_approval("local").unwrap_or(true),
        )
    }

    /// Write a deletion policy as the two `[delete_approval]` booleans.
    ///
    /// Both directions are written **explicitly**, even when the value matches the daemon default.
    /// The alternative — removing a key to let the default apply — leaves the same policy expressed
    /// two different ways in two different files, and the tab's whole promise is that the radio you
    /// can see is the rule that is running.
    pub fn set_deletion_policy(&mut self, policy: DeletionPolicy) {
        let (remote, local) = policy.directions();
        self.set_delete_approval("remote", remote);
        self.set_delete_approval("local", local);
    }

    /// Validate the current document against the daemon's own `FileConfig` parser (which enforces
    /// `deny_unknown_fields` and field types). Returns `Invalid` if the daemon would reject it.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let rendered = self.to_toml_string();
        toml::from_str::<proton_drive_sync_engine::config::FileConfig>(&rendered)
            .map_err(|e| ConfigError::Invalid(e.to_string()))?;
        // AND THE CHECKS A PARSE CANNOT MAKE. `FileConfig` is a serde shape: every value below is
        // well-typed TOML and still a config the daemon exits on at `validate_runtime_config`
        // (src/config.rs) — which is the worst possible moment to find out, because by then the GUI
        // has said "Saved" and the daemon that was running the old settings is gone.
        //
        // An ABSENT key is not one of these: the daemon fills it from its own default. An EMPTY
        // one is, and it is what a settings form produces when someone clears a field.
        // `proton_cli` is STRICTER THAN THE DAEMON, deliberately. `validate_runtime_config` does
        // not check it, so an empty value starts a daemon that then fails every single pass with
        // `os error 2` — the ENOENT loop #158 traced, arriving from a settings field someone
        // cleared rather than from a PATH race. There is no config in which an empty CLI works.
        for key in ["local_root", "remote_root", "proton_cli"] {
            if self.get_str(key).is_some_and(|v| v.trim().is_empty()) {
                return Err(ConfigError::Invalid(format!("{key} must not be empty")));
            }
        }
        // The globs are compiled at startup and a bad pattern is fatal there. Compiling them here
        // against a throwaway root is exactly the daemon's own check: the root only decides which
        // paths are ignored, and this call passes none.
        proton_drive_sync_engine::index::ScanOptions::new(
            Path::new("/"),
            &[],
            &self.get_string_array("include"),
            &self.get_string_array("exclude"),
        )
        .map_err(|e| ConfigError::Invalid(format!("invalid scan filter configuration: {e}")))?;
        Ok(())
    }

    /// Validate, then write the document atomically with mode `0600`. Never writes a config the
    /// daemon would refuse to start on.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        write_atomic_0600(path, self.to_toml_string().as_bytes())
            .map_err(|e| ConfigError::Io(e.to_string()))
    }
}

/// Expand a leading `~` in a config value, using **the engine's own** expander.
///
/// The GUI is the daemon's second shell-less reader of the same file, and `~` is where the two
/// silently disagreed: the daemon expands `local_root = "~/ProtonDrive"` at config resolution while
/// the GUI joined the literal onto the filesystem, so every GUI-side path feature operated on a
/// directory named `~` under the process's working directory (#135). Sharing the function is the
/// point — a second implementation is a second set of `~user` semantics to keep in step.
///
/// A value the engine refuses (`~user`, or `~` with no `HOME`) comes back **verbatim**. That config
/// is one the daemon will not start on either, and keeping the literal is what lets the eventual
/// error name the string the user typed instead of a working directory they never chose.
pub fn expand_config_path(value: impl Into<std::path::PathBuf>, field: &str) -> std::path::PathBuf {
    let literal = value.into();
    proton_drive_sync_engine::config::expand_tilde(literal.clone(), field).unwrap_or(literal)
}

#[cfg(unix)]
fn write_atomic_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let base = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");

    // Create a fresh, non-predictable sibling temp with `create_new` (O_EXCL) and mode 0600: it
    // never follows a pre-existing symlink and never clobbers an existing file, so the rename below
    // is a genuine atomic replace. Retry on the (vanishingly rare) name collision.
    let (mut file, tmp) = loop {
        let candidate = path.with_file_name(format!(
            ".{base}.{}.{}.tmp",
            std::process::id(),
            unique_suffix()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(file) => break (file, candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };

    let write = (|| {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()
    })();
    if let Err(e) = write.and_then(|_| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(unix)]
fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
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
    fn an_empty_root_is_refused_before_it_reaches_the_daemon() {
        // A settings form produces this the moment someone clears a field. It parses as valid TOML
        // and the daemon exits on it at `validate_runtime_config` — after the GUI has said "Saved"
        // and the process running the old settings is already gone.
        for key in ["local_root", "remote_root", "proton_cli"] {
            let doc = ConfigDoc::from_toml_str(&format!("{key} = \"\"\n")).unwrap();
            let err = doc.validate().unwrap_err();
            assert!(
                err.to_string()
                    .contains(&format!("{key} must not be empty")),
                "got {err:?}"
            );
        }
        // ABSENT is not empty: the daemon fills an absent key from its own default.
        ConfigDoc::from_toml_str("remote_root = \"/Drive/x\"\n")
            .unwrap()
            .validate()
            .expect("an absent local_root is the daemon's default, not a refusal");
    }

    #[test]
    fn a_glob_the_daemon_cannot_compile_is_refused() {
        // `exclude = ["["]` is well-typed TOML and a fatal `invalid scan filter configuration` at
        // startup. Same for an include pattern, which the Advanced tab writes.
        for key in ["include", "exclude"] {
            let doc = ConfigDoc::from_toml_str(&format!("{key} = [\"[\"]\n")).unwrap();
            let err = doc.validate().unwrap_err();
            assert!(
                err.to_string()
                    .contains("invalid scan filter configuration"),
                "got {err:?} for {key}"
            );
        }
        ConfigDoc::from_toml_str("exclude = [\"*.tmp\", \"video-raw/**\"]\n")
            .unwrap()
            .validate()
            .expect("the patterns the Settings screen writes must still pass");
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
    fn an_empty_config_reads_as_the_daemon_default_policy() {
        // Not "unknown": `config.rs` resolves an unset direction to `true`, so a config with no
        // `[delete_approval]` table is already asking about every deletion.
        let doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::AskEveryTime);
    }

    #[test]
    fn each_radio_card_round_trips_through_the_two_daemon_keys() {
        for (policy, remote, local) in [
            (DeletionPolicy::AskEveryTime, true, true),
            (DeletionPolicy::OnlyPermanent, false, true),
            (DeletionPolicy::Never, false, false),
            (DeletionPolicy::OnlyRecoverable, true, false),
        ] {
            let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
            doc.set_deletion_policy(policy);
            assert_eq!(
                (
                    doc.get_delete_approval("remote"),
                    doc.get_delete_approval("local")
                ),
                (Some(remote), Some(local)),
                "{policy:?} must write the documented direction pair"
            );
            assert_eq!(doc.get_deletion_policy(), policy);
            doc.validate()
                .unwrap_or_else(|e| panic!("{policy:?} must satisfy the daemon parser: {e}"));
        }
    }

    #[test]
    fn the_undrawn_fourth_combination_is_preserved_not_coerced() {
        // A hand-edited `remote = true, local = false` has no radio card. Reading it must not round
        // it to the nearest one, or the next save would rewrite a setting nobody touched.
        let doc = ConfigDoc::from_toml_str(
            "local_root = \"/x\"\n[delete_approval]\nremote = true\nlocal = false\n",
        )
        .unwrap();
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::OnlyRecoverable);
        assert!(!doc.get_deletion_policy().is_drawn());
        for drawn in [
            DeletionPolicy::AskEveryTime,
            DeletionPolicy::OnlyPermanent,
            DeletionPolicy::Never,
        ] {
            assert!(
                drawn.is_drawn(),
                "{drawn:?} has a card in `8a Deletions tab`"
            );
        }
    }

    #[test]
    fn a_half_written_table_still_reads_as_a_policy() {
        // Only one direction set: the other falls back to the daemon default (`true`), so this is
        // "never ask about the recoverable ones" — a real, nameable policy.
        let doc =
            ConfigDoc::from_toml_str("local_root = \"/x\"\n[delete_approval]\nremote = false\n")
                .unwrap();
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::OnlyPermanent);
    }

    #[test]
    fn changing_the_policy_preserves_comments_and_daemon_only_keys() {
        let mut doc = ConfigDoc::from_toml_str(SAMPLE).unwrap();
        doc.set_deletion_policy(DeletionPolicy::Never);
        let rendered = doc.to_toml_string();
        assert!(rendered.contains("# my sync config"), "{rendered}");
        assert!(
            rendered.contains("events_full_scan_every = 10"),
            "{rendered}"
        );
        doc.validate().expect("daemon parser");
    }

    #[test]
    #[cfg(unix)] // 0600 permissions + PermissionsExt are unix-only
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
