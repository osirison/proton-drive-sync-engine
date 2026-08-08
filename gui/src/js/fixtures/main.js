// The main screen's datasets (F9, extended by S1) — `2a`, six frames: three 1040 windows and three
// compact panels.
//
// Written by F4 (the shell frames) and F6 (the panels) into what was then the only fixtures file, and
// moved here unchanged when F9 gave every screen family its own module. The local `now() - N` helper
// became clock.js's shared `ago(N)`.
//
// S1 IS THE FIRST TASK TO NEED MORE THAN THE SHELL FROM THEM, and asking for it changed two of the
// three windows. `2a Syncing` grew an `activity` and a `last_plan_summary` — the pass it is in the
// middle of, which F4 had no screen to hand them to — and `2a Needs you`'s queues turned out to be
// one conflict and two deletions rather than three conflicts, which is the only reading its attention
// band can actually be rendered from (DEVIATIONS §64). Both changes are the frame read more closely,
// not a new interpretation of it.
//
// These six are the only frames whose SCREENS exist, so they are the only ones carrying a `fids` map.
// The other nine modules in this directory are datasets waiting for S2–S10 to build something to
// compare — see frames.js.
//
// Every string with a copy-deck entry comes from `ui/copy.js`; the rest — `2 minutes ago`,
// `12,480 files`, `14s ago` — is formatter output, written literally because a fixture must reproduce
// the FRAME. Deriving them from `format.js` against a moving clock would make the gate's input depend
// on when it ran, which is the one thing a fixture may not do. See clock.js.
//
// `menu: true` asks for the standard tray menu for this state (`TRAY_MENU` in ui/compact.js). It is a
// flag rather than the rows themselves because no fixture module may import that component:
// ui/compact.js imports `fid` from frames.js, and `import-x/no-cycle` is an error.

import { MAIN } from "../ui/copy.js";
import { ago } from "./clock.js";
import { SHELL_FIDS, compactFids, mainFids } from "./fids.js";

/**
 * A `PlanSummary` (src/sync.rs) with every counter at zero, so a fixture states only the counters
 * its frame actually implies and cannot accidentally leave a field undefined that `format.dash()`
 * would then render as an em-dash — "unknown" where the daemon would have said zero.
 *
 * `total` is NEVER written by hand. `PlanSummary::from_plan` sets it to `plan.len()`, so a summary
 * whose total disagrees with its own counters is a reply the daemon cannot emit — DEVIATIONS §61,
 * which the first version of `9a Review`'s fixture learned the expensive way. `plan()` sums instead.
 */
const PLAN_ZEROES = {
  uploads: 0,
  downloads: 0,
  remote_directories_created: 0,
  local_directories_created: 0,
  local_moves: 0,
  remote_moves: 0,
  auto_links: 0,
  conflicts: 0,
  type_conflicts: 0,
  remote_deletes: 0,
  local_deletes: 0,
  purges: 0,
  skipped_unsupported: 0,
};

function plan(counters) {
  const summary = { ...PLAN_ZEROES, ...counters };
  return {
    ...summary,
    total: Object.values(summary).reduce((a, b) => a + b, 0),
    destructive_actions: summary.remote_deletes + summary.local_deletes + summary.purges,
  };
}

/**
 * Shaped exactly like the daemon's `StatusPayload` so the app cannot tell a fixture from a live
 * reply — anything that needs a special case here would be a special case in the app.
 *
 * The compact frames carry a `panel` instead: the arguments `ui/compact.js` takes. They are not a
 * status payload and should not be made into one — F6 ships the component, and deriving a panel from
 * a live status is S1's job for the window and S8's for the tray. A fixture that guessed at that
 * mapping now would be a third answer nobody had agreed to.
 */
const ROOTS = { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" };

/**
 * The pass `2a Syncing` and `2a Needs you` are both in the middle of: three changes, two leaving and
 * one arriving, fourteen seconds old, with `docs/spec.md` on the wire.
 *
 * `activity` is verbatim `SyncActivity` (src/ipc.rs) and carries ONE transfer, because that is what
 * the type holds — `activity.transfer` is a single `Option<TransferActivity>`, set at the top of each
 * upload or download and replaced by the next one. The frames draw three rows and two directions at
 * once; the reply cannot say that, and the gap is filed rather than invented here (DEVIATIONS §63).
 *
 * `bytes_done` IS NULL AND CANNOT BE OTHERWISE ON AN UPLOAD. daemon.rs fills `bytes_total` from the
 * local file's size for an upload and leaves `bytes_done` empty; for a download it samples
 * `bytes_done` from the staging directory and has no total at all, because a remote listing carries
 * no size. The two fields are never both present on one transfer, so a percentage is unreachable by
 * construction — which is what the drawn progress bar needs. Also §63.
 */
const PASS = {
  summary: plan({ uploads: 2, downloads: 1 }),
  activity: {
    phase: "executing",
    detail: "docs/spec.md",
    action_index: 1,
    action_total: 3,
    since_epoch_secs: ago(14),
    transfer: {
      direction: "upload",
      path: "docs/spec.md",
      // 1.2 MB through `format.bytes` — the size chip the frame draws on this row.
      bytes_total: 1200000,
      bytes_done: null,
      started_epoch_secs: ago(14),
    },
  },
};

export const MAIN_FIXTURES = {
  "2a Settled": {
    fids: { ...SHELL_FIDS["2a Settled"], ...mainFids({ state: "settled", buttons: 2 }) },
    status: {
      state: "idle",
      response: {
        // `running`, not `idle`. The daemon's own word is only ever `syncing` / `paused` /
        // `running` (src/daemon.rs:389-393,426); `idle` is the DERIVED state and lives on
        // `state` above. This shape came from F4 carrying an `idle` here, which the two
        // modules that copied it inherited before plan.js wrote the rule down.
        status: "running",
        paused: false,
        syncing: false,
        pending_changes: 0,
        last_sync_epoch_secs: ago(120),
        pending_deletions: [],
        config: ROOTS,
      },
    },
    conflicts: [],
  },
  "2a Syncing": {
    fids: {
      ...SHELL_FIDS["2a Syncing"],
      ...mainFids({ state: "syncing", tail: "columns", column: "left", rowIndex: 0, rowsInColumn: 2 }),
    },
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: ago(14),
        last_plan_summary: PASS.summary,
        pending_deletions: [],
        config: ROOTS,
        activity: PASS.activity,
      },
    },
    conflicts: [],
  },
  "2a Needs you": {
    fids: {
      ...SHELL_FIDS["2a Needs you"],
      ...mainFids({
        state: "syncing",
        tail: "columns",
        column: "left",
        rowIndex: 0,
        rowsInColumn: 1,
        band: 2,
      }),
    },
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: ago(14),
        // The two withheld deletions are plan rows like any other, so they are in the summary the
        // daemon publishes before the transfers start (daemon.rs `execute_plan_and_commit`).
        last_plan_summary: plan({ uploads: 2, downloads: 1, remote_deletes: 1, local_deletes: 1 }),
        /**
         * ONE CONFLICT AND TWO DELETIONS, not three conflicts — read off the band the frame draws.
         *
         * The first version of this fixture pinned three conflicts and an empty deletion queue,
         * which made the shell's chip right (`3 waiting`, ring dot) and the band impossible: its
         * second row says `Two deletions are waiting on you` / `1 removes from this computer
         * permanently · 1 goes to Proton's Trash`, and no conflict can produce that sentence. So the
         * frame's own state is 1 + 2, the chip's count is the SUM, and the ring wins over the fill —
         * which falsifies DEVIATIONS §44's "nothing draws decisions and deletions at once". §64.
         *
         * Verbatim `PendingDeletion`, and the directions are the ones the band's sub-line names:
         * `local` applies the delete on this computer and is the permanent one, `remote` moves
         * Proton's copy to the Trash. `deletions.js` documents the same pairing at length.
         */
        pending_deletions: [
          {
            path: "photos/2019/IMG_0421.jpg",
            direction: "local",
            entity_kind: "file",
            fingerprint: "9f2c4a1b7e0d3856c9a2f4b18d7e0c3a5b6f9d21",
            detected_epoch_secs: ago(9 * 60),
          },
          {
            path: "archive/old-notes.md",
            direction: "remote",
            entity_kind: "file",
            fingerprint: "0b4f1e7a2c9d6e35a8f0c1b2d3e4f5a6b7c8d9e0",
            detected_epoch_secs: ago(6 * 60),
          },
        ],
        config: ROOTS,
        activity: PASS.activity,
      },
    },
    // `scan_conflicts` returns `{ original, sidecar }` pairs (gui-core/src/conflicts.rs) — NOT a
    // `path`. The three entries this replaced were `{ path }`, which nothing had noticed because
    // `unresolvedConflictCount()` only reads `.length`; the band is the first thing to render a
    // conflict's own path, and it would have drawn `undefined · both copies kept, nothing lost`.
    conflicts: [{ original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt" }],
  },

  "2a Compact settled": {
    fids: compactFids({ state: "settled", tail: "footer", tailAt: 1, buttons: 2 }),
    panel: {
      state: "settled",
      headline: MAIN.compact.upToDate,
      sub: "2 minutes ago",
      subMono: true,
      footer: {
        status: "12,480 files",
        buttons: [{ label: MAIN.pause }, { label: MAIN.compact.open, kind: "secondaryAlt" }],
      },
    },
  },

  "2a Compact syncing": {
    fids: compactFids({
      state: "syncing",
      tail: "footer",
      tailAt: 2,
      buttons: 2,
      rows: ["up", "down"],
    }),
    panel: {
      state: "syncing",
      headline: MAIN.syncing(3),
      count: 3,
      // 0.64 and 0.31 of a 330px track — the two bars the frame draws, to the pixel.
      transfers: [
        { direction: "up", name: "docs/spec.md", progress: 0.64 },
        { direction: "down", name: "reports/q3-summary.pdf", progress: 0.31 },
      ],
      footer: {
        status: "14s ago",
        buttons: [{ label: MAIN.pause }, { label: MAIN.compact.open, kind: "secondaryAlt" }],
      },
    },
  },

  "2a Compact needs you": {
    fids: compactFids({ state: "needsYou", tail: "footer", tailAt: 1, buttons: 1 }),
    panel: {
      state: "needsYou",
      headline: MAIN.compact.needYou(3),
      count: 3,
      // Two sentences that break in a fixed place, not a paragraph that wraps.
      sub: [MAIN.compact.conflictLine, MAIN.compact.deletionLine],
      action: { label: MAIN.compact.review },
      footer: { status: MAIN.compact.syncingContinues, buttons: [{ label: MAIN.compact.later }] },
    },
  },
};
