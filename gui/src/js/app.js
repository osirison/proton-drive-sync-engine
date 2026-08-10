// The app shell and router (F4). Replaces the v1 build's 214px sidebar, title bar and tab registry
// with the design-v2 skeleton: a 52px header, the screen, and either the four doors or a footer
// action bar. 02-shell.md — "Every window shares this skeleton. Build it once."
//
// WHAT THIS COMMIT DELETES, and why it has to. app.css, components.css and legacy-tokens.css go,
// which F1 recorded in legacy-tokens.css's own header as F4's job. Every v1 screen module is styled
// entirely by those files — 2 to 60 class references each — so they cannot outlive them: left in
// place they would render as unstyled markup, which is worse than an honest "not built yet". They
// are replaced by placeholders, one per route, each naming the S-task that fills it in.
//
// The route table lives in routes.js so an S-task edits its own screen module and one line there,
// never this file.

import { api } from "./api.js";
import * as store from "./store.js";
import { ROUTES, FOOTER_ORDER, isOverlay, isDialog, resolveRoute, nextOnboardingLatch } from "./routes.js";
import { el } from "./ui/el.js";
import {
  renderHeader,
  updateHeader,
  renderFooterNav,
  updateFooterNav,
  renderActionBar,
  screenPlaceholder,
} from "./ui/chrome.js";
import { dialog, dialogHead, focusTrap } from "./ui/dialog.js";
import { renderCompactPanel, trayMenu } from "./ui/compact.js";
import { ACTIVITY, CHROME } from "./ui/copy.js";
import { clock, since } from "./ui/format.js";
import { renderMain, updateMain, unmountMain } from "./screens/main.js";
import { renderConflicts, advanceAfter, skipTo } from "./screens/conflicts.js";
import {
  renderDeletions,
  updateDeletions,
  unmountDeletions,
  armedItem,
  itemKey,
  BULK_KEY,
} from "./screens/deletions.js";
import {
  renderPlan,
  updatePlan,
  unmountPlan,
  renderPlanBar,
  updatePlanBar,
  footerKindOf,
} from "./screens/plan.js";
import {
  renderActivity,
  renderDetailsBody,
  renderNeverSyncedBody,
  renderFilePendingBody,
  footerVariantOf,
  neverSyncedFrom,
  passesSummaryOf,
} from "./screens/activity.js";
import { severityOf } from "./ui/rows.js";
import { activeFixture } from "./fixtures/frames.js";
import { mountPreview, applyPreviewTheme } from "./fixtures/preview.js";

// ---- shell state ----
let route = "main"; // the root or door currently showing
// TWO OVERLAY LAYERS, because F5 measured that "overlay" was two things (routes.js `presentation`).
// A screen overlay REPLACES the body; a dialog FLOATS over whatever body is showing. One slot for
// both cannot represent a dialog over a screen overlay — and every screen overlay draws the four
// doors, so opening `Details` from the Deletions screen is a click away. Collapsed into one slot it
// silently dropped the user back to the door underneath, which is the exact "lose your place"
// failure F4's note on the `details` route warns about. DEVIATIONS §57b.
let screenStack = []; // [{ id, back }] — body-replacing overlays, innermost last
let dialogOverlay = null; // the floating one, at most one at a time
let dialogReturn = null; // where to send focus when it closes — see focusKeyOf
let menuOpen = false;
let configInfo = null;
let configLoaded = false; // has the GUI config file been read at least once (even if empty)?
let statusPolled = false; // has at least one get_status round trip completed (success or failure)?
let onboardingLatch = false; // sticky: are we in the first-run onboarding takeover? (see routes.js)
let pollTimer = null;
let lastConflictScan = 0;

// ---- theme ----
// The toggle moved out of the title bar and into the ⋯ menu (02-shell.md). Persistence is
// unchanged: an explicit choice beats the media query in both directions, which is why tokens.css
// declares the light palette twice.
function initTheme() {
  const saved = localStorage.getItem("theme");
  if (saved === "light" || saved === "dark") document.documentElement.setAttribute("data-theme", saved);
  // `?theme=` beats the stored choice, and writes nothing back — it is a preview override for
  // looking at a light frame on a dark machine, not a decision the user made. Applied last so it
  // wins; see fixtures/preview.js for why it is never inferred from a `12a` label.
  applyPreviewTheme();
}
function currentTheme() {
  return (
    document.documentElement.getAttribute("data-theme") ||
    (window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark")
  );
}
function toggleTheme() {
  const next = currentTheme() === "light" ? "dark" : "light";
  document.documentElement.setAttribute("data-theme", next);
  localStorage.setItem("theme", next);
  render();
}

// ---- the status chip ----

/**
 * Which of the six chip variants this moment is in, and the mono string beside it.
 *
 * `2a Needs you` settles both halves of this, and S1 is what made the second half visible. Its chip
 * reads `3 waiting` with the 1px decision RING while a transfer is also in flight — so a decision
 * outranks transfer — and its band draws ONE conflict and TWO deletions, so the 3 is the SUM of the
 * two queues and the ring wins over the filled dot in their company.
 *
 * That falsifies half of DEVIATIONS §44, which recorded "nothing draws decisions and deletions at
 * once" and chose the deletions-first order from it. The frame does draw both at once; the earlier
 * reading came from the fixture, which pinned three conflicts and an empty deletion queue against a
 * band whose second row says `Two deletions are waiting on you`. Deletions still win when they are
 * ALONE, which is what `4a Deletions` draws. §64.
 *
 * `paused`, `unreachable` and `authExpired` have NO drawn chip anywhere in the prototype. They take
 * the quiet form with their own text rather than an invented colour — the hexagon and the main
 * screen carry those states, which is where the design puts them.
 */
function chipFor() {
  if (onboardingLatch) return { variant: "step", text: CHROME.chips.step(1) };
  // The plan screen owns the chip while it is open, outranking a waiting decision: `06-plan.md`
  // says it reads `rehearsal · nothing has changed` for the whole visit.
  if (activeRoute() === "plan") return { variant: "rehearsal", text: CHROME.chips.rehearsal };

  const state = store.select.daemonState();
  const decisions = store.select.unresolvedConflictCount();
  // `visibleDeletions()`, NOT the raw store. The chip, the main screen's attention band and the
  // deletions screen are three renderings of one sentence — "this many things are waiting on you" —
  // and the daemon keeps an answered deletion in `pending_deletions` until a pass consumes it
  // (for a KEPT one, for ever: #224). Counted from the store, this chip reads `2 waiting` with the
  // destructive dot while the screen under it says `Nothing waiting to be deleted`. One filtered
  // view feeds all three, so they cannot disagree.
  const deletions = visibleDeletions().length;

  if (decisions + deletions > 0) {
    return {
      variant: decisions > 0 ? "decisions" : "deletions",
      text: CHROME.chips.waiting(decisions + deletions),
    };
  }
  if (state === "running") return { variant: "syncing", text: "syncing" };
  if (state === "paused") return { variant: "idle", text: "paused" };
  if (state === "unreachable") return { variant: "idle", text: "unreachable" };
  if (state === "authExpired") return { variant: "idle", text: "sign-in expired" };
  if (state === "firstRun") return { variant: "idle", text: "first run" };
  return { variant: "idle", text: "idle" };
}

// ---- routing ----

function navigate(id) {
  if (!ROUTES[id]) throw new Error(`app: no route "${id}"`);
  if (isOverlay(id)) return openOverlay(id);
  // IMPLEMENTATION-PLAN §3.3's assumption, and on Settings and Plan there is no other way back:
  // clicking the door you are already on returns to the main screen.
  const was = route;
  route = route === id ? "main" : id;
  // Leaving discards the plan; arriving is handled at the mount in `render()`, because a door is not
  // the only way in. Moving the token here also drops an in-flight rehearsal's reply where it lands,
  // rather than writing it into state the next visit throws away.
  if (was === "plan") resetPlanScreen();
  // Same rule for the activity screen: a tab, a half-typed path and a filesystem walk are all
  // per-visit, so leaving drops them rather than carrying them into a session that may be hours old.
  if (was === "activity") resetActivityScreen();
  // A door leaves everything stacked over the old screen behind — both layers, not just the top
  // one. Cleared directly rather than by popping: the door itself keeps focus, so there is no
  // return target to honour.
  screenStack = [];
  dialogOverlay = null;
  dialogReturn = null;
  render();
}

/**
 * A stable way to find the control that opened an overlay, AFTER the overlay has closed.
 *
 * Holding the element itself is not enough and was the bug Copilot caught: opening an overlay
 * replaces the body, so an opener that lived there is disconnected by the time we want to focus it
 * and `isConnected` is false — focus silently never returns. A key survives the rebuild because it
 * is looked up again in the new tree.
 *
 * The element is kept as a fallback for openers with no key that happen to survive (a door button,
 * now that the footer is patched rather than rebuilt).
 */
function focusKeyOf(node) {
  if (!(node instanceof HTMLElement)) return null;
  if (node.dataset.focusKey) return node.dataset.focusKey;
  if (node.dataset.route) return `[data-route="${node.dataset.route}"]`;
  return null;
}

function openOverlay(id, opener = null) {
  const back = { key: focusKeyOf(opener ?? document.activeElement), node: opener ?? document.activeElement };
  if (isDialog(id)) {
    dialogOverlay = id;
    dialogReturn = back;
  } else {
    // A dialog belonged to the screen it was opened over; moving to a different screen closes it
    // rather than leaving it floating above something it was never about.
    dialogOverlay = null;
    dialogReturn = null;
    screenStack.push({ id, back });
  }
  // Conflicts is entered fresh every time: the queue starts at the top, and the "what you settled"
  // tally that the cleared state reads is a claim about THIS visit. See resetConflictScreen.
  if (id === "conflicts") resetConflictScreen();
  // Deletions is entered on the QUEUE, always. A confirmation left armed from a previous visit
  // would put a full-window "Delete photos/2019 from this computer?" in front of somebody who has
  // just clicked a notification — a question they did not ask, about the most destructive thing the
  // app can do. The decisions themselves are not reset; only what is open. See deletionsDecided.
  if (id === "deletions") deletionArmed = null;
  render();
}

/** Focus returns to whatever opened the layer. Re-queried after the render, because the node it
 *  opened from may have been rebuilt in the meantime — see focusKeyOf. */
function restoreFocus(back) {
  const target =
    (back?.key && document.querySelector(back.key)) || (back?.node?.isConnected ? back.node : null);
  target?.focus();
}

/**
 * Move focus onto the control that replaced the one the user was standing on: a body swap leaves
 * focus on `<body>`, out of reach of the keyboard without tabbing from the top.
 *
 * Call from a control's own handler only. Taking focus on mount would draw a focus ring on every
 * fixture, which the fidelity gate renders cold.
 */
function focusAfterSwap(selector) {
  // A microtask: the caller has just re-rendered and `setBody` may still be inserting the target.
  // `focus()` on a detached element is a silent no-op.
  queueMicrotask(() => document.querySelector(selector)?.focus());
}

/** Close the topmost layer. The dialog is always above the screen stack, so it goes first. */
function closeOverlay() {
  if (dialogOverlay) {
    dialogOverlay = null;
    const back = dialogReturn;
    dialogReturn = null;
    render();
    restoreFocus(back);
    return true;
  }
  const top = screenStack[screenStack.length - 1];
  if (!top) return false;
  // The takeover is not dismissible: it is entered by the latch and left by the daemon coming up.
  // Defensive — the latch drives onboarding without ever putting it on this stack.
  if (ROUTES[top.id]?.takeover) return false;
  screenStack.pop();
  render();
  restoreFocus(top.back);
  return true;
}

// ---- keyboard map (02-shell.md / 14-behaviour-and-state.md) ----

/**
 * The shell owns the shortcuts that are about the WINDOW; the ones that act on a screen's own
 * controls are re-broadcast as events so the screen that owns them can listen without the shell
 * importing it. `Ctrl F` and `Ctrl S` reach a lookup field and a Save button that F4 does not
 * build; dispatching is how they stay wired now and keep working when S5/S6 land.
 */
function onKeydown(e) {
  const ctrl = e.ctrlKey || e.metaKey;

  if (e.key === "Escape") {
    if (menuOpen) {
      menuOpen = false;
      render();
      e.preventDefault();
      return;
    }
    // `Press Esc to cancel.` — and it has to be taken HERE, ahead of `closeOverlay`. The armed
    // confirmation is a body of the deletions screen rather than a route (see routes.js), so the
    // topmost thing on the screen stack is Deletions itself: left to the line below, Esc would
    // dismiss the whole queue instead of the confirmation over it, which is the frame's own caption
    // doing the opposite of what it says. Cancelling leaves the queue exactly where it was.
    // Asked of the QUEUE, not of the flag. The takeover can stop showing without anything clearing
    // `deletionArmed` — the pass applies the deletion, another client approves it, the daemon
    // restarts and publishes an empty snapshot — and a stale flag would swallow the Esc that was
    // meant to leave the screen, so the first press would do nothing visible and a second would be
    // needed. `armedItem` is the same question `bodyOf` asks to decide what is drawn.
    if (activeRoute() === "deletions" && armedItem(visibleDeletions(), deletionArmed)) {
      deletionArmed = null;
      render();
      e.preventDefault();
      return;
    }
    // Esc also cancels a confirmation, which is a screen's business — it gets the event only if no
    // overlay took it.
    if (closeOverlay()) e.preventDefault();
    else document.dispatchEvent(new CustomEvent("shell:cancel"));
    return;
  }

  if (ctrl && e.key.toLowerCase() === "f") {
    e.preventDefault();
    if (route !== "activity") navigate("activity");
    document.dispatchEvent(new CustomEvent("shell:focus-lookup"));
    return;
  }
  if (ctrl && e.key === ",") {
    e.preventDefault();
    navigate("settings");
    return;
  }
  if (ctrl && e.key.toLowerCase() === "s") {
    e.preventDefault();
    document.dispatchEvent(new CustomEvent("shell:save"));
    return;
  }
  if (ctrl && e.key.toLowerCase() === "w") {
    e.preventDefault();
    api.closeWindow();
    return;
  }
  if (ctrl && e.key.toLowerCase() === "q") {
    e.preventDefault();
    api.quitApp();
    return;
  }
  // ← → move between conflicts. Not swallowed when a control has focus: arrows inside a text field,
  // a select or a slider mean what they normally mean.
  if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
    const t = e.target;
    if (t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)))
      return;
    document.dispatchEvent(
      new CustomEvent("shell:step", { detail: { delta: e.key === "ArrowLeft" ? -1 : 1 } }),
    );
  }
}

// The event F4 defined and nothing consumed until S2. Kept as an event rather than a direct call so
// the key handler stays ignorant of which screen is up. F4 predicted S3's queue would step the same
// way and it does not: the deletions screen shows every waiting item at once, so there is no
// position to move, and neither `05-deletions.md` nor `14-behaviour-and-state.md` asks for one.
// The listener stays single-consumer until a screen that pages needs it.
document.addEventListener("shell:step", (e) => {
  if (activeRoute() !== "conflicts") return;
  stepConflict(e.detail?.delta ?? 0);
});

// F4's other unconsumed event, and S5 is what it was waiting for. `Ctrl F` is drawn as a hint in
// the empty lookup field on `7a Activity quiet`, so it was a promise the app had not yet kept.
//
// After a frame, not immediately: the key handler navigates to the activity door first, and the
// input does not exist until that render has run.
document.addEventListener("shell:focus-lookup", () => {
  if (activeRoute() !== "activity") return;
  queueMicrotask(() => {
    const node = activityInputRef.node;
    if (!node) return;
    node.focus();
    // To the END of whatever is already there. Focusing a contenteditable leaves the caret at
    // offset 0, so Ctrl F on a field holding a path would type into the middle of it.
    putCaret(node, null);
  });
});

/**
 * Which screen the body is showing — the innermost overlay, or the door you are on.
 *
 * A FIXTURE MAY NAME ITS OWN ROUTE, and until S2 nothing needed to. Every mapped frame so far was
 * either the main screen (which is the default) or a compact panel (which `mountFramePanel`
 * intercepts before this is reached), so selecting `?frame=3a Conflict` left the app on `main` and
 * quietly drew the main screen against the conflicts fixture. That is not a blank screen — it is
 * WORSE than one, because `fid()` keys by slot NAME: `main.js` stamps `fid(view.mark, "hexagon")`,
 * the conflicts table also has a `hexagon`, and the gate ends up comparing the 168px main hero
 * against a 44px on-seam mark and reporting it as a size failure on a screen nobody rendered.
 */
function activeRoute() {
  if (onboardingLatch) return "onboarding";
  return activeFixture()?.route ?? screenStack[screenStack.length - 1]?.id ?? route;
}

// ---- the ⋯ menu ----

// No frame draws this menu open (DEVIATIONS.md §45), so its contents come from 02-shell.md's one
// sentence: the theme toggle moved here from the title bar.
function renderMenu() {
  if (!menuOpen) return null;
  const light = currentTheme() === "light";
  return el(
    "div",
    { class: "menu-popover", role: "menu" },
    el(
      "button",
      { class: "menu-item", role: "menuitem", onClick: toggleTheme },
      light ? "Dark theme" : "Light theme",
    ),
  );
}

// ---- render ----

// Rendered nodes, held across polls. THE SHELL IS BUILT ONCE AND PATCHED, never rebuilt on a timer.
//
// This is not an optimisation. The shell re-renders on every status poll (~2 s), and a rebuild
// destroys whatever the user is standing on: tab to a door and focus drops to `<body>` inside 1.2
// seconds — measured on the first version of this file, not theorised. `14-behaviour-and-state.md`
// is explicit that every control must be keyboard-reachable "because this is a desktop app", and a
// window that silently discards focus twice a second is not.
//
// It is also the constraint F2 wrote down. `updateHexagon`'s comment says the screens "must call
// this rather than re-rendering", because replaceChildren restarts both CSS animations from 0% —
// the v1 spinner bug. The chip's `blip` dot is the same hazard one primitive earlier, and every
// hexagon S1 puts on the main screen will be too.
const dom = {
  header: null,
  // A SCREEN IS SEVERAL SIBLINGS, not one node. `shell.css` says why: the frames put the header, the
  // screen's own blocks and the footer as direct children of the 1040×764 root, "so a screen's
  // flex:1 block is a direct flex child exactly as drawn". S1 is the first screen with more than one
  // — a 394px hero, a flex:1 tail and, when something is waiting, a band — and wrapping the three in
  // a container would put a flex context between them and the window that the design does not have.
  bodyNodes: [],
  footer: null,
  footerKind: null,
  // Which screen an action bar belongs to. Two screens' bars are both `actionBar` and are not
  // interchangeable — see the footer block in render().
  footerOwner: null,
  bodyRoute: null,
  // The dialog layer (F5). Keyed like the body, and for a harder reason: the armed deletion's
  // typed-`DELETE` field CLEARS ON BLUR by design, so a layer rebuilt on the ~2s poll would destroy
  // the field mid-word and make the gate impossible to finish. Same failure class as the focus loss
  // this cache was built for, one level in.
  dialog: null,
  dialogRoute: null,
  dialogSignature: null, // what the mounted dialog's body was built from — see the layer below
  dialogDetach: null,
  panel: null,
  // The preview's own pages (F9). Latched for the same reason `panel` is, and it is not hypothetical:
  // `render()` returning early does not stop the poll — `main()` starts it unconditionally and
  // `store.subscribe(render)` re-enters here on every reply — so without this the frame index rebuilt
  // its whole list every ~2 s and a tabbed-to link lost focus, which is the failure this cache exists
  // for. Its content cannot change either: it is derived from the registry, which is a constant.
  preview: false,
};

/**
 * The compact frames' preview route (F6).
 *
 * `?frame=2a Compact syncing` renders the 362px panel ALONE, because that is what the frame is —
 * a panel, not a 1040 window with a panel somewhere in it. Without this the harness opens the shell,
 * finds no `data-fid` and files eight frames under "screen not built yet", so the component would
 * ship with nothing comparing it to what it was measured from.
 *
 * Mounted ONCE and then left alone. A fixture cannot change, and rebuilding on the ~2s poll would
 * restart both hexagon segments from 0% — the failure `updateHexagon` exists to prevent, and the one
 * `updateCompactPanel` gives the real screens a way around.
 *
 * F9 generalises this to every frame; the shape it needs is the same one F4 established when it
 * hand-wrote the first three fixtures into a file nominally belonging to that task.
 */
function mountFramePanel(root) {
  if (dom.panel) return true;
  const spec = activeFixture()?.panel;
  if (!spec) return false;
  dom.panel = renderCompactPanel({
    ...spec,
    // `menu: true` means "the standard rows for this state". Resolved here rather than in the
    // fixture because fixtures/frames.js cannot import ui/compact.js — that module imports `fid`
    // from it, and the cycle is a lint error.
    menu: spec.menu === true ? trayMenu(spec.state) : (spec.menu ?? null),
  });
  root.replaceChildren(dom.panel);
  return true;
}

function render() {
  const root = document.getElementById("app-root");
  // The preview's own pages — the frame index, and the diagnostic for a `?frame=` label that has no
  // fixture. Both take the window: the shell never renders behind them. (The poll still runs — this
  // is a `render()` early return, not a boot switch — which is exactly why `dom.preview` latches.)
  // First, ahead of the panel mount, because an unknown label must not fall through to the generic
  // mock and draw a plausible screen that is not the frame you asked for.
  if (dom.preview) return;
  if (mountPreview(root)) {
    dom.preview = true;
    return;
  }
  if (mountFramePanel(root)) return;
  // The ⋯ menu comes down FIRST, so that for the rest of this pass the only things between the
  // header and the footer are the screen's own blocks — which is the invariant `setBody` is written
  // against. It goes back up at the end. See the note there.
  root.querySelector(".menu-popover")?.remove();
  const st = store.select.daemonState();

  // The folder pair: the running daemon's reported roots are ground truth; the GUI config file is
  // the fallback when no daemon is reachable. Em-dashes only when neither knows.
  const live = store.select.response()?.config ?? null;
  const localRoot = live?.local_root ?? configInfo?.local_root ?? null;
  const remoteRoot = live?.remote_root ?? configInfo?.remote_root ?? null;

  // Latched, not a raw read of the daemon state — see routes.js for the whole reason.
  const wasOnboarding = onboardingLatch;
  onboardingLatch = nextOnboardingLatch(
    onboardingLatch,
    st,
    Boolean(localRoot && remoteRoot),
    configLoaded,
    statusPolled,
  );
  // ENTERING the takeover discards both layers. Hiding them is not enough: the latch releases when
  // the daemon comes up, and anything still held would be restored on the way out — so finishing a
  // first-run setup would land you on the Conflicts screen with a Details dialog over it, from
  // before the daemon was wiped. Reproduced, not theorised; DEVIATIONS §57c.
  //
  // Edge-triggered on purpose. Clearing on every render while the latch is true would be the same
  // thing here, but it would also quietly forbid onboarding from ever opening a layer of its own,
  // which is S7's call to make and not this line's.
  if (onboardingLatch && !wasOnboarding) {
    screenStack = [];
    dialogOverlay = null;
    dialogReturn = null;
  }

  // The two layers, read back out. A dialog floats over whatever body is showing — which may be a
  // screen overlay and not `route`, and getting that wrong is what loses the user's place. See
  // routes.js `isDialog` and DEVIATIONS §57.
  //
  // A FIXTURE MAY ALSO NAME THE DIALOG IT DRAWS, for the same reason `activeRoute` lets it name a
  // route: three of S5's six frames ARE dialogs (`6a Details`, `7a Never synced`, `7a File
  // pending`), and `dialogOverlay` is module state no `?frame=` can reach. Without this the harness
  // opens the underlying screen and files all three under "screen not built yet".
  const dialogRoute = onboardingLatch ? null : (activeFixture()?.ui?.dialog ?? dialogOverlay);
  const active = activeRoute();
  const spec = ROUTES[active];
  const chip = chipFor();

  // `active`, not `route && !overlay`. A DIALOG MUST NOT CHANGE THE SCREEN UNDERNEATH IT: keyed off
  // the raw `overlay` this flips false when Details opens over the main screen, which swaps the
  // footer's mono line away and grows a home button in the header — the shell visibly rearranging
  // behind a panel that is supposed to be sitting on top of it. `active` already collapses a dialog
  // back to its underlying route, so the screen beneath renders as though nothing had opened.
  const onMain = active === "main";
  // An attention band is showing when something is waiting on a decision — which is what the two
  // attention chip variants mean. S1 draws the band itself; the footer only needs to know it is
  // there, because the band displaces the mono line.
  const banded = chip.variant === "decisions" || chip.variant === "deletions";
  const headerOpts = {
    chip: chip.variant,
    chipText: chip.text,
    // Onboarding drops the ⋯ button, not just the chip — both 9a frames have four header slots.
    hasMenu: !onboardingLatch,
    // `&& !onboardingLatch` is NOT redundant, and leaving it off was a regression this file already
    // shipped once. `onMain` used to read `route === "main" && !overlay`, which is TRUE during the
    // takeover on a fresh machine — route is still "main" and no overlay is open — so the mark
    // stayed an <img>. Rewriting it as `active === "main"` for the dialog layer flipped that: active
    // is "onboarding", so the mark became a <button class="app-home">.
    //
    // Not a cosmetic slot. routes.js says the takeover "cannot be dismissed with Esc", and 02-shell
    // makes the app mark the home affordance now that onboarding has no footer nav — so a home
    // button there is a working door out of a flow that is not supposed to have one, on a machine
    // with no folder pair chosen yet.
    hasHome: !onMain && !onboardingLatch,
  };
  const navOpts = {
    // `active`, NOT the module `route`. S5 is the first screen whose frames draw a LIT door — all
    // three activity windows paint `Activity` at --text and the other three at --text-4 — and under
    // `?frame=` the module `route` is still "main", so the gate would have compared four unlit
    // doors against three lit frames. `activeRoute()` collapses a dialog back to its underlying
    // route and yields the overlay id for a screen overlay, which is why the `kind === "door"`
    // test stays: `conflicts` and `deletions` are overlays, and their frames draw no lit door.
    active: ROUTES[active]?.kind === "door" ? active : null,
    // The mono line is drawn on the settled and syncing main screens ONLY. `2a Needs you` is also
    // the main screen and drops it — the attention band has taken the space, and the footer tightens
    // from 22/20 to 20/16 to match. Measured, and the fidelity gate caught the first version of this
    // line assuming every main screen was the same.
    //
    // `tight` is the fourth variant and was dead until S5: `7a Activity quiet` and `7a File lookup`
    // are 18/14 while `6a Activity passes` — the same screen, the other tab — is the standard 18/15.
    // So the variant is per-STATE, not per-route, exactly as `footerKindOf` is for the plan screen.
    variant: onMain
      ? banded
        ? "banded"
        : "withLine"
      : active === "activity"
        ? footerVariantOf(activityProps())
        : "standard",
    line: onMain && !banded ? `${localRoot ?? "—"} ⇄ ${remoteRoot ?? "—"}` : null,
  };

  // --- header: patched in place, rebuilt only when its shape changes
  if (!dom.header || !updateHeader(dom.header, headerOpts)) {
    const built = renderHeader({
      ...headerOpts,
      onMenu: headerOpts.hasMenu
        ? () => {
            menuOpen = !menuOpen;
            render();
          }
        : null,
      onHome: headerOpts.hasHome ? () => navigate("main") : null,
    });
    if (dom.header) dom.header.replaceWith(built);
    else root.append(built);
    dom.header = built;
  }

  // --- body: mounted when the route changes, PATCHED on every poll in between, so a screen holds
  // its own nodes — and its own running animations — across a status reply.
  // Conflicts rebuilds on every pass rather than patching: unlike the main screen it runs no
  // animation of its own to protect, and `setBody`'s `nextSibling` guard already leaves an
  // unchanged node in place. The crossfade is applied AFTER the swap, and only on an advance.
  if (active === "conflicts") {
    if (dom.bodyRoute !== active) unmountScreens();
    const props = conflictsProps();
    const nodes = renderConflicts(props);
    setBody(nodes);
    crossfadeConflictBody(nodes, props.conflicts[props.index]?.original ?? "(cleared)");
    dom.bodyRoute = active;
  } else if (active === "plan") {
    // Patched, not rebuilt: the gate is a text field that clears on blur, `Checked N ago` counts at
    // second resolution, and the checking body's two CSS animations restart from 0% on a rebuild.
    // `updatePlan` rebuilds only when the plan itself has moved.
    if (dom.bodyRoute !== active) {
      unmountScreens();
      // The fresh-plan rule lives on the mount, not in `navigate`: closing a screen overlay opened
      // over this one pops the stack without touching the route, so arriving is not always a
      // navigation. Every way in passes through here.
      resetPlanScreen();
      setBody(renderPlan(planProps()));
      dom.bodyRoute = active;
    } else {
      const nodes = updatePlan(planProps());
      if (nodes) setBody(nodes);
    }
  } else if (active === "deletions") {
    // PATCHED, NOT REBUILT — the opposite of the conflicts branch above, and the difference is a
    // text field. `4a Deletions` puts a typed-`DELETE` gate on every permanent card, and that field
    // clears on blur by design, so rebuilding the body twice a second would wipe a half-typed word
    // and make the only irreversible action in the app unreachable by keyboard. `updateDeletions`
    // rebuilds only when something the body draws has moved, applies the busy state in place
    // otherwise, and carries a half-typed word across the rebuilds it cannot avoid.
    if (dom.bodyRoute !== active) {
      unmountScreens();
      setBody(renderDeletions(deletionsProps()));
      dom.bodyRoute = active;
    } else {
      const nodes = updateDeletions(deletionsProps());
      if (nodes) setBody(nodes);
    }
  } else if (active === "activity") {
    // REBUILT EVERY PASS, and the one thing that makes that safe is putting the caret back. The
    // body holds a live `<input>`: rebuilding it drops focus and the caret position, so a poll
    // landing mid-word would move the cursor to the front of what someone was typing. Restoring
    // both after the swap is cheaper than the patch path the plan and deletions screens need,
    // because nothing here animates and nothing else here holds state.
    const focused = document.activeElement === activityInputRef.node;
    const caret = focused ? caretOffset() : null;
    if (dom.bodyRoute !== active) unmountScreens();
    setBody(renderActivity(activityProps()));
    dom.bodyRoute = active;
    if (focused && activityInputRef.node) {
      activityInputRef.node.focus();
      putCaret(activityInputRef.node, caret);
    }
  } else if (dom.bodyRoute !== active) {
    unmountScreens();
    setBody(
      active === "main"
        ? renderMain(mainProps(localRoot, remoteRoot))
        : // A screen with no S-task in the route table gets no issue chip rather than an
          // "F4 · issue" with nothing after it. The main screen used to be that case.
          [
            screenPlaceholder(
              spec.label ?? titleFor(active),
              spec.task && spec.issue ? `${spec.task} · issue ${spec.issue}` : null,
            ),
          ],
    );
    dom.bodyRoute = active;
  } else if (active === "main") {
    const nodes = updateMain(mainProps(localRoot, remoteRoot));
    if (nodes) setBody(nodes);
  }

  // --- footer: either the four doors or an action bar — never both, never neither. The 13-to-6
  // split is measured, not chosen; see routes.js.
  //
  // The plan screen answers for itself, and is the only route that does: `5a Plan` and `5a Plan
  // safe` draw an action bar, `5a Checking` draws the four doors. routes.js records only the route's
  // usual answer; the screen records what its current state draws.
  const kind = active === "plan" ? footerKindOf(planProps()) : (spec.footer ?? "doors");
  // Whose bar it is. A plan bar and a placeholder bar are both `actionBar`, and patching one as the
  // other leaves the previous screen's controls in the footer of the next.
  const owner = kind === "actionBar" ? active : null;
  // Action bars are patched too, not just the doors: once a bar holds the gate, a rebuild on the
  // ~2s poll destroys a half-typed `DELETE`. `updatePlanBar` returns false when the bar's shape has
  // changed, which is the signal to rebuild.
  let patched = false;
  if (dom.footer && dom.footerKind === kind && dom.footerOwner === owner) {
    patched =
      kind === "doors"
        ? updateFooterNav(dom.footer, navOpts)
        : owner === "plan"
          ? updatePlanBar(dom.footer, planProps())
          : true;
  }
  if (!patched) {
    const built =
      kind === "actionBar"
        ? owner === "plan"
          ? renderPlanBar(planProps())
          : renderActionBar({
              consequence: "This screen is not built yet.",
              // Onboarding draws 14px 32px 18px: it has no footer nav beneath to carry the margin.
              bottom: spec.takeover ? 18 : 14,
            })
        : renderFooterNav({
            ...navOpts,
            order: FOOTER_ORDER,
            labels: Object.fromEntries(FOOTER_ORDER.map((id) => [id, ROUTES[id].label])),
            onNavigate: navigate,
          });
    if (dom.footer) dom.footer.replaceWith(built);
    else root.append(built);
    dom.footer = built;
    dom.footerKind = kind;
    dom.footerOwner = owner;
  }

  // --- the dialog layer: mounted when the route changes, PATCHED (i.e. left alone) otherwise.
  //
  // The identity check is the whole safety property — see `dom`'s comment. Rebuilding this on the
  // poll would restart the appear animation twice a second and clear the armed deletion's typed
  // field mid-word.
  if (dom.dialogRoute !== dialogRoute) {
    dom.dialogDetach?.();
    dom.dialog?.remove();
    dom.dialog = null;
    dom.dialogDetach = null;
    if (dialogRoute) {
      const dspec = ROUTES[dialogRoute];
      const [w, h] = dspec.size ?? [522, null];
      // S5 is the first task to give a dialog a real body; the other three still draw the
      // placeholder. `activityDialog` returns null for a dialog it does not own AND for one whose
      // data has gone — `filePending` describes an in-flight transfer, and there is nothing to say
      // about one that has finished.
      const content = activityDialog(dialogRoute);
      const title = content?.title ?? dspec.label ?? titleFor(dialogRoute);
      // `7a File pending` draws no title row at all, so it is named for a screen reader directly
      // rather than pointing at a heading it does not have. `dialog()` enforces exactly one of the
      // two, which is what makes this an either/or rather than a pair of optional fields.
      const headless = content?.head === false;
      const built = dialog({
        width: w,
        height: h,
        tone: dspec.tone ?? "plain",
        padding: dspec.padding ?? null,
        label: headless ? (content.label ?? title) : null,
        labelledBy: headless ? null : "dialog-title",
        children: [
          headless
            ? null
            : dialogHead({
                title,
                subtitle: content?.subtitle ?? null,
                id: "dialog-title",
                size: w >= 600 ? "wide" : "compact",
                // Per route, not always. `8a Save refused` and `9a CLI missing` draw no ✕ at all —
                // they are asking you to choose between two repairs, and a dismiss button in the
                // corner is a third answer the design does not offer. Esc still closes them,
                // through F4's chain.
                onClose: dspec.closable ? () => closeOverlay() : null,
              }),
          ...(content?.children ?? [
            screenPlaceholder(
              title,
              dspec.task && dspec.issue ? `${dspec.task} · issue ${dspec.issue}` : null,
            ),
          ]),
        ],
      });
      root.append(built);
      dom.dialog = built;
      // Attached after append: the trap focuses on attach, and focus() on a detached node is a
      // silent no-op that leaves the keyboard on whatever opened the dialog.
      dom.dialogDetach = focusTrap(built);
      dom.dialogSignature = content?.signature ?? null;
    }
    dom.dialogRoute = dialogRoute;
  } else if (dialogRoute && dom.dialog) {
    // A MOUNTED DIALOG HAS TO BE ABLE TO CHANGE, and until S5 none of them could: the identity
    // check above is the only thing that ever rebuilds one, so `6a Details` — eight live counters —
    // would have frozen at whatever the reply held on the render that opened it. On a real machine
    // that is the poll before the panel appeared; under `?frame=` it is an empty store, which is
    // how the gate found it (four rows drawing an em-dash where the frame draws a value).
    //
    // Keyed on a SIGNATURE rather than rebuilt every pass, because the poll runs twice a second and
    // the surface carries the appear animation and the focus trap. Only the children below the head
    // are replaced, and focus is carried across by position — `Copy all` must survive a counter
    // moving underneath it.
    const content = activityDialog(dialogRoute);
    if (content?.signature && content.signature !== dom.dialogSignature) {
      const surface = dom.dialog.querySelector(".dialog");
      const head = surface.querySelector(".dialog-head");
      const focusables = [...surface.querySelectorAll("button, input, [tabindex]")];
      const at = focusables.indexOf(document.activeElement);
      while (surface.lastChild && surface.lastChild !== head) surface.lastChild.remove();
      surface.append(...content.children);
      if (at >= 0) {
        const next = [...surface.querySelectorAll("button, input, [tabindex]")];
        (next[at] ?? surface).focus();
      }
      dom.dialogSignature = content.signature;
    }
  }

  // --- the ⋯ menu, the one part that is genuinely torn down and rebuilt. It has no animation and
  // no focus to lose that closing it would not have taken anyway.
  //
  // REBUILT HERE, TORN DOWN AT THE TOP. The removal used to sit on the line above this one, which
  // put a stale popover between the header and the body for the whole of `setBody` — and `setBody`
  // decides whether a block has moved by asking whether it is its anchor's `nextSibling`. With the
  // menu open, every poll therefore answered "no" for every block and re-inserted the entire screen,
  // restarting the hexagon's two travelling segments and the glow. Exactly the failure the patching
  // discipline exists to prevent, reintroduced by a node that is not part of the screen at all.
  //
  // Appended, never passed to replaceChildren: `replaceChildren(null)` appends the literal string
  // "null" as a TEXT NODE. The v1 app.js carried that guard and a comment saying so; dropping it in
  // the rewrite printed "null" in the corner of the window, and every class-based assertion still
  // passed — it took looking at a screenshot.
  const menu = renderMenu();
  if (menu) dom.header.after(menu);
}

function titleFor(id) {
  return id.replace(/([A-Z])/g, " $1").replace(/^./, (c) => c.toUpperCase());
}

/**
 * Put this list of blocks between the header and the footer, moving as little as possible.
 *
 * The `nextSibling` guard is the whole function: a node that is already in the right place is left
 * alone, because re-inserting one restarts every CSS animation inside it — the hexagon's two
 * travelling segments, the glow's `breathe`, the chip's `blip`. So a poll that changes nothing moves
 * nothing, and a decision arriving appends one block and touches neither of the other two.
 */
/**
 * Drop every screen's cached view before mounting a different one — all of them, not just the one
 * being left. Each module's `update*` reads a module-level `view` to decide whether to rebuild, so a
 * screen left holding a stale one patches nodes no longer in the document when it is next opened.
 */
function unmountScreens() {
  unmountMain();
  unmountDeletions();
  unmountPlan();
}

function setBody(nodes) {
  for (const node of dom.bodyNodes) if (!nodes.includes(node)) node.remove();
  let anchor = dom.header;
  for (const node of nodes) {
    if (anchor.nextSibling !== node) anchor.after(node);
    anchor = node;
  }
  dom.bodyNodes = nodes;
}

// ---- the conflicts screen (S2) ----
//
// Module-level for the same reason `menuOpen` is: `render()` rebuilds the body from scratch on
// every 2s poll, so a screen that owned its own state would forget which conflict you were on
// twice a second.

let conflictIndex = 0;
let conflictDiffOpen = false;
let conflictPair = null; // the two versions' bytes, or null when they could not be read
let conflictPairKey = null; // which conflict `conflictPair` HOLDS — moves with it, never ahead of it
let conflictPairInFlight = null; // which conflict is being read right now, if any
/**
 * WHAT YOU DECIDED WHILE YOU WERE HERE, and only while you were here.
 *
 * `3a Conflicts cleared` reads `You settled 3 conflicts — 2 kept both versions, 1 took Proton's`,
 * which is a claim about THIS VISIT. Nothing on disk records it: a resolved conflict leaves a
 * sidecar or it leaves nothing, and neither says which button was pressed. So the tally is counted
 * here as the choices are made, and reset on entry — a cleared screen reached without deciding
 * anything shows the sentence with no counts rather than yesterday's.
 */
let conflictsSettled = { total: 0, keptBoth: 0, tookProton: 0 };
/** The conflict the body was last built for — what makes an ADVANCE distinguishable from a poll. */
let conflictShowing = null;

/** Entering the screen fresh: the queue starts at the top and the tally starts empty. */
function resetConflictScreen() {
  conflictIndex = 0;
  conflictDiffOpen = false;
  conflictPair = null;
  conflictPairKey = null;
  conflictPairInFlight = null;
  conflictsSettled = { total: 0, keptBoth: 0, tookProton: 0 };
  conflictShowing = null;
}

/**
 * Fetch the two versions' text for the conflict now showing, once.
 *
 * Guarded on the conflict's own path rather than on `conflictPair == null`, because a pair that
 * legitimately reads as two empty files is indistinguishable from one not yet fetched — and the
 * unguarded version re-reads both files off disk on every poll.
 *
 * `conflictPairKey` NAMES WHAT `conflictPair` HOLDS, and never what was asked for. That is the whole
 * design, and it is a safety property rather than tidiness: `conflictsProps` decides whether the
 * bytes belong to the conflict on screen by comparing exactly those two, so any moment where the key
 * runs ahead of the value is a moment the cards, the diff and the line counts are drawn from the
 * PREVIOUS file under the current file's name — on the one screen whose job is choosing which
 * version to destroy.
 *
 * There are two such moments and they need different answers, which is why there are two variables:
 *
 *   · BEFORE the read. This function is called from inside `conflictsProps` and deliberately not
 *     awaited, so setting the key here yields at the `await` and hands the caller a key that already
 *     matches while `conflictPair` still holds the old bytes. `conflictPairInFlight` takes that role
 *     instead, so the key stays behind and the gap renders as "no pair yet" — which the cards
 *     already handle by falling back to the metadata row, per `04-conflicts.md`.
 *   · AFTER it. Hold `›` and two reads overlap; the first conflict's reply can arrive second. The
 *     `requested` check drops a reply that a later request has superseded.
 *
 * A failed read still claims the slot — `conflictPair` null, key set — so a file that cannot be read
 * is asked for once rather than on every poll.
 */
async function ensureConflictPair(conflict) {
  if (!conflict) return;
  const requested = conflict.original;
  if (conflictPairKey === requested || conflictPairInFlight === requested) return;
  conflictPairInFlight = requested;
  let pair = null;
  try {
    pair = await api.readConflictPair(conflict);
  } catch (error) {
    // Not fatal and not a placeholder: the cards fall back to the metadata row alone, which is what
    // `04-conflicts.md` asks for when the content cannot be read. A pair invented here would be a
    // diff of files nobody has seen.
    console.error("read_conflict_pair failed:", error);
  }
  if (conflictPairInFlight !== requested) return;
  conflictPairInFlight = null;
  conflictPair = pair;
  conflictPairKey = requested;
  render();
}

async function chooseConflict(conflict, choice) {
  try {
    await api.resolveConflict(conflict, choice);
  } catch (error) {
    console.error("resolve_conflict failed:", error);
    return;
  }
  conflictsSettled = {
    total: conflictsSettled.total + 1,
    keptBoth: conflictsSettled.keptBoth + (choice === "keep_both" ? 1 : 0),
    tookProton: conflictsSettled.tookProton + (choice === "use_proton" ? 1 : 0),
  };
  // Re-scan before moving: settling one removes it, and `advanceAfter` needs the list it is
  // adjusting against. Keeping the old length here is what makes the last conflict dead-end.
  let next = store.select.conflicts();
  try {
    next = await api.scanConflicts();
    store.setConflicts(next);
  } catch (error) {
    console.error("scan_conflicts failed:", error);
  }
  conflictIndex = advanceAfter(conflictIndex, next);
  conflictDiffOpen = false;
  // The settled file's bytes are gone in both senses — dropped together, so nothing can name them.
  conflictPair = null;
  conflictPairKey = null;
  conflictPairInFlight = null;
  render();
}

/** Everything the conflicts screen reads, plus the actions it can take. */
function conflictsProps() {
  const conflicts = store.select.conflicts();
  const at = Math.min(conflictIndex, Math.max(0, conflicts.length - 1));
  const conflict = conflicts[at] ?? null;
  // THE `ui` BLOCK, CONSUMED FOR THE FIRST TIME. F9 gave every fixture a slot for the screen state
  // that no daemon reply can carry — which tab, which step, which dialog — and left it for the
  // screens to read; S1's frames needed none, so S2 is the first. It supplies exactly two things
  // here: whether the disclosure is open, and the tally the cleared state reads. Live, both come
  // from the module state above and this is inert.
  const ui = activeFixture()?.ui ?? null;
  // Fired and not awaited: the screen renders now with whatever it has, and `ensureConflictPair`
  // calls `render()` again when the bytes land. Awaiting here would block the body on two file
  // reads and show a blank window while they happen.
  ensureConflictPair(conflict);
  return {
    conflicts,
    index: at,
    diffOpen: ui?.diff ?? conflictDiffOpen,
    pair: conflictPairKey === conflict?.original ? conflictPair : null,
    settled: ui?.settled ?? conflictsSettled,
    onChoose: (choice) => chooseConflict(conflict, choice),
    onLater: () => {
      conflictIndex = skipTo(at, conflicts.length);
      conflictDiffOpen = false;
      render();
    },
    onOpenDiff: () => {
      conflictDiffOpen = true;
      render();
    },
    onHideDiff: () => {
      conflictDiffOpen = false;
      render();
    },
    // `Open both in an editor` is DRAWN AND INERT, and that is the process rather than an
    // oversight. There is no command that opens a path, and `commands.rs` is explicit that a
    // screen task never adds one — a screen needing something the surface lacks files a C-item,
    // which adds the command before the screen is built. S2 found this one too late for that, so
    // the button is drawn (the frame draws it) and does nothing until the C-item lands. §74.
    onOpenBoth: null,
    onBack: () => navigate("main"),
    onPrev: () => stepConflict(-1),
    onNext: () => stepConflict(1),
  };
}

/** `‹ ›` and the arrow keys. Clamped, not wrapped — only `Decide later` wraps. */
function stepConflict(delta) {
  const total = store.select.conflicts().length;
  if (!total) return;
  const next = Math.min(total - 1, Math.max(0, conflictIndex + delta));
  if (next === conflictIndex) return;
  conflictIndex = next;
  conflictDiffOpen = false;
  render();
}

/**
 * The 220ms crossfade, applied to the body that just arrived.
 *
 * ONLY WHEN THE CONFLICT CHANGES. A poll re-renders the same conflict twice a second, and animating
 * that would leave the screen permanently pulsing; the first mount is excluded too, so the fidelity
 * gate — which renders a fixture cold and reads computed styles immediately — never catches a body
 * mid-animation and compares `animation-name: cf-appear` against the frame's `none`.
 *
 * A FADE-IN RATHER THAN A TRUE CROSSFADE, and `04-conflicts.md` asks for the latter. A real one
 * needs both bodies alive and stacked, which needs a positioned wrapper — and `renderConflicts`
 * returns window-root SIBLINGS precisely so the seam's `left: 50%` resolves against the 1040px
 * window. The wrapper would move the seam. DEVIATIONS §74.
 */
function crossfadeConflictBody(nodes, showing) {
  const advanced = conflictShowing !== null && conflictShowing !== showing;
  conflictShowing = showing;
  if (!advanced) return;
  for (const node of nodes) {
    node.classList.add("cf-advancing");
    node.addEventListener("animationend", () => node.classList.remove("cf-advancing"), { once: true });
  }
}

// ---- the deletions screen (S3) ----
//
// Module-level for the reason the conflicts state is, and for one more: this screen holds a text
// field whose contents clear on blur by design, so it may not be rebuilt on the poll at all. The
// state that survives a poll therefore has to live somewhere the poll does not touch.

/**
 * Which permanent deletion's confirmation is up — `{ path, fingerprint }`, never a bare path.
 *
 * A confirmation is about one exact thing. A path is a slot, and a later deletion can move into it:
 * arm `notes.txt`, let that deletion resolve, then have the file come back and be deleted again.
 * Keyed on the path alone the takeover re-binds itself to the NEW deletion and reappears with a
 * live `Delete permanently` that no word was typed for. `armedItem` matches the fingerprint too,
 * which is what the daemon pins its own approvals to.
 */
let deletionArmed = null;
/** `path_sync_status` replies, by path — the size and mtime a file card draws. */
const deletionStatuses = new Map();
const deletionStatusInFlight = new Set();
/** Item keys with a control command in flight, plus `"all"` for the bulk one. */
const deletionBusy = new Set();
/**
 * WHAT YOU HAVE ALREADY DECIDED, while this app has been open.
 *
 * The daemon keeps a withheld deletion in `pending_deletions` until a pass consumes it, so the ~2s
 * poll would put an approved row straight back on screen — and a KEPT row comes back for ever,
 * because the wire has no way to refuse a deletion at all (#224: `deny` revokes an approval, and
 * withholding is already the default). Keyed by `(path, direction)` and PINNED TO THE FINGERPRINT,
 * which is what the daemon pins its own approvals to: the same path deleted again with different
 * content is a different question and gets asked again.
 *
 * Not reset on navigation. `resetConflictScreen` clears its tally on entry because that tally is a
 * claim about one visit; this is a record of decisions, and forgetting them on the way back into
 * the screen would re-ask everything you just answered.
 */
const deletionsDecided = new Map();

/**
 * The queue as the screen sees it: what the daemon is withholding, less what you have answered.
 *
 * PRUNES AS IT READS. An entry whose item is no longer withheld has done its job, and dropping it
 * is what lets a later deletion of the same path be asked about — a fingerprint can legitimately
 * repeat (a directory's is its `volumeId~nodeId`, which does not change), so absence from the live
 * queue is the only reliable signal that the decision has been consumed.
 */
function visibleDeletions() {
  const live = store.select.pendingDeletions();
  const queued = new Set(live.map(itemKey));
  for (const key of deletionsDecided.keys()) if (!queued.has(key)) deletionsDecided.delete(key);
  // The size-and-mtime cache is pruned on the same signal, and for a sharper reason: it is keyed by
  // PATH, so a `notes.txt` that is deleted, settled, replaced and deleted again would otherwise draw
  // the first file's `4 KB` and `last edited Jan 2026` on a card about the second. Dropping the
  // entry when the deletion leaves the queue makes the next one ask again — which also gives a read
  // that failed on a busy index a second chance instead of remembering the failure for the session.
  const paths = new Set(live.map((item) => item.path));
  for (const path of deletionStatuses.keys()) if (!paths.has(path)) deletionStatuses.delete(path);
  return live.filter((item) => deletionsDecided.get(itemKey(item)) !== item.fingerprint);
}

/**
 * Fetch one file's index record — its size and mtime — once.
 *
 * KEYED BY PATH IN A MAP, which is what makes the conflicts screen's stale-reply race structurally
 * impossible here rather than guarded against: a reply is filed under the path it was asked for, so
 * a late one can only ever overwrite its own entry. A failed or absent record still claims the slot
 * (as `null`), so a file the daemon cannot answer for is asked about once and not on every poll.
 *
 * Directories are not asked at all: a directory record's `file_size` is not a subtree total (#208)
 * and its mtime is the directory's own, so neither is a fact about what you would lose.
 */
async function ensurePathStatus(item) {
  if (item.entity_kind === "directory") return;
  const path = item.path;
  if (deletionStatuses.has(path) || deletionStatusInFlight.has(path)) return;
  deletionStatusInFlight.add(path);
  let status = null;
  try {
    status = await api.pathSyncStatus(path);
  } catch (error) {
    console.error("path_sync_status failed:", error);
  }
  deletionStatusInFlight.delete(path);
  deletionStatuses.set(path, status);
  render();
}

/**
 * Approve or keep one withheld deletion.
 *
 * APPROVING ASKS FOR A PASS. `approve` records a standing approval and the daemon's own reply says
 * so — "run `proton-sync syncnow` to apply now" — because the delete itself happens inside the next
 * reconcile. Without the nudge the row sits there until the scan interval comes round, which reads
 * as a button that did not work; `03-main-screen.md` makes the same argument for `Sync now`.
 *
 * KEEPING CANNOT DO WHAT ITS LABEL SAYS, and that is #224 rather than a bug here. `deny` revokes an
 * approval — which is exactly right if you approved this and changed your mind, and a no-op
 * otherwise — but nothing on the wire refuses a deletion durably or puts the file back on the other
 * side. Phase 1 sends the command it has and remembers the decision for this session; DEVIATIONS
 * §75 records what a user actually gets.
 */
async function decideDeletion(item, approve) {
  const key = itemKey(item);
  if (deletionBusy.has(key)) return;
  deletionBusy.add(key);
  render();

  let settled = false;
  try {
    const reply = await (approve ? api.approve(item.path) : api.deny(item.path));
    settled = acknowledged(reply);
  } catch (error) {
    console.error(approve ? "approve failed:" : "deny failed:", error);
  }
  deletionBusy.delete(key);
  if (settled) {
    deletionsDecided.set(key, item.fingerprint);
    if (deletionArmed?.path === item.path && deletionArmed?.fingerprint === item.fingerprint) {
      deletionArmed = null;
    }
  }
  if (settled && approve) {
    try {
      await api.syncNow();
    } catch (error) {
      console.error("sync_now failed:", error);
    }
  }
  clearTimeout(pollTimer);
  poll();
}

/**
 * `Keep both files` — the one bulk action, and the safe one.
 *
 * SCOPED TO THE WHOLE PENDING LIST, not to what is on screen, because that is what the wire's
 * `"all"` selector does. The two differ by exactly the items you have already answered this
 * session: approve one, then keep the rest, and `deny all` revokes the approval you just gave —
 * which is the right reading of a button that says *keep both files*, but only if the screen
 * remembers it that way too. Marking the visible ones alone would leave the approved item recorded
 * as approved and hidden, while the daemon had just been told to keep it.
 */
async function keepAllDeletions() {
  const items = store.select.pendingDeletions();
  if (!visibleDeletions().length || deletionBusy.has(BULK_KEY)) return;
  deletionBusy.add(BULK_KEY);
  render();
  let settled = false;
  try {
    // `literalPath: false` with the explicit "all" selector. A file literally named `all` is a real
    // path and would otherwise be the only thing denied (#60); the flag is what keeps the reserved
    // word and a filename apart on this wire.
    const reply = await api.deny(BULK_KEY, false);
    settled = acknowledged(reply);
  } catch (error) {
    console.error("deny all failed:", error);
  }
  deletionBusy.delete(BULK_KEY);
  if (settled) {
    for (const item of items) deletionsDecided.set(itemKey(item), item.fingerprint);
    deletionArmed = null;
  }
  clearTimeout(pollTimer);
  poll();
}

/**
 * Did the daemon actually act on that approve/deny?
 *
 * NOT "did a reply come back". A dead socket resolves rather than rejects — the Tauri commands
 * return a payload either way — and, more subtly, `apply_approval_command` answers `Ok` with
 * `no pending deletion matches '<path>'` when the selector is absent from the snapshot it is
 * holding, which the GUI can reach by acting on a queue that is up to two seconds stale. Treated as
 * a decision, that hides a row nothing was recorded for.
 *
 * Read as a POSITIVE match on the acknowledgement rather than a blacklist of the three `no …`
 * replies, so the failure direction is safe: if the daemon ever rewords them, the GUI stops
 * recording decisions and rows stay visible, instead of hiding deletions that never happened.
 */
function acknowledged(reply) {
  if (!reply?.response || reply.error) return false;
  return /^(approved|denied) /.test(reply.response.message ?? "");
}

/** Everything the deletions screen reads, plus the actions it can take. */
function deletionsProps() {
  const items = visibleDeletions();
  // Fired and not awaited, like the conflict pair: the cards render now with what they have, and
  // each reply calls `render()` when it lands.
  for (const item of items) ensurePathStatus(item);
  // The `ui` block again (F9). Live, `armed` is the module state above and this is inert; it is how
  // `4a Armed` says which of the two queued items has its gate satisfied.
  const ui = activeFixture()?.ui ?? null;
  return {
    items,
    statuses: deletionStatuses,
    busy: deletionBusy,
    armed: ui?.armed ?? deletionArmed,
    handlers: {
      onArm: (item) => {
        // Guarded on severity as well as on the gate, because arming is the step that leads to the
        // only irreversible action in the app and `severityOf` is the one place that decides which
        // direction that is.
        if (severityOf(item.direction) !== "permanent") return;
        deletionArmed = { path: item.path, fingerprint: item.fingerprint };
        render();
        // The body swap leaves focus on a button that no longer exists, i.e. on `<body>` — so the
        // keyboard arrives at a full-window confirmation with nothing selected. Focus goes to the
        // SAFE button: `Enter` on a screen that just appeared must not be the irreversible one.
        //
        // Here and not in the screen module, because it is a consequence of THIS transition rather
        // than of the body being drawn: the fidelity gate renders the same body cold, and a screen
        // that focused on mount would put a focus ring in front of it.
        document.querySelector(".dl-armed-keep")?.focus();
      },
      onConfirmArmed: (item) => decideDeletion(item, true),
      onTrash: (item) => decideDeletion(item, true),
      onKeep: (item) => decideDeletion(item, false),
      onKeepAll: () => keepAllDeletions(),
    },
  };
}

// ---- the plan screen (S4) ----
//
// Module-level like the other screens' state, and this is the only screen driven by a command
// rather than by the poll, so a rehearsal's result must outlive the renders taken while it is in
// flight.

/** The last `DryRunPayload`, the daemon's message if it refused, and when the answer landed. */
let planDryRun = null;
let planError = null;
let planCheckedAt = null;
/**
 * `run_dry_run` cannot be cancelled, so `Stop` and `Check again` can only stop believing an answer
 * that is still coming. Three tokens, not one:
 *
 *   · `planSeq`      the rehearsal wanted now; bumped on enter, leave, re-check and stop. Strictly
 *                    increasing, so an abandoned reply can never be claimed by a later request.
 *   · `planWaiting`  the token of the child actually running, or null. The one-at-a-time guard reads
 *                    this: two `proton-syncd --dry-run` children shell the same `proton-drive` CLI,
 *                    whose SQLite cache is not concurrency-safe (#23).
 *   · `planAnswered` the token whose answer `planDryRun`/`planError` hold — what lets a re-check keep
 *                    the previous plan in hand and `Stop` put it back.
 */
let planSeq = 0;
let planWaiting = null;
let planAnswered = null;

/** Entering or leaving the screen: no plan, no error, and a rehearsal on its way. */
function resetPlanScreen() {
  planDryRun = null;
  planError = null;
  planCheckedAt = null;
  planAnswered = null;
  planSeq += 1;
}

/**
 * Run the rehearsal, once per visit and once per `Check again`.
 *
 * Fired and not awaited, like `ensureConflictPair` and `ensurePathStatus`: the screen renders the
 * checking body now and this calls `render()` again when the answer lands.
 *
 * Not re-fired on the poll: a screen holding an in-flight or finished rehearsal returns early, or
 * every status tick would shell a fresh `proton-syncd --dry-run`, which walks the whole remote.
 */
async function ensurePlan() {
  // One child at a time. Guarding on `planWaiting` rather than on the current token is what stops
  // two remote walks at once (#23) — two clicks on the door is all it takes. The abandoned child
  // still runs to completion (there is no cancel); its reply is dropped by the token check below,
  // which re-enters here for whatever is wanted now.
  if (planWaiting !== null || planAnswered === planSeq) return;
  const seq = planSeq;
  planWaiting = seq;
  let payload = null;
  let error = null;
  try {
    payload = await api.runDryRun();
  } catch (e) {
    // The daemon's own string, verbatim: `14-behaviour-and-state.md` shows it on a failed
    // rehearsal, and voice rule 4 forbids paraphrasing one.
    error = String(e);
  }
  planWaiting = null;
  // Superseded by a re-check, a `Stop`, or a leave and return. Drop the answer rather than overwrite
  // what the screen holds now, and re-render: the `planWaiting` guard was closed while this was in
  // flight, so whatever is wanted now can only start from here.
  if (seq !== planSeq) {
    render();
    return;
  }
  planAnswered = seq;
  if (payload?.report) {
    planDryRun = payload;
    planError = null;
  } else {
    // A resolved reply that is not a report is not a plan: `run_dry_run` either returns a
    // `DryRunPayload` or fails, so this is the browser-preview mock answering `null` for a frame
    // that describes no rehearsal. Treating it as an empty plan would claim the next sync moves
    // nothing, over a screen that has been told nothing.
    planDryRun = null;
    planError = error ?? "the rehearsal returned no plan";
  }
  planCheckedAt = Math.floor(Date.now() / 1000);
  render();
}

/** Everything the plan screen reads, plus the actions it can take. */
function planProps() {
  // The `ui` block (F9): `5a Checking` carries no `dryRun` at all, so the mock resolves null and the
  // live path would read that as a failure. `ui.checking` is screen state no daemon reply can carry,
  // which is what that slot is for.
  const ui = activeFixture()?.ui ?? null;
  ensurePlan();
  return {
    // Only the answer belonging to the token in flight. A re-check keeps the previous plan in hand
    // so `Stop` can put it back, but it must not be drawn under a live `Run this sync`. `bodyOf`
    // would keep it off screen anyway (checking outranks a payload); this holds where the data is
    // chosen rather than relying on the body's ordering.
    dryRun: planAnswered === planSeq ? planDryRun : null,
    error: planAnswered === planSeq ? planError : null,
    checking: ui?.checking ?? planAnswered !== planSeq,
    checkedAt: ui?.checkedAt ?? planCheckedAt,
    handlers: {
      // A re-check keeps what it will replace until the replacement lands: the token moves (so the
      // screen draws `checking`, nothing stale) but the plan stays in hand for `Stop`. Focus follows
      // the body — the button just pressed is about to stop existing.
      onCheck: () => {
        planSeq += 1;
        render();
        focusAfterSwap(".pl-stop");
      },
      // `Stop` cannot stop the child: `run_dry_run` has no cancel, so the running
      // `proton-syncd --dry-run` finishes and its answer is dropped where it lands. It is read-only,
      // so the cost is CPU. What the button can do is claim the answer already in hand — the token
      // moves and `planAnswered` follows it, so the screen returns to that plan with its
      // `Checked N ago` unchanged. With nothing to go back to the design draws no state for a
      // rehearsal nobody finished, so leave for the main screen.
      onStop: () => {
        if (planAnswered === null) {
          navigate("main");
          return;
        }
        planSeq += 1;
        planAnswered = planSeq;
        render();
      },
      // `Run this sync` asks the daemon for a pass; see `runNow` in screens/plan.js for what the
      // typed word can and cannot authorise. It leaves whether or not the command landed, which is
      // deliberate: `sync_now` resolves rather than rejects on a dead socket, so a failure here is
      // silent, and the main screen is where both outcomes are legible (syncing hero vs
      // unreachable).
      onRun: async () => {
        await command(api.syncNow);
        navigate("main");
      },
    },
  };
}

/** Everything the main screen reads, plus the actions it can take. */
function mainProps(localRoot, remoteRoot) {
  return {
    daemonState: store.select.daemonState(),
    response: store.select.response(),
    conflicts: store.select.conflicts(),
    // The same filtered view the chip and the deletions screen read — see `chipFor`. The band says
    // `Two deletions are waiting on you`, and it must not say it about ones you have answered.
    deletions: visibleDeletions(),
    localRoot,
    remoteRoot,
    handlers: {
      onSyncNow: () => command(api.syncNow),
      onPause: () => command(api.pause),
      onResume: () => command(api.resume),
      onConflicts: () => navigate("conflicts"),
      onDeletions: () => navigate("deletions"),
    },
  };
}

/**
 * Run a control command and re-poll immediately rather than waiting out the ~2s tick.
 *
 * `Sync now` reaching state B "within ~1s" (`03-main-screen.md`) is not something the daemon can
 * deliver on its own: `Syncnow` is an immediate ack and the pass runs on the daemon's main loop, so
 * the state the button promises only becomes visible on the next status reply. Asking for one now is
 * the difference between a button that responds and a button that appears not to have worked.
 */
async function command(run) {
  try {
    await run();
  } catch (error) {
    console.error("control command failed:", error);
  }
  clearTimeout(pollTimer);
  poll();
}

// ---- the activity screen (S5) ----
//
// Two tabs and a lookup, all three of them screen-local: `routes.js` has ONE `activity` door and no
// sub-route, which is right — a tab is not a place, and neither is a half-typed path. What that
// costs is that both reset on leaving, and `07-activity.md` asks for nothing else.

/** `"files"` or `"passes"`. The pills only exist on the passes tab; `Sync passes` is the way in. */
let activityTab = "files";
/** What is in the lookup field, and the answer for it — the second only moves when a reply lands. */
let activityQuery = "";
let activityLookup = null; // { path, status } — `status` null-but-present means "asked, not found"
let activityLookupInFlight = null;
/**
 * `skip_rule_usage`'s report, and whether it has been asked for on this visit.
 *
 * ASKED ONCE PER VISIT, NEVER ON THE POLL. This command WALKS THE LOCAL TREE — that is the whole
 * reason it can answer a question the index cannot — so firing it every two seconds would put a
 * full metadata walk of someone's sync folder on a timer. The plan screen guards `run_dry_run` the
 * same way and for the same reason.
 */
let skipRuleReport = null;
let skipRuleAsked = false;

/** Entering or leaving: no tab memory, no query, no answer, and the walk to be asked for again. */
function resetActivityScreen() {
  activityTab = "files";
  activityQuery = "";
  activityLookup = null;
  activityLookupInFlight = null;
  skipRuleReport = null;
  skipRuleAsked = false;
}

/** The exclude rules' cost, once per visit. Fired and not awaited, like every other screen's fetch. */
async function ensureSkipRules() {
  // A fixture already carries the answer, and firing the command under `?frame=` would walk the
  // developer's own home directory to render a still.
  if (activeFixture()) return;
  if (skipRuleAsked) return;
  skipRuleAsked = true;
  const exclude = configInfo?.exclude ?? [];
  // Nothing excluded is not a reason to walk the tree: the band counts files a RULE hides, and with
  // no rules the answer is known without asking.
  if (exclude.length === 0) return;
  try {
    skipRuleReport = await api.skipRuleUsage(exclude, configInfo?.include ?? []);
  } catch (error) {
    console.error("skip_rule_usage failed:", error);
  }
  render();
}

/**
 * Look one path up.
 *
 * AN EXACT RELATIVE PATH, and that is narrower than the frame implies: `7a File lookup` draws the
 * query `spec.md` resolving to `docs/spec.md`, which is a name-to-path SEARCH, and no Phase-1
 * command lists or searches local files — `path_sync_status` opens the index at the path it is
 * given and nothing else. So a bare name that is not at the root MISSES, and the deck's own
 * `No file by that name in your sync folder.` is the honest answer rather than a failure. G17.
 *
 * The consequence for the drawn `1 match`: the count is only ever 0 or 1 here, so the plural arm of
 * `ACTIVITY.matches` is unreachable until that gap closes.
 */
async function lookupPath(query) {
  const path = query.trim().replace(/^\/+/, "");
  if (!path) {
    activityLookup = null;
    activityLookupInFlight = null;
    render();
    return;
  }
  // Latest-wins. Typing outruns the round trip, and an early reply landing after a later one would
  // put the verdict for `doc` under the word `docs/spec.md`.
  activityLookupInFlight = path;
  let status = null;
  try {
    status = await api.pathSyncStatus(path);
  } catch (error) {
    console.error("path_sync_status failed:", error);
  }
  if (activityLookupInFlight !== path) return;
  activityLookupInFlight = null;
  activityLookup = { path, status };
  render();
}

/** Everything the activity screen reads, plus the actions it can take. */
function activityProps() {
  const ui = activeFixture()?.ui ?? null;
  const response = store.select.response();
  const history = response?.status_history ?? [];
  const lastSync = response?.last_sync_epoch_secs ?? null;

  // Fired and not awaited — see `ensureSkipRules`. Never on the passes tab: nothing there draws it.
  const tab = ui?.tab ?? activityTab;
  if (tab === "files") ensureSkipRules();

  const never = neverSyncedFrom(ui?.skipRules ?? skipRuleReport);
  return {
    tab,
    query: ui?.query ?? activityQuery,
    lookup: ui?.lookup ?? activityLookup,
    editedAt: ui?.clock?.edited ?? null,
    never,
    history,
    localRoot: response?.config?.local_root ?? configInfo?.local_root ?? null,
    remoteRoot: response?.config?.remote_root ?? configInfo?.remote_root ?? null,
    // Both sub-lines are claims about WHEN, so both are omitted rather than guessed when the daemon
    // has not reported a pass yet.
    quietSub: lastSync != null ? ACTIVITY.quietSub(clockAt(ui, "since", lastSync), since(lastSync)) : null,
    checkedAgo: lastSync != null ? since(lastSync, "short") : null,
    // THE PINNED CLOCK LITERAL WINS UNDER A FIXTURE, and this screen is the first that needed it.
    // `clock.js` states the rule: a DURATION is pinned as an epoch offset (`ago(120)` is always "2
    // minutes ago" wherever it runs), but an epoch rendered as `14:32` moves with the machine's
    // timezone and across midnight — so a frame drawing an absolute time pins the string beside the
    // epoch and the screen reads that one.
    //
    // Not a gate convenience. Without it the lookup sub-line renders a different time on every run
    // and its width lands where it lands: it happened to be 1px out when this was written, and it
    // would have been green at some hours and red at others — a gate that fails by the clock is
    // worse than one that fails.
    agreedAt: lastSync != null ? clockAt(ui, "agreed", lastSync) : null,
    passesSub: passesSummaryOf(history),
    onQuery: (value) => {
      activityQuery = value;
      lookupPath(value);
      render();
    },
    onClearQuery: () => {
      activityQuery = "";
      activityLookup = null;
      activityLookupInFlight = null;
      render();
    },
    onPasses: () => {
      activityTab = "passes";
      render();
    },
    onFiles: () => {
      activityTab = "files";
      render();
    },
    onDetails: () => navigate("details"),
    onShowNeverSynced: () => navigate("neverSynced"),
    inputRef: activityInputRef,
  };
}

/** A frame's pinned clock string if it has one for this slot, else the live value. */
function clockAt(ui, slot, epochSecs) {
  return ui?.clock?.[slot] ?? clock(epochSecs);
}

/**
 * The caret's offset in the lookup field, and how to put it back after a rebuild.
 *
 * The SELECTION API, not `selectionStart` — the field is a contenteditable span (see
 * `lookupField`), and `selectionStart` is `undefined` on one, which reads as "no caret" and
 * silently sends the cursor to the front of whatever someone was typing.
 */
function caretOffset() {
  const sel = window.getSelection();
  return sel && sel.rangeCount ? sel.getRangeAt(0).startOffset : null;
}

function putCaret(node, offset) {
  const text = node.firstChild;
  const range = document.createRange();
  // Clamped, and defaulting to the end. A rebuild can shorten the text under the caret, and a
  // programmatic focus (Ctrl F on a field that already holds a path) wants the end, not the front.
  const length = text?.length ?? 0;
  if (text) range.setStart(text, Math.min(offset ?? length, length));
  else range.setStart(node, 0);
  range.collapse(true);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
}

/** Where the lookup field is, so Ctrl F and a rebuild can both put the caret back in it. */
const activityInputRef = { node: null };

/** What each of this screen's three dialogs draws, and whether it wears a title row at all. */
function activityDialog(id) {
  const props = activityProps();
  if (id === "details") {
    const body = {
      counters: store.select.statCounters(),
      // The FIXTURE's config first. `configInfo` is filled by `refreshConfig`, which is a round
      // trip — so under `?frame=` the first render has none, and two of these eight rows would
      // draw a dash where the frame draws a value.
      config: activeFixture()?.config ?? configInfo,
      socketOk: Boolean(store.select.response()) && !store.select.error(),
      historyCount: props.history.length,
    };
    return {
      signature:
        JSON.stringify(body.counters) + JSON.stringify(body.config) + body.socketOk + body.historyCount,
      children: renderDetailsBody(body),
    };
  }
  if (id === "neverSynced") {
    return {
      subtitle: ACTIVITY.neverSyncedDialog.sub,
      title: props.never
        ? ACTIVITY.neverSyncedDialog.title(props.never.total)
        : ACTIVITY.neverSyncedDialog.title(0),
      children: renderNeverSyncedBody({
        never: props.never,
        onClose: () => closeOverlay(),
        onChangeRule: () => navigate("settings"),
      }),
    };
  }
  if (id === "filePending") {
    const transfer = store.select.response()?.activity?.transfer ?? null;
    if (!transfer) return null;
    // NO TITLE ROW AND NO ✕ — this dialog draws neither, so it takes no `dialogHead` and needs an
    // `aria-label` of its own instead of pointing at a heading that does not exist.
    return {
      head: false,
      label: ACTIVITY.lookup.pending,
      signature: JSON.stringify(transfer),
      children: renderFilePendingBody({ transfer }),
    };
  }
  return null;
}

// ---- data ----
async function refreshConfig() {
  try {
    configInfo = await api.readConfig();
    // A missing config file reads back as an empty doc (not an error), so a successful read means we
    // now *know* whether a folder pair exists — the signal nextOnboardingLatch needs to distinguish a
    // fresh machine from a config file that simply hasn't been read yet.
    configLoaded = true;
  } catch (_) {
    /* config not readable yet — leave placeholders */
  }
  render();
}

async function poll() {
  let payload = null;
  try {
    payload = await api.getStatus();
    // Set before setStatus (which synchronously re-renders) so the onboarding-routing gate sees that
    // a real poll has now completed — only then may an `unreachable` reply mean a genuinely fresh
    // machine rather than the pre-poll default.
    statusPolled = true;
    store.setStatus(payload);
  } catch (e) {
    statusPolled = true;
    store.setStatus({ state: "unreachable", error: String(e) });
  }
  const now = Date.now();
  if (now - lastConflictScan > 15000) {
    lastConflictScan = now;
    try {
      store.setConflicts(await api.scanConflicts());
    } catch (_) {
      store.setConflicts([]);
    }
    // Re-read the GUI config file on the same slow cadence: onboarding or an external edit may
    // have (re)written it since boot, and it also drives the no-daemon fallback pair display.
    refreshConfig();
  }
  // Pending deletions ride on the status reply itself — no second IPC round trip per tick.
  if (payload?.response) store.setPendingDeletions(payload.response.pending_deletions ?? []);
  scheduleNextPoll();
}

function scheduleNextPoll() {
  clearTimeout(pollTimer);
  pollTimer = setTimeout(poll, document.hasFocus() ? 2000 : 10000);
}

// ---- boot ----
function main() {
  initTheme();
  store.subscribe(render);
  render();
  refreshConfig();
  poll();
  window.addEventListener("focus", scheduleNextPoll);
  document.addEventListener("keydown", onKeydown);

  // Tray menu items ask the shell to navigate. Routed through the api facade — no direct
  // window.__TAURI__ here.
  api.onTrayNavigate((id) => {
    if (typeof id !== "string") return;
    // tray.rs is Rust and does not move when this table does — it still emits the v1 `history`.
    const target = resolveRoute(id);
    if (ROUTES[target]) navigate(target);
    else console.warn(`tray-navigate: no route for "${id}" — add an alias in routes.js`);
  });
}

main();
