//! **Spike (0) — id-identity HARD GATE for event-driven reconcile.**
//!
//! Event-driven steady state hinges on one unproven identity: a `proton_id` the engine stores from
//! a `filesystem list` (the composed `volumeId~nodeId`) must equal
//! [`events::node_uid`]`(volume, LinkID)` for the *same* node, where `LinkID` is what that node's
//! volume event carries. If it does not, [`index::path_for_proton_id`] silently degrades to
//! always-fallback and the O(changes) path never engages. This test proves the identity against a
//! real account. **If it fails, the stream-only design is invalid and must be rethought** — do not
//! flip `events_driven` on.
//!
//! Two `#[ignore]` checks:
//!
//! 1. **Read-only identity** (default): lists a real remote folder and verifies, for a real node,
//!    that the stored id is the *composed* `volumeId~nodeId` (not a raw `LinkID` — `proton.rs`
//!    prefers `node.id`), that its volume half matches the configured volume, and that
//!    [`node_uid`] round-trips it. This is the exact bridge the resolver relies on.
//!
//! 2. **Write round-trip** (opt-in, `PROTON_SYNC_LIVE_WRITE=1`): uploads a probe file, polls the
//!    volume event stream for its `Created` event, and asserts `node_uid(volume, event.LinkID)`
//!    equals the probe's listed composed id — the full create → event → resolve round trip. It
//!    deletes the probe afterward.
//!
//! ```bash
//! PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
//! PROTON_SYNC_LIVE_REMOTE_ROOT=/Drive/RemoteFolder \
//!   cargo test --test events_identity_live -- --ignored --nocapture
//! # add PROTON_SYNC_LIVE_WRITE=1 to also run the write round-trip
//! # set PROTON_SYNC_LIVE_CLI=/path/to/proton-drive if not on PATH
//! ```
#![cfg(unix)]

use proton_drive_sync_engine::events::{
    EventsClient, RemoteChangeKind, node_uid, volume_id_from_proton_id,
};
use proton_drive_sync_engine::proton::{ProtonClient, ProtonDriveClient};
use proton_drive_sync_engine::session::{CliKeyringSession, CurlHttpTransport};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const APP_VERSION: &str = "cli-drive@0.5.0";

fn env_var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live id-identity gate"))
}

fn proton_client() -> ProtonDriveClient {
    let cli = std::env::var("PROTON_SYNC_LIVE_CLI").unwrap_or_else(|_| "proton-drive".to_owned());
    ProtonDriveClient::new(cli)
}

#[test]
#[ignore = "requires a logged-in proton-drive CLI, PROTON_SYNC_EVENTS_VOLUME and PROTON_SYNC_LIVE_REMOTE_ROOT"]
fn live_stored_id_is_the_composed_uid_the_bridge_expects() {
    let volume = env_var("PROTON_SYNC_EVENTS_VOLUME");
    let remote_root = PathBuf::from(env_var("PROTON_SYNC_LIVE_REMOTE_ROOT"));
    let client = proton_client();

    let entities = client
        .list_entities(&remote_root)
        .expect("list the remote root");
    assert!(
        !entities.is_empty(),
        "the remote root must contain at least one node to check identity against"
    );

    // Pick any node that carries an id.
    let (path, id) = entities
        .iter()
        .find_map(|(path, entity)| entity.remote_id().map(|id| (path.clone(), id)))
        .expect("at least one remote node exposes an id");

    // The stored id must be the COMPOSED volumeId~nodeId, not a raw LinkID. `proton.rs` prefers
    // `node.id` over `node.uid`, so this guards the #21-review landmine directly.
    assert!(
        id.contains('~'),
        "stored proton_id must be the composed volumeId~nodeId (got a raw id: {id:?}) — \
         the node_uid bridge would always-fallback otherwise"
    );

    // Volume derivation: the id's volume half must equal the configured volume.
    assert_eq!(
        volume_id_from_proton_id(&id),
        Some(volume.as_str()),
        "the listed id's volume half must equal the configured volume"
    );

    // Round-trip: recomposing volume + raw node id via node_uid must reproduce the listed id — the
    // exact operation the resolver performs to bridge an event's raw LinkID.
    let (listed_volume, raw_node_id) = id.split_once('~').expect("composed id splits on '~'");
    assert_eq!(
        node_uid(listed_volume, raw_node_id),
        id,
        "node_uid(volume, rawNodeId) must reproduce the listed composed id"
    );

    eprintln!("id-identity (read-only) OK: {} -> {id}", path.display());
}

#[test]
#[ignore = "opt-in write round-trip: also set PROTON_SYNC_LIVE_WRITE=1 (uploads then deletes a probe file)"]
fn live_created_event_link_id_bridges_to_the_listed_node_id() {
    if std::env::var("PROTON_SYNC_LIVE_WRITE").ok().as_deref() != Some("1") {
        eprintln!("skipping write round-trip: set PROTON_SYNC_LIVE_WRITE=1 to enable");
        return;
    }
    let volume = env_var("PROTON_SYNC_EVENTS_VOLUME");
    let remote_root = PathBuf::from(env_var("PROTON_SYNC_LIVE_REMOTE_ROOT"));
    let client = proton_client();

    let session = CliKeyringSession::from_cli_keyring().expect("read the reused CLI session");
    let events = EventsClient::new(CurlHttpTransport::new(), session, APP_VERSION);

    // Capture the cursor BEFORE the mutation so the create event is guaranteed in the delta.
    let cursor0 = events
        .latest_cursor(&volume)
        .expect("latest cursor before the probe upload");

    // Upload a uniquely-named probe file. A fixed name keeps the test idempotent-ish; it is
    // deleted at the end regardless of assertions.
    let probe_rel = PathBuf::from("proton-sync-identity-probe.txt");
    let scratch = std::env::temp_dir().join("proton-sync-identity-probe.txt");
    std::fs::write(&scratch, b"identity probe").expect("write probe scratch file");
    client
        .upload(&scratch, &remote_root, &probe_rel)
        .expect("upload the probe file");

    // Find the probe's composed id from a fresh listing.
    let listed_id = {
        let entities = client
            .list_entities(&remote_root)
            .expect("list after upload");
        entities
            .get(&probe_rel)
            .and_then(|entity| entity.remote_id())
            .expect("the uploaded probe must appear in the listing with an id")
    };

    // Poll the event stream until the probe's Created event arrives (events can lag a few seconds).
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut bridged = None;
    let mut cursor = cursor0;
    'poll: while Instant::now() < deadline {
        let page = events
            .events_since(&volume, &cursor)
            .expect("fetch the events delta");
        for change in &page.changes {
            if matches!(change.kind, RemoteChangeKind::Created)
                && node_uid(&volume, &change.node_id) == listed_id
            {
                bridged = Some(change.node_id.clone());
                break 'poll;
            }
        }
        cursor = page.latest_event_id;
        if !page.more {
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    // Clean up the probe before asserting, so a failure never leaves the account dirty.
    let remote_path = remote_root.join(&probe_rel);
    let _ = client.delete(&remote_path);
    let _ = std::fs::remove_file(&scratch);

    let raw_link_id = bridged.expect(
        "no Created event bridged to the probe's listed id — the stream-only design's core \
         identity does NOT hold; STOP and rethink before enabling events_driven",
    );
    eprintln!(
        "id-identity (write round-trip) OK: event LinkID {raw_link_id} -> node_uid == listed id {listed_id}"
    );
}
