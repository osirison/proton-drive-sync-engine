// History screen (S5, #86). Own ONLY this file. Render a reverse-chronological list of
// `ctx.select.response().status_history` (coloured dot, mono time, label, mono summary). State the
// 20-entry / restart-persisted limit and point to `journalctl --user -u proton-syncd` for more.
// Do not edit screens.js, app.js, or other screen modules.

import { el, dash } from "../components.js";

// Matches `STATUS_HISTORY_LIMIT` in src/daemon.rs — the daemon keeps only this many entries,
// persisted across restarts. State it explicitly so the screen never reads as a full audit log.
const HISTORY_LIMIT = 20;

/** HH:MM:SS in the viewer's local time — the mono ~66px time column. */
function formatTime(epochSecs) {
  if (epochSecs == null) return dash(null);
  const d = new Date(epochSecs * 1000);
  const pad = (n) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/**
 * One-line mono summary from a PlanSummary, e.g. "7↑ 2↓ · 2 conflicts". Uploads/downloads always
 * show; other counters only appear when non-zero, so the line stays short and scannable.
 */
function formatSummary(summary) {
  if (!summary) return null;
  const parts = [`${summary.uploads}↑ ${summary.downloads}↓`];
  const moves = (summary.local_moves || 0) + (summary.remote_moves || 0);
  const deletes = (summary.remote_deletes || 0) + (summary.local_deletes || 0) + (summary.purges || 0);
  if (summary.conflicts) parts.push(`${summary.conflicts} conflict${summary.conflicts === 1 ? "" : "s"}`);
  if (moves) parts.push(`${moves} move${moves === 1 ? "" : "s"}`);
  if (summary.auto_links) parts.push(`${summary.auto_links} auto-link${summary.auto_links === 1 ? "" : "s"}`);
  if (deletes) parts.push(`${deletes} delete${deletes === 1 ? "" : "s"}`);
  if (summary.skipped_unsupported) parts.push(`${summary.skipped_unsupported} skipped`);
  return parts.join(" · ");
}

/** The mono summary column: the error text for a failed pass, else this pass's counters. */
function summaryFor(entry) {
  if (entry.last_error) return entry.last_error;
  return formatSummary(entry.successful_sync_summary) ?? formatSummary(entry.plan_summary) ?? dash(null);
}

/** One history row: coloured dot, mono time, label, mono summary. Colour is never the only
 * signal — the label text and summary are always shown regardless of dot colour. */
function renderRow(entry) {
  const isError = entry.last_error != null;
  // Same token pairing the app shell already uses for its status-pill dot (app.css
  // `.state-pill.is-unreachable .dot` / `.is-running .dot`): a solid fill for the dot, the
  // softer `-text` variant for the label so large red text stays readable on dark backgrounds.
  const dotColor = isError ? "var(--danger-fill)" : "var(--download-text)";
  const labelClass = isError ? "dir-destructive" : "dir-download";
  return el(
    "div",
    { class: "ledger-row" },
    el("span", { style: `width:7px;height:7px;border-radius:50%;flex:none;background:${dotColor}` }),
    el("span", { class: "mono", style: "width:66px;flex:none;color:var(--muted)" }, formatTime(entry.epoch_secs)),
    el("span", { class: labelClass, style: "flex:1" }, entry.message || (isError ? "sync failed" : "sync completed")),
    el("span", { class: "meta" }, summaryFor(entry)),
  );
}

export function renderHistory(container, ctx) {
  const { select } = ctx;
  const st = select.daemonState();
  const resp = select.response();
  const entries = (resp?.status_history ?? []).slice().reverse(); // newest first

  const header = el(
    "div",
    { style: "margin-bottom:14px" },
    el("div", { style: "font-size:var(--fs-section);font-weight:600" }, "History"),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:4px" },
      `status_history — the daemon keeps only the last ${HISTORY_LIMIT} entries, persisted across restarts.`,
    ),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted-2);margin-top:2px" },
      "For older history: journalctl --user -u proton-syncd",
    ),
  );

  // Explicit empty/unknown states — never a blank panel (design §3.5, #86 acceptance).
  const emptyText =
    st === "unreachable" ? "Daemon unreachable — no history to show." : "No sync history yet.";

  const card = el("div", { class: "card" });
  if (entries.length === 0) {
    card.append(el("div", { class: "ledger-empty" }, emptyText));
  } else {
    for (const entry of entries) card.append(renderRow(entry));
  }

  container.replaceChildren(header, card);
}
