//! **Live gate for the shared-volume event scope (#456, ADR 0006).**
//!
//! A pair skips an event as "somewhere else on the volume" only when it knows which node its own
//! folder is (the *root uid*) and every folder of its tree has a composed id. Two facts about a real
//! account carry that, and neither can be shown by a fake:
//!
//! 1. the root uid can be learned: `proton-drive filesystem list --json` names the folder, either on
//!    its own wrapper node or as the entry for it in its parent's listing, and the id is the
//!    composed `volumeId~nodeId` of the volume the event stream is scoped to;
//! 2. an event for a node created directly in the folder names **that same uid** as its parent
//!    (`node_uid(volume, ParentLinkID) == root uid`), which is what makes a direct child placeable
//!    and every other parent foreign.
//!
//! **This is the gate before the skip is relied on**, as `events_identity_live.rs` was for
//! `events_driven`: if the first fails the pair never learns its root and nothing is skipped (the
//! pre-#456 behaviour, safely); if the second fails the skip would drop events inside the folder —
//! do not ship it.
//!
//! Two `#[ignore]` checks:
//!
//! 1. **Read-only** (default): learns the root uid through the client and checks its shape and its
//!    volume; names which source answered, and checks the other source agrees when both can.
//!
//! 2. **Write round-trip** (opt-in, `PROTON_SYNC_LIVE_WRITE=1`): uploads a probe at the root, polls
//!    the stream for its `Created` event, asserts the event's parent is the root uid, and reports
//!    whether the root itself received any event for the upload (it would make every upload a
//!    "the folder's own node changed" full walk). Deletes the probe.
//!
//! ```bash
//! PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
//! PROTON_SYNC_LIVE_REMOTE_ROOT=/Drive/RemoteFolder \
//!   cargo test --test events_scope_live -- --ignored --nocapture
//! # add PROTON_SYNC_LIVE_WRITE=1 to also run the write round-trip
//! # set PROTON_SYNC_LIVE_CLI=/path/to/proton-drive if not on PATH
//! ```
#![cfg(unix)]

use proton_drive_sync_engine::events::{
    EventsClient, RemoteChangeKind, node_uid, volume_id_from_proton_id,
};
use proton_drive_sync_engine::proton::{ProtonClient, ProtonDriveClient, RemoteEntity};
use proton_drive_sync_engine::session::{CliKeyringSession, CurlHttpTransport};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const APP_VERSION: &str = "cli-drive@0.5.0";

fn env_var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live event-scope gate"))
}

fn proton_client() -> ProtonDriveClient {
    let cli = std::env::var("PROTON_SYNC_LIVE_CLI").unwrap_or_else(|_| "proton-drive".to_owned());
    ProtonDriveClient::new(cli)
}

/// The id the root's entry carries in its parent's listing — the client's second source, computed
/// here on its own so the two can be compared.
fn root_uid_by_parent_listing(client: &ProtonDriveClient, remote_root: &Path) -> Option<String> {
    let parent = remote_root.parent()?;
    let name = remote_root.file_name()?;
    let siblings = client.list_directory(parent, Path::new("")).ok()?;
    match siblings.get(Path::new(name))? {
        RemoteEntity::Directory(directory) => directory.id.clone(),
        RemoteEntity::File(_) => None,
    }
}

#[test]
#[ignore = "requires a logged-in proton-drive CLI, PROTON_SYNC_EVENTS_VOLUME and PROTON_SYNC_LIVE_REMOTE_ROOT"]
fn live_the_remote_root_names_itself_with_a_composed_uid_of_the_events_volume() {
    let volume = env_var("PROTON_SYNC_EVENTS_VOLUME");
    let remote_root = PathBuf::from(env_var("PROTON_SYNC_LIVE_REMOTE_ROOT"));
    let client = proton_client();

    let uid = client
        .remote_root_uid(&remote_root)
        .expect("list the remote root")
        .expect(
            "neither the root's wrapper node nor its entry in its parent's listing carries a \
             composed uid — a pair would never learn its root and would skip nothing",
        );

    // No id is ever printed: an id is account data, and a failing live run says which check failed
    // without it. The checks compare; they do not echo.
    assert!(
        uid.contains('~'),
        "the root uid must be the composed volumeId~nodeId, and this one has no `~`"
    );
    assert!(
        volume_id_from_proton_id(&uid) == Some(volume.as_str()),
        "the root uid's volume half must be the events volume"
    );

    // Which source answered: the parent listing is the one that does not depend on what the CLI
    // prints for the wrapper. When it can answer, it must agree with the client.
    match root_uid_by_parent_listing(&client, &remote_root) {
        Some(by_parent) => {
            assert!(by_parent == uid, "the two sources must name one node");
            eprintln!("root uid OK (the parent listing names the same node)");
        }
        None => eprintln!(
            "root uid OK (from the root listing's wrapper; the parent listing cannot name the \
             root here)"
        ),
    }
}

#[test]
#[ignore = "opt-in write round-trip: also set PROTON_SYNC_LIVE_WRITE=1 (uploads then deletes a probe file)"]
fn live_a_node_created_in_the_root_names_the_root_uid_as_its_parent() {
    if std::env::var("PROTON_SYNC_LIVE_WRITE").ok().as_deref() != Some("1") {
        eprintln!("skipping write round-trip: set PROTON_SYNC_LIVE_WRITE=1 to enable");
        return;
    }
    let volume = env_var("PROTON_SYNC_EVENTS_VOLUME");
    let remote_root = PathBuf::from(env_var("PROTON_SYNC_LIVE_REMOTE_ROOT"));
    let client = proton_client();
    let root_uid = client
        .remote_root_uid(&remote_root)
        .expect("list the remote root")
        .expect("the root must be nameable (see the read-only gate)");

    let session = CliKeyringSession::from_cli_keyring().expect("read the reused CLI session");
    let events = EventsClient::new(CurlHttpTransport::new(), session, APP_VERSION);
    // Before the mutation, so the create event is guaranteed to be in the delta.
    let cursor0 = events
        .latest_cursor(&volume)
        .expect("latest cursor before the probe upload");

    let probe_rel = PathBuf::from("proton-sync-scope-probe.txt");
    let scratch = std::env::temp_dir().join("proton-sync-scope-probe.txt");
    std::fs::write(&scratch, b"scope probe").expect("write probe scratch file");
    client
        .upload(&scratch, &remote_root, &probe_rel)
        .expect("upload the probe file");
    let probe_uid = client
        .list_entities(&remote_root)
        .expect("list after upload")
        .get(&probe_rel)
        .and_then(|entity| entity.remote_id())
        .expect("the uploaded probe must appear in the listing with an id");

    // Poll until the probe's Created event arrives (events can lag a few seconds), keeping every
    // event seen on the way: the root's own node appearing among them matters.
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut parent_of_probe = None;
    let mut root_events = Vec::new();
    let mut cursor = cursor0;
    'poll: while Instant::now() < deadline {
        let page = events
            .events_since(&volume, &cursor)
            .expect("fetch the events delta");
        for change in &page.changes {
            let uid = node_uid(&volume, &change.node_id);
            if uid == root_uid {
                root_events.push(format!("{:?} trashed={}", change.kind, change.trashed));
            }
            if matches!(change.kind, RemoteChangeKind::Created) && uid == probe_uid {
                parent_of_probe = change.parent_id.clone();
                break 'poll;
            }
        }
        cursor = page.latest_event_id;
        if !page.more {
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    // Clean up the probe before asserting, so a failure never leaves the account dirty.
    let _ = client.delete(&remote_root.join(&probe_rel));
    let _ = std::fs::remove_file(&scratch);

    let parent = parent_of_probe.expect(
        "no Created event for the probe arrived: the stream does not report a node created in the \
         folder, and the skip cannot be relied on",
    );
    assert!(
        node_uid(&volume, &parent) == root_uid,
        "a node created directly in the folder must name the folder's own uid as its parent — \
         otherwise a direct child would read as foreign and be skipped"
    );
    if root_events.is_empty() {
        eprintln!("scope round-trip OK: the parent is the root uid; the root itself got no event");
    } else {
        eprintln!(
            "scope round-trip OK: the parent is the root uid, BUT the root itself received events \
             for the upload: {root_events:?}. Every upload would then be read as \"the folder's \
             own node changed\" and walk: the root-event rule needs narrowing before this ships"
        );
    }
}
