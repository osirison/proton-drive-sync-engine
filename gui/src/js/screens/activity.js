// Activity screen (S2, #83). Own ONLY this file. Build the full-height activity ledger with the
// filter chips promoted to the header, reusing `renderLedger` from ../components.js. Rows come from
// `ctx.select.response().status_history` (and error rows from `last_error`); chip counts derive from
// the visible rows. Do not edit screens.js, app.js, or other screen modules.

import { el, renderLedger, relativeTime } from "../components.js";

// ---- status_history -> ledger row mapping (the ONE spot; replace once the daemon exposes real
// per-action rows — see the engine-gap note in the design handoff). A `last_error` entry becomes
// an `error` row (`error` is not a plan action — it comes from status, per #83's acceptance
// criteria). Everything else has no structured action in status_history yet, so it gets the
// neutral `auto_link` action (no direction colour, no destructive tint) rather than guessing one
// from freeform message text, which would misclassify (e.g. "removed 3 files" reading as a move).
function rowsFromStatusHistory(resp) {
  return (resp?.status_history ?? [])
    .slice()
    .reverse() // status_history is oldest -> newest; ledger reads newest first
    .map((entry) => ({
      action: entry.last_error ? "error" : "auto_link",
      path: entry.last_error || entry.message || "",
      meta: relativeTime(entry.epoch_secs),
    }));
}

export function renderActivity(container, ctx) {
  const { select, actions } = ctx;
  const st = select.daemonState();
  const resp = select.response();

  // Unreachable/first-run get an explicit message, never a blank panel or zeroed chips.
  const emptyText =
    st === "unreachable"
      ? "Daemon unreachable — no activity to show."
      : st === "firstRun"
        ? "Nothing has synced yet."
        : "No recent activity.";

  const ledgerEl = renderLedger({
    rows: rowsFromStatusHistory(resp),
    filter: select.ledgerFilter(),
    onFilter: actions.setLedgerFilter,
    provenance: "from status_history",
    emptyText,
  });

  // Full-height ledger (design §3.2): the ledger BODY owns the scroll, not the whole screen, so
  // the title and filter chips stay pinned. `renderLedger`'s own `.ledger`/`.ledger-body` classes
  // are already flex/scroll-ready (components.css); we only need to give them room to grow, via
  // inline style on the nodes it returned rather than editing that shared CSS.
  ledgerEl.style.flex = "1";
  ledgerEl.style.minHeight = "0";
  const ledgerBody = ledgerEl.querySelector(".ledger-body");
  if (ledgerBody) ledgerBody.style.flex = "1";

  const header = el(
    "div",
    { style: "flex:none" },
    el("div", { style: "font-size:var(--fs-section);font-weight:600" }, "Activity"),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:2px" },
      "recent status history",
    ),
  );

  const ledgerCard = el(
    "div",
    {
      class: "card",
      style: "flex:1;min-height:0;display:flex;flex-direction:column;overflow:hidden",
    },
    ledgerEl,
  );

  const wrap = el(
    "div",
    { style: "display:flex;flex-direction:column;height:100%;min-height:0;gap:var(--gap-card)" },
    header,
    ledgerCard,
  );

  container.replaceChildren(wrap);
}
