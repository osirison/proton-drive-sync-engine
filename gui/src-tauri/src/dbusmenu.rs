//! The native right-click menu, spoken directly (S8 follow-up, #252).
//!
//! # What this closes
//!
//! `10-tray.md` §Behaviour: *"Left-click opens the panel; right-click opens the menu alone (KDE
//! convention)."* S8 shipped the left half — `sni.rs` publishes `Activate`, and a click on the
//! indicator opens the compact panel at the click's coordinates — and gave right-click the panel
//! too, because a native menu needs a second protocol and a right click that produced nothing would
//! read as a broken tray. DEVIATIONS §82j, resolved in §89.
//!
//! # The premise, checked before any of this was written
//!
//! Publishing a `Menu` object path is exactly the thing that could have broken the half that already
//! worked, so it was the first question asked. Plasma's own `StatusNotifierItem.qml` routes a left
//! click on **`ItemIsMenu` alone**:
//!
//! ```text
//! // applets/systemtray/qml/StatusNotifierItem.qml
//! if (model.ItemIsMenu) { Plasmoid.openContextMenu(...) } else { Plasmoid.activate(...) }
//! ```
//!
//! and `statusnotifieritemsource.cpp` shows the other side of the same coin — right click prefers a
//! `com.canonical.dbusmenu` importer when the item exposes a `Menu`, and falls back to calling
//! `ContextMenu()` on the item when it does not:
//!
//! ```text
//! if (m_menuImporter) { m_menuImporter->updateMenu(); }
//! else { qCWarning(...) << "Could not find DBusMenu interface, falling back to calling ContextMenu()";
//!        m_statusNotifierItemInterface->call(QDBus::NoBlock, u"ContextMenu"_s, x, y); }
//! ```
//!
//! So `ItemIsMenu` stays `false`, `ContextMenu` stays implemented (it is what a host with no
//! dbusmenu importer will call), and this is additive.
//!
//! # The layout was measured, not read off the specification
//!
//! Same method as `sni.rs`: a libappindicator item was published on this session and its menu read
//! back over the bus. `GetLayout(0, -1, [])` answered
//!
//! ```text
//! (uint32 2, (0, {'children-display': <'submenu'>}, [
//!   <(2, {'label': <'Open Drive Sync'>}, @av [])>,
//!   <(3, {'enabled': <true>, 'type': <'separator'>}, @av [])>,
//!   <(4, {'enabled': <false>, 'label': <'Nothing synced yet'>}, @av [])>,
//!   <(5, {'label': <'Quit — stops syncing'>}, @av [])>]))
//! ```
//!
//! with `Version = 3`, `Status = "normal"`, `TextDirection = "ltr"`. That is the shape produced here:
//! a root that says it has a submenu, and one flat level of children carrying `label`, or `type`
//! when they are a rule. Properties a row leaves out take their documented defaults (`enabled` and
//! `visible` are both true), which is what the specimen relies on too.
//!
//! To repeat the measurement: **neither `busctl` nor `gdbus` can send that call**, because both
//! parse the `-1` recursion depth as one of their own options (`busctl: invalid option -- '1'`). A
//! four-line Python caller through `Gio.DBusConnection.call_sync` with a `(iias)` variant does it.
//! And `strings` on `libdbusmenu-glib.so.4` prints the whole interface XML, argument names included
//! — which is where the two mistakes in the first draft of this file were eventually found.
//!
//! # What the consumer does, and the two choices that follow from it
//!
//! Plasma's importer is `libdbusmenuqt`'s `DBusMenuImporter`. It calls `GetLayout(id, 1, [])` — one
//! level, all properties — then `AboutToShow(id)` before showing, and `Event(id, "clicked", …)` on a
//! selection; it listens for `LayoutUpdated`.
//!
//!   * **`AboutToShow` returns `true`.** The return means "you need to refresh": on `true` the
//!     importer re-reads the layout before drawing it, on `false` it draws what it cached. These rows
//!     depend on the daemon's state, which moves under the menu, so the answer is always yes. It
//!     costs one round trip per right click and it is the difference between `Pause syncing` and
//!     `Resume syncing` on a daemon that paused since the menu was last built.
//!   * **`LayoutUpdated` is emitted anyway**, on the poll, for a host that caches harder than the
//!     `AboutToShow` contract promises.
//!
//! `ItemsPropertiesUpdated` and `ItemActivationRequested` are deliberately not declared: this
//! replaces the whole layout rather than patching properties, and nothing here asks a host to open
//! the menu on the program's behalf. A host does not need them to exist — a D-Bus signal is matched
//! by name on the bus, not found by introspection.

#![cfg(target_os = "linux")]

use std::collections::HashMap;

use tauri::AppHandle;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, StructureBuilder, Value};
use zbus::{interface, Connection};

use crate::tray_menu::{self, Entry};

/// Under the item's own path, which is where every non-libappindicator item puts it. The path only
/// has to match what the `Menu` property advertises.
pub const MENU_PATH: &str = "/StatusNotifierItem/Menu";

/// dbusmenu's `(ia{sv}av)`: an id, its properties, and its children — each child a variant holding
/// one of these again.
type Item = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// The menu a host reads. `rows` is whatever the poll last decided; `revision` is what tells a host
/// the layout it holds is out of date.
pub struct TrayMenu {
    rows: &'static [Entry],
    revision: u32,
    app: AppHandle,
}

impl TrayMenu {
    /// Act on one event, and say whether it was acted on. `false` is the `idErrors` answer.
    ///
    /// A `clicked` on an id no row set has ever carried is the only failure: an event name this does
    /// not know is a host narrating (`opened`, `closed`, `hovered`) and is handled by ignoring it,
    /// which is not an error to report back.
    fn dispatch(&self, id: i32, event_id: &str) -> bool {
        if event_id != "clicked" {
            return true;
        }
        // Looked up across EVERY row set rather than the one currently published: see
        // `tray_menu::action_for_dbus_id`. The menu on screen may predate the last poll.
        let Some(action) = tray_menu::action_for_dbus_id(id) else {
            eprintln!("tray: the native menu sent an unknown row id {id}");
            return false;
        };
        // ONTO THE MAIN THREAD. This runs on the D-Bus task, and the actions behind these rows show
        // and hide GTK windows — which off the main loop is undefined behaviour that presents as
        // nothing happening at all (`panel.rs`'s header, and the same hop `panel::toggle` makes).
        //
        // The hop's own failure — the event loop gone, which is the app on its way out — is an
        // `idErrors` answer too, and was being discarded. "Handled" has to mean the work was
        // scheduled, not that a row was recognised.
        let app = self.app.clone();
        match app
            .clone()
            .run_on_main_thread(move || crate::tray::handle_menu_event(&app, action))
        {
            Ok(()) => true,
            Err(error) => {
                eprintln!("tray: could not run {action:?} on the main thread: {error}");
                false
            }
        }
    }

    pub fn new(app: AppHandle, rows: &'static [Entry]) -> Self {
        Self {
            rows,
            // Not 0. `LayoutUpdated(0, …)` from a program that has just started is indistinguishable
            // from a host's own "nothing fetched yet", and libdbusmenuqt's importer ignores the
            // revision entirely — but the GNOME extension does not, and it is the host this build
            // cannot test.
            revision: 1,
            app,
        }
    }
}

/// One row, as a host reads it.
///
/// A row carries `label` and nothing else; a separator carries `type`. Everything omitted takes its
/// specified default — `enabled` and `visible` are true, `type` is `"standard"` — which is what the
/// measured libappindicator layout does, and every property NOT sent is one that cannot disagree
/// with the panel's copy of the same row.
fn properties(entry: &Entry) -> HashMap<String, Value<'static>> {
    let mut props = HashMap::new();
    match entry {
        Entry::Separator => {
            props.insert("type".to_string(), Value::from("separator"));
        }
        Entry::Row { .. } => {
            props.insert("label".to_string(), Value::from(entry.folded_label()));
        }
    }
    props
}

/// The root's properties, in one place: `layout` puts them on the wire and `GetGroupProperties` /
/// `GetProperty` answer for the same node, and a host that got two different answers for id 0 would
/// draw the menu or not depending on which it asked.
fn root_properties() -> HashMap<String, Value<'static>> {
    // WITHOUT THIS THE MENU IS EMPTY. `children-display` is how a node says it has a submenu at all;
    // an importer that does not see it has no reason to read the children it was just handed.
    HashMap::from([("children-display".to_string(), Value::from("submenu"))])
}

/// What a property means when this menu does not set it — the specification's own defaults, and the
/// reason a row can be two keys long.
fn default_property(name: &str) -> Option<Value<'static>> {
    Some(match name {
        "type" => Value::from("standard"),
        "label" => Value::from(""),
        "enabled" | "visible" => Value::from(true),
        "children-display" | "icon-name" | "toggle-type" => Value::from(""),
        "toggle-state" => Value::from(-1i32),
        _ => return None,
    })
}

fn to_owned(props: HashMap<String, Value<'static>>) -> zbus::Result<HashMap<String, OwnedValue>> {
    props
        .into_iter()
        .map(|(key, value)| Ok((key, OwnedValue::try_from(value)?)))
        .collect()
}

/// A child of the root, as a variant. Childless: this menu is one level deep, and the root is the
/// only node that says otherwise.
fn child(entry: &Entry) -> zbus::Result<OwnedValue> {
    let structure = StructureBuilder::new()
        .add_field(entry.dbus_id())
        .add_field(properties(entry))
        .add_field(Vec::<Value<'static>>::new())
        .build()?;
    Ok(OwnedValue::try_from(Value::from(structure))?)
}

/// The whole layout, as a pure function of the rows.
///
/// Pure on purpose — the same split `sync.rs` uses on the engine side. Everything above is a D-Bus
/// shell around this, so the shape a host will read can be tested without a bus, a session, or a
/// desktop.
fn layout(rows: &[Entry]) -> zbus::Result<Item> {
    let children = rows.iter().map(child).collect::<zbus::Result<Vec<_>>>()?;
    Ok((0, to_owned(root_properties())?, children))
}

/// The `GetGroupProperties` answer, pure. An empty `ids` means every item — **including the root**,
/// which is not a row and is the one node carrying `children-display`.
fn group_properties(
    rows: &[Entry],
    ids: &[i32],
) -> zbus::Result<Vec<(i32, HashMap<String, OwnedValue>)>> {
    let wanted = |id: i32| ids.is_empty() || ids.contains(&id);
    let mut out = Vec::with_capacity(rows.len() + 1);
    if wanted(0) {
        out.push((0, to_owned(root_properties())?));
    }
    for entry in rows.iter().filter(|entry| wanted(entry.dbus_id())) {
        out.push((entry.dbus_id(), to_owned(properties(entry))?));
    }
    Ok(out)
}

/// One row, childless, for a host that asks about a leaf by id.
fn leaf(rows: &[Entry], id: i32) -> zbus::Result<Item> {
    match rows.iter().find(|entry| entry.dbus_id() == id) {
        Some(entry) => Ok((id, to_owned(properties(entry))?, Vec::new())),
        // NOT an error. An id this menu does not have is the ordinary result of a host holding a
        // layout from before the rows changed, and answering `GetLayout` with a D-Bus error there
        // makes an importer log a failure for a menu that is simply out of date.
        None => Ok((id, HashMap::new(), Vec::new())),
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl TrayMenu {
    /// The layout, from `parent_id` down.
    ///
    /// `recursion_depth` and `property_names` are both accepted and ignored, deliberately. The menu
    /// is two levels total, so any depth from `-1` (everything) to `1` (Plasma's own call) describes
    /// the same tree; and a host that asks for a subset of properties is specified to receive them,
    /// but receiving the two this menu sets instead is what every importer already handles — it
    /// reads the keys it knows. Neither is worth a branch that only one host would exercise.
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Item)> {
        let item = if parent_id == 0 {
            layout(self.rows)
        } else {
            leaf(self.rows, parent_id)
        };
        match item {
            Ok(item) => Ok((self.revision, item)),
            Err(error) => Err(zbus::fdo::Error::Failed(format!(
                "tray: could not build the menu layout: {error}"
            ))),
        }
    }

    /// Properties for a set of ids. Asked by hosts that refresh a menu without re-reading its shape.
    ///
    /// **The root is in the answer.** An empty id list means every item, and id 0 is an item — it is
    /// the one carrying `children-display`, without which a host draws an empty menu. It is not in
    /// `rows` (it is synthesised in `layout`), so a filter over `rows` alone silently omits the one
    /// property the menu cannot be read without.
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<Vec<(i32, HashMap<String, OwnedValue>)>> {
        group_properties(self.rows, &ids)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    /// One property of one item.
    ///
    /// **A property this menu does not set is not a property the item does not have.** `properties`
    /// emits `label` or `type` and nothing else, because everything omitted takes its specified
    /// default — so answering `GetProperty(6, "enabled")` with an error would tell a host that a row
    /// it is drawing has no enabled state, when the answer is `true`. The defaults are answered
    /// here; an id the menu has never drawn is still an error, because there is no row to describe.
    ///
    /// This is deliberately MORE generous than the reference server, which errors on an unset
    /// property too ("The property '%s' does not exist on menuitem with ID of %d", in
    /// libdbusmenu-glib) — and no importer in either toolkit calls this method at all, so nothing
    /// observed depends on either behaviour. Answering the specified default cannot mislead a host;
    /// an error about a property with a documented value can.
    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let set = if id == 0 {
            root_properties()
        } else {
            let entry = self
                .rows
                .iter()
                .find(|entry| entry.dbus_id() == id)
                .ok_or_else(|| {
                    zbus::fdo::Error::InvalidArgs(format!("no menu row with id {id}"))
                })?;
            properties(entry)
        };
        let value = match set.get(&name) {
            Some(value) => value.try_clone().map_err(|error: zbus::zvariant::Error| {
                zbus::fdo::Error::Failed(error.to_string())
            })?,
            None => default_property(&name).ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!("{name:?} is not a dbusmenu property"))
            })?,
        };
        OwnedValue::try_from(value).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    /// A row was chosen — or a menu was opened, or closed, or hovered over.
    ///
    /// Only `clicked` does anything. The other three are the host narrating; a tray that acted on
    /// `hovered` would run a sync because a pointer crossed a row.
    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        self.dispatch(id, event_id);
    }

    /// The group form of `Event`, whose out-arg is **`idErrors` — the ids that could NOT be
    /// handled**, not the ids that were.
    ///
    /// THIS RETURNED EVERY ID, which says the opposite of what it meant: a host reading the
    /// canonical name would have been told that every click in the batch failed. The name is in the
    /// interface XML libdbusmenu-glib compiles in (`<arg type="ai" name="idErrors" direction="out">`)
    /// and `Version = 3` — which this publishes — is what switches a client into event grouping.
    ///
    /// Ids are dispatched at most once each. One D-Bus message asking for the same row ten thousand
    /// times is a message any process on the session bus can send, and every `clicked` behind these
    /// rows spawns a thread to talk to the daemon.
    ///
    /// A `HashSet` rather than a `Vec`, because the first version of that guard was `Vec::contains`
    /// in the loop — quadratic in exactly the input the paragraph above says to expect, which is
    /// spending the CPU it was written to save.
    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        let mut seen = std::collections::HashSet::with_capacity(events.len());
        let mut errors = Vec::new();
        for (id, event_id, _data, _timestamp) in events {
            if !seen.insert(id) {
                continue;
            }
            if !self.dispatch(id, &event_id) {
                errors.push(id);
            }
        }
        errors
    }

    /// Asked immediately before a host draws the menu. `true` means "re-read the layout first".
    ///
    /// ALWAYS TRUE. The rows are a function of the daemon's state and the poll behind them runs
    /// every two seconds; a host that draws its cached copy shows `Pause syncing` on a daemon that
    /// paused a moment ago, which is the one thing `10-tray.md` asks these rows not to do.
    ///
    /// **It also dismisses the panel**, which is not a side effect but the point: before #252 a right
    /// click reached `ContextMenu` and toggled the panel shut, and that was the way out of the one
    /// state `lib.rs` documents as unrecoverable-by-blur — a compositor that refuses the panel the
    /// focus it asked for, so no blur ever arrives to hide it. With a menu published, the right click
    /// goes to the host instead, and the menu would open over a panel that will not leave.
    fn about_to_show(&self, _id: i32) -> bool {
        crate::panel::hide(&self.app);
        true
    }

    /// The group form. Its out-args are `updatesNeeded` and **`idErrors`** — not "removed". Every id
    /// asked about needs an update, for the reason above, and none of them is an error: an id this
    /// menu no longer draws is a host holding a stale layout, which the refresh it is about to make
    /// is what fixes.
    ///
    /// **An empty list answers `[0]` rather than "nothing needs updating".** Whether an empty `ids`
    /// means "all" here is undocumented — the reference XML annotates neither argument, and the
    /// convention is only spelled out for `GetGroupProperties` — so the answer is the one that is
    /// safe under both readings: this menu has exactly one node that can be shown, `about_to_show`
    /// says yes for it unconditionally, and a host told "nothing" would draw a menu that has gone
    /// stale.
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        if ids.is_empty() {
            return (vec![0], Vec::new());
        }
        (ids, Vec::new())
    }

    /// 3 is what libdbusmenu implements and what the measured specimen reports. Claiming a version
    /// this does not implement is how a host decides to call a method that is not here.
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    /// `"normal"`, never `"notice"`. The other value asks a host to draw the menu larger and louder
    /// for an urgent state, and `10-tray.md` rules that out in the same words it rules out
    /// `NeedsAttention` on the item: the needs-you form "adds mass rather than a badge".
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    /// Empty, and published anyway. No row sets an `icon-name` — `10-tray.md` draws a text menu —
    /// but a host that reads the whole property set at import time should find every property the
    /// interface declares rather than an error on one of them.
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    /// The layout changed. The revision is what a host compares against the one it holds.
    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

/// Publish the rows a state draws, and tell any host holding the old ones.
///
/// A no-op when the rows have not changed: `LayoutUpdated` makes a host re-read the menu, and doing
/// that every two seconds is a round trip per tick for a menu nobody has opened.
///
/// `live` gates the SIGNAL alone — the rows are always published, because a host that comes back
/// reads the layout rather than being told about it. See `Sni::update`.
pub async fn set_rows(conn: &Connection, rows: &'static [Entry], live: bool) -> zbus::Result<()> {
    let iface = conn
        .object_server()
        .interface::<_, TrayMenu>(MENU_PATH)
        .await?;
    let revision = {
        let mut menu = iface.get_mut().await;
        if menu.rows == rows {
            return Ok(());
        }
        menu.rows = rows;
        // Wrapping is not a real case at one increment per state change, but a revision that went
        // backwards would be read as older than the one a host holds. Skipping 0 keeps every
        // revision this ever publishes greater than the "nothing fetched yet" a host starts with.
        menu.revision = menu.revision.wrapping_add(1).max(1);
        menu.revision
    };
    if !live {
        return Ok(());
    }
    // Parent 0: the root's children are what changed, which is the whole menu.
    TrayMenu::layout_updated(iface.signal_emitter(), revision, 0).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui_core::state::DaemonState;

    fn root_of(state: DaemonState) -> Item {
        layout(tray_menu::rows_for(state)).expect("the layout builds")
    }

    fn string_prop(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
        props
            .get(key)
            .and_then(|v| String::try_from(v.clone()).ok())
    }

    /// A child variant, unwrapped back into (id, props).
    fn children_of(item: &Item) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        item.2
            .iter()
            .map(|value| {
                let structure = match Value::from(value.clone()) {
                    Value::Structure(structure) => structure,
                    other => panic!("a child came out as {other:?} rather than a struct"),
                };
                let fields = structure.into_fields();
                let id = i32::try_from(fields[0].try_clone().unwrap()).expect("the id is an i32");
                let props = HashMap::<String, OwnedValue>::try_from(
                    OwnedValue::try_from(fields[1].try_clone().unwrap()).unwrap(),
                )
                .expect("the properties are a{sv}");
                (id, props)
            })
            .collect()
    }

    #[test]
    fn the_root_says_it_has_a_submenu() {
        // Drop `children-display` and every host draws an empty menu — the children are still on the
        // wire, and nothing has been told to look at them.
        let root = root_of(DaemonState::Idle);
        assert_eq!(root.0, 0, "the root's id is 0");
        assert_eq!(
            string_prop(&root.1, "children-display").as_deref(),
            Some("submenu")
        );
    }

    #[test]
    fn every_row_reaches_the_wire_with_its_own_id_and_its_label() {
        for state in tray_menu::ALL_STATES {
            let root = root_of(*state);
            let rows = tray_menu::rows_for(*state);
            let children = children_of(&root);
            assert_eq!(children.len(), rows.len(), "{state:?} lost a row");
            for (entry, (id, props)) in rows.iter().zip(children) {
                assert_eq!(
                    id,
                    entry.dbus_id(),
                    "{state:?}: a row changed id in transit"
                );
                match entry {
                    Entry::Separator => {
                        assert_eq!(string_prop(&props, "type").as_deref(), Some("separator"));
                        assert!(!props.contains_key("label"), "a rule with a label");
                    }
                    Entry::Row { .. } => {
                        assert_eq!(string_prop(&props, "label"), Some(entry.folded_label()));
                        assert!(!props.contains_key("type"), "a row typed as something");
                    }
                }
            }
        }
    }

    #[test]
    fn the_sub_labels_survive_the_crossing() {
        // The words `10-tray.md` calls the single worst misunderstanding a tray app can cause, on
        // the surface most people will read them on. A GTK menu item and a QAction are both one
        // string, so this is the only place they can be.
        let labels: Vec<String> = children_of(&root_of(DaemonState::Idle))
            .iter()
            .filter_map(|(_, props)| string_prop(props, "label"))
            .collect();
        assert!(
            labels.contains(&"Close window — keeps syncing".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"Quit — stops syncing".to_string()),
            "{labels:?}"
        );
    }

    #[test]
    fn a_leaf_is_asked_for_by_id_and_answers_childless() {
        let rows = tray_menu::rows_for(DaemonState::Idle);
        let quit = rows
            .iter()
            .find(|entry| matches!(entry, Entry::Row { id, .. } if *id == "quit"))
            .unwrap();
        let item = leaf(rows, quit.dbus_id()).expect("the leaf builds");
        assert_eq!(item.0, quit.dbus_id());
        assert_eq!(string_prop(&item.1, "label"), Some(quit.folded_label()));
        assert!(item.2.is_empty(), "a row has no children");
    }

    #[test]
    fn an_id_the_menu_no_longer_draws_is_answered_rather_than_refused() {
        // A host holding a layout from before the rows changed asks about an id that is gone. That
        // is ordinary, not an error: `Resume syncing` exists in the paused set alone.
        let resume = tray_menu::rows_for(DaemonState::Paused)
            .iter()
            .find(|entry| matches!(entry, Entry::Row { id, .. } if *id == "resume"))
            .unwrap()
            .dbus_id();
        let item = leaf(tray_menu::rows_for(DaemonState::Idle), resume).expect("still answers");
        assert_eq!(item.0, resume);
        assert!(
            item.1.is_empty(),
            "no properties for a row that is not drawn"
        );
    }

    #[test]
    fn a_group_refresh_of_everything_includes_the_root() {
        // THE ROOT IS AN ITEM. A host that refreshes with `GetGroupProperties([], [])` after a
        // LayoutUpdated and gets rows but no id 0 has just lost `children-display`, which is the one
        // property the difference between a menu and an empty menu turns on. Id 0 is not in `rows` —
        // it is synthesised — so a filter over the rows alone drops it silently.
        let rows = tray_menu::rows_for(DaemonState::Idle);
        let all = group_properties(rows, &[]).expect("builds");
        assert_eq!(all.len(), rows.len() + 1, "the root is missing");
        assert_eq!(all[0].0, 0);
        assert_eq!(
            string_prop(&all[0].1, "children-display").as_deref(),
            Some("submenu")
        );

        // And a specific list answers that list, root included when asked for by id.
        let some = group_properties(rows, &[0, 7]).expect("builds");
        assert_eq!(
            some.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![0, 7]
        );
        let just_a_row = group_properties(rows, &[7]).expect("builds");
        assert_eq!(just_a_row.len(), 1, "id 0 arrived unasked for");
    }

    #[test]
    fn a_property_this_menu_leaves_out_answers_with_its_default() {
        // `properties` sets `label` or `type` and nothing else, because everything else has a
        // specified default. Answering an error for `enabled` would tell a host that a row it is
        // drawing has no enabled state; the answer is `true`.
        assert_eq!(default_property("enabled"), Some(Value::from(true)));
        assert_eq!(default_property("visible"), Some(Value::from(true)));
        assert_eq!(default_property("type"), Some(Value::from("standard")));
        // And a name that is not a dbusmenu property at all is still an error rather than a
        // confident empty string.
        assert!(default_property("proton-sync-invented-this").is_none());
    }

    #[test]
    fn the_root_answers_the_same_thing_however_it_is_asked() {
        // Three call paths reach id 0 — GetLayout, GetGroupProperties, GetProperty — and a host that
        // got `submenu` from one and nothing from another would draw the menu or not depending on
        // which one it happened to use.
        let rows = tray_menu::rows_for(DaemonState::Paused);
        let from_layout = layout(rows).expect("builds").1;
        let from_group = group_properties(rows, &[0]).expect("builds")[0].1.clone();
        assert_eq!(
            string_prop(&from_layout, "children-display"),
            string_prop(&from_group, "children-display")
        );
    }

    #[test]
    fn an_id_no_menu_has_ever_drawn_is_not_an_action() {
        // What `EventGroup` reports as an `idErrors` entry, and what `Event` logs rather than
        // silently swallowing. The separator is in here on purpose: it is a row on the wire and NOT
        // an action, so a host that somehow clicked it must not resolve to one.
        assert_eq!(tray_menu::action_for_dbus_id(42), None);
        assert_eq!(tray_menu::action_for_dbus_id(0), None);
        assert_eq!(
            tray_menu::action_for_dbus_id(tray_menu::SEPARATOR_ID),
            None,
            "a separator resolved to an action"
        );
    }

    #[test]
    fn a_click_on_a_stale_menu_still_means_what_its_label_said() {
        // The reason the ids are actions, seen from the host's side.
        // `tray_menu::positions_collide_and_the_worst_pair_is_the_one_10_tray_md_names` establishes
        // the collision itself: `Close window — keeps syncing` stands where `Quit — stops syncing`
        // will stand once a pass starts. This is what saves the click — the id the host was handed
        // for the row under the pointer still resolves to that row after the layout has moved on.
        let settled = tray_menu::rows_for(DaemonState::Idle);
        let position = settled
            .iter()
            .position(|entry| matches!(entry, Entry::Row { id, .. } if *id == "closeWindow"))
            .expect("the settled menu can close the window");
        let close = settled[position];

        let syncing = tray_menu::rows_for(DaemonState::Running);
        assert!(
            matches!(syncing[position], Entry::Row { id, .. } if id == "quit"),
            "the collision this test is built on has moved; see the tray_menu test that computes it"
        );
        assert_eq!(
            tray_menu::action_for_dbus_id(close.dbus_id()),
            Some("closeWindow")
        );
        assert_ne!(close.dbus_id(), syncing[position].dbus_id());
    }
}
