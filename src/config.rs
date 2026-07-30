use crate::daemon::DaemonConfig;
use crate::index::ScanOptions;
use crate::paths::{
    default_global_lock_path, default_lockfile_path, default_socket_path, default_state_db_path,
};
use crate::proton::CommandPolicy;
use crate::{AppResult, boxed_error};
use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Default number of incremental event-driven passes between forced full-tree resyncs when
/// `events_driven` is on and no explicit value is configured. `0` disables the periodic safety
/// resync entirely, so after the first (startup) full-tree snapshot the daemon stays purely
/// event-driven until it is restarted or the event stream forces a fallback. The periodic resync
/// is opt-in: set a positive value to reinstate a self-healing full walk every N passes.
const DEFAULT_EVENTS_FULL_SCAN_EVERY: u64 = 0;

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
    pub dry_run: bool,
    pub no_dry_run: bool,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub events_driven: bool,
    pub no_events_driven: bool,
    pub events_full_scan_every: Option<u64>,
    /// Coarse opt-out for the delete-approval guard: when set, disables approval for **both**
    /// directions globally (equivalent to `[delete_approval] remote = false, local = false`).
    /// Per-direction and per-subtree granularity lives in the per-directory `.proton-sync.toml`
    /// files (see `crate::dirconfig`); the CLI keeps only this blunt escape hatch.
    pub no_delete_approval: bool,
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
    /// Daemon-wide default for the directional delete-approval guard (the bottom of the
    /// per-directory inheritance chain). Each direction defaults to `true` (protected) when unset.
    #[serde(default, alias = "delete-approval")]
    delete_approval: Option<FileDeleteApproval>,
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
    } else {
        let file = file_config.delete_approval.unwrap_or_default();
        (file.remote.unwrap_or(true), file.local.unwrap_or(true))
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
    let default_command_policy = CommandPolicy::default();

    let config = DaemonConfig {
        local_root,
        remote_root,
        db_path,
        socket_path: input
            .socket_path
            .or(file_config.socket_path)
            .map(|path| expand_tilde(path, "socket_path"))
            .transpose()?
            .unwrap_or_else(default_socket_path),
        lockfile_path,
        // Not user-overridable: the single-instance guarantee must key on a fixed per-user path so
        // it holds regardless of --socket-path / --local-root (see `default_global_lock_path`).
        global_lock_path: default_global_lock_path(),
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
    };
    validate_runtime_config(&config)?;

    Ok((config, dry_run))
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
fn expand_tilde(path: PathBuf, field_name: &str) -> AppResult<PathBuf> {
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

fn validate_runtime_config(config: &DaemonConfig) -> AppResult<()> {
    if config.local_root.as_os_str().is_empty() {
        return Err(boxed_error("local_root must not be empty"));
    }
    if config.remote_root.as_os_str().is_empty() {
        return Err(boxed_error("remote_root must not be empty"));
    }
    // Unlike db_path/lockfile_path, a relative socket_path is *not* resolved under local_root —
    // the control socket must not live under the sync root (see `paths::default_socket_path`).
    // Used verbatim, a relative value would bind against the daemon's current working directory,
    // so reject it outright. The XDG default is always absolute; only explicit overrides hit this.
    if !config.socket_path.is_absolute() {
        return Err(boxed_error(format!(
            "socket_path must be an absolute path, got relative `{}`: a relative socket path \
             would resolve against the daemon's working directory; pass an absolute path (for \
             example under $XDG_RUNTIME_DIR)",
            config.socket_path.display()
        )));
    }
    ScanOptions::new(
        &config.local_root,
        std::slice::from_ref(&config.db_path),
        &config.include_patterns,
        &config.exclude_patterns,
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
}
