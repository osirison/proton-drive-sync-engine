// The main screen (S1) — one hexagon, one sentence, and the seam.
//
// `03-main-screen.md`. It replaces the v1 Overview's nine competing regions with three states drawn
// on ONE skeleton, and the whole design rests on a property that is easy to lose in a rewrite:
// **the hexagon never moves.** The hero is a fixed 394px block that centres it, so settled, syncing,
// paused and unreachable all put the mark on the same 168 pixels and only what surrounds it changes.
// A hero sized to its content would drift by a line's height every time the sub-line changed.
//
// THREE THINGS THIS MODULE OWNS THAT NOTHING ELSE CAN.
//
//   · WHICH STATE IS SHOWING. `14-behaviour-and-state.md`: "needs-decision is additive, not
//     exclusive". Conflicts and withheld deletions coexist with settled, syncing and paused — the
//     hexagon carries the TRANSFER state and the band carries the decisions. `2a Needs you` is the
//     proof: it is `2a Syncing` with a band under it, down to the same gradient ids (DEVIATIONS §24
//     — there is no crimson hero, and the mark's numeral is 3 transfers, not 3 decisions).
//   · THE STRUCTURE, not just the styling. The screen is two or three SIBLINGS of the window root
//     (`shell.css`: "no wrapper element, so a screen's flex:1 block is a direct flex child exactly
//     as drawn"), which is why `renderMain` returns an array and `app.js` splices it in.
//   · WHEN TO PATCH AND WHEN TO REBUILD. The shell re-renders on every ~2s status poll. Rebuilding
//     the mark there restarts both travelling segments from 0% — the failure `updateHexagon` exists
//     to prevent — so a poll patches text and the numeral in place, and only a genuine state change
//     builds a new mark, behind the 220ms crossfade the design asks for.
//
// WHAT PHASE 1 CANNOT DRAW, all recorded in DEVIATIONS.md §63 with the issue that closes each:
// the settled sub-line's `12,480 files · 41.2 GB` (G7/#207), the footer's `386 MB sent · 1.1 GB
// received today` (G2/#191 — the shell draws the folder pair instead), the second and third transfer
// rows and the queued one (#211 — `SyncActivity` carries a single in-flight transfer), and the
// per-file progress bar (#98 — `bytes_total` and `bytes_done` are never both present, so no
// percentage exists to draw).

import { el } from "../ui/el.js";
import { MAIN, TRAY } from "../ui/copy.js";
import { bytes, clock, since } from "../ui/format.js";
import { renderHexagon, updateHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button } from "../ui/controls.js";
import { transferRow, eyebrow } from "../ui/rows.js";
import { attentionBand, bandButton } from "../ui/bands.js";
import { fid } from "../fixtures/frames.js";

/** The hero mark, at the one size this screen draws (`01-foundations.md` §6, `strokeForSize`). */
const HERO_SIZE = 168;

/**
 * The design's own cap: "Transfer rows appear in flight order, cap at ~6 visible with `+n more` in
 * mono if exceeded." It is also what keeps the columns inside the grid — they are drawn
 * `overflow:visible` like 21 of the 22 in-scope windows, so nothing clips a seventh row.
 */
const MAX_ROWS = 6;

// ------------------------------------------------------------------------------ the states ----

/**
 * Which hero the moment is in. Deliberately NOT the daemon's derived `state` verbatim: `running`
 * splits by whether a pass is actually in flight, and the two decision states are a question about
 * the queue rather than about the daemon.
 *
 * Order is the order the design puts them in. Unreachable outranks everything because it is the one
 * state where nothing else on the screen can be trusted to be current; paused outranks a decision
 * because the sentence "nothing will move until you resume" is true of the decisions too.
 */
export function heroStateOf({ daemonState, syncing, waiting }) {
  if (daemonState === "unreachable") return "unreachable";
  // BEFORE `syncing`, and before the settled fall-through, which is where it landed first and is a
  // false all-clear: a daemon whose Proton session has lapsed is reachable, reports nothing in
  // flight, and would otherwise draw `Everything is up to date` over a sync that cannot happen.
  // `routes.js` releases the onboarding latch on this state specifically so the main screen can
  // carry it — "we must actually hand off to the main screen's Re-authenticate action rather than
  // trap the user in the wizard" — so a fall-through is a broken hand-off, not just a missing state.
  if (daemonState === "authExpired") return "authExpired";
  if (daemonState === "paused") return "paused";
  if (syncing) return "syncing";
  // `14-behaviour-and-state.md`: "Only when nothing is transferring does the hexagon itself take the
  // decision form." No frame draws it at 168px — DEVIATIONS §24 measured the crimson mark as
  // existing only at ≤72px — so this is prose-normative and unverified by the gate. §67.
  if (waiting > 0) return "decision";
  return "settled";
}

/**
 * The screen's whole view model, derived once so the render and the patch cannot disagree about it.
 *
 * `activity` is the live pass (`SyncActivity`); `summary` is the plan the daemon published before
 * the transfers started, which is where `2 leaving, 1 arriving` comes from — `uploads` and
 * `downloads` on `last_plan_summary`, not a count of the rows drawn.
 */
export function mainView(props = {}) {
  const {
    daemonState = "unreachable",
    response = null,
    conflicts = [],
    deletions = [],
    localRoot = null,
    remoteRoot = null,
  } = props;

  const activity = response?.activity ?? null;
  const summary = response?.last_plan_summary ?? null;
  const pending = response?.pending_changes ?? null;
  const waiting = conflicts.length + deletions.length;
  const hero = heroStateOf({ daemonState, syncing: Boolean(response?.syncing), waiting });

  return {
    hero,
    waiting,
    conflicts,
    deletions,
    localRoot,
    remoteRoot,
    pending,
    summary,
    activity,
    lastSync: response?.last_sync_epoch_secs ?? null,
    // The numeral is the pending count — "the count in the hexagon is transfers, not decisions".
    numeral: hero === "syncing" ? pending : hero === "decision" ? waiting : null,
    transfers: transfersOf(activity),
  };
}

/**
 * The rows the columns draw, from the one transfer the reply can describe.
 *
 * An array of one, not a scalar, because the shape the screen wants is the list the design draws and
 * the shape the wire has is a single `Option<TransferActivity>`. Keeping the seam between them here
 * means the day #211 lands, this function grows a branch and nothing below it changes.
 *
 * `detail` is the size chip, and it is present on an upload and absent on a download — not a choice:
 * `bytes_total` is the local file's size for an upload, and a remote listing carries no size at all.
 */
function transfersOf(activity) {
  const t = activity?.transfer;
  if (!t) return [];
  return [
    {
      direction: t.direction === "download" ? "down" : "up",
      name: t.path,
      detail: t.bytes_total == null ? null : bytes(t.bytes_total),
      state: "active",
      // No percentage is computable from a reply that never carries both ends of one — see the
      // module header and `transferRow`'s own note. `null` means "no track", not "0%".
      progress: t.bytes_done != null && t.bytes_total != null ? t.bytes_done / t.bytes_total : null,
    },
  ];
}

// ------------------------------------------------------------------------------ the copy ----

/** The headline, per state. */
function headlineOf(v) {
  switch (v.hero) {
    case "syncing":
      return MAIN.syncing(v.pending ?? 0);
    case "paused":
      return MAIN.paused;
    case "unreachable":
      // The deck writes this sentence once, under the outage banner, and three surfaces quote it.
      // Reaching into `TRAY` for it is the copy module working: a second constant saying the same
      // thing here is exactly the drift `ui/copy.js` exists to prevent.
      return TRAY.unreachableTitle;
    case "authExpired":
      return MAIN.authExpired;
    case "decision":
      return MAIN.compact.needYou(v.waiting);
    default:
      return MAIN.settled;
  }
}

/**
 * The sub-line, per state — the one place Phase 1's data gaps are visible on this screen.
 *
 * Settled draws `last synced 2 minutes ago · 12,480 files · 41.2 GB` and the reply carries only the
 * timestamp: no command reports an index-wide file count or byte total (G7, #207). The clause is
 * omitted rather than faked, which is the fallback `14-behaviour-and-state.md` prescribes for a
 * missing capability, and `MAIN.settledSub` is left for the day #207 lands.
 */
function subOf(v) {
  switch (v.hero) {
    case "syncing":
      return MAIN.syncingSub(
        since(v.activity?.since_epoch_secs ?? v.lastSync),
        v.summary?.uploads ?? 0,
        v.summary?.downloads ?? 0,
      );
    case "paused":
      return MAIN.pausedSub(v.pending ?? 0, clock(v.lastSync));
    case "unreachable":
      return TRAY.unreachableBody(v.pending ?? 0);
    case "authExpired":
      return MAIN.authExpiredSub(v.pending ?? 0);
    default:
      return MAIN.settledSubTime(since(v.lastSync));
  }
}

/** The sub-line the syncing hero shows once something is waiting: `3 other changes are waiting…`. */
function syncingSubWithDecisions(v) {
  return v.waiting > 0 ? MAIN.otherWaiting(v.waiting) : subOf(v);
}

function subTextOf(v) {
  return v.hero === "syncing" ? syncingSubWithDecisions(v) : subOf(v);
}

// ------------------------------------------------------------------------------ the pieces ----

/**
 * Which of the five forms the mark takes. `10-tray.md`: **only five forms exist** — a solid filled
 * hexagon is not a state and must not be reintroduced.
 *
 * `authExpired` shares the struck mark with `unreachable`, which is the design's own grouping:
 * `11-notifications.md` puts "an outage, expired session, or full disk" behind one struck `#FF3B3B`
 * icon. Both mean *Proton is out of reach*; only the sentence underneath differs.
 */
const MARK_STATE = {
  syncing: "syncing",
  decision: "needsNumeral",
  paused: "paused",
  unreachable: "unreachable",
  authExpired: "unreachable",
  settled: "settled",
};

function heroMark(v) {
  const state = MARK_STATE[v.hero];
  if (!state) throw new Error(`main: no mark measured for hero state "${v.hero}"`);
  return renderHexagon({
    size: HERO_SIZE,
    state,
    // `masked` tracks whether the mark sits OVER the seam, not which state it is in (DEVIATIONS
    // §25) — so it follows the same condition the seam does, and `2a Settled`'s mark correctly
    // carries no fill.
    masked: v.hero === "syncing",
    numeral: v.numeral,
    class: "main-mark",
  });
}

/** One side of the seam: the direction's label, and the folder it stands for. */
function sideLabel(side, root) {
  const up = side === "local";
  return el(
    "div",
    { class: `main-side main-side-${side}` },
    eyebrow({
      tone: up ? "up" : "down",
      text: up ? MAIN.sideLocal : MAIN.sideRemote,
      align: up ? "start" : "end",
    }),
    el("div", { class: "main-side-path" }, root ?? "—"),
  );
}

/**
 * The hero's buttons.
 *
 * `Sync now` DISAPPEARS mid-sync — "it's meaningless mid-sync" — and that is the only reason the
 * count of buttons changes, which is why `updateMain` treats a change here as a rebuild of the row
 * rather than something to patch.
 */
function heroActions(v, handlers) {
  const buttons = [];
  if (v.hero === "paused") {
    buttons.push(action(MAIN.resume, "secondaryOutlined", handlers.onResume));
  } else if (v.hero === "unreachable" || v.hero === "authExpired") {
    // `Try again now` and not `11a Outage`'s `Sign in`: NOTHING IN THE COMMAND SURFACE SIGNS IN.
    // Re-authentication is `proton-drive login` in a terminal — the daemon reuses that CLI's keyring
    // session — so a `Sign in` button here would be a control with no action behind it, which is
    // worse than the honest one. Retrying is exactly right once the user has signed in elsewhere.
    // DEVIATIONS §67.
    buttons.push(action(TRAY.tryAgain, "secondaryOutlined", handlers.onSyncNow));
  } else {
    if (v.hero !== "syncing") buttons.push(action(MAIN.syncNow, "secondaryOutlined", handlers.onSyncNow));
    // Filled while syncing and outlined when settled: the mid-sync button sits ON the seam and its
    // own fill is what masks the hairline behind it (`seam.js` rule 3 — pass `surface:null` and keep
    // the button's fill). Both are the same colour role; only the surface differs.
    buttons.push(
      action(MAIN.pause, v.hero === "syncing" ? "secondaryFilled" : "quietOutlined", handlers.onPause),
    );
  }
  return el("div", { class: "main-actions" }, buttons);
}

function action(label, kind, onClick) {
  return button({ kind, size: "bar", fontSize: "13.5px", label, onClick: onClick ?? null });
}

/**
 * The attention band's rows, one per CATEGORY — never one per item. Two conflicts and a deletion
 * queue are one interruption; three stacked boxes would read as three (`bands.js`).
 *
 * A band routes and never acts: both buttons open the screen that owns the decision. `bands.js`
 * enforces that by only offering the `decision` kind, and it is the reason there is no `Approve` here.
 */
function bandItems(v, handlers) {
  const items = [];
  if (v.conflicts.length) {
    items.push({
      tone: "decision",
      title: MAIN.band.conflictTitle(v.conflicts.length),
      // The path is the first conflict's, which is the whole story only while there is one of them.
      // The deck gives no plural form for this clause; S2 owns the queue and can settle it there.
      note: MAIN.band.conflictSub(v.conflicts[0].original),
      action: bandButton({ label: MAIN.band.conflictAction, onClick: handlers.onConflicts }),
    });
  }
  if (v.deletions.length) {
    // `local` applies the delete on this computer — the permanent one. `remote` moves Proton's copy
    // to the Trash, which is recoverable. Column and direction name the same side from opposite
    // ends; `fixtures/deletions.js` documents the pairing at length.
    const permanent = v.deletions.filter((d) => d.direction === "local").length;
    items.push({
      tone: "destructive",
      title: MAIN.band.deletionTitle(v.deletions.length),
      note: MAIN.band.deletionSub(permanent, v.deletions.length - permanent),
      action: bandButton({ label: MAIN.band.deletionAction, onClick: handlers.onDeletions }),
    });
  }
  return items;
}

// ------------------------------------------------------------------------------ mount ----

/**
 * What is currently on screen, so a poll can patch it. Module-level because exactly one main screen
 * exists at a time — the same reasoning `app.js`'s `dom` cache is built on, one level in.
 */
let view = null;

/** Build the screen. Returns the window-root siblings, in order. */
export function renderMain(props = {}) {
  const v = mainView(props);
  const handlers = props.handlers ?? {};

  const hero = el("div", { class: "main-hero" });
  const mark = heroMark(v);
  const headline = el("div", { class: "main-headline" }, headlineOf(v));
  const sub = el("div", { class: "main-sub" }, subTextOf(v));
  const glow = el("div", { class: "hex-glow", "aria-hidden": "true" });
  const seam = renderSeam({ site: seamSiteOf(v) });
  const sides = [sideLabel("local", v.localRoot), sideLabel("remote", v.remoteRoot)];

  // The seam is the FIRST child and nothing wraps it: everything positioned after it in the DOM
  // paints over it, which is half of how the masks work (`seam.js` rule 3).
  if (v.hero === "syncing") hero.append(seam, ...sides);
  else if (v.hero === "settled") hero.append(glow);
  hero.append(mark, headline, sub);
  const actions = heroActions(v, handlers);
  hero.append(actions);
  applyMasks(v, { headline, sub });

  const columns = el("div", { class: "main-columns" });
  const spacer = el("div", { class: "main-spacer" });
  const bandWrap = el("div", { class: "main-band-wrap" });

  view = { v, handlers, hero, mark, headline, sub, glow, seam, sides, actions, columns, spacer, bandWrap };
  fillColumns(v);
  fillBand(v, handlers);
  stampFids(v);
  return blocksOf(v);
}

/**
 * Patch what is on screen. Returns the (possibly reordered) blocks so the shell can splice them, or
 * `null` when nothing is mounted.
 *
 * NOTHING HERE REBUILDS THE MARK UNLESS ITS STATE CHANGED. That is the whole point of the function:
 * `2a Syncing` → `2a Needs you` is a band appearing under an unchanged hexagon, and re-rendering it
 * would restart `hexup` and `hexdn` from 0% twice a second.
 */
export function updateMain(props = {}) {
  if (!view) return null;
  const next = mainView(props);
  const handlers = props.handlers ?? view.handlers;
  const prev = view.v;

  if (next.hero !== prev.hero) {
    crossfadeMark(next);
    // The blocks the hero grows and loses with its state. Everything is prepended, so the mark, the
    // headline and the sub-line are never re-parented — a moved node restarts its own animations,
    // which is the failure this whole update path exists to avoid.
    if (next.hero === "syncing") {
      // REBUILT, never re-attached. The seam held here was built for the site the screen mounted in,
      // and entering syncing while a decision is already waiting needs the SHORT one — re-attaching
      // the mount-time `mainHero` runs a 150px overhang straight into the attention band, which is
      // the rule-2 violation `auditSeams` exists to catch and which no frame exercises, because a
      // frame is one rendering and this is a transition between two.
      view.seam = renderSeam({ site: seamSiteOf(next) });
      view.hero.prepend(view.seam, ...view.sides);
    } else {
      view.seam.remove();
      for (const side of view.sides) side.remove();
    }
    if (next.hero === "settled") view.hero.prepend(view.glow);
    else view.glow.remove();
    const actions = heroActions(next, handlers);
    view.actions.replaceWith(actions);
    view.actions = actions;
  } else if (next.hero === "syncing" && bandShowing(next) !== bandShowing(prev)) {
    // The seam shortens to stop above the band and lengthens again when it goes: two drawn sites,
    // not one site with a computed height (`seam.js` SEAM_SITES).
    const seam = renderSeam({ site: seamSiteOf(next) });
    view.seam.replaceWith(seam);
    view.seam = seam;
  }

  updateHexagon(view.mark, { numeral: next.numeral });
  setText(view.headline, headlineOf(next));
  setText(view.sub, subTextOf(next));
  view.sides[0].querySelector(".main-side-path").textContent = next.localRoot ?? "—";
  view.sides[1].querySelector(".main-side-path").textContent = next.remoteRoot ?? "—";
  applyMasks(next, view);

  view.v = next;
  view.handlers = handlers;
  fillColumns(next);
  fillBand(next, handlers, bandShowing(next) && !bandShowing(prev));
  stampFids(next);
  return blocksOf(next);
}

/** Forget the mounted screen. The shell removes the nodes; this drops the references to them. */
export function unmountMain() {
  endFade?.();
  view = null;
}

// ------------------------------------------------------------------------------ internals ----

const bandShowing = (v) => v.waiting > 0;
const seamSiteOf = (v) => (bandShowing(v) ? "mainHeroAttention" : "mainHero");

function blocksOf(v) {
  const blocks = [view.hero, v.hero === "syncing" ? view.columns : view.spacer];
  if (bandShowing(v)) blocks.push(view.bandWrap);
  return blocks;
}

/** Only write when it changed: an unchanged assignment still invalidates layout for the whole line. */
function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

/**
 * The seam mask, applied and removed with the seam itself.
 *
 * Both padding values are the frame's, not a tier: the 32px headline takes 18px and the 13px mono
 * sub-line takes 16px with 2px above and below — `seam.js` §37 records that no function of font-size
 * reproduces the set, so a screen quotes its own frame.
 */
function applyMasks(v, nodes) {
  const masked = v.hero === "syncing";
  seamMask(nodes.headline, masked ? { pad: 18 } : { pad: null, surface: null });
  seamMask(nodes.sub, masked ? { pad: 16, padY: 2 } : { pad: null, surface: null });
  if (!masked) {
    // `seamMask` only ever sets, so clearing is the caller's job. `position` stays: every hero
    // element is `position:relative` in `2a Settled` too, to stack over the glow.
    for (const node of [nodes.headline, nodes.sub]) {
      node.style.removeProperty("background");
      node.style.removeProperty("padding");
    }
  }
}

/**
 * Swap the mark for one in the new state, over 220ms, WITHOUT MOVING IT.
 *
 * A true crossfade needs both marks on screen at once, and the incoming one must not take space or
 * the hero reflows and the hexagon jumps — which is the one thing `03-main-screen.md` says it must
 * never do, and a line item on the design's own acceptance checklist. So the incoming mark is
 * absolutely positioned over the outgoing one's exact box for the length of the fade and then drops
 * back into flow, at which point the outgoing one is removed.
 *
 * A `transition` rather than an `animation`: `animation-name` and its three friends are asserted
 * properties, and a fade declared as an animation would sit on the mark at rest and fail every frame
 * that maps it. A transition is invisible to a gate that reads a static tree, which is exactly right
 * for something that only exists between two states.
 */
let fadeTimer = null;
let endFade = null;

function crossfadeMark(next) {
  // A second state change inside 220ms lands here with a fade still running. Settle the first one
  // now rather than layering two: the alternative leaves an absolutely-positioned mark that the
  // next call adopts as its outgoing node and never puts back into flow.
  endFade?.();

  const outgoing = view.mark;
  const incoming = heroMark(next);
  // Pinned to the outgoing mark's exact box. An SVG element has no `offsetTop`, so this is measured
  // rather than read — and the hero has no border, so its border box and its padding box share an
  // origin and the two rects subtract cleanly.
  const from = outgoing.getBoundingClientRect();
  const hero = view.hero.getBoundingClientRect();
  incoming.classList.add("is-entering");
  incoming.style.top = `${from.top - hero.top}px`;
  incoming.style.left = `${from.left - hero.left}px`;
  outgoing.after(incoming);
  view.mark = incoming;

  // The incoming mark stays OUT OF FLOW for the whole fade. Dropping it in while the outgoing one is
  // still there would put two 168px marks in a centring column and move the hexagon — the one thing
  // this screen must never do.
  endFade = () => {
    clearTimeout(fadeTimer);
    fadeTimer = null;
    endFade = null;
    outgoing.remove();
    incoming.classList.remove("is-entering");
    incoming.style.removeProperty("top");
    incoming.style.removeProperty("left");
    incoming.style.removeProperty("opacity");
  };

  // Two frames: one for the engine to accept the starting opacity, one to start the transition from
  // it. Setting both in the same frame is a style change with nothing to transition from.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      incoming.style.opacity = "1";
      outgoing.style.opacity = "0";
    }),
  );
  incoming.addEventListener("transitionend", () => endFade?.(), { once: true });
  // Under `prefers-reduced-motion` `main.css` drops the transition and `transitionend` never fires.
  // Without this the outgoing mark would sit over the new one forever.
  fadeTimer = setTimeout(() => endFade?.(), 400);
}

/**
 * What a list is made of, as a string — so a poll that changes nothing rebuilds nothing.
 *
 * THIS IS NOT AN OPTIMISATION AND THE COMMENT ON `app.js`'s `dom` cache says why. The shell
 * re-renders every ~2 seconds; `replaceChildren` on a block a user has tabbed into drops focus to
 * `<body>` inside two ticks. The band holds `Compare` and `Review` — the two controls this screen
 * exists to offer — so rebuilding it on a timer makes them unreachable from the keyboard, which
 * `14-behaviour-and-state.md` requires "because this is a desktop app". Same hazard the shell hit,
 * one layer in, and the same answer: rebuild on a change, never on a tick.
 */
const signature = (parts) => parts.join("");

/**
 * The two columns and their rows. Rebuilt when the set of rows changes, which is a row's whole
 * identity here — a file name, a direction, a size and a fraction. There is no per-row animation to
 * preserve, so nothing finer than "did the list change" is needed.
 */
function fillColumns(v) {
  const sig =
    v.hero !== "syncing"
      ? ""
      : signature(v.transfers.map((t) => `${t.direction}|${t.name}|${t.detail}|${t.progress}`));
  if (view.columnsSig === sig) return;
  view.columnsSig = sig;

  if (v.hero !== "syncing") {
    view.columns.replaceChildren();
    return;
  }
  const left = el("div", { class: "main-column main-column-left" });
  const right = el("div", { class: "main-column main-column-right" });
  const shown = v.transfers.slice(0, MAX_ROWS);
  for (const t of shown) (t.direction === "up" ? left : right).append(transferRow(t));
  const hidden = v.transfers.length - shown.length;
  if (hidden > 0) right.append(el("div", { class: "main-more" }, MAIN.andMore(hidden)));
  view.columns.replaceChildren(left, right);
}

// The two columns are always both drawn, even when one is empty: the grid's 1fr 1fr is what puts the
// seam between them, and a single child would centre one column across the whole width.

/**
 * The band, and the one animation on this screen that must not exist on a first render.
 *
 * `arriving` is true only when a decision turns up on a screen that was already showing — the
 * design's own trigger ("New decision arrives → attention band slides in"). Mounting straight into
 * the banded state is not an arrival, and declaring the slide unconditionally would leave the
 * fidelity gate measuring `opacity:0` on a node the frame records at 1, because the harness freezes
 * every animation at its first keyframe. See `main.css`.
 */
function fillBand(v, handlers, arriving = false) {
  const items = bandItems(v, handlers);
  const sig = signature(items.map((i) => `${i.title}|${i.note}`));
  if (view.bandSig === sig) return;
  view.bandSig = sig;

  if (!items.length) {
    view.bandWrap.replaceChildren();
    view.bandWrap.classList.remove("is-entering");
    return;
  }
  view.bandWrap.replaceChildren(attentionBand({ items }));
  if (!arriving) return;
  // Removed and re-added around a forced reflow, not simply added: adding a class an element already
  // carries does not restart its animation, so a SECOND decision arriving after a first would appear
  // without sliding. It can already be there — under `prefers-reduced-motion` the animation is `none`
  // and `animationend` never fires to take it off.
  view.bandWrap.classList.remove("is-entering");
  void view.bandWrap.offsetWidth;
  view.bandWrap.classList.add("is-entering");
  view.bandWrap.addEventListener("animationend", () => view.bandWrap.classList.remove("is-entering"), {
    once: true,
  });
}

/**
 * Hand every mapped node its `data-fid`. A no-op in the live app — `fid()` only stamps when a
 * `?frame=` label is selected and that frame declares the slot — so this costs nothing at runtime
 * and is the only thing that makes the style gate able to see this screen at all.
 *
 * Stamped after every rebuild rather than once at mount, because the columns and the band are
 * replaced wholesale and a fresh node carries no attribute.
 */
function stampFids(v) {
  fid(view.hero, "hero");
  fid(view.mark, "hexagon");
  for (const [i, path] of [...view.mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
  fid(view.mark.querySelector("text"), "hexNumeral");
  const defs = view.mark.querySelector("defs");
  if (defs) {
    fid(defs, "hexDefs");
    for (const [i, grad] of [...defs.children].entries()) {
      fid(grad, "hexGradient", i);
      for (const [j, stop] of [...grad.children].entries()) fid(stop, "hexStop", i, j);
    }
  }
  fid(view.headline, "headline");
  fid(view.sub, "sub");
  fid(view.actions, "actions");
  for (const [i, btn] of [...view.actions.children].entries()) fid(btn, "action", i);

  if (v.hero === "syncing") {
    fid(view.seam, "seam");
    fid(view.sides[0], "sideLocal");
    fid(view.sides[0].children[0], "sideLocalLabel");
    fid(view.sides[0].children[1], "sideLocalPath");
    fid(view.sides[1], "sideRemote");
    fid(view.sides[1].children[0], "sideRemoteLabel");
    fid(view.sides[1].children[1], "sideRemotePath");
    fid(view.columns, "columns");
    fid(view.columns.children[0], "columnLeft");
    fid(view.columns.children[1], "columnRight");
    // ONE ROW IS MAPPED, and which one is the fixture's business rather than this loop's: the
    // frames draw two columns of rows and Phase 1 fills exactly one of them (#211). `mainFids`
    // names the row the frame puts first in whichever column the in-flight transfer landed in.
    const row = view.columns.querySelector(".transfer-row");
    if (row) {
      fid(row, "transferRow");
      const body = row.querySelector(".transfer-body");
      fid(body, "transferBody");
      fid(row.querySelector(".transfer-name"), "transferName");
      fid(row.querySelector(".transfer-detail"), "transferDetail");
      fid(row.querySelector(".transfer-arrow"), "transferArrow");
      fid(row.querySelector(".transfer-track"), "transferTrack");
      fid(row.querySelector(".transfer-fill"), "transferFill");
    }
  } else {
    fid(view.glow, "glow");
    fid(view.spacer, "spacer");
  }

  if (bandShowing(v)) {
    fid(view.bandWrap, "bandWrap");
    const band = view.bandWrap.firstElementChild;
    fid(band, "band");
    for (const [i, item] of [...(band?.children ?? [])].entries()) {
      fid(item, "bandItem", i);
      fid(item.querySelector(".dot"), "bandDot", i);
      const body = item.querySelector(".band-item-body");
      fid(body, "bandBody", i);
      fid(body?.children[0], "bandTitle", i);
      fid(body?.children[1], "bandNote", i);
      fid(item.querySelector("button"), "bandAction", i);
    }
  }
}
