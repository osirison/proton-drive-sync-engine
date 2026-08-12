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
//! change is still on screen with the ids it was built from — and if those ids were positions, a
//! click on the `Pause syncing` the user is looking at would arrive as whatever now sits third in
//! the new list. On the settled→paused change that is `Quit`.
//!
//! So the id travels with the row. A stale menu dispatches the action its label promised, or none.

use gui_core::state::DaemonState;

/// One row, or the rule between two groups of them.
///
/// `id` is the vocabulary `commands::tray_row` dispatches — the same strings `ui/compact.js` sends
/// from the panel. `dbus_id` is what a native menu row is called on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Row {
        id: &'static str,
        dbus_id: i32,
        label: &'static str,
        /// The second, quieter line the panel draws under the label. `10-tray.md` calls the two rows
        /// that carry one "the single worst misunderstanding a tray app can cause".
        sub: Option<&'static str>,
    },
    Separator,
}

/// A separator carries no action, so one id serves every set — no set has two, and
/// `every_row_in_a_set_has_its_own_id` fails if one ever grows a second.
pub const SEPARATOR_ID: i32 = 90;

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
            Entry::Row { label, .. } => (*label).to_string(),
            Entry::Separator => String::new(),
        }
    }

    pub fn dbus_id(&self) -> i32 {
        match self {
            Entry::Row { dbus_id, .. } => *dbus_id,
            Entry::Separator => SEPARATOR_ID,
        }
    }
}

// The rows themselves. Shared consts rather than per-set literals, so an action has one id and one
// label everywhere it appears — which is what makes the id stable across a state change.
const OPEN: Entry = Entry::Row {
    id: "open",
    dbus_id: 1,
    label: "Open Drive Sync",
    sub: None,
};
const SYNC_NOW: Entry = Entry::Row {
    id: "syncNow",
    dbus_id: 2,
    label: "Sync now",
    sub: None,
};
const PAUSE: Entry = Entry::Row {
    id: "pause",
    dbus_id: 3,
    label: "Pause syncing",
    sub: None,
};
const RESUME: Entry = Entry::Row {
    id: "resume",
    dbus_id: 4,
    label: "Resume syncing",
    sub: None,
};
const TRY_AGAIN: Entry = Entry::Row {
    id: "tryAgain",
    dbus_id: 5,
    label: "Try again now",
    sub: None,
};
const CLOSE_WINDOW: Entry = Entry::Row {
    id: "closeWindow",
    dbus_id: 6,
    label: "Close window",
    sub: Some("keeps syncing"),
};
const QUIT: Entry = Entry::Row {
    id: "quit",
    dbus_id: 7,
    label: "Quit",
    sub: Some("stops syncing"),
};
const SEP: Entry = Entry::Separator;

// The five sets, and they are `ui/compact.js`'s five: `settled`, `syncing`, `paused`, `unreachable`
// and `deferToWindow`. `needsYou` is not a sixth — the panel's needs-you list is `settled`'s rows,
// because `Review them` is the panel's own decision button rather than a menu row.
const SETTLED: &[Entry] = &[OPEN, SYNC_NOW, PAUSE, SEP, CLOSE_WINDOW, QUIT];
// `Sync now` is absent while a pass is running: it would do nothing.
const SYNCING: &[Entry] = &[OPEN, PAUSE, SEP, CLOSE_WINDOW, QUIT];
// The two states that are not moving files lead with the row that fixes them and drop
// `Close window` — with nothing syncing, `keeps syncing` would be a lie.
const PAUSED: &[Entry] = &[RESUME, OPEN, SEP, QUIT];
const UNREACHABLE: &[Entry] = &[TRY_AGAIN, OPEN, SEP, QUIT];
// An expired session and a daemon that has never synced are both fixed in the window, not by
// retrying a sync. The panel is keyed by FORM (both wear the struck mark) and the menu by CAUSE.
// DEVIATIONS §82g.
const DEFER_TO_WINDOW: &[Entry] = &[OPEN, SEP, QUIT];

/// The rows for a state. The same mapping `screens/tray.js` makes for the panel, made once here for
/// both native menus.
pub fn rows_for(state: DaemonState) -> &'static [Entry] {
    match state {
        DaemonState::Idle => SETTLED,
        DaemonState::Running => SYNCING,
        DaemonState::Paused => PAUSED,
        DaemonState::Unreachable => UNREACHABLE,
        DaemonState::AuthExpired | DaemonState::FirstRun => DEFER_TO_WINDOW,
    }
}

/// What a numeric id means, looked up across EVERY set rather than the one on screen.
///
/// This is the stale-menu case the module header describes, and it is why the lookup is not
/// `rows_for(current_state).iter().find(...)`: the click that arrives may be on a menu built two
/// seconds and one state change ago, and the row the user pressed is the row that must run. An id
/// no set has ever drawn returns `None` and the caller says so.
pub fn action_for_dbus_id(dbus_id: i32) -> Option<&'static str> {
    ALL_STATES
        .iter()
        .flat_map(|state| rows_for(*state))
        .find_map(|entry| match entry {
            Entry::Row {
                id, dbus_id: at, ..
            } if *at == dbus_id => Some(*id),
            _ => None,
        })
}

/// Every state, for the tests and for anything that has to walk the whole table.
pub const ALL_STATES: &[DaemonState] = &[
    DaemonState::Idle,
    DaemonState::Running,
    DaemonState::Paused,
    DaemonState::Unreachable,
    DaemonState::AuthExpired,
    DaemonState::FirstRun,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn rows(state: DaemonState) -> Vec<Entry> {
        rows_for(state).to_vec()
    }

    #[test]
    fn every_row_in_a_set_has_its_own_id() {
        // Two rows sharing an id in one menu is a click that dispatches the wrong one — and the
        // separator's shared id is only safe while no set has two separators, which is what this
        // pins.
        for state in ALL_STATES {
            let mut seen = HashSet::new();
            for entry in rows_for(*state) {
                assert!(
                    seen.insert(entry.dbus_id()),
                    "{state:?} has two rows with id {}",
                    entry.dbus_id()
                );
            }
        }
    }

    #[test]
    fn an_id_means_the_same_action_in_every_state() {
        // THE STALE-MENU RACE. A host keeps the layout it was given until `LayoutUpdated` reaches
        // it; the poll changes the rows every two seconds. If `3` were "third row" rather than
        // "pause", a click on the `Pause syncing` still on screen would arrive after a
        // settled→paused change as the third row of the paused set, which is `Quit`.
        let mut action: HashMap<i32, &'static str> = HashMap::new();
        for state in ALL_STATES {
            for entry in rows_for(*state) {
                if let Entry::Row { id, dbus_id, .. } = entry {
                    let seen = action.entry(*dbus_id).or_insert(id);
                    assert_eq!(seen, id, "id {dbus_id} means two different things");
                }
            }
        }
        // And the other direction: one action, one number, or the same row is two rows to a host.
        let ids: HashSet<_> = action.values().collect();
        assert_eq!(ids.len(), action.len(), "an action has two ids");
    }

    #[test]
    fn no_row_claims_the_root_s_id() {
        // `0` is the root of a dbusmenu layout. A row calling itself 0 is a row a host will treat
        // as the menu it belongs to.
        for state in ALL_STATES {
            for entry in rows_for(*state) {
                assert_ne!(entry.dbus_id(), 0, "{state:?} has a row with the root's id");
            }
        }
    }

    #[test]
    fn every_row_is_an_action_this_build_can_perform() {
        // The other half of `tray.rs`'s `both_indicators_speak_one_vocabulary`: that test proves the
        // ids the PANEL sends are known, this one proves the ids the NATIVE menus send are.
        for state in ALL_STATES {
            for entry in rows_for(*state) {
                if let Entry::Row { id, .. } = entry {
                    assert!(
                        crate::commands::tray_row(id).is_some(),
                        "{state:?} draws {id:?} and nothing dispatches it"
                    );
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
                !rows(state)
                    .iter()
                    .any(|e| matches!(e, Entry::Row { id, .. } if *id == "closeWindow")),
                "{state:?} offers `Close window — keeps syncing`"
            );
        }
    }

    #[test]
    fn every_set_can_be_left() {
        // A tray menu with no way out is the failure mode of a menu built per state: each one is
        // written on its own and the one written last forgets.
        for state in ALL_STATES {
            let ids: Vec<_> = rows(*state)
                .iter()
                .filter_map(|e| match e {
                    Entry::Row { id, .. } => Some(*id),
                    Entry::Separator => None,
                })
                .collect();
            assert!(ids.contains(&"quit"), "{state:?} cannot be quit");
            assert!(ids.contains(&"open"), "{state:?} cannot open the window");
        }
    }

    #[test]
    fn the_sub_labels_fold_rather_than_vanish() {
        let quit = QUIT.folded_label();
        assert_eq!(quit, "Quit — stops syncing");
        assert_eq!(CLOSE_WINDOW.folded_label(), "Close window — keeps syncing");
        assert!(SEP.folded_label().is_empty());
    }

    /// The `TRAY` block of `ui/copy.js`, parsed. Only the one-line string entries — the block also
    /// holds two template functions, which have no counterpart here.
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
            (CLOSE_WINDOW, "closeWindow", Some("closeWindowSub")),
            (QUIT, "quit", Some("quitSub")),
        ];
        for (entry, key, sub_key) in expected {
            let Entry::Row { label, sub, .. } = entry else {
                unreachable!("the table's rows are rows")
            };
            let deck_label = deck
                .get(key)
                .unwrap_or_else(|| panic!("ui/copy.js's TRAY has no {key:?}"));
            assert_eq!(label, deck_label, "TRAY.{key} says {deck_label:?}");

            match sub_key {
                Some(sub_key) => {
                    let deck_sub = deck
                        .get(sub_key)
                        .unwrap_or_else(|| panic!("ui/copy.js's TRAY has no {sub_key:?}"));
                    assert_eq!(sub.expect("this row has a sub-label"), deck_sub);
                    // And the fold, composed from what was parsed rather than from a literal: the
                    // point of the test is that this side is not written down twice.
                    assert_eq!(entry.folded_label(), format!("{deck_label} — {deck_sub}"));
                }
                None => assert!(sub.is_none(), "TRAY.{key} has no sub-label in the deck"),
            }
        }
    }
}
