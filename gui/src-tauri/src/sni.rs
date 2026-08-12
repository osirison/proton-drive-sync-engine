//! The tray item, spoken directly (S8, #187).
//!
//! # Why this exists rather than `tray-icon`
//!
//! `10-tray.md`'s whole interaction is *left click opens the compact panel*. That cannot be built on
//! `tray-icon`'s Linux backend, and the reason is not a missing feature but a missing event:
//! `tray-icon-0.24.1/src/platform_impl/gtk/` contains **no reference to `TrayIconEvent`**. There is
//! no code path that emits one. Three consequences were verified in the vendored source and then on
//! a live session, not read off a changelog:
//!
//!   * the `Click` handler the shipped `tray.rs` registered was dead code on Linux;
//!   * `set_tooltip` is literally `Ok(())` with the argument dropped, so every poll computed a
//!     tooltip string that nothing consumed;
//!   * `rect()` returns `None`, which is why `IMPLEMENTATION-PLAN.md` §6 predicted the indicator's
//!     position would not be queryable and told S8 to fall back to a fixed corner.
//!
//! Introspecting the item libappindicator actually published put it beyond doubt: it exposes
//! `Scroll` and `SecondaryActivate` and **no `Activate` and no `ContextMenu`**. A host has nothing
//! to call. The menu is the only interaction it can offer, which is exactly the text menu design-v2
//! is replacing.
//!
//! # What this does instead
//!
//! Implements `org.kde.StatusNotifierItem` and registers with `org.kde.StatusNotifierWatcher` —
//! the same protocol libappindicator speaks, minus its menu-only shape. The model is
//! **xembedsniproxy**, KDE's own X11 bridge, whose published item is `Activate` + `ContextMenu` +
//! `ItemIsMenu = false` with no `Menu` property at all. Copying a shape Plasma itself ships is
//! better evidence than any spec paragraph about what a host honours.
//!
//! Measured on this project's own KDE/X11 session before any of it was written: a hand-rolled item
//! registered, was read (`GetAll`), and a left click arrived as **`Activate(x: 3192, y: 2112)`** —
//! with the click's screen coordinates, which also disposes of the positioning sub-risk. A symbolic
//! SVG under `IconThemePath` that declared `#ff00ff` rendered **white**, so the desktop does the
//! recolouring `10-tray.md` asks for.
//!
//! # What is deliberately not here
//!
//! **No `com.canonical.dbusmenu`.** Right-click therefore opens the panel rather than a native menu,
//! which `10-tray.md` gives to the menu alone by KDE convention. dbusmenu is a second protocol with
//! its own layout-revision model and is an S8-sized task by itself; the panel already contains every
//! row that menu would have. Recorded as DEVIATIONS §82j and filed as a follow-up.

#![cfg(target_os = "linux")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::Mutex;
use zbus::object_server::SignalEmitter;
use zbus::{connection, interface, proxy, Connection};

/// Where the item lives on our own connection. `xembedsniproxy` uses this path and so does every
/// non-libappindicator item on the bus; libappindicator's `/org/ayatana/NotificationItem/...` is its
/// own convention and nothing requires it.
const ITEM_PATH: &str = "/StatusNotifierItem";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// What the tray is currently showing. Written by the status poll, read by the D-Bus properties.
pub struct TrayItem {
    icon: String,
    title: String,
    app: AppHandle,
}

#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl TrayItem {
    // ---- the four things a host may call.

    /// Left click. The design's primary interaction, and the one libappindicator cannot deliver.
    ///
    /// `x`/`y` are the click in screen coordinates — measured, not hoped for. They go to the panel
    /// so it can open under the indicator instead of at the spec's fixed fallback corner.
    async fn activate(&self, x: i32, y: i32) {
        crate::panel::toggle(&self.app, Some((x, y)));
    }

    /// Right click. `10-tray.md` gives this to the native menu alone, by KDE convention, and this
    /// build has no native menu to give it to — see the module header on dbusmenu. The panel
    /// contains every row that menu would have, so this opens the panel rather than doing nothing:
    /// a right click that produces no response reads as a broken tray, not as a deliberate absence.
    async fn context_menu(&self, x: i32, y: i32) {
        crate::panel::toggle(&self.app, Some((x, y)));
    }

    /// Middle click. Deliberately inert: the design gives it no meaning, and a tray that does
    /// something unlabelled on a click nobody documented is the opposite of what `10-tray.md` asks
    /// for ("the labels say what each does").
    async fn secondary_activate(&self, _x: i32, _y: i32) {}

    /// Scroll. Inert for the same reason.
    async fn scroll(&self, _delta: i32, _orientation: &str) {}

    // ---- the properties. Exactly the set xembedsniproxy publishes, plus the two icon ones.

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "proton-drive-sync"
    }

    /// Shown by hosts that display a label or a tooltip. `tray-icon`'s `set_tooltip` discarded this
    /// on Linux, so until now it went nowhere.
    #[zbus(property)]
    fn title(&self) -> String {
        self.title.clone()
    }

    /// **Always `Active`, never `NeedsAttention`.**
    ///
    /// `NeedsAttention` invites a host to blink the icon or swap in `AttentionIconName`, and the
    /// design has no such state: `10-tray.md` says the needs-you form "adds *mass* rather than a
    /// badge, so it's noticeable without being alarming". A blinking tray icon is precisely the
    /// alarm that sentence rules out. The five forms carry the message; the protocol does not get a
    /// sixth channel to carry it louder.
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    /// The icon-theme NAME, not a path to a bitmap. This is the whole point: a name lets the desktop
    /// load our symbolic SVG at its own panel size and recolour it to its own text colour.
    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.icon.clone()
    }

    /// Where those names resolve. The five SVGs are written here at startup — see `icons.rs`.
    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        crate::icons::theme_dir().to_string_lossy().into_owned()
    }

    /// **`false` is what makes left click reach `Activate`.**
    ///
    /// The property means "this item is only a menu; prefer showing it, or sending `ContextMenu`,
    /// over `Activate`". libappindicator's items are menus in exactly that sense, which is why a
    /// left click on one opens a menu on every desktop. Saying `false` — and publishing no `Menu`
    /// object path, as xembedsniproxy does not — is what asks the host for the click itself.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    /// Emitted when the glyph changes. Hosts cache by icon name, so this is what makes them look
    /// again; `PropertiesChanged` alone is not honoured by every host.
    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// The live item: the connection that owns it, kept so it can be updated and so it stays registered.
///
/// **Dropping the `Connection` unregisters the item** — the watcher notices the bus name vanish and
/// drops it from `RegisteredStatusNotifierItems`. So this is held in Tauri's managed state for the
/// life of the process, not in a local that goes out of scope after `setup`.
pub struct Sni {
    conn: Connection,
    /// Set once the item is registered, so the poll does not try to update an item that never came
    /// up (no watcher on the bus, for instance) and log a failure every two seconds for it.
    live: Arc<AtomicBool>,
}

impl Sni {
    /// Publish the item and register it with the watcher.
    ///
    /// Returns `Err` when there is no watcher on the bus — a session with no SNI host at all, which
    /// is a real configuration (a bare window manager, or GNOME without the AppIndicator extension).
    /// The caller falls back to the Tauri tray rather than leaving the user with no indicator.
    pub async fn start(app: AppHandle, icon: String, title: String) -> zbus::Result<Self> {
        // The spec's own name form: `org.kde.StatusNotifierItem-<pid>-<id>`. A well-known name
        // rather than the unique one, because that is what a real item registers and because the
        // watcher's registry is keyed on it.
        let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        let item = TrayItem {
            icon,
            title,
            app: app.clone(),
        };
        let conn = connection::Builder::session()?
            .name(name.as_str())?
            .serve_at(ITEM_PATH, item)?
            .build()
            .await?;

        StatusNotifierWatcherProxy::new(&conn)
            .await?
            .register_status_notifier_item(&name)
            .await?;

        let live = Arc::new(AtomicBool::new(true));
        watch_for_host_restarts(conn.clone(), name, live.clone());
        Ok(Self { conn, live })
    }

    /// Point the item at a different glyph. A no-op when the name has not changed — a host that is
    /// told its icon changed will reload it, and doing that every two seconds makes a tray icon
    /// flicker for no reason.
    pub async fn set_icon(&self, icon: &str, title: &str) -> zbus::Result<()> {
        if !self.live.load(Ordering::Relaxed) {
            return Ok(());
        }
        let iface = self
            .conn
            .object_server()
            .interface::<_, TrayItem>(ITEM_PATH)
            .await?;
        {
            let mut item = iface.get_mut().await;
            if item.icon == icon && item.title == title {
                return Ok(());
            }
            item.icon = icon.to_string();
            item.title = title.to_string();
        }
        // Both the property change and the SNI-specific signal: hosts differ about which they
        // listen to, and emitting one is how an icon silently stops updating on somebody's desktop.
        let emitter = iface.signal_emitter();
        iface.get().await.icon_name_changed(emitter).await?;
        iface.get().await.title_changed(emitter).await?;
        TrayItem::new_icon(emitter).await?;
        TrayItem::new_title(emitter).await?;
        Ok(())
    }
}

/// Re-register when the host restarts.
///
/// THE CLASSIC WAY AN SNI ITEM DISAPPEARS FOREVER. `RegisterStatusNotifierItem` is a one-shot call
/// into a registry the watcher holds in memory; when plasmashell crashes or is restarted — which it
/// is, routinely, by anyone changing a panel setting — that registry is rebuilt empty and every item
/// that does not register again is simply gone until the application restarts. The user sees a tray
/// icon that vanished and never came back, with nothing in any log.
///
/// The spike did not need this because it ran for two minutes. A daemon-adjacent GUI runs for weeks.
fn watch_for_host_restarts(conn: Connection, name: String, live: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        let dbus = match zbus::fdo::DBusProxy::new(&conn).await {
            Ok(proxy) => proxy,
            Err(error) => {
                eprintln!("tray: cannot watch for host restarts: {error}");
                return;
            }
        };
        let mut changes = match dbus.receive_name_owner_changed().await {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("tray: cannot watch for host restarts: {error}");
                return;
            }
        };
        use futures_util::StreamExt;
        while let Some(signal) = changes.next().await {
            let Ok(args) = signal.args() else { continue };
            if args.name() != WATCHER {
                continue;
            }
            // An empty new owner is the watcher going away; a non-empty one is it coming back, which
            // is the edge to act on. Registering against a watcher that has just vanished would fail
            // and, worse, would leave `live` false with nothing to set it true again.
            if args.new_owner().is_none() {
                live.store(false, Ordering::Relaxed);
                continue;
            }
            match StatusNotifierWatcherProxy::new(&conn).await {
                Ok(watcher) => match watcher.register_status_notifier_item(&name).await {
                    Ok(()) => {
                        live.store(true, Ordering::Relaxed);
                        eprintln!("tray: re-registered with a restarted status-notifier host");
                    }
                    Err(error) => eprintln!("tray: re-registration failed: {error}"),
                },
                Err(error) => eprintln!("tray: no watcher to re-register with: {error}"),
            }
        }
    });
}

/// Guard the `Mutex` around the live item. `None` until `start` succeeds, and permanently `None`
/// when there is no host on the bus.
pub type SniState = Arc<Mutex<Option<Sni>>>;
