// The deletion queue's datasets (F9) — `4a`, four frames: three window/dialog and one panel.
//
// (`4a Compact` is the 362px panel and belongs to F6's `panel` family rather than to the status
// payloads above it. It sits at the foot of this file, moved from frames.js by F9 so that every
// frame of a set is in the module named for the set, and unchanged otherwise.)
//
// THE QUEUE IS WRITTEN ONCE AND LIVES IN THE STATUS, and that is a statement about the wire rather
// than a shortcut. `list_pending_deletions` has no reply of its own: commands.rs sends a plain
// `Status` and returns `response.pending_deletions` from it, so on a real daemon the command's
// answer IS that field. `api.js` reads through to it for the same reason.
//
// So there is no top-level `deletions` key here. An earlier version carried one beside the status
// — the same array, so nothing was wrong — under a comment saying the two could not disagree. They
// could: nothing stopped a later edit changing one, and the mock preferred whichever it found
// first. A second source of truth for one list is the thing that eventually disagrees with itself,
// and the argument for writing it once is the argument for storing it once.
//
// SEVERITY IS THE SCREEN AND `direction` IS THE FIELD, which is the one thing here that is easy to
// get backwards. `DeleteDirection::Local` means "apply the delete on the local disk, because it went
// first on Proton" — the frame's permanent left column, whose facts line says `deleted on Proton 22m
// ago`. `Remote` is the mirror: the file went here first, and approving moves Proton's copy to the
// Trash — the recoverable right column, `deleted here 6m ago`. Column and direction name the same
// side; they are just named from different ends.
//
// The chip reads `2 waiting` in the deletions variant — filled destructive dot, not the decision
// ring (DEVIATIONS §43) — which is only correct while `conflicts` is empty: the ring wins whenever a
// conflict is among what is waiting, and the count is the sum of both queues (§44, as S1 corrected it
// in §64). `conflicts.js` holds the mirror image.

// `MAIN`/`DELETIONS` and `compactFids` are needed by the `4a Compact` panel below; `ago` by the
// three window frames above it.
import { MAIN, DELETIONS } from "../ui/copy.js";
import { ago } from "./clock.js";
import { compactFids } from "./fids.js";

/**
 * The two withheld deletions, verbatim `PendingDeletion` (src/ipc.rs), in the order the frame's two
 * columns read left to right.
 *
 * `detected_epoch_secs` is what the facts strip's `22m ago` / `6m ago` are rendered from — the mono
 * `short` register of `format.js`'s `since()` — so the drawn strings move with no clock but their
 * own offsets. `fingerprint` draws nowhere, and is still real data rather than a blank: it is what an
 * approval is pinned to (a file's baseline SHA-1, a directory's composed `volumeId~nodeId`), the
 * daemon refuses one whose fingerprint no longer matches, and the GUI's approve/deny acknowledgement
 * pill is keyed on it — a row with an empty fingerprint is a row that cannot be approved in preview.
 * The bytes themselves are opaque; only their shape and their stability matter.
 *
 * WHAT THE CARDS DRAW THAT THIS REPLY CANNOT PRODUCE — the facts strip, in full:
 *
 *   · `1,204 photos, 8.4 GB` (`DELETIONS.folderConsequence`, and the `1,204` in
 *     `DELETIONS.armedTitle` on `4a Armed`) is a SUBTREE aggregate over a directory. No command
 *     aggregates one, and a directory's `file_size` in `file_index` is not a subtree total.
 *   · `last opened Mar 2024` is an atime. The index stores mtime only — and an absolute date is a
 *     literal by clock.js's rule in any case.
 *   · `4 KB` and `last edited Jan 2026` are reachable today — `path_sync_status` returns the index
 *     record's `file_size` and `mtime` for one path — just not from the deletions reply. So they are
 *     CARRIED, in `pathStatus` below, keyed by path: a second command's reply is exactly what the
 *     fixture entry shape has that key for, and leaving a reproducible node unreproducible because
 *     its data arrives from a different command would be the rule misapplied. `last edited Jan 2026`
 *     is still absolute and so is not derived from the `mtime` — see the note there.
 *
 * The first two ARE genuinely unproducible, and neither is one of the ten capabilities in
 * `14-behaviour-and-state.md` nor one of the four Phase-2 gaps in `IMPLEMENTATION-PLAN.md` §4. They
 * are left out rather than invented, and filed as their own gap. The deck narrows the `4a Armed` half
 * to exactly one number: `DELETIONS.armedBody` already hardcodes `photos/2019` and `8.4 GB`, so only
 * `armedTitle(n)`'s count has nowhere to come from.
 */
const PENDING = [
  {
    path: "photos/2019",
    direction: "local",
    entity_kind: "directory",
    fingerprint: "vol_2QF9xR7k~node_7pK3mD4c",
    detected_epoch_secs: ago(22 * 60),
  },
  {
    path: "archive/old-notes.md",
    direction: "remote",
    entity_kind: "file",
    fingerprint: "0b4f1e7a2c9d6e35a8f0c1b2d3e4f5a6b7c8d9e0",
    detected_epoch_secs: ago(6 * 60),
  },
];

/**
 * `path_sync_status` replies, keyed by the path asked for — verbatim `EmblemStatus`
 * (`commands.rs`), whose every field but `tracked` is nullable because a record may not exist.
 *
 * ONE ROW, BECAUSE ONE ROW CAN BE ANSWERED. The file card's `4 KB` is `file_size` through
 * `format.bytes`, which renders 4096 as exactly `4 KB`. The folder card's `1,204 photos, 8.4 GB` is
 * a subtree aggregate and there is no `EmblemStatus` for a directory that carries one, so
 * `photos/2019` is absent here rather than present with an invented total — the difference between
 * the two cards is the difference between a gap and a lookup.
 *
 * `last edited Jan 2026` is NOT rendered from `mtime`. It is an absolute date, which clock.js says a
 * fixture writes literally, and the epoch here would format to whatever month the reader's timezone
 * puts it in. The pin is corroboration — a real index record has one — not the drawn string's source.
 */
const PATH_STATUS = {
  "archive/old-notes.md": {
    tracked: true,
    sync_status: "synced",
    entity_kind: "file",
    file_size: 4096,
    mtime: 1_767_000_000,
    proton_id: "vol_2QF9xR7k~node_1aB8xY2z",
  },
};

/**
 * The daemon behind all three frames, differing only in what is queued — which is the honest model:
 * approving or keeping a deletion empties the queue without changing anything else about the pass.
 *
 * Idle and not syncing because no `4a` frame draws a transfer or a syncing chip, and `derive_state`
 * calls that `Idle` (reachable, not paused, nothing pending). `last_sync_epoch_secs` draws nowhere on
 * these screens but is pinned regardless: a null last-sync with an empty history reads as FIRST RUN,
 * whose `counters_unknown()` blanks the shell's counters to em-dashes.
 */
const idleWith = (pendingDeletions) => ({
  state: "idle",
  response: {
    // `running`, not `idle`. The daemon's own word is only ever `syncing` / `paused` /
    // `running` (src/daemon.rs:389-393,426); `idle` is the DERIVED state, and it is
    // already on `state` above. Copied from main.js's shape, `idle` and all.
    status: "running",
    paused: false,
    syncing: false,
    pending_changes: 0,
    last_sync_epoch_secs: ago(120),
    pending_deletions: pendingDeletions,
    config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
  },
});

export const DELETION_FIXTURES = {
  "4a Deletions": {
    status: idleWith(PENDING),
    pathStatus: PATH_STATUS,
    conflicts: [],
  },

  // The armed takeover is the same queue with one item's gate satisfied — a body swap over the same
  // screen, not a dialog (DEVIATIONS §57). So it is the same data plus the smallest thing that names
  // the difference: WHICH item is armed. Nothing about the typed word belongs here; the field is
  // empty in `4a Deletions` (the frame draws its `DELETE` placeholder, not a value) and clears on
  // blur by design, so it is state the screen owns for as long as a keystroke.
  //
  // The count in `Delete 1,204 photos from this computer?` is deliberately NOT smuggled in beside
  // `armed`. It is data about the folder, not about what is open, and the gap it belongs to is named
  // on `PENDING` above.
  "4a Armed": {
    status: idleWith(PENDING),
    pathStatus: PATH_STATUS,
    conflicts: [],
    ui: { armed: "photos/2019" },
  },

  // Nothing waiting: an empty queue, and nothing else. Both of the frame's strings are fixed deck
  // constants (`DELETIONS.emptyTitle` / `emptySub`), and the empty state is what the screen shows
  // when both columns have collapsed — so the absence IS the dataset. The status still carries the
  // empty array explicitly rather than omitting the field, because an absent `pending_deletions` and
  // an empty one are the same on this wire (`#[serde(default)]`) and only one of them says so.
  "4a Empty": {
    status: idleWith([]),
    conflicts: [],
  },

  // ---- the compact panel (F6). Written when ui/compact.js was built, and moved here from
  // frames.js by F9 so every 4a frame lives in one module. Unchanged: it is the component's
  // arguments, not a status payload, and the two must not be conflated.
  "4a Compact": {
    fids: compactFids({ state: "deletions", tail: "footer", tailAt: 3, buttons: 1 }),
    panel: {
      state: "deletions",
      headline: DELETIONS.compact.title(2),
      count: 2,
      deletions: [
        { severity: "permanent", name: "photos/2019", note: DELETIONS.compact.permanent },
        { severity: "recoverable", name: "archive/old-notes.md", note: DELETIONS.compact.recoverable },
      ],
      // `Review them`, and nothing that approves anything — see ui/compact.js.
      action: { label: DELETIONS.compact.review },
      footer: { status: MAIN.compact.syncingContinues, buttons: [{ label: MAIN.compact.later }] },
    },
  },
};
