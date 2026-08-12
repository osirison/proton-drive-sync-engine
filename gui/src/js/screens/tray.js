// The tray panel (S8) — `10a Settled`, `10a Syncing`, `10a Offline`, `10a Paused`, and the panel
// inside `10a In situ`.
//
// It is `ui/compact.js` in the `tray` family with the menu section as its tail, so almost nothing
// here is layout: F6 built the panel and the four frames that draw it already pass the fidelity
// gate. What this module owns is the question F6 left open — WHICH panel, from a status reply.
//
// THE DERIVATION IS S1's, DELIBERATELY. `heroStateOf` is imported from `screens/main.js` rather than
// reimplemented, and that is the single most important line in the file. The window and the tray are
// two views of one moment, and a person who opens both at once and reads two different sentences has
// been told the app does not know what is happening. Every rule in that function was paid for once —
// unreachable outranking everything, `pending > 0` counting as syncing even when `syncing` is false,
// `authExpired` not falling through to a false all-clear — and a second copy would relitigate all of
// them badly. §82f.
//
// WHAT IS NEW HERE IS THE TWO STATES THE WINDOW NEVER SEES. `app.js` intercepts `firstRun` with the
// onboarding takeover before the main screen renders, so S1's derivation has no branch for it and
// falls through to `settled` — `Everything is up to date`, on a daemon that has never synced a
// file. The tray has no takeover to hide behind. `authExpired` reaches S1 but shares the struck mark
// with `unreachable` there, and in a MENU the two part company: `Try again now` cannot fix an
// expired session. Both are handled below, both are tested, and no frame draws either. §82g.

import { MAIN, TRAY } from "../ui/copy.js";
import { clock, since } from "../ui/format.js";
import { renderCompactPanel, updateCompactPanel, trayMenu } from "../ui/compact.js";
import { heroStateOf } from "./main.js";

/**
 * The hero state S1 derives → the panel arrangement `ui/compact.js` draws.
 *
 * Five forms, and `10-tray.md` is explicit that there is no sixth. `authExpired` and `unreachable`
 * share one — "both mean *Proton is out of reach*; only the sentence underneath differs", which is
 * S1's own note and `11-notifications.md`'s grouping (one struck icon behind "an outage, expired
 * session, or full disk").
 */
const PANEL_STATE = {
  settled: "settled",
  syncing: "syncing",
  decision: "needsYou",
  paused: "paused",
  unreachable: "unreachable",
  authExpired: "unreachable",
  // The third of `11-notifications.md`'s one-icon trio ("an outage, expired session, or full disk"),
  // and `14-behaviour-and-state.md`'s own route into the struck mark: "unreachable is entered after
  // a failed pass and retry". #246.
  failed: "unreachable",
  firstRun: "needsYou",
};

/**
 * The menu rows, which are NOT keyed by the panel form.
 *
 * Two daemon states share the struck hexagon and share nothing else: the row that fixes an outage is
 * `Try again now`, and the row that fixes an expired session is signing in, which lives in the
 * window. Offering `Try again now` for expired auth is offering a button that cannot do what the
 * sentence above it asks — the failure `10-tray.md` is otherwise careful about ("the labels say what
 * each does"). So the panel takes the form and the menu takes the cause.
 */
const MENU_STATE = {
  settled: "settled",
  syncing: "syncing",
  decision: "needsYou",
  paused: "paused",
  unreachable: "unreachable",
  authExpired: "deferToWindow",
  // `unreachable`'s rows, not `deferToWindow`'s: this is the one struck state where `Try again now`
  // is unambiguously a working control, because the daemon is answering and `syncnow` reaches it.
  // Same table, same reasoning, in `tray_menu.rs` for the native menus.
  failed: "unreachable",
  firstRun: "deferToWindow",
};

/**
 * The whole panel, derived once.
 *
 * Same shape of argument as `mainView` and for the same reason: the render and the ~2s patch must
 * not be able to disagree about what state this is.
 */
export function trayView(props = {}) {
  const { daemonState = "unreachable", response = null, conflicts = [], deletions = [] } = props;

  const activity = response?.activity ?? null;
  const summary = response?.last_plan_summary ?? null;
  const queued = response?.pending_changes ?? null;
  const waiting = conflicts.length + deletions.length;
  const lastSync = response?.last_sync_epoch_secs ?? null;

  // `firstRun` is not a hero S1 knows, so it is answered before asking. Everything else goes to the
  // shared derivation — including the `pending > 0` rule, which is why a tray opened seconds after
  // an edit says "syncing" rather than "up to date" exactly as the window does.
  const hero =
    daemonState === "firstRun"
      ? "firstRun"
      : heroStateOf({
          daemonState,
          syncing: Boolean(response?.syncing),
          waiting,
          pending: queued ?? 0,
        });

  // The same two numbers S1 reconciles: the watch queue is the answer while work waits and no plan
  // exists, the plan is the answer once there is one. Deletions are excluded — "the count in the
  // hexagon is transfers, not decisions".
  const moving = summary ? summary.uploads + summary.downloads : null;
  const changes = response?.syncing ? (moving ?? queued) : queued;

  return {
    state: PANEL_STATE[hero],
    menuState: MENU_STATE[hero],
    hero,
    ...copyFor(hero, { changes, waiting, queued, lastSync, activity, summary }),
    transfers: PANEL_STATE[hero] === "syncing" ? transfersOf(activity) : [],
  };
}

/**
 * The in-flight transfer, at panel scale.
 *
 * `10a Syncing` draws TWO rows and the reply carries at most one: `SyncActivity.transfer` is a
 * single in-flight file (#211, G10 — "status reports one in-flight transfer, the main screen draws a
 * queue"). One row is what is true, and a second invented row would be a file that is not moving.
 *
 * No `detail` and no `state`, unlike S1's: the compact row is flat — `rows.js` takes `size:
 * "compact"` — and neither of the two drawn compact transfer rows carries a size chip.
 */
function transfersOf(activity) {
  const t = activity?.transfer;
  if (!t) return [];
  return [
    {
      direction: t.direction === "download" ? "down" : "up",
      name: t.path,
      // `null` is "no track", not "0%" — a reply never carries both ends of a percentage for a
      // download until the staging directory has something in it.
      progress: t.bytes_done != null && t.bytes_total != null ? t.bytes_done / t.bytes_total : null,
    },
  ];
}

/**
 * Headline, sub-line and count, per state.
 *
 * Every string is `ui/copy.js`'s. Three surfaces quote `Can't reach Proton Drive` — the window, this
 * panel and the outage notification — and the deck exists so they cannot drift.
 */
function copyFor(hero, v) {
  switch (hero) {
    case "syncing":
      return {
        headline: MAIN.syncing(v.changes),
        count: v.changes,
        // No sub-line. `10a Syncing` draws the headline, then the transfer rows, and nothing
        // between them — the panel is 362px and the window's `started 2 minutes ago · 2 leaving,
        // 1 arriving` is a line it does not have room for. Measured off the frame, not chosen.
        sub: null,
      };

    case "paused":
      return {
        headline: MAIN.paused,
        sub: MAIN.pausedSub(v.queued ?? 0, clock(v.lastSync)),
      };

    case "unreachable":
      return {
        headline: TRAY.unreachableTitle,
        // `null` when the counters are unknown, which for an unreachable daemon they always are —
        // `unreachableBody` then DROPS the count clause and keeps the reassurance. `0 changes are
        // waiting` would be a false all-clear at the exact moment the app cannot see anything.
        sub: TRAY.unreachableBody(null),
        // `retrying in 40s · last reached 13:58` is drawn and is not derivable: nothing in the reply
        // says when the next attempt is, and an unreachable daemon is not answering to be asked.
        // Omitted rather than filled (#213 covers the pass clock). §82h.
        meta: null,
      };

    case "authExpired":
      return {
        headline: MAIN.authExpired,
        sub: MAIN.authExpiredSub(v.queued ?? 0),
      };

    case "failed":
      return {
        headline: MAIN.failed,
        // The reassurance and the count, and NOT the daemon's string: the panel is 362px wide with
        // no block sized to quote one, and a stderr wrapped into a sub-line is the paraphrase-by-
        // truncation voice rule 4 forbids. `Open Drive Sync` is a row on this very menu, and the
        // window has the block. #246.
        sub: MAIN.failedSub(v.queued ?? 0),
      };

    case "decision":
      return {
        headline: MAIN.compact.needYou(v.waiting),
        count: v.waiting,
        // The two lines `2a Compact needs you` draws, as an array so the break falls where the
        // design put it rather than wherever 362px happens to wrap.
        sub: [MAIN.compact.conflictLine, MAIN.compact.deletionLine],
        action: { label: MAIN.compact.review, id: "review" },
      };

    case "firstRun":
      return {
        headline: TRAY.nothingSyncedYet,
        sub: TRAY.nothingSyncedYetSub,
        // No count: nothing is waiting, and `renderHexagon` draws no `<text>` at all for a null
        // numeral — which is the shape this needs. A `0` inside the mark would be a queue of zero
        // things presented as a decision.
        count: null,
        action: { label: TRAY.open, id: "open" },
      };

    default:
      return {
        headline: MAIN.compact.upToDate,
        // `2 minutes ago · 12,480 files` is drawn; Phase 1 has the timestamp and not the count —
        // no command reports an index-wide file total (G7, #207). The remaining clause is a
        // relative time, not a sentence, so there is no deck string to own it: it IS `since()`.
        // The day #207 lands this gains ` · ${count(files)} files` and a copy entry with it.
        sub: since(v.lastSync),
        subMono: true,
      };
  }
}

/**
 * Build the panel. `onSelect(id)` takes every menu row AND the hero action — `Review them` and
 * `Open Drive Sync` are dispatched by the same id space as the rows, so the window that wires this
 * up has one handler rather than two that can disagree about what `open` means.
 */
export function renderTrayPanel(view, onSelect = null) {
  return renderCompactPanel({
    state: view.state,
    family: "tray",
    headline: view.headline,
    sub: view.sub ?? null,
    subMono: view.subMono ?? false,
    meta: view.meta ?? null,
    count: view.count ?? null,
    transfers: view.transfers ?? [],
    action: view.action ? { ...view.action, onClick: () => onSelect?.(view.action.id) } : null,
    menu: trayMenu(view.menuState, onSelect),
  });
}

/**
 * Patch across a poll. Returns false when the panel's shape changed, which is the caller's signal to
 * render a fresh one — same contract as `updateCompactPanel`, which does the work.
 *
 * The tray polls on the same ~2s cadence as the window, and this is where that matters most: a
 * rebuild restarts the syncing mark's animation from 0% and drops keyboard focus out of the menu,
 * in a panel people click through in about a second.
 */
export function updateTrayPanel(node, view) {
  if (!node) return false;
  return updateCompactPanel(node, {
    state: view.state,
    // THE MENU IS PART OF WHAT IS ON SCREEN, and passing only `state` made a patch blind to it.
    // Three hero states share the `unreachable` PANEL form and two of them want different ROWS
    // (`MENU_STATE` above), so `failed` → `authExpired` between two polls patched the headline to
    // `Proton Drive is asking you to sign in again` over a menu still offering `Try again now` —
    // the row that cannot do what the sentence above it asks. Built without handlers because only
    // the ids are compared; a mismatch returns false and the caller renders a fresh panel. #246.
    menu: trayMenu(view.menuState),
    headline: view.headline,
    sub: Array.isArray(view.sub) ? undefined : (view.sub ?? undefined),
    meta: view.meta ?? undefined,
    count: view.count ?? null,
    transfers: view.transfers ?? [],
  });
}
