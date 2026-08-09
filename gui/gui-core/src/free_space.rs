//! How much room is left where the sync folder lives (C4, #177).
//!
//! `9a Review` states free space on the **download** side only — `Needs 38.4 GB free. You have
//! 214 GB.` — because that is the one thing about the first sync that can actually fail. Uploading
//! cannot run out of local disk; bringing 12,000 files down can.
//!
//! # Only half this sentence is Phase 1, and it is not the half the fallback expects
//!
//! `14-behaviour-and-state.md` gives the fallback as *omit the "You have 214 GB" clause* — it
//! assumes the **needs** half is always known and the **have** half is what might fail. The engine
//! is the other way round: [`PlannedAction`] carries `path`, `destination_path`, `action`,
//! `entity_kind`, `conflict_path` and `remote_id`, and **no size at any level of the dry-run
//! surface**, so nothing can total the bytes a download plan would fetch. That is issue #206 (G6).
//!
//! So this module supplies the half that exists, exactly and always, and the screen states the
//! clause it can stand behind. Recorded in `DEVIATIONS.md` rather than papered over with an
//! estimate: a wrong "Needs 38.4 GB" on the screen whose entire job is to promise nothing gets
//! deleted would be the worst possible place to guess.
//!
//! [`PlannedAction`]: crate::wire::PlannedAction

use std::path::{Path, PathBuf};

/// Room on the filesystem holding a path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FreeSpace {
    /// Bytes available **to this user** — `f_bavail`, not `f_bfree`. The difference is the
    /// reserved-for-root pool, which a desktop app can see and can never write into, and quoting it
    /// would promise space that a download then fails to use.
    pub available: u64,
    /// The filesystem's total size, for context ("214 GB of 500 GB").
    pub total: u64,
    /// The directory actually measured. When the sync folder does not exist yet — the normal case
    /// during onboarding, where step 1 proposes `~/ProtonDrive` — this is the nearest existing
    /// ancestor, which is the filesystem the folder will be created on.
    pub measured_at: PathBuf,
}

/// Measure the filesystem a path lives on, or will live on.
///
/// Walking up to the nearest existing ancestor is the point of this function rather than a
/// nicety: onboarding asks about free space *before* the folder is created, so a plain `statvfs` on
/// the chosen path would return `ENOENT` on the one screen that needs the number.
#[cfg(unix)]
pub fn for_path(path: &Path) -> Result<FreeSpace, String> {
    let measured_at = nearest_existing(path)
        .ok_or_else(|| format!("no existing directory above {}", path.display()))?;
    let stats = rustix::fs::statvfs(&measured_at)
        .map_err(|e| format!("statvfs {}: {e}", measured_at.display()))?;

    // `f_frsize` is the fragment size the block counts are in. Zero would be a nonsense filesystem;
    // multiplying by it anyway would silently report zero bytes free and make the review screen
    // claim there is no room.
    let block = stats.f_frsize;
    if block == 0 {
        return Err(format!(
            "{} reports a zero block size",
            measured_at.display()
        ));
    }
    Ok(FreeSpace {
        available: stats.f_bavail.saturating_mul(block),
        total: stats.f_blocks.saturating_mul(block),
        measured_at,
    })
}

/// Off unix there is no `statvfs`, and this crate keeps a stub for every unix-only path so it still
/// type-checks (CI is Linux-only, so nothing else would catch the omission). `Err` is the honest
/// answer: the screen's fallback is to omit the clause, which is exactly what an error produces.
#[cfg(not(unix))]
pub fn for_path(path: &Path) -> Result<FreeSpace, String> {
    Err(format!(
        "free space for {} is not available on this platform",
        path.display()
    ))
}

/// The nearest ancestor of `path` (including `path`) that exists and is a directory.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    let mut candidate = path;
    loop {
        if candidate.is_dir() {
            return Some(candidate.to_path_buf());
        }
        candidate = candidate.parent()?;
        // `parent()` of a relative single-component path is `""`, which `is_dir()` answers `false`
        // for even though the current directory exists. Stop there rather than looping.
        if candidate.as_os_str().is_empty() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_directory_reports_its_own_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let space = for_path(dir.path()).expect("statvfs on a temp dir");
        assert_eq!(space.measured_at, dir.path());
        assert!(space.total > 0, "a mounted filesystem has a size");
        assert!(
            space.available <= space.total,
            "available {} exceeds total {}",
            space.available,
            space.total
        );
    }

    #[test]
    fn a_folder_that_does_not_exist_yet_measures_where_it_would_go() {
        // The onboarding case: `~/ProtonDrive` is a proposal, not a directory.
        let dir = tempfile::tempdir().unwrap();
        let proposed = dir.path().join("ProtonDrive/nested/deeper");
        let space = for_path(&proposed).expect("nearest existing ancestor");
        assert_eq!(space.measured_at, dir.path());
        assert!(space.total > 0);
    }

    #[test]
    fn a_file_standing_where_the_folder_should_be_measures_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ProtonDrive");
        std::fs::write(&file, b"not a directory").unwrap();
        let space = for_path(&file).expect("walks up past the file");
        assert_eq!(space.measured_at, dir.path());
    }

    #[test]
    fn a_relative_path_with_no_existing_ancestor_is_an_error_not_a_zero() {
        assert!(for_path(Path::new("nowhere-at-all-xyzzy")).is_err());
    }

    #[test]
    fn the_root_filesystem_always_answers() {
        let space = for_path(Path::new("/")).expect("/ is always mounted");
        assert_eq!(space.measured_at, PathBuf::from("/"));
        assert!(space.total > 0);
    }
}
