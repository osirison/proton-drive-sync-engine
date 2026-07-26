// App shell bootstrap + router (F2). Builds the chrome/sidebar/footer, wires the theme toggle,
// runs the status polling loop (2 s focused / 10 s unfocused, socket error = its own state), and
// mounts the active screen. Screens are pure render(container, ctx) functions from screens.js.

import { api } from "./api.js";
import * as store from "./store.js";
import { SCREENS } from "./screens.js";
import { matrixFor } from "./state-matrix.js";
import { el, dash } from "./components.js";
import { renderOnboarding } from "./screens/onboarding.js";

let activeTab = "overview";
let configInfo = null;
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
      showConfirm("Start it with: systemctl --user start proton-syncd");
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

  // safety banner only on the screens that require it
  dom.banner.hidden = !screenById(activeTab).banner;

  // footer
  const socketOk = st !== "unreachable";
  dom.footer.replaceChildren(
    el("span", { class: "sb-item" }, `daemon: ${matrix.pillMono}`),
    el("span", { class: "sb-item" }, `cli: ${configInfo?.proton_cli ?? "proton-drive"}`),
    el("span", { class: "sb-item" }, `socket: ${socketOk ? "connected" : "down"}`),
    api.isMock() ? el("span", { class: "lock-holder" }, "preview (mock data)") : null,
  );

  // mount active screen — first-run is a full takeover, not a normal tab (S8, #89)
  if (st === "firstRun") renderOnboarding(dom.screen, ctx);
  else screenById(activeTab).render(dom.screen, ctx);
}

// ---- data ----
async function refreshConfig() {
  try {
    configInfo = await api.readConfig();
    const localName = (configInfo.local_root || "").split("/").filter(Boolean).pop() || "ProtonDrive";
    dom.pairCard.querySelector(".pair-name").textContent = localName;
    dom.pairCard.querySelector(".pair-path").textContent =
      `${configInfo.local_root ?? "—"} ⇄ ${configInfo.remote_root ?? "—"}`;
    dom.rootPair.textContent = `${configInfo.local_root ?? "—"} ⇄ ${configInfo.remote_root ?? "—"}`;
  } catch (_) {
    /* config not readable yet — leave placeholders */
  }
}

async function poll() {
  try {
    store.setStatus(await api.getStatus());
  } catch (e) {
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
  }
  try {
    store.setPendingDeletions(await api.listPendingDeletions());
  } catch (_) {
    /* keep last */
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
}

main();
