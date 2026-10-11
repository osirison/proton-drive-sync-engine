//! Starting a folder pair over when the person changes its Proton folder in Settings (#453, the Settings
//! half; the engine half is the hold in `daemon.rs`, `StandingCause::RemoteRootChanged`).
//!
//! **Why this exists.** A pair's index is the last agreed state of one local folder against **one** Proton
//! folder. Pointed at another, every path the old folder had and the new one lacks reads as deleted on
//! Proton. The engine refuses to run such a pair (it is held until `reset-index`, or until the folder is put
//! back), which is the right net and the wrong place to leave a person who just pressed `Save`: this app
//! cannot send `reset-index`. Maintainer decision (#453, 2026-10-10): changing a folder's Proton folder asks
//! once, and on confirmation starts that folder over from both sides — files only on one side are copied to
//! the other, a difference is kept as a conflict copy, nothing is deleted — and the folder is not held.
//!
//! **What "start over" is here: the engine's own `reset_index_state`, offline.** It is the transaction the
//! daemon runs for `proton-sync reset-index` (baseline, event cursors, warm-start count, delete approvals,
//! the recorded Proton folder and the rest of what the daemon learned), applied to the index file while no
//! daemon has it open. Not [`crate::set_aside`], which Remove uses and which moves the whole `.sync`
//! directory out of the folder: that would also move the pair's **pause**, which `reset_index_state`
//! keeps on purpose (`index::load_pair_paused`: a start-over that quietly resumed the folder would sync one
//! the person had told it to leave alone).
//!
//! **When it may run.** Only with the daemon not holding the pair: the caller stops the daemon first and
//! [`start_over`] then takes the pair's own lockfile non-blockingly, the same proof [`crate::set_aside`]
//! uses, which does not depend on a socket answering. A lock another process holds, or one that cannot be
//! asked, is a refusal and the index is not opened.
//!
//! **What it never does.** It never makes anything: no folder, no `.sync`, no index, no lockfile. A folder
//! with no index has nothing to start over and is left exactly as it was. It never touches a file of the
//! person's. And it never follows a relative path, which the daemon resolves against *its* working
//! directory and not this app's ([`locate`] shares [`crate::set_aside::plan`]'s reading of that).

use crate::config_io::PairView;
use crate::set_aside::{self, LockProbe, Planned};
use proton_drive_sync_engine::config::same_remote_folder;
use proton_drive_sync_engine::index::{open_database, reset_index_state};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Whether saving `staged` as a pair's `remote_root` changes the Proton folder it syncs with, given the
/// value the config file holds for it now.
///
/// **The engine's rule, asked of the engine** ([`same_remote_folder`]): `/Drive/X`, `Drive/X` and `Drive/X/`
/// are one folder and are not a change; a subfolder, a parent, another case and a trailing space are other
/// folders and are. The values are compared as written, because that is what the daemon reads.
///
/// `false` when the file holds no Proton folder for the pair (there is no baseline to resume against it
/// and nothing to start over from) and when `staged` is empty (a save the config writer refuses in the
/// engine's words; it is not this module's to prompt for).
pub fn remote_folder_changed(saved: Option<&str>, staged: &str) -> bool {
    let Some(saved) = saved.filter(|saved| !saved.trim().is_empty()) else {
        return false;
    };
    if staged.trim().is_empty() {
        return false;
    }
    !same_remote_folder(Path::new(saved), Path::new(staged))
}

/// Why the folder was not started over. Every one of them means **the index was not changed**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The app cannot say where the folder's history is, or whether it is there: a relative path, a folder
    /// that cannot be read (an unplugged drive looks the same), a lockfile that cannot be asked. Not "there
    /// is no history".
    CannotLocate(String),
    /// A daemon holds the pair's lockfile: the index is in use.
    InUse(String),
    /// The index is there and could not be reset: not a database, not a file, a locked or full disk.
    Failed(String),
}

impl Refusal {
    /// The sentence, without which kind it was.
    pub fn reason(&self) -> &str {
        match self {
            Self::CannotLocate(reason) | Self::InUse(reason) | Self::Failed(reason) => reason,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// What a start-over did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// The index that was reset, or `None` when the folder had none (so nothing needed resetting and
    /// nothing was made).
    pub index: Option<PathBuf>,
}

/// Where the folder's index is, **looking and touching nothing**: `Ok(None)` when it is not there,
/// `Ok(Some(path))` when it is a file, and a [`Refusal`] when the disk cannot say.
///
/// "Not there" is a claim about a folder that could be read, so this is the question to ask **before**
/// stopping a daemon for the sake of the answer. The reading of what can and cannot be looked at is
/// [`set_aside::plan`]'s and is not restated: a relative path, a folder that cannot be listed, a state
/// path under a folder that cannot be listed.
pub fn locate(view: &PairView) -> Result<Option<PathBuf>, Refusal> {
    if let Planned::Undetermined(undetermined) = set_aside::plan(view) {
        return Err(Refusal::CannotLocate(undetermined.reason));
    }
    let Some(db) = view.db_path.as_ref() else {
        return Err(Refusal::CannotLocate(
            "the config gives this folder no index path, so the app cannot tell where its sync \
             history is"
                .to_owned(),
        ));
    };
    match fs::metadata(db) {
        Ok(meta) if meta.is_file() => Ok(Some(db.clone())),
        Ok(_) => Err(Refusal::Failed(format!(
            "{} is not a file, so it is not a sync index to start over",
            db.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Refusal::CannotLocate(format!(
            "could not look at the index {}: {error}",
            db.display()
        ))),
    }
}

/// Start the folder over: take the pair's lockfile as the proof that no daemon has it open, then run the
/// engine's own `reset_index_state` on its index.
///
/// The transaction is the daemon's (`reset_index_state` is one transaction), so a failure leaves the index
/// as it was. The pair's pause and its history of what happened are not learned state and stay.
pub fn start_over(view: &PairView) -> Result<Done, Refusal> {
    let located = locate(view)?;
    // The proof that nobody has it, held until the reset is done. A missing lockfile is no holder (it is
    // never created here).
    let _held = match set_aside::acquire(view.lockfile_path.as_deref()) {
        LockProbe::Free(guard) => guard,
        LockProbe::Held => {
            return Err(Refusal::InUse(format!(
                "a sync service still has {} open",
                view.lockfile_path.as_deref().map_or_else(
                    || "this folder's sync history".to_owned(),
                    |l| l.display().to_string()
                )
            )));
        }
        LockProbe::Unknown(why) => {
            return Err(Refusal::CannotLocate(format!(
                "could not tell whether a sync service has the folder's sync history open: {why}"
            )));
        }
    };
    let Some(db) = located else {
        return Ok(Done { index: None });
    };
    let connection = open_database(&db).map_err(|error| {
        Refusal::Failed(format!(
            "the index {} could not be opened: {error}",
            db.display()
        ))
    })?;
    reset_index_state(&connection).map_err(|error| {
        Refusal::Failed(format!(
            "the index {} could not be reset: {error}",
            db.display()
        ))
    })?;
    drop(connection);
    Ok(Done { index: Some(db) })
}

/// An index as a daemon leaves it, and what is in one — the fixtures of this module's tests and of the
/// Tauri commands' (`feature = "test-support"`), so there is one definition of a folder with history.
/// Written by the engine's own functions, so what a test asserts about an index is what the daemon reads.
#[cfg(any(test, all(feature = "test-support", unix)))]
pub mod testing {
    use super::PairView;
    use proton_drive_sync_engine::index::{
        EntityKind, FileRecord, PassKind, PassOutcomeKind, SyncStatus, begin_pass, finish_pass,
        index_totals, load_event_cursor, load_pair_paused, load_recorded_remote_root,
        load_warm_start_count, open_database, recent_passes, record_remote_root_of_baseline,
        store_event_cursor, store_pair_paused, store_warm_start_count, upsert_record,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A pair named `docs` whose folder is `root` with the default layout: the index and the lockfile
    /// in `root/.sync`. Nothing is made.
    pub fn view_of(root: &Path) -> PairView {
        let state = root.join(".sync");
        PairView {
            name: "docs".to_owned(),
            local_root: Some(root.to_owned()),
            remote_root: Some(PathBuf::from("/Drive/new")),
            db_path: Some(state.join("sync_index.db")),
            lockfile_path: Some(state.join("proton-sync.lock")),
            conflict_suffix: None,
        }
    }

    /// Give `view`'s folder the history of a pair that has synced against `/Drive/old`: one file of the
    /// person's in the folder, and an index holding a baseline row for it, the Proton folder the baseline
    /// describes, a cursor, a warm-start count, a finished pass in its history, and — as asked — the
    /// pause the person set. The folder is made if it is not there.
    pub fn give_history(view: &PairView, paused: bool) {
        let root = view.local_root.as_ref().expect("a folder");
        let db = view.db_path.as_ref().expect("an index path");
        let lock = view.lockfile_path.as_ref().expect("a lockfile path");
        fs::create_dir_all(db.parent().expect("a state directory")).unwrap();
        fs::create_dir_all(lock.parent().expect("a lock directory")).unwrap();
        fs::write(root.join("mine.txt"), "the person's file").unwrap();
        fs::write(lock, "").unwrap();
        let connection = open_database(db).unwrap();
        upsert_record(
            &connection,
            &FileRecord {
                file_path: PathBuf::from("mine.txt"),
                entity_kind: EntityKind::File,
                file_size: 17,
                mtime: 1,
                sha1_hash: Some("ab".repeat(20)),
                proton_id: Some("vol~node".to_owned()),
                sync_status: SyncStatus::Synced,
            },
        )
        .unwrap();
        assert!(record_remote_root_of_baseline(&connection, Path::new("Drive/old")).unwrap());
        store_event_cursor(&connection, "vol", "event-9", 100).unwrap();
        store_warm_start_count(&connection, 3).unwrap();
        let pass = begin_pass(&connection, 1_000, PassKind::Incremental).unwrap();
        finish_pass(
            &connection,
            pass,
            5,
            PassOutcomeKind::Clean,
            1,
            0,
            0,
            0,
            None,
        )
        .unwrap();
        store_pair_paused(&connection, paused).unwrap();
    }

    /// What an index holds, as the daemon would read it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Facts {
        /// Files in the baseline.
        pub baseline_files: u64,
        /// The Proton folder the baseline describes (`None` while the baseline is empty, whatever the
        /// table holds: the engine's rule).
        pub recorded_root: Option<PathBuf>,
        pub has_cursor: bool,
        pub warm_starts: u64,
        pub paused: bool,
        /// Passes in the history of what happened.
        pub passes: usize,
    }

    /// Read `view`'s index as the engine does. Panics if there is none, which is the test's mistake.
    pub fn facts(view: &PairView) -> Facts {
        let connection = open_database(view.db_path.as_ref().expect("an index path")).unwrap();
        Facts {
            baseline_files: index_totals(&connection).unwrap().files,
            recorded_root: load_recorded_remote_root(&connection).unwrap(),
            has_cursor: load_event_cursor(&connection, "vol").unwrap().is_some(),
            warm_starts: load_warm_start_count(&connection).unwrap(),
            paused: load_pair_paused(&connection).unwrap(),
            passes: recent_passes(&connection, 10).unwrap().len(),
        }
    }

    /// The facts of an index that has been started over: no baseline, nothing recorded, no cursor, and
    /// the history and the pause as they were.
    pub fn started_over(paused: bool) -> Facts {
        Facts {
            baseline_files: 0,
            recorded_root: None,
            has_cursor: false,
            warm_starts: 0,
            paused,
            passes: 1,
        }
    }

    /// The facts of the index [`give_history`] makes.
    pub fn with_history(paused: bool) -> Facts {
        Facts {
            baseline_files: 1,
            recorded_root: Some(PathBuf::from("Drive/old")),
            has_cursor: true,
            warm_starts: 3,
            paused,
            passes: 1,
        }
    }
}

#[cfg(test)]
mod tests;
