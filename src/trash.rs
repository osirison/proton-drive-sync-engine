//! How a local entity goes away when the engine mirrors a remote deletion.
//!
//! ONE FUNCTION, ONE DECISION. [`dispose`] is the only place in the engine that removes a local
//! file or directory as a *sync side effect*, so "what happens when a deletion applies" has exactly
//! one answer and cannot drift between the executor, the wire and the UI. (`fs::remove_file`
//! appears elsewhere for scratch files, sockets and lockfiles — none of those is a user's content.)
//!
//! THE DEFAULT IS RECOVERABLE. Before this module a `LocalDelete` unlinked the file: no trash, no
//! undo, and no copy left on Proton to pull back. Every warning on the Deletions screen — the
//! `Permanent · this computer` column, the typed-`DELETE` gate, the interrupting banner — existed to
//! compensate for that one call. [`LocalDeleteMode::Trash`] removes the loss those warnings defend
//! against, which is what lets them come down; [`LocalDeleteMode::Permanent`] restores the old
//! behaviour *and* every warning with it. The friction follows the consequence, not the direction.
//!
//! A FAILED TRASH MOVE IS NEVER A REMOVAL. The `Trash` arm returns the error rather than falling
//! back to unlinking. The caller is `execute_plan_and_commit`, whose `Err` path is the #136 partial
//! pass: the entity stays on disk, its baseline row is not purged, the event cursor is held, the
//! path is re-queued and the deletion is planned again next pass. A fallback would make the trash a
//! courtesy and the permanent removal the real behaviour, which inverts the point of the module.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::index::EntityKind;
use crate::{AppResult, boxed_error};

/// What a folder pair does with a local entity when a deletion applies to it.
///
/// A **spelling of a consequence**, not of a mechanism the user should have to reason about: the
/// question a person answers on the Deletions tab is "can I get it back". Modelled on
/// [`crate::config::DeletionPolicy`] — the other named-enum config key — down to the `as_str` /
/// [`FromStr`] / [`Self::ALL`] trio that keeps the TOML spelling, the error message listing the
/// accepted values, and the serde rename in step.
///
/// Distinct from [`crate::ipc::LocalDisposal`], which is the *wire* form. This one names the
/// mechanism a user chooses; that one names the consequence a client draws, and a `RemoteDelete`
/// has a disposal but no mode. Do not unify them: `"trash"` never crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalDeleteMode {
    /// Move the entity to the desktop trash, where the user's file manager can restore it. The
    /// default, and what an empty config means.
    #[default]
    Trash,
    /// Remove the entity from disk. Unrecoverable, and the behaviour of every build before this
    /// module existed.
    Permanent,
}

impl LocalDeleteMode {
    /// The TOML/CLI spelling. Kept in step with the serde rename by
    /// `every_mode_spelling_round_trips_through_serde_and_from_str`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trash => "trash",
            Self::Permanent => "permanent",
        }
    }

    /// Whether a deletion applied under this mode can be undone by the user. The one definition
    /// behind [`crate::ipc::LocalDisposal`], so the wire cannot say "recoverable" about a mode that
    /// unlinks.
    pub fn is_recoverable(self) -> bool {
        matches!(self, Self::Trash)
    }

    /// Every mode, for exhaustive tests and for naming the choices in an error message.
    pub const ALL: [Self; 2] = [Self::Trash, Self::Permanent];
}

impl fmt::Display for LocalDeleteMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LocalDeleteMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .ok_or_else(|| {
                format!(
                    "unknown local delete mode {value:?} (expected one of: {})",
                    Self::ALL
                        .into_iter()
                        .map(Self::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

/// Remove `path` the way `mode` says to.
///
/// `path` must already be absolute and inside the pair's local root — the caller clears
/// `safe_local_path` first, and this function does not re-derive that. `kind` is only read by the
/// `Permanent` arm, which needs to pick between the two `std::fs` calls; the trash move is one
/// operation for a file and for a whole directory alike.
///
/// A missing `path` is **not** an error in either mode: the caller's `exists()` check already
/// short-circuits it, and a deletion that has nothing left to delete has succeeded.
pub fn dispose(mode: LocalDeleteMode, path: &Path, kind: EntityKind) -> AppResult<()> {
    #[cfg(test)]
    match test_hook::intercept(mode, path, kind) {
        Some(result) => return result,
        // NOT A LINT — a guardrail that cannot be forgotten. Without it a lib test that flips a
        // pair to `Trash` and omits the hook would move its temp files into the developer's real
        // `~/.local/share/Trash`, silently and on every `cargo test`. Integration tests link the
        // library compiled WITHOUT `cfg(test)`, so `tests/trash_disposal.rs` still exercises the
        // real crate; this arm binds the lib test binary only.
        None if mode == LocalDeleteMode::Trash => panic!(
            "a lib test disposed of {} in Trash mode with no hook installed: call \
             `trash::test_hook::install_fake_trash(...)` first, or the real desktop trash is \
             what the test would write into",
            path.display()
        ),
        None => {}
    }
    match mode {
        LocalDeleteMode::Permanent => {
            if kind == EntityKind::Directory {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
            Ok(())
        }
        // NAMES THE PATH. `trash::Error`'s own `Display` is its `Debug` form and mentions no
        // target, which would reach a user as a `failed_items` row that does not say what failed.
        LocalDeleteMode::Trash => ::trash::delete(path).map_err(|error| {
            boxed_error(format!(
                "could not move {} to the trash: {error}",
                path.display()
            ))
        }),
    }
}

/// The seam daemon tests dispose through, so a `cargo test` run never writes into the developer's
/// real `~/.local/share/Trash`.
///
/// WHY A HOOK AND NOT AN ENVIRONMENT REDIRECT. The `trash` crate resolves the home trash by reading
/// `XDG_DATA_HOME` on every call, so redirecting it means `std::env::set_var` — `unsafe` under
/// edition 2024 because `setenv` races *any* concurrent `getenv`, not merely one on the same key.
/// The lib test binary runs thousands of tests across threads and several of them read `HOME` and
/// the other XDG variables, so that race is real there rather than theoretical. A thread-local hook
/// has no such hazard, needs no mutex, and cannot leak between tests. It is the same decision the
/// rest of the crate already makes for every external side effect: `Daemon<C: ProtonClient>` injects
/// a fake rather than talking to Proton.
///
/// THE REAL CRATE IS STILL PROVEN, just not from here: `tests/trash_disposal.rs` is a separate
/// process whose only tests are these, so it can redirect `XDG_DATA_HOME` exactly once through a
/// `OnceLock` — ordering the single write before every trash-crate read in that binary — and assert
/// that a real `.trashinfo` entry lands.
#[cfg(test)]
pub(crate) mod test_hook {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use super::LocalDeleteMode;
    use crate::AppResult;
    use crate::index::EntityKind;

    type Hook = Box<dyn Fn(LocalDeleteMode, &Path, EntityKind) -> AppResult<()>>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static CALLS: RefCell<Vec<(LocalDeleteMode, PathBuf, EntityKind)>> =
            const { RefCell::new(Vec::new()) };
    }

    /// Consulted by [`super::dispose`] before it touches the filesystem. `None` means no hook is
    /// installed on this thread and the real disposal runs.
    pub(crate) fn intercept(
        mode: LocalDeleteMode,
        path: &Path,
        kind: EntityKind,
    ) -> Option<AppResult<()>> {
        HOOK.with(|hook| {
            let hook = hook.borrow();
            let hook = hook.as_ref()?;
            CALLS.with(|calls| calls.borrow_mut().push((mode, path.to_path_buf(), kind)));
            Some(hook(mode, path, kind))
        })
    }

    /// Installs `hook` for this thread and removes it on drop, so one test's fake can never be seen
    /// by another even when the test panics.
    #[must_use]
    pub(crate) struct HookGuard;

    impl Drop for HookGuard {
        fn drop(&mut self) {
            HOOK.with(|hook| *hook.borrow_mut() = None);
            CALLS.with(|calls| calls.borrow_mut().clear());
        }
    }

    /// A fake trash that succeeds: `Trash` moves the entity into `trash_dir` (flattening the name,
    /// which is enough to assert "it went to the trash and not to oblivion"), `Permanent` still
    /// removes it for real, so a permanent-mode test asserts against the genuine `std::fs` calls.
    pub(crate) fn install_fake_trash(trash_dir: PathBuf) -> HookGuard {
        install(move |mode, path, kind| {
            if mode == LocalDeleteMode::Permanent {
                if kind == EntityKind::Directory {
                    std::fs::remove_dir_all(path)?;
                } else {
                    std::fs::remove_file(path)?;
                }
                return Ok(());
            }
            std::fs::create_dir_all(&trash_dir)?;
            let name = path.file_name().ok_or_else(|| {
                crate::boxed_error(format!("{} has no file name", path.display()))
            })?;
            std::fs::rename(path, trash_dir.join(name))?;
            Ok(())
        })
    }

    /// A fake trash that refuses every `Trash` disposal, leaving the entity on disk — the shape
    /// design D4 is about. `Permanent` still runs, so a mixed plan can be exercised.
    pub(crate) fn install_failing_trash() -> HookGuard {
        install(|mode, path, kind| {
            if mode == LocalDeleteMode::Permanent {
                if kind == EntityKind::Directory {
                    std::fs::remove_dir_all(path)?;
                } else {
                    std::fs::remove_file(path)?;
                }
                return Ok(());
            }
            Err(crate::boxed_error(format!(
                "could not move {} to the trash: the trash is unavailable",
                path.display()
            )))
        })
    }

    fn install(
        hook: impl Fn(LocalDeleteMode, &Path, EntityKind) -> AppResult<()> + 'static,
    ) -> HookGuard {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        CALLS.with(|calls| calls.borrow_mut().clear());
        HookGuard
    }

    /// Every disposal this thread has been asked for since the hook was installed, in order.
    pub(crate) fn calls() -> Vec<(LocalDeleteMode, PathBuf, EntityKind)> {
        CALLS.with(|calls| calls.borrow().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_default_mode_is_the_recoverable_one() {
        // The whole change in one assertion: a config that says nothing must not unlink.
        assert_eq!(LocalDeleteMode::default(), LocalDeleteMode::Trash);
        assert!(LocalDeleteMode::default().is_recoverable());
        assert!(!LocalDeleteMode::Permanent.is_recoverable());
    }

    #[test]
    fn every_mode_spelling_round_trips_through_serde_and_from_str() {
        for mode in LocalDeleteMode::ALL {
            let json = serde_json::to_string(&mode).expect("serialize");
            assert_eq!(
                json,
                format!("\"{}\"", mode.as_str()),
                "the serde rename and as_str must agree for {mode:?}"
            );
            assert_eq!(
                serde_json::from_str::<LocalDeleteMode>(&json).expect("deserialize"),
                mode
            );
            assert_eq!(mode.as_str().parse::<LocalDeleteMode>().expect("parse"), mode);
            assert_eq!(mode.to_string(), mode.as_str());
        }
    }

    #[test]
    fn an_unknown_spelling_names_every_accepted_value() {
        let error = "bin".parse::<LocalDeleteMode>().expect_err("must not parse");
        // The message is what a user sees on a typo'd config, so it has to carry the answer.
        assert!(error.contains("bin"), "{error}");
        for mode in LocalDeleteMode::ALL {
            assert!(error.contains(mode.as_str()), "{error} must name {mode}");
        }
    }

    #[test]
    fn permanent_removes_a_file_and_a_whole_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let file = root.path().join("doomed.txt");
        fs::write(&file, b"bytes").expect("write");
        dispose(LocalDeleteMode::Permanent, &file, EntityKind::File).expect("dispose the file");
        assert!(!file.exists(), "permanent mode must unlink the file");

        let dir = root.path().join("doomed-dir");
        fs::create_dir_all(dir.join("nested")).expect("mkdir");
        fs::write(dir.join("nested/inner.txt"), b"bytes").expect("write");
        dispose(LocalDeleteMode::Permanent, &dir, EntityKind::Directory).expect("dispose the dir");
        assert!(!dir.exists(), "permanent mode must remove the whole tree");
    }

    #[test]
    fn a_failing_trash_leaves_the_entity_on_disk_and_returns_an_error() {
        // The invariant the removed warnings rest on (design D4): the failure mode of the new path
        // is "the file is still there", never "the file is gone anyway".
        let root = tempfile::tempdir().expect("tempdir");
        let file = root.path().join("survivor.txt");
        fs::write(&file, b"bytes").expect("write");

        let _guard = test_hook::install_failing_trash();
        let error = dispose(LocalDeleteMode::Trash, &file, EntityKind::File)
            .expect_err("a failing trash must surface as an error");

        assert!(file.exists(), "the entity must survive a failed trash move");
        assert_eq!(fs::read(&file).expect("read"), b"bytes");
        assert!(error.to_string().contains("survivor.txt"), "{error}");
    }

    #[test]
    fn a_hook_sees_the_mode_the_path_and_the_kind() {
        let root = tempfile::tempdir().expect("tempdir");
        let trash = root.path().join("fake-trash");
        let file = root.path().join("moved.txt");
        fs::write(&file, b"bytes").expect("write");

        let _guard = test_hook::install_fake_trash(trash.clone());
        dispose(LocalDeleteMode::Trash, &file, EntityKind::File).expect("dispose");

        assert!(!file.exists(), "the entity leaves its original path");
        assert!(trash.join("moved.txt").exists(), "and lands in the trash");
        assert_eq!(
            test_hook::calls(),
            vec![(LocalDeleteMode::Trash, file, EntityKind::File)]
        );
    }

    #[test]
    fn the_hook_is_removed_when_its_guard_drops() {
        let root = tempfile::tempdir().expect("tempdir");
        let file = root.path().join("real.txt");
        fs::write(&file, b"bytes").expect("write");
        {
            let _guard = test_hook::install_failing_trash();
        }
        // No hook: `Permanent` reaches the real `std::fs` call rather than a stale fake.
        dispose(LocalDeleteMode::Permanent, &file, EntityKind::File).expect("dispose");
        assert!(!file.exists());
        assert!(test_hook::calls().is_empty());
    }
}
