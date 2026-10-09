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
//!
//! **A reply never says more than was found out.** "Nothing to move" is said only of a folder that
//! could be read and held no state; a folder that could not be looked at (an unplugged drive, no
//! permission, a path that is relative and so means something different to the daemon) is pending
//! with that reason. "Moved" is said only of what was moved: a link moves as the link, and the reply
//! says where what it pointed at still is; a move that stopped part-way names both halves.

use super::RestartOutcome;
use gui_core::config_io::{self, ConfigDoc, PairInit, PairView};
use gui_core::set_aside::{self, Context, Done, MovedItem, Plan, Planned, Settled};
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
    /// It was moved. `to` is the directory it is in now. `left_behind` lists any item that was a
    /// link: the link moved and what it pointed at did not.
    Moved {
        pair: String,
        to: String,
        items: Vec<MovedReport>,
        left_behind: Vec<LeftBehind>,
        notes: Vec<String>,
        message: String,
    },
    /// **Only links** were moved: the history itself is where each link pointed, untouched. Not
    /// `moved`, because a screen reading that would tell the person their history is out of the folder.
    LinkMoved {
        pair: String,
        to: String,
        items: Vec<MovedReport>,
        left_behind: Vec<LeftBehind>,
        notes: Vec<String>,
        message: String,
    },
    /// There was no history on disk for this pair, in a folder that could be read. `notes` lists what
    /// was found and left alone (a `.sync` that is a file).
    NothingToMove {
        pair: String,
        notes: Vec<String>,
        message: String,
    },
    /// It could not be moved now (or not all of it), and has been recorded so that it still can be.
    /// `record` is where. `moved` is what **did** move before it stopped, into `moved_to`; `still_at`
    /// is what is still where it was — empty when the app could not look, and the reason says so.
    Pending {
        pair: String,
        reason: String,
        record: Option<String>,
        moved: Vec<MovedReport>,
        moved_to: Option<String>,
        still_at: Vec<String>,
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
    /// Set when `from` was a link: only the link moved, and this is what it pointed at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
}

/// A link that moved, and where the thing it pointed at still is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LeftBehind {
    pub link: String,
    pub target: String,
}

fn moved_item_report(item: &MovedItem) -> MovedReport {
    MovedReport {
        from: lossy(&item.from),
        to: lossy(&item.to),
        how: match item.how {
            set_aside::How::Rename => "rename",
            set_aside::How::Copy => "copy",
        }
        .to_owned(),
        link_target: item.link_target.as_deref().map(lossy),
    }
}

fn notes_sentence(notes: &[String]) -> String {
    notes
        .iter()
        .map(|note| format!(" Note: {note}."))
        .collect::<String>()
}

/// `looked_in` is the folder that was read and held none: the reply names it, because "nothing" is a
/// claim about where the app looked (a drive that is not mounted can leave an empty folder that reads
/// the same).
fn nothing_report(pair: &str, looked_in: Option<&Path>, notes: &[String]) -> SetAsideReport {
    let said = match looked_in {
        Some(folder) => format!(
            "'{pair}' had no sync history to move: its folder {} could be read and held none.",
            lossy(folder)
        ),
        None => format!("'{pair}' had no sync history on disk to move."),
    };
    SetAsideReport::NothingToMove {
        pair: pair.to_owned(),
        notes: notes.to_vec(),
        message: format!("{said}{}", notes_sentence(notes)),
    }
}

fn moved_report(pair: &str, done: &Done) -> SetAsideReport {
    if done.moved.is_empty() {
        return nothing_report(pair, None, &done.notes);
    }
    let items: Vec<MovedReport> = done.moved.iter().map(moved_item_report).collect();
    let left_behind: Vec<LeftBehind> = done
        .moved
        .iter()
        .filter_map(|item| {
            item.link_target.as_deref().map(|target| LeftBehind {
                link: lossy(&item.from),
                target: lossy(target),
            })
        })
        .collect();
    let links = left_behind
        .iter()
        .map(|left| format!("{} was a link to {}", left.link, left.target))
        .collect::<Vec<_>>()
        .join("; ");
    let to = lossy(&done.to);
    if left_behind.len() == done.moved.len() {
        return SetAsideReport::LinkMoved {
            pair: pair.to_owned(),
            message: format!(
                "Only a LINK was moved for '{pair}', not its sync history. {links}. The link is now \
                 in {to}; what it pointed at, which is where the history actually is, was left \
                 exactly as it was and was not touched. Adding the folder again starts a new index \
                 in the folder, and the old one stays where it is.{}",
                notes_sentence(&done.notes)
            ),
            to,
            items,
            left_behind,
            notes: done.notes.clone(),
        };
    }
    let mut message = format!(
        "The sync history of '{pair}' was moved to {to}, outside every sync folder. Your files \
         were not touched. Adding the folder again starts fresh: it matches and downloads, and \
         deletes nothing."
    );
    if !left_behind.is_empty() {
        message.push_str(&format!(
            " Part of it was a link, though: {links}. Only the link moved; what it points at was \
             not touched and is still where it was."
        ));
    }
    message.push_str(&notes_sentence(&done.notes));
    SetAsideReport::Moved {
        pair: pair.to_owned(),
        to,
        items,
        left_behind,
        notes: done.notes.clone(),
        message,
    }
}

/// What a pending report is built from.
struct PendingFacts<'a> {
    pair: &'a str,
    reason: &'a str,
    /// The note kept so the move can still happen, when one could be written.
    record: Option<&'a Path>,
    /// What is still where it was.
    still_at: &'a [PathBuf],
    /// What moved before it stopped, and where to.
    moved: &'a [MovedItem],
    moved_to: Option<&'a Path>,
    /// The app could not look at the disk: `still_at` is not "none", it is unknown.
    unlooked: bool,
}

fn pending_report(facts: &PendingFacts<'_>) -> SetAsideReport {
    let PendingFacts {
        pair,
        reason,
        record,
        still_at,
        moved,
        moved_to,
        unlooked,
    } = *facts;
    let where_it_is = still_at
        .iter()
        .map(|item| lossy(item))
        .collect::<Vec<_>>()
        .join(", ");
    let message = if unlooked {
        let kept = match record {
            Some(record) => format!(
                "A note of it is kept at {}; it is tried again the next time a folder is added or \
                 removed here, and it can be deleted if the folder is gone for good. Until it has \
                 been looked at, adding the same folder again could resume an old history and read \
                 what changed since as deletions to approve.",
                lossy(record)
            ),
            None => "Look in that folder yourself: a `.sync` folder, or the index file the config \
                     names, left there would be resumed if the same folder is added again, and \
                     what changed since read as deletions to approve. Move it out of the folder \
                     before adding it."
                .to_owned(),
        };
        format!(
            "The app could not look at the sync history of '{pair}': {reason}. It moved nothing, \
             and cannot say whether there is any to move. {kept}"
        )
    } else {
        // "yet" only where a note promises it will be tried again.
        let not_moved = if record.is_some() {
            "NOT moved yet"
        } else {
            "NOT moved"
        };
        let head = match (moved.is_empty(), moved_to) {
            (false, Some(to)) => format!(
                "Part of the sync history of '{pair}' WAS moved to {}: {}. The rest was \
                 {not_moved}: {reason}. ",
                lossy(to),
                moved
                    .iter()
                    .map(|item| format!("{} to {}", lossy(&item.from), lossy(&item.to)))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            _ => format!("The sync history of '{pair}' was {not_moved}: {reason}. "),
        };
        match record {
            Some(record) => format!(
                "{head}It is still at {where_it_is}, and a note of it is kept at {}; it moves the \
                 next time a folder is added or removed here. Until then, adding the same folder \
                 again would resume that history and read what changed since as deletions to \
                 approve.",
                lossy(record)
            ),
            None => format!(
                "{head}It is still at {where_it_is}, and the app could not keep a note of it. Move \
                 it out of the folder yourself before adding the same folder again, or that \
                 history is resumed and what changed since is read as deletions to approve."
            ),
        }
    };
    SetAsideReport::Pending {
        pair: pair.to_owned(),
        reason: reason.to_owned(),
        record: record.map(lossy),
        moved: moved.iter().map(moved_item_report).collect(),
        moved_to: moved_to.map(lossy),
        still_at: still_at.iter().map(|item| lossy(item)).collect(),
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
        Settled::NothingLeft {
            pair,
            notes,
            had_items,
        } => SetAsideReport::NothingToMove {
            message: if had_items {
                format!(
                    "An earlier removal of '{pair}' had history to move, and none is left on disk \
                     now (it was moved or deleted by hand), so the note of it was dropped.{}",
                    notes_sentence(&notes)
                )
            } else {
                format!(
                    "The folder of the earlier removal of '{pair}' can now be read and holds no \
                     history; nothing was moved, so the note of it was dropped.{}",
                    notes_sentence(&notes)
                )
            },
            pair,
            notes,
        },
        Settled::StillPending { pair, reason } => SetAsideReport::Pending {
            message: format!(
                "The earlier removal of '{pair}' still could not move its history: {reason}."
            ),
            pair,
            reason,
            record: None,
            moved: Vec::new(),
            moved_to: None,
            still_at: Vec::new(),
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
    /// Things worth the person's attention that did not stop the add.
    pub warnings: Vec<String>,
}

/// An add that was refused. The earlier removals this call settled before it looked at the request are
/// kept: that happened, and the person is told even though the add did not.
#[derive(Debug)]
pub struct AddPairFailure {
    pub error: String,
    pub settled_earlier: Vec<SetAsideReport>,
}

impl AddPairFailure {
    fn plain(error: impl ToString) -> Self {
        Self {
            error: error.to_string(),
            settled_earlier: Vec::new(),
        }
    }

    /// The error as the command returns it: the refusal, then what the call did regardless.
    pub fn into_message(self) -> String {
        if self.settled_earlier.is_empty() {
            return self.error;
        }
        let settled = self
            .settled_earlier
            .iter()
            .map(|report| match report {
                SetAsideReport::Moved { message, .. }
                | SetAsideReport::LinkMoved { message, .. }
                | SetAsideReport::NothingToMove { message, .. }
                | SetAsideReport::Pending { message, .. }
                | SetAsideReport::Superseded { message, .. } => message.as_str(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "{} (This did not stop the app finishing an earlier removal first: {settled})",
            self.error
        )
    }
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

/// The app's set-aside histories live in `<state dir>/removed-pairs`. A sync folder that contains
/// them would upload them as ordinary files, so the add says so. Not a refusal: a folder that holds
/// the whole home directory is a legitimate choice, and the state directory is under it.
fn state_dir_warnings(state_dir: Option<&Path>, new_root: &Path) -> Vec<String> {
    let Some(state_dir) = state_dir else {
        return Vec::new();
    };
    let removed_pairs = state_dir.join(set_aside::REMOVED_PAIRS_DIR);
    if !set_aside::real_path(&removed_pairs).starts_with(set_aside::real_path(new_root)) {
        return Vec::new();
    }
    vec![format!(
        "{} is inside this folder. The sync history of folders removed in this app is set aside \
         there, and this folder would upload it as ordinary files. Move the app's state directory \
         (XDG_STATE_HOME) out of the folder, or choose a folder that does not contain it.",
        lossy(&removed_pairs)
    )]
}

/// Everything an add is refused for, on a copy of the file: the engine's rules (the name, the roots, a
/// lexical overlap, `dry_run = true` beside a second pair), then the two local-root checks the engine
/// does not make, then the real-path overlap, which follows symlinks and so touches the disk. Nothing is
/// written. **The one body of "would this add go ahead"**, read by the add itself and by the check the
/// dialog makes before it (`check_add_pair_file`) — two copies of a refusal are how a dialog comes to
/// say "fine" about an add the command then refuses.
///
/// The order of the refusals is part of the contract, and a test holds it: a file that is simply invalid
/// is told so before anything is asked of the filesystem.
fn prepare_add(
    mut doc: ConfigDoc,
    name: &str,
    request: &AddPairRequest,
) -> Result<ConfigDoc, String> {
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
    Ok(doc)
}

/// The blocking half of `add_pair`: load, settle what an earlier removal left, add, check, save.
///
/// Any refusal leaves the file byte-identical (see [`prepare_add`] for which, and in what order).
///
/// **The earlier removals are settled first, before the request is looked at**, because a pending move
/// has to run against a config that does not yet hold the folder being added: settled afterwards it
/// would find the folder configured again and be dropped, leaving the old index to be resumed. That
/// ordering means a refused add can still have moved an earlier history, so a refusal carries what was
/// settled ([`AddPairFailure`]).
pub(super) fn add_pair_file(
    path: &Path,
    state_dir: Option<&Path>,
    name: &str,
    request: &AddPairRequest,
    now: SystemTime,
) -> Result<AddPairReply, AddPairFailure> {
    let doc = ConfigDoc::load(path).map_err(AddPairFailure::plain)?;
    // Only against a config that can be read: what is configured NOW is what keeps an earlier removal
    // from moving a folder's history out from under a pair that has since been added back, and a file
    // that cannot be read says nothing about that.
    let settled_earlier = match config_io::pair_views(&doc.to_toml_string()) {
        Ok(configured) => settle_earlier(state_dir, &configured, now),
        Err(_) => Vec::new(),
    };
    let refuse = |error: String| AddPairFailure {
        error,
        settled_earlier: settled_earlier.clone(),
    };

    let local_root = request.local_root.trim();
    let doc = prepare_add(doc, name, request).map_err(refuse)?;
    doc.save(path).map_err(|error| refuse(text(error)))?;

    let saved = doc.to_toml_string();
    let views = config_io::pair_views(&saved).map_err(|error| refuse(text(error)))?;
    let surviving_index = views
        .iter()
        .find(|view| view.name == name)
        .and_then(|view| surviving_index_of(view, state_dir));
    let warnings = state_dir_warnings(
        state_dir,
        &config_io::expand_config_path(local_root, "local_root"),
    );
    Ok(AddPairReply {
        pair: name.to_owned(),
        path: lossy(path),
        restart_needed: true,
        surviving_index,
        settled_earlier,
        warnings,
    })
}

/// What the add dialog is told before anything is written (#102 phase 5c-2, brief 3.5): every refusal
/// the add itself would make, and the two things it would only report afterwards.
#[derive(Debug, serde::Serialize)]
pub struct AddPairCheck {
    /// A name for the folder, from its own name (`suggest_pair_name`), distinct from the pairs the file
    /// has. Offered when the person has typed none; never a validation.
    pub suggested_name: String,
    /// The engine's refusal of the NAME, in its own words (voice rule 4), or null. Asked on its own and
    /// first so a dialog can draw it under the name field while the rest is still being typed.
    pub name_error: Option<String>,
    /// What the add would refuse with once the name passed — the words `add_pair` itself uses — or null
    /// when it would go ahead. Null as well while a root is still empty: nothing is asked of a half-typed
    /// form beyond its name.
    pub refusal: Option<String>,
    /// An index from an earlier run that the new pair would resume, named BEFORE the save so the
    /// confirmation can say so (brief A9). Null when there is none, and null when the add would be
    /// refused anyway.
    pub surviving_index: Option<SurvivingIndex>,
    /// Things worth the person's attention that would not stop the add.
    pub warnings: Vec<String>,
}

/// The blocking half of `check_add_pair`: [`add_pair_file`] without the part that changes anything — no
/// earlier removal is settled, nothing is saved — answering what it would have said.
pub(super) fn check_add_pair_file(
    path: &Path,
    state_dir: Option<&Path>,
    name: &str,
    request: &AddPairRequest,
) -> AddPairCheck {
    let nothing = |suggested_name: String| AddPairCheck {
        suggested_name,
        name_error: None,
        refusal: None,
        surviving_index: None,
        warnings: Vec::new(),
    };
    let doc = match ConfigDoc::load(path) {
        Ok(doc) => doc,
        Err(error) => {
            return AddPairCheck {
                refusal: Some(text(error)),
                ..nothing(config_io::suggest_pair_name(&request.local_root, &[]))
            };
        }
    };
    let existing = doc.pair_names();
    let others: Vec<&str> = existing.iter().map(String::as_str).collect();
    let mut check = nothing(config_io::suggest_pair_name(&request.local_root, &others));
    // The engine's verdict on the name, and only on the name: the same call `add_pair` makes first.
    if let Err(sentence) = config_io::validate_pair_name_among(name, &others, others.len()) {
        check.name_error = Some(sentence);
        return check;
    }
    let (local_root, remote_root) = (request.local_root.trim(), request.remote_root.trim());
    if local_root.is_empty() || remote_root.is_empty() {
        return check;
    }
    match prepare_add(doc, name, request) {
        Err(refusal) => check.refusal = Some(refusal),
        Ok(prepared) => {
            check.surviving_index = config_io::pair_views(&prepared.to_toml_string())
                .ok()
                .and_then(|views| {
                    views
                        .iter()
                        .find(|view| view.name == name)
                        .and_then(|view| surviving_index_of(view, state_dir))
                });
            check.warnings = state_dir_warnings(
                state_dir,
                &config_io::expand_config_path(local_root, "local_root"),
            );
        }
    }
    check
}

fn surviving_index_of(view: &PairView, state_dir: Option<&Path>) -> Option<SurvivingIndex> {
    let index = set_aside::surviving_index(view)?;
    let root = view.local_root.as_deref()?;
    let pending = state_dir.is_some_and(|dir| {
        set_aside::pending(dir).iter().any(|(_, record)| {
            set_aside::real_path(&record.plan.local_root) == set_aside::real_path(root)
        })
    });
    // The words are the maintainer's (decision D8), and the command names the pair: `--pair` is a
    // global flag of `proton-sync`, and `reset-index --yes` alone resets the DEFAULT pair, which with
    // two folders is usually not the one that was just added.
    let mut message = format!(
        "This folder already holds sync history from an earlier setup ({}); adding it resumes from \
         that history, so anything changed since may show up as deletions to approve. To start fresh \
         instead, run proton-sync reset-index --yes --pair {} after adding. This app cannot reset \
         an index.",
        lossy(&index),
        view.name
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

    let set_aside = match set_aside::plan(&removed.view) {
        Planned::Nothing { notes } => nothing_report(
            &removed.name,
            removed.view.local_root.as_deref(),
            &notes,
        ),
        Planned::Undetermined(undetermined) => {
            // Only a reason a later look could clear is worth a note; a path the app can never
            // locate safely is not.
            let record = undetermined
                .retry
                .then(|| {
                    let plan = Plan::of_view(&removed.view, Vec::new(), Vec::new());
                    state_dir.and_then(|dir| {
                        set_aside::record_pending(dir, &plan, &undetermined.reason, now).ok()
                    })
                })
                .flatten();
            pending_report(&PendingFacts {
                pair: &removed.name,
                reason: &undetermined.reason,
                record: record.as_deref(),
                still_at: &[],
                moved: &[],
                moved_to: None,
                unlooked: true,
            })
        }
        Planned::Move(plan) => match (&release, state_dir) {
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
                        // What did move stays moved; what is left is what is pending, and the reply
                        // names both.
                        let left = Plan {
                            items: failed.remaining.clone(),
                            ..plan.clone()
                        };
                        let record = set_aside::record_pending(state_dir, &left, &failed.reason, now)
                            .ok();
                        pending_report(&PendingFacts {
                            pair: &plan.pair,
                            reason: &failed.reason,
                            record: record.as_deref(),
                            still_at: &failed.remaining,
                            moved: &failed.moved,
                            moved_to: failed.to.as_deref(),
                            unlooked: false,
                        })
                    }
                }
            }
        },
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
    pending_report(&PendingFacts {
        pair: &plan.pair,
        reason,
        record: record.as_deref(),
        still_at: &plan.items,
        moved: &[],
        moved_to: None,
        unlooked: false,
    })
}
