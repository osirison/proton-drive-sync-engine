// Plan-a-sync fixtures (F9) — the three `5a` frames, one deterministic dataset each.
//
// The rehearsal screen is the only one in the design driven by a COMMAND RATHER THAN A POLL: S4
// calls `run_dry_run`, which shells `proton-syncd --dry-run` and resolves once with a
// `DryRunPayload` (`{ report, requires_delete_gate, files_at_risk }` — see
// `gui/src-tauri/src/commands.rs`). So each entry here carries two things that answer two different
// questions: a `status` (what the daemon is, which is what the shell around the screen renders) and
// a `dryRun` (what the rehearsal found, which is the screen itself).
//
// `5a Checking` carries no `dryRun` at all, and that absence IS the frame: it is the state between
// the call and its answer.
//
// WHAT IS DELIBERATELY MISSING, and it is the most important thing in this file. `5a Plan` draws
// `3 files, 4.1 MB` leaving and `2 files, 2.6 MB` arriving; `5a Plan safe` draws a size on every
// row (`1.2 MB`, `2.8 MB`, `96 KB`, `2.4 MB`, `184 KB`). **No byte total exists anywhere in the
// dry-run surface.** `PlanSummary` counts actions and `PlannedAction` carries `path`,
// `destination_path`, `action`, `entity_kind`, `conflict_path`, `remote_id` — and no size. That is
// the substance of engine gap G2 (per-direction byte totals, #191), landing on a `5a` frame rather
// than on the `2a`/`7a` frames IMPLEMENTATION-PLAN.md lists for it. Inventing a `bytes` field on a
// planned action here would pre-empt that design, so the counts are pinned and the sizes are not.
//
// `Run it without the deletion` (drawn in `5a Plan`'s footer) is G3 (#192), the filtered apply. It
// is not a data question at all — nothing in `DryRunPayload` says whether a partial apply is
// possible — so there is nothing to leave out; S4 hides the button until the daemon can honour it
// (06-plan.md: "if unavailable, hide the button rather than faking it").

import { ago } from "./clock.js";
import { action, summaryOf } from "./dryrun.js";

/**
 * The daemon behind all three rehearsal frames: reachable, idle, nothing in flight.
 *
 * Shaped exactly as `ControlResponse` reaches the frontend, which is worth two notes because both
 * are easy to get subtly wrong:
 *
 *   · `status` is the DAEMON's own word and is only ever `syncing` / `paused` / `running` (see
 *     `ControlShared::response` in src/daemon.rs). `idle` is the *derived* `DaemonState` and belongs
 *     in `state`, never in `response.status`.
 *   · `last_sync_epoch_secs` must be present, or `derive_state` returns `FirstRun` (a reachable
 *     daemon with no last sync and an empty history is the onboarding signal) and the shell would
 *     render the takeover instead of the screen.
 *
 * None of the three frames draws a relative time from this value — `5a Plan safe`'s
 * `Checked 40 seconds ago` is about the REHEARSAL, not the last sync (see its `ui.checkedAt`) — so
 * the 2-minute offset is just a plausible recent pass.
 */
function idleDaemon() {
  return {
    state: "idle",
    response: {
      status: "running",
      paused: false,
      syncing: false,
      reconcile_seq: 41,
      pending_changes: 0,
      message: "daemon status",
      last_sync_epoch_secs: ago(120),
      last_error: null,
      last_plan_summary: null,
      last_successful_sync_summary: null,
      status_history: [],
      pending_deletions: [],
      config: {
        local_root: "~/ProtonDrive",
        remote_root: "/Drive/RemoteFolder",
        db_path: "~/ProtonDrive/.sync/sync_index.db",
      },
      activity: null,
    },
  };
}

/**
 * The plan `5a Plan` previews: nine actions, one of them destructive.
 *
 * THE ROW ORDER IS THE DRAWN ORDER, and that is not a coincidence to be tidied away.
 * `plan.rs::sorted_for_display` floats display-destructive rows to the top and is otherwise stable,
 * so a plan written in the order the frame draws it comes out of the sort in the order the frame
 * draws it. (The daemon's own emission order would differ; nothing in the screen depends on it.)
 *
 * The distinction `plan.rs` encodes and the design conflated: `requires_delete_gate` keys on
 * `SyncAction::delete_direction()`, so a `purge`-only plan is display-destructive but never gated.
 * This plan has no purge, so the two sets coincide here — which is exactly why the fixture states
 * both explicitly rather than letting a reader infer one from the other.
 */
const DESTRUCTIVE_PLAN = [
  action("archive/old-notes.md", "remote_delete", { remote_id: "8b3c1f2a~e91d4a77b0c5" }),
  action("docs/spec.md", "upload"),
  action("photos/trip/img_0042.jpg", "upload"),
  action("notes/scratch.md", "upload"),
  // The folder the three uploads need. `entity_kind: "directory"` is the engine's own spelling
  // (`EntityKind` serializes snake_case: `file` / `directory`).
  action("photos/trip", "create_remote_directory", { entity_kind: "directory" }),
  action("reports/q3-summary.pdf", "download", { remote_id: "8b3c1f2a~4d70e2f9a118" }),
  action("design/logo.svg", "download", { remote_id: "8b3c1f2a~c02a5b6e7d34" }),
  // A rename that happened on Proton: the local copy moves to match, so the action is `move_local`
  // and `destination_path` carries where it lands. The frame draws both paths in one row
  // (`notes/old.md → notes/archive/old.md`).
  action("notes/old.md", "move_local", {
    destination_path: "notes/archive/old.md",
    remote_id: "8b3c1f2a~f5619ac8d270",
  }),
  // Both sides changed it; the engine keeps both copies and names the sidecar. The `.proton-cloud`
  // spelling is the engine's conflict convention (src/sync.rs), not a choice made here.
  action("notes/todo.txt", "conflict", {
    conflict_path: "notes/todo.proton-cloud.txt",
    remote_id: "8b3c1f2a~2ab7c9e04f61",
  }),
];

/**
 * The same plan with the delete and the conflict gone — seven harmless actions, which is the whole
 * of `5a Plan safe`. It is the control case for the safety logic: `requires_delete_gate: false` with
 * `destructive_actions: 0` proves the gate is not merely "the plan is big".
 *
 * Grouped as drawn: the three uploads and the new folder under `Leaving this computer`, then the two
 * downloads and the move under `Arriving from Proton`. With nothing destructive, `sorted_for_display`
 * is a no-op and this order survives untouched.
 */
const SAFE_PLAN = DESTRUCTIVE_PLAN.filter(
  (row) => row.action !== "remote_delete" && row.action !== "conflict",
);

export const PLAN_FIXTURES = {
  // ---- 5a Checking — the rehearsal in flight -------------------------------------------------
  //
  // A 522×766 window (not a dialog: it keeps the app header and the four footer doors, and is drawn
  // narrow only because a centred empty state does not need 1040px — DEVIATIONS §48a).
  //
  // There is no `dryRun` key because the command has not resolved. That is the whole state.
  //
  // The progress line `8,431 of 12,480 files` is `PLAN.checkingProgress(done, total)`, so the
  // fixture pins two numbers and the app renders the string (rule 2). NEITHER HALF HAS A SOURCE, and
  // they are two separate gaps that happen to meet in one sentence:
  //
  //   · `8,431` — `run_dry_run` is a single async command with NO progress channel: it resolves
  //     once, at the end. The daemon's `SyncActivity` does carry `files_scanned`, but it describes
  //     the daemon's own reconcile, not the GUI's separate `--dry-run` child process. **G9 (#209.)**
  //   · `of 12,480` — the index-wide file count, which nothing reports. **G7 (#207)**, and the same
  //     number `7a Activity quiet` and `8a Settings` draw.
  //
  // Both are pinned rather than omitted, and the distinction is the one the contract draws: neither
  // is a field SHAPE that could pre-empt the design of the thing that fills it. `12,480` is a scalar
  // and will be a scalar whatever #207 lands as, so carrying it settles nothing — where a
  // `{ upBytes, downBytes }` for G2 would have. `activity.js` and `settings.js` carry the same count
  // for the same reason, so the three frames drawing it agree.
  "5a Checking": {
    status: idleDaemon(),
    localTotals: { files: 12_480 },
    ui: { checking: true, scanned: 8431 },
  },

  // ---- 5a Plan — a plan that would destroy something ------------------------------------------
  //
  // Nine actions, one of them a `remote_delete`, which is what arms the typed-DELETE gate. The
  // distinction `plan.rs` encodes and the design conflated: `requires_delete_gate` keys on
  // `SyncAction::delete_direction()`, so a `purge`-only plan is display-destructive but never
  // gated. This plan has no purge, so the two sets happen to coincide here — which is exactly why
  // the fixture says both explicitly rather than letting a reader infer one from the other.
  //
  // THE ROW ORDER IS THE DRAWN ORDER, and that is not a coincidence to be tidied away:
  // `plan.rs::sorted_for_display` floats display-destructive rows to the top and is otherwise
  // stable, so a plan written in the order the frame draws it comes out of the sort in the order
  // the frame draws it. (The daemon's own emission order would differ; nothing in the screen
  // depends on it.)
  "5a Plan": {
    status: idleDaemon(),
    dryRun: {
      report: {
        // Derived, never written. `PlanSummary::from_plan` is what the daemon does, so a summary
        // typed beside its plan is one fact stated twice — and `9a Review` shipped the version where
        // the two disagreed. See dryrun.js.
        summary: summaryOf(DESTRUCTIVE_PLAN),
        plan: DESTRUCTIVE_PLAN,
      },
      requires_delete_gate: true,
      // Only gated rows contribute — the user-data files a destructive apply would remove. A purge
      // would never appear here even though it is tinted like one.
      files_at_risk: ["archive/old-notes.md"],
    },
  },

  // ---- 5a Plan safe — the ordinary plan, and the reason the screen shrinks ---------------------
  //
  // The same seven harmless actions as above with the delete and the conflict gone: nothing is
  // destructive, so there is no band, no gate, and `Run this sync` is simply enabled. It is the
  // control case for the safety logic — `requires_delete_gate: false` with `destructive_actions: 0`
  // proves the gate is not merely "the plan is big".
  "5a Plan safe": {
    status: idleDaemon(),
    dryRun: {
      report: {
        summary: summaryOf(SAFE_PLAN),
        plan: SAFE_PLAN,
      },
      requires_delete_gate: false,
      files_at_risk: [],
    },
    // `Checked 40 seconds ago against both sides.` is `PLAN.checkedAgo(<relative time>)`, and the
    // time it measures is WHEN THE CLIENT RAN THE REHEARSAL — `DryRunPayload` carries no timestamp,
    // because the payload is the answer and not the asking. So it is UI state, pinned as an epoch
    // offset (rule 3: a relative render is an offset, never a literal), and `since()` turns
    // `ago(40)` into `40 seconds ago` on any machine at any hour.
    //
    // A GETTER, and the reason is measurable rather than stylistic. A fixture module is imported
    // once, so a plain `checkedAt: ago(40)` freezes the offset at import and every later render
    // measures from there: the harness reaches extraction a second or two after load (`goto`
    // `networkidle0`, an optional theme reload, `waitForSelector`, then the app's own 2s poll), and
    // the line reads `41 seconds ago`, then `43`. That drift is invisible at minute resolution —
    // `ago(120)` says "2 minutes ago" for a full minute either side — and fatal at second
    // resolution, which is the only register this frame draws. Reading the property re-runs `ago`,
    // so the offset is 40 at the moment it is rendered, whenever that is. Same clock rule, applied
    // where it actually lands.
    ui: {
      get checkedAt() {
        return ago(40);
      },
    },
  },
};
