// Activity fixtures (F9) — the four `7a` frames plus the two `6a` panels that belong to the same
// screen (S5 owns all six; `6a Activity passes` is the Sync-passes tab and `6a Details` is the
// panel every footer *Details* link opens).
//
// This is the screen where the design asks for the most that the daemon does not yet say, so the
// shape of every entry here is decided by one question: **does a Phase-1 command return this?** The
// twenty commands in `gui/src-tauri/src/commands.rs` are the whole surface, and where the frame
// draws something none of them produces, the fixture leaves it out and says so rather than
// inventing a field shape that would pre-empt the design of the thing that fills the gap.
//
// Six such omissions live in this file. Each is commented at the frame that draws it; collected
// here so the size of it is visible in one place:
//
//   1. per-pass DURATION — `6a Activity passes`, the twenty chart bars
//   2. whole-index totals — `7a Activity quiet`, `12,480 files · 41.2 GB` on both seam sides
//      (now filed as G7, #207; carried as `localTotals`, since a scalar pre-empts no design)
//   3. per-file recent activity — `7a Activity quiet`, the `Last things to move` rows
//   4. never-synced enumeration — `7a Never synced`. HALF-CLOSED by C2: `skip_rule_usage` walks
//      the local tree and reports what each exclude rule matches, with sizes, so the two
//      rule-matched rows and the band's count are live now. The other two — a socket and a
//      symlink — remain out of reach for a different reason (see the frame's own note)
//   5. per-path history — `7a File lookup`, the `This file's history` block (G1, #190)
//   6. upload bytes-so-far — `7a File pending`, the progress bar
//
// Only (5) is one of the recorded gaps G1–G4.
//
// THE CLOCK, because this screen draws more absolute times than any other. `clock.js` states the one
// rule: a value the app renders as a DURATION is pinned as an epoch offset (`ago(120)` is always
// "2 minutes ago"), and a value the frame draws as a CLOCK TIME is written literally, because an
// epoch formatted as `14:32` depends on the machine's timezone and changes across midnight. Both
// registers describe the same events here, so both are pinned: the wire's own epoch field carries
// the offset, and `ui.clock` carries the literals beside it. `ui.clock` holds values only — never a
// sentence — so the deck keeps owning the words around them.

import { ACTIVITY } from "../ui/copy.js";
import { ago } from "./clock.js";
import { activityFids } from "./fids.js";
// `summary` here is the counters-only form: these are `status_history` entries, whose summaries
// describe plans the daemon has already discarded, so there is no plan to derive one from. `total`
// and `destructive_actions` are still derived — they are the two a hand-written summary gets wrong.
import { summaryFromCounts as summary } from "./dryrun.js";

/**
 * The daemon behind the five idle frames. See `plan.js`'s twin for the two traps in this shape:
 * `response.status` is the daemon's own word (`running` / `paused` / `syncing`, never `idle` — the
 * derived state lives in `state`), and `last_sync_epoch_secs` must be present or `derive_state`
 * reports `FirstRun` and the shell renders onboarding over the screen.
 *
 * `last_sync_epoch_secs: ago(120)` is the 14:32 pass that four of these frames refer to — as
 * `2 minutes ago` in `7a Activity quiet`'s sub-line, as `checked 2m ago` in its seam, and as
 * `most recent 14:32` in the passes chart. One event, three renders, one pinned number.
 */
function idleDaemon(response = {}) {
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
      ...response,
    },
  };
}

/**
 * A `read_config` reply. Defaults are the daemon's own (300s scan interval, events-driven on, the
 * two delete-approval guards on), so a frame that overrides one is visibly saying something.
 *
 * Note this is NOT `status.response.config`, which is `RunningConfigInfo` — three paths describing
 * the *live* daemon. Both appear in this file and they are different objects for different
 * questions: what the file says, versus what the process is actually running.
 */
function fileConfig(overrides = {}) {
  return {
    path: "~/.config/proton-sync/proton-sync.toml",
    exists: true,
    toml: 'local_root = "~/ProtonDrive"\nremote_root = "/Drive/RemoteFolder"\n',
    local_root: "~/ProtonDrive",
    remote_root: "/Drive/RemoteFolder",
    scan_interval_secs: 300,
    events_driven: true,
    include: [],
    exclude: [],
    proton_cli: "proton-drive",
    proton_timeout_secs: 60,
    proton_list_attempts: 3,
    delete_approval_remote: true,
    delete_approval_local: true,
    ...overrides,
  };
}

/**
 * One `StatusHistoryEntry`, as `record_status_history` writes it.
 *
 * `message` is the daemon's: `sync completed`, `sync failed`, or (since #136) `sync completed with
 * N failed item(s)` — the row's `Finished cleanly` / `Couldn't reach Proton Drive` are the deck's
 * words, chosen by S5 from `last_error` being absent or present. A partial pass sets `last_error`
 * too, so it draws as a failed row until S5 gains a state for it. A fixture that put the drawn
 * sentence in `message` would be feeding the screen its own answer.
 */
function pass(epochSecs, { error = null, planned = null, done = null } = {}) {
  return {
    epoch_secs: epochSecs,
    message: error ? "sync failed" : "sync completed",
    last_error: error,
    plan_summary: planned,
    successful_sync_summary: done,
  };
}

export const ACTIVITY_FIXTURES = {
  // ---- 6a Activity passes — the Sync-passes tab -----------------------------------------------
  //
  // Six passes over two hours, most recent 14:32. `status_history` is written OLDEST FIRST because
  // that is how the daemon pushes it (`record_status_history` appends and drains from the front);
  // the frame lists it newest first, which is the screen's reversal to make, not the fixture's.
  //
  // Each row's detail line — `2 sent, 1 brought here · 1 conflict kept`, `4 brought here · 1 move
  // followed`, `1 sent`, `nothing to do` — is composed by S5 from the counters, so the counters are
  // what is pinned. `successful_sync_summary` is the right one to read: it is what the pass DID,
  // where `plan_summary` is what it set out to do. On a successful pass the daemon sets both to the
  // same summary (src/daemon.rs assigns `last_plan_summary` when planning and
  // `last_successful_sync_summary` on completion), so both are carried here to match.
  //
  // TWO TRAPS IN THE FAILED PASS, both faithfully reproduced rather than tidied away:
  //
  //   · `last_successful_sync_summary` is a CARRY-OVER field, not this pass's result. The daemon
  //     never clears it, so the 13:58 failure ships the 13:12 pass's summary — which is why the row
  //     must key its outcome on `last_error`, and why a screen that keyed on "has a summary" would
  //     quietly caption a failure with an earlier pass's counts.
  //   · the error string is carried verbatim (voice rule 4 — never paraphrase a daemon error), and
  //     is imported from the deck rather than retyped because the copy gate proves that constant
  //     against the frame. It is the one string in this file that is DATA rather than words the
  //     screen chooses: it stands in for whatever the daemon really said.
  //
  // `nothing to do` is an all-zero summary and emphatically not `null`: `format.js`'s `dash()`
  // doctrine makes a null summary mean "not measured", and this pass measured zero.
  //
  // WHAT NO COMMAND RETURNS. The duration chart — twenty bars whose heights are, as drawn,
  // 38/54/31/72/44/26/61/35/100/48/29/66/41/33/57/24/69/37/52/45 percent of the 56px track, with
  // the failure at 100% in `#FF6B6B` and the most recent in `#E8EBF0` — needs how long each pass
  // took. `StatusHistoryEntry` records `epoch_secs`, `message`, `last_error` and two summaries, and
  // no duration; nothing else on the wire carries one either. So there is no `duration_ms` here and
  // there are six entries rather than twenty: twenty would look like data for a chart that still
  // could not be drawn. The contract routes this frame to G2 (per-direction byte totals, #191), but
  // the frame draws counts, not bytes — what it actually lacks is a per-pass duration, which G2 as
  // written in IMPLEMENTATION-PLAN.md (`2a Syncing` footer, `7a` seam counts) does not cover.
  "6a Activity passes": {
    fids: activityFids("passes"),
    status: idleDaemon({
      last_plan_summary: summary({ uploads: 2, downloads: 1, conflicts: 1 }),
      last_successful_sync_summary: summary({ uploads: 2, downloads: 1, conflicts: 1 }),
      status_history: [
        // 12:30 — a pass with nothing to do.
        pass(ago(7440), { planned: summary({}), done: summary({}) }),
        // 12:45 — `1 sent`.
        pass(ago(6540), {
          planned: summary({ uploads: 1 }),
          done: summary({ uploads: 1 }),
        }),
        // 13:12 — `4 brought here · 1 move followed`.
        pass(ago(4920), {
          planned: summary({ downloads: 4, local_moves: 1 }),
          done: summary({ downloads: 4, local_moves: 1 }),
        }),
        // 13:58 — the failure. `plan_summary` is what this pass had planned before it could not
        // reach Proton; `successful_sync_summary` is 13:12's, carried over (see above).
        pass(ago(2160), {
          error: ACTIVITY.passes.exampleDaemonError,
          planned: summary({ uploads: 2 }),
          done: summary({ downloads: 4, local_moves: 1 }),
        }),
        // 14:17 — `2 sent`. This is the pass the failed row points at with `retried at 14:17 and
        // worked`: the retry is not a field, it is the next successful pass, and its clock time is
        // already in `ui.clock.passes`.
        pass(ago(1020), {
          planned: summary({ uploads: 2 }),
          done: summary({ uploads: 2 }),
        }),
        // 14:32 — `2 sent, 1 brought here · 1 conflict kept`, and the daemon's last word.
        pass(ago(120), {
          planned: summary({ uploads: 2, downloads: 1, conflicts: 1 }),
          done: summary({ uploads: 2, downloads: 1, conflicts: 1 }),
        }),
      ],
    }),
    route: "activity",
    ui: {
      tab: "passes",
      clock: {
        // Same order as `status_history`, oldest first, so the two zip.
        passes: ["12:30", "12:45", "13:12", "13:58", "14:17", "14:32"],
        // `how long each took · 12:45 onward` — where the twenty-pass window starts. Note the frame
        // contradicts itself here: it lists a 12:30 row above a chart that claims to begin at
        // 12:45. Reproduced as drawn; flagged rather than silently corrected.
        chartFrom: "12:45",
      },
    },
  },

  // ---- 6a Details — where the daemon's own words live ------------------------------------------
  //
  // A 522×462 dialog of eight `key · value` rows, and every one of them already exists:
  //
  //   pending_changes ← response.pending_changes
  //   conflicts, destructive_actions, skipped_unsupported ← response.last_plan_summary.*
  //       (these three live INSIDE the nullable summary, not at the top level of the reply — the
  //        one shape correction gui-core's crate docs call out by name)
  //   scan_interval, event_stream ← read_config's scan_interval_secs / events_driven
  //   source ← which ledger the screen read; `status_history` means the socket's own list rather
  //       than the `<db>.status.json` sidecar, so it is a fact about this round trip
  //   socket ← `connected`, which is the round trip having succeeded at all (an unreachable daemon
  //       gives a StatusPayload with `error` set and no `response`)
  //
  // The last two are therefore derived from the fixture rather than pinned in it: a fixture that
  // stated `socket: "connected"` beside a reply that plainly connected would be asserting its own
  // premise. One history entry is carried so `source · status_history` is true of this dataset.
  "6a Details": {
    fids: activityFids("details"),
    status: idleDaemon({
      pending_changes: 0,
      // `total` is `plan.len()` — the sum of every counter, not a subset of them
      // (`PlanSummary::from_plan`) — and `destructive_actions` is
      // `remote_deletes + local_deletes + purges`. Neither is written here: `summaryFromCounts`
      // derives both and refuses to accept either, which is stricter than stating them and is what
      // this panel needs, since a wrong one HERE would be read as the truth about a sync.
      last_plan_summary: summary({
        uploads: 2,
        downloads: 1,
        conflicts: 3,
        remote_deletes: 1,
        skipped_unsupported: 1,
      }),
      status_history: [pass(ago(120), { done: summary({ uploads: 2, downloads: 1, conflicts: 1 }) })],
    }),
    config: fileConfig(),
    // A DIALOG FLOATS OVER A BODY, and naming the route is what decides which one. Without it
    // `activeRoute()` falls through to `main`, so the main screen renders underneath and stamps its
    // own `data-fid`s — which is harmless only because every slot in this dialog's map is prefixed.
    // Naming the route makes the thing behind the scrim the screen this dialog is actually opened
    // from, which is what the design says it is.
    route: "activity",
    ui: { tab: "files", dialog: "details" },
  },

  // ---- 7a Activity quiet — the default, and the hardest frame to source -------------------------
  //
  // The daemon half is straightforward: idle, last pass 14:32 (`ago(120)`), which the title renders
  // as `2 minutes ago` and the seam as `checked 2m ago` — the same epoch in the deck's long and
  // short registers.
  //
  // `next full check in 4m` IS derivable and so is derived: the countdown is a relative render, and
  // `scan_interval_secs` minus the age of the last check is what it counts down. Three notes on
  // that number, because it is the least obvious value in this file.
  //
  // It is not the 300s default: 300 with a check 120s old leaves 180s and would draw `3m`, and the
  // frame draws `4m` beside a check it calls `2 minutes ago`. `6a Details` in this same file draws
  // `scan_interval 300s`, so the two frames disagree about the interval and each fixture reproduces
  // its own rather than averaging them.
  //
  // It is 370 rather than the round 360 to keep the answer off a bucket boundary. 360 leaves
  // *exactly* 240s at import, so a screen that floors reads `4m` at load and `3m` one second later;
  // 370 leaves 250s, which floors and rounds to `4m` with ten seconds of slack either way. The
  // page's poll re-renders this every two seconds, so the slack is not theoretical.
  //
  // And the "full check" it counts down to is really G4's absent `full_scan_schedule` (#193);
  // `scan_interval` is the Phase-1 stand-in, not the thing the design means.
  //
  // WHAT NO COMMAND RETURNS — three of this file's six omissions, all on this one frame:
  //
  //   · `12,480` and `41.2 GB`, twice. Nothing returns index-wide totals: `index_read.rs` exposes
  //     `record_for_path` and `path_for_id`, both single-path, and no command wraps anything else.
  //     The byte half is G2 (#191); the file-count half is not covered by any recorded gap.
  //   · `4 files are never synced`. See `7a Never synced` below for why the count is as unavailable
  //     as the list.
  //   · `Last things to move` — three rows of path + direction + when, and `7 files in the last 3
  //     days`. `status_history` is per-PASS and carries no paths; `path_sync_status` answers about
  //     one path you already know. Nothing lists recently-moved files. This is adjacent to G1
  //     (#190) but distinct: G1 is one file's history, this is the tail of every file's.
  //
  // So this entry carries the status, the config, and the tab — and the screen it drives is honest
  // about being mostly Phase 2.
  "7a Activity quiet": {
    fids: activityFids("quiet"),
    status: idleDaemon({
      status_history: [pass(ago(120), { done: summary({ uploads: 2, downloads: 1, conflicts: 1 }) })],
    }),
    // The same machine as `7a Never synced`, which this screen's `Show them` opens: one skip rule,
    // `*.tmp`. A screen and the dialog it links to must not describe two different configurations.
    config: fileConfig({ scan_interval_secs: 370, exclude: ["*.tmp"] }),
    // G7 (#207). Same key and same numbers as `8a Settings` and `5a Checking` — the three frames
    // that draw this count were describing one missing capability in three different ways until it
    // was filed, and a fixture carrying it on one frame and not the others was that inconsistency
    // made durable.
    localTotals: { files: 12480, bytes: 41_200_000_000 },
    // WHAT C2 CHANGED. This block was recorded here as unbuildable — "the command does not exist
    // yet … counting them means walking the filesystem, not reading the index" — and that is
    // precisely what `skip_rule_usage` now does. So the never-synced band and the dialog's first
    // group are live data, and this is the report the command returns for this machine.
    //
    // `unique_files`, not `files`: a path matched by two rules is ONE file that is never synced.
    skipRules: {
      rules: [
        {
          pattern: "*.tmp",
          files: 2,
          bytes: 2_940_000,
          unique_files: 2,
          unique_bytes: 2_940_000,
          samples: [
            { path: "exports/draft.tmp", bytes: 2_100_000 },
            { path: "exports/render-final.tmp", bytes: 840_000 },
          ],
          folder_exists: null,
          error: null,
        },
      ],
      // DISTINCT FILES HIDDEN BY AT LEAST ONE RULE — the union, which is what the band counts.
      // Not the size of the tree: that is `considered_files`, "everything the daemon would sync if
      // there were no rules at all", and the two were transposed here at first.
      total_files: 2,
      total_bytes: 2_940_000,
      considered_files: 12_482,
      unreadable_directories: 0,
      unreadable_entries: 0,
    },
    route: "activity",
    ui: {
      tab: "files",
      // `Nothing has needed to move since 14:32.` — the absolute half of `ago(120)` above.
      clock: { since: "14:32" },
    },
  },

  // ---- 7a File lookup — one file, answered --------------------------------------------------
  //
  // `path_sync_status` is a real command and this is exactly its reply: `EmblemStatus`
  // (`tracked`, `sync_status`, `entity_kind`, `file_size`, `mtime`, `proton_id`) read out of the
  // index for one relative path. It answers the verdict (`sync_status: "synced"` → `Safely on both
  // sides`) and both side cards' sizes, which is precisely what IMPLEMENTATION-PLAN.md says Phase 1
  // ships here.
  //
  // `1 match` is not pinned: the search returned this one file, and `ACTIVITY.matches(n)` counts
  // what the screen has.
  //
  // The two absolute paths — `~/ProtonDrive/docs/spec.md` and `/Drive/RemoteFolder/docs/spec.md` —
  // are the roots in `status.response.config` joined onto the queried path, so they are composed by
  // the screen from pinned values rather than written out as strings.
  //
  // `linked · id 4c8f…9a21` elides the composed `volumeId~nodeId` this record actually stores; the
  // frame shows the first four and last four characters of the NODE half. Which four is S5's
  // eliding rule to write — the fixture's job is to carry a real id that produces those eight.
  //
  // WHAT IS LEFT OUT. `This file's history` — four rows of `glyph · sentence · when`, from
  // `Sent to Proton Drive` (today 14:32) back to `First brought to this computer` (12 Jul) — is
  // engine gap G1 (#190), a per-path history query that does not exist. `path_sync_status` returns
  // the CURRENT state of one record and keeps no past, so there is nothing here to shape, and
  // 07-activity.md says so itself: "If it can only answer 'current state', ship the verdict and the
  // two side cards and omit the history block."
  //
  // One thing the two side cards need that even Phase 1 cannot give: `received 14:32` on the Proton
  // card. `EmblemStatus.mtime` is the LOCAL modification time and there is no remote-side timestamp
  // in the reply at all. Both clock literals are pinned below (rule 3), but only the local one has
  // a field behind it.
  "7a File lookup": {
    fids: activityFids("lookup"),
    status: idleDaemon(),
    // KEYED BY PATH, because `path_sync_status` takes one. A flat `EmblemStatus` here reads as the
    // answer to every question, and `api.js` serves this key by looking the asked-for path up — so a
    // flat one resolved to `undefined` and the frame drew an UNTRACKED file: no verdict, no size, no
    // id, which is very nearly the opposite of what it says. `deletions.js` carries the same shape
    // for the same reason.
    pathStatus: {
      "docs/spec.md": {
        tracked: true,
        sync_status: "synced",
        entity_kind: "file",
        file_size: 1_200_000, // `1.2 MB` through format.js's decimal `bytes()`
        // 14:31, a minute before the 14:32 pass that agreed the two sides.
        mtime: ago(180),
        proton_id: "8b3c1f2a~4c8f2e7d10b64f2ca39c5e0b8d7f9a21",
      },
    },
    route: "activity",
    ui: {
      tab: "files",
      query: "spec.md",
      path: "docs/spec.md",
      // What the screen actually renders from — `path_sync_status`'s reply for the typed path,
      // held beside the query the way the live screen holds it. `pathStatus` above is the command's
      // whole keyed table; this is the one answer the frame is showing.
      lookup: {
        path: "docs/spec.md",
        status: {
          tracked: true,
          sync_status: "synced",
          entity_kind: "file",
          file_size: 1_200_000,
          get mtime() {
            return ago(180);
          },
          proton_id: "8b3c1f2a~4c8f2e7d10b64f2ca39c5e0b8d7f9a21",
        },
      },
      clock: { edited: "14:31", received: "14:32", agreed: "14:32" },
    },
  },

  // ---- 7a File pending — the same lookup, mid-flight ------------------------------------------
  //
  // A 600×239 dialog for a file that has no answer yet, and the only frame in this file driven by a
  // SYNCING daemon. `SyncActivity` is the live "what is it doing right now" surface, and its
  // `transfer` block is exactly this frame: direction, path, total size, and when it started.
  // `Started 8 seconds ago` is `since(started_epoch_secs)`, so it is pinned as an offset.
  //
  // `bytes_done: null` IS THE POINT, not an omission of convenience. The wire's own documentation
  // (`TransferActivity` in src/ipc.rs) says bytes-so-far is observable for downloads only — it is
  // sampled from the download staging directory — while an upload is opaque until the CLI child
  // exits. `bytes_total` is genuinely known for an upload (the local file's size), so the frame's
  // `2.8 MB` is sourced and the progress bar it draws at 41% (223.86px of a 546px track) is not.
  // A determinate bar for an upload needs a daemon that can measure one; until then the honest
  // render is indeterminate. Not covered by G1–G4.
  //
  // `only on this computer so far` needs no data — it is what "an upload is in flight" means.
  "7a File pending": {
    fids: activityFids("filePending"),
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        reconcile_seq: 41,
        pending_changes: 3,
        message: "daemon status",
        last_sync_epoch_secs: ago(900),
        last_error: null,
        // The pass in flight planned five actions; the last one that FINISHED was the 15-minutes-ago
        // pass, whose summary is what `last_successful_sync_summary` still holds. The two disagree
        // mid-pass by design — that is what makes the carry-over field readable as "last good".
        last_plan_summary: summary({ uploads: 3, downloads: 2 }),
        last_successful_sync_summary: summary({ uploads: 2 }),
        status_history: [pass(ago(900), { done: summary({ uploads: 2 }) })],
        pending_deletions: [],
        config: {
          local_root: "~/ProtonDrive",
          remote_root: "/Drive/RemoteFolder",
          db_path: "~/ProtonDrive/.sync/sync_index.db",
        },
        activity: {
          phase: "executing",
          detail: "photos/trip/img_0042.jpg",
          folders_listed: null,
          files_scanned: null,
          action_index: 3,
          action_total: 5,
          transfer: {
            direction: "upload",
            path: "photos/trip/img_0042.jpg",
            bytes_total: 2_800_000, // `2.8 MB`
            bytes_done: null, // downloads only — see above
            // A getter for the same reason `5a Plan safe`'s `checkedAt` is one: `Started 8 seconds
            // ago` is a SECOND-resolution render, and a fixture module is imported once. Frozen at
            // import, the line reads `10 seconds ago` by the time the harness extracts and keeps
            // climbing on every 2s poll; read as a property, `ago(8)` is 8 at the moment it renders.
            // Minute-resolution offsets elsewhere in this file need no such care.
            get started_epoch_secs() {
              return ago(8);
            },
          },
          get since_epoch_secs() {
            return ago(8);
          },
        },
      },
    },
    // A DIALOG FLOATS OVER A BODY, and naming the route is what decides which one. Without it
    // `activeRoute()` falls through to `main`, so the main screen renders underneath and stamps its
    // own `data-fid`s — which is harmless only because every slot in this dialog's map is prefixed.
    // Naming the route makes the thing behind the scrim the screen this dialog is actually opened
    // from, which is what the design says it is.
    route: "activity",
    ui: {
      tab: "files",
      // routes.js NOW has an id for this one — S5 added `filePending`, content-sized at [600, null]
      // with no `dialogHead`, which is what this frame draws.
      dialog: "filePending",
      path: "photos/trip/img_0042.jpg",
    },
  },

  // ---- 7a Never synced — the dialog with almost nothing behind it ------------------------------
  //
  // A 602×602 dialog grouped by WHY, and the grouping is the only part with a source: the rule
  // `*.tmp` is an entry in `read_config`'s `exclude` array, which is a real reply from a real
  // command, so it is pinned.
  //
  // Everything else is unavailable, and for two different reasons worth keeping apart:
  //
  //   · `exports/draft.tmp · 2.1 MB` and `exports/render-final.tmp · 840 KB` — the files a rule is
  //     hiding. SOURCED SINCE C2, and the paragraph this replaces is why it took a command of its
  //     own: IMPLEMENTATION-PLAN.md's capability #2 said "match each exclude glob against the
  //     index", and the engine's selective-sync invariant filters excluded paths out of the local
  //     scan, the remote listing AND the base index — so an excluded file is never IN the index to
  //     be matched. The note ended "counting them means walking the filesystem, not reading the
  //     index", and `skip_rule_usage` is that walk. `samples` carries a size per file because this
  //     dialog draws `path · size` rows and the walk already stats every file it counts.
  //   · `.cache/session.sock · a socket` and `projects/current → ~/work/q3 · a shortcut` — the
  //     files nothing could sync. These are further out of reach still: `scan_local_files_with_
  //     options` keeps only entries where `file_type.is_file()`, so a socket or a symlink never
  //     enters the index in the first place, and `SkipUnsupported` (the one action that sounds like
  //     this) is about a REMOTE file the CLI cannot download as bytes — a different fact entirely.
  //
  // So the fixture carries the rule and not the rows. The same absence is why `7a Activity quiet`
  // cannot draw its `4 files are never synced` band: the count is as unavailable as the list.
  "7a Never synced": {
    fids: activityFids("neverSynced"),
    status: idleDaemon({
      // `skipped_unsupported: 1` is the closest the wire comes to this dialog's subject, and it is
      // not close: it counts REMOTE files the CLI could not fetch, not local files a rule hides.
      // Carried so the number is on the page, not as a stand-in for the four rows.
      last_plan_summary: summary({
        uploads: 2,
        downloads: 1,
        conflicts: 1,
        skipped_unsupported: 1,
      }),
    }),
    config: fileConfig({ exclude: ["*.tmp"] }),
    // WHAT C2 CHANGED. This block was recorded here as unbuildable — "the command does not exist
    // yet … counting them means walking the filesystem, not reading the index" — and that is
    // precisely what `skip_rule_usage` now does. So the never-synced band and the dialog's first
    // group are live data, and this is the report the command returns for this machine.
    //
    // `unique_files`, not `files`: a path matched by two rules is ONE file that is never synced.
    skipRules: {
      rules: [
        {
          pattern: "*.tmp",
          files: 2,
          bytes: 2_940_000,
          unique_files: 2,
          unique_bytes: 2_940_000,
          samples: [
            { path: "exports/draft.tmp", bytes: 2_100_000 },
            { path: "exports/render-final.tmp", bytes: 840_000 },
          ],
          folder_exists: null,
          error: null,
        },
      ],
      // DISTINCT FILES HIDDEN BY AT LEAST ONE RULE — the union, which is what the band counts.
      // Not the size of the tree: that is `considered_files`, "everything the daemon would sync if
      // there were no rules at all", and the two were transposed here at first.
      total_files: 2,
      total_bytes: 2_940_000,
      considered_files: 12_482,
      unreadable_directories: 0,
      unreadable_entries: 0,
    },
    // A DIALOG FLOATS OVER A BODY, and naming the route is what decides which one. Without it
    // `activeRoute()` falls through to `main`, so the main screen renders underneath and stamps its
    // own `data-fid`s — which is harmless only because every slot in this dialog's map is prefixed.
    // Naming the route makes the thing behind the scrim the screen this dialog is actually opened
    // from, which is what the design says it is.
    route: "activity",
    ui: { tab: "files", dialog: "neverSynced" },
  },
};
