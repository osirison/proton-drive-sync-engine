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
use crate::tray_menu::{self, Entry, TrayPair};
use gui_core::ipc;
use gui_core::pairs::Target;
use gui_core::state::{aggregate_state, derive_state, pair_states, DaemonState, PairState};
use gui_core::wire::{ControlCommand, ControlResponse};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "proton-sync-tray";

/// The rows the fallback text menu was last built from. `None` until there is a fallback tray at all
/// — which on a session with a status-notifier host is for ever.
///
/// **Rows, not the daemon state** (#102 phase 5d). This was a `DaemonState`, and with a row per
/// folder the menu depends on more than the state: pausing one of two folders changes the rows and
/// leaves the aggregate state alone (`Idle` → `Paused` only if no other folder outranks it), and a
/// detector keyed by state would then keep offering `Pause photos` for a folder that is paused.
static FALLBACK_ROWS: Mutex<Option<Vec<Entry>>> = Mutex::new(None);

/// Whether the fallback menu built from `built` has to be rebuilt to show `rows`.
fn fallback_is_stale(built: Option<&[Entry]>, rows: &[Entry]) -> bool {
    built != Some(rows)
}

/// What `install_fallback` has to do about the text menu, decided apart from the window system.
#[derive(Debug, PartialEq, Eq)]
enum FallbackStep {
    /// No tray exists yet: build the menu and the tray.
    Build,
    /// The tray exists and shows other rows: replace its menu.
    Rebuild,
    /// The tray exists and shows these rows already: touch nothing, because replacing the menu the
    /// user has open is at best wasted work.
    Keep,
}

fn fallback_step(tray_exists: bool, built: Option<&[Entry]>, rows: &[Entry]) -> FallbackStep {
    if !tray_exists {
        FallbackStep::Build
    } else if fallback_is_stale(built, rows) {
        FallbackStep::Rebuild
    } else {
        FallbackStep::Keep
    }
}

/// The poll that keeps the glyph current. `10-tray.md` asks for "the daemon's status stream, not a
/// timer", and there is no stream to subscribe to — the control socket answers questions and does
/// not push (#101, E4, explicitly deferred). Two seconds matches the window's own cadence, so the
/// tray and the panel never disagree by more than one tick. DEVIATIONS §82i.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What the tray is showing. Compared before every update so a poll that changes nothing does
/// nothing: telling a host its icon changed makes it reload, and doing that twice a second is a
/// tray icon that flickers for no reason.
///
/// **`rows` is in here because the MENU depends on it and the other fields do not.** An expired
/// session and an unreachable daemon already share a glyph, so a change between them moves only the
/// title — and if a future title ever stopped distinguishing them, the rows would silently stop
/// updating with nothing to point at. What is compared has to be everything the update depends on.
///
/// It was the aggregate `DaemonState` until folders arrived (#102 phase 5d), and a state is not
/// everything the menu depends on any more: pausing ONE of two folders changes a row's label and
/// can leave the state, the glyph and — if the title already named it — the title exactly as they
/// were. The rows themselves are what is compared, so what is published and what is compared cannot
/// differ.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    icon: &'static str,
    title: String,
    rows: Vec<Entry>,
}

/// Most folders the title names before it says `+n more`. A status-notifier `Title` is one line.
const TITLE_FOLDERS_NAMED: usize = 3;

/// The most characters a folders title may have — **one definition**: [`folders_title`] shortens the
/// folder NAMES to fit it and the test reads it. Bounding the count alone left the length to whoever
/// named the folders, and a name may be 64 characters (`[A-Za-z0-9._-]{1,64}`): three of them beside
/// their states ran to about 290 characters on a hover label that is one line.
const TITLE_MAX_CHARS: usize = 140;

/// What every folders title begins with.
const TITLE_PREFIX: &str = "Proton Drive Sync — ";

/// `name`, cut to at most `budget` characters with the last of them `…` when anything was cut.
/// Counted in characters, not bytes: a name cut mid-character is not a name.
fn shorten(name: &str, budget: usize) -> String {
    let budget = budget.max(1);
    if name.chars().count() <= budget {
        return name.to_owned();
    }
    let mut cut: String = name.chars().take(budget - 1).collect();
    cut.push('…');
    cut
}

/// What a folder's state is called in the title.
///
/// A folder that has not had its turn is `waiting` while another folder's pass runs and `starting`
/// while none does. The title already names the folder that is syncing, so it does not say `for
/// documents` again — and a phrase that carried another folder's name could not be bounded by this
/// folder's own budget (`folders_title`). The panel and the window name it.
fn phrase(pair: &PairState) -> &'static str {
    match pair.state {
        DaemonState::Running => "syncing",
        DaemonState::Idle => "up to date",
        DaemonState::Queued if pair.waiting_for.is_some() => "waiting",
        DaemonState::Queued => "starting",
        DaemonState::Paused => "paused",
        DaemonState::AuthExpired => "sign-in expired",
        DaemonState::FirstRun => "nothing synced yet",
        DaemonState::Failed => "last sync failed",
        DaemonState::Unreachable => "daemon unreachable",
    }
}

/// The title when folders disagree: each folder and what it is doing, worst first, at most
/// [`TITLE_FOLDERS_NAMED`] and then `+n more` — `documents paused, photos up to date`. It says what the
/// glyph cannot: the glyph is the worst state (`aggregate_state`), and this is which folders are in
/// it and which are not.
///
/// **Bounded by length as well as count** ([`TITLE_MAX_CHARS`]): what is not a name — the prefix, the
/// states, the separators, `+n more` — is measured, and the rest is shared out evenly as each name's
/// budget. A name that fits is untouched; one that does not is cut with `…`. It is the names that
/// give, because the state beside a name is what the title is for.
fn folders_title(pairs: &[PairState]) -> String {
    let mut ordered: Vec<&PairState> = pairs.iter().collect();
    // Stable, so folders of equal rank keep the order the daemon lists them in.
    ordered.sort_by_key(|pair| std::cmp::Reverse(pair.rank));
    let named: Vec<&PairState> = ordered.iter().take(TITLE_FOLDERS_NAMED).copied().collect();
    let more = ordered.len().saturating_sub(TITLE_FOLDERS_NAMED);
    let tail = (more > 0).then(|| format!("+{more} more"));

    let fixed = TITLE_PREFIX.chars().count()
        // ` {state}` after each name,
        + named.iter().map(|pair| 1 + phrase(pair).chars().count()).sum::<usize>()
        // `, ` between clauses (the tail is one),
        + 2 * named.len().saturating_sub(1)
        + tail.as_ref().map_or(0, |tail| 2 + tail.chars().count());
    let budget = TITLE_MAX_CHARS.saturating_sub(fixed) / named.len().max(1);

    let mut clauses: Vec<String> = named
        .iter()
        .map(|pair| format!("{} {}", shorten(&pair.name, budget), phrase(pair)))
        .collect();
    clauses.extend(tail);
    format!("{TITLE_PREFIX}{}", clauses.join(", "))
}

/// Everything the tray is about to show, from what one reply said. Pure, so what the three surfaces
/// (the glyph, the title and the rows) are built from can be tested without a socket or a desktop.
fn shown_for(
    state: DaemonState,
    states: &[PairState],
    folders: &[TrayPair],
    response: Option<&ControlResponse>,
) -> Shown {
    Shown {
        icon: glyph_for(state),
        title: title_for(state, response, states),
        rows: tray_menu::rows_for(state, folders),
    }
}

/// What one poll tick decides, from the reply alone: what the tray is to show, and whether the
/// indicator has to be told.
struct Tick {
    next: Shown,
    push: bool,
}

/// Everything `spawn_poll` does with a reply that is not I/O — pure but for the roster it refreshes,
/// so the glue from a status reply to what is shown can be driven without a socket, a window or a bus.
///
/// **The roster.** Every tray click is judged against the folder list the app last heard
/// (`roster_has_many`), and this poll runs whether or not a webview is polling, so it is what keeps
/// that list fresh while only the tray is up. A reply that listed no folders (an error, or a daemon
/// older than folder pairs) leaves it as it was.
///
/// **What it shows.** The glyph is the worst folder's state, the title names the folders the glyph
/// cannot, and the rows are the folder group (`pairs_of`). A reply that lists none is ranked by what
/// it says itself and its rows are today's.
///
/// **Whether to push.** A tick is shown once something has shown it: `shown` is what the last
/// successful push carried, and a tick that differs from it in ANY of the icon, the title or the
/// rows is pushed — the rows being in the comparison is what lets pausing one of two folders reach a
/// menu whose glyph and title did not move. `retry_indicator` pushes an unchanged tick, for an
/// indicator that never came up.
fn observe(
    paths: &Mutex<RuntimePaths>,
    reply: &Result<ControlResponse, ipc::IpcError>,
    shown: Option<&Shown>,
    retry_indicator: bool,
) -> Tick {
    let described = derive_state(reply.as_ref());
    let (state, states, folders) = match reply {
        Ok(response) => {
            paths.lock().unwrap().remember_daemon_reply(response);
            let states = pair_states(response, described);
            (
                aggregate_state(described, &states),
                states,
                tray_menu::pairs_of(response, described),
            )
        }
        Err(_) => (described, Vec::new(), Vec::new()),
    };
    let next = shown_for(state, &states, &folders, reply.as_ref().ok());
    let push = retry_indicator || shown != Some(&next);
    Tick { next, push }
}

/// What `shown` becomes after a push. Only a push that REACHED something counts: an unreached one is
/// left unset on purpose, so the next tick re-attempts this exact state instead of waiting for the
/// daemon to change into another one.
fn after_push(next: Shown, reached: bool) -> Option<Shown> {
    reached.then_some(next)
}

/// The label a host shows beside or under the icon. The v1 build computed one of these every five
/// seconds into a function that discarded it; this one reaches `Title` on the item.
fn title_for(
    state: DaemonState,
    response: Option<&ControlResponse>,
    pairs: &[PairState],
) -> String {
    // Two folders or more that are NOT all in the glyph's state: say which is which. When they all
    // are, the plain title is true of every one of them and the same as it was at one folder.
    if pairs.len() >= 2 && pairs.iter().any(|pair| pair.state != state) {
        return folders_title(pairs);
    }
    // The count in `syncing (3 changes)` is the DEFAULT folder's (the reply describes that one). Said
    // of several folders it would be a number about one of them, so with several it is left out.
    let response = if pairs.len() >= 2 { None } else { response };
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
        // Every folder has yet to have its turn and none is running one (a mix is the folders title's).
        DaemonState::Queued => "Proton Drive Sync — starting".into(),
        DaemonState::Paused => "Proton Drive Sync — paused".into(),
        DaemonState::AuthExpired => "Proton Drive Sync — sign-in expired".into(),
        // NOT "0 pending". `counters_unknown()` is true here and 14-behaviour-and-state.md's rule is
        // absolute: unknown is never zero.
        DaemonState::FirstRun => "Proton Drive Sync — nothing synced yet".into(),
        DaemonState::Unreachable => "Proton Drive Sync — daemon unreachable".into(),
        // The daemon IS answering, so this is not "unreachable" — its last pass failed. The title
        // is a one-line hover label and the daemon's own string can be any length, so the string
        // itself stays in the window, where there is a block sized to hold it (#246).
        DaemonState::Failed => "Proton Drive Sync — last sync failed".into(),
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
fn install_fallback(app: &AppHandle, rows: &[Entry]) -> tauri::Result<()> {
    let tray = app.tray_by_id(TRAY_ID);
    let built = FALLBACK_ROWS.lock().unwrap().clone();
    // Rebuilt only when the rows would differ, for the same reason `set_rows` and `set_icon` are
    // no-ops on an unchanged value: this is now reached on every 30-second retry tick as well as
    // on a state change, and a live GTK menu is not a description of a menu — replacing the one
    // the user has open is at best wasted work. The decision is `fallback_step`'s, apart from the
    // window system, so it can be tested.
    match fallback_step(tray.is_some(), built.as_deref(), rows) {
        FallbackStep::Keep => return Ok(()),
        FallbackStep::Rebuild => {
            let menu = fallback_menu(app, rows)?;
            if let Some(tray) = tray {
                tray.set_menu(Some(menu))?;
            }
            *FALLBACK_ROWS.lock().unwrap() = Some(rows.to_vec());
            return Ok(());
        }
        FallbackStep::Build => {}
    }
    let menu = fallback_menu(app, rows)?;
    *FALLBACK_ROWS.lock().unwrap() = Some(rows.to_vec());
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
fn fallback_menu(app: &AppHandle, rows: &[Entry]) -> tauri::Result<Menu<tauri::Wry>> {
    // The items have to outlive the borrows handed to `with_items`, so they are built first and
    // referenced after — a GTK menu item is a live object, not a description of one.
    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = Vec::new();
    for entry in rows {
        match entry {
            Entry::Separator { .. } => items.push(Box::new(PredefinedMenuItem::separator(app)?)),
            // The id is the action in the shared vocabulary — `pause@photos` for a folder's row —
            // so a click on this menu is read by `handle_menu_event` exactly as the native one is.
            Entry::Row { id, .. } => items.push(Box::new(MenuItem::with_id(
                app,
                id.as_ref(),
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
            let polled = tauri::async_runtime::spawn_blocking(move || match socket {
                // An unlocatable socket (#277) IS unreachable, and must reach `derive_state` as
                // such — the offline glyph with its reason, not a skipped tick.
                Err(reason) => Err(ipc::IpcError::Unreachable(reason)),
                // The default pair: the tray acts on it alone until its rows learn to name a pair
                // (#102 phase 5d).
                Ok(socket) => ipc::command(
                    &socket,
                    Target::DEFAULT,
                    ControlCommand::Status,
                    ipc::DEFAULT_TIMEOUT,
                ),
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
            // WHAT THE GLYPH, THE TITLE AND THE MENU ARE ABOUT, whether the roster every tray click
            // is judged against is refreshed, and whether this tick is pushed at all, are
            // `observe`'s — the part of this loop with something to test.
            let Tick { next, push } = observe(
                &app.state::<Mutex<RuntimePaths>>(),
                &reply,
                shown.as_ref(),
                retry_indicator,
            );
            if push {
                // Unset when the push reached nothing, so the next tick re-attempts this exact
                // state rather than waiting for the daemon to change into another one.
                let reached = update(&app, &next).await;
                shown = after_push(next, reached);
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

/// The status-notifier item, as far as `update` needs it: told the glyph, the title and the rows.
///
/// A trait so the one place that hands a tick to the item (`push_shown`) can be driven by a recording
/// stand-in — the real item needs a session bus and an `AppHandle`, which a test may not touch. The
/// future is `Send` because `update` runs inside a spawned task.
#[cfg(target_os = "linux")]
trait Indicator {
    fn push(
        &self,
        icon: &str,
        title: &str,
        rows: &[Entry],
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
}

#[cfg(target_os = "linux")]
impl Indicator for crate::sni::Sni {
    async fn push(&self, icon: &str, title: &str, rows: &[Entry]) -> Result<(), String> {
        self.update(icon, title, rows)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Tell a live indicator what `next` says — the glyph, the title **and the rows**. `true` when it
/// took it.
#[cfg(target_os = "linux")]
async fn push_shown<I: Indicator>(item: &I, next: &Shown) -> bool {
    match item.push(next.icon, &next.title, &next.rows).await {
        Ok(()) => true,
        Err(error) => {
            eprintln!("tray: could not update the indicator: {error}");
            false
        }
    }
}

/// Push a state to whichever indicator exists, bringing one up if none does. `true` when the state
/// reached something.
#[cfg(target_os = "linux")]
async fn update(app: &AppHandle, next: &Shown) -> bool {
    let sni = app.state::<crate::sni::SniState>();
    let mut guard = sni.lock().await;
    if let Some(item) = guard.as_ref() {
        return push_shown(item, next).await;
    }
    // First tick, a session with no host, or a host that had not started yet when this app did.
    // Retried every tick until one of them succeeds — see the call site.
    match crate::sni::Sni::start(
        app.clone(),
        next.icon.to_string(),
        next.title.clone(),
        next.rows.clone(),
    )
    .await
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
            let rows = next.rows.clone();
            let scheduled = app
                .clone()
                .run_on_main_thread(move || {
                    if let Err(error) = install_fallback(&app, &rows) {
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
async fn update(app: &AppHandle, next: &Shown) -> bool {
    let app = app.clone();
    let rows = next.rows.clone();
    app.clone()
        .run_on_main_thread(move || {
            if let Err(error) = install_fallback(&app, &rows) {
                eprintln!("tray: the fallback tray failed: {error}");
            }
        })
        .is_ok()
}

/// `Open Drive Sync`, from whichever of the three menus asked — and `commands::tray_action` calls
/// this rather than keeping the copy it used to have. Two copies of it existed, the panel's row went
/// through one and both native menus through the other, and the bug below was fixed in one of them
/// first: the native menu raised the window and the panel's own row did not. §92b.
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        // `set_focus` alone raised NOTHING when the window was already open behind other windows —
        // the stacking order was identical before and after, so `Open Drive Sync` was inert in the
        // one case a tray row is for. `focus::present` explains what the compositor was refusing,
        // and hops to the main thread itself, which is what lets an async command call this.
        crate::focus::present(&window);
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
        // The rows that talk to the daemon, folder rows included, are `commands::tray_control_row` —
        // the same body the panel's rows run, so the three menus cannot do different things for one id.
        Some(
            row @ (TrayRow::SyncNow
            | TrayRow::Pause
            | TrayRow::Resume
            | TrayRow::PausePair(_)
            | TrayRow::ResumePair(_)),
        ) => control_row_in_background(app, row),
        Some(TrayRow::CloseWindow) => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        // The panel's own button (`review@photos`): no native menu draws it, but it is part of the id
        // space this table reads, so a click that arrived anyway does what the panel's does.
        Some(TrayRow::ReviewPair(name)) => review_in_background(app, name),
        Some(TrayRow::Start) => start_service_in_background(app),
        Some(TrayRow::Quit) => crate::commands::quit_stopping_the_daemon(app.clone()),
        None => eprintln!("tray: no action for menu id {id:?}"),
    }
}

/// `Review them` at two folders or more, off the main thread — `select_for_review` is an async
/// command body (a file write behind it), and the window opens once the folder is selected so it is
/// drawn for the right one. A selection that fails opens the window all the same.
fn review_in_background(app: &AppHandle, name: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(error) =
            tauri::async_runtime::block_on(crate::commands::select_for_review(app.clone(), &name))
        {
            eprintln!(
                "tray: could not select {name:?} for review ({error}); the window opens on the \
                 folder it already shows"
            );
        }
        show_window(&app);
    });
}

/// `Start the sync service`, off the main thread — `send_command`'s shape, for the same reason.
///
/// `handle_menu_event` is called ON the GTK main thread (its doc comment says callers must be), and
/// `start_service_impl` shells `systemctl --user start`, which blocks until the unit reports
/// started. Doing that inline freezes the desktop's tray for seconds and, on the window side of the
/// same loop, is what aborts WebKitGTK (#142/#143).
///
/// `crate::commands::start_service_and_adopt`, NOT `start_service_impl` directly (#359 review) —
/// this row used to call the impl on its own and gate nothing, the same gap the tray PANEL's row
/// had, so a manual `systemctl --user restart proton-syncd` left this door unable to recover the
/// session either. `tauri::async_runtime::block_on` is safe here: this closure already runs on its
/// own dedicated OS thread (never the GTK main thread, never a tokio worker), and the shared
/// function's own internal `spawn_blocking` still keeps the actual `systemctl`/socket I/O off
/// whichever thread ends up polling it.
///
/// Nothing polls the result: the tray's own ~2s status poll is what notices the daemon came up. The
/// failure goes to stderr because a native menu row has no surface to report into — the window's
/// button is the path that shows a reason.
fn start_service_in_background(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<Mutex<RuntimePaths>>();
        match tauri::async_runtime::block_on(crate::commands::start_service_and_adopt(&state)) {
            Ok(detail) => eprintln!("tray: {detail}"),
            Err(error) => eprintln!("tray: could not start the daemon: {error}"),
        }
    });
}

/// A row that talks to the daemon, off the main thread — `start_service_in_background`'s shape, and
/// for the same reason: `handle_menu_event` runs ON the GTK main thread and a control-socket round
/// trip blocks up to `DEFAULT_TIMEOUT`. The reply is the panel's to publish; a native menu row has
/// no surface for it, so a failure reaches stderr from inside `tray_control_row`.
fn control_row_in_background(app: &AppHandle, row: crate::commands::TrayRow) {
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = tauri::async_runtime::block_on(crate::commands::tray_control_row(app, &row));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_run_daemon_is_never_described_with_a_count() {
        // `counters_unknown()` is true for FirstRun, and the tray is the surface most likely to
        // fossilise a zero: it is a string, not a rendered number, so no em-dash rule catches it.
        let title = title_for(DaemonState::FirstRun, None, &[]);
        assert!(!title.contains('0'), "{title}");
        assert!(title.contains("nothing synced yet"), "{title}");
    }

    fn state_of(name: &str, state: DaemonState) -> PairState {
        PairState {
            name: name.to_owned(),
            state,
            rank: gui_core::state::severity(state),
            waiting_for: None,
        }
    }

    /// A folder that has not had its turn, waiting for `behind` (or, with none, not yet reached).
    fn queued_of(name: &str, behind: Option<&str>) -> PairState {
        PairState {
            waiting_for: behind.map(str::to_owned),
            ..state_of(name, DaemonState::Queued)
        }
    }

    fn folder_of(name: &str, state: DaemonState, paused: bool) -> TrayPair {
        TrayPair {
            name: name.to_owned(),
            paused,
            syncing: matches!(state, DaemonState::Running),
            rank: gui_core::state::severity(state),
        }
    }

    /// D3's own example, character for character: the glyph is the worst state and the title says
    /// what it cannot.
    #[test]
    fn the_title_names_the_folders_the_glyph_cannot() {
        let states = [
            state_of("documents", DaemonState::Paused),
            state_of("photos", DaemonState::Idle),
        ];
        let state = aggregate_state(DaemonState::Idle, &states);
        assert_eq!(state, DaemonState::Paused);
        assert_eq!(
            title_for(state, None, &states),
            "Proton Drive Sync — documents paused, photos up to date"
        );
        // Worst first, whatever order the daemon lists them in.
        let swapped = [
            state_of("photos", DaemonState::Idle),
            state_of("documents", DaemonState::Paused),
        ];
        assert_eq!(
            title_for(state, None, &swapped),
            "Proton Drive Sync — documents paused, photos up to date"
        );
    }

    #[test]
    fn the_title_names_three_folders_and_then_says_how_many_more() {
        let states = [
            state_of("a", DaemonState::Idle),
            state_of("b", DaemonState::Failed),
            state_of("c", DaemonState::Paused),
            state_of("d", DaemonState::Idle),
            state_of("e", DaemonState::Idle),
        ];
        assert_eq!(
            title_for(DaemonState::Failed, None, &states),
            "Proton Drive Sync — b last sync failed, c paused, a up to date, +2 more"
        );
        // Exactly three: nothing is left over to count.
        assert_eq!(
            title_for(DaemonState::Failed, None, &states[..3]),
            "Proton Drive Sync — b last sync failed, c paused, a up to date"
        );
    }

    /// The live report: the default folder is in a long pass, the other two have not had their turn.
    /// The title must not say either of them is up to date, and the glyph is the syncing one.
    #[test]
    fn the_title_says_a_folder_without_a_turn_is_waiting_and_never_up_to_date() {
        let states = [
            state_of("documents", DaemonState::Running),
            queued_of("photos", Some("documents")),
            queued_of("videos", Some("documents")),
        ];
        let state = aggregate_state(DaemonState::Idle, &states);
        assert_eq!(state, DaemonState::Running);
        assert_eq!(glyph_for(state), "proton-sync-syncing-symbolic");
        let title = title_for(state, None, &states);
        assert_eq!(
            title,
            "Proton Drive Sync — documents syncing, photos waiting, videos waiting"
        );
        assert!(!title.contains("up to date"), "{title}");
    }

    #[test]
    fn with_nothing_running_the_title_says_starting() {
        // Every folder is due at start-up, and none has been reached yet.
        let all = [queued_of("documents", None), queued_of("photos", None)];
        let state = aggregate_state(DaemonState::Idle, &all);
        assert_eq!(state, DaemonState::Queued);
        assert_eq!(title_for(state, None, &all), "Proton Drive Sync — starting");
        // The glyph is the moving one, not the needs-you one: nothing is being asked of the person.
        assert_eq!(glyph_for(state), "proton-sync-syncing-symbolic");
        // Beside one that has finished, the folders are named and the glyph is not the settled one.
        let mixed = [
            state_of("documents", DaemonState::Idle),
            queued_of("photos", None),
        ];
        let state = aggregate_state(DaemonState::Idle, &mixed);
        assert_eq!(state, DaemonState::Queued);
        assert_ne!(glyph_for(state), glyph_for(DaemonState::Idle));
        assert_eq!(glyph_for(state), "proton-sync-syncing-symbolic");
        assert_eq!(
            title_for(state, None, &mixed),
            "Proton Drive Sync — photos starting, documents up to date"
        );
    }

    /// When every folder is in the glyph's state the title is the one it always was: nothing differs
    /// from the glyph, so there is nothing to name — and at several folders the count in `syncing
    /// (3 changes)` is one folder's, so it is not said.
    #[test]
    fn folders_that_all_agree_get_the_plain_title() {
        let idle = [
            state_of("a", DaemonState::Idle),
            state_of("b", DaemonState::Idle),
        ];
        assert_eq!(
            title_for(DaemonState::Idle, None, &idle),
            "Proton Drive Sync — up to date"
        );
        let busy = [
            state_of("a", DaemonState::Running),
            state_of("b", DaemonState::Running),
        ];
        assert_eq!(
            title_for(DaemonState::Running, None, &busy),
            "Proton Drive Sync — syncing"
        );
    }

    /// `the_menu_signature_is_part_of_shown`. Pausing the fourth of four failed folders changes its
    /// row's label and leaves the glyph and the title exactly as they were — the title names three
    /// and says `+1 more`. If `Shown` compared the icon and the title alone, the menu would go on
    /// offering `Pause d` for a folder that is paused until the daemon-level state changed.
    #[test]
    fn the_menu_signature_is_part_of_shown() {
        let failed = |name: &str| {
            (
                state_of(name, DaemonState::Failed),
                folder_of(name, DaemonState::Failed, false),
            )
        };
        let (sa, fa) = failed("a");
        let (sb, fb) = failed("b");
        let (sc, fc) = failed("c");
        let states_before = [sa, sb, sc, state_of("d", DaemonState::Idle)];
        let folders_before = [fa, fb, fc, folder_of("d", DaemonState::Idle, false)];
        let mut states_after = states_before.clone();
        states_after[3] = state_of("d", DaemonState::Paused);
        let mut folders_after = folders_before.clone();
        folders_after[3] = folder_of("d", DaemonState::Paused, true);

        let before = shown_for(DaemonState::Failed, &states_before, &folders_before, None);
        let after = shown_for(DaemonState::Failed, &states_after, &folders_after, None);
        assert_eq!(
            before.icon, after.icon,
            "the premise: the glyph is the same"
        );
        assert_eq!(before.title, after.title, "the premise: so is the title");
        assert_ne!(before.rows, after.rows, "but the row for d changed");
        assert!(
            before != after,
            "so what was shown changed, and must be shown again"
        );
    }

    /// `the_fallback_follows_the_pair_list`: the text menu is rebuilt when the ROWS change, not only
    /// when the daemon-level state does.
    #[test]
    fn the_fallback_follows_the_pair_list() {
        let both_running = [
            folder_of("a", DaemonState::Running, false),
            folder_of("b", DaemonState::Running, false),
        ];
        let a_paused = [
            folder_of("a", DaemonState::Paused, true),
            folder_of("b", DaemonState::Running, false),
        ];
        // The same aggregate state (`Running` outranks `Paused`), different rows.
        let before = tray_menu::rows_for(DaemonState::Running, &both_running);
        let after = tray_menu::rows_for(DaemonState::Running, &a_paused);
        assert_ne!(before, after, "the premise: pausing a folder changes a row");
        assert!(
            !fallback_is_stale(Some(&before), &before),
            "nothing changed"
        );
        assert!(
            fallback_is_stale(Some(&before), &after),
            "the folder list changed and the state did not: the text menu is out of date"
        );
        assert!(fallback_is_stale(None, &before), "never built is stale");
    }

    #[test]
    fn an_unreachable_daemon_reports_no_counters_at_all() {
        let title = title_for(DaemonState::Unreachable, None, &[]);
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
            // The panel's `N more folders` row: it opens the window, and has no native counterpart.
            "more",
            "syncNow",
            "tryAgain",
            "pause",
            "resume",
            "closeWindow",
            "quit",
            "pause@photos",
            "resume@my-folder.2",
            // The panel's `Review them` at two folders or more, naming the folder that holds them.
            "review@photos",
        ] {
            assert!(
                crate::commands::tray_row(id).is_some(),
                "the panel can send {id:?} and nothing here answers it"
            );
        }
        // And the shapes that are NOT rows: an unknown id must be refused rather than folded into
        // some default, or a typo in a menu table becomes a row that quietly does the wrong thing.
        for id in [
            "sync_now",
            "close_window",
            "",
            "Quit",
            // A folder row must name exactly one folder, and `@` cannot occur in a folder's name.
            "pause@",
            "pause@a@b",
            "@photos",
            "quit@photos",
            "pauseAll",
            "review@",
            "review@a@b",
        ] {
            assert!(
                crate::commands::tray_row(id).is_none(),
                "{id:?} resolved to an action it should not have"
            );
        }
    }

    // ---- from a reply to what is shown (#102 phase 5d review) ---------------------------------------
    //
    // The glue between a status reply and the indicator — `observe`, `after_push`, `fallback_step` and
    // `push_shown` — had no test: eight reverts of it (a folder's flags forced, the poll listing no
    // folders, the roster never refreshed, the fallback ignoring a row change, a tick shown once, the
    // item given no rows) all passed the suite. None of these needs a bus, a window or a socket.

    use crate::tray_menu::fixtures::{reply, Folder};

    fn scratch_paths() -> (tempfile::TempDir, Mutex<RuntimePaths>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve_at(&dir.path().join("proton-sync.toml"));
        (dir, Mutex::new(paths))
    }

    fn ids_of(rows: &[Entry]) -> Vec<String> {
        rows.iter()
            .map(|entry| entry.action().unwrap_or("—").to_owned())
            .collect()
    }

    fn roster_len(paths: &Mutex<RuntimePaths>) -> usize {
        paths.lock().unwrap().daemon.pairs.len()
    }

    /// A reply listing two folders is drawn as two folders: the glyph is the worst one's, the rows are
    /// the folder group, and the roster every tray click is judged against has heard of both.
    ///
    /// Reverts: the poll lists no folders (the whole two-folder menu is off); the poll never refreshes
    /// the roster (`Pause photos` is then refused as a click from a menu that never existed).
    #[test]
    fn a_reply_listing_two_folders_is_drawn_as_two_folders_and_remembered() {
        let (_dir, paths) = scratch_paths();
        let response = reply(&[Folder::new("documents"), Folder::new("photos").paused()]);

        let tick = observe(&paths, &Ok(response), None, false);

        assert_eq!(
            ids_of(&tick.next.rows),
            [
                "open",
                "syncNow",
                "—",
                "resume@photos",
                "pause@documents",
                "—",
                "closeWindow",
                "quit"
            ],
            "the paused folder first, one pause row for each, and no `Pause syncing`"
        );
        assert_eq!(
            tick.next.title,
            "Proton Drive Sync — photos paused, documents up to date"
        );
        assert_eq!(tick.next.icon, glyph_for(DaemonState::Paused));
        assert!(tick.push, "nothing was shown before");
        assert_eq!(
            roster_len(&paths),
            2,
            "a tray click is judged against the folders this poll last heard"
        );
    }

    /// An unreachable daemon draws today's rows and leaves the roster as it was: there is no list to
    /// replace the one the app last heard.
    #[test]
    fn an_unreachable_daemon_draws_todays_rows_and_forgets_no_folder() {
        let (_dir, paths) = scratch_paths();
        paths
            .lock()
            .unwrap()
            .remember_daemon_reply(&reply(&[Folder::new("documents"), Folder::new("photos")]));

        let tick = observe(
            &paths,
            &Err(ipc::IpcError::Unreachable("no socket".into())),
            None,
            false,
        );

        assert_eq!(
            ids_of(&tick.next.rows),
            ["start", "open", "—", "quit"],
            "the rows for a daemon that is not running"
        );
        assert_eq!(roster_len(&paths), 2, "an error replaces no list");
    }

    /// `Shown` is the icon, the title AND the rows: pausing the fourth of four failed folders moves
    /// neither the glyph nor the title (which names three and says `+1 more`), and the menu has to be
    /// pushed all the same — or it goes on offering `Pause d` for a folder that is paused.
    ///
    /// Revert: show a tick once (push only when nothing was shown), or compare the icon and title alone.
    #[test]
    fn a_changed_row_is_pushed_though_the_glyph_and_the_title_did_not_move() {
        let (_dir, paths) = scratch_paths();
        let failing = |name: &'static str| Folder::new(name).failed();
        let before = reply(&[failing("a"), failing("b"), failing("c"), Folder::new("d")]);
        let after = reply(&[
            failing("a"),
            failing("b"),
            failing("c"),
            Folder::new("d").paused(),
        ]);

        let first = observe(&paths, &Ok(before.clone()), None, false);
        let unchanged = observe(&paths, &Ok(before), Some(&first.next), false);
        let changed = observe(&paths, &Ok(after), Some(&first.next), false);

        assert!(first.push, "the first tick is shown");
        assert!(!unchanged.push, "a poll that changed nothing does nothing");
        assert_eq!(
            changed.next.icon, first.next.icon,
            "the premise: same glyph"
        );
        assert_eq!(changed.next.title, first.next.title, "and the same title");
        assert_ne!(changed.next.rows, first.next.rows, "but a different row");
        assert!(changed.push, "so it is shown again");
    }

    /// An indicator that never came up is retried on the cadence even when nothing changed.
    #[test]
    fn a_retry_pushes_an_unchanged_tick() {
        let (_dir, paths) = scratch_paths();
        let response = reply(&[Folder::new("a")]);
        let first = observe(&paths, &Ok(response.clone()), None, false);
        assert!(!observe(&paths, &Ok(response.clone()), Some(&first.next), false).push);
        assert!(observe(&paths, &Ok(response), Some(&first.next), true).push);
    }

    /// A push that reached nothing leaves `shown` unset, so the next tick tries the same state again;
    /// one that reached something records it.
    #[test]
    fn only_a_push_that_reached_something_counts_as_shown() {
        let (_dir, paths) = scratch_paths();
        let tick = observe(&paths, &Ok(reply(&[Folder::new("a")])), None, false);
        assert_eq!(after_push(tick.next.clone(), true), Some(tick.next.clone()));
        assert_eq!(after_push(tick.next, false), None);
    }

    /// The fallback text menu: built when there is no tray, rebuilt when the rows changed, and left
    /// alone when they did not.
    ///
    /// Revert: ignore a row change once a menu exists.
    #[test]
    fn the_fallback_is_built_rebuilt_or_left_alone_by_its_rows() {
        let both = [
            TrayPair {
                name: "a".into(),
                paused: false,
                syncing: false,
                rank: 0,
            },
            TrayPair {
                name: "b".into(),
                paused: false,
                syncing: false,
                rank: 0,
            },
        ];
        let mut a_paused = both.clone();
        a_paused[0].paused = true;
        let before = tray_menu::rows_for(DaemonState::Idle, &both);
        let after = tray_menu::rows_for(DaemonState::Idle, &a_paused);
        assert_ne!(before, after, "the premise");

        assert_eq!(
            fallback_step(false, None, &before),
            FallbackStep::Build,
            "no tray yet"
        );
        assert_eq!(
            fallback_step(false, Some(&before), &before),
            FallbackStep::Build,
            "no tray, whatever was built before"
        );
        assert_eq!(
            fallback_step(true, Some(&before), &after),
            FallbackStep::Rebuild,
            "a folder was paused: the menu offers Resume now"
        );
        assert_eq!(
            fallback_step(true, Some(&before), &before),
            FallbackStep::Keep,
            "the same rows: the open menu is not replaced"
        );
        assert_eq!(
            fallback_step(true, None, &before),
            FallbackStep::Rebuild,
            "a tray whose rows are not recorded cannot be assumed current"
        );
    }

    /// The status-notifier item is told the glyph, the title AND the rows of the tick — the same ones
    /// `observe` decided — and a refusal is reported as not reached.
    ///
    /// Revert: hand the item no rows (a right-click menu that never updates).
    #[cfg(target_os = "linux")]
    #[test]
    fn the_indicator_is_given_the_ticks_glyph_title_and_rows() {
        struct Recorder {
            seen: Mutex<Vec<(String, String, Vec<Entry>)>>,
            refuse: bool,
        }
        impl Indicator for Recorder {
            async fn push(&self, icon: &str, title: &str, rows: &[Entry]) -> Result<(), String> {
                self.seen
                    .lock()
                    .unwrap()
                    .push((icon.to_owned(), title.to_owned(), rows.to_vec()));
                if self.refuse {
                    Err("the host went away".into())
                } else {
                    Ok(())
                }
            }
        }

        let (_dir, paths) = scratch_paths();
        let next = observe(
            &paths,
            &Ok(reply(&[Folder::new("a"), Folder::new("b").paused()])),
            None,
            false,
        )
        .next;

        let item = Recorder {
            seen: Mutex::new(Vec::new()),
            refuse: false,
        };
        assert!(tauri::async_runtime::block_on(push_shown(&item, &next)));
        let seen = item.seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            [(next.icon.to_owned(), next.title.clone(), next.rows.clone())]
        );
        assert!(
            ids_of(&seen[0].2).contains(&"resume@b".to_owned()),
            "the rows are the folder group"
        );

        let refusing = Recorder {
            seen: Mutex::new(Vec::new()),
            refuse: true,
        };
        assert!(
            !tauri::async_runtime::block_on(push_shown(&refusing, &next)),
            "a push the item refused did not reach it"
        );
    }

    // ---- the title is bounded by length as well as count ---------------------------------------------

    fn long(name: char) -> String {
        std::iter::repeat_n(name, 64).collect()
    }

    /// Three folders with 64-character names (the longest a folder may have) and the longest state
    /// phrases ran to about 290 characters. The names give; the states stay.
    ///
    /// Revert: bound the count only (take the names whole).
    #[test]
    fn a_title_of_long_folder_names_stays_under_the_limit() {
        let states = [
            state_of(&long('a'), DaemonState::FirstRun),
            state_of(&long('b'), DaemonState::Unreachable),
            state_of(&long('c'), DaemonState::Failed),
            state_of(&long('d'), DaemonState::Idle),
            state_of(&long('e'), DaemonState::Idle),
        ];
        let title = title_for(DaemonState::Unreachable, None, &states);
        assert!(
            title.chars().count() <= TITLE_MAX_CHARS,
            "{} characters: {title}",
            title.chars().count()
        );
        // What the title is FOR survives: every named folder still carries its state, in order, and
        // the count of the rest.
        assert!(title.starts_with(TITLE_PREFIX), "{title}");
        for phrase in [
            "daemon unreachable",
            "last sync failed",
            "nothing synced yet",
        ] {
            assert!(title.contains(phrase), "{phrase} is gone from {title}");
        }
        assert!(title.ends_with("+2 more"), "{title}");
        // The names were cut, each with the ellipsis, and none was cut to nothing.
        assert!(title.contains('…'), "{title}");
        assert!(!title.contains(&long('b')), "a name went in whole: {title}");
    }

    /// A name that fits is untouched: the limit costs a short-named person nothing.
    #[test]
    fn short_names_are_not_cut() {
        let states = [
            state_of("documents", DaemonState::Paused),
            state_of("photos", DaemonState::Idle),
        ];
        assert_eq!(
            title_for(DaemonState::Paused, None, &states),
            "Proton Drive Sync — documents paused, photos up to date"
        );
        assert_eq!(shorten("photos", 6), "photos");
        assert_eq!(shorten("photos", 5), "phot…");
        assert_eq!(shorten("photos", 1), "…");
        // Characters, not bytes.
        assert_eq!(shorten("ééééé", 3), "éé…");
    }
}
