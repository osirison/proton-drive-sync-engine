//! Desktop notifications, spoken directly (S9, #188).
//!
//! # Why not `tauri-plugin-notification`
//!
//! `11-notifications.md` puts two properties on every banner that the plugin cannot express:
//!
//!   * **Action buttons.** `Keep them` / `Review`, `Compare` / `Later`, `Try again now` / `Open
//!     Drive Sync`. The plugin's desktop path (`desktop.rs`) builds a `notify_rust::Notification`
//!     from a title, a body and an icon and never touches `.action()`, so there is no way through
//!     it to a button. The absence of a `Delete` button is a deliberate safety property of this
//!     design; the presence of the two safe ones is the rest of it.
//!   * **Never stack two.** "Coalesce within a 30-second window. Never stack more than one Drive
//!     Sync banner." That is `replaces_id` on the wire, which the plugin does not expose either.
//!
//! # What this does instead
//!
//! `org.freedesktop.Notifications` — the same interface `notify-rust` speaks — over the zbus
//! connection S8 already pulls in. Measured on this project's own session before it was written:
//! `GetCapabilities` on Plasma 6.7.4 answers with `actions` and `persistence` among others, and
//! `GetServerInformation` is `("Plasma", "KDE", "6.7.4", "1.2")` — so the risk
//! `IMPLEMENTATION-PLAN.md` §6 flags ("the notification server may not support actions") is real in
//! principle and answered in fact on the two desktops this app targets.
//!
//! A server that does NOT advertise `actions` still gets the banner: the sentence is the part that
//! matters, and both actions are reachable in the window anyway. `capabilities()` reports what the
//! server said so the caller can log it rather than guess.
//!
//! # The copy is not here
//!
//! Every string arrives from the webview, built by `ui/notification.js` out of `ui/copy.js`. That is
//! the whole reason the trigger logic lives in JS: one copy module means the banner, the screen and
//! the tray cannot drift, and `copy-gate.mjs` checks that module against the frames. This file
//! carries no user-visible text at all — `Drive Sync` reaches it as `payload.app`.

#[cfg(target_os = "linux")]
use std::sync::Arc;

use tauri::AppHandle;

/// What the webview asks to be shown. Built by `payloadFor` in `ui/notification.js`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NotifyPayload {
    /// The notification server's application name — `Drive Sync`, not the window's title.
    pub app: String,
    /// Which of the four events this is. Round-tripped back to the webview with an action.
    pub kind: String,
    pub summary: String,
    pub body: String,
    /// A symbolic icon name from the tray theme `icons.rs` installs, so the banner and the
    /// indicator resolve the same drawing.
    pub icon: String,
    pub actions: Vec<NotifyAction>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NotifyAction {
    pub id: String,
    pub label: String,
}

/// What the webview is told when a banner is clicked.
#[derive(Clone, serde::Serialize)]
pub struct ActionEvent {
    pub id: u32,
    pub kind: String,
    pub action: String,
}

/// The one banner that may be on screen, or `None` before the first send.
#[cfg(target_os = "linux")]
pub type NotifierState = Arc<tokio::sync::Mutex<Option<Notifier>>>;

/// Show one banner, connecting on first use.
///
/// CONNECTING LAZILY rather than at startup: a session with no notification server is a session
/// with no banners, and paying a D-Bus round trip at launch to find that out would delay the window
/// for a surface that may never be used. A connection failure is reported and not fatal.
#[cfg(target_os = "linux")]
pub async fn send(app: AppHandle, payload: NotifyPayload) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<NotifierState>().inner().clone();
    let mut guard = state.lock().await;
    if guard.is_none() {
        let notifier = Notifier::connect(app.clone())
            .await
            .map_err(|e| format!("no notification server: {e}"))?;
        // Logged once, on connect, because it is the one runtime answer to the risk
        // `IMPLEMENTATION-PLAN.md` §6 raises: a server without `actions` shows the sentence and
        // drops the two buttons, and nothing else in the app can tell you that happened.
        match notifier.capabilities().await {
            Ok(caps) if !caps.iter().any(|c| c == "actions") => eprintln!(
                "notify: this notification server advertises no `actions` capability — banners will \
                 show without their buttons ({})",
                caps.join(", ")
            ),
            Ok(_) => {}
            Err(error) => eprintln!("notify: could not read the server's capabilities: {error}"),
        }
        *guard = Some(notifier);
    }
    let notifier = guard.as_mut().expect("just connected");
    notifier
        .show(&payload)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Take our banner down, if one is up. A no-op when nothing has ever been sent.
#[cfg(target_os = "linux")]
pub async fn close(app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<NotifierState>().inner().clone();
    let mut guard = state.lock().await;
    match guard.as_mut() {
        Some(notifier) => notifier.close().await.map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Off Linux there is no `org.freedesktop.Notifications`. The window says everything the banner
/// would, so this answers `Ok` rather than failing a build target over an addition.
#[cfg(not(target_os = "linux"))]
pub async fn send(_app: AppHandle, _payload: NotifyPayload) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn close(_app: AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "linux")]
use tauri::Emitter;
#[cfg(target_os = "linux")]
use zbus::{proxy, Connection};

#[cfg(target_os = "linux")]
#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// The connection and the one banner that may be on screen.
#[cfg(target_os = "linux")]
pub struct Notifier {
    connection: Connection,
    /// The id of our live banner, or `None` once the server tells us it closed.
    ///
    /// `replaces_id` IS THE NEVER-STACK RULE, and it is tracked rather than assumed: passing a stale
    /// id to a server that has forgotten it is server-defined behaviour (most create a new banner,
    /// which is exactly the stacking this is meant to prevent), so a `NotificationClosed` for our id
    /// clears it and the next banner is a fresh one.
    live: u32,
    /// Which event the live banner is about, so an `ActionInvoked` can say what was acted on.
    kind: String,
}

#[cfg(target_os = "linux")]
impl Notifier {
    /// Connect and start listening for the two signals. Failing is not fatal: a session with no
    /// notification server is a session with no banners, and everything the banners say is also in
    /// the window.
    pub async fn connect(app: AppHandle) -> zbus::Result<Self> {
        let connection = Connection::session().await?;
        spawn_signal_listener(app, connection.clone()).await?;
        Ok(Self {
            connection,
            live: 0,
            kind: String::new(),
        })
    }

    /// What the server says it can do. `actions` is the one this design cares about.
    pub async fn capabilities(&self) -> zbus::Result<Vec<String>> {
        NotificationsProxy::new(&self.connection)
            .await?
            .get_capabilities()
            .await
    }

    /// Show one banner, replacing ours if one is still up.
    pub async fn show(&mut self, payload: &NotifyPayload) -> zbus::Result<u32> {
        let proxy = NotificationsProxy::new(&self.connection).await?;
        // Flattened id/label pairs, which is what the interface takes.
        let actions: Vec<&str> = payload
            .actions
            .iter()
            .flat_map(|a| [a.id.as_str(), a.label.as_str()])
            .collect();

        let mut hints = std::collections::HashMap::new();
        // `normal`, never `critical`. A critical notification is one the server may refuse to
        // dismiss on a timer, and `11-notifications.md`'s whole argument is that the corner of the
        // screen is not where an irreversible decision is made — the banner points at the window.
        hints.insert("urgency", zbus::zvariant::Value::U8(1));
        hints.insert(
            "desktop-entry",
            zbus::zvariant::Value::new("proton-sync-gui"),
        );

        let id = proxy
            .notify(
                &payload.app,
                self.live,
                &payload.icon,
                &payload.summary,
                &payload.body,
                &actions,
                hints,
                // The server's own default timeout. A sync app deciding how long a desktop shows
                // its banners is the kind of thing "use the desktop's own notification chrome"
                // rules out.
                -1,
            )
            .await?;
        self.live = id;
        self.kind = payload.kind.clone();
        Ok(id)
    }

    /// The event the live banner is about, if the id matches ours.
    pub fn kind_of(&self, id: u32) -> Option<&str> {
        (self.live == id && self.live != 0).then_some(self.kind.as_str())
    }

    /// Forget the live banner once the server says it is gone.
    pub fn forget(&mut self, id: u32) {
        if self.live == id {
            self.live = 0;
            self.kind.clear();
        }
    }

    /// Take our banner down (the webview asking, e.g. because the queue emptied on its own).
    pub async fn close(&mut self) -> zbus::Result<()> {
        if self.live == 0 {
            return Ok(());
        }
        let id = self.live;
        self.live = 0;
        self.kind.clear();
        NotificationsProxy::new(&self.connection)
            .await?
            .close_notification(id)
            .await
    }
}

#[cfg(target_os = "linux")]
/// The two signals, on their own task.
///
/// A SIGNAL STREAM AND NOT A CALLBACK, because `ActionInvoked` is broadcast to every listener on the
/// bus: the id has to be matched against ours before anything happens, or clicking `Discard` in
/// another application's banner would run one of ours. `kind_of` is that check.
async fn spawn_signal_listener(app: AppHandle, connection: Connection) -> zbus::Result<()> {
    use futures_util::StreamExt;
    use tauri::Manager;

    let proxy = NotificationsProxy::new(&connection).await?;
    let mut invoked = proxy.receive_action_invoked().await?;
    let mut closed = proxy.receive_notification_closed().await?;

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                Some(signal) = invoked.next() => {
                    let Ok(args) = signal.args() else { continue };
                    let state = app.state::<NotifierState>();
                    let kind = {
                        let guard = state.lock().await;
                        guard.as_ref().and_then(|n| n.kind_of(args.id).map(str::to_owned))
                    };
                    // Not ours: another application's banner, or one of ours the server has already
                    // forgotten. Silence is the only correct response.
                    let Some(kind) = kind else { continue };
                    let _ = app.emit(
                        "notification-action",
                        ActionEvent { id: args.id, kind, action: args.action_key.clone() },
                    );
                }
                Some(signal) = closed.next() => {
                    let Ok(args) = signal.args() else { continue };
                    let state = app.state::<NotifierState>();
                    let mut guard = state.lock().await;
                    if let Some(notifier) = guard.as_mut() {
                        notifier.forget(args.id);
                    }
                }
                else => break,
            }
        }
    });
    Ok(())
}
