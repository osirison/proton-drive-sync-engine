//! Adding and removing folder pairs from the app (#102 phase 5b-2): everything the two commands do that
//! is not the `#[tauri::command]` wrapper itself.
//!
//! **Add** writes one `[[pair]]` table and says a restart is needed. **Remove** is the longer
//! sequence, because of maintainer decision D8: removing a pair sets its sync history aside, outside
//! every sync root, so that adding the same folder back starts fresh (a bootstrap: it matches and
//! downloads and never deletes) instead of resuming an index that would read everything that changed
//! in between as deletions.
//!
//! The history cannot be moved while a daemon has it open, so the order is the design:
//!
//! 1. the pair is removed from the config file (atomically, the file untouched on any refusal);
//! 2. the daemon is restarted off it, through the app's existing restart path;
//! 3. the daemon is asked which pairs it runs **now**, and the pair must not be among them;
//! 4. only then is the history moved — and the pair's own lockfile is taken first, which is the proof
//!    that does not depend on a socket answering (`gui_core::set_aside::acquire`).
//!
//! A step that cannot be taken is not an error and not a silent skip: the move is recorded as
//! **pending**, the reply says so and why, and it is retried the next time a folder is added or
//! removed. Every step is a function of its inputs — the restart and the question to the daemon are
//! passed in — so the sequence is tested without a daemon, a socket or `systemctl`.

use super::RestartOutcome;
use gui_core::config_io::{self, ConfigDoc, PairInit, PairView};
use gui_core::set_aside::{self, Context, Done, Plan, Settled};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn text(error: impl ToString) -> String {
    error.to_string()
}

fn lossy(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// ---- reports -------------------------------------------------------------------------------------

/// What happened to a removed pair's history, as the reply tells it. Tagged, so a screen reads the
/// outcome and not a sentence; `message` is the sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SetAsideReport {
    /// It was moved. `to` is the directory it is in now.
    Moved {
        pair: String,
        to: String,
        items: Vec<MovedReport>,
        message: String,
    },
    /// There was no history on disk for this pair.
    NothingToMove { pair: String, message: String },
    /// It could not be moved now, and has been recorded so that it still can be. `record` is where.
    Pending {
        pair: String,
        reason: String,
        record: Option<String>,
        message: String,
    },
    /// An earlier pending move whose folder has been added back since: nothing was moved, and the
    /// record was dropped, because that history is the re-added pair's own.
    Superseded { pair: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MovedReport {
    pub from: String,
    pub to: String,
    /// `rename` or `copy`.
    pub how: String,
}

fn moved_report(pair: &str, done: &Done) -> SetAsideReport {
    if done.moved.is_empty() {
        return SetAsideReport::NothingToMove {
            pair: pair.to_owned(),
            message: format!("'{pair}' had no sync history on disk to move."),
        };
    }
    SetAsideReport::Moved {
        pair: pair.to_owned(),
        to: lossy(&done.to),
        items: done
            .moved
            .iter()
            .map(|item| MovedReport {
                from: lossy(&item.from),
                to: lossy(&item.to),
                how: match item.how {
                    set_aside::How::Rename => "rename",
                    set_aside::How::Copy => "copy",
                }
                .to_owned(),
            })
            .collect(),
        message: format!(
            "The sync history of '{pair}' was moved to {}, outside every sync folder. Your files \
             were not touched. Adding the folder again starts fresh: it matches and downloads, and \
             deletes nothing.",
            lossy(&done.to)
        ),
    }
}

fn pending_report(
    pair: &str,
    reason: &str,
    record: Option<&Path>,
    items: &[PathBuf],
) -> SetAsideReport {
    let where_it_is = items
        .iter()
        .map(|item| lossy(item))
        .collect::<Vec<_>>()
        .join(", ");
    let message = match record {
        Some(record) => format!(
            "The sync history of '{pair}' was NOT moved yet: {reason}. It is still at {where_it_is}, \
             and a note of it is kept at {}; it moves the next time a folder is added or removed \
             here. Until then, adding the same folder again would resume that history and read what \
             changed since as deletions to approve.",
            lossy(record)
        ),
        None => format!(
            "The sync history of '{pair}' was NOT moved: {reason}. It is still at {where_it_is}, \
             and the app could not keep a note of it. Move it out of the folder yourself before \
             adding the same folder again, or that history is resumed and what changed since is read \
             as deletions to approve."
        ),
    };
    SetAsideReport::Pending {
        pair: pair.to_owned(),
        reason: reason.to_owned(),
        record: record.map(lossy),
        message,
    }
}

fn settled_report(settled: Settled) -> SetAsideReport {
    match settled {
        Settled::Moved { pair, done } => moved_report(&pair, &done),
        Settled::Superseded { pair } => SetAsideReport::Superseded {
            message: format!(
                "An earlier removal of '{pair}' had not finished moving its history, and the folder \
                 has been added back since: that history is the re-added pair's own, so it was left \
                 where it is."
            ),
            pair,
        },
        Settled::StillPending { pair, reason } => SetAsideReport::Pending {
            message: format!(
                "The earlier removal of '{pair}' still could not move its history: {reason}."
            ),
            pair,
            reason,
            record: None,
        },
    }
}

/// Retry what an earlier removal could not finish, against the pairs configured now.
fn settle_earlier(
    state_dir: Option<&Path>,
    configured: &[PairView],
    now: SystemTime,
) -> Vec<SetAsideReport> {
    let Some(state_dir) = state_dir else {
        return Vec::new();
    };
    set_aside::settle_pending(&Context {
        state_dir,
        configured,
        now,
    })
    .into_iter()
    .map(settled_report)
    .collect()
}

// ---- add -----------------------------------------------------------------------------------------

/// The add dialog's answers (the pair's name travels beside them as the command's `pair`).
#[derive(serde::Deserialize)]
pub struct AddPairRequest {
    pub local_root: String,
    pub remote_root: String,
    /// The skip rules the dialog staged, if any.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// An index from an earlier run that the new pair would resume.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SurvivingIndex {
    pub path: String,
    /// The removal that should have moved it has not finished (a record of it is kept).
    pub set_aside_pending: bool,
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
pub struct AddPairReply {
    pub pair: String,
    /// The config file that was written.
    pub path: String,
    /// The running daemon, if any, has not picked the new pair up. The caller restarts it
    /// (`restart_service`, `only_if_running`) and polls until `pairs[]` lists the name.
    pub restart_needed: bool,
    /// Present when the chosen folder still holds an index the new pair will resume.
    pub surviving_index: Option<SurvivingIndex>,
    /// Earlier removals whose history moved (or was dropped) as a side effect of this call.
    pub settled_earlier: Vec<SetAsideReport>,
}

/// The two refusals about the local root that the engine does not make (#431): a relative path, which
/// the daemon would resolve against its own working directory and then watch the wrong folder, and a
/// folder that is not there. The app never creates the folder.
fn check_local_root(local_root: &str) -> Result<(), String> {
    let expanded = config_io::expand_config_path(local_root, "local_root");
    if !expanded.is_absolute() {
        return Err(format!(
            "the folder `{local_root}` is not a full path. The sync daemon would resolve it against \
             its own working directory and watch the wrong place: choose the folder by its full \
             path, or start it with `~`"
        ));
    }
    match std::fs::metadata(&expanded) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(format!("`{}` is not a folder", expanded.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "the folder `{}` does not exist. The app does not create it: create it first, or \
             choose another",
            expanded.display()
        )),
        Err(error) => Err(format!(
            "the folder `{}` cannot be read: {error}",
            expanded.display()
        )),
    }
}

/// The blocking half of `add_pair`: load, settle what an earlier removal left, add, check, save.
///
/// **The order of the refusals is part of the contract**, and a test holds it: the engine's own
/// validation (the name, the roots, a lexical overlap, `dry_run = true` beside a second pair) speaks
/// first, in its words; then the two local-root checks the engine does not make; then the real-path
/// overlap, which follows symlinks and so touches the disk. A file that is simply invalid is told so
/// before anything is asked of the filesystem. Any refusal leaves the file byte-identical.
pub(super) fn add_pair_file(
    path: &Path,
    state_dir: Option<&Path>,
    name: &str,
    request: &AddPairRequest,
    now: SystemTime,
) -> Result<AddPairReply, String> {
    let mut doc = ConfigDoc::load(path).map_err(text)?;
    // Only against a config that can be read: what is configured NOW is what keeps an earlier removal
    // from moving a folder's history out from under a pair that has since been added back, and a file
    // that cannot be read says nothing about that.
    let settled_earlier = match config_io::pair_views(&doc.to_toml_string()) {
        Ok(configured) => settle_earlier(state_dir, &configured, now),
        Err(_) => Vec::new(),
    };

    let (local_root, remote_root) = (request.local_root.trim(), request.remote_root.trim());
    doc.add_pair(PairInit {
        name: name.to_owned(),
        local_root: local_root.to_owned(),
        remote_root: remote_root.to_owned(),
        exclude: request.exclude.clone(),
    })
    .map_err(text)?;
    check_local_root(local_root)?;
    config_io::real_path_conflicts(&doc.to_toml_string()).map_err(text)?;
    doc.save(path).map_err(text)?;

    let saved = doc.to_toml_string();
    let views = config_io::pair_views(&saved).map_err(text)?;
    let surviving_index = views
        .iter()
        .find(|view| view.name == name)
        .and_then(|view| surviving_index_of(view, state_dir));
    Ok(AddPairReply {
        pair: name.to_owned(),
        path: lossy(path),
        restart_needed: true,
        surviving_index,
        settled_earlier,
    })
}

fn surviving_index_of(view: &PairView, state_dir: Option<&Path>) -> Option<SurvivingIndex> {
    let index = set_aside::surviving_index(view)?;
    let root = view.local_root.as_deref()?;
    let pending = state_dir.is_some_and(|dir| {
        set_aside::pending(dir).iter().any(|(_, record)| {
            set_aside::real_path(&record.plan.local_root) == set_aside::real_path(root)
        })
    });
    let mut message = format!(
        "{} is a sync index from an earlier run in this folder. The new pair resumes it, and anything \
         that changed on either side since is read as a deletion to approve. This app cannot reset an \
         index; `proton-sync reset-index --yes` can.",
        lossy(&index)
    );
    if pending {
        message.push_str(
            " It is there because the removal that should have moved it has not finished; that move \
             will be dropped rather than run, now that the folder is configured again.",
        );
    }
    Some(SurvivingIndex {
        path: lossy(&index),
        set_aside_pending: pending,
        message,
    })
}

// ---- remove --------------------------------------------------------------------------------------

/// What removing a pair from the file did, and what the sequence after it needs to know.
#[derive(Debug)]
pub(super) struct RemovedPair {
    pub name: String,
    /// The pair as the file described it BEFORE the removal: its folder and its state files.
    pub view: PairView,
    /// The pairs the file declares now.
    pub remaining: Vec<PairView>,
    /// Set when the removed pair was the first: the pair that is the default one now (the pair a
    /// command that names none reaches), which the confirmation said it would be.
    pub new_default: Option<String>,
}

/// The blocking first step of `remove_pair`: take the table out of the file and save it.
pub(super) fn remove_pair_file(path: &Path, name: &str) -> Result<RemovedPair, String> {
    let mut doc = ConfigDoc::load(path).map_err(text)?;
    let before = config_io::pair_views(&doc.to_toml_string()).map_err(text)?;
    doc.remove_pair(name).map_err(text)?;
    let index = before
        .iter()
        .position(|view| view.name == name)
        .ok_or_else(|| format!("the config has no folder pair named {name:?}"))?;
    doc.save(path).map_err(text)?;
    let remaining = config_io::pair_views(&doc.to_toml_string()).map_err(text)?;
    let new_default = (index == 0)
        .then(|| remaining.first().map(|view| view.name.clone()))
        .flatten();
    Ok(RemovedPair {
        name: name.to_owned(),
        view: before[index].clone(),
        remaining,
        new_default,
    })
}

/// How long to wait for the restarted daemon to say which pairs it runs, and how often to ask.
pub(super) const DAEMON_ANSWER_WAIT: Duration = Duration::from_secs(10);
pub(super) const DAEMON_ASK_EVERY: Duration = Duration::from_millis(500);

#[derive(Debug, serde::Serialize)]
pub struct RemovePairReply {
    pub pair: String,
    pub path: String,
    /// Set when the removed pair was the first: the pair a command naming none reaches now.
    pub new_default: Option<String>,
    /// What the restart that releases the pair's history did (the app's existing restart path).
    pub restart: RestartOutcome,
    /// The running daemon, if any, still has the old pair list: the restart did not finish.
    pub restart_needed: bool,
    pub set_aside: SetAsideReport,
    pub settled_earlier: Vec<SetAsideReport>,
}

/// Whether a daemon may still have the removed pair's history open, from what the restart did and
/// what the daemon says it runs. **`Held` is the default**: every answer that is not positive
/// evidence that the pair is gone leaves the history where it is.
enum Release {
    Free,
    Held(String),
}

/// Steps 2 and 3: restart the daemon and check the pair is gone from its list.
///
/// `ask_pairs` is the daemon's `pairs[]` right now (`None` when it did not answer, or answered as a
/// daemon that predates the list, which cannot confirm anything). `sleep` is injected so the wait is
/// not.
fn release_of(
    pair: &str,
    restart: &Result<RestartOutcome, String>,
    mut ask_pairs: impl FnMut() -> Option<Vec<String>>,
    mut sleep: impl FnMut(Duration),
) -> Release {
    let outcome = match restart {
        Ok(outcome) => outcome,
        Err(reason) => {
            return Release::Held(format!("the restart could not be attempted: {reason}"))
        }
    };
    match outcome {
        // Nothing was running, or the stop was confirmed and only the start failed: there is no
        // daemon to hold the pair, and the lockfile check that follows is the proof.
        RestartOutcome::NotRunning | RestartOutcome::NotStarted { .. } => Release::Free,
        RestartOutcome::Restarted { .. } => {
            let mut waited = Duration::ZERO;
            loop {
                match ask_pairs() {
                    Some(names) if names.iter().any(|name| name == pair) => {
                        return Release::Held(
                            "the restarted daemon still lists the pair, so it was not started on \
                             the edited config"
                                .to_owned(),
                        );
                    }
                    Some(_) => return Release::Free,
                    None if waited >= DAEMON_ANSWER_WAIT => {
                        return Release::Held(
                            "the restarted daemon did not say which pairs it runs, so the app \
                             could not confirm it let go of this one"
                                .to_owned(),
                        );
                    }
                    None => {
                        sleep(DAEMON_ASK_EVERY);
                        waited += DAEMON_ASK_EVERY;
                    }
                }
            }
        }
        RestartOutcome::NeverStopped { reason } | RestartOutcome::Undetermined { reason } => {
            Release::Held(reason.clone())
        }
        RestartOutcome::Unknown => Release::Held(
            "the restart ended in a way this version of the app does not know".to_owned(),
        ),
    }
}

/// The whole removal after the file is saved (steps 2 to 4 of the module doc), with the two things
/// that touch a daemon passed in.
pub(super) fn finish_removal(
    removed: &RemovedPair,
    config_path: &Path,
    state_dir: Option<&Path>,
    restart: impl FnOnce() -> Result<RestartOutcome, String>,
    ask_pairs: impl FnMut() -> Option<Vec<String>>,
    sleep: impl FnMut(Duration),
    now: SystemTime,
) -> RemovePairReply {
    let settled_earlier = settle_earlier(state_dir, &removed.remaining, now);
    let restart = restart();
    let release = release_of(&removed.name, &restart, ask_pairs, sleep);

    let plan = set_aside::plan(&removed.view);
    let set_aside = if plan.items.is_empty() {
        SetAsideReport::NothingToMove {
            pair: removed.name.clone(),
            message: format!("'{}' had no sync history on disk to move.", removed.name),
        }
    } else {
        match (&release, state_dir) {
            (Release::Held(reason), _) => record_pending(&plan, reason, state_dir, now),
            (Release::Free, None) => record_pending(
                &plan,
                "the app has no state directory to put it in (neither XDG_STATE_HOME nor HOME names \
                 an absolute location)",
                None,
                now,
            ),
            (Release::Free, Some(state_dir)) => {
                let context = Context {
                    state_dir,
                    configured: &removed.remaining,
                    now,
                };
                match set_aside::execute(&plan, &context) {
                    Ok(done) => moved_report(&removed.name, &done),
                    Err(failed) => {
                        // What did move stays moved; what is left is what is pending.
                        let left = Plan {
                            items: failed.remaining.clone(),
                            ..plan.clone()
                        };
                        record_pending(&left, &failed.reason, Some(state_dir), now)
                    }
                }
            }
        }
    };
    let restart_needed = !matches!(
        restart,
        Ok(RestartOutcome::Restarted { .. } | RestartOutcome::NotRunning)
    );
    RemovePairReply {
        pair: removed.name.clone(),
        path: lossy(config_path),
        new_default: removed.new_default.clone(),
        restart: restart.unwrap_or_else(|reason| RestartOutcome::Undetermined { reason }),
        restart_needed,
        set_aside,
        settled_earlier,
    }
}

fn record_pending(
    plan: &Plan,
    reason: &str,
    state_dir: Option<&Path>,
    now: SystemTime,
) -> SetAsideReport {
    let record = state_dir.and_then(|dir| set_aside::record_pending(dir, plan, reason, now).ok());
    pending_report(&plan.pair, reason, record.as_deref(), &plan.items)
}
