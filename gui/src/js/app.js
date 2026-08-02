// App shell bootstrap + router (F2). Builds the chrome/sidebar/footer, wires the theme toggle,
// runs the status polling loop (2 s focused / 10 s unfocused, socket error = its own state), and
// mounts the active screen. Screens are pure render(container, ctx) functions from screens.js.

import { api } from "./api.js";
import * as store from "./store.js";
import { SCREENS } from "./screens.js";
import { matrixFor, nextOnboardingLatch } from "./state-matrix.js";
import { el } from "./components.js";
import { renderOnboarding } from "./screens/onboarding.js";

let activeTab = "overview";
let configInfo = null;
let configLoaded = false; // has the GUI config file been read at least once (even if empty)?
let statusPolled = false; // has at least one get_status round trip completed (success or failure)?
let onboardingLatch = false; // sticky: are we in the first-run onboarding takeover? (see state-matrix.js)
let pollTimer = null;
let lastConflictScan = 0;

const dom = {};

function screenById(id) {
  return SCREENS.find((s) => s.id === id) ?? SCREENS[0];
}

// ---- theme ----
function initTheme() {
  const saved = localStorage.getItem("theme");
  if (saved === "light" || saved === "dark") {
    document.documentElement.setAttribute("data-theme", saved);
  }
}
function toggleTheme() {
  const current =
    document.documentElement.getAttribute("data-theme") ||
    (window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark");
  const next = current === "light" ? "dark" : "light";
  document.documentElement.setAttribute("data-theme", next);
  localStorage.setItem("theme", next);
}

// ---- confirmation strip ----
function showConfirm(message, isError = false) {
  dom.confirm.hidden = false;
  dom.confirm.classList.toggle("is-error", isError);
  dom.confirmText.textContent = message;
}
function hideConfirm() {
  dom.confirm.hidden = true;
}

// ---- actions passed to screens ----
const actions = {
  setTab,
  setLedgerFilter: (f) => store.setLedgerFilter(f),
  async runAction(action) {
    const cmd = action.cmd;
    if (cmd === "syncNow" || cmd === "pause" || cmd === "resume") {
      showConfirm(`${action.label}…`);
      try {
        const payload = await api[cmd]();
        store.setStatus(payload);
        showConfirm(payload?.response?.message || payload?.error || "done", !!payload?.error);
      } catch (e) {
        showConfirm(String(e), true);
      }
    } else if (cmd === "previewPlan") {
      setTab("plan");
    } else if (cmd === "chooseFolders") {
      setTab("settings");
    } else if (cmd === "startService") {
      showConfirm("Starting proton-syncd…");
      try {
        const message = await api.startService();
        showConfirm(message || "start requested — waiting for the daemon to come up");
      } catch (e) {
        showConfirm(String(e), true);
      }
    } else if (cmd === "journal") {
      showConfirm("View logs with: journalctl --user -u proton-syncd");
    } else if (cmd === "reauth") {
      showConfirm("Re-authenticate with: proton-drive login");
    }
  },
};

const ctx = { select: store.select, api, actions };

// ---- shell construction ----
function buildShell() {
  const root = document.getElementById("app-root");

  // title bar
  dom.pill = el("span", { class: "state-pill" }, el("span", { class: "dot" }), el("span", { class: "pill-text mono" }, "…"));
  dom.rootPair = el("span", { class: "root-pair" }, "");
  const titlebar = el(
    "header",
    { class: "titlebar" },
    el("img", { class: "app-icon", src: "assets/icon.svg", alt: "" }),
    el("span", { class: "app-title" }, "Proton Drive Sync"),
    dom.rootPair,
    el("span", { class: "spacer" }),
    el("button", { class: "tb-btn theme-toggle mono", onClick: toggleTheme }, "◐ Theme"),
    dom.pill,
  );

  // sidebar nav (pre-declared from SCREENS)
  dom.navRows = {};
  const nav = el("nav", { class: "sidebar" });
  nav.append(el("div", { class: "label" }, "Folder pair"));
  dom.pairCard = el(
    "div",
    { class: "pair-card" },
    el("div", { class: "pair-square" }),
    el("div", {}, el("div", { class: "pair-name" }, "ProtonDrive"), el("div", { class: "pair-path mono" }, "—")),
  );
  nav.append(dom.pairCard);

  for (const s of SCREENS) {
    const count = el("span", { class: "nav-count mono" }, "");
    const badge = el("span", { class: "nav-badge", hidden: true }, "");
    const row = el(
      "button",
      { class: "nav-row", onClick: () => setTab(s.id) },
      el("span", { class: "nav-icon" }, s.icon),
      el("span", {}, s.label),
      s.badge ? badge : count,
    );
    dom.navRows[s.id] = { row, count, badge };
    nav.append(row);
  }

  dom.footerBlock = el("div", { class: "footer-block" });
  nav.append(dom.footerBlock);

  // content
  dom.banner = el(
    "div",
    { class: "safety-banner" },
    el("span", { class: "glyph" }, "▲"),
    el(
      "span",
      {},
      "Deleting on Proton Drive deletes the local file permanently — not to trash — and folders recursively.",
    ),
  );
  dom.confirmText = el("span", {}, "");
  dom.confirm = el(
    "div",
    { class: "confirm-strip", hidden: true },
    dom.confirmText,
    el("button", { class: "dismiss", onClick: hideConfirm }, "×"),
  );
  dom.screen = el("div", { id: "screen" });
  const content = el("main", { class: "content" }, dom.banner, dom.confirm, dom.screen);

  const body = el("div", { class: "body" }, nav, content);

  // footer
  dom.footer = el("footer", { class: "statusbar" });

  root.replaceChildren(titlebar, body, dom.footer);
}

function setTab(id) {
  activeTab = id;
  render();
}

// ---- render (called on every store change + tab switch) ----
function render() {
  const st = store.select.daemonState();
  const matrix = matrixFor(st);

  // state pill
  dom.pill.className = `state-pill is-${st}`;
  dom.pill.querySelector(".pill-text").textContent = matrix.pillMono;

  // nav active + counters/badges
  for (const s of SCREENS) {
    const refs = dom.navRows[s.id];
    refs.row.classList.toggle("active", s.id === activeTab);
    if (s.id === "activity" && refs.count) {
      const n = store.select.response()?.status_history?.length ?? 0;
      refs.count.textContent = n ? String(n) : "";
    }
    if (s.id === "conflicts" && refs.badge) {
      const n = store.select.unresolvedConflictCount();
      refs.badge.hidden = n === 0;
      refs.badge.textContent = String(n);
    }
    if (s.id === "deletions" && refs.badge) {
      const n = store.select.pendingDeletions().length;
      refs.badge.hidden = n === 0;
      refs.badge.textContent = String(n);
    }
  }

  // folder pair: the running daemon's reported roots are ground truth; the GUI config file is the
  // fallback when no daemon is reachable. Em-dashes only when neither knows.
  const live = store.select.response()?.config ?? null;
  const localRoot = live?.local_root ?? configInfo?.local_root ?? null;
  const remoteRoot = live?.remote_root ?? configInfo?.remote_root ?? null;
  const pairText = `${localRoot ?? "—"} ⇄ ${remoteRoot ?? "—"}`;
  const localName = (localRoot || "").split("/").filter(Boolean).pop() || "ProtonDrive";
  dom.pairCard.querySelector(".pair-name").textContent = localName;
  const pairPath = dom.pairCard.querySelector(".pair-path");
  pairPath.textContent = pairText;
  pairPath.title = pairText;
  dom.rootPair.textContent = pairText;
  dom.rootPair.title = pairText;

  // footer — build then drop the null-ish entries; `replaceChildren(null)` would render a literal
  // "null" text node.
  const socketOk = st !== "unreachable";
  const footerItems = [
    el("span", { class: "sb-item" }, `daemon: ${matrix.pillMono}`),
    el("span", { class: "sb-item" }, `cli: ${configInfo?.proton_cli ?? "proton-drive"}`),
    el("span", { class: "sb-item" }, `socket: ${socketOk ? "connected" : "down"}`),
    api.isMock() ? el("span", { class: "lock-holder" }, "preview (mock data)") : null,
  ].filter(Boolean);
  dom.footer.replaceChildren(...footerItems);

  // onboarding is a full-window takeover, not a normal tab (S8, #89). It's a *latched* decision
  // (see nextOnboardingLatch): entered on firstRun OR a genuinely fresh machine (a completed poll
  // reports the daemon unreachable and no folder pair is configured), and held across the mid-flow
  // config write so writing the pair in step 2 doesn't eject the user to the unreachable screen.
  // `localRoot`/`remoteRoot` were resolved above for the folder-pair display (live daemon config
  // first, GUI config file second).
  onboardingLatch = nextOnboardingLatch(
    onboardingLatch,
    st,
    Boolean(localRoot && remoteRoot),
    configLoaded,
    statusPolled,
  );

  // safety banner only on the screens that require it — and never stacked over the onboarding
  // takeover, which shows its own (opposite) "the first sync is a non-destructive merge" banner.
  dom.banner.hidden = !screenById(activeTab).banner || onboardingLatch;

  // mount active screen
  if (onboardingLatch) renderOnboarding(dom.screen, ctx);
  else screenById(activeTab).render(dom.screen, ctx);
}

// ---- data ----
async function refreshConfig() {
  // Loads the GUI config file for the footer + fallback pair display; the actual pair rendering
  // happens in render(), which prefers the live daemon-reported roots over this file.
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
  if (payload?.response) {
    store.setPendingDeletions(payload.response.pending_deletions ?? []);
  }
  scheduleNextPoll();
}

function scheduleNextPoll() {
  clearTimeout(pollTimer);
  const interval = document.hasFocus() ? 2000 : 10000;
  pollTimer = setTimeout(poll, interval);
}

// ---- boot ----
function main() {
  initTheme();
  buildShell();
  store.subscribe(render);
  render();
  refreshConfig();
  poll();
  window.addEventListener("focus", scheduleNextPoll);

  // Tray menu items ("Resolve conflicts", "Settings", "View journal") ask the shell to switch tabs
  // (S7). Routed through the api facade — no direct window.__TAURI__ here.
  api.onTrayNavigate((tab) => {
    if (typeof tab === "string") setTab(tab);
  });
}

main();
