// Overview screen (S1, #82). Owns ONLY this file. Renders into the container it's given, reading
// live state via `ctx.select` and firing commands via `ctx.actions`. This is a working scaffold
// (proves F1+F2+F3 end-to-end); S1 refines visuals + adds real transfer rows.

import { matrixFor } from "../state-matrix.js";
import { el, renderHexagon, renderStatTiles, renderLedger, relativeTime, dash } from "../components.js";

// One-time keyframe for the indeterminate transfer bar (see `transferRows` below for why it's
// indeterminate rather than a filled percentage). Scoped to this screen's own render output — not
// `components.css` — so it never leaks and never accumulates across re-renders (it's replaced with
// the rest of the container's children on every `renderOverview` call).
function indeterminateStyleTag() {
  return el(
    "style",
    {},
    "@keyframes ps-indeterminate{0%{transform:translateX(-120%)}100%{transform:translateX(360%)}}",
  );
}

/**
 * Honest transfer-row data. The daemon's own `[i/N]` in-flight counter (src/daemon.rs,
 * `begin_transfer_spinner`) is a stderr-only terminal spinner — it never crosses the IPC socket,
 * so `StatusResponse` (src/ipc.rs) carries no live "how many of this pass are done" field. What IS
 * on the wire: `last_plan_summary` (this pass's planned upload/download counts) and
 * `pending_changes` (the daemon-wide pending count). Render those honestly — direction + count,
 * never a fabricated percentage or filename — one row per non-zero direction, falling back to a
 * single undirected line when no plan has been computed yet this pass.
 */
function transferRows(planSummary, pending) {
  const uploads = planSummary?.uploads ?? 0;
  const downloads = planSummary?.downloads ?? 0;
  const rows = [];
  if (uploads > 0) rows.push({ dir: "up", label: `Uploading ${uploads} file${uploads === 1 ? "" : "s"}` });
  if (downloads > 0) rows.push({ dir: "down", label: `Downloading ${downloads} file${downloads === 1 ? "" : "s"}` });
  if (rows.length === 0 && pending) {
    rows.push({ dir: "sync", label: `Syncing ${pending} pending change${pending === 1 ? "" : "s"}` });
  }
  return rows;
}

// A `.transfer-row` + 4px `.progress` bar per direction. The fraction transferred is unknowable
// (see `transferRows`), so the bar is deliberately indeterminate — a sliding segment, not a
// specific width% — rather than pretending a completion fraction we don't have.
function renderTransferRow(row) {
  const arrow = row.dir === "up" ? "↑" : row.dir === "down" ? "↓" : "↻";
  const fill =
    row.dir === "up" ? "var(--upload-fill)" : row.dir === "down" ? "var(--download-fill)" : "var(--muted-2)";
  return el(
    "div",
    { class: "transfer-row" },
    el("span", { class: "mono", style: "width:14px;flex:none;text-align:center;color:var(--muted-2)" }, arrow),
    el("span", { class: "path" }, row.label),
    el(
      "div",
      { class: "progress", style: "position:relative" },
      el("span", {
        style: `position:absolute;inset:0 auto 0 0;width:38%;background:${fill};animation:ps-indeterminate 1.3s ease-in-out infinite`,
      }),
    ),
  );
}

export function renderOverview(container, ctx) {
  const { select, actions } = ctx;
  const st = select.daemonState();
  const matrix = matrixFor(st);
  const resp = select.response();
  const counters = select.statCounters();
  const pending = select.pendingChanges();
  const conflicts = select.unresolvedConflictCount();
  const planSummary = select.planSummary();

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

  // Transfer rows only while actively reconciling (design §3.1); `showTransfers` is the
  // state-matrix's own flag for this, so it stays correct if the matrix ever grows a second
  // transfer-bearing state.
  const transfers = matrix.showTransfers ? transferRows(planSummary, pending) : [];
  const transferSection = transfers.length
    ? el(
        "div",
        { style: "display:flex;flex-direction:column;gap:7px;margin-top:11px" },
        indeterminateStyleTag(),
        transfers.map(renderTransferRow),
      )
    : null;

  const statusCard = el(
    "div",
    { class: "card status-card", style: "flex:1;min-width:0" },
    hex,
    el(
      "div",
      { style: "flex:1;min-width:0" },
      el("div", { class: "headline" }, matrix.headline),
      el("div", { class: "subline" }, subline),
      transferSection,
    ),
    el("div", { class: "actions" }, actionButtons),
  );

  const needsYouCard =
    !select.countersUnknown() && conflicts > 0
      ? el(
          "div",
          { class: "needs-you", style: "width:210px;flex:none;display:flex;flex-direction:column" },
          el("div", { class: "ny-label" }, "Needs you"),
          el("div", { class: "ny-count" }, String(conflicts)),
          el("div", { class: "subline mono" }, `${conflicts} unresolved conflict(s)`),
          el(
            "button",
            {
              class: "btn primary",
              style: "margin-top:auto;width:100%",
              onClick: () => actions.setTab("conflicts"),
            },
            "Resolve now",
          ),
        )
      : null;

  const topRow = el(
    "div",
    { style: "display:flex;gap:var(--gap-card);align-items:stretch" },
    statusCard,
    needsYouCard,
  );

  const children = [topRow];

  children.push(el("div", { style: "margin-top:var(--gap-card)" }, renderStatTiles(counters)));

  // Ledger rows from status_history (S2 replaces with real per-action rows once available).
  // `last_error` is Option<String>: a *non-null* value is an error row, even if it's "" — same
  // fix as activity.js/history.js (review findings on #108/#109).
  const rows = (resp?.status_history ?? [])
    .slice()
    .reverse()
    .map((entry) => ({
      action: entry.last_error != null ? "error" : "auto_link",
      path: entry.last_error != null ? entry.last_error : (entry.message ?? ""),
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
      { class: "card", style: "margin-top:var(--gap-card)" },
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
