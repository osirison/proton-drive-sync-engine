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
//! **The three answers of looking, and why the third exists.** [`plan`] answers [`Planned::Move`],
//! [`Planned::Nothing`] or [`Planned::Undetermined`]. "Nothing" is a claim that every place the pair's
//! state could be was looked at and held none, so it needs a folder that can be read: an unplugged
//! drive, an unreadable folder and a path that is relative (which the daemon resolves against *its*
//! working directory, not this app's) are all "could not look", and none of them is "no history". A
//! pair that could not be looked at is reported pending with the reason, never as having nothing.
//!
//! **Where it goes.** Outside every sync root — the destination is checked against the real path of
//! every folder the config names, because history inside a sync folder is history that gets uploaded.
//!
//! **When it moves.** Only when no daemon holds it: the caller restarts the daemon off the pair first,
//! and [`acquire`] then takes the pair's own lockfile non-blockingly, which is the one proof of "nobody
//! has this open" that does not depend on a socket answering. A move that cannot happen now is a
//! [`Pending`] record that says why, retried by [`settle_pending`].
//!
//! **How it moves.** `rename`; on `EXDEV` a copy that is made durable and then compared byte for byte
//! with its source before the source is touched. Nothing is deleted until the copy is verified, and a
//! copy that fails verification is removed and the original left exactly as it was.
//!
//! **What a record is trusted for.** A [`Pending`] record names the pair's *identity* — its folder and
//! the paths its [`PairView`] gave — and [`settle_pending`] re-plans from that identity on the disk as
//! it is then; the items listed in the record are only checked against it, and a record that names
//! anything that is not that pair's state is refused. That bounds what a hand-edited or planted record
//! can move to the state of a pair it describes. It cannot do more: the identity itself is the record's
//! word, and the record lives in the app's own owner-only state directory, so writing one takes the
//! access that already reaches every file the app could move.

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

// ---- paths in the bookkeeping --------------------------------------------------------------------

/// A path as JSON that reads back as **the same path**.
///
/// `serde_json` refuses a `PathBuf` that is not UTF-8, and a lossy rendering is no use here: a pending
/// record is acted on later, so it must name the folder that is on disk and not one that looks like
/// it. A UTF-8 path is the plain string it always was (so a manifest or a record stays readable); one
/// that is not is `{"hex": <its bytes>, "lossy": <how to display it>}`, and only `hex` is read back.
mod exact_path {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Text(String),
        Bytes {
            hex: String,
            #[serde(default)]
            lossy: String,
        },
    }

    fn to_wire(path: &Path) -> Wire {
        match path.to_str() {
            Some(text) => Wire::Text(text.to_owned()),
            None => Wire::Bytes {
                hex: path
                    .as_os_str()
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                lossy: path.to_string_lossy().into_owned(),
            },
        }
    }

    fn from_wire(wire: Wire) -> Result<PathBuf, String> {
        match wire {
            Wire::Text(text) => Ok(PathBuf::from(text)),
            Wire::Bytes { hex, .. } => {
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                for pair in hex.as_bytes().chunks(2) {
                    let digits = std::str::from_utf8(pair)
                        .ok()
                        .filter(|digits| digits.len() == 2)
                        .and_then(|digits| u8::from_str_radix(digits, 16).ok());
                    bytes.push(digits.ok_or_else(|| format!("`{hex}` is not a hex string"))?);
                }
                Ok(PathBuf::from(OsString::from_vec(bytes)))
            }
        }
    }

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        to_wire(path).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        from_wire(Wire::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }

    /// `Option<PathBuf>`.
    pub mod option {
        use super::{Wire, from_wire, to_wire};
        use serde::{Deserialize, Deserializer, Serialize, Serializer};
        use std::path::PathBuf;

        pub fn serialize<S: Serializer>(
            path: &Option<PathBuf>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            path.as_deref().map(to_wire).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<PathBuf>, D::Error> {
            Option::<Wire>::deserialize(deserializer)?
                .map(from_wire)
                .transpose()
                .map_err(serde::de::Error::custom)
        }
    }

    /// `Vec<PathBuf>`.
    pub mod list {
        use super::{Wire, from_wire, to_wire};
        use serde::{Deserialize, Deserializer, Serializer};
        use std::path::PathBuf;

        pub fn serialize<S: Serializer>(
            paths: &[PathBuf],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(paths.iter().map(|path| to_wire(path)))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<PathBuf>, D::Error> {
            Vec::<Wire>::deserialize(deserializer)?
                .into_iter()
                .map(from_wire)
                .collect::<Result<_, _>>()
                .map_err(serde::de::Error::custom)
        }
    }
}

// ---- what would be moved -------------------------------------------------------------------------

/// What would be moved for one pair, found by looking at the disk now — and who the pair is, which is
/// what a record of it keeps (see the module doc: [`settle_pending`] re-plans from the identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub pair: String,
    /// The pair's folder as the config gave it (empty when it gave none).
    #[serde(with = "exact_path")]
    pub local_root: PathBuf,
    /// The index the config places, when it places one.
    #[serde(default, with = "exact_path::option")]
    pub db_path: Option<PathBuf>,
    /// The pair's own lockfile: what proves nobody holds the state.
    #[serde(default, with = "exact_path::option")]
    pub lockfile: Option<PathBuf>,
    /// The `.sync` directory when there is one, and every named state file outside it that exists.
    #[serde(default, with = "exact_path::list")]
    pub items: Vec<PathBuf>,
    /// What planning saw and left alone (a `.sync` that is a file, a state file that is a folder).
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Plan {
    /// A plan carrying `view`'s identity and the given findings.
    pub fn of_view(view: &PairView, items: Vec<PathBuf>, notes: Vec<String>) -> Self {
        Self {
            pair: view.name.clone(),
            local_root: view.local_root.clone().unwrap_or_default(),
            db_path: view.db_path.clone(),
            lockfile: view.lockfile_path.clone(),
            items,
            notes,
        }
    }

    /// The pair this plan is about, as the config described it.
    fn view(&self) -> PairView {
        PairView {
            name: self.pair.clone(),
            local_root: (!self.local_root.as_os_str().is_empty()).then(|| self.local_root.clone()),
            remote_root: None,
            db_path: self.db_path.clone(),
            lockfile_path: self.lockfile.clone(),
            conflict_suffix: None,
        }
    }
}

/// The disk could not say whether there is history to move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undetermined {
    /// Why, in a sentence a person can act on.
    pub reason: String,
    /// Whether looking again later could answer it (a drive that comes back), and so whether a record
    /// of the move is worth keeping. A path the app can never locate safely is not.
    pub retry: bool,
}

/// What looking at a pair's state found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Planned {
    /// There is state to move.
    Move(Plan),
    /// Every place the pair's state could be was readable and held none.
    Nothing { notes: Vec<String> },
    /// The disk could not say. **Not** "nothing": the history may be right there.
    Undetermined(Undetermined),
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

/// Every path that is this pair's state if it exists: its `.sync` and the files its view names. The
/// whole of what a move, or a record asking for one, may touch.
fn state_candidates(view: &PairView) -> Vec<PathBuf> {
    let mut all = Vec::new();
    if let Some(root) = &view.local_root {
        all.push(sync_state_dir(root));
    }
    all.extend(named_state_files(view));
    all
}

fn undetermined(reason: String, retry: bool) -> Planned {
    Planned::Undetermined(Undetermined { reason, retry })
}

/// What setting this pair's history aside would move. Pure reading of the disk; moves nothing.
///
/// **"Nothing" needs a folder that can be read.** `NotFound` for a path under a folder that was
/// opened is an absence (for a state file outside the pair's folder, the folder it sits in must be
/// listable too); every other answer — the folder itself missing (its drive may be unplugged),
/// unreadable, not a folder, or any error looking at a state path — is [`Planned::Undetermined`].
/// A relative path is refused before the disk is touched at all: the daemon resolves one against its
/// own working directory, which is not this app's, and looking relative to ours would find (and move)
/// some other folder's state.
///
/// **The limit of "nothing".** A drive that is not mounted but whose mount point is still an empty
/// folder reads exactly like a folder with no history: both can be listed and neither holds a `.sync`.
/// Nothing in the folder tells them apart, so this does not try; the reply says what was looked at.
///
/// The `.sync` directory is taken **whole** (a symlink there is moved as the link and never
/// followed). A `.sync` that is a plain file is not the engine's state and is left alone, with a note.
/// A named state file inside `.sync` is covered by that; one outside it is listed on its own, and
/// only when it is a file — a directory named as a state file is a config the engine accepts, and is
/// not this module's to move.
pub fn plan(view: &PairView) -> Planned {
    let Some(root) = view.local_root.as_deref() else {
        return undetermined(
            "the config gives this pair no local_root, so the app cannot tell where its history is"
                .to_owned(),
            false,
        );
    };
    for (key, path) in [
        ("local_root", Some(root)),
        ("db_path", view.db_path.as_deref()),
        ("lockfile_path", view.lockfile_path.as_deref()),
    ] {
        if let Some(path) = path.filter(|path| !path.is_absolute()) {
            return undetermined(
                format!(
                    "the config's {key} `{}` is a relative path. The daemon resolves a relative path \
                     against its own working directory, which this app cannot know, so it cannot \
                     locate the history safely and moved nothing",
                    path.display()
                ),
                false,
            );
        }
    }
    // The root has to be a folder that can be listed before anything under it can be called absent.
    if let Err(error) = fs::read_dir(root) {
        return undetermined(
            format!(
                "the folder {} cannot be read ({error}); a drive that is unplugged or not mounted \
                 looks the same, so the app cannot tell whether its history is there and moved \
                 nothing",
                root.display()
            ),
            true,
        );
    }

    let mut items = Vec::new();
    let mut notes = Vec::new();
    let state = sync_state_dir(root);
    let mut state_is_not_a_directory = false;
    match fs::symlink_metadata(&state) {
        Ok(meta) if meta.is_dir() || meta.file_type().is_symlink() => items.push(state.clone()),
        Ok(_) => {
            state_is_not_a_directory = true;
            notes.push(format!(
                "{} is a file, not a directory, so it is not the sync engine's state and was left \
                 where it is",
                state.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return undetermined(
                format!("could not look at {}: {error}", state.display()),
                true,
            );
        }
    }
    for file in named_state_files(view) {
        // Inside the state directory it is covered by moving that — or cannot exist, when `.sync` is
        // not a directory. And never the folder itself, or something it sits inside.
        if file.starts_with(&state) || root.starts_with(&file) || items.contains(&file) {
            continue;
        }
        match fs::symlink_metadata(&file) {
            Ok(meta) if meta.is_dir() => notes.push(format!(
                "{} is a folder, not a state file, so it was left where it is",
                file.display()
            )),
            Ok(_) => items.push(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Absent only if the folder it would be in can itself be listed: a state file on a
                // drive that is not mounted has no parent to list, and that is not an absence.
                if let Some(parent) = file.parent()
                    && let Err(parent_error) = fs::read_dir(parent)
                {
                    return undetermined(
                        format!(
                            "the folder {} cannot be read ({parent_error}), so the app cannot \
                             tell whether {} is there; a drive that is unplugged or not mounted \
                             looks the same, and nothing was moved",
                            parent.display(),
                            file.display()
                        ),
                        true,
                    );
                }
            }
            Err(error) => {
                return undetermined(
                    format!("could not look at {}: {error}", file.display()),
                    true,
                );
            }
        }
    }
    if items.is_empty() {
        return Planned::Nothing { notes };
    }
    let mut plan = Plan::of_view(view, items, notes);
    // A lockfile under a `.sync` that is a file cannot exist, so there is nothing to ask.
    plan.lockfile = plan
        .lockfile
        .filter(|lock| !(state_is_not_a_directory && lock.starts_with(&state)));
    Planned::Move(plan)
}

/// The index file this pair would resume if it were added back, when one is on disk. The add flow
/// names it (brief A9, E18): **not** a thing the app can reset — only `proton-sync reset-index` can.
pub fn surviving_index(view: &PairView) -> Option<PathBuf> {
    let db = view.db_path.as_ref().filter(|db| db.is_absolute())?;
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
    state_candidates(view).iter().any(|file| {
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
    /// Copied, made durable, compared with its source, and only then removed from where it was.
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedItem {
    #[serde(with = "exact_path")]
    pub from: PathBuf,
    #[serde(with = "exact_path")]
    pub to: PathBuf,
    pub how: How,
    /// Set when the item was a symbolic link: **only the link moved**. What it points at is where it
    /// always was, untouched — this app never follows a link.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "exact_path::option"
    )]
    pub link_target: Option<PathBuf>,
}

/// The operations that touch the disk, as values, so a test can make the filesystem boundary appear
/// (`EXDEV`), a copy go wrong, and the order of the steps be seen, without two filesystems or a
/// broken disk.
pub struct Mover<'a> {
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    pub same: &'a dyn Fn(&Path, &Path) -> io::Result<bool>,
    /// Make a file or a directory durable (`fsync`). Called for everything a copy wrote, before the
    /// copy is compared and before the original is removed.
    pub sync: &'a dyn Fn(&Path) -> io::Result<()>,
}

/// `fsync` a file or a directory: opened read-only, which is how a directory is synced.
pub fn sync_path(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn crosses_devices(error: &io::Error) -> bool {
    // `ErrorKind::CrossesDevices` is the typed spelling; the raw errno is `EXDEV` (18 on Linux), kept
    // because a platform that maps it to `Uncategorized` would otherwise read as a plain failure.
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(18)
}

/// Copy `from` to `to`, calling `sync` on every file and directory written once its contents are in
/// place. A link is recreated as a link (it has no contents to make durable; its directory is synced).
fn copy_entry(from: &Path, to: &Path, sync: &dyn Fn(&Path) -> io::Result<()>) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)
    } else if kind.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_entry(&entry.path(), &to.join(entry.file_name()), sync)?;
        }
        // After the contents: a read-only directory would refuse them.
        fs::set_permissions(to, meta.permissions())?;
        sync(to)
    } else if kind.is_file() {
        fs::copy(from, to)?;
        sync(to)
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

/// Move one item. Rename if the filesystem allows; otherwise copy, make the copy durable, compare, and
/// only then remove.
///
/// **The order is the safety.** The original is removed after — and only after — `sync` has put the
/// copy on the disk and `same` has said the copy is it: a power cut between the copy and the removal
/// must leave the original, never an original gone and a copy that was only in memory. A copy that
/// cannot be made, made durable or does not compare equal is removed (it is ours: it is under a
/// directory this move just created) and the original is left untouched.
fn move_one(from: &Path, to: &Path, mover: &Mover<'_>) -> Result<How, String> {
    match (mover.rename)(from, to) {
        Ok(()) => return Ok(How::Rename),
        Err(error) if crosses_devices(&error) => {}
        Err(error) => return Err(format!("could not move {}: {error}", from.display())),
    }
    let copied = copy_entry(from, to, mover.sync).and_then(|()| {
        // The new name, too: the directory entry that makes the copy findable.
        to.parent().map_or(Ok(()), |parent| (mover.sync)(parent))
    });
    if let Err(error) = copied {
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
    #[serde(with = "exact_path")]
    pub to: PathBuf,
    pub moved: Vec<MovedItem>,
    /// What planning saw and left alone.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Why a set-aside did not (finish) happen, and what is left to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub reason: String,
    /// What did get moved before the failure (already in its place; never moved back).
    pub moved: Vec<MovedItem>,
    /// The directory `moved` went to, when anything did.
    pub to: Option<PathBuf>,
    /// What is still where it was.
    pub remaining: Vec<PathBuf>,
}

impl Failed {
    fn nothing_moved(reason: String, plan: &Plan) -> Self {
        Self {
            reason,
            moved: Vec::new(),
            to: None,
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
            sync: &sync_path,
        },
    )
}

pub fn execute_with(plan: &Plan, context: &Context<'_>, mover: &Mover<'_>) -> Result<Done, Failed> {
    if plan.items.is_empty() {
        return Ok(Done {
            to: PathBuf::new(),
            moved: Vec::new(),
            notes: plan.notes.clone(),
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
        // Before the move, while the link is still where it was.
        let link_target = fs::symlink_metadata(item)
            .ok()
            .filter(|meta| meta.file_type().is_symlink())
            .and_then(|_| fs::read_link(item).ok());
        match move_one(item, &stored, mover) {
            Ok(how) => {
                moved.push(MovedItem {
                    from: item.clone(),
                    to: stored,
                    how,
                    link_target,
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
            notes: plan.notes.clone(),
        }),
        Some(reason) => {
            if moved.is_empty() {
                // An empty directory of ours: nothing was put in it, so nothing is lost by removing it.
                let _ = fs::remove_dir(&destination);
            }
            let to = (!moved.is_empty()).then_some(destination);
            Err(Failed {
                reason,
                moved,
                to,
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
    #[serde(with = "exact_path")]
    local_root: &'a Path,
    removed_at: String,
    note: &'static str,
    items: &'a [MovedItem],
    notes: &'a [String],
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
        notes: &plan.notes,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
    fs::write(destination.join(MANIFEST_NAME), bytes)
}

// ---- a move that could not happen yet -----------------------------------------------------------

/// A set-aside that was due and could not happen: kept so that it still can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// Who the pair was (its folder and state paths) and what was found to move when this was
    /// recorded. The identity is what a retry plans from; the items are only checked against it.
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
        plan: plan.clone(),
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
    /// Looked at again, the pair has no state left to move (it was moved or deleted by hand). The
    /// record is dropped.
    NothingLeft {
        pair: String,
        notes: Vec<String>,
        /// Whether the note listed items to move. A note filed because the folder could not be read
        /// lists none.
        had_items: bool,
    },
    /// Still cannot happen; the record stays.
    StillPending { pair: String, reason: String },
}

/// Retry every pending set-aside, against the pairs configured **now**. Safe to call at any time and
/// as often as a caller likes: each one re-checks what [`execute`] checks.
///
/// **A record is not the plan.** Each is re-derived from the pair's identity on the disk as it is now
/// (so a `.sync` that has been replaced, or a drive that came back, is seen as it is), and a record
/// whose items are not all that pair's state — its `.sync`, or a file its paths name — is refused and
/// left where it is, with the reason, instead of moving what it names.
pub fn settle_pending(context: &Context<'_>) -> Vec<Settled> {
    let mut results = Vec::new();
    for (path, record) in pending(context.state_dir) {
        let pair = record.plan.pair.clone();
        // By real path: `~/Sync`, `/home/me/Sync` and a link to it are one folder.
        let readded = !record.plan.local_root.as_os_str().is_empty()
            && context.configured.iter().any(|view| {
                view.local_root.as_deref().is_some_and(|root| {
                    canonicalize_best_effort(root)
                        == canonicalize_best_effort(&record.plan.local_root)
                })
            });
        if readded {
            let _ = fs::remove_file(&path);
            results.push(Settled::Superseded { pair });
            continue;
        }
        let view = record.plan.view();
        let own_state = state_candidates(&view);
        if let Some(stranger) = record
            .plan
            .items
            .iter()
            .find(|item| !own_state.contains(item))
        {
            results.push(Settled::StillPending {
                reason: format!(
                    "the note {} names {}, which is not part of the sync state of folder pair \
                     '{pair}' (its `.sync` folder, or a file its index and lock paths name), so \
                     nothing was moved. If you did not write that note, delete it",
                    path.display(),
                    stranger.display()
                ),
                pair,
            });
            continue;
        }
        match plan(&view) {
            Planned::Move(plan) => match execute(&plan, context) {
                Ok(done) => {
                    let _ = fs::remove_file(&path);
                    results.push(Settled::Moved { pair, done });
                }
                Err(failed) => results.push(Settled::StillPending {
                    pair,
                    reason: failed.reason,
                }),
            },
            Planned::Nothing { notes } => {
                let _ = fs::remove_file(&path);
                results.push(Settled::NothingLeft {
                    pair,
                    notes,
                    had_items: !record.plan.items.is_empty(),
                });
            }
            Planned::Undetermined(undetermined) => results.push(Settled::StillPending {
                pair,
                reason: undetermined.reason,
            }),
        }
    }
    results
}

#[cfg(test)]
mod tests;
