use clap::Parser;
use proton_drive_sync_engine::config::{
    DEFAULT_LOG_LEVEL, DaemonConfigInput, DeletionPolicy, resolve_runtime_config,
};
use proton_drive_sync_engine::daemon::{Daemon, preview_plan};
use proton_drive_sync_engine::sync::DryRunReport;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "proton-syncd",
    about = "Bidirectional Proton Drive background sync daemon"
)]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    local_root: Option<PathBuf>,
    #[arg(long)]
    remote_root: Option<PathBuf>,
    #[arg(long)]
    db_path: Option<PathBuf>,
    #[arg(long)]
    socket_path: Option<PathBuf>,
    #[arg(long)]
    lockfile_path: Option<PathBuf>,
    #[arg(long)]
    scan_interval_secs: Option<u64>,
    #[arg(long)]
    proton_cli: Option<PathBuf>,
    #[arg(long)]
    proton_timeout_secs: Option<u64>,
    #[arg(long)]
    proton_list_attempts: Option<usize>,
    /// Maximum planned downloads bundled into one proton-drive invocation (chunked by
    /// destination directory; each chunk is checkpoint-committed on landing). 1 disables
    /// batching and downloads one file per invocation.
    #[arg(long = "download-batch-size", value_name = "N")]
    download_batch_size: Option<usize>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long = "no-dry-run", conflicts_with = "dry_run")]
    no_dry_run: bool,
    #[arg(long = "include", value_name = "GLOB")]
    include_patterns: Vec<String>,
    #[arg(long = "exclude", value_name = "GLOB")]
    exclude_patterns: Vec<String>,
    /// Detect remote changes from Proton's volume event stream instead of a full-tree walk.
    /// This is the default; the flag is kept for explicitness and to override a config-file
    /// `events_driven = false`.
    #[arg(long = "events-driven")]
    events_driven: bool,
    /// Opt out of event-driven detection and use full-tree-walk-only remote change detection.
    #[arg(long = "no-events-driven", conflicts_with = "events_driven")]
    no_events_driven: bool,
    /// Force a full-tree resync every N incremental passes (event-driven mode only). Default 0
    /// disables the periodic resync entirely: after the first (startup) full snapshot the daemon
    /// stays purely event-driven. Set a positive N to reinstate a self-healing safety resync.
    #[arg(long = "events-full-scan-every", value_name = "N")]
    events_full_scan_every: Option<u64>,
    /// Warm-start on boot: the first pass after startup replays the remote from the saved event
    /// cursor (O(changes)) instead of a full-tree walk, while still scanning the local tree. This
    /// is the default; the flag is kept for explicitness and to override a config `warm_start =
    /// false`.
    #[arg(long = "warm-start")]
    warm_start: bool,
    /// Opt out of the warm start: always do a full-tree walk on the first pass after boot.
    #[arg(long = "no-warm-start", conflicts_with = "warm_start")]
    no_warm_start: bool,
    /// Do a full-tree walk instead of a warm start every N warm starts (self-heal across reboots).
    /// Default 30. 0 disables the periodic full walk (warm-start every boot).
    #[arg(long = "warm-start-full-walk-every", value_name = "N")]
    warm_start_full_walk_every: Option<u64>,
    /// Warm-start only if the saved event cursor is at most N seconds old; otherwise full-walk.
    /// Default 604800 (7 days). 0 disables the age check.
    #[arg(long = "warm-start-max-cursor-age-secs", value_name = "SECS")]
    warm_start_max_cursor_age_secs: Option<u64>,
    /// Do a full-tree walk on this boot's first pass instead of a warm start (one-shot; e.g. to
    /// self-heal suspected drift). While the daemon is running, `proton-sync resync` does the same.
    #[arg(long = "full-walk")]
    full_walk: bool,
    /// Disable the delete-approval guard globally (both directions). By default deletions are
    /// withheld pending approval; set this to let every delete apply automatically. For finer
    /// control, use `[delete_approval]` in the config file or per-directory `.proton-sync.toml`.
    #[arg(long = "no-delete-approval")]
    no_delete_approval: bool,
    /// The delete-approval guard as one named setting: ask_every_time (default), only_permanent
    /// (let the recoverable Drive-side deletions through), never, or only_recoverable. Overrides
    /// the config file's `deletion_policy` / `[delete_approval]`; `--no-delete-approval` overrides
    /// this.
    #[arg(long = "deletion-policy", value_name = "POLICY")]
    deletion_policy: Option<DeletionPolicy>,
    /// Log verbosity as a `tracing` filter directive (`info`, `debug`, `crate::module=warn`, …).
    /// Overrides `RUST_LOG`, which overrides the config file's `log_level`.
    #[arg(long = "log-level", value_name = "DIRECTIVE")]
    log_level: Option<String>,
    /// Suffix used to name conflict sidecars: `{stem}.{suffix}.{ext}`. Default `proton-cloud`.
    /// Changing it leaves sidecars written under the old suffix behind as ordinary files.
    #[arg(long = "conflict-suffix", value_name = "SUFFIX")]
    conflict_suffix: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    // Tracing is initialized AFTER the config resolves, because the config is now what decides the
    // verbosity (`log_level`). Clap's own parse errors print themselves and exit before this, and a
    // resolution failure falls back to the default filter below so its `error!` is never silent.
    let cli = Cli::parse();
    let resolved = resolve_runtime_config(cli.into());
    init_tracing(
        resolved
            .as_ref()
            .map(|(config, _)| config.log_filter.as_str())
            .unwrap_or(DEFAULT_LOG_LEVEL),
    );
    let (config, dry_run) = match resolved {
        Ok(config) => config,
        Err(error) => {
            error!(%error, "failed to resolve daemon configuration");
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    if dry_run {
        info!("running dry-run sync plan");
        return match preview_plan(&config) {
            Ok(plan) => match serde_json::to_string_pretty(&DryRunReport::new(plan)) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    error!(%error, "failed to serialize dry-run report");
                    eprintln!("failed to serialize dry-run report: {error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                error!(%error, "dry-run sync plan failed");
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    match Daemon::new(config) {
        Ok(daemon) => match daemon.run().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error!(%error, "daemon exited with error");
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            error!(%error, "failed to initialize daemon");
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

impl From<Cli> for DaemonConfigInput {
    fn from(cli: Cli) -> Self {
        Self {
            config: cli.config,
            local_root: cli.local_root,
            remote_root: cli.remote_root,
            db_path: cli.db_path,
            socket_path: cli.socket_path,
            lockfile_path: cli.lockfile_path,
            scan_interval_secs: cli.scan_interval_secs,
            proton_cli: cli.proton_cli,
            proton_timeout_secs: cli.proton_timeout_secs,
            proton_list_attempts: cli.proton_list_attempts,
            download_batch_size: cli.download_batch_size,
            dry_run: cli.dry_run,
            no_dry_run: cli.no_dry_run,
            include_patterns: cli.include_patterns,
            exclude_patterns: cli.exclude_patterns,
            events_driven: cli.events_driven,
            no_events_driven: cli.no_events_driven,
            events_full_scan_every: cli.events_full_scan_every,
            warm_start: cli.warm_start,
            no_warm_start: cli.no_warm_start,
            warm_start_full_walk_every: cli.warm_start_full_walk_every,
            warm_start_max_cursor_age_secs: cli.warm_start_max_cursor_age_secs,
            force_full_walk: cli.full_walk,
            no_delete_approval: cli.no_delete_approval,
            deletion_policy: cli.deletion_policy,
            log_level: cli.log_level,
            // Read here, at the process edge, so `resolve_runtime_config` stays a pure function of
            // its input and parallel tests cannot race on the environment.
            rust_log: std::env::var("RUST_LOG").ok(),
            conflict_suffix: cli.conflict_suffix,
        }
    }
}

/// `directive` is already resolved (`--log-level` > `RUST_LOG` > `log_level` > `info`) and already
/// validated by `resolve_log_filter`, so the fallback here is only reachable for the
/// resolution-failed path above.
fn init_tracing(directive: &str) {
    let filter =
        EnvFilter::try_new(directive).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_LEVEL));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_advanced_flags_parse_into_config_input() {
        // `--deletion-policy` reaches clap through `DeletionPolicy`'s `FromStr`, which the compiler
        // proves exists and nothing else exercises — including its error, which is the only place a
        // user learns what the four spellings are.
        let cli = Cli::try_parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--deletion-policy",
            "only_permanent",
            "--log-level",
            "debug",
            "--conflict-suffix",
            "from-cloud",
        ])
        .expect("the advanced flags must parse");
        let input: DaemonConfigInput = cli.into();
        assert_eq!(input.deletion_policy, Some(DeletionPolicy::OnlyPermanent));
        assert_eq!(input.log_level.as_deref(), Some("debug"));
        assert_eq!(input.conflict_suffix.as_deref(), Some("from-cloud"));

        let error = Cli::try_parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--deletion-policy",
            "ask_sometimes",
        ])
        .expect_err("an unknown policy must be rejected, not silently defaulted")
        .to_string();
        assert!(
            error.contains("ask_every_time"),
            "the error must name the spellings that do work: {error}"
        );
    }

    #[test]
    fn conflicting_dry_run_flags_are_rejected_by_cli_parser() {
        let result = Cli::try_parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--dry-run",
            "--no-dry-run",
        ]);

        assert!(result.is_err(), "conflicting dry-run flags must fail");
    }

    #[test]
    fn conflicting_events_driven_flags_are_rejected_by_cli_parser() {
        let result = Cli::try_parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--events-driven",
            "--no-events-driven",
        ]);

        assert!(
            result.is_err(),
            "passing both --events-driven and --no-events-driven must fail"
        );
    }

    #[test]
    fn conflicting_warm_start_flags_are_rejected_by_cli_parser() {
        let result = Cli::try_parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--warm-start",
            "--no-warm-start",
        ]);

        assert!(
            result.is_err(),
            "passing both --warm-start and --no-warm-start must fail"
        );
    }

    #[test]
    fn warm_start_flags_parse_into_config_input() {
        let cli = Cli::parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--full-walk",
            "--warm-start-full-walk-every",
            "15",
            "--warm-start-max-cursor-age-secs",
            "600",
        ]);

        let input = DaemonConfigInput::from(cli);

        assert!(input.force_full_walk);
        assert_eq!(input.warm_start_full_walk_every, Some(15));
        assert_eq!(input.warm_start_max_cursor_age_secs, Some(600));
    }

    #[test]
    fn proton_policy_flags_parse_into_config_input() {
        let cli = Cli::parse_from([
            "proton-syncd",
            "--local-root",
            "sync-root",
            "--remote-root",
            "/Drive/Config",
            "--proton-timeout-secs",
            "12",
            "--proton-list-attempts",
            "5",
        ]);

        let input = DaemonConfigInput::from(cli);

        assert_eq!(input.proton_timeout_secs, Some(12));
        assert_eq!(input.proton_list_attempts, Some(5));
    }
}
