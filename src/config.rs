use crate::daemon::{
    DEFAULT_WARM_START_FULL_WALK_EVERY, DEFAULT_WARM_START_MAX_CURSOR_AGE_SECS, DaemonConfig,
    WarmStartConfig,
};
use crate::index::ScanOptions;
use crate::trash::LocalDeleteMode;
use crate::paths::{
    default_global_lock_path, default_lockfile_path, default_socket_path, default_state_db_path,
};
use crate::proton::CommandPolicy;
use crate::sync::{ConflictNaming, validate_conflict_suffix};
use crate::{AppResult, boxed_error};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

/// Default number of incremental event-driven passes between forced full-tree resyncs when
/// `events_driven` is on and no explicit value is configured. `0` disables the periodic safety
/// resync entirely, so after the first (startup) full-tree snapshot the daemon stays purely
/// event-driven until it is restarted or the event stream forces a fallback. The periodic resync
/// is opt-in: set a positive value to reinstate a self-healing full walk every N passes.
const DEFAULT_EVENTS_FULL_SCAN_EVERY: u64 = 0;

/// Default maximum number of planned downloads bundled into one `proton-drive filesystem
/// download` invocation. Large enough to amortize the CLI's per-spawn startup cost across a
/// bulk download, small enough that every ~25 files a checkpoint commits and a failure loses
/// at most one chunk of progress. `1` disables batching (one subprocess per file).
const DEFAULT_DOWNLOAD_BATCH_SIZE: usize = 25;

/// The verbosity used when nothing configures one. Matches the historical `init_tracing` fallback.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// The delete-approval guard expressed as one coarse setting — the `deletion_policy` key (#194).
///
/// Not a second mechanism: it is a **spelling** of the two `[delete_approval]` booleans the guard
/// has always run on, and resolves to exactly that pair. `remote` gates the *recoverable*
/// direction (a file leaving this computer lands in Proton's Trash and can be pulled back);
/// `local` gates the *permanent* one (a file removed from disk is gone). Both layers that carry
/// the guard — this daemon-wide config and the per-directory `.proton-sync.toml`
/// ([`crate::dirconfig`]) — accept either spelling, and **refuse a file that uses both**, because
/// two spellings of one setting in one file have no defensible precedence.
///
/// Four combinations, three of which the Settings screen draws.
/// [`Self::OnlyRecoverable`] has no control: it exists so a hand-written config is *named* rather
/// than rounded to the nearest card and silently rewritten on the next save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionPolicy {
    /// Every deletion waits for a person. The daemon default, and what an empty config means.
    AskEveryTime,
    /// Recoverable deletions go through; permanent ones wait.
    OnlyPermanent,
    /// Nothing waits.
    Never,
    /// Permanent deletions go through; recoverable ones wait. Undrawn.
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

    /// The `(remote, local)` pair this policy resolves to. Inverse of [`Self::from_directions`].
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

    /// The TOML/CLI spelling. Kept in step with the serde rename by
    /// `every_policy_spelling_round_trips_through_serde_and_from_str`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskEveryTime => "ask_every_time",
            Self::OnlyPermanent => "only_permanent",
            Self::Never => "never",
            Self::OnlyRecoverable => "only_recoverable",
        }
    }

    /// Every policy, for exhaustive tests and for naming the choices in an error message.
    pub const ALL: [Self; 4] = [
        Self::AskEveryTime,
        Self::OnlyPermanent,
        Self::Never,
        Self::OnlyRecoverable,
    ];
}

impl fmt::Display for DeletionPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DeletionPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|policy| policy.as_str() == value)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|policy| policy.as_str()).collect();
                format!("unknown deletion_policy `{value}`; expected one of {names:?}")
            })
    }
}

/// The name of the pair a config file with no `[[pair]]` table describes (ADR 0005 §2 rule 2).
///
/// A file with no `[[pair]]` is **one implicit pair called `default`** — permanently, not as a
/// migration step. Nothing is rewritten and nothing is asked of the user. Exported because phase 3
/// puts this name on the wire (`ControlRequest.pair` omitted means this pair), and a validated
/// string literal that each layer re-spells is how two spellings of one name happen.
pub const DEFAULT_PAIR_NAME: &str = "default";

/// Whether a config key describes **the process** or **one folder pair** (ADR 0005 §2).
///
/// Multi-pair (#102) turns every key into that question, and the answer is not a preference for
/// three of them: `proton_cli`, `proton_timeout_secs` and `proton_list_attempts` construct the one
/// shared `ProtonDriveClient`, and one client is one [`crate::proton::CliGate`] (#23). N clients
/// would be N gates, i.e. no serialization of the `proton-drive` children at all — so those three
/// are daemon-wide *by force*, and making them per-pair would mean moving
/// [`crate::proton::CommandPolicy`] off the client and onto every call.
///
/// The classification is machine-checked in both directions, which is the whole point of it
/// existing in phase 1 rather than being discovered in phase 4:
/// - [`ConfigKey::scope`] is an exhaustive match with **no `_` arm**, so a new variant cannot be
///   added without answering the question.
/// - `every_file_config_key_is_classified_exactly_once` compares [`ConfigKey::ALL`] against the
///   keys a fully-populated [`FileConfig`] serializes to, so a new *field* cannot be added without
///   a variant either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScope {
    /// The process, and the one shared `proton-drive` client: one value for the whole daemon.
    Daemon,
    /// One folder pair — what to sync, what to skip, how often, how a pass behaves. Belongs inside
    /// a `[[pair]]` table, and at the top level of a file that has none (the implicit pair).
    Pair,
}

/// Every key a config **file** may set, so the per-pair/daemon-wide split is a value the code can
/// read rather than a table in a document (ADR 0005 §2).
///
/// `pair` itself is deliberately not a variant: it is the *container* for per-pair keys, not a
/// setting with a scope. The key-set test names that exclusion explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    LocalRoot,
    RemoteRoot,
    DbPath,
    SocketPath,
    LockfilePath,
    ScanIntervalSecs,
    ProtonCli,
    ProtonTimeoutSecs,
    ProtonListAttempts,
    DownloadBatchSize,
    IncludePatterns,
    ExcludePatterns,
    DryRun,
    EventsDriven,
    EventsFullScanEvery,
    WarmStart,
    WarmStartFullWalkEvery,
    WarmStartMaxCursorAgeSecs,
    DeleteApproval,
    DeletionPolicyKey,
    LocalDeleteMode,
    LogLevel,
    ConflictSuffix,
}

impl ConfigKey {
    /// Every key. Completeness is pinned by `every_file_config_key_is_classified_exactly_once`,
    /// which compares these spellings against the keys [`FileConfig`] actually has: a variant
    /// missing from here whose field exists shows up as an unclassified key, and a variant listed
    /// here with no field shows up as a key the parser does not know.
    pub const ALL: [Self; 23] = [
        Self::LocalRoot,
        Self::RemoteRoot,
        Self::DbPath,
        Self::SocketPath,
        Self::LockfilePath,
        Self::ScanIntervalSecs,
        Self::ProtonCli,
        Self::ProtonTimeoutSecs,
        Self::ProtonListAttempts,
        Self::DownloadBatchSize,
        Self::IncludePatterns,
        Self::ExcludePatterns,
        Self::DryRun,
        Self::EventsDriven,
        Self::EventsFullScanEvery,
        Self::WarmStart,
        Self::WarmStartFullWalkEvery,
        Self::WarmStartMaxCursorAgeSecs,
        Self::DeleteApproval,
        Self::DeletionPolicyKey,
        Self::LocalDeleteMode,
        Self::LogLevel,
        Self::ConflictSuffix,
    ];

    /// The canonical (snake_case) TOML spelling. Every key also carries a kebab-case serde alias;
    /// a parsed [`FileConfig`] has already resolved either spelling to one field, so nothing that
    /// reads a *parsed* config needs the alias.
    pub fn spelling(self) -> &'static str {
        match self {
            Self::LocalRoot => "local_root",
            Self::RemoteRoot => "remote_root",
            Self::DbPath => "db_path",
            Self::SocketPath => "socket_path",
            Self::LockfilePath => "lockfile_path",
            Self::ScanIntervalSecs => "scan_interval_secs",
            Self::ProtonCli => "proton_cli",
            Self::ProtonTimeoutSecs => "proton_timeout_secs",
            Self::ProtonListAttempts => "proton_list_attempts",
            Self::DownloadBatchSize => "download_batch_size",
            Self::IncludePatterns => "include_patterns",
            Self::ExcludePatterns => "exclude_patterns",
            Self::DryRun => "dry_run",
            Self::EventsDriven => "events_driven",
            Self::EventsFullScanEvery => "events_full_scan_every",
            Self::WarmStart => "warm_start",
            Self::WarmStartFullWalkEvery => "warm_start_full_walk_every",
            Self::WarmStartMaxCursorAgeSecs => "warm_start_max_cursor_age_secs",
            Self::DeleteApproval => "delete_approval",
            Self::DeletionPolicyKey => "deletion_policy",
            Self::LocalDeleteMode => "local_delete_mode",
            Self::LogLevel => "log_level",
            Self::ConflictSuffix => "conflict_suffix",
        }
    }

    /// Which scope this key belongs to. **Exhaustive, no `_` arm** — see [`KeyScope`] for why that
    /// is the mechanism rather than a convention, and for why the three `proton_*` keys have no
    /// choice about their answer (#23).
    pub fn scope(self) -> KeyScope {
        match self {
            // Describes a tree: what to sync, what to skip, how often, how a pass behaves, what a
            // deletion needs.
            Self::LocalRoot
            | Self::RemoteRoot
            | Self::DbPath
            | Self::LockfilePath
            | Self::ScanIntervalSecs
            | Self::DownloadBatchSize
            | Self::IncludePatterns
            | Self::ExcludePatterns
            | Self::DryRun
            | Self::EventsDriven
            | Self::EventsFullScanEvery
            | Self::WarmStart
            | Self::WarmStartFullWalkEvery
            | Self::WarmStartMaxCursorAgeSecs
            | Self::DeleteApproval
            | Self::DeletionPolicyKey
            | Self::LocalDeleteMode
            | Self::ConflictSuffix => KeyScope::Pair,
            // Describes the process.
            Self::SocketPath | Self::LogLevel => KeyScope::Daemon,
            // Daemon-wide *because the client is shared*: these three are `CommandPolicy` and the
            // executable path, i.e. what the one `ProtonDriveClient` is constructed with. One
            // client is one `CliGate` (#23).
            Self::ProtonCli | Self::ProtonTimeoutSecs | Self::ProtonListAttempts => {
                KeyScope::Daemon
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DaemonConfigInput {
    pub config: Option<PathBuf>,
    pub local_root: Option<PathBuf>,
    pub remote_root: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
    pub lockfile_path: Option<PathBuf>,
    pub scan_interval_secs: Option<u64>,
    pub proton_cli: Option<PathBuf>,
    pub proton_timeout_secs: Option<u64>,
    pub proton_list_attempts: Option<usize>,
    pub download_batch_size: Option<usize>,
    pub dry_run: bool,
    pub no_dry_run: bool,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub events_driven: bool,
    pub no_events_driven: bool,
    pub events_full_scan_every: Option<u64>,
    /// Opt-in the first-pass warm start explicitly (default on; kept for symmetry / to override a
    /// config-file `warm_start = false`).
    pub warm_start: bool,
    /// Disable the first-pass warm start (always full-walk on boot).
    pub no_warm_start: bool,
    /// Force a full walk instead of a warm start every N warm starts (across restarts). `0`
    /// disables the periodic full walk.
    pub warm_start_full_walk_every: Option<u64>,
    /// Warm-start only if the persisted event cursor is at most this many seconds old (`0`
    /// disables the age gate).
    pub warm_start_max_cursor_age_secs: Option<u64>,
    /// One-shot `--full-walk`: force this boot's first pass to a full-tree walk.
    pub force_full_walk: bool,
    /// Coarse opt-out for the delete-approval guard: when set, disables approval for **both**
    /// directions globally (equivalent to `[delete_approval] remote = false, local = false`).
    /// Per-direction and per-subtree granularity lives in the per-directory `.proton-sync.toml`
    /// files (see `crate::dirconfig`); the CLI keeps only this blunt escape hatch.
    pub no_delete_approval: bool,
    /// `--deletion-policy`: the guard as one named setting. Beaten by `no_delete_approval`, beats
    /// anything the file says (including its `[delete_approval]` table).
    pub deletion_policy: Option<DeletionPolicy>,
    /// `--local-delete-mode`: what a local deletion does to the entity. Beats the file.
    pub local_delete_mode: Option<LocalDeleteMode>,
    /// `--log-level`: a `tracing` filter directive (`info`, `debug`, `crate::module=warn`, …).
    pub log_level: Option<String>,
    /// The process's `RUST_LOG`, passed in rather than read here so resolution stays pure and
    /// parallel tests cannot race on the environment (same reason as `expand_tilde_with_home`).
    pub rust_log: Option<String>,
    /// `--conflict-suffix`: how conflict sidecars are named. See [`ConflictNaming`].
    pub conflict_suffix: Option<String>,
}

// `Serialize` under `cfg(test)` only: it is what lets
// `every_file_config_key_is_classified_exactly_once` read this struct's real key set instead of a
// hand-written list that could drift. Nothing ships a serialized `FileConfig` — the GUI's writer
// round-trips the document with `toml_edit`.
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    #[serde(alias = "local-root")]
    local_root: Option<PathBuf>,
    #[serde(alias = "remote-root")]
    remote_root: Option<PathBuf>,
    #[serde(alias = "db-path")]
    db_path: Option<PathBuf>,
    #[serde(alias = "socket-path")]
    socket_path: Option<PathBuf>,
    #[serde(alias = "lockfile-path")]
    lockfile_path: Option<PathBuf>,
    #[serde(alias = "scan-interval-secs")]
    scan_interval_secs: Option<u64>,
    #[serde(alias = "proton-cli")]
    proton_cli: Option<PathBuf>,
    #[serde(alias = "proton-timeout-secs")]
    proton_timeout_secs: Option<u64>,
    #[serde(alias = "proton-list-attempts")]
    proton_list_attempts: Option<usize>,
    #[serde(alias = "download-batch-size")]
    download_batch_size: Option<usize>,
    #[serde(default, alias = "include")]
    include_patterns: Option<Vec<String>>,
    #[serde(default, alias = "exclude")]
    exclude_patterns: Option<Vec<String>>,
    #[serde(default, alias = "dry-run")]
    dry_run: Option<bool>,
    #[serde(default, alias = "events-driven")]
    events_driven: Option<bool>,
    #[serde(default, alias = "events-full-scan-every")]
    events_full_scan_every: Option<u64>,
    #[serde(default, alias = "warm-start")]
    warm_start: Option<bool>,
    #[serde(default, alias = "warm-start-full-walk-every")]
    warm_start_full_walk_every: Option<u64>,
    #[serde(default, alias = "warm-start-max-cursor-age-secs")]
    warm_start_max_cursor_age_secs: Option<u64>,
    /// Daemon-wide default for the directional delete-approval guard (the bottom of the
    /// per-directory inheritance chain). Each direction defaults to `true` (protected) when unset.
    #[serde(default, alias = "delete-approval")]
    delete_approval: Option<FileDeleteApproval>,
    /// The same guard as one named setting (#194). Mutually exclusive with `delete_approval` —
    /// see [`DeletionPolicy`] and [`resolve_file_delete_approval`].
    #[serde(default, alias = "deletion-policy")]
    deletion_policy: Option<DeletionPolicy>,
    /// What a local deletion does to the entity: `trash` (the default) moves it to the desktop
    /// trash, `permanent` removes it from disk. A different question from the guard above, which
    /// decides only whether a deletion waits for a person — see [`LocalDeleteMode`].
    #[serde(default, alias = "local-delete-mode")]
    local_delete_mode: Option<LocalDeleteMode>,
    /// Daemon log verbosity as a `tracing` filter directive. Outranked by the process's
    /// `RUST_LOG`, which outranks nothing else — see [`resolve_log_filter`].
    #[serde(default, alias = "log-level")]
    log_level: Option<String>,
    /// Conflict-sidecar suffix (`{stem}.{suffix}.{ext}`); default `proton-cloud`. Changing it
    /// orphans sidecars already on disk — see [`ConflictNaming`].
    #[serde(default, alias = "conflict-suffix")]
    conflict_suffix: Option<String>,
    /// The `[[pair]]` tables: folder pairs, each a `(local_root, remote_root)` this daemon syncs
    /// (#102, ADR 0005 §2).
    ///
    /// **A file with no `[[pair]]` is one implicit pair called [`DEFAULT_PAIR_NAME`]**, whose
    /// values are this file's top-level per-pair keys. That is permanent, not a migration step:
    /// every config written before multi-pair keeps working byte-identically, unrewritten. The two
    /// spellings are therefore mutually exclusive — see [`resolve_pairs`].
    ///
    /// Not a [`ConfigKey`]: this is the container for per-pair keys, not a setting with a scope.
    #[serde(default)]
    pair: Option<Vec<FilePair>>,
}

impl FileConfig {
    /// Whether this file sets `key` **at the top level** (either spelling — the parse already
    /// resolved the kebab-case alias).
    ///
    /// Exhaustive by variant with no `_` arm, so a new [`ConfigKey`] cannot be added without
    /// saying where to look for it. Read by [`resolve_pairs`] to enforce ADR 0005 §2 rule 1: a file
    /// that sets a per-pair key at the top level *and* declares `[[pair]]` tables has written one
    /// setting two ways, which has no defensible precedence.
    fn key_present(&self, key: ConfigKey) -> bool {
        match key {
            ConfigKey::LocalRoot => self.local_root.is_some(),
            ConfigKey::RemoteRoot => self.remote_root.is_some(),
            ConfigKey::DbPath => self.db_path.is_some(),
            ConfigKey::SocketPath => self.socket_path.is_some(),
            ConfigKey::LockfilePath => self.lockfile_path.is_some(),
            ConfigKey::ScanIntervalSecs => self.scan_interval_secs.is_some(),
            ConfigKey::ProtonCli => self.proton_cli.is_some(),
            ConfigKey::ProtonTimeoutSecs => self.proton_timeout_secs.is_some(),
            ConfigKey::ProtonListAttempts => self.proton_list_attempts.is_some(),
            ConfigKey::DownloadBatchSize => self.download_batch_size.is_some(),
            ConfigKey::IncludePatterns => self.include_patterns.is_some(),
            ConfigKey::ExcludePatterns => self.exclude_patterns.is_some(),
            ConfigKey::DryRun => self.dry_run.is_some(),
            ConfigKey::EventsDriven => self.events_driven.is_some(),
            ConfigKey::EventsFullScanEvery => self.events_full_scan_every.is_some(),
            ConfigKey::WarmStart => self.warm_start.is_some(),
            ConfigKey::WarmStartFullWalkEvery => self.warm_start_full_walk_every.is_some(),
            ConfigKey::WarmStartMaxCursorAgeSecs => self.warm_start_max_cursor_age_secs.is_some(),
            ConfigKey::DeleteApproval => self.delete_approval.is_some(),
            ConfigKey::DeletionPolicyKey => self.deletion_policy.is_some(),
            ConfigKey::LocalDeleteMode => self.local_delete_mode.is_some(),
            ConfigKey::LogLevel => self.log_level.is_some(),
            ConfigKey::ConflictSuffix => self.conflict_suffix.is_some(),
        }
    }
}

/// One `[[pair]]` table: a folder pair, plus every [`KeyScope::Pair`] key scoped to it.
///
/// Hosts exactly the per-pair keys — pinned by `a_pair_table_hosts_exactly_the_per_pair_keys`, so a
/// key classified per-pair that this table cannot express is a test failure rather than a surprise
/// in phase 4.
///
/// `deny_unknown_fields` must be repeated here for the same reason [`FileDeleteApproval`] repeats
/// it: serde's deny on [`FileConfig`] does not recurse into nested tables, so without it a typo
/// inside a `[[pair]]` table would be silently ignored (#64).
///
/// `name` is the only **required** field. The roots stay optional because a *file* is not the only
/// source of them — `--local-root` / `--remote-root` amend the single pair, and
/// [`validate_file_config_text`] is scoped to what a file alone can decide (the GUI validates
/// documents it may be part-way through editing).
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(deny_unknown_fields)]
struct FilePair {
    /// The pair's identity and, from phase 3, the wire selector. Required, unique, and matched
    /// byte-exactly; see [`validate_pair_name`].
    name: String,
    #[serde(default, alias = "local-root")]
    local_root: Option<PathBuf>,
    #[serde(default, alias = "remote-root")]
    remote_root: Option<PathBuf>,
    #[serde(default, alias = "db-path")]
    db_path: Option<PathBuf>,
    #[serde(default, alias = "lockfile-path")]
    lockfile_path: Option<PathBuf>,
    #[serde(default, alias = "scan-interval-secs")]
    scan_interval_secs: Option<u64>,
    #[serde(default, alias = "download-batch-size")]
    download_batch_size: Option<usize>,
    #[serde(default, alias = "include")]
    include_patterns: Option<Vec<String>>,
    #[serde(default, alias = "exclude")]
    exclude_patterns: Option<Vec<String>>,
    #[serde(default, alias = "dry-run")]
    dry_run: Option<bool>,
    #[serde(default, alias = "events-driven")]
    events_driven: Option<bool>,
    #[serde(default, alias = "events-full-scan-every")]
    events_full_scan_every: Option<u64>,
    #[serde(default, alias = "warm-start")]
    warm_start: Option<bool>,
    #[serde(default, alias = "warm-start-full-walk-every")]
    warm_start_full_walk_every: Option<u64>,
    #[serde(default, alias = "warm-start-max-cursor-age-secs")]
    warm_start_max_cursor_age_secs: Option<u64>,
    #[serde(default, alias = "delete-approval")]
    delete_approval: Option<FileDeleteApproval>,
    #[serde(default, alias = "deletion-policy")]
    deletion_policy: Option<DeletionPolicy>,
    #[serde(default, alias = "local-delete-mode")]
    local_delete_mode: Option<LocalDeleteMode>,
    #[serde(default, alias = "conflict-suffix")]
    conflict_suffix: Option<String>,
}

/// The `[delete_approval]` table in the daemon config file. Names the *target* of the deletion
/// being gated; unset directions default to protected.
///
/// `deny_unknown_fields` must be repeated here: serde's deny on [`FileConfig`] does not recurse
/// into nested tables, so without it a typo like `remot = false` would be silently ignored and
/// the guard would stay on despite the user's intent (#64).
#[derive(Debug, Default, Clone, Deserialize)]
#[cfg_attr(test, derive(Serialize))]
#[serde(deny_unknown_fields)]
struct FileDeleteApproval {
    remote: Option<bool>,
    local: Option<bool>,
}

/// One folder pair as a config **file** states it, from whichever of the two spellings the file
/// uses: a `[[pair]]` table, or the top-level keys of a file that declares none.
///
/// This is the single projection everything per-pair reads, which is what keeps the implicit pair
/// from being a second code path: [`resolve_runtime_config`] takes its per-pair values from here
/// and its daemon-wide values from [`FileConfig`] directly, so a top-level file and the equivalent
/// one-`[[pair]]` file resolve to the same `DaemonConfig` by construction (pinned by
/// `a_top_level_file_and_the_equivalent_pair_table_resolve_identically`).
///
/// Both constructors are **exhaustive struct literals with no `..Default::default()`**, so adding a
/// field here is a build failure until both sources answer for it.
#[derive(Debug, Clone)]
struct PairFileConfig {
    name: String,
    local_root: Option<PathBuf>,
    remote_root: Option<PathBuf>,
    db_path: Option<PathBuf>,
    lockfile_path: Option<PathBuf>,
    scan_interval_secs: Option<u64>,
    download_batch_size: Option<usize>,
    include_patterns: Option<Vec<String>>,
    exclude_patterns: Option<Vec<String>>,
    dry_run: Option<bool>,
    events_driven: Option<bool>,
    events_full_scan_every: Option<u64>,
    warm_start: Option<bool>,
    warm_start_full_walk_every: Option<u64>,
    warm_start_max_cursor_age_secs: Option<u64>,
    delete_approval: Option<FileDeleteApproval>,
    deletion_policy: Option<DeletionPolicy>,
    local_delete_mode: Option<LocalDeleteMode>,
    conflict_suffix: Option<String>,
}

impl PairFileConfig {
    /// The implicit pair (ADR 0005 §2 rule 2): a file with no `[[pair]]` is one pair called
    /// [`DEFAULT_PAIR_NAME`], and its values are the file's top-level per-pair keys.
    fn from_top_level(config: &FileConfig) -> Self {
        Self {
            name: DEFAULT_PAIR_NAME.to_owned(),
            local_root: config.local_root.clone(),
            remote_root: config.remote_root.clone(),
            db_path: config.db_path.clone(),
            lockfile_path: config.lockfile_path.clone(),
            scan_interval_secs: config.scan_interval_secs,
            download_batch_size: config.download_batch_size,
            include_patterns: config.include_patterns.clone(),
            exclude_patterns: config.exclude_patterns.clone(),
            dry_run: config.dry_run,
            events_driven: config.events_driven,
            events_full_scan_every: config.events_full_scan_every,
            warm_start: config.warm_start,
            warm_start_full_walk_every: config.warm_start_full_walk_every,
            warm_start_max_cursor_age_secs: config.warm_start_max_cursor_age_secs,
            delete_approval: config.delete_approval.clone(),
            deletion_policy: config.deletion_policy,
            local_delete_mode: config.local_delete_mode,
            conflict_suffix: config.conflict_suffix.clone(),
        }
    }

    /// An explicit `[[pair]]` table.
    fn from_table(pair: &FilePair) -> Self {
        Self {
            name: pair.name.clone(),
            local_root: pair.local_root.clone(),
            remote_root: pair.remote_root.clone(),
            db_path: pair.db_path.clone(),
            lockfile_path: pair.lockfile_path.clone(),
            scan_interval_secs: pair.scan_interval_secs,
            download_batch_size: pair.download_batch_size,
            include_patterns: pair.include_patterns.clone(),
            exclude_patterns: pair.exclude_patterns.clone(),
            dry_run: pair.dry_run,
            events_driven: pair.events_driven,
            events_full_scan_every: pair.events_full_scan_every,
            warm_start: pair.warm_start,
            warm_start_full_walk_every: pair.warm_start_full_walk_every,
            warm_start_max_cursor_age_secs: pair.warm_start_max_cursor_age_secs,
            delete_approval: pair.delete_approval.clone(),
            deletion_policy: pair.deletion_policy,
            local_delete_mode: pair.local_delete_mode,
            conflict_suffix: pair.conflict_suffix.clone(),
        }
    }
}

/// The folder pairs a config file declares, in file order (ADR 0005 §2).
///
/// This is the **one** definition of the pair shape, called by both readers of a config file — the
/// daemon's [`resolve_runtime_config`] and, through [`validate_file_config_text`], the GUI's config
/// writer. A second copy is how the daemon and the GUI ended up disagreeing about `~` (#135).
///
/// Enforces the rules that are **structural**, i.e. the ones about the pair *set*, which no CLI
/// flag can change the answer to (a flag cannot say which pair it amends):
///
/// 1. **Both spellings is an error.** A per-pair key at the top level *and* a `[[pair]]` table is
///    refused, naming both — exactly as `deletion_policy` + `[delete_approval]` is refused. One
///    setting written two ways has no defensible precedence, and refusing is also what lets a
///    round-trip writer know which spelling it may rewrite. Daemon-wide keys are untouched: they
///    belong at the top level and stay there. Which keys are which comes from [`ConfigKey::scope`].
/// 2. **No `[[pair]]` at all is one implicit pair called [`DEFAULT_PAIR_NAME`]** — permanent, not a
///    migration step. An *explicitly empty* `pair = []` is refused instead: it is a statement, and
///    reading it as "one implicit pair" would make it silently mean the opposite of what it says.
/// 3. **`name` is required, charset-bounded, unique case-insensitively, and `default` is reserved
///    for the first pair** — see [`validate_pair_names`] and [`validate_pair_name`].
/// 4. **No two pairs' roots may collide or nest**, nor may one pair's `db_path`/`lockfile_path`
///    land inside or on another pair's — see [`validate_pair_roots`].
///
/// **Both spellings go through the same checks** (#339). The implicit-pair arm used to return
/// before rules 3 and 4, so a `[[pair]]` file was refused over `~user`, a `db_path` a flag replaces
/// or a same-pair state collision while the byte-identical top-level file started. The arms differ
/// now only in *where the values come from*, which is the single [`PairFileConfig`] projection.
///
/// Value-level checks a flag can mask (an empty root, a `~user` path, a bad glob, a zero
/// `download_batch_size`, one file used as both index and lockfile) deliberately live in
/// [`validate_pair_file_values`] and [`validate_runtime_config`] instead: on the daemon's path the
/// *merged* value is what matters, and `--local-root /x` over a file's `local_root = ""` starts fine
/// today. Nothing here may refuse a value a flag replaces — that is the rule the layer exists under.
///
/// The **order is meaningful**: the first pair is the default pair (the one a client predating
/// `ControlRequest.pair` addresses). Reordering the tables is what would change it, which is why the
/// order is preserved rather than sorted, and why rule 3 reserves `default` for the first table.
fn resolve_pairs(config: &FileConfig) -> AppResult<Vec<PairFileConfig>> {
    let pairs = match &config.pair {
        None => vec![PairFileConfig::from_top_level(config)],
        Some(tables) if tables.is_empty() => {
            return Err(boxed_error(
                "config declares `pair = []`: an empty pair list syncs nothing. Declare at least \
                 one `[[pair]]` table, or remove the key and use top-level \
                 `local_root`/`remote_root` (a file with no `[[pair]]` is one pair named `default`)",
            ));
        }
        Some(tables) => {
            let top_level_per_pair_keys: Vec<&str> = ConfigKey::ALL
                .into_iter()
                .filter(|key| key.scope() == KeyScope::Pair && config.key_present(*key))
                .map(ConfigKey::spelling)
                .collect();
            if !top_level_per_pair_keys.is_empty() {
                // Named once, not twice: repeating the list read as "move `a`, `b` and `c` into the
                // table *it* belongs to", whose grammar drifts the moment there is more than one key.
                return Err(boxed_error(format!(
                    "config sets per-pair {} at the top level and also declares `[[pair]]` tables: \
                     they are two spellings of one setting; move each per-pair key into the \
                     `[[pair]]` table it belongs to, or delete the `[[pair]]` tables",
                    describe_quoted(&top_level_per_pair_keys),
                )));
            }
            tables.iter().map(PairFileConfig::from_table).collect()
        }
    };
    validate_pair_names(&pairs)?;
    validate_pair_roots(&pairs)?;
    Ok(pairs)
}

/// `` `a` `` / `` `a`, `b` and `c` `` — so an error naming several things reads as a sentence.
///
/// Deliberately not named for keys: it renders pair *names* too
/// ([`refuse_unsupported_pair_count`]), and a helper whose name says "keys" while it quotes names is
/// a comment that lies (#339).
fn describe_quoted(keys: &[&str]) -> String {
    let quoted: Vec<String> = keys.iter().map(|key| format!("`{key}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Characters a pair `name` may use. Narrow on purpose (ADR 0005 §2 rule 3): a name is a CLI
/// argument (`proton-sync --pair NAME`) and a wire selector, so it must never need quoting and must
/// never look like a path.
///
/// The charset alone does not deliver that, which is why [`validate_pair_name`] adds two refusals
/// on top of it (#339): `.` and `..` are spelled entirely within it and *are* path components, and
/// a leading `-` is spelled within it and *is* option syntax.
const PAIR_NAME_CHARS: &str = "letters, digits, `.`, `_` and `-`";

/// The two names that are path components whatever the charset says.
const PAIR_NAME_RESERVED_PATHS: [&str; 2] = [".", ".."];

/// The longest a pair `name` may be. Bounds an error message and a future wire selector; nothing
/// about a folder needs more.
const PAIR_NAME_MAX_LEN: usize = 64;

/// `name` is required, `[A-Za-z0-9._-]{1,64}`, unique **case-insensitively**, and
/// [`DEFAULT_PAIR_NAME`] belongs to the first pair.
///
/// Case-insensitive uniqueness is #298's rule applied one layer up: names are matched byte-exactly
/// on the wire, so `Photos` and `photos` would be two pairs a person cannot tell apart while a
/// selector resolves to exactly one of them. Refusing at startup is the only place that ambiguity
/// can be removed rather than resolved arbitrarily.
///
/// **`default` is a sentinel with two surfaces** (#339), and reserving it is the `all` decision (ADR
/// 0005 §4, the #140 lesson) applied to the other one. A request that names no pair addresses the
/// *first* table (§2 rule 6 / §7), which is what `default` means — so a later table called `default`
/// gives one selector two answers: an omitted selector reaches the first pair and `--pair default`
/// reaches this one. It is **not** refused outright, because the first pair may legitimately be
/// called that: it is what the implicit pair is already named, and it is what §7's
/// promote-to-`[[pair]]` rewrite would write for a pre-existing single-pair file. Folded like the
/// uniqueness rule above, for the same reason — a person cannot tell `Default` from `default`.
fn validate_pair_names(pairs: &[PairFileConfig]) -> AppResult<()> {
    let mut seen: Vec<String> = Vec::with_capacity(pairs.len());
    for (position, pair) in pairs.iter().enumerate() {
        validate_pair_name(&pair.name)?;
        let folded = pair.name.to_ascii_lowercase();
        // The duplicate check runs FIRST. Two tables both named `default` are two names that are
        // the same, not one name in the wrong position, and the reservation's advice ("move its
        // table first") would produce two `default`s if it spoke about them.
        if seen.contains(&folded) {
            return Err(boxed_error(format!(
                "two `[[pair]]` tables are named `{}` (names are compared without regard to \
                 case, because a selector that matches two pairs can only pick one of them): give \
                 each pair a distinct name",
                pair.name
            )));
        }
        if position > 0 && folded == DEFAULT_PAIR_NAME {
            return Err(boxed_error(format!(
                "pair `{}` is named `{DEFAULT_PAIR_NAME}` but is not the first `[[pair]]` table: \
                 that name already means the pair a command addresses when it names none, which is \
                 the first table, so one selector would have two answers. Rename this pair, or \
                 move its table first",
                pair.name
            )));
        }
        seen.push(folded);
    }
    Ok(())
}

fn validate_pair_name(name: &str) -> AppResult<()> {
    if name.is_empty() {
        return Err(boxed_error(
            "a `[[pair]]` table has an empty `name`: every pair needs a name, which is how a \
             command says which folder it means",
        ));
    }
    // The charset runs BEFORE the length (#339): `name.len()` is bytes, so a 40-character accented
    // name is 80 of them and used to be reported as "longer than 64 characters", which names the
    // wrong problem. Once the charset holds every character is one byte, and the two agree.
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(boxed_error(format!(
            "pair name `{name}` contains `{bad}`: a name may use only {PAIR_NAME_CHARS}, so that \
             it is always a safe command argument and never looks like a path"
        )));
    }
    if name.len() > PAIR_NAME_MAX_LEN {
        return Err(boxed_error(format!(
            "pair name `{name}` is longer than {PAIR_NAME_MAX_LEN} characters"
        )));
    }
    // The two refusals the charset cannot express, and without which its own justification is a
    // claim the code does not honour (#339).
    if PAIR_NAME_RESERVED_PATHS.contains(&name) {
        return Err(boxed_error(format!(
            "pair name `{name}` is a path component: the charset exists so that a name never looks \
             like a path, and `.` and `..` are the two names that do while using only \
             {PAIR_NAME_CHARS}"
        )));
    }
    if name.starts_with('-') {
        return Err(boxed_error(format!(
            "pair name `{name}` starts with `-`: a name is a command argument (`proton-sync --pair \
             {name}`), and an argument starting with `-` is read as an option rather than as the \
             name of a folder pair"
        )));
    }
    Ok(())
}

/// One pair's path as this layer compares it: which pair it belongs to (by position, since that is
/// the only identity a comparison may not get wrong), the key it was written under, the value the
/// user wrote, and the value to compare.
struct ComparablePath<'a> {
    pair: usize,
    name: &'a str,
    field: &'a str,
    written: PathBuf,
    key: PathBuf,
}

/// No two pairs may share a `local_root` or a `remote_root`, and neither may **nest** inside
/// another pair's; nor may one pair's `db_path` / `lockfile_path` be another pair's, or sit inside
/// another pair's `local_root` (ADR 0005 §2 rule 4).
///
/// Nesting has a concrete consequence, not a tidiness argument: [`crate::index`]'s
/// `is_sync_state_path` matches only the **first** component of a relative path, because a `.sync`
/// deeper in the tree is ordinary user data. So a pair nested under another pair's root would have
/// its own state directory — SQLite index, WAL, lockfile, status/metrics sidecars — scanned and
/// uploaded to Proton Drive as the outer pair's ordinary files. The remote half is the mirror: two
/// pairs over one remote subtree plan opposing actions for it.
///
/// **The state-path half is that same consequence reached around the rule** (#339): the rule was
/// written root-vs-root, so a `db_path` explicitly placed inside *another* pair's `local_root` — no
/// nesting of roots anywhere — produced exactly the upload the doc comment names.
/// `scan_options_from_config` is handed only this pair's own `db_path`, so the outer pair has no
/// way to know the file it is uploading is a live SQLite index.
///
/// A duplicated `lockfile_path` is worth its own check because of how it fails otherwise:
/// `LockGuard::acquire` uses `try_lock_exclusive`, and `flock` treats two descriptors on one inode
/// as independent *even in one process*, so the second pair would report "another daemon is already
/// running" — true, and incomprehensible. This check runs before any lock is taken.
///
/// **Every rule here is about two pairs**, which is what makes it structural: a CLI flag cannot say
/// which pair it amends, so no flag can change any of these answers. The same-pair question — one
/// file used as both a pair's index and its lockfile — a flag *can* change, and it lives with the
/// other flag-maskable rules ([`require_distinct_state_paths`], called from
/// [`validate_pair_file_values`] and [`validate_runtime_config`]).
///
/// The comparison is **lexical**, because a config file must be checkable without touching the
/// filesystem (the GUI validates documents for paths that may not exist yet). It therefore does not
/// see two roots that reach one directory through a symlink, a `..`, or a relative path resolved
/// against the daemon's working directory; catching those would need `canonicalize` on live paths,
/// which belongs to the daemon's own startup rather than to a file check. `~` is expanded
/// **best-effort** ([`expand_tilde_for_comparison`]) — reading `$HOME` is not touching the
/// filesystem, and it is what keeps `~/Sync` and `/home/me/Sync` from reading as two roots — but a
/// value it cannot expand is compared verbatim rather than refused, because refusing here would
/// refuse a value a flag replaces.
fn validate_pair_roots(pairs: &[PairFileConfig]) -> AppResult<()> {
    let mut local_roots: Vec<ComparablePath<'_>> = Vec::new();
    let mut remote_roots: Vec<ComparablePath<'_>> = Vec::new();
    let mut state_paths: Vec<ComparablePath<'_>> = Vec::new();
    for (position, pair) in pairs.iter().enumerate() {
        let local_root = pair
            .local_root
            .as_deref()
            .map(expand_tilde_for_comparison)
            .map(|root| local_comparison_key(&root));
        for (field, override_value, default_for) in [
            (
                "db_path",
                pair.db_path.as_deref(),
                default_state_db_path as fn(&Path) -> PathBuf,
            ),
            (
                "lockfile_path",
                pair.lockfile_path.as_deref(),
                default_lockfile_path as fn(&Path) -> PathBuf,
            ),
        ] {
            if let Some(key) =
                comparison_state_path(local_root.as_deref(), override_value, default_for)
            {
                state_paths.push(ComparablePath {
                    pair: position,
                    name: &pair.name,
                    field,
                    written: override_value.map_or_else(|| key.clone(), Path::to_path_buf),
                    key,
                });
            }
        }
        if let (Some(written), Some(key)) = (pair.local_root.clone(), local_root) {
            local_roots.push(ComparablePath {
                pair: position,
                name: &pair.name,
                field: "local_root",
                written,
                key,
            });
        }
        if let Some(remote_root) = &pair.remote_root {
            remote_roots.push(ComparablePath {
                pair: position,
                name: &pair.name,
                field: "remote_root",
                written: remote_root.clone(),
                key: remote_root_comparison_key(remote_root),
            });
        }
    }
    check_no_overlap(
        &local_roots,
        "the inner pair's `.sync` state directory — its SQLite index, lockfile and sidecars — \
         would be scanned and uploaded to Proton Drive as the outer pair's ordinary files",
    )?;
    check_no_overlap(
        &remote_roots,
        "both pairs would plan actions for one remote subtree, each undoing the other's",
    )?;
    for (position, path) in state_paths.iter().enumerate() {
        if let Some(other) = state_paths[..position]
            .iter()
            .find(|other| other.pair != path.pair && other.key == path.key)
        {
            return Err(boxed_error(format!(
                "pair `{}`'s {} and pair `{}`'s {} both resolve to `{}`: no two of these may be \
                 the same file — `flock` treats two descriptors on one inode as independent, so a \
                 shared lockfile surfaces as a spurious \"already running\", and a shared index has \
                 two writers of one baseline",
                path.name,
                path.field,
                other.name,
                other.field,
                path.key.display()
            )));
        }
    }
    for path in &state_paths {
        if let Some(root) = local_roots
            .iter()
            .find(|root| root.pair != path.pair && path.key.starts_with(&root.key))
        {
            return Err(boxed_error(format!(
                "pair `{}`'s {} `{}` is inside pair `{}`'s local_root `{}`: pair `{}` would scan \
                 that file and upload pair `{}`'s live SQLite index and lockfile to Proton Drive \
                 as its own, because `is_sync_state_path` ignores only a top-level `.sync`",
                path.name,
                path.field,
                path.written.display(),
                root.name,
                root.written.display(),
                root.name,
                path.name
            )));
        }
    }
    Ok(())
}

/// Refuses any path that equals, contains, or is contained by an earlier one belonging to a
/// *different* pair. Component-wise (`Path::starts_with`), so `/a/b` bounds `/a/b/c` but not
/// `/a/bc`.
///
/// The message renders the value the user **wrote**, not the comparison key: a file saying
/// `remote_root = "/Drive/X"` was told about `` `Drive/X` ``, which is a path it does not contain
/// (#339).
fn check_no_overlap(paths: &[ComparablePath<'_>], consequence: &str) -> AppResult<()> {
    for (position, path) in paths.iter().enumerate() {
        for other in &paths[..position] {
            if other.pair == path.pair {
                continue;
            }
            let relation = if path.key == other.key {
                "the same path as"
            } else if path.key.starts_with(&other.key) {
                "inside"
            } else if other.key.starts_with(&path.key) {
                "a parent of"
            } else {
                continue;
            };
            return Err(boxed_error(format!(
                "pair `{}`'s {} `{}` is {relation} pair `{}`'s `{}`: {consequence}",
                path.name,
                path.field,
                path.written.display(),
                other.name,
                other.written.display()
            )));
        }
    }
    Ok(())
}

/// A **local** path reduced to what it *addresses*, for comparison only: a leading `./` is
/// dropped, so `local_root = "A"` and `local_root = "./A"` are one directory rather than two that
/// can never overlap (#339 round 2). Only the *leading* one matters — `Path::components` already
/// drops a non-leading `.`, and `Path`'s own equality and `starts_with` compare component-wise.
///
/// The leading `/` is deliberately **not** dropped, which is where this differs from
/// [`remote_root_comparison_key`]: for a local path absolute and relative are two different
/// locations, while `/Drive/X` and `Drive/X` are one Drive location.
///
/// A path that is *nothing but* `.` keeps its literal form. Reducing it to the empty path would
/// make it a prefix of every path there is, and an absolute root would then be refused as "inside"
/// a relative one.
fn local_comparison_key(path: &Path) -> PathBuf {
    let stripped: PathBuf = path
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect();
    if stripped.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        stripped
    }
}

/// A `remote_root` reduced to what it *addresses*, for comparison only.
///
/// `/Drive/Photos` and `Drive/Photos` name one Drive location — `proton.rs`'s
/// `normalize_remote_path` strips the root either way — so the leading separator and any `.` are
/// dropped before comparing. `..` is kept verbatim rather than resolved: resolving it lexically
/// would make `/Drive/a/../b` compare equal to `/Drive/a/b`, which it is not.
fn remote_root_comparison_key(remote_root: &Path) -> PathBuf {
    remote_root
        .components()
        .filter(|component| !matches!(component, Component::RootDir | Component::CurDir))
        .collect()
}

/// Which file a `db_path` / `lockfile_path` names, once `~` is out of the way: an absolute override
/// verbatim, a relative one under `local_root`, and otherwise the per-root `.sync` default.
///
/// The **one** definition of "which file is this pair's index", shared by the daemon's merge path
/// ([`effective_state_path`]) and the comparison layer ([`comparison_state_path`]) so the two cannot
/// answer it differently. The `~` expansion is what the two differ on, and only that.
fn state_path_from(
    local_root: &Path,
    override_value: Option<PathBuf>,
    default_for: impl Fn(&Path) -> PathBuf,
) -> PathBuf {
    match override_value {
        Some(path) => resolve_path(local_root, path),
        None => default_for(local_root),
    }
}

/// The `db_path` / `lockfile_path` a pair will really use, from values that are already the merged
/// ones. Refuses a `~` it cannot expand, because this *is* the value the daemon will open.
fn effective_state_path(
    local_root: &Path,
    override_value: Option<PathBuf>,
    field_name: &str,
    default_for: impl Fn(&Path) -> PathBuf,
) -> AppResult<PathBuf> {
    let override_value = override_value
        .map(|path| expand_tilde(path, field_name))
        .transpose()?;
    Ok(state_path_from(local_root, override_value, default_for))
}

/// The same file, for **comparison between pairs** — and `None` when the file does not place it.
///
/// Never fails, by construction: this runs before any flag is merged, so every value it sees may
/// still be replaced (#339). Two things it therefore does differently from [`effective_state_path`]:
/// `~` is expanded best-effort, and a *relative* override with no `local_root` yet has no answer
/// rather than a wrong one. An **absolute** override is placed either way, which is what lets two
/// rootless pairs sharing one `db_path` still be caught — roots are optional in a `[[pair]]`,
/// supplied later by a flag, and collecting state paths only inside `if let Some(local_root)` meant
/// those two were never compared with each other.
fn comparison_state_path(
    local_root: Option<&Path>,
    override_value: Option<&Path>,
    default_for: impl Fn(&Path) -> PathBuf,
) -> Option<PathBuf> {
    match override_value {
        Some(value) => {
            let value = expand_tilde_for_comparison(value);
            if value.is_absolute() {
                Some(value)
            } else {
                local_root.map(|root| state_path_from(root, Some(value), default_for))
            }
        }
        None => local_root.map(default_for),
    }
}

/// `~` expanded when it can be, kept **verbatim** when it cannot — for comparison only.
///
/// The structural layer may not refuse a value a CLI flag replaces (#339), and `expand_tilde` is
/// fallible (`~user`, or `~` with no `HOME`). Those values *are* refused — on the merge path, where
/// the value is the one the daemon will really use, and in [`validate_pair_file_values`] for a
/// reader that has no flags. Here they are only compared, and comparing the literal is exactly what
/// the top-level spelling has always done. Expanding what it can is not optional either: `~/Sync`
/// and `/home/me/Sync` are one directory, and a comparison that read them as two would miss the
/// nesting it exists to refuse. Same best-effort shape, for the same reason, as `gui-core`'s
/// `expand_config_path`.
fn expand_tilde_for_comparison(path: &Path) -> PathBuf {
    let literal = path.to_path_buf();
    // The error text is discarded: nothing here can fail its way into a message, so the field name
    // is never read.
    expand_tilde(literal.clone(), "a pair path").unwrap_or(literal)
}

/// One file cannot be both a pair's SQLite index and its lockfile.
///
/// A **same-pair** rule, and one a flag can change the answer to (`--db-path` / `--lockfile-path`
/// replace both values), so it runs where the values that will really be used are: on the file's own
/// values for a reader with no flags ([`validate_pair_file_values`]) and on the merged values for
/// the daemon ([`validate_runtime_config`]). It used to run in the structural layer, on written
/// values, on the `[[pair]]` arm only — so a `[[pair]]` file was refused over two values its flags
/// replaced while the top-level spelling was never checked at all (#339).
fn require_distinct_state_paths(
    pair: Option<&str>,
    db_path: &Path,
    lockfile_path: &Path,
) -> AppResult<()> {
    if db_path == lockfile_path {
        // Named when the reader knows which table it is reading (the file half is per pair), and
        // nameless on the merge path, where `DaemonConfig` is one pair and has no name to give.
        // The base rule this replaced named its pair, so dropping it would be a downgrade the
        // moment there is more than one table (#339 round 2).
        let subject = match pair {
            Some(name) => format!("pair `{name}`'s db_path and lockfile_path"),
            None => "db_path and lockfile_path".to_owned(),
        };
        return Err(boxed_error(format!(
            "{subject} both resolve to `{}`: one file cannot be both this pair's \
             SQLite index and its lockfile — the lockfile is `flock`ed for the whole life of the \
             daemon and the index is a database SQLite opens and writes, so naming one file for \
             both makes the single-instance check depend on the database and gives the database a \
             whole-file advisory lock",
            db_path.display()
        )));
    }
    Ok(())
}

/// Refuses more than one folder pair, which is what makes phase 1 of #102 shippable on its own: the
/// config *shape* lands now so phases 2–4 have something to build against, while the capability
/// does not exist yet — one `PairRuntime` per pair, a pair selector on the wire, and a scheduler
/// that serializes passes through the one `CliGate` are still to come.
///
/// Called by **both** readers of a config file. The GUI's half matters as much as the daemon's:
/// `ConfigDoc::save` never writes a config the daemon would refuse to start on, so a file this
/// would reject at startup must be rejected at save time too.
///
/// Deliberately separate from [`resolve_pairs`], and run *after* it: lifting the cap is then one
/// function, and until then a genuinely broken multi-pair file still reports what is wrong with it
/// rather than being masked by "not yet supported".
fn refuse_unsupported_pair_count(pairs: &[PairFileConfig]) -> AppResult<()> {
    if pairs.len() > 1 {
        let names: Vec<&str> = pairs.iter().map(|pair| pair.name.as_str()).collect();
        return Err(boxed_error(format!(
            "config declares {} folder pairs ({}), and syncing more than one pair is not yet \
             supported: keep one `[[pair]]` table and remove the rest",
            pairs.len(),
            describe_quoted(&names),
        )));
    }
    Ok(())
}

pub fn resolve_runtime_config(input: DaemonConfigInput) -> AppResult<(DaemonConfig, bool)> {
    // The config-file path is itself a local-filesystem path, so it gets the same `~` treatment
    // as the values inside it (see `expand_tilde` below).
    let config_path = input
        .config
        .map(|path| expand_tilde(path, "--config"))
        .transpose()?;
    let file_config = load_file_config(config_path.as_ref())?;
    // The pair shape is resolved before anything is merged, and before any lock is taken (#102, ADR
    // 0005 §2): a file with no `[[pair]]` is one implicit pair named `default` whose values are the
    // top-level per-pair keys, so every config written before multi-pair resolves exactly as it
    // always has. More than one pair is refused until the runtime can serialize their passes.
    let pairs = resolve_pairs(&file_config)?;
    refuse_unsupported_pair_count(&pairs)?;
    // `DaemonConfig` below stays the **fused resolved input**: one flat struct a config file and the
    // CLI flags merge into, which is also what the one-shot `--dry-run` preview and every test
    // fixture builds. Phase 2 splits it at the *runtime* boundary instead
    // (`daemon::DaemonConfig::into_parts` → a process-wide half and the pair's own), so the daemon
    // holds exactly one copy of `local_root` without this type — and its 27 construction sites —
    // growing a pair dimension it cannot yet express. Making the *input* a `Vec<PairConfig>` is the
    // same change as lifting `refuse_unsupported_pair_count`, so it belongs to the phase that lifts
    // it (4), not to the one that moves the fields (2). What matters here is that every per-pair
    // value now comes from ONE projection
    // (`PairFileConfig`) rather than from the top-level keys directly — a `[[pair]]` file and the
    // equivalent top-level file therefore cannot diverge. CLI flags still outrank the file and mean
    // "the single pair", so `--local-root ~/x` keeps working over either spelling.
    // `resolve_pairs` never returns an empty list (a file with no `[[pair]]` is one implicit pair,
    // and an explicit `pair = []` is refused there), so this reports a broken invariant rather than
    // quietly substituting a pair nothing in the file asked for.
    let pair = pairs.into_iter().next().ok_or_else(|| {
        boxed_error("config resolved to no folder pair at all; this is a bug in resolve_pairs")
    })?;
    let dry_run = if input.no_dry_run {
        false
    } else if input.dry_run {
        true
    } else {
        pair.dry_run.unwrap_or(false)
    };
    // Event-driven ("snapshot + stream") remote sync is the default. `--no-events-driven` (or
    // `events_driven = false` in the config file) opts back into full-tree-walk-only detection.
    // Precedence mirrors `dry_run`: explicit opt-out flag > explicit opt-in flag > file value >
    // default (on). When the reused CLI session/keyring is unavailable the daemon still degrades
    // to full-tree snapshots at runtime (see `build_event_source`), so defaulting on is safe.
    let events_driven = if input.no_events_driven {
        false
    } else if input.events_driven {
        true
    } else {
        pair.events_driven.unwrap_or(true)
    };
    // Delete-approval guard defaults (the root of the per-directory inheritance chain). Each
    // direction is ON (protected) by default; the coarse `--no-delete-approval` flag forces both
    // off, otherwise the config file's `[delete_approval]` values apply per direction. Per-subtree
    // overrides live in `.proton-sync.toml` files, resolved at reconcile time by `crate::dirconfig`.
    let (delete_approval_remote, delete_approval_local) = if input.no_delete_approval {
        (false, false)
    } else if let Some(policy) = input.deletion_policy {
        policy.directions()
    } else {
        resolve_file_delete_approval(pair.deletion_policy, pair.delete_approval.as_ref())?
    };
    // Warm start (first-pass event-driven reconcile). Enabled by default; precedence mirrors
    // `events_driven`: explicit opt-out flag > explicit opt-in flag > file value > default (on).
    // `full_walk_every` and `max_cursor_age_secs` accept `0` as a meaningful "disabled" sentinel,
    // so — like `events_full_scan_every` — they are not clamped up to 1.
    let warm_start_enabled = if input.no_warm_start {
        false
    } else if input.warm_start {
        true
    } else {
        pair.warm_start.unwrap_or(true)
    };
    let warm_start = WarmStartConfig {
        enabled: warm_start_enabled,
        full_walk_every: input
            .warm_start_full_walk_every
            .or(pair.warm_start_full_walk_every)
            .unwrap_or(DEFAULT_WARM_START_FULL_WALK_EVERY),
        max_cursor_age: Duration::from_secs(
            input
                .warm_start_max_cursor_age_secs
                .or(pair.warm_start_max_cursor_age_secs)
                .unwrap_or(DEFAULT_WARM_START_MAX_CURSOR_AGE_SECS),
        ),
        force_full_walk: input.force_full_walk,
    };
    // Every local-filesystem path a user can hand us goes through `expand_tilde` first: the
    // daemon runs shell-less (systemd unit, GUI spawn), so nothing else ever expands `~` on its
    // behalf. `remote_root` is deliberately excluded — it is a Drive-side path where `~` has no
    // meaning.
    let local_root = expand_tilde(
        input
            .local_root
            .or(pair.local_root)
            .ok_or_else(|| boxed_error("missing required --local-root or config local_root"))?,
        "local_root",
    )?;
    let remote_root = input
        .remote_root
        .or(pair.remote_root)
        .ok_or_else(|| boxed_error("missing required --remote-root or config remote_root"))?;
    // Both resolved before the struct literal below (which moves `local_root`), and both through
    // the same `effective_state_path` the pair-collision check uses: a relative override joins
    // under `local_root` (so it lands where `scan_options_from_config` ignores it), an absolute one
    // is used as-is, and the default is the per-root `.sync` path. One definition, so "which file
    // is this pair's index" cannot be answered two ways.
    let db_path = effective_state_path(
        &local_root,
        input.db_path.or(pair.db_path),
        "db_path",
        default_state_db_path,
    )?;
    let lockfile_path = effective_state_path(
        &local_root,
        input.lockfile_path.or(pair.lockfile_path),
        "lockfile_path",
        default_lockfile_path,
    )?;
    // Resolved before the struct literal because both defaults are now fallible (#74) and must
    // stay LAZY: an explicit --socket-path must not fail because the /tmp fallback — which this
    // run never touches — is hostile.
    let socket_path = match input.socket_path.or(file_config.socket_path) {
        Some(path) => expand_tilde(path, "socket_path")?,
        None => default_socket_path()?,
    };
    let global_lock_path = default_global_lock_path()?;
    let default_command_policy = CommandPolicy::default();

    let config = DaemonConfig {
        local_root,
        remote_root,
        db_path,
        socket_path,
        lockfile_path,
        // Not user-overridable: the single-instance guarantee must key on a fixed per-user path so
        // it holds regardless of --socket-path / --local-root (see `default_global_lock_path`).
        global_lock_path,
        scan_interval: Duration::from_secs(
            input
                .scan_interval_secs
                .or(pair.scan_interval_secs)
                .unwrap_or(300)
                .max(1),
        ),
        proton_cli: input
            .proton_cli
            .or(file_config.proton_cli)
            .map(|path| expand_tilde(path, "proton_cli"))
            .transpose()?
            .unwrap_or_else(|| PathBuf::from("proton-drive")),
        proton_timeout: resolve_positive_duration_secs(
            input.proton_timeout_secs,
            file_config.proton_timeout_secs,
            default_command_policy.timeout.as_secs(),
            "proton_timeout_secs",
        )?,
        proton_list_attempts: resolve_positive_usize(
            input.proton_list_attempts,
            file_config.proton_list_attempts,
            default_command_policy.list_attempts,
            "proton_list_attempts",
        )?,
        download_batch_size: resolve_positive_usize(
            input.download_batch_size,
            pair.download_batch_size,
            DEFAULT_DOWNLOAD_BATCH_SIZE,
            "download_batch_size",
        )?,
        include_patterns: merge_patterns(input.include_patterns, pair.include_patterns),
        exclude_patterns: merge_patterns(input.exclude_patterns, pair.exclude_patterns),
        events_driven,
        // `0` is a valid, meaningful value here (periodic safety resync disabled), so it is *not*
        // clamped up to 1 the way a zero scan interval would be. The daemon treats 0 as "never
        // auto-resync" (see `effective_full_scan_every` in `daemon.rs`).
        events_full_scan_every: input
            .events_full_scan_every
            .or(pair.events_full_scan_every)
            .unwrap_or(DEFAULT_EVENTS_FULL_SCAN_EVERY),
        delete_approval_remote,
        delete_approval_local,
        warm_start,
        log_filter: resolve_log_filter(
            input.log_level.as_deref(),
            input.rust_log.as_deref(),
            file_config.log_level.as_deref(),
        )?,
        // Flag > file > default, and the default is `Trash`: a config that says nothing must not
        // unlink. Read off `pair` rather than `file_config` because the key describes one tree.
        local_delete_mode: input
            .local_delete_mode
            .or(pair.local_delete_mode)
            .unwrap_or_default(),
        conflict_naming: match input.conflict_suffix.or(pair.conflict_suffix) {
            Some(suffix) => ConflictNaming::new(&suffix)?,
            None => ConflictNaming::default(),
        },
    };
    validate_runtime_config(&config)?;

    Ok((config, dry_run))
}

/// The control socket a **client** should talk to, resolved with the daemon's own precedence:
/// explicit `--socket-path` > the config file's `socket_path` > the XDG default. Shared with
/// `proton-sync` so a file-configured socket is not invisible to the control CLI, which otherwise
/// had to repeat `--socket-path` on every invocation (#63).
///
/// The default stays **lazy**, like [`resolve_runtime_config`]: an explicit path must not fail
/// because the fail-closed shared-/tmp fallback (#74) — which this run never touches — is hostile.
/// A `config_path` is read whenever it is given, so a malformed config is reported rather than
/// silently ignored.
pub fn resolve_control_socket_path(
    explicit: Option<PathBuf>,
    config_path: Option<&Path>,
) -> AppResult<PathBuf> {
    let config_path = config_path
        .map(|path| expand_tilde(path.to_path_buf(), "--config"))
        .transpose()?;
    let from_file = load_file_config(config_path.as_ref())?.socket_path;
    match explicit.or(from_file) {
        Some(path) => {
            let path = expand_tilde(path, "socket_path")?;
            require_absolute_socket_path(&path)?;
            Ok(path)
        }
        None => default_socket_path(),
    }
}

/// The `(remote, local)` guard pair a config **file** asks for, from whichever of the two
/// spellings it uses.
///
/// **Both spellings in one file is an error**, not a precedence puzzle: `deletion_policy` and
/// `[delete_approval]` are one setting written two ways, and silently preferring either would make
/// a file whose two halves disagree run as something neither half says. Naming both keys is also
/// what lets a round-trip writer know which one it may rewrite.
///
/// An existing `[delete_approval]`-only file is unaffected: no `deletion_policy`, same two
/// booleans, same unset-means-protected default.
fn resolve_file_delete_approval(
    policy: Option<DeletionPolicy>,
    table: Option<&FileDeleteApproval>,
) -> AppResult<(bool, bool)> {
    match (policy, table) {
        (Some(_), Some(_)) => Err(boxed_error(
            "config sets both `deletion_policy` and `[delete_approval]`: they are two spellings \
             of one setting; keep whichever you prefer and delete the other",
        )),
        (Some(policy), None) => Ok(policy.directions()),
        (None, Some(table)) => Ok((table.remote.unwrap_or(true), table.local.unwrap_or(true))),
        (None, None) => Ok((true, true)),
    }
}

/// The `tracing` filter directive the daemon runs with.
///
/// Precedence is **`--log-level` > `RUST_LOG` > the config file's `log_level` > `info`**. The env
/// var sits in the middle deliberately: it is the documented ad-hoc verbosity control (`RUST_LOG`
/// is what every log-related doc in this repo tells you to set), so a config file must not
/// silently outrank it — but an explicit flag on this invocation must outrank both. An empty env
/// value counts as unset, matching `EnvFilter::try_from_default_env`.
///
/// **A configured value is validated and fatal; the env var is best-effort.** `--log-level` and
/// `log_level` are deliberate settings, and `EnvFilter` is permissive enough that a typo in one is
/// worse than an error: `inf0` parses fine as the *target* directive `inf0=trace`, which silences
/// the daemon completely while looking like it was accepted. So a configured value that is neither
/// a bare level nor an explicit `target=level` directive is refused (see
/// [`validate_log_directive`]). `RUST_LOG` keeps its historical forgiving behaviour — an ambient
/// env var must not stop a daemon from starting — and an unusable one falls through to the next
/// source instead of to `info`.
pub fn resolve_log_filter(
    flag: Option<&str>,
    env: Option<&str>,
    file: Option<&str>,
) -> AppResult<String> {
    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
    }
    if let Some(directive) = non_empty(flag) {
        validate_log_directive(directive, "--log-level")?;
        return Ok(directive.to_owned());
    }
    if let Some(directive) = non_empty(env)
        && tracing_subscriber::EnvFilter::try_new(directive).is_ok()
    {
        return Ok(directive.to_owned());
    }
    if let Some(directive) = non_empty(file) {
        validate_log_directive(directive, "log_level")?;
        return Ok(directive.to_owned());
    }
    Ok(DEFAULT_LOG_LEVEL.to_owned())
}

/// Levels a bare `log_level` may name. Anything else without an explicit `=` is a typo, not a
/// target filter — see [`resolve_log_filter`].
const LOG_LEVELS: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];

/// Syntax check plus the bare-word rule. `target=level` directives go through `EnvFilter`
/// untouched, so `proton_drive_sync_engine::transfer=warn` still works.
///
/// The bare-word rule is applied **per comma-separated segment**, not to the string as a whole.
/// `EnvFilter` reads a bare word it does not recognise as a *target* directive at `trace`, so
/// `inf0` silences the daemon while parsing cleanly — the typo this validation exists to catch.
/// A whole-string check misses it the moment it shares a list with a valid directive
/// (`inf0,proton_drive_sync_engine=debug`), which is the shape a user reaches for precisely when
/// they are hand-editing levels.
fn validate_log_directive(directive: &str, source: &str) -> AppResult<()> {
    tracing_subscriber::EnvFilter::try_new(directive)
        .map_err(|error| boxed_error(format!("invalid {source} `{directive}`: {error}")))?;
    for segment in directive.split(',') {
        let segment = segment.trim();
        if segment.is_empty() || segment.contains('=') {
            continue;
        }
        if !LOG_LEVELS.contains(&segment.to_ascii_lowercase().as_str()) {
            return Err(boxed_error(format!(
                "invalid {source} `{directive}`: `{segment}` is not one of {LOG_LEVELS:?}, and a \
                 bare word that is not a level is read as a target to log at `trace`. Use an \
                 explicit `target=level` directive such as `proton_drive_sync_engine=debug`"
            )));
        }
    }
    Ok(())
}

/// Every check a config **file** can fail that `toml::from_str::<FileConfig>` cannot see.
///
/// The serde shape only proves keys and types. Everything below is well-typed TOML that still
/// stops the daemon at startup, and a second reader of this file (the GUI's config writer) has no
/// way to know that without re-implementing the daemon's own rules — which is how the two ended up
/// disagreeing about `~` (#135). One function, both callers.
///
/// Scoped to what a *file* alone can decide: a missing `local_root` is not an error here (a flag
/// may supply it), and nothing on the filesystem is touched.
///
/// **Per-pair keys are checked per pair, whichever spelling the file uses** (#102): the implicit
/// `default` pair's values are the top-level keys, so an existing single-pair file is checked
/// exactly as it always was, while a `[[pair]]` file gets the same rules inside every table rather
/// than silently skipping them. The pair *structure* rules (both spellings, names, root collisions,
/// and the "more than one pair is not yet supported" refusal) come from the same [`resolve_pairs`]
/// and [`refuse_unsupported_pair_count`] the daemon starts on — the GUI must not be able to save a
/// file the daemon would then refuse to start on.
pub fn validate_file_config_text(text: &str) -> AppResult<()> {
    let config = parse_file_config(text)
        .map_err(|error| boxed_error(format!("failed to parse config: {error}")))?;
    let pairs = resolve_pairs(&config)?;
    refuse_unsupported_pair_count(&pairs)?;
    for pair in &pairs {
        validate_pair_file_values(pair)?;
    }
    resolve_log_filter(None, None, config.log_level.as_deref())?;
    if let Some(socket_path) = config.socket_path {
        // Expand FIRST: `socket_path = "~/run/x.sock"` is a path the daemon accepts, and checking
        // the literal would reject it as relative.
        require_absolute_socket_path(&expand_tilde(socket_path, "socket_path")?)?;
    }
    // The last local-filesystem path a config file can set, and the one this function did not
    // expand while `resolve_runtime_config` did (#339 round 2): the GUI writes this key from the
    // Settings Advanced tab, `ConfigDoc::save` validates only through here, and every packaged
    // unit launches the daemon flagless — so an unexpandable value written here is a daemon that
    // will not start, which is exactly the contract the per-pair `~` checks above exist for. A
    // bare command name (`proton-drive`, resolved through `PATH`) has no `~` component and passes
    // through untouched.
    // The last local-filesystem path a config file can set, and the one this function did not
    // expand while `resolve_runtime_config` did (#339 round 2): the GUI writes this key from the
    // Settings Advanced tab, `ConfigDoc::save` validates only through here, and every packaged
    // unit launches the daemon flagless — so an unexpandable value written here is a daemon that
    // will not start, which is exactly the contract the per-pair `~` checks above exist for. A
    // bare command name (`proton-drive`, resolved through `PATH`) has no `~` component and passes
    // through untouched.
    if let Some(proton_cli) = config.proton_cli {
        expand_tilde(proton_cli, "proton_cli")?;
    }
    resolve_positive_duration_secs(None, config.proton_timeout_secs, 1, "proton_timeout_secs")?;
    resolve_positive_usize(None, config.proton_list_attempts, 1, "proton_list_attempts")?;
    Ok(())
}

/// The per-pair checks a **file** can fail, for one pair.
///
/// These are the value-level rules, deliberately kept out of [`resolve_pairs`] — which both readers
/// share — because a CLI flag can change the answer to every one of them on the daemon's path:
/// `--local-root /x` over a file's `local_root = ""` starts fine today, and moving that refusal
/// earlier would break a config that works. The daemon still catches the *merged* value where it
/// always did (`validate_runtime_config`, `resolve_positive_usize`, `ConflictNaming::new`), so this
/// is the file-shaped half of one rule rather than a second rule.
///
/// The empty-root rule is here for a reason phase 1 created: `gui-core`'s `ConfigDoc::validate`
/// used to catch an empty root by reading the **top-level** key, which a `[[pair]]` file does not
/// have — so the check has to see pairs, which means it has to live here.
fn validate_pair_file_values(pair: &PairFileConfig) -> AppResult<()> {
    resolve_file_delete_approval(pair.deletion_policy, pair.delete_approval.as_ref())?;
    if let Some(suffix) = &pair.conflict_suffix {
        validate_conflict_suffix(suffix)?;
    }
    require_non_blank_root(pair.local_root.as_deref(), "local_root")?;
    require_non_blank_root(pair.remote_root.as_deref(), "remote_root")?;
    // `~` is refused HERE rather than in the structural layer (#339). The daemon refuses it on the
    // merged value — the one it will really open — and a flag can replace this one; but a file
    // reader has no flags, so `ConfigDoc::save` must still refuse a `~user` path the flagless daemon
    // would not start on. Per-pair by nature, so one place covers both spellings.
    for (field, value) in [
        ("local_root", pair.local_root.as_ref()),
        ("db_path", pair.db_path.as_ref()),
        ("lockfile_path", pair.lockfile_path.as_ref()),
    ] {
        if let Some(value) = value {
            expand_tilde(value.clone(), field)?;
        }
    }
    // The same-pair state collision, on the file's own values. `comparison_state_path` answers
    // `None` for a value this file does not place (a relative override with no `local_root` yet),
    // which is the case a flag still decides.
    let local_root = pair.local_root.as_deref().map(expand_tilde_for_comparison);
    if let (Some(db_path), Some(lockfile_path)) = (
        comparison_state_path(
            local_root.as_deref(),
            pair.db_path.as_deref(),
            default_state_db_path,
        ),
        comparison_state_path(
            local_root.as_deref(),
            pair.lockfile_path.as_deref(),
            default_lockfile_path,
        ),
    ) {
        require_distinct_state_paths(Some(&pair.name), &db_path, &lockfile_path)?;
    }
    resolve_positive_usize(None, pair.download_batch_size, 1, "download_batch_size")?;
    // Compiled against a throwaway root: the root only decides which paths are ignored, and this
    // call passes none. Same check `validate_runtime_config` makes.
    ScanOptions::new(
        Path::new("/"),
        &[],
        pair.include_patterns.as_deref().unwrap_or_default(),
        pair.exclude_patterns.as_deref().unwrap_or_default(),
        &ConflictNaming::default(),
    )
    .map_err(|error| boxed_error(format!("invalid scan filter configuration: {error}")))?;
    Ok(())
}

/// A root that is *present but blank* is always a mistake — a cleared settings field, not a default.
/// **Absent is not blank**: the daemon fills an absent `local_root` from a flag, and refusing one
/// here would reject a config that starts.
///
/// Blank means empty **or whitespace-only**, which is what a cleared form field actually produces;
/// a path whose bytes are not valid UTF-8 is never blank (nothing there is whitespace).
///
/// One definition for both the file check ([`validate_pair_file_values`], where the value is one a
/// flag may still override) and the merged runtime check ([`validate_runtime_config`], where it is
/// the value the daemon will actually use). The message is load-bearing: `gui-core`'s
/// `an_empty_root_is_refused_before_it_reaches_the_daemon` matches on it, and the same rule used to
/// live over there reading top-level keys — which a `[[pair]]` file does not have.
///
/// **Sharing it changed the daemon's answer in one case, and that is deliberate** (#339 §6): the
/// pre-#332 `validate_runtime_config` tested `as_os_str().is_empty()`, so a daemon whose merged
/// `local_root` was `"   "` started, and now does not. A directory named `"   "` is legal on Unix,
/// so this is a real behaviour change — a negligible one, since the value it refuses is what a
/// cleared settings field produces and not what anyone types on purpose.
fn require_non_blank_root(value: Option<&Path>, field_name: &str) -> AppResult<()> {
    if value.is_some_and(|path| path.to_str().is_some_and(|text| text.trim().is_empty())) {
        return Err(boxed_error(format!("{field_name} must not be empty")));
    }
    Ok(())
}

pub fn load_file_config(path: Option<&PathBuf>) -> AppResult<FileConfig> {
    let Some(path) = path else {
        return Ok(FileConfig::default());
    };
    let config = fs::read_to_string(path).map_err(|error| {
        boxed_error(format!("failed to read config {}: {error}", path.display()))
    })?;
    parse_file_config(&config).map_err(|error| {
        boxed_error(format!(
            "failed to parse config {}: {error}",
            path.display()
        ))
    })
}

pub fn parse_file_config(config: &str) -> Result<FileConfig, toml::de::Error> {
    toml::from_str(config)
}

fn resolve_path(local_root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        local_root.join(path)
    }
}

/// Expands a leading `~` component (`~` alone or `~/...`) to the user's home directory.
///
/// The daemon is spawned without a shell (systemd unit, GUI), so nothing expands `~` on its
/// behalf: a config value like `local_root = "~/Documents"` used to reach the filesystem layer
/// verbatim, making the daemon create and sync a directory literally named `~` under its working
/// directory — while the `proton-drive` CLI it shells *does* expand `~` in its arguments, so the
/// daemon and the CLI silently operated on two different trees (every download landed in the
/// expanded path and the daemon then found its literal-path scratch directory empty). Expanding
/// once here, at config resolution, keeps every consumer — direct fs calls, scratch directories,
/// and the shelled CLI — pointed at the same tree.
///
/// `~user` forms are rejected with an actionable error rather than guessed at; paths that do not
/// start with a `~` component pass through untouched.
///
/// Public because the daemon is not the only shell-less consumer of these values: the GUI reads the
/// same config file and joins `local_root` onto its own filesystem work (conflict scans, emblem
/// lookups, the free-space check). A second expansion written over there would be a second set of
/// `~user` semantics to keep in step, and the whole bug class this function exists for is two
/// components disagreeing about what one path means (#135).
pub fn expand_tilde(path: PathBuf, field_name: &str) -> AppResult<PathBuf> {
    expand_tilde_with_home(path, field_name, std::env::var_os("HOME"))
}

fn expand_tilde_with_home(
    path: PathBuf,
    field_name: &str,
    home: Option<OsString>,
) -> AppResult<PathBuf> {
    let mut components = path.components();
    let Some(Component::Normal(first)) = components.next() else {
        return Ok(path);
    };
    if first == OsStr::new("~") {
        let home = home.filter(|value| !value.is_empty()).ok_or_else(|| {
            boxed_error(format!(
                "cannot expand `~` in {field_name} `{}`: the HOME environment variable is not \
                 set (or is empty); use an absolute path instead",
                path.display()
            ))
        })?;
        let rest = components.as_path().to_path_buf();
        let mut expanded = PathBuf::from(home);
        if !rest.as_os_str().is_empty() {
            expanded.push(rest);
        }
        return Ok(expanded);
    }
    if first.as_encoded_bytes().starts_with(b"~") {
        return Err(boxed_error(format!(
            "cannot expand {field_name} `{}`: `~user` paths are not supported; use an absolute \
             path instead",
            path.display()
        )));
    }
    Ok(path)
}

fn resolve_positive_duration_secs(
    input_value: Option<u64>,
    file_value: Option<u64>,
    default_value: u64,
    field_name: &str,
) -> AppResult<Duration> {
    let value = input_value.or(file_value).unwrap_or(default_value);
    if value == 0 {
        return Err(boxed_error(format!(
            "{field_name} must be greater than zero"
        )));
    }
    Ok(Duration::from_secs(value))
}

fn resolve_positive_usize(
    input_value: Option<usize>,
    file_value: Option<usize>,
    default_value: usize,
    field_name: &str,
) -> AppResult<usize> {
    let value = input_value.or(file_value).unwrap_or(default_value);
    if value == 0 {
        return Err(boxed_error(format!(
            "{field_name} must be greater than zero"
        )));
    }
    Ok(value)
}

/// Unlike db_path/lockfile_path, a relative socket_path is *not* resolved under local_root — the
/// control socket must not live under the sync root (see `paths::default_socket_path`). Used
/// verbatim, a relative value would bind against the daemon's current working directory, so reject
/// it outright. The XDG default is always absolute; only explicit overrides hit this. Shared with
/// [`resolve_control_socket_path`] so the control CLI rejects the same values the daemon does.
fn require_absolute_socket_path(socket_path: &Path) -> AppResult<()> {
    if !socket_path.is_absolute() {
        return Err(boxed_error(format!(
            "socket_path must be an absolute path, got relative `{}`: a relative socket path \
             would resolve against the daemon's working directory; pass an absolute path (for \
             example under $XDG_RUNTIME_DIR)",
            socket_path.display()
        )));
    }
    Ok(())
}

fn validate_runtime_config(config: &DaemonConfig) -> AppResult<()> {
    require_non_blank_root(Some(&config.local_root), "local_root")?;
    require_non_blank_root(Some(&config.remote_root), "remote_root")?;
    require_absolute_socket_path(&config.socket_path)?;
    // On the values the daemon will really open, so a flag that creates the collision is caught and
    // a flag that fixes one written in the file is honoured (#339).
    require_distinct_state_paths(None, &config.db_path, &config.lockfile_path)?;
    ScanOptions::new(
        &config.local_root,
        std::slice::from_ref(&config.db_path),
        &config.include_patterns,
        &config.exclude_patterns,
        &config.conflict_naming,
    )
    .map_err(|error| boxed_error(format!("invalid scan filter configuration: {error}")))?;
    Ok(())
}

fn merge_patterns(cli_patterns: Vec<String>, config_patterns: Option<Vec<String>>) -> Vec<String> {
    if cli_patterns.is_empty() {
        config_patterns.unwrap_or_default()
    } else {
        cli_patterns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::DEFAULT_CONFLICT_SUFFIX;
    use tempfile::tempdir;

    #[test]
    fn config_file_supplies_required_daemon_options() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
db_path = "state/index.db"
socket_path = "/tmp/from-config.sock"
lockfile_path = "/tmp/from-config.lock"
scan_interval_secs = 42
proton_cli = "fake-proton-drive"
proton_timeout_secs = 17
proton_list_attempts = 4
include = ["Documents/**"]
exclude = ["**/*.tmp"]
dry_run = true
"#,
        )
        .expect("write config");

        let (config, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("sync-root"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/RemoteFolder"));
        assert_eq!(config.db_path, PathBuf::from("sync-root/state/index.db"));
        assert_eq!(config.scan_interval, Duration::from_secs(42));
        assert_eq!(config.proton_cli, PathBuf::from("fake-proton-drive"));
        assert_eq!(config.proton_timeout, Duration::from_secs(17));
        assert_eq!(config.proton_list_attempts, 4);
        assert_eq!(config.include_patterns, vec!["Documents/**"]);
        assert_eq!(config.exclude_patterns, vec!["**/*.tmp"]);
    }

    #[test]
    fn explicit_cli_values_override_config_file_values() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "config-root"
remote_root = "/Drive/Config"
proton_timeout_secs = 10
proton_list_attempts = 2
include = ["config/**"]
"#,
        )
        .expect("write config");

        let (config, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            local_root: Some(PathBuf::from("cli-root")),
            remote_root: Some(PathBuf::from("/Drive/Cli")),
            proton_timeout_secs: Some(22),
            proton_list_attempts: Some(5),
            dry_run: true,
            include_patterns: vec!["cli/**".to_owned()],
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("cli-root"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/Cli"));
        assert_eq!(config.proton_timeout, Duration::from_secs(22));
        assert_eq!(config.proton_list_attempts, 5);
        assert_eq!(config.include_patterns, vec!["cli/**"]);
    }

    #[test]
    fn relative_db_and_lockfile_overrides_resolve_under_local_root() {
        // A relative override for either state path must land under `local_root` (where the scanner
        // ignores it), consistent with each other — not relative to the process CWD.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("/home/me/Proton")),
            remote_root: Some(PathBuf::from("/Drive/X")),
            db_path: Some(PathBuf::from("state/custom.db")),
            lockfile_path: Some(PathBuf::from("state/custom.lock")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(
            config.db_path,
            PathBuf::from("/home/me/Proton/state/custom.db")
        );
        assert_eq!(
            config.lockfile_path,
            PathBuf::from("/home/me/Proton/state/custom.lock"),
            "a relative lockfile override must resolve under local_root like db_path"
        );
    }

    #[test]
    fn absolute_lockfile_override_is_used_as_is() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("/home/me/Proton")),
            remote_root: Some(PathBuf::from("/Drive/X")),
            lockfile_path: Some(PathBuf::from("/run/user/1000/custom.lock")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(
            config.lockfile_path,
            PathBuf::from("/run/user/1000/custom.lock")
        );
    }

    #[test]
    fn tilde_local_root_from_config_file_expands_to_the_home_directory() {
        // Regression: a hand- or GUI-written `local_root = "~/Documents"` used to be taken
        // literally, so the daemon synced into a directory actually named `~` while the shelled
        // `proton-drive` CLI expanded `~` and wrote downloads into the real home directory —
        // every download then failed with an empty scratch directory.
        let home = std::env::var_os("HOME").expect("HOME is set in the test environment");
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "~/Sync Root"
remote_root = "/Drive/RemoteFolder"
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        let expanded_root = PathBuf::from(&home).join("Sync Root");
        assert_eq!(config.local_root, expanded_root);
        assert!(
            config.db_path.starts_with(&expanded_root),
            "derived state paths must follow the expanded root, not the literal `~`: {}",
            config.db_path.display()
        );
        assert!(
            config.lockfile_path.starts_with(&expanded_root),
            "derived lockfile must follow the expanded root, not the literal `~`: {}",
            config.lockfile_path.display()
        );
    }

    #[test]
    fn tilde_expands_in_every_local_path_option_but_not_remote_root() {
        let home = std::env::var_os("HOME").expect("HOME is set in the test environment");
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("~/Proton")),
            remote_root: Some(PathBuf::from("/Drive/X")),
            db_path: Some(PathBuf::from("~/state/index.db")),
            lockfile_path: Some(PathBuf::from("~/state/daemon.lock")),
            socket_path: Some(PathBuf::from("~/run/daemon.sock")),
            proton_cli: Some(PathBuf::from("~/bin/proton-drive")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        let home = PathBuf::from(&home);
        assert_eq!(config.local_root, home.join("Proton"));
        assert_eq!(config.db_path, home.join("state/index.db"));
        assert_eq!(config.lockfile_path, home.join("state/daemon.lock"));
        assert_eq!(config.socket_path, home.join("run/daemon.sock"));
        assert_eq!(config.proton_cli, home.join("bin/proton-drive"));
        assert_eq!(
            config.remote_root,
            PathBuf::from("/Drive/X"),
            "remote_root is a Drive-side path where `~` has no meaning"
        );
    }

    #[test]
    fn tilde_username_local_root_is_rejected_with_an_actionable_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("~alice/Proton")),
            remote_root: Some(PathBuf::from("/Drive/X")),
            ..DaemonConfigInput::default()
        })
        .expect_err("`~user` paths should be rejected, not treated as literal directories");

        assert!(
            error
                .to_string()
                .contains("`~user` paths are not supported"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn tilde_config_flag_path_expands_to_the_home_directory() {
        // The `--config` path is a local-filesystem path like any other: a literal `~` must be
        // expanded before the file is read, not passed to `fs::read_to_string` verbatim. The
        // path below does not exist, so resolution fails — but the error must name the
        // *expanded* location.
        let home = std::env::var_os("HOME").expect("HOME is set in the test environment");
        let missing = "proton-sync-test-nonexistent-config.toml";

        let error = resolve_runtime_config(DaemonConfigInput {
            config: Some(PathBuf::from("~").join(missing)),
            ..DaemonConfigInput::default()
        })
        .expect_err("a nonexistent config file should fail to load");

        let message = error.to_string();
        let expanded = PathBuf::from(&home).join(missing);
        assert!(
            message.contains(&expanded.display().to_string()),
            "the error must reference the expanded config path, got: {message}"
        );
    }

    #[test]
    fn expand_tilde_with_home_covers_the_edge_shapes() {
        let home = Some(OsString::from("/home/tester"));

        // Bare `~` becomes the home directory itself, with no trailing component.
        assert_eq!(
            expand_tilde_with_home(PathBuf::from("~"), "local_root", home.clone())
                .expect("bare tilde"),
            PathBuf::from("/home/tester")
        );
        // A `~` that is not the leading component is a literal directory name.
        assert_eq!(
            expand_tilde_with_home(PathBuf::from("data/~/x"), "local_root", home.clone())
                .expect("inner tilde"),
            PathBuf::from("data/~/x")
        );
        // Absolute and plain relative paths pass through untouched.
        assert_eq!(
            expand_tilde_with_home(PathBuf::from("/opt/sync"), "local_root", home.clone())
                .expect("absolute"),
            PathBuf::from("/opt/sync")
        );
        assert_eq!(
            expand_tilde_with_home(PathBuf::from("sync-root"), "local_root", home.clone())
                .expect("relative"),
            PathBuf::from("sync-root")
        );
        // A filename that merely starts with `~` (e.g. an editor backup) is `~user` shaped and
        // rejected rather than silently misread.
        expand_tilde_with_home(PathBuf::from("~alice"), "local_root", home.clone())
            .expect_err("~user must be rejected");
        // Without HOME, expansion fails loudly instead of falling back to a literal `~`, and the
        // error names the offending field.
        let error = expand_tilde_with_home(PathBuf::from("~/x"), "db_path", None)
            .expect_err("missing HOME must be an error");
        let message = error.to_string();
        assert!(
            message.contains("HOME environment variable") && message.contains("db_path"),
            "unexpected error: {message}"
        );
        // An empty HOME is as good as unset.
        expand_tilde_with_home(PathBuf::from("~/x"), "db_path", Some(OsString::new()))
            .expect_err("empty HOME must be an error");
    }

    #[test]
    fn relative_socket_path_from_config_file_returns_targeted_config_error() {
        // Unlike db_path/lockfile_path, socket_path is never resolved under local_root (the socket
        // must not live in the sync root), so a relative value would bind against the daemon's
        // CWD. It must be rejected with an actionable error instead (#63).
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
socket_path = "run/daemon.sock"
"#,
        )
        .expect("write config");

        let error = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect_err("relative socket_path from the config file should fail");

        let message = error.to_string();
        assert!(
            message.contains("socket_path must be an absolute path")
                && message.contains("run/daemon.sock"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn relative_socket_path_from_cli_flag_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            socket_path: Some(PathBuf::from("relative.sock")),
            ..DaemonConfigInput::default()
        })
        .expect_err("relative socket_path from the CLI flag should fail");

        let message = error.to_string();
        assert!(
            message.contains("socket_path must be an absolute path")
                && message.contains("relative.sock"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn absolute_socket_path_override_is_used_as_is() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            socket_path: Some(PathBuf::from("/run/user/1000/custom.sock")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(
            config.socket_path,
            PathBuf::from("/run/user/1000/custom.sock")
        );
    }

    /// #63: a file-configured `socket_path` used to be invisible to `proton-sync`, so every
    /// invocation had to repeat `--socket-path`. The control CLI now resolves through the same
    /// precedence and the same validation as the daemon.
    #[test]
    fn the_control_socket_resolver_reads_the_config_file_and_the_flag_wins() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "/home/tester/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
socket_path = "/run/user/1000/from-file.sock"
"#,
        )
        .expect("write config");

        assert_eq!(
            resolve_control_socket_path(None, Some(&config_path)).expect("from file"),
            PathBuf::from("/run/user/1000/from-file.sock"),
        );
        assert_eq!(
            resolve_control_socket_path(
                Some(PathBuf::from("/run/user/1000/from-flag.sock")),
                Some(&config_path)
            )
            .expect("flag wins"),
            PathBuf::from("/run/user/1000/from-flag.sock"),
        );
    }

    #[test]
    fn the_control_socket_resolver_rejects_the_same_relative_paths_the_daemon_does() {
        // A client resolving a relative value against its OWN working directory would look for the
        // socket somewhere the daemon never bound it, and report an unreachable daemon.
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "/home/tester/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
socket_path = "run/daemon.sock"
"#,
        )
        .expect("write config");

        for error in [
            resolve_control_socket_path(None, Some(&config_path))
                .expect_err("relative file value must be rejected"),
            resolve_control_socket_path(Some(PathBuf::from("relative.sock")), None)
                .expect_err("relative flag value must be rejected"),
        ] {
            assert!(
                error
                    .to_string()
                    .contains("socket_path must be an absolute path"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn the_control_socket_resolver_reports_a_malformed_config_instead_of_ignoring_it() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(&config_path, "socket_pathh = \"/run/typo.sock\"\n").expect("write config");

        let error = resolve_control_socket_path(None, Some(&config_path))
            .expect_err("a config the daemon would reject must not be silently ignored");

        assert!(
            error.to_string().contains("failed to parse config"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn events_options_resolve_from_file_and_default_scan_interval() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
events_driven = false
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            !config.events_driven,
            "an explicit `events_driven = false` in the file must override the default-on"
        );
        assert_eq!(
            config.events_full_scan_every, DEFAULT_EVENTS_FULL_SCAN_EVERY,
            "an unset periodic-resync interval falls back to the default"
        );
    }

    #[test]
    fn events_driven_defaults_on_when_unset() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            config.events_driven,
            "event-driven sync is on by default when neither flag nor config value is set"
        );
    }

    #[test]
    fn explicit_no_events_driven_overrides_default_and_config_file() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "config-root"
remote_root = "/Drive/Config"
events_driven = true
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            no_events_driven: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            !config.events_driven,
            "--no-events-driven must override both the default-on and a config-file opt-in"
        );
    }

    #[test]
    fn explicit_cli_events_flag_and_interval_override_defaults() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            events_driven: true,
            events_full_scan_every: Some(5),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(config.events_driven);
        assert_eq!(config.events_full_scan_every, 5);
    }

    #[test]
    fn zero_events_full_scan_every_is_preserved_as_disabled() {
        // The periodic safety resync is opt-in: a configured 0 must be preserved (not clamped up to
        // 1) so the daemon reads it as "disabled" and stays purely event-driven after the startup
        // snapshot. `daemon::effective_full_scan_every` maps this 0 to `u64::MAX`.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            events_driven: true,
            events_full_scan_every: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(config.events_full_scan_every, 0);
    }

    #[test]
    fn events_full_scan_every_defaults_to_disabled() {
        // The shipped default disables the periodic resync entirely.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(config.events_full_scan_every, 0);
        assert_eq!(DEFAULT_EVENTS_FULL_SCAN_EVERY, 0);
    }

    #[test]
    fn warm_start_defaults_on_with_the_documented_bounds() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(config.warm_start.enabled, "warm start is on by default");
        assert_eq!(
            config.warm_start.full_walk_every,
            DEFAULT_WARM_START_FULL_WALK_EVERY
        );
        assert_eq!(
            config.warm_start.max_cursor_age,
            Duration::from_secs(DEFAULT_WARM_START_MAX_CURSOR_AGE_SECS)
        );
        assert!(!config.warm_start.force_full_walk);
    }

    #[test]
    fn no_warm_start_flag_disables_it_over_a_config_file_opt_in() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
warm_start = true
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            no_warm_start: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            !config.warm_start.enabled,
            "--no-warm-start must override a config-file opt-in"
        );
    }

    #[test]
    fn warm_start_bounds_resolve_flag_over_file_over_default() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
warm_start_full_walk_every = 10
warm_start_max_cursor_age_secs = 3600
"#,
        )
        .expect("write config");

        // File values beat the defaults.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path.clone()),
            ..DaemonConfigInput::default()
        })
        .expect("file config");
        assert_eq!(config.warm_start.full_walk_every, 10);
        assert_eq!(config.warm_start.max_cursor_age, Duration::from_secs(3600));

        // Explicit flags beat the file.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            warm_start_full_walk_every: Some(50),
            warm_start_max_cursor_age_secs: Some(120),
            force_full_walk: true,
            ..DaemonConfigInput::default()
        })
        .expect("flag config");
        assert_eq!(config.warm_start.full_walk_every, 50);
        assert_eq!(config.warm_start.max_cursor_age, Duration::from_secs(120));
        assert!(
            config.warm_start.force_full_walk,
            "--full-walk sets the one-shot force flag"
        );
    }

    #[test]
    fn zero_warm_start_bounds_are_preserved_as_disabled_sentinels() {
        // Both bounds treat 0 as "disabled" (never periodic full walk / no age gate), so — like
        // events_full_scan_every — a configured 0 must be preserved rather than clamped up to 1.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            warm_start_full_walk_every: Some(0),
            warm_start_max_cursor_age_secs: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert_eq!(config.warm_start.full_walk_every, 0);
        assert_eq!(config.warm_start.max_cursor_age, Duration::ZERO);
    }

    #[test]
    fn delete_approval_defaults_on_for_both_directions_when_unset() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            config.delete_approval_remote && config.delete_approval_local,
            "the delete-approval guard must default ON for both directions"
        );
    }

    #[test]
    fn no_delete_approval_flag_disables_both_directions() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            no_delete_approval: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(!config.delete_approval_remote);
        assert!(!config.delete_approval_local);
    }

    #[test]
    fn config_file_delete_approval_table_sets_directions_independently() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
[delete_approval]
remote = false
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(
            !config.delete_approval_remote,
            "an explicit remote = false in the file must disable the remote-delete guard"
        );
        assert!(
            config.delete_approval_local,
            "an unset local direction must stay protected by default"
        );
    }

    #[test]
    fn typoed_key_inside_delete_approval_table_fails_to_load() {
        // serde's `deny_unknown_fields` on `FileConfig` does not recurse into nested tables, so
        // the nested struct must carry its own deny — otherwise `remot = false` would be silently
        // dropped and the guard would stay on despite the user's intent (#64).
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
[delete_approval]
remot = false
"#,
        )
        .expect("write config");

        let error = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect_err("a typoed [delete_approval] key must fail to load");

        let message = error.to_string();
        assert!(
            message.contains("failed to parse config"),
            "error must point at the config file: {message}"
        );
        assert!(
            message.contains("unknown field `remot`"),
            "error must name the unknown key so the typo is findable: {message}"
        );
    }

    #[test]
    fn no_delete_approval_flag_overrides_a_config_file_that_enabled_it() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
[delete_approval]
remote = true
local = true
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            no_delete_approval: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(!config.delete_approval_remote);
        assert!(!config.delete_approval_local);
    }

    #[test]
    fn every_policy_spelling_round_trips_through_serde_and_from_str() {
        // Three spellings of one enum — the TOML value, the CLI value, and `as_str` — and a config
        // round trip has to write back what it read. A rename that moved only one of them would
        // otherwise show up as a config that silently reverts to the default.
        for policy in DeletionPolicy::ALL {
            let toml_text = format!("deletion_policy = \"{}\"\n", policy.as_str());
            let parsed = parse_file_config(&toml_text).expect("parse");
            assert_eq!(parsed.deletion_policy, Some(policy), "{policy:?} via TOML");
            assert_eq!(
                policy.as_str().parse::<DeletionPolicy>().expect("from_str"),
                policy,
                "{policy:?} via FromStr"
            );
            assert_eq!(policy.to_string(), policy.as_str());
            let (remote, local) = policy.directions();
            assert_eq!(DeletionPolicy::from_directions(remote, local), policy);
        }
        assert!(
            "asks_every_time".parse::<DeletionPolicy>().is_err(),
            "an unknown spelling must be an error, not the default"
        );
    }

    #[test]
    fn deletion_policy_resolves_to_the_same_pair_as_the_table_spelling() {
        for policy in DeletionPolicy::ALL {
            let directory = tempdir().expect("tempdir");
            let config_path = directory.path().join("proton-sync.toml");
            fs::write(
                &config_path,
                format!(
                    "local_root = \"sync-root\"\nremote_root = \"/Drive/R\"\ndeletion_policy = \"{}\"\n",
                    policy.as_str()
                ),
            )
            .expect("write config");

            let (config, _) = resolve_runtime_config(DaemonConfigInput {
                config: Some(config_path),
                ..DaemonConfigInput::default()
            })
            .expect("runtime config");

            assert_eq!(
                (config.delete_approval_remote, config.delete_approval_local),
                policy.directions(),
                "{policy:?} must resolve to the two booleans the guard has always run on"
            );
        }
    }

    #[test]
    fn a_config_using_both_deletion_spellings_is_refused_and_names_both() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
deletion_policy = "never"
[delete_approval]
remote = true
"#,
        )
        .expect("write config");

        let error = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect_err("two spellings of one setting must not resolve to a guessed precedence");

        let message = error.to_string();
        assert!(
            message.contains("deletion_policy") && message.contains("delete_approval"),
            "the error must name both keys so the fix is obvious: {message}"
        );
    }

    #[test]
    fn an_existing_delete_approval_config_is_unaffected_by_the_new_key() {
        // THE COMPATIBILITY CLAIM, asserted rather than assumed: adding `deletion_policy` must not
        // change what any config written before it means. Every direction combination, through the
        // old spelling, plus the unset-is-protected default.
        for (remote, local) in [(true, true), (true, false), (false, true), (false, false)] {
            let directory = tempdir().expect("tempdir");
            let config_path = directory.path().join("proton-sync.toml");
            fs::write(
                &config_path,
                format!(
                    "local_root = \"sync-root\"\nremote_root = \"/Drive/R\"\n[delete_approval]\nremote = {remote}\nlocal = {local}\n"
                ),
            )
            .expect("write config");

            let (config, _) = resolve_runtime_config(DaemonConfigInput {
                config: Some(config_path),
                ..DaemonConfigInput::default()
            })
            .expect("an existing delete_approval config must keep resolving");

            assert_eq!(
                (config.delete_approval_remote, config.delete_approval_local),
                (remote, local)
            );
        }
    }

    #[test]
    fn the_deletion_policy_flag_beats_the_file_and_no_delete_approval_beats_the_flag() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
[delete_approval]
remote = true
local = true
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path.clone()),
            deletion_policy: Some(DeletionPolicy::OnlyPermanent),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert_eq!(
            (config.delete_approval_remote, config.delete_approval_local),
            (false, true),
            "an explicit flag outranks the file, including its table spelling"
        );

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            deletion_policy: Some(DeletionPolicy::AskEveryTime),
            no_delete_approval: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert!(
            !config.delete_approval_remote && !config.delete_approval_local,
            "--no-delete-approval stays the blunt override it has always been"
        );
    }

    #[test]
    fn log_filter_precedence_is_flag_then_env_then_file() {
        assert_eq!(
            resolve_log_filter(Some("debug"), Some("trace"), Some("warn")).expect("flag"),
            "debug",
            "an explicit flag outranks an ambient RUST_LOG"
        );
        assert_eq!(
            resolve_log_filter(None, Some("trace"), Some("warn")).expect("env"),
            "trace",
            "RUST_LOG outranks the file: it is what every log doc in this repo tells you to set"
        );
        assert_eq!(
            resolve_log_filter(None, None, Some("warn")).expect("file"),
            "warn"
        );
        assert_eq!(
            resolve_log_filter(None, None, None).expect("default"),
            DEFAULT_LOG_LEVEL
        );
        assert_eq!(
            resolve_log_filter(None, Some(""), Some("warn")).expect("empty env"),
            "warn",
            "an empty RUST_LOG counts as unset, matching EnvFilter::try_from_default_env"
        );
        assert_eq!(
            resolve_log_filter(None, None, Some("proton_drive_sync_engine::transfer=warn"))
                .expect("target directive"),
            "proton_drive_sync_engine::transfer=warn",
            "the per-module directives the daemon docs recommend must still be configurable"
        );
    }

    #[test]
    fn a_configured_log_level_is_fatal_while_a_broken_rust_log_is_not() {
        // `EnvFilter` is permissive: `inf0` parses happily as the TARGET directive `inf0=trace`,
        // which silences the daemon while looking accepted. That is fine to shrug off for an
        // ambient env var and not fine for a setting someone deliberately wrote.
        for source in ["--log-level", "log_level"] {
            let (flag, file) = if source == "--log-level" {
                (Some("inf0"), None)
            } else {
                (None, Some("inf0"))
            };
            let error = resolve_log_filter(flag, None, file)
                .expect_err("a bare non-level must be refused")
                .to_string();
            assert!(
                error.contains(source),
                "the error must name its source: {error}"
            );
        }
        assert_eq!(
            resolve_log_filter(None, Some("!!!"), Some("warn")).expect("bad env falls through"),
            "warn",
            "an unusable RUST_LOG must not stop the daemon starting"
        );
    }

    #[test]
    fn a_typo_hides_inside_a_directive_list_too() {
        // The bare-word rule is per segment. Sharing a list with a valid `target=level` is exactly
        // how a hand-edited level ends up looking legitimate: `EnvFilter` accepts the whole string,
        // and `inf0` still becomes a target logged at `trace` while the daemon's own output stops.
        for directive in [
            "inf0,proton_drive_sync_engine=debug",
            "proton_drive_sync_engine=debug,inf0",
            "info,warn,inf0",
        ] {
            let error = resolve_log_filter(None, None, Some(directive))
                .expect_err("a bare non-level in any segment must be refused")
                .to_string();
            assert!(
                error.contains("inf0"),
                "the error must name the offending segment, not just the whole value: {error}"
            );
        }
        // Real multi-directive values still pass, including a bare level leading the list.
        for directive in [
            "info,proton_drive_sync_engine=debug",
            "proton_drive_sync_engine::transfer=warn,proton_drive_sync_engine=info",
            "warn",
        ] {
            resolve_log_filter(None, None, Some(directive))
                .unwrap_or_else(|error| panic!("`{directive}` must stay valid: {error}"));
        }
    }

    #[test]
    fn conflict_suffix_resolves_flag_over_file_over_default() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
conflict_suffix = "from-file"
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/R")),
            ..DaemonConfigInput::default()
        })
        .expect("default");
        assert_eq!(config.conflict_naming.suffix(), DEFAULT_CONFLICT_SUFFIX);

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path.clone()),
            ..DaemonConfigInput::default()
        })
        .expect("file");
        assert_eq!(config.conflict_naming.suffix(), "from-file");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            conflict_suffix: Some("from-flag".to_owned()),
            ..DaemonConfigInput::default()
        })
        .expect("flag");
        assert_eq!(config.conflict_naming.suffix(), "from-flag");
    }

    #[test]
    fn an_unusable_conflict_suffix_from_the_file_returns_a_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/R")),
            conflict_suffix: Some("nested/suffix".to_owned()),
            ..DaemonConfigInput::default()
        })
        .expect_err("a suffix holding a separator would place the sidecar in another directory");

        assert!(
            error.to_string().contains("path separator"),
            "unexpected error: {error}"
        );
    }

    /// The GUI's config writer calls this instead of re-implementing the daemon's rules; every
    /// entry below is well-typed TOML the daemon still exits on, and each one used to be
    /// invisible until startup.
    #[test]
    fn validate_file_config_text_catches_what_the_serde_shape_cannot() {
        for (text, needle) in [
            ("socket_path = \"run/daemon.sock\"\n", "absolute path"),
            ("log_level = \"inf0\"\n", "invalid log_level"),
            ("conflict_suffix = \"a/b\"\n", "path separator"),
            ("proton_timeout_secs = 0\n", "greater than zero"),
            ("proton_list_attempts = 0\n", "greater than zero"),
            ("download_batch_size = 0\n", "greater than zero"),
            ("exclude = [\"[\"]\n", "invalid scan filter"),
            (
                "deletion_policy = \"never\"\n[delete_approval]\nlocal = false\n",
                "two spellings of one setting",
            ),
            ("frobnicate = 1\n", "failed to parse config"),
        ] {
            let error = validate_file_config_text(text)
                .expect_err("must be refused")
                .to_string();
            assert!(error.contains(needle), "expected {needle:?} in: {error}");
        }
        // And the shapes it must NOT refuse: an absent key is the daemon's own default, `0` is a
        // meaningful sentinel on the events/warm-start knobs, and `~` is expanded before the
        // socket path is checked for absoluteness.
        validate_file_config_text(
            "socket_path = \"~/run/x.sock\"\nevents_full_scan_every = 0\n\
             warm_start_full_walk_every = 0\nwarm_start_max_cursor_age_secs = 0\n\
             [delete_approval]\nremote = false\n",
        )
        .expect("a valid config must pass");
    }

    #[test]
    fn explicit_no_dry_run_overrides_config_file_dry_run() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "config-root"
remote_root = "/Drive/Config"
dry_run = true
"#,
        )
        .expect("write config");

        let (_, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            no_dry_run: true,
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");

        assert!(!dry_run);
    }

    #[test]
    fn invalid_include_glob_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            include_patterns: vec!["[".to_owned()],
            ..DaemonConfigInput::default()
        })
        .expect_err("invalid include glob should fail");

        assert!(
            error
                .to_string()
                .contains("invalid scan filter configuration"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn zero_proton_timeout_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_timeout_secs: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect_err("zero Proton timeout should fail");

        assert_eq!(
            error.to_string(),
            "proton_timeout_secs must be greater than zero"
        );
    }

    #[test]
    fn zero_proton_list_attempts_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_list_attempts: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect_err("zero Proton list attempts should fail");

        assert_eq!(
            error.to_string(),
            "proton_list_attempts must be greater than zero"
        );
    }

    #[test]
    fn empty_local_root_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::new()),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty local root should fail");

        assert_eq!(error.to_string(), "local_root must not be empty");
    }

    #[test]
    fn empty_remote_root_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::new()),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty remote root should fail");

        assert_eq!(error.to_string(), "remote_root must not be empty");
    }

    #[test]
    fn download_batch_size_resolves_flag_over_file_over_default() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            r#"
local_root = "sync-root"
remote_root = "/Drive/RemoteFolder"
download_batch_size = 5
"#,
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/RemoteFolder")),
            ..DaemonConfigInput::default()
        })
        .expect("default config");
        assert_eq!(
            config.download_batch_size, 25,
            "unset download_batch_size resolves to the default"
        );

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path.clone()),
            ..DaemonConfigInput::default()
        })
        .expect("file config");
        assert_eq!(
            config.download_batch_size, 5,
            "file value beats the default"
        );

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            download_batch_size: Some(9),
            ..DaemonConfigInput::default()
        })
        .expect("flag config");
        assert_eq!(
            config.download_batch_size, 9,
            "explicit flag beats the file"
        );
    }

    #[test]
    fn zero_download_batch_size_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/RemoteFolder")),
            download_batch_size: Some(0),
            ..DaemonConfigInput::default()
        })
        .expect_err("zero download batch size should fail");

        assert_eq!(
            error.to_string(),
            "download_batch_size must be greater than zero"
        );
    }

    // ---- #102 phase 1: the config understands folder pairs -------------------------------------

    /// A `FileConfig` with **every** key set. The literal is exhaustive on purpose — no
    /// `..Default::default()` — so adding a field to `FileConfig` is a build failure here until the
    /// key is classified, which is the half of the `KeyScope` guarantee the compiler can enforce.
    fn file_config_with_every_key_set() -> FileConfig {
        FileConfig {
            local_root: Some(PathBuf::from("/local")),
            remote_root: Some(PathBuf::from("/Drive/remote")),
            db_path: Some(PathBuf::from("/local/.sync/index.db")),
            socket_path: Some(PathBuf::from("/run/user/1000/proton-sync.sock")),
            lockfile_path: Some(PathBuf::from("/local/.sync/proton-sync.lock")),
            scan_interval_secs: Some(300),
            proton_cli: Some(PathBuf::from("/usr/bin/proton-drive")),
            proton_timeout_secs: Some(300),
            proton_list_attempts: Some(3),
            download_batch_size: Some(25),
            include_patterns: Some(vec!["Documents/**".to_owned()]),
            exclude_patterns: Some(vec!["*.tmp".to_owned()]),
            dry_run: Some(false),
            events_driven: Some(true),
            events_full_scan_every: Some(0),
            warm_start: Some(true),
            warm_start_full_walk_every: Some(30),
            warm_start_max_cursor_age_secs: Some(604_800),
            delete_approval: Some(FileDeleteApproval {
                remote: Some(true),
                local: Some(true),
            }),
            deletion_policy: Some(DeletionPolicy::AskEveryTime),
            local_delete_mode: Some(LocalDeleteMode::Permanent),
            log_level: Some("info".to_owned()),
            conflict_suffix: Some(DEFAULT_CONFLICT_SUFFIX.to_owned()),
            pair: Some(vec![file_pair_with_every_key_set()]),
        }
    }

    /// As above, for one `[[pair]]` table.
    fn file_pair_with_every_key_set() -> FilePair {
        FilePair {
            name: "documents".to_owned(),
            local_root: Some(PathBuf::from("/local")),
            remote_root: Some(PathBuf::from("/Drive/remote")),
            db_path: Some(PathBuf::from("/local/.sync/index.db")),
            lockfile_path: Some(PathBuf::from("/local/.sync/proton-sync.lock")),
            scan_interval_secs: Some(300),
            download_batch_size: Some(25),
            include_patterns: Some(vec!["Documents/**".to_owned()]),
            exclude_patterns: Some(vec!["*.tmp".to_owned()]),
            dry_run: Some(false),
            events_driven: Some(true),
            events_full_scan_every: Some(0),
            warm_start: Some(true),
            warm_start_full_walk_every: Some(30),
            warm_start_max_cursor_age_secs: Some(604_800),
            delete_approval: Some(FileDeleteApproval {
                remote: Some(true),
                local: Some(true),
            }),
            deletion_policy: Some(DeletionPolicy::AskEveryTime),
            local_delete_mode: Some(LocalDeleteMode::Permanent),
            conflict_suffix: Some(DEFAULT_CONFLICT_SUFFIX.to_owned()),
        }
    }

    /// The keys a struct *has*, read from the struct itself so a hand-written list cannot drift.
    ///
    /// **Through JSON, not TOML** (#339). TOML has no null, so a field left `None` was omitted from
    /// the serialized table — and therefore from both sides of every comparison built on this. The
    /// compiler forces a new field to be mentioned in the exhaustive fixtures below, and `None` —
    /// the natural value for a fresh `Option` — made it invisible again: a real, parseable,
    /// unclassified key passed `every_file_config_key_is_classified_exactly_once`,
    /// `a_pair_table_hosts_exactly_the_per_pair_keys` and rule 1 alike. JSON has null, so the key
    /// survives whatever the value is, which is the property the guards need. Nothing else about
    /// these types is JSON — the file is TOML and stays TOML.
    fn top_level_keys<T: Serialize>(value: &T) -> Vec<String> {
        let object = serde_json::to_value(value).expect("serialize");
        let mut keys: Vec<String> = object
            .as_object()
            .expect("an object")
            .keys()
            .map(String::from)
            .collect();
        keys.sort();
        keys
    }

    fn spellings(scope: Option<KeyScope>) -> Vec<String> {
        let mut names: Vec<String> = ConfigKey::ALL
            .into_iter()
            .filter(|key| scope.is_none_or(|wanted| key.scope() == wanted))
            .map(|key| key.spelling().to_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_file_config_key_is_classified_exactly_once() {
        // THE OTHER HALF of the `KeyScope` guarantee. `scope()` being an exhaustive match makes a
        // new *variant* answer the per-pair question; this makes a new *field* have a variant at
        // all. Both directions are checked, because either gap is silent: an unclassified field is
        // a key rule 1 never notices at the top level, and a variant with no field is a key nothing
        // can ever set.
        //
        // `pair` is the one deliberate exclusion: it is the container for per-pair keys, not a
        // setting with a scope of its own.
        let mut expected = spellings(None);
        expected.push("pair".to_owned());
        expected.sort();
        assert_eq!(
            top_level_keys(&file_config_with_every_key_set()),
            expected,
            "every FileConfig key must appear in ConfigKey::ALL exactly once (and vice versa)"
        );
        let mut distinct = spellings(None);
        distinct.dedup();
        assert_eq!(distinct, spellings(None), "two keys share a spelling");
    }

    #[test]
    fn a_pair_table_hosts_exactly_the_per_pair_keys() {
        // A key classified per-pair that a `[[pair]]` table cannot express would be a key the user
        // can only set daemon-wide — i.e. the classification would be a claim the file shape does
        // not honour. `name` is the pair's identity, not a setting, so it is the one extra.
        let mut expected = spellings(Some(KeyScope::Pair));
        expected.push("name".to_owned());
        expected.sort();
        assert_eq!(top_level_keys(&file_pair_with_every_key_set()), expected);
    }

    #[test]
    fn a_config_that_says_nothing_about_deletions_trashes_rather_than_unlinks() {
        // THE WHOLE CHANGE, at the layer a user's existing file goes through. Every config written
        // before this key existed says nothing, so the default is what those installs get — and it
        // must be the recoverable one, or nothing about the removed warnings is safe.
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            "local_root = \"sync-root\"\nremote_root = \"/Drive/RemoteFolder\"\n",
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert_eq!(config.local_delete_mode, LocalDeleteMode::Trash);
    }

    #[test]
    fn the_local_delete_mode_flag_beats_the_file_which_beats_the_default() {
        let directory = tempdir().expect("tempdir");
        let config_path = directory.path().join("proton-sync.toml");
        fs::write(
            &config_path,
            "local_root = \"sync-root\"\nremote_root = \"/Drive/RemoteFolder\"\n\
             local_delete_mode = \"permanent\"\n",
        )
        .expect("write config");

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path.clone()),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert_eq!(
            config.local_delete_mode,
            LocalDeleteMode::Permanent,
            "the file beats the default"
        );

        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(config_path),
            local_delete_mode: Some(LocalDeleteMode::Trash),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert_eq!(
            config.local_delete_mode,
            LocalDeleteMode::Trash,
            "an explicit flag beats the file"
        );
    }

    #[test]
    fn an_unrecognised_local_delete_mode_is_refused_naming_the_key_and_both_choices() {
        // The daemon must not start on an assumed default. Nothing in the repo pinned this wording
        // for `deletion_policy` either — it comes from the serde derive, and this is what says the
        // derive's message actually answers the user's question rather than merely failing.
        let error = validate_file_config_text(
            "local_root = \"/tmp/x\"\nremote_root = \"/Drive/X\"\nlocal_delete_mode = \"bin\"\n",
        )
        .expect_err("an unknown mode must be refused")
        .to_string();
        assert!(error.contains("local_delete_mode"), "{error}");
        assert!(error.contains("bin"), "{error}");
        for mode in LocalDeleteMode::ALL {
            assert!(error.contains(mode.as_str()), "{error} must name {mode}");
        }
    }

    #[test]
    fn two_pair_tables_may_choose_different_local_delete_modes() {
        // The key is `KeyScope::Pair`, and this is the layer that has to prove it: two folder pairs
        // may reasonably disagree about whether deletions are recoverable. It has to run here
        // rather than through `resolve_runtime_config`, because `refuse_unsupported_pair_count`
        // rejects any two-pair config before the daemon ever sees one.
        let config = parse_file_config(
            "[[pair]]\nname = \"docs\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             local_delete_mode = \"trash\"\n\
             [[pair]]\nname = \"scratch\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
             local_delete_mode = \"permanent\"\n",
        )
        .expect("parse");
        let pairs = resolve_pairs(&config).expect("resolve pairs");
        assert_eq!(
            pairs
                .iter()
                .map(|pair| (pair.name.as_str(), pair.local_delete_mode))
                .collect::<Vec<_>>(),
            vec![
                ("docs", Some(LocalDeleteMode::Trash)),
                ("scratch", Some(LocalDeleteMode::Permanent)),
            ]
        );
    }

    #[test]
    fn setting_the_mode_at_the_top_level_beside_a_pair_table_is_refused_as_two_spellings() {
        // Rule 1 (ADR 0005 §2) reaches the new key for free BECAUSE it reads the classification
        // rather than a list — but only if `ConfigKey::LocalDeleteMode` is classified `Pair` and
        // `key_present` answers for it. This is what proves both, from the outside.
        let config = parse_file_config(
            "local_delete_mode = \"permanent\"\n\
             [[pair]]\nname = \"docs\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
        )
        .expect("parse");
        let error = resolve_pairs(&config)
            .expect_err("a per-pair key at the top level beside [[pair]] must be refused")
            .to_string();
        assert!(error.contains("local_delete_mode"), "{error}");
    }

    #[test]
    fn the_shared_proton_client_keys_cannot_be_per_pair() {
        // NOT a preference. One process holds one `ProtonDriveClient`, and one client is one
        // `CliGate` (#23) — N clients would be N gates, i.e. no serialization of the `proton-drive`
        // children at all. These three construct that client, so per-pair values for them would
        // have to move `CommandPolicy` off the client and onto every call.
        for key in [
            ConfigKey::ProtonCli,
            ConfigKey::ProtonTimeoutSecs,
            ConfigKey::ProtonListAttempts,
        ] {
            assert_eq!(
                key.scope(),
                KeyScope::Daemon,
                "{} lives on the shared client and cannot be per-pair (#23)",
                key.spelling()
            );
        }
    }

    #[test]
    fn a_file_with_no_pair_table_is_one_implicit_pair_called_default() {
        // The permanent shape of every config written before multi-pair: nothing is rewritten,
        // nothing is migrated, and the top-level keys ARE that pair's values.
        let config =
            parse_file_config("local_root = \"/x\"\nremote_root = \"/Drive/x\"\n").expect("parse");
        let pairs = resolve_pairs(&config).expect("resolve pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].name, DEFAULT_PAIR_NAME);
        assert_eq!(pairs[0].local_root.as_deref(), Some(Path::new("/x")));
        assert_eq!(pairs[0].remote_root.as_deref(), Some(Path::new("/Drive/x")));

        // And an entirely empty file is still one pair — the daemon then reports the missing root,
        // not a missing pair.
        let empty = parse_file_config("").expect("parse");
        let pairs = resolve_pairs(&empty).expect("resolve pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].name, DEFAULT_PAIR_NAME);
    }

    /// What `resolve_runtime_config` answers for one config file and one set of flags, as a string
    /// — `Ok` **or** `Err`, because "the two spellings agree" has to cover the refusals too.
    ///
    /// Debug-string equality because `DaemonConfig` has no `PartialEq` and this must compare EVERY
    /// field, including ones a later phase adds.
    fn resolve_spelling(text: &str, input: &DaemonConfigInput) -> String {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(&path, text).expect("write config");
        let input = DaemonConfigInput {
            config: Some(path),
            ..input.clone()
        };
        match resolve_runtime_config(input) {
            Ok((config, dry_run)) => format!("Ok({config:?}, dry_run = {dry_run})"),
            Err(error) => format!("Err({error})"),
        }
    }

    /// The same config written the two supposedly equivalent ways, resolved with the same flags.
    fn assert_spellings_agree(daemon_wide: &str, per_pair_body: &str, input: &DaemonConfigInput) {
        let flat = format!("{daemon_wide}{per_pair_body}");
        let tabled = format!("{daemon_wide}\n[[pair]]\nname = \"docs\"\n{per_pair_body}");
        assert_eq!(
            resolve_spelling(&flat, input),
            resolve_spelling(&tabled, input),
            "a `[[pair]]` file and the equivalent top-level file must resolve to the same answer, \
             for {per_pair_body:?} with {input:?}"
        );
    }

    #[test]
    fn a_top_level_file_and_the_equivalent_pair_table_resolve_identically() {
        // The whole point of routing every per-pair value through ONE projection: the implicit pair
        // is not a second code path.
        //
        // The fixture is deliberately hostile (#339). It used to be absolute, distinct, non-`~`
        // paths, which is exactly the shape that cannot see the divergence it was written to pin:
        // the structural layer (`resolve_pairs` -> `validate_pair_roots`) ran `expand_tilde` and
        // `effective_state_path` — both fallible — *before* any flag was merged and on the
        // `[[pair]]` arm only, so a `[[pair]]` file was refused over values the daemon would never
        // use while the byte-identical top-level file started. Every case below therefore carries a
        // `~`, a relative path, or colliding state paths, in both spellings.
        // An explicit socket keeps the comparison off the XDG default, which is env-dependent.
        let daemon_wide = "socket_path = \"/tmp/pair-equivalence.sock\"\nlog_level = \"debug\"\n\
             proton_cli = \"/usr/bin/proton-drive\"\nproton_timeout_secs = 17\n\
             proton_list_attempts = 4\n";
        let rich_body = "local_root = \"/local/docs\"\nremote_root = \"/Drive/Docs\"\n\
             db_path = \"state/index.db\"\nlockfile_path = \"state/lock\"\n\
             scan_interval_secs = 42\ndownload_batch_size = 7\ninclude = [\"Documents/**\"]\n\
             exclude = [\"**/*.tmp\"]\ndry_run = true\nevents_driven = false\n\
             events_full_scan_every = 9\nwarm_start = false\nwarm_start_full_walk_every = 11\n\
             warm_start_max_cursor_age_secs = 13\ndeletion_policy = \"only_permanent\"\n\
             local_delete_mode = \"permanent\"\n\
             conflict_suffix = \"cloud-copy\"\n";

        for (body, input) in [
            // The original fixture, unchanged: absolute, distinct, no `~`.
            (rich_body, DaemonConfigInput::default()),
            // A `~user` root a flag replaces. `~user` rather than `~/` because it does not depend
            // on `HOME` being set, and it is the value the reproduction in #339 used.
            (
                "local_root = \"~bob/x\"\nremote_root = \"/Drive/Docs\"\n",
                DaemonConfigInput {
                    local_root: Some(PathBuf::from("/tmp/real")),
                    ..DaemonConfigInput::default()
                },
            ),
            // The same value with NO flag to rescue it: both spellings must refuse it, and with the
            // same words.
            (
                "local_root = \"~bob/x\"\nremote_root = \"/Drive/Docs\"\n",
                DaemonConfigInput::default(),
            ),
            // State paths that collide, both replaced by flags: the cleanest proof that the layer
            // was refusing over values it would never use.
            (
                "local_root = \"/local/docs\"\nremote_root = \"/Drive/Docs\"\n\
                 db_path = \"/tmp/same\"\nlockfile_path = \"/tmp/same\"\n",
                DaemonConfigInput {
                    db_path: Some(PathBuf::from("/tmp/a")),
                    lockfile_path: Some(PathBuf::from("/tmp/b")),
                    ..DaemonConfigInput::default()
                },
            ),
            // And the same collision with no flags: one file cannot be both this pair's index and
            // its lockfile, in EITHER spelling (the top-level one never had that check).
            (
                "local_root = \"/local/docs\"\nremote_root = \"/Drive/Docs\"\n\
                 db_path = \"/tmp/same\"\nlockfile_path = \"/tmp/same\"\n",
                DaemonConfigInput::default(),
            ),
            // A relative root, with relative state paths under it.
            (
                "local_root = \"relative-root\"\nremote_root = \"/Drive/Docs\"\n\
                 db_path = \"state/index.db\"\nlockfile_path = \"state/lock\"\n",
                DaemonConfigInput::default(),
            ),
            // A `~/` root, which both spellings must expand the same way (or refuse the same way
            // when `HOME` is unset — the comparison is on the answer, not on success).
            (
                "local_root = \"~/docs\"\nremote_root = \"/Drive/Docs\"\n\
                 db_path = \"~/docs/state/index.db\"\n",
                DaemonConfigInput::default(),
            ),
        ] {
            assert_spellings_agree(daemon_wide, body, &input);
        }
    }

    #[test]
    fn more_than_one_pair_is_refused_at_startup_and_at_save_time() {
        // Phase 1 lands the SHAPE so phases 2-4 have something to build against; the capability
        // needs a `PairRuntime` per pair, a wire selector and a scheduler. Refused on BOTH readers'
        // paths: `ConfigDoc::save` never writes a config the daemon would refuse to start on.
        let text = "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n";
        let error = validate_file_config_text(text).expect_err("two pairs must be refused");
        let message = error.to_string();
        assert!(message.contains("not yet supported"), "got {message}");
        assert!(
            message.contains("`a`") && message.contains("`b`"),
            "the refusal must name the pairs it found, got {message}"
        );

        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(&path, text).expect("write config");
        let error = resolve_runtime_config(DaemonConfigInput {
            config: Some(path),
            ..DaemonConfigInput::default()
        })
        .expect_err("two pairs must be refused at startup");
        assert!(
            error.to_string().contains("not yet supported"),
            "got {error}"
        );
    }

    #[test]
    fn one_pair_table_is_accepted_and_runs_as_that_pair() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(
            &path,
            "[[pair]]\nname = \"documents\"\nlocal_root = \"/local/docs\"\n\
             remote_root = \"/Drive/Docs\"\nexclude = [\"*.tmp\"]\n",
        )
        .expect("write config");
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert_eq!(config.local_root, PathBuf::from("/local/docs"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/Docs"));
        assert_eq!(config.exclude_patterns, vec!["*.tmp"]);
        // The state paths still default under the pair's own root.
        assert_eq!(
            config.db_path,
            default_state_db_path(Path::new("/local/docs"))
        );
    }

    #[test]
    fn a_top_level_per_pair_key_beside_a_pair_table_is_refused_naming_both() {
        // The `deletion_policy` + `[delete_approval]` precedent: one setting written two ways has
        // no defensible precedence, and refusing is what lets a round-trip writer know which
        // spelling it may rewrite.
        let error = validate_file_config_text(
            "local_root = \"/x\"\nexclude = [\"*.tmp\"]\n\
             \n[[pair]]\nname = \"a\"\nremote_root = \"/Drive/a\"\n",
        )
        .expect_err("both spellings must be refused");
        let message = error.to_string();
        assert!(message.contains("`local_root`"), "got {message}");
        assert!(message.contains("`exclude_patterns`"), "got {message}");
        assert!(message.contains("[[pair]]"), "got {message}");
    }

    #[test]
    fn a_daemon_wide_key_beside_a_pair_table_is_exactly_where_it_belongs() {
        // The other side of rule 1, and the reason it reads `ConfigKey::scope` rather than "any key
        // at all": the socket, the log level and the three shared-client keys have nowhere else to
        // go.
        validate_file_config_text(
            "socket_path = \"/tmp/x.sock\"\nlog_level = \"debug\"\n\
             proton_cli = \"/usr/bin/proton-drive\"\nproton_timeout_secs = 30\n\
             proton_list_attempts = 2\n\
             \n[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
        )
        .expect("daemon-wide keys belong at the top level even with `[[pair]]` tables");
    }

    #[test]
    fn an_explicitly_empty_pair_list_is_refused_rather_than_read_as_the_default_pair() {
        // `pair = []` is a statement. Treating it as "one implicit pair" would make an explicit
        // "sync nothing" silently mean its opposite.
        let error =
            validate_file_config_text("pair = []\n").expect_err("an empty pair list is refused");
        assert!(error.to_string().contains("syncs nothing"), "got {error}");
    }

    #[test]
    fn nested_local_roots_are_refused_because_the_inner_pairs_sync_state_would_upload() {
        // THE CONCRETE FAILURE, not a tidiness rule: `index::is_sync_state_path` matches only the
        // FIRST component of a relative path (a deeper `.sync` is ordinary user data), so the inner
        // pair's `.sync` index, lockfile and sidecars would be scanned and uploaded to Proton Drive
        // as the outer pair's files.
        let nested = "[[pair]]\nname = \"outer\"\nlocal_root = \"/home/me/Sync\"\n\
             remote_root = \"/Drive/Outer\"\n\
             \n[[pair]]\nname = \"inner\"\nlocal_root = \"/home/me/Sync/Photos\"\n\
             remote_root = \"/Drive/Inner\"\n";
        let error = validate_file_config_text(nested).expect_err("nested local roots are refused");
        let message = error.to_string();
        assert!(message.contains("inside"), "got {message}");
        assert!(message.contains(".sync"), "got {message}");
        assert!(
            !message.contains("not yet supported"),
            "a genuinely broken multi-pair file must say what is wrong with it rather than be \
             masked by the count gate, got {message}"
        );

        // The other direction (outer declared second) is the same refusal.
        let reversed = "[[pair]]\nname = \"inner\"\nlocal_root = \"/home/me/Sync/Photos\"\n\
             remote_root = \"/Drive/Inner\"\n\
             \n[[pair]]\nname = \"outer\"\nlocal_root = \"/home/me/Sync\"\n\
             remote_root = \"/Drive/Outer\"\n";
        let error =
            validate_file_config_text(reversed).expect_err("nested local roots are refused");
        assert!(error.to_string().contains("a parent of"), "got {error}");

        // A sibling that merely shares a name PREFIX is not nested: `/home/me/Sync2` is not inside
        // `/home/me/Sync`, and a byte-prefix check would wrongly refuse it. Such a file is refused
        // by the COUNT gate instead, which is how this test tells the two apart.
        let siblings = "[[pair]]\nname = \"one\"\nlocal_root = \"/home/me/Sync\"\n\
             remote_root = \"/Drive/One\"\n\
             \n[[pair]]\nname = \"two\"\nlocal_root = \"/home/me/Sync2\"\n\
             remote_root = \"/Drive/Two\"\n";
        let error = validate_file_config_text(siblings)
            .expect_err("two pairs are still refused by the count gate");
        assert!(
            error.to_string().contains("not yet supported"),
            "sibling roots must reach the count gate, not the nesting check, got {error}"
        );
    }

    #[test]
    fn identical_or_nested_remote_roots_are_refused_across_both_spellings_of_a_drive_path() {
        // The mirror of the local rule: two pairs over one remote subtree plan opposing actions for
        // it. `/Drive/X` and `Drive/X` name ONE Drive location (`proton.rs::normalize_remote_path`
        // strips the root either way), so the leading separator must not hide a collision.
        for (a, b) in [
            ("/Drive/Docs", "/Drive/Docs"),
            ("/Drive/Docs", "/Drive/Docs/Reports"),
            ("/Drive/Docs", "Drive/Docs/Reports"),
            ("Drive/Docs", "/Drive/Docs"),
        ] {
            let text = format!(
                "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"{a}\"\n\
                 \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"{b}\"\n"
            );
            let error = validate_file_config_text(&text)
                .expect_err("colliding remote roots must be refused");
            let message = error.to_string();
            assert!(message.contains("remote_root"), "{a} vs {b}: got {message}");
            // The message must name the paths the user WROTE, not the comparison keys they were
            // reduced to: a file saying `/Drive/X` was told about `Drive/X`, which is a path it
            // does not contain (#339).
            assert!(
                message.contains(&format!("`{a}`")) && message.contains(&format!("`{b}`")),
                "the refusal must quote what the file says, {a} vs {b}: got {message}"
            );
        }
    }

    #[test]
    fn two_pairs_sharing_an_index_or_lockfile_are_refused_before_the_lock_is_taken() {
        // Otherwise `LockGuard::acquire` reports "another daemon is already running" — true, because
        // `flock` treats two descriptors on one inode as independent even in one process, and
        // incomprehensible. So the CONFIG has to say it, before any lock is taken.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             lockfile_path = \"/tmp/shared.lock\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
             lockfile_path = \"/tmp/shared.lock\"\n",
        )
        .expect_err("a shared lockfile is refused");
        assert!(error.to_string().contains("lockfile_path"), "got {error}");

        // A shared index is the same refusal, and an explicit override that collides with another
        // pair's DEFAULT is caught too — the check compares effective paths, not written ones.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
             db_path = \"/a/.sync/sync_index.db\"\n",
        )
        .expect_err("a shared index is refused");
        assert!(error.to_string().contains("db_path"), "got {error}");
    }

    #[test]
    fn pair_names_are_unique_ignoring_ascii_case() {
        // #298's rule one layer up: names are matched byte-exactly on the wire, so `Photos` and
        // `photos` would be two pairs a person cannot tell apart while a selector resolves to
        // exactly one. The ambiguity is removed at startup rather than resolved arbitrarily.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"Photos\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"photos\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n",
        )
        .expect_err("names differing only in case are refused");
        assert!(
            error.to_string().contains("without regard to"),
            "got {error}"
        );

        // Including when the duplicated name is `default`, which is also reserved for the first
        // table: two tables both called it are two names that are the same, and the reservation's
        // advice ("move its table first") would produce two `default`s (#339 round 2).
        let error = validate_file_config_text(
            "[[pair]]\nname = \"default\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"Default\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n",
        )
        .expect_err("two `default`s are refused");
        assert!(
            error.to_string().contains("without regard to"),
            "a duplicate is a duplicate before it is a position error, got {error}"
        );
    }

    #[test]
    fn a_pair_name_must_be_a_safe_command_argument() {
        // A name is a CLI argument and a wire selector, so it must never need quoting and never
        // look like a path.
        //
        // The bare forms are the ones the charset admitted while the doc comment claimed it
        // prevented them (#339): `.` and `..` ARE path components, and `-h` / `--pair` are option
        // syntax, all spelled entirely in `[A-Za-z0-9._-]`.
        for name in [
            "",
            "my docs",
            "../escape",
            "docs/reports",
            "caf\u{e9}",
            "a*b",
            ".",
            "..",
            "-h",
            "--pair",
            "-",
        ] {
            let text = format!(
                "[[pair]]\nname = \"{name}\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n"
            );
            assert!(
                validate_file_config_text(&text).is_err(),
                "`{name}` must not be a valid pair name"
            );
        }
        for name in ["docs", "My-Docs", "photos_2024", "a.b", "x"] {
            let text = format!(
                "[[pair]]\nname = \"{name}\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n"
            );
            validate_file_config_text(&text)
                .unwrap_or_else(|error| panic!("`{name}` must be a valid pair name: {error}"));
        }
        let long = "a".repeat(PAIR_NAME_MAX_LEN + 1);
        validate_file_config_text(&format!(
            "[[pair]]\nname = \"{long}\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n"
        ))
        .expect_err("an over-long name is refused");

        // The charset is checked BEFORE the length, because `name.len()` is bytes: a 40-character
        // accented name is 80 of them and used to be refused for being "longer than 64 characters"
        // (#339). Once the charset holds, every character is one byte and the two agree.
        let accented = "\u{e9}".repeat(40);
        let error = validate_file_config_text(&format!(
            "[[pair]]\nname = \"{accented}\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n"
        ))
        .expect_err("an accented name is refused");
        assert!(
            !error.to_string().contains("longer than"),
            "a 40-character name must not be reported as too long, got {error}"
        );
    }

    #[test]
    fn a_per_pair_value_rule_applies_inside_a_pair_table_too() {
        // Every value check the top-level spelling has always had must reach the table spelling, or
        // moving a key into `[[pair]]` would silently switch its validation off.
        //
        // Each case is a WHOLE table, not a suffix appended to a fixture: an earlier version of this
        // test appended `local_root = ""` under a fixture that already set `local_root = "/a"`, so
        // TOML's duplicate-key error refused it and the test passed while the rule it names was
        // deleted. Every case here must fail for the reason it is written for, which is what the
        // per-case `needle` pins.
        for (table, needle) in [
            (
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\nexclude = [\"[\"]",
                "invalid scan filter",
            ),
            (
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 conflict_suffix = \"bad/suffix\"",
                "path separator",
            ),
            (
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 download_batch_size = 0",
                "download_batch_size must be greater than zero",
            ),
            (
                "name = \"a\"\nlocal_root = \"\"\nremote_root = \"/Drive/a\"",
                "local_root must not be empty",
            ),
            (
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"   \"",
                "remote_root must not be empty",
            ),
            (
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 deletion_policy = \"never\"\n\n[pair.delete_approval]\nremote = false",
                "two spellings of one setting",
            ),
        ] {
            let text = format!("[[pair]]\n{table}\n");
            let error = validate_file_config_text(&text)
                .expect_err("a per-pair value rule must reach inside a pair table");
            assert!(
                error.to_string().contains(needle),
                "expected {needle:?} for {table:?}, got {error}"
            );
        }
    }

    #[test]
    fn a_typo_inside_a_pair_table_is_refused_rather_than_ignored() {
        // serde's `deny_unknown_fields` on `FileConfig` does not recurse into nested tables, so
        // `FilePair` repeats it — otherwise `exclud = [...]` would be silently ignored and the user
        // would sync files they told us to skip (#64).
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             exclud = [\"*.tmp\"]\n",
        )
        .expect_err("a typo inside a pair table is refused");
        assert!(error.to_string().contains("exclud"), "got {error}");
    }

    #[test]
    fn every_pair_refusal_renders_as_a_sentence() {
        // Asserts the RENDERED message, not the escape. A `\`-newline continuation written by a
        // patch script through a non-raw string loses the backslash and bakes the next line's
        // indentation into the literal — a long run of spaces mid-sentence that `cargo fmt`,
        // `clippy -D warnings` and every substring assertion above are all blind to
        // (docs/agent-notes/python-patch-scripts-and-rust-string-continuations.md).
        let refusals = [
            "pair = []\n",
            "local_root = \"/x\"\n\n[[pair]]\nname = \"a\"\nremote_root = \"/Drive/a\"\n",
            "[[pair]]\nname = \"a b\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"A\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/a/inner\"\nremote_root = \"/Drive/b\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             lockfile_path = \"/tmp/one.lock\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
             lockfile_path = \"/tmp/one.lock\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n",
            // #339's refusals, each of which is also a sentence.
            "[[pair]]\nname = \".\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
            "[[pair]]\nname = \"-h\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"default\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
             db_path = \"/a/index.db\"\n",
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"/tmp/one\"\n\
             lockfile_path = \"/tmp/one\"\n",
            "local_root = \"~bob/x\"\nremote_root = \"/Drive/a\"\n",
        ];
        for text in refusals {
            let message = validate_file_config_text(text)
                .expect_err("each of these is a refusal")
                .to_string();
            assert!(
                !message.contains("  "),
                "a run of spaces means a lost `\\` continuation: {message:?}"
            );
        }
    }

    #[test]
    fn a_pair_table_reads_the_same_kebab_case_aliases_the_top_level_does() {
        // A hand-written config may legitimately use either spelling anywhere, and `gui-core`'s
        // `key_in_use` writes back the one the file already uses.
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(
            &path,
            "[[pair]]\nname = \"a\"\nlocal-root = \"/local\"\nremote-root = \"/Drive/a\"\n\
             scan-interval-secs = 61\ndownload-batch-size = 3\nconflict-suffix = \"cloud\"\n\
             warm-start-full-walk-every = 5\nevents-full-scan-every = 7\ndry-run = true\n",
        )
        .expect("write config");
        let (config, dry_run) = resolve_runtime_config(DaemonConfigInput {
            config: Some(path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert!(dry_run);
        assert_eq!(config.local_root, PathBuf::from("/local"));
        assert_eq!(config.scan_interval, Duration::from_secs(61));
        assert_eq!(config.download_batch_size, 3);
        assert_eq!(config.events_full_scan_every, 7);
        assert_eq!(config.warm_start.full_walk_every, 5);
    }

    #[test]
    fn a_flag_still_amends_the_single_pair() {
        // `--local-root` and friends keep meaning "the single pair" whichever spelling the file
        // uses — a flag cannot say WHICH pair it amends, which is a question phase 4 has to answer
        // when it lifts the count gate.
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(
            &path,
            "[[pair]]\nname = \"a\"\nlocal_root = \"/from-file\"\nremote_root = \"/Drive/File\"\n",
        )
        .expect("write config");
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(path),
            local_root: Some(PathBuf::from("/from-flag")),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert_eq!(config.local_root, PathBuf::from("/from-flag"));
        assert_eq!(config.remote_root, PathBuf::from("/Drive/File"));
    }

    #[test]
    fn a_pair_tables_delete_approval_seeds_the_guard_like_the_top_level_one() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(
            &path,
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             deletion_policy = \"only_permanent\"\n",
        )
        .expect("write config");
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            config: Some(path),
            ..DaemonConfigInput::default()
        })
        .expect("runtime config");
        assert!(!config.delete_approval_remote);
        assert!(config.delete_approval_local);
    }

    #[test]
    fn a_file_value_a_flag_replaces_is_never_refused_in_either_spelling() {
        // Byte-identical behaviour: the file-shaped refusals deliberately do NOT run on the
        // daemon's merge path, because `--local-root /x` over a file's `local_root = ""` starts
        // today and phase 1 must not change that. The merged value is still checked
        // (`validate_runtime_config`), which is what catches the case no flag rescues.
        //
        // Run in BOTH spellings (#339). This test states the flag-maskability principle in its own
        // comment and used to exercise the top-level arm only — which is precisely the arm where
        // the principle was not violated. `~bob/x` is the second case because it is the one the
        // structural layer refused before any flag was merged.
        for (body, expected_error) in [
            (
                "local_root = \"\"\nremote_root = \"/Drive/x\"\n",
                "local_root must not be empty",
            ),
            (
                "local_root = \"~bob/x\"\nremote_root = \"/Drive/x\"\n",
                "cannot expand local_root `~bob/x`: `~user` paths are not supported; use an \
                 absolute path instead",
            ),
        ] {
            for text in [
                body.to_owned(),
                format!("[[pair]]\nname = \"docs\"\n{body}"),
            ] {
                let flagged = DaemonConfigInput {
                    local_root: Some(PathBuf::from("/from-flag")),
                    ..DaemonConfigInput::default()
                };
                assert!(
                    resolve_spelling(&text, &flagged).starts_with("Ok("),
                    "a flag still rescues a file value the daemon will never use, in {text:?}"
                );
                assert_eq!(
                    resolve_spelling(&text, &DaemonConfigInput::default()),
                    format!("Err({expected_error})"),
                    "with no flag the merged value is refused, in {text:?}"
                );
            }
        }
    }

    #[test]
    fn the_key_set_a_classification_guard_reads_keeps_a_field_left_unset() {
        // The hole #339 found in `every_file_config_key_is_classified_exactly_once` and
        // `a_pair_table_hosts_exactly_the_per_pair_keys`: `top_level_keys` serialized the fixture
        // through `toml::Value`, and TOML has no null — so a field left `None` was omitted from
        // BOTH sides of the comparison. The compiler forces a new field to be MENTIONED in the
        // exhaustive fixture, and `None`, the natural value for a fresh `Option`, made it invisible
        // again: a real, parseable, unclassified key passed every guard.
        //
        // No test can name a field that does not exist yet, so what is pinned here is the property
        // that made it invisible — the key set must not depend on the values.
        assert_eq!(
            top_level_keys(&FileConfig::default()),
            top_level_keys(&file_config_with_every_key_set()),
            "a FileConfig field must appear in the key set whatever its value"
        );
        assert_eq!(
            top_level_keys(&FilePair::default()),
            top_level_keys(&file_pair_with_every_key_set()),
            "a FilePair field must appear in the key set whatever its value"
        );
    }

    #[test]
    fn every_local_path_a_file_can_set_is_refused_by_both_readers_or_neither() {
        // The never-brick contract, as a sweep rather than a list (#339 round 2). `ConfigDoc::save`
        // validates ONLY through `validate_file_config_text`, and every packaged unit launches the
        // daemon flagless (`ExecStart=/usr/bin/proton-syncd --config …`), so a key the file reader
        // waves through and the daemon refuses is a config the GUI can write and the daemon will
        // not start on. `proton_cli` was that key: expanded by `resolve_runtime_config` and by
        // nothing on the file's path.
        //
        // `~user` because it is the value `expand_tilde` refuses without depending on the
        // environment. Each case is written in BOTH spellings — a per-pair key inside the table, a
        // daemon-wide key at the top level beside it, which is where each belongs.
        let roots = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n";
        for (key, flat, tabled) in [
            (
                "local_root",
                "local_root = \"~bob/x\"\nremote_root = \"/Drive/a\"\n".to_owned(),
                "[[pair]]\nname = \"a\"\nlocal_root = \"~bob/x\"\nremote_root = \"/Drive/a\"\n"
                    .to_owned(),
            ),
            (
                "db_path",
                format!("{roots}db_path = \"~bob/x\"\n"),
                format!("[[pair]]\nname = \"a\"\n{roots}db_path = \"~bob/x\"\n"),
            ),
            (
                "lockfile_path",
                format!("{roots}lockfile_path = \"~bob/x\"\n"),
                format!("[[pair]]\nname = \"a\"\n{roots}lockfile_path = \"~bob/x\"\n"),
            ),
            (
                "socket_path",
                format!("{roots}socket_path = \"~bob/x.sock\"\n"),
                format!("socket_path = \"~bob/x.sock\"\n\n[[pair]]\nname = \"a\"\n{roots}"),
            ),
            (
                "proton_cli",
                format!("{roots}proton_cli = \"~bob/pd\"\n"),
                format!("proton_cli = \"~bob/pd\"\n\n[[pair]]\nname = \"a\"\n{roots}"),
            ),
        ] {
            for text in [flat, tabled] {
                assert!(
                    validate_file_config_text(&text).is_err(),
                    "the file reader must refuse an unexpandable {key}, in {text:?}"
                );
                assert!(
                    resolve_spelling(&text, &DaemonConfigInput::default()).starts_with("Err("),
                    "the daemon must refuse an unexpandable {key}, in {text:?}"
                );
            }
        }

        // A bare command name is not a path and must pass both readers untouched, or a
        // `PATH`-resolved `proton-drive` would stop working.
        let bare = format!("{roots}proton_cli = \"proton-drive\"\n");
        validate_file_config_text(&bare).expect("a PATH-resolved command name is not a `~` path");
        assert!(resolve_spelling(&bare, &DaemonConfigInput::default()).starts_with("Ok("));
    }

    #[test]
    fn a_local_root_written_with_a_leading_dot_slash_is_the_same_root() {
        // `A` and `./A` are one directory. `remote_root_comparison_key` dropped `Component::CurDir`
        // and the local side did not, so the same normalization existed in one of the two places
        // that needed it — the repo's dominant bug shape, sitting inside the function this fix
        // declares safe (#339 round 2). Only a LEADING `./` can do this: `Path::components` drops a
        // non-leading `.`, and `Path` compares component-wise.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"sync\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"./sync\"\nremote_root = \"/Drive/b\"\n",
        )
        .expect_err("`sync` and `./sync` are one root");
        let message = error.to_string();
        assert!(message.contains("the same path as"), "got {message}");
        assert!(
            !message.contains("not yet supported"),
            "the collision must be named rather than masked by the count gate, got {message}"
        );

        // A root that is nothing but `.` keeps its literal form: reduced to the empty path it would
        // be a prefix of everything, and an absolute root would be refused as "inside" it.
        validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \".\"\nremote_root = \"/Drive/a\"\n\
             db_path = \"/tmp/a.db\"\nlockfile_path = \"/tmp/a.lock\"\n",
        )
        .expect("an absolute state path is not inside a relative root");
    }

    #[test]
    fn a_file_shaped_refusal_says_which_pair_it_is_reading() {
        // The same-pair state collision is the one refusal that moved from a layer that had the
        // pair name to one that did not. The file reader is per pair and still has it; the merge
        // path is one pair and has none to give (#339 round 2).
        let error = validate_file_config_text(
            "[[pair]]\nname = \"photos\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             db_path = \"/tmp/one\"\nlockfile_path = \"/tmp/one\"\n",
        )
        .expect_err("one file cannot be both the index and the lockfile");
        assert!(error.to_string().contains("`photos`"), "got {error}");
    }

    #[test]
    fn a_pairs_state_paths_may_not_sit_inside_another_pairs_local_root() {
        // #339. The nesting rule was written root-vs-root, and its own stated consequence was
        // reachable around it: `ScanOptions::new` is handed only THIS pair's `db_path`, and
        // `index::is_sync_state_path` ignores only a TOP-LEVEL `.sync`, so pair `a` would scan and
        // upload pair `b`'s live SQLite index and lockfile — verbatim the failure the rule cites as
        // its reason. Nothing compared a state path against another pair's `local_root`.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"/home/me/A\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/home/me/B\"\nremote_root = \"/Drive/b\"\n\
             db_path = \"/home/me/A/index.db\"\n",
        )
        .expect_err("a state path inside another pair's root is refused");
        let message = error.to_string();
        assert!(message.contains("db_path"), "got {message}");
        assert!(message.contains("local_root"), "got {message}");
        assert!(
            !message.contains("not yet supported"),
            "a genuinely broken multi-pair file must say what is wrong with it rather than be \
             masked by the count gate, got {message}"
        );

        // Same shape, one layer down: `validate_pair_roots` used to collect state paths only inside
        // `if let Some(local_root)`, so two pairs with no root at all (legal — a flag may supply it)
        // and one shared absolute `db_path` were never compared with each other.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nremote_root = \"/Drive/a\"\ndb_path = \"/tmp/shared.db\"\n\
             \n[[pair]]\nname = \"b\"\nremote_root = \"/Drive/b\"\ndb_path = \"/tmp/shared.db\"\n",
        )
        .expect_err("two rootless pairs sharing an index are refused");
        assert!(error.to_string().contains("db_path"), "got {error}");
        assert!(
            !error.to_string().contains("not yet supported"),
            "got {error}"
        );
    }

    #[test]
    fn one_file_cannot_be_both_a_pairs_index_and_its_lockfile() {
        // The same-pair half of the state-path rule, and the one a FLAG can change the answer to
        // (`--db-path` / `--lockfile-path` replace both values), so it lives where the values that
        // will really be used are: the file-shaped check for a reader with no flags, and
        // `validate_runtime_config` for the merged value. The `[[pair]]` spelling had it and the
        // top-level spelling never did (#339).
        for body in [
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"/tmp/one\"\n\
             lockfile_path = \"/tmp/one\"\n",
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"state/x\"\n\
             lockfile_path = \"state/x\"\n",
        ] {
            for text in [body.to_owned(), format!("[[pair]]\nname = \"a\"\n{body}")] {
                let error = validate_file_config_text(&text)
                    .expect_err("one file cannot be both the index and the lockfile");
                assert!(
                    error.to_string().contains("lockfile_path"),
                    "in {text:?}: got {error}"
                );
            }
        }
    }

    #[test]
    fn an_explicit_pair_named_default_must_be_the_pair_an_unqualified_client_addresses() {
        // #339, and the `all` decision (ADR 0005 §4) applied to the other sentinel: `default` is
        // the name of the pair a request that names none addresses (§2 rule 6 / §7 — the first
        // table), so a LATER table called `default` gives one selector two answers. The name is not
        // refused outright: the GUI's promote-to-`[[pair]]` rewrite (§7) names the pre-existing pair
        // exactly that.
        validate_file_config_text(
            "[[pair]]\nname = \"default\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
        )
        .expect("the first pair may be called `default`, which is what it already is");

        for name in ["default", "Default"] {
            let error = validate_file_config_text(&format!(
                "[[pair]]\nname = \"photos\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 \n[[pair]]\nname = \"{name}\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n"
            ))
            .expect_err("a later pair may not be called `default`");
            let message = error.to_string();
            assert!(message.contains("default"), "got {message}");
            assert!(
                !message.contains("not yet supported"),
                "the collision must be named rather than masked by the count gate, got {message}"
            );
        }
    }
}
