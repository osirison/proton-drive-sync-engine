// The main screen's datasets (F9) — `2a`, six frames: three 1040 windows and three compact panels.
//
// Written by F4 (the shell frames) and F6 (the panels) into what was then the only fixtures file, and
// moved here unchanged when F9 gave every screen family its own module. The local `now() - N` helper
// became clock.js's shared `ago(N)`; nothing else about them changed, and `npm run fidelity` reports
// the same 12,441 assertions across the same 11 mapped frames as it did before the move.
//
// These six are the only frames whose SCREENS exist, so they are the only ones carrying a `fids` map.
// The other nine modules in this directory are datasets waiting for S1–S10 to build something to
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
import { SHELL_FIDS, compactFids } from "./fids.js";

/**
 * Shaped exactly like the daemon's `StatusPayload` so the app cannot tell a fixture from a live
 * reply — anything that needs a special case here would be a special case in the app.
 *
 * The compact frames carry a `panel` instead: the arguments `ui/compact.js` takes. They are not a
 * status payload and should not be made into one — F6 ships the component, and deriving a panel from
 * a live status is S1's job for the window and S8's for the tray. A fixture that guessed at that
 * mapping now would be a third answer nobody had agreed to.
 */
export const MAIN_FIXTURES = {
  "2a Settled": {
    fids: SHELL_FIDS["2a Settled"],
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
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    conflicts: [],
  },
  "2a Syncing": {
    fids: SHELL_FIDS["2a Syncing"],
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: ago(14),
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    conflicts: [],
  },
  "2a Needs you": {
    fids: SHELL_FIDS["2a Needs you"],
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: ago(14),
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    // Three decisions waiting: the chip reads `3 waiting` with the ring dot, which is what the
    // frame draws even though a transfer is also in flight (DEVIATIONS.md §44).
    conflicts: [{ path: "notes/todo.txt" }, { path: "docs/spec.md" }, { path: "a/b.txt" }],
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
