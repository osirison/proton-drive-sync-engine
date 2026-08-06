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
import { ROUTES, FOOTER_ORDER, isOverlay, nextOnboardingLatch } from "./routes.js";
import { el } from "./ui/el.js";
import { renderHeader, renderFooterNav, renderActionBar, screenPlaceholder } from "./ui/chrome.js";

// ---- shell state ----
let route = "main"; // the root or door currently showing
let overlay = null; // the overlay stacked over it, if any
let overlayOpener = null; // the element to return focus to when the overlay closes
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

function openOverlay(id, opener = null) {
  overlay = id;
  overlayOpener = opener ?? document.activeElement;
  render();
}

function closeOverlay() {
  if (!overlay) return false;
  // The takeover is not dismissible: it is entered by the latch and left by the daemon coming up.
  if (ROUTES[overlay]?.takeover) return false;
  overlay = null;
  render();
  // Focus returns to whatever opened it — a dialog that drops focus to <body> makes the keyboard
  // user start the screen again.
  if (overlayOpener?.isConnected) overlayOpener.focus();
  overlayOpener = null;
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

  const active = onboardingLatch ? "onboarding" : (overlay ?? route);
  const spec = ROUTES[active];
  const chip = chipFor();

  const header = renderHeader({
    chip: chip.variant,
    chipText: chip.text,
    // Onboarding drops the ⋯ button, not just the chip — both 9a frames have four header slots.
    onMenu: onboardingLatch
      ? null
      : () => {
          menuOpen = !menuOpen;
          render();
        },
    onHome: route === "main" && !overlay ? null : () => navigate("main"),
  });

  // The main screen has no S-task of its own in the route table, so it gets no issue chip rather
  // than an "F4 · issue" with nothing after it.
  const body = screenPlaceholder(
    spec.label ?? titleFor(active),
    spec.task && spec.issue ? `${spec.task} · issue ${spec.issue}` : null,
  );

  // Either the four doors or an action bar — never both, never neither. The 13-to-6 split is
  // measured, not chosen; see routes.js.
  const footer =
    (spec.footer ?? "doors") === "actionBar"
      ? renderActionBar({
          consequence: "This screen is not built yet.",
          // Onboarding draws 14px 32px 18px: it has no footer nav beneath to carry the margin.
          bottom: spec.takeover ? 18 : 14,
        })
      : renderFooterNav({
          order: FOOTER_ORDER,
          active: ROUTES[route]?.kind === "door" ? route : null,
          labels: Object.fromEntries(FOOTER_ORDER.map((id) => [id, ROUTES[id].label])),
          onNavigate: navigate,
          // The mono line is drawn only on the settled and syncing main screens; every other footer
          // with doors omits it and tightens by 4px.
          variant: route === "main" && !overlay ? "withLine" : "standard",
          line: route === "main" && !overlay ? `${localRoot ?? "—"} ⇄ ${remoteRoot ?? "—"}` : null,
        });

  // `.filter(Boolean)` is load-bearing: `replaceChildren(null)` appends a literal "null" TEXT NODE
  // rather than nothing, so a closed ⋯ menu would print the word in the top-left corner. The v1
  // app.js carried this guard and a comment saying so; dropping it in the rewrite reintroduced the
  // bug, and every class-based assertion still passed — it took looking at a screenshot.
  root.replaceChildren(...[header, renderMenu(), body, footer].filter(Boolean));
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
    if (typeof id === "string" && ROUTES[id]) navigate(id);
  });
}

main();
