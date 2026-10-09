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
import { pairTable, withPair } from "./pairmap.js";
import {
  ROUTES,
  FOOTER_ORDER,
  isOverlay,
  isDialog,
  nextOnboardingLatch,
  releasesOnboarding,
  configHasPair,
  entersOnboardingTakeover,
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
import { selectorRows } from "./ui/selector.js";
import { dialog, dialogHead, focusTrap } from "./ui/dialog.js";
import { renderCompactPanel, trayMenu, TRAY_FOLDER_CAP } from "./ui/compact.js";
import { bannerFor, payloadFor, renderBanner } from "./ui/notification.js";
import { decide, emptyState, notifierViews, restoreState } from "./notifier.js";
import { runBannerAction } from "./banner-actions.js";
import { trayView, renderTrayPanel, updateTrayPanel } from "./screens/tray.js";
import { ACTIVITY, CHROME, FOLDERS, ONBOARDING, SETTINGS } from "./ui/copy.js";
import { clock, since } from "./ui/format.js";
import { renderMain, updateMain, unmountMain, clearsStartError } from "./screens/main.js";
import { renderConflicts, advanceAfter, skipTo } from "./screens/conflicts.js";
import {
  renderDeletions,
  updateDeletions,
  unmountDeletions,
  armedItem,
  itemKey,
  statusKey,
  BULK_KEY,
} from "./screens/deletions.js";
import {
  renderPlan,
  updatePlan,
  unmountPlan,
  renderPlanBar,
  updatePlanBar,
  footerKindOf,
  isGated,
} from "./screens/plan.js";
import {
  renderActivity,
  renderDetailsBody,
  renderNeverSyncedBody,
  renderFilePendingBody,
  footerVariantOf,
  neverSyncedSubject,
  cannotSyncFrom,
  normaliseQuery,
  passesSummaryOf,
  searchOutcome,
} from "./screens/activity.js";
import {
  renderSettings,
  renderSettingsBar,
  renderSaveRefused,
  settingsBarShape,
  configUpdate,
  formatSchedule,
  isDirty,
  removalCost,
  restartEndingOf,
  restartUnresolved,
  clearsRestartFailure,
  saveNoteFor,
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
import {
  BUSY_PHASES,
  WAIT_LIMIT,
  accountOf,
  addKeyOf,
  addRequestOf,
  addViewOf,
  blankAdd,
  blankRemove,
  listRows,
  remotePathForProbe,
  removalOf,
  replyAbout,
  settledLinesOf,
  waitStepOf,
  withRule,
} from "./folders.js";
import { addFolderShape, removeFolderShape, renderAddFolder, renderRemoveFolder } from "./screens/folders.js";
import { severityOfItem } from "./ui/rows.js";
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
/**
 * Is the folder selector's popover showing (#102 phase 5c-1). Dropped by a switch of folder
 * (`noticeSelection`): choosing a row closes it, and a selection that moved under an open popover —
 * the tray's `Review them`, the other webview — leaves it listing a choice that has been made.
 */
let pairMenuOpen = false;
/**
 * What the config file says, ONE `read_config` REPLY PER FOLDER PAIR (#102 phase 5b-1), keyed by the pair
 * the reply says it describes (`reply.pair`). Its per-pair values are that pair's table of the file; its
 * daemon-wide values are the top level, the same in every one. Keyed rather than single so a pair that
 * has been switched to is drawn from ITS settings and never from the previous pair's for a poll, and a
 * reply that lands after a switch is a fact about the pair it was asked for (`viewedConfig`).
 */
let configByPair = pairTable();
/**
 * The file's whole roster (`read_config.pairs`), the same in every reply. Kept apart from the replies
 * because the first-run check and the pair count ask "how many folders are there" and "is any of them
 * placed", which are not questions about whichever pair is on screen — and answering them from the
 * viewed pair's reply would let a momentary mismatch of names read as a machine with no folders.
 */
let configRoster = [];
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

/**
 * How many folder pairs the app knows of: the daemon's list or the config file's, whichever is
 * longer (see `render`). Never reads a state a screen could mistake for "the daemon is down".
 */
function pairCountNow() {
  return Math.max(store.select.pairs().length, configRoster.length);
}

/**
 * The config file as it describes the pair on screen: the last `read_config` reply FOR it, or `null`
 * before one has landed. This is the reader the screens use — the single config reply the app held when
 * there was one folder, and for one folder it is the same object.
 */
function viewedConfig() {
  return configByPair.get(store.select.pairName()) ?? null;
}

/**
 * The config as the first-run check reads it (`configHasPair`): the WHOLE roster, plus the viewed
 * pair's flat roots as the fallback that function documents. `null` before any read, like the single
 * reply it replaces.
 */
function firstRunConfig() {
  if (!configLoaded) return null;
  const viewed = viewedConfig();
  return {
    pairs: configRoster,
    local_root: viewed?.local_root ?? null,
    remote_root: viewed?.remote_root ?? null,
  };
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
  // A DIALOG WITH SOMETHING IN FLIGHT IS NOT LEFT BY GOING SOMEWHERE ELSE EITHER. Esc and the ✕ were the
  // only ways out `closeOverlay` guarded, and Ctrl+, Ctrl+F, a banner's `Review` and the tray's navigate
  // event all arrive here: the keypress is consumed, nothing moves, and the answer lands on the dialog
  // that asked. Before anything below, so not even `openerError` is touched. See `leaveDialog`.
  if (dialogVetoed(dialogOverlay)) return;
  // A REFUSED OPEN IS ABOUT THE BUTTON THAT WAS CLICKED, and nothing else. `openerError` is one
  // variable feeding five sites across two screens, so a failure left standing is drawn under a
  // different button on a screen the user has since walked to — as the reason THAT one did nothing.
  // §95a records the identical shape on `serviceStartError`, which outlived its own subject.
  openerError = null;
  if (isOverlay(id)) return openOverlay(id);
  // The lit door is a no-op, NOT a toggle back to main (2026-08-13; Home is its own door now). It
  // has to stay a no-op rather than a re-navigate: re-entering resets the screen, which would drop
  // a half-typed lookup or an in-flight rehearsal.
  //
  // Only when nothing is stacked over it. With an overlay open the same click is the way back down
  // to the screen underneath, so it must fall through and clear the layers.
  if (route === id && screenStack.length === 0 && !dialogOverlay) return;
  const was = route;
  route = id;
  // Leaving discards the plan; arriving is handled at the mount in `render()`, because a door is not
  // the only way in. Moving the token here also drops an in-flight rehearsal's reply where it lands,
  // rather than writing it into state the next visit throws away.
  //
  // `was !== id` guards both: the fall-through above re-enters the route you are already on to shut
  // an overlay, and that is not leaving it — resetting there would wipe the plan or the lookup you
  // came back to.
  if (was !== id) {
    if (was === "plan") resetPlanScreen();
    // Same rule for the activity screen: a tab, a half-typed path and a filesystem walk are all
    // per-visit, so leaving drops them rather than carrying them into a session that may be hours old.
    if (was === "activity") resetActivityScreen();
  }
  // A door leaves everything stacked over the old screen behind — both layers, not just the top
  // one. Cleared directly rather than by popping: the door itself keeps focus, so there is no
  // return target to honour.
  screenStack = [];
  leaveDialog();
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
  // Opening something over a dialog with something in flight is leaving it: a screen overlay drops the
  // dialog and a second dialog takes its place, and neither may happen to one that is mid-flight. The
  // caller's open is refused, not queued — see `leaveDialog` for what that costs the one async caller.
  if (dialogVetoed(dialogOverlay)) return;
  const back = { key: focusKeyOf(opener ?? document.activeElement), node: opener ?? document.activeElement };
  if (isDialog(id)) {
    // A dialog that is REPLACED lets go of what it held, exactly as one that is closed does. The same id is
    // not a replacement: the caller has just set the state this one is about to be drawn from.
    if (dialogOverlay !== id) releaseFolderState(dialogOverlay);
    dialogOverlay = id;
    dialogReturn = back;
  } else {
    // A dialog belonged to the screen it was opened over; moving to a different screen closes it
    // rather than leaving it floating above something it was never about.
    leaveDialog();
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

/**
 * THE ONE WAY A DIALOG IS LEFT (review of #450, F1). Every site that used to clear `dialogOverlay` by hand
 * — a door, a screen overlay opening over it, Esc and the ✕, the first-run takeover, a dialog that ended
 * itself — goes through here, because a dialog that goes without letting go of what it held leaves that
 * state running: an add whose dialog had been dropped went on to pop a merge dialog out of nowhere, and a
 * removal's answer (a set-aside still pending included) was written to a dialog nobody could see.
 *
 * TWO HALVES, AND THEY ARE NOT THE SAME QUESTION. Whether a dialog MAY be left is `dialogVetoed`, asked by
 * the callers that can be refused (`closeOverlay`, `navigate`, `openOverlay`, the folder openers): a dialog
 * with something in flight stays, the keypress is consumed, and nothing moves. What happens to it WHEN it
 * is left is this function and has no answer to give back: it drops the dialog, its return target and the
 * folder state it held. The callers that cannot be refused call it directly — the takeover (it covers
 * everything, and a dialog from before it was armed has nothing left to be about) and the dialogs that end
 * themselves (`filePending`, a finished merge, an orphan).
 *
 * WHO CAN REACH A VETOED DIALOG: Esc and the ✕ (`closeOverlay`); Ctrl+, Ctrl+F, a banner's `Review` and the
 * tray's navigate event (`navigate`, and `openOverlay` under it); `openAddFolder`/`openRemoveFolder` and
 * `saveSettings`' failure dialog (`openOverlay`). A mouse click on a door is stopped by the scrim.
 * `saveSettings` is the one that loses something by being refused: a save that failed while an add was in
 * flight draws no `Save refused` over it, and says nothing but its staged edits still being staged.
 */
function leaveDialog() {
  releaseFolderState(dialogOverlay);
  dialogOverlay = null;
  dialogReturn = null;
}

/** Close the topmost layer. The dialog is always above the screen stack, so it goes first. */
function closeOverlay() {
  // Same rule as `navigate`: leaving the surface the failure was about retires it. A dialog is left
  // through here and not through `navigate`, so the clear has to be in both.
  openerError = null;
  if (dialogOverlay) {
    // A dialog with something in flight cannot be left, and the keypress that asked is CONSUMED — the
    // same `true` a closed dialog answers, so Esc does not fall through to a screen behind it.
    if (dialogVetoed(dialogOverlay)) return true;
    const back = dialogReturn;
    leaveDialog();
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
    // shell's other Esc targets exist in that window. Blur hides it too where the panel is an
    // ordinary window (`lib.rs`), but a keyboard user who never leaves the panel would otherwise
    // have no way out of it without picking a row — and where the panel is a layer surface, that
    // blur never arrives: no focus event is delivered there, so the blur arm is dead in both
    // directions and this is the only way out from inside the panel that does not also fire a row.
    // Whether the keypress reaches it on that path is open — `lib.rs` has the measurement and what
    // would settle it.
    if (isTraySurface()) {
      api.hideTrayPanel();
      e.preventDefault();
      return;
    }
    // The folder selector's popover comes down first and takes its Esc with it: the keypress that
    // closes a popover must not also leave the screen under it. (`onSelectorKey` handles the key
    // when focus is inside the control; this is for the popover opened by pointer, with focus elsewhere.)
    if (pairMenuOpen) {
      closePairMenu({ refocus: true });
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
    // Consumed whole under a dialog that may not be left: refusing the navigation below is not enough on
    // the Activity screen itself, where the event would move focus into the lookup behind the scrim.
    if (dialogVetoed(dialogOverlay)) return;
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
    // The selector's rows are a list the arrows walk (`onSelectorKey`); they are not a step through
    // the conflicts behind it.
    if (t instanceof HTMLElement && t.closest(".pair-select")) return;
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

// No frame draws this menu open (DEVIATIONS.md §46), so its contents come from 02-shell.md's one
// sentence: the theme toggle moved here from the title bar.
function renderMenu() {
  if (!menuOpen) return null;
  const light = currentTheme() === "light";
  return el(
    "div",
    { class: "menu-popover", role: "menu" },
    // AT EVERY COUNT (decision D7): below two folders there is no list to put the button in, and the
    // menu is where a person with one folder finds the second. Nothing about it is drawn — this menu is
    // undrawn (DEVIATIONS §46) — so it costs no frame.
    el("button", { class: "menu-item", role: "menuitem", onClick: openAddFolder }, FOLDERS.addFolder),
    el(
      "button",
      { class: "menu-item", role: "menuitem", onClick: toggleTheme },
      light ? "Dark theme" : "Light theme",
    ),
  );
}

// ---- the folder selector (#102 phase 5c-1) ----
//
// Built by `ui/selector.js`; this is what it is wired to. The control is patched across polls and its
// handlers are CONSTANT (`SELECTOR_HANDLERS` reads module state when an event arrives), so nothing
// about the control has to be rebound when the folders or the selection change.

/** The selected folder's `PairSummary`, or null (a daemon that lists none, or nothing heard yet). */
function selectedSummary() {
  const name = store.select.pairName();
  return store.select.pairs().find((summary) => summary.name === name) ?? null;
}

/**
 * What is waiting on a person in `pair`: its conflicts plus the withheld deletions not yet answered.
 *
 * For the SELECTED folder that is the chip's own sum, from the same two readers, so the popover can
 * never disagree with the chip beside it. For another folder it is what the store holds for it: the
 * conflict scan the poll runs for it every 60 s, and its deletion queue, which is fetched only while
 * its summary says it has some (E6) — so a count of 0 in the summary is believed over a list that is
 * stale, and a list never fetched falls back to the summary's count rather than to 0 (unknown is never
 * zero).
 */
function waitingIn(pair) {
  if (pair === store.select.pairName()) {
    return store.select.unresolvedConflictCount() + visibleDeletions().length;
  }
  const summary = store.select.pairs().find((entry) => entry.name === pair);
  const queued = summary?.pending_deletions ?? 0;
  let deletions = 0;
  if (queued > 0) {
    deletions = store.select.deletionsFreshOf(pair) ? visibleDeletionsOf(pair).length : queued;
  }
  return store.select.conflictsOf(pair).length + deletions;
}

/**
 * The selector's props — or `null`, which is the whole of decision D2. It is built from the folders the
 * DAEMON lists and from nothing else: a folder only the settings file knows cannot be chosen (there
 * would be nothing to show for it), and a daemon that lists none or one gets the header it always had.
 */
function selectorProps() {
  const pairs = store.select.pairs();
  if (pairs.length < 2) {
    pairMenuOpen = false;
    return null;
  }
  const name = store.select.pairName();
  return {
    name,
    rows: selectorRows({
      pairs,
      pairStates: store.select.pairStates(),
      selected: name,
      waiting: waitingIn,
      // THE STORE KEEPS THE LAST ROSTER AND THE LAST STATES ACROSS A FAILED READ, so a stopped daemon would
      // leave every row saying `up to date` beside a chip that says `unreachable` (#246). The store knows
      // whether what it holds is still what the daemon said (`rosterLive`), and the tray panel asks the same.
      reachable: store.select.rosterLive(),
    }),
    // A FRAME NAMES ITS OWN `open`, as it names its own route and dialog: `2a Two folders open` is a
    // popover, and `pairMenuOpen` is module state no `?frame=` can reach.
    open: activeFixture()?.ui?.pairMenuOpen ?? pairMenuOpen,
    handlers: SELECTOR_HANDLERS,
  };
}

function openPairMenu() {
  pairMenuOpen = true;
  menuOpen = false;
  render();
  // Into the list, on the folder that is chosen: a keyboard user who opened it has somewhere to go,
  // and Enter on it is "keep this one". After the render, which is what builds the rows.
  focusAfterSwap(".pair-row.is-selected");
}

/** Close it, and put the keyboard back on the pill when it was standing in the popover. */
function closePairMenu({ refocus = false } = {}) {
  if (!pairMenuOpen) return;
  pairMenuOpen = false;
  render();
  if (refocus) document.querySelector(".pair-pill")?.focus();
}

/**
 * Choose a folder. The popover closes first so the window never shows a menu over a screen that is
 * about to change; the choice is Rust's (`select_pair` validates it, stores it and tells the other
 * webview), and the poll that follows is what moves the window — the store follows the reply.
 *
 * A choice of the folder already shown asks Rust for nothing. A refusal (the folder went away since the
 * list was drawn) leaves the window where it was: the next poll's roster has already lost it.
 */
async function pickPair(name) {
  const was = store.select.pairName();
  closePairMenu({ refocus: true });
  if (typeof name !== "string" || name === was) return;
  try {
    await api.selectPair(name);
  } catch (error) {
    console.error("select_pair failed:", error);
    return;
  }
  clearTimeout(pollTimer);
  poll();
}

/**
 * The keys of the control, wherever focus is in it. Arrows walk the rows (clamped, not wrapped — a
 * list this short has ends), Home and End jump, Esc closes and gives the keyboard back to the pill, and
 * Down on the pill opens it. Tab is not handled: rows other than the chosen one are out of the tab
 * order, and leaving the control closes it (`focusin` in `main`).
 */
function onSelectorKey(event) {
  const rows = [...document.querySelectorAll(".pair-row")];
  switch (event.key) {
    case "ArrowDown":
    case "ArrowUp": {
      event.preventDefault();
      if (!pairMenuOpen) {
        openPairMenu();
        return;
      }
      if (rows.length === 0) return;
      const at = rows.indexOf(event.target);
      const down = event.key === "ArrowDown";
      const next =
        at < 0 ? (down ? 0 : rows.length - 1) : Math.min(rows.length - 1, Math.max(0, at + (down ? 1 : -1)));
      rows[next].focus();
      return;
    }
    case "Home":
    case "End":
      if (!pairMenuOpen || rows.length === 0) return;
      event.preventDefault();
      rows[event.key === "Home" ? 0 : rows.length - 1].focus();
      return;
    case "Escape":
      if (!pairMenuOpen) return;
      event.preventDefault();
      event.stopPropagation();
      closePairMenu({ refocus: true });
      return;
    default:
  }
}

const SELECTOR_HANDLERS = {
  onToggle: () => (pairMenuOpen ? closePairMenu({ refocus: true }) : openPairMenu()),
  onPick: (name) => pickPair(name),
  onKey: (event) => onSelectorKey(event),
};

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
  // The doors when they are NOT the footer itself — i.e. under an action bar. Latched like the
  // footer so a poll patches them instead of destroying the door the user is standing on.
  footerDoors: null,
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
 *
 * `mountFrameBanner` is deliberately NOT a caller. A banner is drawn by the desktop's notification
 * server in the shipped app, so nothing measures it and nothing sizes a window to it — the trap
 * needs the feedback loop, not just the container. It would still be capped in a short viewport,
 * which is latent rather than harmless: the day anything sizes a window to a banner, this is the
 * comment that says where to look.
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
    //
    // `menuState` FIRST, and `state` only as the default, because the panel's form and its rows are
    // two different keys — `screens/tray.js`'s own split ("the panel takes the form and the menu
    // takes the cause"). `10a Offline` is the frame that needs it: it draws the struck panel, which
    // is the `unreachable` FORM, over `Try again now`, which is the `outage` row set. A fixture
    // naming neither would ask `trayMenu` for a set called after a form, and `trayMenu` throws.
    //
    // `pairs` is the folder list a frame at TWO FOLDERS OR MORE is drawn for (`10a Two folders`): the
    // same `{ name, paused, syncing, rank }` the payload carries, through the same `trayMenu` the live
    // panel uses, capped as the live panel is.
    menu:
      spec.menu === true
        ? trayMenu(spec.menuState ?? spec.state, null, spec.pairs ?? [], { cap: TRAY_FOLDER_CAP })
        : (spec.menu ?? null),
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
    // The folders, and the state Rust derived for each (two or more draw a worst-folder panel). THE LIVE
    // ROSTER: the store keeps the last one across a failed read, and a panel drawn from it said
    // `Up to date` with `Sync now` and a pause row per folder over a daemon that was not there (#246).
    // With none, this is the one-folder panel, which draws a stopped daemon as it always did.
    pairs: store.select.livePairs(),
    pairStates: store.select.livePairStates(),
  });
  if (dom.trayPanel && updateTrayPanel(dom.trayPanel, view)) {
    reportTrayHeight();
    return true;
  }
  dom.trayPanel = renderTrayPanel(view, (id) => trayActionStatus(id));
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
 * re-arms it. Where the panel is a layer surface the correction lands live, the same as everywhere
 * else — MEASURED (#385, refutation measured 2026-09-16, corrects this file's earlier claim that it
 * did not): the wire shows the new size reaching the surface within half a second of this call, and
 * again on every later state change, with `panel.rs` re-placing the panel for the height it just set
 * rather than a stale one (#401). A cap would still be wrong here: it would latch the first
 * measurement and then fight every one of those later live corrections, not merely survive a reopen
 * — so the rule stands unchanged on every path.
 */
function reportTrayHeight() {
  requestAnimationFrame(() => {
    const height = dom.trayPanel?.offsetHeight;
    if (height) api.resizeTrayPanel(height);
  });
}

function render() {
  noticeSelection();
  retireStaleNotices();
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
  //
  // At two folders or more the roots are the SELECTED folder's own `PairSummary` — named by its pair, so
  // there is no question which folder a root belongs to — and then the reply's `config`, which is the
  // same facts for a daemon that lists no summaries (#102 phase 5c-1).
  const shown = selectedSummary();
  const live = store.select.response()?.config ?? null;
  const localRoot = shown?.local_root ?? live?.local_root ?? viewedConfig()?.local_root ?? null;
  const remoteRoot = shown?.remote_root ?? live?.remote_root ?? viewedConfig()?.remote_root ?? null;
  // How many folder pairs there are, from the running daemon's list and from the config file's, taking
  // the larger: either one saying "two" is enough for the takeover to stay shut (E14), and a stopped
  // daemon has no list at all. Both are zero for a legacy daemon with no `[[pair]]` tables, so the
  // one-folder app reads zero or one here and nothing below moves.
  const pairCount = pairCountNow();

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
  // And the same rule for the start button's failure, which is a fact about a STOPPED SERVICE.
  // `clearsStartError` is its own predicate rather than `reachable`: that one is the onboarding
  // release set, and this string's subject is narrower — the socket answering at all, by any route.
  // A daemon started from the tray row, from Settings' restart or from a terminal retires this
  // message just as much as the button does, and none of those three passes through `startService`.
  // Without it the next outage would be diagnosed with a superseded reason. Found by review.
  if (serviceStartError && clearsStartError(st)) serviceStartError = null;
  // A pause the daemon could not save is a fact about its NEXT RESTART. A daemon that stopped answering
  // has, as far as this window can tell, been through one.
  if (st === "unreachable") unsavedPauses.clear();
  // AND THE SAME RULE FOR THE SAVE'S RESTART (#335), which is a fact about a DAEMON and so has to be
  // re-validated against one: systemd ships `Restart=on-failure`, so after a start that failed the
  // service can come up on the new settings by itself while the bar still offers to restart it.
  //
  // NOT AGAINST `st` ALONE. This render can be the one `saveSettings` fires the instant the outcome
  // was recorded, and `st` is then the last COMPLETED poll — which in the `not_started` case is
  // necessarily a reachable daemon, because the restart only stopped anything after the probe said
  // it was running. So the latch carries the request clock it was written at and only a strictly
  // newer answer may retire it; `clearsRestartFailure` holds the rest of the rule, including why
  // only `not_started` is retired at all. `clearsStartError` supplies "the socket answers", so
  // there is one definition of that and not two.
  if (
    settingsSaveOutcome &&
    clearsRestartFailure(settingsSaveOutcome, {
      socketAnswers: clearsStartError(st),
      statusIssue: store.select.statusIssue(),
    })
  ) {
    settingsSaveOutcome = null;
  }
  onboardingLatch =
    onboardingStage !== null
      ? false
      : onboardingFailure
        ? true
        : nextOnboardingLatch(
            onboardingLatch,
            st,
            // The roots the daemon reports or the file's top level, OR a pair the file declares in a
            // `[[pair]]` table — those roots are in no top-level key, and with the daemon stopped a
            // check made of the first two alone called that machine fresh (F-J).
            Boolean(localRoot && remoteRoot) || configHasPair(firstRunConfig()),
            configLoaded,
            statusPolled,
            pairCount,
          );
  // ENTERING the takeover discards three things left behind by whatever ran before it. Hiding them
  // is not enough: the latch releases when the daemon comes up, and anything still held would be
  // restored on the way out — so finishing a first-run setup would land you on the Conflicts screen
  // with a Details dialog over it, from before the daemon was wiped. Reproduced, not theorised;
  // DEVIATIONS §57c. `onboardingDetour` joined the other two for the same reason, the other
  // direction (#337): it is the flow's OWN leftover, not the main app's, and `resetOnboardingFlow`
  // only clears it when the flow ends by completing — a session that instead ends by the latch
  // releasing out from under an open detour (the daemon syncs before `Start the first sync` is ever
  // clicked) leaves it set, and a later re-entry must not open back inside it.
  //
  // Edge-triggered on purpose (`entersOnboardingTakeover`, not "while latched"). Clearing on every
  // render while the latch is true would quietly forbid onboarding from ever opening a layer of its
  // own — a detour is opened from INSIDE an already-armed takeover, so that would null it the
  // render right after `onDetour` sets it.
  if (entersOnboardingTakeover(wasOnboarding, onboardingLatch)) {
    screenStack = [];
    // Not refusable: the takeover covers everything. Through `leaveDialog` all the same, so a folder
    // dialog from before it was armed takes its state with it instead of surfacing under the takeover.
    leaveDialog();
    // resetOnboardingFlow(), NOT an enumerated subset — #360. Its own comment says a (re-)armed
    // takeover opens at step 1 with no plan and an unticked box, and `onboardingStep`/`onboardingDryRun`/
    // `onboardingAgreed`/`onboardingSkipRules`/`onboardingRoots` were left to carry whatever a PRIOR
    // session (before the latch released) had reached — a stale rehearsal, agreed to, on step 2.
    //
    // EXCEPT when `onboardingFailure` is why this is an arm at all. `failOnboardingMerge` writes
    // `onboardingStep = "review"` and the error text ONE RENDER before this fires (it forces the
    // latch back to true via the ternary above, false→true, which IS this edge) — that write is for
    // THIS re-entry to show, not leftover from an earlier one. Resetting here would erase the
    // failure before it is ever painted, which is the same trap #246 was filed for: a fresh cause
    // becomes indistinguishable from an old one because the same wipe runs over both.
    //
    // A genuine re-arm never carries a STALE `onboardingFailure` — NOT because reaching `firstRun`
    // "requires passing through reachable first" (it does not: a poll samples every ~2s, and
    // unreachable→firstRun needs no intervening reachable tick at all). The ternary above pins the
    // latch to `true` only on a render where `onboardingFailure` is set AND `onboardingStage` is
    // null — the STAGE arm outranks the failure arm, so this is NOT a standing pin: clicking
    // `Start the first sync` again after a failure sets `onboardingStage = "firstSync"` without
    // touching `onboardingFailure`, and THAT render computes the latch false with a failure still
    // standing. Pinning alone proves nothing about staleness; the argument is about what CANNOT have
    // been true the render before.
    //
    // If THIS render's latch is true via the failure arm, `onboardingStage` is null this render (the
    // stage arm would have forced `false` otherwise). Had `onboardingStage` ALSO been null with
    // `onboardingFailure` ALSO truthy the render BEFORE, that render's latch would have been true by
    // the same arm, making `wasOnboarding` true and this render a HELD session, not an arm. So an
    // arm edge with `onboardingFailure` truthy forces `onboardingStage` to have just TRANSITIONED to
    // null. `onboardingStage = null` has two assigning writers, plus its `let` declaration:
    // `failOnboardingMerge` (sets a fresh reason in the same call) and `resetOnboardingFlow` (clears
    // it in the same call) — only the first leaves `onboardingFailure` truthy. So the failure this
    // edge carries was fresh the instant `onboardingStage` dropped to `null`, never a leftover from
    // an earlier session. A THIRD assigning writer of `onboardingStage = null` that does not keep
    // that same pairing would break this argument — grep for `onboardingStage = null` before adding
    // one.
    //
    // Only reachable from the two UNREACHABLE-daemon failure paths — `onStart`'s catch (no systemd
    // unit, no `proton-syncd` on PATH) and the 8-poll no-answer timeout in `advanceOnboardingStage`.
    // A failure against a daemon that answered derives to the `failed` state instead, which IS in
    // `releasesOnboarding`'s set, so the daemon-reachable clear ABOVE (line ~760) runs before the
    // ternary and the arm never fires with a failure attached at all.
    if (onboardingFailure) {
      onboardingDetour = null;
    } else {
      resetOnboardingFlow();
    }
  }
  // The CLI check runs before the flow has a config, so it is asked as soon as the takeover opens
  // (and once per app run). The merge's own progress is what advances the flow past it.
  if (onboardingLatch || activeRoute() === "onboarding") ensureCliCheck();
  advanceOnboardingStage();
  // The folders flows (#102 phase 5c-2): a dialog whose state is gone goes with it, the add dialog's wait
  // for the restarted daemon moves on, and a just-added folder's merge ends with its first pass.
  dropOrphanedFolderDialog();
  advanceAddFolder();
  advanceFolderMerge();

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
    leaveDialog();
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
  // Onboarding drops the ⋯ button, not just the chip — both 9a frames have four header slots.
  // Keyed off the ROUTE and not the latch alone: `9a Review` carries a written folder pair, which
  // is exactly the state the latch does not re-enter on, so a fixture selecting step 2 renders
  // with the latch open and would grow a ⋯ the frame does not draw.
  const hasMenu = !onboardingLatch && active !== "onboarding";
  const headerOpts = {
    chip: chip.variant,
    chipText: chip.text,
    hasMenu,
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
    // THE FOLDER SELECTOR, at two folders or more and on every screen that has a menu: the takeover
    // has neither, and at two folders it never arms anyway (E14). `null` below two — the header is then
    // the one the 22 spacer-pinning frames were measured against.
    pairs: hasMenu ? selectorProps() : null,
  };
  const navOpts = {
    // `active`, NOT the module `route`. S5 is the first screen whose frames draw a LIT door — all
    // three activity windows paint `Activity` at --text and the other three at --text-4 — and under
    // `?frame=` the module `route` is still "main", so the gate would have compared four unlit
    // doors against three lit frames. `activeRoute()` collapses a dialog back to its underlying
    // route and yields the overlay id for a screen overlay, which is why the kind test stays:
    // `conflicts` and `deletions` are overlays, and their frames draw no lit door.
    //
    // `root` joins `door` since Home became one (2026-08-13). The main-screen frames draw every door
    // unlit, so this lights one they do not — DEVIATIONS §94.
    active: ["door", "root"].includes(ROUTES[active]?.kind) ? active : null,
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
    //
    // THE CHOOSER MADE THAT INCOMPLETE. A search that matches several files draws up to 50 rows in
    // a scroller, and a rebuild put a user who had scrolled to row 40 back at row 1 every two
    // seconds — and dropped the keyboard off whichever row they had tabbed to. So the field's caret
    // is no longer the only thing carried across: the list's scroll offset and the focused row's
    // path come too. Keyed by PATH rather than by index, because a landing reply can reorder rows.
    const focused = document.activeElement === activityInputRef.node;
    const caret = focused ? caretOffset() : null;
    const matchList = document.querySelector(".activity-matches-list");
    const scrolled = matchList?.scrollTop ?? 0;
    const rowPath =
      document.activeElement instanceof HTMLElement
        ? (document.activeElement.closest(".activity-match")?.dataset.matchPath ?? null)
        : null;
    if (dom.bodyRoute !== active) unmountScreens();
    setBody(renderActivity(activityProps()));
    dom.bodyRoute = active;
    if (focused && activityInputRef.node) {
      activityInputRef.node.focus();
      putCaret(activityInputRef.node, caret);
    }
    const nextList = document.querySelector(".activity-matches-list");
    if (nextList && scrolled) nextList.scrollTop = scrolled;
    if (rowPath) {
      const row = [...document.querySelectorAll(".activity-match")].find(
        (node) => node.dataset.matchPath === rowPath,
      );
      row?.focus();
    }
  } else if (active === "settings") {
    // REBUILT EVERY PASS, with the focused field's caret put back — the same trade the activity
    // screen makes and for the same reason, except that this screen has five text fields rather
    // than one. Everything they hold lives in `settingsByPair`, so a rebuild loses nothing except
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

  // --- footer: the screen's action bar when it has one, and the doors beneath it. The frames drew
  // the two as alternatives (13 to 6); the 2026-08-13 decision keeps navigation on every screen but
  // the onboarding takeover. See routes.js and DEVIATIONS §94.
  //
  // The plan screen answers for itself, and is the only route that does: `5a Plan` and `5a Plan
  // safe` draw an action bar, `5a Checking` draws no bar at all. routes.js records only the route's
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
            // `settingsByPair`, and every one of them changes on the same keystroke. `dataset.shape`
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
  // WHICH DOOR THE KEYBOARD IS STANDING ON, before either node below can be replaced.
  //
  // The doors are drawn on every screen now, so they outlive the screen under them — but the NODE
  // does not: the nav is `dom.footer` on a doors screen and `dom.footerDoors` under an action bar,
  // and crossing between the two (or changing the nav's padding variant) rebuilds it. Focus then
  // lands on `<body>` with the doors still drawn in the same place, which is worse than the old
  // behaviour where they visibly went away. Two of this file's own comments assert the property —
  // `navigate` clears the overlay stack because "the door itself keeps focus", and `focusKeyOf`
  // calls a door button an opener that survives. Measured: standing on a door while the plan
  // rehearsal lands drops the keyboard with no user action at all.
  const standingOn =
    document.activeElement instanceof HTMLElement
      ? (document.activeElement.closest(".door")?.dataset.route ?? null)
      : null;

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
        : buildFooterNav(navOpts);
    if (dom.footer) dom.footer.replaceWith(built);
    else root.append(built);
    dom.footer = built;
    dom.footerKind = kind;
    dom.footerOwner = owner;
  }

  // The doors under an action bar. A second node rather than one composite footer, so every patch
  // path above keeps working and the bar's own position among the root's children — which is what
  // the fidelity mapping is keyed on — does not move.
  //
  // Not on the takeover: nothing it could navigate to works until the flow finishes.
  if (kind === "actionBar" && !spec.takeover) {
    // `standard`, and never the mono line: the line is the main screen's folder pair, and this nav
    // sits under a bar that has already said what the screen is about.
    const belowOpts = { ...navOpts, variant: "standard", line: null };
    if (!dom.footerDoors || !updateFooterNav(dom.footerDoors, belowOpts)) {
      const built = buildFooterNav(belowOpts);
      if (dom.footerDoors) dom.footerDoors.replaceWith(built);
      else dom.footer.after(built);
      dom.footerDoors = built;
    }
  } else if (dom.footerDoors) {
    dom.footerDoors.remove();
    dom.footerDoors = null;
  }

  // Put the keyboard back on the same door, in whichever node now holds it. Only when it fell off:
  // a patched nav keeps its own button, and re-focusing one that never lost focus would fight a
  // caret somewhere else on a later render.
  if (
    standingOn &&
    !(document.activeElement instanceof HTMLElement && document.activeElement.closest(".door"))
  ) {
    document.querySelector(`.footer-nav [data-route="${standingOn}"]`)?.focus();
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
      // A dialog may name the control the keyboard starts in (`content.focus`, a selector): the add
      // dialog opens in its name field and not on the ✕ the title row would give it.
      // Not under `?frame=`: a field that takes focus on mount draws the focus ring in every render of
      // the frame, and the fidelity gate renders cold (see `focusAfterSwap`).
      if (content?.focus && !activeFixture()) built.querySelector(content.focus)?.focus();
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
      // A FIELD IS FOUND AGAIN BY NAME AND KEEPS ITS CARET. Position is the wrong key for the folder
      // dialogs: a button comes or goes with the phase (`Cancel` while busy, `Restart it now`), so the
      // Nth control is a different one afterwards; and a field rebuilt under a keystroke that moved the
      // dialog's shape (the engine's answer arriving) must not put the caret back at the start.
      const typing = document.activeElement instanceof HTMLInputElement ? document.activeElement : null;
      const typingField = typing && surface.contains(typing) ? (typing.dataset.field ?? null) : null;
      const typingRange = typing
        ? [typing.selectionStart, typing.selectionEnd, typing.selectionDirection]
        : null;
      // THE HEAD IS REBUILT TOO, and leaving it out was a bug rather than an economy: `7a Never
      // synced`'s title COUNTS the rules (`4 files are never synced`), so a head that survives the
      // update keeps whatever number was known when the dialog opened — which, on the render that
      // mounts it, is none. The surface itself stays, so the appear animation does not restart.
      surface.replaceChildren(...dialogChildren(dspec, content, title, content.head === false, w));
      const refocus = typingField
        ? [...surface.querySelectorAll("input")].find((input) => input.dataset.field === typingField)
        : null;
      if (refocus) {
        refocus.focus();
        if (typingRange?.[0] != null) {
          refocus.setSelectionRange(
            typingRange[0],
            typingRange[1] ?? typingRange[0],
            typingRange[2] ?? "none",
          );
        }
      } else if (at >= 0) {
        const next = [...surface.querySelectorAll("button, input, [tabindex]")];
        (next[at] ?? surface).focus();
      }
      dom.dialogSignature = content.signature;
    }
  }

  // The merge dialog's two moving numbers, patched rather than rebuilt: its mark is the syncing
  // hexagon, and a rebuild restarts both travelling segments from 0% twice a second.
  if ((dialogRoute === "firstSync" || dialogRoute === "folderMerge") && dom.dialog) {
    const merging = mergeReplyOf(dialogRoute);
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

/** The doors. One builder for both mounts — the footer itself, and the nav under an action bar. */
function buildFooterNav(navOpts) {
  return renderFooterNav({
    ...navOpts,
    order: FOOTER_ORDER,
    labels: Object.fromEntries(FOOTER_ORDER.map((id) => [id, ROUTES[id].label])),
    onNavigate: navigate,
  });
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
 * What names a conflict: the folder it was found in AND its path. A path alone is a slot two folders
 * can both fill — `note.txt` in conflict in `docs` and in `photos` — and a read issued for one is
 * indistinguishable by that key from a read issued for the other, so the older folder's reply, landing
 * after the selection moved, was accepted as the newer folder's and its file content drawn on the card
 * for a different file. `pair` is the tag `store` puts on every list it hands out (`DEFAULT_PAIR` for a
 * daemon that lists none), so every conflict this is asked of carries one. JSON, not a joined string:
 * no choice of separator can make two different `(pair, path)`s collide.
 */
const conflictKeyOf = (conflict) => JSON.stringify([conflict.pair ?? null, conflict.original]);
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
  const requested = conflictKeyOf(conflict);
  if (conflictPairKey === requested || conflictPairInFlight === requested) return;
  conflictPairInFlight = requested;
  let pair = null;
  try {
    // The pair the conflict was FOUND in, which travels on the conflict (`store` tags every list it
    // hands out): its paths are relative to that pair's root, and the selection can have moved on.
    pair = await api.readConflictPair(conflict, { pair: conflict.pair });
  } catch (error) {
    // Not fatal and not a placeholder: the cards fall back to the metadata row alone, which is what
    // `04-conflicts.md` asks for when the content cannot be read. A pair invented here would be a
    // diff of files nobody has seen.
    console.error("read_conflict_pair failed:", error);
  }
  if (conflictPairInFlight !== requested) return;
  conflictPairInFlight = null;
  // A reply for a folder that is no longer the one shown. `noticeSelection` has already dropped the
  // read it issued, so this is unreachable while that holds — and it is the check that keeps the
  // cards honest if it ever stops: the folder is part of the key above, and also of this.
  if (conflict.pair !== undefined && conflict.pair !== store.select.pairName()) return;
  conflictPair = pair;
  conflictPairKey = requested;
  render();
}

async function chooseConflict(conflict, choice) {
  // THE FOLDER ON SCREEN WHEN THE BUTTON WAS PRESSED. The decision itself goes to the folder the card
  // was drawn for (`conflict.pair`), whatever happens next. What follows it is a different question: it
  // moves the position, the tally and the cached bytes of the screen that is shown, and if the
  // selection has moved on while the call was out, that screen belongs to another folder, whose own
  // card would be pulled back to a position it never held and whose tally would count a choice it did
  // not make. `noticeSelection` has already reset those for the new folder.
  const shown = store.select.pairName();
  const stillShown = () => store.select.pairName() === shown;
  try {
    await api.resolveConflict(conflict, choice, { pair: conflict.pair });
  } catch (error) {
    console.error("resolve_conflict failed:", error);
    return;
  }
  if (stillShown()) {
    conflictsSettled = {
      total: conflictsSettled.total + 1,
      keptBoth: conflictsSettled.keptBoth + (choice === "keep_both" ? 1 : 0),
      tookProton: conflictsSettled.tookProton + (choice === "use_proton" ? 1 : 0),
    };
  }
  // Re-scan before moving: settling one removes it, and `advanceAfter` needs the list it is
  // adjusting against. Keeping the old length here is what makes the last conflict dead-end.
  // Filed under the folder it was found in, so it is a true fact about that folder either way.
  let next = store.select.conflicts();
  try {
    next = await api.scanConflicts({ pair: conflict.pair });
    store.setConflicts(next, conflict.pair);
  } catch (error) {
    console.error("scan_conflicts failed:", error);
  }
  if (!stillShown()) return;
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
    pair: conflict && conflictPairKey === conflictKeyOf(conflict) ? conflictPair : null,
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
    // `Open both in an editor` (#220, §74) — live since the openers landed as a C-item.
    //
    // BOTH RELATIVE PATHS COME OFF THE `Conflict`, and neither is rebuilt here. The sidecar name has
    // two forms (`{stem}.proton-cloud.{ext}` and the extensionless `{name}.proton-cloud`) and
    // `gui_core::conflicts` is the one place that knows which this is; deriving it in JS would be a
    // second copy of that rule, which is this project's most-recorded bug shape.
    onOpenBoth: conflict
      ? () => runOpener(() => api.openPaths([conflict.original, conflict.sidecar], { pair: conflict.pair }))
      : null,
    openError: openerError,
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
/** `path_sync_status` replies, by `statusKey` (pair and path) — the size and mtime a file card draws. */
const deletionStatuses = new Map();
const deletionStatusInFlight = new Set();
/** Item keys with a control command in flight, plus `"all"` for the bulk one. */
const deletionBusy = new Set();
/**
 * WHAT YOU HAVE ALREADY DECIDED, while this app has been open.
 *
 * The daemon keeps a withheld deletion in `pending_deletions` until a pass consumes it, so the ~2s
 * poll would put a decided row straight back on screen for as long as that pass takes. It is a
 * window now rather than a session-long fiction: both answers are recorded by the daemon (`approve`
 * a standing approval, `keep` a purge), so the row leaves the queue by itself on the next pass.
 * Keyed by `(path, direction)` and PINNED TO THE FINGERPRINT,
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
  // Pruned for THIS pair only: the keys carry their pair, and what the selected queue no longer holds
  // says nothing about another pair's — a decision recorded there is still waiting for its pass.
  const mine = `${store.select.pairName()}\u0000`;
  for (const key of deletionsDecided.keys()) {
    if (key.startsWith(mine) && !queued.has(key)) deletionsDecided.delete(key);
  }
  // The size-and-mtime cache is pruned on the same signal, and for a sharper reason: it is keyed by
  // PATH, so a `notes.txt` that is deleted, settled, replaced and deleted again would otherwise draw
  // the first file's `4 KB` and `last edited Jan 2026` on a card about the second. Dropping the
  // entry when the deletion leaves the queue makes the next one ask again — which also gives a read
  // that failed on a busy index a second chance instead of remembering the failure for the session.
  const statuses = new Set(live.map(statusKey));
  for (const key of deletionStatuses.keys()) {
    if (key.startsWith(mine) && !statuses.has(key)) deletionStatuses.delete(key);
  }
  return live.filter((item) => deletionsDecided.get(itemKey(item)) !== item.fingerprint);
}

/**
 * The withheld deletions of `pair`, less what has been answered. `null`, or the folder on screen, is
 * `visibleDeletions()` itself — the chip, the band and the Deletions screen read that one. Any other
 * folder is what the poll fetched for it, and only while its summary says it has some: a list fetched
 * earlier and not since is not evidence about a queue the summary now counts as empty. Empty for a
 * folder whose queue has not been fetched yet — a caller that needs "unknown" asks `deletionsFreshOf`.
 *
 * `deletionsFreshOf` AND NOT "WAS EVER FETCHED": a list fetched before the count last changed is the old
 * queue's, and `Keep them` on a banner built from it keeps files the person was never shown.
 */
function visibleDeletionsOf(pair) {
  if (pair == null || pair === store.select.pairName()) return visibleDeletions();
  const summary = store.select.pairs().find((entry) => entry.name === pair);
  if (!(summary?.pending_deletions > 0) || !store.select.deletionsFreshOf(pair)) return [];
  return store.select
    .pendingDeletionsOf(pair)
    .filter((item) => deletionsDecided.get(itemKey(item)) !== item.fingerprint);
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
  // Keyed by pair AND path: two folders can each be about to lose a `notes.txt`, and the size and
  // mtime of the one are not the other's. The lookup is the pair the ROW belongs to.
  const key = statusKey(item);
  if (deletionStatuses.has(key) || deletionStatusInFlight.has(key)) return;
  deletionStatusInFlight.add(key);
  let status = null;
  try {
    status = await api.pathSyncStatus(item.path, { pair: item.pair });
  } catch (error) {
    console.error("path_sync_status failed:", error);
  }
  deletionStatusInFlight.delete(key);
  deletionStatuses.set(key, status);
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
 * KEEPING ASKS FOR NOTHING EXTRA. `keep` (#224) purges the deletion's baseline record, which is what
 * makes the refusal durable — the planner stops deriving the action instead of the screen
 * remembering an answer — and the daemon schedules its own pass to put the other side back, so
 * there is no `syncNow` nudge on this branch. `deny` is still the right command for *revoking an
 * approval*, and nothing on this screen does that: every button here is about the deletion, not
 * about a permission.
 */
async function decideDeletion(item, approve) {
  const key = itemKey(item);
  if (deletionBusy.has(key)) return;
  deletionBusy.add(key);
  render();

  let settled = false;
  try {
    // THE ROW'S PAIR, not the selected one: the path is relative to that pair's root, and the
    // selection can move while the round trip is in flight (two webviews, a banner action).
    const reply = await (approve
      ? api.approve(item.path, true, null, { pair: item.pair })
      : api.keep(item.path, true, { pair: item.pair }));
    settled = acknowledged(reply);
  } catch (error) {
    console.error(approve ? "approve failed:" : "keep failed:", error);
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
      // And the nudge goes to the pair the approval was for, not to wherever the selection is now.
      await api.syncNow({ pair: item.pair });
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
 * session: approve one, then keep the rest, and `keep all` refuses the deletion you just approved —
 * which is the right reading of a button that says *keep both files*, but only if the screen
 * remembers it that way too. Marking the visible ones alone would leave the approved item recorded
 * as approved and hidden, while the daemon had just been told to keep it.
 */
async function keepAllDeletions() {
  const items = store.select.pendingDeletions();
  if (!visibleDeletions().length || deletionBusy.has(BULK_KEY)) return;
  // One queue, one pair: every item the screen holds was filed under the same one, and `all` means
  // all of THAT pair's. Captured before the round trip, which is when the selection can move.
  const pair = items[0].pair;
  deletionBusy.add(BULK_KEY);
  render();
  let settled = false;
  try {
    // `literalPath: false` with the explicit "all" selector. A file literally named `all` is a real
    // path and would otherwise be the only thing kept (#60); the flag is what keeps the reserved
    // word and a filename apart on this wire.
    const reply = await api.keep(BULK_KEY, false, { pair });
    settled = acknowledged(reply);
  } catch (error) {
    console.error("keep all failed:", error);
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
 * Did the daemon actually act on that approve/keep?
 *
 * NOT "did a reply come back". A dead socket resolves rather than rejects — the Tauri commands
 * return a payload either way — and, more subtly, the daemon answers `Ok` with `no pending deletion
 * matches '<path>'` when the selector is absent from the snapshot it is holding, which the GUI can
 * reach by acting on a queue that is up to two seconds stale. Treated as a decision, that hides a
 * row nothing was recorded for. `keep` adds two more of exactly that shape — an ambiguous selector
 * and a fingerprint that has moved both answer `nothing was kept …` — and the positive match
 * refuses all of them without knowing they exist.
 *
 * Read as a POSITIVE match on the acknowledgement rather than a blacklist of the `no …` replies, so
 * the failure direction is safe: if the daemon ever rewords them, the GUI stops recording decisions
 * and rows stay visible, instead of hiding deletions that never happened.
 */
function acknowledged(reply) {
  if (!reply?.response || reply.error) return false;
  return /^(approved|denied|kept) /.test(reply.response.message ?? "");
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
        if (severityOfItem(item) !== "permanent") return;
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
/**
 * THE PAIR THE PLAN IN HAND WAS MADE FOR — and therefore the pair `Run this sync` applies to. A token
 * is a plan's identity within ONE pair, and the selection can move after the rehearsal (two
 * webviews, a banner action): an apply that asked "which pair is selected?" when it was pressed
 * would run A's reviewed deletions against B, where the token is `stale` at best. The plan carries
 * its pair, set with the answer it describes and cleared with it.
 */
let planPair = null;

/** Entering or leaving the screen: no plan, no error, and a rehearsal on its way. */
function resetPlanScreen() {
  planDryRun = null;
  planError = null;
  planCheckedAt = null;
  planAnswered = null;
  planPair = null;
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
  // The pair this rehearsal is FOR, captured before it leaves.
  const pair = store.select.pairName();
  let payload = null;
  let error = null;
  try {
    payload = await api.runDryRun({ pair });
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
    planPair = pair;
    planError = null;
  } else {
    // A resolved reply that is not a report is not a plan: `run_dry_run` either returns a
    // `DryRunPayload` or fails, so this is the browser-preview mock answering `null` for a frame
    // that describes no rehearsal. Treating it as an empty plan would claim the next sync moves
    // nothing, over a screen that has been told nothing.
    planDryRun = null;
    planPair = null;
    planError = error ?? "the rehearsal returned no plan";
  }
  planCheckedAt = Math.floor(Date.now() / 1000);
  render();
}

/**
 * Authorise this plan's deletions before the pass that plans them (#227).
 *
 * THE DIRECTION COMES FROM THE ACTION, and it has to: nothing is pending yet, so the daemon cannot
 * look the deletion up to find out which of the two at that path is meant — and a path with no
 * direction authorises nothing, which is the same rule an ambiguous selector already answers to.
 * `remote_delete` removes the copy on Proton Drive, `local_delete` the one here; `isGated` is the
 * one place that pair is enumerated, so this cannot drift from what the typed word is guarding.
 *
 * The daemon pins each approval to the path's own baseline fingerprint, so an approval recorded
 * here authorises exactly the deletion the rehearsal described and nothing the plan changed into
 * afterwards. It refuses a path it has no record for rather than storing one that could never
 * match, which is why a failure here is worth nothing more than a log: the pass withholds that one
 * and it arrives on the Deletions screen.
 */
async function approvePlannedDeletions(plan, pair) {
  for (const row of plan) {
    if (!isGated(row.action)) continue;
    const direction = row.action === "remote_delete" ? "remote" : "local";
    try {
      await api.approve(String(row.path), true, direction, { pair });
    } catch (error) {
      console.error("approve failed:", error);
    }
  }
}

/**
 * What a press of `Run this sync` commits to, read once: the pair the plan was made for, the token
 * that names that plan, and its rows. A token is a plan's identity within ONE pair, so the two travel
 * together from the moment of the press — see `onRun`.
 */
function pressedPlan() {
  return {
    pair: planPair,
    token: planDryRun?.token ?? null,
    rows: planDryRun?.report?.plan ?? [],
  };
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
    progress: ui?.progress ?? planProgress(),
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
      // `Run this sync` authorises this plan's deletions and then asks for the pass. The typed word
      // means what the design says it means (#227): each gated row is approved BEFORE the sync is
      // scheduled, so the pass that plans the deletion also finds the approval for it and applies
      // it, instead of withholding it for the Deletions screen to ask a second time.
      //
      // Awaited in order and one at a time, because a `syncnow` that overtakes an approval is a
      // deletion deferred — which is the old behaviour, safe but not what was asked for. A refused
      // or failed approval is left where it falls: the pass then withholds that one, and the
      // Deletions screen has it. Nothing is deleted that was not agreed to either way.
      //
      // It leaves whether or not the command landed, which is deliberate: `sync_now` resolves
      // rather than rejects on a dead socket, so a failure here is silent, and the main screen is
      // where both outcomes are legible (syncing hero vs unreachable).
      onRun: async () => {
        // THE PRESS IS THE COMMIT POINT: the plan's pair AND its token are taken once, here. A
        // selection change while the approvals are being sent resets the plan screen (and with it
        // `planPair` and the plan in hand), and the apply that follows must still be the one that was
        // reviewed, for the pair it was reviewed for — the approvals have already gone there.
        const press = pressedPlan();
        await approvePlannedDeletions(press.rows, press.pair);
        await runReviewedPlan(false, press);
      },
      // `Run it without the deletion` (#192). No approvals: the deletions are what is being left
      // out, and approving them here would authorise on the Deletions screen exactly what this
      // button says it is not doing.
      onRunWithout: () => runReviewedPlan(true, pressedPlan()),
    },
  };
}

/**
 * The `5a Checking` progress line's two numbers (#209), or `null` while nothing can say them.
 *
 * Both come off the status poll the shell already runs. The `kind` test is what makes them THIS
 * rehearsal's: a plan pass publishes `PLAN_PASS_KIND` on its pass block, so a sync that happened to
 * start while the screen was open cannot lend its `files_scanned` to a rehearsal's line.
 */
function planProgress() {
  const reply = store.select.response();
  const activity = reply?.activity ?? null;
  if (activity?.pass?.kind !== "plan") return null;
  if (typeof activity.files_scanned !== "number") return null;
  return { scanned: activity.files_scanned, total: reply?.index_totals?.files ?? null };
}

/**
 * Apply the plan the user just reviewed, by its token (#100), and act on the typed answer.
 *
 * The token is what makes this different from the `syncnow` it replaces: the daemon re-plans and
 * runs the plan only if it is still the same plan. On a divergence nothing ran and the daemon holds
 * a NEW plan, so the screen stays put and re-checks — `06-plan.md`: "if the plan changes while the
 * gate is armed, clear the input", which a rebuild of the bar does by construction.
 *
 * Without a token (the `--dry-run` child path, i.e. onboarding before a daemon exists) there is
 * nothing holding the plan to apply by name, so this falls back to the pre-#100 `syncnow`.
 */
async function runReviewedPlan(skipDestructive, { pair, token }) {
  if (!token) {
    await command(() => api.syncNow({ pair }));
    navigate("main");
    return;
  }
  let outcome;
  try {
    // `pair` is the one the plan was made for — never "the selected pair", which is exactly the
    // wrong-target apply the token cannot catch: it names a plan within a pair, not a pair.
    outcome = await api.applyPlan(token, skipDestructive, { pair });
  } catch (error) {
    // A dead socket. Reported where every socket failure is legible.
    console.error("apply failed:", error);
    navigate("main");
    return;
  }
  // Typed, never matched as prose (#103). `diverged` and `stale` are the two that keep the user
  // here, because both mean "the plan you reviewed is not the plan any more".
  if (outcome?.state === "diverged" || outcome?.state === "stale") {
    planSeq += 1;
    render();
    return;
  }
  navigate("main");
}

/** A `start_service` asked for and not yet answered. Drives the main screen's busy button. */
let serviceStarting = false;
/** Why the last start attempt failed, quoted on the main screen. Cleared by the next attempt. */
let serviceStartError = null;

/**
 * Start the sync daemon from the window — the main screen's `Start the sync service`.
 *
 * NOT `command(api.startService)`. That helper swallows its rejection into a `console.error`, which
 * is right for the control-socket commands (they resolve with the failure inside the payload, so a
 * throw there is a bug rather than a diagnosis) and wrong for this one: `start_service` REJECTS,
 * and the message it rejects with is the only account of why. A machine with neither a systemd unit
 * nor a config file gets a sentence naming both; swallowed, that is a button that does nothing
 * forever with the reason in a console nobody has open.
 *
 * The latch is also the double-click guard. `systemctl --user start` blocks until the unit reports
 * started, so this promise is outstanding for seconds while the button sits there.
 */
async function startService() {
  if (serviceStarting) return;
  serviceStarting = true;
  serviceStartError = null;
  render();
  try {
    await api.startService();
  } catch (error) {
    serviceStartError = String(error?.message ?? error);
  }
  serviceStarting = false;
  // A RESOLVED PROMISE MEANS "ASKED", NOT "RUNNING" — the same caveat onboarding's merge wait was
  // built around. Nothing here decides the daemon came up: the poll asks the socket, and the hero
  // leaves `unreachable` when it answers. Re-polling now rather than waiting out the ~2s tick is
  // what makes the button feel like it worked, on `command`'s own reasoning.
  clearTimeout(pollTimer);
  poll();
}

// ---- the openers (#220/#231) ----

/**
 * Why the last open did not happen. One variable for all four buttons: only one can be clicked at a
 * time, and the sentence is only ever about the click that just failed.
 */
let openerError = null;

/**
 * Run one of the four openers and keep its refusal.
 *
 * NOT `command(...)`. That helper folds a rejection into `console.error`, which is right for the
 * control-socket commands — their failure travels inside a RESOLVED payload, so a throw there is a
 * bug and not a diagnosis — and wrong for these: `open_paths`/`open_folder`/`open_remote`/
 * `open_system_log` genuinely reject, and the message is the only account of why nothing opened.
 * Swallowing it is the silence these four buttons already had, moved one layer down.
 */
async function runOpener(call) {
  if (openerError !== null) {
    openerError = null;
    render();
  }
  try {
    await call();
  } catch (error) {
    openerError = String(error?.message ?? error);
    render();
  }
}

/** The three handlers every screen with an opener passes down, plus the reason the last one failed. */
function openerProps() {
  // The pair the screen is drawn for, taken as the props are built (see `mainProps`).
  const pair = store.select.pairName();
  return {
    openError: openerError,
    onOpenFolder: (path) => runOpener(() => api.openFolder(path, { pair })),
    onOpenRemote: () => runOpener(() => api.openRemote()),
    onOpenLog: () => runOpener(() => api.openSystemLog()),
  };
}

/** Everything the main screen reads, plus the actions it can take. */
function mainProps(localRoot, remoteRoot) {
  // THE PAIR THIS SCREEN IS DRAWN FOR, captured as the props are built — not read when a button is
  // pressed. The selection can move between the render and the click (two webviews, a banner action),
  // and a `Pause` that read it then would pause whichever folder was selected by the time it ran
  // rather than the one whose hero the person was looking at. The hero's buttons call through the
  // LATEST props (`main.js`), so this is always the pair on screen.
  const pair = store.select.pairName();
  const notice = mainNotice();
  return {
    pairCount: pairCountNow(),
    // The folder the hero is about, named on its pause button at two folders or more (decision D11).
    // The same count the folder selector is drawn at, so the pill and the button appear together.
    pair: store.select.pairs().length >= 2 ? pair : null,
    notice,
    daemonState: store.select.daemonState(),
    response: store.select.response(),
    conflicts: store.select.conflicts(),
    // The same filtered view the chip and the deletions screen read — see `chipFor`. The band says
    // `Two deletions are waiting on you`, and it must not say it about ones you have answered.
    deletions: visibleDeletions(),
    localRoot,
    remoteRoot,
    starting: serviceStarting,
    startError: serviceStartError,
    handlers: {
      onSyncNow: () => command(() => api.syncNow({ pair })),
      onPause: () => setPaused(pair, true),
      onResume: () => setPaused(pair, false),
      // The folder the notice that holds this button is about: where the outcome of the restart is filed.
      onRestartSyncing: () => restartSyncing(notice?.kind === "pairNotRunning" ? notice.name : null),
      onStartService: startService,
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

/**
 * A pause or resume the daemon applied and could not save (`ControlResponse.pause_unsaved`, decision
 * D12), BY FOLDER: `{ kind: "pauseUnsaved" | "resumeUnsaved", reason, issue }`.
 *
 * KEPT PER FOLDER, and spoken only while that folder is the one on screen — a notice about `photos` over a
 * hero about `docs` would be a sentence about the wrong folder. The tray's row can pause a folder that is
 * not the one this window shows; the event is filed under the folder it names and is there when that folder
 * is next looked at, which a single slot dropped (review of #447).
 *
 * IT ENDS WHEN IT STOPS BEING TRUE, and each of these is a rule of `retireStaleNotices`:
 *   · the hero's next press of Pause or Resume (it has its own answer — `setPaused`);
 *   · a status reply, issued after the notice, that shows the folder's pause state is no longer the one the
 *     notice describes — a pause saved, a resume done from the tray or `proton-sync resume`, none of which
 *     reach this window as an event;
 *   · the daemon not answering: "if syncing restarts first" is about a restart that has now happened.
 * `issue` is the status-request clock (`store.beginStatus`) at the moment it was noted: a reply that left
 * before that carries evidence from before the pause, and cannot end it.
 */
const unsavedPauses = pairTable();

/**
 * Note a pause or resume reply's `pause_unsaved`, from the hero's own press or from the tray's row
 * (which the window hears of as an event: the panel is dismissed by the time the reply arrives).
 */
function noteUnsavedPause(pair, paused, reason) {
  if (typeof reason !== "string" || typeof pair !== "string") return;
  unsavedPauses.set(pair, {
    kind: paused ? "pauseUnsaved" : "resumeUnsaved",
    reason,
    issue: store.select.statusesIssued(),
  });
  render();
}

/**
 * What the daemon last said about whether `pair` is paused, and which request said it — `{ paused, issue }`,
 * or null when it has said nothing about that folder. At two folders or more that is the folder's own
 * summary, whichever folder is on screen; a daemon that lists none has only the reply about the one folder.
 */
function pausedAccordingToDaemon(pair) {
  const summary = store.select.pairs().find((entry) => entry.name === pair);
  if (summary) return { paused: summary.paused === true, issue: store.select.pairsIssue() };
  const reply = store.select.response();
  if (reply && pair === store.select.pairName()) {
    return { paused: reply.paused === true, issue: store.select.statusIssue() };
  }
  return null;
}

/**
 * Drop what has stopped being true. Called at the top of every render, beside `noticeSelection`, so a
 * status reply that settles a question is the render that stops saying it.
 *
 *   · An unsaved pause or resume whose folder the daemon now reports in the OTHER pause state, on a reply
 *     issued after the notice (see `unsavedPauses`); and one for a folder the daemon does not list.
 *   · A failed restart (`restartOutcomes`) for a folder that is not the one the notice is about any more.
 */
function retireStaleNotices() {
  for (const [pair, note] of unsavedPauses) {
    const known =
      pair === store.select.pairName() || store.select.pairs().some((entry) => entry.name === pair);
    const said = pausedAccordingToDaemon(pair);
    const stillTrue =
      !said || said.issue <= note.issue || (note.kind === "pauseUnsaved" ? said.paused : !said.paused);
    if (!known || !stillTrue) unsavedPauses.delete(pair);
  }
  const unknown = store.select.pairUnknown();
  for (const name of restartOutcomes.keys()) if (name !== unknown) restartOutcomes.delete(name);
}

/**
 * The hero's Pause and Resume. Not `command(...)`, which discards the reply: this reads it, because the
 * reply is where the daemon says a pause did not reach the index. `pair` is the folder the hero was
 * drawn for, captured by the props (class W).
 */
async function setPaused(pair, paused) {
  unsavedPauses.delete(pair);
  let reply = null;
  try {
    reply = await (paused ? api.pause({ pair }) : api.resume({ pair }));
  } catch (error) {
    console.error("control command failed:", error);
  }
  noteUnsavedPause(pair, paused, reply?.response?.pause_unsaved);
  clearTimeout(pollTimer);
  poll();
}

/**
 * `Restart syncing` on the notice for a folder the daemon does not run. A restart is daemon-wide, so the
 * busy flag is one flag; why the last try did not work is a fact about THE FOLDER THE NOTICE WAS FOR, keyed
 * by it, and it goes when the notice is about another folder. As one global `{ failed, reason }` it opened
 * the next folder's notice already saying a restart had failed, in the last folder's words (review of #447).
 */
let restartBusy = false;
const restartOutcomes = pairTable();

async function restartSyncing(name) {
  if (restartBusy) return;
  restartBusy = true;
  restartOutcomes.delete(name);
  render();
  try {
    // Not `onlyIfRunning`: the daemon IS running, on the settings it started with, which is the thing
    // being fixed.
    const outcome = await api.restartService();
    if (restartEndingOf(outcome) !== "restarted") {
      restartOutcomes.set(name, { reason: String(outcome?.reason ?? outcome?.detail ?? "") || null });
    }
  } catch (error) {
    restartOutcomes.set(name, { reason: String(error?.message ?? error) });
  }
  restartBusy = false;
  clearTimeout(pollTimer);
  poll();
}

/**
 * What the notice block says right now (`screens/main.js` `noticeOf`), or null.
 *
 * A folder the app remembered and the daemon does not run is shown ONLY when the settings file does list
 * it: that is the case a restart fixes. A remembered folder that is in neither is stale preference, and
 * the window already shows the default folder, which is all there is to say.
 */
function mainNotice() {
  // A FRAME NAMES THE REPLY IT WAS DRAWN FOR, since no frame can press Resume (`ui.unsavedPause`).
  const named = activeFixture()?.ui?.unsavedPause;
  if (named) return { kind: named.kind, reason: named.reason };
  const unknown = store.select.pairUnknown();
  if (unknown && configRoster.some((entry) => entry.name === unknown)) {
    const failure = restartOutcomes.get(unknown);
    return {
      kind: "pairNotRunning",
      name: unknown,
      busy: restartBusy,
      failed: failure != null,
      reason: failure?.reason ?? null,
    };
  }
  const unsaved = unsavedPauses.get(store.select.pairName());
  if (unsaved) return { kind: unsaved.kind, reason: unsaved.reason };
  return null;
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
/**
 * Every file the search found for the resolved query, and how many there were before the cap.
 * `{ query, matches: [{ path, status }], total }`, or null when nothing has been asked yet.
 */
let activityMatches = null;
/**
 * Which match the user picked out of a list of several. Cleared on every new reply, so a chosen
 * file never survives the query that found it.
 */
let activityChosen = null;
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
  activityMatches = null;
  activityChosen = null;
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
  // `configLoaded`, NOT the reply. The rules come from the config file, and `read_config` is a
  // round trip — so the first render of this screen has no config at all, sees an empty `exclude`,
  // and would latch `skipRuleAsked` on the strength of not having asked yet. The band would then
  // never appear until you left the screen and came back. Latching only once the config is
  // genuinely known is what makes "no rules" mean no rules.
  if (skipRuleAsked || !configLoaded) return;
  skipRuleAsked = true;
  // The pair this walk is for. A switch resets the screen (and `skipRuleAsked` with it), and a walk
  // that was already running must not land on the pair that replaced it.
  const pair = store.select.pairName();
  const exclude = configByPair.get(pair)?.exclude ?? [];
  // Nothing excluded is not a reason to walk the tree: the band counts files a RULE hides, and with
  // no rules the answer is known without asking.
  if (exclude.length === 0) return;
  try {
    const report = await api.skipRuleUsage(exclude, configByPair.get(pair)?.include ?? [], { pair });
    if (pair === store.select.pairName()) skipRuleReport = report;
  } catch (error) {
    console.error("skip_rule_usage failed:", error);
  }
  render();
}

/**
 * Search the index for what was typed.
 *
 * A NAME, A TRAILING PATH OR A FRAGMENT — `search_files` matches all three, so `spec.md` resolves to
 * `docs/spec.md` the way `7a File lookup` draws it. This used to be `path_sync_status`, which opens
 * the index AT the string it is given: a bare name that was not at the root missed, and the screen
 * said the file did not exist. That was G21, and closing it is what makes the plural arm of
 * `ACTIVITY.matches` reachable.
 *
 * One match resolves straight to the verdict. Several are listed for the user to pick from — the
 * screen cannot choose for them, and answering for the first would be a verdict about a file they
 * did not ask about.
 */
async function lookupPath(query) {
  const typed = normaliseQuery(query);
  if (!typed) {
    activityLookup = null;
    activityMatches = null;
    activityChosen = null;
    activityLookupInFlight = null;
    render();
    return;
  }
  // Latest-wins. Typing outruns the round trip, and an early reply landing after a later one would
  // put the verdict for `doc` under the word `docs/spec.md`.
  activityLookupInFlight = typed;
  // The pair the lookup is ABOUT, captured with it. A switch resets the screen and clears the
  // in-flight marker, which is what drops a reply that lands afterwards.
  const lookupFor = store.select.pairName();
  let reply = null;
  let failure = null;
  try {
    // THE RAW STRING, not `normaliseQuery`'s. That one strips the leading `/` so the field's own
    // count can be keyed on what is in it — and the backend's root-stripping is a PATH prefix
    // match, which an absolute path with its slash removed can never satisfy. Sent normalised, a
    // pasted `/home/me/ProtonDrive/docs/spec.md` reached the index as the literal
    // `home/me/ProtonDrive/docs/spec.md` and matched nothing, which is exactly the input
    // `relative_query` exists to serve.
    reply = await api.searchFiles(String(query ?? "").trim(), undefined, { pair: lookupFor });
  } catch (error) {
    console.error("search_files failed:", error);
    // KEPT, not swallowed. A caught error and a name nothing matches both leave the screen with no
    // file to describe, and it must not tell someone their file is missing when the search is what
    // failed. The daemon's own words go through untouched, to be quoted in mono.
    failure = String(error?.message ?? error);
  }
  if (activityLookupInFlight !== typed) return;
  activityLookupInFlight = null;
  // One match resolves, none is a miss carrying the failure, several are a list. Pure and pinned in
  // `activity.test.js` — see `searchOutcome`.
  const outcome = searchOutcome(reply, typed, failure);
  activityMatches = outcome.matches;
  activityLookup = outcome.lookup;
  activityChosen = null;
  // THE PENDING DIALOG'S TRIGGER, and it is the only one the data supports. `7a File lookup` and
  // `7a File pending` are the same lookup in two states — a file that is settled, and a file that
  // is moving right now — so looking up the file the daemon is currently transferring is what
  // tells the two apart. Nothing else could: `SyncActivity` carries exactly ONE in-flight transfer
  // (#211), so a lookup for any other moving file cannot reach this state at all.
  //
  // Against the RESOLVED path, not the query: `spec.md` and `docs/spec.md` are the same file, and
  // the transfer names it the way the index does. A list of several has no one file to describe, so
  // it opens nothing until one is chosen — see `onChooseMatch`.
  //
  // Latched, so dismissing it sticks. The condition stays true for as long as the transfer runs.
  if (activityLookup && offerPendingDialog(activityLookup.path)) return;
  render();
}

/**
 * Open `7a File pending` for a path that is moving right now, once per path.
 *
 * Returns true when it took over, so the caller stops rather than rendering the screen underneath.
 */
function offerPendingDialog(path) {
  // NEVER OVER SOMETHING THE USER OPENED. A reply lands ~180ms plus a round trip after the last
  // keystroke, and nothing on the way out of this screen cancels it — so a search started before
  // `Show them`, `Details` or a notification's `Review` was clicked would put this dialog over a
  // surface it is not about, taking the one the user asked for down with it (`openOverlay` nulls
  // `dialogOverlay` for a screen overlay and overwrites `dialogReturn` for a dialog). Silently
  // dropped rather than queued: the offer is about a transfer that is happening NOW.
  if (dialogOverlay || screenStack.length) return false;
  const moving = store.select.response()?.activity?.transfer ?? null;
  if (moving?.path !== path || activityPendingShown === path) return false;
  activityPendingShown = path;
  navigate("filePending");
  return true;
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

  // BOTH HALVES, one subject. The rule-matched files come from the disk walk; the ones nothing
  // can sync come from the daemon's standing list (#232), which is on every status reply.
  const never = neverSyncedSubject(skipRuleReport, response?.unsyncable);
  return {
    tab,
    query: ui?.query ?? activityQuery,
    lookup: ui?.lookup ?? activityLookup,
    // `ui.lookup` IS a fixture's whole answer — one file, resolved — so a frame that pins one draws
    // the verdict and never the chooser. Only a live search fills this.
    matches: ui?.lookup ? null : (ui?.matches ?? activityMatches),
    editedAt: ui?.clock?.edited ?? null,
    // The Proton card's twin. Read only when `last_transfer` is actually there — see
    // `receivedAtFrom`, which takes this as the pinned rendering of a time it has already sourced.
    receivedAt: ui?.clock?.received ?? null,
    never,
    history,
    localRoot: response?.config?.local_root ?? viewedConfig()?.local_root ?? null,
    remoteRoot: response?.config?.remote_root ?? viewedConfig()?.remote_root ?? null,
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
      activityMatches = null;
      activityChosen = null;
      activityLookupInFlight = null;
      activityPendingShown = null;
      clearTimeout(activityLookupTimer);
      render();
    },
    // One file out of several. The row already carries the status the search read, so choosing is
    // not a second round trip — and the pending dialog is offered here for the same reason it is
    // offered on a single match: this is the moment the screen knows which file is meant.
    //
    // A CLICK OUTRANKS A SEARCH THE USER HAS ALREADY MOVED PAST. The chooser goes on drawing the
    // last answer while a newer query is debounced, so the row can be clicked with a reply still
    // coming — and that reply would replace the file just chosen with the one nobody picked.
    // Cancelling both the timer and the in-flight token drops it: `lookupPath` bails on a token
    // that has moved.
    onChooseMatch: (match) => {
      clearTimeout(activityLookupTimer);
      activityLookupTimer = null;
      activityLookupInFlight = null;
      activityChosen = match.path;
      activityLookup = { path: match.path, status: match.status, error: null };
      if (offerPendingDialog(match.path)) return;
      render();
    },
    // Back to the list from a file that was chosen out of one. Not a re-search: the answers are
    // already held, and asking again would put a fresh reply under a query nobody retyped.
    onBackToMatches: () => {
      activityChosen = null;
      activityLookup = null;
      render();
    },
    chosen: activityChosen,
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
    ...openerProps(),
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
  if (id === "addFolder") return addFolderDialog();
  if (id === "removeFolder") return removeFolderDialog();
  if (id === "folderMerge") return folderMergeDialog();
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
      // The FIXTURE's config first. The viewed pair's reply is filled by `refreshConfig`, which is a
      // round trip — so under `?frame=` the first render has none, and two of these eight rows would
      // draw a dash where the frame draws a value.
      config: activeFixture()?.config ?? viewedConfig(),
      socketOk: Boolean(store.select.response()) && !store.select.error(),
      historyCount: props.history.length,
      // The dialog's own `Open the system log` (#231) — the same handler the passes tab's copy of
      // that button uses, so the two cannot drift apart.
      onOpenLog: props.onOpenLog,
      openError: props.openError,
    };
    return {
      signature:
        JSON.stringify(body.counters) +
        JSON.stringify(body.config) +
        body.socketOk +
        body.historyCount +
        String(body.openError),
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
      signature: JSON.stringify(transfer) + String(props.openError),
      children: renderFilePendingBody({
        transfer,
        onOpenFolder: props.onOpenFolder,
        openError: props.openError,
      }),
    };
  }
  return null;
}

// ---- the settings screen (S6) ----
//
// THE STAGED EDIT LIVES HERE, NOT IN THE DOM, and it has to: the body is rebuilt or patched on
// every 2s poll, so a form that kept its half-typed folder path in an `<input>`'s value would lose
// it twice a second. `settingsByPair` is the only record of what has been changed and not saved —
// `Discard changes` empties the viewed pair's slot and nothing else.

let settingsTab = "folders";
/**
 * WHAT IS STAGED ON THIS SCREEN, ONE SLOT PER FOLDER PAIR (#102 phase 5b-1, brief 3.4): `pair name →
 * { edits, drafts, scheduleMonthly, notice }`. Settings edits the pair on screen, so what a person has
 * typed belongs to that pair — and a screen that held ONE object would apply a half-edited skip list
 * for `docs` to `photos` the moment the selection moved. Kept per pair, nothing is lost by switching
 * (no modal, no discard) and nothing is ever written to a pair it was not typed for: a save reads the
 * slot of the pair it was started for (`saveSettings`) and sends that pair's name with the write.
 *
 * The slot's fields:
 *
 * - `edits`: staged fields, keyed exactly as `ConfigPayload` names them. Empty means nothing to save.
 *   Every staging path assigns a FRESH object, so identity is an exact "nothing was staged since" test.
 * - `drafts`: the two add fields, ONE PER LIST. Not config fields: a draft is staged only once `Add`
 *   is pressed. Two, not one, and the reason is that the lists mean opposite things. A pattern typed
 *   into the skip tab HIDES what it matches; the same pattern in Advanced's include list makes it the
 *   only thing that syncs. One shared buffer would carry a half-typed `*.psd` across a tab switch and
 *   hand it to whichever `Add` was pressed next — inverting what the person meant, on the two
 *   settings that decide what is backed up at all.
 * - `scheduleMonthly`: which full-sweep editor the schedule panel is showing (#193). `null` until
 *   someone picks a segment, and back to `null` whenever a schedule is set or cleared — at which point
 *   the schedule itself says which editor to show. Frontend state, deliberately: a mode is not a
 *   schedule, so switching it stages nothing and leaves the screen clean. It has to OUTRANK the
 *   configured schedule while it is set, or the Weekly/Monthly control is inert on every daemon that
 *   has one — see `schedulePanel`.
 * - `notice`: what the bar says about the last thing asked for in this pair — in flight, or failed.
 *   Every one of these was silence before the S6 review: `resync` RESOLVES with a socket error folded
 *   into its payload rather than rejecting, so a `Sweep now` against a dead daemon did nothing at all
 *   and said nothing at all; a failed `restart_service` wrote its reason into a variable only the
 *   refusal dialog reads, and nothing opens that dialog from there. Per pair because a sweep is.
 */
let settingsByPair = pairTable();
const BLANK_STAGING = Object.freeze({
  edits: {},
  drafts: { exclude: "", include: "" },
  scheduleMonthly: null,
  notice: null,
});
/** What is staged for `pair` — the blank slot for one nothing was typed for. Read-only. */
const stagedFor = (pair) => settingsByPair.get(pair) ?? BLANK_STAGING;
/** Replace some of `pair`'s staged state. Never touches another pair's. */
function patchStaging(pair, patch) {
  settingsByPair = withPair(settingsByPair, pair, { ...stagedFor(pair), ...patch });
}
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
 * What the restart the last save asked for did — `{ ending, reason }`, or null for no save.
 *
 * ONE VARIABLE, BECAUSE IT IS ONE FACT (#335). It was two — a note and a failure reason — and they
 * were set, cleared and re-validated in different places, which is how a `Restart it now` outlived
 * the state it was for and how navigating away lost one. The ending is `RestartOutcome`'s own tag
 * (`restarted` · `not_running` · `not_started` · `never_stopped` · `undetermined`, or `unknown` for
 * a backend this build cannot name); the sentence comes from `saveNoteFor` and the retry slot from
 * `restartUnresolved`, so neither can describe a different ending than the other.
 *
 * SET FOR A CONFIG WRITE ONLY. A policy-only save writes `gui.toml`, which the daemon never reads,
 * so a sentence about the sync service would be about a file nothing is waiting on — and the
 * restart itself is skipped for the same reason, rather than bouncing the daemon for a setting it
 * has never heard of. What a policy-only save leaves behind is what any form leaves behind: a
 * `Save` that has gone quiet because there is nothing left to save. It leaves an EARLIER save's
 * unresolved ending exactly where it was, which is the state that save did nothing about.
 */
let settingsSaveOutcome = null;
/** A restart asked for and not yet answered. `restart_service` can take ten seconds. */
let settingsRestarting = false;

/** Entering or leaving: nothing staged, no draft, no refusal, and the walk to be asked for again. */
function resetSettingsScreen() {
  settingsTab = "folders";
  // EVERY PAIR'S: walking away from the screen discards what was staged on it, for all of them. It is a
  // switch of pair that keeps them (see `settingsByPair`), not a switch of screen.
  settingsByPair = pairTable();
  // WITH THE REST OF THE STAGED STATE. It is one of the two things a person can stage on this
  // screen and it lives outside the pair slots (it is not a daemon-config key), so leaving it out
  // here made it the one edit that survived walking away: the card stayed chosen and the screen
  // stayed dirty about a value nothing had written, for the life of the window.
  notifyPolicyEdit = null;
  settingsSaving = false;
  settingsSweeping = false;
  settingsRestarting = false;
  settingsError = null;
  clearSaveOutcome();
  skipRuleReport = null;
  skipRuleAsked = false;
}

/**
 * Forget what the last save left behind — **the settled endings only** (#335).
 *
 * A SETTLED ENDING IS TRANSIENT AND AN UNRESOLVED ONE IS A STATE. `Saved. The sync service
 * restarted` is an acknowledgement of something finished, and it stops being interesting the moment
 * anything else happens. An unresolved ending is not: the file on disk is running ahead of the
 * service, that is still true after walking to Activity and back, and in the two endings where the
 * daemon is UP on the old settings this screen's bar is the only restart control left in the app —
 * so forgetting it here left no way out from inside the app at all.
 *
 * That is why an EDIT calls this too rather than clearing outright. The old rule ("an edit
 * invalidates the pair, because the next `Save` restarts again") was true of the note and false of
 * the failure: staging a change and discarding it lost the retry for good, and a staged
 * notification policy saves without restarting anything. What keeps a retry from being offered
 * beside a config write that is about to happen anyway is `barActionOf`, which yields the slot
 * while a daemon-config change is staged and takes it back when there is none.
 */
function clearSaveOutcome() {
  if (!restartUnresolved(settingsSaveOutcome?.ending)) settingsSaveOutcome = null;
}

/**
 * Stage one field, for `pair`. An edit forgets a SETTLED save's sentence — it is no longer describing
 * what is on disk — and keeps an unresolved restart, which still is. See `clearSaveOutcome`.
 *
 * `pair` is the one the control was drawn for, taken when the screen's props were built, so a click on
 * a screen that is a poll stale stages into the pair it showed and not the one selected since.
 */
function stageSetting(key, value, pair = store.select.pairName()) {
  patchStaging(pair, { edits: { ...stagedFor(pair).edits, [key]: value }, notice: null });
  clearSaveOutcome();
  render();
}

/**
 * The Folders tab's list (#102 phase 5c-2): one row per folder in the settings file's order, with the word
 * for each one's state, and the three things a row can do. Choosing a folder is `pickPair`, the same call
 * the header's popover makes, so the two ways to choose one are one way.
 */
function foldersProps() {
  return {
    rows: listRows({
      roster: folderRoster(),
      pairs: store.select.pairs(),
      pairStates: store.select.pairStates(),
      selected: store.select.pairName(),
      reachable: store.select.rosterLive(),
    }),
    handlers: { onPick: pickPair, onRemove: openRemoveFolder, onAdd: openAddFolder },
  };
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
  // THE PAIR THIS SCREEN IS DRAWN FOR, taken as the props are built (see `mainProps`) and carried by
  // every handler below: what is staged, saved, swept or chosen is that pair's, whichever is selected
  // by the time the handler runs.
  const pair = store.select.pairName();
  const slot = stagedFor(pair);
  const ui = activeFixture()?.ui ?? null;
  const saved = activeFixture()?.config ?? configByPair.get(pair) ?? {};
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
    : slot.edits;
  const config = { ...saved, ...edits };
  const skip = activeFixture()?.skipRules ?? skipRuleReport;
  // THE SAME FILTER S5's DIALOG USES, not a second reading of the list. This tab's panel counts and
  // names the group that dialog enumerates, and `See them` opens it — three surfaces that must
  // agree by construction rather than by two functions staying in step (#232).
  const cannot = cannotSyncFrom(store.select.response()?.unsyncable);
  // The staged policy counts as dirty like any other control, even though it is not part of the
  // config `write_config` sends — the footer promises "nothing is written until you save", and a
  // control that saved itself on click would be the one exception nobody was told about.
  const policyStaged = notifyPolicyEdit != null && notifyPolicyEdit !== notifyPolicy;
  // THE ONE PREDICATE FOR "a change the daemon reads is staged" (#335). `isDirty` IS
  // `configUpdate(...)` non-empty, which is the exact gate `saveSettings` puts its restart behind —
  // so the warning that a save will interrupt a pass, the bar's retry slot and the restart itself
  // are three readers of one answer rather than three definitions of one question. A fixture's
  // `dirty` names it too, and it is one frame: `8a Skip rules` is the only one that sets it, and
  // what it stages is `removing: "video-raw/**"` — an `exclude` entry, which is a config field. No
  // frame stages a notification policy, so the two readings cannot disagree on a drawn state.
  const configStaged = ui?.dirty ?? isDirty(saved, edits);
  const dirty = ui?.dirty ?? (configStaged || policyStaged);
  // Normalised once, where the reply lands — this is only reading it back.
  const ending = settingsSaveOutcome?.ending ?? null;
  return {
    tab,
    saved,
    config,
    skip,
    cannot,
    dirty,
    configStaged,
    onSeeUnsyncable: () => navigate("neverSynced"),
    // The frame names it; otherwise the staged value, then what is on disk.
    notifyPolicy: ui?.notifyPolicy ?? notifyPolicyEdit ?? notifyPolicy,
    drafts: slot.drafts,
    // Which schedule editor is showing. A FRAME NAMES IT (`8a Schedule monthly`), which is what
    // makes the monthly variant reachable by the fixture harness at all — without this the frame
    // rendered the weekly panel and every monthly node went unexercised by every gate.
    scheduleDraftMonthly: ui?.schedule ? ui.schedule === "monthly" : slot.scheduleMonthly,
    saving: settingsSaving,
    // The ending the last save's restart left UNRESOLVED, or null (#320/#335) — the one post-save
    // state with an action attached, which is why the bar reads it rather than reading the sentence
    // it also decides. The token and not a reason string: `never_stopped` (the daemon is up on the
    // old settings) and `not_started` (nothing is running) want the same button and opposite
    // re-validation, and a string cannot say which is which.
    restartEnding: restartUnresolved(ending) ? ending : null,
    // The amber line, when a single removal is staged. Any other staged change leaves the neutral
    // note: the deck has one cost sentence and it says `One rule removed`, so a second removal has
    // no wording and inventing a plural would be inventing the number in it too.
    cost: removalCost(saved.exclude, config.exclude, skip),
    // WHAT JUST HAPPENED, which outranks even the cost line — see `barNoteOf`. A control that
    // failed has to be able to say so over standing information about a staged change, or the fix
    // for one silence introduces another.
    // `restarting` OUTRANKS `saving`, and it did not have to before: the restart now happens INSIDE
    // a save (#320), where `settingsSaving` is still true and is the slower of the two by an order
    // of magnitude. Reporting `Saving…` for the eight seconds the daemon takes to stop would name
    // the wrong step and look stuck on it.
    notice:
      slot.notice ?? (settingsRestarting ? SETTINGS.restarting : settingsSaving ? SETTINGS.saving : null),
    // What the save left behind: the sentence for the ending it had (#320/#335). Built from the
    // ending rather than stored beside it, so the sentence and the button can never describe two
    // different endings.
    note: ending == null ? null : saveNoteFor(ending, settingsSaveOutcome?.reason ?? ""),
    // WHETHER THE CONFIG IS KNOWN AT ALL. `read_config` rejects an unparseable file and
    // `refreshConfig` swallows it, so the pair's reply stays absent — and `?? {}` would draw that as an
    // empty, valid config: both folder fields blank, live updates on, and a deletion policy card
    // selected that is not the one running. A screen may not answer for a file it could not read.
    // `configLoaded && !configError`, and the second half is not redundant: `refreshConfig` runs on
    // a timer, so a file that PARSED once and stops parsing later leaves `configLoaded` true with a
    // stale reply behind it. The screen would then keep a deletion-policy card selected from
    // the last good read, underneath a banner saying the file could not be read — answering for it
    // and disclaiming it in the same breath.
    //
    // AND A REPLY FOR THIS PAIR (#102 phase 5b-1). `configLoaded` says some pair's file was read; the
    // pair that has just been switched to has no reply of its own until the next read lands, and the
    // screen may not answer for it with another pair's values or with an empty config. For one pair
    // the two are the same fact: the reply is filed in the same step that sets `configLoaded`.
    loaded: Boolean(activeFixture()) || (configLoaded && !configError && configByPair.get(pair) != null),
    configError: activeFixture() ? null : configError,
    // HOW MANY FOLDERS THERE ARE, and the list that is drawn only when there are two or more (#102
    // phase 5c-2). `pairCountNow()` is the larger of the daemon's list and the file's, so a folder only one
    // of them knows still counts: a save restarts the one daemon under all of them.
    folderCount: pairCountNow(),
    folders: pairCountNow() >= 2 ? foldersProps() : null,
    // The daemon is mid-pass, or one has just been asked for: `Sweep now` would queue behind it
    // with nothing to show for the click. A plan rehearsal counts here on purpose — it holds the
    // same main loop, so a sweep asked for during one queues behind it just the same.
    syncing: settingsSweeping || Boolean(store.select.response()?.syncing),
    // A DIFFERENT QUESTION, AND THE INTERRUPT WARNING'S (#335): is a pass that moves files running?
    // `syncing` is claimed by a plan-only rehearsal too — it must be, or `activity` is gated off
    // every status reply (`CLAUDE.md`) — so the warning `Saving … stops the sync that is running
    // now` named a sync that was not running whenever the Plan screen was mid-rehearsal. The pass
    // block's `kind` is the wire-visible half of `daemon.rs`'s `a_counted_pass_is_running`, and
    // `planProgress` already reads it for the same reason. `!== "plan"` and not `=== <something>`:
    // an activity block that is briefly absent counts as a sync, which is the safe direction for a
    // warning about interrupting one.
    //
    // `settingsSweeping` covers ONE ROUND TRIP and no more — the click until `resync` acks — which
    // is a real window in which a restart destroys a `force_full_walk` latch nothing has consumed,
    // and it is the same half-second of optimism the `Sweep now` button itself is disabled for.
    // Once the sweep is actually running it is a counted pass like any other and the clause above
    // is what reports it; this does not stand in for that.
    countedSync:
      settingsSweeping ||
      (Boolean(store.select.response()?.syncing) && store.select.response()?.activity?.pass?.kind !== "plan"),
    handlers: {
      onTab: (id) => {
        settingsTab = id;
        render();
      },
      onRoot: (key, value) => stageSetting(key, value, pair),
      onField: (key, value) => stageSetting(key, value, pair),
      onEvents: (on) => stageSetting("events_driven", on, pair),
      // #193. `null` stages the EMPTY STRING, not a missing key: `write_config` clears a key whose
      // value is empty, and clearing is how a scheduled sweep is turned off — there is no off value
      // to write. Staging `undefined` would drop the field from the update and leave the old
      // schedule on disk while the screen showed none.
      onSchedule: (schedule) =>
        stageSetting("full_scan_schedule", schedule ? formatSchedule(schedule) : "", pair),
      // The editor's mode, NOT a setting. With no schedule configured there is nothing to convert,
      // and switching a live one would move the sweep to a day nobody chose — so this stages
      // nothing and the screen stays clean until a day is picked.
      onScheduleMode: (monthly) => {
        patchStaging(pair, { scheduleMonthly: monthly });
        render();
      },
      // ONE FIELD, not three. `deletion_policy` is a daemon key now (#194) and `set_deletion_policy`
      // always writes both directions, in whichever spelling the file already uses — so staging the
      // two booleans beside it would briefly put both spellings in one document, which is a config
      // the daemon refuses to start on. A card that set one direction would still leave a pair no
      // card describes (DEVIATIONS §68); the engine's enum is what guarantees it cannot.
      onPolicy: (policy) => {
        patchStaging(pair, { notice: null, edits: { ...stagedFor(pair).edits, deletion_policy: policy.id } });
        clearSaveOutcome();
        render();
      },
      // ITS OWN KEY, staged independently of the policy above. The two are adjacent on the tab and
      // are two settings: one decides whether a deletion waits for you, this one what happens when
      // it goes ahead. Staging them together would make choosing one write the other.
      onDisposal: (disposal) => {
        patchStaging(pair, {
          notice: null,
          edits: { ...stagedFor(pair).edits, local_delete_mode: disposal.id },
        });
        clearSaveOutcome();
        render();
      },
      // Staged, not written. `null` once it matches what is saved, so choosing the card that is
      // already selected does not mark the screen dirty — the same rule `configUpdate` applies to
      // every other control.
      onNotifyPolicy: (id) => {
        patchStaging(pair, { notice: null });
        notifyPolicyEdit = id === notifyPolicy ? null : id;
        clearSaveOutcome();
        render();
      },
      // The Activity link inside the rules sheet. A real route change, not decoration.
      onRoute: (id) => navigate(id),
      onDraft: (key, value) => {
        patchStaging(pair, { drafts: { ...stagedFor(pair).drafts, [key]: value } });
        render();
      },
      onAddRule: () => addPattern("exclude", pair),
      onRemoveRule: (pattern) => removePattern("exclude", pattern, pair),
      onAddInclude: () => addPattern("include", pair),
      onRemoveInclude: (pattern) => removePattern("include", pattern, pair),
      onChoose: () => chooseLocalRoot(pair),
      onSweep: () => sweepNow(pair),
      onSave: () => saveSettings(pair),
      onDiscard: () => {
        patchStaging(pair, { edits: {}, drafts: BLANK_STAGING.drafts });
        notifyPolicyEdit = null;
        clearSaveOutcome();
        render();
      },
      onRestart: () => restartAfterSave(pair),
    },
  };
}

/** The current staged value of a list field for `pair`, saved-or-staged. */
const stagedList = (key, pair) =>
  stagedFor(pair).edits[key] ?? (activeFixture()?.config ?? configByPair.get(pair))?.[key] ?? [];

function addPattern(key, pair) {
  const pattern = stagedFor(pair).drafts[key].trim();
  // A duplicate is not an error and not a second row: the rule is already there, so the field
  // clears and nothing is staged.
  if (pattern && !stagedList(key, pair).includes(pattern)) {
    patchStaging(pair, { edits: { ...stagedFor(pair).edits, [key]: [...stagedList(key, pair), pattern] } });
    clearSaveOutcome();
  }
  patchStaging(pair, { drafts: { ...stagedFor(pair).drafts, [key]: "" } });
  render();
}

/** Un-stages an addition and stages a removal, from one path — both are "not in the staged list". */
function removePattern(key, pattern, pair) {
  patchStaging(pair, {
    edits: { ...stagedFor(pair).edits, [key]: stagedList(key, pair).filter((p) => p !== pattern) },
  });
  clearSaveOutcome();
  render();
}

/**
 * `Choose…`. A dismissed picker answers `null`, which is not an error and must not read as one —
 * and a picker that could not OPEN rejects, which is an error and must not read as a dismissal.
 * `choose_folder` returns `Result<Option<String>, String>` precisely so the two stay apart.
 */
async function chooseLocalRoot(pair) {
  try {
    const picked = await api.chooseFolder(
      stagedFor(pair).edits.local_root ?? configByPair.get(pair)?.local_root ?? null,
    );
    // Staged into the pair the dialog was opened for: it is modal, but it is not instant.
    if (picked) stageSetting("local_root", picked, pair);
  } catch (error) {
    patchStaging(pair, { notice: SETTINGS.chooseFailed(String(error?.message ?? error)) });
    render();
  }
}

/** `Sweep now` — a full-tree walk on the next pass, which `sync_now` is not. */
async function sweepNow(pair) {
  if (settingsSweeping) return;
  settingsSweeping = true;
  patchStaging(pair, { notice: SETTINGS.sweeping });
  render();
  try {
    // THE REPLY HAS TO BE READ, not just awaited. `resync` is a status command, and every one of
    // them folds a socket failure into the payload rather than rejecting (`commands.rs`) — so
    // against a stopped daemon, or one older than `ControlCommand::Resync`, the `catch` below never
    // fires and an unread reply is a button that does nothing and says nothing.
    const reply = await api.resync({ pair });
    patchStaging(pair, { notice: reply?.error ? SETTINGS.sweepFailed(reply.error) : null });
  } catch (error) {
    patchStaging(pair, { notice: SETTINGS.sweepFailed(String(error?.message ?? error)) });
  }
  // Released as soon as the daemon has answered. From here the button stays disabled on the reply's
  // own `syncing`, which is the fact rather than our memory of having asked — so re-poll for it.
  settingsSweeping = false;
  clearTimeout(pollTimer);
  poll();
}

/**
 * Write the staged edits, and only them — then restart the service that has to run them (#320).
 *
 * The refusal opens `8a Save refused` rather than being swallowed: `write_config` rejects a config
 * the daemon's own parser would refuse, and a save that silently did nothing is the failure that
 * dialog exists to prevent.
 *
 * **A CONFIG SAVE RESTARTS THE DAEMON.** There is no reload path in the engine — no SIGHUP, no
 * watcher — so a written file and a running daemon disagreed until somebody pressed a second
 * button, and that gap is reachable by an ordinary sequence: change the sync folder, open Plan, and
 * the preview is the file's pair while `Run` executes the daemon's (#320). Restarting here makes
 * the mismatch unreachable rather than reporting it. The interruption is the accepted cost, and
 * `SETTINGS.saveInterrupts` is what makes it something the person saw coming.
 *
 * FIVE endings, all of them said out loud and each with its own sentence (#335): restarted; not
 * running, so nothing was started (see `restart_service`'s `only_if_running`); the start failed, so
 * nothing is running at all; it never stopped, so the OLD daemon is still up on the OLD settings;
 * or it could not be told apart, so nothing was done. The last three keep `Restart it now` on the
 * bar until the state they name is over — see `restartUnresolved` and `clearsRestartFailure`.
 */
async function saveSettings(pair = store.select.pairName()) {
  // `pair` IS THE ONE THE EDITS WERE STAGED FOR (class W). The screen's `Save` passes the pair it was
  // drawn for; Ctrl S, which has no screen of its own to ask, passes the pair on screen now. Either
  // way it is fixed here, before anything is awaited: the selection can move while the write is in
  // flight, and the write, the refresh and the restart must all be about the same folder.
  const saved = activeFixture()?.config ?? configByPair.get(pair) ?? {};
  const update = configUpdate(saved, stagedFor(pair).edits);
  const policy = notifyPolicyEdit != null && notifyPolicyEdit !== notifyPolicy ? notifyPolicyEdit : null;
  if (settingsSaving || (Object.keys(update).length === 0 && !policy)) return;
  settingsSaving = true;
  settingsError = null;
  render();
  // THE MAP AS IT WAS SENT. Every staging path assigns a fresh object, so identity is an exact
  // "nothing was staged since" test — and clearing the whole map on the way back would discard a
  // keystroke typed while the write was in flight, while telling the person it had been saved.
  const sent = stagedFor(pair).edits;
  try {
    // TWO FILES, AND `notify_policy` NEVER GOES IN THE DAEMON'S. Its config parser is
    // `deny_unknown_fields`, so one stray key stops the daemon starting; the GUI's own `gui.toml`
    // is where a GUI-local preference belongs.
    //
    // THE CONFIG GOES FIRST, because it is the one that can be refused. `8a Save refused` says
    // "Nothing was saved. Your old settings are still running", and a policy written before a
    // refusal would make that sentence false about the one thing that HAD been written.
    if (Object.keys(update).length) await api.writeConfig(update, { pair });
    if (policy) {
      await api.writeNotifyPolicy(policy);
      notifyPolicy = policy;
      notifyPolicyEdit = null;
    }
    if (stagedFor(pair).edits === sent) patchStaging(pair, { edits: {} });
    // The rules changed under the report, so the counts on the skip tab are about a config that is
    // no longer on disk. Ask again rather than showing yesterday's numbers next to today's rules.
    skipRuleReport = null;
    skipRuleAsked = false;
    await refreshConfig(pair);
    // ONLY FOR A DAEMON-CONFIG WRITE. A policy-only save touches `gui.toml`, which the daemon never
    // reads, so bouncing it would interrupt a transfer for a setting it has never heard of.
    if (Object.keys(update).length) await restartForSave(pair);
  } catch (error) {
    settingsError = String(error?.message ?? error);
    openOverlay("saveRefused");
  }
  settingsSaving = false;
  render();
}

/**
 * The restart a save owns. Records the ending whichever way it ends, and never throws: its caller's
 * `catch` opens `8a Save refused`, whose sentence is `Nothing was saved` — false here, because the
 * file is written and only the restart failed.
 *
 * THE ENDING COMES OFF THE PAYLOAD, NOT OFF `catch` (#335). `restart_service` answers a typed
 * `RestartOutcome` on the Ok side precisely so this cannot go back to reading a rejection's string:
 * `not_started` and `never_stopped` are opposite states of someone's files and used to arrive as the
 * same `Err`. A rejection now means the request itself never got as far as an ending, which is
 * `undetermined` — the one answer that claims nothing.
 */
async function restartForSave(pair) {
  settingsRestarting = true;
  // WITH ITS SIBLING'S CLEAR, which it was missing (#335). `barNoteOf` puts `notice` first, so a
  // stale `The full sweep didn't start …` from a `Sweep now` minutes ago masked ALL of the endings
  // below — `Restart it now` appearing on the bar with no sentence explaining why. The saved pair's:
  // the restart is daemon-wide, and the notice that would mask its ending is the one on the screen
  // that asked for it.
  patchStaging(pair, { notice: null });
  render();
  try {
    // `onlyIfRunning`: a save is not a request to start syncing. See `restart_service`.
    const outcome = await api.restartService(true);
    noteRestartOutcome(outcome);
  } catch (error) {
    noteRestartFailure(error);
  }
  settingsRestarting = false;
  clearTimeout(pollTimer);
  poll();
}

/** The typed ending, normalised once, where the reply lands. */
function noteRestartOutcome(outcome) {
  // Display only, and the daemon's own words unrewritten (voice rule 4). `detail` on the endings
  // that worked, `reason` on the ones that did not — no sentence is built from it.
  latchRestart(restartEndingOf(outcome), String(outcome?.reason ?? outcome?.detail ?? ""));
}

/**
 * A rejection is not an ending: the command failed before it could report one.
 *
 * `undetermined` and not a failure ending, because the two failure endings are claims about what is
 * running — one says nothing is, the other says the old process still is — and a request that never
 * reached the daemon has observed neither.
 */
function noteRestartFailure(error) {
  latchRestart("undetermined", String(error?.message ?? error));
}

/**
 * THE ONE PLACE THE LATCH IS BUILT, and therefore the one place its evidence floor is stamped.
 *
 * `evidenceFloor` is the newest status request issued *before* this outcome existed, so every
 * answer already in hand — and every poll still in flight — is older than the thing it would be
 * judging. Only a request issued after this line may retire the latch (`clearsRestartFailure`).
 * Two construction sites would mean one of them forgetting the stamp, and a latch with no floor
 * defaults to 0 and is cleared by the very first poll: the bug this replaces, silently restored.
 */
function latchRestart(ending, reason) {
  settingsSaveOutcome = { ending, reason, evidenceFloor: store.select.statusesIssued() };
}

/**
 * `Restart it now` — the retry a failed save-restart leaves behind, and the only place that button
 * survives (#320).
 *
 * NOT `onlyIfRunning`. The failure this answers may have stopped the daemon and failed to start it
 * again, which is the state where "it was not running, so do nothing" would be exactly wrong: the
 * screen would report success and leave nothing running.
 *
 * IT WRITES THE SAME LATCH ITS CALLER DOES, and it has to (#335): a retry that failed again leaves
 * an ending of its own, and one recorded as a bare notice would sit outside the re-validation on
 * the poll — so a daemon that came back up would not retire it, and one that did not come back
 * would lose it on the next navigation.
 */
async function restartAfterSave(pair = store.select.pairName()) {
  if (settingsRestarting) return;
  settingsRestarting = true;
  patchStaging(pair, { notice: null });
  render();
  try {
    // `restart_service` answers the ending on the Ok payload, so this reads the reply rather than
    // treating "it resolved" as "it worked" — the trap #140 recorded on the approve/deny buttons
    // and #320's own review recorded again on `resync`.
    noteRestartOutcome(await api.restartService());
  } catch (error) {
    // `restart_service` DOES reject on an infrastructure failure, unlike the status commands — and
    // it waits up to eight seconds for the daemon to stop, so this is both a real failure path and
    // a slow one. Its reason went into `settingsError` before the S6 review, which only the refusal
    // dialog reads and only `saveSettings` opens.
    noteRestartFailure(error);
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
/**
 * The sub-screen the flow has open, or null (#244) — `skip` or `actions`.
 *
 * ORTHOGONAL TO THE STEP, deliberately. A detour is opened from a step and closed back onto it, and
 * the step never moves while one is open — so "return to the point in setup you left" needs nothing
 * remembered and cannot restore the wrong one. It is also why the header still reads `step 1 of 2`
 * inside a detour: the step it belongs to is the step you are on.
 *
 * CLEARED IN TWO PLACES, not one, and neither is optional (#337): `resetOnboardingFlow` when the
 * flow ends by completing, `render`'s `entersOnboardingTakeover` edge when it instead ends by the
 * latch releasing out from under an open detour — see that function for why the flow can end either
 * way and why only the latch's edge (not "while latched") may touch this. Since #360 the edge reaches
 * this field via `resetOnboardingFlow()` itself (alongside the four other fields that comment names),
 * except on the one arm that carries a FRESH `onboardingFailure` rather than a stale prior session —
 * that branch clears this field directly, because the rest of what `resetOnboardingFlow` would wipe
 * is exactly what the failure needs kept.
 */
let onboardingDetour = null;
/**
 * The skip rules the flow has staged, or null before anything is staged (then the config's own).
 *
 * NOT WRITTEN UNTIL `See what will happen` WRITES THE PAIR. The flow promises nothing happens until
 * you approve the plan, and the rehearsal on step 2 has to be a rehearsal OF these rules — which it
 * is, because `run_dry_run` shells `proton-syncd --dry-run` against the file step 1 has just
 * written.
 */
let onboardingSkipRules = null;
/** The add field's draft. Module state, not the input's: this body is rebuilt on the ~2s poll. */
let onboardingSkipDraft = "";
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

/**
 * The skip rules as they stand: staged if anything has been staged, else the config's own.
 *
 * A FUNCTION, not a value captured per render — the same rule `onboardingRootsNow` states. And the
 * config's own rather than an empty list, because the takeover is entered on `firstRun` too: a
 * reachable daemon that has never synced HAS a config, and offering an empty list over it would
 * stage the removal of every rule already in the file the moment anything else was added.
 */
function onboardingSkipRulesNow() {
  if (onboardingSkipRules) return onboardingSkipRules;
  return activeFixture()?.config?.exclude ?? viewedConfig()?.exclude ?? [];
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

/**
 * The pair the first-run flow runs against. THE TAKEOVER NEVER ARMS AT TWO PAIRS (E14), so while it is
 * up there is exactly one, and the selection IS it — which is the only reason this may read the
 * selection when a class-W command runs instead of carrying a captured value. Anything that can run
 * at two pairs carries its pair (`item.pair`, `planPair`, the props' `pair`).
 */
const onboardingPair = () => store.select.pairName();

/** The rehearsal behind step 2 — one child at a time, the same token discipline as `ensurePlan`. */
async function ensureOnboardingPlan() {
  if (onboardingWaiting !== null || onboardingAnswered === onboardingSeq) return;
  const seq = onboardingSeq;
  onboardingWaiting = seq;
  let payload = null;
  let error = null;
  try {
    payload = await api.runDryRun({ pair: onboardingPair() });
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
    local: live?.local_root ?? viewedConfig()?.local_root ?? PROPOSED_LOCAL,
    remote: live?.remote_root ?? viewedConfig()?.remote_root ?? PROPOSED_REMOTE,
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
    detour: onboardingDetour,
    skipRules: onboardingSkipRulesNow(),
    skipDraft: onboardingSkipDraft,
    local: roots.local,
    remote: roots.remote,
    dryRun: onboardingAnswered === onboardingSeq ? onboardingDryRun : null,
    error: onboardingAnswered === onboardingSeq ? onboardingError : null,
    checking: onboardingAnswered !== onboardingSeq,
    // THE FRAME'S OWN TIMING WHEN A FRAME NAMES ONE, and this is a determinism fix rather than a
    // fidelity one (#193's CI run is what surfaced it). `checkedAt` feeds `since()`, so the drawn
    // string is whatever the wall clock says at the moment the harness measures: `0 seconds ago`
    // usually, `1 second ago` if the run happens to land on that second — and the singular drops a
    // character, taking 6.6px off the span. The recorded deviation was therefore right only for
    // runs that missed second 1, and a slower CI machine failed the build on a screen nothing had
    // touched.
    //
    // `planTiming.workedOutEpochSecs` was already in the fixture, declared and read by nothing —
    // the same shape as `ui.schedule` one screen over. Reading it renders `40 seconds ago`, which
    // is what the frame draws, and a few seconds of drift stays inside that bucket.
    checkedAt: activeFixture()?.planTiming?.workedOutEpochSecs ?? onboardingCheckedAt,
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
      // The two sub-screens (#244). Opening one changes nothing else about the flow: the step is
      // untouched, so closing it returns to exactly where it was opened from.
      onDetour: (id) => {
        onboardingDetour = id;
        onboardingSkipDraft = "";
        render();
      },
      onCloseDetour: () => {
        onboardingDetour = null;
        onboardingSkipDraft = "";
        render();
      },
      onSkipDraft: (value) => {
        // NO RENDER. The field is a live `<input>` and this body is rebuilt on the ~2s poll — a
        // render per keystroke would rebuild the field under the caret. The draft is read back on
        // the next rebuild, and `Add` is what makes it visible.
        onboardingSkipDraft = value;
      },
      onAddSkipRule: () => {
        const pattern = onboardingSkipDraft.trim();
        // A duplicate is not an error and not a second row — S6's own rule for the same field.
        if (pattern && !onboardingSkipRulesNow().includes(pattern)) {
          onboardingSkipRules = [...onboardingSkipRulesNow(), pattern];
        }
        onboardingSkipDraft = "";
        render();
      },
      onRemoveSkipRule: (pattern) => {
        onboardingSkipRules = onboardingSkipRulesNow().filter((p) => p !== pattern);
        render();
      },
      onNext: async () => {
        if (onboardingStarting) return;
        onboardingStarting = true;
        try {
          // The pair has to be ON DISK before the rehearsal: `run_dry_run` shells
          // `proton-syncd --dry-run`, which reads the config file and not this screen.
          const chosen = onboardingRootsNow();
          // The pair these roots are written FOR, captured before anything is awaited (class W): the
          // folder the takeover is about — the only one there is, since it never arms beside two.
          const pair = store.select.pairName();
          // THE SKIP RULES GO WITH IT, and only when they are DIFFERENT from what is on disk
          // (#244). `write_config` edits the TOML in place and a field sent as `Some` is a key
          // written, so sending an unchanged list would materialise an `exclude = []` line in a
          // file that never had one — the same promise `configUpdate` keeps on the Settings screen,
          // and by the same array comparison. "Touched" is not the test: adding a rule and removing
          // it again leaves a staged `[]` that is identical to the absent key it would write.
          const update = { local_root: chosen.local, remote_root: chosen.remote };
          const staged = configUpdate(
            { exclude: configByPair.get(pair)?.exclude ?? [] },
            { exclude: onboardingSkipRulesNow() },
          );
          if ("exclude" in staged) update.exclude = staged.exclude;
          await api.writeConfig(update, { pair });
          await refreshConfig(pair);
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

/**
 * The merge dialog (`9a First sync`), for whichever flow is showing it: the first-ever setup's, or a
 * folder that was just added (decision D6). One body, so the two cannot come to draw a different merge.
 *
 * `summary` is the REHEARSED plan's, and the added folder's is `null` because none was made: with no
 * plan there is no footer sentence, and so nothing is claimed about what the merge will or did do to a
 * file (`mergeFooterText`). That is the point of passing it rather than reading it here.
 */
function mergeDialogContent(reply, summary, handlers) {
  const activity = reply?.activity ?? null;
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
      handlers,
    }),
  };
}

/**
 * The reply a merge dialog draws its two numbers from. An added folder's is the status reply only while
 * it says it is ABOUT that folder: the window may still be describing the one selected before, and that
 * folder's pass is not this merge.
 */
function mergeReplyOf(id) {
  const reply = store.select.response();
  if (id !== "folderMerge") return reply;
  return folderMerge ? replyAbout(reply, folderMerge.pair) : null;
}

/** `9a First sync` for a folder that was just added: no plan, so no footer — and `Pause` pauses THAT folder. */
function folderMergeDialog() {
  const merge = folderMerge;
  if (!merge) return null;
  return mergeDialogContent(mergeReplyOf("folderMerge"), null, {
    // CAPTURED, class W: the folder the dialog was opened for. Pausing ends the watch, as it does in the
    // first-run merge — a paused folder completes no pass, and the dialog would wait for ever.
    onPause: async () => {
      const pair = merge.pair;
      await command(() => api.pause({ pair }));
      if (folderMerge === merge) {
        folderMerge = null;
        if (dialogOverlay === "folderMerge") leaveDialog();
      }
      render();
    },
  });
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
    // THE FIXTURE'S PLAN FIRST, the same fallback the other two branches take (`?? cliPresence`,
    // `?? onboardingAgreed`): the footer sentence comes from the step-2 rehearsal, which is module
    // state no `?frame=` can reach, so without this the one in-flight claim this flow makes about
    // someone's files is never compared against the frame that draws it.
    const summary = (activeFixture()?.dryRun ?? onboardingDryRun)?.report?.summary ?? null;
    return mergeDialogContent(reply, summary, {
      // PAUSING ENDS THE FLOW. A paused daemon completes no pass, so `mergeOutcomeOf` would
      // wait forever behind a dialog with no ✕ and no Esc. Handing off to the main screen —
      // which draws `Paused` and a `Resume` — is the same call routes.js makes for a state
      // onboarding cannot resolve. The consent is not obtained on this path; the daemon's own
      // delete guard is on by default, so every deletion still goes through the Deletions
      // screen. §79k.
      onPause: async () => {
        const pair = onboardingPair();
        await command(() => api.pause({ pair }));
        resetOnboardingFlow();
        render();
      },
    });
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
            const pair = onboardingPair();
            await command(() => api.resume({ pair }));
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
 *
 * Called from two kinds of place, not one (#360): here, where the flow completes on its own terms
 * (agreeing to the consent, or resuming past it), AND from `render`'s arm edge, for the re-entry the
 * comment above is actually about — a session that ended some OTHER way (the latch released out from
 * under it) and is now starting over. `onboardingRoots` is reset here too rather than kept as a
 * convenience: `onboardingRootsNow()` falls back to the live/saved pair when this is null, so nothing
 * typed and WRITTEN is lost, and keeping a stale draft would let it silently outrank a pair that
 * changed since (Settings, a direct edit) — the opposite of "a configured pair beats the proposal".
 */
function resetOnboardingFlow() {
  onboardingStage = null;
  onboardingStep = "folders";
  // WITH THE STEP, because a detour is a place inside one (#244): a later re-entry must open at
  // step 1 and not inside a sub-screen of the flow that ended. The staged rules go for the same
  // reason the roots do — they were written when the pair was, and a re-entry is a new decision.
  onboardingDetour = null;
  onboardingSkipRules = null;
  onboardingSkipDraft = "";
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
      const pair = onboardingPair();
      command(() => api.pause({ pair }));
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

// ---- adding and removing folders (#102 phase 5c-2) ----
//
// Adding is a DIALOG, opened from Settings and from the ⋯ menu at any count, and removing is a
// confirmation opened from a row of the Settings list. The first-run takeover is not reused: it writes the
// implicit single pair and at two folders it is shut (E14). What IS reused is the merge dialog, which the
// add shows for the NEW folder once the daemon runs it (decision D6).
//
// THE ORDER IS THE DESIGN, and each step waits for the one before it:
//
//   1. `Check folders`: the engine is asked whether the add would go ahead (`check_add_pair`), and both
//      sides are priced (`probe_folder`). Nothing is written. A change to a field makes the check stale,
//      and the button is `Check folders` again (`addViewOf`).
//   2. `Add folder`: ONE `add_pair`, which promotes an implicit file to `[[pair]]` form if it has to and
//      carries the staged skip rules in the same write.
//   3. `restart_service(only_if_running)`: the daemon only reads its file at start. An ending that leaves
//      something wrong is said in the dialog AND latched for the Settings bar's `Restart it now`.
//   4. Wait for the daemon's `pairs[]` to list the name. The FILE lists it from step 2; only the daemon's
//      list says it is running.
//   5. `showFolder(name)`: select it, and have the store describe it — the only step that moves the
//      selection without a click, so everything before it carries the name it captured at step 1.
//   6. The merge dialog, watching THE NEW FOLDER'S `reconcile_seq` — read from its own summary the moment
//      it was first listed, and compared only against a reply that says it is about that folder.

/** The add dialog's state — `folders.js` `blankAdd()` — or null. */
let addFolder = null;
/** The removal dialog's state — `blankRemove(name)` — or null. */
let removeFolder = null;
/** The merge of a folder that was just added: `{ pair, seq, waits }`, or null. */
let folderMerge = null;
/** The debounce between a keystroke and the engine's answer about it. */
let addFolderTimer = null;
/** How long the engine is given to answer about what was typed, after the last keystroke. */
const ADD_CHECK_MS = 150;

/** The roster the dialogs speak from: the frame's, else the settings file's — which is in FILE order. */
const folderRoster = () => activeFixture()?.config?.pairs ?? configRoster;

function openAddFolder() {
  menuOpen = false;
  // Before the state below is touched: it would replace the flow of a dialog that may not be left.
  if (dialogVetoed(dialogOverlay)) {
    render();
    return;
  }
  removeFolder = null;
  folderMerge = null;
  clearTimeout(addFolderTimer);
  addFolder = blankAdd();
  openOverlay("addFolder");
}

function openRemoveFolder(name) {
  if (dialogVetoed(dialogOverlay)) return;
  clearTimeout(addFolderTimer);
  addFolder = null;
  folderMerge = null;
  removeFolder = blankRemove(name);
  openOverlay("removeFolder");
}

/** The state the open dialog belongs to is gone (or was never there): a dialog cannot outlive it. */
function dropOrphanedFolderDialog() {
  if (activeFixture()) return;
  const orphaned =
    (dialogOverlay === "addFolder" && !addFolder) ||
    (dialogOverlay === "removeFolder" && !removeFolder) ||
    (dialogOverlay === "folderMerge" && !folderMerge);
  if (!orphaned) return;
  leaveDialog();
}

/** Something is in flight in this dialog and it may not be left: see `routes.js`'s note on `addFolder`. */
function dialogVetoed(id) {
  if (id === "addFolder") return Boolean(addFolder && BUSY_PHASES.includes(addFolder.phase));
  if (id === "removeFolder") return removeFolder?.phase === "removing";
  return false;
}

/** A dialog was left: what it held goes with it. The merge's watch ends; the daemon's pass does not. */
function releaseFolderState(id) {
  if (id === "addFolder") {
    clearTimeout(addFolderTimer);
    addFolder = null;
  } else if (id === "removeFolder") {
    removeFolder = null;
  } else if (id === "folderMerge") {
    folderMerge = null;
  }
}

// ---- the add dialog: typing ----

/** The flow, if it is the one being asked about and is not mid-flight (a late event from a closed dialog is nothing). */
function liveAdd(flow) {
  return (
    addFolder === flow &&
    !BUSY_PHASES.includes(flow.phase) &&
    flow.phase !== "added" &&
    flow.phase !== "listed"
  );
}

function editAddFolder(field, value) {
  const flow = addFolder;
  if (!flow || !liveAdd(flow)) return;
  flow.error = null;
  if (field === "draft") {
    flow.draft = value;
    return;
  }
  if (field === "name") flow.nameTouched = true;
  flow[field] = value;
  // NO PRICE IS DROPPED HERE. The prices belong to `checkedKey` — they are stored with it, at the end of the
  // check — and are drawn only while the text on screen is the text that was checked (`view.checked`), so an
  // edit hides them without losing them and a return to the same text brings back what the check found.
  // Nulling the edited side's price made the dialog arm `Add folder` again with that side's price missing.
  // Anything in flight is about text that has moved on.
  flow.seq += 1;
  if (flow.phase === "checking") flow.phase = "form";
  clearTimeout(addFolderTimer);
  addFolderTimer = setTimeout(() => askEngineAboutAdd(flow), ADD_CHECK_MS);
  render();
}

/**
 * Ask the engine whether this add would go ahead, and file the answer under the text it was asked about.
 * Returns the reply, or `null` when it was overtaken (typed on, or the dialog was left). A failure of the
 * command itself is a refusal in its own words, never a silent pass — the add would then be the first
 * thing to find out.
 *
 * When the name field has not been touched it FOLLOWS the folder's own name (`suggested_name`); the answer
 * for the old name is then stale, so it asks again, once: the second time the field already says it.
 */
async function askEngineAboutAdd(flow) {
  if (addFolder !== flow) return null;
  const key = addKeyOf(flow);
  const seq = (flow.seq += 1);
  let reply;
  try {
    reply = await api.checkAddPair(flow.name.trim(), addRequestOf(flow));
  } catch (error) {
    reply = {
      suggested_name: "",
      name_error: null,
      refusal: String(error?.message ?? error),
      surviving_index: null,
      warnings: [],
    };
  }
  if (addFolder !== flow || flow.seq !== seq) return null;
  flow.pre = reply;
  flow.preKey = key;
  const suggestion = reply?.suggested_name;
  if (!flow.nameTouched && flow.local.trim() && suggestion && flow.name !== suggestion) {
    flow.name = suggestion;
    render();
    return askEngineAboutAdd(flow);
  }
  render();
  return reply;
}

async function chooseAddFolder() {
  const flow = addFolder;
  if (!flow || !liveAdd(flow)) return;
  let picked;
  try {
    picked = await api.chooseFolder(flow.local.trim() || null);
  } catch (error) {
    flow.error = SETTINGS.chooseFailed(String(error?.message ?? error));
    render();
    return;
  }
  if (!picked || !liveAdd(flow)) return;
  editAddFolder("local", picked);
  clearTimeout(addFolderTimer);
  askEngineAboutAdd(flow);
}

function addFolderRule() {
  const flow = addFolder;
  if (!flow || !liveAdd(flow)) return;
  flow.rules = withRule(flow.rules, flow.draft);
  flow.draft = "";
  render();
}

function removeAddFolderRule(rule) {
  const flow = addFolder;
  if (!flow || !liveAdd(flow)) return;
  flow.rules = flow.rules.filter((existing) => existing !== rule);
  render();
}

// ---- the add dialog: the button ----

async function pressAddFolder() {
  const flow = addFolder;
  if (!flow) return;
  const view = addViewOf(flow);
  if (view.primary === "done") {
    // `Done` on a listed folder carries on to the merge the folder's first pass is watched by; on any other
    // settled dialog there is nothing to carry on to.
    if (flow.phase === "listed" && flow.merge) showFolderMerge(flow.merge);
    else closeOverlay();
    return;
  }
  if (!view.primaryEnabled) return;
  if (view.primary === "check") await checkAddFolder(flow);
  else if (view.primary === "add") await commitAddFolder(flow);
}

/** `Check folders`: the engine's answer first, then the price of both sides — only if the add could go ahead. */
async function checkAddFolder(flow) {
  if (flow.phase !== "form") return;
  flow.phase = "checking";
  flow.error = null;
  clearTimeout(addFolderTimer);
  render();
  const key = addKeyOf(flow);
  const pre = await askEngineAboutAdd(flow);
  // Overtaken: the text moved on while it was being asked, or the dialog was left.
  if (addFolder !== flow) return;
  if (!pre || addKeyOf(flow) !== key || pre.name_error || pre.refusal) {
    flow.phase = "form";
    render();
    return;
  }
  const measure = (side) =>
    api
      .probeFolder(side, side === "remote" ? remotePathForProbe(flow.remote) : flow.local.trim())
      .catch((error) => ({ error: String(error?.message ?? error) }));
  const [local, remote] = await Promise.all([measure("local"), measure("remote")]);
  if (addFolder !== flow) return;
  flow.phase = "form";
  if (addKeyOf(flow) === key) {
    flow.probes = { local, remote };
    flow.checkedKey = key;
  }
  render();
}

/**
 * `Add folder`: the write, the restart, then the wait. **`name` is captured here, before anything is
 * awaited** (class W): the dialog's field cannot change under a busy dialog, but the name is the one thing
 * every later step is addressed by, and a step that read the field would be one edit away from adding one
 * folder and merging another.
 */
async function commitAddFolder(flow) {
  if (flow.phase !== "form" || addViewOf(flow).primary !== "add") return;
  const name = flow.name.trim();
  flow.phase = "adding";
  flow.error = null;
  render();
  try {
    // The reply is kept: the add settles what an earlier removal could not finish before it looks at the
    // request, and the accounts of that are the person's to read (review of #450, F9).
    flow.settled = settledLinesOf(await api.addPair(addRequestOf(flow), { pair: name }));
  } catch (error) {
    // The engine's refusal, in its words — and the check that passed is no longer a fact about the file.
    flow.phase = "form";
    flow.error = String(error?.message ?? error);
    flow.checkedKey = null;
    render();
    return;
  }
  await refreshConfig();
  flow.phase = "restarting";
  render();
  let ending;
  let reason;
  try {
    const outcome = await api.restartService(true);
    ending = restartEndingOf(outcome);
    reason = String(outcome?.reason ?? outcome?.detail ?? "");
  } catch (error) {
    ending = "undetermined";
    reason = String(error?.message ?? error);
  }
  // LATCHED WHETHER OR NOT THE DIALOG IS STILL THERE: an ending that left the file ahead of the service
  // is a standing fact the Settings bar offers a way out of, and a dialog closed under it must not lose it.
  if (restartUnresolved(ending)) latchRestart(ending, reason);
  if (addFolder !== flow) return;
  flow.ending = ending;
  flow.reason = reason;
  if (restartUnresolved(ending)) {
    flow.phase = "unresolved";
  } else if (ending === "not_running") {
    flow.phase = "added";
  } else {
    flow.phase = "waiting";
    flow.waits = 0;
    flow.seenIssue = store.select.statusesIssued();
  }
  render();
  clearTimeout(pollTimer);
  poll();
}

/**
 * The merge dialog of the folder that was just added, in place of the add dialog: the dialog layer swaps the
 * surface in place, and the add flow is over (the watch of the new folder's first pass is the merge's).
 */
function showFolderMerge(merge) {
  folderMerge = merge;
  addFolder = null;
  dialogOverlay = "folderMerge";
  render();
}

/** `Restart it now`, in the add dialog after a restart that did not work: the Settings bar's own retry. */
async function retryAddFolderRestart() {
  const flow = addFolder;
  if (!flow || flow.phase !== "unresolved") return;
  flow.phase = "restarting";
  render();
  let ending;
  let reason;
  try {
    const outcome = await api.restartService();
    ending = restartEndingOf(outcome);
    reason = String(outcome?.reason ?? outcome?.detail ?? "");
  } catch (error) {
    ending = "undetermined";
    reason = String(error?.message ?? error);
  }
  if (restartUnresolved(ending)) latchRestart(ending, reason);
  else if (settingsSaveOutcome && restartUnresolved(settingsSaveOutcome.ending)) settingsSaveOutcome = null;
  if (addFolder !== flow) return;
  flow.ending = ending;
  flow.reason = reason;
  if (restartUnresolved(ending)) flow.phase = "unresolved";
  else if (ending === "not_running") flow.phase = "added";
  else {
    flow.phase = "waiting";
    flow.waits = 0;
    flow.seenIssue = store.select.statusesIssued();
  }
  render();
  clearTimeout(pollTimer);
  poll();
}

/**
 * Called by every render: while the add dialog waits for the restarted daemon to list the new folder, count
 * the polls that have come and gone, and when the folder is there, select it and start watching its merge.
 */
let addFolderSelecting = false;
function advanceAddFolder() {
  const flow = addFolder;
  if (activeFixture() || !flow || flow.phase !== "waiting" || addFolderSelecting) return;
  // ONE POLL, ONE TICK — not one per render, which also happens on every keystroke elsewhere.
  const issued = store.select.statusesIssued();
  if (issued !== flow.seenIssue) {
    flow.seenIssue = issued;
    flow.waits += 1;
  }
  const name = flow.name.trim();
  const listed = store.select.pairs();
  const step = waitStepOf({
    name,
    listed: listed.map((entry) => entry.name),
    waits: flow.waits,
    limit: WAIT_LIMIT,
  });
  if (step === "wait") return;
  if (step === "timeout") {
    flow.phase = "unresolved";
    flow.ending = "not_listed";
    render();
    return;
  }
  // Read from the NEW folder's own summary, at the moment the daemon first listed it: a pass that
  // finishes after this is the merge, and one that finished before it is not.
  const seq = listed.find((entry) => entry.name === name)?.reconcile_seq ?? 0;
  addFolderSelecting = true;
  showFolder(name)
    .then((selected) => {
      addFolderSelecting = false;
      if (addFolder !== flow) return;
      if (!selected) {
        flow.phase = "unresolved";
        flow.ending = "not_listed";
        render();
        return;
      }
      const merge = { pair: name, seq, waits: 0 };
      // An earlier removal this add finished has an account to read, and the merge dialog has no room for it
      // and ends by itself: the dialog rests on the listed folder until the person has read it (F9).
      if (flow.settled.length > 0) {
        flow.phase = "listed";
        flow.merge = merge;
        render();
        return;
      }
      showFolderMerge(merge);
    })
    .catch(() => {
      addFolderSelecting = false;
    });
}

/**
 * Called by every render: the merge dialog of a folder that was just added ends when ITS first pass has
 * completed. The reply must say it is about that folder — the window may still be describing the one that
 * was selected before, for a poll, and that folder's pass counter is not this one's.
 */
function advanceFolderMerge() {
  const merge = folderMerge;
  if (activeFixture() || !merge || dialogOverlay !== "folderMerge") return;
  const reply = replyAbout(store.select.response(), merge.pair);
  if (!reply) return;
  if (mergeOutcomeOf(reply, merge.seq) === "waiting") return;
  // Done, or failed — and either way the dialog has nothing left to say that the folder's own hero does
  // not: it names the failure, in the daemon's words, beside the Pause that is still there.
  folderMerge = null;
  leaveDialog();
}

// ---- the add dialog: what it draws ----

/** The line said in place of the notices while something is in flight, or after the add — `SETTINGS`' words where they are the same. */
function addFolderSentence(flow) {
  switch (flow.phase) {
    case "adding":
      return SETTINGS.saving;
    case "restarting":
      return SETTINGS.restarting;
    case "waiting":
      return FOLDERS.add.waiting(flow.name.trim());
    case "added":
      return saveNoteFor("not_running");
    case "listed":
      return FOLDERS.add.listed(flow.name.trim());
    case "unresolved":
      return flow.ending === "not_listed"
        ? FOLDERS.add.notListed(flow.name.trim())
        : saveNoteFor(flow.ending, flow.reason);
    default:
      return null;
  }
}

function addFolderDialog() {
  const flow = activeFixture()?.addFolder ?? addFolder;
  if (!flow) return null;
  const view = addViewOf(flow);
  const sentence = addFolderSentence(flow);
  return {
    title: FOLDERS.add.title,
    subtitle: FOLDERS.add.sub,
    signature: addFolderShape({ flow, view }) + String(sentence),
    // The keyboard starts in the name field, not on the ✕ the title row would otherwise give it.
    focus: '[data-field="folder-name"]',
    children: renderAddFolder({
      flow,
      view,
      sentence,
      handlers: {
        onField: editAddFolder,
        onChoose: chooseAddFolder,
        onPrimary: pressAddFolder,
        onCancel: () => closeOverlay(),
        onAddRule: addFolderRule,
        onRemoveRule: removeAddFolderRule,
        onRestart: retryAddFolderRestart,
      },
    }),
  };
}

// ---- the removal dialog ----

function removeFolderDialog() {
  const flow = activeFixture()?.removeFolder ?? removeFolder;
  if (!flow) return null;
  const removal = removalOf({
    name: flow.pair,
    roster: folderRoster(),
    setAsideDir: (activeFixture()?.config ?? viewedConfig())?.set_aside_dir ?? null,
  });
  const account = flow.reply ? removalAccount(flow) : null;
  return {
    title: removal.title,
    signature: removeFolderShape({ flow, removal, account }),
    children: renderRemoveFolder({
      flow,
      removal,
      account,
      handlers: {
        onCancel: () => closeOverlay(),
        onConfirm: confirmRemoveFolder,
        onDone: () => closeOverlay(),
      },
    }),
  };
}

/** The answer's sentences: the command's own account, and a restart that did not work in the Settings save's words. */
function removalAccount(flow) {
  const ending = restartEndingOf(flow.reply.restart);
  const restartNote = restartUnresolved(ending)
    ? saveNoteFor(ending, String(flow.reply.restart?.reason ?? flow.reply.restart?.detail ?? ""))
    : null;
  return accountOf(flow.reply, { name: flow.pair, restartNote });
}

async function confirmRemoveFolder() {
  const flow = removeFolder;
  if (!flow || flow.phase !== "confirm") return;
  // CAPTURED, class W: the folder the confirmation was drawn for, whatever is selected when the daemon answers.
  const pair = flow.pair;
  flow.phase = "removing";
  flow.error = null;
  render();
  let reply;
  try {
    reply = await api.removePair({ pair });
  } catch (error) {
    flow.phase = "confirm";
    flow.error = String(error?.message ?? error);
    render();
    return;
  }
  const ending = restartEndingOf(reply?.restart);
  if (restartUnresolved(ending)) {
    latchRestart(ending, String(reply?.restart?.reason ?? reply?.restart?.detail ?? ""));
  }
  // What was typed for a folder that no longer exists must not wait for one of the same name to be added.
  patchStaging(pair, { edits: {}, drafts: BLANK_STAGING.drafts, scheduleMonthly: null, notice: null });
  if (removeFolder === flow) {
    flow.reply = reply;
    flow.phase = "done";
  }
  // THE SELECTION LEAVES A FOLDER THAT IS GONE BEFORE ANYTHING ASKS ABOUT IT BY NAME (review of #450). Rust
  // does not move it: it holds the choice and falls back to the default folder only when it reads one, and the
  // window's own record is the last reply, which still says the removed folder. `refreshConfig` below asks for
  // the folder the store settled on, so a removal of the selected folder asked for the removed name, was
  // refused (`no folder pair named …`), and left the Settings screen drawing that refusal until the next read.
  // A folder that was not selected moves nothing. `undefined` below is "the folder on screen"; `null` is "name
  // none", which Rust answers for the selection it holds and falls back to the default folder for.
  let readFor;
  if (!activeFixture() && store.select.pairName() === pair) readFor = await leaveRemovedFolder(pair);
  await refreshConfig(readFor);
  clearTimeout(pollTimer);
  poll();
}

/**
 * Move the selection off `pair`, a folder that has just been removed, onto one that can be selected, and say
 * which. That is the first folder the settings file still holds that the daemon also lists: the file's first
 * remaining folder is not enough, because `select_pair` is refused for a folder the daemon does not run (one
 * added and not restarted onto yet) and a refusal leaves the selection on the removed name, which the config
 * read that follows then asked for and was refused. `null` when none can be selected; the caller then names no
 * folder and Rust answers for the default one, so the removed name is never asked for.
 */
async function leaveRemovedFolder(pair) {
  const listed = new Set(store.select.pairs().map((entry) => entry?.name));
  for (const { name } of folderRoster()) {
    if (name === pair || !listed.has(name)) continue;
    if (await showFolder(name)) return name;
  }
  return null;
}

// ---- data ----
/**
 * Read the config file for one pair and file the reply under the pair IT says it describes.
 *
 * `pair` is the pair on screen once a status has said which that is. Before one has, naming none is
 * right and naming the placeholder `default` would be wrong: a daemon whose only pair is called `docs`
 * would be asked about a pair that does not exist. Nothing is named, Rust answers for the selection it
 * holds, and the reply says which pair that was.
 */
async function refreshConfig(pair = store.select.settledPair() ?? undefined) {
  try {
    const info = await api.readConfig({ pair });
    // Filed by what the reply says, then by what was asked, then by the pair on screen: the browser
    // preview's replies carry no `pair`, and a real one always does.
    configByPair = withPair(configByPair, info?.pair ?? pair ?? store.select.pairName(), info);
    configRoster = Array.isArray(info?.pairs) ? info.pairs : [];
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
    // Shape-checked rather than trusted, and a state saved before folders is read as the default
    // folder's own — `restoreState` says how and why.
    return restoreState(JSON.parse(localStorage.getItem(NOTIFIER_KEY) ?? "null"));
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
  // EVERY FOLDER THE WINDOW CAN SEE, from the data the poll already fetched (`notifierViews` says which
  // and why the live roster): nothing is asked of the daemon for the notifier. At one folder this is the
  // one view it always was.
  const { views, roster } = notifierViews(store.select);
  const { event, state, resolved } = decide({
    state: notifierState,
    views,
    roster,
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
 * selector. On the wire that refuses EVERY withheld deletion, so a mixed queue would have this
 * banner answer for a recoverable deletion it never mentioned. Keeping is always the safe
 * direction, but doing more than the button says is not the same as safe.
 *
 * `pair` IS THE FOLDER THE BANNER WAS ABOUT (#102 phase 5e), `null` at one folder. It used to be read
 * off the screen — `visibleDeletions()` is the SELECTED folder's queue — which at two folders keeps the
 * wrong folder's items: the banner says `photos`, the window is showing `documents`, and the press
 * refuses `documents`' deletions and leaves `photos`' for the next pass to carry out. Each item carries
 * the folder it was fetched for (`item.pair`), so the request that keeps it names that folder too.
 */
async function keepPermanentDeletions(pair = null) {
  const items = visibleDeletionsOf(pair).filter((item) => severityOfItem(item) === "permanent");
  if (!items.length) return;
  for (const item of items) {
    const key = itemKey(item);
    if (deletionBusy.has(key)) continue;
    deletionBusy.add(key);
    try {
      if (acknowledged(await api.keep(item.path, true, { pair: item.pair }))) {
        deletionsDecided.set(key, item.fingerprint);
      }
    } catch (error) {
      console.error("keep failed:", error);
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
 *
 * AT TWO FOLDERS OR MORE the event names its folder (`pair`) and the steps are `runBannerAction`'s: the
 * window is selected onto the folder before it navigates, and `retry` and `keep` are addressed to that
 * folder by name. At one the event has none and every case is what it always was.
 */
function onNotificationAction({ kind, action, pair } = {}) {
  runBannerAction(
    { kind, action, pair },
    {
      keep: (folder) => keepPermanentDeletions(folder),
      tryAgain: () => trayActionStatus("tryAgain").catch(reportBannerFailure),
      syncPair: (folder) => syncFolderNow(folder).catch(reportBannerFailure),
      open: () => trayActionStatus("open").catch(reportBannerFailure),
      select: showFolder,
      navigate,
      warn: (message) => console.warn(message),
    },
  ).catch(reportBannerFailure);
}

/** A banner action that threw is logged and nothing else: there is no surface left to put it on. */
function reportBannerFailure(error) {
  console.error("notification-action failed:", error);
}

/**
 * `retry` for a banner about one folder: a `syncnow` addressed to THAT folder (a write: the folder is the
 * banner's, never the selection's). The tray's `Try again now` is not this — at two folders it syncs every
 * unpaused one, which is not what a banner about `photos` says it will do.
 */
function syncFolderNow(pair) {
  const issue = store.beginStatus();
  return api.syncNow({ pair }).then((payload) => store.setStatus(payload, issue, pair));
}

/**
 * Make the window about `pair`, and say whether it is. `select_pair` is the one writer of the selection
 * (Rust validates it, stores it and tells the other webview); this then reads that folder's status itself
 * and files it, so the store is on the folder BEFORE the caller navigates — waiting for the next poll
 * would draw the screen for the folder that was left, and a poll here also runs the shown folder's
 * conflict scan, which can take as long as the tree is big. The ordinary poll is started as well, not
 * waited for. A folder the daemon refuses (it went away since the banner was drawn) answers `false` and
 * leaves the selection as it was.
 */
async function showFolder(pair) {
  if (pair === store.select.pairName()) return true;
  try {
    await api.selectPair(pair);
  } catch (error) {
    console.error("select_pair failed:", error);
    return false;
  }
  const issue = store.beginStatus();
  try {
    store.setStatus(await api.getStatus({ pair }), issue, pair);
  } catch (error) {
    console.error("get_status failed:", error);
  }
  clearTimeout(pollTimer);
  poll();
  return true;
}

/**
 * A tray action, whose reply is a status payload like the poll's — and is published like one.
 *
 * ONE HELPER RATHER THAN FOUR CALL SITES, because the id has to be allocated **before** the request
 * is issued (`store.beginStatus`) and a `.then` chain written inline is exactly where that ordering
 * gets lost. The reply is a real observation and may retire a latch; a stale one may not.
 */
function trayActionStatus(id) {
  const issue = store.beginStatus();
  return api.trayAction(id).then((payload) => store.setStatus(payload, issue));
}

/**
 * This webview's status poll. The WINDOW asks for nothing in particular, which means the selected pair
 * (Rust holds the selection). The TRAY PANEL asks for its own command, `tray_status`, which Rust
 * answers about the DEFAULT pair whatever the window has selected — and, because the reply lists
 * every folder, that is all the panel needs to draw a worst-folder hero and a pause row for each.
 *
 * This used to be `get_status` with `{ pair: <the first name in the roster> }`, and the roster is
 * empty until a reply has listed it: the panel's very first poll named no pair, which Rust reads as
 * the SELECTED one, so with a non-default folder selected the panel drew it for one tick (phase 5a-2's
 * recorded gap). A command that has no pair to name has nothing to get wrong on a first call.
 */
function pollStatus() {
  return isTraySurface() ? api.getTrayStatus() : api.getStatus();
}

/**
 * The pair the screens were last drawn for, so a change of it is noticed once.
 *
 * `null` until a status has said: the first answer is not a SWITCH, and nothing is reset by it.
 */
let viewedPair = null;

/**
 * Called at the top of every render: when the pair on screen is not the one the screens were built
 * for, drop what describes the old one BEFORE anything is drawn from it. A switch is a shape change,
 * not an update (4.6): a plan rehearsed for A, an armed deletion on one of A's rows, A's conflict
 * position and a lookup half-typed against A's index mean nothing for B, and a deletions view
 * patched from A's rows into B's would carry A's `armed` state onto B's nodes.
 *
 * What is reset and what is not is `gui/test/pair-ledger.test.js`'s table, enforced against this
 * function's source: every PER-PAIR-RESET binding must be cleared here (or by a reset it calls).
 */
function noticeSelection() {
  // `settledPair`, not `pairName`: before any status has said, `pairName` is the placeholder
  // `default`, and the first real answer ("docs", for a daemon whose default pair has another name)
  // would read as a switch away from it and reset screens nothing has drawn yet.
  const now = store.select.settledPair();
  if (now === null) return;
  const was = viewedPair;
  viewedPair = now;
  if (was === null || was === now) return;
  resetConflictScreen();
  resetPlanScreen();
  resetActivityScreen();
  pairMenuOpen = false;
  deletionArmed = null;
  deletionBusy.clear();
  deletionStatusInFlight.clear();
  // So the first conflict scan of the pair now shown is immediate, not up to 15 s away.
  lastConflictScan = 0;
}

async function poll() {
  // ALLOCATED BEFORE THE REQUEST GOES OUT, so the answer can be compared against things that
  // happened while it was in flight (#335). See `store.beginStatus`.
  const issue = store.beginStatus();
  try {
    const payload = await pollStatus();
    // Set before setStatus (which synchronously re-renders) so the onboarding-routing gate sees that
    // a real poll has now completed — only then may an `unreachable` reply mean a genuinely fresh
    // machine rather than the pre-poll default.
    statusPolled = true;
    store.setStatus(payload, issue);
  } catch (e) {
    statusPolled = true;
    store.setStatus({ state: "unreachable", error: String(e) }, issue);
  }
  const now = Date.now();
  if (now - lastConflictScan > 15000) {
    lastConflictScan = now;
    // Scanned for the pair that is shown NOW and filed under it, so a scan that lands after the
    // selection has moved is a fact about a pair that is no longer on screen and not a replacement
    // for the one that is (the store keys it by pair; see `store.js`).
    const pair = store.select.pairName();
    // NOT DATED (`setConflicts`): the notifier reads the shown folder from this list as it always did, and the
    // moment it stops being the shown one `refreshOtherPair` scans it again and dates that — a minute at the
    // most, saying nothing about it, is the price of not stamping a second call site nothing depends on.
    try {
      store.setConflicts(await api.scanConflicts({ pair }), pair);
    } catch (_) {
      store.setConflicts([], pair);
    }
    // Re-read the GUI config file on the same slow cadence: onboarding or an external edit may
    // have (re)written it since boot, and it also drives the no-daemon fallback pair display.
    refreshConfig();
  }
  // NOT AWAITED. A folder that is not on screen may be on a mount that never answers, and a poll that waits
  // for its scan never schedules the next one: the shown folder's polls stopped for as long as it hung
  // (review of #447). The other folders refresh beside the poll, one job each, and the poll goes on.
  refreshOtherPairs();
  // The withheld deletions ride on the status reply itself — no second IPC round trip per tick — and
  // `store.setStatus` files them with it, under the pair the reply describes.
  //
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

/** How often the folders that are NOT on screen are scanned for conflicts (brief E6): a scan walks a tree. */
const OTHER_SCAN_MS = 60000;
/** When each other folder was last scanned, by name — a `Map`, because a folder may be called `constructor`. */
const lastOtherScan = pairTable();

/**
 * The folders whose refresh (below) has not finished, by name. A `Set`, which has no `constructor` to answer
 * for a folder called that. One job per folder at a time: a scan that hangs is still the one scan, and the
 * next poll — two seconds later — does not stack another on it, nor a second read of the same queue.
 */
const otherInFlight = new Set();

/**
 * Keep the folders that are NOT on screen far enough up to date to say whether they are asking for a
 * person — the folder selector's marker and counts, and nothing else (#102 phase 5c-1).
 *
 * Cheap on purpose, and the cost is stated (E6). A folder's withheld deletions are fetched only while
 * its summary says it has some, so a quiet folder costs no request at all; its conflicts are a disk scan
 * of its root, run at most once a minute. Each reply is filed under the folder it describes, so none of
 * it can reach the screen that is showing another one. The tray panel does none of this: it is a second
 * webview running this file, and the marker is the window's.
 *
 * FIRE AND FORGET, per folder (`poll` does not wait for it): see the comment at the call.
 */
function refreshOtherPairs() {
  if (activeFixture() || isTraySurface()) return;
  const shown = store.select.pairName();
  for (const summary of store.select.pairs()) {
    const pair = summary.name;
    if (pair === shown || otherInFlight.has(pair)) continue;
    otherInFlight.add(pair);
    refreshOtherPair(pair, summary.pending_deletions > 0).finally(() => otherInFlight.delete(pair));
  }
}

/** One other folder's refresh. Never rejects: every request is caught, so the guard above is always released. */
async function refreshOtherPair(pair, hasQueue) {
  if (hasQueue) {
    const issue = store.beginStatus();
    try {
      // `pair` is passed on so a read that FAILS is filed under it, not under the folder on screen.
      store.setStatus(await api.getStatus({ pair }), issue, pair);
    } catch (error) {
      console.error("get_status failed:", error);
    }
  }
  const now = Date.now();
  if (now - (lastOtherScan.get(pair) ?? 0) > OTHER_SCAN_MS) {
    lastOtherScan.set(pair, now);
    // Read before the request leaves: the scan is evidence about the moment it left, and the store dates it
    // against the roster with this (`setConflicts`) — the notifier speaks from a scan only if it is dated.
    const asOf = store.select.statusesIssued();
    try {
      store.setConflicts(await api.scanConflicts({ pair }), pair, asOf);
    } catch (error) {
      console.error("scan_conflicts failed:", error);
    }
  }
}

function scheduleNextPoll() {
  clearTimeout(pollTimer);
  pollTimer = setTimeout(poll, document.hasFocus() ? 2000 : 10000);
}

// ---- boot ----
function main() {
  initTheme();
  // The tray panel runs this same file and follows the pair its REPLIES describe, not the window's
  // selection (see `pollStatus`).
  if (isTraySurface()) store.configure({ follows: "reply" });
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

  // `select_pair` ran (from this webview or the other): poll at once rather than showing the old pair
  // for up to two seconds. The store follows the reply, and `noticeSelection` does the rest.
  api.onPairSelected(() => {
    clearTimeout(pollTimer);
    poll();
  });
  // A tray row paused or resumed a folder and the daemon could not save it (decision D12). The panel is
  // dismissed before its reply arrives, so the window is the surface that can say so. MAIN WINDOW ONLY,
  // for `onNotificationAction`'s reason: `app.emit` reaches every webview.
  if (!isTraySurface()) {
    api.onPauseUnsaved(({ pair, paused, reason } = {}) =>
      noteUnsavedPause(pair ?? store.select.pairName(), Boolean(paused), reason),
    );
  }
  // The folder selector closes when a press lands outside it or the keyboard leaves it. Plain document
  // listeners, because the popover is part of the header's patched tree and has no node of its own to
  // hang a backdrop on (a backdrop would be a sibling of the header — `selector.js` says why not).
  document.addEventListener("pointerdown", (event) => {
    if (pairMenuOpen && !(event.target instanceof Element && event.target.closest(".pair-select"))) {
      closePairMenu();
    }
  });
  document.addEventListener("focusin", (event) => {
    if (pairMenuOpen && event.target instanceof Element && !event.target.closest(".pair-select")) {
      closePairMenu();
    }
  });

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
