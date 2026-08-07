// Onboarding fixtures (F9) — the five `9a` frames, from `docs/design-v2/09-onboarding.md`.
//
// ONBOARDING IS THE ONE FLOW WHERE THE DAEMON IS NOT THERE, and that is not an edge case to be
// tolerated — it is the state four of these five frames are drawn in. `routes.js`'s
// `nextOnboardingLatch` enters the takeover on exactly `unreachable + statusPolled + configLoaded +
// no folder pair`, which is what a fresh machine reports, so each fixture below pins the status the
// step actually runs against rather than a convenient reachable one. Getting that wrong would make
// every one of these frames unreachable in the preview for the same reason PR #131 had to fix in the
// app: onboarding that waits for a daemon never appears on the machine that needs it.
//
// WHAT THESE FRAMES DRAW THAT NOTHING RETURNS YET. Two of the gaps are Phase-1 cheap work with
// issues already filed, and are pinned in the shape those issues describe:
//
//   · free space — `Needs 38.4 GB free. You have 214 GB.` on `9a Review`. C4 (#177): `statvfs` on
//     the local root, exposed as a Tauri command. Pinned as `freeSpace: { needed, available }`.
//   · the distribution — `Detected Debian` and `sudo apt install proton-drive` on `9a CLI missing`.
//     C5 (#178): read `/etc/os-release`, and show tarball instructions rather than guess if that
//     fails. Pinned as `cli: { installed, distro }`.
//
// The rest has no command and no issue, and is flagged where it is pinned: the per-side folder
// counts and sizes on `9a Folders`, the account line beside them, `9a Review`'s already-matching
// count and per-direction byte totals, its "about 25 minutes", and `9a First sync`'s `44 sent` /
// `115 received` split.
//
// THE 9a SET'S NUMBERS DO NOT RECONCILE ACROSS FRAMES, and no fixture can make them: `9a Folders`
// says 341 local files and 12,139 remote ones, which only sums to `9a Consent`'s 12,480 if the two
// sides overlap in nothing, while `9a Review` says 11,798 files already match on both sides (and
// then brings down 341 — the local count, on the remote side). Each fixture reproduces ITS frame;
// they are not a single scenario, and treating them as one would mean picking which frame to break.

import { ago } from "./clock.js";

/**
 * A fresh machine: no daemon, so no `response` at all. `StatusPayload` omits `response` on failure
 * (`skip_serializing_if`) and carries `error` instead — the shape the UI must render as em-dashes
 * and never as zeroes. The message is `IpcError::Unreachable`'s own text for a missing socket.
 */
const NO_DAEMON = {
  state: "unreachable",
  error:
    "daemon unreachable: connect /run/user/1000/proton-sync.sock: No such file or directory (os error 2)",
};

/**
 * `read_config` before anything has been chosen. `ConfigDoc::load` turns a missing file into an
 * empty document, so this is a successful read of nothing — which is precisely what the latch needs
 * to distinguish "not set up" from "not polled yet".
 */
const NO_CONFIG = {
  path: "~/.config/proton-sync/proton-sync.toml",
  exists: false,
  toml: "",
  local_root: null,
  remote_root: null,
  scan_interval_secs: null,
  events_driven: null,
  include: [],
  exclude: [],
  proton_cli: null,
  proton_timeout_secs: null,
  proton_list_attempts: null,
  delete_approval_remote: null,
  delete_approval_local: null,
};

/**
 * The pair once step 1 has written it — the same two roots every other screen's fixtures use.
 *
 * Everything else stays absent on purpose. Onboarding writes the two roots and nothing more, so a
 * real `read_config` here returns `null` for `events_driven`, `proton_cli`, `scan_interval_secs` and
 * the approval pair: the keys are not in the file, and the daemon runs on its own defaults for them.
 * A fixture that filled them in would describe a config no step of this flow ever wrote.
 */
const CHOSEN_CONFIG = {
  ...NO_CONFIG,
  exists: true,
  toml: `local_root = "~/ProtonDrive"\nremote_root = "/Drive/RemoteFolder"\n`,
  local_root: "~/ProtonDrive",
  remote_root: "/Drive/RemoteFolder",
};

export const ONBOARDING_FIXTURES = {
  /**
   * Step 1. The header draws `step 1 of 2` in place of a status chip (`CHROME.chips.step`), which is
   * the one place the shell's chip is not daemon-derived — so the unreachable status behind it is
   * invisible here, and still has to be right for the takeover to be showing at all.
   *
   * `folders` IS NOT A COMMAND REPLY AND NO ISSUE ASKS FOR IT. Every value in it is drawn:
   *   · the two paths are PROPOSALS, not config — `config` is still empty, which is why this step is
   *     showing. The cards offer `~/ProtonDrive` and `/Drive/RemoteFolder` for the user to accept.
   *   · the counts and sizes are the point of the step ("it's how someone notices they picked the
   *     wrong folder", 09-onboarding.md), and nothing returns them: the remote side would have to
   *     come from parsing `list_remote`'s JSON, the local side from a scan the GUI does not do.
   *   · the account line has no source whatsoever — there is no account command, and the daemon
   *     never sees an email address or a quota.
   *
   * Sizes are pinned as byte counts, not strings, so `format.bytes` renders them: 2_100_000_000 is
   * `2.1 GB`, 39_100_000_000 is `39.1 GB`.
   */
  "9a Folders": {
    status: NO_DAEMON,
    config: NO_CONFIG,
    folders: {
      local: { path: "~/ProtonDrive", files: 341, bytes: 2_100_000_000 },
      remote: { path: "/Drive/RemoteFolder", files: 12_139, bytes: 39_100_000_000 },
      // `ONBOARDING.signedIn(email, used, total)` — and see the note on `9a Review`'s `freeSpace`
      // about what `format.bytes` does to a whole number of GB.
      account: { email: "you@proton.me", used: 39_100_000_000, total: 500_000_000_000 },
    },
    ui: { step: "folders" },
  },

  /**
   * Step 2 — the plan preview, and the one frame here with a real command behind its main content.
   *
   * `dryRun` is `run_dry_run`'s `DryRunPayload` verbatim. It works with no daemon because the command
   * shells `proton-syncd --dry-run` itself — but it does need the config file, which is why this step
   * carries `CHOSEN_CONFIG` while step 1 carries none: step 1 must write the pair before this screen
   * can compute anything.
   *
   * `plan` IS DELIBERATELY EMPTY. The frame draws no action rows — only `See all 471 actions`, whose
   * count is `summary.total`. A fixture cannot carry 471 rows, and carrying four "representative"
   * ones would be worse: a screen reading `plan.length` would get a plausible-looking number instead
   * of an obviously wrong one. The rows belong to the screen `See all 471 actions` opens.
   *
   * `requires_delete_gate: false` and `files_at_risk: []` are the payload restating the headline:
   * nothing gets deleted today.
   */
  "9a Review": {
    status: NO_DAEMON,
    config: CHOSEN_CONFIG,
    dryRun: {
      report: {
        // 128 up + 341 down + 2 conflicts = 471, the total the button names.
        summary: {
          total: 471,
          uploads: 128,
          downloads: 341,
          remote_directories_created: 0,
          local_directories_created: 0,
          local_moves: 0,
          remote_moves: 0,
          auto_links: 0,
          conflicts: 2,
          type_conflicts: 0,
          remote_deletes: 0,
          local_deletes: 0,
          purges: 0,
          skipped_unsupported: 3,
          destructive_actions: 0,
        },
        plan: [],
      },
      requires_delete_gate: false,
      files_at_risk: [],
    },
    // C4 (#177): `statvfs` on the local root. The download side is the only one that can fail on
    // space, which is why it and not the upload side states it.
    //
    // NOTE FOR WHOEVER RENDERS THIS: `format.bytes` gives `214.0 GB`, and the frame draws `214 GB`.
    // Every whole number of GB has the same problem (`500 GB` in `9a Folders`' account line). The
    // numbers here are the true ones; the trailing `.0` is a defect in the formatter's one-decimal
    // rule, not something to dodge by writing the string.
    freeSpace: { needed: 38_400_000_000, available: 214_000_000_000 },
    // Three drawn numbers with no source at all. `alreadyMatching` counts files the plan does NOT
    // act on, so it is absent from `PlanSummary` by construction; the two byte totals are absent
    // because `PlannedAction` carries no size. G2 (#191) is the nearest filed work — per-direction
    // byte totals — but it is scoped to what a sync pass moved, not to what a plan would move.
    planTotals: { alreadyMatching: 11_798, upBytes: 1_400_000_000, downBytes: 38_400_000_000 },
    // `worked out 40 seconds ago · about 25 minutes to finish` (`ONBOARDING.workedOut`). The first
    // half is relative, so it is an offset per the clock convention; the second is an estimate no
    // command produces — `run_dry_run` reports what would happen, never how long it would take.
    planTiming: { workedOutEpochSecs: ago(40), etaSecs: 1500 },
    ui: { step: "review" },
  },

  /**
   * The merge in flight (602×542 — the drawn size, not the spec's 600×540; DEVIATIONS §48).
   *
   * This is the first frame in the flow with a daemon: `Start the first sync` started the service, so
   * the status is a real reply with a real `SyncActivity`. `action_index`/`action_total` are genuine
   * fields, and `159 of 471 done` is exactly what they hold — the only part of this screen the
   * command surface already answers.
   *
   * `pending_changes: 312` is the hexagon's numeral. It is `action_total - action_index` on purpose:
   * the files still to move, consistent with the activity beside it, so a screen may take the numeral
   * from either and cannot be caught out by them disagreeing.
   */
  "9a First sync": {
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        reconcile_seq: 1,
        pending_changes: 312,
        message: "first sync in progress",
        last_sync_epoch_secs: null,
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
        activity: {
          phase: "executing",
          // No file name is drawn on this screen — the transfer detail belongs to `2a Syncing`'s
          // rows, and inventing one here would put a string on screen the frame does not have.
          detail: null,
          folders_listed: null,
          files_scanned: null,
          action_index: 159,
          action_total: 471,
          transfer: null,
          since_epoch_secs: ago(480),
        },
      },
    },
    config: CHOSEN_CONFIG,
    // The split bar and its two labels. NO COMMAND REPORTS PER-DIRECTION PROGRESS WITHIN A PASS:
    // `SyncActivity` counts actions, not directions, and G2 (#191) covers byte totals per direction
    // for a finished window rather than a running pass. So all four are pinned.
    //
    // `up`/`down` are the two fills as fractions — 48px and 88px of the 400px track — and not
    // 44/471 and 115/471, which would be 9% and 24%. The frame's bar is drawn, not computed, and the
    // fixture reproduces the frame.
    progress: { sent: 44, received: 115, up: 0.12, down: 0.22, remainingSecs: 1020 },
    ui: { step: "firstSync" },
  },

  /**
   * The consent dialog, after the merge.
   *
   * `paused: true` is load-bearing, not decoration: `Syncing stays paused until you agree.` is a
   * claim about the daemon, and 09-onboarding.md is explicit that continuous sync does not begin
   * until the box is checked. ONE CONSEQUENCE S7 HAS TO ANSWER: a paused daemon derives to
   * `DaemonState::Paused`, and `nextOnboardingLatch` releases the takeover on any reachable state —
   * so this dialog must float over the main screen, or onboarding needs a latch the daemon state
   * cannot release.
   *
   * The two conflicts the copy mentions are carried as a real `scan_conflicts` reply rather than
   * left implicit in the sentence, so the number the dialog states is the number the app can count.
   */
  "9a Consent": {
    status: {
      state: "paused",
      response: {
        status: "paused",
        paused: true,
        syncing: false,
        reconcile_seq: 1,
        pending_changes: 0,
        message: "paused",
        last_sync_epoch_secs: ago(60),
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
    },
    config: CHOSEN_CONFIG,
    conflicts: [
      { original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt" },
      { original: "docs/spec.md", sidecar: "docs/spec.proton-cloud.md" },
    ],
    // `ONBOARDING.doneSub(files, size)` — the totals of the merged pair. Same unsourced pair of
    // numbers the Settings screen draws (`12,480 files, 41.2 GB`); no command returns them.
    mergedTotals: { files: 12_480, bytes: 41_200_000_000 },
    // The box is drawn unchecked and `Start syncing` disabled (`#2A2E36`/`#6D7783`), which is the
    // state the whole dialog exists to hold. The checkbox is the only one in all 51 frames
    // (DEVIATIONS §55).
    ui: { step: "consent", agreed: false },
  },

  /**
   * The precondition that only ever appears when it fails.
   *
   * C5 (#178) supplies `distro` from `/etc/os-release`, and with it the right install command. Note
   * what that lands on: the deck hard-codes Debian in BOTH strings it needs — `Detected Debian` sits
   * inside `ONBOARDING.cliMissingBody`, and `ONBOARDING.cliInstallCommand` is `sudo apt install
   * proton-drive` — so a detected distribution has nowhere to go until those two entries take the
   * distribution as an argument. The fixture pins the detection result; the strings stay the deck's.
   *
   * There is no daemon and no config here: the CLI check runs before either exists, which is why
   * this dialog can precede step 1.
   */
  "9a CLI missing": {
    status: NO_DAEMON,
    config: NO_CONFIG,
    cli: { installed: false, distro: "debian" },
    ui: { step: "cliMissing" },
  },
};
