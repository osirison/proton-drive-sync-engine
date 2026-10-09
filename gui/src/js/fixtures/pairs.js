// The datasets of the window at TWO FOLDERS OR MORE (#102 phase 5c-1) — six dark frames and two light.
//
// Every other fixture in this directory is a reply from a daemon that predates folder pairs (no `pair`,
// no `pairs`), and `fidelity:n1` holds the one-folder app to those. These are the opposite on purpose:
// the replies a CURRENT daemon sends when it runs more than one pair, which is the only state the
// selector, the pause button that names its folder, and the notice for a folder the daemon does not run
// are ever drawn in. So they carry what `StatusPayload` carries — `selected`, `pairs`, `pair_states`,
// and on the reply `pair`/`pairs` — and `check-n1-identity.mjs` does not try to rewrite them as one pair
// (a frame that lists two folders has no one-folder rendering to be identical to).
//
//   2a Two folders           the pill at rest: `documents` selected, nothing waiting anywhere
//   2a Two folders open      the list open: four folders, one in each of four states, one with a count
//   2a Two folders waiting   `photos` has deletions waiting; the pill carries the ring, the chip does not
//   2a Two folders failed    `photos`' last pass did not finish; the pill carries the solid dot (the problem
//                            form), the chip does not — and the frame is NOT in the hue gate's settled set
//   2a Folder not running    `photos` is in the settings file and not in the running daemon
//   2a Two folders unsaved   a resume the daemon could not write down
//   12a Two folders light    ← 2a Two folders
//   12a Two folders open light ← 2a Two folders open
//
// THE SUB-LINE IS `last synced 2 minutes ago` AND NOTHING AFTER IT. `2a Settled` draws `· 12,480 files ·
// 41.2 GB` and the app cannot (G7, #207), which is a recorded deviation there. These frames are new, so
// they are drawn at what the app can say: a second deviation row for a clause the design never drew
// here would record a gap in a drawing nobody made.
//
// Every string with a copy-deck entry comes from `ui/copy.js`; `2 minutes ago` is formatter output,
// written literally for the reason `main.js` gives.

import { ago } from "./clock.js";
import { SHELL_FIDS, mainFids, pairSelectFids, noticeFids } from "./fids.js";

/** What a pair's `PairSummary` carries, from the three roots a folder has and the numbers a test cares about. */
const summary = (name, over = {}) => ({
  name,
  local_root: `~/${name[0].toUpperCase()}${name.slice(1)}`,
  remote_root: `/Drive/${name[0].toUpperCase()}${name.slice(1)}`,
  db_path: `~/${name[0].toUpperCase()}${name.slice(1)}/.sync/sync_index.db`,
  paused: false,
  syncing: false,
  reconcile_seq: 7,
  last_sync_epoch_secs: ago(120),
  last_error: null,
  pending_changes: 0,
  pending_deletions: 0,
  ...over,
});

/** `gui_core::PairState`: Rust's own derivation for a folder, with the rank it sorts by. */
const stateOf = (name, state, rank) => ({ name, state, rank });

/**
 * The reply as the daemon sends it for the pair it is about (`documents`, the default and the selected
 * one): the settled `2a Settled` reply with `pair`/`pairs` on it. `pending_deletions` here is the LIST
 * (the selected folder's own queue), where each `PairSummary` carries a COUNT.
 */
function replyFor(pairs, over = {}) {
  return {
    status: "running",
    paused: false,
    syncing: false,
    reconcile_seq: 7,
    pending_changes: 0,
    message: "sync completed",
    last_sync_epoch_secs: ago(120),
    last_error: null,
    pending_deletions: [],
    config: {
      local_root: "~/Documents",
      remote_root: "/Drive/Documents",
      db_path: "~/Documents/.sync/sync_index.db",
    },
    pair: "documents",
    pairs,
    ...over,
  };
}

/** A well-formed `read_config` reply (`ConfigPayload`), listing `names` — what the file says, in file order. */
function configListing(names) {
  return {
    path: "~/.config/proton-sync/proton-sync.toml",
    exists: true,
    toml: "",
    pair: "documents",
    pairs: names.map((name) => ({
      name,
      local_root: `~/${name[0].toUpperCase()}${name.slice(1)}`,
      remote_root: `/Drive/${name[0].toUpperCase()}${name.slice(1)}`,
    })),
    local_root: "~/Documents",
    remote_root: "/Drive/Documents",
    scan_interval_secs: null,
    full_scan_schedule: null,
    events_driven: null,
    include: [],
    exclude: [],
    proton_cli: null,
    proton_timeout_secs: null,
    proton_list_attempts: null,
    delete_approval_remote: null,
    delete_approval_local: null,
    deletion_policy: "ask_every_time",
    local_delete_mode: "trash",
  };
}

const TWO = [summary("documents"), summary("photos")];

/** The window's shell slots: `2a Settled`'s, plus the selector's. */
const shell = (options) => ({ ...SHELL_FIDS["2a Settled"], ...pairSelectFids(options) });

/**
 * The four folders of the open list, one in each state the list can draw: `documents` is chosen and
 * idle, `photos` is mid-pass with two deletions waiting on a person, `music` is paused by its owner and
 * `archive`'s last pass did not finish. The count is the ONLY thing the list says about waiting — and
 * only `photos` has one, which is also why the pill carries the ring.
 */
const FOUR = [
  summary("documents"),
  summary("photos", { syncing: true, pending_changes: 3, pending_deletions: 2 }),
  summary("music", { paused: true, pending_changes: 1 }),
  summary("archive", { last_error: "the remote listing timed out" }),
];
const FOUR_STATES = [
  stateOf("documents", "idle", 0),
  stateOf("photos", "running", 2),
  stateOf("music", "paused", 1),
  stateOf("archive", "failed", 4),
];

/** Shared by the three settled frames that differ only in what the pill and the band say. */
const settledStatus = (pairs, states, extra = {}) => ({
  state: "idle",
  selected: "documents",
  pairs,
  pair_states: states,
  response: replyFor(pairs),
  ...extra,
});

const IDLE_TWO = [stateOf("documents", "idle", 0), stateOf("photos", "idle", 0)];

export const PAIR_FIXTURES = {
  "2a Two folders": {
    fids: { ...shell({}), ...mainFids({ state: "settled", buttons: 2 }) },
    status: settledStatus(TWO, IDLE_TWO),
    conflicts: [],
  },

  "2a Two folders open": {
    fids: { ...shell({ marker: true, rows: 4, selected: 0 }), ...mainFids({ state: "settled", buttons: 2 }) },
    // The chip is the SELECTED folder's alone (decision D4): `documents` is idle, so it reads `idle`
    // however much is waiting in `photos`. The ring on the pill is what says so.
    ui: { pairMenuOpen: true },
    status: settledStatus(FOUR, FOUR_STATES),
    conflicts: [],
  },

  "2a Two folders waiting": {
    fids: { ...shell({ marker: true }), ...mainFids({ state: "settled", buttons: 2 }) },
    status: settledStatus(
      [summary("documents"), summary("photos", { pending_deletions: 2 })],
      [stateOf("documents", "idle", 0), stateOf("photos", "idle", 0)],
    ),
    conflicts: [],
  },

  "2a Two folders failed": {
    fids: { ...shell({ marker: true }), ...mainFids({ state: "settled", buttons: 2 }) },
    // `photos` is the folder whose last pass did not finish; `documents` is on screen and fine, so the chip
    // reads `idle` and the window says nothing — the solid dot on the pill is the only place it shows.
    status: settledStatus(
      [summary("documents"), summary("photos", { last_error: "the remote listing timed out" })],
      [stateOf("documents", "idle", 0), stateOf("photos", "failed", 4)],
    ),
    conflicts: [],
  },

  "2a Folder not running": {
    fids: {
      ...SHELL_FIDS["2a Folder not running"],
      ...pairSelectFids({}),
      ...mainFids({ state: "settled", buttons: 2 }),
      ...noticeFids({ at: 2, action: true }),
    },
    // The daemon runs `documents` and `music`; the file also lists `photos`, which this window remembered
    // (`pair_unknown`). It shows the default folder and says so.
    status: settledStatus(
      [summary("documents"), summary("music")],
      [stateOf("documents", "idle", 0), stateOf("music", "idle", 0)],
      { pair_unknown: "photos" },
    ),
    config: configListing(["documents", "music", "photos"]),
    conflicts: [],
  },

  "2a Two folders unsaved": {
    fids: {
      ...SHELL_FIDS["2a Folder not running"],
      ...pairSelectFids({}),
      ...mainFids({ state: "settled", buttons: 2 }),
      ...noticeFids({ at: 2, reason: true }),
    },
    // `ui.unsavedPause`: a frame cannot press Resume, so it names the reply that Resume would have
    // brought home — the folder it was for, and the daemon's own reason.
    ui: { unsavedPause: { kind: "resumeUnsaved", reason: "attempt to write a readonly database" } },
    status: settledStatus(TWO, IDLE_TWO),
    conflicts: [],
  },

  // ---------------------------------------------------------------------------------- the light pair ----
  "12a Two folders light": { sameAs: "2a Two folders" },
  "12a Two folders open light": { sameAs: "2a Two folders open" },
};
