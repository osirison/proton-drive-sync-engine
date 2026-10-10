//! **Live gate for the shared-volume event scope (#456, ADR 0006).**
//!
//! A pair skips an event as "somewhere else on the volume" only when it knows which node its own
//! folder is (the *root uid*) and every folder of its tree has a composed id. Two facts about a real
//! account carry that, and neither can be shown by a fake:
//!
//! 1. the root uid can be learned: `proton-drive filesystem list --json` names the folder, either on
//!    its own wrapper node or as the entry for it in its parent's listing, and the id is the
//!    composed `volumeId~nodeId` of the volume the event stream is scoped to;
//! 2. an event for a node that is directly in the folder names **that same uid** as its parent
//!    (`node_uid(volume, ParentLinkID) == root uid`), whether the event is the node's creation or a
//!    later change to it (a rename is the one a delta carries most). That is what makes a direct
//!    child placeable and every other parent foreign. An event that names no parent, or another
//!    one, would be read as news about somebody else's node.
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
//! 2. **Write round-trip** (opt-in, `PROTON_SYNC_LIVE_WRITE=1`): uploads a small probe file at the
//!    root and polls the stream for its `Created` event; renames the probe within the root and
//!    polls for the `Updated` event that follows; asserts that **both** name the root uid as their
//!    parent; and reports whether the root itself received any event for either (that would make
//!    every upload or rename a "the folder's own node changed" full walk). The probe is trashed at
//!    the end, **and also when an assertion fails** (a drop guard, best effort), so a failed run
//!    does not leave the account dirty. The client's only delete is a trash, so the probe ends in
//!    the account's trash; nothing is restored from it and it is not emptied.
//!
//! Nothing here prints an id (an id is account data): every assertion message is a constant and the
//! checks compare ids without echoing them. The probe is named `proton-sync-scope-probe-<pid>.txt`
//! (renamed to `...-renamed.txt`), and the test refuses to start if that name already exists.
//!
//! Run it on a real account, with the desktop keyring unlocked (the events session is the logged-in
//! CLI's, read from the OS keyring) and `DBUS_SESSION_BUS_ADDRESS` set:
//!
//! ```bash
//! PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
//! PROTON_SYNC_LIVE_REMOTE_ROOT=/my-files/Videos \
//! PROTON_SYNC_LIVE_WRITE=1 \
//!   cargo test --test events_scope_live -- --ignored --nocapture
//! ```
//!
//! | variable | for | meaning |
//! | --- | --- | --- |
//! | `PROTON_SYNC_EVENTS_VOLUME` | both checks | the volume id: a key under `"drive"` in `~/.local/share/proton-drive-cli/events.json` |
//! | `PROTON_SYNC_LIVE_REMOTE_ROOT` | both checks | the folder to test in, as the pair's `remote_root` spells it (here `/my-files/Videos`) |
//! | `PROTON_SYNC_LIVE_WRITE` | write check | `1` to run it; without it the check prints a line and passes |
//! | `PROTON_SYNC_LIVE_CLI` | optional | path of `proton-drive` when it is not on `PATH` |
//!
//! Leave `PROTON_SYNC_LIVE_WRITE` out for the read-only check alone.
#![cfg(unix)]

use proton_drive_sync_engine::events::{
    EventsClient, HttpTransport, RemoteChange, RemoteChangeKind, SessionProvider, node_uid,
    volume_id_from_proton_id,
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

/// The probe on the remote. Dropping it trashes every name it has had, best effort, so a failed
/// assertion never leaves the account dirty. Errors are ignored on purpose: a name that no longer
/// exists is the normal case for all but one of them.
struct Probe<'a> {
    client: &'a ProtonDriveClient,
    remote_root: PathBuf,
    /// Every name the probe has had or may have, relative to the root.
    names: Vec<PathBuf>,
    scratch: PathBuf,
}

impl Probe<'_> {
    fn trash_every_name(&mut self) {
        for name in self.names.drain(..) {
            let _ = self.client.delete(&self.remote_root.join(name));
        }
    }
}

impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.trash_every_name();
        let _ = std::fs::remove_file(&self.scratch);
    }
}

/// Polls the stream from `cursor` until `wanted` accepts an event, for up to 45 s (events lag a few
/// seconds). Every event naming the root's own node is noted in `root_events` on the way, kind
/// only: the root appearing among them matters.
fn await_event<T: HttpTransport, S: SessionProvider>(
    events: &EventsClient<T, S>,
    volume: &str,
    root_uid: &str,
    cursor: &str,
    root_events: &mut Vec<String>,
    wanted: impl Fn(&RemoteChange) -> bool,
) -> Option<RemoteChange> {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut cursor = cursor.to_owned();
    while Instant::now() < deadline {
        let page = events
            .events_since(volume, &cursor)
            .expect("fetch the events delta");
        for change in &page.changes {
            if node_uid(volume, &change.node_id) == root_uid {
                root_events.push(format!("{:?} trashed={}", change.kind, change.trashed));
            }
            if wanted(change) {
                return Some(change.clone());
            }
        }
        cursor = page.latest_event_id;
        if !page.more {
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    None
}

#[test]
#[ignore = "opt-in write round-trip: also set PROTON_SYNC_LIVE_WRITE=1 (uploads, renames then trashes a probe file)"]
fn live_events_for_a_node_in_the_root_name_the_root_uid_as_their_parent() {
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

    let stem = format!("proton-sync-scope-probe-{}", std::process::id());
    let probe_rel = PathBuf::from(format!("{stem}.txt"));
    let renamed_rel = PathBuf::from(format!("{stem}-renamed.txt"));
    let in_root = client
        .list_directory(&remote_root, Path::new(""))
        .expect("list the remote root before the probe");
    assert!(
        !in_root.contains_key(&probe_rel) && !in_root.contains_key(&renamed_rel),
        "a file with the probe's name is already in the folder; refusing to touch it"
    );

    // Before the mutation, so the create event is guaranteed to be in the delta.
    let cursor0 = events
        .latest_cursor(&volume)
        .expect("latest cursor before the probe upload");

    let scratch = std::env::temp_dir().join(format!("{stem}.txt"));
    std::fs::write(&scratch, b"scope probe").expect("write probe scratch file");
    let mut probe = Probe {
        client: &client,
        remote_root: remote_root.clone(),
        // Set before the call that makes it: a call that fails halfway may still have made it.
        names: vec![probe_rel.clone()],
        scratch,
    };
    client
        .upload(&probe.scratch, &remote_root, &probe_rel)
        .expect("upload the probe file");
    let probe_uid = client
        .list_directory(&remote_root, Path::new(""))
        .expect("list the remote root after the upload")
        .get(&probe_rel)
        .and_then(RemoteEntity::remote_id)
        .expect("the uploaded probe must appear in the listing with an id");

    let mut root_events = Vec::new();

    // 1. Its creation.
    let created = await_event(
        &events,
        &volume,
        &root_uid,
        &cursor0,
        &mut root_events,
        |change| {
            matches!(change.kind, RemoteChangeKind::Created)
                && node_uid(&volume, &change.node_id) == probe_uid
        },
    )
    .expect(
        "no Created event for the probe arrived: the stream does not report a node created in the \
         folder, and the skip cannot be relied on",
    );
    assert!(
        created
            .parent_id
            .as_deref()
            .is_some_and(|parent| node_uid(&volume, parent) == root_uid),
        "a node created directly in the folder must name the folder's own uid as its parent — \
         otherwise a direct child would read as foreign and be skipped"
    );

    // 2. A later change to it: renamed within the root, which is what an edit or a move looks
    // like to a pair that already holds the node. From a cursor taken now, so the creation (and
    // anything the upload itself produced before this point) is not read as the rename.
    let cursor1 = events
        .latest_cursor(&volume)
        .expect("latest cursor before the rename");
    probe.names.push(renamed_rel.clone());
    client
        .rename_or_move(&remote_root, &probe_rel, &renamed_rel)
        .expect("rename the probe within the root");
    probe.names.retain(|name| name != &probe_rel);
    let updated = await_event(
        &events,
        &volume,
        &root_uid,
        &cursor1,
        &mut root_events,
        |change| {
            matches!(change.kind, RemoteChangeKind::Updated)
                && !change.trashed
                && node_uid(&volume, &change.node_id) == probe_uid
        },
    )
    .expect(
        "no Updated event for the renamed probe arrived: a change to a node in the folder is not \
         reported, and the pair would never see an edit",
    );
    assert!(
        updated
            .parent_id
            .as_deref()
            .is_some_and(|parent| node_uid(&volume, parent) == root_uid),
        "an update to a node directly in the folder must name the folder's own uid as its parent — \
         otherwise an edit of a file already synced would read as foreign and be skipped"
    );

    // Trash the probe now rather than leave it to the guard, so a failure to do it is a failure of
    // the run; the guard still tries every name if an assertion above had failed first.
    client
        .delete(&remote_root.join(&renamed_rel))
        .expect("trash the probe");
    probe.names.retain(|name| name != &renamed_rel);

    if root_events.is_empty() {
        eprintln!(
            "scope round-trip OK: create and rename both name the root uid as their parent; the \
             root itself got no event"
        );
    } else {
        eprintln!(
            "scope round-trip OK: create and rename both name the root uid as their parent, BUT \
             the root itself received {} event(s) {root_events:?} while the probe was made and \
             renamed. Every upload would then be read as \"the folder's own node changed\" and \
             walk: the root-event rule needs narrowing before this ships",
            root_events.len()
        );
    }
}
