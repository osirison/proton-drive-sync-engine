//! The system tray (S8, #187) — the glyph, and what drives it.
//!
//! # What changed, and why it is not a refactor
//!
//! The v1 tray was a text menu where the label WAS the status report: `Sync now (3 pending)`,
//! `Resolve 1 conflict`, `Close window (keeps syncing in the tray)`. `10-tray.md` replaces it with
//! the compact panel — you see the state rather than parsing a list of verbs — and that turned out
//! to be unreachable through the library the v1 tray was built on. `sni.rs` carries the evidence;
//! the short version is that libappindicator publishes no `Activate` method, so no click a user
//! makes can reach this program.
//!
//! So the indicator is `sni.rs` now, and this module is what feeds it: one poll, one place that
//! decides what the tray is showing, and a fallback for a session with no status-notifier host.
//!
//! # Two paths deleted rather than ported
//!
//! Both were dead on Linux and neither was visible as dead:
//!
//!   * the `TrayIconEvent::Click` handler. `tray-icon`'s GTK backend emits no such event, ever, so
//!     "left click toggles the window into view" never happened. It read as working code.
//!   * `tooltip_for`. `set_tooltip` on Linux is `Ok(())` with the argument dropped, so this built a
//!     status string every five seconds and threw it away. The SNI item's `Title` property is the
//!     surface that actually shows it, and it is fed below.
//!
//! # The fallback
//!
//! A session with no `org.kde.StatusNotifierWatcher` — a bare window manager, GNOME without the
//! AppIndicator extension — gets the Tauri tray, because no indicator at all is worse than a menu.
//! It is a text menu with the design's own labels, and it cannot open the panel: without `Activate`
//! there is no click to open it on. That is the whole reason for `sni.rs`, restated as a fallback.

use crate::config_path::RuntimePaths;
use gui_core::ipc;
use gui_core::state::{derive_state, DaemonState};
use gui_core::wire::{ControlCommand, ControlResponse};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "proton-sync-tray";

/// The state the fallback text menu was last built for. `None` until there is a fallback tray at all
/// — which on a session with a status-notifier host is for ever.
static FALLBACK_STATE: Mutex<Option<DaemonState>> = Mutex::new(None);

/// The poll that keeps the glyph current. `10-tray.md` asks for "the daemon's status stream, not a
/// timer", and there is no stream to subscribe to — the control socket answers questions and does
/// not push (#101, E4, explicitly deferred). Two seconds matches the window's own cadence, so the
/// tray and the panel never disagree by more than one tick. DEVIATIONS §82i.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What the tray is showing. Compared before every update so a poll that changes nothing does
/// nothing: telling a host its icon changed makes it reload, and doing that twice a second is a
/// tray icon that flickers for no reason.
///
/// **`state` is in here because the MENU depends on it and the other two fields do not.** An expired
/// session and an unreachable daemon already share a glyph, so a change between them moves only the
/// title — and if a future title ever stopped distinguishing them, the rows would silently stop
/// updating with nothing to point at. What is compared has to be everything the update depends on.
#[derive(Clone, PartialEq, Eq)]
struct Shown {
    icon: &'static str,
    title: String,
    state: DaemonState,
}

/// The label a host shows beside or under the icon. The v1 build computed one of these every five
/// seconds into a function that discarded it; this one reaches `Title` on the item.
fn title_for(state: DaemonState, response: Option<&ControlResponse>) -> String {
    match state {
        DaemonState::Running => {
            // NOT `pending_changes` ALONE, and the live daemon proved it within a minute of this
            // shipping: the tray read `syncing (0 pending)` during a real pass. `pending_changes` is
            // the filesystem-watch queue, so a pass driven entirely by Proton — a second device
            // uploading, the first reconcile after a restart — carries an empty queue while
            // downloading. The plan knows: `uploads + downloads` is what the pass will move.
            //
            // The same trap S1 documents on the headline (`Syncing 0 changes` with a literal 0
            // inside the mark), reached by a different route. The panel and the tray title now
            // answer with the same number.
            let moving = response
                .and_then(|r| r.last_plan_summary.as_ref())
                .map(|s| s.uploads + s.downloads);
            let queued = response.map(|r| r.pending_changes);
            match moving.or(queued) {
                Some(n) => format!("Proton Drive Sync — syncing ({n} changes)"),
                None => "Proton Drive Sync — syncing".into(),
            }
        }
        DaemonState::Idle => "Proton Drive Sync — up to date".into(),
        DaemonState::Paused => "Proton Drive Sync — paused".into(),
        DaemonState::AuthExpired => "Proton Drive Sync — sign-in expired".into(),
        // NOT "0 pending". `counters_unknown()` is true here and 14-behaviour-and-state.md's rule is
        // absolute: unknown is never zero.
        DaemonState::FirstRun => "Proton Drive Sync — nothing synced yet".into(),
        DaemonState::Unreachable => "Proton Drive Sync — daemon unreachable".into(),
    }
}

/// Install the tray and start the poll.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    if let Err(error) = crate::icons::install() {
        // Not fatal: the SNI item still comes up, and a host that cannot resolve the name shows a
        // blank icon rather than nothing. Loud in the log, because a blank tray icon is otherwise
        // unexplainable.
        eprintln!("tray: could not write the glyph theme directory: {error}");
    }
    spawn_poll(app.clone());
    Ok(())
}

/// The Tauri tray, for a session with no status-notifier host. Built only when `sni.rs` could not
/// register — two indicators for one app is worse than a plain one.
///
/// **The rows follow the state on every tick**, which they did not before #252: this returned early
/// whenever the tray already existed, so the fallback menu was whatever the FIRST poll decided and
/// stayed that way for the life of the process. A session that started while the daemon was down
/// offered `Try again now` and never `Pause syncing`, on the one desktop with no panel to correct
/// it. The SNI item never had the bug — it has always been re-fed by `update` — which is why the
/// text menu is the copy that quietly went stale.
fn install_fallback(app: &AppHandle, state: DaemonState) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // Rebuilt only when the rows would differ, for the same reason `set_rows` and `set_icon` are
        // no-ops on an unchanged value: this is now reached on every 30-second retry tick as well as
        // on a state change, and a live GTK menu is not a description of a menu — replacing the one
        // the user has open is at best wasted work.
        if FALLBACK_STATE.lock().unwrap().as_ref() == Some(&state) {
            return Ok(());
        }
        let menu = fallback_menu(app, state)?;
        tray.set_menu(Some(menu))?;
        *FALLBACK_STATE.lock().unwrap() = Some(state);
        return Ok(());
    }
    let menu = fallback_menu(app, state)?;
    *FALLBACK_STATE.lock().unwrap() = Some(state);
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("no default window icon for the fallback tray".into())
        })?)
        .menu(&menu)
        // No `on_tray_icon_event`. The GTK backend emits none — that is the fact S8 turned on, and
        // a handler here would be the same dead code it deleted.
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

/// The fallback's rows, built from `tray_menu` — the same table the native right-click menu (#252)
/// is built from, so the three surfaces cannot drift into three menus.
///
/// The sub-labels are FOLDED INTO THE LABEL with an em-dash, in `Entry::folded_label`. A GTK menu
/// item is a single string; `Close window` and `Quit` carry a second baseline-aligned span in the
/// panel and cannot here. What they must not do is lose the words: 10-tray.md calls this "the single
/// worst misunderstanding a tray app can cause", and the v1 build spelled it out for the same
/// reason. DEVIATIONS §82k.
fn fallback_menu(app: &AppHandle, state: DaemonState) -> tauri::Result<Menu<tauri::Wry>> {
    // The items have to outlive the borrows handed to `with_items`, so they are built first and
    // referenced after — a GTK menu item is a live object, not a description of one.
    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = Vec::new();
    for entry in crate::tray_menu::rows_for(state) {
        match entry {
            crate::tray_menu::Entry::Separator => {
                items.push(Box::new(PredefinedMenuItem::separator(app)?))
            }
            crate::tray_menu::Entry::Row { id, .. } => items.push(Box::new(MenuItem::with_id(
                app,
                *id,
                entry.folded_label(),
                true,
                None::<&str>,
            )?)),
        }
    }
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        items.iter().map(|item| item.as_ref()).collect();
    Menu::with_items(app, &refs)
}

fn spawn_poll(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut shown: Option<Shown> = None;
        let mut tick: u64 = 0;
        loop {
            let socket = {
                let state = app.state::<Mutex<RuntimePaths>>();
                let guard = state.lock().unwrap();
                guard.socket_path.clone()
            };
            // The control socket is synchronous and blocks up to DEFAULT_TIMEOUT against a daemon
            // that is not answering. On the async runtime that would stall every other task —
            // including the D-Bus connection serving the tray item, which is how a tray stops
            // responding to clicks whenever the daemon is down.
            let polled = tauri::async_runtime::spawn_blocking(move || {
                ipc::command(&socket, ControlCommand::Status, ipc::DEFAULT_TIMEOUT)
            })
            .await;
            // A join failure is this task's own bug, not the daemon's, and it must not be folded
            // into "daemon unreachable" — that would paint the offline glyph over a daemon that is
            // running perfectly well. Skip the tick and leave the last glyph up.
            let Ok(reply) = polled else {
                eprintln!("tray: status poll did not complete; leaving the glyph as it was");
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            };
            let state = derive_state(reply.as_ref());
            let response = reply.ok();

            let next = Shown {
                icon: glyph_for(state),
                title: title_for(state, response.as_ref()),
                state,
            };
            // **A tick is only "shown" once something has shown it.** Two reasons it might not have
            // been, and the second one is a bug this file shipped: the indicator may never have come
            // up. `Sni::start` runs only when `update` does, and `update` ran only on a state change
            // — so a session where this app and the panel start together and this one wins the race
            // registered nothing, fell back to the text menu, and stayed there for as long as an
            // idle daemon stayed idle.
            //
            // Retried on a 30-second cadence rather than every tick, because `Sni::start` opens a
            // fresh D-Bus connection: fast enough that a panel starting late is a blip, cheap enough
            // that a session which will never have a host is not paying for one every two seconds.
            tick = tick.wrapping_add(1);
            let retry_indicator = tick.is_multiple_of(15) && !indicator_is_up(&app).await;
            if shown.as_ref() != Some(&next) || retry_indicator {
                if update(&app, state, &next).await {
                    shown = Some(next);
                } else {
                    // Left unset on purpose: the next tick re-attempts this exact state rather than
                    // waiting for the daemon to change into another one.
                    shown = None;
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

#[cfg(target_os = "linux")]
fn glyph_for(state: DaemonState) -> &'static str {
    crate::icons::glyph_for(state)
}

/// Off Linux there is no SNI and no symbolic theme; the name is unused but the poll's shape is
/// shared, so it still has to produce one.
#[cfg(not(target_os = "linux"))]
fn glyph_for(_state: DaemonState) -> &'static str {
    "proton-sync"
}

/// Is there an indicator carrying the state right now? The retry above turns on this.
#[cfg(target_os = "linux")]
async fn indicator_is_up(app: &AppHandle) -> bool {
    app.state::<crate::sni::SniState>().lock().await.is_some()
}

#[cfg(not(target_os = "linux"))]
async fn indicator_is_up(app: &AppHandle) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}

/// Push a state to whichever indicator exists, bringing one up if none does. `true` when the state
/// reached something.
#[cfg(target_os = "linux")]
async fn update(app: &AppHandle, state: DaemonState, next: &Shown) -> bool {
    let rows = crate::tray_menu::rows_for(state);
    let sni = app.state::<crate::sni::SniState>();
    let mut guard = sni.lock().await;
    if let Some(item) = guard.as_ref() {
        if let Err(error) = item.update(next.icon, &next.title, rows).await {
            eprintln!("tray: could not update the indicator: {error}");
            return false;
        }
        return true;
    }
    // First tick, a session with no host, or a host that had not started yet when this app did.
    // Retried every tick until one of them succeeds — see the call site.
    match crate::sni::Sni::start(app.clone(), next.icon.to_string(), next.title.clone(), rows).await
    {
        Ok(item) => {
            eprintln!("tray: registered a status-notifier item");
            *guard = Some(item);
            drop(guard);
            // THE FALLBACK HAS TO GO, or a session that started before its panel shows TWO
            // indicators for one app — and the surviving text menu is the stale one, because
            // `install_fallback` is never reached again once the item is up.
            let app = app.clone();
            let _ = app.clone().run_on_main_thread(move || {
                if app.tray_by_id(TRAY_ID).is_some() {
                    app.remove_tray_by_id(TRAY_ID);
                    eprintln!("tray: removed the fallback text menu");
                }
            });
            true
        }
        Err(error) => {
            // Once. This retries every two seconds now, and a bare window manager would otherwise
            // write this line 30 times a minute for the life of the session.
            if !NO_HOST_REPORTED.swap(true, Ordering::Relaxed) {
                eprintln!("tray: no status-notifier host ({error}); falling back to a text menu");
            }
            drop(guard);
            let app = app.clone();
            // The fallback DID take the state — provided the hop reached the event loop. Returning
            // `true` unconditionally here would record a tick as shown on a menu that was never
            // built; returning it when the hop succeeded is what is knowable from this side, since
            // `install_fallback`'s own failure happens on the other thread and logs there. That one
            // is covered too: `indicator_is_up` stays false while there is no SNI item, so the
            // 30-second retry comes back around.
            let scheduled = app
                .clone()
                .run_on_main_thread(move || {
                    if let Err(error) = install_fallback(&app, state) {
                        eprintln!("tray: the fallback tray failed too: {error}");
                    }
                })
                .is_ok();
            if !scheduled {
                eprintln!("tray: could not reach the event loop to build the fallback menu");
            }
            scheduled
        }
    }
}

/// Whether the "no status-notifier host" line has been written. See `update`.
#[cfg(target_os = "linux")]
static NO_HOST_REPORTED: AtomicBool = AtomicBool::new(false);

#[cfg(not(target_os = "linux"))]
async fn update(app: &AppHandle, state: DaemonState, _next: &Shown) -> bool {
    let app = app.clone();
    app.clone()
        .run_on_main_thread(move || {
            if let Err(error) = install_fallback(&app, state) {
                eprintln!("tray: the fallback tray failed: {error}");
            }
        })
        .is_ok()
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// A row was chosen, on either native menu — the fallback's, or the dbusmenu's (#252) — dispatched
/// through the SAME table the panel uses.
///
/// `commands::tray_row` is that table. Before it there were two: this file matched `sync_now` and
/// `commands::tray_action` matched `syncNow`, each understanding its own menu perfectly and neither
/// understanding the other's — while a comment here claimed they were one id space. Nothing was
/// broken, which is what made it worth fixing rather than leaving: two vocabularies that happen to
/// work are a trap for whoever edits one of them.
///
/// **Callers must already be on the main thread.** Both window paths below are GTK calls.
pub fn handle_menu_event(app: &AppHandle, id: &str) {
    use crate::commands::TrayRow;
    match crate::commands::tray_row(id) {
        Some(TrayRow::Open) => show_window(app),
        Some(TrayRow::SyncNow) => send_command(app, ControlCommand::Syncnow),
        Some(TrayRow::Pause) => send_command(app, ControlCommand::Pause),
        Some(TrayRow::Resume) => send_command(app, ControlCommand::Resume),
        Some(TrayRow::CloseWindow) => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        Some(TrayRow::Quit) => crate::commands::quit_stopping_the_daemon(app.clone()),
        None => eprintln!("tray: no action for menu id {id:?}"),
    }
}

fn send_command(app: &AppHandle, command: ControlCommand) {
    let app = app.clone();
    std::thread::spawn(move || {
        let socket = {
            let state = app.state::<Mutex<RuntimePaths>>();
            let guard = state.lock().unwrap();
            guard.socket_path.clone()
        };
        let _ = ipc::command(&socket, command, ipc::DEFAULT_TIMEOUT);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_run_daemon_is_never_described_with_a_count() {
        // `counters_unknown()` is true for FirstRun, and the tray is the surface most likely to
        // fossilise a zero: it is a string, not a rendered number, so no em-dash rule catches it.
        let title = title_for(DaemonState::FirstRun, None);
        assert!(!title.contains('0'), "{title}");
        assert!(title.contains("nothing synced yet"), "{title}");
    }

    #[test]
    fn an_unreachable_daemon_reports_no_counters_at_all() {
        let title = title_for(DaemonState::Unreachable, None);
        assert!(!title.contains("pending"), "{title}");
    }

    #[test]
    fn both_indicators_speak_one_vocabulary() {
        // THE BUG THIS PINS shipped and worked: the fallback menu built `sync_now`/`try_again`/
        // `close_window` and the panel sent `syncNow`/`tryAgain`/`closeWindow`, each dispatched by
        // its own `match` in its own file. Nothing failed — every handler understood its own menu —
        // so nothing but a comment claimed they were the same thing, and the comment was wrong.
        //
        // These strings are `ui/compact.js`'s `TRAY_MENU` ids. `gui/test/compact.test.js` holds the
        // JS side of the same contract; this is the half that would otherwise drift silently,
        // because Rust does not move when a JS table does.
        for id in [
            "open",
            "review",
            "syncNow",
            "tryAgain",
            "pause",
            "resume",
            "closeWindow",
            "quit",
        ] {
            assert!(
                crate::commands::tray_row(id).is_some(),
                "the panel can send {id:?} and nothing here answers it"
            );
        }
        // And the shapes that are NOT rows: an unknown id must be refused rather than folded into
        // some default, or a typo in a menu table becomes a row that quietly does the wrong thing.
        for id in ["sync_now", "close_window", "", "Quit"] {
            assert!(
                crate::commands::tray_row(id).is_none(),
                "{id:?} resolved to an action it should not have"
            );
        }
    }
}
