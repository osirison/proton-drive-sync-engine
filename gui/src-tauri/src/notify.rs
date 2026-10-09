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
    /// The folder pair the banner is about, **at two folders or more** (#102 phase 5e); absent at one.
    ///
    /// The NAME, not a rendering: `app` above is `Drive Sync · photos` for a server to draw, and a click
    /// has to act on `photos` however long after the banner went up it lands and whatever the window is
    /// showing by then. Kept beside the notification id and put back on [`ActionEvent`], which is the
    /// only route the name has — the server's `ActionInvoked` signal carries an id and an action key and
    /// nothing else. `#[serde(default)]`: the one-folder payload never sent it, and this field is the
    /// only one of the struct that may be missing.
    #[serde(default)]
    pub pair: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NotifyAction {
    pub id: String,
    pub label: String,
}

/// What the webview is told when a banner is clicked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActionEvent {
    pub id: u32,
    pub kind: String,
    pub action: String,
    /// The folder pair the banner was sent about, or nothing for a banner sent without one (the
    /// one-folder event is the three fields above, byte for byte).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
}

/// The banner we sent LAST, kept after it closes — what a click is attributed to.
///
/// Pure and free of the bus, so the attribution can be tested without a notification server: a click is
/// ours only if its id is this one, and then it carries everything the webview needs to act on the right
/// thing — the event kind AND the folder the banner was about.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sent {
    id: u32,
    kind: String,
    pair: Option<String>,
}

#[cfg(target_os = "linux")]
impl Sent {
    fn of(id: u32, payload: &NotifyPayload) -> Self {
        Self {
            id,
            kind: payload.kind.clone(),
            pair: payload.pair.clone(),
        }
    }

    /// The event for a click on `action` of notification `id`, or `None` when `id` is not this banner's
    /// (another application's, or one of ours the server has already forgotten).
    fn action_event(&self, id: u32, action: &str) -> Option<ActionEvent> {
        (self.id == id).then(|| ActionEvent {
            id,
            kind: self.kind.clone(),
            action: action.to_owned(),
            pair: self.pair.clone(),
        })
    }
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
        let mut notifier = Notifier::connect(app.clone())
            .await
            .map_err(|e| format!("no notification server: {e}"))?;
        // Logged once, on connect, because it is the one runtime answer to the risk
        // `IMPLEMENTATION-PLAN.md` §6 raises: a server without `actions` shows the sentence and
        // drops the two buttons, and nothing else in the app can tell you that happened.
        match notifier.capabilities().await {
            Ok(caps) => {
                if !caps.iter().any(|c| c == "actions") {
                    eprintln!(
                        "notify: this notification server advertises no `actions` capability — \
                         banners will show without their buttons ({})",
                        caps.join(", ")
                    );
                }
                notifier.note_capabilities(&caps);
            }
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
    /// The id to pass as `replaces_id`, or `0` once the server says ours closed.
    ///
    /// `replaces_id` IS THE NEVER-STACK RULE, and it is tracked rather than assumed: measured on
    /// Plasma, `Notify(replaces_id: 26)` returns `26` and replaces in place, and an id the server
    /// never issued comes back unchanged — but the spec requires neither, and a server that
    /// allocates a fresh id for one it has forgotten would stack the banner this rule prevents.
    live: u32,
    /// The id, event and folder of the banner we sent LAST, kept after it closes.
    ///
    /// NOT CLEARED WITH `live`, and that asymmetry is the point. A server emits `ActionInvoked` and
    /// `NotificationClosed` for the same click, `tokio::select!` picks randomly between two ready
    /// branches, and clearing the attribution on close would therefore drop the click about half
    /// the time — including the click that keeps someone's files.
    last: Option<Sent>,
    /// Whether to assume the server parses markup in a body. Read once, at connect — and `true`
    /// until it is, which is the direction that fails safe. See [`assume_markup`].
    markup: bool,
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
            last: None,
            markup: assume_markup(None),
        })
    }

    /// What the server says it can do. `actions` is the one this design cares about.
    ///
    /// ON THE SAME DEADLINE AS `show`, and for the same reason one call earlier: this runs on the
    /// connect path with the state mutex held, so a server that accepts the call and never answers
    /// would block every later banner AND every close for the life of the process. `show` was given
    /// a timeout for exactly that and this was left without one, which is the same bug at the only
    /// other place it can happen.
    pub async fn capabilities(&self) -> zbus::Result<Vec<String>> {
        let proxy = NotificationsProxy::new(&self.connection).await?;
        tokio::time::timeout(DBUS_DEADLINE, proxy.get_capabilities())
            .await
            .map_err(|_| zbus::Error::Failure("the notification server did not answer".into()))?
    }

    /// Record what the server said it can do. Read once, at connect.
    pub fn note_capabilities(&mut self, caps: &[String]) {
        self.markup = assume_markup(Some(caps));
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
        // The application's desktop-file id, which is what a server matches to find our name and
        // icon — `tauri.conf.json`'s `identifier`, and the file `setup.sh` installs.
        hints.insert(
            "desktop-entry",
            zbus::zvariant::Value::new("app.protondrivesync.engine"),
        );

        let icon = icon_path(&payload.icon);
        let body = if self.markup {
            escape_markup(&payload.body)
        } else {
            payload.body.clone()
        };

        // A ROUND TRIP WITH A DEADLINE — see `DBUS_DEADLINE`.
        let id = tokio::time::timeout(
            DBUS_DEADLINE,
            proxy.notify(
                &payload.app,
                self.live,
                &icon,
                &payload.summary,
                &body,
                &actions,
                hints,
                // The server's own default timeout. A sync app deciding how long a desktop shows
                // its banners is the kind of thing "use the desktop's own notification chrome"
                // rules out.
                -1,
            ),
        )
        .await
        .map_err(|_| zbus::Error::Failure("the notification server did not answer".into()))??;

        self.live = id;
        self.last = Some(Sent::of(id, payload));
        Ok(id)
    }

    /// The event a click on `action` of notification `id` is about, if `id` is one of ours — with the
    /// folder the banner was sent about.
    ///
    /// Reads `last` rather than `live`, so a click still resolves when the close signal for the very
    /// same click has already been handled.
    pub fn action_event(&self, id: u32, action: &str) -> Option<ActionEvent> {
        self.last.as_ref()?.action_event(id, action)
    }

    /// Stop replacing a banner the server says is gone. The attribution in `last` survives it.
    pub fn forget(&mut self, id: u32) {
        if self.live == id {
            self.live = 0;
        }
    }

    /// Take our banner down (the webview asking, because the thing it was about resolved itself).
    pub async fn close(&mut self) -> zbus::Result<()> {
        if self.live == 0 {
            return Ok(());
        }
        let id = self.live;
        let proxy = NotificationsProxy::new(&self.connection).await?;
        tokio::time::timeout(DBUS_DEADLINE, proxy.close_notification(id))
            .await
            .map_err(|_| zbus::Error::Failure("the notification server did not answer".into()))??;
        // AFTER the call, not before: a failed close leaves the banner up, and forgetting its id
        // first would mean the next send opened a second one beside it.
        self.live = 0;
        Ok(())
    }
}

/// The icon a server can actually load.
///
/// `icons.rs` writes the five symbolic SVGs to `$XDG_RUNTIME_DIR/proton-sync-tray` and hands that
/// path to the STATUS-NOTIFIER HOST as `IconThemePath`. A notification server is a different process
/// again and is told nothing, so a bare `proton-sync-attention-symbolic` resolves against the
/// system icon themes — where this application installs nothing — and the banner arrives with no
/// icon at all. The spec allows an absolute path, so that is what goes on the wire when the file is
/// there, and the name survives as the fallback for a desktop that does have it installed.
#[cfg(target_os = "linux")]
fn icon_path(name: &str) -> String {
    let file = crate::icons::theme_dir().join(format!("{name}.svg"));
    if file.is_file() {
        file.to_string_lossy().into_owned()
    } else {
        name.to_string()
    }
}

/// How long any call to the notification server may take before it is treated as unanswered.
///
/// EVERY ROUND TRIP HERE RUNS UNDER THE STATE MUTEX, so one that never returns holds it for the
/// life of the process — and takes every later banner and every close with it. Ten seconds is far
/// longer than any notification server takes and far shorter than "for ever".
#[cfg(target_os = "linux")]
const DBUS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether to escape the body, from what the server said it can do — or from not knowing.
///
/// UNKNOWN MEANS ESCAPE. `GetCapabilities` is a round trip and can fail: a transient bus error, a
/// server mid-restart, a minimal server that answers `Notify` and not this. Defaulting to "does not
/// parse markup" would send a user-chosen path into a parser that does, and the two failures are
/// not the same size — an unescaped `<b>` in a filename is markup injection into a banner, while an
/// over-escaped one is a visible `&amp;` in a path on a server we could not ask. Fail towards the
/// second. Found by Copilot's second pass, in code written to answer its first.
#[cfg(target_os = "linux")]
pub fn assume_markup(caps: Option<&[String]>) -> bool {
    match caps {
        Some(caps) => caps.iter().any(|c| c == "body-markup"),
        None => true,
    }
}

/// The three characters a `body-markup` server parses.
///
/// The deletion banner's body carries a PATH, which is the one part of any of these sentences a
/// person chooses. A file called `<b>` would render as markup on Plasma (which advertises
/// `body-markup`) and can break the parse outright on a stricter one.
#[cfg(target_os = "linux")]
pub fn escape_markup(body: &str) -> String {
    body.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
                    let event = {
                        let guard = state.lock().await;
                        guard
                            .as_ref()
                            .and_then(|n| n.action_event(args.id, &args.action_key))
                    };
                    // Not ours: another application's banner, or one of ours the server has already
                    // forgotten. Silence is the only correct response.
                    let Some(event) = event else { continue };
                    let _ = app.emit("notification-action", event);
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
        // THE LOOP ONLY ENDS WHEN BOTH STREAMS DO, which is the connection going away — the server
        // restarting, or the bus dropping us. `send` connects only when the state is `None`, so
        // leaving it populated here would mean every later banner was sent over a dead connection
        // and no click ever came back. Clearing it makes the next send reconnect.
        eprintln!("notify: the notification server's signal stream ended; will reconnect on the next banner");
        let state = app.state::<NotifierState>();
        *state.lock().await = None;
    });
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{assume_markup, escape_markup, ActionEvent, NotifyPayload, Sent};

    /// The payload the webview sends, as JSON: `pair` is present at two folders or more and absent at one.
    fn payload(pair: Option<&str>) -> NotifyPayload {
        let mut value = serde_json::json!({
            "app": "Drive Sync",
            "kind": "conflict",
            "summary": "You both changed a.txt",
            "body": "Both versions are safe.",
            "icon": "proton-sync-attention-symbolic",
            "actions": [{ "id": "compare", "label": "Compare" }],
        });
        if let Some(pair) = pair {
            value["pair"] = pair.into();
            value["app"] = format!("Drive Sync · {pair}").into();
        }
        serde_json::from_value(value).expect("the webview's payload deserializes")
    }

    #[test]
    fn a_click_carries_the_folder_its_banner_was_sent_about() {
        // The server's `ActionInvoked` signal says an id and an action key and nothing else, so the folder
        // can only come back from what was kept beside the id when the banner went out.
        let sent = Sent::of(41, &payload(Some("photos")));
        assert_eq!(
            sent.action_event(41, "compare"),
            Some(ActionEvent {
                id: 41,
                kind: "conflict".into(),
                action: "compare".into(),
                pair: Some("photos".into()),
            })
        );
    }

    #[test]
    fn a_click_on_somebody_elses_banner_is_not_ours() {
        // `ActionInvoked` is broadcast to every listener on the bus. An id that is not the last one we
        // sent is another application's, or one the server forgot, and acts on nothing.
        let sent = Sent::of(41, &payload(Some("photos")));
        assert_eq!(sent.action_event(42, "keep"), None);
    }

    #[test]
    fn a_banner_sent_without_a_folder_reports_none_and_serializes_as_it_always_did() {
        // One folder: the payload never sent a `pair` (it deserializes without one), the click carries
        // none, and the event the webview receives is the three fields it always was.
        let sent = Sent::of(7, &payload(None));
        let event = sent.action_event(7, "keep").expect("ours");
        assert_eq!(event.pair, None);
        assert_eq!(
            serde_json::to_value(&event).expect("serializes"),
            serde_json::json!({ "id": 7, "kind": "conflict", "action": "keep" })
        );
        // …and with one, the name is on the wire under the key the webview reads.
        let named = Sent::of(8, &payload(Some("photos")));
        let wire =
            serde_json::to_value(named.action_event(8, "keep").expect("ours")).expect("serializes");
        assert_eq!(wire["pair"], "photos");
    }

    #[test]
    fn an_unreadable_capability_list_escapes() {
        // The failure this exists for: `GetCapabilities` can fail while `Notify` works, and the
        // body carries a path the user chose.
        assert!(assume_markup(None));
        assert!(assume_markup(Some(&["body".into(), "body-markup".into()])));
        // Only an explicit answer that lacks it turns escaping off.
        assert!(!assume_markup(Some(&["body".into(), "actions".into()])));
        assert!(!assume_markup(Some(&[])));
    }

    #[test]
    fn a_path_cannot_open_a_tag() {
        assert_eq!(
            escape_markup("photos/<b>2019</b> & more"),
            "photos/&lt;b&gt;2019&lt;/b&gt; &amp; more"
        );
        // The ampersand goes FIRST, or the entities it introduces would be escaped again.
        assert_eq!(escape_markup("a & <b"), "a &amp; &lt;b");
    }
}
