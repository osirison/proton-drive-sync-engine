// Screen registry (F2). Every screen slot + nav entry is pre-declared here so each parallel screen
// task (S1–S11) only fills in its own `render` and never edits the router/nav — the collision-free
// seam. Overview is scaffolded live (proving F1+F2+F3 end-to-end); the rest are placeholders their
// S-task fills in.

import { matrixFor } from "./state-matrix.js";
import { el, renderHexagon, renderStatTiles, renderLedger, relativeTime, dash } from "./components.js";

function placeholder(title, issue) {
  return (container) => {
    container.replaceChildren(
      el(
        "div",
        { class: "screen-placeholder" },
        el("h2", {}, title),
        el("p", {}, "This screen is built by its own task on the shared foundation (F1–F3)."),
        el("div", { class: "issue" }, issue),
      ),
    );
  };
}

// The Overview scaffold — live status card + needs-you + stat tiles + ledger, all via F3 components
// and the single store selectors. S1 (#82) refines the visuals and transfer rows.
function renderOverview(container, ctx) {
  const { select, actions } = ctx;
  const st = select.daemonState();
  const matrix = matrixFor(st);
  const resp = select.response();
  const counters = select.statCounters();
  const pending = select.pendingChanges();
  const conflicts = select.unresolvedConflictCount();

  const idleFlag = !["running", "paused"].includes(st);
  const hex = renderHexagon({
    pending,
    uploading: st === "running",
    downloading: st === "running",
    paused: st === "paused",
    idle: idleFlag,
  });

  let subline = matrix.subline;
  if (st === "idle") subline = `last synced ${relativeTime(resp?.last_sync_epoch_secs)}`;
  else if (st === "running") subline = `${dash(pending)} change(s) pending`;

  const actionButtons = matrix.actions.map((a) =>
    el(
      "div",
      {},
      el(
        "button",
        { class: `btn ${a.kind === "primary" ? "primary" : ""}`, onClick: () => actions.runAction(a) },
        a.label,
      ),
      el("div", { class: "cmd-hint mono" }, a.hint),
    ),
  );

  const statusCard = el(
    "div",
    { class: "card status-card" },
    hex,
    el(
      "div",
      {},
      el("div", { class: "headline" }, matrix.headline),
      el("div", { class: "subline" }, subline),
    ),
    el("div", { class: "actions" }, actionButtons),
  );

  const children = [statusCard];

  if (!select.countersUnknown() && conflicts > 0) {
    children.push(
      el(
        "div",
        { class: "needs-you", style: "margin-top:16px" },
        el("div", { class: "ny-label" }, "Needs you"),
        el("div", { class: "ny-count" }, String(conflicts)),
        el("div", { class: "subline mono" }, `${conflicts} unresolved conflict(s)`),
        el(
          "button",
          { class: "btn primary", style: "margin-top:10px;width:100%", onClick: () => actions.setTab("conflicts") },
          "Resolve now",
        ),
      ),
    );
  }

  const tiles = el("div", { style: "margin-top:16px" }, renderStatTiles(counters));
  children.push(tiles);

  // Ledger rows from status_history (S2 replaces with real per-action rows once available).
  const rows = (resp?.status_history ?? [])
    .slice()
    .reverse()
    .map((entry) => ({
      action: entry.last_error ? "error" : "auto_link",
      path: entry.last_error ? entry.last_error : entry.message,
      meta: relativeTime(entry.epoch_secs),
    }));

  const emptyText =
    st === "unreachable"
      ? "Daemon unreachable — no activity to show."
      : st === "firstRun"
        ? "Nothing has synced yet."
        : "No recent activity.";

  const ledger = el(
    "div",
    { class: "card", style: "margin-top:16px" },
    renderLedger({
      rows,
      filter: select.ledgerFilter(),
      onFilter: actions.setLedgerFilter,
      provenance: "from status_history",
      emptyText,
    }),
  );
  children.push(ledger);

  container.replaceChildren(...children);
}

export const SCREENS = [
  { id: "overview", label: "Overview", icon: "◇", banner: true, issue: "S1 #82", render: renderOverview },
  { id: "activity", label: "Activity", icon: "≡", counter: true, issue: "S2 #83", render: placeholder("Activity", "S2 · #83 — full activity ledger") },
  { id: "conflicts", label: "Conflicts", icon: "⚠", badge: true, issue: "S3 #84", render: placeholder("Conflicts", "S3 · #84 — conflict resolution") },
  { id: "deletions", label: "Deletions", icon: "⊘", badge: false, issue: "S9 #90", render: placeholder("Delete approvals", "S9 · #90 — review withheld deletions (approve/deny)") },
  { id: "plan", label: "Plan preview", icon: "▤", banner: true, issue: "S4 #85", render: placeholder("Plan preview", "S4 · #85 — dry-run review + DELETE gate") },
  { id: "history", label: "History", icon: "◷", issue: "S5 #86", render: placeholder("History", "S5 · #86 — status history (last 20)") },
  { id: "settings", label: "Settings", icon: "⚙", banner: true, issue: "S6 #87", render: placeholder("Settings", "S6 · #87 — config editor + selective sync") },
];
