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
    /// `--log-level`: a `tracing` filter directive (`info`, `debug`, `crate::module=warn`, …).
    pub log_level: Option<String>,
    /// The process's `RUST_LOG`, passed in rather than read here so resolution stays pure and
    /// parallel tests cannot race on the environment (same reason as `expand_tilde_with_home`).
    pub rust_log: Option<String>,
    /// `--conflict-suffix`: how conflict sidecars are named. See [`ConflictNaming`].
    pub conflict_suffix: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
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
    /// Daemon log verbosity as a `tracing` filter directive. Outranked by the process's
    /// `RUST_LOG`, which outranks nothing else — see [`resolve_log_filter`].
    #[serde(default, alias = "log-level")]
    log_level: Option<String>,
    /// Conflict-sidecar suffix (`{stem}.{suffix}.{ext}`); default `proton-cloud`. Changing it
    /// orphans sidecars already on disk — see [`ConflictNaming`].
    #[serde(default, alias = "conflict-suffix")]
    conflict_suffix: Option<String>,
}

/// The `[delete_approval]` table in the daemon config file. Names the *target* of the deletion
/// being gated; unset directions default to protected.
///
/// `deny_unknown_fields` must be repeated here: serde's deny on [`FileConfig`] does not recurse
/// into nested tables, so without it a typo like `remot = false` would be silently ignored and
/// the guard would stay on despite the user's intent (#64).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDeleteApproval {
    remote: Option<bool>,
    local: Option<bool>,
}

pub fn resolve_runtime_config(input: DaemonConfigInput) -> AppResult<(DaemonConfig, bool)> {
    // The config-file path is itself a local-filesystem path, so it gets the same `~` treatment
    // as the values inside it (see `expand_tilde` below).
    let config_path = input
        .config
        .map(|path| expand_tilde(path, "--config"))
        .transpose()?;
    let file_config = load_file_config(config_path.as_ref())?;
    let dry_run = if input.no_dry_run {
        false
    } else if input.dry_run {
        true
    } else {
        file_config.dry_run.unwrap_or(false)
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
        file_config.events_driven.unwrap_or(true)
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
        resolve_file_delete_approval(
            file_config.deletion_policy,
            file_config.delete_approval.as_ref(),
        )?
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
        file_config.warm_start.unwrap_or(true)
    };
    let warm_start = WarmStartConfig {
        enabled: warm_start_enabled,
        full_walk_every: input
            .warm_start_full_walk_every
            .or(file_config.warm_start_full_walk_every)
            .unwrap_or(DEFAULT_WARM_START_FULL_WALK_EVERY),
        max_cursor_age: Duration::from_secs(
            input
                .warm_start_max_cursor_age_secs
                .or(file_config.warm_start_max_cursor_age_secs)
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
            .or(file_config.local_root)
            .ok_or_else(|| boxed_error("missing required --local-root or config local_root"))?,
        "local_root",
    )?;
    let remote_root = input
        .remote_root
        .or(file_config.remote_root)
        .ok_or_else(|| boxed_error("missing required --remote-root or config remote_root"))?;
    let db_path = input
        .db_path
        .or(file_config.db_path)
        .map(|path| expand_tilde(path, "db_path"))
        .transpose()?
        .map(|path| resolve_path(&local_root, path))
        .unwrap_or_else(|| default_state_db_path(&local_root));
    // Resolved before the struct literal below (which moves `local_root`), mirroring `db_path`:
    // a relative override joins under `local_root` (so it lands where `scan_options_from_config`
    // ignores it), an absolute one is used as-is, and the default is the per-root `.sync` path.
    let lockfile_path = input
        .lockfile_path
        .or(file_config.lockfile_path)
        .map(|path| expand_tilde(path, "lockfile_path"))
        .transpose()?
        .map(|path| resolve_path(&local_root, path))
        .unwrap_or_else(|| default_lockfile_path(&local_root));
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
                .or(file_config.scan_interval_secs)
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
            file_config.download_batch_size,
            DEFAULT_DOWNLOAD_BATCH_SIZE,
            "download_batch_size",
        )?,
        include_patterns: merge_patterns(input.include_patterns, file_config.include_patterns),
        exclude_patterns: merge_patterns(input.exclude_patterns, file_config.exclude_patterns),
        events_driven,
        // `0` is a valid, meaningful value here (periodic safety resync disabled), so it is *not*
        // clamped up to 1 the way a zero scan interval would be. The daemon treats 0 as "never
        // auto-resync" (see `effective_full_scan_every` in `daemon.rs`).
        events_full_scan_every: input
            .events_full_scan_every
            .or(file_config.events_full_scan_every)
            .unwrap_or(DEFAULT_EVENTS_FULL_SCAN_EVERY),
        delete_approval_remote,
        delete_approval_local,
        warm_start,
        log_filter: resolve_log_filter(
            input.log_level.as_deref(),
            input.rust_log.as_deref(),
            file_config.log_level.as_deref(),
        )?,
        conflict_naming: match input.conflict_suffix.or(file_config.conflict_suffix) {
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
pub fn validate_file_config_text(text: &str) -> AppResult<()> {
    let config = parse_file_config(text)
        .map_err(|error| boxed_error(format!("failed to parse config: {error}")))?;
    resolve_file_delete_approval(config.deletion_policy, config.delete_approval.as_ref())?;
    resolve_log_filter(None, None, config.log_level.as_deref())?;
    if let Some(suffix) = &config.conflict_suffix {
        validate_conflict_suffix(suffix)?;
    }
    if let Some(socket_path) = config.socket_path {
        // Expand FIRST: `socket_path = "~/run/x.sock"` is a path the daemon accepts, and checking
        // the literal would reject it as relative.
        require_absolute_socket_path(&expand_tilde(socket_path, "socket_path")?)?;
    }
    resolve_positive_duration_secs(None, config.proton_timeout_secs, 1, "proton_timeout_secs")?;
    resolve_positive_usize(None, config.proton_list_attempts, 1, "proton_list_attempts")?;
    resolve_positive_usize(None, config.download_batch_size, 1, "download_batch_size")?;
    // Compiled against a throwaway root: the root only decides which paths are ignored, and this
    // call passes none. Same check `validate_runtime_config` makes.
    ScanOptions::new(
        Path::new("/"),
        &[],
        &config.include_patterns.unwrap_or_default(),
        &config.exclude_patterns.unwrap_or_default(),
        &ConflictNaming::default(),
    )
    .map_err(|error| boxed_error(format!("invalid scan filter configuration: {error}")))?;
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
    if config.local_root.as_os_str().is_empty() {
        return Err(boxed_error("local_root must not be empty"));
    }
    if config.remote_root.as_os_str().is_empty() {
        return Err(boxed_error("remote_root must not be empty"));
    }
    require_absolute_socket_path(&config.socket_path)?;
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
}
