//! Read/write the daemon's TOML config, safely.
//!
//! Two hard constraints drive this module:
//! 1. The daemon parses its config with `#[serde(deny_unknown_fields)]` (on `FileConfig` *and* the
//!    nested `[delete_approval]` table), so a single stray key makes the daemon **fail to start**.
//! 2. A user's config carries comments and daemon-only keys the Settings UI does not expose
//!    (`db_path`, `events_full_scan_every`, …) that must survive a save.
//!
//! So we edit the document **in place** with `toml_edit` (comments + untouched keys preserved) and,
//! before writing, hand the whole rendered document to the engine's own
//! `config::validate_file_config_text` — the `FileConfig` parser *plus* every post-parse rule the
//! daemon exits on that a serde shape cannot see (a relative `socket_path`, an unusable
//! `log_level` or `conflict_suffix`, both deletion spellings at once, a bad glob). Re-deriving
//! those here is how the two halves came to disagree about `~` (#135). The include/exclude
//! selective-sync globs use the **bare** keys `include` / `exclude` (not `*_patterns`).
//!
//! **A value is written back in the spelling the file already uses**, and the daemon gives every
//! setting two spellings to get wrong:
//! 1. a kebab-case alias for each key (`log-level` for `log_level`) — writing the other one leaves
//!    both in the file, which serde rejects as `duplicate field` (`key_in_use`);
//! 2. `deletion_policy` against the `[delete_approval]` table, which the daemon refuses together
//!    ([`ConfigDoc::set_deletion_policy`]).
//!
//! Either way a writer with a favourite spelling bricks every config written the other way, and
//! bricks it *silently* — the file still parses as TOML, and only the daemon's next start says so.
//! One rule covers both: read either, write back the one already there.

use std::borrow::Cow;
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

/// What Settings → Deletions calls `deletion_policy` — **the engine's own enum**, re-exported.
///
/// It used to be a copy defined here, mapping the tab's three radio cards onto the daemon's two
/// `[delete_approval]` booleans. The daemon has the key natively now (#194), so a copy would be two
/// enums whose serde spellings have to stay in step by hand — and the one that decides what the
/// daemon *does* is the engine's. Re-exported rather than aliased so `use config_io::DeletionPolicy`
/// keeps working.
///
/// | card                              | `remote` | `local` |
/// | --------------------------------- | -------- | ------- |
/// | Ask me every time *(recommended)* | `true`   | `true`  |
/// | Only ask about permanent ones     | `false`  | `true`  |
/// | Never ask                         | `false`  | `false` |
///
/// Two booleans have four states and the tab draws three, which is why
/// [`DeletionPolicy::OnlyRecoverable`] exists: a hand-edited config can hold `remote = true, local
/// = false`, and the UI must be able to say so rather than round it to the nearest card and
/// silently rewrite a setting the user never touched.
pub use proton_drive_sync_engine::config::DeletionPolicy;

/// What Settings → Deletions calls `local_delete_mode` — **the engine's own enum**, re-exported for
/// the same reason [`DeletionPolicy`] is: the one that decides what the daemon *does* is the
/// engine's, and a copy here would be two enums whose serde spellings stay in step by hand.
///
/// | card                            | key value     |
/// | ------------------------------- | ------------- |
/// | Move them to the trash *(rec.)* | `"trash"`     |
/// | Delete them permanently         | `"permanent"` |
///
/// A DIFFERENT SETTING FROM [`DeletionPolicy`], drawn beside it on the same tab. That one decides
/// whether a deletion waits for you; this one decides what happens once it goes ahead. Two spellings
/// of one setting is what `deletion_policy` and `[delete_approval]` are — this is not that, and the
/// key has exactly one spelling, so [`ConfigDoc::set_local_delete_mode`] needs none of
/// [`ConfigDoc::set_deletion_policy`]'s round-trip care.
pub use proton_drive_sync_engine::trash::LocalDeleteMode;

/// How conflict sidecars are named (`conflict_suffix`), re-exported so the Tauri shell can resolve
/// one from the config file without depending on the engine crate directly — and so there is only
/// ever the engine's definition of what a sidecar looks like.
pub use proton_drive_sync_engine::sync::ConflictNaming;

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

    /// The spelling **this document** uses for `key`, which is not always the one the caller
    /// asked for.
    ///
    /// `FileConfig` gives every key a kebab-case alias (`log-level` for `log_level`,
    /// `local-root` for `local_root`, …), so a hand-written config may legitimately use either.
    /// A reader that knows only the snake_case spelling draws a setting that is *in force* as
    /// though it were unset — and a writer that knows only the snake_case spelling then adds a
    /// **second** spelling of the same field, which serde rejects outright:
    ///
    /// ```text
    /// log_level = "debug"
    /// log-level = "debug"   # duplicate field `log_level`
    /// ```
    ///
    /// That is a config the daemon will not parse and [`Self::save`] therefore refuses to write,
    /// so the user's saves fail forever with an error naming a key they never typed twice. Same
    /// rule as [`Self::set_deletion_policy`], for the same reason: read either spelling, write
    /// back the one the file already uses, and only default to snake_case when it uses neither.
    fn key_in_use<'a>(&self, key: &'a str) -> Cow<'a, str> {
        if self.doc.get(key).is_some() || !key.contains('_') {
            return Cow::Borrowed(key);
        }
        let kebab = key.replace('_', "-");
        if self.doc.get(&kebab).is_some() {
            return Cow::Owned(kebab);
        }
        Cow::Borrowed(key)
    }

    // ---- getters (top-level scalars) ----
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.doc
            .get(&self.key_in_use(key))
            .and_then(Item::as_str)
            .map(str::to_string)
    }
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.doc
            .get(&self.key_in_use(key))
            .and_then(Item::as_integer)
    }
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.doc.get(&self.key_in_use(key)).and_then(Item::as_bool)
    }
    pub fn get_string_array(&self, key: &str) -> Vec<String> {
        self.doc
            .get(&self.key_in_use(key))
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
        let key = self.key_in_use(key).into_owned();
        self.doc[&key] = value(v);
    }
    pub fn set_int(&mut self, key: &str, v: i64) {
        let key = self.key_in_use(key).into_owned();
        self.doc[&key] = value(v);
    }
    pub fn set_bool(&mut self, key: &str, v: bool) {
        let key = self.key_in_use(key).into_owned();
        self.doc[&key] = value(v);
    }
    pub fn set_string_array(&mut self, key: &str, items: &[String]) {
        let mut array = Array::new();
        for item in items {
            array.push(item.as_str());
        }
        let key = self.key_in_use(key).into_owned();
        self.doc[&key] = value(array);
    }
    /// Remove a top-level key entirely (e.g. clearing an override so the daemon default applies).
    pub fn remove(&mut self, key: &str) {
        let key = self.key_in_use(key).into_owned();
        self.doc.remove(&key);
    }

    // ---- the nested `[delete_approval]` table (`remote` / `local` booleans) ----
    pub fn get_delete_approval(&self, direction: &str) -> Option<bool> {
        self.doc
            .get(&self.key_in_use("delete_approval"))
            .and_then(Item::as_table)
            .and_then(|t| t.get(direction))
            .and_then(Item::as_bool)
    }
    pub fn set_delete_approval(&mut self, direction: &str, enabled: bool) {
        let table_key = self.key_in_use("delete_approval").into_owned();
        if self.doc.get(&table_key).and_then(Item::as_table).is_none() {
            self.doc.insert(&table_key, Item::Table(Table::new()));
        }
        if let Some(table) = self.doc[&table_key].as_table_mut() {
            table.insert(direction, value(enabled));
        }
    }

    /// The `deletion_policy` key, when the file uses that spelling. `None` means the file either
    /// says nothing or uses `[delete_approval]` — [`Self::get_deletion_policy`] resolves both.
    pub fn get_deletion_policy_key(&self) -> Option<DeletionPolicy> {
        self.get_str("deletion_policy")
            .and_then(|value| value.parse().ok())
    }

    /// The deletion policy the config currently expresses, in **either** spelling.
    ///
    /// The native `deletion_policy` key is read first; otherwise the two `[delete_approval]`
    /// booleans, where an unset direction reads as `true` — the daemon's own default
    /// (`config.rs`: `table.remote.unwrap_or(true)`) — so an empty config is `AskEveryTime` rather
    /// than an unknown. A file holding *both* is one the daemon refuses to start on; it reads as
    /// the native key here, and [`Self::set_deletion_policy`] is what repairs it.
    pub fn get_deletion_policy(&self) -> DeletionPolicy {
        self.get_deletion_policy_key().unwrap_or_else(|| {
            DeletionPolicy::from_directions(
                self.get_delete_approval("remote").unwrap_or(true),
                self.get_delete_approval("local").unwrap_or(true),
            )
        })
    }

    /// What a local deletion will do to the entity — `trash` when the file says nothing, which is
    /// the daemon's own default and therefore what an untouched config means.
    ///
    /// An UNRECOGNISED value reads as `None` and so as the default here, where the daemon refuses
    /// to start on it. That asymmetry is deliberate: this getter feeds a settings screen that must
    /// render *something*, and the file's own error is reported by [`Self::validate`], which is the
    /// one place that decides whether a config is loadable.
    pub fn get_local_delete_mode_key(&self) -> Option<LocalDeleteMode> {
        self.get_str("local_delete_mode")
            .and_then(|value| value.parse().ok())
    }

    /// [`Self::get_local_delete_mode_key`], with the daemon's default filled in.
    pub fn get_local_delete_mode(&self) -> LocalDeleteMode {
        self.get_local_delete_mode_key().unwrap_or_default()
    }

    /// Write the mode back. One key, one spelling — nothing to repair.
    pub fn set_local_delete_mode(&mut self, mode: LocalDeleteMode) {
        self.set_str("local_delete_mode", mode.as_str());
    }

    /// Write a deletion policy back **in the spelling the file already uses**.
    ///
    /// This is not a style preference, it is the round trip. The daemon rejects a config that sets
    /// `deletion_policy` *and* `[delete_approval]` (they are two spellings of one setting, with no
    /// defensible precedence), so a writer that always emitted its favourite key would brick every
    /// config written the other way — silently, since a save that parses is a save that looks fine
    /// until the daemon next starts. Hence:
    ///
    /// | the file already has | this writes |
    /// | -------------------- | ----------- |
    /// | `deletion_policy`    | `deletion_policy` |
    /// | `[delete_approval]`  | both booleans, table untouched otherwise |
    /// | neither              | `deletion_policy` (the native key) |
    /// | **both**             | `deletion_policy`, and the table is **removed** |
    ///
    /// The last row is the only one that deletes anything a user typed, and it is the only one
    /// where doing nothing leaves a config the daemon will not start on. Repairing it is the point.
    ///
    /// In the `[delete_approval]` case both directions are written explicitly, even when the value
    /// matches the daemon default: leaving a key absent would express the same policy two different
    /// ways in two different files, and the tab's promise is that the radio you can see is the rule
    /// that is running.
    pub fn set_deletion_policy(&mut self, policy: DeletionPolicy) {
        let has_policy_key = self.doc.get(&self.key_in_use("deletion_policy")).is_some();
        let has_table = self.doc.get(&self.key_in_use("delete_approval")).is_some();
        if has_policy_key || !has_table {
            self.set_str("deletion_policy", policy.as_str());
            if has_table {
                self.remove("delete_approval");
            }
            return;
        }
        let (remote, local) = policy.directions();
        self.set_delete_approval("remote", remote);
        self.set_delete_approval("local", local);
    }

    /// Validate the current document against the daemon's own `FileConfig` parser (which enforces
    /// `deny_unknown_fields` and field types). Returns `Invalid` if the daemon would reject it.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // THE PARSE IS NOT THE CHECK. `FileConfig` is a serde shape; a bad glob, a relative
        // `socket_path`, an unparseable `log_level`, a `conflict_suffix` holding a `/`, or both
        // deletion spellings at once are all well-typed TOML the daemon still exits on — which is
        // the worst possible moment to find out, because by then the GUI has said "Saved" and the
        // daemon that was running the old settings is gone.
        //
        // Every one of those rules lives in the engine (`validate_file_config_text`) and is called
        // from here rather than re-implemented. A second copy is how the daemon and the GUI ended
        // up disagreeing about what `~` means (#135), and this file would need the daemon's own
        // `tracing` filter parser to have the same opinion about `log_level`.
        proton_drive_sync_engine::config::validate_file_config_text(&self.to_toml_string())
            .map_err(|e| ConfigError::Invalid(e.to_string()))?;
        // AND ONE RULE THAT IS STRICTER THAN THE DAEMON, deliberately. An ABSENT key is not this:
        // the daemon fills it from its own default. An EMPTY one is, and it is what a settings form
        // produces when someone clears a field. `validate_runtime_config` does not check
        // `proton_cli`, so an empty value starts a daemon that then fails every single pass with
        // `os error 2` — the ENOENT loop #158 traced, arriving from a settings field someone
        // cleared rather than from a PATH race. There is no config in which an empty CLI works.
        //
        // `local_root` / `remote_root` used to be checked here too and no longer are: the engine's
        // `validate_pair_file_values` now refuses an empty root, **per pair**. That matters rather
        // than being tidy-up — this loop reads TOP-LEVEL keys, and a `[[pair]]` file (#102) keeps
        // its roots inside the table, so the check went blind exactly where a second copy is worst.
        // `proton_cli` stays because it is daemon-wide: it is a top-level key by classification
        // (`ConfigKey::scope`), so reading it at the top level is correct for every config shape.
        if self
            .get_str("proton_cli")
            .is_some_and(|v| v.trim().is_empty())
        {
            return Err(ConfigError::Invalid(
                "proton_cli must not be empty".to_owned(),
            ));
        }
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
    fn an_unparseable_file_refuses_the_write_before_a_byte_is_touched() {
        // THIS IS WHAT MAKES A STALE BASE HARMLESS, and it is the reason the Settings screen keeps
        // drawing the last good config under its "these aren't the settings that are running"
        // banner rather than blanking every field. `write_config` (src-tauri) opens with
        // `ConfigDoc::load(&path)?`, so while the file on disk does not parse, NO save can write
        // anything at all — the diff the screen computed against stale values never reaches a file.
        //
        // Pinned rather than assumed: if that first line ever stopped reloading, the stale base
        // would become a real defect and this test is what would say so.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.toml");
        std::fs::write(&path, "local_root = \n").unwrap();
        let err = match ConfigDoc::load(&path) {
            Err(e) => e,
            Ok(_) => panic!("an unparseable file must not load"),
        };
        assert!(matches!(err, ConfigError::Parse(_)), "got {err:?}");
        assert!(
            err.to_string().starts_with("config is not valid TOML"),
            "got {err}"
        );
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
    fn the_local_delete_mode_round_trips_and_the_daemon_accepts_what_the_screen_writes() {
        let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        // An untouched config: no key, and `trash` is what the daemon does — so it is what the tab
        // must draw. EVERY install that predates this key is in exactly this state.
        assert_eq!(doc.get_local_delete_mode_key(), None);
        assert_eq!(doc.get_local_delete_mode(), LocalDeleteMode::Trash);

        for mode in LocalDeleteMode::ALL {
            doc.set_local_delete_mode(mode);
            assert_eq!(doc.get_local_delete_mode_key(), Some(mode));
            assert_eq!(doc.get_local_delete_mode(), mode);
            // THE HALF THAT MATTERS: `save` validates through the daemon's own parser, so a value
            // this screen can write and the daemon cannot start on is a config the GUI bricks.
            doc.validate()
                .expect("the daemon must accept what the Deletions tab writes");
        }
        assert!(doc.to_toml_string().contains("local_delete_mode = \"permanent\""));
    }

    #[test]
    fn the_kebab_spelling_of_the_mode_is_read_and_rewritten_in_place() {
        // `key_in_use` resolves either spelling, and a file written the kebab way must not gain a
        // second snake-cased key beside it — that is one setting written twice.
        let mut doc =
            ConfigDoc::from_toml_str("local-delete-mode = \"permanent\"\n").unwrap();
        assert_eq!(doc.get_local_delete_mode(), LocalDeleteMode::Permanent);
        doc.set_local_delete_mode(LocalDeleteMode::Trash);
        let text = doc.to_toml_string();
        assert!(text.contains("local-delete-mode = \"trash\""), "{text}");
        assert!(!text.contains("local_delete_mode"), "{text}");
    }

    #[test]
    fn an_unreadable_mode_draws_the_default_but_still_fails_validation() {
        // The asymmetry is deliberate. The getter feeds a screen that must render SOMETHING; the
        // file's own error belongs to `validate`, which is the one place that decides whether a
        // config is loadable — and the daemon refuses to start on this file.
        let doc = ConfigDoc::from_toml_str(
            "local_root = \"/x\"\nremote_root = \"/Drive/X\"\nlocal_delete_mode = \"bin\"\n",
        )
        .unwrap();
        assert_eq!(doc.get_local_delete_mode_key(), None);
        assert_eq!(doc.get_local_delete_mode(), LocalDeleteMode::Trash);
        let error = doc.validate().expect_err("the daemon parser must refuse it");
        assert!(error.to_string().contains("local_delete_mode"), "{error}");
    }

    #[test]
    fn an_empty_config_reads_as_the_daemon_default_policy() {
        // Not "unknown": `config.rs` resolves an unset direction to `true`, so a config with no
        // `[delete_approval]` table is already asking about every deletion.
        let doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::AskEveryTime);
    }

    #[test]
    fn each_radio_card_round_trips_through_the_native_key() {
        // A config with neither spelling gains the native `deletion_policy` key.
        for policy in DeletionPolicy::ALL {
            let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
            doc.set_deletion_policy(policy);
            assert_eq!(
                doc.get_str("deletion_policy").as_deref(),
                Some(policy.as_str()),
                "{policy:?} must write the native key"
            );
            assert_eq!(doc.get_deletion_policy(), policy);
            doc.validate()
                .unwrap_or_else(|e| panic!("{policy:?} must satisfy the daemon parser: {e}"));
        }
    }

    #[test]
    fn a_delete_approval_file_keeps_being_written_as_delete_approval() {
        // THE BUG THIS EXISTS TO STOP. The daemon refuses a config that sets both spellings, so a
        // writer that always emitted its favourite key would brick every config written the other
        // way — and the save would still say "Saved", because the file parses. Every existing
        // `[delete_approval]` config in the world is this case.
        for policy in DeletionPolicy::ALL {
            let mut doc = ConfigDoc::from_toml_str(
                "local_root = \"/x\"\n[delete_approval]\nremote = true\nlocal = true\n",
            )
            .unwrap();
            doc.set_deletion_policy(policy);
            let (remote, local) = policy.directions();
            assert_eq!(
                (
                    doc.get_delete_approval("remote"),
                    doc.get_delete_approval("local")
                ),
                (Some(remote), Some(local)),
                "{policy:?} must stay in the table spelling the file already uses"
            );
            assert_eq!(
                doc.get_str("deletion_policy"),
                None,
                "{policy:?} must not add the key that would make this file unstartable"
            );
            assert_eq!(doc.get_deletion_policy(), policy);
            doc.validate()
                .unwrap_or_else(|e| panic!("{policy:?} must satisfy the daemon: {e}"));
        }
    }

    #[test]
    fn a_deletion_policy_file_keeps_being_written_as_deletion_policy() {
        let mut doc =
            ConfigDoc::from_toml_str("local_root = \"/x\"\ndeletion_policy = \"never\"\n").unwrap();
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::Never);
        doc.set_deletion_policy(DeletionPolicy::OnlyPermanent);
        assert_eq!(
            doc.get_str("deletion_policy").as_deref(),
            Some("only_permanent")
        );
        assert_eq!(
            doc.get_delete_approval("remote"),
            None,
            "the table spelling must not appear beside the key spelling"
        );
        doc.validate().expect("daemon parser");
    }

    #[test]
    fn a_file_holding_both_spellings_is_repaired_by_the_next_save() {
        // The daemon will not start on this file. Reading it must still name a policy, saving must
        // normalize it to one spelling, and the result must be a config that starts — which is the
        // only case where this writer removes something a user typed.
        let mut doc = ConfigDoc::from_toml_str(
            "local_root = \"/x\"\ndeletion_policy = \"never\"\n[delete_approval]\nremote = true\n",
        )
        .unwrap();
        assert!(
            doc.validate().is_err(),
            "a file with both spellings is one the daemon refuses"
        );
        assert_eq!(
            doc.get_deletion_policy(),
            DeletionPolicy::Never,
            "the native key is what a mixed file reads as"
        );
        doc.set_deletion_policy(DeletionPolicy::AskEveryTime);
        assert_eq!(
            doc.get_str("deletion_policy").as_deref(),
            Some("ask_every_time")
        );
        assert_eq!(doc.get_delete_approval("remote"), None);
        doc.validate()
            .expect("the save must leave a config the daemon starts on");
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

    /// A config that uses every shape this writer can get wrong at once: `~` paths the daemon
    /// expands (so persisting the expansion would silently repoint the sync root), three `0`
    /// sentinels that mean "disabled" (so normalizing one to a real number would turn a periodic
    /// full walk back on), comments, daemon-only keys, and the legacy `[delete_approval]` spelling.
    const ADVERSARIAL: &str = r#"# hand-written
local_root = "~/ProtonDrive"          # expanded by the daemon, NOT by this file
db_path = "~/.local/state/proton/index.db"
proton_cli = "~/bin/proton-drive"
socket_path = "~/run/proton-sync.sock"
remote_root = "/Drive/RemoteFolder"

# 0 is a MEANING here, not an absence: no periodic resync, no periodic full walk, no age gate.
events_full_scan_every = 0
warm_start_full_walk_every = 0
warm_start_max_cursor_age_secs = 0

conflict_suffix = "from-cloud"
exclude = ["*.tmp"]

[delete_approval]
remote = false
local = true
"#;

    #[test]
    fn a_save_changes_only_what_it_was_asked_to_change() {
        // PARSE -> EDIT -> RENDER -> RE-PARSE, asserting the FILE is semantically unchanged apart
        // from the one key the caller touched. Every literal below is one this writer could
        // plausibly rewrite: an expanded `~`, a sentinel normalized to a real number.
        let mut doc = ConfigDoc::from_toml_str(ADVERSARIAL).unwrap();
        doc.set_str("log_level", "debug");
        let reparsed = ConfigDoc::from_toml_str(&doc.to_toml_string()).unwrap();

        for (key, expected) in [
            ("local_root", "~/ProtonDrive"),
            ("db_path", "~/.local/state/proton/index.db"),
            ("proton_cli", "~/bin/proton-drive"),
            ("socket_path", "~/run/proton-sync.sock"),
        ] {
            assert_eq!(
                reparsed.get_str(key).as_deref(),
                Some(expected),
                "{key} must stay the literal the user wrote — expanding it here repoints it at \
                 whatever HOME this process happens to have (#135)"
            );
        }
        for key in [
            "events_full_scan_every",
            "warm_start_full_walk_every",
            "warm_start_max_cursor_age_secs",
        ] {
            assert_eq!(
                reparsed.get_int(key),
                Some(0),
                "{key}'s 0 is a disabled sentinel and must survive verbatim"
            );
        }
        assert_eq!(
            reparsed.get_str("conflict_suffix").as_deref(),
            Some("from-cloud")
        );
        assert_eq!(reparsed.get_delete_approval("remote"), Some(false));
        assert_eq!(reparsed.get_delete_approval("local"), Some(true));
        assert_eq!(
            reparsed.get_deletion_policy(),
            DeletionPolicy::OnlyPermanent
        );
        assert_eq!(reparsed.get_str("log_level").as_deref(), Some("debug"));
        assert!(
            reparsed.to_toml_string().contains("# hand-written"),
            "comments survive a save"
        );
        reparsed.validate().expect("daemon parser");
    }

    #[test]
    fn the_settings_the_writer_did_not_design_for_survive_a_policy_change() {
        // The same fixture edited through the OTHER new surface. A policy change rewrites the
        // deletion keys and must leave every unrelated setting exactly as written.
        let mut doc = ConfigDoc::from_toml_str(ADVERSARIAL).unwrap();
        doc.set_deletion_policy(DeletionPolicy::Never);
        let rendered = doc.to_toml_string();
        let reparsed = ConfigDoc::from_toml_str(&rendered).unwrap();

        assert_eq!(reparsed.get_deletion_policy(), DeletionPolicy::Never);
        assert_eq!(
            reparsed.get_str("local_root").as_deref(),
            Some("~/ProtonDrive")
        );
        assert_eq!(reparsed.get_int("events_full_scan_every"), Some(0));
        assert_eq!(
            reparsed.get_str("conflict_suffix").as_deref(),
            Some("from-cloud")
        );
        assert!(rendered.contains("# hand-written"));
        reparsed.validate().expect("daemon parser");
    }

    #[test]
    fn the_advanced_keys_round_trip_through_the_document() {
        // G23/#237. `socket_path`, `log_level` and `conflict_suffix` are file keys; the fourth gap
        // (*Reset the index*) is a command, not a setting, and has no key here by design.
        let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        doc.set_str("socket_path", "/run/user/1000/custom.sock");
        doc.set_str("log_level", "proton_drive_sync_engine=debug,warn");
        doc.set_str("conflict_suffix", "from-cloud");
        doc.validate().expect("daemon parser");

        let reparsed = ConfigDoc::from_toml_str(&doc.to_toml_string()).unwrap();
        assert_eq!(
            reparsed.get_str("socket_path").as_deref(),
            Some("/run/user/1000/custom.sock")
        );
        assert_eq!(
            reparsed.get_str("log_level").as_deref(),
            Some("proton_drive_sync_engine=debug,warn")
        );
        assert_eq!(
            reparsed.get_str("conflict_suffix").as_deref(),
            Some("from-cloud")
        );
    }

    #[test]
    fn a_kebab_case_config_is_read_and_written_in_its_own_spelling() {
        // `FileConfig` aliases every key, so `log-level` is a setting genuinely IN FORCE. Reading
        // only the snake_case spelling drew it as unset; writing only the snake_case spelling then
        // added a second spelling of the same field, and serde rejects that outright as
        // `duplicate field` — so `save` refused, and the user's saves failed forever with an error
        // naming a key they never typed twice. Pre-existing for `local_root` and friends; the
        // Advanced keys just made it three more.
        let mut doc = ConfigDoc::from_toml_str(
            "local-root = \"/home/u/Drive\"\nlog-level = \"debug\"\nscan-interval-secs = 120\n\
             conflict-suffix = \"from-cloud\"\n[delete-approval]\nremote = false\n",
        )
        .unwrap();

        assert_eq!(doc.get_str("local_root").as_deref(), Some("/home/u/Drive"));
        assert_eq!(doc.get_str("log_level").as_deref(), Some("debug"));
        assert_eq!(doc.get_int("scan_interval_secs"), Some(120));
        assert_eq!(doc.get_delete_approval("remote"), Some(false));
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::OnlyPermanent);

        doc.set_str("log_level", "warn");
        doc.set_int("scan_interval_secs", 300);
        doc.set_delete_approval("local", false);
        let rendered = doc.to_toml_string();

        assert!(rendered.contains("log-level = \"warn\""), "{rendered}");
        assert!(
            !rendered.contains("log_level"),
            "a second spelling of one field is a config the daemon will not parse:\n{rendered}"
        );
        assert!(!rendered.contains("scan_interval_secs"), "{rendered}");
        assert!(!rendered.contains("[delete_approval]"), "{rendered}");
        doc.validate()
            .expect("a kebab-case config must still be saveable after an edit");
        assert_eq!(
            ConfigDoc::from_toml_str(&rendered)
                .unwrap()
                .get_deletion_policy(),
            DeletionPolicy::Never
        );
    }

    #[test]
    fn a_snake_case_config_is_untouched_by_the_alias_rule() {
        // The common case must not change: with no kebab spelling present, snake_case is written.
        let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        doc.set_str("log_level", "warn");
        doc.set_deletion_policy(DeletionPolicy::Never);
        let rendered = doc.to_toml_string();
        assert!(rendered.contains("log_level = \"warn\""), "{rendered}");
        assert!(!rendered.contains("log-level"), "{rendered}");
        assert!(rendered.contains("deletion_policy"), "{rendered}");
        doc.validate().expect("daemon parser");
    }

    #[test]
    fn a_tilde_socket_path_validates_and_stays_a_tilde() {
        // Ordering: the daemon expands `~` and THEN requires an absolute path, so a validator that
        // checked the literal would reject a config the daemon accepts — and must not tempt the
        // writer into persisting the expansion to make its own check pass.
        let doc = ConfigDoc::from_toml_str("socket_path = \"~/run/proton-sync.sock\"\n").unwrap();
        doc.validate()
            .expect("`~` is expanded before the absolute-path check");
        assert_eq!(
            doc.get_str("socket_path").as_deref(),
            Some("~/run/proton-sync.sock")
        );
    }

    #[test]
    fn the_new_keys_are_refused_when_the_daemon_would_exit_on_them() {
        // Each of these is well-typed TOML that stops the daemon at startup — the moment after the
        // GUI has said "Saved" and the process running the old settings is gone.
        for (toml, needle) in [
            ("socket_path = \"run/daemon.sock\"\n", "absolute path"),
            // `EnvFilter` would take this as the TARGET directive `inf0=trace` and silence the
            // daemon entirely, which is worse than an error because it looks accepted.
            ("log_level = \"inf0\"\n", "invalid log_level"),
            ("log_level = \"a=b=c\"\n", "invalid log_level"),
            ("conflict_suffix = \"\"\n", "must not be empty"),
            ("conflict_suffix = \"a/b\"\n", "path separator"),
            ("conflict_suffix = \".hidden\"\n", "start or end with `.`"),
            (
                "deletion_policy = \"never\"\n[delete_approval]\nremote = true\n",
                "two spellings of one setting",
            ),
        ] {
            let error = ConfigDoc::from_toml_str(toml)
                .unwrap()
                .validate()
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "expected {needle:?} in: {error}");
        }
    }

    #[test]
    fn a_pair_table_no_longer_fails_every_save() {
        // THE PHASE-1 UNLOCK (#102). `save` validates the WHOLE document through the engine, so
        // before `FileConfig` learned the `pair` key a config containing `[[pair]]` did not merely
        // stop the daemon — it made every GUI save fail, including saves of entirely unrelated keys.
        // There was no "hand-write a multi-pair file and try it" path at all.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proton-sync.toml");
        let mut doc = ConfigDoc::from_toml_str(
            "# hand-written\nlog_level = \"info\"\n\n\
             [[pair]]\nname = \"documents\"\nlocal_root = \"/home/me/Documents\"\n\
             remote_root = \"/Drive/Docs\"\n",
        )
        .unwrap();
        doc.validate().expect("a one-pair document is valid");

        // An edit to a key that has nothing to do with pairs saves, and leaves the table alone.
        doc.set_str("log_level", "debug");
        doc.save(&path).expect("save");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[[pair]]"), "got {written}");
        assert!(written.contains("name = \"documents\""), "got {written}");
        assert!(written.contains("log_level = \"debug\""), "got {written}");
        assert!(written.contains("# hand-written"), "comments preserved");
    }

    #[test]
    fn editing_a_per_pair_key_of_a_pair_file_is_refused_legibly_rather_than_split_in_two() {
        // THE PHASE-1 BOUNDARY, pinned rather than discovered later. This writer only knows
        // top-level keys, so asking it to change a per-pair setting of a `[[pair]]` document would
        // produce a file that states one setting twice — which the engine refuses, so the save fails
        // with an error naming both spellings instead of writing a config with two `local_root`s.
        //
        // That is the correct phase-1 answer, not a bug: the GUI has no array-of-tables API yet
        // (ADR 0005 phase 5b), and nothing in the GUI *writes* `[[pair]]`, so only a hand-edited
        // file reaches this. Daemon-wide keys are unaffected — see
        // `a_pair_table_no_longer_fails_every_save`.
        let mut doc = ConfigDoc::from_toml_str(
            "[[pair]]\nname = \"documents\"\nlocal_root = \"/home/me/Documents\"\n\
             remote_root = \"/Drive/Docs\"\n",
        )
        .unwrap();
        doc.set_str("local_root", "/home/me/Elsewhere");
        let error = doc.validate().unwrap_err().to_string();
        assert!(
            error.contains("two spellings of one setting"),
            "got {error}"
        );
        assert!(error.contains("`local_root`"), "got {error}");
    }

    #[test]
    fn a_document_the_daemon_would_refuse_to_start_on_is_still_refused() {
        // The never-brick contract has to hold for the new shape too, or the GUI becomes the way to
        // write a config that stops the daemon. Two pairs are refused because the capability does
        // not exist yet; the other two are the shape rules that will still be rules after it does.
        for (toml, needle) in [
            (
                "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\n\
                 [[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n",
                "not yet supported",
            ),
            (
                "local_root = \"/x\"\n\n[[pair]]\nname = \"a\"\nremote_root = \"/Drive/a\"\n",
                "two spellings of one setting",
            ),
            (
                "[[pair]]\nname = \"a\"\nlocal_root = \"\"\nremote_root = \"/Drive/a\"\n",
                "local_root must not be empty",
            ),
        ] {
            let error = ConfigDoc::from_toml_str(toml)
                .unwrap()
                .validate()
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "expected {needle:?} in: {error}");
        }
    }
}
