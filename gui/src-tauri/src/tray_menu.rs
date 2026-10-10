//! The tray's rows, in one table (S8 follow-up, #252).
//!
//! # Why this is a module and not two `match`es
//!
//! There are now THREE surfaces drawing the same menu: the compact panel's menu section
//! (`ui/compact.js`'s `TRAY_MENU`), the native right-click menu (`dbusmenu.rs`), and the text menu a
//! session with no status-notifier host falls back to (`tray.rs`). #252 asked for the third to be
//! built from a shared table rather than a third copy, and the reason is on record twice: S8 found
//! the fallback menu and the panel dispatching `sync_now` against `syncNow` — two vocabularies that
//! each worked, under a comment claiming they were one — and `commands.rs` still carried a doc
//! comment pointing at a `FALLBACK_IDS` table that never existed. This is that table.
//!
//! The panel's copy is still `ui/copy.js`'s `TRAY` block, because the panel is a DOM the copy gate
//! reads. This side is Rust and no gate can see it, so `the_labels_are_the_copy_deck_s` reads that
//! file and compares — the drift the id test cannot catch, since ids drifting is a dead row and
//! labels drifting is a row that lies.
//!
//! # The numeric id is the ACTION, not the position
//!
//! dbusmenu identifies a row by an `i32`, and a host holds the layout it was given until it is told
//! otherwise. The rows here change with the daemon's state, so a menu that opened before a state
//! change is still on screen with the ids it was built from — and if those ids were positions, the
//! click would arrive as whatever now stands where the user pressed.
//!
//! The worst of those collisions is the pair `10-tray.md` cares most about: **`Close window — keeps
//! syncing` and `Quit — stops syncing` stand in the same place in different states.** `Close window`
//! is 5th while settled and 4th while syncing; `Quit` is 5th while syncing and 4th while paused. So
//! a menu opened on an idle daemon and clicked once a pass had started would have stopped the
//! daemon, on the row whose whole job is to say that it will not — and settled→syncing happens on
//! every pass. (`positions_collide_and_the_worst_pair_is_the_one_10_tray_md_names` walks every set
//! and fails if that stops being true, because it is the reason for this design.)
//!
//! So the id travels with the row. A stale menu dispatches the action its label promised, or none.
//!
//! # A folder's row names the folder (#102 phase 5d)
//!
//! With two folders or more the one `Pause syncing` row becomes one row per folder, and a row's
//! action then includes WHICH folder: `pause@photos`, `resume@photos`. The `@` is outside the
//! folder-name charset (`[A-Za-z0-9._-]`), so a name can never forge an action, and the panel and
//! the fallback menu carry that string as the row's id.
//!
//! A native row still needs a number. Those come from a process-lifetime registry that issues a
//! fresh `i32` per `(action, folder name)` the first time it is asked for one and **never reuses or
//! reassigns it**. That is the whole point of it: a number that meant `Pause photos` when the menu
//! was drawn still means `Pause photos` after the folder list has changed under it, so a stale click
//! lands on the folder its label named — and if that folder no longer exists, the daemon's own
//! byte-exact selector rule answers "no such pair" and nothing is paused. A click is never read as
//! a position in the list, and never as "whichever folder is first now".
//!
//! There is NO `Pause all` row, and no daemon-wide state behind one: every folder has its own pause
//! (maintainer ruling, issue #102).

use gui_core::state::{pair_states, DaemonState};
use gui_core::wire::ControlResponse;
use std::borrow::Cow;
use std::sync::Mutex;

/// One row, or the rule between two groups of them.
///
/// `id` is the vocabulary `commands::tray_row` dispatches — the same strings `ui/compact.js` sends
/// from the panel. `dbus_id` is what a native menu row is called on the wire.
///
/// Owned strings (`Cow`) since folders arrived: a folder's row carries the folder's name, so the
/// table can no longer be all `&'static`. The static rows are still `const`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Row {
        id: Cow<'static, str>,
        dbus_id: i32,
        label: Cow<'static, str>,
        /// The second, quieter line the panel draws under the label. `10-tray.md` calls the two rows
        /// that carry one "the single worst misunderstanding a tray app can cause".
        sub: Option<Cow<'static, str>>,
    },
    /// A rule carries no action. It has an id because a layout cannot hold two items with one:
    /// the first rule is `SEPARATOR_ID` (the only one any set had before folders) and the second,
    /// which only the folder group's closing rule uses, is `SECOND_SEPARATOR_ID`.
    Separator { dbus_id: i32 },
}

/// The rule every set had before folders arrived. A separator is not an action, so it resolves to
/// none (`action_for_dbus_id`).
pub const SEPARATOR_ID: i32 = 90;
/// The rule that closes the folder group, in the sets that have one.
pub const SECOND_SEPARATOR_ID: i32 = 91;

impl Entry {
    /// What a NATIVE menu row says. Both native menus are one string per row — a GTK menu item and a
    /// `QAction` alike — so the sub-label folds in behind an em-dash instead of becoming the second
    /// baseline-aligned span the panel draws. What it must not do is lose the words. DEVIATIONS §82k.
    pub fn folded_label(&self) -> String {
        match self {
            Entry::Row {
                label,
                sub: Some(sub),
                ..
            } => format!("{label} — {sub}"),
            Entry::Row { label, .. } => label.to_string(),
            Entry::Separator { .. } => String::new(),
        }
    }

    pub fn dbus_id(&self) -> i32 {
        match self {
            Entry::Row { dbus_id, .. } | Entry::Separator { dbus_id } => *dbus_id,
        }
    }

    /// The action this row dispatches, or `None` for a rule. The tests read menus through it; the
    /// menus themselves dispatch by number (`action_for_dbus_id`) or by id (`tray_row`).
    #[cfg(test)]
    pub fn action(&self) -> Option<&str> {
        match self {
            Entry::Row { id, .. } => Some(id),
            Entry::Separator { .. } => None,
        }
    }
}

// The rows themselves. Shared consts rather than per-set literals, so an action has one id and one
// label everywhere it appears — which is what makes the id stable across a state change.
const OPEN: Entry = Entry::Row {
    id: Cow::Borrowed("open"),
    dbus_id: 1,
    label: Cow::Borrowed("Open Drive Sync"),
    sub: None,
};
const SYNC_NOW: Entry = Entry::Row {
    id: Cow::Borrowed("syncNow"),
    dbus_id: 2,
    label: Cow::Borrowed("Sync now"),
    sub: None,
};
const PAUSE: Entry = Entry::Row {
    id: Cow::Borrowed("pause"),
    dbus_id: 3,
    label: Cow::Borrowed("Pause syncing"),
    sub: None,
};
const RESUME: Entry = Entry::Row {
    id: Cow::Borrowed("resume"),
    dbus_id: 4,
    label: Cow::Borrowed("Resume syncing"),
    sub: None,
};
const TRY_AGAIN: Entry = Entry::Row {
    id: Cow::Borrowed("tryAgain"),
    dbus_id: 5,
    label: Cow::Borrowed("Try again now"),
    sub: None,
};
/// Starts the daemon rather than talking to it — the only row here that is not a control command.
///
/// Its own id and its own `dbus_id`, on the rule this module is built on: `tryAgain` could not have
/// become "start the service when unreachable, retry the sync when failed" without making an id mean
/// two things, which is precisely the stale-menu collision the header describes. `8` because 1–7 and
/// the separator's 90 are taken.
const START: Entry = Entry::Row {
    id: Cow::Borrowed("start"),
    dbus_id: 8,
    label: Cow::Borrowed("Start the sync service"),
    sub: None,
};
const CLOSE_WINDOW: Entry = Entry::Row {
    id: Cow::Borrowed("closeWindow"),
    dbus_id: 6,
    label: Cow::Borrowed("Close window"),
    sub: Some(Cow::Borrowed("keeps syncing")),
};
const QUIT: Entry = Entry::Row {
    id: Cow::Borrowed("quit"),
    dbus_id: 7,
    label: Cow::Borrowed("Quit"),
    sub: Some(Cow::Borrowed("stops syncing")),
};
const SEP: Entry = Entry::Separator {
    dbus_id: SEPARATOR_ID,
};
const SEP_AFTER_FOLDERS: Entry = Entry::Separator {
    dbus_id: SECOND_SEPARATOR_ID,
};

/// One folder, as the tray needs it to draw a menu: its name, the two flags the rows are chosen by,
/// and where it ranks when folders disagree (`gui_core::state::severity`, the one rank table).
///
/// A plain value with no `DaemonState` in it on purpose: the menu depends on exactly these four
/// facts, so those are what `Shown` compares and what a stale-menu test has to vary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPair {
    pub name: String,
    pub paused: bool,
    pub syncing: bool,
    /// Higher is worse. The folder group lists the worst first.
    pub rank: u8,
}

/// The folders a status reply lists, as the tray draws them. Empty for a daemon that lists none (one
/// that predates the selector), which is how every caller below tells "legacy" from "two folders".
pub fn pairs_of(response: &ControlResponse, described: DaemonState) -> Vec<TrayPair> {
    response
        .pairs
        .iter()
        .zip(pair_states(response, described))
        .map(|(summary, state)| TrayPair {
            name: summary.name.clone(),
            paused: summary.paused,
            syncing: summary.syncing,
            rank: state.rank,
        })
        .collect()
}

/// What a folder row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairAction {
    Pause,
    Resume,
}

impl PairAction {
    pub fn word(self) -> &'static str {
        match self {
            PairAction::Pause => "pause",
            PairAction::Resume => "resume",
        }
    }

    /// The row's id in the panel's and the fallback's vocabulary: `pause@photos`.
    pub fn row_id(self, name: &str) -> String {
        format!("{}@{name}", self.word())
    }

    /// `Pause photos` / `Resume photos` — decision D11, and `TRAY.pausePair`/`TRAY.resumePair`'s text
    /// (`the_labels_are_the_copy_deck_s` holds the two together).
    fn label(self, name: &str) -> String {
        match self {
            PairAction::Pause => format!("Pause {name}"),
            PairAction::Resume => format!("Resume {name}"),
        }
    }
}

/// The first number a folder row is issued. Clear of the static rows (1–8) and the rules (90, 91).
const FIRST_FOLDER_ROW_ID: i32 = 1000;

/// Every `(action, folder)` the tray has ever drawn a native row for, and the number it was given.
///
/// **Append-only and never reassigned**, which is what makes a native id an action and not a
/// position (see the module header). The number of entries is bounded by the number of distinct
/// folder names this process has ever seen, times two — a handful, in a process whose folder list
/// changes only when a person edits their config.
struct Registry {
    next: i32,
    issued: Vec<(PairAction, String, i32)>,
}

impl Registry {
    const fn new() -> Self {
        Self {
            next: FIRST_FOLDER_ROW_ID,
            issued: Vec::new(),
        }
    }

    /// The number for `(action, name)`, issuing a fresh one the first time.
    fn id_for(&mut self, action: PairAction, name: &str) -> i32 {
        if let Some((_, _, id)) = self
            .issued
            .iter()
            .find(|(a, n, _)| *a == action && n == name)
        {
            return *id;
        }
        let id = self.next;
        self.next += 1;
        self.issued.push((action, name.to_owned(), id));
        id
    }

    /// What a number was issued for. `None` for one that never was.
    fn lookup(&self, dbus_id: i32) -> Option<(PairAction, &str)> {
        self.issued
            .iter()
            .find(|(_, _, id)| *id == dbus_id)
            .map(|(action, name, _)| (*action, name.as_str()))
    }
}

/// The process's registry. One, because the native menu and the fallback must agree on what a
/// number means, and a click arrives long after the menu that issued it was replaced.
static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

fn folder_row(action: PairAction, name: &str) -> Entry {
    let dbus_id = REGISTRY
        .lock()
        .expect("the row registry is never held across a panic")
        .id_for(action, name);
    Entry::Row {
        id: Cow::Owned(action.row_id(name)),
        dbus_id,
        label: Cow::Owned(action.label(name)),
        sub: None,
    }
}

// The sets at ONE folder and for a daemon that lists none, and they are `ui/compact.js`'s:
// `settled`, `syncing`, `paused`, `outage`, `notRunning` and `deferToWindow`. `needsYou` is not one
// of them — the panel's needs-you list is `settled`'s rows, because `Review them` is the panel's
// own decision button rather than a menu row.
//
// Functions, not `const` slices: a row is no longer `Copy`, and these are built per call.
fn settled() -> Vec<Entry> {
    vec![OPEN, SYNC_NOW, PAUSE, SEP, CLOSE_WINDOW, QUIT]
}
// `Sync now` is absent while a pass is running: it would do nothing.
fn syncing() -> Vec<Entry> {
    vec![OPEN, PAUSE, SEP, CLOSE_WINDOW, QUIT]
}
// The states that are not moving files lead with the row that fixes them and drop `Close window` —
// with nothing syncing, `keeps syncing` would be a lie.
fn paused() -> Vec<Entry> {
    vec![RESUME, OPEN, SEP, QUIT]
}
/// PROTON is out of reach and the daemon is answering, which is `DaemonState::Failed`.
///
/// Called `UNREACHABLE` until it was not: it served `Unreachable` too, and there `Try again now`
/// dispatches `Syncnow` at a control socket that did not answer — a row that cannot do the thing its
/// label promises, on the one menu `10-tray.md` asks to be honest above all else. The name went with
/// the state that kept it.
fn outage() -> Vec<Entry> {
    vec![TRY_AGAIN, OPEN, SEP, QUIT]
}
/// The DAEMON is not running, which is what `DaemonState::Unreachable` means: the control socket did
/// not answer. Nothing to retry against, so the row that fixes it starts the service. No frame draws
/// this set — `10-tray.md`'s table has no such state. DEVIATIONS §95.
fn not_running() -> Vec<Entry> {
    vec![START, OPEN, SEP, QUIT]
}
// An expired session and a daemon that has never synced are both fixed in the window, not by
// retrying a sync. The panel is keyed by FORM (both wear the struck mark) and the menu by CAUSE.
// DEVIATIONS §82g.
fn defer_to_window() -> Vec<Entry> {
    vec![OPEN, SEP, QUIT]
}

/// The rows for a state at one folder, or for a daemon that lists none — **exactly the rows this
/// menu has always had**, ids and labels included (`n1_rows_are_todays_rows`).
fn single_folder_rows(state: DaemonState) -> Vec<Entry> {
    match state {
        // `Queued` is only derived beside other folders, so this arm is for completeness; it takes
        // the rows of the state it replaces (an idle folder), which is what a lone folder would have.
        DaemonState::Idle | DaemonState::Queued => settled(),
        DaemonState::Running => syncing(),
        DaemonState::Paused => paused(),
        // THESE TWO WERE ONE ARM, and splitting them is the fix. `Try again now` is unambiguously
        // right for `Failed` — the daemon is ANSWERING, so a retry reaches it — and was a dead
        // control for `Unreachable`, where the control socket is what did not respond. The panel
        // groups both with `authExpired` by form and the menu parts them by cause, as above.
        DaemonState::Unreachable => not_running(),
        DaemonState::Failed => outage(),
        DaemonState::AuthExpired | DaemonState::FirstRun => defer_to_window(),
    }
}

/// The rows for a state and a folder list. The same mapping `screens/tray.js` makes for the panel,
/// made once here for both native menus.
///
/// **Below two folders this is `single_folder_rows`**: the one `Pause syncing` row, `Sync now` chosen
/// by state alone. At two or more the pause row becomes the folder group — one row per folder, the
/// worst-ranked first, `Pause {name}` or `Resume {name}` by that folder's own flag — between two rules:
///
/// ```text
/// Open Drive Sync
/// Sync now                       only while some UNPAUSED folder is idle
/// ─────
/// Pause documents                 Resume documents, while that folder is paused
/// Pause photos
/// ─────
/// Close window   keeps syncing    only while some folder is unpaused
/// Quit           stops syncing
/// ```
///
/// It is chosen from the FOLDER LIST, not from the aggregate state alone (decision D5): the aggregate
/// is `Running` as soon as any folder syncs, and a state-keyed set would then take `Sync now` away
/// from the idle folders for as long as another one's pass lasts. The group is absent where there is
/// nothing to pause or no daemon to send to — an expired session, a folder that has never synced,
/// a daemon that is not running — exactly the sets that never carried `Pause syncing`.
pub fn rows_for(state: DaemonState, pairs: &[TrayPair]) -> Vec<Entry> {
    if pairs.len() < 2 {
        return single_folder_rows(state);
    }
    match state {
        // A folder waiting for its turn has all the controls an idle one has: it can be paused, and
        // `Sync now` has somebody to wake.
        DaemonState::Idle | DaemonState::Running | DaemonState::Queued | DaemonState::Paused => {
            let mut rows = vec![OPEN];
            // Some unpaused folder is idle: `Sync now` has somebody to wake. Not the aggregate state,
            // which says only that SOMETHING is syncing.
            if pairs.iter().any(|pair| !pair.paused && !pair.syncing) {
                rows.push(SYNC_NOW);
            }
            rows.push(SEP);
            rows.extend(folder_group(pairs));
            rows.push(SEP_AFTER_FOLDERS);
            // `keeps syncing` is a promise, and it is true only while some folder is unpaused.
            if pairs.iter().any(|pair| !pair.paused) {
                rows.push(CLOSE_WINDOW);
            }
            rows.push(QUIT);
            rows
        }
        // A folder can fail while another runs and can be paused, so the retry set gains the group.
        DaemonState::Failed => {
            let mut rows = vec![TRY_AGAIN, OPEN, SEP];
            rows.extend(folder_group(pairs));
            rows.push(SEP_AFTER_FOLDERS);
            rows.push(QUIT);
            rows
        }
        DaemonState::Unreachable | DaemonState::AuthExpired | DaemonState::FirstRun => {
            single_folder_rows(state)
        }
    }
}

/// One row per folder, the worst first and ties in the order the daemon lists them (a stable sort).
fn folder_group(pairs: &[TrayPair]) -> Vec<Entry> {
    let mut ordered: Vec<&TrayPair> = pairs.iter().collect();
    ordered.sort_by_key(|pair| std::cmp::Reverse(pair.rank));
    ordered
        .into_iter()
        .map(|pair| {
            folder_row(
                if pair.paused {
                    PairAction::Resume
                } else {
                    PairAction::Pause
                },
                &pair.name,
            )
        })
        .collect()
}

/// What a numeric id means, looked up across EVERY set rather than the one on screen — and across
/// every folder row the registry has ever issued.
///
/// This is the stale-menu case the module header describes, and it is why the lookup is not
/// `rows_for(current_state).iter().find(...)`: the click that arrives may be on a menu built two
/// seconds and one state change ago, and the row the user pressed is the row that must run. An id
/// no set has ever drawn returns `None` and the caller says so.
///
/// The answer is the row's id in the shared vocabulary (`pause@photos` for a folder's row), which
/// `commands::tray_row` parses.
pub fn action_for_dbus_id(dbus_id: i32) -> Option<String> {
    let fixed = ALL_STATES
        .iter()
        .flat_map(|state| single_folder_rows(*state))
        .find_map(|entry| match entry {
            Entry::Row {
                id, dbus_id: at, ..
            } if at == dbus_id => Some(id.into_owned()),
            _ => None,
        });
    fixed.or_else(|| {
        REGISTRY
            .lock()
            .expect("the row registry is never held across a panic")
            .lookup(dbus_id)
            .map(|(action, name)| action.row_id(name))
    })
}

/// Every state, for the tests and for anything that has to walk the whole table.
///
/// `action_for_dbus_id` walks it to answer a click, so a variant missing here is a row whose id
/// resolves to nothing — pinned by `all_states_lists_every_variant` below.
pub const ALL_STATES: &[DaemonState] = &[
    DaemonState::Idle,
    DaemonState::Running,
    DaemonState::Queued,
    DaemonState::Paused,
    DaemonState::Unreachable,
    DaemonState::AuthExpired,
    DaemonState::FirstRun,
    DaemonState::Failed,
];

/// Status replies for the tests that drive the tray from a reply (`pairs_of`, and `tray.rs`'s
/// `observe`): built from the raw per-folder flags the daemon sends, so what is asserted is the whole
/// path from "this folder is paused" on the wire to a row in a menu.
#[cfg(test)]
pub(crate) mod fixtures {
    use gui_core::wire::{ControlResponse, PairSummary};

    /// One folder of a reply, by the facts its state is derived from.
    #[derive(Clone)]
    pub struct Folder {
        pub name: &'static str,
        pub paused: bool,
        pub syncing: bool,
        pub failed: bool,
        pub queued: usize,
        /// No finished pass in this run: the daemon starts every folder so, and only a pass sets it.
        pub unsynced: bool,
    }

    impl Folder {
        pub fn new(name: &'static str) -> Self {
            Self {
                name,
                paused: false,
                syncing: false,
                failed: false,
                queued: 0,
                unsynced: false,
            }
        }
        pub fn unsynced(mut self) -> Self {
            self.unsynced = true;
            self
        }
        pub fn paused(mut self) -> Self {
            self.paused = true;
            self
        }
        pub fn syncing(mut self) -> Self {
            self.syncing = true;
            self
        }
        pub fn failed(mut self) -> Self {
            self.failed = true;
            self
        }
    }

    fn summary(folder: &Folder) -> PairSummary {
        PairSummary {
            name: folder.name.to_owned(),
            local_root: format!("/home/u/{}", folder.name).into(),
            remote_root: format!("/Drive/{}", folder.name).into(),
            db_path: format!("/home/u/{}/.sync/i.db", folder.name).into(),
            paused: folder.paused,
            syncing: folder.syncing,
            reconcile_seq: 1,
            last_sync_epoch_secs: (!folder.unsynced).then_some(1),
            last_error: folder.failed.then(|| "boom".to_owned()),
            pending_changes: folder.queued,
            pending_deletions: 0,
        }
    }

    /// A status reply listing `folders`, the first of which it describes (the default pair): its
    /// top-level fields are that folder's, as a real reply's are.
    pub fn reply(folders: &[Folder]) -> ControlResponse {
        let first = &folders[0];
        ControlResponse {
            status: "running".into(),
            paused: first.paused,
            syncing: first.syncing,
            reconcile_seq: 1,
            pending_changes: first.queued,
            message: String::new(),
            pause_unsaved: None,
            last_sync_epoch_secs: (!first.unsynced).then_some(1),
            last_error: first.failed.then(|| "boom".to_owned()),
            last_plan_summary: None,
            last_successful_sync_summary: None,
            status_history: Vec::new(),
            pending_deletions: Vec::new(),
            failed_items: Vec::new(),
            failed_item_count: 0,
            config: None,
            activity: None,
            unsyncable: Vec::new(),
            history: None,
            file_history: None,
            index_totals: None,
            listing: None,
            plan: None,
            apply: None,
            auth: Default::default(),
            pair: Some(first.name.to_owned()),
            pairs: folders.iter().map(summary).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{reply, Folder};
    use super::*;
    use gui_core::state::severity;
    use std::collections::{HashMap, HashSet};

    /// A folder as the menu sees it, with the rank its state gets from the one rank table.
    fn folder(name: &str, state: DaemonState, paused: bool, syncing: bool) -> TrayPair {
        TrayPair {
            name: name.to_owned(),
            paused,
            syncing,
            rank: severity(state),
        }
    }

    fn idle(name: &str) -> TrayPair {
        folder(name, DaemonState::Idle, false, false)
    }

    fn rows(state: DaemonState) -> Vec<Entry> {
        rows_for(state, &[])
    }

    fn ids(rows: &[Entry]) -> Vec<String> {
        rows.iter()
            .map(|entry| entry.action().unwrap_or("—").to_owned())
            .collect()
    }

    /// The enum as a chain: every variant names the next one and the last names `None`.
    ///
    /// A `Vec<DaemonState>` built by hand would be the third hand-written copy of the enum in this
    /// file. This one is a MATCH, so a variant added to `DaemonState` stops it compiling — which is
    /// the only construction available here that a later author cannot satisfy by doing nothing.
    fn chain() -> Vec<DaemonState> {
        fn next(state: DaemonState) -> Option<DaemonState> {
            match state {
                DaemonState::Idle => Some(DaemonState::Running),
                DaemonState::Running => Some(DaemonState::Queued),
                DaemonState::Queued => Some(DaemonState::Paused),
                DaemonState::Paused => Some(DaemonState::Unreachable),
                DaemonState::Unreachable => Some(DaemonState::AuthExpired),
                DaemonState::AuthExpired => Some(DaemonState::FirstRun),
                DaemonState::FirstRun => Some(DaemonState::Failed),
                DaemonState::Failed => None,
            }
        }
        let mut all = vec![DaemonState::Idle];
        while let Some(state) = next(*all.last().expect("seeded")) {
            all.push(state);
        }
        all
    }

    #[test]
    fn all_states_lists_every_variant() {
        // `ALL_STATES` is a hand-written copy of an enum, and three tests plus `action_for_dbus_id`
        // walk it. A state missing from it has untested rows and an id the click lookup cannot
        // resolve — a menu row that does nothing when pressed, on the surface nobody inspects.
        for state in chain() {
            assert!(
                ALL_STATES.contains(&state),
                "{state:?} is a DaemonState variant that ALL_STATES does not list"
            );
        }
        assert_eq!(
            ALL_STATES.len(),
            chain().len(),
            "ALL_STATES lists something twice, or something the enum no longer has"
        );
    }

    /// The folder lists the tests below walk every state against: none, one, two, a mixed three.
    fn folder_lists() -> Vec<Vec<TrayPair>> {
        vec![
            vec![],
            vec![idle("docs")],
            vec![idle("documents"), idle("photos")],
            vec![
                folder("documents", DaemonState::Paused, true, false),
                idle("photos"),
            ],
            vec![
                folder("a", DaemonState::Running, false, true),
                folder("b", DaemonState::Failed, false, false),
                folder("c", DaemonState::Paused, true, false),
            ],
        ]
    }

    #[test]
    fn every_row_in_a_set_has_its_own_id() {
        // Two rows sharing an id in one menu is a click that dispatches the wrong one — and a layout
        // with two items of one id is a malformed layout. Rules used to share one id because no set
        // had two; the folder group's closing rule has its own, and this pins that for every state
        // against every folder list.
        for state in ALL_STATES {
            for pairs in folder_lists() {
                let mut seen = HashSet::new();
                for entry in rows_for(*state, &pairs) {
                    assert!(
                        seen.insert(entry.dbus_id()),
                        "{state:?} with {} folders has two rows with id {}",
                        pairs.len(),
                        entry.dbus_id()
                    );
                }
            }
        }
    }

    #[test]
    fn an_id_means_the_same_action_in_every_state() {
        // THE STALE-MENU RACE. A host keeps the layout it was given until `LayoutUpdated` reaches
        // it; the poll changes the rows every two seconds. An id that meant a POSITION would
        // therefore mean a different row than the one under the pointer — see the test below for
        // which pair that costs most.
        let mut action: HashMap<i32, String> = HashMap::new();
        for state in ALL_STATES {
            for pairs in folder_lists() {
                for entry in rows_for(*state, &pairs) {
                    if let Entry::Row { id, dbus_id, .. } = entry {
                        let seen = action.entry(dbus_id).or_insert_with(|| id.to_string());
                        assert_eq!(*seen, *id, "id {dbus_id} means two different things");
                    }
                }
            }
        }
        // And the other direction: one action, one number, or the same row is two rows to a host.
        let ids: HashSet<_> = action.values().collect();
        assert_eq!(ids.len(), action.len(), "an action has two ids");
    }

    #[test]
    fn positions_collide_and_the_worst_pair_is_the_one_10_tray_md_names() {
        // THE MEASUREMENT BEHIND THE DESIGN, rather than a sentence asserting it. Walk every pair of
        // sets and every position, and collect the places where the same position carries a
        // different action — those are the mis-dispatches a positional id would produce.
        //
        // The first version of this comment named the wrong pair (it said a stale `Pause syncing`
        // click would land on `Quit`; the paused set's third row is the separator, so it would have
        // landed on nothing). The real worst case is worse, which is why the test computes it.
        let mut collisions = Vec::new();
        for a in ALL_STATES {
            for b in ALL_STATES {
                let (left, right) = (rows(*a), rows(*b));
                for index in 0..left.len().min(right.len()) {
                    if let (Some(x), Some(y)) = (left[index].action(), right[index].action()) {
                        if x != y {
                            collisions.push((x.to_owned(), y.to_owned()));
                        }
                    }
                }
            }
        }
        assert!(
            !collisions.is_empty(),
            "no position ever changes meaning, so this design has no reason to exist"
        );
        assert!(
            collisions.contains(&("closeWindow".to_owned(), "quit".to_owned())),
            "`Close window — keeps syncing` and `Quit — stops syncing` no longer share a position. \
             That is the example the module header and DEVIATIONS §89b give for why an id is an \
             action; fix the prose to name whichever pair collides now, rather than this test. \
             Collisions found: {collisions:?}"
        );
    }

    #[test]
    fn no_row_claims_the_root_s_id() {
        // `0` is the root of a dbusmenu layout. A row calling itself 0 is a row a host will treat
        // as the menu it belongs to.
        for state in ALL_STATES {
            for pairs in folder_lists() {
                for entry in rows_for(*state, &pairs) {
                    assert_ne!(entry.dbus_id(), 0, "{state:?} has a row with the root's id");
                }
            }
        }
    }

    #[test]
    fn every_row_is_an_action_this_build_can_perform() {
        // The other half of `tray.rs`'s `both_indicators_speak_one_vocabulary`: that test proves the
        // ids the PANEL sends are known, this one proves the ids the NATIVE menus send are.
        for state in ALL_STATES {
            for pairs in folder_lists() {
                for entry in rows_for(*state, &pairs) {
                    if let Some(id) = entry.action() {
                        assert!(
                            crate::commands::tray_row(id).is_some(),
                            "{state:?} draws {id:?} and nothing dispatches it"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_state_that_is_not_syncing_never_offers_to_keep_syncing() {
        // `Close window — keeps syncing` on a paused daemon is a label that does not do what it
        // says, which is the one thing `10-tray.md` asks of these rows.
        for state in [
            DaemonState::Paused,
            DaemonState::Unreachable,
            DaemonState::AuthExpired,
            DaemonState::FirstRun,
        ] {
            assert!(
                !ids(&rows(state)).contains(&"closeWindow".to_owned()),
                "{state:?} offers `Close window — keeps syncing`"
            );
        }
        // And with folders: when EVERY folder is paused, nothing keeps syncing.
        let all_paused = [
            folder("a", DaemonState::Paused, true, false),
            folder("b", DaemonState::Paused, true, false),
        ];
        assert!(
            !ids(&rows_for(DaemonState::Paused, &all_paused)).contains(&"closeWindow".to_owned())
        );
    }

    #[test]
    fn every_set_can_be_left() {
        // A tray menu with no way out is the failure mode of a menu built per state: each one is
        // written on its own and the one written last forgets.
        for state in ALL_STATES {
            for pairs in folder_lists() {
                let ids = ids(&rows_for(*state, &pairs));
                assert!(ids.contains(&"quit".to_owned()), "{state:?} cannot be quit");
                assert!(
                    ids.contains(&"open".to_owned()),
                    "{state:?} cannot open the window"
                );
            }
        }
    }

    #[test]
    fn the_sub_labels_fold_rather_than_vanish() {
        let quit = QUIT.folded_label();
        assert_eq!(quit, "Quit — stops syncing");
        assert_eq!(CLOSE_WINDOW.folded_label(), "Close window — keeps syncing");
        assert!(SEP.folded_label().is_empty());
    }

    // ---- folders (#102 phase 5d) -----------------------------------------------------------------

    /// One row, as `(id, dbus_id, label, sub)` — the shape the table below is written in.
    type Wanted = (&'static str, i32, &'static str, Option<&'static str>);

    /// Today's rows, written out as DATA and not derived from the consts above: the test compares the
    /// menu against this, so a const that changed is a failure and not an agreement with itself.
    /// Taken from `main` at 60f5e3c (`tray_menu.rs`, before folders).
    fn todays_rows(state: DaemonState) -> Vec<Option<Wanted>> {
        const OPEN: Wanted = ("open", 1, "Open Drive Sync", None);
        const SYNC_NOW: Wanted = ("syncNow", 2, "Sync now", None);
        const PAUSE: Wanted = ("pause", 3, "Pause syncing", None);
        const RESUME: Wanted = ("resume", 4, "Resume syncing", None);
        const TRY_AGAIN: Wanted = ("tryAgain", 5, "Try again now", None);
        const START: Wanted = ("start", 8, "Start the sync service", None);
        const CLOSE: Wanted = ("closeWindow", 6, "Close window", Some("keeps syncing"));
        const QUIT: Wanted = ("quit", 7, "Quit", Some("stops syncing"));
        let some = Some;
        match state {
            // `Queued` is never derived at one folder; it is given the rows of the state it replaces.
            DaemonState::Idle | DaemonState::Queued => vec![
                some(OPEN),
                some(SYNC_NOW),
                some(PAUSE),
                None,
                some(CLOSE),
                some(QUIT),
            ],
            DaemonState::Running => vec![some(OPEN), some(PAUSE), None, some(CLOSE), some(QUIT)],
            DaemonState::Paused => vec![some(RESUME), some(OPEN), None, some(QUIT)],
            DaemonState::Failed => vec![some(TRY_AGAIN), some(OPEN), None, some(QUIT)],
            DaemonState::Unreachable => vec![some(START), some(OPEN), None, some(QUIT)],
            DaemonState::AuthExpired | DaemonState::FirstRun => {
                vec![some(OPEN), None, some(QUIT)]
            }
        }
    }

    fn assert_todays_rows(state: DaemonState, rows: &[Entry], what: &str) {
        let wanted = todays_rows(state);
        assert_eq!(rows.len(), wanted.len(), "{state:?} {what}: {rows:?}");
        for (entry, want) in rows.iter().zip(wanted) {
            match (entry, want) {
                (Entry::Separator { dbus_id }, None) => {
                    assert_eq!(*dbus_id, SEPARATOR_ID, "{state:?} {what}")
                }
                (
                    Entry::Row {
                        id,
                        dbus_id,
                        label,
                        sub,
                    },
                    Some((want_id, want_dbus, want_label, want_sub)),
                ) => {
                    assert_eq!(id.as_ref(), want_id, "{state:?} {what}");
                    assert_eq!(*dbus_id, want_dbus, "{state:?} {what}: {want_id}'s dbus id");
                    assert_eq!(label.as_ref(), want_label, "{state:?} {what}");
                    assert_eq!(sub.as_deref(), want_sub, "{state:?} {what}");
                }
                (entry, want) => panic!("{state:?} {what}: {entry:?} is not {want:?}"),
            }
        }
    }

    /// D2, and acceptance 1: at one folder and for a daemon that lists none, every row, id and label
    /// is exactly today's. Revert: always emit the folder group.
    #[test]
    fn n1_rows_are_todays_rows() {
        for state in ALL_STATES {
            assert_todays_rows(
                *state,
                &rows_for(*state, &[]),
                "for a daemon that lists none",
            );
            for flags in [(false, false), (false, true), (true, false)] {
                let one = [folder("docs", *state, flags.0, flags.1)];
                assert_todays_rows(*state, &rows_for(*state, &one), "at one folder");
            }
        }
    }

    fn labels_of(rows: &[Entry]) -> Vec<String> {
        rows.iter()
            .filter_map(|entry| match entry {
                Entry::Row { label, .. } => Some(label.to_string()),
                Entry::Separator { .. } => None,
            })
            .collect()
    }

    /// Acceptance 2: two folders, one paused — `Resume` for it, `Pause` for the other, and no row
    /// anywhere says `Pause syncing`.
    #[test]
    fn a_paused_folder_offers_resume_and_the_other_offers_pause() {
        let pairs = [
            folder("documents", DaemonState::Paused, true, false),
            idle("photos"),
        ];
        let menu = rows_for(DaemonState::Paused, &pairs);
        let labels = labels_of(&menu);
        assert!(
            labels.contains(&"Resume documents".to_owned()),
            "{labels:?}"
        );
        assert!(labels.contains(&"Pause photos".to_owned()), "{labels:?}");
        assert!(
            !labels.contains(&"Pause documents".to_owned()),
            "{labels:?}"
        );
        assert!(!labels.contains(&"Resume photos".to_owned()), "{labels:?}");
    }

    /// Acceptance 2, the other half, over every state and every folder list of two or more.
    #[test]
    fn no_row_at_two_folders_says_pause_syncing() {
        for state in ALL_STATES {
            for pairs in folder_lists().into_iter().filter(|p| p.len() >= 2) {
                for entry in rows_for(*state, &pairs) {
                    if let Entry::Row { id, label, .. } = &entry {
                        assert_ne!(label, "Pause syncing", "{state:?} {id}");
                        assert_ne!(label, "Resume syncing", "{state:?} {id}");
                        assert_ne!(id, "pause", "{state:?}: the unaddressed pause row");
                        assert_ne!(id, "resume", "{state:?}: the unaddressed resume row");
                    }
                }
            }
        }
    }

    /// Maintainer ruling (#102): every folder has its own pause, and nothing pauses everything. Every
    /// row's id is one of the fixed ones or `pause@<a listed folder>` / `resume@<a listed folder>`,
    /// and no label says "all". Revert: add a `pauseAll` row.
    #[test]
    fn no_row_offers_pause_all() {
        const FIXED: [&str; 8] = [
            "open",
            "syncNow",
            "pause",
            "resume",
            "tryAgain",
            "start",
            "closeWindow",
            "quit",
        ];
        for state in ALL_STATES {
            for pairs in folder_lists() {
                let names: Vec<&str> = pairs.iter().map(|p| p.name.as_str()).collect();
                for entry in rows_for(*state, &pairs) {
                    let Entry::Row { id, label, .. } = &entry else {
                        continue;
                    };
                    let allowed = FIXED.contains(&id.as_ref())
                        || names.iter().any(|name| {
                            id.as_ref() == format!("pause@{name}")
                                || id.as_ref() == format!("resume@{name}")
                        });
                    assert!(
                        allowed,
                        "{state:?}: {id:?} is not a fixed row or one folder's own"
                    );
                    assert!(
                        !label.to_lowercase().contains(" all"),
                        "{state:?}: {label:?} reads like a pause-everything control"
                    );
                }
            }
        }
    }

    /// D5, and the rule 4.4 gives for the aggregate row: `Sync now` is present whenever some unpaused
    /// folder is idle, and absent only when every unpaused folder is already syncing. It is chosen
    /// from the folder list, not from the aggregate state. Revert: derive the set from the aggregate
    /// state alone (a `Running` set has no `Sync now`).
    #[test]
    fn sync_now_is_present_while_one_pair_is_idle() {
        let has_sync_now = |state, pairs: &[TrayPair]| {
            ids(&rows_for(state, pairs)).contains(&"syncNow".to_owned())
        };
        let busy = |name: &str| folder(name, DaemonState::Running, false, true);
        let paused = |name: &str| folder(name, DaemonState::Paused, true, false);

        // The aggregate is `Running` the moment any folder syncs; the other one is idle and wants it.
        assert!(has_sync_now(DaemonState::Running, &[busy("a"), idle("b")]));
        // Every unpaused folder is already syncing.
        assert!(!has_sync_now(DaemonState::Running, &[busy("a"), busy("b")]));
        // A paused folder is not Sync now's to wake: only unpaused ones count.
        assert!(!has_sync_now(
            DaemonState::Running,
            &[busy("a"), paused("b")]
        ));
        // The aggregate is `Paused`, and the other folder is idle and unpaused.
        assert!(has_sync_now(DaemonState::Paused, &[paused("a"), idle("b")]));
        // Everything paused: nobody to sync.
        assert!(!has_sync_now(
            DaemonState::Paused,
            &[paused("a"), paused("b")]
        ));
        // All idle (the settled set): present, as it always was.
        assert!(has_sync_now(DaemonState::Idle, &[idle("a"), idle("b")]));
    }

    /// The folder group is worst first, and ties keep the daemon's order.
    #[test]
    fn the_folder_group_lists_the_worst_folder_first() {
        let pairs = [
            idle("a"),
            folder("b", DaemonState::Failed, false, false),
            idle("c"),
        ];
        let group: Vec<String> = ids(&rows_for(DaemonState::Failed, &pairs))
            .into_iter()
            .filter(|id| id.contains('@'))
            .collect();
        assert_eq!(group, ["pause@b", "pause@a", "pause@c"]);
    }

    /// Acceptance 3: a click on an id issued for `photos` is `pause@photos` even if the folder list
    /// has since changed, and an id nothing ever issued is no action. Revert: issue ids by position.
    #[test]
    fn a_stale_menu_hits_the_pair_its_label_named() {
        let before = rows_for(
            DaemonState::Idle,
            &[idle("stale-documents"), idle("stale-photos")],
        );
        let photos_row = before
            .iter()
            .find(|entry| entry.action() == Some("pause@stale-photos"))
            .expect("the menu has a row for photos");
        let documents_row = before
            .iter()
            .find(|entry| entry.action() == Some("pause@stale-documents"))
            .expect("and one for documents");
        assert_eq!(photos_row.folded_label(), "Pause stale-photos");

        // The list changes under the menu: documents is gone, a new folder leads, photos moved up.
        let _after = rows_for(
            DaemonState::Idle,
            &[idle("stale-music"), idle("stale-photos")],
        );

        assert_eq!(
            action_for_dbus_id(photos_row.dbus_id()).as_deref(),
            Some("pause@stale-photos"),
            "the id the host was handed for `Pause stale-photos` still means that"
        );
        // And the row for the folder that went still says what it said: it is the daemon's
        // byte-exact selector rule that refuses it, not a guess made here.
        assert_eq!(
            action_for_dbus_id(documents_row.dbus_id()).as_deref(),
            Some("pause@stale-documents")
        );
        assert_eq!(action_for_dbus_id(987_654), None, "an id nothing issued");
    }

    /// The registry never reuses a number for another meaning, and gives the same pair the same one.
    #[test]
    fn the_registry_is_append_only() {
        let mut registry = Registry::new();
        let a = registry.id_for(PairAction::Pause, "a");
        let b = registry.id_for(PairAction::Pause, "b");
        let resume_a = registry.id_for(PairAction::Resume, "a");
        assert_eq!(
            [a, b, resume_a].iter().collect::<HashSet<_>>().len(),
            3,
            "three meanings, three numbers"
        );
        assert_eq!(registry.id_for(PairAction::Pause, "a"), a, "stable");
        assert!(a >= FIRST_FOLDER_ROW_ID, "clear of the static rows");
        assert_eq!(registry.lookup(b), Some((PairAction::Pause, "b")));
        assert_eq!(registry.lookup(FIRST_FOLDER_ROW_ID - 1), None);
        // A fourth issue is a fourth number: nothing is ever handed out twice.
        let c = registry.id_for(PairAction::Pause, "c");
        assert!(![a, b, resume_a].contains(&c));
    }

    #[test]
    fn a_rule_is_not_an_action() {
        for id in [SEPARATOR_ID, SECOND_SEPARATOR_ID, 0] {
            assert_eq!(action_for_dbus_id(id), None, "{id}");
        }
    }

    /// The `TRAY` block of `ui/copy.js`, parsed. The one-line string entries, plus the two folder
    /// templates, which are read as their template text (`Pause ${name}`).
    fn copy_deck() -> HashMap<String, String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/js/ui/copy.js");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read the copy deck at {}: {e}", path.display()));
        let block = source
            .split_once("export const TRAY = {")
            .expect("ui/copy.js has no TRAY block")
            .1;
        let block = block
            .split_once("\n};")
            .expect("the TRAY block never closes")
            .0;

        let mut deck = HashMap::new();
        for line in block.lines() {
            let line = line.trim();
            // `pausePair: (name) => `Pause ${name}`,` — a one-line template, kept as its text.
            if let Some((key, rest)) = line.split_once(": (name) => `") {
                if let Some(template) = rest.strip_suffix("`,") {
                    deck.insert(key.to_string(), template.to_string());
                }
                continue;
            }
            let Some((key, rest)) = line.split_once(": \"") else {
                continue;
            };
            if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let Some(value) = rest.strip_suffix("\",") else {
                continue;
            };
            deck.insert(key.to_string(), value.to_string());
        }
        deck
    }

    /// `ui/compact.js`'s `TRAY_MENU`, parsed into `key -> [row id | "—"]`. Comment lines are skipped
    /// rather than matched around: the block carries a twelve-line doc comment above `deferToWindow`.
    fn panel_menu() -> HashMap<String, Vec<String>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/js/ui/compact.js");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read the panel's menu at {}: {e}", path.display()));
        let block = source
            .split_once("export const TRAY_MENU = {")
            .expect("ui/compact.js has no TRAY_MENU")
            .1
            .split_once("\n};")
            .expect("the TRAY_MENU block never closes")
            .0;

        let mut sets: HashMap<String, Vec<String>> = HashMap::new();
        let mut current: Option<String> = None;
        for line in block.lines() {
            let line = line.trim();
            if line.starts_with('*') || line.starts_with("//") || line.starts_with("/*") {
                continue;
            }
            if let Some(key) = line.strip_suffix(": [") {
                current = Some(key.to_string());
                sets.insert(key.to_string(), Vec::new());
            } else if line == "]," {
                current = None;
            } else if let Some(key) = current.as_ref() {
                let rows = sets.get_mut(key).expect("the key was just inserted");
                if line.starts_with("{ separator: true }") {
                    rows.push("—".to_string());
                } else if let Some(rest) = line.strip_prefix("{ id: \"") {
                    rows.push(
                        rest.split_once('"')
                            .expect("an id opens and closes")
                            .0
                            .to_string(),
                    );
                }
            }
        }
        sets
    }

    #[test]
    fn the_panel_and_the_native_menus_draw_the_same_rows_in_the_same_order() {
        // WHAT "ONE TABLE FOR THREE MENUS" ACTUALLY MEANS, checked rather than claimed. The panel's
        // rows are a JS literal and these are a Rust const, and they cannot be one object — so the
        // only thing that can keep them one menu is a comparison. Without this, adding a row to
        // `TRAY_MENU.paused` and not here gives a left click and a right click different menus for
        // the same daemon, and every gate in both languages stays green.
        //
        // `needsYou` has no Rust counterpart: S1's derivation folds it into the idle state, and the
        // panel's needs-you list IS the settled list — which is itself an invariant worth holding.
        let panel = panel_menu();
        assert_eq!(panel.len(), 7, "the panel's row sets: {:?}", panel.keys());

        let ours = |state: DaemonState| -> Vec<String> { ids(&rows(state)) };
        for (key, state) in [
            ("settled", DaemonState::Idle),
            ("needsYou", DaemonState::Idle),
            ("syncing", DaemonState::Running),
            ("paused", DaemonState::Paused),
            // The two halves of what was one `unreachable` set on both sides. The names are the
            // point: `outage` is Proton out of reach with the daemon answering, `notRunning` is the
            // daemon itself, and each language calls the same state the same thing.
            ("outage", DaemonState::Failed),
            ("notRunning", DaemonState::Unreachable),
            ("deferToWindow", DaemonState::AuthExpired),
        ] {
            let theirs = panel
                .get(key)
                .unwrap_or_else(|| panic!("ui/compact.js's TRAY_MENU has no {key:?}"));
            assert!(!theirs.is_empty(), "{key} parsed as an empty menu");
            assert_eq!(*theirs, ours(state), "{key} against {state:?}");
        }
    }

    /// Acceptance 6, for the case the static table cannot cover: the panel's `trayMenuFor` and these
    /// native rows for the same `(state, folders)`. They are two implementations in two languages, so
    /// the only thing that can hold them together is a corpus both assert against —
    /// `gui/test/tray-menu-corpus.json`, whose JS half is `gui/test/tray-menu.test.js`. Ids AND labels
    /// AND sub-labels, in order, rules included.
    #[test]
    fn the_native_rows_are_the_corpus_the_panel_is_held_to() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test/tray-menu-corpus.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read the corpus at {}: {e}", path.display()));
        let corpus: serde_json::Value = serde_json::from_str(&text).expect("the corpus parses");
        let cases = corpus.as_array().expect("the corpus is a list");
        assert!(
            cases.len() >= 15,
            "the corpus shrank to {} cases",
            cases.len()
        );

        for case in cases {
            let what = case["case"].as_str().expect("every case is named");
            let state = match case["state"].as_str().expect("a state") {
                "idle" => DaemonState::Idle,
                "running" => DaemonState::Running,
                "queued" => DaemonState::Queued,
                "paused" => DaemonState::Paused,
                "unreachable" => DaemonState::Unreachable,
                "authExpired" => DaemonState::AuthExpired,
                "failed" => DaemonState::Failed,
                "firstRun" => DaemonState::FirstRun,
                other => panic!("{what}: the corpus names a state {other:?} this build lacks"),
            };
            let pairs: Vec<TrayPair> = case["pairs"]
                .as_array()
                .expect("pairs")
                .iter()
                .map(|pair| TrayPair {
                    name: pair["name"].as_str().expect("a name").to_owned(),
                    paused: pair["paused"].as_bool().expect("paused"),
                    syncing: pair["syncing"].as_bool().expect("syncing"),
                    rank: pair["rank"].as_u64().expect("rank") as u8,
                })
                .collect();

            let ours: Vec<serde_json::Value> = rows_for(state, &pairs)
                .iter()
                .map(|entry| match entry {
                    Entry::Separator { .. } => serde_json::json!("-"),
                    Entry::Row { id, label, sub, .. } => serde_json::json!([id, label, sub]),
                })
                .collect();
            assert_eq!(
                serde_json::Value::Array(ours),
                case["rows"],
                "{what}: the native rows and the corpus disagree"
            );
        }
    }

    #[test]
    fn the_labels_are_the_copy_deck_s() {
        // NOTHING ELSE CHECKS THIS. `copy-gate.mjs` compares `ui/copy.js` against the frames, and
        // the frames are the panel; these strings are a second copy in a language that gate cannot
        // read. A word changed on one side and not the other is two menus for one app — and the
        // native one is the copy most people see, because it is the one that opens on right-click.
        let deck = copy_deck();
        let expected = [
            (OPEN, "open", None),
            (SYNC_NOW, "syncNow", None),
            (PAUSE, "pause", None),
            (RESUME, "resume", None),
            (TRY_AGAIN, "tryAgain", None),
            (START, "start", None),
            (CLOSE_WINDOW, "closeWindow", Some("closeWindowSub")),
            (QUIT, "quit", Some("quitSub")),
        ];
        for (entry, key, sub_key) in expected {
            let Entry::Row { label, sub, .. } = &entry else {
                unreachable!("the table's rows are rows")
            };
            let deck_label = deck
                .get(key)
                .unwrap_or_else(|| panic!("ui/copy.js's TRAY has no {key:?}"));
            assert_eq!(label.as_ref(), deck_label, "TRAY.{key} says {deck_label:?}");

            match sub_key {
                Some(sub_key) => {
                    let deck_sub = deck
                        .get(sub_key)
                        .unwrap_or_else(|| panic!("ui/copy.js's TRAY has no {sub_key:?}"));
                    assert_eq!(sub.as_deref().expect("this row has a sub-label"), deck_sub);
                    // And the fold, composed from what was parsed rather than from a literal: the
                    // point of the test is that this side is not written down twice.
                    assert_eq!(entry.folded_label(), format!("{deck_label} — {deck_sub}"));
                }
                None => assert!(sub.is_none(), "TRAY.{key} has no sub-label in the deck"),
            }
        }

        // The two folder templates (D11): the deck's text with `${name}` filled in is what this
        // side formats. Revert: change one template in Rust only.
        for (key, action) in [
            ("pausePair", PairAction::Pause),
            ("resumePair", PairAction::Resume),
        ] {
            let template = deck
                .get(key)
                .unwrap_or_else(|| panic!("ui/copy.js's TRAY has no {key:?}"));
            for name in ["photos", "my-folder.2"] {
                assert_eq!(
                    action.label(name),
                    template.replace("${name}", name),
                    "TRAY.{key} says {template:?}"
                );
            }
        }
    }

    // ---- from a reply to a row (#102 phase 5d review) ------------------------------------------------
    //
    // `pairs_of` is the glue between what the daemon says and what a menu is built from, and it had no
    // test: the rows were verified from `TrayPair`s written by hand, which cannot notice a field the
    // glue reads wrongly. These start one step earlier, at the raw per-folder facts on the wire.

    /// What each folder in a reply is, as `pairs_of` hands it to the menu: its own flags and its own
    /// rank — written as literals (1 paused, 2 queued, 3 syncing, 5 failed, 0 idle), not read back from
    /// `severity`, so a rank that stops following the state is a failure here and not an agreement.
    ///
    /// Reverts: `paused` forced `false`; `syncing` forced `false`; `rank` forced `0`.
    #[test]
    fn a_reply_becomes_each_folders_own_flags_and_rank() {
        let response = reply(&[
            Folder::new("a").paused(),
            Folder::new("b").syncing(),
            Folder::new("c").failed(),
            Folder::new("d"),
            Folder::new("e").unsynced(),
        ]);
        let described = gui_core::state::derive_state(Ok(&response));
        let want = |name: &str, paused, syncing, rank| TrayPair {
            name: name.to_owned(),
            paused,
            syncing,
            rank,
        };
        assert_eq!(
            pairs_of(&response, described),
            vec![
                want("a", true, false, 1),
                want("b", false, true, 3),
                want("c", false, false, 5),
                want("d", false, false, 0),
                want("e", false, false, 2),
            ]
        );
    }

    /// The live report, through the whole path: the default folder is in a long pass and the other two
    /// have not had their turn. They are `Queued`, which is not `FirstRun`, so the menu keeps its
    /// folder group — a person can still pause any of the three while they wait.
    #[test]
    fn folders_waiting_for_their_turn_keep_their_pause_rows() {
        let wanted = |rest: &[&str]| -> Vec<String> {
            ["open", "syncNow", "—"]
                .into_iter()
                .chain(rest.iter().copied())
                .chain(["—", "closeWindow", "quit"])
                .map(String::from)
                .collect()
        };
        let group = ["pause@documents", "pause@photos", "pause@videos"];

        let response = reply(&[
            Folder::new("documents").syncing(),
            Folder::new("photos").unsynced(),
            Folder::new("videos").unsynced(),
        ]);
        let described = gui_core::state::derive_state(Ok(&response));
        let states = pair_states(&response, described);
        let aggregate = gui_core::state::aggregate_state(described, &states);
        assert_eq!(aggregate, DaemonState::Running);
        assert_eq!(
            ids(&rows_for(aggregate, &pairs_of(&response, described))),
            wanted(&group)
        );

        // And at start-up, with nothing running yet: all three wait, and the group is still there.
        let response = reply(&[
            Folder::new("documents").unsynced(),
            Folder::new("photos").unsynced(),
            Folder::new("videos").unsynced(),
        ]);
        let described = gui_core::state::derive_state(Ok(&response));
        let states = pair_states(&response, described);
        let aggregate = gui_core::state::aggregate_state(described, &states);
        assert_eq!(aggregate, DaemonState::Queued);
        assert_eq!(
            ids(&rows_for(aggregate, &pairs_of(&response, described))),
            wanted(&group)
        );
    }

    /// A reply that lists no folders (a daemon older than the selector) draws today's rows.
    #[test]
    fn a_reply_that_lists_no_folders_lists_none() {
        let mut response = reply(&[Folder::new("a")]);
        response.pairs.clear();
        assert!(pairs_of(&response, DaemonState::Idle).is_empty());
    }

    /// The whole path, over every combination of two folders' facts: the raw flags on the wire →
    /// `derive_state` → `pair_states` → `aggregate_state` → `pairs_of` → `rows_for`, against an
    /// expectation written from the rules (D3's rank, D5's `Sync now`, D13-free native rows) and
    /// sharing no code with them. 16 × 16 = 256 replies.
    #[test]
    fn two_folders_draw_the_rows_their_facts_call_for() {
        // (paused, syncing, failed, queued): 0 idle .. 5 failed; the rank is the brief's, in order
        // failed 5 > running 3 > paused 1 > idle 0, and `paused` beats `syncing` and `failed`.
        fn oracle(paused: bool, syncing: bool, failed: bool, queued: bool) -> (&'static str, u8) {
            if paused {
                ("paused", 1)
            } else if syncing || (!failed && queued) {
                ("running", 3)
            } else if failed {
                ("failed", 5)
            } else {
                ("idle", 0)
            }
        }
        let folder = |name: &'static str, bits: u32| {
            let mut folder = Folder::new(name);
            folder.paused = bits & 1 != 0;
            folder.syncing = bits & 2 != 0;
            folder.failed = bits & 4 != 0;
            folder.queued = usize::from(bits & 8 != 0) * 3;
            folder
        };
        let flags = |bits: u32| (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);

        let mut checked = 0;
        for a in 0..16u32 {
            for b in 0..16u32 {
                let response = reply(&[folder("a", a), folder("b", b)]);
                let described = gui_core::state::derive_state(Ok(&response));
                let states = pair_states(&response, described);
                let aggregate = gui_core::state::aggregate_state(described, &states);
                let got = ids(&rows_for(aggregate, &pairs_of(&response, described)));

                let (pa, pb) = (flags(a), flags(b));
                let ((state_a, rank_a), (state_b, rank_b)) = (
                    oracle(pa.0, pa.1, pa.2, pa.3),
                    oracle(pb.0, pb.1, pb.2, pb.3),
                );
                let worst = if rank_a >= rank_b { state_a } else { state_b };
                let mut group = [("a", pa.0, rank_a), ("b", pb.0, rank_b)];
                group.sort_by_key(|(_, _, rank)| std::cmp::Reverse(*rank));
                let group_ids: Vec<String> = group
                    .iter()
                    .map(|(name, paused, _)| {
                        format!("{}@{name}", if *paused { "resume" } else { "pause" })
                    })
                    .collect();
                let some_idle_unpaused = (!pa.0 && !pa.1) || (!pb.0 && !pb.1);
                let some_unpaused = !pa.0 || !pb.0;

                let mut want: Vec<String> = Vec::new();
                if worst == "failed" {
                    want.extend(["tryAgain", "open", "—"].map(String::from));
                    want.extend(group_ids);
                    want.extend(["—", "quit"].map(String::from));
                } else {
                    want.push("open".into());
                    if some_idle_unpaused {
                        want.push("syncNow".into());
                    }
                    want.push("—".into());
                    want.extend(group_ids);
                    want.push("—".into());
                    if some_unpaused {
                        want.push("closeWindow".into());
                    }
                    want.push("quit".into());
                }
                assert_eq!(got, want, "a={a:04b} b={b:04b} aggregate={aggregate:?}");
                checked += 1;
            }
        }
        assert_eq!(checked, 256);
    }
}
