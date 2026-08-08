// Conflict-screen fixtures (F9) — one deterministic dataset per `3a` frame.
//
// A CONFLICT IS A FACT ABOUT THE DISK, NOT ABOUT THE DAEMON, and that one observation shapes the
// whole module. `ControlResponse` carries no conflict field at all; `scan_conflicts` walks the local
// root for `*.proton-cloud[.ext]` sidecars and returns `{ original, sidecar }` pairs
// (gui-core/src/conflicts.rs). So all three frames share one status payload and differ only in what
// the scan finds — the daemon really is in the same state before and after you settle three files,
// which is exactly what the frames draw.
//
// THE CHIP IS WHAT TIES THE TWO HALVES TOGETHER. `3a Conflict` and `3a Conflict diff` draw
// `3 waiting`, and DEVIATIONS §44 settles the priority behind it: a decision outranks a transfer,
// deletions outrank decisions, and nothing draws both at once. Three conflicts with
// `pending_deletions: []` is therefore the only combination that makes the drawn chip the *decision*
// variant (1px ring dot, §43) rather than the deletions one (filled). `deletions.js` holds the exact
// mirror image, and the two files are only consistent if both keep their opposite side empty.
//
// THIS MODULE IMPORTS NO COPY, and that is not an oversight of rule 1. It carries no strings that the
// user reads: every fixed string on the three screens is already a constant in `ui/copy.js`
// (`CONFLICTS.*`, numbers baked in — `You settled 3 files…`, `2 lines differ · 3 lines identical`)
// which the screen renders directly, and everything else the frames draw is rendered by the app from
// a number pinned below. The only strings here are data — paths, and the two files' actual bytes.

import { ago } from "./clock.js";

/**
 * The three conflicts the chip counts, in the order the frames draw them: `notes/todo.txt` fills the
 * window at `1 of 3`, and the diff view's "Still waiting after this one" list is `design/logo.svg`
 * at `2 of 3` then `photos/trip` at `3 of 3`.
 *
 * Sidecar names are what `sync::conflict_copy_path` would actually have written, both of its forms:
 * `{stem}.proton-cloud.{ext}` for the two files, and the extensionless `{name}.proton-cloud` for
 * `photos/trip` — which is a *type* conflict (a folder here, a file on Proton), and the engine does
 * write a sidecar for those too (daemon.rs downloads the remote file beside the local directory), so
 * a real scan does return all three.
 *
 * TWO THINGS THE FRAME ASKS FOR THAT THIS REPLY CANNOT SAY, both left to S2:
 *
 *   · THE ORDER IS NOT THE REPLY'S ORDER. `scan_conflicts` ends with `out.sort()`, so a real reply
 *     is `design`, `notes`, `photos` — while the frame puts `notes/todo.txt` first with `‹`
 *     disabled, i.e. genuinely at position 1. Written in the drawn order, because reproducing the
 *     frame is what a fixture is for; the divergence is real and belongs in the screen's own
 *     decision about queue order, not hidden here by silently sorting.
 *   · WHICH ROW IS A TYPE CONFLICT IS NOT DERIVABLE. The queue draws `both changed it` against
 *     `design/logo.svg` and `a folder here, a file there` against `photos/trip`
 *     (`CONFLICTS.bothChanged` / `CONFLICTS.typeConflict`). Nothing in `Conflict` says which, and
 *     `read_conflict_pair` cannot tell either: reading a directory fails with EISDIR, which lands in
 *     the same `binary_or_large: true` arm as a JPEG.
 */
const QUEUE = [
  { original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt" },
  { original: "design/logo.svg", sidecar: "design/logo.proton-cloud.svg" },
  { original: "photos/trip", sidecar: "photos/trip.proton-cloud" },
];

/**
 * Both sides of `notes/todo.txt`, verbatim `ConflictPair` — one constant, shared by `3a Conflict`
 * and `3a Conflict diff` because they are two views of the same conflict and a second copy could
 * only ever drift from the first.
 *
 * The texts are the diff's own lines, read straight off the frame: four lines on the left, five on
 * the right, three of them identical — which is what makes `CONFLICTS.diffCounts` ("2 lines differ ·
 * 3 lines identical") true, counting the changed pair and the absent line. The version cards' `4
 * lines`/`5 lines` and the prose summaries (capability #3, Phase 1 via C3) come from these same two
 * strings plus the mtimes, which is why the disclosure needs no extra data.
 *
 * THE FRAME CONTRADICTS ITSELF ON THE RIGHT-HAND SIZE and the fixture reproduces the frame rather
 * than picking a side. `41 bytes` is exactly the left text's length; `44 bytes` is not the right
 * text's — those five lines are 53. Both numbers are drawn, so `size` is pinned to the drawn one and
 * `text` to the drawn lines. (api.js's generic mock already made this same choice.)
 *
 * The mtimes are `ago()` offsets, not the drawn clock times, and they are corroborated rather than
 * chosen: the frame draws `edited 14:38` and `edited 14:41`, three minutes apart, and the deck's card
 * copy says the two changes were `5 minutes ago` and `2 minutes ago` — the same three minutes. So
 * 300/120 is the frame's own arithmetic. What no epoch can reproduce is `14:38` itself: rendered from
 * a timestamp it moves with the machine's timezone and the hour of the run, and clock.js's rule is
 * that such a string is written literally — but `ConflictSide` has no field to write it into, and
 * inventing one would be inventing a command reply. Named here so S2 meets it as a known node rather
 * than a mystery.
 */
const TODO_PAIR = {
  original: {
    exists: true,
    size: 41,
    mtime_epoch_secs: ago(300),
    text: "# Todo\n- buy milk\n- call Alice\n- ship v1\n",
    binary_or_large: false,
  },
  sidecar: {
    exists: true,
    size: 44,
    mtime_epoch_secs: ago(120),
    text: "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n",
    binary_or_large: false,
  },
};

/**
 * One daemon, three frames. Idle and not syncing because none of the three draws a transfer or a
 * syncing chip — `derive_state` calls that `Idle` (reachable, not paused, nothing pending), and
 * `state` has to agree with the response or the shell renders a state the response contradicts.
 *
 * `last_sync_epoch_secs` is the one field here no `3a` node draws, and it is pinned anyway because
 * `derive_state` reads it: a null last-sync with an empty history is FIRST RUN, not idle, and
 * `counters_unknown()` would blank the shell's counters to em-dashes.
 */
const IDLE = {
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
    pending_deletions: [],
    config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
  },
};

export const CONFLICT_FIXTURES = {
  "3a Conflict": {
    status: IDLE,
    conflicts: QUEUE,
    conflictPair: TODO_PAIR,
  },

  // The disclosure, not a different screen: same daemon, same queue, same pair — one thing is open.
  // `diff` is the whole difference, per rule 5; which conflict is open is not restated because it is
  // still the first, which the queue's own order already says.
  "3a Conflict diff": {
    status: IDLE,
    conflicts: QUEUE,
    conflictPair: TODO_PAIR,
    ui: { diff: true },
  },

  // Nothing left to decide — an empty scan, and that alone. The frame's own sub, `You settled 3
  // files. Two kept both versions, one took Proton's copy.`, is a fixed deck constant with its
  // numbers baked in (`CONFLICTS.clearedSub`), so the fixture needs no memory of what was chosen.
  //
  // Worth knowing before it looks like a bug: this frame draws NO status chip, while `2a Settled` —
  // also idle, also chipped in the same header — draws `idle`. Same data, two drawings. Nothing a
  // fixture can express either way; the narrow 522px window is still the product window, header and
  // doors included (DEVIATIONS §48).
  "3a Conflicts cleared": {
    status: IDLE,
    conflicts: [],
  },
};
