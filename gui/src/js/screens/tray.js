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
// onboarding takeover before the main screen renders (at one pair), so S1's derivation falls through
// to `settled` for it unless asked otherwise — `Everything is up to date`, on a daemon that has never
// synced a file. The tray has no takeover to hide behind, so it asks (`drawsFirstRun`). `authExpired`
// reaches S1 but shares the struck mark with `unreachable` there, and in a MENU the two part company:
// `Try again now` cannot fix an expired session. Both are handled below, both are tested, and no
// frame draws either. §82g.

import { MAIN, TRAY } from "../ui/copy.js";
import { clock, since } from "../ui/format.js";
import {
  renderCompactPanel,
  updateCompactPanel,
  bindMenu,
  folderMenuRows,
  TRAY_FOLDER_CAP,
} from "../ui/compact.js";
import { heroStateOf, transfersOf } from "./main.js";

/**
 * How many transfer rows the panel draws, and it is a HARD cap rather than a preference.
 *
 * `10a Syncing` and `2a Compact syncing` both draw exactly two, in a 362px panel whose height sizes
 * the tray window — and the panel has no `+n more` line, because no frame draws one there. Until
 * #211 the reply could describe one transfer, so nothing ever reached this; the window is now up to
 * six, and handing all six to a 298px panel would grow it past the height the window was measured
 * at. The count is not lost: the headline above these rows is `Syncing N changes`.
 *
 * The window screen's own cap lives in `screens/main.js` — a different number for a different
 * surface, which is why neither imports the other's.
 */
const PANEL_ROWS = 2;

/**
 * The hero state S1 derives → the panel arrangement `ui/compact.js` draws.
 *
 * Five forms, and `10-tray.md` is explicit that there is no sixth. `authExpired`, `failed` and
 * `unreachable` share one, on `11-notifications.md`'s grouping — one struck icon behind "an outage,
 * expired session, or full disk".
 *
 * S1's own note used to justify that with "both mean *Proton is out of reach*", and for `unreachable`
 * it is not true: the socket is what did not answer. The FORM still holds — nothing is syncing and
 * something is wrong, which is what the struck mark says — but it is the only claim the three share,
 * and `MENU_STATE` below is where they stop sharing. §95.
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
  // A folder that has not had its turn wears the MOVING mark (the syncing glyph, in the column the settled
  // panel has and without its seam): it has not synced anything in this run, so the settled form would say
  // it had, and it asks nothing of the person, so the needs-you form would say it did. Something is
  // syncing, or is about to start. It draws no count, no rows and no button — there is nothing in flight,
  // nothing to decide and nothing to open the window for (#455).
  queued: "waiting",
};

/**
 * The menu rows, which are NOT keyed by the panel form.
 *
 * THREE daemon states share the struck hexagon and want three different menus, because the thing
 * that fixes each is different: a failed pass is retried (`Try again now` reaches a daemon that is
 * answering), an expired session is fixed by signing in — which is a terminal, not a menu — and a
 * daemon that is not running is fixed by starting it. Offering a row that cannot do what its label
 * says is the failure `10-tray.md` is otherwise careful about ("the labels say what each does").
 *
 * So the panel takes the form and the menu takes the cause. Two of these were one entry until §95:
 * `unreachable` and `failed` both pointed at the set holding `Try again now`, and on the first of
 * them there was no daemon on the other end of the socket for that retry to reach.
 */
const MENU_STATE = {
  settled: "settled",
  syncing: "syncing",
  decision: "needsYou",
  paused: "paused",
  // NOT `outage`, which is what this said while that set was called `unreachable` — and the two
  // words looking alike is exactly how it went unnoticed. `derive_state` answers `unreachable` when
  // the CONTROL SOCKET did not respond, so the daemon is not there to be asked for anything: a
  // `Try again now` here retried a sync against nothing, which is a row that cannot do what it says.
  // The thing that fixes a stopped service is starting it, and that is `notRunning`'s lead row.
  unreachable: "notRunning",
  authExpired: "deferToWindow",
  // `outage`'s rows, not `deferToWindow`'s: this is the one struck state where `Try again now` is
  // unambiguously a working control, because the daemon is answering and `syncnow` reaches it.
  // Same table, same reasoning, in `tray_menu.rs` for the native menus.
  failed: "outage",
  firstRun: "deferToWindow",
  // The settled rows, and at two folders or more the pause row of every folder: a folder waiting for
  // its turn can be paused like any other (`tray_menu.rs` puts `Queued` beside `Idle` and `Running`).
  queued: "settled",
};

/**
 * The folders of a payload, joined to the state Rust derived for each — or `[]` when there are fewer
 * than two, or when the payload cannot say what state every one of them is in.
 *
 * `[]` is the one-folder panel, byte for byte what it always was (decision D2), and it is also the
 * answer to an inconsistent payload: a folder with no derived state is not defaulted to `idle`, which
 * would be the false all-clear this whole module exists to prevent — the panel simply does not claim
 * to know about several folders. `rank` is Rust's (`gui_core::state::severity`), never recomputed here.
 */
export function foldersOf(pairs, pairStates) {
  if (!Array.isArray(pairs) || pairs.length < 2 || !Array.isArray(pairStates)) return [];
  const derived = new Map(pairStates.map((entry) => [entry.name, entry]));
  const folders = [];
  for (const summary of pairs) {
    const entry = derived.get(summary.name);
    if (!entry) return [];
    folders.push({
      name: summary.name,
      paused: Boolean(summary.paused),
      syncing: Boolean(summary.syncing),
      rank: entry.rank ?? 0,
      state: entry.state,
      // The folder a `queued` one waits for (Rust's `waiting_for`); `null` when nothing is running.
      waitingFor: entry.waiting_for ?? null,
      summary,
    });
  }
  return folders;
}

/**
 * What a folder known only by its summary can say to `trayView`: the reply fields it reads, from the
 * numbers a `PairSummary` carries. A summary has no live activity and no plan, so a syncing folder that
 * is not the one the reply describes has a count and no transfer rows — the reply's `activity` belongs
 * to the folder that is syncing and is only on THAT folder's reply.
 */
const factsOfSummary = (summary) => ({
  syncing: summary.syncing,
  paused: summary.paused,
  pending_changes: summary.pending_changes,
  last_sync_epoch_secs: summary.last_sync_epoch_secs,
  last_error: summary.last_error,
  activity: null,
  last_plan_summary: null,
});

/**
 * Decisions waiting on a person in one folder, as far as the tray can see them.
 *
 * Every folder's withheld deletions are in its summary, so those are counted for all of them. Its
 * conflicts are not: they come from a disk walk the tray runs for ONE folder (the one the reply
 * describes), so only that folder has any to count. `scanned` is that folder's own lists, or `null`
 * for a folder the tray only knows by its summary. The reply's own deletion list and the summary's
 * count describe the same deletions, so the larger stands rather than their sum.
 */
function decisionsIn(folder, scanned) {
  const own = folder.summary.pending_deletions ?? 0;
  return scanned ? Math.max(own, scanned.deletions) + scanned.conflicts : own;
}

/**
 * What the panel gives up to a decision waiting elsewhere: a folder in one of these states has nothing
 * moving or wrong in it. A paused one is the person's own doing, an up to date one has nothing to say, and
 * a queued one is only waiting for its turn — which can last the whole of another folder's pass.
 */
const GIVES_WAY_TO_A_DECISION = new Set(["idle", "queued", "paused"]);

/**
 * The folders whose own decision the panel draws. Not a paused one: its deletions wait behind the pause,
 * exactly as they do at one folder ("paused outranks a decision").
 */
const DRAWS_ITS_DECISION = new Set(["idle", "queued"]);

/**
 * The folder whose panel is drawn: the worst by rank (Rust's, never recomputed here), ties to the
 * first the daemon lists — **except that a folder that is up to date or waiting for its turn and has a
 * decision waiting outranks one that is merely paused, queued or up to date with nothing to decide.**
 *
 * Paused is what a person did on purpose, and the decision is the one thing the tray cannot do for
 * them; an all-clear, a "Paused" or a "Waiting for documents" panel over a withheld deletion in another
 * folder is how it goes unseen. "Needs you" outranks "waiting" (review of #459): a queued folder is
 * ranked above an idle one, so without this a deletion held by a finished folder lost its `Review them`
 * for as long as the other folder's pass ran. The glyph and the title are not touched by this — they
 * follow `severity` alone — and what does outrank a decision is still everything that is moving or
 * wrong: syncing, a failed pass, a lapsed session, a stopped daemon. Those panels have no `Review
 * them`, at one folder as at several.
 */
function panelFolderOf(folders, decisions) {
  const worst = folders.reduce((best, folder) => (folder.rank > best.rank ? folder : best));
  if (!GIVES_WAY_TO_A_DECISION.has(worst.state)) return worst;
  const holder = folders.find((folder, i) => DRAWS_ITS_DECISION.has(folder.state) && decisions[i] > 0);
  return holder ?? worst;
}

/**
 * The whole panel, derived once.
 *
 * Same shape of argument as `mainView` and for the same reason: the render and the ~2s patch must
 * not be able to disagree about what state this is.
 *
 * **With two folders or more** (`pairs` and `pairStates` from the payload, decision D3) the hero is
 * the WORST folder's own — its headline and sub-line unchanged — preceded by its name, and the menu
 * gains one pause row per folder. The folder the reply describes keeps everything it had (conflicts,
 * live transfers, the first-run hero); any other is read from its summary, which is less, and says so
 * by drawing less rather than by inventing. Below two, `pair` is `null` and the rows are the fixed
 * set, so a one-folder panel is the one it always was.
 *
 * **What `Needs you` counts at several folders is what the tray can see**: the withheld deletions of
 * EVERY folder, and the conflicts of the one it scans (`decisionsIn`). It used to count the panel
 * folder's alone, and a deletion waiting in another folder was hidden behind "Up to date" or "Paused"
 * with no way in. `Review them` names the folder it is drawn for (`review@photos`), so the window it
 * opens shows that folder.
 */
export function trayView(props = {}) {
  const {
    daemonState = "unreachable",
    response = null,
    conflicts = [],
    deletions = [],
    pairs = [],
    pairStates = [],
  } = props;

  const folders = foldersOf(pairs, pairStates);
  if (folders.length === 0) {
    return {
      ...panelOf(daemonState, response, conflicts.length + deletions.length),
      pair: null,
      menuRows: null,
    };
  }

  const scannedName = response?.pair ?? null;
  const decisions = folders.map((folder) =>
    decisionsIn(
      folder,
      folder.name === scannedName ? { deletions: deletions.length, conflicts: conflicts.length } : null,
    ),
  );
  const waiting = decisions.reduce((sum, n) => sum + n, 0);

  const shown = panelFolderOf(folders, decisions);
  const described = scannedName === shown.name;
  // The decisions are drawn only for a folder that holds one. Every state but a queued one ignores them
  // (it is moving, wrong or paused first), so this changes nothing for them; a queued folder beside a
  // paused folder's deletion must stay a waiting panel, not take a `Review them` that is not its own.
  const decisive = decisions[folders.indexOf(shown)] > 0 ? waiting : 0;
  const panel = described
    ? panelOf(daemonState, response, decisive, shown.name, shown.waitingFor)
    : panelOf(shown.state, factsOfSummary(shown.summary), decisive, shown.name, shown.waitingFor);
  const view = {
    ...panel,
    pair: shown.name,
    menuRows: folderMenuRows(panel.menuState, folders, { cap: TRAY_FOLDER_CAP }),
  };
  // The decision button names its folder; the one-folder panel's is the bare `review` it always was.
  if (panel.action?.id === "review") view.action = { ...panel.action, id: `review@${shown.name}` };
  return view;
}

/**
 * One folder's panel, from its state and the reply fields. This is the whole of what `trayView` was
 * before folders; it is a function of ONE folder, so the several-folder view is that function applied
 * to the worst one rather than a second derivation.
 *
 * `folder` is the folder's name at two folders or more and `null` at one. It changes one sentence: a
 * paused hero names the folder that is paused (`TRAY.pausedSubPair`), because another folder may be
 * syncing under it and "nothing will move" would be untrue of the app. `waitingFor` is the folder a
 * `queued` folder waits for, or `null` when nothing is running yet.
 */
function panelOf(daemonState, response, waiting, folder = null, waitingFor = null) {
  const activity = response?.activity ?? null;
  const summary = response?.last_plan_summary ?? null;
  const queued = response?.pending_changes ?? null;
  const lastSync = response?.last_sync_epoch_secs ?? null;

  // Everything goes to the shared derivation — including the `pending > 0` rule, which is why a tray
  // opened seconds after an edit says "syncing" rather than "up to date" exactly as the window does.
  // `firstRun` used to be answered here, before asking, because S1 had no such hero; it has one now
  // (#102 phase 5a-2), for a surface with no takeover, and this is that surface — so the rule is
  // `heroStateOf`'s alone and not also a copy in this file.
  const hero = heroStateOf({
    daemonState,
    syncing: Boolean(response?.syncing),
    waiting,
    pending: queued ?? 0,
    drawsFirstRun: true,
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
    ...copyFor(hero, { changes, waiting, queued, lastSync, activity, summary, folder, waitingFor }),
    transfers:
      PANEL_STATE[hero] === "syncing" ? transfersOf(activity, { compact: true }).slice(0, PANEL_ROWS) : [],
  };
}

/** The rows a view draws: the folder menu when it has one, else the fixed set for its menu state. */
const menuRowsOf = (view) => view.menuRows ?? folderMenuRows(view.menuState, []);

/**
 * Headline, sub-line and count, per state.
 *
 * Every string is `ui/copy.js`'s, and the deck exists so the window and this panel cannot drift: the
 * two are one moment seen twice, so every case below reaches for the same constant `screens/main.js`
 * does. `unreachable` is the case that proves the rule — when its sentence changed, it changed on
 * both surfaces at once, because neither of them owns one.
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
        sub:
          v.folder == null
            ? MAIN.pausedSub(v.queued ?? 0, clock(v.lastSync))
            : TRAY.pausedSubPair(v.queued ?? 0, clock(v.lastSync), v.folder),
      };

    case "unreachable":
      return {
        // NOT `TRAY.unreachableTitle`. `10a Offline` draws `Can't reach Proton Drive` on this panel
        // and the frame is answering a different question: the socket is what did not respond, and
        // Proton takes no part in that round trip. A daemon that is up and cannot reach Proton is
        // `failed` or `authExpired` below, both of which still say so. DEVIATIONS §95.
        headline: MAIN.notRunning,
        // No count clause to drop here — `notRunningSub` is a plain string, because the reply that
        // would carry the number is the one that never arrived. Same rule as `unreachableBody(null)`,
        // reached by having nothing to count rather than by discarding it.
        sub: MAIN.notRunningSub,
        // `retrying in 40s · last reached 13:58` is drawn and is not derivable: nothing in the reply
        // says when the next attempt is, and a daemon that is not running is not answering to be
        // asked. Omitted rather than filled (#213 covers the pass clock). §82h.
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

    case "queued":
      // The window's two sentences (`screens/main.js`), and no button: the one `firstRun` has opens the
      // window to choose folders, which a person looking at a list of folders has done.
      return {
        headline: v.waitingFor ? TRAY.waitingTitle(v.waitingFor) : TRAY.startingTitle,
        sub: v.waitingFor ? TRAY.waitingSub(v.folder, v.waitingFor) : TRAY.startingSub(v.folder),
        count: null,
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
  const node = renderCompactPanel({
    state: view.state,
    family: "tray",
    headline: view.headline,
    sub: view.sub ?? null,
    subMono: view.subMono ?? false,
    meta: view.meta ?? null,
    count: view.count ?? null,
    transfers: view.transfers ?? [],
    action: view.action ? { ...view.action, onClick: () => onSelect?.(view.action.id) } : null,
    menu: bindMenu(menuRowsOf(view), onSelect),
    pair: view.pair ?? null,
  });
  actionIds.set(node, view.action?.id ?? null);
  return node;
}

/**
 * The id the decision button was BUILT to send, per panel. The button's handler is bound at build time
 * and the panel is patched in place across polls, so an id that changes between two polls — `Review
 * them` moving from one folder's decisions to another's, with the panel still a needs-you panel — has
 * to be a shape change, or the patched panel would name one folder and send another.
 */
const actionIds = new WeakMap();

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
  if (actionIds.has(node) && actionIds.get(node) !== (view.action?.id ?? null)) return false;
  return updateCompactPanel(node, {
    state: view.state,
    // THE MENU IS PART OF WHAT IS ON SCREEN, and passing only `state` made a patch blind to it.
    // Three hero states share the `unreachable` PANEL form and two of them want different ROWS
    // (`MENU_STATE` above), so `failed` → `authExpired` between two polls patched the headline to
    // `Proton Drive is asking you to sign in again` over a menu still offering `Try again now` —
    // the row that cannot do what the sentence above it asks. Built without handlers because only
    // the ids are compared; a mismatch returns false and the caller renders a fresh panel. #246.
    menu: bindMenu(menuRowsOf(view)),
    pair: view.pair ?? null,
    headline: view.headline,
    sub: Array.isArray(view.sub) ? undefined : (view.sub ?? undefined),
    meta: view.meta ?? undefined,
    count: view.count ?? null,
    transfers: view.transfers ?? [],
  });
}
