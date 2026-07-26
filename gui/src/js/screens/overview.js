// Overview screen (S1, #82). Owns ONLY this file. Renders into the container it's given, reading
// live state via `ctx.select` and firing commands via `ctx.actions`. This is a working scaffold
// (proves F1+F2+F3 end-to-end); S1 refines visuals + adds real transfer rows.

import { matrixFor } from "../state-matrix.js";
import { el, renderHexagon, renderStatTiles, renderLedger, relativeTime, dash } from "../components.js";

export function renderOverview(container, ctx) {
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

  children.push(el("div", { style: "margin-top:16px" }, renderStatTiles(counters)));

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

  children.push(
    el(
      "div",
      { class: "card", style: "margin-top:16px" },
      renderLedger({
        rows,
        filter: select.ledgerFilter(),
        onFilter: actions.setLedgerFilter,
        provenance: "from status_history",
        emptyText,
      }),
    ),
  );

  container.replaceChildren(...children);
}
