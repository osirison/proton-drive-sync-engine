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
// `3 waiting`, and DEVIATIONS §43/§44 settle the priority behind it: a decision outranks a transfer,
// the chip's number is the SUM of the two queues, and the decision RING wins whenever a conflict is
// among them — deletions take the filled dot only when they are alone, which is what `4a Deletions`
// draws. (§44 originally recorded "nothing draws decisions and deletions at once" and chose the
// order from that; `2a Needs you` draws both, and S1 reopened it — §64.)
//
// Three conflicts with `pending_deletions: []` is still the right dataset here, and now for a
// narrower reason: it is what these two frames are IN, not what the chip variant requires.
// `deletions.js` holds the exact mirror image.
//
// THIS MODULE IMPORTS NO COPY, and that is not an oversight of rule 1. It carries no strings that the
// user reads: every fixed string on the three screens is already a constant in `ui/copy.js`
// (`CONFLICTS.*`, numbers baked in — `You settled 3 files…`, `2 lines differ · 3 lines identical`)
// which the screen renders directly, and everything else the frames draw is rendered by the app from
// a number pinned below. The only strings here are data — paths, and the two files' actual bytes.

import { ago } from "./clock.js";
import { conflictFids } from "./fids.js";

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
 * ONE OF THE TWO THINGS THIS FIXTURE LEFT TO S2 IS NOW IN THE REPLY. It recorded that "which row is
 * a type conflict is not derivable" — nothing in `Conflict` said which, and `read_conflict_pair`
 * could not tell either, since reading a directory fails with EISDIR and lands in the same
 * `binary_or_large: true` arm as a JPEG. S2 needs the distinction (`04-conflicts.md` hides the
 * disclosure for a type conflict and gives it different card copy), so `scan_conflicts` now answers
 * it: it is standing at the original with a path in hand, and one `symlink_metadata` settles it.
 * Hence `kind` below — `type` on `photos/trip`, which is what makes the queue's
 * `a folder here, a file there` renderable rather than hard-coded.
 *
 * THE OTHER ONE STANDS. The order is not the reply's order: `scan_conflicts` ends with `out.sort()`,
 * so a real reply is `design`, `notes`, `photos` — while the frame puts `notes/todo.txt` first with
 * `‹` disabled, i.e. genuinely at position 1. Written in the drawn order, because reproducing the
 * frame is what a fixture is for; the divergence is real and belongs in the screen's own decision
 * about queue order, not hidden here by silently sorting.
 */
const QUEUE = [
  { original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt", kind: "content" },
  { original: "design/logo.svg", sidecar: "design/logo.proton-cloud.svg", kind: "content" },
  // `a folder here, a file there` — a folder locally, a file on Proton. The engine downloads the
  // remote file beside the local directory, so a sidecar exists and a real scan returns all three.
  { original: "photos/trip", sidecar: "photos/trip.proton-cloud", kind: "type" },
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
  /**
   * How each side moved from the version they last agreed on (#217) — and this is a COMPUTED reply
   * field, so it is derived from the frames rather than chosen.
   *
   * `3a Conflict diff` draws yours at four lines against Proton's five, line 2 differing and line 5
   * unmatched, under `2 lines differ · 3 lines identical`. That is self-consistent. Solving it for
   * an ancestor gives exactly one answer that is also a genuine two-sided conflict:
   *
   *     # Todo | - buy oat milk | - call Alice | - ship v1
   *
   * — you changed line 2 back, Proton added line 5. Taking the drawn CARDS at their word instead
   * (`You added a line` / `Changed a line and added one`) forces the ancestor to equal your copy,
   * under which the local file never moved and the planner reaches `(Unchanged, Changed)` and plans
   * a plain download. The drawn card sentences describe a state this engine cannot produce, so the
   * diff frame is the half this fixture follows. `src/ancestor.rs` pins the arithmetic; DEVIATIONS
   * §105 records the difference.
   */
  happened: {
    mine: { added: 0, changed: 1, removed: 0 },
    theirs: { added: 1, changed: 0, removed: 0 },
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
    route: "conflicts",
    fids: conflictFids("card"),
  },

  // The disclosure, not a different screen: same daemon, same queue, same pair — one thing is open.
  // `diff` is the whole difference, per rule 5; which conflict is open is not restated because it is
  // still the first, which the queue's own order already says.
  "3a Conflict diff": {
    status: IDLE,
    conflicts: QUEUE,
    conflictPair: TODO_PAIR,
    ui: { diff: true },
    route: "conflicts",
    fids: conflictFids("diff"),
  },

  // Nothing left to decide — an empty scan, and that alone. The frame's own sub, `You settled 3
  // files. Two kept both versions, one took Proton's copy.`, is a fixed deck constant with its
  // numbers baked in (`CONFLICTS.clearedSub`), so the fixture needs no memory of what was chosen.
  //
  // Worth knowing before it looks like a bug: this frame draws NO status chip, while `2a Settled` —
  // also idle, also chipped in the same header — draws `idle`. Same data, two drawings. Nothing a
  // fixture can express either way; the narrow 522px window is still the product window, header and
  // doors included (DEVIATIONS §48).
  //
  // `settled` IS NOT MEMORY OF WHAT WAS CHOSEN — it is what the SCREEN counted while you were on it,
  // and the sentence is a template rather than the constant this comment first called it. Nothing on
  // disk can supply it (a resolved conflict leaves a sidecar or nothing, and neither says which
  // button was pressed), so app.js counts as the choices are made and this pins the count the frame
  // draws. Reaching the frame's own wording needs 3 · 2 · 1; an unpinned fixture renders the
  // zero-form of the same sentence, which is a different string and a red gate.
  "3a Conflicts cleared": {
    status: IDLE,
    conflicts: [],
    ui: { settled: { total: 3, keptBoth: 2, tookProton: 1 } },
    route: "conflicts",
    fids: conflictFids("cleared"),
  },
};
