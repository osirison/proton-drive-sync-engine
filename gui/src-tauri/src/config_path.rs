//! Runtime path resolution.
//!
//! The daemon has **no canonical config path**, so the GUI *owns* the convention:
//! `$XDG_CONFIG_HOME/proton-sync/proton-sync.toml` (default `~/.config/proton-sync/proton-sync.toml`).
//! From that file (via `gui-core`'s reader) we resolve the socket, index DB, and roots the commands
//! need, falling back to the engine's own defaults when a key is unset.
//!
//! A daemon may also be running with flags or a different config file entirely. The status reply
//! carries its *live* resolved roots (`RunningConfigInfo`, and one `PairSummary` per pair); every
//! status-shaped round trip caches them here so a command can fall back to the daemon's ground truth
//! when the GUI config doesn't provide a value.
//!
//! **Pair-indexed (#102 phase 5a).** Everything a folder pair owns — its roots, its index, how its
//! conflict sidecars are spelled — is a slot of [`PairPaths`] (what the file says) beside a slot of
//! [`PairReported`] (what the daemon last said), looked up **by pair name**. What stays singular is
//! what is singular in the daemon: the config path, the control socket, the `proton-drive` command.

use gui_core::config_io::{ConflictNaming, DEFAULT_PAIR_NAME};
use gui_core::gui_prefs;
use gui_core::pairs::{resolve_selection, wire_selector, PairCapability};
use gui_core::wire::ControlResponse;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A base-directory environment value, honoured **only when it is absolute** (#286). Every XDG
/// reader in this crate goes through it; the value is taken as an argument (never read here) so
/// the rule is testable without mutating process-wide environment variables, which races every
/// other test in the binary and is `unsafe` since edition 2024.
///
/// The XDG Base Directory specification is explicit: these variables "must be absolute", and an
/// implementation that meets a relative one "should consider the path invalid and ignore it". A
/// relative value is resolved against the *process's* working directory — for a desktop launcher,
/// whatever it happened to leave behind — so the GUI would write its config, dial its socket, and
/// drop its tray glyphs somewhere neither the user nor the daemon chose. The socket is the one
/// with a visible symptom: the daemon binds one resolution and the GUI dials another, which reads
/// as `unreachable` against a healthy daemon.
///
/// `is_absolute` subsumes the emptiness check these readers used to make on their own (an empty
/// path is not absolute) and catches the literal `~` no shell expanded for a GUI process (#135).
pub fn absolute_dir(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Paths the command layer needs, resolved once at startup and re-resolved after a config write.
///
/// **`socket_path` is re-resolved LATER than the rest** (#336): `commands::write_config` refreshes
/// every other field the moment a save lands, but leaves this one exactly where it was, because the
/// restart that follows a save has to dial the OLD value to shut the still-live old daemon down.
/// `commands::restart_service` moves it, once the restart's own outcome confirms nothing needs that
/// address any more (`commands::old_socket_is_settled`).
///
/// **Every re-resolve names the file it reads** ([`Self::resolve_at`]); only the process's startup
/// asks the environment where that file is ([`Self::resolve`]). A re-resolve that asked again would
/// read — and a test that saved first would then WRITE — the developer's real config.
pub struct RuntimePaths {
    pub config_path: PathBuf,
    /// `Err` only when the GUI config sets no `socket_path` **and** the engine's default fails
    /// closed (#74 — `XDG_RUNTIME_DIR` unset and the shared-/tmp fallback is not a directory this
    /// user owns at 0700). Carried as a `Result` rather than a guessed path so the UI reports the
    /// real reason instead of a wrong "connect: No such file" (#277).
    pub socket_path: Result<PathBuf, String>,
    pub proton_cli: String,
    /// Whether the file states its pairs as `[[pair]]` tables rather than as the implicit one-pair
    /// top-level keys. The child `--dry-run` is addressed differently for the two (`dry_run_args`).
    pub pair_tables: bool,
    /// The pairs the FILE declares, in file order — the first is the default pair. An implicit
    /// one-pair file is one pair named `default`. Empty when the file cannot be read as a config at
    /// all (the daemon would not start on it either), in which case the daemon's own report is the
    /// only source.
    pub pairs: Vec<PairPaths>,
    /// Why `pairs` is empty when the reason is the FILE: the engine's own message for a config it
    /// cannot read. `None` for a file it reads, however little that file says.
    pub config_error: Option<String>,
    /// What the running daemon last said, per pair. Fallbacks only: an explicit value in the GUI
    /// config always wins.
    pub daemon: DaemonView,
    /// The pair the person last chose, as `gui.toml` holds it (#102 phase 5a-2) — **a remembered
    /// preference, not a fact**: it may name a pair that no longer exists, and is never rewritten on
    /// that evidence (a daemon that is restarting, or on an older config, momentarily knows fewer
    /// pairs than the person has). [`Self::selected_pair`] is what the app acts on.
    ///
    /// Loaded by [`Self::resolve_at`] from the `gui.toml` beside `config_path`, so every
    /// re-resolve — a save, a restart — re-reads the one file that holds it instead of each having
    /// to remember to carry it across; written only by `commands::select_pair`.
    pub selected: Option<String>,
}

/// Which pair a command means. The three questions are different and none may stand in for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask<'a> {
    /// The default pair — what a request naming none reaches, and what the tray acts on until it
    /// learns to name a pair (phase 5d).
    Default,
    /// The pair the person has selected, validated against what exists. **Only a class-R read asks
    /// this** (a wrong pair costs a wrong screen, not data); a write, a deletion or the start of a
    /// multi-step flow names its pair ([`Self::Named`]), so the selection moving between a click and
    /// its execution can make a screen stale and never make a deletion land elsewhere.
    Selected,
    /// A pair by name, byte-exactly, which must be one the app knows.
    Named(&'a str),
    /// [`Self::Named`], with the name **always written on the wire**, the default pair's included.
    ///
    /// The default pair is addressed by omission (`wire_selector`), and a row drawn for a folder that
    /// WAS the default is the one place that goes wrong: a click on a stale `Pause documents`, after a
    /// daemon restart changed which folder stands first, reaches the daemon unaddressed and pauses
    /// whichever folder is the default now. Written out, the daemon's own byte-exact selector rule
    /// answers "no such pair" and nothing is paused. Only a request that can be sent to a daemon known
    /// to list its folders may use it — an older daemon ignores a selector it does not read, which
    /// the reply's shape reports (`Answer::NotUnderstood`) and the caller refuses.
    Explicit(&'a str),
}

impl<'a> Ask<'a> {
    /// A class-R command's optional `pair` argument: naming one overrides the selection, naming
    /// none means the selection.
    pub fn read(pair: Option<&'a str>) -> Self {
        pair.map_or(Ask::Selected, Ask::Named)
    }
}

/// One folder pair as the GUI config file states it.
pub struct PairPaths {
    pub name: String,
    /// From the file: an explicit `db_path`, or the per-root default derived from `local_root`.
    /// `None` when the file places neither — the daemon-reported path may still fill in.
    pub db_path: Option<PathBuf>,
    pub local_root: Option<PathBuf>,
    /// **The file's value, never merged with the daemon's.** There is no `effective_remote_root`
    /// (see below): the two are kept apart so a caller can tell "the file says X" from "the daemon
    /// says X".
    pub remote_root: Option<PathBuf>,
    /// How THIS pair's daemon spells conflict sidecars (`conflict_suffix`, which is per pair in the
    /// engine). Resolved from the same file the daemon reads, because the GUI's conflict scanner
    /// walks the disk looking for exactly the names the daemon wrote — a scanner holding the default
    /// while the daemon runs a custom suffix reports "no conflicts" on a folder full of them. An
    /// invalid value falls back to the default rather than failing the whole resolve: that config
    /// does not start a daemon either, and the Settings screen's own validation is what reports it.
    pub conflict_naming: ConflictNaming,
}

/// What the daemon last said about itself, from the most recent status-shaped reply.
#[derive(Default)]
pub struct DaemonView {
    /// Whether the daemon understands a pair selector, as that reply showed.
    pub capability: PairCapability,
    /// One entry per pair the daemon runs. A daemon that predates multi-pair reports its one pair
    /// as `config`, which is stored here as a single entry named after the reply's `pair`
    /// (`default` when it names none).
    pub pairs: Vec<PairReported>,
}

/// One pair's roots as the DAEMON reports them — the live values, however it was launched.
pub struct PairReported {
    pub name: String,
    pub local_root: PathBuf,
    pub remote_root: PathBuf,
    pub db_path: PathBuf,
}

/// A command's answer to "which pair?": the name its path slots are looked up by, and the selector
/// its request carries. They differ in exactly one case — the default pair has a name and no
/// selector, because the default pair is addressed by omission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRef {
    pub name: String,
    pub selector: Option<String>,
}

/// The GUI's owned config path convention.
pub fn gui_config_path() -> PathBuf {
    gui_config_path_in(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn gui_config_path_in(config_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = absolute_dir(config_home)
        // Same rule for `HOME`: a relative one lands the config in the same per-cwd place.
        .or_else(|| absolute_dir(home).map(|home| home.join(".config")))
        // Last resort. Was a relative `.config`, which is the very shape the rule above rejects —
        // the config would be written under, and re-read from, whatever cwd each launch had.
        .unwrap_or_else(std::env::temp_dir);
    base.join("proton-sync").join("proton-sync.toml")
}

use gui_core::config_io::expand_config_path as expand;

// There is deliberately no `default_socket_path` here any more (#277). This crate had a private
// copy, and a private copy is how the GUI ended up dialling `<temp>/proton-sync.sock` while the
// daemon bound `<temp>/proton-drive-sync-<uid>/proton-sync.sock` — a healthy daemon rendered
// `unreachable`. `RuntimePaths::resolve` delegates to `gui_core::ipc::default_socket_path`, so the
// #286 absolute-XDG rule reaches the socket through the engine's own `paths::default_socket_path`
// rather than through a second implementation of it here. What the deleted
// `a_relative_runtime_dir_does_not_move_the_socket_the_gui_dials` asserted is now asserted where
// the code is: `paths::a_relative_runtime_dir_falls_through_to_the_validated_fallback` for the
// rule, and gui-core's `the_default_socket_path_is_never_the_unnamespaced_temp_one_the_gui_used_
// to_build` for the GUI resolving to exactly the engine's answer.

impl RuntimePaths {
    /// Resolve from the config file the environment points at. **Only the process's startup calls
    /// this** (`lib.rs`); every later re-resolve and every test names its file with
    /// [`Self::resolve_at`]. A source scan (`isolation_scan`) holds that line.
    pub fn resolve() -> Self {
        Self::resolve_at(&gui_config_path())
    }

    /// Resolve from `config_path`, applying engine defaults where a key is unset.
    pub fn resolve_at(config_path: &Path) -> Self {
        Self::resolve_with_default_socket(config_path, environment_default_socket)
    }

    /// [`Self::resolve_at`], with the answer to "which socket, when the file names none" supplied.
    /// Production always supplies the environment's ([`environment_default_socket`]); the seam is
    /// here so that choice can be tested without a test ever resolving the real one.
    fn resolve_with_default_socket(
        config_path: &Path,
        default_socket: impl FnOnce() -> Result<PathBuf, String>,
    ) -> Self {
        let loaded = gui_core::config_io::ConfigDoc::load(config_path);
        let load_error = loaded.as_ref().err().map(ToString::to_string);
        let doc = loaded.ok();
        let get = |key: &str| doc.as_ref().and_then(|d| d.get_str(key));

        let socket_path = match get("socket_path") {
            Some(value) => Ok(expand(value, "socket_path")),
            None => default_socket(),
        };
        let proton_cli = get("proton_cli")
            .map(|value| expand(value, "proton_cli").to_string_lossy().into_owned())
            .unwrap_or_else(|| "proton-drive".to_string());

        // THE PAIRS COME FROM THE ENGINE (`config::pair_views`), not from this file's own reading of
        // the keys. It was a second derivation of the db-path default and of the `~` rule, and two
        // readers of one format is how the GUI and the daemon came to disagree about `~` in the first
        // place (#135). The engine expands `~` once, in `local_root` before the index path derives
        // from it, and never in `remote_root`, which is a Drive path where `~` means nothing.
        //
        // **A file the engine refuses declares no pairs, and the app keeps the reason.** Before
        // this the top-level keys were read whatever else the file held, so `local_root` still
        // placed a folder beside a key the daemon would reject. The daemon does not start on such a
        // file either, so reading it leniently here would show a folder nothing is syncing. The
        // reason travels instead (`config_error`), because "no pairs" with no explanation reads to
        // every command that needs a folder as "local_root is not configured", which is false.
        let (pairs, config_error) = match &doc {
            Some(doc) => match pair_paths_from_text(&doc.to_toml_string()) {
                Ok(pairs) => (pairs, None),
                Err(error) => (Vec::new(), Some(error)),
            },
            None => (Vec::new(), load_error),
        };

        Self {
            config_path: config_path.to_owned(),
            socket_path,
            proton_cli,
            pair_tables: doc.as_ref().is_some_and(|d| d.declares_pair_tables()),
            pairs,
            config_error,
            daemon: DaemonView::default(),
            selected: gui_prefs::load_selected_pair(&gui_prefs::gui_prefs_path(config_path)),
        }
    }

    /// What to tell someone whose command needs a folder the app cannot place. `what` is the plain
    /// answer ("local_root is not configured"), which is right only when nothing is wrong with the
    /// file: when the file is the reason there are no pairs, the file's own error is the answer, in
    /// the engine's words.
    pub fn unplaced(&self, what: &str) -> String {
        match &self.config_error {
            Some(error) => format!("the config file has an error: {error}"),
            None => what.to_owned(),
        }
    }

    /// Cache what a status-shaped reply says about the daemon: which pairs it runs, where, and
    /// whether it understands a selector at all.
    ///
    /// **Keyed by the pair, never by "the last reply".** A reply addressed to one pair describes
    /// that pair at its top level, so caching it as *the* daemon config would paint pair B's roots
    /// over pair A's the moment something asks about B. The list a current daemon sends carries
    /// every pair's three paths, so it replaces the cache wholesale and a reply about one pair can
    /// never overwrite another's slot. A daemon that predates the list reports one pair as
    /// `config`, which is filed under the name the reply gives it.
    pub fn remember_daemon_reply(&mut self, response: &ControlResponse) {
        self.daemon.capability = PairCapability::from_reply(response);
        if !response.pairs.is_empty() {
            self.daemon.pairs = response
                .pairs
                .iter()
                .map(|pair| PairReported {
                    name: pair.name.clone(),
                    local_root: pair.local_root.clone(),
                    remote_root: pair.remote_root.clone(),
                    db_path: pair.db_path.clone(),
                })
                .collect();
        } else if let Some(info) = &response.config {
            self.daemon.pairs = vec![PairReported {
                name: response
                    .pair
                    .clone()
                    .unwrap_or_else(|| DEFAULT_PAIR_NAME.to_owned()),
                local_root: info.local_root.clone(),
                remote_root: info.remote_root.clone(),
                db_path: info.db_path.clone(),
            }];
        }
    }

    /// The names a command may address: the daemon's pairs once it has answered, otherwise the
    /// file's. In the daemon's order, so the first is the default pair either way.
    pub fn known_pair_names(&self) -> Vec<&str> {
        if self.daemon.pairs.is_empty() {
            self.pairs.iter().map(|pair| pair.name.as_str()).collect()
        } else {
            self.daemon
                .pairs
                .iter()
                .map(|pair| pair.name.as_str())
                .collect()
        }
    }

    /// The default pair — the one a request that names none reaches. With nothing known it is
    /// still called `default`, which is what the implicit pair of an empty file is.
    pub fn default_pair_name(&self) -> String {
        self.known_pair_names()
            .first()
            .map_or_else(|| DEFAULT_PAIR_NAME.to_owned(), |name| (*name).to_owned())
    }

    /// Which pair a command means, and how its request must address it.
    ///
    /// `None` is the default pair. A name is validated against the **known** set and an unknown one
    /// is refused — never read as the default, because a command that meant one folder and acted on
    /// another is the failure this layer exists to prevent. The wire selector is absent for the
    /// default pair (`wire_selector`), so a one-pair setup sends exactly what it always sent.
    pub fn resolve_pair(&self, requested: Option<&str>) -> Result<PairRef, String> {
        let default = self.default_pair_name();
        // A command that names no pair means the default one, and goes through the SAME rule as one
        // that names it: there is one place a chosen pair becomes a wire selector, and a request for
        // the default pair is the case that rule exists for.
        let selected = match requested {
            None => default.clone(),
            Some(requested) => {
                let known = self.known_pair_names();
                if !known.contains(&requested) {
                    return Err(if known.is_empty() {
                        // Nothing known is either "nothing configured" or "a file the engine
                        // refuses", and only the second has a reason worth giving: the file's own.
                        self.unplaced(&format!(
                            "no folder pair named {requested:?}: no folder pairs are configured"
                        ))
                    } else {
                        format!(
                            "no folder pair named {requested:?}: the configured pairs are {}",
                            known
                                .iter()
                                .map(|name| format!("{name:?}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    });
                }
                requested.to_owned()
            }
        };
        Ok(PairRef {
            selector: wire_selector(&selected, &default),
            name: selected,
        })
    }

    /// Resolve a command's [`Ask`] to the pair it addresses.
    pub fn resolve_ask(&self, ask: Ask<'_>) -> Result<PairRef, String> {
        match ask {
            Ask::Default => self.resolve_pair(None),
            Ask::Selected => Ok(self.selected_pair()),
            Ask::Named(name) => self.resolve_pair(Some(name)),
            Ask::Explicit(name) => {
                let mut pair = self.resolve_pair(Some(name))?;
                pair.selector = Some(pair.name.clone());
                Ok(pair)
            }
        }
    }

    /// The pair the app acts on when nothing names one: the remembered choice, **validated on every
    /// call and never written back**, and the default pair whenever it cannot be trusted.
    ///
    /// - The choice counts only while the daemon is known to read a selector
    ///   ([`PairCapability::MultiPair`]). Before its first reply (`Unknown`) and against one that
    ///   predates the field (`Legacy`) the answer is the default pair, so every request goes out
    ///   unaddressed — the one shape a daemon of any age acts on correctly. A selector sent to a
    ///   daemon that ignores it is not an error it reports: it acts on its only pair.
    /// - A choice that names no pair the daemon runs is the default pair
    ///   ([`resolve_selection`]).
    ///
    /// So `Unknown` costs one poll at start-up, showing the default pair before the capability is
    /// learned. That is the price of not guessing.
    pub fn selected_pair(&self) -> PairRef {
        let default = self.default_pair_name();
        let name = if self.daemon.capability == PairCapability::MultiPair {
            resolve_selection(self.selected.as_deref(), &self.known_pair_names())
                .map_or_else(|| default.clone(), str::to_owned)
        } else {
            default.clone()
        };
        PairRef {
            selector: wire_selector(&name, &default),
            name,
        }
    }

    /// Remember `name` as the choice, if it is a pair this app knows. Memory only: the caller writes
    /// `gui.toml` first, so a failed write leaves the choice unchanged rather than half made.
    pub fn select(&mut self, name: &str) -> Result<(), String> {
        self.resolve_pair(Some(name))?;
        self.selected = Some(name.to_owned());
        Ok(())
    }

    /// The name the config FILE gives the pair `selected` — what the child `proton-syncd --dry-run`
    /// must be told, because it reads the file and nothing else.
    ///
    /// A selection is validated against [`Self::known_pair_names`], which is the DAEMON's list once
    /// it has answered, so the name a pair is selected by and the name the file gives it can differ:
    /// a daemon started before the file grew `[[pair]]` tables calls its one pair `default`, while
    /// the file's only table is `photos`. Handing the child the daemon's name made the preview fail
    /// with "names no configured folder pair" where it used to run.
    ///
    /// **By name first** — the file may have been reordered since the daemon read it. **Then by
    /// position**, because the daemon's order is the file's order (`resolve_pairs`) and the first is
    /// the default pair in both: the table standing where the daemon's pair stands. That second
    /// step only counts when the table's name is not also one of the daemon's own pairs, since then
    /// it is demonstrably a different pair and handing it over would preview the wrong folder.
    /// `None` when neither holds; the caller keeps the selected name, and the engine's refusal says
    /// which pairs the file does have.
    pub fn file_pair_name(&self, selected: &str) -> Option<&str> {
        if let Some(pair) = self.pair(selected) {
            return Some(pair.name.as_str());
        }
        let known = self.known_pair_names();
        let position = known.iter().position(|name| *name == selected)?;
        let candidate = self.pairs.get(position)?;
        (!known.contains(&candidate.name.as_str())).then_some(candidate.name.as_str())
    }

    /// What the file says about `pair`, if it declares one by that name.
    pub fn pair(&self, pair: &str) -> Option<&PairPaths> {
        self.pairs.iter().find(|candidate| candidate.name == pair)
    }

    /// What the daemon last reported about `pair`.
    pub fn reported(&self, pair: &str) -> Option<&PairReported> {
        self.daemon
            .pairs
            .iter()
            .find(|candidate| candidate.name == pair)
    }

    /// The local root commands should act on for `pair`: GUI config first, daemon-reported second.
    pub fn effective_local_root(&self, pair: &str) -> Option<PathBuf> {
        self.pair(pair)
            .and_then(|configured| configured.local_root.clone())
            .or_else(|| {
                self.reported(pair)
                    .map(|reported| reported.local_root.clone())
            })
    }

    // `effective_remote_root` WAS HERE and went with `list_remote` (#311). It resolved the remote
    // root for a listing the GUI ran itself, and that is the one question this process must not
    // answer: `run_dry_run` reads `PairPaths::remote_root` and `PairReported::remote_root` RAW and
    // separately, because `daemon_plans_the_same_roots` has to tell a configured root from a
    // daemon-reported one rather than collapse them. A resolver that hides which of the two
    // answered has no caller left, and would be the wrong shape for the one caller there is. Pair
    // indexing does not change that: it makes it two slots PER PAIR, not one merged getter.
    // (`the_two_remote_roots_stay_two` holds the line.)

    /// The index DB for read-only lookups on `pair`: GUI config first, daemon-reported second.
    pub fn effective_db_path(&self, pair: &str) -> Option<PathBuf> {
        self.pair(pair)
            .and_then(|configured| configured.db_path.clone())
            .or_else(|| self.reported(pair).map(|reported| reported.db_path.clone()))
    }

    /// How `pair`'s conflict sidecars are named. The default naming for a pair the file does not
    /// declare, which is the daemon's own default and so right until the daemon says otherwise.
    pub fn conflict_naming(&self, pair: &str) -> ConflictNaming {
        self.pair(pair)
            .map(|configured| configured.conflict_naming.clone())
            .unwrap_or_default()
    }
}

/// The control socket when the config file names none: the one the daemon binds by default.
///
/// The default comes from the engine (`gui_core::ipc::default_socket_path`), never from a private
/// copy here: the copy this replaced pointed at `<temp>/proton-sync.sock` while the daemon bound
/// `<temp>/proton-drive-sync-<uid>/proton-sync.sock` whenever `XDG_RUNTIME_DIR` was unset (#277).
#[cfg(not(test))]
fn environment_default_socket() -> Result<PathBuf, String> {
    gui_core::ipc::default_socket_path()
}

/// **In a test build the environment's socket does not exist as far as this crate is concerned.**
///
/// A test that resolves a config in a temp directory and forgets to point the socket at its fake
/// daemon used to inherit `$XDG_RUNTIME_DIR/proton-sync.sock`, which on a developer's machine is the
/// live daemon's: the next command a test ran (`pause`, `resync`, `approve`) was delivered to it.
/// Measured with a listener bound at that path — it received `"command":"pause"`, and the isolation
/// scan stayed green. A convention a test has to remember is not a guard, so the default is removed
/// instead: the socket is an `Err` that says what to do, every command folds that into an
/// `unreachable` payload before it opens anything, and there is no environment socket to reach. A
/// test names its socket (`paths.socket_path = Ok(fake.socket_path().to_owned())`), or it gets this.
///
/// An `Err` and not a panic: most tests never touch the socket and must keep running; the ones that
/// do fail on their own assertion with the reason in the message. The other way round — a test
/// writing `gui_core::ipc::default_socket_path()` into the field itself — is `isolation_scan`'s.
#[cfg(test)]
fn environment_default_socket() -> Result<PathBuf, String> {
    Err(
        "this test build does not resolve the environment's control socket: name one \
         (`paths.socket_path = Ok(<a fake daemon's socket>)`)"
            .to_owned(),
    )
}

/// The engine's reading of `text`, as this module's slots. `Err` carries the engine's own message
/// when the text is not a config file it can parse.
fn pair_paths_from_text(text: &str) -> Result<Vec<PairPaths>, String> {
    gui_core::config_io::pair_views(text)
        .map(|views| {
            views
                .into_iter()
                .map(|view| PairPaths {
                    conflict_naming: view
                        .conflict_suffix
                        .as_deref()
                        .and_then(|suffix| ConflictNaming::new(suffix).ok())
                        .unwrap_or_default(),
                    name: view.name,
                    db_path: view.db_path,
                    local_root: view.local_root,
                    remote_root: view.remote_root,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // #286. Asserted through the resolved paths, not the predicate: a relative value fails nothing
    // loudly — it resolves against whatever working directory the launcher left, per process, so
    // the symptom is two processes disagreeing about one path rather than an error anywhere.

    #[test]
    fn a_relative_config_home_falls_through_to_the_home_default() {
        let path = gui_config_path_in(Some(OsString::from(".config")), Some("/home/me".into()));

        assert_eq!(
            path,
            PathBuf::from("/home/me/.config/proton-sync/proton-sync.toml")
        );
    }

    #[test]
    fn a_relative_home_leaves_no_relative_config_path_behind() {
        // Both values invalid: the last resort must still be absolute, or the GUI writes its config
        // under one cwd and re-reads nothing under the next.
        let path = gui_config_path_in(Some(OsString::new()), Some(OsString::from("home/me")));

        assert!(path.is_absolute(), "config path: {}", path.display());
        assert_eq!(
            path,
            std::env::temp_dir()
                .join("proton-sync")
                .join("proton-sync.toml")
        );
    }

    #[test]
    fn only_absolute_env_values_are_honoured() {
        assert_eq!(
            absolute_dir(Some(OsString::from("/run/user/1000"))),
            Some(PathBuf::from("/run/user/1000"))
        );
        assert_eq!(absolute_dir(None), None);
        // Empty: what the old emptiness checks caught, still caught.
        assert_eq!(absolute_dir(Some(OsString::new())), None);
        assert_eq!(absolute_dir(Some(OsString::from(".config"))), None);
        // A literal `~` no shell expanded is a relative component, not $HOME (#135).
        assert_eq!(absolute_dir(Some(OsString::from("~/.config"))), None);
    }

    // ---- pair-indexed slots (#102 phase 5a) ----

    use gui_core::testing::{FakeDaemon, FakePair};
    use gui_core::wire::ControlCommand;

    /// `text` written as a config file in a fresh temp directory, resolved at THAT path — never the
    /// environment's.
    fn resolved(text: &str) -> (RuntimePaths, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.toml");
        std::fs::write(&path, text).unwrap();
        (RuntimePaths::resolve_at(&path), dir)
    }

    const TWO_PAIRS: &str = "\
socket_path = \"/tmp/gui-two-pairs.sock\"
proton_cli = \"/usr/bin/fake-proton-drive\"

[[pair]]
name = \"photos\"
local_root = \"/local/photos\"
remote_root = \"/Drive/Photos\"
conflict_suffix = \"alpha\"

[[pair]]
name = \"docs\"
local_root = \"/local/docs\"
remote_root = \"/Drive/Docs\"
db_path = \"/state/docs.db\"
conflict_suffix = \"beta\"
";

    /// Acceptance 3: each pair has ITS OWN roots, index and sidecar spelling. The failure this
    /// guards is a slot shared between pairs, which would scan one pair's folder for another's
    /// conflicts and read one pair's index for another's files.
    #[test]
    fn resolve_at_gives_each_pair_its_own_roots_db_and_conflict_suffix() {
        let (paths, _dir) = resolved(TWO_PAIRS);
        assert!(paths.pair_tables);
        assert_eq!(paths.known_pair_names(), ["photos", "docs"]);
        assert_eq!(
            paths.effective_local_root("photos"),
            Some(PathBuf::from("/local/photos"))
        );
        assert_eq!(
            paths.effective_local_root("docs"),
            Some(PathBuf::from("/local/docs"))
        );
        // The per-root `.sync` default for one, an explicit override for the other.
        assert_eq!(
            paths.effective_db_path("photos"),
            Some(PathBuf::from("/local/photos/.sync/sync_index.db"))
        );
        assert_eq!(
            paths.effective_db_path("docs"),
            Some(PathBuf::from("/state/docs.db"))
        );
        assert_eq!(paths.conflict_naming("photos").suffix(), "alpha");
        assert_eq!(paths.conflict_naming("docs").suffix(), "beta");
        // A pair nothing declares has no slot, and falls back to the default sidecar spelling.
        assert_eq!(paths.effective_local_root("nope"), None);
        assert_eq!(paths.effective_db_path("nope"), None);
        assert_eq!(
            paths.conflict_naming("nope"),
            ConflictNaming::default(),
            "an undeclared pair is not given another pair's suffix"
        );
        // Daemon-wide keys are read once.
        assert_eq!(paths.proton_cli, "/usr/bin/fake-proton-drive");
        assert_eq!(
            paths.socket_path.as_deref().ok(),
            Some(Path::new("/tmp/gui-two-pairs.sock"))
        );
    }

    #[test]
    fn an_implicit_one_pair_file_is_one_pair_called_default_with_the_slots_it_always_had() {
        let (paths, dir) = resolved(
            "local_root = \"/local/x\"\nremote_root = \"/Drive/X\"\nconflict_suffix = \"mine\"\n",
        );
        assert!(!paths.pair_tables);
        assert_eq!(paths.known_pair_names(), ["default"]);
        assert_eq!(paths.default_pair_name(), "default");
        assert_eq!(
            paths.effective_local_root("default"),
            Some(PathBuf::from("/local/x"))
        );
        assert_eq!(
            paths.effective_db_path("default"),
            Some(PathBuf::from("/local/x/.sync/sync_index.db"))
        );
        assert_eq!(
            paths.pair("default").unwrap().remote_root,
            Some(PathBuf::from("/Drive/X"))
        );
        assert_eq!(paths.conflict_naming("default").suffix(), "mine");
        assert_eq!(paths.config_path, dir.path().join("proton-sync.toml"));
    }

    /// `resolve_at` reads the file it is handed and no other: the one property the real-config
    /// protection rests on. A file that does not exist is an empty config, not an error and not a
    /// fall-through to the environment's.
    #[test]
    fn resolve_at_reads_only_the_file_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nothing-here.toml");
        let paths = RuntimePaths::resolve_at(&missing);
        assert_eq!(paths.config_path, missing);
        assert!(!paths.pair_tables);
        // The implicit pair of an empty file: present, and placing nothing.
        assert_eq!(paths.known_pair_names(), ["default"]);
        assert_eq!(paths.effective_local_root("default"), None);
        assert_eq!(paths.effective_db_path("default"), None);
        assert_eq!(paths.proton_cli, "proton-drive");
    }

    /// A file the engine cannot parse names no pair — the daemon would not start on it either, and
    /// the daemon's own report is then the only source. **And the app keeps the engine's reason**
    /// (`config_error`), because "no pair" otherwise reads as "nothing configured" to every command
    /// that needs a folder, which is a different thing to tell someone whose folder IS configured
    /// and whose file has one key the daemon would refuse.
    #[test]
    fn a_file_the_engine_cannot_parse_declares_no_pairs_and_says_why() {
        let (paths, _dir) = resolved("no_such_key = 1\nlocal_root = \"/x\"\n");
        assert!(paths.pairs.is_empty());
        assert_eq!(paths.default_pair_name(), "default");
        let reason = paths
            .config_error
            .as_deref()
            .expect("the engine's reason is kept");
        assert!(reason.contains("no_such_key"), "{reason}");
        // A file the engine reads has no error to carry.
        let (fine, _dir) = resolved("local_root = \"/x\"\n");
        assert_eq!(fine.config_error, None);
    }

    /// What a command answers when it needs a folder the app cannot place: the config file's own
    /// error when that is the reason, and "not configured" only when nothing is wrong with the file.
    #[test]
    fn an_unplaceable_folder_is_blamed_on_the_file_when_the_file_is_the_reason() {
        let (broken, _dir) = resolved("no_such_key = 1\nlocal_root = \"/x\"\n");
        let said = broken.unplaced("local_root is not configured");
        assert!(said.starts_with("the config file has an error: "), "{said}");
        assert!(said.contains("no_such_key"), "{said}");
        assert!(!said.contains("not configured"), "{said}");

        let (empty, _dir) = resolved("");
        assert_eq!(
            empty.unplaced("local_root is not configured"),
            "local_root is not configured"
        );
    }

    /// A file that is not even TOML is the same case: the reason is the file's, not a missing
    /// setting.
    #[test]
    fn a_file_that_is_not_toml_is_blamed_too() {
        let (paths, _dir) = resolved("local_root = = broken\n");
        assert!(paths.pairs.is_empty());
        let said = paths.unplaced("local_root is not configured");
        assert!(said.starts_with("the config file has an error: "), "{said}");
    }

    /// The daemon's own report still fills in beside a file the engine refuses: the broken file is
    /// no reason to lose the folder a running daemon says it is syncing.
    #[test]
    fn a_running_daemons_report_still_places_a_folder_beside_a_refused_file() {
        let daemon = FakeDaemon::multi_pair(vec![FakePair::with_roots(
            "default",
            Path::new("/reported/only"),
            Path::new("/Drive/Only"),
            Path::new("/reported/only.db"),
        )])
        .start();
        let (mut paths, _dir) = resolved("no_such_key = 1\n");
        let reply = gui_core::ipc::command(
            daemon.socket_path(),
            gui_core::pairs::Target::DEFAULT,
            ControlCommand::Status,
            gui_core::ipc::DEFAULT_TIMEOUT,
        )
        .unwrap();
        paths.remember_daemon_reply(&reply);
        assert_eq!(
            paths.effective_local_root("default"),
            Some(PathBuf::from("/reported/only"))
        );
    }

    // ---- the environment's control socket is not reachable from a test (J2b) ----

    /// A file that names no `socket_path` USED to fall through to the environment's default, so a
    /// test that called `resolve_at(<temp file>)` and forgot to override the socket dialled the
    /// developer's real daemon the moment a command ran (measured: a listener at the default path
    /// received `"command":"pause"`). In a test build the default is now a refusal, so there is no
    /// socket to forget about.
    #[test]
    fn a_test_build_never_falls_back_to_the_environments_control_socket() {
        let (paths, _dir) = resolved("local_root = \"/x\"\n");
        let reason = paths
            .socket_path
            .as_ref()
            .expect_err("a test must name its socket; the environment's is not an option");
        assert!(reason.contains("test build"), "{reason}");
        // A file that names one still gets exactly that one.
        let (named, _dir) = resolved("socket_path = \"/tmp/named-by-the-file.sock\"\n");
        assert_eq!(
            named.socket_path.as_deref().ok(),
            Some(Path::new("/tmp/named-by-the-file.sock"))
        );
    }

    /// And the command layer stops at that refusal: it reports the reason, rather than dialling
    /// anything and reporting `No such file`.
    #[test]
    fn a_command_on_a_socketless_test_app_reports_the_refusal_and_dials_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve_at(&dir.path().join("proton-sync.toml"));
        let app = tauri::test::mock_builder()
            .manage(std::sync::Mutex::new(paths))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app should build");
        let payload = tauri::async_runtime::block_on(crate::commands::pause(
            app.handle().clone(),
            "default".to_owned(),
        ));
        let error = serde_json::to_value(&payload).unwrap()["error"]
            .as_str()
            .expect("no socket is an error, not a state")
            .to_owned();
        assert!(error.contains("test build"), "{error}");
        assert!(!error.contains("No such file"), "it dialled: {error}");
    }

    /// The production seam is unchanged: with nothing in the file, the default is the environment's
    /// (here, the closure standing in for it), and a file that names a socket never asks.
    #[test]
    fn outside_a_test_build_the_default_socket_is_used_exactly_when_the_file_names_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-sync.toml");
        let asked = std::cell::Cell::new(false);
        let default = || {
            asked.set(true);
            Ok(PathBuf::from("/the/default.sock"))
        };

        std::fs::write(&path, "local_root = \"/x\"\n").unwrap();
        let unnamed = RuntimePaths::resolve_with_default_socket(&path, default);
        assert_eq!(
            unnamed.socket_path.as_deref().ok(),
            Some(Path::new("/the/default.sock"))
        );
        assert!(asked.get());

        asked.set(false);
        std::fs::write(&path, "socket_path = \"/the/files.sock\"\n").unwrap();
        let named = RuntimePaths::resolve_with_default_socket(&path, default);
        assert_eq!(
            named.socket_path.as_deref().ok(),
            Some(Path::new("/the/files.sock"))
        );
        assert!(
            !asked.get(),
            "the environment is asked only when the file is silent"
        );
    }

    #[test]
    fn the_file_wins_and_the_daemon_fills_the_gaps_per_pair() {
        let daemon = FakeDaemon::multi_pair(vec![
            FakePair::with_roots(
                "photos",
                Path::new("/reported/photos"),
                Path::new("/Drive/ReportedPhotos"),
                Path::new("/reported/photos.db"),
            ),
            FakePair::with_roots(
                "docs",
                Path::new("/reported/docs"),
                Path::new("/Drive/ReportedDocs"),
                Path::new("/reported/docs.db"),
            ),
        ])
        .start();
        let (mut paths, _dir) = resolved(
            "[[pair]]\nname = \"photos\"\nlocal_root = \"/local/photos\"\n\
             [[pair]]\nname = \"docs\"\n",
        );
        let reply = gui_core::ipc::command(
            daemon.socket_path(),
            gui_core::pairs::Target::DEFAULT,
            ControlCommand::Status,
            gui_core::ipc::DEFAULT_TIMEOUT,
        )
        .unwrap();
        paths.remember_daemon_reply(&reply);
        // `photos` is placed by the file; `docs` has nothing, so the daemon's answer stands in.
        assert_eq!(
            paths.effective_local_root("photos"),
            Some(PathBuf::from("/local/photos"))
        );
        assert_eq!(
            paths.effective_local_root("docs"),
            Some(PathBuf::from("/reported/docs"))
        );
        // The file's `photos` places no db, so the DERIVED default (under the file's root) wins
        // over the daemon's: a configured pair always places its index.
        assert_eq!(
            paths.effective_db_path("photos"),
            Some(PathBuf::from("/local/photos/.sync/sync_index.db"))
        );
        assert_eq!(
            paths.effective_db_path("docs"),
            Some(PathBuf::from("/reported/docs.db"))
        );
    }

    /// A reply about ONE pair must never be filed as another's. The old single slot cached
    /// `response.config` — which describes only the pair the request addressed — as THE daemon
    /// config, so asking about `b` repainted `a`.
    #[test]
    fn a_reply_addressed_to_one_pair_never_overwrites_another_pairs_slot() {
        let daemon = FakeDaemon::multi_pair(vec![
            FakePair::with_roots(
                "a",
                Path::new("/l/a"),
                Path::new("/Drive/a"),
                Path::new("/l/a.db"),
            ),
            FakePair::with_roots(
                "b",
                Path::new("/l/b"),
                Path::new("/Drive/b"),
                Path::new("/l/b.db"),
            ),
        ])
        .start();
        let (mut paths, _dir) = resolved("");
        let ask_about = |target| {
            gui_core::ipc::command(
                daemon.socket_path(),
                target,
                ControlCommand::Status,
                gui_core::ipc::DEFAULT_TIMEOUT,
            )
            .unwrap()
        };
        let about_b = ask_about(gui_core::pairs::Target::named("b"));
        assert_eq!(about_b.pair.as_deref(), Some("b"), "the premise");
        paths.remember_daemon_reply(&about_b);
        assert_eq!(
            paths.reported("a").map(|p| p.local_root.clone()),
            Some(PathBuf::from("/l/a")),
            "a reply addressed to `b` must leave `a`'s slot holding `a`'s roots"
        );
        assert_eq!(
            paths.reported("b").map(|p| p.local_root.clone()),
            Some(PathBuf::from("/l/b"))
        );
        assert_eq!(paths.daemon.capability, PairCapability::MultiPair);
        // And then the other way round.
        paths.remember_daemon_reply(&ask_about(gui_core::pairs::Target::DEFAULT));
        assert_eq!(
            paths.reported("b").map(|p| p.db_path.clone()),
            Some(PathBuf::from("/l/b.db"))
        );
    }

    /// A daemon that predates the list reports its one pair as `config`, and that is filed under a
    /// name — `default` unless the reply says otherwise — so the same lookups work against it.
    #[test]
    fn a_legacy_reply_is_filed_as_the_one_pair_called_default() {
        let daemon = FakeDaemon::legacy(FakePair::with_roots(
            "ignored-by-a-legacy-reply",
            Path::new("/l/only"),
            Path::new("/Drive/only"),
            Path::new("/l/only.db"),
        ))
        .start();
        let reply = gui_core::ipc::command(
            daemon.socket_path(),
            gui_core::pairs::Target::DEFAULT,
            ControlCommand::Status,
            gui_core::ipc::DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert_eq!(PairCapability::from_reply(&reply), PairCapability::Legacy);
        let (mut paths, _dir) = resolved("");
        paths.remember_daemon_reply(&reply);
        assert_eq!(paths.daemon.capability, PairCapability::Legacy);
        assert_eq!(paths.known_pair_names(), ["default"]);
        assert_eq!(
            paths.effective_local_root("default"),
            Some(PathBuf::from("/l/only"))
        );
    }

    /// `PairRef`: the default pair has a name and no selector; any other pair has both; an unknown
    /// name is refused and never read as the default.
    #[test]
    fn a_command_is_addressed_by_omission_for_the_default_pair_and_refused_for_an_unknown_one() {
        let (paths, _dir) = resolved(TWO_PAIRS);
        let omitted = paths.resolve_pair(None).unwrap();
        assert_eq!(
            omitted,
            PairRef {
                name: "photos".into(),
                selector: None
            }
        );
        // Naming the default pair explicitly is the same request as naming none.
        assert_eq!(paths.resolve_pair(Some("photos")).unwrap(), omitted);
        assert_eq!(
            paths.resolve_pair(Some("docs")).unwrap(),
            PairRef {
                name: "docs".into(),
                selector: Some("docs".into())
            }
        );
        let error = paths.resolve_pair(Some("Docs")).unwrap_err();
        assert!(
            error.contains("\"Docs\"") && error.contains("\"photos\""),
            "{error}"
        );
        // Nothing configured and nothing heard: only the default is addressable.
        let (nothing, _dir) = resolved("no_such_key = 1\n");
        assert_eq!(nothing.resolve_pair(None).unwrap().name, "default");
        assert!(nothing.resolve_pair(Some("default")).is_err());
    }
}
