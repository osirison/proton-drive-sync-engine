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

// ---- shell state ----
let route = "main"; // the root or door currently showing
let overlay = null; // the overlay stacked over it, if any
let overlayReturn = null; // where to send focus when the overlay closes — see focusKeyOf
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
 * Priority is measured where the frames settle it and chosen where they do not. `2a Needs you` is
 * syncing AND has three decisions waiting, and its chip reads `3 waiting` — so a decision outranks
 * transfer. Nothing draws decisions and deletions at once; deletions win here because the deletion
 * is the one that ends with a file gone. Recorded in DEVIATIONS.md §43.
 *
 * `paused`, `unreachable` and `authExpired` have NO drawn chip anywhere in the prototype. They take
 * the quiet form with their own text rather than an invented colour — the hexagon and the main
 * screen carry those states, which is where the design puts them.
 */
function chipFor() {
  if (onboardingLatch) return { variant: "step", text: "step 1 of 2" };

  const state = store.select.daemonState();
  const decisions = store.select.unresolvedConflictCount();
  const deletions = store.select.pendingDeletions().length;

  if (deletions > 0) return { variant: "deletions", text: `${deletions} waiting` };
  if (decisions > 0) return { variant: "decisions", text: `${decisions} waiting` };
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
  route = route === id ? "main" : id;
  closeOverlay();
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
  const from = opener ?? document.activeElement;
  overlay = id;
  overlayReturn = { key: focusKeyOf(from), node: from };
  render();
}

function closeOverlay() {
  if (!overlay) return false;
  // The takeover is not dismissible: it is entered by the latch and left by the daemon coming up.
  if (ROUTES[overlay]?.takeover) return false;
  overlay = null;
  const back = overlayReturn;
  overlayReturn = null;
  render();
  // Focus returns to whatever opened it — a dialog that drops focus to <body> makes the keyboard
  // user start the screen again. Re-queried after the render, because the node it opened from may
  // have been rebuilt in the meantime.
  const target =
    (back?.key && document.querySelector(back.key)) || (back?.node?.isConnected ? back.node : null);
  target?.focus();
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
  body: null,
  footer: null,
  footerKind: null,
  bodyRoute: null,
  // The dialog layer (F5). Keyed like the body, and for a harder reason: the armed deletion's
  // typed-`DELETE` field CLEARS ON BLUR by design, so a layer rebuilt on the ~2s poll would destroy
  // the field mid-word and make the gate impossible to finish. Same failure class as the focus loss
  // this cache was built for, one level in.
  dialog: null,
  dialogRoute: null,
  dialogDetach: null,
};

function render() {
  const root = document.getElementById("app-root");
  const st = store.select.daemonState();

  // The folder pair: the running daemon's reported roots are ground truth; the GUI config file is
  // the fallback when no daemon is reachable. Em-dashes only when neither knows.
  const live = store.select.response()?.config ?? null;
  const localRoot = live?.local_root ?? configInfo?.local_root ?? null;
  const remoteRoot = live?.remote_root ?? configInfo?.remote_root ?? null;

  // Latched, not a raw read of the daemon state — see routes.js for the whole reason.
  onboardingLatch = nextOnboardingLatch(
    onboardingLatch,
    st,
    Boolean(localRoot && remoteRoot),
    configLoaded,
    statusPolled,
  );

  // A dialog floats over the screen you were on; every other overlay replaces its body. Only the
  // second kind reaches `active`, which is what keeps the screen underneath mounted — and mounted is
  // the whole point, because F4's note on the `details` route asks for exactly that: "clicking it
  // must not lose your place". See routes.js `isDialog` and DEVIATIONS §57.
  const dialogRoute = !onboardingLatch && overlay && isDialog(overlay) ? overlay : null;
  const active = onboardingLatch ? "onboarding" : dialogRoute ? route : (overlay ?? route);
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
    active: ROUTES[route]?.kind === "door" ? route : null,
    // The mono line is drawn on the settled and syncing main screens ONLY. `2a Needs you` is also
    // the main screen and drops it — the attention band has taken the space, and the footer tightens
    // from 22/20 to 20/16 to match. Measured, and the fidelity gate caught the first version of this
    // line assuming every main screen was the same.
    variant: onMain ? (banded ? "banded" : "withLine") : "standard",
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

  // --- body: replaced only when the route changes, so a screen can hold its own nodes across polls
  if (dom.bodyRoute !== active) {
    // The main screen has no S-task of its own in the route table, so it gets no issue chip rather
    // than an "F4 · issue" with nothing after it.
    const built = screenPlaceholder(
      spec.label ?? titleFor(active),
      spec.task && spec.issue ? `${spec.task} · issue ${spec.issue}` : null,
    );
    if (dom.body) dom.body.replaceWith(built);
    else root.append(built);
    dom.body = built;
    dom.bodyRoute = active;
  }

  // --- footer: either the four doors or an action bar — never both, never neither. The 13-to-6
  // split is measured, not chosen; see routes.js.
  const kind = spec.footer ?? "doors";
  const navPatched = kind === "doors" && dom.footerKind === "doors" && updateFooterNav(dom.footer, navOpts);
  if (!navPatched) {
    const built =
      kind === "actionBar"
        ? renderActionBar({
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
      const title = dspec.label ?? titleFor(dialogRoute);
      const built = dialog({
        width: w,
        height: h,
        tone: dspec.tone ?? "plain",
        labelledBy: "dialog-title",
        children: [
          dialogHead({
            title,
            id: "dialog-title",
            size: w >= 600 ? "wide" : "compact",
            // Esc closes it too, through F4's precedence chain. This is the pointer affordance.
            onClose: () => closeOverlay(),
          }),
          screenPlaceholder(title, dspec.task && dspec.issue ? `${dspec.task} · issue ${dspec.issue}` : null),
        ],
      });
      root.append(built);
      dom.dialog = built;
      // Attached after append: the trap focuses on attach, and focus() on a detached node is a
      // silent no-op that leaves the keyboard on whatever opened the dialog.
      dom.dialogDetach = focusTrap(built);
    }
    dom.dialogRoute = dialogRoute;
  }

  // --- the ⋯ menu, the one part that is genuinely torn down and rebuilt. It has no animation and
  // no focus to lose that closing it would not have taken anyway.
  root.querySelector(".menu-popover")?.remove();
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
