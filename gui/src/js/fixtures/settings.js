// Settings fixtures (F9) — the five `8a` frames, from `docs/design-v2/08-settings.md`.
//
// FOUR OF THE FIVE ARE ONE SCREEN. `8a Settings`, `8a Skip rules`, `8a Deletions tab` and
// `8a Schedule monthly` are the same Settings window with a different tab showing — the last two are
// even drawn as 600px CROPS of it (`frame-classes.mjs`), not as windows of their own. So they share
// one config dataset and differ only by a `ui` selector, which is the whole point of contract rule 5:
// a fixture that wrote four unrelated configs would be claiming the tabs are four screens, and the
// first thing S6 would have to do is un-claim it. `8a Save refused` is the odd one out — a dialog
// over that window, carrying the daemon's own refusal.
//
// WHAT THE SETTINGS SCREEN DRAWS THAT NO COMMAND RETURNS. `read_config` (commands.rs) is the only
// settings-shaped reply Phase 1 has, and it returns the config file's keys and nothing else. Four
// kinds of drawn value have no source in it:
//
//   · the live match counts on every skip rule — `Skipping 2 files right now`,
//     `hiding 4 files, 3.1 GB in total`, `Matching nothing`. C2 (#175) supplies these, and NOT from
//     the index the issue names: an active exclude rule matches zero index records by construction,
//     because excluded files are filtered out before anything is ever written. `skip_rule_usage`
//     walks the local tree instead (metadata only) — see `gui-core/src/skip_rules.rs` and
//     DEVIATIONS §69. `skipRules` below is that command's reply, verbatim.
//   · the folder totals — `12,480 files, 41.2 GB in here today`, `A full check of all 12,480 files`.
//     No command returns them; C2 is the nearest neighbour (it walks the same index) but is scoped
//     to per-rule counts. Filed as G7 (#207) once F9 found the same numbers on six frames across
//     four screens. Pinned as `localTotals`.
//   · the schedule — `full_scan_schedule · weekly sun 03:00`. G4 (#193). See `8a Schedule monthly`.
//   · a rule's `added 14 Jul` date. A TOML array of globs carries no per-entry timestamps, so this
//     has no possible source in the config file; it is pinned as a literal on the rule it belongs to.
//
// The deletions tab needs none of that: C1 (#174) maps its three cards straight onto the existing
// `[delete_approval] remote/local` keys, which `read_config` already returns.

import { SETTINGS } from "../ui/copy.js";
import { ago } from "./clock.js";
import { settingsFids } from "./fids.js";

/**
 * The daemon behind all five frames: reachable, not paused, nothing in flight — every `8a` header
 * draws the `idle` chip, and `8a Save refused` needs this state to be TRUE rather than convenient,
 * because "your old settings are still running" is the sentence 08-settings.md calls the important
 * one. A refused save changes nothing, including this.
 *
 * `status` is the daemon's own string, and the daemon only ever sends `syncing`/`paused`/`running`
 * (`Daemon::response` in src/daemon.rs) — `idle` is the DERIVED state that `gui-core`'s `state.rs`
 * computes, which is why it appears on `state` and not inside `response`.
 */
const IDLE_STATUS = {
  state: "idle",
  response: {
    status: "running",
    paused: false,
    syncing: false,
    reconcile_seq: 41,
    pending_changes: 0,
    message: "sync completed",
    // Nothing in these frames draws it; the shell would render it relatively, so it follows the
    // clock convention rather than being pinned to a fixed epoch (see clock.js).
    last_sync_epoch_secs: ago(120),
    last_error: null,
    last_plan_summary: null,
    last_successful_sync_summary: null,
    status_history: [],
    pending_deletions: [],
    // `RunningConfigInfo` (three paths), NOT the `read_config` payload below — two different types
    // that both answer to `config`, and only this one belongs on a status reply.
    config: {
      local_root: "~/ProtonDrive",
      remote_root: "/Drive/RemoteFolder",
      db_path: "~/ProtonDrive/.sync/sync_index.db",
    },
    activity: null,
  },
};

/**
 * The same daemon, plus its standing "cannot be synced" list (#232) — `UnsyncableItem`s verbatim.
 *
 * Only the skip tab draws them, so only that frame carries them: on any other tab the panel is a
 * block the screen does not build, and a fixture that shipped the list everywhere would be claiming
 * the other three frames chose not to draw it.
 *
 * Three rows for a panel that says `Two more files`, and the third is why: `remote_not_downloadable`
 * is a real file on Proton Drive rather than a non-file in your folder, so it is in neither the
 * count nor the kinds. `7a Never synced` — which `See them` opens — makes the identical exclusion
 * from the identical list.
 */
const SKIP_STATUS = {
  ...IDLE_STATUS,
  response: {
    ...IDLE_STATUS.response,
    unsyncable: [
      {
        path: ".cache/session.sock",
        entity_kind: "file",
        reason: "local_socket",
        first_seen_epoch_secs: ago(86_400 * 12),
      },
      {
        path: "projects/current",
        entity_kind: "file",
        reason: "local_symlink",
        first_seen_epoch_secs: ago(86_400 * 12),
      },
      {
        path: "Unsorted/Networth",
        entity_kind: "file",
        reason: "remote_not_downloadable",
        first_seen_epoch_secs: ago(86_400 * 150),
      },
    ],
  },
};

/**
 * The one config dataset, shaped exactly like `read_config`'s `ConfigPayload`.
 *
 * Every value here is a value one of the four frames draws:
 *   · `local_root` / `remote_root` — the two inputs on the Folders tab.
 *   · `events_driven: true` — the live-updates toggle is drawn ON (44×26 filled `#F2F4F7`).
 *   · `exclude` — the three patterns the What-to-skip tab lists, in the order it lists them. This is
 *     the SAVED state; the staged removal on that frame lives in its `ui`, not here.
 *   · `delete_approval_*: true, true` — per C1 (#174) that pair is *Ask me every time*, the card the
 *     Deletions tab draws selected (15px dot, 4px `#F2F4F7` ring). The other two cards are
 *     `remote: false, local: true` (only permanent ones) and `false, false` (never ask).
 *
 * `scan_interval_secs` is drawn by NOTHING — the panel that would show it is the schedule panel G4
 * replaces — but `read_config` always returns it, and a fixture that omitted it would be a reply no
 * daemon could produce. It is here for that reason alone.
 */
const CONFIG = {
  path: "~/.config/proton-sync/proton-sync.toml",
  exists: true,
  toml: `# proton-sync
local_root = "~/ProtonDrive"
remote_root = "/Drive/RemoteFolder"
events_driven = true
scan_interval_secs = 300
exclude = ["*.tmp", "video-raw/**", "old-backups/**"]

[delete_approval]
remote = true
local = true
`,
  local_root: "~/ProtonDrive",
  remote_root: "/Drive/RemoteFolder",
  scan_interval_secs: 300,
  events_driven: true,
  include: [],
  exclude: ["*.tmp", "video-raw/**", "old-backups/**"],
  proton_cli: "proton-drive",
  proton_timeout_secs: 60,
  proton_list_attempts: 3,
  delete_approval_remote: true,
  delete_approval_local: true,
  // The same pair, as the radio group binds to it (C1). `true, true` is the card the Deletions tab
  // draws selected.
  deletion_policy: "ask_every_time",
};

/**
 * The local root's size, for `12,480 files, 41.2 GB in here today` (`SETTINGS.pairLocalNote`) and
 * `A full check of all 12,480 files` (`SETTINGS.fullScanSub`). Both deck entries take numbers, so
 * the fixture pins numbers and the screen formats them — 41_200_000_000 is `41.2 GB` through
 * `format.bytes`, which is decimal and one decimal place.
 *
 * NO COMMAND RETURNS THIS. It is the same pair `2a Settled` draws in its sub-line, and the same
 * count `7a Activity quiet` and `5a Checking` draw — six frames across four screens — so it was
 * never a Settings problem alone. Now filed as G7 (#207); `plan.js`'s `5a Checking` carries it
 * under the same key so the frames that draw it agree.
 */
const LOCAL_TOTALS = { files: 12480, bytes: 41_200_000_000 };

/**
 * The skip rules with their live effects — `skip_rule_usage`'s reply, as C2 (#175) shipped it.
 *
 * THE FIELDS CHANGED FROM WHAT THIS FIXTURE FIRST ANTICIPATED, in one place that matters. `stale`
 * was a boolean; the command returns `folder_exists`, because the drawn copy makes two different
 * claims and one boolean cannot hold both. `Matching nothing` is about the count, while `no such
 * folder here any more — safe to remove` is about the FOLDER — and the frame proves they come
 * apart: `video-raw/**` matches files and its second line still says `the folder still exists on
 * this computer` (`SETTINGS.ruleAdded`). A rule matching nothing whose folder is still there is
 * idle, not safe to remove.
 *
 * THE BYTE DISCRIMINATOR IS GONE, AND A SECOND FRAME IS WHAT RETIRED IT. This block used to give
 * `*.tmp` `bytes: 0`, read off `8a Skip rules`' own arithmetic — `hiding 4 files, 3.1 GB in total`
 * minus `video-raw/**`'s `3.1 GB` leaves nothing at that precision — so that `bytes` could be the
 * field telling `skippingNow(n)` (`Skipping 2 files right now`) from `skippingSize(n, size)`
 * (`Skipping 2 files, 3.1 GB`). DEVIATIONS §69a already recorded that live data has no such
 * discriminator and that S6 must decide the sub-line on something else.
 *
 * `7a Never synced` settles it from the other side: it draws THE SAME TWO FILES with their sizes —
 * `exports/draft.tmp · 2.1 MB` and `exports/render-final.tmp · 840 KB`. Those are drawn numbers, not
 * inferred ones, so the rule's total is 2.94 MB and the fixture says so. Every drawn string survives:
 * 3.1 GB + 2.94 MB still renders `3.1 GB` through `format.bytes`. The alternative — samples summing
 * to 2.94 MB inside a rule claiming zero — is a dataset no walk could produce, and the fixture's own
 * rule is that a screen and the dialog it links to must not describe two different configurations.
 *
 * `unique_*` is what the staged-removal cost line counts — see the note on `8a Skip rules`. Here it
 * equals `files`/`bytes` on every row, because no two of these rules overlap.
 */
const SKIP_RULES = {
  rules: [
    {
      pattern: "*.tmp",
      files: 2,
      // The sum of its two samples, which are the two files `7a Never synced` draws by name.
      bytes: 2_940_000,
      unique_files: 2,
      unique_bytes: 2_940_000,
      samples: [
        { path: "exports/draft.tmp", bytes: 2_100_000 },
        { path: "exports/render-final.tmp", bytes: 840_000 },
      ],
      // A bare file glob is anchored at no folder, so it makes no claim about one.
      folder_exists: null,
      error: null,
    },
    {
      pattern: "video-raw/**",
      files: 2,
      bytes: 3_100_000_000,
      unique_files: 2,
      unique_bytes: 3_100_000_000,
      samples: [
        { path: "video-raw/a-roll.mov", bytes: 1_600_000_000 },
        { path: "video-raw/b-roll.mov", bytes: 1_500_000_000 },
      ],
      // `the folder still exists on this computer`, in the deck's own words.
      folder_exists: true,
      error: null,
    },
    // Matches nothing AND its folder is gone: the row the tab exists to make findable, drawn at
    // `opacity:.62` with `no such folder here any more — safe to remove`. Both halves are needed —
    // the count alone would say the same thing about an empty folder that is still there.
    {
      pattern: "old-backups/**",
      files: 0,
      bytes: 0,
      unique_files: 0,
      unique_bytes: 0,
      samples: [],
      folder_exists: false,
      error: null,
    },
  ],
  // `hiding 4 files, 3.1 GB in total` — a count of distinct FILES, not the sum of the rows: the
  // command counts a doubly-matched file once, and here nothing overlaps so the two agree anyway.
  // The bytes are the two rules summed (3.1 GB + 2.94 MB), which `format.bytes` renders `3.1 GB` —
  // the drawn string, reached by arithmetic rather than by rounding one of the parts to zero.
  total_files: 4,
  total_bytes: 3_102_940_000,
  // NOT DRAWN. No frame states a denominator, so this is the only number in the family it could be
  // (`12,480 files … in here today`, the same pair the header uses). Nothing renders it; it is here
  // because the reply carries it and a fixture that omitted it would describe a different reply.
  considered_files: LOCAL_TOTALS.files,
  unreadable_directories: 0,
  unreadable_entries: 0,
};

/**
 * `added 14 Jul` on `video-raw/**` — the only row whose second line is the added-date
 * (`SETTINGS.ruleAdded`) rather than sample paths or the stale note.
 *
 * ITS OWN KEY BECAUSE NO COMMAND RETURNS IT. An `exclude = [...]` array in TOML records no
 * per-entry timestamps, so there is nowhere for this to come from — and putting it inside a
 * `skip_rule_usage` row would claim the command answers it. Literal absolute date, per the clock
 * convention.
 */
const RULE_ADDED = { "video-raw/**": "14 Jul" };

export const SETTINGS_FIXTURES = {
  /**
   * Tab 1, at rest. Save is drawn DISABLED (`#2A2E36` on `#6D7783`) and the footer carries the
   * neutral `SETTINGS.saveNote`, so this frame is the not-dirty half of the pair with `8a Skip
   * rules` — hence no `dirty` flag here. The asymmetry is the whole difference between the two
   * footers, and it is data, not styling.
   */
  "8a Settings": {
    status: IDLE_STATUS,
    config: CONFIG,
    localTotals: LOCAL_TOTALS,
    route: "settings",
    ui: { tab: "folders", schedule: "weekly" },
    fids: settingsFids("folders"),
  },

  /**
   * Tab 2. Two things differ from `8a Settings` beyond the tab: Save is ENABLED and the footer note
   * is replaced by the amber cost line `One rule removed — 2 files, 3.1 GB will start syncing.`
   * (`SETTINGS.ruleRemovedCost`) — so a removal is STAGED but not saved.
   *
   * `removing` names which rule, and it has to: the cost line's `2 files, 3.1 GB` are that rule's
   * C2 counts, so without an anchor those two numbers would have nowhere to come from. `config
   * .exclude` still lists all three patterns because nothing has been written yet, which is what
   * "Nothing is written until you save" means.
   *
   * THE COST LINE READS `unique_files`/`unique_bytes`, NOT `files`/`bytes`. Removing a rule only
   * starts syncing the files no *other* rule still hides, and `will start syncing` is a promise
   * about what happens next. The two agree on this frame because these three rules do not overlap —
   * which is exactly why the distinction has to be written down rather than discovered later.
   *
   * THE FRAME IS INTERNALLY INCONSISTENT HERE and no dataset can fix it: it draws `video-raw/**` in
   * the list at full opacity while the footer says its rule was removed, and it counts that rule
   * inside `hiding 4 files, 3.1 GB in total`. Whether a staged removal dims its row, strikes it, or
   * leaves it alone is S6's call; the fixture states the saved config and the staged edit and lets
   * the screen decide.
   */
  "8a Skip rules": {
    status: SKIP_STATUS,
    config: CONFIG,
    skipRules: SKIP_RULES,
    ruleAdded: RULE_ADDED,
    // The bottom panel IS pinned now (#232), and not off a plan summary: `skipped_unsupported`
    // counts REMOTE files the CLI could not fetch, which is a different fact and was never this
    // panel's subject. `ControlResponse.unsyncable` is the daemon's standing list of what its own
    // local walk drops, so `Two more files … a socket and a shortcut` is two rows and their kinds.
    // The remote row is carried too, and is excluded from both the count and the kinds — the same
    // exclusion `7a Never synced` makes, which `See them` opens.
    route: "settings",
    ui: { tab: "skip", dirty: true, removing: "video-raw/**" },
    fids: settingsFids("skip"),
  },

  /**
   * Tab 3, drawn as a 600px crop. Nothing here is new data — the selected card IS
   * `delete_approval_remote/local`, per C1 (#174), and both are `true` in the shared config.
   *
   * The mono key line the frame draws, `deletion_policy · applies to both directions`, names a key
   * the daemon does not have; G5 (#194) would mint it natively. Until then it is a label over two
   * existing keys.
   *
   * `read_config` DOES now return a `deletion_policy` field, and it is not the alias: it is the two
   * booleans classified, which is what a radio group binds to. Nothing about it goes into the TOML
   * — `config.toml` above still shows `[delete_approval]`, because that is what is written. The
   * distinction is the whole of C1: a name for the mapping, not a new key.
   */
  "8a Deletions tab": {
    status: IDLE_STATUS,
    config: CONFIG,
    route: "settings",
    ui: { tab: "deletions" },
    fids: settingsFids("deletions"),
  },

  /**
   * The monthly variant of the schedule panel, drawn as a 600px crop of tab 1.
   *
   * ENGINE GAP G4 (#193). The frame draws a whole schedule — `Monthly` selected, day chips 1…20 with
   * 15 lit, a `03:00` stepper, and the key line `full_scan_schedule · monthly day 15, 03:00` — and
   * there is no `full_scan_schedule` key, no daemon scheduler, and no command that returns any of
   * it. The config this fixture carries is the real one: `scan_interval_secs`, which
   * IMPLEMENTATION-PLAN.md says Phase 1 presents in plain language inside the same panel shell.
   *
   * So `ui.schedule` says only WHICH VARIANT is showing — the one word that distinguishes this frame
   * from `8a Settings`, in the same form as `{ tab: "skip" }`. The day and the time are deliberately
   * absent: those are the fields G4 will name, and pinning them now would settle its shape from a
   * fixture. This frame is not fully reproducible until G4 lands, which is the honest position.
   *
   * One copy discrepancy to know about, since it is the same panel in both frames: this crop draws
   * `A full check of all 12,480 files as a safety net.` and stops, where `8a Settings` continues
   * `… It's slow, so it runs on a schedule rather than constantly.` — the deck's `SETTINGS
   * .fullScanSub` carries only the longer form.
   */
  "8a Schedule monthly": {
    status: IDLE_STATUS,
    config: CONFIG,
    localTotals: LOCAL_TOTALS,
    route: "settings",
    ui: { tab: "folders", schedule: "monthly" },
    fids: settingsFids("monthly"),
  },

  /**
   * The refused save (DEVIATIONS §57: a dialog, no ✕, two repairs and no dismiss).
   *
   * `saveError` is the daemon's own words in mono — voice rule 4, and the deck carries the example
   * verbatim so it is imported rather than retyped. `config` is UNCHANGED, and `status` is the same
   * untouched idle daemon: together they are what makes "Nothing was saved — your old settings are
   * still running" a fact about the fixture and not just a sentence.
   *
   * TWO THINGS ABOUT THIS STRING THAT NO PHASE-1 CODE CAN PRODUCE, both worth knowing before S6
   * builds the dialog:
   *   · `write_config` refuses on `ConfigDoc::validate`, which is a serde/TOML check against
   *     `FileConfig`. It never probes Proton Drive, so it cannot say `not found` about a remote
   *     folder. Producing this refusal at all needs a remote-existence probe (`list_remote`) before
   *     the write — nobody's issue today.
   *   · the error that IS produced arrives as `config would be rejected by the daemon: …`
   *     (`ConfigError::Display`), while the frame draws the bare reason. Whatever S6 shows in that
   *     mono box must be the reason alone.
   */
  "8a Save refused": {
    status: IDLE_STATUS,
    config: CONFIG,
    saveError: SETTINGS.refusedDaemonExample,
    route: "settings",
    ui: { dialog: "saveRefused" },
    fids: settingsFids("refused"),
  },
};
