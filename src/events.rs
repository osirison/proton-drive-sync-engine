//! Remote change **detection** via Proton Drive's volume event stream.
//!
//! Instead of re-walking the whole remote tree every scan (O(folders) `filesystem list`
//! calls, see [`crate::proton`]), the engine can ask Proton "what changed since cursor X?"
//! and get back an O(changes) delta. This module is that detection path.
//!
//! Two facts (verified live, 2026-07) shape the design:
//!
//! * **Detection needs only a session, no decryption.** The volume events endpoint returns
//!   the change-relevant fields — event type, node id, parent id, trashed/shared flags — in
//!   server cleartext (the node *name* is not in the event at all). So this is plain
//!   authenticated HTTP + JSON parsing; no PGP, no unlocked keys. Name resolution for a
//!   newly-created node happens elsewhere, against the index or a targeted lookup.
//! * **The auth surface is injectable.** Obtaining/refreshing a Proton session is a separate
//!   concern with its own open decision (an independent *forked* session is the intended
//!   provider). This module is therefore generic over a [`SessionProvider`] and an
//!   [`HttpTransport`]: the fetch + normalization logic here is fully testable without a
//!   network or any particular session mechanism, and the concrete transport/session drop in
//!   behind these traits.
//!
//! See `docs/adr/0001-remote-change-detection-via-volume-events.md`.

use crate::{AppResult, boxed_error};
use serde::Deserialize;

/// Default Proton Drive API origin. Requests are `GET`s against this host.
pub const DRIVE_API_BASE: &str = "https://drive-api.proton.me";

/// Proton API envelopes carry a `Code`; `1000` means success.
const PROTON_API_SUCCESS_CODE: i64 = 1000;

/// Raw `Link.EventType` values from the volume events endpoint. `2` and `3` are both
/// "updated" (3 covers rename/move and trashed-state changes); unknown values are ignored
/// for forward compatibility.
const EVENT_TYPE_DELETED: u8 = 0;
const EVENT_TYPE_CREATED: u8 = 1;
const EVENT_TYPE_UPDATED_A: u8 = 2;
const EVENT_TYPE_UPDATED_B: u8 = 3;

/// The kind of change an event reports, normalized from the raw numeric `EventType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteChangeKind {
    /// A node was created (`EventType` 1).
    Created,
    /// A node's metadata changed — content, rename/move, or trashed state (`EventType` 2/3).
    /// Inspect [`RemoteChange::trashed`] to tell a trashing apart from an ordinary update.
    Updated,
    /// A node was permanently removed (`EventType` 0). Note that *trashing* a node arrives as
    /// [`RemoteChangeKind::Updated`] with `trashed == true`, not as `Deleted`.
    Deleted,
}

/// One normalized remote change. Keyed by opaque node ids (`node_id` / `parent_id`), never by
/// path: the planner resolves ids to paths via the index. `parent_id` is absent when the API
/// omits it (e.g. volume-root-level nodes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteChange {
    pub kind: RemoteChangeKind,
    pub node_id: String,
    pub parent_id: Option<String>,
    /// The node's trashed state after this event. A `true` here on an [`RemoteChangeKind::Updated`]
    /// is a *trashing* and should be treated as a remote removal by the planner.
    pub trashed: bool,
    pub shared: bool,
    /// Per-event id (distinct from the page cursor); useful for logging/dedup.
    pub event_id: String,
}

/// One page of the volume event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeEventPage {
    /// The cursor to persist and pass on the next call. Advances even when `changes` is empty.
    pub latest_event_id: String,
    /// `true` when more pages are immediately available (paginate again from `latest_event_id`).
    pub more: bool,
    /// `true` when the server asks the client to discard its cursor and perform a full
    /// reconvergence scan (the events analogue of a "tree refresh"). Callers must honor this.
    pub refresh: bool,
    /// The normalized changes in this page. Unknown/unhandled event types are dropped, so a
    /// non-empty page can still yield zero changes.
    pub changes: Vec<RemoteChange>,
}

/// A minimal HTTP response: the pieces the events logic needs.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Performs authenticated-agnostic HTTP `GET`s. Injected so the events logic is testable
/// without a network and the concrete HTTP client stays a swappable concern (the library
/// intentionally ships no networking dependency).
pub trait HttpTransport: Send + Sync {
    fn get(&self, url: &str, headers: &[(String, String)]) -> AppResult<HttpResponse>;
}

/// Supplies Proton session authentication headers and can refresh them on expiry.
///
/// The intended production implementation owns an independent (forked) session and refreshes
/// it via Proton's auth API; a temporary session-reuse implementation is used only as a test
/// harness. Either way the events logic below only depends on this trait.
pub trait SessionProvider: Send + Sync {
    /// Headers that authenticate a request, e.g. `x-pm-uid` and `Authorization: Bearer …`.
    fn auth_headers(&self) -> AppResult<Vec<(String, String)>>;
    /// Refresh the session after the API returns `401`. Returns `Ok` if new credentials are
    /// available (the caller then retries once).
    fn refresh(&self) -> AppResult<()>;
}

/// Fetches volume event deltas over an [`HttpTransport`], authenticating via a
/// [`SessionProvider`] and transparently refreshing once on a `401`.
pub struct EventsClient<T: HttpTransport, S: SessionProvider> {
    transport: T,
    session: S,
    api_base: String,
    app_version: String,
}

impl<T: HttpTransport, S: SessionProvider> EventsClient<T, S> {
    /// Builds a client against the production Drive API. `app_version` is sent as
    /// `x-pm-appversion` (Proton validates it).
    pub fn new(transport: T, session: S, app_version: impl Into<String>) -> Self {
        Self {
            transport,
            session,
            api_base: DRIVE_API_BASE.to_owned(),
            app_version: app_version.into(),
        }
    }

    /// Overrides the API origin (for tests / alternate environments).
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Returns the current latest event id for `volume_id` — the cursor to start from when the
    /// engine has no stored cursor yet.
    pub fn latest_cursor(&self, volume_id: &str) -> AppResult<String> {
        let url = format!(
            "{}/drive/volumes/{}/events/latest",
            self.api_base, volume_id
        );
        let body = self.authorized_get(&url)?;
        parse_latest_event_id(&body)
    }

    /// Fetches the delta for `volume_id` since `cursor`, normalized into a [`VolumeEventPage`].
    pub fn events_since(&self, volume_id: &str, cursor: &str) -> AppResult<VolumeEventPage> {
        // `volume_id` and `cursor` are Proton base64url ids (no `/`); inserted literally to
        // match what the official client sends, which the server expects verbatim.
        let url = format!(
            "{}/drive/v2/volumes/{}/events/{}",
            self.api_base, volume_id, cursor
        );
        let body = self.authorized_get(&url)?;
        parse_volume_events(&body)
    }

    /// Issues the request, refreshing the session and retrying **once** on a `401`.
    fn authorized_get(&self, url: &str) -> AppResult<String> {
        let response = self.send(url)?;
        let response = if response.status == 401 {
            self.session.refresh()?;
            self.send(url)?
        } else {
            response
        };
        if response.status == 401 {
            return Err(boxed_error(
                "proton events request unauthorized even after refreshing the session",
            ));
        }
        if response.status != 200 {
            return Err(boxed_error(format!(
                "proton events request to {url} failed: HTTP {} {}",
                response.status,
                truncate_for_error(&response.body)
            )));
        }
        Ok(response.body)
    }

    fn send(&self, url: &str) -> AppResult<HttpResponse> {
        let mut headers = self.session.auth_headers()?;
        headers.push(("x-pm-appversion".to_owned(), self.app_version.clone()));
        headers.push(("Accept".to_owned(), "application/json".to_owned()));
        self.transport.get(url, &headers)
    }
}

/// Parses a `.../events/latest` response into its cursor.
pub fn parse_latest_event_id(json: &str) -> AppResult<String> {
    let raw: RawLatest = serde_json::from_str(json)?;
    ensure_success(raw.code)?;
    Ok(raw.event_id)
}

/// Parses a `.../v2/volumes/{id}/events/{cursor}` response into a normalized [`VolumeEventPage`].
pub fn parse_volume_events(json: &str) -> AppResult<VolumeEventPage> {
    let raw: RawEventsResponse = serde_json::from_str(json)?;
    ensure_success(raw.code)?;
    let changes = raw
        .events
        .into_iter()
        .filter_map(|event| {
            // Skip — never fail the whole page — any event we can't act on: an unknown/absent
            // event type, or (defensively, since the wire shape of a hard delete is
            // unconfirmed) an event without a usable node id. The strict fields inside `Link`
            // are optional so a sparse event can't break deserialization of its neighbours.
            let kind = change_kind(event.event_type?)?;
            let link = event.link?;
            let node_id = link.link_id?;
            Some(RemoteChange {
                kind,
                node_id,
                parent_id: link.parent_link_id,
                trashed: link.is_trashed,
                shared: link.is_shared,
                event_id: event.event_id,
            })
        })
        .collect();
    Ok(VolumeEventPage {
        latest_event_id: raw.event_id,
        more: raw.more,
        refresh: raw.refresh,
        changes,
    })
}

fn change_kind(event_type: u8) -> Option<RemoteChangeKind> {
    match event_type {
        EVENT_TYPE_DELETED => Some(RemoteChangeKind::Deleted),
        EVENT_TYPE_CREATED => Some(RemoteChangeKind::Created),
        EVENT_TYPE_UPDATED_A | EVENT_TYPE_UPDATED_B => Some(RemoteChangeKind::Updated),
        // Unknown/unsupported event types are ignored so a newly-introduced type never
        // crashes a scan; it just won't be acted on until this map is extended.
        _ => None,
    }
}

fn ensure_success(code: i64) -> AppResult<()> {
    if code == PROTON_API_SUCCESS_CODE {
        Ok(())
    } else {
        Err(boxed_error(format!(
            "proton events API returned non-success code {code}"
        )))
    }
}

fn truncate_for_error(body: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut chars = body.chars();
    let truncated: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

// --- raw wire types (Proton uses PascalCase; only the *ID fields need explicit renames) ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawEventsResponse {
    code: i64,
    #[serde(rename = "EventID")]
    event_id: String,
    more: bool,
    refresh: bool,
    #[serde(default)]
    events: Vec<RawEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawEvent {
    #[serde(rename = "EventID", default)]
    event_id: String,
    // Optional/defaulted throughout so a single event with an unexpected or sparse shape is
    // dropped by the parse step above rather than throwing and taking the whole page with it.
    #[serde(default)]
    event_type: Option<u8>,
    #[serde(default)]
    link: Option<RawLink>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawLink {
    #[serde(rename = "LinkID", default)]
    link_id: Option<String>,
    #[serde(rename = "ParentLinkID", default)]
    parent_link_id: Option<String>,
    #[serde(default)]
    is_shared: bool,
    #[serde(default)]
    is_trashed: bool,
}

#[derive(Debug, Deserialize)]
struct RawLatest {
    #[serde(rename = "EventID")]
    event_id: String,
    #[serde(rename = "Code", default = "default_success_code")]
    code: i64,
}

fn default_success_code() -> i64 {
    PROTON_API_SUCCESS_CODE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const CREATE: &str = include_str!("../tests/fixtures/volume-events-create.json");
    const TRASH: &str = include_str!("../tests/fixtures/volume-events-trash.json");
    const MIXED: &str = include_str!("../tests/fixtures/volume-events-mixed.json");
    const REFRESH: &str = include_str!("../tests/fixtures/volume-events-refresh.json");
    const LATEST: &str = include_str!("../tests/fixtures/volume-events-latest.json");

    #[test]
    fn parses_a_create_event() {
        let page = parse_volume_events(CREATE).expect("parse create");
        assert_eq!(page.latest_event_id, "cursor-after-create");
        assert!(!page.more && !page.refresh);
        assert_eq!(page.changes.len(), 1);
        let change = &page.changes[0];
        assert_eq!(change.kind, RemoteChangeKind::Created);
        assert_eq!(change.node_id, "node-created");
        assert_eq!(change.parent_id.as_deref(), Some("node-root"));
        assert!(!change.trashed);
    }

    #[test]
    fn trashing_arrives_as_updated_with_trashed_true() {
        // Regression guard for the key subtlety: a trash is EventType 3 (Updated) with
        // IsTrashed=true, not a Deleted (EventType 0). The planner keys removal off `trashed`.
        let page = parse_volume_events(TRASH).expect("parse trash");
        let change = &page.changes[0];
        assert_eq!(change.kind, RemoteChangeKind::Updated);
        assert!(change.trashed, "a trashed node must set trashed=true");
    }

    #[test]
    fn parses_mixed_page_with_delete_update_and_missing_parent() {
        let page = parse_volume_events(MIXED).expect("parse mixed");
        assert!(page.more, "mixed fixture advertises another page");
        let kinds: Vec<_> = page.changes.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![RemoteChangeKind::Deleted, RemoteChangeKind::Updated]
        );
        // An unknown event type in the fixture must be dropped, not error the whole page.
        assert_eq!(page.changes.len(), 2, "unknown event type must be skipped");
        // The update event has a null ParentLinkID → parent_id is None.
        assert_eq!(page.changes[1].parent_id, None);
        assert!(page.changes[1].shared);
    }

    #[test]
    fn tolerates_a_sparse_delete_link_and_still_reports_the_deletion() {
        // The wire shape of a hard delete (EventType 0) is unconfirmed; its `Link` may carry
        // only `LinkID` (no IsShared/IsTrashed/ParentLinkID). It must parse — not throw — and
        // still surface the deletion with sensible defaults.
        let body = concat!(
            r#"{"Code":1000,"EventID":"c","More":false,"Refresh":false,"#,
            r#""Events":[{"EventID":"e","EventType":0,"Link":{"LinkID":"gone"}}]}"#
        );
        let page = parse_volume_events(body).expect("a sparse delete link must still parse");
        assert_eq!(page.changes.len(), 1);
        assert_eq!(page.changes[0].kind, RemoteChangeKind::Deleted);
        assert_eq!(page.changes[0].node_id, "gone");
        assert_eq!(page.changes[0].parent_id, None);
        assert!(!page.changes[0].trashed && !page.changes[0].shared);
    }

    #[test]
    fn one_malformed_event_is_dropped_without_failing_the_page() {
        // An event missing its Link, and one missing EventType, must both be skipped while the
        // surrounding valid event still comes through — a single bad event never breaks a scan.
        let body = concat!(
            r#"{"Code":1000,"EventID":"c","More":false,"Refresh":false,"Events":["#,
            r#"{"EventID":"a","EventType":1,"Link":{"LinkID":"n1","ParentLinkID":"p","IsShared":false,"IsTrashed":false}},"#,
            r#"{"EventID":"b","EventType":1},"#,
            r#"{"EventID":"d","Link":{"LinkID":"n2"}}"#,
            r#"]}"#
        );
        let page = parse_volume_events(body).expect("malformed events must not fail the page");
        assert_eq!(page.changes.len(), 1, "only the fully-valid event survives");
        assert_eq!(page.changes[0].node_id, "n1");
    }

    #[test]
    fn surfaces_the_refresh_signal() {
        let page = parse_volume_events(REFRESH).expect("parse refresh");
        assert!(
            page.refresh,
            "refresh must be surfaced so callers can full-scan"
        );
        assert!(page.changes.is_empty());
    }

    #[test]
    fn latest_cursor_response_parses_to_the_event_id() {
        assert_eq!(
            parse_latest_event_id(LATEST).expect("parse latest"),
            "the-current-latest"
        );
    }

    #[test]
    fn rejects_a_non_success_api_code() {
        let body = r#"{"Code":2000,"EventID":"x","More":false,"Refresh":false,"Events":[]}"#;
        let error = parse_volume_events(body).expect_err("non-1000 code must error");
        assert!(error.to_string().contains("non-success code 2000"));
    }

    // --- EventsClient logic (401 → refresh → retry, header injection) with fakes ---

    struct FakeSession {
        refreshes: Mutex<usize>,
    }
    impl SessionProvider for FakeSession {
        fn auth_headers(&self) -> AppResult<Vec<(String, String)>> {
            Ok(vec![
                ("x-pm-uid".to_owned(), "uid-abc".to_owned()),
                ("Authorization".to_owned(), "Bearer tok".to_owned()),
            ])
        }
        fn refresh(&self) -> AppResult<()> {
            *self.refreshes.lock().unwrap() += 1;
            Ok(())
        }
    }

    /// Transport that replays a queued sequence of responses and records the headers it saw.
    struct ScriptedTransport {
        responses: Mutex<Vec<HttpResponse>>,
        seen_headers: Mutex<Vec<Vec<(String, String)>>>,
    }
    impl ScriptedTransport {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                seen_headers: Mutex::new(Vec::new()),
            }
        }
    }
    impl HttpTransport for ScriptedTransport {
        fn get(&self, _url: &str, headers: &[(String, String)]) -> AppResult<HttpResponse> {
            self.seen_headers.lock().unwrap().push(headers.to_vec());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(boxed_error("scripted transport exhausted"));
            }
            Ok(responses.remove(0))
        }
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_owned(),
        }
    }
    fn unauthorized() -> HttpResponse {
        HttpResponse {
            status: 401,
            body: r#"{"Code":401,"Error":"Invalid access token"}"#.to_owned(),
        }
    }

    fn client(responses: Vec<HttpResponse>) -> EventsClient<ScriptedTransport, FakeSession> {
        EventsClient::new(
            ScriptedTransport::new(responses),
            FakeSession {
                refreshes: Mutex::new(0),
            },
            "test-app@0.0.0",
        )
        .with_api_base("https://example.test")
    }

    #[test]
    fn events_since_returns_the_parsed_page_and_sends_auth_plus_appversion() {
        let events = client(vec![ok(CREATE)]);
        let page = events.events_since("VOL", "CUR").expect("events");
        assert_eq!(page.changes.len(), 1);
        // The one request carried the session headers plus the app version and Accept.
        let seen = events.transport.seen_headers.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let headers = &seen[0];
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "x-pm-uid" && v == "uid-abc")
        );
        assert!(headers.iter().any(|(k, _)| k == "Authorization"));
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "x-pm-appversion" && v == "test-app@0.0.0")
        );
    }

    #[test]
    fn refreshes_once_and_retries_after_a_401() {
        let events = client(vec![unauthorized(), ok(CREATE)]);
        let page = events
            .events_since("VOL", "CUR")
            .expect("events after refresh");
        assert_eq!(page.changes.len(), 1);
        assert_eq!(
            *events.session.refreshes.lock().unwrap(),
            1,
            "exactly one refresh should have happened"
        );
    }

    #[test]
    fn errors_when_still_unauthorized_after_a_refresh() {
        let events = client(vec![unauthorized(), unauthorized()]);
        let error = events
            .events_since("VOL", "CUR")
            .expect_err("should give up");
        assert!(
            error
                .to_string()
                .contains("unauthorized even after refreshing")
        );
        assert_eq!(*events.session.refreshes.lock().unwrap(), 1);
    }

    #[test]
    fn errors_on_a_non_200_non_401_status() {
        let events = client(vec![HttpResponse {
            status: 503,
            body: "upstream down".to_owned(),
        }]);
        let error = events.latest_cursor("VOL").expect_err("5xx must error");
        assert!(error.to_string().contains("HTTP 503"));
    }

    #[test]
    fn latest_cursor_hits_the_latest_endpoint_and_parses_it() {
        let events = client(vec![ok(LATEST)]);
        assert_eq!(
            events.latest_cursor("VOL").expect("cursor"),
            "the-current-latest"
        );
    }
}
