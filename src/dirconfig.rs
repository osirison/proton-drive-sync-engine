//! Hierarchical, per-directory configuration — the `.gitignore`-style layer.
//!
//! A [`DIRECTORY_CONFIG_FILE_NAME`] file placed in any directory under the sync root applies to
//! that directory and everything beneath it; a deeper file overrides a shallower one
//! (nearest-ancestor wins), and any option a file leaves unset inherits from its nearest ancestor
//! (ultimately the daemon-wide default). The file is **machine-local**: it is ignored by scanning,
//! planning, the base-index filter, and the watcher (see [`crate::index::should_ignore_path`]), so
//! it is never uploaded to the remote — a safety policy must not be silently disable-able by a
//! remote-authored file.
//!
//! The module is deliberately generic over *which* settings it carries: today it resolves only the
//! directional delete-approval guard, but a new per-directory setting is added by giving
//! [`DirectorySettings`] a new optional field and [`EffectiveSettings`] a resolved counterpart —
//! the ancestor-walk merge in [`DirectoryConfigResolver::resolve`] needs no other change.

use crate::sync::DeleteDirection;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tracing::warn;

/// Name of the per-directory settings file. See the module docs for its inheritance semantics.
pub const DIRECTORY_CONFIG_FILE_NAME: &str = ".proton-sync.toml";

/// One directory's settings, as parsed from its [`DIRECTORY_CONFIG_FILE_NAME`]. Every field is
/// optional: an unset field inherits. Unknown keys are ignored (no `deny_unknown_fields`) so a
/// file authored for a newer engine still parses on an older one.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct DirectorySettings {
    #[serde(alias = "delete-approval")]
    delete_approval: Option<DeleteApprovalSettings>,
}

/// The directional delete-approval guard, as configured in a directory file. `remote`/`local`
/// name the *target* of the deletion being gated.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct DeleteApprovalSettings {
    /// Require approval before a deletion is propagated to **Proton Drive** (something was deleted
    /// locally → its remote copy would be deleted). Gates [`DeleteDirection::Remote`].
    remote: Option<bool>,
    /// Require approval before a deletion is propagated to the **local disk** (something was
    /// deleted/trashed remotely → its local copy would be deleted). Gates [`DeleteDirection::Local`].
    local: Option<bool>,
}

impl DirectorySettings {
    /// Overlay this directory's set options onto the inherited `effective` value. Only fields that
    /// are `Some` override; unset fields leave the inherited value untouched.
    fn apply_to(&self, effective: &mut EffectiveSettings) {
        if let Some(delete_approval) = &self.delete_approval {
            if let Some(remote) = delete_approval.remote {
                effective.require_remote_delete_approval = remote;
            }
            if let Some(local) = delete_approval.local {
                effective.require_local_delete_approval = local;
            }
        }
    }
}

/// Fully-resolved settings for one path: every option has a concrete value after merging the
/// daemon-wide default with the applicable directory files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveSettings {
    pub require_remote_delete_approval: bool,
    pub require_local_delete_approval: bool,
}

impl EffectiveSettings {
    /// Whether a deletion in the given direction requires user approval here.
    pub fn requires_approval(&self, direction: DeleteDirection) -> bool {
        match direction {
            DeleteDirection::Remote => self.require_remote_delete_approval,
            DeleteDirection::Local => self.require_local_delete_approval,
        }
    }
}

/// Resolves [`EffectiveSettings`] for entity paths by walking the directory files above them.
///
/// Holds a per-instance parse cache so building one resolver per reconcile pass reads each
/// directory file at most once, no matter how many deletions fall under it. A missing, unreadable,
/// or malformed file contributes no override (fail-safe: the guard stays in whatever state it
/// inherits), and an unreadable/malformed one is logged once (the cache prevents log spam).
pub struct DirectoryConfigResolver {
    local_root: PathBuf,
    default_settings: EffectiveSettings,
    cache: HashMap<PathBuf, DirectorySettings>,
}

impl DirectoryConfigResolver {
    /// `default_settings` is the daemon-wide default (from the resolved [`crate::daemon::DaemonConfig`]),
    /// the bottom of the inheritance chain.
    pub fn new(local_root: &Path, default_settings: EffectiveSettings) -> Self {
        Self {
            local_root: local_root.to_path_buf(),
            default_settings,
            cache: HashMap::new(),
        }
    }

    /// Effective settings for `relative_path` (an entity relative to the sync root). Applies the
    /// daemon default, then every ancestor directory's file from the root downward, so a deeper
    /// (nearer) directory overrides a shallower one. A directory entity's *own* file applies to it
    /// (a file placed in a folder governs that folder and its contents); a file entity has no file
    /// of its own, so only its ancestors count.
    pub fn resolve(&mut self, relative_path: &Path, is_directory: bool) -> EffectiveSettings {
        let mut effective = self.default_settings;
        for directory in directory_chain(relative_path, is_directory) {
            self.settings_for_directory(&directory)
                .apply_to(&mut effective);
        }
        effective
    }

    fn settings_for_directory(&mut self, relative_directory: &Path) -> DirectorySettings {
        if let Some(cached) = self.cache.get(relative_directory) {
            return cached.clone();
        }
        let settings = self.load_directory(relative_directory);
        self.cache
            .insert(relative_directory.to_path_buf(), settings.clone());
        settings
    }

    fn load_directory(&self, relative_directory: &Path) -> DirectorySettings {
        let path = self
            .local_root
            .join(relative_directory)
            .join(DIRECTORY_CONFIG_FILE_NAME);
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<DirectorySettings>(&contents) {
                Ok(settings) => settings,
                Err(error) => {
                    warn!(
                        path = %path.display(),
                        %error,
                        "ignoring malformed per-directory config; delete-approval protection stays in effect here"
                    );
                    DirectorySettings::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DirectorySettings::default()
            }
            Err(error) => {
                warn!(
                    path = %path.display(),
                    %error,
                    "could not read per-directory config; delete-approval protection stays in effect here"
                );
                DirectorySettings::default()
            }
        }
    }
}

/// The directories whose config files apply to `relative_path`, in root-first order (so callers
/// can overlay them and let the last — deepest — win). Always includes the sync root (the empty
/// path). When `include_self` is true (a directory entity), the path's own directory is included;
/// otherwise (a file entity) only its ancestors are.
fn directory_chain(relative_path: &Path, include_self: bool) -> Vec<PathBuf> {
    let components: Vec<&std::ffi::OsStr> = relative_path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    let depth = if include_self {
        components.len()
    } else {
        components.len().saturating_sub(1)
    };

    let mut chain = vec![PathBuf::new()];
    let mut accumulated = PathBuf::new();
    for component in components.into_iter().take(depth) {
        accumulated.push(component);
        chain.push(accumulated.clone());
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const PROTECTED: EffectiveSettings = EffectiveSettings {
        require_remote_delete_approval: true,
        require_local_delete_approval: true,
    };

    fn write_config(root: &Path, relative_dir: &str, contents: &str) {
        let dir = root.join(relative_dir);
        fs::create_dir_all(&dir).expect("create dir");
        fs::write(dir.join(DIRECTORY_CONFIG_FILE_NAME), contents).expect("write config");
    }

    #[test]
    fn resolves_to_default_when_no_files_exist() {
        let root = tempdir().expect("tempdir");
        let mut resolver = DirectoryConfigResolver::new(root.path(), PROTECTED);
        let effective = resolver.resolve(Path::new("a/b/c.txt"), false);
        assert!(effective.require_remote_delete_approval);
        assert!(effective.require_local_delete_approval);
    }

    #[test]
    fn root_file_sets_the_global_default_for_the_tree() {
        let root = tempdir().expect("tempdir");
        write_config(
            root.path(),
            "",
            "[delete_approval]\nremote = false\nlocal = false\n",
        );
        let mut resolver = DirectoryConfigResolver::new(root.path(), PROTECTED);
        let effective = resolver.resolve(Path::new("deep/nested/file.txt"), false);
        assert!(!effective.require_remote_delete_approval);
        assert!(!effective.require_local_delete_approval);
    }

    #[test]
    fn nearest_ancestor_wins_over_shallower_ones() {
        let root = tempdir().expect("tempdir");
        write_config(root.path(), "", "[delete_approval]\nlocal = false\n");
        // A deeper directory re-enables the guard it inherited as off.
        write_config(root.path(), "keep", "[delete_approval]\nlocal = true\n");
        let mut resolver = DirectoryConfigResolver::new(root.path(), PROTECTED);

        assert!(
            !resolver
                .resolve(Path::new("other/file.txt"), false)
                .require_local_delete_approval,
            "root's local=false must apply outside the overriding subtree"
        );
        assert!(
            resolver
                .resolve(Path::new("keep/file.txt"), false)
                .require_local_delete_approval,
            "the deeper keep/ file must override the root's local=false"
        );
    }

    #[test]
    fn partial_override_leaves_the_other_direction_inherited() {
        let root = tempdir().expect("tempdir");
        // Only the remote direction is opted out; local must stay protected by default.
        write_config(root.path(), "sub", "[delete_approval]\nremote = false\n");
        let mut resolver = DirectoryConfigResolver::new(root.path(), PROTECTED);
        let effective = resolver.resolve(Path::new("sub/file.txt"), false);
        assert!(!effective.require_remote_delete_approval);
        assert!(effective.require_local_delete_approval);
    }

    #[test]
    fn a_directory_entity_honors_its_own_file() {
        let root = tempdir().expect("tempdir");
        write_config(root.path(), "sub", "[delete_approval]\nlocal = false\n");
        let mut resolver = DirectoryConfigResolver::new(root.path(), PROTECTED);
        // Resolving for the directory `sub` itself must see `sub`'s own file.
        assert!(
            !resolver
                .resolve(Path::new("sub"), true)
                .require_local_delete_approval
        );
        // A *file* named `sub` (no config of its own) must not — only its ancestors count.
        assert!(
            resolver
                .resolve(Path::new("sub"), false)
                .require_local_delete_approval
        );
    }

    #[test]
    fn malformed_file_stays_protected() {
        let root = tempdir().expect("tempdir");
        write_config(root.path(), "sub", "this is not valid toml : : :");
        let mut resolver = DirectoryConfigResolver::new(root.path(), PROTECTED);
        let effective = resolver.resolve(Path::new("sub/file.txt"), false);
        assert!(
            effective.require_remote_delete_approval && effective.require_local_delete_approval,
            "a malformed directory config must never weaken the guard"
        );
    }

    #[test]
    fn requires_approval_reads_the_matching_direction() {
        let effective = EffectiveSettings {
            require_remote_delete_approval: false,
            require_local_delete_approval: true,
        };
        assert!(!effective.requires_approval(DeleteDirection::Remote));
        assert!(effective.requires_approval(DeleteDirection::Local));
    }
}
