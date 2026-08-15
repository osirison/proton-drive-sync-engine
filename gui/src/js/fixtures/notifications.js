// Notification fixtures (S9) — the five `11a` frames.
//
// TWO OF THEM ARE NOT NOTIFICATIONS. `11a Rules` and `11a Settings` are the two halves of Settings ›
// Notifications, re-rendered at 600 and 520 the way `8a Deletions tab` re-renders its tab at 600 —
// `frame-classes.mjs` calls all four crops now, and they carry a `status` + `route` + `ui` like every
// other settings frame rather than a `notification` payload. F9 classed them `notification` because
// nothing had yet decided where they live. DEVIATIONS §83.
//
// THE COPY IS NOT HERE. F9 carried every drawn string in this file because no module rendered them;
// they are `NOTIFY` in `ui/copy.js` now, gated verbatim against these frames by `copy-gate.mjs`. What
// stays is what a fixture is for: the DATA behind a banner, and which card is chosen.
//
// A FIXTURE CARRIES AN EVENT, NOT A SENTENCE. `notification.event` is what the triggers in `app.js`
// produce and `ui/notification.js` turns into a banner, so a frame proves the builder as well as the
// drawing. Three of the drawn sentences are longer than the ones Phase 1 can build (the subtree count
// G8 #208, the index totals G7 #207); `known-deviations.mjs` carries the exact rows.

import { ago } from "./clock.js";
import { notifyFids, settingsFids } from "./fids.js";

/**
 * The settings window behind both crops. Minimal on purpose: nothing on this tab reads the daemon —
 * the rules sheet is the policy itself and `notify_policy` is GUI-local (IMPLEMENTATION-PLAN row 6),
 * so a richer status would be describing data the tab never touches.
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

/**
 * `at` IS AN EPOCH AND A GETTER. The header's time is a relative render (`now`, `2m ago`, `14:12` —
 * all three in the specimen), so rule 3 pins it as an offset; and a plain `at: ago(0)` is evaluated
 * once at import, which ages by the time the harness reaches extraction. Reading the property
 * re-runs `ago`, so it is 0 whenever it renders. Same reason `plan.js` measured on `ui.checkedAt`.
 */
const NOW = {
  get at() {
    return ago(0);
  },
};

export const NOTIFICATION_FIXTURES = {
  // ------------------------------------------------------------------------- the two banners ----

  /**
   * Several conflicts are ONE banner with the count as the hexagon's numeral (§Grouping), coalesced
   * within 30 seconds and never stacked. Five paths, because the count is derived from them rather
   * than declared beside them — a count and a list that can disagree is two answers.
   */
  "11a Grouped": {
    notification: {
      ...NOW,
      event: {
        kind: "conflict",
        paths: [
          "notes/todo.txt",
          "docs/plan.md",
          "src/main.rs",
          "photos/2019/index.json",
          "archive/old-notes.md",
        ],
      },
    },
    fids: notifyFids("grouped"),
  },

  /**
   * The fourth trigger. `changes` is the daemon's `pending_changes`, which is what the drawn `61
   * changes are waiting` counts; null would drop the clause, and no frame draws that.
   */
  "11a Outage": {
    notification: {
      ...NOW,
      event: { kind: "outage", changes: 61 },
    },
    fids: notifyFids("outage"),
  },

  // ---------------------------------------------------------- the two Settings › Notifications ----

  /** The rules sheet: four events that interrupt, twelve categories that stay silent, one hard rule. */
  "11a Rules": {
    status: IDLE_STATUS,
    route: "settings",
    ui: { tab: "notifications" },
    fids: settingsFids("notifyRules"),
  },

  /**
   * The `notify_policy` cards. The frame draws the first as chosen — a 4px ring on a lifted card
   * where the other two sit hairlined — which is the only state here, and it is named by its policy
   * value rather than by an index now that S9 has defined the vocabulary.
   */
  "11a Settings": {
    status: IDLE_STATUS,
    route: "settings",
    ui: { tab: "notifications", notifyPolicy: "only_when_needed" },
    fids: settingsFids("notifyPolicy"),
  },

  // -------------------------------------------------------------------------------- the mock ----

  /**
   * Three banners over a desktop mock. A specimen: the bar, the clock and the wallpaper are scenery
   * (`SPECIMEN_ARTEFACT`), and the three banners are the product.
   *
   * IT IS THE ONLY GATE COVERAGE THREE OF THE FOUR BANNERS HAVE — permanent deletion (the one that
   * can cost files), conflict and first sync are drawn nowhere else. So it carries data, unlike the
   * `10a In situ` panel mock, and its banners are mapped.
   *
   * The three ages are the three registers `notifyTime` has to produce: under a minute, under an
   * hour, and beyond it. The third is an epoch rather than the drawn `14:12` because a clock time
   * moves across a timezone (`clock.js`) — any `HH:MM` is the same width in mono, so the box the
   * gate compares is the same one.
   */
  "11a In situ": {
    specimen: {
      note: "three banners — permanent deletion, conflict, first sync finished — over a desktop mock whose bar, clock and wallpaper are scenery",
    },
    desktop: {
      banners: [
        {
          get at() {
            return ago(0);
          },
          // ONE QUEUE ITEM, and a directory: the frame's banner is about `photos/2019` being deleted
          // on Proton, which is one withheld deletion whose subtree holds 1,204 photos. `files` is
          // the daemon's `subtree_files` (#208) — the same number `4a Deletions` draws on the card,
          // and what the title counts instead of the queue's own length.
          event: { kind: "deletion", paths: ["photos/2019"], entity: "folder", files: 1204 },
        },
        {
          get at() {
            return ago(120);
          },
          event: { kind: "conflict", paths: ["notes/todo.txt"] },
        },
        {
          get at() {
            return ago(7200);
          },
          // No files/bytes: `first sync finished — 12,480 files, 41.2 GB` is G7 (#207), and the rule
          // for a missing capability is to drop the clause rather than fill it.
          event: { kind: "firstSync" },
        },
      ],
    },
    fids: notifyFids("inSitu"),
  },
};
