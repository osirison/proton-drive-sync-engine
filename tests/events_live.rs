//! Live end-to-end check for the detection path against the real Proton Drive volume events
//! API, driving the **library** types end to end: [`EventsClient`] over the real
//! [`CurlHttpTransport`] authenticated by [`CliKeyringSession`] (the current CLI-session-reuse
//! provider).
//!
//! `#[ignore]` and read-only. Requires the `proton-drive` CLI logged in, the desktop keyring
//! unlocked, `DBUS_SESSION_BUS_ADDRESS` set, and the volume id (a key under `drive` in
//! `~/.local/share/proton-drive-cli/events.json`):
//!
//! ```bash
//! PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
//!   cargo test --test events_live -- --ignored --nocapture
//! ```
#![cfg(unix)]

use proton_drive_sync_engine::events::EventsClient;
use proton_drive_sync_engine::session::{CliKeyringSession, CurlHttpTransport};

#[test]
#[ignore = "requires a logged-in proton-drive CLI, an unlocked keyring, and PROTON_SYNC_EVENTS_VOLUME"]
fn live_detection_core_reads_the_volume_event_stream() {
    let volume = std::env::var("PROTON_SYNC_EVENTS_VOLUME")
        .expect("set PROTON_SYNC_EVENTS_VOLUME to the drive volume id");
    let session = CliKeyringSession::from_cli_keyring().expect("read the reused CLI session");
    let client = EventsClient::new(CurlHttpTransport::new(), session, "cli-drive@0.5.0");

    // Bootstrap: fetch the current cursor (proves auth + transport + parse end to end).
    let cursor = client
        .latest_cursor(&volume)
        .expect("latest_cursor should authenticate and return a cursor");
    assert!(!cursor.is_empty(), "cursor must be non-empty");

    // Fetch the delta since that cursor. A just-fetched cursor usually yields zero changes, but
    // the response must still parse into a valid page carrying the next cursor.
    let page = client
        .events_since(&volume, &cursor)
        .expect("events_since should return a parseable page");
    assert!(
        !page.latest_event_id.is_empty(),
        "the page must carry the next cursor"
    );

    eprintln!(
        "live events OK: changes={} more={} refresh={} next_cursor={}",
        page.changes.len(),
        page.more,
        page.refresh,
        page.latest_event_id
    );
}
