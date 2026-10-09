//! Setting a removed folder pair's sync history aside (#102 phase 5b-2, maintainer decision D8).
//!
//! **Why this exists.** A pair's history is its index (`<root>/.sync/sync_index.db`, by default), and
//! the index is what makes the next pass two-way: *baseline says present, remote says missing* plans a
//! local DELETE. Remove a pair, add the same folder back, and the daemon finds the old index and
//! resumes it — reading everything that changed in between as deletions (brief A9). D8: removing a
//! pair moves its history out of the folder, so adding the folder again starts fresh, and a fresh
//! start is a bootstrap, which matches and downloads and never deletes.
//!
//! **What is moved, and only that.** The pair's *state*: the `<local_root>/.sync` directory, and any
//! of the files its [`PairView`] names (the index and SQLite's `-wal`/`-shm`/`-journal`, the two JSON
//! sidecars, the lockfile) that live somewhere else. Never a file the person made: the directory is
//! the engine's own (`index::should_ignore_path` ignores it everywhere), and the rest are named by the
//! config, not found by looking.
//!
//! **Where it goes.** Outside every sync root — the destination is checked against the real path of
//! every folder the config names, because history inside a sync folder is history that gets uploaded.
//!
//! **When it moves.** Only when no daemon holds it: the caller restarts the daemon off the pair first,
//! and [`acquire`] then takes the pair's own lockfile non-blockingly, which is the one proof of "nobody
//! has this open" that does not depend on a socket answering. A move that cannot happen now is a
//! [`Pending`] record that says why, retried by [`settle_pending`].
//!
//! **How it moves.** `rename`; on `EXDEV` a copy that is compared byte for byte with its source
//! before the source is touched. Nothing is deleted until the copy is verified, and a copy that fails
//! verification is removed and the original left exactly as it was.

use crate::config_io::PairView;
use proton_drive_sync_engine::index::canonicalize_best_effort;
use proton_drive_sync_engine::paths::sync_state_dir;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The directory (under the app's state directory) the set-aside copies go in.
pub const REMOVED_PAIRS_DIR: &str = "removed-pairs";
/// The directory (under the app's state directory) holding the records of moves that have not
/// happened yet.
pub const PENDING_DIR: &str = "pending-set-asides";
/// Written into each set-aside, so a person finding the directory knows what it is and where each
/// piece came from.
pub const MANIFEST_NAME: &str = "MANIFEST.json";

/// What would be moved for one pair, found by looking at the disk now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub pair: String,
    pub local_root: PathBuf,
    /// The `.sync` directory when there is one, and every named state file outside it that exists.
    pub items: Vec<PathBuf>,
    /// The pair's own lockfile: what proves nobody holds the state.
    pub lockfile: Option<PathBuf>,
}

/// Append `suffix` to a path's file name (`sync_index.db` → `sync_index.db-wal`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The files a [`PairView`]'s index and lockfile come with, whether or not they exist.
fn named_state_files(view: &PairView) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(db) = &view.db_path {
        files.push(db.clone());
        for suffix in ["-wal", "-shm", "-journal"] {
            files.push(with_suffix(db, suffix));
        }
        files.push(crate::sidecars::status_history_path(db));
        files.push(crate::sidecars::metrics_path(db));
    }
    if let Some(lock) = &view.lockfile_path {
        files.push(lock.clone());
    }
    files
}

fn exists_without_following(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// What setting this pair's history aside would move. Pure reading of the disk; moves nothing.
///
/// The `.sync` directory is taken **whole** (a symlink there is moved as the link and never
/// followed). A named state file inside it is covered by that; one outside it is listed on its own,
/// and only when it is a file — a directory named as a state file is a config the engine refuses, and
/// is not this module's to move.
pub fn plan(view: &PairView) -> Plan {
    let mut items = Vec::new();
    if let Some(root) = &view.local_root {
        let state = sync_state_dir(root);
        if exists_without_following(&state) {
            items.push(state.clone());
        }
        for file in named_state_files(view) {
            let covered = file.starts_with(&state);
            let is_file = fs::symlink_metadata(&file).is_ok_and(|meta| !meta.is_dir());
            // Never the folder itself, or something it sits inside: a state file cannot be the root.
            let encloses_root = root.starts_with(&file);
            if !covered && is_file && !encloses_root && !items.contains(&file) {
                items.push(file);
            }
        }
    }
    Plan {
        pair: view.name.clone(),
        local_root: view.local_root.clone().unwrap_or_default(),
        items,
        lockfile: view.lockfile_path.clone(),
    }
}

/// The index file this pair would resume if it were added back, when one is on disk. The add flow
/// names it (brief A9, E18): **not** a thing the app can reset — only `proton-sync reset-index` can.
pub fn surviving_index(view: &PairView) -> Option<PathBuf> {
    let db = view.db_path.as_ref()?;
    fs::symlink_metadata(db)
        .is_ok_and(|meta| meta.is_file())
        .then(|| db.clone())
}

// ---- the proof that nobody holds it -------------------------------------------------------------

/// Holding a pair's lockfile exclusively, for as long as it is alive.
#[derive(Debug)]
pub struct LockGuard {
    // Held for its Drop: closing the descriptor releases the `flock`.
    _file: Option<File>,
}

/// What asking a pair's lockfile showed.
#[derive(Debug)]
pub enum LockProbe {
    /// Nobody holds it (or there is no lockfile to hold). The guard keeps it that way until dropped.
    Free(LockGuard),
    /// A daemon holds it: the state is in use.
    Held,
    /// The file could not be opened or asked. **Evidence of neither**: nothing is moved on it.
    Unknown(String),
}

/// Take `lockfile` exclusively without waiting — the daemon's own `try_lock_exclusive` — to learn
/// whether a daemon holds the pair. A missing file is no holder. Never creates the file.
pub fn acquire(lockfile: Option<&Path>) -> LockProbe {
    let Some(lockfile) = lockfile else {
        return LockProbe::Free(LockGuard { _file: None });
    };
    let file = match File::open(lockfile) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return LockProbe::Free(LockGuard { _file: None });
        }
        Err(error) => return LockProbe::Unknown(format!("{}: {error}", lockfile.display())),
    };
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => LockProbe::Free(LockGuard { _file: Some(file) }),
        Err(errno) if errno == rustix::io::Errno::WOULDBLOCK => LockProbe::Held,
        Err(errno) => LockProbe::Unknown(format!("{}: {errno}", lockfile.display())),
    }
}

// ---- where it goes ------------------------------------------------------------------------------

/// `20261009T142530Z`, for a directory name that sorts by time. No date crate: this app has none, and
/// the conversion is the standard days-to-civil one.
pub fn utc_stamp(at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Days since 1970-01-01 → civil date (Howard Hinnant's algorithm, shifted to a March year).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// A pair's name as a directory-name part: the engine's charset already makes it one, and this does
/// not rely on that.
fn directory_part(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.starts_with('.') {
        format!("pair{cleaned}")
    } else {
        cleaned
    }
}

/// Why a location is not one the history may go to, if it is not: it is inside a sync folder (where
/// it would be uploaded) or around one (where the folder would be inside the history).
fn outside_every_root(destination: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let real = canonicalize_best_effort(destination);
    for root in roots {
        let root_real = canonicalize_best_effort(root);
        if real.starts_with(&root_real) || root_real.starts_with(&real) {
            return Err(format!(
                "{} is inside or around the sync folder {}: set-aside history there would be \
                 uploaded as ordinary files",
                destination.display(),
                root.display()
            ));
        }
    }
    Ok(())
}

/// A folder as the filesystem names it — every link resolved, a part that does not exist yet resolved
/// as far as it exists — so "the same folder" has the engine's answer here as it does in the
/// daemon's overlap rule.
pub fn real_path(path: &Path) -> PathBuf {
    canonicalize_best_effort(path)
}

/// Whether `item` is one of `view`'s own files, or holds them, by real path: the state of a pair that
/// is configured — so not history, and not this module's to move.
fn belongs_to(item: &Path, view: &PairView) -> bool {
    let item = canonicalize_best_effort(item);
    let mut owned: Vec<PathBuf> = named_state_files(view);
    if let Some(root) = &view.local_root {
        owned.push(sync_state_dir(root));
    }
    owned.iter().any(|file| {
        let file = canonicalize_best_effort(file);
        file.starts_with(&item) || item.starts_with(&file)
    })
}

// ---- moving -------------------------------------------------------------------------------------

/// How one item got there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// One `rename` (the same filesystem).
    Rename,
    /// Copied, compared with its source, and only then removed from where it was.
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedItem {
    pub from: PathBuf,
    pub to: PathBuf,
    pub how: How,
}

/// The two operations that touch the disk, as values, so a test can make the filesystem boundary
/// appear (`EXDEV`) and a copy go wrong without needing two filesystems or a broken disk.
pub struct Mover<'a> {
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    pub same: &'a dyn Fn(&Path, &Path) -> io::Result<bool>,
}

fn crosses_devices(error: &io::Error) -> bool {
    // `ErrorKind::CrossesDevices` is the typed spelling; the raw errno is `EXDEV` (18 on Linux), kept
    // because a platform that maps it to `Uncategorized` would otherwise read as a plain failure.
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(18)
}

fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)
    } else if kind.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_entry(&entry.path(), &to.join(entry.file_name()))?;
        }
        // After the contents: a read-only directory would refuse them.
        fs::set_permissions(to, meta.permissions())
    } else if kind.is_file() {
        fs::copy(from, to).map(|_| ())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{} is not a file, directory or link", from.display()),
        ))
    }
}

/// Whether two trees are the same: same kinds, same names, same link targets, same bytes.
fn same_tree(a: &Path, b: &Path) -> io::Result<bool> {
    let (meta_a, meta_b) = (fs::symlink_metadata(a)?, fs::symlink_metadata(b)?);
    let (kind_a, kind_b) = (meta_a.file_type(), meta_b.file_type());
    if kind_a.is_symlink() || kind_b.is_symlink() {
        return Ok(kind_a.is_symlink()
            && kind_b.is_symlink()
            && fs::read_link(a)? == fs::read_link(b)?);
    }
    if kind_a.is_dir() || kind_b.is_dir() {
        if !(kind_a.is_dir() && kind_b.is_dir()) {
            return Ok(false);
        }
        let names = |path: &Path| -> io::Result<Vec<OsString>> {
            let mut names = fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()?;
            names.sort();
            Ok(names)
        };
        let (names_a, names_b) = (names(a)?, names(b)?);
        if names_a != names_b {
            return Ok(false);
        }
        for name in names_a {
            if !same_tree(&a.join(&name), &b.join(&name))? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if meta_a.len() != meta_b.len() {
        return Ok(false);
    }
    let (mut file_a, mut file_b) = (File::open(a)?, File::open(b)?);
    let (mut buffer_a, mut buffer_b) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let read_a = read_full(&mut file_a, &mut buffer_a)?;
        let read_b = read_full(&mut file_b, &mut buffer_b)?;
        if read_a != read_b || buffer_a[..read_a] != buffer_b[..read_b] {
            return Ok(false);
        }
        if read_a == 0 {
            return Ok(true);
        }
    }
}

/// `read` until the buffer is full or the file ends: two files are compared by blocks of the same
/// size, which a short read on one would otherwise misalign.
fn read_full(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn remove_entry(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Move one item. Rename if the filesystem allows; otherwise copy, compare, and only then remove.
///
/// **The order is the safety.** The original is removed after — and only after — `same` has said the
/// copy is it. A copy that cannot be made or does not compare equal is removed (it is ours: it is
/// under a directory this move just created) and the original is left untouched.
fn move_one(from: &Path, to: &Path, mover: &Mover<'_>) -> Result<How, String> {
    match (mover.rename)(from, to) {
        Ok(()) => return Ok(How::Rename),
        Err(error) if crosses_devices(&error) => {}
        Err(error) => return Err(format!("could not move {}: {error}", from.display())),
    }
    if let Err(error) = copy_entry(from, to) {
        let _ = remove_entry(to);
        return Err(format!("could not copy {}: {error}", from.display()));
    }
    match (mover.same)(from, to) {
        Ok(true) => {}
        Ok(false) => {
            let _ = remove_entry(to);
            return Err(format!(
                "the copy of {} did not match it, so the original was left where it was",
                from.display()
            ));
        }
        Err(error) => {
            let _ = remove_entry(to);
            return Err(format!(
                "could not compare the copy of {} with it ({error}), so the original was left \
                 where it was",
                from.display()
            ));
        }
    }
    remove_entry(from).map_err(|error| {
        format!(
            "copied {} to {} and verified the copy, but could not remove the original ({error}): \
             remove it by hand before adding the folder again",
            from.display(),
            to.display()
        )
    })?;
    Ok(How::Copy)
}

/// What a finished set-aside did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// The directory the history went to.
    pub to: PathBuf,
    pub moved: Vec<MovedItem>,
}

/// Why a set-aside did not (finish) happen, and what is left to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub reason: String,
    /// What did get moved before the failure (already in its place; never moved back).
    pub moved: Vec<MovedItem>,
    /// What is still where it was.
    pub remaining: Vec<PathBuf>,
}

impl Failed {
    fn nothing_moved(reason: String, plan: &Plan) -> Self {
        Self {
            reason,
            moved: Vec::new(),
            remaining: plan.items.clone(),
        }
    }
}

/// Everything a move needs to be safe, beyond the plan: where the app keeps its state, and what is
/// configured now.
pub struct Context<'a> {
    /// The app's state directory; the history goes under [`REMOVED_PAIRS_DIR`] in it.
    pub state_dir: &'a Path,
    /// The pairs the config declares **now** (after the removal): their folders are off limits as a
    /// destination, and their own state is not this pair's to move.
    pub configured: &'a [PairView],
    pub now: SystemTime,
}

/// Set the plan's items aside (see the module doc for the order of every step). Needs the pair's
/// lockfile free; refuses a destination under any configured folder or under the pair's own; refuses
/// an item that is a configured pair's state (the folder was added back).
pub fn execute(plan: &Plan, context: &Context<'_>) -> Result<Done, Failed> {
    let rename = |from: &Path, to: &Path| fs::rename(from, to);
    let same = |a: &Path, b: &Path| same_tree(a, b);
    execute_with(
        plan,
        context,
        &Mover {
            rename: &rename,
            same: &same,
        },
    )
}

pub fn execute_with(plan: &Plan, context: &Context<'_>, mover: &Mover<'_>) -> Result<Done, Failed> {
    if plan.items.is_empty() {
        return Ok(Done {
            to: PathBuf::new(),
            moved: Vec::new(),
        });
    }
    let fail = |reason: String| Failed::nothing_moved(reason, plan);
    for view in context.configured {
        if let Some(item) = plan.items.iter().find(|item| belongs_to(item, view)) {
            return Err(fail(format!(
                "{} is the state of folder pair '{}', which is configured: it is that pair's \
                 history now, not a removed one's",
                item.display(),
                view.name
            )));
        }
    }
    let mut roots: Vec<PathBuf> = context
        .configured
        .iter()
        .filter_map(|view| view.local_root.clone())
        .collect();
    roots.push(plan.local_root.clone());
    let removed_pairs = context.state_dir.join(REMOVED_PAIRS_DIR);
    outside_every_root(&removed_pairs, &roots).map_err(fail)?;

    // The proof that nobody has it, held until the move is done.
    let _held = match acquire(plan.lockfile.as_deref()) {
        LockProbe::Free(guard) => guard,
        LockProbe::Held => {
            return Err(fail(format!(
                "a sync daemon still holds {}",
                plan.lockfile.as_deref().map_or_else(
                    || "this pair's state".to_owned(),
                    |l| l.display().to_string()
                )
            )));
        }
        LockProbe::Unknown(why) => {
            return Err(fail(format!(
                "could not tell whether a daemon holds the pair: {why}"
            )));
        }
    };

    let destination =
        fresh_directory(&removed_pairs, &plan.pair, context.now).map_err(|error| {
            fail(format!(
                "could not make {}: {error}",
                removed_pairs.display()
            ))
        })?;
    let mut moved: Vec<MovedItem> = Vec::new();
    let mut remaining: Vec<PathBuf> = plan.items.clone();
    let mut failure = None;
    for (number, item) in plan.items.iter().enumerate() {
        let name = item
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stored = destination.join(format!("{number:02}-{name}"));
        match move_one(item, &stored, mover) {
            Ok(how) => {
                moved.push(MovedItem {
                    from: item.clone(),
                    to: stored,
                    how,
                });
                remaining.retain(|left| left != item);
            }
            Err(reason) => {
                failure = Some(reason);
                break;
            }
        }
    }
    // A record of where each piece came from, written whether or not everything moved — but not into
    // a directory that holds nothing, which is removed below.
    if !moved.is_empty() {
        let _ = write_manifest(&destination, plan, &moved, context.now);
    }
    match failure {
        None => Ok(Done {
            to: destination,
            moved,
        }),
        Some(reason) => {
            if moved.is_empty() {
                // An empty directory of ours: nothing was put in it, so nothing is lost by removing it.
                let _ = fs::remove_dir(&destination);
            }
            Err(Failed {
                reason,
                moved,
                remaining,
            })
        }
    }
}

/// A new, empty, owner-only directory `<removed_pairs>/<name>-<stamp>` (with `-2`, `-3` … if two
/// removals share a second). The parents are made owner-only as well: an index lists every file name
/// the person synced.
fn fresh_directory(removed_pairs: &Path, pair: &str, now: SystemTime) -> io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(removed_pairs)?;
    let base = format!("{}-{}", directory_part(pair), utc_stamp(now));
    for attempt in 1..=1000 {
        let candidate = if attempt == 1 {
            removed_pairs.join(&base)
        } else {
            removed_pairs.join(format!("{base}-{attempt}"))
        };
        let mut one = fs::DirBuilder::new();
        one.mode(0o700);
        match one.create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "a thousand set-asides in one second",
    ))
}

#[derive(Serialize)]
struct Manifest<'a> {
    pair: &'a str,
    local_root: &'a Path,
    removed_at: String,
    note: &'static str,
    items: &'a [MovedItem],
}

fn write_manifest(
    destination: &Path,
    plan: &Plan,
    moved: &[MovedItem],
    now: SystemTime,
) -> io::Result<()> {
    let manifest = Manifest {
        pair: &plan.pair,
        local_root: &plan.local_root,
        removed_at: utc_stamp(now),
        note: "The sync history of a folder pair removed in Proton Drive Sync. Nothing here is \
               needed to sync; deleting this directory forgets that history for good.",
        items: moved,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
    fs::write(destination.join(MANIFEST_NAME), bytes)
}

// ---- a move that could not happen yet -----------------------------------------------------------

/// A set-aside that was due and could not happen: kept so that it still can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub plan: Plan,
    /// Why it could not happen when it was recorded.
    pub reason: String,
    pub recorded_at: String,
}

/// Write the record of a move that has not happened (atomically, owner-only). Its path is returned.
pub fn record_pending(
    state_dir: &Path,
    plan: &Plan,
    reason: &str,
    now: SystemTime,
) -> io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let directory = state_dir.join(PENDING_DIR);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let record = Pending {
        plan: Plan {
            items: plan.items.clone(),
            ..plan.clone()
        },
        reason: reason.to_owned(),
        recorded_at: utc_stamp(now),
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(io::Error::other)?;
    let stem = format!("{}-{}", directory_part(&plan.pair), utc_stamp(now));
    for attempt in 1..=1000 {
        let name = if attempt == 1 {
            format!("{stem}.json")
        } else {
            format!("{stem}-{attempt}.json")
        };
        let path = directory.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many pending records",
    ))
}

/// Every record of a pending set-aside, oldest first. A record that cannot be read is skipped, not
/// guessed at.
pub fn pending(state_dir: &Path) -> Vec<(PathBuf, Pending)> {
    let Ok(entries) = fs::read_dir(state_dir.join(PENDING_DIR)) else {
        return Vec::new();
    };
    let mut records: Vec<(PathBuf, Pending)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            let record = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
            Some((path, record))
        })
        .collect();
    records.sort_by(|a, b| a.0.cmp(&b.0));
    records
}

/// What retrying one pending set-aside came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// It happened.
    Moved { pair: String, done: Done },
    /// The folder is a configured pair again: that history is its own now. The record is dropped and
    /// nothing was moved.
    Superseded { pair: String },
    /// Still cannot happen; the record stays.
    StillPending { pair: String, reason: String },
}

/// Retry every pending set-aside, against the pairs configured **now**. Safe to call at any time and
/// as often as a caller likes: each one re-checks what [`execute`] checks.
pub fn settle_pending(context: &Context<'_>) -> Vec<Settled> {
    let mut results = Vec::new();
    for (path, record) in pending(context.state_dir) {
        let pair = record.plan.pair.clone();
        // Re-plan from what is on disk: the items may have gone, or grown, since it was recorded.
        let still_there: Vec<PathBuf> = record
            .plan
            .items
            .iter()
            .filter(|item| exists_without_following(item))
            .cloned()
            .collect();
        let plan = Plan {
            items: still_there,
            ..record.plan.clone()
        };
        let readded = context.configured.iter().any(|view| {
            view.local_root.as_deref().is_some_and(|root| {
                canonicalize_best_effort(root) == canonicalize_best_effort(&plan.local_root)
            })
        });
        if readded {
            let _ = fs::remove_file(&path);
            results.push(Settled::Superseded { pair });
            continue;
        }
        match execute(&plan, context) {
            Ok(done) => {
                let _ = fs::remove_file(&path);
                results.push(Settled::Moved { pair, done });
            }
            Err(failed) => results.push(Settled::StillPending {
                pair,
                reason: failed.reason,
            }),
        }
    }
    results
}

#[cfg(test)]
mod tests;
