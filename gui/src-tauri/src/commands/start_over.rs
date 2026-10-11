//! Changing a folder's Proton folder from Settings (#453, the Settings half): everything the two commands
//! do that is not the `#[tauri::command]` wrapper itself.
//!
//! **The problem.** A folder's index is the last agreed state of one local folder against one Proton
//! folder. Saved pointing at another, the daemon would compare the old baseline with the new folder and
//! plan every path the old one had and the new one lacks as deleted on Proton. Since #458 the engine holds
//! such a pair (`StandingCause::RemoteRootChanged`) and its exits are `reset-index` or putting the folder
//! back — neither of which this app can do from a Settings screen. Maintainer decision (#453): changing the
//! Proton folder of an existing folder asks once, and on confirmation **starts that folder over from both
//! sides** (files only on one side are copied to the other, a difference is kept as a conflict copy, nothing
//! is deleted) and the folder is not held.
//!
//! **The order is the design, and each step may fail without leaving the folder resumable against the wrong
//! Proton folder:**
//!
//! 1. everything the save would be refused for is asked **on a copy of the file**, and where the folder's
//!    history is and whether it can be reached is asked **before anything is stopped** — a refusal here
//!    changes nothing at all;
//! 2. the daemon is asked whether it is running, and **an answer that does not come is a refusal**: a daemon
//!    that may be running may be holding this folder as held or unavailable, which holds neither its index
//!    nor its lockfile, so the lockfile proves nothing about it and a reset under it would be undone by it
//!    (it lifts the hold, bootstraps against the Proton folder it still has in memory and records it). A
//!    running daemon is stopped and the stop is confirmed (an absent socket);
//! 3. the folder's index is started over (`gui_core::start_over`: the engine's own `reset_index_state`, under
//!    the pair's lockfile, which is the proof that no daemon has it open);
//! 4. the file is written;
//! 5. the daemon, if this stopped it, is started again.
//!
//! The history is therefore gone **before** the file names the new Proton folder, so the app's own path
//! never produces "new folder, old baseline". A failure at step 2 changes nothing; at step 3 the index is
//! as it was (one transaction) and the old settings run again; at step 4 the folder starts over under its
//! old Proton folder; at step 5 the file and the index are both done and the daemon is simply not up. In no
//! case does the old baseline meet the new folder — and if something outside the app edits the file, the
//! engine's hold is still the net.
//!
//! **Why not the other order** (write the file, restart, let the held pair be reset): that is a window with
//! the new folder in the file and the old baseline on disk, held only by a daemon of the same version; and
//! with no daemon running at save time there is nobody to send the reset to, which would leave the folder
//! held with no way out in the app. This order needs no daemon to be running.
//!
//! Every step that touches a daemon is behind [`Service`], so the sequence is tested without a socket, a
//! process or `systemctl` (`docs/agent-notes/gui-tests-that-shell-systemctl.md`).

use super::{check_update, pair_admin, ConfigUpdate, DaemonProbe, RestartOutcome, NO_ANSWER};
use gui_core::config_io::{self, ConfigDoc, PairView};
use gui_core::start_over::{self as history, Refusal};
use std::path::Path;
use std::time::Duration;

fn text(error: impl ToString) -> String {
    error.to_string()
}

/// A sentence starts with a capital. The engine's and the disk's clauses are written to sit inside a
/// longer sentence and begin without one.
fn capitalised(clause: &str) -> String {
    let mut chars = clause.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// What a refusal says when **nothing has been touched yet**: its clause, that nothing was changed, and
/// what the person can do about it where there is something.
fn refused_before_anything_changed(refusal: &Refusal) -> String {
    let advice = refusal
        .advice()
        .map(|advice| format!(" {advice}"))
        .unwrap_or_default();
    format!(
        "{}. Nothing was changed.{advice}",
        capitalised(refusal.reason())
    )
}

// ---- the daemon, as the sequence needs it --------------------------------------------------------

/// What the sequence needs of the sync service. The real one dials the control socket and shells
/// `systemctl` (`commands.rs`'s `LiveService`); a test's keeps a log and can fail on cue.
pub(super) trait Service {
    /// Whether the daemon answers on its control socket. `Unknown` is evidence of neither, and the
    /// sequence **refuses** on it: see the module doc, step 2.
    fn probe(&mut self) -> DaemonProbe;
    /// Ask it to exit and wait for the socket to go quiet. `Ok` is a confirmed stop; the `Err` is the
    /// ending that says why not.
    fn stop(&mut self) -> Result<(), RestartOutcome>;
    /// Start it again (`Restarted`, or `NotStarted` with the reason).
    fn start(&mut self) -> RestartOutcome;
    /// Wait. Injected so a test does not.
    fn sleep(&mut self, how_long: Duration);
}

/// How long a daemon that was just stopped is given to let go of the pair's lockfile (its process may
/// outlive its socket by a moment), and how often the app looks.
pub(super) const LOCK_WAIT: Duration = Duration::from_secs(3);
pub(super) const LOCK_ASK_EVERY: Duration = Duration::from_millis(100);

// ---- the question --------------------------------------------------------------------------------

/// The answer to "does saving this Proton folder change the one the folder syncs with?".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RemoteRootCheck {
    /// Whether it is another folder **by the engine's rule** (`/Drive/X` and `Drive/X/` are one folder).
    pub changed: bool,
    /// The folder the file holds for this pair now, as written. `None` when it holds none.
    pub from: Option<String>,
    /// The folder asked about, as written.
    pub to: String,
}

/// The blocking half of `check_proton_folder_change`: read the file and ask the engine's rule. **Reads
/// nothing else and changes nothing** — no daemon is asked, no index is opened.
///
/// Compared against the file's value for the pair and not against anything the caller holds: the file is
/// what the field changed from, and a window's copy of it can be a poll old.
pub(super) fn check_file(
    path: &Path,
    pair: &str,
    remote_root: &str,
) -> Result<RemoteRootCheck, String> {
    let doc = ConfigDoc::load(path).map_err(text)?;
    let from = pair_of(&doc, pair)?.get_str("remote_root");
    Ok(RemoteRootCheck {
        changed: history::remote_folder_changed(from.as_deref(), remote_root),
        from,
        to: remote_root.to_owned(),
    })
}

/// `pair` as the config places it now, with the engine's defaults filled in (`config_io::pair_views`).
fn view_of(doc: &ConfigDoc, pair: &str) -> Result<PairView, String> {
    config_io::pair_views(&doc.to_toml_string())
        .map_err(text)?
        .into_iter()
        .find(|view| view.name == pair)
        .ok_or_else(|| format!("the settings have no folder named {pair:?}"))
}

fn pair_of<'a>(doc: &'a ConfigDoc, pair: &str) -> Result<config_io::PairRead<'a>, String> {
    doc.pair(pair).ok_or_else(|| {
        config_io::ConfigError::NoSuchPair {
            name: pair.to_owned(),
            known: doc.pair_names(),
        }
        .to_string()
    })
}

// ---- the sequence --------------------------------------------------------------------------------

/// What a start-over that saved did.
#[derive(Debug, serde::Serialize)]
pub struct StartOverReply {
    pub pair: String,
    /// The config file that was written.
    pub path: String,
    /// What the restart did, in the words a save's restart uses — the screen reads the same ending.
    pub restart: RestartOutcome,
    /// The index that was started over, or `None` when the folder had none (nothing was made).
    pub index: Option<String>,
}

/// The sentence for a stop that did not happen, in the daemon's own words where it gave some.
fn stop_failure(outcome: &RestartOutcome) -> String {
    match outcome {
        RestartOutcome::NeverStopped { reason } | RestartOutcome::Undetermined { reason } => {
            reason.clone()
        }
        other => format!("it ended in a way this step does not know ({other:?})"),
    }
}

/// Start the daemon again after something went wrong with the daemon stopped, and say how that went.
/// Said only when this sequence is what stopped it; otherwise the daemon is as it was.
fn started_again(service: &mut dyn Service, stopped: bool) -> String {
    if !stopped {
        return String::new();
    }
    match service.start() {
        RestartOutcome::Restarted { .. } => " The sync service was started again.".to_owned(),
        RestartOutcome::NotStarted { reason } => {
            format!(" The sync service could not be started again: {reason}.")
        }
        other => format!(" The sync service was asked to start again ({other:?})."),
    }
}

/// The blocking body of `change_proton_folder`. See the module doc for the order and why; the steps are
/// numbered here as there.
///
/// `Err` means **the settings file was not changed**. When the daemon had been stopped it was started
/// again, and the message says what became of that. Where the folder's index had already been started
/// over, the message says so: it will start over under the folder the file still names.
pub(super) fn change_proton_folder_file(
    config_path: &Path,
    pair: &str,
    update: &ConfigUpdate,
    service: &mut dyn Service,
) -> Result<StartOverReply, String> {
    // ---- 1. everything that can be asked first, on a copy; nothing stopped, nothing written ----
    let mut doc = ConfigDoc::load(config_path).map_err(text)?;
    let saved = pair_of(&doc, pair)?.get_str("remote_root");
    let Some(staged) = update.remote_root.as_deref() else {
        return Err(
            "This save does not change the Proton Drive folder, so there is nothing to start over. \
             Nothing was changed."
                .to_owned(),
        );
    };
    if !history::remote_folder_changed(saved.as_deref(), staged) {
        return Err(format!(
            "{pair} already syncs with that Proton Drive folder, so there is nothing to start over. \
             Nothing was changed: save again to apply the other settings."
        ));
    }
    let before = view_of(&doc, pair)?;
    check_update(&mut doc, pair, update)?;
    // The save validates too, but only after the index is reset: a root the daemon would refuse to start
    // on is told now, in the engine's words, while nothing has been touched.
    doc.validate().map_err(text)?;
    pair_admin::check_drive_folder(staged)?;
    let view = view_of(&doc, pair)?;
    // Where the history is that would meet the new Proton folder: the folder the file will name after this
    // save. A folder the save moves the pair to that is certainly not there has none.
    let folder = history::local_folder_moved(&before, &view);
    history::locate(&view, folder).map_err(|refusal| refused_before_anything_changed(&refusal))?;

    // ---- 2. the daemon, if it is running ----
    let probe = service.probe();
    if probe == DaemonProbe::Unknown {
        // Not "not running". A daemon that may be alive may hold this folder as held or unavailable, and
        // then it holds neither the lockfile nor the index: the reset would go through, the daemon would
        // lift its hold, bootstrap against the Proton folder it still has in memory and record it, and the
        // next restart would hold the folder against the new one. The plain save declines to act on the
        // same answer (`RestartPlan::LeaveUndetermined`), and so does this.
        return Err(format!("{}. Nothing was changed.", capitalised(NO_ANSWER)));
    }
    let stopped = probe == DaemonProbe::Running;
    if stopped {
        match service.stop() {
            Ok(()) | Err(RestartOutcome::NotRunning) => {}
            Err(outcome) => {
                return Err(format!(
                    "The sync service did not stop ({}), so nothing was changed.",
                    stop_failure(&outcome)
                ));
            }
        }
    }

    // ---- 3. the folder's index is started over ----
    // A daemon that was just stopped may hold the lockfile for a moment after its socket goes quiet.
    let mut waited = Duration::ZERO;
    let reset = loop {
        match history::start_over(&view, folder) {
            Err(Refusal::InUse(_)) if stopped && waited < LOCK_WAIT => {
                service.sleep(LOCK_ASK_EVERY);
                waited += LOCK_ASK_EVERY;
            }
            other => break other,
        }
    };
    let done = match reset {
        Ok(done) => done,
        Err(refusal) => {
            let again = started_again(service, stopped);
            return Err(format!(
                "{}. Nothing was saved and the folder's index was not changed.{again}",
                capitalised(refusal.reason())
            ));
        }
    };

    // ---- 4. the file ----
    if let Err(error) = doc.save(config_path) {
        let again = started_again(service, stopped);
        let reset_note = if done.index.is_some() {
            format!(
                " The sync history of {pair} had already been started over, so it will start over \
                 again under the Proton Drive folder the settings still name."
            )
        } else {
            String::new()
        };
        return Err(format!("{error}. Nothing was saved.{reset_note}{again}"));
    }

    // ---- 5. the daemon, if this stopped it ----
    let restart = if stopped {
        service.start()
    } else {
        // `NotRunning` is all that is left: an answer that did not come was refused above.
        RestartOutcome::NotRunning
    };
    Ok(StartOverReply {
        pair: pair.to_owned(),
        path: config_path.display().to_string(),
        restart,
        index: done.index.map(|index| index.display().to_string()),
    })
}
