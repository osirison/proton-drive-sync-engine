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
//! selective-sync globs are written as the **bare** keys `include` / `exclude` when the file uses
//! neither spelling (not `*_patterns`), and as whichever the file already uses when it does.
//!
//! **A value is written back in the spelling the file already uses**, and the daemon gives every
//! setting more than one spelling to get wrong:
//! 1. the engine's list of spellings for each key ([`proton_drive_sync_engine::config::ConfigKey::spellings`]):
//!    a kebab-case alias for each (`log-level` for `log_level`), and `include`/`exclude` for the glob
//!    lists — writing another one leaves both in the file, which serde rejects as `duplicate field`
//!    (`key_in_use_in`);
//! 2. `deletion_policy` against the `[delete_approval]` table, which the daemon refuses together
//!    ([`ConfigDoc::set_deletion_policy`]).
//!
//! Either way a writer with a favourite spelling bricks every config written the other way, and
//! bricks it *silently* — the file still parses as TOML, and only the daemon's next start says so.
//! One rule covers both: read either, write back the one already there.
//!
//! **Folder pairs (#102 phase 5b-1).** A `[[pair]]` table is a TOML table like the top level of the
//! file, so every read and write below is a function over *a table*, and the top level
//! ([`ConfigDoc`]), a pair table ([`PairRead`], [`PairEdit`]) and an inline pair are instantiations of
//! the same code. Which table a key may be written to is the engine's own classification
//! ([`proton_drive_sync_engine::config::ConfigKey::scope`]), not a list kept here: a per-pair key
//! belongs in a pair's table and a daemon-wide one at the top level, and either mistake is a typed
//! [`ConfigError::WrongScope`] before the engine's "two spellings of one setting" has to say it.

use proton_drive_sync_engine::config::{ConfigKey, KeyScope};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{
    Array, ArrayOfTables, DocumentMut, Item, RawString, Table, TableLike, Value, value,
};

/// Why a config read/validate/write failed.
#[derive(Debug)]
pub enum ConfigError {
    Io(String),
    /// The file on disk is not valid TOML.
    Parse(String),
    /// The edited document would be rejected by the daemon's own parser (unknown key, wrong type).
    /// The message is the daemon parser's own error, surfaced verbatim to the user.
    Invalid(String),
    /// A key was written to a table it does not belong in (#102 phase 5b-1). `scope` is what the
    /// **key** is, by the engine's own classification ([`ConfigKey::scope`]) — which also says which
    /// table was the wrong one: a per-pair key is wrong at the top level of a file that declares
    /// `[[pair]]` tables, and a daemon-wide key is wrong inside one. Refused here, in a sentence about
    /// the key, rather than left to reach the engine's "two spellings of one setting" for a setting
    /// the caller only put in the wrong place.
    WrongScope {
        key: String,
        scope: KeyScope,
    },
    /// The file writes its folder pairs as an inline array (`pair = [{ … }]`), which is valid TOML and
    /// a pair list the daemon reads, and not one this editor can change: `toml_edit` holds it as a
    /// value, not as tables. Read, never edited, and never silently converted — rewriting the layout a
    /// person chose is not a save. `file` is the path it was loaded from, when it was loaded from one.
    InlinePairs {
        file: Option<PathBuf>,
    },
    /// No folder pair of that name in this file. `known` is the names it does have, in file order.
    NoSuchPair {
        name: String,
        known: Vec<String>,
    },
    /// A pair name the engine refuses ([`proton_drive_sync_engine::config::validate_pair_name_among`]),
    /// carried **verbatim**: it is the sentence the daemon would exit with for a `[[pair]]` table of
    /// that name, and a form that shows it as someone types must show the same words.
    PairName(String),
    /// Removing this pair would leave the config with none. The engine refuses an empty `pair = []`
    /// (a daemon with no folder has nothing to do), so the refusal is made here, before a file is
    /// written that the daemon would not start on.
    LastPair {
        name: String,
    },
    /// A value the app will not write because the config's other readers could not read it back
    /// (`setup.sh` / `uninstall.sh` find each pair's folder by a line grep and do not unescape). The
    /// engine accepts such a value; this is the app's own narrower rule about what it WRITES.
    Unwritable {
        field: &'static str,
        reason: &'static str,
    },
    /// A rewrite that was supposed to leave the file meaning what it meant came out meaning
    /// something else (promotion, add, remove). **A bug of this module, not of the file**: it is
    /// returned instead of the result, with the file as it was, so the worst a mistake here can do is
    /// refuse a save.
    ChangedMeaning(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(m) => write!(f, "config i/o error: {m}"),
            ConfigError::Parse(m) => write!(f, "config is not valid TOML: {m}"),
            ConfigError::Invalid(m) => write!(f, "config would be rejected by the daemon: {m}"),
            ConfigError::WrongScope {
                key,
                scope: KeyScope::Pair,
            } => write!(
                f,
                "`{key}` is a per-pair setting: in a config that declares `[[pair]]` tables it is \
                 written inside the pair's own table, not at the top level"
            ),
            ConfigError::WrongScope {
                key,
                scope: KeyScope::Daemon,
            } => write!(
                f,
                "`{key}` is a daemon-wide setting: it is written at the top level of the config, \
                 not inside a pair's table"
            ),
            ConfigError::InlinePairs { file } => {
                match file {
                    Some(file) => write!(f, "{} writes", file.display())?,
                    None => write!(f, "this config writes")?,
                }
                write!(
                    f,
                    " its folder pairs as an inline array (`pair = [{{ … }}]`). This app reads that \
                     but edits `[[pair]]` tables only, so it will not change them: edit the file by \
                     hand, or write each pair as a `[[pair]]` table"
                )
            }
            ConfigError::NoSuchPair { name, known } => {
                write!(f, "the config has no folder pair named {name:?}")?;
                if known.is_empty() {
                    write!(f, " (it declares none)")
                } else {
                    write!(
                        f,
                        " (it has {})",
                        known
                            .iter()
                            .map(|known| format!("{known:?}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            ConfigError::PairName(sentence) => write!(f, "{sentence}"),
            ConfigError::LastPair { name } => write!(
                f,
                "{name:?} is the only folder pair in the config, and a config needs at least one: \
                 add the folder that replaces it first, or stop the sync service instead"
            ),
            ConfigError::Unwritable { field, reason } => {
                write!(f, "this app will not write that {field}: {reason}")
            }
            ConfigError::ChangedMeaning(what) => write!(
                f,
                "the config was left as it was, because rewriting it would have changed what it \
                 says ({what}). This is a fault in the app, not in your file"
            ),
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
/// key has exactly one spelling, so [`set_local_delete_mode_in`] needs none of
/// [`set_deletion_policy_in`]'s round-trip care.
pub use proton_drive_sync_engine::trash::LocalDeleteMode;

/// How conflict sidecars are named (`conflict_suffix`), re-exported so the Tauri shell can resolve
/// one from the config file without depending on the engine crate directly — and so there is only
/// ever the engine's definition of what a sidecar looks like.
pub use proton_drive_sync_engine::sync::ConflictNaming;

/// The folder pairs a config file declares, and the name of the implicit one — **the engine's own
/// reading**, re-exported so the GUI never derives a pair's paths (or the `~` rule, or the per-root
/// `.sync` default) a second time (#102 phase 5a, #135). `pair_views` is tolerant of a document
/// part-way through an edit; see its doc for exactly what it does and does not refuse.
///
/// `real_path_conflicts` is the same engine's on-disk overlap rule asked of a file's **text** (phase
/// 5b-1): the half of the daemon's boot check that follows symlinks, which no lexical validator can
/// see. It touches the filesystem, so it is called from a blocking thread.
pub use proton_drive_sync_engine::config::{
    DEFAULT_PAIR_NAME, PairView, pair_views, real_path_conflicts, validate_pair_name,
    validate_pair_name_among,
};

/// How a config file states its folder pairs (ADR 0005 §2), which the pair list alone cannot say: an
/// implicit pair and a `[[pair]]` table named `default` read identically through [`pair_views`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairLayout {
    /// No `pair` key: the top-level keys ARE one pair, called [`DEFAULT_PAIR_NAME`]. Permanent, not a
    /// migration step — and the layout every config this app has ever written has.
    Implicit,
    /// `[[pair]]` tables, one per folder pair. The only layout this editor changes.
    Tables,
    /// A `pair` key that is not tables — in practice `pair = [{ … }]`. Valid TOML and a pair list the
    /// daemon reads; read here, refused as an edit ([`ConfigError::InlinePairs`]).
    InlineArray,
}

/// What a new `[[pair]]` table is born with (#102 phase 5b-2): the add-folder dialog's whole answer,
/// carried in ONE write so that adding a folder with skip rules is one save and so one restart, not
/// two. Every field becomes a `key = value` line of its own in the table — see
/// [`ConfigDoc::add_pair`] for why the shape is fixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairInit {
    /// The pair's name: a wire selector, a `gui.toml` key and a tray id, so it is checked by the
    /// engine's own rule ([`validate_pair_name_among`]) and never changed afterwards.
    pub name: String,
    /// As the person wrote it. A leading `~` is the engine's to expand, not this module's.
    pub local_root: String,
    /// A Drive path, as the person wrote it.
    pub remote_root: String,
    /// The skip rules the dialog staged (written as `exclude`); empty writes no key at all.
    pub exclude: Vec<String>,
}

/// An in-memory, edit-in-place view of a config file. Getters read known keys; setters mutate only
/// the targeted key, leaving comments and every other key untouched.
///
/// **One implementation, three views (#102 phase 5b-1).** Every getter and setter is a free function
/// over a TOML table (`get_str_in`, `set_str_in`, …), and the top level of the file, a `[[pair]]`
/// table and an inline pair read and write through the SAME code — a second copy of the spelling
/// rule or the deletion-policy round trip is how a pair table would come to disagree with the file
/// around it. [`ConfigDoc`]'s own methods are the root instantiation; [`ConfigDoc::pair`] and
/// [`ConfigDoc::pair_mut`] hand out the others.
pub struct ConfigDoc {
    doc: DocumentMut,
    /// Where it was loaded from, so a refusal can name the file.
    source: Option<PathBuf>,
}

/// A pair's settings, read-only: a `[[pair]]` table, an inline pair, or — for a file with no `pair`
/// key — the top level of the file, which is that one pair.
pub struct PairRead<'a> {
    table: &'a dyn TableLike,
}

/// A pair's settings, editable: a `[[pair]]` table, or the top level of an implicit file. **Never an
/// inline pair** ([`ConfigDoc::pair_mut`] refuses those). Writes only per-pair keys; a daemon-wide
/// key is [`ConfigError::WrongScope`], because it is written once, at the top level of the file.
pub struct PairEdit<'a> {
    table: &'a mut dyn TableLike,
}

// ---- the table-generic reads and writes ------------------------------------------------------------

/// The spelling **this table** uses for `key`, which is not always the one the caller asked for.
///
/// The parser gives a key more than one spelling ([`ConfigKey::spellings`]): a kebab-case alias for
/// all of them (`log-level` for `log_level`), and for the two glob lists `include`/`exclude` instead
/// of `include-patterns`. So a hand-written config may legitimately use either, and a reader that
/// knows one spelling draws a setting that is *in force* as though it were unset — and a writer that
/// knows one then adds a **second** spelling of the same field, which serde rejects outright:
///
/// ```text
/// log_level = "debug"
/// log-level = "debug"   # duplicate field `log_level`
/// ```
///
/// That is a config the daemon will not parse and [`ConfigDoc::save`] therefore refuses to write, so
/// the user's saves fail forever with an error naming a key they never typed twice. Same rule as
/// [`set_deletion_policy_in`], for the same reason: read any spelling, write back the one the table
/// already uses, and only when it uses none write the spelling the caller named.
///
/// The set of spellings is **the engine's** — this file used to know snake_case → kebab-case and
/// nothing else, so `include_patterns` (which the parser accepts, spelled by neither) read as an empty
/// skip list and the next save wrote `include` beside it (F-D). A key the engine does not list keeps
/// the old snake → kebab rule, which is all there is to know about it.
fn key_in_use_in<'a>(table: &dyn TableLike, key: &'a str) -> Cow<'a, str> {
    if table.get(key).is_some() {
        return Cow::Borrowed(key);
    }
    if let Some(known) = ConfigKey::from_spelling(key) {
        if let Some(spelling) = known
            .spellings()
            .iter()
            .find(|spelling| table.get(spelling).is_some())
        {
            return Cow::Borrowed(spelling);
        }
        return Cow::Borrowed(key);
    }
    if !key.contains('_') {
        return Cow::Borrowed(key);
    }
    let kebab = key.replace('_', "-");
    if table.get(&kebab).is_some() {
        return Cow::Owned(kebab);
    }
    Cow::Borrowed(key)
}

fn get_str_in(table: &dyn TableLike, key: &str) -> Option<String> {
    table
        .get(&key_in_use_in(table, key))
        .and_then(Item::as_str)
        .map(str::to_string)
}

fn get_int_in(table: &dyn TableLike, key: &str) -> Option<i64> {
    table
        .get(&key_in_use_in(table, key))
        .and_then(Item::as_integer)
}

fn get_bool_in(table: &dyn TableLike, key: &str) -> Option<bool> {
    table
        .get(&key_in_use_in(table, key))
        .and_then(Item::as_bool)
}

fn get_string_array_in(table: &dyn TableLike, key: &str) -> Vec<String> {
    table
        .get(&key_in_use_in(table, key))
        .and_then(Item::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Put `item` at `key`: replace the value of a key that is there (keeping the key, and so the
/// comments written above it), insert a new one otherwise. This is `table[key] = item` for a
/// `Table`, written over [`TableLike`] so a pair table and an inline pair take the same path — and
/// so a save changes the bytes of the key it was asked to change and of no other.
fn put_in(table: &mut dyn TableLike, key: &str, item: Item) {
    match table.get_mut(key) {
        Some(slot) => *slot = item,
        None => {
            table.insert(key, item);
        }
    }
}

fn set_str_in(table: &mut dyn TableLike, key: &str, v: &str) {
    let key = key_in_use_in(table, key).into_owned();
    put_in(table, &key, value(v));
}

fn set_int_in(table: &mut dyn TableLike, key: &str, v: i64) {
    let key = key_in_use_in(table, key).into_owned();
    put_in(table, &key, value(v));
}

fn set_bool_in(table: &mut dyn TableLike, key: &str, v: bool) {
    let key = key_in_use_in(table, key).into_owned();
    put_in(table, &key, value(v));
}

fn set_string_array_in(table: &mut dyn TableLike, key: &str, items: &[String]) {
    let mut array = Array::new();
    for item in items {
        array.push(item.as_str());
    }
    let key = key_in_use_in(table, key).into_owned();
    put_in(table, &key, value(array));
}

/// Remove a key entirely (e.g. clearing an override so the daemon default applies).
fn remove_in(table: &mut dyn TableLike, key: &str) {
    let key = key_in_use_in(table, key).into_owned();
    table.remove(&key);
}

// ---- the nested `[delete_approval]` table (`remote` / `local` booleans) ----

/// The table is read as a table **or an inline table** (`delete_approval = { remote = false }`): the
/// daemon reads both, and a reader of one drew the other as "unset" — a policy of "ask about every
/// deletion" over a pair the daemon is running as "never ask". The pair tables of an inline array are
/// inline tables all the way down, so there the second form is the only one there is.
fn get_delete_approval_in(table: &dyn TableLike, direction: &str) -> Option<bool> {
    table
        .get(&key_in_use_in(table, "delete_approval"))
        .and_then(Item::as_table_like)
        .and_then(|t| t.get(direction))
        .and_then(Item::as_bool)
}

fn set_delete_approval_in(table: &mut dyn TableLike, direction: &str, enabled: bool) {
    let table_key = key_in_use_in(table, "delete_approval").into_owned();
    if table
        .get(&table_key)
        .and_then(Item::as_table_like)
        .is_none()
    {
        table.insert(&table_key, Item::Table(Table::new()));
    }
    if let Some(nested) = table.get_mut(&table_key).and_then(Item::as_table_like_mut) {
        nested.insert(direction, value(enabled));
    }
}

/// The `deletion_policy` key, when the table uses that spelling. `None` means the table either says
/// nothing or uses `[delete_approval]` — [`get_deletion_policy_in`] resolves both.
fn get_deletion_policy_key_in(table: &dyn TableLike) -> Option<DeletionPolicy> {
    get_str_in(table, "deletion_policy").and_then(|value| value.parse().ok())
}

/// The deletion policy the table currently expresses, in **either** spelling.
///
/// The native `deletion_policy` key is read first; otherwise the two `[delete_approval]`
/// booleans, where an unset direction reads as `true` — the daemon's own default
/// (`config.rs`: `table.remote.unwrap_or(true)`) — so an empty config is `AskEveryTime` rather
/// than an unknown. A file holding *both* is one the daemon refuses to start on; it reads as
/// the native key here, and [`set_deletion_policy_in`] is what repairs it.
fn get_deletion_policy_in(table: &dyn TableLike) -> DeletionPolicy {
    get_deletion_policy_key_in(table).unwrap_or_else(|| {
        DeletionPolicy::from_directions(
            get_delete_approval_in(table, "remote").unwrap_or(true),
            get_delete_approval_in(table, "local").unwrap_or(true),
        )
    })
}

/// What a local deletion will do to the entity — `trash` when the table says nothing, which is
/// the daemon's own default and therefore what an untouched config means.
///
/// An UNRECOGNISED value reads as `None` and so as the default here, where the daemon refuses
/// to start on it. That asymmetry is deliberate: this getter feeds a settings screen that must
/// render *something*, and the file's own error is reported by [`ConfigDoc::validate`], which is the
/// one place that decides whether a config is loadable.
fn get_local_delete_mode_key_in(table: &dyn TableLike) -> Option<LocalDeleteMode> {
    get_str_in(table, "local_delete_mode").and_then(|value| value.parse().ok())
}

/// Write the mode back. One key, one spelling — nothing to repair.
fn set_local_delete_mode_in(table: &mut dyn TableLike, mode: LocalDeleteMode) {
    set_str_in(table, "local_delete_mode", mode.as_str());
}

/// Write a deletion policy back **in the spelling the table already uses**.
///
/// This is not a style preference, it is the round trip. The daemon rejects a config that sets
/// `deletion_policy` *and* `[delete_approval]` (they are two spellings of one setting, with no
/// defensible precedence), so a writer that always emitted its favourite key would brick every
/// config written the other way — silently, since a save that parses is a save that looks fine
/// until the daemon next starts. Hence:
///
/// | the table already has | this writes |
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
///
/// **Per table**: the two spellings are exclusive within a pair, and a pair's `delete_approval` is
/// a nested `[pair.delete_approval]` table, so this runs over a `[[pair]]` table exactly as it
/// does over the top level.
fn set_deletion_policy_in(table: &mut dyn TableLike, policy: DeletionPolicy) {
    let has_policy_key = table
        .get(&key_in_use_in(table, "deletion_policy"))
        .is_some();
    let has_table = table
        .get(&key_in_use_in(table, "delete_approval"))
        .is_some();
    if has_policy_key || !has_table {
        set_str_in(table, "deletion_policy", policy.as_str());
        if has_table {
            remove_in(table, "delete_approval");
        }
        return;
    }
    let (remote, local) = policy.directions();
    set_delete_approval_in(table, "remote", remote);
    set_delete_approval_in(table, "local", local);
}

/// The reads every view of a table has. One list of methods, instantiated for the top level of the
/// file ([`ConfigDoc`]) and for a pair ([`PairRead`], [`PairEdit`]); the bodies are the free
/// functions above, so there is nothing here to keep in step with them.
macro_rules! table_reads {
    () => {
        pub fn get_str(&self, key: &str) -> Option<String> {
            get_str_in(self.read_table(), key)
        }
        pub fn get_int(&self, key: &str) -> Option<i64> {
            get_int_in(self.read_table(), key)
        }
        pub fn get_bool(&self, key: &str) -> Option<bool> {
            get_bool_in(self.read_table(), key)
        }
        pub fn get_string_array(&self, key: &str) -> Vec<String> {
            get_string_array_in(self.read_table(), key)
        }
        /// The nested `[delete_approval]` table's `remote` / `local` boolean.
        pub fn get_delete_approval(&self, direction: &str) -> Option<bool> {
            get_delete_approval_in(self.read_table(), direction)
        }
        /// See [`get_deletion_policy_key_in`].
        pub fn get_deletion_policy_key(&self) -> Option<DeletionPolicy> {
            get_deletion_policy_key_in(self.read_table())
        }
        /// See [`get_deletion_policy_in`].
        pub fn get_deletion_policy(&self) -> DeletionPolicy {
            get_deletion_policy_in(self.read_table())
        }
        /// See [`get_local_delete_mode_key_in`].
        pub fn get_local_delete_mode_key(&self) -> Option<LocalDeleteMode> {
            get_local_delete_mode_key_in(self.read_table())
        }
        /// [`Self::get_local_delete_mode_key`], with the daemon's default filled in.
        pub fn get_local_delete_mode(&self) -> LocalDeleteMode {
            self.get_local_delete_mode_key().unwrap_or_default()
        }
    };
}

/// The writes a view that may be edited has. Each first asks the view for **the table this key
/// belongs in** (`table_for`), which is also where a key that belongs in no table this view holds is
/// refused: a setting written to the wrong table is a typed refusal ([`ConfigError::WrongScope`])
/// before anything changes, not a file the engine refuses later.
macro_rules! table_writes {
    () => {
        pub fn set_str(&mut self, key: &str, v: &str) -> Result<(), ConfigError> {
            set_str_in(self.table_for(key)?, key, v);
            Ok(())
        }
        pub fn set_int(&mut self, key: &str, v: i64) -> Result<(), ConfigError> {
            set_int_in(self.table_for(key)?, key, v);
            Ok(())
        }
        pub fn set_bool(&mut self, key: &str, v: bool) -> Result<(), ConfigError> {
            set_bool_in(self.table_for(key)?, key, v);
            Ok(())
        }
        pub fn set_string_array(&mut self, key: &str, items: &[String]) -> Result<(), ConfigError> {
            set_string_array_in(self.table_for(key)?, key, items);
            Ok(())
        }
        /// Remove a key entirely (e.g. clearing an override so the daemon default applies).
        pub fn remove(&mut self, key: &str) -> Result<(), ConfigError> {
            remove_in(self.table_for(key)?, key);
            Ok(())
        }
        pub fn set_delete_approval(
            &mut self,
            direction: &str,
            enabled: bool,
        ) -> Result<(), ConfigError> {
            set_delete_approval_in(self.table_for("delete_approval")?, direction, enabled);
            Ok(())
        }
        /// See [`set_deletion_policy_in`].
        pub fn set_deletion_policy(&mut self, policy: DeletionPolicy) -> Result<(), ConfigError> {
            set_deletion_policy_in(self.table_for("deletion_policy")?, policy);
            Ok(())
        }
        /// See [`set_local_delete_mode_in`].
        pub fn set_local_delete_mode(&mut self, mode: LocalDeleteMode) -> Result<(), ConfigError> {
            set_local_delete_mode_in(self.table_for("local_delete_mode")?, mode);
            Ok(())
        }
    };
}

/// The name a pair table gives itself.
fn name_of(table: &dyn TableLike) -> Option<&str> {
    table.get("name").and_then(Item::as_str)
}

impl<'a> PairRead<'a> {
    fn read_table(&self) -> &dyn TableLike {
        self.table
    }

    table_reads!();
}

impl PairEdit<'_> {
    fn read_table(&self) -> &dyn TableLike {
        &*self.table
    }

    /// A pair table takes the per-pair keys and nothing else. A key the engine lists as daemon-wide
    /// is [`ConfigError::WrongScope`]; one it does not list at all is refused as the unknown field
    /// the daemon would refuse it as (`name` included: a pair is never renamed here — its name is a
    /// wire selector, a `gui.toml` key and a tray id).
    fn table_for(&mut self, key: &str) -> Result<&mut dyn TableLike, ConfigError> {
        match ConfigKey::from_spelling(key).map(ConfigKey::scope) {
            Some(KeyScope::Pair) => Ok(&mut *self.table),
            Some(KeyScope::Daemon) => Err(ConfigError::WrongScope {
                key: key.to_owned(),
                scope: KeyScope::Daemon,
            }),
            None => Err(ConfigError::Invalid(format!("unknown field `{key}`"))),
        }
    }

    table_reads!();
    table_writes!();
}

/// A writer for one pair that puts **each key where the engine says it belongs**: a daemon-wide key
/// at the top level of the file, a per-pair key in the named pair's table. What a save uses, because
/// one Settings save carries both kinds and must write them in the order it was asked to (a key a
/// file did not have is appended where it is written, so the order is part of what a save produces).
///
/// The routing is [`ConfigKey::scope`] and nothing kept here, so a key the engine adds is routed
/// before this file hears of it. A key the engine does not list is refused: this is the path a
/// caller's own field names take, and a name nobody classified is a typo, not a setting.
pub struct PairWriter<'a> {
    doc: &'a mut ConfigDoc,
    pair: &'a str,
}

impl PairWriter<'_> {
    fn table_for(&mut self, key: &str) -> Result<&mut dyn TableLike, ConfigError> {
        match ConfigKey::from_spelling(key).map(ConfigKey::scope) {
            Some(KeyScope::Daemon) => Ok(self.doc.doc.as_table_mut()),
            Some(KeyScope::Pair) => Ok(self.doc.pair_mut(self.pair)?.table),
            None => Err(ConfigError::Invalid(format!("unknown field `{key}`"))),
        }
    }

    table_writes!();
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
        let mut doc = Self::from_toml_str(&text)?;
        doc.source = Some(path.to_owned());
        Ok(doc)
    }

    /// Parse a document from a TOML string.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(Self { doc, source: None })
    }

    /// Render the current document back to TOML (comments and layout preserved).
    pub fn to_toml_string(&self) -> String {
        self.doc.to_string()
    }

    /// How the document states its folder pairs (see [`PairLayout`]).
    pub fn layout(&self) -> PairLayout {
        match self.doc.get("pair") {
            None => PairLayout::Implicit,
            Some(Item::ArrayOfTables(_)) => PairLayout::Tables,
            Some(_) => PairLayout::InlineArray,
        }
    }

    /// Whether the document states its folder pairs as `[[pair]]` tables (or the inline
    /// `pair = [{ … }]` spelling of the same list) rather than as the top-level keys of the one
    /// implicit pair (#102, ADR 0005 §2).
    ///
    /// A question about the **file's shape**, which the pair list alone cannot answer: the implicit
    /// pair and a table named `default` read identically through `pair_views`. It matters to a
    /// caller that has to hand the file to the daemon's own binary, because a pair table is
    /// addressed with `--pair NAME` and never with the per-pair root flags that amend the implicit
    /// pair.
    pub fn declares_pair_tables(&self) -> bool {
        self.layout() != PairLayout::Implicit
    }

    /// The names of the folder pairs the document declares, in file order — the first is the default
    /// pair. An implicit file is one pair called [`DEFAULT_PAIR_NAME`]. Read from the document itself,
    /// so a half-edited one that [`pair_views`] cannot parse still lists the pairs it has.
    pub fn pair_names(&self) -> Vec<String> {
        match self.doc.get("pair") {
            None => vec![DEFAULT_PAIR_NAME.to_owned()],
            Some(Item::ArrayOfTables(tables)) => tables
                .iter()
                .filter_map(|table| name_of(table))
                .map(str::to_owned)
                .collect(),
            Some(Item::Value(Value::Array(array))) => array
                .iter()
                .filter_map(Value::as_inline_table)
                .filter_map(|table| name_of(table))
                .map(str::to_owned)
                .collect(),
            Some(_) => Vec::new(),
        }
    }

    /// One pair's settings for reading, found by name (byte-exact, like the wire selector). An
    /// implicit file's one pair is the top level of the file. **Reads every layout**, an inline array
    /// included — a pair this editor will not change is still one the Settings screen has to show.
    pub fn pair(&self, name: &str) -> Option<PairRead<'_>> {
        let table: &dyn TableLike = match self.doc.get("pair") {
            None => (name == DEFAULT_PAIR_NAME).then_some(self.doc.as_table() as &dyn TableLike)?,
            Some(Item::ArrayOfTables(tables)) => tables
                .iter()
                .find(|table| name_of(*table) == Some(name))
                .map(|table| table as &dyn TableLike)?,
            Some(Item::Value(Value::Array(array))) => array
                .iter()
                .filter_map(Value::as_inline_table)
                .find(|table| name_of(*table) == Some(name))
                .map(|table| table as &dyn TableLike)?,
            Some(_) => return None,
        };
        Some(PairRead { table })
    }

    /// One pair's settings for editing. `Err` for a name the file does not have
    /// ([`ConfigError::NoSuchPair`]) and for an inline-array file ([`ConfigError::InlinePairs`]),
    /// whatever the name.
    pub fn pair_mut(&mut self, name: &str) -> Result<PairEdit<'_>, ConfigError> {
        let layout = self.layout();
        if layout == PairLayout::InlineArray {
            return Err(self.inline_pairs());
        }
        let known = self.pair_names();
        let missing = || ConfigError::NoSuchPair {
            name: name.to_owned(),
            known: known.clone(),
        };
        let table: &mut dyn TableLike = match layout {
            PairLayout::Implicit => {
                if name != DEFAULT_PAIR_NAME {
                    return Err(missing());
                }
                self.doc.as_table_mut()
            }
            PairLayout::Tables => self
                .doc
                .get_mut("pair")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|tables| {
                    tables
                        .iter_mut()
                        .find(|table| name_of(*table) == Some(name))
                })
                .ok_or_else(missing)?,
            PairLayout::InlineArray => return Err(self.inline_pairs()),
        };
        Ok(PairEdit { table })
    }

    fn inline_pairs(&self) -> ConfigError {
        ConfigError::InlinePairs {
            file: self.source.clone(),
        }
    }

    fn read_table(&self) -> &dyn TableLike {
        self.doc.as_table()
    }

    /// A writer for the pair named `pair` that routes each key to the table it belongs in
    /// ([`PairWriter`]).
    pub fn writer_for<'a>(&'a mut self, pair: &'a str) -> PairWriter<'a> {
        PairWriter { doc: self, pair }
    }

    /// The top level of the file takes the daemon-wide keys always, and a per-pair key only where the
    /// top level IS the pair (an implicit file). Beside `[[pair]]` tables a per-pair key written here
    /// is the "two spellings of one setting" the engine refuses, so it is refused first, in a
    /// sentence about the key ([`ConfigError::WrongScope`]); beside an inline array nothing can be
    /// done about it at all ([`ConfigError::InlinePairs`]). A key the engine does not list is let
    /// through for [`Self::validate`] to refuse, exactly as it always was.
    fn table_for(&mut self, key: &str) -> Result<&mut dyn TableLike, ConfigError> {
        if ConfigKey::from_spelling(key).map(ConfigKey::scope) == Some(KeyScope::Pair) {
            match self.layout() {
                PairLayout::Implicit => {}
                PairLayout::Tables => {
                    return Err(ConfigError::WrongScope {
                        key: key.to_owned(),
                        scope: KeyScope::Pair,
                    });
                }
                PairLayout::InlineArray => return Err(self.inline_pairs()),
            }
        }
        Ok(self.doc.as_table_mut())
    }

    // ---- getters and setters of the top level (the root instantiation of the functions above) ----
    table_reads!();
    table_writes!();

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
        //
        // `local_root` / `remote_root` used to be checked here too and no longer are: the engine's
        // `validate_pair_file_values` now refuses an empty root, **per pair**. That matters rather
        // than being tidy-up — this loop reads TOP-LEVEL keys, and a `[[pair]]` file (#102) keeps
        // its roots inside the table, so the check went blind exactly where a second copy is worst.
        // `proton_cli` used to get the same treatment here, for the same "top-level keys only" gap:
        // `validate_runtime_config` used to leave it unchecked, so an empty value started a daemon
        // that then failed every single pass with `os error 2` — the ENOENT loop #158 traced,
        // arriving from a settings form someone cleared rather than a PATH race. #356 closed that
        // gap in the engine itself (`validate_file_config_text`, which is daemon-wide and reads
        // `proton_cli` at the top level regardless of pair shape, plus the merged check in
        // `validate_runtime_config`), so the one call below now refuses it and this file needs no
        // second copy of the rule.
        proton_drive_sync_engine::config::validate_file_config_text(&self.to_toml_string())
            .map_err(|e| ConfigError::Invalid(e.to_string()))?;
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

// ---- adding, removing and promoting folder pairs (#102 phase 5b-2) --------------------------------

/// Settings by key, in a form that does not care how the file laid them out.
type KeyMap = BTreeMap<String, toml::Value>;

/// **What a config file says**, with the layout taken out: the daemon-wide keys, the keys nobody
/// classified, and each pair's per-pair keys in file order. A rewrite of the file (promoting it to
/// `[[pair]]` tables, adding a table, removing one) is correct exactly when this changes in the one
/// way it was asked to — and it is read through the `toml` crate, not through the `toml_edit` calls
/// that did the rewrite, so a mistake in the moving cannot also be a mistake in the checking.
///
/// A key of the implicit pair at the top level and the same key inside a `[[pair]]` table are the same
/// entry here, which is what makes "promotion changes nothing" a comparison rather than a belief.
#[derive(Debug, PartialEq)]
struct Meaning {
    daemon: KeyMap,
    /// Top-level keys the engine does not list, and per-pair keys left beside `[[pair]]` tables. The
    /// daemon refuses both, so this is empty in every file that validates; kept so a rewrite can
    /// neither drop nor invent one without the comparison noticing.
    rest: KeyMap,
    /// `(name, per-pair keys without `name`)`, in file order.
    pairs: Vec<(String, KeyMap)>,
}

impl Meaning {
    fn of(text: &str) -> Result<Self, ConfigError> {
        let root: toml::Table = text
            .parse()
            .map_err(|error: toml::de::Error| ConfigError::Parse(error.to_string()))?;
        let (mut daemon, mut rest, mut root_pair) = (KeyMap::new(), KeyMap::new(), KeyMap::new());
        let mut tables: Option<Vec<toml::Value>> = None;
        for (key, value) in &root {
            if key == "pair" {
                tables = Some(value.as_array().cloned().unwrap_or_default());
                continue;
            }
            let into = match ConfigKey::from_spelling(key).map(ConfigKey::scope) {
                Some(KeyScope::Pair) => &mut root_pair,
                Some(KeyScope::Daemon) => &mut daemon,
                None => &mut rest,
            };
            into.insert(key.clone(), value.clone());
        }
        let pairs = match tables {
            None => vec![(DEFAULT_PAIR_NAME.to_owned(), root_pair)],
            Some(tables) => {
                rest.extend(root_pair);
                tables
                    .iter()
                    .map(|table| {
                        let table = table.as_table().cloned().unwrap_or_default();
                        let name = table
                            .get("name")
                            .and_then(toml::Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let keys = table
                            .into_iter()
                            .filter(|(key, _)| key != "name")
                            .collect::<KeyMap>();
                        (name, keys)
                    })
                    .collect()
            }
        };
        Ok(Self {
            daemon,
            rest,
            pairs,
        })
    }
}

/// The folder pairs of `text` as the ENGINE reads them, for the "paths are unchanged" half of every
/// rewrite's self-check. A text the engine cannot read as a config is its refusal, in its words.
fn views_of(text: &str) -> Result<Vec<PairView>, ConfigError> {
    pair_views(text).map_err(|error| ConfigError::Invalid(error.to_string()))
}

/// A value the app is willing to put on a line of its own. `setup.sh` and `uninstall.sh` find each
/// pair's folder by grepping for `local_root = "…"` (`config_values`): a key that starts its own line,
/// in a basic or a literal string, with no unescaping. A control character would split the value over
/// lines and a `"` or `\` is an escape the reader would not undo — both would make `uninstall.sh`
/// read a folder that is not the pair's, and it purges what it reads. The engine accepts such values
/// (they are legal on Linux); this is narrower on purpose, and only about what the app WRITES.
fn writable_on_one_line(field: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.chars().any(char::is_control) {
        return Err(ConfigError::Unwritable {
            field,
            reason: "it contains a control character, such as a line break",
        });
    }
    if value.contains(['"', '\\']) {
        return Err(ConfigError::Unwritable {
            field,
            reason: "it contains a `\"` or a `\\`, which the install and uninstall scripts do not \
                     unescape when they read the config",
        });
    }
    Ok(())
}

/// Cut the file's header off the front of `first`'s prefix: every line up to and including the LAST
/// blank one, which is returned; what follows it stays as the key's own comment. `None` — and nothing
/// cut — when the prefix has no blank line, because a comment sitting directly on a key is that key's.
fn detach_header(root: &mut Table, first: &str) -> Option<String> {
    let mut key = root.key_mut(first)?;
    let decor = key.leaf_decor_mut();
    let prefix = decor.prefix().and_then(RawString::as_str)?.to_owned();
    let mut cut = None;
    let mut offset = 0;
    for line in prefix.split_inclusive('\n') {
        offset += line.len();
        if line.trim().is_empty() {
            cut = Some(offset);
        }
    }
    let cut = cut?;
    decor.set_prefix(prefix[cut..].to_owned());
    Some(prefix[..cut].to_owned())
}

impl ConfigDoc {
    /// A copy of the document to rewrite. Every operation below edits a copy and only then replaces
    /// `self`, so any failure — a refused name, a file the engine would reject, a self-check that
    /// disagrees — leaves the document, and so the file the caller saves it to, **byte-identical**.
    fn cloned(&self) -> Self {
        Self {
            doc: self.doc.clone(),
            source: self.source.clone(),
        }
    }

    /// The table the folder pairs live in, for a document whose layout is [`PairLayout::Tables`].
    fn pair_tables_mut(&mut self) -> &mut ArrayOfTables {
        self.doc
            .get_mut("pair")
            .and_then(Item::as_array_of_tables_mut)
            .expect("the caller checked the layout is `[[pair]]` tables")
    }

    /// Rewrite an implicit single-pair file as one `[[pair]]` table named `default`, **meaning the
    /// same thing** (ADR 0005 §7). A no-op on a file that already has tables; an inline array is
    /// [`ConfigError::InlinePairs`], because rewriting the layout a person chose is not a save.
    ///
    /// Driven by the ENGINE's classification, not a list kept here: every key whose
    /// [`ConfigKey::scope`] is [`KeyScope::Pair`] — under whichever spelling the file uses — moves
    /// into the new table, in the order the file had them, **with its comments** (the key and its
    /// value are moved as they are, never re-created, so the decor travels). A daemon-wide key stays
    /// at the top level, comments and order untouched; `[delete_approval]` moves as
    /// `[pair.delete_approval]`.
    ///
    /// Done on a copy and checked before it is kept: the result must pass the engine's
    /// `validate_file_config_text`, say the same thing key for key ([`Meaning`]), and read as the
    /// same pair through `pair_views`. A disagreement is [`ConfigError::ChangedMeaning`] and the
    /// document is as it was.
    pub fn promote_to_pair_tables(&mut self) -> Result<(), ConfigError> {
        match self.layout() {
            PairLayout::Tables => return Ok(()),
            PairLayout::InlineArray => return Err(self.inline_pairs()),
            PairLayout::Implicit => {}
        }
        *self = self.promoted()?;
        Ok(())
    }

    fn promoted(&self) -> Result<Self, ConfigError> {
        let mut next = self.cloned();
        let root = next.doc.as_table_mut();
        let moving: Vec<String> = root
            .iter()
            .map(|(key, _)| key.to_owned())
            .filter(|key| {
                ConfigKey::from_spelling(key).map(ConfigKey::scope) == Some(KeyScope::Pair)
            })
            .collect();
        // A comment belongs to the key below it, so it moves with that key — except the file's own
        // header: when the FIRST key of the file is one that moves, the comment block above it that a
        // blank line separates from it is a title for the file, and a title that ends up inside a
        // `[[pair]]` table, under `name = "default"`, says something the file no longer means.
        let header = moving
            .first()
            .filter(|first| {
                root.iter()
                    .next()
                    .is_some_and(|(key, _)| key == first.as_str())
            })
            .and_then(|first| detach_header(root, first));
        let mut pair = Table::new();
        pair.insert("name", value(DEFAULT_PAIR_NAME));
        for key in moving {
            if let Some((key, item)) = root.remove_entry(&key) {
                pair.insert_formatted(&key, item);
            }
        }
        if let Some(header) = header {
            // Back at the top: above the first key that stayed, or above the table when none did.
            let first_staying = root.iter().next().map(|(key, _)| key.to_owned());
            match first_staying.and_then(|key| root.key_mut(&key)) {
                Some(mut key) => {
                    let decor = key.leaf_decor_mut();
                    let kept = decor
                        .prefix()
                        .and_then(RawString::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    // The header already ends in a blank line; the blank line the key had above it
                    // would make two.
                    let kept = kept.strip_prefix('\n').unwrap_or(&kept);
                    decor.set_prefix(format!("{header}{kept}"));
                }
                None => pair.decor_mut().set_prefix(header),
            }
        }
        let mut tables = ArrayOfTables::new();
        tables.push(pair);
        root.insert("pair", Item::ArrayOfTables(tables));

        next.validate()?;
        let (before, after) = (self.to_toml_string(), next.to_toml_string());
        if Meaning::of(&before)? != Meaning::of(&after)? {
            return Err(ConfigError::ChangedMeaning(
                "a setting moved, changed or was lost in the promotion".to_owned(),
            ));
        }
        if views_of(&before)? != views_of(&after)? {
            return Err(ConfigError::ChangedMeaning(
                "the folder's paths read differently after the promotion".to_owned(),
            ));
        }
        Ok(next)
    }

    /// Append a `[[pair]]` table for a new folder (#102 phase 5b-2), promoting an implicit file first.
    /// One write: the name, both roots and the staged skip rules arrive together, so the caller saves
    /// and restarts once.
    ///
    /// **The table's shape is fixed**, because `uninstall.sh` reads it with a line grep: `name`,
    /// `local_root`, `remote_root` and (when there are any) `exclude`, each a scalar or a one-line
    /// array on a line of its own — no inline table, no dotted key. A value the grep could not read
    /// back is [`ConfigError::Unwritable`].
    ///
    /// Refused, with the file untouched: an inline-array file, a name the engine refuses
    /// ([`ConfigError::PairName`], its sentence verbatim), and **anything the daemon would then refuse
    /// to start on** — the whole document goes through the engine's `validate_file_config_text`, so a
    /// lexical overlap, a missing root beside other pairs and `dry_run = true` beside a second pair
    /// (maintainer decision M3) each answer in the engine's own words. The overlap through a symlink
    /// is the one rule a text cannot see; `real_path_conflicts` asks it of the result.
    pub fn add_pair(&mut self, init: PairInit) -> Result<(), ConfigError> {
        *self = self.with_pair_added(&init)?;
        Ok(())
    }

    fn with_pair_added(&self, init: &PairInit) -> Result<Self, ConfigError> {
        let mut next = match self.layout() {
            PairLayout::InlineArray => return Err(self.inline_pairs()),
            PairLayout::Implicit => self.promoted()?,
            PairLayout::Tables => self.cloned(),
        };
        let existing = next.pair_names();
        let others: Vec<&str> = existing.iter().map(String::as_str).collect();
        validate_pair_name_among(&init.name, &others, others.len())
            .map_err(ConfigError::PairName)?;
        writable_on_one_line("local_root", &init.local_root)?;
        writable_on_one_line("remote_root", &init.remote_root)?;
        for rule in &init.exclude {
            writable_on_one_line("skip rule", rule)?;
        }

        let mut table = Table::new();
        table.insert("name", value(init.name.as_str()));
        table.insert("local_root", value(init.local_root.as_str()));
        table.insert("remote_root", value(init.remote_root.as_str()));
        if !init.exclude.is_empty() {
            set_string_array_in(&mut table, "exclude", &init.exclude);
        }
        next.pair_tables_mut().push(table);

        next.validate()?;
        let (before, after) = (self.to_toml_string(), next.to_toml_string());
        let (old, new) = (Meaning::of(&before)?, Meaning::of(&after)?);
        let mut expected = old.pairs;
        let kept = expected.len();
        let mut added = KeyMap::new();
        added.insert(
            "local_root".to_owned(),
            toml::Value::from(init.local_root.as_str()),
        );
        added.insert(
            "remote_root".to_owned(),
            toml::Value::from(init.remote_root.as_str()),
        );
        if !init.exclude.is_empty() {
            added.insert(
                "exclude".to_owned(),
                toml::Value::Array(
                    init.exclude
                        .iter()
                        .map(|rule| rule.as_str().into())
                        .collect(),
                ),
            );
        }
        expected.push((init.name.clone(), added));
        if (new.daemon, new.rest, new.pairs) != (old.daemon, old.rest, expected) {
            return Err(ConfigError::ChangedMeaning(
                "the new folder pair is not exactly the one that was asked for, or another \
                 setting changed with it"
                    .to_owned(),
            ));
        }
        if views_of(&before)? != views_of(&after)?[..kept] {
            return Err(ConfigError::ChangedMeaning(
                "an existing folder pair's paths read differently after the add".to_owned(),
            ));
        }
        Ok(next)
    }

    /// Remove the `[[pair]]` table called `name` (#102 phase 5b-2): the table, with the comments
    /// written above it, and nothing else. **The folder, its index and the remote are not this
    /// module's** — it edits a document.
    ///
    /// Refused, with the document untouched: a name the file does not have
    /// ([`ConfigError::NoSuchPair`]), the **last** pair ([`ConfigError::LastPair`] — the engine refuses
    /// an empty list), and an inline-array file. An implicit file has exactly one pair, so removing it
    /// is the last-pair refusal. A file left with one table stays in `[[pair]]` form: the layout is
    /// never rewritten behind a person's back. Removing the FIRST table makes the next one the default
    /// pair (the pair a command addressed to no pair reaches); the caller says so.
    pub fn remove_pair(&mut self, name: &str) -> Result<(), ConfigError> {
        *self = self.with_pair_removed(name)?;
        Ok(())
    }

    fn with_pair_removed(&self, name: &str) -> Result<Self, ConfigError> {
        let known = self.pair_names();
        let missing = || ConfigError::NoSuchPair {
            name: name.to_owned(),
            known: known.clone(),
        };
        match self.layout() {
            PairLayout::InlineArray => return Err(self.inline_pairs()),
            PairLayout::Implicit => {
                return Err(if name == DEFAULT_PAIR_NAME {
                    ConfigError::LastPair {
                        name: name.to_owned(),
                    }
                } else {
                    missing()
                });
            }
            PairLayout::Tables => {}
        }
        let mut next = self.cloned();
        let tables = next.pair_tables_mut();
        let index = tables
            .iter()
            .position(|table| name_of(table) == Some(name))
            .ok_or_else(missing)?;
        if tables.len() < 2 {
            return Err(ConfigError::LastPair {
                name: name.to_owned(),
            });
        }
        tables.remove(index);

        next.validate()?;
        let (before, after) = (self.to_toml_string(), next.to_toml_string());
        let (old, new) = (Meaning::of(&before)?, Meaning::of(&after)?);
        let mut expected = old.pairs;
        expected.remove(index);
        if (new.daemon, new.rest, new.pairs) != (old.daemon, old.rest, expected) {
            return Err(ConfigError::ChangedMeaning(
                "a pair other than the one removed changed, or a setting at the top level did"
                    .to_owned(),
            ));
        }
        let mut expected_views = views_of(&before)?;
        expected_views.remove(index);
        if views_of(&after)? != expected_views {
            return Err(ConfigError::ChangedMeaning(
                "a remaining folder pair's paths read differently after the removal".to_owned(),
            ));
        }
        Ok(next)
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
mod pair_admin_tests;

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
        doc.set_string_array("exclude", &["*.tmp".into(), "node_modules/".into()])
            .unwrap();
        doc.set_int("scan_interval_secs", 120).unwrap();

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
        //
        // `proton_cli` used to be caught here by a second, GUI-only copy of this exact rule
        // (`ConfigDoc::validate` read the top-level key itself and refused it directly). #356 moved
        // that refusal into the engine (`validate_file_config_text` daemon-wide, plus the merged
        // check in `validate_runtime_config`) and deleted the copy, so `proton_cli` now reaches this
        // assertion the same way `local_root`/`remote_root` always have: through the one call to
        // `validate_file_config_text` below. The message is unchanged either way, which is the point
        // — this test passing proves deleting the GUI-side copy changed nothing a caller can see.
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

    /// The blank rule, not an is-empty check: `"   "` carries no visible bytes but is not the empty
    /// string. `local_root`/`remote_root`/`proton_cli` above are only exercised with `""`; this pins
    /// the whitespace-only form specifically for `proton_cli` now that #356 routes it through the
    /// same engine call as the other two, rather than through the deleted GUI-only copy.
    #[test]
    fn a_whitespace_only_proton_cli_is_refused_the_same_as_an_empty_one() {
        let err = ConfigDoc::from_toml_str("proton_cli = \"   \"\n")
            .unwrap()
            .validate()
            .unwrap_err();
        assert!(
            err.to_string().contains("proton_cli must not be empty"),
            "got {err:?}"
        );
    }

    /// #341's sibling to the test above: `db_path`/`lockfile_path` have no root of their own, but a
    /// blank override joins onto `local_root` and silently becomes the sync root itself, which the
    /// GUI would then happily save. For these two keys `ConfigDoc::validate` is
    /// `validate_file_config_text` and nothing else, and since #356 that is true of `proton_cli` too
    /// — so this proves the refusal the engine added there is inherited rather than needing a second
    /// copy over here.
    #[test]
    fn an_empty_state_path_is_refused_before_it_reaches_the_daemon() {
        for key in ["db_path", "lockfile_path"] {
            let doc = ConfigDoc::from_toml_str(&format!(
                "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n{key} = \"   \"\n"
            ))
            .unwrap();
            let err = doc.validate().unwrap_err();
            assert!(
                err.to_string()
                    .contains(&format!("{key} must not be empty")),
                "got {err:?}"
            );
        }
        // ABSENT is not empty: the daemon fills an absent db_path/lockfile_path from the per-root
        // `.sync` default.
        ConfigDoc::from_toml_str("local_root = \"/a\"\nremote_root = \"/Drive/a\"\n")
            .unwrap()
            .validate()
            .expect("absent db_path/lockfile_path are the daemon's default, not a refusal");
    }

    /// #357's sibling to the test above, same reasoning: `.`/`..`/`foo/..` are not blank but reach
    /// the same directory-not-a-file failure, and this GUI-side call has no copy of that rule either
    /// — it is inherited from `validate_file_config_text` exactly the way the blank check is.
    #[test]
    fn a_degenerate_state_path_is_refused_before_it_reaches_the_daemon() {
        for key in ["db_path", "lockfile_path"] {
            for degenerate in [".", "..", "foo/..", "state/.", "state/", "~", "~/"] {
                let doc = ConfigDoc::from_toml_str(&format!(
                    "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n{key} = \"{degenerate}\"\n"
                ))
                .unwrap();
                let err = doc.validate().unwrap_err();
                assert!(
                    err.to_string().contains(&format!("{key} must name a file")),
                    "{key} = {degenerate:?}: got {err:?}"
                );
            }
            // Every one of these lexically resembles a degenerate spelling somewhere in it, but
            // still names a real file, and must still save.
            for benign in ["foo/../bar.db", "./index.db", ".hidden.db", "~/x.db"] {
                ConfigDoc::from_toml_str(&format!(
                    "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n{key} = \"{benign}\"\n"
                ))
                .unwrap()
                .validate()
                .unwrap_or_else(|e| panic!("{key} = {benign:?} names a real file: {e:?}"));
            }
        }
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
        doc.set_delete_approval("remote", false).unwrap();
        doc.set_delete_approval("local", true).unwrap();
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
            doc.set_local_delete_mode(mode).unwrap();
            assert_eq!(doc.get_local_delete_mode_key(), Some(mode));
            assert_eq!(doc.get_local_delete_mode(), mode);
            // THE HALF THAT MATTERS: `save` validates through the daemon's own parser, so a value
            // this screen can write and the daemon cannot start on is a config the GUI bricks.
            doc.validate()
                .expect("the daemon must accept what the Deletions tab writes");
        }
        assert!(
            doc.to_toml_string()
                .contains("local_delete_mode = \"permanent\"")
        );
    }

    #[test]
    fn the_kebab_spelling_of_the_mode_is_read_and_rewritten_in_place() {
        // `key_in_use` resolves either spelling, and a file written the kebab way must not gain a
        // second snake-cased key beside it — that is one setting written twice.
        let mut doc = ConfigDoc::from_toml_str("local-delete-mode = \"permanent\"\n").unwrap();
        assert_eq!(doc.get_local_delete_mode(), LocalDeleteMode::Permanent);
        doc.set_local_delete_mode(LocalDeleteMode::Trash).unwrap();
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
        let error = doc
            .validate()
            .expect_err("the daemon parser must refuse it");
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
            doc.set_deletion_policy(policy).unwrap();
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
            doc.set_deletion_policy(policy).unwrap();
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
        doc.set_deletion_policy(DeletionPolicy::OnlyPermanent)
            .unwrap();
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
        doc.set_deletion_policy(DeletionPolicy::AskEveryTime)
            .unwrap();
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
        doc.set_deletion_policy(DeletionPolicy::Never).unwrap();
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
        doc.set_bool("events_driven", false).unwrap();
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
        doc.set_str("log_level", "debug").unwrap();
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
        doc.set_deletion_policy(DeletionPolicy::Never).unwrap();
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
        doc.set_str("socket_path", "/run/user/1000/custom.sock")
            .unwrap();
        doc.set_str("log_level", "proton_drive_sync_engine=debug,warn")
            .unwrap();
        doc.set_str("conflict_suffix", "from-cloud").unwrap();
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

        doc.set_str("log_level", "warn").unwrap();
        doc.set_int("scan_interval_secs", 300).unwrap();
        doc.set_delete_approval("local", false).unwrap();
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
        doc.set_str("log_level", "warn").unwrap();
        doc.set_deletion_policy(DeletionPolicy::Never).unwrap();
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
        doc.set_str("log_level", "debug").unwrap();
        doc.save(&path).expect("save");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[[pair]]"), "got {written}");
        assert!(written.contains("name = \"documents\""), "got {written}");
        assert!(written.contains("log_level = \"debug\""), "got {written}");
        assert!(written.contains("# hand-written"), "comments preserved");
    }

    #[test]
    fn a_per_pair_key_of_a_pair_file_is_written_in_its_pairs_table_and_refused_at_the_top_level() {
        // WAS: `editing_a_per_pair_key_of_a_pair_file_is_refused_legibly_rather_than_split_in_two`,
        // the phase-1 boundary. This writer only knew top-level keys, so asking it to change a
        // per-pair setting of a `[[pair]]` document wrote a file that states one setting twice,
        // which the engine refused with an error naming both spellings. Phase 5b-1 is the
        // array-of-tables API that lifts it: the setting is written where it belongs, and writing it
        // at the top level is refused by the key's scope BEFORE it can become that file.
        let mut doc = ConfigDoc::from_toml_str(
            "[[pair]]\nname = \"documents\"\nlocal_root = \"/home/me/Documents\"\n\
             remote_root = \"/Drive/Docs\"\n",
        )
        .unwrap();

        let error = doc.set_str("local_root", "/home/me/Elsewhere").unwrap_err();
        assert!(
            matches!(
                &error,
                ConfigError::WrongScope { key, scope: KeyScope::Pair } if key == "local_root"
            ),
            "got {error:?}"
        );
        assert!(
            error.to_string().contains("per-pair") && error.to_string().contains("`local_root`"),
            "got {error}"
        );
        assert!(
            !doc.to_toml_string().contains("Elsewhere"),
            "a refused write changes nothing"
        );

        doc.pair_mut("documents")
            .unwrap()
            .set_str("local_root", "/home/me/Elsewhere")
            .unwrap();
        doc.validate()
            .expect("the edit left a config the daemon starts on");
        assert_eq!(
            doc.pair("documents")
                .unwrap()
                .get_str("local_root")
                .as_deref(),
            Some("/home/me/Elsewhere")
        );
        assert_eq!(doc.to_toml_string().matches("local_root").count(), 1);

        // The engine's own rule is still what stands behind this one: a document that somebody
        // already wrote with both spellings is refused by `validate`, in the engine's words.
        let both = ConfigDoc::from_toml_str(
            "local_root = \"/x\"\n[[pair]]\nname = \"a\"\nremote_root = \"/Drive/a\"\n",
        )
        .unwrap();
        let message = both.validate().unwrap_err().to_string();
        assert!(
            message.contains("two spellings of one setting") && message.contains("`local_root`"),
            "got {message}"
        );
    }

    /// A two-pair file with comments in every table, daemon-wide keys at the top and one value that
    /// carries a trailing comment: every shape a save could disturb.
    const TWO_PAIRS: &str = "# hand-written\n\
        log_level = \"info\"   # daemon-wide\n\
        proton_cli = \"proton-drive\"\n\
        \n\
        # the first pair\n\
        [[pair]]\n\
        name = \"documents\"\n\
        local_root = \"/home/me/Documents\"   # docs root\n\
        remote_root = \"/Drive/Docs\"\n\
        exclude = [\"*.tmp\"]\n\
        \n\
        # the second pair\n\
        [[pair]]\n\
        name = \"photos\"\n\
        local_root = \"/home/me/Pictures\"\n\
        remote_root = \"/Drive/Photos\"\n\
        scan_interval_secs = 600\n";

    /// The lines that differ between two renderings, as `(before, after)`. Panics unless both have
    /// the same number of lines, which is how an edit that ADDS a line is told apart from one that
    /// changes a line.
    fn changed_lines(before: &str, after: &str) -> Vec<(String, String)> {
        let (before, after): (Vec<_>, Vec<_>) = (before.lines().collect(), after.lines().collect());
        assert_eq!(before.len(), after.len(), "line count moved:\n{after:?}");
        before
            .iter()
            .zip(&after)
            .filter(|(b, a)| b != a)
            .map(|(b, a)| ((*b).to_owned(), (*a).to_owned()))
            .collect()
    }

    #[test]
    fn a_two_pair_file_saves_daemon_wide_edits_and_each_pairs_own() {
        // WAS: `a_two_pair_file_saves_daemon_wide_edits_and_refuses_per_pair_ones`. Phase 4c let the
        // daemon run two pairs, which this writer inherited as a document it could SAVE; what it could
        // not do was edit a per-pair key. It can now, and what it must still refuse is the write that
        // names no pair's table.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proton-sync.toml");
        std::fs::write(&path, TWO_PAIRS).unwrap();

        // A daemon-wide edit saves, and leaves both tables and the comments alone.
        let mut doc = ConfigDoc::load(&path).unwrap();
        doc.validate().expect("a two-pair document is valid");
        doc.set_str("log_level", "debug").unwrap();
        doc.save(&path).expect("a daemon-wide edit saves");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("log_level = \"debug\""), "got {written}");
        assert_eq!(written.matches("[[pair]]").count(), 2, "got {written}");
        assert!(written.contains("# hand-written"), "comments preserved");

        // A per-pair edit of ONE pair saves, and the other pair's table is the same bytes.
        let before = std::fs::read_to_string(&path).unwrap();
        let mut doc = ConfigDoc::load(&path).unwrap();
        doc.pair_mut("photos")
            .unwrap()
            .set_str("local_root", "/home/me/Elsewhere")
            .unwrap();
        doc.save(&path).expect("a per-pair edit saves");
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            changed_lines(&before, &after),
            vec![(
                "local_root = \"/home/me/Pictures\"".to_owned(),
                "local_root = \"/home/me/Elsewhere\"".to_owned()
            )],
            "a save of one key changes one line"
        );
        let table_of = |text: &str, name: &str| {
            text.split("[[pair]]")
                .find(|table| table.contains(&format!("name = \"{name}\"")))
                .map(str::to_owned)
        };
        assert_eq!(
            table_of(&before, "documents"),
            table_of(&after, "documents"),
            "the pair that was not named is byte-identical"
        );

        // A per-pair edit that names no pair's table is refused, and nothing is written.
        let before = std::fs::read_to_string(&path).unwrap();
        let mut doc = ConfigDoc::load(&path).unwrap();
        let error = doc.set_str("local_root", "/home/me/Nowhere").unwrap_err();
        assert!(
            matches!(error, ConfigError::WrongScope { .. }),
            "got {error:?}"
        );
        doc.save(&path)
            .expect("nothing was changed, so the save is the file as it was");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_save_changes_only_what_it_was_asked_to_change_across_two_pairs() {
        // `a_save_changes_only_what_it_was_asked_to_change`, with a second table beside the first. The
        // new way for a save to be wrong is to be RIGHT about the key and WRONG about the table.
        struct Edit {
            name: &'static str,
            apply: fn(&mut ConfigDoc),
            was: &'static str,
            now: &'static str,
        }
        let edits = [
            Edit {
                name: "photos' interval",
                apply: |doc| {
                    doc.pair_mut("photos")
                        .unwrap()
                        .set_int("scan_interval_secs", 900)
                        .unwrap();
                },
                was: "scan_interval_secs = 600",
                now: "scan_interval_secs = 900",
            },
            Edit {
                name: "documents' skip rules",
                apply: |doc| {
                    doc.pair_mut("documents")
                        .unwrap()
                        .set_string_array("exclude", &["*.tmp".to_owned(), "build/".to_owned()])
                        .unwrap();
                },
                was: "exclude = [\"*.tmp\"]",
                now: "exclude = [\"*.tmp\", \"build/\"]",
            },
            Edit {
                name: "documents' root keeps its trailing comment out of the way",
                apply: |doc| {
                    doc.pair_mut("documents")
                        .unwrap()
                        .set_str("local_root", "/home/me/Docs2")
                        .unwrap();
                },
                was: "local_root = \"/home/me/Documents\"   # docs root",
                now: "local_root = \"/home/me/Docs2\"",
            },
            Edit {
                name: "the daemon-wide log level",
                apply: |doc| {
                    doc.set_str("log_level", "debug").unwrap();
                },
                was: "log_level = \"info\"   # daemon-wide",
                now: "log_level = \"debug\"",
            },
        ];
        for Edit {
            name,
            apply,
            was,
            now,
        } in edits
        {
            let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
            apply(&mut doc);
            let after = doc.to_toml_string();
            assert_eq!(
                changed_lines(TWO_PAIRS, &after),
                vec![(was.to_owned(), now.to_owned())],
                "{name}: one line, and the one asked for"
            );
            doc.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        }

        // A key the table did not have is ADDED to the table named, and to no other.
        let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
        doc.pair_mut("documents")
            .unwrap()
            .set_bool("events_driven", false)
            .unwrap();
        let after = doc.to_toml_string();
        let (documents, photos) = after.split_once("# the second pair").unwrap();
        assert!(documents.contains("events_driven = false"), "{after}");
        assert!(!photos.contains("events_driven"), "{after}");
        assert_eq!(after.matches("events_driven").count(), 1, "{after}");
    }

    #[test]
    fn the_layout_and_the_pair_names_are_read_from_the_document() {
        for (text, layout, names) in [
            ("", PairLayout::Implicit, vec!["default"]),
            (
                "local_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
                PairLayout::Implicit,
                vec!["default"],
            ),
            (TWO_PAIRS, PairLayout::Tables, vec!["documents", "photos"]),
            (
                "pair = [{ name = \"a\", local_root = \"/a\" }, { name = \"b\" }]\n",
                PairLayout::InlineArray,
                vec!["a", "b"],
            ),
            // An explicit empty list is a statement, not an implicit pair (the engine refuses it).
            ("pair = []\n", PairLayout::InlineArray, vec![]),
        ] {
            let doc = ConfigDoc::from_toml_str(text).unwrap();
            assert_eq!(doc.layout(), layout, "{text:?}");
            assert_eq!(doc.pair_names(), names, "{text:?}");
            assert_eq!(doc.declares_pair_tables(), layout != PairLayout::Implicit);
        }
    }

    #[test]
    fn a_pair_is_found_by_its_exact_name_and_the_implicit_one_is_the_top_level() {
        let doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
        assert_eq!(
            doc.pair("photos").unwrap().get_int("scan_interval_secs"),
            Some(600)
        );
        assert_eq!(
            doc.pair("documents").unwrap().get_int("scan_interval_secs"),
            None,
            "a pair's table is its own, and does not read the other's"
        );
        assert!(
            doc.pair("Photos").is_none(),
            "names are matched byte-exactly"
        );
        assert!(
            doc.pair("default").is_none(),
            "a tables file has no implicit pair"
        );
        // The top level of a tables file holds the daemon-wide keys and no pair's.
        assert_eq!(doc.get_str("log_level").as_deref(), Some("info"));
        assert_eq!(doc.get_str("local_root"), None);

        // THE IMPLICIT PAIR IS THE ROOT TABLE: one code path for N=1 and N>=2 (ADR 0005 §2).
        let mut doc = ConfigDoc::from_toml_str(SAMPLE).unwrap();
        assert_eq!(
            doc.pair(DEFAULT_PAIR_NAME).unwrap().get_str("local_root"),
            doc.get_str("local_root")
        );
        assert!(doc.pair("photos").is_none());
        doc.pair_mut(DEFAULT_PAIR_NAME)
            .unwrap()
            .set_string_array("exclude", &["x".to_owned()])
            .unwrap();
        assert_eq!(doc.get_string_array("exclude"), vec!["x".to_owned()]);

        let missing = doc
            .pair_mut("photos")
            .err()
            .expect("an unknown pair is refused");
        assert!(
            matches!(&missing, ConfigError::NoSuchPair { name, known } if name == "photos" && known == &["default".to_owned()]),
            "got {missing:?}"
        );
        assert!(missing.to_string().contains("\"photos\""), "{missing}");
    }

    #[test]
    fn a_daemon_wide_key_is_refused_inside_a_pair_table_and_a_pair_name_is_not_a_setting() {
        let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
        for key in ["log_level", "log-level", "socket_path", "proton_cli"] {
            let error = doc
                .pair_mut("photos")
                .unwrap()
                .set_str(key, "x")
                .unwrap_err();
            assert!(
                matches!(
                    &error,
                    ConfigError::WrongScope {
                        scope: KeyScope::Daemon,
                        ..
                    }
                ),
                "{key}: {error:?}"
            );
            assert!(error.to_string().contains("daemon-wide"), "{error}");
        }
        // `name` is a wire selector, a `gui.toml` key and a tray id: the editor never renames.
        let error = doc
            .pair_mut("photos")
            .unwrap()
            .set_str("name", "pictures")
            .unwrap_err();
        assert!(matches!(error, ConfigError::Invalid(_)), "got {error:?}");
        assert_eq!(doc.pair_names(), ["documents", "photos"]);
        assert_eq!(
            doc.to_toml_string(),
            TWO_PAIRS,
            "no refused write changed a byte"
        );
    }

    /// The routing guard: iterate the ENGINE's keys, and each must be writable through exactly one of
    /// the top level of a `[[pair]]` file and a pair's own table. A key the engine adds is a key this
    /// test routes with no list kept here; one that routes both ways, or neither, is a user's save
    /// landing in the wrong place.
    #[test]
    fn every_config_key_routes_to_exactly_one_table() {
        for key in ConfigKey::ALL {
            for spelling in key.spellings() {
                let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
                let at_root = doc.set_str(spelling, "v");
                let in_pair = doc.pair_mut("photos").unwrap().set_str(spelling, "v");
                assert_eq!(
                    (at_root.is_ok(), in_pair.is_ok()),
                    (
                        key.scope() == KeyScope::Daemon,
                        key.scope() == KeyScope::Pair
                    ),
                    "`{spelling}` ({:?}) must be writable through exactly one table: root {at_root:?}, \
                     pair {in_pair:?}",
                    key.scope()
                );
                // The routed writer a save uses puts it in the same place, byte for byte.
                let mut routed = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
                routed
                    .writer_for("photos")
                    .set_str(spelling, "v")
                    .unwrap_or_else(|e| panic!("`{spelling}` through the routed writer: {e}"));
                assert_eq!(
                    routed.to_toml_string(),
                    doc.to_toml_string(),
                    "`{spelling}`: the routed writer and the table writer disagree about where it goes"
                );
                // And where it landed is where the engine says it belongs.
                let rendered: toml::Table = toml::from_str(&doc.to_toml_string()).unwrap();
                let original: toml::Table = toml::from_str(TWO_PAIRS).unwrap();
                let (pairs, was) = (
                    rendered["pair"].as_array().unwrap(),
                    original["pair"].as_array().unwrap(),
                );
                // The pair that was not named is the table it was, whatever the key.
                assert_eq!(pairs[0], was[0], "`{spelling}` moved the first pair");
                // Under whichever spelling the table already used: a write goes where the key is.
                let holds = |table: &dyn Fn(&str) -> Option<String>| {
                    key.spellings()
                        .iter()
                        .filter(|s| table(s).as_deref() == Some("v"))
                        .count()
                };
                let in_root = |s: &str| rendered.get(s).and_then(|v| v.as_str()).map(str::to_owned);
                let in_second =
                    |s: &str| pairs[1].get(s).and_then(|v| v.as_str()).map(str::to_owned);
                match key.scope() {
                    KeyScope::Daemon => {
                        assert_eq!(pairs[1], was[1], "`{spelling}` moved a pair");
                        assert_eq!(holds(&in_root), 1, "`{spelling}` at the top level, once");
                    }
                    KeyScope::Pair => {
                        assert_eq!(holds(&in_root), 0, "`{spelling}` stays out of the root");
                        assert_eq!(holds(&in_second), 1, "`{spelling}` in the named pair, once");
                    }
                }
            }
        }
    }

    #[test]
    fn a_routed_save_writes_in_the_order_it_was_asked_and_refuses_what_nobody_classified() {
        // A key a file did not have is APPENDED where it is written, so the order of a save's writes
        // is part of what the save produces. An implicit file takes both scopes at the top level in
        // exactly the order given.
        let mut doc = ConfigDoc::from_toml_str("").unwrap();
        {
            let mut writer = doc.writer_for(DEFAULT_PAIR_NAME);
            writer.set_str("proton_cli", "/bin/proton-drive").unwrap();
            writer.set_str("local_root", "/home/u/Drive").unwrap();
            writer.set_int("scan_interval_secs", 60).unwrap();
            writer.set_str("log_level", "debug").unwrap();
        }
        assert_eq!(
            doc.to_toml_string(),
            "proton_cli = \"/bin/proton-drive\"\nlocal_root = \"/home/u/Drive\"\n\
             scan_interval_secs = 60\nlog_level = \"debug\"\n"
        );

        // Over tables the same sequence splits: daemon-wide keys at the top, per-pair in the table.
        let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
        {
            let mut writer = doc.writer_for("photos");
            writer.set_int("scan_interval_secs", 30).unwrap();
            writer.set_str("log_level", "warn").unwrap();
            let unknown = writer.set_str("frobnicate", "x").unwrap_err();
            assert!(
                matches!(unknown, ConfigError::Invalid(_)),
                "got {unknown:?}"
            );
            let nobody = doc_without_pair_error();
            assert!(
                matches!(nobody, ConfigError::NoSuchPair { .. }),
                "got {nobody:?}"
            );
        }
        assert_eq!(
            changed_lines(TWO_PAIRS, &doc.to_toml_string()),
            vec![
                (
                    "log_level = \"info\"   # daemon-wide".to_owned(),
                    "log_level = \"warn\"".to_owned()
                ),
                (
                    "scan_interval_secs = 600".to_owned(),
                    "scan_interval_secs = 30".to_owned()
                ),
            ]
        );

        fn doc_without_pair_error() -> ConfigError {
            let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
            doc.writer_for("nobody")
                .set_int("scan_interval_secs", 1)
                .unwrap_err()
        }
    }

    #[test]
    fn an_implicit_file_takes_every_key_at_the_top_level_as_it_always_did() {
        // The N=1 half of the routing: the top level IS the pair, so both scopes write there.
        for key in ConfigKey::ALL {
            let mut doc = ConfigDoc::from_toml_str("").unwrap();
            doc.set_str(key.spelling(), "v")
                .unwrap_or_else(|e| panic!("{}: {e}", key.spelling()));
            assert_eq!(doc.get_str(key.spelling()).as_deref(), Some("v"));
        }
    }

    /// The same value under every spelling the parser accepts for every key, at the top level and in a
    /// pair table: read in any, written back in the one already there (F-D, and the kebab rule).
    #[test]
    fn every_accepted_spelling_is_read_and_written_in_kind() {
        for key in ConfigKey::ALL {
            for present in key.spellings() {
                for asked in key.spellings() {
                    // ROOT
                    let mut doc =
                        ConfigDoc::from_toml_str(&format!("{present} = \"old\"\n")).unwrap();
                    assert_eq!(
                        doc.get_str(asked).as_deref(),
                        Some("old"),
                        "`{asked}` must find the value a file spells `{present}`"
                    );
                    doc.set_str(asked, "new").unwrap();
                    assert_eq!(
                        doc.to_toml_string(),
                        format!("{present} = \"new\"\n"),
                        "writing `{asked}` over a file that spells `{present}` must rewrite that line, \
                         not add a second spelling"
                    );
                    doc.remove(asked).unwrap();
                    assert_eq!(
                        doc.to_toml_string(),
                        "",
                        "removing `{asked}` removes `{present}`"
                    );

                    // A PAIR TABLE, the same rule from the same code.
                    if key.scope() == KeyScope::Pair {
                        let mut doc = ConfigDoc::from_toml_str(&format!(
                            "[[pair]]\nname = \"a\"\n{present} = \"old\"\n"
                        ))
                        .unwrap();
                        assert_eq!(
                            doc.pair("a").unwrap().get_str(asked).as_deref(),
                            Some("old"),
                            "in a pair table: `{asked}` vs `{present}`"
                        );
                        doc.pair_mut("a").unwrap().set_str(asked, "new").unwrap();
                        assert_eq!(
                            doc.to_toml_string(),
                            format!("[[pair]]\nname = \"a\"\n{present} = \"new\"\n"),
                            "in a pair table: writing `{asked}` over `{present}`"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_file_that_spells_the_glob_lists_in_full_reads_and_writes_them_in_kind() {
        // F-D, the user-visible half. `include_patterns` is a key the parser accepts and the GUI used
        // to know neither how to read (an empty skip list was drawn over a live one) nor how to write
        // beside (the next save added `exclude`, which serde rejects as a duplicate field).
        let mut doc = ConfigDoc::from_toml_str(
            "local_root = \"/x\"\nexclude_patterns = [\"*.tmp\"]\ninclude_patterns = [\"a/**\"]\n",
        )
        .unwrap();
        assert_eq!(doc.get_string_array("exclude"), vec!["*.tmp".to_owned()]);
        assert_eq!(doc.get_string_array("include"), vec!["a/**".to_owned()]);
        doc.set_string_array("exclude", &["*.tmp".to_owned(), "b/".to_owned()])
            .unwrap();
        let text = doc.to_toml_string();
        assert!(
            text.contains("exclude_patterns = [\"*.tmp\", \"b/\"]"),
            "{text}"
        );
        assert!(!text.contains("exclude ="), "no second spelling:\n{text}");
        doc.validate()
            .expect("the daemon starts on what the save wrote");

        // And a file that spells neither gets the bare key this app has always written.
        let mut doc = ConfigDoc::from_toml_str("local_root = \"/x\"\n").unwrap();
        doc.set_string_array("exclude", &["*.tmp".to_owned()])
            .unwrap();
        assert!(doc.to_toml_string().contains("exclude = [\"*.tmp\"]"));
    }

    #[test]
    fn an_inline_pair_array_is_read_and_never_edited() {
        let text = "# inline\nlog_level = \"info\"\n\
                    pair = [{ name = \"a\", local_root = \"/a\", remote_root = \"/Drive/a\", \
                    exclude = [\"*.tmp\"], delete_approval = { remote = false } }, \
                    { name = \"b\", local_root = \"/b\", remote_root = \"/Drive/b\" }]\n";
        let path_dir = tempfile::tempdir().unwrap();
        let path = path_dir.path().join("proton-sync.toml");
        std::fs::write(&path, text).unwrap();
        let mut doc = ConfigDoc::load(&path).unwrap();

        // READ: the layout, the names, and every getter, through the same code as a table.
        assert_eq!(doc.layout(), PairLayout::InlineArray);
        assert_eq!(doc.pair_names(), ["a", "b"]);
        let a = doc.pair("a").unwrap();
        assert_eq!(a.get_str("local_root").as_deref(), Some("/a"));
        assert_eq!(a.get_string_array("exclude"), vec!["*.tmp".to_owned()]);
        assert_eq!(a.get_deletion_policy(), DeletionPolicy::OnlyPermanent);
        assert_eq!(
            doc.pair("b").unwrap().get_str("remote_root").as_deref(),
            Some("/Drive/b")
        );
        assert!(doc.pair("c").is_none());
        doc.validate().expect("the daemon reads an inline array");

        // EDIT: refused, whatever the name, with the sentence that names the file.
        for name in ["a", "b", "c"] {
            let error = doc
                .pair_mut(name)
                .err()
                .expect("an inline array is not edited");
            assert!(
                matches!(error, ConfigError::InlinePairs { .. }),
                "{name}: {error:?}"
            );
        }
        let error = doc.set_str("local_root", "/z").unwrap_err();
        assert!(
            matches!(error, ConfigError::InlinePairs { .. }),
            "got {error:?}"
        );
        let sentence = error.to_string();
        assert!(sentence.contains(&path.display().to_string()), "{sentence}");
        assert!(sentence.contains("inline array"), "{sentence}");
        assert!(sentence.contains("`[[pair]]` tables"), "{sentence}");

        // A daemon-wide key is not in the way: the top level is still the daemon's.
        doc.set_str("log_level", "debug").unwrap();
        assert_eq!(
            changed_lines(text, &doc.to_toml_string()),
            vec![(
                "log_level = \"info\"".to_owned(),
                "log_level = \"debug\"".to_owned()
            )]
        );
        // And an in-memory document names no file, which the sentence does not pretend otherwise.
        let mut loose = ConfigDoc::from_toml_str(text).unwrap();
        let sentence = loose.pair_mut("a").err().unwrap().to_string();
        assert!(sentence.starts_with("this config writes"), "{sentence}");
    }

    #[test]
    fn a_policy_per_pair_is_written_in_the_spelling_that_pairs_table_already_uses() {
        // `set_deletion_policy` keeps its round-trip rule PER TABLE. Two pairs, two spellings.
        let mut doc = ConfigDoc::from_toml_str(
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             deletion_policy = \"never\"\n\n\
             [[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\n\
             [pair.delete_approval]\nremote = true\nlocal = true\n",
        )
        .unwrap();
        doc.validate().expect("two spellings in two tables is fine");
        assert_eq!(
            doc.pair("a").unwrap().get_deletion_policy(),
            DeletionPolicy::Never
        );
        assert_eq!(
            doc.pair("b").unwrap().get_deletion_policy(),
            DeletionPolicy::AskEveryTime
        );

        doc.pair_mut("a")
            .unwrap()
            .set_deletion_policy(DeletionPolicy::OnlyPermanent)
            .unwrap();
        doc.pair_mut("b")
            .unwrap()
            .set_deletion_policy(DeletionPolicy::Never)
            .unwrap();
        doc.validate().expect("each pair kept its own spelling");
        let reread = ConfigDoc::from_toml_str(&doc.to_toml_string()).unwrap();
        assert_eq!(
            reread
                .pair("a")
                .unwrap()
                .get_str("deletion_policy")
                .as_deref(),
            Some("only_permanent")
        );
        assert_eq!(
            reread.pair("a").unwrap().get_delete_approval("remote"),
            None
        );
        assert_eq!(
            reread.pair("b").unwrap().get_delete_approval("remote"),
            Some(false)
        );
        assert_eq!(
            reread.pair("b").unwrap().get_delete_approval("local"),
            Some(false)
        );
        assert_eq!(reread.pair("b").unwrap().get_str("deletion_policy"), None);
    }

    #[test]
    fn a_delete_approval_table_made_for_the_first_pair_lands_under_the_first_pair() {
        // `toml_edit` renders a table by position, and a table made later has none: the risk is a
        // `[pair.delete_approval]` that attaches to the NEXT `[[pair]]` header. It must not.
        let mut doc = ConfigDoc::from_toml_str(TWO_PAIRS).unwrap();
        doc.pair_mut("documents")
            .unwrap()
            .set_delete_approval("remote", false)
            .unwrap();
        let text = doc.to_toml_string();
        let (documents, photos) = text.split_once("# the second pair").unwrap();
        assert!(documents.contains("[pair.delete_approval]"), "{text}");
        assert!(!photos.contains("delete_approval"), "{text}");
        let reread = ConfigDoc::from_toml_str(&text).unwrap();
        assert_eq!(
            reread
                .pair("documents")
                .unwrap()
                .get_delete_approval("remote"),
            Some(false)
        );
        assert_eq!(
            reread.pair("photos").unwrap().get_delete_approval("remote"),
            None
        );
        reread.validate().expect("daemon parser");
    }

    #[test]
    fn an_inline_delete_approval_table_is_read_and_edited_in_place() {
        // The daemon reads `delete_approval = { remote = false }` as it reads the table. This reader
        // used to see only a real table: it drew "ask about every deletion" over a policy of never
        // asking, and on a write REPLACED the inline table with an empty one, dropping the direction
        // nobody touched. Now one table-like reader serves a pair table and an inline pair alike.
        let mut doc = ConfigDoc::from_toml_str(
            "local_root = \"/x\"\ndelete_approval = { remote = false, local = false }\n",
        )
        .unwrap();
        assert_eq!(doc.get_delete_approval("remote"), Some(false));
        assert_eq!(doc.get_deletion_policy(), DeletionPolicy::Never);
        doc.set_delete_approval("remote", true).unwrap();
        assert_eq!(
            doc.get_delete_approval("local"),
            Some(false),
            "the untouched direction survives"
        );
        assert_eq!(doc.get_delete_approval("remote"), Some(true));
        doc.validate().expect("daemon parser");
    }

    #[test]
    fn the_refusals_say_what_is_wrong_and_what_to_do() {
        let wrong_table = ConfigError::WrongScope {
            key: "local_root".to_owned(),
            scope: KeyScope::Pair,
        };
        assert!(
            wrong_table
                .to_string()
                .contains("inside the pair's own table")
        );
        let wrong_table = ConfigError::WrongScope {
            key: "log_level".to_owned(),
            scope: KeyScope::Daemon,
        };
        assert!(wrong_table.to_string().contains("top level"));
        let none = ConfigError::NoSuchPair {
            name: "x".to_owned(),
            known: Vec::new(),
        };
        assert!(none.to_string().contains("declares none"), "{none}");
        let some = ConfigError::NoSuchPair {
            name: "x".to_owned(),
            known: vec!["a".to_owned(), "b".to_owned()],
        };
        assert!(some.to_string().contains("\"a\", \"b\""), "{some}");
    }

    /// The N=1 identity corpus (#102 phase 5b-1): the top-level reads and writes, run against the
    /// output **main's** `ConfigDoc` produced before the table-generic refactor. A refactor of the one
    /// writer every Settings save goes through is "no change for one pair" only if the bytes say so,
    /// and a `contains` assertion does not: this compares whole renderings and every getter.
    ///
    /// `testdata/n1-config-corpus.json` is data, not a snapshot of this implementation — it was
    /// recorded from the commit before the refactor and is never regenerated from the code under test.
    mod n1_identity {
        use super::*;
        use serde_json::{Value, json};

        fn read(doc: &ConfigDoc, op: &[Value]) -> Value {
            let key = op.get(1).and_then(Value::as_str).unwrap_or("");
            match op[0].as_str().unwrap() {
                "get_str" => json!(doc.get_str(key)),
                "get_int" => json!(doc.get_int(key)),
                "get_bool" => json!(doc.get_bool(key)),
                "get_string_array" => json!(doc.get_string_array(key)),
                "get_delete_approval" => json!(doc.get_delete_approval(key)),
                "get_deletion_policy" => json!(doc.get_deletion_policy().as_str()),
                "get_deletion_policy_key" => {
                    json!(doc.get_deletion_policy_key().map(|p| p.as_str()))
                }
                "get_local_delete_mode" => json!(doc.get_local_delete_mode().as_str()),
                "get_local_delete_mode_key" => {
                    json!(doc.get_local_delete_mode_key().map(|p| p.as_str()))
                }
                other => panic!("unknown read {other}"),
            }
        }

        fn apply(doc: &mut ConfigDoc, op: &[Value]) {
            let key = op.get(1).and_then(Value::as_str).unwrap_or("");
            match op[0].as_str().unwrap() {
                "set_str" => doc.set_str(key, op[2].as_str().unwrap()).unwrap(),
                "set_int" => doc.set_int(key, op[2].as_i64().unwrap()).unwrap(),
                "set_bool" => doc.set_bool(key, op[2].as_bool().unwrap()).unwrap(),
                "set_string_array" => {
                    let items: Vec<String> = op[2]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect();
                    doc.set_string_array(key, &items).unwrap()
                }
                "remove" => doc.remove(key).unwrap(),
                "set_delete_approval" => doc
                    .set_delete_approval(key, op[2].as_bool().unwrap())
                    .unwrap(),
                "set_deletion_policy" => doc
                    .set_deletion_policy(op[1].as_str().unwrap().parse::<DeletionPolicy>().unwrap())
                    .unwrap(),
                "set_local_delete_mode" => doc
                    .set_local_delete_mode(
                        op[1].as_str().unwrap().parse::<LocalDeleteMode>().unwrap(),
                    )
                    .unwrap(),
                other => panic!("unknown op {other}"),
            }
        }

        #[test]
        fn one_pair_settings_edits_are_byte_identical_to_mains() {
            let corpus: Value =
                serde_json::from_str(include_str!("../testdata/n1-config-corpus.json")).unwrap();
            let reads = corpus["reads"].as_array().unwrap();
            let cases = corpus["cases"].as_array().unwrap();
            assert!(cases.len() >= 30, "the corpus lost cases: {}", cases.len());
            for case in cases {
                let name = case["name"].as_str().unwrap();
                let mut doc = ConfigDoc::from_toml_str(case["input"].as_str().unwrap()).unwrap();
                let before: Vec<Value> = reads
                    .iter()
                    .map(|r| read(&doc, r.as_array().unwrap()))
                    .collect();
                assert_eq!(
                    json!(before),
                    case["expected"]["before"],
                    "{name}: reads before any edit"
                );
                for op in case["ops"].as_array().unwrap() {
                    apply(&mut doc, op.as_array().unwrap());
                }
                assert_eq!(
                    doc.to_toml_string(),
                    case["expected"]["rendered"].as_str().unwrap(),
                    "{name}: the rendering moved"
                );
                let after: Vec<Value> = reads
                    .iter()
                    .map(|r| read(&doc, r.as_array().unwrap()))
                    .collect();
                assert_eq!(
                    json!(after),
                    case["expected"]["after"],
                    "{name}: reads after the edits"
                );
            }
        }
    }

    #[test]
    fn a_document_the_daemon_would_refuse_to_start_on_is_still_refused() {
        // The never-brick contract has to hold for the new shape too, or the GUI becomes the way to
        // write a config that stops the daemon. Two pairs used to be refused here because the
        // daemon could not run them; since phase 4c it can, so a well-formed two-pair document is
        // valid (see `a_two_pair_file_saves_daemon_wide_edits_and_each_pairs_own`) and the
        // first two rows are the rules that only apply BESIDE other pairs.
        for (toml, needle) in [
            (
                "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\n\
                 [[pair]]\nname = \"b\"\nlocal_root = \"/b\"\n",
                "every `[[pair]]` table must set both",
            ),
            (
                "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\n\
                 [[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
                 dry_run = true\n",
                "`dry_run = true`",
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

    /// The file's SHAPE, which the pair list cannot show: an implicit pair and a table named
    /// `default` read identically through `pair_views`, but the daemon's binary is addressed
    /// differently for the two.
    #[test]
    fn a_document_says_whether_it_declares_pair_tables() {
        for (text, tables) in [
            ("", false),
            ("local_root = \"/x\"\nremote_root = \"/Drive/x\"\n", false),
            (
                "[[pair]]\nname = \"default\"\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
                true,
            ),
            // The inline spelling is the same list, and just as much "not the implicit pair".
            ("pair = [{ name = \"a\", local_root = \"/x\" }]\n", true),
        ] {
            let doc = ConfigDoc::from_toml_str(text).unwrap();
            assert_eq!(doc.declares_pair_tables(), tables, "{text:?}");
        }
        // And the engine's reading of the two shapes is the same single pair, which is exactly why
        // the question needs its own answer.
        let implicit = pair_views("local_root = \"/x\"\nremote_root = \"/Drive/x\"\n").unwrap();
        let table = pair_views(
            "[[pair]]\nname = \"default\"\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
        )
        .unwrap();
        assert_eq!(implicit, table);
    }
}
