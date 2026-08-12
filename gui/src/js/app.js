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
import {
  ROUTES,
  FOOTER_ORDER,
  isOverlay,
  isDialog,
  nextOnboardingLatch,
  releasesOnboarding,
} from "./routes.js";
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
import { bannerFor, payloadFor, renderBanner } from "./ui/notification.js";
import { decide, emptyState } from "./notifier.js";
import { trayView, renderTrayPanel, updateTrayPanel } from "./screens/tray.js";
import { ACTIVITY, CHROME, ONBOARDING, SETTINGS } from "./ui/copy.js";
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
  normaliseQuery,
  passesSummaryOf,
} from "./screens/activity.js";
import {
  renderSettings,
  renderSettingsBar,
  renderSaveRefused,
  settingsBarShape,
  configUpdate,
  isDirty,
  removalCost,
} from "./screens/settings.js";
import {
  renderOnboarding,
  updateOnboarding,
  unmountOnboarding,
  renderOnboardingFooter,
  onboardingBarShape,
  mergeOutcomeOf,
  firstSyncShape,
  updateFirstSync,
  renderFirstSync,
  renderConsent,
  renderCliMissing,
} from "./screens/onboarding.js";
import { severityOf } from "./ui/rows.js";
import { activeFixture, fid } from "./fixtures/frames.js";
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
/**
 * Why the last `read_config` failed, or null.
 *
 * `configLoaded` alone cannot tell "not read yet" from "will never read": `read_config` rejects an
 * unparseable or unreadable file and `refreshConfig` swallowed it, so a config with a typo in it
 * left the Settings screen drawing an EMPTY, VALID config — blank folders, live updates on, and a
 * deletion-policy card selected that is not the one the daemon is running on.
 */
let configError = null;
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
 * `paused`, `unreachable`, `authExpired` and `failed` have NO drawn chip anywhere in the prototype.
 * They take the quiet form with their own text rather than an invented colour — the hexagon and the
 * main screen carry those states, which is where the design puts them.
 */
function chipFor() {
  // `step 1 of 2` / `step 2 of 2` — the one chip in the app that is not daemon-derived. Keyed off
  // the route as well as the latch, because a `?frame=` selecting either step polls its own
  // unreachable status and the latch has not closed yet on the first render.
  if (onboardingLatch || activeRoute() === "onboarding") {
    return { variant: "step", text: CHROME.chips.step(onboardingStepNow() === "review" ? 2 : 1) };
  }
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
  // #246 at 11px. Without this arm a failed pass falls through to `idle` HERE too — the chip is a
  // second derivation of the same question, and it would have gone on saying `idle` in the corner
  // of a window whose hero says the last sync didn't finish. The daemon's own word for it, which is
  // what `record_status_history` writes as the pass's message ("sync failed", src/daemon.rs).
  //
  // NOT S5's words for the same pass: `passRowFor` labels every failed row `Couldn't reach Proton
  // Drive`, which §90d rules out for this state precisely because a pass can fail with Proton
  // perfectly reachable. That is a pre-existing S5 wording and not this chip's business (#258).
  if (state === "failed") return { variant: "idle", text: "sync failed" };
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

/**
 * Is this window the tray panel? `panel.rs` opens `index.html?surface=tray`.
 *
 * Read from the URL rather than asked of Tauri's window label, for one reason that is not
 * convenience: it works in a browser. Every other design-v2 surface can be opened with `?frame=`
 * and looked at, and a tray panel that could only be seen by running the packaged app on a desktop
 * with a status-notifier host would be the one screen in the build nobody could review.
 */
function isTraySurface() {
  return new URLSearchParams(location.search).get("surface") === "tray";
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
    // The tray panel is a popover and Esc dismisses it — before anything else, because none of the
    // shell's other Esc targets exist in that window. Blur hides it too (`lib.rs`), but a keyboard
    // user who never leaves the panel would otherwise have no way out of it at all.
    if (isTraySurface()) {
      api.hideTrayPanel();
      e.preventDefault();
      return;
    }
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
  // The two standalone banner frames (S9). Latched for `panel`'s reason.
  banner: null,
  // The tray panel (S8). Latched like `panel` and for a harder reason: it holds an animated mark
  // and a focusable menu, and the poll behind it runs every ~2s.
  trayPanel: null,
  // The preview's own pages (F9). Latched for the same reason `panel` is, and it is not hypothetical:
  // `render()` returning early does not stop the poll — `main()` starts it unconditionally and
  // `store.subscribe(render)` re-enters here on every reply — so without this the frame index rebuilt
  // its whole list every ~2 s and a tabbed-to link lost focus, which is the failure this cache exists
  // for. Its content cannot change either: it is derived from the registry, which is a constant.
  preview: false,
};

/**
 * Mark the root as holding a compact panel rather than the shell, so it stops capping its height.
 *
 * THE ROOT WAS SHRINKING THE PANEL, AND IN THE TRAY WINDOW THAT IS A ONE-WAY RATCHET. `#app-root` is
 * a `height:100vh` flex column built for the shell, and a flex item shrinks: the panel measures
 * `min(its content, the viewport)`. `reportTrayHeight` sends that measurement to `panel.rs`, which
 * sizes the WINDOW to it — so the number is capped by the thing it sets. One short measurement and
 * the panel can never grow again, at any state, for the life of the window: the settled panel
 * reported 302 where it draws 365, and the two rows past the cut were `Close window · keeps syncing`
 * and `Quit · stops syncing` — the pair `10-tray.md` calls the single worst misunderstanding a tray
 * app can cause. Reproduced by opening the panel on a cold webview profile, where the first frame is
 * measured before the window settles at its built size; the race is incidental, the latch is not.
 *
 * Both mounts below take it, and they must stay in step: the fidelity gate only ever renders the
 * preview one, so dropping it from `mountTrayPanel` alone leaves the gate green and the app broken.
 */
function panelSurface(root) {
  root.classList.add("is-panel-surface");
  return root;
}

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
  panelSurface(root).replaceChildren(dom.panel);
  return true;
}

/**
 * The tray panel surface (S8) — `index.html?surface=tray`, which is the URL `panel.rs` opens.
 *
 * A QUERY PARAMETER RATHER THAN A SECOND HTML FILE. `index.html`'s own comment warns that its
 * stylesheet chain is easy to forget a link of and that forgetting one is silent — a mark that stops
 * animating, a seam that snaps. A second copy of that chain is the same trap with a second chance to
 * fall into it, and the day someone adds a stylesheet to one file and not the other the tray panel
 * is a blank window with no error. It also means `?surface=tray` opens the panel in a browser, which
 * is how the states no frame draws were looked at.
 *
 * PATCHED, NOT REBUILT, and this is the surface where that matters most: the syncing panel holds an
 * animated hexagon segment, the poll is ~2s, and a rebuild restarts the animation from 0% and drops
 * focus out of the menu — in a popover people click through in about a second. `updateTrayPanel`
 * returns false when the SHAPE changed (a different state, a different row count), which is the
 * signal to render a fresh one; `ui/compact.js` documents that contract and S8 is its first caller.
 */
/**
 * The two standalone banner frames (S9) — `11a Outage` and `11a Grouped`.
 *
 * A banner is not a window and not a panel: it is drawn by the desktop's notification server in the
 * shipped app, so it has no route and nothing in the shell renders one. The frames still have to be
 * reproducible, and the component that draws them is the same one `payloadFor` flattens for D-Bus —
 * which is what makes the drawn banner and the delivered one the same sentence.
 *
 * 520px, which both frames declare and neither `11a In situ` banner does. Mounted once: a fixture
 * cannot change, and the poll behind this would otherwise rebuild it every ~2s.
 */
function mountFrameBanner(root) {
  if (dom.banner) return true;
  const spec = activeFixture()?.notification;
  if (!spec) return false;
  dom.banner = renderBanner(bannerFor(spec.event), { at: spec.at, width: 520 });
  root.replaceChildren(dom.banner);
  return true;
}

function mountTrayPanel(root) {
  if (!isTraySurface()) return false;
  // Before the first render rather than beside `replaceChildren`, because the patch path below
  // returns without touching the root and `reportTrayHeight` runs on both.
  panelSurface(root);
  const view = trayView({
    daemonState: store.select.daemonState(),
    response: store.select.response(),
    conflicts: store.select.conflicts(),
    deletions: store.select.pendingDeletions(),
  });
  if (dom.trayPanel && updateTrayPanel(dom.trayPanel, view)) {
    reportTrayHeight();
    return true;
  }
  dom.trayPanel = renderTrayPanel(view, (id) => {
    api.trayAction(id).then((payload) => store.setStatus(payload));
  });
  root.replaceChildren(dom.trayPanel);
  reportTrayHeight();
  return true;
}

/**
 * Tell the window how tall the panel came out.
 *
 * The four drawn states span 321.5px to 441.5px and Phase 1 omits lines the frames draw (the offline
 * panel has no `retrying in 40s`), so no fixed height is right for more than one of them: too short
 * clips the menu, too tall leaves a band of empty panel under it. Measuring the DOM is the only
 * source that is right in every state including the ones nothing drew.
 *
 * `requestAnimationFrame` because the panel has just been put in the document and has no layout yet
 * — reading `offsetHeight` in the same tick returns the previous state's height, which is the subtle
 * version of this bug: the panel is the right size one poll late, every time.
 *
 * THIS MEASUREMENT SETS THE WINDOW IT IS MEASURED IN, so nothing may cap the measured node at the
 * window's own size or the loop latches at its first wrong answer and no later poll can undo it.
 * `panelSurface` is what holds that open; a rule that shrinks `.compact-panel` to its container
 * re-arms it.
 */
function reportTrayHeight() {
  requestAnimationFrame(() => {
    const height = dom.trayPanel?.offsetHeight;
    if (height) api.resizeTrayPanel(height);
  });
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
  if (mountFrameBanner(root)) return;
  // THE TRAY PANEL TAKES THE WINDOW, AND IT TAKES IT BEFORE THE ONBOARDING LATCH.
  //
  // That order is the whole of the decision. `nextOnboardingLatch` returns true for a fresh machine,
  // and the takeover is a 1040px, four-step, undismissable surface — drawn inside a 362px borderless
  // popover it would be an unusable sliver of a wizard with no way out, over a daemon the user was
  // only glancing at. The tray's own answer for that state is `Nothing has synced yet` with a row
  // that opens the window, which is where onboarding belongs. See `screens/tray.js`.
  if (mountTrayPanel(root)) return;
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
  //
  // THE FLOW OVERRIDES THE LATCH once it is past the takeover, and it has to: `derive_state` returns
  // `firstRun` for a reachable daemon that has never synced, which is exactly what the daemon is
  // between `start_service` and its first pass beginning — so the latch would re-enter mid-merge and
  // redraw step 2, with its stale plan, behind the merge dialog. `nextOnboardingLatch` stays pure;
  // this is the caller knowing something the daemon state cannot say.
  const wasOnboarding = onboardingLatch;
  // `nextOnboardingLatch`'s RELEASE SET, ASKED FOR RATHER THAN RESTATED. This line used to be a
  // second copy of that list, and the copy was not a duplicated screen — it was a KILL SWITCH for
  // the original: `onboardingFailure` short-circuits the ternary below, so any state the two lists
  // disagreed about made the release arm in `routes.js` dead code on exactly its own path. Adding
  // `failed` there and not here latched the wizard shut on a failed first sync, which is the
  // opposite of what that arm is for, and three comments claimed otherwise while it did. #246.
  //
  // `firstRun` is left out of the set on purpose: the latch treats it as an ENTRY trigger, not a
  // release, and `counters_unknown()` groups it with `unreachable` in both gui-core and store.js.
  // Leaving the failure latched there changes nothing — `nextOnboardingLatch` returns true for
  // `firstRun` anyway, so both arms of the ternary agree. (A failed pass cannot derive to `firstRun`
  // within one daemon process in any case: `record_status_history` runs on the same pass, and
  // `firstRun` requires an empty history.)
  const reachable = releasesOnboarding(st);
  // A merge that failed against a daemon that then came up is not onboarding's problem any more.
  if (onboardingFailure && reachable) onboardingFailure = null;
  onboardingLatch =
    onboardingStage !== null
      ? false
      : onboardingFailure
        ? true
        : nextOnboardingLatch(
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
  // The CLI check runs before the flow has a config, so it is asked as soon as the takeover opens
  // (and once per app run). The merge's own progress is what advances the flow past it.
  if (onboardingLatch || activeRoute() === "onboarding") ensureCliCheck();
  advanceOnboardingStage();

  // The two layers, read back out. A dialog floats over whatever body is showing — which may be a
  // screen overlay and not `route`, and getting that wrong is what loses the user's place. See
  // routes.js `isDialog` and DEVIATIONS §57.
  //
  // A FIXTURE MAY ALSO NAME THE DIALOG IT DRAWS, for the same reason `activeRoute` lets it name a
  // route: three of S5's six frames ARE dialogs (`6a Details`, `7a Never synced`, `7a File
  // pending`), and `dialogOverlay` is module state no `?frame=` can reach. Without this the harness
  // opens the underlying screen and files all three under "screen not built yet".
  // `filePending` describes a transfer that is happening, and when it finishes there is nothing left
  // for the dialog to say — so it closes itself rather than letting the fallback below replace its
  // body with the not-built-yet placeholder.
  //
  // A REPLY THAT SAYS "NO TRANSFER" IS NOT THE SAME AS NO REPLY. `response()` is null whenever the
  // poll throws — `poll()` publishes `{ state: "unreachable" }` with no `response` — so testing the
  // transfer alone closed the dialog on one failed round trip, while the upload it describes was
  // still running. That is this project's own rule about unknown never rendering as zero
  // (`countersUnknown`, `dash()`), one layer up: an absent answer is not an answer.
  const reply = store.select.response();
  if (dialogOverlay === "filePending" && !activeFixture() && reply && !reply.activity?.transfer) {
    dialogOverlay = null;
    dialogReturn = null;
    activityPendingTransfer = null;
  }
  // ONBOARDING'S OWN DIALOGS OUTRANK BOTH. `9a CLI missing` floats over the takeover — the takeover
  // used to null this line outright — and `9a First sync` / `9a Consent` float over whatever the
  // released latch left behind. None of the three is in `dialogOverlay`, so Esc cannot reach them.
  const dialogRoute = onboardingDialog() ?? (onboardingLatch ? null : dialogOverlay);
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
    // Keyed off the ROUTE and not the latch alone: `9a Review` carries a written folder pair, which
    // is exactly the state the latch does not re-enter on, so a fixture selecting step 2 renders
    // with the latch open and would grow a ⋯ the frame does not draw.
    hasMenu: !onboardingLatch && active !== "onboarding",
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
    hasHome: !onMain && !onboardingLatch && active !== "onboarding",
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
  } else if (active === "settings") {
    // REBUILT EVERY PASS, with the focused field's caret put back — the same trade the activity
    // screen makes and for the same reason, except that this screen has five text fields rather
    // than one. Everything they hold lives in `settingsEdits`, so a rebuild loses nothing except
    // the selection, which is restored below; patching instead would mean a diff over four tab
    // bodies to protect state that is not in the DOM in the first place.
    if (dom.bodyRoute !== active) {
      unmountScreens();
      resetSettingsScreen();
    }
    // EVERY CONTROL, NOT JUST THE TEXT FIELDS. This screen is a form with twenty-one focusable
    // controls and five inputs, and it is rebuilt on every poll — so restoring only the inputs left
    // the keyboard on `<body>` within two seconds of tabbing to a radio card, a tab pill or the
    // toggle. `data-sfocus` names each one (see `focusable` in screens/settings.js); the scan
    // rather than a selector is because a rule's id carries its pattern, which can hold anything.
    const focused = document.activeElement;
    const key = focused instanceof HTMLElement ? focused.closest("[data-sfocus]")?.dataset.sfocus : null;
    // The SELECTION, not just the caret: a poll landing on a double-clicked path segment collapsed
    // it, so the next keystroke inserted where it should have replaced.
    const input = focused instanceof HTMLInputElement ? focused : null;
    const range = input ? [input.selectionStart, input.selectionEnd, input.selectionDirection] : null;
    // AND THE SCROLL POSITION, which the focus restore below does not cover. Two blocks on this
    // screen are taller than their box — the skip tab's rule list and the notifications tab's rules
    // sheet — and a rebuild every ~2s put both back at the top, which makes the bottom of either
    // one physically unreadable. Keyed by name rather than by node, because the node is a new one.
    const scrolled = new Map();
    for (const node of document.querySelectorAll("[data-scroll]")) {
      if (node.scrollTop) scrolled.set(node.dataset.scroll, node.scrollTop);
    }
    setBody(renderSettings(settingsProps()));
    for (const node of document.querySelectorAll("[data-scroll]")) {
      const at = scrolled.get(node.dataset.scroll);
      if (at) node.scrollTop = at;
    }
    dom.bodyRoute = active;
    if (key) {
      const next = [...document.querySelectorAll("[data-sfocus]")].find((n) => n.dataset.sfocus === key);
      if (next) {
        next.focus();
        if (range && next instanceof HTMLInputElement && range[0] != null) {
          next.setSelectionRange(range[0], range[1] ?? range[0], range[2] ?? "none");
        }
      }
    }
  } else if (active === "onboarding") {
    // PATCHED, NOT REBUILT, for the same reason the settings screen is: step 1 holds the remote
    // path in a live `<input>`, and a rebuild on the ~2s poll would move the caret to the end of
    // whatever is being typed. `updateOnboarding` rebuilds only when the body itself has moved.
    if (dom.bodyRoute !== active) {
      unmountScreens();
      setBody(renderOnboarding(onboardingProps()));
      dom.bodyRoute = active;
    } else {
      const nodes = updateOnboarding(onboardingProps());
      if (nodes) setBody(nodes);
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
          : // The settings bar is REBUILT rather than patched, and it holds no typed state to
            // protect: `Save`'s enabled-ness, the note and the amber cost line all move with
            // `settingsEdits`, and every one of them changes on the same keystroke. `dataset.shape`
            // is what makes the rebuild conditional — an unchanged bar is left where it is.
            owner === "settings"
            ? settingsBarUnchanged(dom.footer)
            : // Same trade on the onboarding bar, and it matters more: `See what will happen` arms
              // as the remote field is typed into, and rebuilding the bar under the caret is what
              // `updateOnboarding` is avoiding one layer up.
              owner === "onboarding"
              ? dom.footer.dataset.shape === onboardingBarShape(onboardingProps())
              : true;
  }
  if (!patched) {
    const built =
      kind === "actionBar"
        ? owner === "plan"
          ? renderPlanBar(planProps())
          : owner === "settings"
            ? renderSettingsBar(settingsProps())
            : owner === "onboarding"
              ? renderOnboardingFooter(onboardingProps())
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
      const content = dialogContentFor(dialogRoute);
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
        children: dialogChildren(dspec, content, title, headless),
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
    const content = dialogContentFor(dialogRoute);
    if (content?.signature && content.signature !== dom.dialogSignature) {
      const dspec = ROUTES[dialogRoute];
      const [w] = dspec.size ?? [522, null];
      const title = content.title ?? dspec.label ?? titleFor(dialogRoute);
      const surface = dom.dialog.querySelector(".dialog");
      const focusables = [...surface.querySelectorAll("button, input, [tabindex]")];
      const at = focusables.indexOf(document.activeElement);
      // THE HEAD IS REBUILT TOO, and leaving it out was a bug rather than an economy: `7a Never
      // synced`'s title COUNTS the rules (`4 files are never synced`), so a head that survives the
      // update keeps whatever number was known when the dialog opened — which, on the render that
      // mounts it, is none. The surface itself stays, so the appear animation does not restart.
      surface.replaceChildren(...dialogChildren(dspec, content, title, content.head === false, w));
      if (at >= 0) {
        const next = [...surface.querySelectorAll("button, input, [tabindex]")];
        (next[at] ?? surface).focus();
      }
      dom.dialogSignature = content.signature;
    }
  }

  // The merge dialog's two moving numbers, patched rather than rebuilt: its mark is the syncing
  // hexagon, and a rebuild restarts both travelling segments from 0% twice a second.
  if (dialogRoute === "firstSync" && dom.dialog) {
    const merging = store.select.response();
    updateFirstSync(dom.dialog.querySelector(".dialog"), {
      pending: remainingOf(merging?.activity ?? null, merging),
      activity: merging?.activity ?? null,
    });
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
  unmountOnboarding();
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
let activityLookupTimer = null;
/**
 * Which in-flight path this session has already offered the pending dialog for.
 *
 * A LATCH, not a flag, and without it the dialog is a trap: the trigger is "the looked-up path is
 * the one moving", which stays true after you dismiss it — so Esc would close the dialog and the
 * next render would open it again.
 */
let activityPendingShown = null;
/** The in-flight transfer the pending dialog is describing, held across a poll that came back empty. */
let activityPendingTransfer = null;

/**
 * How long to wait after a keystroke before asking the index.
 *
 * `path_sync_status` is SYNCHRONOUS on the Rust side and its own module header warns it "can hold
 * the loop for its full 3s index busy timeout". Asking on every keystroke puts one index open per
 * character into a queue behind the daemon's own writer; typing a 20-character path is 20 of them,
 * and the answers arrive in an order the latest-wins guard then has to throw away. 180ms is below
 * the ~250ms that reads as lag and above a fast typist's inter-key gap.
 */
const LOOKUP_DEBOUNCE_MS = 180;
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
  activityPendingShown = null;
  activityPendingTransfer = null;
  clearTimeout(activityLookupTimer);
  activityLookupTimer = null;
  skipRuleReport = null;
  skipRuleAsked = false;
}

/** The exclude rules' cost, once per visit. Fired and not awaited, like every other screen's fetch. */
async function ensureSkipRules() {
  // `configLoaded`, NOT `configInfo`. The rules come from the config file, and `read_config` is a
  // round trip — so the first render of this screen has no config at all, sees an empty `exclude`,
  // and would latch `skipRuleAsked` on the strength of not having asked yet. The band would then
  // never appear until you left the screen and came back. Latching only once the config is
  // genuinely known is what makes "no rules" mean no rules.
  if (skipRuleAsked || !configLoaded) return;
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
  const path = normaliseQuery(query);
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
  let failure = null;
  try {
    status = await api.pathSyncStatus(path);
  } catch (error) {
    console.error("path_sync_status failed:", error);
    // KEPT, not swallowed. A caught error and a path that is not in the index both leave `status`
    // null, and the screen must not tell someone their file is missing when the check is what
    // failed. The daemon's own words go through untouched, to be quoted in mono.
    failure = String(error?.message ?? error);
  }
  if (activityLookupInFlight !== path) return;
  activityLookupInFlight = null;
  activityLookup = { path, status, error: failure };
  // THE PENDING DIALOG'S TRIGGER, and it is the only one the data supports. `7a File lookup` and
  // `7a File pending` are the same lookup in two states — a file that is settled, and a file that
  // is moving right now — so looking up the file the daemon is currently transferring is what
  // tells the two apart. Nothing else could: `SyncActivity` carries exactly ONE in-flight transfer
  // (#211), so a lookup for any other moving file cannot reach this state at all.
  //
  // Latched, so dismissing it sticks. The condition stays true for as long as the transfer runs.
  const moving = store.select.response()?.activity?.transfer ?? null;
  if (moving?.path === path && activityPendingShown !== path) {
    activityPendingShown = path;
    navigate("filePending");
    return;
  }
  render();
}

/**
 * A dialog's children — the head, then whatever the screen puts under it.
 *
 * SHARED BY THE MOUNT AND THE UPDATE, which is the whole point. Written twice, the update quietly
 * grew a different dialog from the one that opened: the first version rebuilt only the body, so
 * `7a Never synced`'s counted title stayed at whatever was known before its data arrived.
 */
function dialogChildren(dspec, content, title, headless, width = dspec.size?.[0] ?? 522) {
  const head = headless
    ? null
    : dialogHead({
        title,
        subtitle: content?.subtitle ?? null,
        id: "dialog-title",
        size: width >= 600 ? "wide" : "compact",
        // Per route, not always. `8a Save refused` and `9a CLI missing` draw no ✕ at all — they
        // are asking you to choose between two repairs, and a dismiss button in the corner is a
        // third answer the design does not offer. Esc still closes them, through F4's chain.
        onClose: dspec.closable ? () => closeOverlay() : null,
      });
  if (head) {
    // The head's own nodes, stamped here because this is where they are built. `dialogHead` cannot
    // do it: `ui/dialog.js` is a foundation primitive and importing `fixtures/frames.js` there
    // would close the cycle that module's header forbids.
    fid(head, "dlgHead");
    fid(head.querySelector(".dialog-headings"), "dlgHeadings");
    fid(head.querySelector(".dialog-title"), "dlgTitle");
    fid(head.querySelector(".dialog-subtitle"), "dlgSub");
    fid(head.querySelector(".dialog-close"), "dlgClose");
  }
  return [
    head,
    ...(content?.children ?? [
      screenPlaceholder(title, dspec.task && dspec.issue ? `${dspec.task} · issue ${dspec.issue}` : null),
    ]),
  ].filter(Boolean);
}

/** Everything the activity screen reads, plus the actions it can take. */
function activityProps() {
  const ui = activeFixture()?.ui ?? null;
  const response = store.select.response();
  const history = response?.status_history ?? [];
  const lastSync = response?.last_sync_epoch_secs ?? null;

  // Fired and not awaited — see `ensureSkipRules`. Not on the passes tab, which draws none of it —
  // but the never-synced DIALOG needs the same report, and it opens over either tab.
  const tab = ui?.tab ?? activityTab;
  if (tab === "files" || dialogOverlay === "neverSynced" || ui?.dialog === "neverSynced") ensureSkipRules();

  const never = neverSyncedFrom(skipRuleReport);
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
    // WHETHER THE TWO SIDES ARE KNOWN TO AGREE, and nothing else may stand in for it. `Both sides
    // agree` over a settled hexagon is the strongest claim this app makes; `derive_state` reports
    // `idle` only for a daemon that answered and has nothing outstanding, and a last pass is what
    // gives the claim a moment to be true at. `copy.js` records the identical failure on the main
    // screen — a state falling through to `Everything is up to date` "would be a false all-clear on
    // a daemon that cannot reach Proton at all".
    agreed: store.select.daemonState() === "idle" && lastSync != null,
    onQuery: (value) => {
      activityQuery = value;
      // The field repaints NOW and the index is asked later — the two are deliberately not
      // coupled. A control that waits 180ms to show what you typed is a broken control.
      clearTimeout(activityLookupTimer);
      activityLookupTimer = setTimeout(() => lookupPath(value), LOOKUP_DEBOUNCE_MS);
      render();
    },
    onClearQuery: () => {
      activityQuery = "";
      activityLookup = null;
      activityLookupInFlight = null;
      activityPendingShown = null;
      clearTimeout(activityLookupTimer);
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
  // Guarded for the same reason `caretOffset` is: `getSelection()` answers null in a detached or
  // sandboxed document, and this one would take the whole render down with it.
  if (!sel) return;
  sel.removeAllRanges();
  sel.addRange(range);
}

/** Where the lookup field is, so Ctrl F and a rebuild can both put the caret back in it. */
const activityInputRef = { node: null };

/**
 * Which screen owns the dialog that is open. One function per screen rather than one growing
 * switch: a dialog's contents are the screen's business, and `activityDialog` already returns null
 * for anything it does not own.
 */
function dialogContentFor(id) {
  if (id === "firstSync" || id === "consent" || id === "cliMissing") return onboardingDialogContent(id);
  return id === "saveRefused" ? settingsDialog(id) : activityDialog(id);
}

/**
 * `8a Save refused`. No title row and no ✕ — the frame draws neither, and the route says so.
 *
 * Returns null with no error to show, which is what keeps a dismissed refusal dismissed: the
 * dialog's own `Go back and fix it` clears the error as it closes.
 */
function settingsDialog(id) {
  if (id !== "saveRefused") return null;
  const error = activeFixture()?.saveError ?? settingsError;
  if (!error) return null;
  return {
    head: false,
    label: SETTINGS.refusedTitleUnknown,
    signature: String(error),
    children: [
      renderSaveRefused({
        error,
        onBack: () => {
          settingsError = null;
          closeOverlay();
        },
      }),
    ],
  };
}

/** What each of this screen's three dialogs draws, and whether it wears a title row at all. */
function activityDialog(id) {
  const props = activityProps();
  if (id === "details") {
    const summary = store.select.planSummary();
    const body = {
      // NOT `statCounters()`, and the difference is one row. That selector answers the MAIN
      // screen's tiles, where `conflicts` means "unresolved sidecars on disk" — a scan of the
      // filesystem. This panel is labelled with the wire's own field names, and `conflicts` is
      // literally a `PlanSummary` field: what the last plan found. The two are different
      // quantities, and the gate cannot tell them apart here because both drew a single digit.
      counters: {
        pending_changes: store.select.pendingChanges(),
        conflicts: summary?.conflicts ?? null,
        destructive_actions: summary?.destructive_actions ?? null,
        skipped_unsupported: summary?.skipped_unsupported ?? null,
      },
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
      signature: JSON.stringify(props.never),
      children: renderNeverSyncedBody({
        never: props.never,
        onClose: () => closeOverlay(),
        onChangeRule: () => navigate("settings"),
      }),
    };
  }
  if (id === "filePending") {
    // The LAST TRANSFER SEEN, when the daemon has gone quiet. The close above now keeps the dialog
    // open through an unreachable poll, so this has to have something to draw — and the last thing
    // known to be true beats both a placeholder and a blank. `started_epoch_secs` keeps the
    // sub-line honest while it waits: the transfer did start then, however long ago that now reads.
    const live = store.select.response()?.activity?.transfer ?? null;
    if (live) activityPendingTransfer = live;
    const transfer = live ?? activityPendingTransfer;
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

// ---- the settings screen (S6) ----
//
// THE STAGED EDIT LIVES HERE, NOT IN THE DOM, and it has to: the body is rebuilt or patched on
// every 2s poll, so a form that kept its half-typed folder path in an `<input>`'s value would lose
// it twice a second. `settingsEdits` is the only record of what has been changed and not saved —
// `Discard changes` is `settingsEdits = {}` and nothing else.

let settingsTab = "folders";
/** Staged fields, keyed exactly as `ConfigPayload` names them. Empty means nothing to save. */
let settingsEdits = {};
/**
 * The two add fields, ONE PER LIST. Not config fields: a draft is staged only once `Add` is pressed.
 *
 * Two, not one, and the reason is that the lists mean opposite things. A pattern typed into the skip
 * tab HIDES what it matches; the same pattern in Advanced's include list makes it the only thing
 * that syncs. One shared buffer would carry a half-typed `*.psd` across a tab switch and hand it to
 * whichever `Add` was pressed next — inverting what the person meant, on the two settings that
 * decide what is backed up at all.
 */
let settingsDrafts = { exclude: "", include: "" };
let settingsSaving = false;
/**
 * A `Sweep now` in flight.
 *
 * The daemon reports `syncing` and the button disables on it, but the poll is 2s away and the click
 * has to answer NOW: without this the button stays live for up to two seconds after being pressed,
 * which is how a second sweep gets queued by someone who thought the first one missed. PR #140
 * filed the same shape on the approve/deny buttons.
 */
let settingsSweeping = false;
/** The daemon's refusal, verbatim. Non-null is what opens `8a Save refused`. */
let settingsError = null;
/**
 * A save landed and the daemon is still running the old config — see `SETTINGS.savedNote`.
 *
 * SET FOR A CONFIG WRITE ONLY. A policy-only save writes `gui.toml`, which the daemon never reads,
 * so "the sync service is still running the old settings until it restarts" would be a sentence
 * about a file nothing is waiting on — and it also swaps `Discard changes` for `Restart the
 * service`, offering to bounce the daemon for a setting the daemon has never heard of. What a
 * policy-only save leaves behind is what any form leaves behind: a `Save` that has gone quiet
 * because there is nothing left to save.
 */
let settingsSaved = false;
/** A restart asked for and not yet answered. `restart_service` can take ten seconds. */
let settingsRestarting = false;
/**
 * What the bar says about the last thing that was asked for — in flight, or failed.
 *
 * Every one of these was silence before the S6 review: `resync` RESOLVES with a socket error folded
 * into its payload rather than rejecting, so a `Sweep now` against a dead daemon did nothing at all
 * and said nothing at all; a failed `restart_service` wrote its reason into a variable only the
 * refusal dialog reads, and nothing opens that dialog from there.
 */
let settingsNotice = null;

/** Entering or leaving: nothing staged, no draft, no refusal, and the walk to be asked for again. */
function resetSettingsScreen() {
  settingsTab = "folders";
  settingsEdits = {};
  // WITH THE REST OF THE STAGED STATE. It is one of the two things a person can stage on this
  // screen and it lives outside `settingsEdits` (it is not a daemon-config key), so leaving it out
  // here made it the one edit that survived walking away: the card stayed chosen and the screen
  // stayed dirty about a value nothing had written, for the life of the window.
  notifyPolicyEdit = null;
  settingsDrafts = { exclude: "", include: "" };
  settingsSaving = false;
  settingsSweeping = false;
  settingsRestarting = false;
  settingsNotice = null;
  settingsError = null;
  settingsSaved = false;
  skipRuleReport = null;
  skipRuleAsked = false;
}

/** Stage one field. Any edit clears the saved notice: it is no longer describing what is on disk. */
function stageSetting(key, value) {
  settingsEdits = { ...settingsEdits, [key]: value };
  settingsSaved = false;
  settingsNotice = null;
  render();
}

/**
 * Everything the settings screen reads, plus the actions it can take.
 *
 * `saved` and `config` are BOTH here and they are different things: `saved` is the config on disk
 * (what the rules list and every measured count describe) and `config` is that with the staged edits
 * on top (what every control shows). `screens/settings.js`'s `rulesBlock` explains why the tab needs
 * both rather than one merged view.
 */
function settingsProps() {
  const ui = activeFixture()?.ui ?? null;
  const saved = activeFixture()?.config ?? configInfo ?? {};
  const tab = ui?.tab ?? settingsTab;
  // Fired and not awaited — see `ensureSkipRules`. Only the tab that draws the counts asks for the
  // walk; the other three would pay for a full metadata pass of the sync folder to draw nothing.
  if (tab === "skip") ensureSkipRules();
  // THE FIXTURE'S STAGED EDIT. `8a Skip rules` draws a removal staged but not saved, which is a
  // frontend state and not a config — so the frame names the rule in its `ui` and the edit is
  // reconstructed here, rather than the fixture shipping a second config that disagrees with the
  // first about what is on disk.
  const edits = ui?.removing
    ? { exclude: (saved.exclude ?? []).filter((p) => p !== ui.removing) }
    : settingsEdits;
  const config = { ...saved, ...edits };
  const skip = activeFixture()?.skipRules ?? skipRuleReport;
  // The staged policy counts as dirty like any other control, even though it is not part of the
  // config `write_config` sends — the footer promises "nothing is written until you save", and a
  // control that saved itself on click would be the one exception nobody was told about.
  const policyStaged = notifyPolicyEdit != null && notifyPolicyEdit !== notifyPolicy;
  const dirty = ui?.dirty ?? (isDirty(saved, edits) || policyStaged);
  return {
    tab,
    saved,
    config,
    skip,
    dirty,
    // The frame names it; otherwise the staged value, then what is on disk.
    notifyPolicy: ui?.notifyPolicy ?? notifyPolicyEdit ?? notifyPolicy,
    drafts: settingsDrafts,
    saving: settingsSaving,
    justSaved: settingsSaved,
    // The amber line, when a single removal is staged. Any other staged change leaves the neutral
    // note: the deck has one cost sentence and it says `One rule removed`, so a second removal has
    // no wording and inventing a plural would be inventing the number in it too.
    cost: removalCost(saved.exclude, config.exclude, skip),
    // WHAT JUST HAPPENED, which outranks even the cost line — see `barNoteOf`. A control that
    // failed has to be able to say so over standing information about a staged change, or the fix
    // for one silence introduces another.
    notice:
      settingsNotice ?? (settingsSaving ? SETTINGS.saving : settingsRestarting ? SETTINGS.restarting : null),
    // What a save left behind: the daemon is still running the old config until it restarts.
    note: settingsSaved ? SETTINGS.savedNote : null,
    // WHETHER THE CONFIG IS KNOWN AT ALL. `read_config` rejects an unparseable file and
    // `refreshConfig` swallows it, so `configInfo` stays null — and `?? {}` would draw that as an
    // empty, valid config: both folder fields blank, live updates on, and a deletion policy card
    // selected that is not the one running. A screen may not answer for a file it could not read.
    // `configLoaded && !configError`, and the second half is not redundant: `refreshConfig` runs on
    // a timer, so a file that PARSED once and stops parsing later leaves `configLoaded` true with a
    // stale `configInfo` behind it. The screen would then keep a deletion-policy card selected from
    // the last good read, underneath a banner saying the file could not be read — answering for it
    // and disclaiming it in the same breath.
    loaded: Boolean(activeFixture()) || (configLoaded && !configError),
    configError: activeFixture() ? null : configError,
    // The daemon is mid-pass, or one has just been asked for: `Sweep now` would queue behind it
    // with nothing to show for the click.
    syncing: settingsSweeping || Boolean(store.select.response()?.syncing),
    handlers: {
      onTab: (id) => {
        settingsTab = id;
        render();
      },
      onRoot: (key, value) => stageSetting(key, value),
      onField: (key, value) => stageSetting(key, value),
      onEvents: (on) => stageSetting("events_driven", on),
      onInterval: (secs) => stageSetting("scan_interval_secs", secs),
      // Writes BOTH booleans, always — a card that set one would leave a pair no card describes
      // (DEVIATIONS §68). `deletion_policy` goes with them because `write_config` applies it after
      // the two and a stale value there would overwrite what was just chosen.
      onPolicy: (policy) => {
        settingsNotice = null;
        settingsEdits = {
          ...settingsEdits,
          delete_approval_remote: policy.remote,
          delete_approval_local: policy.local,
          deletion_policy: policy.id,
        };
        settingsSaved = false;
        render();
      },
      // Staged, not written. `null` once it matches what is saved, so choosing the card that is
      // already selected does not mark the screen dirty — the same rule `configUpdate` applies to
      // every other control.
      onNotifyPolicy: (id) => {
        settingsNotice = null;
        notifyPolicyEdit = id === notifyPolicy ? null : id;
        settingsSaved = false;
        render();
      },
      // The Activity link inside the rules sheet. A real route change, not decoration.
      onRoute: (id) => navigate(id),
      onDraft: (key, value) => {
        settingsDrafts = { ...settingsDrafts, [key]: value };
        render();
      },
      onAddRule: () => addPattern("exclude"),
      onRemoveRule: (pattern) => removePattern("exclude", pattern),
      onAddInclude: () => addPattern("include"),
      onRemoveInclude: (pattern) => removePattern("include", pattern),
      onChoose: chooseLocalRoot,
      onSweep: sweepNow,
      onSave: saveSettings,
      onDiscard: () => {
        settingsEdits = {};
        settingsDrafts = { exclude: "", include: "" };
        notifyPolicyEdit = null;
        settingsSaved = false;
        render();
      },
      onRestart: restartAfterSave,
    },
  };
}

/** The current staged value of a list field, saved-or-staged. */
const stagedList = (key) => settingsEdits[key] ?? (activeFixture()?.config ?? configInfo)?.[key] ?? [];

function addPattern(key) {
  const pattern = settingsDrafts[key].trim();
  // A duplicate is not an error and not a second row: the rule is already there, so the field
  // clears and nothing is staged.
  if (pattern && !stagedList(key).includes(pattern)) {
    settingsEdits = { ...settingsEdits, [key]: [...stagedList(key), pattern] };
    settingsSaved = false;
  }
  settingsDrafts = { ...settingsDrafts, [key]: "" };
  render();
}

/** Un-stages an addition and stages a removal, from one path — both are "not in the staged list". */
function removePattern(key, pattern) {
  settingsEdits = { ...settingsEdits, [key]: stagedList(key).filter((p) => p !== pattern) };
  settingsSaved = false;
  render();
}

/**
 * `Choose…`. A dismissed picker answers `null`, which is not an error and must not read as one —
 * and a picker that could not OPEN rejects, which is an error and must not read as a dismissal.
 * `choose_folder` returns `Result<Option<String>, String>` precisely so the two stay apart.
 */
async function chooseLocalRoot() {
  try {
    const picked = await api.chooseFolder(settingsEdits.local_root ?? configInfo?.local_root ?? null);
    if (picked) stageSetting("local_root", picked);
  } catch (error) {
    settingsNotice = SETTINGS.chooseFailed(String(error?.message ?? error));
    render();
  }
}

/** `Sweep now` — a full-tree walk on the next pass, which `sync_now` is not. */
async function sweepNow() {
  if (settingsSweeping) return;
  settingsSweeping = true;
  settingsNotice = SETTINGS.sweeping;
  render();
  try {
    // THE REPLY HAS TO BE READ, not just awaited. `resync` is a status command, and every one of
    // them folds a socket failure into the payload rather than rejecting (`commands.rs`) — so
    // against a stopped daemon, or one older than `ControlCommand::Resync`, the `catch` below never
    // fires and an unread reply is a button that does nothing and says nothing.
    const reply = await api.resync();
    settingsNotice = reply?.error ? SETTINGS.sweepFailed(reply.error) : null;
  } catch (error) {
    settingsNotice = SETTINGS.sweepFailed(String(error?.message ?? error));
  }
  // Released as soon as the daemon has answered. From here the button stays disabled on the reply's
  // own `syncing`, which is the fact rather than our memory of having asked — so re-poll for it.
  settingsSweeping = false;
  clearTimeout(pollTimer);
  poll();
}

/**
 * Write the staged edits, and only them.
 *
 * The refusal opens `8a Save refused` rather than being swallowed: `write_config` rejects a config
 * the daemon's own parser would refuse, and a save that silently did nothing is the failure that
 * dialog exists to prevent. A successful save clears the staging and leaves the note saying the
 * daemon is still running the old config, because it is — there is no reload path in the engine.
 */
async function saveSettings() {
  const saved = activeFixture()?.config ?? configInfo ?? {};
  const update = configUpdate(saved, settingsEdits);
  const policy = notifyPolicyEdit != null && notifyPolicyEdit !== notifyPolicy ? notifyPolicyEdit : null;
  if (settingsSaving || (Object.keys(update).length === 0 && !policy)) return;
  settingsSaving = true;
  settingsError = null;
  render();
  // THE MAP AS IT WAS SENT. Every staging path assigns a fresh object, so identity is an exact
  // "nothing was staged since" test — and clearing the whole map on the way back would discard a
  // keystroke typed while the write was in flight, while telling the person it had been saved.
  const sent = settingsEdits;
  try {
    // TWO FILES, AND `notify_policy` NEVER GOES IN THE DAEMON'S. Its config parser is
    // `deny_unknown_fields`, so one stray key stops the daemon starting; the GUI's own `gui.toml`
    // is where a GUI-local preference belongs.
    //
    // THE CONFIG GOES FIRST, because it is the one that can be refused. `8a Save refused` says
    // "Nothing was saved. Your old settings are still running", and a policy written before a
    // refusal would make that sentence false about the one thing that HAD been written.
    if (Object.keys(update).length) await api.writeConfig(update);
    if (policy) {
      await api.writeNotifyPolicy(policy);
      notifyPolicy = policy;
      notifyPolicyEdit = null;
    }
    if (settingsEdits === sent) settingsEdits = {};
    settingsSaved = Object.keys(update).length > 0;
    // The rules changed under the report, so the counts on the skip tab are about a config that is
    // no longer on disk. Ask again rather than showing yesterday's numbers next to today's rules.
    skipRuleReport = null;
    skipRuleAsked = false;
    await refreshConfig();
  } catch (error) {
    settingsError = String(error?.message ?? error);
    openOverlay("saveRefused");
  }
  settingsSaving = false;
  render();
}

/** The saved-but-not-live prompt's action. Failure leaves the note saying so, not silence. */
async function restartAfterSave() {
  if (settingsRestarting) return;
  settingsRestarting = true;
  settingsNotice = null;
  render();
  try {
    await api.restartService();
    settingsSaved = false;
    settingsNotice = null;
  } catch (error) {
    // `restart_service` DOES reject, unlike the status commands — and it waits up to eight seconds
    // for the daemon to stop, so this is both a real failure path and a slow one. Its reason went
    // into `settingsError` before the review, which only the refusal dialog reads and only
    // `saveSettings` opens.
    settingsNotice = SETTINGS.restartFailed(String(error?.message ?? error));
  }
  settingsRestarting = false;
  clearTimeout(pollTimer);
  poll();
}

/** True when the bar on screen already draws what the current state says. */
function settingsBarUnchanged(node) {
  return node.dataset.shape === settingsBarShape(settingsProps());
}

// Ctrl S. The shell owns the key and the screen owns what it means, so the event is how they meet.
document.addEventListener("shell:save", () => {
  // NOT BEHIND A DIALOG. `activeRoute()` collapses a dialog back to the route underneath, so
  // without this a Ctrl+S while `8a Save refused` is up would re-run the save behind the modal —
  // and a retry that SUCCEEDED would leave "Nothing was saved" on screen over a config that was.
  if (activeRoute() === "settings" && !dialogOverlay) saveSettings();
});

// ---- onboarding (S7) ----
//
// The takeover holds the two steps; the three dialogs are driven from `onboardingStage` rather than
// through `openOverlay`, because none of them is opened by the user and none may be closed by Esc.
//
// The flow ENDS at `Start the first sync`: starting the daemon makes it reachable, which releases
// the latch by design (`nextOnboardingLatch`), so the merge and the consent float over the main
// screen. That is the answer to a takeover that cannot survive its own success. DEVIATIONS §79.

/** The proposals step 1 offers. `setup.sh`'s own two examples, and both are editable here. */
const PROPOSED_LOCAL = "~/ProtonDrive";
const PROPOSED_REMOTE = "/Drive/RemoteFolder";

let onboardingStep = "folders";
let onboardingRoots = null; // { local, remote } — proposals until step 1 writes them
let onboardingSeq = 0; // the rehearsal token, the same shape as the plan screen's
let onboardingAnswered = null;
let onboardingWaiting = null;
let onboardingDryRun = null;
let onboardingError = null;
let onboardingCheckedAt = null;
let onboardingStage = null; // null | "firstSync" | "consent"
let onboardingMergeSeq = null; // the daemon's pass counter when the merge started
let onboardingMergeSeen = false; // has the daemon answered at all since the merge started?
let onboardingMergeWaits = 0; // polls with no answer before it ever answered
let onboardingFailure = null; // the merge's reason for failing, until the flow or the daemon moves
let onboardingPauseTries = 0; // how many times the consent's pause has been asked for
let onboardingAgreed = false;
let onboardingStarting = false;
let onboardingFreeSpace = null;
let onboardingFreeSpaceAsked = null; // the local root the answer is about
let cliPresence = null; // `check_cli`'s reply, or null before it has answered
let cliAsked = false;
let cliChecking = false; // a check in flight — holds the dialog up across `Check again`

/** Files still to move — the activity's own counters, falling back to the watch queue. */
function remainingOf(activity, reply) {
  if (activity?.action_total != null) {
    return Math.max(0, activity.action_total - (activity.action_index ?? 0));
  }
  return reply?.pending_changes ?? null;
}

/** Which step is showing. A fixture names it; otherwise the flow's own state does. */
function onboardingStepNow() {
  const named = activeFixture()?.ui?.step;
  if (named === "review" || named === "folders") return named;
  return onboardingStep;
}

/** Which dialog the flow has open, if any. A fixture may name one directly. */
function onboardingDialog() {
  const named = activeFixture()?.ui?.dialog;
  if (named) return named;
  // NOT `cliChecking ||`: the first check is in flight on every first run, and holding the dialog up
  // for it flashes "the command line tool isn't installed" before anything has been checked. A
  // RE-check keeps the dialog because `cliPresence` still holds the answer it is re-asking.
  if (cliPresence?.installed === false) return "cliMissing";
  return onboardingStage;
}

/**
 * The CLI check: a silent precondition that only surfaces when it fails.
 *
 * Asked once per app run. A rejected call leaves `cliPresence` null — "we could not ask" is not
 * "it is missing", and putting a blocking dialog in front of someone on the strength of a failed
 * round trip would be the same false alarm as rendering an unknown count as zero.
 */
async function ensureCliCheck(again = false) {
  if (cliChecking || (cliAsked && !again)) return;
  cliAsked = true;
  cliChecking = true;
  try {
    // Assigned only on success: `Check again` on a machine where the round trip itself fails must
    // leave the dialog saying what it said, not flip to the tarball branch as though detection had
    // come back with nothing.
    cliPresence = await api.checkCli();
  } catch (error) {
    console.error("check_cli failed:", error);
  }
  cliChecking = false;
  render();
}

/**
 * C4, for the download side of step 2. Keyed on the local root rather than asked once: `Back`, a
 * different folder and `See what will happen` again is a question about a different disk.
 */
async function ensureFreeSpace(root) {
  if (onboardingFreeSpaceAsked === root) return;
  onboardingFreeSpaceAsked = root;
  try {
    // `null` rather than the root itself: the folder may not exist yet, and `free_space` walks up to
    // the nearest existing ancestor of the CONFIGURED root, which step 1 has just written.
    onboardingFreeSpace = await api.freeSpace(null);
  } catch (error) {
    console.error("free_space failed:", error);
  }
  render();
}

/** The rehearsal behind step 2 — one child at a time, the same token discipline as `ensurePlan`. */
async function ensureOnboardingPlan() {
  if (onboardingWaiting !== null || onboardingAnswered === onboardingSeq) return;
  const seq = onboardingSeq;
  onboardingWaiting = seq;
  let payload = null;
  let error = null;
  try {
    payload = await api.runDryRun();
  } catch (e) {
    error = String(e);
  }
  onboardingWaiting = null;
  if (seq !== onboardingSeq) {
    render();
    return;
  }
  onboardingAnswered = seq;
  if (payload?.report) {
    onboardingDryRun = payload;
    onboardingError = null;
    onboardingCheckedAt = Math.floor(Date.now() / 1000);
  } else {
    onboardingDryRun = null;
    onboardingError = error ?? "the rehearsal returned nothing";
    onboardingCheckedAt = null;
  }
  render();
}

/**
 * The pair as it stands. A FUNCTION, not a value captured per render, because step 1's field and its
 * `Choose…` button are built once and live across polls: a handler holding the roots from the render
 * that built it puts the other side back to its proposal on the next keystroke.
 */
function onboardingRootsNow() {
  if (onboardingRoots) return onboardingRoots;
  // A CONFIGURED PAIR BEATS THE PROPOSAL. The latch enters on `firstRun` as well as on a fresh
  // machine — a reachable daemon that has never synced — and that one HAS a config. Proposing
  // `~/ProtonDrive` over it and writing that back on `See what will happen` would repoint someone's
  // daemon at a folder they never chose.
  const live = store.select.response()?.config ?? null;
  return {
    local: live?.local_root ?? configInfo?.local_root ?? PROPOSED_LOCAL,
    remote: live?.remote_root ?? configInfo?.remote_root ?? PROPOSED_REMOTE,
  };
}

function onboardingProps() {
  const roots = onboardingRootsNow();
  const step = onboardingStepNow();
  if (step === "review") {
    ensureOnboardingPlan();
    ensureFreeSpace(roots.local);
  }
  return {
    step,
    local: roots.local,
    remote: roots.remote,
    dryRun: onboardingAnswered === onboardingSeq ? onboardingDryRun : null,
    error: onboardingAnswered === onboardingSeq ? onboardingError : null,
    checking: onboardingAnswered !== onboardingSeq,
    checkedAt: onboardingCheckedAt,
    freeSpace: onboardingFreeSpace,
    handlers: {
      onRoot: (which, value) => {
        onboardingRoots = { ...onboardingRootsNow(), [which]: value };
        // Safe under the caret: step 1's body signature does not carry the roots, so nothing here
        // rebuilds the field. The footer does rebuild — which is the point, since emptying the path
        // must disarm `See what will happen` in the same keystroke rather than 2s later.
        render();
      },
      onChooseLocal: async () => {
        try {
          const picked = await api.chooseFolder(onboardingRootsNow().local);
          // A cancelled picker resolves with null. Keeping the proposal is the whole point.
          if (picked) {
            onboardingRoots = { ...onboardingRootsNow(), local: picked };
            render();
          }
        } catch (error) {
          console.error("choose_folder failed:", error);
        }
      },
      onNext: async () => {
        if (onboardingStarting) return;
        onboardingStarting = true;
        try {
          // The pair has to be ON DISK before the rehearsal: `run_dry_run` shells
          // `proton-syncd --dry-run`, which reads the config file and not this screen.
          const pair = onboardingRootsNow();
          await api.writeConfig({ local_root: pair.local, remote_root: pair.remote });
          await refreshConfig();
          onboardingStep = "review";
          onboardingSeq += 1;
        } catch (error) {
          // Surfaced through step 2's failed body rather than swallowed: a refused write is exactly
          // the case where the next screen would otherwise rehearse the OLD config.
          onboardingStep = "review";
          onboardingSeq += 1;
          onboardingAnswered = onboardingSeq;
          onboardingError = String(error?.message ?? error);
          onboardingDryRun = null;
        }
        onboardingStarting = false;
        render();
      },
      onBack: () => {
        onboardingStep = "folders";
        // The token moves so a return to step 2 rehearses again — the pair may have changed.
        onboardingSeq += 1;
        render();
      },
      onCheck: () => {
        onboardingSeq += 1;
        render();
      },
      onStart: async () => {
        if (onboardingStarting) return;
        onboardingStarting = true;
        onboardingStage = "firstSync";
        onboardingMergeSeq = store.select.response()?.reconcile_seq ?? null;
        onboardingMergeSeen = false;
        onboardingMergeWaits = 0;
        render();
        try {
          await api.startService();
        } catch (error) {
          // `start_service` REJECTS, unlike the status commands — no systemd unit and no
          // `proton-syncd` on PATH is the common first-run failure, and its reason is the only thing
          // that tells someone which. Back to step 2, with the daemon's own words.
          failOnboardingMerge(String(error?.message ?? error));
        }
        onboardingStarting = false;
        clearTimeout(pollTimer);
        poll();
      },
    },
  };
}

/** What each of the flow's three dialogs draws. */
function onboardingDialogContent(id) {
  if (id === "cliMissing") {
    return {
      head: false,
      label: ONBOARDING.cliMissingTitle,
      signature: JSON.stringify(cliPresence),
      children: renderCliMissing({
        cli: activeFixture()?.cli ?? cliPresence,
        handlers: { onCheckCli: () => ensureCliCheck(true) },
      }),
    };
  }
  if (id === "firstSync") {
    const reply = store.select.response();
    const activity = reply?.activity ?? null;
    // THE FIXTURE'S PLAN FIRST, the same fallback the other two branches take (`?? cliPresence`,
    // `?? onboardingAgreed`): the footer sentence comes from the step-2 rehearsal, which is module
    // state no `?frame=` can reach, so without this the one in-flight claim this flow makes about
    // someone's files is never compared against the frame that draws it.
    const summary = (activeFixture()?.dryRun ?? onboardingDryRun)?.report?.summary ?? null;
    return {
      head: false,
      label: ONBOARDING.progressTitle,
      // SHAPE ONLY — see `firstSyncShape`. The numbers move every poll and are patched in place.
      signature: firstSyncShape({ activity, summary }),
      children: renderFirstSync({
        // NOT `pending_changes`, which S1 already documents as the trap it is: it is the local
        // filesystem-watch queue, and a pass driven by Proton — which the first merge always is —
        // carries an EMPTY one while downloading, so the mark would read 0 for the whole merge.
        // `action_total - action_index` is the files still to move, which is what the frame draws.
        pending: remainingOf(activity, reply),
        activity,
        summary,
        handlers: {
          // PAUSING ENDS THE FLOW. A paused daemon completes no pass, so `mergeOutcomeOf` would
          // wait forever behind a dialog with no ✕ and no Esc. Handing off to the main screen —
          // which draws `Paused` and a `Resume` — is the same call routes.js makes for a state
          // onboarding cannot resolve. The consent is not obtained on this path; the daemon's own
          // delete guard is on by default, so every deletion still goes through the Deletions
          // screen. §79k.
          onPause: async () => {
            await command(api.pause);
            resetOnboardingFlow();
            render();
          },
        },
      }),
    };
  }
  if (id === "consent") {
    const summary = onboardingDryRun?.report?.summary ?? null;
    // The sidecars on disk first, the reviewed plan second: `2 files are waiting for you to pick a
    // version` is a claim about NOW, and `scan_conflicts` is the only thing that counts them. The
    // plan's own figure is the fallback for a scan that has not come back.
    const conflicts = store.select.unresolvedConflictCount() || (summary?.conflicts ?? 0);
    return {
      head: false,
      label: ONBOARDING.consentTitle,
      // `conflicts` in the signature, not just the plan's: the scan lands on the first poll, after
      // the dialog has mounted, and a signature that misses it leaves the sentence saying nothing
      // is waiting when two files are.
      signature: JSON.stringify([onboardingAgreed, conflicts]),
      children: renderConsent({
        agreed: activeFixture()?.ui?.agreed ?? onboardingAgreed,
        conflicts,
        handlers: {
          onAgree: (on) => {
            onboardingAgreed = on;
            render();
          },
          onStartSyncing: async () => {
            if (!onboardingAgreed) return;
            // `resume` RESOLVES with its error inside the payload rather than rejecting, so the
            // dialog closes on the round trip landing, not on the daemon being resumed. Deliberate,
            // and the same call S4's `Run this sync` makes: the main screen behind is where both
            // outcomes are legible (`Resume` on a paused daemon, `Try again now` on an unreachable
            // one), and holding someone inside a consent they have already given is worse.
            await command(api.resume);
            resetOnboardingFlow();
            render();
            // The dialogs are not opened through `openOverlay`, so there is no `dialogReturn` to
            // restore — focus would land on `<body>`. The main screen's own action is where someone
            // who has just agreed should be standing.
            focusAfterSwap(".main-actions .btn");
          },
        },
      }),
    };
  }
  return null;
}

/**
 * The flow is over. Everything it holds is per-run, and a later re-entry — a config wiped from under
 * a machine whose daemon is gone — must open at step 1 with no plan and an unticked box, not at
 * step 2 with yesterday's rehearsal already agreed to.
 */
function resetOnboardingFlow() {
  onboardingStage = null;
  onboardingStep = "folders";
  onboardingRoots = null;
  onboardingSeq += 1;
  onboardingAnswered = null;
  onboardingDryRun = null;
  onboardingError = null;
  onboardingCheckedAt = null;
  onboardingAgreed = false;
  onboardingFreeSpace = null;
  onboardingFreeSpaceAsked = null;
  onboardingFailure = null;
  onboardingPauseTries = 0;
  onboardingMergeSeq = null;
  onboardingMergeSeen = false;
  onboardingMergeWaits = 0;
}

/** Take the merge dialog down and put its reason on step 2, where `Back` and `Check again` are. */
function failOnboardingMerge(reason) {
  onboardingStage = null;
  onboardingStep = "review";
  onboardingAnswered = onboardingSeq;
  onboardingDryRun = null;
  onboardingError = reason;
  onboardingCheckedAt = null;
  // The latch cannot bring the takeover back on its own — the pair is written by now, which is
  // exactly the condition it declines to re-enter on — so the failure is latched here instead. It
  // only holds the takeover while the daemon is UNREACHABLE; a reachable daemon that failed a pass
  // is the main screen's business, which is routes.js's own rule about not trapping someone in a
  // wizard that cannot fix their problem.
  onboardingFailure = reason;
}

/**
 * Advance the flow when the merge finishes, and make `Syncing stays paused until you agree.` true.
 *
 * Nothing starts a daemon paused, so the claim is made true here rather than drawn as a claim about
 * a daemon that is still running: the pass the person approved completes, then the daemon is paused
 * and the consent dialog opens. Leaving without agreeing leaves it paused, which is what the
 * sentence says. §79.
 */
/** How many polls will re-ask for the pause before the flow stops hammering the socket. */
const PAUSE_ATTEMPTS = 5;

function advanceOnboardingStage() {
  if (activeFixture()) return;
  // `Syncing stays paused until you agree.` IS ENFORCED, NOT ASSERTED. `pause` resolves with its
  // error inside the payload rather than rejecting, so a request that never landed is invisible to
  // its caller — and the sentence beside the checkbox would be a claim about someone's files that
  // nothing had checked. The poll re-asks until the daemon says it is paused, then stops.
  if (onboardingStage === "consent") {
    const reply = store.select.response();
    if (!reply || reply.paused) {
      onboardingPauseTries = 0;
      return;
    }
    if (onboardingPauseTries < PAUSE_ATTEMPTS) {
      onboardingPauseTries += 1;
      command(api.pause);
    }
    return;
  }
  if (onboardingStage !== "firstSync") return;
  const reply = store.select.response();
  // A DAEMON THAT NEVER CAME UP. `start_service` resolving means the unit was asked to start, not
  // that it is running — so a dialog that only ever advances on a reply would sit over an
  // unreachable machine claiming a merge was under way. Bounded to a handful of polls, and only
  // before the first answer: a blip after the merge has begun is not a failure to start.
  if (!reply) {
    if (onboardingMergeSeen) return;
    if (++onboardingMergeWaits < 8) return;
    failOnboardingMerge(store.select.error() ?? "the daemon did not start");
    return;
  }
  onboardingMergeSeen = true;
  const outcome = mergeOutcomeOf(reply, onboardingMergeSeq);
  if (outcome === "waiting") return;
  if (outcome === "failed") {
    failOnboardingMerge(reply.last_error);
    return;
  }
  onboardingStage = "consent";
  onboardingPauseTries = 0;
}

// ---- data ----
async function refreshConfig() {
  try {
    configInfo = await api.readConfig();
    configError = null;
    // A missing config file reads back as an empty doc (not an error), so a successful read means we
    // now *know* whether a folder pair exists — the signal nextOnboardingLatch needs to distinguish a
    // fresh machine from a config file that simply hasn't been read yet.
    configLoaded = true;
  } catch (error) {
    // Recorded rather than swallowed. Nothing retries differently, but the screen that draws this
    // file has to be able to say it could not be read instead of describing one that is not there.
    configError = String(error?.message ?? error);
  }
  render();
}

// ---- notifications (S9, C6) ----

/**
 * The trigger state, across restarts.
 *
 * localStorage FOR THE SAME REASON THE THEME IS THERE: it is GUI-local, per-machine, and losing it
 * costs one repeated banner rather than anything about anyone's files. `notify_policy` is NOT here —
 * it is a setting a person chose, so it lives in a file they can read and edit (`gui.toml`).
 */
const NOTIFIER_KEY = "notifier";

function loadNotifierState() {
  try {
    const saved = JSON.parse(localStorage.getItem(NOTIFIER_KEY) ?? "null");
    // Shape-checked rather than trusted: a half-written or older value must not make `decide` throw
    // inside the status poll, which would take the whole window's refresh down with it.
    //
    // `typeof null === "object"` PASSES THIS, and that is safe rather than overlooked — reviewed and
    // reproduced, because it is the obvious place to assume otherwise. Object spread of `null` is a
    // no-op by specification, so `{ said: null }` resolves to `said: {}`, which is the empty state
    // this function would have returned anyway. It does not throw, and neither does `{ said: [] }`.
    if (saved && typeof saved === "object" && typeof saved.said === "object") {
      return { ...emptyState(), ...saved, said: { ...saved.said } };
    }
  } catch (_) {
    /* unreadable storage is an empty state, not a failure */
  }
  return emptyState();
}

let notifierState = loadNotifierState();
/** The saved policy, and the one staged on the Settings tab. */
let notifyPolicy = "only_when_needed";
let notifyPolicyEdit = null;
/**
 * Whether the policy has been read off disk yet.
 *
 * NOTHING INTERRUPTS BEFORE IT HAS. `refreshNotifyPolicy` is a command round trip and `poll()`
 * starts beside it, so the first evaluation could otherwise run against the DEFAULT — and someone
 * who chose `Never` would be interrupted exactly once per launch, by the one setting whose whole
 * purpose is that they are not.
 */
let notifyPolicyLoaded = false;

async function refreshNotifyPolicy() {
  try {
    notifyPolicy = await api.readNotifyPolicy();
    notifyPolicyLoaded = true;
  } catch (error) {
    // The default is what `gui_prefs::load_notify_policy` answers for every unreadable case anyway,
    // so a failed read changes nothing about what is shown — it is logged because a command that
    // cannot be reached is worth knowing about. The latch is still set: a command that cannot be
    // reached will not become reachable, and refusing to notify for ever is the wrong failure.
    console.error("read_notify_policy failed:", error);
    notifyPolicyLoaded = true;
  }
  render();
}

/**
 * Decide whether anything should interrupt, and say it.
 *
 * Called at the end of every poll, after the conflicts and the deletion queue are in the store, so
 * the four triggers see one consistent picture rather than two ticks of one.
 */
function evaluateNotifications() {
  const { event, state, resolved } = decide({
    state: notifierState,
    view: {
      response: store.select.response(),
      conflicts: store.select.conflicts(),
      daemonState: store.select.daemonState(),
    },
    // THE SAVED VALUE, never the staged one. The Settings footer promises "nothing is written until
    // you save", and this setting IS the written thing — a staged `Never` that silenced the deletion
    // banner before anyone pressed Save would be the one exception nobody was told about, in the
    // direction that costs files.
    policy: notifyPolicy,
    nowMs: Date.now(),
  });
  notifierState = state;
  try {
    localStorage.setItem(NOTIFIER_KEY, JSON.stringify(state));
  } catch (_) {
    /* a full or disabled storage costs a repeated banner, nothing more */
  }
  if (!event) {
    // The banner's subject is gone — the deletion was approved, the conflict resolved, the daemon
    // came back. A persistent banner (Plasma advertises `persistence`) would otherwise sit there
    // asking about something already decided, and its buttons would act on an empty queue.
    if (resolved) api.closeNotification().catch((error) => console.error("close_notification:", error));
    return;
  }
  api.sendNotification(payloadFor(bannerFor(event))).catch((error) => {
    // A desktop with no notification server, or one that refused. Not fatal and not retried: the
    // same event is still in the window, and a retry loop against a server that is not there would
    // be the noisiest possible way to be silent.
    console.error("send_notification failed:", error);
  });
}

/**
 * `Keep them` — the permanent deletions the banner named, and only those.
 *
 * NOT `keepAllDeletions`, which is the screen's `Keep both files` and sends the reserved `all`
 * selector. On the wire that deletes EVERY row from `delete_approvals`, so a mixed queue would have
 * this banner revoke a standing approval the user granted for a recoverable deletion it never
 * mentioned. Keeping is always safe, but doing more than the button says is not the same as safe.
 *
 * It carries S3's caveat unchanged (#224): `deny` revokes an approval and nothing on the wire
 * refuses a withheld deletion durably. What it does do is true — a withheld deletion is not applied,
 * so the files stay — and the row comes back next pass, on the screen, where the decision belongs.
 */
async function keepPermanentDeletions() {
  const items = visibleDeletions().filter((item) => severityOf(item.direction) === "permanent");
  if (!items.length) return;
  for (const item of items) {
    const key = itemKey(item);
    if (deletionBusy.has(key)) continue;
    deletionBusy.add(key);
    try {
      if (acknowledged(await api.deny(item.path))) deletionsDecided.set(key, item.fingerprint);
    } catch (error) {
      console.error("deny failed:", error);
    }
    deletionBusy.delete(key);
  }
  // ONE poll at the end, not one per item: a banner about a folder with a thousand withheld files
  // would otherwise ask the daemon a thousand times over.
  clearTimeout(pollTimer);
  poll();
}

/**
 * A banner's button. The ids are `SAFE_ACTIONS` — no destructive one exists to arrive here.
 *
 * `trayAction` FOR THE THREE THAT OPEN OR RETRY, because they are the tray's own rows doing the
 * tray's own job: `review`/`open` show the window and `retry` is `Try again now`, which is a sync.
 * One id space, one handler, as `tray_row` already documents.
 */
function onNotificationAction({ kind, action } = {}) {
  switch (action) {
    case "keep":
      keepPermanentDeletions();
      return;
    case "later":
      // Dismiss. The thing is still in the window, which is the whole design of this action.
      return;
    case "retry":
      api.trayAction("tryAgain").then((payload) => store.setStatus(payload));
      return;
    case "compare":
    case "review":
      api.trayAction("open").then((payload) => store.setStatus(payload));
      navigate(kind === "deletion" ? "deletions" : "conflicts");
      return;
    case "open":
      api.trayAction("open").then((payload) => store.setStatus(payload));
      return;
    default:
      console.warn(`notification-action: no handler for "${action}"`);
  }
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
  // LAST, and after the conflict scan above, so the four triggers see one consistent picture.
  //
  // TWO EXCLUSIONS, AND THE SECOND IS THE ONE THAT BITES. A frame preview never notifies: `?frame=`
  // is a fixture, and a design surface raising a real desktop banner would be the preview reaching
  // outside the window. And THE TRAY PANEL IS A SECOND WEBVIEW RUNNING THIS FILE
  // (`index.html?surface=tray`, `panel.rs`) — it calls `main()`, so it polls, and without this it
  // would evaluate the same triggers against its own copy of the state, race the main window on the
  // same localStorage key and send a second time. `replaces_id` would stop them stacking and
  // nothing would stop the banner re-popping every time the panel is opened.
  if (!activeFixture() && !isTraySurface() && notifyPolicyLoaded) evaluateNotifications();
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
  refreshNotifyPolicy();
  poll();
  window.addEventListener("focus", scheduleNextPoll);
  document.addEventListener("keydown", onKeydown);

  // Tray menu items ask the shell to navigate. Routed through the api facade — no direct
  // window.__TAURI__ here.
  // MAIN WINDOW ONLY. `app.emit` broadcasts to every webview, so the tray panel — which runs this
  // same file — would run the handler a second time for one click: two deny sweeps over the queue,
  // and a `navigate()` that moves the panel's own route to a screen it cannot draw.
  if (!isTraySurface()) api.onNotificationAction(onNotificationAction);

  api.onTrayNavigate((id) => {
    if (typeof id !== "string") return;
    // Nothing emits this today: S8's tray acts through `commands::tray_action` rather than asking
    // the shell to navigate, and the alias table that used to translate its one dead id went with
    // it. The listener stays because the event is a seam a later task may want, and an id with no
    // route now says so instead of being quietly rewritten into a different screen.
    if (ROUTES[id]) navigate(id);
    else console.warn(`tray-navigate: no route for "${id}"`);
  });
}

main();
