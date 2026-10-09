use crate::daemon::{
    DEFAULT_WARM_START_FULL_WALK_EVERY, DEFAULT_WARM_START_MAX_CURSOR_AGE_SECS, DaemonConfig,
    WarmStartConfig,
};
use crate::index::ScanOptions;
use crate::paths::{
    default_global_lock_path, default_lockfile_path, default_socket_path, default_state_db_path,
};
use crate::proton::CommandPolicy;
use crate::sync::{ConflictNaming, validate_conflict_suffix};
use crate::trash::LocalDeleteMode;
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
/// existing in phase 1 rather than being discovered when the runtime needed it (phase 4):
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
    /// The user-facing full-sweep schedule (#193). Per-pair: it describes a tree, exactly as
    /// `scan_interval_secs` beside it does.
    FullScanSchedule,
}

impl ConfigKey {
    /// Every key. Completeness is pinned by `every_file_config_key_is_classified_exactly_once`,
    /// which compares these spellings against the keys [`FileConfig`] actually has: a variant
    /// missing from here whose field exists shows up as an unclassified key, and a variant listed
    /// here with no field shows up as a key the parser does not know.
    pub const ALL: [Self; 24] = [
        Self::LocalRoot,
        Self::RemoteRoot,
        Self::DbPath,
        Self::SocketPath,
        Self::LockfilePath,
        Self::ScanIntervalSecs,
        Self::FullScanSchedule,
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
            Self::FullScanSchedule => crate::schedule::FULL_SCAN_SCHEDULE_KEY,
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

    /// Every spelling the file parser accepts for this key, the canonical one first (#102 phase
    /// 5b-1). A client that edits the file finds the key in whichever of these the file already uses
    /// and writes it back in that one: a second spelling of one key is a `duplicate field` the daemon
    /// will not start on.
    ///
    /// Two shapes, and they are not the same rule. Every key has the kebab-case alias
    /// ([`Self::spelling`] with `_` as `-`) **except** the two glob lists, which are spelled
    /// `include`/`exclude` and do **not** accept `include-patterns`. The aliases are `#[serde(alias)]`
    /// attributes on [`FileConfig`] and [`FilePair`], not data, so this table is a hand-written second
    /// copy of them — and `the_engine_accepts_exactly_the_listed_spellings` is what keeps it honest, in
    /// both directions, by reading the names serde itself says it accepts.
    ///
    /// Exhaustive by variant with no `_` arm, like [`Self::spelling`] and [`Self::scope`].
    pub fn spellings(self) -> &'static [&'static str] {
        match self {
            Self::LocalRoot => &["local_root", "local-root"],
            Self::RemoteRoot => &["remote_root", "remote-root"],
            Self::DbPath => &["db_path", "db-path"],
            Self::SocketPath => &["socket_path", "socket-path"],
            Self::LockfilePath => &["lockfile_path", "lockfile-path"],
            Self::ScanIntervalSecs => &["scan_interval_secs", "scan-interval-secs"],
            Self::FullScanSchedule => &[
                crate::schedule::FULL_SCAN_SCHEDULE_KEY,
                "full-scan-schedule",
            ],
            Self::ProtonCli => &["proton_cli", "proton-cli"],
            Self::ProtonTimeoutSecs => &["proton_timeout_secs", "proton-timeout-secs"],
            Self::ProtonListAttempts => &["proton_list_attempts", "proton-list-attempts"],
            Self::DownloadBatchSize => &["download_batch_size", "download-batch-size"],
            Self::IncludePatterns => &["include_patterns", "include"],
            Self::ExcludePatterns => &["exclude_patterns", "exclude"],
            Self::DryRun => &["dry_run", "dry-run"],
            Self::EventsDriven => &["events_driven", "events-driven"],
            Self::EventsFullScanEvery => &["events_full_scan_every", "events-full-scan-every"],
            Self::WarmStart => &["warm_start", "warm-start"],
            Self::WarmStartFullWalkEvery => {
                &["warm_start_full_walk_every", "warm-start-full-walk-every"]
            }
            Self::WarmStartMaxCursorAgeSecs => &[
                "warm_start_max_cursor_age_secs",
                "warm-start-max-cursor-age-secs",
            ],
            Self::DeleteApproval => &["delete_approval", "delete-approval"],
            Self::DeletionPolicyKey => &["deletion_policy", "deletion-policy"],
            Self::LocalDeleteMode => &["local_delete_mode", "local-delete-mode"],
            Self::LogLevel => &["log_level", "log-level"],
            Self::ConflictSuffix => &["conflict_suffix", "conflict-suffix"],
        }
    }

    /// The key a spelling names, if it is one the parser accepts — the inverse of
    /// [`Self::spellings`], so a client that is handed a key by name can ask which setting it is and
    /// therefore which scope it has. Byte-exact: the parser is.
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|key| key.spellings().contains(&spelling))
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
            | Self::FullScanSchedule
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
    /// `--pair NAME`: which pair a `--dry-run` previews (ADR 0005 §2). Not a per-pair flag — it does
    /// not amend a pair, it *selects* one — and only meaningful for a preview: it is validated
    /// **after** resolution, because whether the run is a preview depends on `--dry-run`, the file's
    /// `dry_run` and `--no-dry-run` together, and clap cannot see the file.
    pub pair: Option<String>,
}

impl DaemonConfigInput {
    /// The per-pair flags this invocation set, spelled as the command line spells them, in the order
    /// the daemon documents them.
    ///
    /// **Exhaustive destructure, no `..`**: a new field must be placed in one of the three groups
    /// below before anything compiles, and the group is the decision. A per-pair flag amends *the*
    /// pair, and with more than one pair a flag cannot say which (ADR 0005 §2), so
    /// [`resolve_runtime_configs`] refuses any of them beside a multi-pair file instead of applying
    /// it to one pair and not the others, or to all of them.
    pub fn per_pair_flags_set(&self) -> Vec<&'static str> {
        let Self {
            // Daemon-wide: one value for the process, so it means the same thing beside any number
            // of pairs.
            config: _,
            socket_path: _,
            proton_cli: _,
            proton_timeout_secs: _,
            proton_list_attempts: _,
            log_level: _,
            rust_log: _,
            // Mode: what the run does, not what a pair is. `--full-walk` is every pair's first
            // pass; `--dry-run`/`--no-dry-run` pick preview or daemon; `--pair` picks which pair a
            // preview shows.
            dry_run: _,
            no_dry_run: _,
            force_full_walk: _,
            pair: _,
            // Per-pair.
            local_root,
            remote_root,
            db_path,
            lockfile_path,
            scan_interval_secs,
            download_batch_size,
            include_patterns,
            exclude_patterns,
            events_driven,
            no_events_driven,
            events_full_scan_every,
            warm_start,
            no_warm_start,
            warm_start_full_walk_every,
            warm_start_max_cursor_age_secs,
            no_delete_approval,
            deletion_policy,
            local_delete_mode,
            conflict_suffix,
        } = self;
        let mut set = Vec::new();
        for (flag, given) in [
            ("--local-root", local_root.is_some()),
            ("--remote-root", remote_root.is_some()),
            ("--db-path", db_path.is_some()),
            ("--lockfile-path", lockfile_path.is_some()),
            ("--scan-interval-secs", scan_interval_secs.is_some()),
            ("--download-batch-size", download_batch_size.is_some()),
            ("--include", !include_patterns.is_empty()),
            ("--exclude", !exclude_patterns.is_empty()),
            ("--events-driven", *events_driven),
            ("--no-events-driven", *no_events_driven),
            ("--events-full-scan-every", events_full_scan_every.is_some()),
            ("--warm-start", *warm_start),
            ("--no-warm-start", *no_warm_start),
            (
                "--warm-start-full-walk-every",
                warm_start_full_walk_every.is_some(),
            ),
            (
                "--warm-start-max-cursor-age-secs",
                warm_start_max_cursor_age_secs.is_some(),
            ),
            ("--no-delete-approval", *no_delete_approval),
            ("--deletion-policy", deletion_policy.is_some()),
            ("--local-delete-mode", local_delete_mode.is_some()),
            ("--conflict-suffix", conflict_suffix.is_some()),
        ] {
            if given {
                set.push(flag);
            }
        }
        set
    }
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
    /// The user-facing full-sweep schedule (#193): `weekly sun 03:00` / `monthly day 15, 03:00`,
    /// parsed by [`crate::schedule::FullScanSchedule`]. Unset means **no scheduled sweep**, which
    /// is what every config written before this key says and must keep meaning.
    #[serde(default, alias = "full-scan-schedule")]
    full_scan_schedule: Option<String>,
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
            ConfigKey::FullScanSchedule => self.full_scan_schedule.is_some(),
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
/// when the runtime reads it.
///
/// `deny_unknown_fields` must be repeated here for the same reason [`FileDeleteApproval`] repeats
/// it: serde's deny on [`FileConfig`] does not recurse into nested tables, so without it a typo
/// inside a `[[pair]]` table would be silently ignored (#64).
///
/// `name` is the only **required** field. The roots stay optional because a *file* is not the only
/// source of them — `--local-root` / `--remote-root` amend the single pair, and
/// [`validate_file_config_text`] is scoped to what a file alone can decide (the GUI validates
/// documents it may be part-way through editing). That holds for **one** pair: beside others no flag
/// can supply a root, so [`require_roots_in_every_table`] makes both mandatory.
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
    #[serde(default, alias = "full-scan-schedule")]
    full_scan_schedule: Option<String>,
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
    full_scan_schedule: Option<String>,
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
            full_scan_schedule: config.full_scan_schedule.clone(),
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
            full_scan_schedule: pair.full_scan_schedule.clone(),
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
/// 5. **With more than one pair, every table sets `local_root` and `remote_root`** — see
///    [`require_roots_in_every_table`]. It runs before rule 4, which compares only the roots it is
///    given and would otherwise skip a pair that has none.
///
/// These are lexical. What no lexical rule can see — two roots reaching one directory through a
/// symlink, or a relative path — is the daemon's own startup check on real paths
/// (`daemon::real_path_overlap`).
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
    if let Some(tables) = &config.pair {
        if tables.is_empty() {
            return Err(boxed_error(
                "config declares `pair = []`: an empty pair list syncs nothing. Declare at least \
                 one `[[pair]]` table, or remove the key and use top-level \
                 `local_root`/`remote_root` (a file with no `[[pair]]` is one pair named `default`)",
            ));
        }
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
    }
    let pairs = pair_files(config);
    validate_pair_names(&pairs)?;
    require_roots_in_every_table(&pairs)?;
    validate_pair_roots(&pairs)?;
    Ok(pairs)
}

/// The pairs a config file **states**, in file order, with no rule applied (#102 phase 5a).
///
/// The projection [`resolve_pairs`] validates and [`pair_views`] reads as it is: a file with no
/// `[[pair]]` is its one implicit pair, otherwise one entry per table. Split out so the strict
/// reader and the tolerant one start from the same list and cannot disagree about which pairs a
/// file has. It does **not** check that the file may be read at all: both spellings at once, an
/// explicit `pair = []` and every name or root rule are `resolve_pairs`' job, and a half-written
/// document a settings screen is part-way through editing must still be viewable.
///
/// With `[[pair]]` tables present the top-level per-pair keys are not read — `resolve_pairs` refuses
/// that file, so no daemon ever starts on a reading that mixed the two. An explicit `pair = []`
/// yields no pairs, which is the honest reading of a file that declares none.
fn pair_files(config: &FileConfig) -> Vec<PairFileConfig> {
    match &config.pair {
        None => vec![PairFileConfig::from_top_level(config)],
        Some(tables) => tables.iter().map(PairFileConfig::from_table).collect(),
    }
}

/// One folder pair as a **client** of the config file needs it: the paths the daemon would act on,
/// derived by the engine's own rules so a second implementation never has to exist (#135, #102
/// phase 5a). Built by [`pair_views`].
///
/// Every `Option` is `None` for one reason only: the file does not place that value. `local_root`
/// and `remote_root` are as the file says them, `local_root` with its leading `~` expanded.
/// `db_path` and `lockfile_path` are the files the daemon would open: an absolute override as it
/// is, a relative one under `local_root`, and the per-root `.sync` default when the file sets none.
/// Without a `local_root` a relative override or a default has nothing to hang off, so it is `None`
/// rather than a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairView {
    pub name: String,
    pub local_root: Option<PathBuf>,
    pub remote_root: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub lockfile_path: Option<PathBuf>,
    /// The file's literal `conflict_suffix`, or `None` for the daemon's default. Not validated here:
    /// a client turns it into a [`ConflictNaming`] and falls back to the default on a value the
    /// daemon would refuse to start on.
    pub conflict_suffix: Option<String>,
}

impl PairView {
    fn from_file(pair: &PairFileConfig) -> Self {
        let local_root = pair
            .local_root
            .clone()
            .map(|path| expand_tilde_or_keep(path, "local_root"));
        let state_path =
            |override_value: Option<&PathBuf>,
             field_name: &str,
             default_for: fn(&Path) -> PathBuf| match override_value {
                // A value `effective_state_path` refuses (blank, or naming a directory) is a file the
                // daemon will not start on, not one it opens: the view does not place it.
                Some(path)
                    if require_non_blank_value(Some(path.as_path()), field_name).is_err()
                        || require_state_path_names_a_file(Some(path.as_path()), field_name)
                            .is_err() =>
                {
                    None
                }
                Some(path) => {
                    let path = expand_tilde_or_keep(path.clone(), field_name);
                    if path.is_absolute() {
                        Some(path)
                    } else {
                        local_root
                            .as_deref()
                            .map(|root| state_path_from(root, Some(path), default_for))
                    }
                }
                None => local_root.as_deref().map(default_for),
            };
        Self {
            name: pair.name.clone(),
            db_path: state_path(pair.db_path.as_ref(), "db_path", default_state_db_path),
            lockfile_path: state_path(
                pair.lockfile_path.as_ref(),
                "lockfile_path",
                default_lockfile_path,
            ),
            local_root,
            remote_root: pair.remote_root.clone(),
            conflict_suffix: pair.conflict_suffix.clone(),
        }
    }
}

/// `~` expanded by the engine's own rule, or the value verbatim when the engine would refuse it
/// (`~user`, or `~` with no `HOME`). The tolerant counterpart of [`expand_tilde`]: a config the
/// daemon will not start on is still one a settings screen has to be able to show, and keeping the
/// literal is what lets the eventual error name the string the person typed.
fn expand_tilde_or_keep(path: PathBuf, field_name: &str) -> PathBuf {
    expand_tilde(path.clone(), field_name).unwrap_or(path)
}

/// The folder pairs a config file declares, as a client reads them (#102 phase 5a, ADR 0005).
///
/// **Tolerant by design.** It fails only when the text is not a config file at all (it does not
/// parse as a [`FileConfig`]); every shape rule — names, root collisions, both spellings at once,
/// an empty `pair = []` — stays [`validate_file_config_text`]'s job, because the readers of this
/// are screens that show a document part-way through an edit. It is built on [`pair_files`], the
/// projection [`resolve_pairs`] validates, not on `resolve_pairs` itself, which refuses exactly the
/// half-written files this has to read.
///
/// Reads no filesystem and no environment beyond `HOME` for `~`. The daemon's strict reader,
/// [`resolve_runtime_configs`], is the oracle: for a complete valid file the paths here equal the
/// ones it resolves, pair for pair (`pair_views_agrees_with_the_daemon_resolver`).
pub fn pair_views(text: &str) -> AppResult<Vec<PairView>> {
    let config = parse_file_config(text)
        .map_err(|error| boxed_error(format!("failed to parse config: {error}")))?;
    Ok(pair_files(&config)
        .iter()
        .map(PairView::from_file)
        .collect())
}

/// One pair's folder and state files: the four things the on-disk overlap rule reads, and nothing
/// else. **The shape the rule is stated over**, so that the daemon (which holds a resolved
/// `PairConfig`) and a client that only has a config file's text ([`pair_views`]) put the same
/// question to the same function instead of each carrying a copy (#102 phase 5b-1, ADR 0005 §2).
#[derive(Debug, Clone, Copy)]
pub struct PairStatePaths<'a> {
    pub name: &'a str,
    pub local_root: &'a Path,
    pub db_path: &'a Path,
    pub lockfile_path: &'a Path,
}

/// One pair's folder and state files as the **filesystem** names them: every symlink resolved, a
/// path that does not exist yet resolved as far as it exists (`index::canonicalize_best_effort`).
struct RealPairPaths {
    local_root: RealPath,
    /// The index and the lockfile: the real directory each lives in, with the file's own name.
    db_path: RealPath,
    lockfile_path: RealPath,
}

/// A configured path beside the real one it resolves to, so a message can say both.
struct RealPath {
    written: PathBuf,
    real: PathBuf,
}

impl RealPath {
    fn of(written: &Path) -> Self {
        Self {
            written: written.to_path_buf(),
            real: crate::index::canonicalize_best_effort(written),
        }
    }

    /// A state **file**: the directory it lives in is resolved and the file's own name is kept, so
    /// a path that does not exist yet (the index, before it is first opened) still has a real
    /// place, and a file that is itself a link is judged by where it sits, which is where the
    /// scanner of whoever owns that directory would find it.
    fn of_state_file(written: &Path) -> Self {
        let real = match (written.parent(), written.file_name()) {
            (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
                crate::index::canonicalize_best_effort(parent).join(name)
            }
            _ => crate::index::canonicalize_best_effort(written),
        };
        Self {
            written: written.to_path_buf(),
            real,
        }
    }

    /// `` `written` `` alone when it is already the real path, otherwise `` `written` (really
    /// `real`) `` — a message that repeats a path as its own alias is noise.
    fn describe(&self) -> String {
        if self.written == self.real {
            format!("`{}`", self.written.display())
        } else {
            format!(
                "`{}` (really `{}`)",
                self.written.display(),
                self.real.display()
            )
        }
    }
}

impl RealPairPaths {
    fn of(paths: &PairStatePaths<'_>) -> Self {
        Self {
            local_root: RealPath::of(paths.local_root),
            db_path: RealPath::of_state_file(paths.db_path),
            lockfile_path: RealPath::of_state_file(paths.lockfile_path),
        }
    }

    fn state_files(&self) -> [(&'static str, &RealPath); 2] {
        [
            ("db_path", &self.db_path),
            ("lockfile_path", &self.lockfile_path),
        ]
    }
}

/// Whether `candidate` and `other` overlap **on disk**, and if so the whole refusal as one
/// sentence naming both pairs and, for every path, both the written and the real form (ADR 0005
/// §2 rule 4, phase 4c). `None` when they do not.
///
/// The config reader's rule is lexical, because a file must be checkable without touching the
/// filesystem, and it says so: it cannot see a symlink or a relative path. This is the half that
/// can, and the consequences are the same ones. A pair whose folder sits inside another's has its
/// `.sync` index, lockfile and sidecars scanned and uploaded as the outer pair's ordinary files
/// (`is_sync_state_path` ignores only a **top-level** `.sync`); two pairs over one folder plan
/// opposing actions for it; and the same shape is reached around the folder rule by a state file
/// placed in another pair's folder, or the same state file named twice. Exact aliasing used to be
/// caught only illegibly, by `flock` on the shared lockfile inode ("daemon already running");
/// nesting through a symlink was not caught at all.
///
/// Symmetric in its two arguments except for the wording. **One function, four callers**: the
/// daemon at boot (every pair against every earlier one, before anything is created or locked), a
/// retry (`Daemon::retry_unavailable`, the pair being promoted against all the others), so a pair
/// that becomes available later stays unavailable on an overlap exactly as one at boot refuses the
/// start, a pair that is already running (`Daemon::stop_overlapping_pairs`, against every other
/// pair whose folder exists, ready or not, and stopping only the ready ones), and
/// [`real_path_conflicts`], which asks it of a file's text before a client writes that file. The
/// daemon's callers go through its own `real_path_overlap` wrapper over `PairConfig`.
pub fn real_path_overlap(
    candidate: &PairStatePaths<'_>,
    other: &PairStatePaths<'_>,
) -> Option<String> {
    let (this, that) = (RealPairPaths::of(candidate), RealPairPaths::of(other));
    let (this_name, that_name) = (candidate.name, other.name);
    let root_relation = if this.local_root.real == that.local_root.real {
        Some("is the same folder as")
    } else if this.local_root.real.starts_with(&that.local_root.real) {
        Some("is inside")
    } else if that.local_root.real.starts_with(&this.local_root.real) {
        Some("contains")
    } else {
        None
    };
    if let Some(relation) = root_relation {
        return Some(format!(
            "folder pair '{this_name}': its local_root {} {relation} folder pair '{that_name}''s \
             local_root {}. Two pairs may not share a folder or nest: the inner pair's `.sync` \
             state directory — its index, lockfile and sidecars — would be scanned and uploaded to \
             Proton Drive as the outer pair's ordinary files, and two pairs over one folder plan \
             opposing actions for it. Symlinks are followed, so this is the folders as they really \
             are",
            this.local_root.describe(),
            that.local_root.describe(),
        ));
    }
    for (field, state) in this.state_files() {
        if state.real.starts_with(&that.local_root.real) {
            return Some(format!(
                "folder pair '{this_name}': its {field} {} is inside folder pair '{that_name}''s \
                 local_root {}: pair '{that_name}' would scan that file and upload pair \
                 '{this_name}''s live SQLite index or lockfile to Proton Drive as its own",
                state.describe(),
                that.local_root.describe(),
            ));
        }
    }
    for (field, state) in that.state_files() {
        if state.real.starts_with(&this.local_root.real) {
            return Some(format!(
                "folder pair '{that_name}': its {field} {} is inside folder pair '{this_name}''s \
                 local_root {}: pair '{this_name}' would scan that file and upload pair \
                 '{that_name}''s live SQLite index or lockfile to Proton Drive as its own",
                state.describe(),
                this.local_root.describe(),
            ));
        }
    }
    for (field, state) in this.state_files() {
        for (other_field, other_state) in that.state_files() {
            if state.real == other_state.real {
                return Some(format!(
                    "folder pair '{this_name}''s {field} {} and folder pair '{that_name}''s \
                     {other_field} {} are the same file: no two of these may be, because `flock` \
                     treats two descriptors on one inode as independent (a shared lockfile \
                     surfaces as a spurious \"already running\") and a shared index has two \
                     writers of one baseline",
                    state.describe(),
                    other_state.describe(),
                ));
            }
        }
    }
    None
}

/// The on-disk overlap rule ([`real_path_overlap`]) asked of a config file's **text**: the first
/// pair that overlaps an earlier one, as the same refusal the daemon would exit with at boot (#102
/// phase 5b-1, ADR 0005 §2 rule 4). `Ok` when no pair does.
///
/// [`validate_file_config_text`] is lexical by design and passes two roots that reach one folder
/// through a symlink; this is the half that follows them, so a client can say so **before** it
/// writes a file the daemon refuses to start on. It touches the filesystem (it canonicalizes), so a
/// caller on an interactive thread runs it on a blocking one. It is **advisory** there — a link made
/// after the check changes the answer — and the daemon's own check stays fatal.
///
/// Pairs are put to the rule in boot's order (each against every earlier one, so the sentence names
/// the same pair as the candidate that boot would), through [`pair_views`] and so tolerant of a
/// document part-way through an edit: a pair that does not place all three paths (no `local_root`
/// yet, or a state path the daemon would refuse) has nothing to compare and is skipped, which is
/// [`validate_file_config_text`]'s to report. Unlike the daemon's running-pair check it does **not**
/// skip a neighbour whose folder does not exist yet: at boot a config that overlaps is fatal whether
/// or not the folder exists, and this is the boot question.
pub fn real_path_conflicts(text: &str) -> AppResult<()> {
    let views = pair_views(text)?;
    let placed: Vec<Option<PairStatePaths<'_>>> = views
        .iter()
        .map(|view| {
            Some(PairStatePaths {
                name: view.name.as_str(),
                local_root: view.local_root.as_deref()?,
                db_path: view.db_path.as_deref()?,
                lockfile_path: view.lockfile_path.as_deref()?,
            })
        })
        .collect();
    for (index, candidate) in placed.iter().enumerate() {
        let Some(candidate) = candidate else {
            continue;
        };
        if let Some(message) = placed[..index]
            .iter()
            .flatten()
            .find_map(|other| real_path_overlap(candidate, other))
        {
            return Err(boxed_error(message));
        }
    }
    Ok(())
}

/// **With more than one pair, every table sets both roots** (ADR 0005 §2, phase 4c).
///
/// With one pair a root is optional in the file because a flag can supply it (`--local-root`
/// amends *the* pair). With several no flag can: they are refused beside a multi-pair file
/// ([`DaemonConfigInput::per_pair_flags_set`]), so a table without a root is a pair that cannot
/// start — and, worse, a pair that [`validate_pair_roots`] silently skips, since it compares only
/// the roots it is given. A rule about the pair *set* that nothing can mask, so it lives here and
/// not in the flag-maskable value layer.
fn require_roots_in_every_table(pairs: &[PairFileConfig]) -> AppResult<()> {
    if pairs.len() < 2 {
        return Ok(());
    }
    for pair in pairs {
        let missing: Vec<&str> = [
            ("local_root", pair.local_root.is_none()),
            ("remote_root", pair.remote_root.is_none()),
        ]
        .into_iter()
        .filter_map(|(key, absent)| absent.then_some(key))
        .collect();
        if !missing.is_empty() {
            return Err(boxed_error(format!(
                "config declares {} folder pairs, so every `[[pair]]` table must set both \
                 `local_root` and `remote_root` (no command-line flag can supply one when a flag \
                 cannot say which pair it amends), but pair `{}` sets no {}",
                pairs.len(),
                pair.name,
                describe_quoted(&missing),
            )));
        }
    }
    Ok(())
}

/// `` `a` `` / `` `a`, `b` and `c` `` — so an error naming several things reads as a sentence.
///
/// Deliberately not named for keys: it renders pair *names* too (the unknown `--pair` selector's
/// message), and a helper whose name says "keys" while it quotes names is a comment that lies
/// (#339).
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

/// What two pair names are compared by: ASCII-folded, because a person cannot tell `Photos` from
/// `photos`. **The one definition of "the same pair name"** — [`validate_pair_names`] and the
/// daemon's own N-pair constructor both compare through it, so a pair set the file reader accepts is
/// never one the runtime refuses, nor the other way round.
pub(crate) fn pair_name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

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
        check_pair_name_among(&pair.name, &seen, position)?;
        seen.push(pair_name_key(&pair.name));
    }
    Ok(())
}

/// One name against the pairs it has to coexist with: its own shape ([`check_pair_name`]), then
/// uniqueness, then the `default` reservation. **The one body of the set rules**, called by the file
/// reader ([`validate_pair_names`], which hands it the names *before* this one, so the later of two
/// equal names is the one reported) and by [`validate_pair_name_among`] (which hands it every other
/// name, because a client adding a name has no "before" to speak of).
///
/// `others` are already folded by [`pair_name_key`]; `position` is where the name sits in the file.
fn check_pair_name_among(name: &str, others: &[String], position: usize) -> AppResult<()> {
    check_pair_name(name)?;
    let folded = pair_name_key(name);
    // The duplicate check runs FIRST. Two tables both named `default` are two names that are
    // the same, not one name in the wrong position, and the reservation's advice ("move its
    // table first") would produce two `default`s if it spoke about them.
    if others.contains(&folded) {
        return Err(boxed_error(format!(
            "two `[[pair]]` tables are named `{name}` (names are compared without regard to \
             case, because a selector that matches two pairs can only pick one of them): give \
             each pair a distinct name"
        )));
    }
    if position > 0 && folded == DEFAULT_PAIR_NAME {
        return Err(boxed_error(format!(
            "pair `{name}` is named `{DEFAULT_PAIR_NAME}` but is not the first `[[pair]]` table: \
             that name already means the pair a command addresses when it names none, which is \
             the first table, so one selector would have two answers. Rename this pair, or \
             move its table first"
        )));
    }
    Ok(())
}

/// Whether `name` is a pair name at all: the shape rules of ADR 0005 §2 rule 3, and nothing about
/// the other pairs ([`validate_pair_name_among`] is that half). The sentence is the one the daemon
/// would exit with for a `[[pair]]` table of that name, because it is the same body — a client that
/// checks a name as someone types it (the add-folder form, #102 phase 5b-2) quotes the daemon rather
/// than keeping a regex of its own, which is how the GUI came to disagree with it about `~` (#135).
pub fn validate_pair_name(name: &str) -> Result<(), String> {
    check_pair_name(name).map_err(|error| error.to_string())
}

/// [`validate_pair_name`], and the rules that need the other pairs: `name` is not any of `existing`
/// (compared without regard to ASCII case, [`pair_name_key`]), and the reserved `default` is the
/// **first** table's alone, so it is refused unless `position` is `0`.
///
/// `existing` is every *other* pair's name. `position` is where `name` will sit among the `[[pair]]`
/// tables: `existing.len()` for a pair appended at the end, which is the only place an app adds one.
/// A pair already in the file is checked with its own name left out of `existing`.
pub fn validate_pair_name_among(
    name: &str,
    existing: &[&str],
    position: usize,
) -> Result<(), String> {
    let others: Vec<String> = existing.iter().map(|other| pair_name_key(other)).collect();
    check_pair_name_among(name, &others, position).map_err(|error| error.to_string())
}

fn check_pair_name(name: &str) -> AppResult<()> {
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
/// filesystem (the GUI validates documents for paths that may not exist yet). `.` and `..` are
/// collapsed on the way in ([`local_comparison_key`], #365) — that much *is* lexical, and leaving
/// `..` alone gave one file two keys and let a config walk around every rule here. What stays
/// invisible is what no lexical rule can see: two roots reaching one directory through a **symlink**,
/// and a relative path resolved against the daemon's working directory; catching those would need
/// `canonicalize` on live paths, which belongs to the daemon's own startup rather than to a file
/// check. `~` is expanded
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

/// A **local** path reduced to what it *addresses*, for comparison only: `.` is dropped and `..`
/// is collapsed against the component it undoes, so `local_root = "A"`, `"./A"` and `"B/../A"` are
/// one directory rather than three that can never overlap (#339 round 2, #365).
///
/// The `..` half is [`crate::lexically_normalized`], shared with `index`'s
/// `canonicalize_best_effort` rather than written twice. It matters here because every rule built
/// on this key asks whether two paths are the same file or one contains the other, and
/// `state/../state/index.db` is `state/index.db` on any POSIX filesystem — comparing the written
/// spellings gave one inode two keys and let a config walk straight through
/// [`require_distinct_state_paths`] into the state it refuses.
///
/// The leading `/` is deliberately **not** dropped, which is where this differs from
/// [`remote_root_comparison_key`]: for a local path absolute and relative are two different
/// locations, while `/Drive/X` and `Drive/X` are one Drive location.
///
/// A path that addresses the **current directory** — `.`, `./`, `a/..` — keys as `.` rather than
/// as the empty path the shared primitive returns. Empty is a prefix of every path there is, so
/// an absolute root would be refused as "inside" it; `.` is a prefix of nothing but itself, since
/// every other key here has had its `CurDir` components removed. Keying them all alike is also the
/// point: returning each one's literal form gave `./a/..` and `a/..` two keys for one directory,
/// which is the same defect this function exists to close.
///
/// # The residual, and it cuts both ways
///
/// This is lexical, so it is **not symlink-aware**, and that is not only a missed refusal:
///
/// - **Missed:** two paths reaching one inode through a symlink still compare unequal, which is
///   what the rules could never do anyway.
/// - **Wrong, and this one is new:** if `current` is a symlink, `current/../index.db` is resolved
///   by the kernel against the symlink's *target*, so collapsing it names a file the path does not
///   reach — and two genuinely different files can be refused as one.
///
/// The trade is taken deliberately. The missed refusal it closes is silent and its consequence is
/// the daemon holding a whole-file advisory lock on its own database; the false match it opens is
/// loud, arrives at startup or at a config save, and is undone by spelling the path without the
/// `..`. Closing the residual needs `canonicalize`, which touches the filesystem and errors on a
/// path that does not exist yet — the normal case for a lockfile on first run — and a config file
/// must be checkable without either. **The refusal message is the part to keep honest**: it says
/// "both resolve to X", and under a symlinked `..` X is the lexical answer rather than the
/// kernel's.
///
/// One more thing it walks through: [`expand_tilde_for_comparison`] deliberately declines to say
/// where an unexpandable `~user` points, and the collapse then treats it as an ordinary component,
/// so `~alice/../x` keys as `x`. Bounded rather than fixed — `expand_tilde` refuses such a value in
/// both readers, so the only effect is which refusal speaks first.
fn local_comparison_key(path: &Path) -> PathBuf {
    let key = crate::lexically_normalized(path);
    if key.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        key
    }
}

/// A `remote_root` reduced to what it *addresses*, for comparison only.
///
/// `/Drive/Photos` and `Drive/Photos` name one Drive location — `proton.rs`'s
/// `normalize_remote_path` strips the root either way — so the leading separator and any `.` are
/// dropped before comparing.
///
/// `..` is kept verbatim rather than resolved, which is where this parts company with
/// [`local_comparison_key`] (#365). The reason is not the arithmetic — lexically `/Drive/a/../b`
/// is `/Drive/b` — but that **no such remote root ever reaches the daemon**:
/// `proton::clean_remote_root_path` answers `None` for any path carrying a `..`, so resolving one
/// here would be this layer alone deciding that two Drive locations are one while nothing
/// downstream agrees, for a value that is refused either way.
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
/// ones (a CLI flag over a file value). Refuses a `~` it cannot expand, because this *is* the value
/// the daemon will open — and refuses a **blank** override for the same reason, and at the same
/// point, before it is a value at all: `state_path_from` cannot tell a blank override from "no
/// override" once `resolve_path` has joined it onto `local_root`, so the pair's index or lockfile
/// silently becomes the sync root directory itself (#341). Checked here rather than on the
/// resolved result so a legitimately-configured `db_path` that happens to equal `local_root` is
/// never rejected — this is a property of the *override*, not of the resolved path.
///
/// The same reasoning extends to a **non-blank** override that still names no file (#357): `.`,
/// `..`, or a path lexically ending in `..` (`foo/..`) reach the identical failure through a
/// one-character value `require_non_blank_value` cannot see. Checked here for the same reason as
/// the blank case — before the join, on the override, never on the resolved result.
fn effective_state_path(
    local_root: &Path,
    override_value: Option<PathBuf>,
    field_name: &str,
    default_for: impl Fn(&Path) -> PathBuf,
) -> AppResult<PathBuf> {
    require_non_blank_value(override_value.as_deref(), field_name)?;
    require_state_path_names_a_file(override_value.as_deref(), field_name)?;
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
    let placed = match override_value {
        Some(value) => {
            let value = expand_tilde_for_comparison(value);
            if value.is_absolute() {
                Some(value)
            } else {
                local_root.map(|root| state_path_from(root, Some(value), default_for))
            }
        }
        None => local_root.map(default_for),
    };
    // Keyed on the way out, never on the way in: `state_path_from` joins the override onto the
    // root, so a `..` inside the override only becomes collapsible once the join has happened
    // (#365). Idempotent for a root that was already keyed by the caller.
    placed.map(|path| local_comparison_key(&path))
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
    // Compared as what they address, not as they are written (#365). The merge-path caller hands
    // this raw resolved values — `resolve_path` is `local_root.join(value)`, which never collapses
    // a `..` — so two spellings of one inode reached the equality test as two different `PathBuf`s
    // and the daemon proceeded into exactly the state this refuses.
    let db_key = local_comparison_key(db_path);
    let lockfile_key = local_comparison_key(lockfile_path);
    if db_key == lockfile_key {
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
            db_key.display()
        )));
    }
    Ok(())
}

/// What a resolved invocation does (ADR 0005 §2, phase 4c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Run the daemon over every resolved pair.
    Daemon,
    /// The one-shot `--dry-run`: preview `pairs[pair]` and exit. One pair per invocation, because a
    /// preview rehearses one tree and the report has no pair dimension.
    Preview { pair: usize },
}

/// Every folder pair a config resolved to, in file order (the first is the default pair), and what
/// to do with them.
#[derive(Debug, Clone)]
pub struct RuntimeConfigs {
    pub pairs: Vec<DaemonConfig>,
    pub mode: RunMode,
}

impl RuntimeConfigs {
    /// The pair a preview shows, when this run is a preview.
    pub fn preview(&self) -> Option<&DaemonConfig> {
        match self.mode {
            RunMode::Preview { pair } => self.pairs.get(pair),
            RunMode::Daemon => None,
        }
    }
}

/// The daemon-wide half of a resolved config, computed **once** and copied into every pair's
/// `DaemonConfig` (ADR 0005 §1): the socket and the user-global lock describe the process, and
/// `proton_cli`, the timeout, the list attempts and the log filter are what the one shared
/// `ProtonDriveClient` is built from. Equal across pairs by construction, which
/// `Daemon::from_pairs` re-checks for a set that was built some other way — it is the one place
/// that says what "agree" means.
struct ProcessValues {
    socket_path: PathBuf,
    global_lock_path: PathBuf,
    proton_cli: PathBuf,
    proton_timeout: Duration,
    proton_list_attempts: usize,
    log_filter: String,
}

impl ProcessValues {
    fn resolve(input: &DaemonConfigInput, file_config: &FileConfig) -> AppResult<Self> {
        // Resolved before any pair's struct literal because both defaults are now fallible (#74)
        // and must stay LAZY: an explicit --socket-path must not fail because the /tmp fallback —
        // which this run never touches — is hostile.
        let socket_path = match input
            .socket_path
            .clone()
            .or(file_config.socket_path.clone())
        {
            Some(path) => expand_tilde(path, "socket_path")?,
            None => default_socket_path()?,
        };
        // Not user-overridable: the single-instance guarantee must key on a fixed per-user path so
        // it holds regardless of --socket-path / --local-root (see `default_global_lock_path`).
        let global_lock_path = default_global_lock_path()?;
        let default_command_policy = CommandPolicy::default();
        Ok(Self {
            socket_path,
            global_lock_path,
            proton_cli: input
                .proton_cli
                .clone()
                .or(file_config.proton_cli.clone())
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
            log_filter: resolve_log_filter(
                input.log_level.as_deref(),
                input.rust_log.as_deref(),
                file_config.log_level.as_deref(),
            )?,
        })
    }
}

/// A per-pair flag beside more than one pair is refused (ADR 0005 §2, maintainer decision M2): a
/// flag amends *the* pair, and with several it cannot say which — applying it to the first would
/// quietly leave the others as they were, applying it to all would override values written in
/// tables on purpose. Every per-pair flag, including the opt-outs (`--no-delete-approval`,
/// `--no-events-driven`, `--no-warm-start`), because a flag that quietly keeps a safeguard on for
/// one pair and not another is the worst of the three.
///
/// **A startup rule and not a file rule**: a file has no flags, so [`validate_file_config_text`]
/// has nothing to check, and it lives beside [`resolve_pairs`] only in being a rule about the pair
/// *set*.
fn refuse_per_pair_flags_beside_several_pairs(
    input: &DaemonConfigInput,
    pairs: &[PairFileConfig],
) -> AppResult<()> {
    if pairs.len() < 2 {
        return Ok(());
    }
    let flags = input.per_pair_flags_set();
    if flags.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = pairs.iter().map(|pair| pair.name.as_str()).collect();
    Err(boxed_error(format!(
        "{} cannot be used with a config that declares {} folder pairs ({}): a per-pair flag \
         amends the one pair and cannot say which of several. Set it inside the `[[pair]]` table \
         it belongs to instead. Flags that describe the whole daemon (--config, --socket-path, \
         --proton-cli, --proton-timeout-secs, --proton-list-attempts, --log-level) and the mode \
         flags (--dry-run, --no-dry-run, --full-walk, --pair) still apply",
        describe_quoted(&flags),
        pairs.len(),
        describe_quoted(&names),
    )))
}

/// `dry_run = true` inside a `[[pair]]` table beside other pairs is refused (maintainer decision
/// M3): a preview rehearses one pair and exits, so a per-pair key cannot pick the *process's* mode —
/// it would either turn every pair into a preview or silently apply to the one pair it was written
/// beside. `dry_run = false` is the default spelled out and is accepted.
///
/// Called by **both** readers: the file reader refuses it outright (a file has no flags), and the
/// daemon refuses it unless `--dry-run` or `--no-dry-run` was given, which is exactly the "pick the
/// mode" a table value cannot do.
fn refuse_dry_run_in_a_pair_table(pairs: &[PairFileConfig]) -> AppResult<()> {
    if pairs.len() < 2 {
        return Ok(());
    }
    let tables: Vec<&str> = pairs
        .iter()
        .filter(|pair| pair.dry_run == Some(true))
        .map(|pair| pair.name.as_str())
        .collect();
    if tables.is_empty() {
        return Ok(());
    }
    Err(boxed_error(format!(
        "{} set{} `dry_run = true`, which cannot be used with {} folder pairs: a dry run previews \
         one pair and exits, so a key inside a `[[pair]]` table cannot decide what the whole daemon \
         does. Remove it, and preview a pair with `proton-syncd --dry-run [--pair NAME]`",
        describe_quoted(&tables),
        if tables.len() == 1 { "s" } else { "" },
        pairs.len(),
    )))
}

/// Resolves a config file and the command line into **every** folder pair it declares, and what to
/// do with them (ADR 0005 §2, phase 4c). The one resolver: [`resolve_runtime_config`] is this with a
/// check that exactly one pair came out.
///
/// The order is the design: the pair shape (including every rule about the pair *set*), then the
/// flag rule, then the mode, then each pair's merge and its own validation, then the preview
/// selector. **Config errors are all-or-nothing** — one table that cannot resolve stops the daemon
/// before anything is locked, because a daemon that starts and silently syncs two pairs of three
/// is worse than one that does not start. Only an *environment* failure (a folder that is not
/// there, a lock held) is per pair, and that is the runtime's to handle (phase 4b).
///
/// With one pair this is byte-for-byte what it was: flags amend the pair, and no message changes.
/// With several, an error from one pair's merge names the pair.
pub fn resolve_runtime_configs(input: DaemonConfigInput) -> AppResult<RuntimeConfigs> {
    // The config-file path is itself a local-filesystem path, so it gets the same `~` treatment
    // as the values inside it (see `expand_tilde` below).
    let config_path = input
        .config
        .clone()
        .map(|path| expand_tilde(path, "--config"))
        .transpose()?;
    let file_config = load_file_config(config_path.as_ref())?;
    // The pair shape is resolved before anything is merged, and before any lock is taken (#102, ADR
    // 0005 §2): a file with no `[[pair]]` is one implicit pair named `default` whose values are the
    // top-level per-pair keys, so every config written before multi-pair resolves exactly as it
    // always has.
    let pairs = resolve_pairs(&file_config)?;
    refuse_per_pair_flags_beside_several_pairs(&input, &pairs)?;
    let preview = if input.no_dry_run {
        false
    } else if input.dry_run {
        true
    } else if pairs.len() > 1 {
        refuse_dry_run_in_a_pair_table(&pairs)?;
        false
    } else {
        pairs.first().and_then(|pair| pair.dry_run).unwrap_or(false)
    };
    let process = ProcessValues::resolve(&input, &file_config)?;
    let several = pairs.len() > 1;
    let mut configs = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let name = pair.name.clone();
        configs.push(merge_pair(&input, pair, &process).map_err(|error| {
            if several {
                boxed_error(format!("folder pair '{name}': {error}"))
            } else {
                error
            }
        })?);
    }
    // After resolution, not before: whether this run is a preview depends on the flags and the file
    // together, and an unknown name is worth reporting against the pairs that really resolved.
    let mode = match (preview, input.pair.as_deref()) {
        (false, None) => RunMode::Daemon,
        (false, Some(_)) => {
            return Err(boxed_error(
                "--pair only selects which pair a dry run previews; the daemon runs every pair. \
                 Add --dry-run to preview one, or use `proton-sync --pair NAME` to address a \
                 running pair",
            ));
        }
        // No `--pair`: the default pair, the first table (ADR 0005 §2 rule 6).
        (true, None) => RunMode::Preview { pair: 0 },
        (true, Some(name)) => {
            // Byte-exact, like the wire (`ControlShared::resolve_pair_index`): two names that differ
            // only in case are refused at startup precisely so this never has to guess.
            let Some(pair) = configs.iter().position(|config| config.name == name) else {
                let names: Vec<&str> = configs.iter().map(|config| config.name.as_str()).collect();
                return Err(boxed_error(format!(
                    "--pair `{name}` names no configured folder pair (names are matched exactly); \
                     the configured pairs are {}",
                    describe_quoted(&names),
                )));
            };
            RunMode::Preview { pair }
        }
    };
    Ok(RuntimeConfigs {
        pairs: configs,
        mode,
    })
}

/// The single-pair entry point: [`resolve_runtime_configs`], then a check that exactly one pair came
/// out, returning it with whether the run is a dry-run preview. What every test fixture and the
/// example-config check call; the binary resolves the list.
pub fn resolve_runtime_config(input: DaemonConfigInput) -> AppResult<(DaemonConfig, bool)> {
    let RuntimeConfigs { mut pairs, mode } = resolve_runtime_configs(input)?;
    if pairs.len() != 1 {
        let names: Vec<&str> = pairs.iter().map(|pair| pair.name.as_str()).collect();
        return Err(boxed_error(format!(
            "config declares {} folder pairs ({}), and this entry point resolves exactly one; \
             resolve them all with `resolve_runtime_configs`",
            pairs.len(),
            describe_quoted(&names),
        )));
    }
    let preview = matches!(mode, RunMode::Preview { .. });
    Ok((pairs.remove(0), preview))
}

/// One pair's resolved config: the file's values for that pair under the command line's, over the
/// daemon-wide half computed once.
///
/// `DaemonConfig` stays the **fused resolved input** — one flat struct a config file and the CLI
/// flags merge into, which is also what the one-shot `--dry-run` preview and every test fixture
/// builds; the runtime splits it per scope (`DaemonConfig::into_parts`). Every per-pair value comes
/// from ONE projection ([`PairFileConfig`]), so a `[[pair]]` file and the equivalent top-level file
/// cannot diverge. The per-pair flags in `input` are applied as they always were — they mean "the
/// single pair" — and [`refuse_per_pair_flags_beside_several_pairs`] guarantees that with several
/// pairs there are none to apply.
fn merge_pair(
    input: &DaemonConfigInput,
    pair: PairFileConfig,
    process: &ProcessValues,
) -> AppResult<DaemonConfig> {
    // By value so each `input.X.or(pair.X)` below reads as it always did. The mode flags
    // (`dry_run`, `no_dry_run`, `pair`) are the run's and are resolved by the caller; nothing here
    // reads them.
    let input = input.clone();
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
    let config = DaemonConfig {
        // The pair's configured name (#102 phase 3, ADR 0005 §4) — `config::DEFAULT_PAIR_NAME`
        // for the implicit single-pair file, the `[[pair]]` table's own `name` otherwise. What a
        // wire selector is matched against. `resolve_pairs` already validated it (charset,
        // uniqueness, the `default`-is-first rule), so no further check is needed here.
        name: pair.name.clone(),
        local_root,
        remote_root,
        db_path,
        // The daemon-wide half: computed once and copied, so every pair's config agrees with every
        // other's by construction (see [`ProcessValues`]).
        socket_path: process.socket_path.clone(),
        lockfile_path,
        global_lock_path: process.global_lock_path.clone(),
        scan_interval: Duration::from_secs(
            input
                .scan_interval_secs
                .or(pair.scan_interval_secs)
                .unwrap_or(300)
                .max(1),
        ),
        // No CLI flag, deliberately: this is a setting a person edits in Settings and a schedule
        // is not a thing to pass on a one-shot command line. `--full-walk` already covers "sweep
        // this start", which is the only question a flag would be asked.
        //
        // Parsed HERE, on the merge path, so a value the daemon cannot read stops it at startup —
        // the `log_level` rule (#237), for the same reason: a schedule that silently never fires
        // is a safety net nobody notices is missing.
        full_scan_schedule: pair
            .full_scan_schedule
            .as_deref()
            .map(crate::schedule::validate)
            .transpose()?,
        proton_cli: process.proton_cli.clone(),
        proton_timeout: process.proton_timeout,
        proton_list_attempts: process.proton_list_attempts,
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
        log_filter: process.log_filter.clone(),
        // Flag > file > default, and the default is `Trash`: a config that says nothing must not
        // unlink. Read off `pair` rather than the file's top level because the key describes one
        // tree.
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
    Ok(config)
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
/// both roots in every table beside other pairs) come from the same [`resolve_pairs`] the daemon
/// starts on, and `dry_run = true` inside a table beside other pairs is refused by the same
/// [`refuse_dry_run_in_a_pair_table`] — the GUI must not be able to save a file the daemon would
/// then refuse to start on. That last rule is one the daemon can be told past with `--dry-run` or
/// `--no-dry-run`; it is still refused here, because a file has no flags to be told past.
pub fn validate_file_config_text(text: &str) -> AppResult<()> {
    let config = parse_file_config(text)
        .map_err(|error| boxed_error(format!("failed to parse config: {error}")))?;
    let pairs = resolve_pairs(&config)?;
    refuse_dry_run_in_a_pair_table(&pairs)?;
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
    // `proton_cli` is daemon-wide **by force** (#356): it constructs the one shared
    // `ProtonDriveClient`, and is used directly rather than joined onto a root, so — unlike
    // `db_path`/`lockfile_path` — the merged value in `validate_runtime_config` is still the literal
    // value here and never needs the `effective_state_path` treatment. A blank value used to pass
    // both readers and start a daemon that ENOENT-loops every pass (#158); this was previously
    // caught only by `gui-core`'s own copy of the rule, which is now inherited from here instead.
    require_non_blank_value(config.proton_cli.as_deref(), "proton_cli")?;
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
    require_non_blank_value(pair.local_root.as_deref(), "local_root")?;
    require_non_blank_value(pair.remote_root.as_deref(), "remote_root")?;
    // `db_path`/`lockfile_path` have no root of their own — they resolve to a file *under*
    // `local_root` — but the same value class shows up the same way: `effective_state_path` joins
    // an empty override onto `local_root` and gets the root back, indistinguishable from "no
    // override" (#341).
    require_non_blank_value(pair.db_path.as_deref(), "db_path")?;
    require_non_blank_value(pair.lockfile_path.as_deref(), "lockfile_path")?;
    // A second, non-blank value class reaching the same failure (#357): `.`/`..`/`foo/..` are not
    // blank but still name no file once `resolve_path` joins them onto `local_root`.
    require_state_path_names_a_file(pair.db_path.as_deref(), "db_path")?;
    require_state_path_names_a_file(pair.lockfile_path.as_deref(), "lockfile_path")?;
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
    // No flag can mask this one — `full_scan_schedule` is file-only (#193) — so the file reader and
    // the merge path agree by construction. It is still checked HERE rather than in the structural
    // layer because it is a value rule, and `ConfigDoc::save` must refuse to write a schedule the
    // daemon would then fail to start on.
    if let Some(schedule) = pair.full_scan_schedule.as_deref() {
        crate::schedule::validate(schedule)?;
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

/// A value that is *present but blank* is always a mistake — a cleared settings field, not a
/// default. **Absent is not blank**: the daemon fills an absent `local_root` from a flag (and
/// `db_path`/`lockfile_path` from a computed default, and `proton_cli` from `"proton-drive"`), and
/// refusing one here would reject a config that starts.
///
/// Blank means empty **or whitespace-only**, which is what a cleared form field actually produces;
/// a path whose bytes are not valid UTF-8 is never blank (nothing there is whitespace).
///
/// Named for the *value class*, not for roots: it started as the root-only rule (hence every
/// example below talking about `local_root`), then #341 reused it verbatim for
/// `db_path`/`lockfile_path` rather than writing a second spelling of "blank" — the same class of
/// mistake reaches `effective_state_path`'s empty-override case exactly the way it used to reach
/// an empty root, and `resolve_path` joining `""` onto `local_root` is indistinguishable from "no
/// override" once it has happened. #356 reused it again for `proton_cli`, which reaches no join at
/// all: it is used directly, so the merged value in `validate_runtime_config` is still the literal
/// blank a file wrote, and the fix is applying an existing rule at a seam #341 did not need to touch
/// rather than inventing a fourth spelling of "blank".
///
/// One definition for the file check ([`validate_pair_file_values`] for the per-pair keys,
/// [`validate_file_config_text`] for daemon-wide `proton_cli` — where either value is one a flag may
/// still override), the merged runtime check for the roots and `proton_cli`
/// ([`validate_runtime_config`], where it is the value the daemon will actually use), and the merged
/// check for `db_path`/`lockfile_path` ([`effective_state_path`]) — the latter runs *before*
/// `resolve_path` folds a blank override onto `local_root`, because by the time
/// `validate_runtime_config` sees either path it has already been resolved to an absolute one and
/// blank is no longer distinguishable from a legitimately-configured path that happens to equal the
/// root. `proton_cli` has no such join, so its merged check lives in `validate_runtime_config`
/// alongside the roots rather than needing `effective_state_path`'s earlier seam. The message is
/// load-bearing: `gui-core`'s `an_empty_root_is_refused_before_it_reaches_the_daemon` matches on it,
/// and the same rule used to live over there reading top-level keys — which a `[[pair]]` file does
/// not have, and which is also where the GUI's own `proton_cli` copy of this rule used to live
/// before #356 deleted it in favour of inheriting this one through `validate_file_config_text`.
///
/// **Sharing it changed the daemon's answer in one case, and that is deliberate** (#339 §6): the
/// pre-#332 `validate_runtime_config` tested `as_os_str().is_empty()`, so a daemon whose merged
/// `local_root` was `"   "` started, and now does not. A directory named `"   "` is legal on Unix,
/// so this is a real behaviour change — a negligible one, since the value it refuses is what a
/// cleared settings field produces and not what anyone types on purpose. The same is true of a
/// `db_path`/`lockfile_path` of `"   "`: it resolves to a file literally named three spaces under
/// `local_root`, which is legal and used to work — refusing it is the same negligible change (#341).
/// A `proton_cli` of `"   "` used to resolve to a command name of three spaces, looked up on `PATH`
/// and never found — refusing it up front is the same change again, and the ENOENT loop it used to
/// produce (#158) is a worse failure mode than a config the daemon refuses to start on (#356).
fn require_non_blank_value(value: Option<&Path>, field_name: &str) -> AppResult<()> {
    if value.is_some_and(|path| path.to_str().is_some_and(|text| text.trim().is_empty())) {
        return Err(boxed_error(format!("{field_name} must not be empty")));
    }
    Ok(())
}

/// A `db_path` / `lockfile_path` override that **lexically names a directory, not a file** — the
/// full #357 value class. Joined onto `local_root` by `resolve_path` (or kept verbatim when already
/// absolute), any of these hands `index::open_database` / `LockGuard::acquire` a directory to open
/// read/write, exactly the failure mode #341 already fixed for a *blank* override, reached here
/// through a non-blank one instead.
///
/// Four arms, each with a spelling only it catches — proved by [`a_direct_truth_table_pins_every_arm_of_require_state_path_names_a_file`],
/// which calls this function directly rather than only through the two readers that wrap it:
///
/// 1. `file_name()` is `None`: witnessed alone by `""` (called directly — in production this is
///    caught earlier, by [`require_non_blank_value`], and ordering — that check runs first at both
///    call sites — is what keeps its message the one a caller sees for a blank value).
/// 2. The raw value **ends with a path separator**: witnessed alone by `state/`. POSIX gives a
///    trailing slash exactly this meaning — "this names a directory" — independent of what a
///    lexical `Path` component walk sees (`Path::new("state/").file_name()` is `Some("state")`).
/// 3. The raw value **ends with `/.`**: witnessed alone by `state/.`. `std::path::Components`
///    silently drops an *interior* `CurDir` when it is not the whole path, so
///    `Path::new("state/.").file_name()` is `Some("state")` — a components- or `file_name()`-based
///    test structurally cannot see this spelling. Only the raw bytes can, which is why this whole
///    function works on them rather than on `Path`'s parsed view.
/// 4. The raw value is exactly `~`: witnessed alone by `~` itself. The bare home directory, not a
///    file inside it, checked before `expand_tilde` runs at either seam — expanding first does not
///    help, because `Path::new("/home/qp").file_name()` is `Some("qp")`, a normal-looking file name;
///    the override has to be read as *itself naming the home directory* before expansion turns it
///    into an ordinary-looking absolute path. `~/x.db` is unaffected: it is not equal to `~`.
///
/// A first version of this rule also carried `.`, `..`, `bytes.ends_with(b"/..")`, and `bytes ==
/// b"~/"` as explicit arms, reasoning that stating the rule completely mattered more than the
/// overlap. Mutation testing against the direct truth table proved that reasoning wrong: **the
/// shadowing is mutual and total, not a one-directional restatement.** `.` and `..` can only ever
/// equal themselves, and for that exact input `file_name()` is unconditionally `None` — arm 1
/// already covers every input arm-3-as-written could match, with no possible counterexample. The
/// same holds for `ends_with(b"/..")`: ending in `/..` makes the last path component exactly `..`,
/// which every `Path` on this platform parses as `ParentDir`, so `file_name()` is `None` for that
/// input too, unconditionally — not "usually", not "in the cases tested". And `bytes == b"~/"`
/// trivially implies `ends_with(b"/")` (arm 2), since `"~/"` ends with `/` by construction. None of
/// those four could ever be the deciding arm for any input, so a truth table covering every
/// spelling anyone could type still could not make them fail if removed — which is the sign to
/// delete rather than to keep documenting as "redundant but harmless". They are gone; do not restore
/// them for symmetry without a fifth, real spelling that needs them.
///
/// This is a **second value class from blank, not a widening of it**: `require_non_blank_value` is
/// `trim().is_empty()`, and none of the four remaining forms are blank — every one had to be typed.
///
/// **Must NOT catch `foo/../bar.db`, `./index.db`, `a/./b.db`, `.hidden.db`, `..hidden.db`, `...`,
/// `~/x.db`, `state/~`, `./~`, or `/home/me/~`.** Each names a real file — `foo/../bar.db`'s last
/// component is a name despite the `..` in the middle, `.hidden.db`/`..hidden.db`/`...` are ordinary
/// filenames that merely start with dots without being exactly `.` or `..`, `~/x.db` is a file under
/// the home directory rather than the directory itself, and `state/~`/`./~`/`/home/me/~` are files
/// **literally named** `~` — `expand_tilde` only ever touches a *leading* `~` component, so a `~`
/// anywhere else in the path is just a character in a file name, and arm 4 does not fire because the
/// raw value is not *exactly* `~`. Checked on the **override itself**, before `resolve_path` joins
/// it onto `local_root` — never as a comparison of the resolved path against `local_root`, which
/// #355 (the discussion that produced this fix) records as the wrong form: it would reject a
/// legitimately-configured `db_path` that happens to equal the root by coincidence rather than by
/// degeneracy. `~user` and `~x` are also unaffected for the same structural reason (not equal to
/// `~`), and still surface `expand_tilde`'s own `~user`-rejection error rather than this one, at
/// both seams — this rule must never steal that diagnosis from its actual cause.
///
/// Works on raw bytes (`OsStrExt::as_bytes`), not `str`: this crate is Unix-only (see the
/// `compile_error!` in `src/lib.rs`), and a non-UTF-8 override must be judged the same way a valid
/// one is rather than panicking or silently passing — `Path::file_name()` itself is already
/// byte-based and needs no UTF-8, and arms 2–4 read the same bytes directly.
fn require_state_path_names_a_file(value: Option<&Path>, field_name: &str) -> AppResult<()> {
    use std::os::unix::ffi::OsStrExt;

    let Some(path) = value else {
        return Ok(());
    };
    let bytes = path.as_os_str().as_bytes();
    let names_no_file = path.file_name().is_none()
        || bytes.ends_with(b"/")
        || bytes.ends_with(b"/.")
        || bytes == b"~";
    if names_no_file {
        return Err(boxed_error(format!(
            "{field_name} must name a file, not a directory (got `{}`)",
            path.display()
        )));
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
    require_non_blank_value(Some(&config.local_root), "local_root")?;
    require_non_blank_value(Some(&config.remote_root), "remote_root")?;
    // `proton_cli` is used directly, never joined onto a root (#356), so — unlike `db_path` and
    // `lockfile_path` — the merged value here is still the literal one a blank override produced,
    // and this check does not need to run any earlier than `validate_runtime_config` does the roots.
    require_non_blank_value(Some(&config.proton_cli), "proton_cli")?;
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
            // #356: an empty `proton_cli` used to pass this reader and start a daemon that
            // ENOENT-loops every pass; only `gui-core`'s own copy of the rule caught it.
            ("proton_cli = \"\"\n", "proton_cli must not be empty"),
            // The blank rule, not an is-empty check, same as every other field it covers.
            ("proton_cli = \"   \"\n", "proton_cli must not be empty"),
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

    /// #341: `db_path = ""` used to pass both readers and resolve to the sync root itself, since
    /// `effective_state_path`'s empty-override case was indistinguishable from "no override" once
    /// `resolve_path` had joined it onto `local_root`. The daemon then failed later at
    /// `index::open_database` — after the config layer had already said yes.
    #[test]
    fn empty_db_path_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            db_path: Some(PathBuf::new()),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty db_path should fail");

        assert_eq!(error.to_string(), "db_path must not be empty");
    }

    /// The sibling of the above: an empty `lockfile_path` resolves to the sync root directory
    /// itself and reaches `LockGuard::acquire`, which opens it read/write — a worse failure mode
    /// than `open_database`'s, since a directory is not a file at all (#341).
    #[test]
    fn empty_lockfile_path_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            lockfile_path: Some(PathBuf::new()),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty lockfile_path should fail");

        assert_eq!(error.to_string(), "lockfile_path must not be empty");
    }

    /// #357: the full value class, not just the `Path::file_name()`-visible corner of it. All of
    /// `.`, `..`, `foo/..` (already `file_name() == None`), `state/` (a trailing separator — POSIX's
    /// own "this names a directory"), `state/.` (an *interior* `CurDir` — `Components` drops it
    /// silently, so `Path::new("state/.").file_name()` is `Some("state")`, and only the raw bytes
    /// can see this spelling), and bare `~`/`~/` (the home directory itself, checked before
    /// `expand_tilde` runs) reach the identical failure: `resolve_path` joins them onto `local_root`
    /// (or, for `..`, walks up out of it; `~` skips the join entirely) and hands
    /// `index::open_database` a directory. Covers both readers' merged output via
    /// `resolve_runtime_config`.
    #[test]
    fn a_db_path_that_normalizes_to_no_file_name_is_refused() {
        for degenerate in [".", "..", "foo/..", "state/.", "state/", "~", "~/"] {
            let error = resolve_runtime_config(DaemonConfigInput {
                local_root: Some(PathBuf::from("sync-root")),
                remote_root: Some(PathBuf::from("/Drive/Config")),
                db_path: Some(PathBuf::from(degenerate)),
                ..DaemonConfigInput::default()
            })
            .expect_err(&format!("db_path = {degenerate:?} should be refused"));

            assert!(
                error.to_string().contains("db_path must name a file"),
                "db_path = {degenerate:?}: got {error}"
            );
        }
    }

    /// The near-miss set the widened rule must NOT touch: every one of these lexically resembles a
    /// degenerate spelling somewhere in it, but the *last* real component (or, for `~/x.db`, the
    /// whole value) still names a real file. `Path::file_name()` alone gets `foo/../bar.db` right;
    /// the byte-level checks added for #357 must not regress it or the others.
    #[test]
    fn db_path_near_miss_spellings_that_look_degenerate_but_are_not_are_still_accepted() {
        for benign in [
            "foo/../bar.db",
            "./index.db",
            "a/./b.db",
            ".hidden.db",
            "..hidden.db",
            "...",
            "~/x.db",
            "state/index.db",
        ] {
            let (config, _) = resolve_runtime_config(DaemonConfigInput {
                local_root: Some(PathBuf::from("sync-root")),
                remote_root: Some(PathBuf::from("/Drive/Config")),
                db_path: Some(PathBuf::from(benign)),
                ..DaemonConfigInput::default()
            })
            .unwrap_or_else(|e| panic!("db_path = {benign:?} must still resolve: {e}"));
            assert!(
                !config.db_path.as_os_str().is_empty(),
                "db_path = {benign:?} resolved to nothing"
            );
        }

        // An absolute override outside `local_root` entirely is explicitly legal (`resolve_path`
        // returns it as-is) and must not be caught by a rule that is about naming a file, not about
        // location.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            db_path: Some(PathBuf::from("/elsewhere/index.db")),
            ..DaemonConfigInput::default()
        })
        .expect("an absolute db_path outside local_root is legal and must not be refused");
        assert_eq!(config.db_path, PathBuf::from("/elsewhere/index.db"));
    }

    /// A non-UTF-8 override must be judged by the same rule as a valid one, never panicking and
    /// never silently waved through: `require_state_path_names_a_file` works on raw
    /// `OsStrExt::as_bytes`, so `\xff.db` (a real, if unusual, file name) is accepted and `\xff/..`
    /// (lexically ending in `..`, exactly like a valid-UTF-8 equivalent) is refused, with a lossy
    /// `Path::display()` in the message either way.
    #[test]
    fn a_non_utf8_db_path_is_judged_by_the_same_rule_as_a_valid_one() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let accepted = PathBuf::from(OsStr::from_bytes(b"\xff.db"));
        resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            db_path: Some(accepted),
            ..DaemonConfigInput::default()
        })
        .expect("a non-UTF-8 file name must still be accepted");

        let refused = PathBuf::from(OsStr::from_bytes(b"\xff/.."));
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            db_path: Some(refused),
            ..DaemonConfigInput::default()
        })
        .expect_err("a non-UTF-8 path lexically ending in .. must still be refused");
        assert!(
            error.to_string().contains("db_path must name a file"),
            "got {error}"
        );
    }

    /// The sibling of the above, for `lockfile_path`: `LockGuard::acquire` opens whatever this
    /// resolves to read/write, so a directory — including the bare home directory a `~` or `~/`
    /// names — is a worse failure than `open_database`'s (#357, mirroring #341's own
    /// db_path/lockfile_path pairing).
    #[test]
    fn a_lockfile_path_that_normalizes_to_no_file_name_is_refused() {
        for degenerate in [".", "..", "foo/..", "state/.", "state/", "~", "~/"] {
            let error = resolve_runtime_config(DaemonConfigInput {
                local_root: Some(PathBuf::from("sync-root")),
                remote_root: Some(PathBuf::from("/Drive/Config")),
                lockfile_path: Some(PathBuf::from(degenerate)),
                ..DaemonConfigInput::default()
            })
            .expect_err(&format!("lockfile_path = {degenerate:?} should be refused"));

            assert!(
                error.to_string().contains("lockfile_path must name a file"),
                "lockfile_path = {degenerate:?}: got {error}"
            );
        }
    }

    /// The blank rule, not an is-empty check: `"   "` carries no visible bytes but is not the empty
    /// string, and `require_non_blank_value` must still catch it via `trim().is_empty()` (#341,
    /// mirroring the same distinction #340 already drew for `local_root`/`remote_root`).
    #[test]
    fn whitespace_only_db_path_is_refused_the_same_as_an_empty_one() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            db_path: Some(PathBuf::from("   ")),
            ..DaemonConfigInput::default()
        })
        .expect_err("whitespace-only db_path should be refused, same as an empty one");

        assert_eq!(error.to_string(), "db_path must not be empty");
    }

    /// The negative case: a legitimate non-blank relative `db_path` still resolves exactly as
    /// before this change — the new check must not touch any value that is not blank. It is also
    /// the #357 negative case restated: `"state/index.db"` is an ordinary relative path with no
    /// `.`/`..` degeneracy at all, so this one assertion proves both value classes (blank and
    /// degenerate) leave it alone, rather than keeping a second, byte-identical test under a #357
    /// name.
    #[test]
    fn a_non_blank_relative_db_path_still_resolves_under_local_root() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            db_path: Some(PathBuf::from("state/index.db")),
            ..DaemonConfigInput::default()
        })
        .expect("a legitimate relative db_path must still resolve");

        assert_eq!(config.db_path, PathBuf::from("sync-root/state/index.db"));
    }

    /// #356: `proton_cli` is daemon-wide **by force**, not per-pair, and it is used directly rather
    /// than joined onto `local_root` — so unlike `db_path`/`lockfile_path` its merged value in
    /// `validate_runtime_config` is still the literal blank a file (or, here, a flag) supplied,
    /// and the fix does not need `effective_state_path`'s earlier seam. An empty `proton_cli` used
    /// to pass both readers and start a daemon that then ENOENT-loops every pass (#158).
    #[test]
    fn empty_proton_cli_returns_targeted_config_error() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_cli: Some(PathBuf::new()),
            ..DaemonConfigInput::default()
        })
        .expect_err("empty proton_cli should fail");

        assert_eq!(error.to_string(), "proton_cli must not be empty");
    }

    /// The blank rule, not an is-empty check: `"   "` carries no visible bytes but is not the empty
    /// string, and `require_non_blank_value` must still catch it via `trim().is_empty()` (#356,
    /// mirroring the same distinction #341 already drew for `db_path`/`lockfile_path`).
    #[test]
    fn whitespace_only_proton_cli_is_refused_the_same_as_an_empty_one() {
        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_cli: Some(PathBuf::from("   ")),
            ..DaemonConfigInput::default()
        })
        .expect_err("whitespace-only proton_cli should be refused, same as an empty one");

        assert_eq!(error.to_string(), "proton_cli must not be empty");
    }

    /// The negative case: a legitimate non-blank `proton_cli` still resolves to exactly its literal
    /// value — no join onto any root — which is the property that lets the merged check sit beside
    /// the roots in `validate_runtime_config` rather than needing `effective_state_path`'s earlier
    /// seam.
    #[test]
    fn a_non_blank_proton_cli_still_resolves_to_its_literal_value() {
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("sync-root")),
            remote_root: Some(PathBuf::from("/Drive/Config")),
            proton_cli: Some(PathBuf::from("/usr/bin/proton-drive")),
            ..DaemonConfigInput::default()
        })
        .expect("a legitimate proton_cli must still resolve");

        assert_eq!(config.proton_cli, PathBuf::from("/usr/bin/proton-drive"));
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
            full_scan_schedule: Some("weekly sun 03:00".to_owned()),
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
            full_scan_schedule: Some("weekly sun 03:00".to_owned()),
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

    /// The names serde says a struct accepts, read from its own refusal of a name it does not know.
    /// `deny_unknown_fields` makes the derive list **every accepted spelling, aliases included**
    /// (`expected one of …`), which is the only place an alias is data rather than an attribute.
    fn names_the_parser_accepts(text: &str) -> Vec<String> {
        let error = parse_file_config(text).expect_err("an unknown key is refused");
        let message = error.message();
        assert!(
            message.starts_with("unknown field `no_such_key`, expected"),
            "the derive's refusal changed shape, so this reads nothing: {message}"
        );
        // `unknown field `no_such_key`, expected one of `a`, `b`` — the backticked names after the
        // first are the accepted ones.
        let mut names: Vec<String> = message
            .split('`')
            .skip(1)
            .step_by(2)
            .skip(1)
            .map(str::to_owned)
            .collect();
        names.sort();
        names
    }

    /// A value of the right TOML type for each key, so a spelling can be tried in a real position.
    /// Exhaustive with no `_` arm: a key added to the engine must say what it holds before this
    /// test can run, which is the point of the test.
    fn sample_value(key: ConfigKey) -> &'static str {
        match key {
            ConfigKey::LocalRoot
            | ConfigKey::RemoteRoot
            | ConfigKey::DbPath
            | ConfigKey::SocketPath
            | ConfigKey::LockfilePath
            | ConfigKey::ProtonCli => "\"/x\"",
            ConfigKey::ScanIntervalSecs
            | ConfigKey::ProtonTimeoutSecs
            | ConfigKey::ProtonListAttempts
            | ConfigKey::DownloadBatchSize
            | ConfigKey::EventsFullScanEvery
            | ConfigKey::WarmStartFullWalkEvery
            | ConfigKey::WarmStartMaxCursorAgeSecs => "1",
            ConfigKey::FullScanSchedule => "\"weekly sun 03:00\"",
            ConfigKey::IncludePatterns | ConfigKey::ExcludePatterns => "[\"a\"]",
            ConfigKey::DryRun | ConfigKey::EventsDriven | ConfigKey::WarmStart => "true",
            ConfigKey::DeleteApproval => "{ remote = true }",
            ConfigKey::DeletionPolicyKey => "\"never\"",
            ConfigKey::LocalDeleteMode => "\"trash\"",
            ConfigKey::LogLevel => "\"info\"",
            ConfigKey::ConflictSuffix => "\"x\"",
        }
    }

    #[test]
    fn the_engine_accepts_exactly_the_listed_spellings() {
        // `ConfigKey::spellings` is a hand-written second copy of attributes, and a client (the GUI's
        // writer) edits a file by it: a spelling the parser accepts and the table omits is a key the
        // client reads as unset and then writes beside the one in force, which serde refuses as a
        // duplicate field. So the table is held to what the parser says, in BOTH directions.
        //
        // 1. The SET, read from serde itself: what a file's top level accepts is every key's
        //    spellings plus the `pair` container, and what a `[[pair]]` table accepts is the per-pair
        //    keys' spellings plus `name`. An alias added to the struct and not to the table (or the
        //    reverse) is a difference here.
        let mut top_level: Vec<String> = ConfigKey::ALL
            .into_iter()
            .flat_map(|key| key.spellings().iter().map(|s| (*s).to_owned()))
            .chain(["pair".to_owned()])
            .collect();
        top_level.sort();
        assert_eq!(
            names_the_parser_accepts("no_such_key = 1\n"),
            top_level,
            "FileConfig accepts a different set of spellings than ConfigKey::spellings lists"
        );
        let mut per_pair: Vec<String> = ConfigKey::ALL
            .into_iter()
            .filter(|key| key.scope() == KeyScope::Pair)
            .flat_map(|key| key.spellings().iter().map(|s| (*s).to_owned()))
            .chain(["name".to_owned()])
            .collect();
        per_pair.sort();
        assert_eq!(
            names_the_parser_accepts("[[pair]]\nname = \"a\"\nno_such_key = 1\n"),
            per_pair,
            "a [[pair]] table accepts a different set of spellings than the per-pair keys list"
        );

        // 2. The BEHAVIOUR: every listed spelling really parses, in the position its scope names,
        //    and a daemon-wide key really does not parse inside a pair table.
        for key in ConfigKey::ALL {
            assert_eq!(
                key.spellings()[0],
                key.spelling(),
                "the canonical spelling is first"
            );
            for spelling in key.spellings() {
                let value = sample_value(key);
                parse_file_config(&format!("{spelling} = {value}\n"))
                    .unwrap_or_else(|e| panic!("top level `{spelling}`: {e}"));
                let in_pair =
                    parse_file_config(&format!("[[pair]]\nname = \"a\"\n{spelling} = {value}\n"));
                assert_eq!(
                    in_pair.is_ok(),
                    key.scope() == KeyScope::Pair,
                    "`{spelling}` inside a [[pair]] table: {in_pair:?}"
                );
            }
        }

        // 3. The near-misses that matter: the glob lists do NOT have the kebab alias every other key
        //    has, which is exactly what a snake-to-kebab-only writer got wrong (F-D).
        for near_miss in [
            "include-patterns",
            "exclude-patterns",
            "Local_Root",
            "local root",
        ] {
            assert!(
                parse_file_config(&format!("\"{near_miss}\" = [\"a\"]\n")).is_err(),
                "`{near_miss}` is not a spelling the parser accepts"
            );
        }
    }

    #[test]
    fn a_spelling_names_one_key_and_one_key_only() {
        // `from_spelling` is the inverse the writer asks ("which setting is this key?"), so two keys
        // sharing a spelling would make its answer depend on iteration order.
        let mut all: Vec<&str> = ConfigKey::ALL
            .into_iter()
            .flat_map(|key| key.spellings().iter().copied())
            .collect();
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "two keys share a spelling");
        for key in ConfigKey::ALL {
            for spelling in key.spellings() {
                assert_eq!(ConfigKey::from_spelling(spelling), Some(key), "{spelling}");
            }
        }
        for unknown in ["pair", "name", "include-patterns", ""] {
            assert_eq!(ConfigKey::from_spelling(unknown), None, "{unknown:?}");
        }
    }

    /// Two pairs whose folders are lexically apart and really one inside the other: the shape the
    /// lexical rule cannot see and the boot check refuses fatally.
    fn two_pairs_through_a_symlink(base: &Path) -> String {
        let outer = base.join("outer");
        fs::create_dir_all(outer.join("inner")).expect("outer/inner");
        let alias = base.join("alias");
        std::os::unix::fs::symlink(outer.join("inner"), &alias).expect("symlink");
        format!(
            "[[pair]]\nname = \"outer\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/a\"\n\n\
             [[pair]]\nname = \"inner\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/b\"\n",
            outer.display(),
            alias.display()
        )
    }

    #[test]
    fn a_symlinked_alias_passes_the_lexical_check_and_fails_the_real_path_one() {
        let directory = tempdir().expect("tempdir");
        let text = two_pairs_through_a_symlink(directory.path());
        validate_file_config_text(&text).expect("the lexical rule cannot see a symlink");
        let error = real_path_conflicts(&text)
            .expect_err("the real folders nest")
            .to_string();
        // The daemon's own sentence, naming the pair boot would name as the candidate (the later
        // table, against each earlier one) and both the written and the real form of the path.
        assert!(
            error.starts_with("folder pair 'inner': its local_root"),
            "{error}"
        );
        assert!(error.contains("is inside folder pair 'outer''s"), "{error}");
        assert!(error.contains("(really `"), "{error}");
    }

    #[test]
    fn real_path_conflicts_asks_the_daemons_function_and_gets_the_daemons_sentence() {
        // The split is behaviour-preserving only if both ends reach ONE body. Put the same two
        // pairs to `real_path_overlap` directly and the text-level answer must be that string.
        let directory = tempdir().expect("tempdir");
        let text = two_pairs_through_a_symlink(directory.path());
        let views = pair_views(&text).expect("views");
        fn paths(view: &PairView) -> PairStatePaths<'_> {
            PairStatePaths {
                name: view.name.as_str(),
                local_root: view.local_root.as_deref().expect("local_root"),
                db_path: view.db_path.as_deref().expect("db_path"),
                lockfile_path: view.lockfile_path.as_deref().expect("lockfile_path"),
            }
        }
        let direct = real_path_overlap(&paths(&views[1]), &paths(&views[0]))
            .expect("the body finds the overlap");
        assert_eq!(
            real_path_conflicts(&text).unwrap_err().to_string(),
            direct,
            "the text-level check must be the body's sentence, verbatim"
        );
    }

    #[test]
    fn real_path_conflicts_has_nothing_to_say_about_apart_unplaced_or_single_pairs() {
        let directory = tempdir().expect("tempdir");
        let base = directory.path();
        for (text, why) in [
            (String::new(), "an empty file is one unplaced pair"),
            (
                "local_root = \"/x\"\nremote_root = \"/Drive/x\"\n".to_owned(),
                "one pair has nothing to overlap with",
            ),
            (
                format!(
                    "[[pair]]\nname = \"a\"\nlocal_root = \"{0}/a\"\nremote_root = \"/Drive/a\"\n\n\
                     [[pair]]\nname = \"b\"\nlocal_root = \"{0}/b\"\nremote_root = \"/Drive/b\"\n",
                    base.display()
                ),
                "two apart folders do not overlap, existing or not",
            ),
            (
                format!(
                    "[[pair]]\nname = \"a\"\nlocal_root = \"{0}/a\"\nremote_root = \"/Drive/a\"\n\n\
                     [[pair]]\nname = \"b\"\nremote_root = \"/Drive/b\"\n",
                    base.display()
                ),
                "a pair with no local_root yet is not placed, and is validate's to report",
            ),
        ] {
            real_path_conflicts(&text).unwrap_or_else(|e| panic!("{why}: {e}"));
        }
        // Not a config at all: the same refusal `pair_views` gives, not a silent pass.
        assert!(real_path_conflicts("local_root = \n").is_err());
    }

    #[test]
    fn a_state_file_placed_in_another_pairs_folder_is_a_real_path_conflict_too() {
        // The rule has four shapes; the folder-in-folder one is covered above. This is the one reached
        // AROUND the folder rule: pair b's index sits inside pair a's tree.
        let directory = tempdir().expect("tempdir");
        let base = directory.path();
        fs::create_dir_all(base.join("a")).expect("a");
        let text = format!(
            "[[pair]]\nname = \"a\"\nlocal_root = \"{0}/a\"\nremote_root = \"/Drive/a\"\n\n\
             [[pair]]\nname = \"b\"\nlocal_root = \"{0}/b\"\nremote_root = \"/Drive/b\"\n\
             db_path = \"{0}/a/b.db\"\n",
            base.display()
        );
        let error = real_path_conflicts(&text).unwrap_err().to_string();
        assert!(
            error.contains("its db_path") && error.contains("is inside folder pair 'a''s"),
            "{error}"
        );
    }

    /// Three pairs, lexically apart, in which the second is clear of both and the third really sits
    /// where the first is: `alias` is a symlink to a folder inside (or around) `one`.
    fn three_pairs_whose_first_and_third_overlap(base: &Path, third_is_inside: bool) -> String {
        let around_one = base.join("around-one");
        let one = around_one.join("one");
        let two = base.join("apart").join("two");
        fs::create_dir_all(one.join("inner")).expect("one/inner");
        fs::create_dir_all(&two).expect("two");
        let alias = base.join("alias");
        // Inside: `alias` leads to a folder under `one`. Around: `alias` leads to the folder that
        // holds `one` and nothing of `two`'s, so the later pair contains the earlier one.
        let target = if third_is_inside {
            one.join("inner")
        } else {
            around_one
        };
        std::os::unix::fs::symlink(&target, &alias).expect("symlink");
        format!(
            "[[pair]]\nname = \"one\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/1\"\n\n\
             [[pair]]\nname = \"two\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/2\"\n\n\
             [[pair]]\nname = \"three\"\nlocal_root = \"{}\"\nremote_root = \"/Drive/3\"\n",
            one.display(),
            two.display(),
            alias.display()
        )
    }

    #[test]
    fn real_path_conflicts_compares_each_pair_with_every_earlier_one_not_only_its_neighbour() {
        // The pair that overlaps is NOT next to the one it overlaps. A comparison of each pair with
        // only the one before it asks (two, one) and (three, two), finds both apart, and passes a
        // file the daemon refuses to start on — fatally, on a unit that restarts every ten seconds.
        let directory = tempdir().expect("tempdir");
        let text = three_pairs_whose_first_and_third_overlap(directory.path(), true);
        validate_file_config_text(&text).expect("the lexical rule sees three apart folders");
        let error = real_path_conflicts(&text)
            .expect_err("pair three really sits inside pair one")
            .to_string();
        assert!(
            error.starts_with("folder pair 'three': its local_root")
                && error.contains("is inside folder pair 'one''s"),
            "the sentence names the later pair as the candidate and the EARLIER, non-adjacent, pair: {error}"
        );
    }

    #[test]
    fn real_path_conflicts_finds_a_later_pair_that_contains_a_non_adjacent_earlier_one() {
        // The other direction of the same rule: the candidate (three) is the OUTER folder.
        let directory = tempdir().expect("tempdir");
        let text = three_pairs_whose_first_and_third_overlap(directory.path(), false);
        let error = real_path_conflicts(&text)
            .expect_err("pair three really contains pair one")
            .to_string();
        assert!(
            error.starts_with("folder pair 'three': its local_root")
                && error.contains("contains folder pair 'one''s"),
            "{error}"
        );
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
        // may reasonably disagree about whether deletions are recoverable. It was written to run at
        // this layer rather than through `resolve_runtime_config` because a two-pair config used to
        // be refused before the daemon ever saw one; `two_pairs_resolve_to_two_runtime_configs_in_
        // file_order` now proves the end-to-end half.
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
    ///
    /// Named `default` in the tabled form (#102 phase 3), not an arbitrary name: the implicit
    /// top-level pair is *always* `DEFAULT_PAIR_NAME`, and `DaemonConfig::name` is now part of the
    /// resolved answer this asserts is identical — a `[[pair]]` table named anything else would be
    /// comparing two different pairs, not two spellings of the same one.
    fn assert_spellings_agree(daemon_wide: &str, per_pair_body: &str, input: &DaemonConfigInput) {
        let flat = format!("{daemon_wide}{per_pair_body}");
        let tabled =
            format!("{daemon_wide}\n[[pair]]\nname = \"{DEFAULT_PAIR_NAME}\"\n{per_pair_body}");
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
    fn more_than_one_pair_is_accepted_at_startup_and_at_save_time() {
        // Phase 4c lifts the count gate (phase 1 refused more than one pair so the SHAPE could land
        // before the runtime that serializes passes existed). Accepted on BOTH readers' paths, for
        // the reason they were both refused: `ConfigDoc::save` never writes a config the daemon
        // would refuse to start on, and the daemon never refuses one the GUI would write.
        let text = "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n";
        validate_file_config_text(text).expect("two pairs are a valid file");

        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(&path, text).expect("write config");
        let configs = resolve_runtime_configs(DaemonConfigInput {
            config: Some(path.clone()),
            ..DaemonConfigInput::default()
        })
        .expect("two pairs must resolve at startup");
        assert_eq!(configs.pairs.len(), 2);

        // The single-pair entry point is not a second way to refuse them quietly: it says it
        // resolves one, and names the way to resolve all.
        let error = resolve_runtime_config(DaemonConfigInput {
            config: Some(path),
            ..DaemonConfigInput::default()
        })
        .expect_err("the single-pair wrapper needs exactly one pair");
        assert!(
            error.to_string().contains("resolve_runtime_configs")
                && error.to_string().contains("`a`")
                && error.to_string().contains("`b`"),
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

    // ---------------------------------------------------------------------------------------
    // #102 phase 4c: the lift. A config may declare any number of pairs; flags that amend "the"
    // pair are refused beside several (maintainer decision M2), and `dry_run = true` in a table is
    // refused beside several (M3).
    // ---------------------------------------------------------------------------------------

    /// A two-pair file with its daemon-wide half spelled out (so the comparison does not lean on
    /// the machine's XDG directories), and two pairs that differ in every way a test reads.
    const TWO_PAIRS: &str = "\
socket_path = \"/tmp/two-pairs.sock\"
proton_cli = \"/usr/bin/fake-proton-drive\"
proton_timeout_secs = 17
proton_list_attempts = 4
log_level = \"warn\"

[[pair]]
name = \"a\"
local_root = \"/local/a\"
remote_root = \"/Drive/a\"
scan_interval_secs = 11
exclude = [\"*.tmp\"]
deletion_policy = \"only_permanent\"

[[pair]]
name = \"b\"
local_root = \"/local/b\"
remote_root = \"/Drive/b\"
scan_interval_secs = 22
events_driven = false
local_delete_mode = \"permanent\"
";

    /// `text` written to a fresh temp file and resolved with `input`, as the binary does.
    fn resolve_all(text: &str, input: DaemonConfigInput) -> AppResult<RuntimeConfigs> {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("proton-sync.toml");
        fs::write(&path, text).expect("write config");
        resolve_runtime_configs(DaemonConfigInput {
            config: Some(path),
            ..input
        })
    }

    #[test]
    fn two_pairs_resolve_to_two_runtime_configs_in_file_order() {
        let configs = resolve_all(TWO_PAIRS, DaemonConfigInput::default()).expect("two pairs");
        assert_eq!(configs.mode, RunMode::Daemon);
        let names: Vec<&str> = configs.pairs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b"], "file order is the default-pair order");
        let (a, b) = (&configs.pairs[0], &configs.pairs[1]);
        // Each pair carries ITS OWN values, not the first table's and not the defaults.
        assert_eq!(a.local_root, PathBuf::from("/local/a"));
        assert_eq!(b.local_root, PathBuf::from("/local/b"));
        assert_eq!(b.remote_root, PathBuf::from("/Drive/b"));
        assert_eq!(a.scan_interval, Duration::from_secs(11));
        assert_eq!(b.scan_interval, Duration::from_secs(22));
        assert_eq!(a.exclude_patterns, vec!["*.tmp"]);
        assert!(b.exclude_patterns.is_empty());
        assert!(a.events_driven && !b.events_driven);
        assert!(!a.delete_approval_remote && a.delete_approval_local);
        assert!(b.delete_approval_remote && b.delete_approval_local);
        assert_eq!(a.local_delete_mode, LocalDeleteMode::Trash);
        assert_eq!(b.local_delete_mode, LocalDeleteMode::Permanent);
        // And each pair's state paths default under ITS root.
        assert_eq!(a.db_path, default_state_db_path(Path::new("/local/a")));
        assert_eq!(b.db_path, default_state_db_path(Path::new("/local/b")));
        assert_eq!(
            b.lockfile_path,
            default_lockfile_path(Path::new("/local/b"))
        );
    }

    // ---- `pair_views` (#102 phase 5a): the tolerant reader a GUI uses ----------------------------

    /// The daemon's own answer for each pair of a COMPLETE file, as the views carry it. The oracle
    /// `pair_views` is held to: it is the strict reader, so a view that disagrees with it is a GUI
    /// operating on a file other than the one the daemon opens.
    fn views_of_the_daemon(text: &str) -> Vec<PairView> {
        resolve_all(text, DaemonConfigInput::default())
            .expect("a complete file resolves")
            .pairs
            .into_iter()
            .map(|config| PairView {
                name: config.name,
                local_root: Some(config.local_root),
                remote_root: Some(config.remote_root),
                db_path: Some(config.db_path),
                lockfile_path: Some(config.lockfile_path),
                // `ConflictNaming` is the resolved form of the literal the view carries; compare in
                // the resolved one so the default (`None`) and its spelling agree.
                conflict_suffix: Some(config.conflict_naming.suffix().to_owned()),
            })
            .collect()
    }

    fn views_of_the_file(text: &str) -> Vec<PairView> {
        pair_views(text)
            .expect("a complete file parses")
            .into_iter()
            .map(|view| PairView {
                conflict_suffix: Some(
                    view.conflict_suffix
                        .map(|suffix| {
                            ConflictNaming::new(&suffix)
                                .expect("a valid suffix")
                                .suffix()
                                .to_owned()
                        })
                        .unwrap_or_else(|| ConflictNaming::default().suffix().to_owned()),
                ),
                ..view
            })
            .collect()
    }

    /// `pair_views` == `resolve_runtime_configs`, pair for pair, over complete files. Every file here
    /// names its daemon-wide half, so the comparison leans on no XDG directory.
    #[test]
    fn pair_views_agrees_with_the_daemon_resolver() {
        let daemon_wide = "socket_path = \"/tmp/pair-views.sock\"\n\
             proton_cli = \"/usr/bin/fake-proton-drive\"\n";
        let corpus = [
            // Implicit pair, nothing but its two roots: both state paths are the `.sync` defaults.
            format!("{daemon_wide}local_root = \"/local/a\"\nremote_root = \"/Drive/a\"\n"),
            // Implicit pair with every path override: absolute, relative (under the root) and a
            // non-default conflict suffix.
            format!(
                "{daemon_wide}local_root = \"/local/a\"\nremote_root = \"/Drive/a\"\n\
                 db_path = \"state/index.db\"\nlockfile_path = \"/var/lock/a.lock\"\n\
                 conflict_suffix = \"cloud-copy\"\n"
            ),
            // The same pair as a `[[pair]]` table.
            format!(
                "{daemon_wide}[[pair]]\nname = \"default\"\nlocal_root = \"/local/a\"\n\
                 remote_root = \"/Drive/a\"\ndb_path = \"state/index.db\"\n"
            ),
            // Two pairs that differ in every way a view reads, kebab and snake spellings alike.
            format!(
                "{daemon_wide}[[pair]]\nname = \"a\"\nlocal_root = \"/local/a\"\n\
                 remote_root = \"/Drive/a\"\nconflict_suffix = \"alpha\"\n\
                 [[pair]]\nname = \"b\"\nlocal-root = \"/local/b\"\nremote-root = \"/Drive/b\"\n\
                 db-path = \"/elsewhere/b.db\"\nlockfile-path = \"b-state/lock\"\n\
                 conflict-suffix = \"beta\"\n"
            ),
            TWO_PAIRS.to_owned(),
            // A leading `~`: both readers expand it with the same function (the comparison is on the
            // answer, so a machine with no HOME is skipped below rather than failing).
            format!(
                "{daemon_wide}local_root = \"~/pair-views\"\nremote_root = \"/Drive/a\"\n\
                 db_path = \"~/pair-views-state/index.db\"\n"
            ),
        ];
        for text in &corpus {
            if resolve_all(text, DaemonConfigInput::default()).is_err() {
                // Only the `~` file can refuse here, and only with HOME unset.
                assert!(
                    text.contains('~'),
                    "a corpus file failed to resolve:\n{text}"
                );
                continue;
            }
            assert_eq!(
                views_of_the_file(text),
                views_of_the_daemon(text),
                "pair_views disagrees with the daemon resolver for:\n{text}"
            );
        }
    }

    /// A settings screen shows a document part-way through an edit, and `resolve_pairs` refuses
    /// every one of these. The view must still answer.
    #[test]
    fn pair_views_survives_a_half_written_file() {
        let half_written = [
            // Two pairs with the same root: `validate_pair_roots` refuses it.
            "[[pair]]\nname = \"a\"\nlocal_root = \"/same\"\nremote_root = \"/Drive/a\"\n\
             [[pair]]\nname = \"b\"\nlocal_root = \"/same\"\nremote_root = \"/Drive/b\"\n",
            // Beside other pairs a table with no roots: `require_roots_in_every_table`.
            "[[pair]]\nname = \"a\"\nlocal_root = \"/local/a\"\nremote_root = \"/Drive/a\"\n\
             [[pair]]\nname = \"b\"\n",
            // A reserved name in the second slot, and a duplicate name.
            "[[pair]]\nname = \"a\"\n[[pair]]\nname = \"default\"\n",
            "[[pair]]\nname = \"a\"\n[[pair]]\nname = \"A\"\n",
            // A name the charset refuses.
            "[[pair]]\nname = \"has space\"\nlocal_root = \"/x\"\nremote_root = \"/Drive/x\"\n",
            // Both spellings at once.
            "local_root = \"/top\"\nremote_root = \"/Drive/top\"\n\
             [[pair]]\nname = \"a\"\nlocal_root = \"/local/a\"\nremote_root = \"/Drive/a\"\n",
        ];
        for text in half_written {
            let config = parse_file_config(text).expect("these parse");
            assert!(
                resolve_pairs(&config).is_err(),
                "the premise: the strict reader refuses this file:\n{text}"
            );
            assert!(
                pair_views(text).is_ok(),
                "the tolerant reader must still answer for:\n{text}"
            );
        }
        // An explicit empty list is a file that declares no pair, not an implicit one.
        assert!(pair_views("pair = []\n").expect("parses").is_empty());
        // Only text that is not a config file at all is an error: bad TOML, an unknown key (the
        // daemon's `deny_unknown_fields`), a table with no name.
        for text in [
            "local_root = [ this is not toml",
            "no_such_key = 1\n",
            "[[pair]]\nlocal_root = \"/x\"\n",
        ] {
            assert!(pair_views(text).is_err(), "should not read: {text}");
        }
    }

    /// The shape rules a view applies by itself, read off a file rather than the daemon: file order,
    /// the implicit pair's name, what is and is not placed.
    #[test]
    fn pair_views_places_only_what_the_file_places() {
        // Nothing at all is still one implicit pair, and nothing is placed.
        let views = pair_views("").expect("an empty file parses");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, DEFAULT_PAIR_NAME);
        assert_eq!(views[0].local_root, None);
        assert_eq!(views[0].db_path, None, "no root, nothing to default under");
        assert_eq!(views[0].lockfile_path, None);

        // A root places both defaults; a relative override is under the root; an absolute one is
        // not; the order of the tables is the order of the views.
        let views = pair_views(
            "[[pair]]\nname = \"z\"\nlocal_root = \"/r/z\"\nremote_root = \"/Drive/z\"\n\
             db_path = \"x/y.db\"\n\
             [[pair]]\nname = \"a\"\nlocal_root = \"/r/a\"\nlockfile_path = \"/abs/a.lock\"\n",
        )
        .expect("parses");
        assert_eq!(
            views.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["z", "a"]
        );
        assert_eq!(views[0].db_path, Some(PathBuf::from("/r/z/x/y.db")));
        assert_eq!(
            views[0].lockfile_path,
            Some(default_lockfile_path(Path::new("/r/z")))
        );
        assert_eq!(views[1].remote_root, None);
        assert_eq!(views[1].lockfile_path, Some(PathBuf::from("/abs/a.lock")));
        assert_eq!(
            views[1].db_path,
            Some(default_state_db_path(Path::new("/r/a")))
        );

        // A `~user` value the engine refuses is kept as typed; a state path the daemon would refuse
        // to open (blank, or a directory) is not placed rather than defaulted behind the user's back.
        let views = pair_views(
            "local_root = \"~nobody-here/x\"\ndb_path = \"\"\nlockfile_path = \"state/\"\n",
        )
        .expect("parses");
        assert_eq!(views[0].local_root, Some(PathBuf::from("~nobody-here/x")));
        assert_eq!(views[0].db_path, None);
        assert_eq!(views[0].lockfile_path, None);
    }

    #[test]
    fn the_daemon_wide_half_is_resolved_once_and_every_pair_carries_it() {
        let configs = resolve_all(TWO_PAIRS, DaemonConfigInput::default()).expect("two pairs");
        for config in &configs.pairs {
            assert_eq!(config.socket_path, PathBuf::from("/tmp/two-pairs.sock"));
            assert_eq!(
                config.proton_cli,
                PathBuf::from("/usr/bin/fake-proton-drive")
            );
            assert_eq!(config.proton_timeout, Duration::from_secs(17));
            assert_eq!(config.proton_list_attempts, 4);
            assert_eq!(config.log_filter, "warn");
            assert_eq!(config.global_lock_path, configs.pairs[0].global_lock_path);
        }
    }

    /// One row per per-pair flag: how it is spelled on the command line, and how to set it. The
    /// table is checked against [`DaemonConfigInput::per_pair_flags_set`] (so a flag the production
    /// list knows and this table does not, or the reverse, fails), and the test below walks it.
    #[allow(clippy::type_complexity)]
    fn per_pair_flag_table() -> Vec<(&'static str, fn(&mut DaemonConfigInput))> {
        vec![
            ("--local-root", |i| i.local_root = Some("/x".into())),
            ("--remote-root", |i| i.remote_root = Some("/Drive/x".into())),
            ("--db-path", |i| i.db_path = Some("/tmp/x.db".into())),
            ("--lockfile-path", |i| {
                i.lockfile_path = Some("/tmp/x.lock".into())
            }),
            ("--scan-interval-secs", |i| i.scan_interval_secs = Some(9)),
            ("--download-batch-size", |i| i.download_batch_size = Some(3)),
            ("--include", |i| i.include_patterns = vec!["a/**".into()]),
            ("--exclude", |i| i.exclude_patterns = vec!["*.tmp".into()]),
            ("--events-driven", |i| i.events_driven = true),
            ("--no-events-driven", |i| i.no_events_driven = true),
            ("--events-full-scan-every", |i| {
                i.events_full_scan_every = Some(4);
            }),
            ("--warm-start", |i| i.warm_start = true),
            ("--no-warm-start", |i| i.no_warm_start = true),
            ("--warm-start-full-walk-every", |i| {
                i.warm_start_full_walk_every = Some(5);
            }),
            ("--warm-start-max-cursor-age-secs", |i| {
                i.warm_start_max_cursor_age_secs = Some(6);
            }),
            ("--no-delete-approval", |i| i.no_delete_approval = true),
            ("--deletion-policy", |i| {
                i.deletion_policy = Some(DeletionPolicy::Never);
            }),
            ("--local-delete-mode", |i| {
                i.local_delete_mode = Some(LocalDeleteMode::Permanent);
            }),
            ("--conflict-suffix", |i| {
                i.conflict_suffix = Some("from-cloud".into())
            }),
        ]
    }

    #[test]
    fn every_daemon_config_input_field_is_classified() {
        // The exhaustive destructure in `per_pair_flags_set` is the compile-time half: a new field
        // cannot be added without being placed in one of three groups. This is the run-time half:
        // the table above names every per-pair flag, so the production list and the test's agree.
        let mut everything = DaemonConfigInput::default();
        for (_, set) in per_pair_flag_table() {
            set(&mut everything);
        }
        let mut produced = everything.per_pair_flags_set();
        let mut named: Vec<&str> = per_pair_flag_table()
            .iter()
            .map(|(flag, _)| *flag)
            .collect();
        produced.sort_unstable();
        named.sort_unstable();
        assert_eq!(
            produced, named,
            "the production list and the test table disagree"
        );

        // The other two groups are not per-pair: setting every one of them produces nothing.
        let allowed = DaemonConfigInput {
            config: Some("/tmp/c.toml".into()),
            socket_path: Some("/tmp/s.sock".into()),
            proton_cli: Some("/usr/bin/p".into()),
            proton_timeout_secs: Some(5),
            proton_list_attempts: Some(2),
            log_level: Some("info".into()),
            rust_log: Some("info".into()),
            dry_run: true,
            no_dry_run: true,
            force_full_walk: true,
            pair: Some("a".into()),
            ..DaemonConfigInput::default()
        };
        assert!(allowed.per_pair_flags_set().is_empty());
    }

    #[test]
    fn per_pair_flags_are_refused_with_more_than_one_pair() {
        for (flag, set) in per_pair_flag_table() {
            let mut input = DaemonConfigInput::default();
            set(&mut input);
            let error = resolve_all(TWO_PAIRS, input.clone())
                .expect_err(&format!("{flag} beside two pairs must be refused"))
                .to_string();
            assert!(
                error.contains(flag),
                "the refusal names {flag}, got {error}"
            );
            assert!(
                error.contains("`a`") && error.contains("`b`"),
                "and the pairs it found, got {error}"
            );
            assert!(
                !error.contains("  "),
                "a run of spaces means a lost `\\`: {error:?}"
            );

            // The same flag beside ONE pair amends it, exactly as before: the rule keys on the
            // count, not on the flag.
            let single = "local_root = \"/local/a\"\nremote_root = \"/Drive/a\"\n";
            let mut input = DaemonConfigInput::default();
            set(&mut input);
            resolve_all(single, input)
                .unwrap_or_else(|error| panic!("{flag} beside one pair is accepted: {error}"));
        }

        // Several at once: every one of them is named, in one message.
        let mut everything = DaemonConfigInput::default();
        for (_, set) in per_pair_flag_table() {
            set(&mut everything);
        }
        let error = resolve_all(TWO_PAIRS, everything)
            .expect_err("all of them")
            .to_string();
        for (flag, _) in per_pair_flag_table() {
            assert!(error.contains(flag), "{flag} missing from {error}");
        }
    }

    #[test]
    fn daemon_wide_flags_still_apply_with_more_than_one_pair() {
        let configs = resolve_all(
            TWO_PAIRS,
            DaemonConfigInput {
                socket_path: Some("/tmp/from-flag.sock".into()),
                proton_cli: Some("/usr/bin/from-flag".into()),
                proton_timeout_secs: Some(99),
                proton_list_attempts: Some(8),
                log_level: Some("debug".into()),
                ..DaemonConfigInput::default()
            },
        )
        .expect("daemon-wide flags are not per-pair");
        assert_eq!(configs.pairs.len(), 2);
        for config in &configs.pairs {
            assert_eq!(config.socket_path, PathBuf::from("/tmp/from-flag.sock"));
            assert_eq!(config.proton_cli, PathBuf::from("/usr/bin/from-flag"));
            assert_eq!(config.proton_timeout, Duration::from_secs(99));
            assert_eq!(config.proton_list_attempts, 8);
            assert_eq!(config.log_filter, "debug");
        }
    }

    #[test]
    fn full_walk_applies_to_every_pair() {
        let configs = resolve_all(
            TWO_PAIRS,
            DaemonConfigInput {
                force_full_walk: true,
                ..DaemonConfigInput::default()
            },
        )
        .expect("--full-walk is a mode flag, allowed beside several pairs");
        assert!(
            configs.pairs.iter().all(|c| c.warm_start.force_full_walk),
            "--full-walk is every pair's first pass, not the first pair's"
        );
        let without = resolve_all(TWO_PAIRS, DaemonConfigInput::default()).expect("two pairs");
        assert!(without.pairs.iter().all(|c| !c.warm_start.force_full_walk));
    }

    #[test]
    fn with_more_than_one_pair_every_table_must_name_both_roots() {
        let header = "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\n";
        for (table_b, missing) in [
            ("local_root = \"/b\"\n", "`remote_root`"),
            ("remote_root = \"/Drive/b\"\n", "`local_root`"),
            ("", "`local_root` and `remote_root`"),
        ] {
            let text = format!("{header}[[pair]]\nname = \"b\"\n{table_b}");
            let message = validate_file_config_text(&text)
                .expect_err("a table without a root beside another pair is refused")
                .to_string();
            assert!(
                message.contains("pair `b`") && message.contains(missing),
                "names the pair and what it lacks ({missing}), got {message}"
            );
            // And the daemon's reader refuses it too, for the same reason (one resolver).
            let startup = resolve_all(&text, DaemonConfigInput::default())
                .expect_err("and at startup")
                .to_string();
            assert!(startup.contains("pair `b`"), "got {startup}");
        }

        // A flag cannot rescue it: beside several pairs there is no flag to try (they are
        // refused), which is the reason the rule exists. Before the lift a single pair's root could
        // come from a flag, and still can.
        validate_file_config_text("[[pair]]\nname = \"a\"\n")
            .expect("one pair: a flag may supply it");
        validate_file_config_text("[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n")
            .expect("two complete tables are fine");
    }

    #[test]
    fn dry_run_true_in_a_pair_table_is_refused_with_more_than_one_pair() {
        let with_dry_run = TWO_PAIRS.replace(
            "events_driven = false\n",
            "events_driven = false\ndry_run = true\n",
        );
        assert!(
            with_dry_run.contains("dry_run = true"),
            "the fixture edit applied"
        );

        // The file reader refuses it outright: a file has no flags to be told past.
        let message = validate_file_config_text(&with_dry_run)
            .expect_err("dry_run = true beside another pair is refused")
            .to_string();
        assert!(
            message.contains("`b`") && message.contains("dry_run"),
            "names the pair and the key, got {message}"
        );
        assert!(!message.contains("  "), "{message:?}");

        // So does the daemon, unless a flag says what the run is.
        let message = resolve_all(&with_dry_run, DaemonConfigInput::default())
            .expect_err("and at startup")
            .to_string();
        assert!(
            message.contains("`b`") && message.contains("dry_run"),
            "got {message}"
        );
        let preview = resolve_all(
            &with_dry_run,
            DaemonConfigInput {
                dry_run: true,
                ..DaemonConfigInput::default()
            },
        )
        .expect("--dry-run says what the run is");
        assert_eq!(preview.mode, RunMode::Preview { pair: 0 });
        let daemon = resolve_all(
            &with_dry_run,
            DaemonConfigInput {
                no_dry_run: true,
                ..DaemonConfigInput::default()
            },
        )
        .expect("--no-dry-run says what the run is");
        assert_eq!(daemon.mode, RunMode::Daemon);

        // `dry_run = false` is the default spelled out, and is accepted everywhere.
        let spelled_out = TWO_PAIRS.replace(
            "events_driven = false\n",
            "events_driven = false\ndry_run = false\n",
        );
        validate_file_config_text(&spelled_out).expect("false is the default");
        assert_eq!(
            resolve_all(&spelled_out, DaemonConfigInput::default())
                .expect("resolves")
                .mode,
            RunMode::Daemon
        );

        // With ONE pair `dry_run = true` still means what it always did.
        let one = resolve_all(
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndry_run = true\n",
            DaemonConfigInput::default(),
        )
        .expect("one pair");
        assert_eq!(one.mode, RunMode::Preview { pair: 0 });
    }

    #[test]
    fn a_preview_selector_resolves_the_same_whether_dry_run_came_from_the_flag_or_the_file() {
        // N = 1. The selector cannot be a clap `requires = "dry_run"`: whether the run is a preview
        // depends on the flag, the file's `dry_run` and `--no-dry-run` together, and clap cannot
        // see the file (`dry_run_cli`'s `pair_selects_a_preview_...` drives the binary).
        let from_file = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndry_run = true\n";
        let from_flag = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n";
        let named = |dry_run: bool| DaemonConfigInput {
            dry_run,
            pair: Some(DEFAULT_PAIR_NAME.to_owned()),
            ..DaemonConfigInput::default()
        };
        assert_eq!(
            resolve_all(from_file, named(false))
                .expect("file dry_run")
                .mode,
            RunMode::Preview { pair: 0 },
            "`dry_run = true` in the file makes it a preview, and --pair selects within it"
        );
        assert_eq!(
            resolve_all(from_flag, named(true))
                .expect("flag dry run")
                .mode,
            RunMode::Preview { pair: 0 }
        );
    }

    #[test]
    fn pair_is_validated_after_resolution_and_only_selects_a_preview() {
        // Not a preview: the daemon runs every pair, so there is nothing for --pair to select.
        let message = resolve_all(
            TWO_PAIRS,
            DaemonConfigInput {
                pair: Some("b".into()),
                ..DaemonConfigInput::default()
            },
        )
        .expect_err("--pair without a dry run is an error")
        .to_string();
        assert!(
            message.contains("--pair only selects which pair a dry run previews"),
            "got {message}"
        );
        // `--no-dry-run` over a file that says `dry_run = true` is a daemon run: still an error.
        let one = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndry_run = true\n";
        resolve_all(
            one,
            DaemonConfigInput {
                no_dry_run: true,
                pair: Some(DEFAULT_PAIR_NAME.to_owned()),
                ..DaemonConfigInput::default()
            },
        )
        .expect_err("--no-dry-run makes it a daemon run, and a daemon run has no selector");

        // A preview with no --pair is the default pair; with one, that pair, matched exactly.
        let preview = |pair: Option<&str>| {
            resolve_all(
                TWO_PAIRS,
                DaemonConfigInput {
                    dry_run: true,
                    pair: pair.map(str::to_owned),
                    ..DaemonConfigInput::default()
                },
            )
        };
        assert_eq!(
            preview(None).expect("default").mode,
            RunMode::Preview { pair: 0 }
        );
        assert_eq!(
            preview(Some("a")).expect("a").mode,
            RunMode::Preview { pair: 0 }
        );
        let configs = preview(Some("b")).expect("b");
        assert_eq!(configs.mode, RunMode::Preview { pair: 1 });
        assert_eq!(configs.preview().expect("the previewed pair").name, "b");
        // Byte-exact, like the wire: `B` is not `b`, and the refusal names the pairs that exist.
        let message = preview(Some("B")).expect_err("unknown name").to_string();
        assert!(
            message.contains("`B`") && message.contains("`a` and `b`"),
            "names what was asked and what exists, got {message}"
        );
        // It is validated AFTER resolution, so a config error elsewhere in the file wins: the user
        // is told the file is broken, not that a name was unknown.
        let broken = TWO_PAIRS.replace("scan_interval_secs = 22", "download_batch_size = 0");
        let message = resolve_all(
            &broken,
            DaemonConfigInput {
                dry_run: true,
                pair: Some("zzz".into()),
                ..DaemonConfigInput::default()
            },
        )
        .expect_err("a broken pair b")
        .to_string();
        assert!(
            message.contains("folder pair 'b'") && message.contains("download_batch_size"),
            "a resolution failure names its pair, got {message}"
        );
    }

    #[test]
    fn a_resolution_failure_beside_one_pair_reads_exactly_as_it_always_did() {
        let text = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndownload_batch_size = 0\n";
        let message = resolve_all(text, DaemonConfigInput::default())
            .expect_err("zero batch size")
            .to_string();
        assert_eq!(
            message, "download_batch_size must be greater than zero",
            "with one pair the message is exactly what it always was"
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
        // `/home/me/Sync`, and a byte-prefix check would wrongly refuse it. With the count gate
        // gone (phase 4c) such a file is simply valid, which is how this test tells the two apart.
        let siblings = "[[pair]]\nname = \"one\"\nlocal_root = \"/home/me/Sync\"\n\
             remote_root = \"/Drive/One\"\n\
             \n[[pair]]\nname = \"two\"\nlocal_root = \"/home/me/Sync2\"\n\
             remote_root = \"/Drive/Two\"\n";
        validate_file_config_text(siblings)
            .expect("sibling roots are not nested, and two pairs are accepted");
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

    /// A `[[pair]]` file with `existing` pairs and then one called `name`, every root apart.
    fn pair_file_with_a_last_pair(existing: &[&str], name: &str) -> String {
        let mut text = String::new();
        for (index, pair) in existing.iter().chain(std::iter::once(&name)).enumerate() {
            text.push_str(&format!(
                "[[pair]]\nname = \"{pair}\"\nlocal_root = \"/p{index}\"\n\
                 remote_root = \"/Drive/p{index}\"\n\n"
            ));
        }
        text
    }

    #[test]
    fn names_are_validated_by_the_engines_rule() {
        // THE CLIENT-FACING WRAPPERS ARE THE FILE READER'S BODY, not a second statement of it. For
        // every rule the set has, the sentence a form gets as someone types is the one the daemon
        // would exit with for a `[[pair]]` table of that name in that place — compared whole, so a
        // regex kept on the other side of the bridge (the thing this exists to prevent) differs.
        let too_long = "a".repeat(PAIR_NAME_MAX_LEN + 1);
        let refused: Vec<(Vec<&str>, &str)> = vec![
            (vec![], ""),
            (vec![], "my docs"),
            (vec![], "caf\u{e9}"),
            (vec![], "."),
            (vec![], ".."),
            (vec![], "-h"),
            (vec![], "--pair"),
            (vec![], too_long.as_str()),
            (vec!["photos"], "Photos"),
            (vec!["Photos"], "photos"),
            (vec!["a"], "default"),
            (vec!["a"], "Default"),
            (vec!["a", "b"], "default"),
            (vec!["default"], "DEFAULT"),
        ];
        for (existing, name) in refused {
            let wrapper = validate_pair_name_among(name, &existing, existing.len());
            let file = validate_file_config_text(&pair_file_with_a_last_pair(&existing, name));
            match (&wrapper, &file) {
                (Err(wrapper), Err(file)) => {
                    assert_eq!(wrapper, &file.to_string(), "`{name}` after {existing:?}")
                }
                _ => panic!(
                    "`{name}` after {existing:?}: wrapper {wrapper:?}, file {file:?}: both must refuse"
                ),
            }
        }

        // And the names a file accepts, a form accepts.
        let accepted: Vec<(Vec<&str>, &str)> = vec![
            (vec![], "default"),
            (vec![], "Default"),
            (vec!["default"], "photos"),
            (vec!["a", "b"], "c.d_e-f"),
        ];
        for (existing, name) in accepted {
            validate_pair_name_among(name, &existing, existing.len())
                .unwrap_or_else(|error| panic!("`{name}` after {existing:?}: {error}"));
            validate_file_config_text(&pair_file_with_a_last_pair(&existing, name)).unwrap_or_else(
                |error| panic!("the file refuses `{name}` after {existing:?}: {error}"),
            );
        }
    }

    #[test]
    fn a_name_is_checked_against_every_other_pair_not_only_the_ones_before_it() {
        // The file reader reports the LATER of two equal names (it has only seen the earlier one);
        // a client adding a name has no "before" and must be refused for a clash with any other
        // pair. Position decides only the `default` reservation.
        assert!(validate_pair_name_among("Photos", &["a", "photos"], 2).is_err());
        assert!(validate_pair_name_among("photos", &["PHOTOS", "a"], 0).is_err());
        assert!(validate_pair_name_among("photos", &["a", "b"], 2).is_ok());
        // The name alone has no other pair to clash with, and no position.
        assert!(validate_pair_name("default").is_ok());
        assert!(validate_pair_name("Photos").is_ok());
        assert!(validate_pair_name("").is_err());
        // `default` belongs to the first table, wherever the list of others is empty or not.
        assert!(validate_pair_name_among("default", &["a"], 0).is_ok());
        assert!(validate_pair_name_among("default", &[], 1).is_err());
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
                // #341: an empty db_path used to pass this reader and resolve to the sync root
                // itself, since it has no equivalent of the roots' blank check.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"\"",
                "db_path must not be empty",
            ),
            (
                // #341's sibling: a whitespace-only lockfile_path is the blank rule, not an
                // is-empty check — it carries no visible bytes but is not the empty string.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 lockfile_path = \"   \"",
                "lockfile_path must not be empty",
            ),
            (
                // #357: `.` is not blank, but `require_state_path_names_a_file` must still catch
                // it — the second, non-blank value class that reaches the same failure.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \".\"",
                "db_path must name a file",
            ),
            (
                // #357's `..` case: worse than `.` because it escapes the pair's own root entirely.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 lockfile_path = \"..\"",
                "lockfile_path must name a file",
            ),
            (
                // #357: `foo/..` lexically ends in `..` and names no file, exactly like a bare `..`.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"foo/..\"",
                "db_path must name a file",
            ),
            (
                // #357 widened: a trailing separator is POSIX's own "this names a directory",
                // independent of any `.`/`..` component — `Path::file_name()` alone cannot see it
                // (it still returns `Some("state")`), only the raw bytes can.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"state/\"",
                "db_path must name a file",
            ),
            (
                // #357 widened: `state/.` is an INTERIOR `CurDir`, which `Components` drops
                // silently — `Path::new("state/.").file_name()` is `Some("state")`, so this
                // spelling is invisible to a `file_name()`-only rule and needs the raw-byte check.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
                 lockfile_path = \"state/.\"",
                "lockfile_path must name a file",
            ),
            (
                // #357 widened: bare `~` names the home directory itself, not a file inside it —
                // checked before `expand_tilde` runs, since post-expansion `/home/x` looks like an
                // ordinary file name to `file_name()`.
                "name = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"~\"",
                "db_path must name a file",
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

    /// #357, at the file reader, in the top-level (implicit `default` pair) spelling the array test
    /// above does not exercise: the full widened value class — `.`, `..`, `foo/..`, a trailing
    /// separator (`state/`), an interior `CurDir` (`state/.`, invisible to `Path::file_name()`), and
    /// bare `~`/`~/` — are refused for both `db_path` and `lockfile_path`, while every near-miss
    /// spelling and an ordinary relative path are untouched.
    #[test]
    fn a_state_path_that_normalizes_to_no_file_name_is_refused_by_the_file_reader() {
        let roots = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\n";
        for key in ["db_path", "lockfile_path"] {
            for degenerate in [".", "..", "foo/..", "state/.", "state/", "~", "~/"] {
                let text = format!("{roots}{key} = \"{degenerate}\"\n");
                let error = validate_file_config_text(&text)
                    .expect_err(&format!("{key} = {degenerate:?} must be refused"));
                assert!(
                    error
                        .to_string()
                        .contains(&format!("{key} must name a file")),
                    "{key} = {degenerate:?}: got {error}"
                );
            }
            // Negative: every one of these lexically resembles a degenerate spelling somewhere in
            // it, but the last real component (or the whole value, for `~/x.db`) still names a
            // file, and a plain relative path is of course untouched.
            for benign in [
                "foo/../bar.db",
                "./index.db",
                "a/./b.db",
                ".hidden.db",
                "..hidden.db",
                "...",
                "~/x.db",
                "state/index.db",
            ] {
                let text = format!("{roots}{key} = \"{benign}\"\n");
                validate_file_config_text(&text)
                    .unwrap_or_else(|e| panic!("{key} = {benign:?} must still pass: {e}"));
            }
        }
    }

    /// **Pins `require_state_path_names_a_file` directly**, calling the function itself rather than
    /// only through the two readers that wrap it — the shape `src/index.rs`'s
    /// `pruning_an_empty_history_deletes_nothing_and_does_not_error` uses to pin the `IS NOT`
    /// retention predicate at `src/index.rs:1273` directly, not only through `prune_history`'s
    /// callers. Without this, `""` — the row that proves arm 1 (`file_name().is_none()`) is
    /// load-bearing at all — is untestable through either real seam: `require_non_blank_value`
    /// catches it first at both, so no reader-level test can ever exercise this function with a
    /// blank value. Every row here is one this function must answer the same way regardless of who
    /// calls it, so a future edit to any arm has something that fails immediately, at the unit that
    /// owns the rule, rather than three call sites away.
    #[test]
    fn a_direct_truth_table_pins_every_arm_of_require_state_path_names_a_file() {
        let refused = [
            ".",
            "..",
            "foo/..",
            "state/.",
            "state/..",
            "state/",
            "~",
            "~/",
            "./",
            "../",
            "./..",
            "../.",
            "state/./",
            "state/../",
            "a/b/..",
            "~/.",
            "//",
            "state//",
            "/",
            "",
        ];
        for value in refused {
            let error = require_state_path_names_a_file(Some(Path::new(value)), "db_path")
                .expect_err(&format!("{value:?} must be refused"));
            assert!(
                error.to_string().contains("db_path must name a file"),
                "{value:?}: got {error}"
            );
        }

        let accepted = [
            "foo/../bar.db",
            "./index.db",
            "a/./b.db",
            ".hidden.db",
            "..hidden.db",
            "...",
            "~/x.db",
            "state/index.db",
            "/elsewhere/index.db",
            // Files literally NAMED `~`, wherever they sit: `expand_tilde` only ever touches a
            // LEADING `~` component, so a `~` anywhere else is just a character in a file name.
            "state/~",
            "./~",
            "/home/me/~",
            "~/sub/x.db",
        ];
        for value in accepted {
            require_state_path_names_a_file(Some(Path::new(value)), "db_path")
                .unwrap_or_else(|e| panic!("{value:?} must be accepted: {e}"));
        }

        // Non-UTF-8: judged the same way a valid path is, never panicking and never silently waved
        // through.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        require_state_path_names_a_file(Some(Path::new(OsStr::from_bytes(b"\xff.db"))), "db_path")
            .expect("a non-UTF-8 file name must be accepted");
        let error = require_state_path_names_a_file(
            Some(Path::new(OsStr::from_bytes(b"\xff/.."))),
            "db_path",
        )
        .expect_err("a non-UTF-8 path lexically ending in .. must be refused");
        assert!(
            error.to_string().contains("db_path must name a file"),
            "got {error}"
        );

        // Absent is not a value to judge: no override at all, nothing is refused.
        require_state_path_names_a_file(None, "db_path").expect("absent is not a refusal");
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
            // Phase 4c's two file rules. (Two valid pairs used to stand here as "refused by the
            // count gate"; they are a valid file now.)
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\n",
            "[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\ndry_run = true\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/b\"\nremote_root = \"/Drive/b\"\n\
             dry_run = true\n",
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
        // uses — a flag cannot say WHICH pair it amends, so beside several pairs every one of them
        // is refused instead (`per_pair_flags_are_refused_with_more_than_one_pair`).
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

    /// The same flag-maskability, for `db_path` (#341): `effective_state_path`'s new blank check
    /// runs on the value *after* `input.db_path.or(pair.db_path)`, so a `--db-path` flag masks a
    /// file's blank value exactly the way `--local-root` already does — `Option::or` never
    /// evaluates the file side once the flag has answered.
    #[test]
    fn a_blank_file_db_path_is_masked_by_a_db_path_flag_the_same_as_local_root() {
        let text = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\ndb_path = \"\"\n";
        let flagged = DaemonConfigInput {
            db_path: Some(PathBuf::from("/from-flag/index.db")),
            ..DaemonConfigInput::default()
        };
        assert!(
            resolve_spelling(text, &flagged).starts_with("Ok("),
            "a db_path flag still rescues a file value the daemon will never use"
        );
        assert_eq!(
            resolve_spelling(text, &DaemonConfigInput::default()),
            "Err(db_path must not be empty)".to_owned(),
            "with no flag the merged blank db_path is refused"
        );
    }

    /// The same flag-maskability, for `proton_cli` (#356): `validate_runtime_config`'s new check
    /// runs on the merged value after `input.proton_cli.or(file_config.proton_cli)`, so a
    /// `--proton-cli` flag masks a file's blank value exactly the way `--db-path` already does.
    #[test]
    fn a_blank_file_proton_cli_is_masked_by_a_proton_cli_flag_the_same_as_db_path() {
        let text = "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nproton_cli = \"\"\n";
        let flagged = DaemonConfigInput {
            proton_cli: Some(PathBuf::from("/from-flag/proton-drive")),
            ..DaemonConfigInput::default()
        };
        assert!(
            resolve_spelling(text, &flagged).starts_with("Ok("),
            "a proton_cli flag still rescues a file value the daemon will never use"
        );
        assert_eq!(
            resolve_spelling(text, &DaemonConfigInput::default()),
            "Err(proton_cli must not be empty)".to_owned(),
            "with no flag the merged blank proton_cli is refused"
        );
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
            message.contains("`/home/me/A/index.db`") && message.contains("`/home/me/A`"),
            "the refusal names both paths it found, got {message}"
        );
        assert!(
            !message.contains("not yet supported"),
            "a genuinely broken multi-pair file must say what is wrong with it rather than be \
             masked by the count gate, got {message}"
        );

        // Same shape, one layer down: `validate_pair_roots` used to collect state paths only inside
        // `if let Some(local_root)`, so two pairs with no root at all (legal then — a flag could
        // supply it) and one shared absolute `db_path` were never compared with each other. Since
        // phase 4c a file can no longer say that (with several pairs every table sets both roots,
        // see `with_more_than_one_pair_every_table_must_name_both_roots`), so this reaches the layer
        // directly: it is kept as the floor under that rule, not as something a file can still do.
        let parsed = parse_file_config(
            "[[pair]]\nname = \"a\"\nremote_root = \"/Drive/a\"\ndb_path = \"/tmp/shared.db\"\n\
             \n[[pair]]\nname = \"b\"\nremote_root = \"/Drive/b\"\ndb_path = \"/tmp/shared.db\"\n",
        )
        .expect("parses");
        let pairs: Vec<PairFileConfig> = parsed
            .pair
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(PairFileConfig::from_table)
            .collect();
        let error = validate_pair_roots(&pairs)
            .expect_err("two rootless pairs sharing an index are refused by the layer itself");
        assert!(error.to_string().contains("db_path"), "got {error}");
    }

    #[test]
    fn a_full_scan_schedule_is_read_by_both_readers_or_refused_by_both() {
        // #193. A schedule that does not parse is a full sweep that silently never runs, so it is
        // fatal at startup like `log_level` — and the FILE reader has to agree, because
        // `ConfigDoc::save` validates through it alone and must never write a config the daemon
        // cannot start on.
        for body in [
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nfull_scan_schedule = \"weekly sun 03:00\"\n",
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nfull_scan_schedule = \"monthly day 15, 03:00\"\n",
            // The kebab alias, like every other key.
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nfull-scan-schedule = \"weekly sun 03:00\"\n",
        ] {
            for text in [body.to_owned(), format!("[[pair]]\nname = \"a\"\n{body}")] {
                validate_file_config_text(&text).unwrap_or_else(|error| {
                    panic!("{text:?} is a schedule the daemon can read: {error}")
                });
            }
        }
        for body in [
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nfull_scan_schedule = \"every sunday\"\n",
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nfull_scan_schedule = \"weekly sun 3:00\"\n",
            "local_root = \"/a\"\nremote_root = \"/Drive/a\"\nfull_scan_schedule = \"monthly day 32, 03:00\"\n",
        ] {
            for text in [body.to_owned(), format!("[[pair]]\nname = \"a\"\n{body}")] {
                let error = validate_file_config_text(&text)
                    .expect_err("a schedule the daemon cannot read is refused");
                assert!(
                    error.to_string().contains("full_scan_schedule"),
                    "in {text:?}: got {error}"
                );
            }
        }

        // The merge path resolves it to a typed value, and an unreadable one stops the daemon
        // there too — the reader that matters most, since it is the one that would otherwise run
        // for months with a safety net nobody notices is off.
        let directory = tempdir().expect("tempdir");
        let resolve = |body: &str| {
            let path = directory.path().join(format!("{}.toml", body.len()));
            fs::write(
                &path,
                format!("local_root = \"/a\"\nremote_root = \"/Drive/a\"\n{body}"),
            )
            .expect("write config");
            resolve_runtime_config(DaemonConfigInput {
                config: Some(path),
                socket_path: Some(PathBuf::from("/run/user/1000/p.sock")),
                ..Default::default()
            })
        };

        let (config, _) = resolve("full_scan_schedule = \"monthly day 31, 04:30\"\n")
            .expect("a readable schedule resolves");
        assert_eq!(
            config.full_scan_schedule.map(|s| s.to_string()).as_deref(),
            Some("monthly day 31, 04:30")
        );

        let error = resolve("full_scan_schedule = \"sometimes\"\n")
            .expect_err("an unreadable schedule stops the daemon");
        assert!(
            error.to_string().contains("full_scan_schedule"),
            "got {error}"
        );

        // THE `key_present` ARM, which the classification guards do NOT cover: `ConfigKey::scope`
        // is exhaustive so the compiler demands an arm there, but `key_present` returning the wrong
        // `bool` is free — measured, the whole 112-test config suite stayed green with this key's
        // arm hard-wired to `false`. Rule 1 reads that answer, so a `false` would accept a file
        // that sets the schedule at the top level AND declares `[[pair]]` tables, silently dropping
        // the top-level one. Found by adversarial review.
        let error = validate_file_config_text(
            "full_scan_schedule = \"weekly sun 03:00\"\n\
             \n[[pair]]\nname = \"a\"\nlocal_root = \"/a\"\nremote_root = \"/Drive/a\"\n",
        )
        .expect_err("one setting written two ways has no defensible precedence");
        assert!(
            error.to_string().contains("full_scan_schedule"),
            "the refusal must name the offending key, got {error}"
        );

        // And unset stays unset: every config written before this key means "no scheduled sweep",
        // and must keep meaning it.
        let (config, _) =
            resolve("scan_interval_secs = 42\n").expect("no schedule is a legal config");
        assert!(config.full_scan_schedule.is_none());
        assert_eq!(config.scan_interval, Duration::from_secs(42));
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
    fn two_names_for_one_state_file_are_one_file_however_the_path_is_spelled() {
        // #365. `require_distinct_state_paths` compared raw, unresolved `PathBuf`s, and
        // `resolve_path` is `local_root.join(value)`, which never collapses a `..`. So
        // `state/index.db` and `state/../state/index.db` were two different `PathBuf`s naming one
        // inode on any POSIX filesystem: both readers said `Ok`, and the daemon proceeded into
        // exactly the state the rule exists to prevent — the index holding a whole-file advisory
        // lock, and the single-instance check depending on the database.
        //
        // Measured before the fix through both real readers: the file reader returned `Ok(())` and
        // `resolve_runtime_config` returned `db_path = "/x/a/state/index.db"` beside
        // `lockfile_path = "/x/a/state/../state/index.db"`.
        let body = "local_root = \"/x/a\"\nremote_root = \"/Drive/a\"\n\
                    db_path = \"state/index.db\"\nlockfile_path = \"state/../state/index.db\"\n";
        for text in [body.to_owned(), format!("[[pair]]\nname = \"a\"\n{body}")] {
            let error = validate_file_config_text(&text)
                .expect_err("two spellings of one file are one file");
            let message = error.to_string();
            assert!(
                message.contains("lockfile_path"),
                "in {text:?}: got {message}"
            );
            // The message says "both resolve to X", so X is the path they resolve TO — the
            // collapsed one. Printing either written spelling would name a path the other half of
            // the sentence contradicts.
            assert!(
                message.contains("`/x/a/state/index.db`"),
                "the refusal must name the file both paths address, got {message}"
            );
            assert!(
                !message.contains(".."),
                "and must not quote a spelling it just collapsed, got {message}"
            );
        }

        // The merge path is the other reader, and the one that matters most: it sees the values the
        // daemon will really open. A flag can create this collision as easily as a file can.
        //
        // BOTH spellings orders, because the rule keys two sides and a test that only ever puts the
        // `..` on the lockfile leaves the db side of that keying unasserted — under which the two
        // readers disagree about the reverse spelling and nothing says so.
        for (db, lockfile) in [
            ("state/index.db", "state/../state/index.db"),
            ("state/../state/index.db", "state/index.db"),
        ] {
            let error = resolve_runtime_config(DaemonConfigInput {
                local_root: Some(PathBuf::from("/x/a")),
                remote_root: Some(PathBuf::from("/Drive/a")),
                db_path: Some(PathBuf::from(db)),
                lockfile_path: Some(PathBuf::from(lockfile)),
                socket_path: Some(PathBuf::from("/run/user/1000/proton-sync.sock")),
                ..Default::default()
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("lockfile_path"),
                "for db={db:?} lockfile={lockfile:?}: got {error}"
            );
        }

        // And the resolved value itself is untouched: normalization is a property of the comparison
        // key, never of the path the daemon opens. A config that is legal keeps its literal paths.
        let (config, _) = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(PathBuf::from("/x/a")),
            remote_root: Some(PathBuf::from("/Drive/a")),
            db_path: Some(PathBuf::from("state/../state/index.db")),
            lockfile_path: Some(PathBuf::from("other.lock")),
            socket_path: Some(PathBuf::from("/run/user/1000/proton-sync.sock")),
            ..Default::default()
        })
        .expect("two different files stay legal however they are spelled");
        assert_eq!(
            config.db_path,
            PathBuf::from("/x/a/state/../state/index.db"),
            "the daemon opens the value it was given, not the comparison key"
        );
    }

    #[test]
    fn a_state_path_escaping_into_another_pairs_root_is_refused_however_it_is_spelled() {
        // The cross-pair half of the same lexical gap (#365). It was latent while the count gate
        // ran after the structure rules and masked it: measured before the fix, the `..` spelling
        // fell through to "more than one folder pair" while the identical collision written
        // absolutely was named correctly. That masking ended when phase 4c lifted the cap, which is
        // why the rule had to be right before then.
        let escaping = "[[pair]]\nname = \"a\"\nlocal_root = \"/x/a\"\nremote_root = \"/Drive/a\"\n\
                        \n[[pair]]\nname = \"b\"\nlocal_root = \"/x/b\"\nremote_root = \"/Drive/b\"\n\
                        db_path = \"../a/index.db\"\n";
        let error =
            validate_file_config_text(escaping).expect_err("a state path inside another root");
        let message = error.to_string();
        assert!(message.contains("db_path"), "got {message}");
        assert!(message.contains("local_root"), "got {message}");
        assert!(
            !message.contains("not yet supported"),
            "a genuinely broken multi-pair file must say what is wrong with it rather than be \
             masked by the count gate, got {message}"
        );
        // The overlap message renders what the user WROTE (#339), which the collapsing must not
        // quietly change: they are told about the path in their file.
        assert!(message.contains("`../a/index.db`"), "got {message}");

        // The root rules themselves too: `/x/a` and `/x/b/../a` are one directory.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"/x/a\"\nremote_root = \"/Drive/a\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"/x/b/../a\"\nremote_root = \"/Drive/b\"\n",
        )
        .expect_err("two spellings of one root are one root");
        assert!(
            error.to_string().contains("the same path as"),
            "got {error}"
        );
    }

    #[test]
    fn a_comparison_key_collapses_what_it_can_and_keeps_what_it_cannot() {
        // A direct truth table over the primitive, because every rule above is one comparison of
        // two of its outputs and the arms that keep a `..` are the ones no config test reaches.
        for (path, expected) in [
            // The ordinary collapse, and repeated.
            ("/x/a/state/../state/index.db", "/x/a/state/index.db"),
            ("a/b/../../c", "c"),
            // `.` as before (#339 round 2), leading and interior.
            ("./A", "A"),
            ("/x/./a", "/x/a"),
            // A root has no parent, so `/..` is `/` rather than an escape above it.
            ("/..", "/"),
            ("/../x", "/x"),
            // A leading `..` cancels nothing, so it is part of what the path addresses and must
            // survive: dropping it would move the path up one level and make `../a` compare equal
            // to `a`, which is a false match — the direction that refuses a working config.
            ("../a", "../a"),
            ("../../a", "../../a"),
            ("a/../../b", "../b"),
            // A path that addresses the current directory keys as `.` — never as the empty path,
            // which is a prefix of every path there is, and never as its own literal, which gave
            // `./a/..` and `a/..` two keys for one directory.
            (".", "."),
            ("./", "."),
            ("a/..", "."),
            ("./a/..", "."),
            ("a/b/../..", "."),
            // `..` itself addresses the parent, not the current directory, so it is untouched.
            ("..", ".."),
        ] {
            assert_eq!(
                local_comparison_key(Path::new(path)),
                PathBuf::from(expected),
                "key for {path:?}"
            );
        }

        // `.` is a prefix of nothing but itself, which is the property that makes it a usable key:
        // every other key has had its `CurDir` components removed, so an absolute root is never
        // read as sitting "inside" a pair whose root is the working directory.
        assert!(!Path::new("/x/a").starts_with("."));
        assert!(!Path::new("a/b").starts_with("."));
        assert!(Path::new(".").starts_with("."));

        // Two spellings of one directory are one root. This is the case the empty-key fallback was
        // masking: both reduce to nothing, and returning each literal gave them two keys — so the
        // collision fell through to the generic pair-count refusal, which is the exact masking this
        // change exists to remove.
        let error = validate_file_config_text(
            "[[pair]]\nname = \"a\"\nlocal_root = \"./a/..\"\nremote_root = \"/Drive/a\"\n\
             db_path = \"/state/a/i.db\"\nlockfile_path = \"/state/a/d.lock\"\n\
             \n[[pair]]\nname = \"b\"\nlocal_root = \"a/..\"\nremote_root = \"/Drive/b\"\n\
             db_path = \"/state/b/i.db\"\nlockfile_path = \"/state/b/d.lock\"\n",
        )
        .expect_err("two spellings of the working directory are one root");
        assert!(
            !error.to_string().contains("not yet supported"),
            "got {error}"
        );

        // A remote root is a different question and is deliberately untouched — not because the
        // arithmetic differs (lexically `/Drive/a/../b` IS `/Drive/b`) but because
        // `proton::clean_remote_root_path` answers `None` for any remote path carrying a `..`, so
        // no such root reaches the daemon and this layer must not be the one deciding it means
        // something.
        assert_eq!(
            remote_root_comparison_key(Path::new("/Drive/a/../b")),
            PathBuf::from("Drive/a/../b")
        );
    }

    #[test]
    fn a_dotdot_through_a_symlink_is_refused_lexically_and_the_message_says_what_it_compared() {
        // The residual, pinned in the direction the doc comment used to deny. Lexical collapsing is
        // only sound when the component a `..` cancels is a real directory: `current` a symlink
        // means `current/..` is the parent of the symlink's TARGET, so the two paths below are two
        // different files that this refuses as one.
        //
        // The trade is deliberate — the missed refusal it closes is silent and ends with the daemon
        // holding a whole-file advisory lock on its own database, while this one is loud, arrives
        // before anything runs, and is undone by spelling the path without the `..`. The test
        // exists so the choice is recorded rather than rediscovered.
        let root = tempfile::tempdir().expect("temp root");
        let elsewhere = tempfile::tempdir().expect("temp elsewhere");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("current"))
            .expect("symlink current -> elsewhere");

        // Ground truth: the two paths are different files.
        let through_link = root.path().join("current/../index.db");
        let literal = root.path().join("index.db");
        std::fs::write(&through_link, b"one").expect("write through the link");
        std::fs::write(&literal, b"two").expect("write the literal");
        assert_ne!(
            std::fs::read(&through_link).expect("read one"),
            std::fs::read(&literal).expect("read two"),
            "the two paths must really be two files, or this test proves nothing"
        );

        let error = resolve_runtime_config(DaemonConfigInput {
            local_root: Some(root.path().to_path_buf()),
            remote_root: Some(PathBuf::from("/Drive/a")),
            db_path: Some(PathBuf::from("current/../index.db")),
            lockfile_path: Some(PathBuf::from("index.db")),
            socket_path: Some(PathBuf::from("/run/user/1000/proton-sync.sock")),
            ..Default::default()
        })
        .expect_err("lexical collapsing cannot see the symlink");
        // And the message names the path it actually compared, so the refusal can be understood
        // rather than merely obeyed: it says "both resolve to X" and X is the lexical answer.
        assert!(
            error
                .to_string()
                .contains(&format!("{}", literal.display())),
            "got {error}"
        );
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
