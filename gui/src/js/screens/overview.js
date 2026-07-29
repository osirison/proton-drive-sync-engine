// Overview screen (S1, #82). Owns ONLY this file. Renders into the container it's given, reading
// live state via `ctx.select` and firing commands via `ctx.actions`. This is a working scaffold
// (proves F1+F2+F3 end-to-end); S1 refines visuals + adds real transfer rows.

import { matrixFor } from "../state-matrix.js";
import { el, renderHexagon, renderStatTiles, renderLedger, relativeTime } from "../components.js";

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

/** `1.4 GiB` for a byte count; null in → null out. */
function humanBytes(n) {
  if (n == null) return null;
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return u === 0 ? `${n} B` : `${v.toFixed(1)} ${units[u]}`;
}

/**
 * One live line for the daemon's `activity` wire field (src/ipc.rs `SyncActivity`): the current
 * phase, file, and byte progress of the in-flight pass. A close cousin of the `proton-sync`
 * CLI's `describe_activity` — same concepts and data, but the casing, separators, and elapsed
 * fragments deliberately follow this screen's own style rather than matching byte-for-byte.
 * Unknown phases from a newer daemon render as their raw token rather than disappearing.
 * Returns null when the daemon reports no activity (older daemon, or the pass is between
 * phases).
 */
function describeActivity(activity) {
  if (!activity) return null;
  const step =
    activity.action_index && activity.action_total
      ? ` · step ${activity.action_index}/${activity.action_total}`
      : "";
  switch (activity.phase) {
    case "scanning-local":
      return (
        "Scanning local files" +
        (activity.files_scanned ? ` — ${activity.files_scanned} seen` : "") +
        (activity.detail ? ` · ${activity.detail}` : "")
      );
    case "listing-remote":
      return (
        "Listing remote folders" +
        (activity.folders_listed ? ` — ${activity.folders_listed} listed` : "") +
        (activity.detail ? ` · ${activity.detail}` : "")
      );
    case "fetching-events":
      return "Checking the remote change feed";
    case "committing":
      return "Committing the sync index";
    case "executing": {
      const t = activity.transfer;
      if (t) {
        const verb = t.direction === "upload" ? "Uploading" : "Downloading";
        let progress = "";
        if (t.bytes_done != null && t.bytes_total) progress = ` — ${humanBytes(t.bytes_done)} / ${humanBytes(t.bytes_total)}`;
        else if (t.bytes_done != null) progress = ` — ${humanBytes(t.bytes_done)} so far`;
        else if (t.bytes_total != null) progress = ` — ${humanBytes(t.bytes_total)}`;
        return `${verb} ${t.path}${progress}${step}`;
      }
      return `${activity.detail || "Applying planned actions"}${step}`;
    }
    default:
      return activity.detail ? `${activity.phase} · ${activity.detail}` : activity.phase;
  }
}

/**
 * Transfer-row data. The first row is the daemon's live `activity` (real filename and, for
 * downloads, real bytes-so-far sampled from the staging directory — see src/ipc.rs
 * `SyncActivity`); the rows after it are the pass's aggregate planned counts from
 * `last_plan_summary`. Both are honest wire data; the bar stays indeterminate because a
 * download's *total* is still unknown (the remote listing carries no size).
 */
function transferRows(planSummary, pending, activity) {
  const rows = [];
  const live = describeActivity(activity);
  if (live) {
    const direction = activity?.transfer?.direction;
    const dir = direction === "upload" ? "up" : direction === "download" ? "down" : "sync";
    rows.push({ dir, label: live });
  }
  const uploads = planSummary?.uploads ?? 0;
  const downloads = planSummary?.downloads ?? 0;
  if (uploads > 0) rows.push({ dir: "up", label: `Uploading ${uploads} file${uploads === 1 ? "" : "s"}` });
  if (downloads > 0) rows.push({ dir: "down", label: `Downloading ${downloads} file${downloads === 1 ? "" : "s"}` });
  if (rows.length === 0 && pending) {
    rows.push({ dir: "sync", label: `Syncing ${pending} pending change${pending === 1 ? "" : "s"}` });
  }
  return rows;
}

/** Duration of the `ps-indeterminate` keyframe above — keep the two in sync. */
const INDETERMINATE_SECONDS = 1.3;

// A `.transfer-row` + 4px `.progress` bar per direction. The fraction transferred is unknowable
// (see `transferRows`), so the bar is deliberately indeterminate — a sliding segment, not a
// specific width% — rather than pretending a completion fraction we don't have. The negative,
// wall-clock-derived animation-delay phase-locks the slide across the shell's poll re-renders:
// without it every rebuild restarted the animation at translateX(-120%), so the segment visibly
// jumped back every ~2 s instead of gliding.
function renderTransferRow(row) {
  const arrow = row.dir === "up" ? "↑" : row.dir === "down" ? "↓" : "↻";
  const fill =
    row.dir === "up" ? "var(--upload-fill)" : row.dir === "down" ? "var(--download-fill)" : "var(--muted-2)";
  const phase = (performance.now() / 1000) % INDETERMINATE_SECONDS;
  return el(
    "div",
    { class: "transfer-row" },
    el("span", { class: "mono", style: "width:14px;flex:none;text-align:center;color:var(--muted-2)" }, arrow),
    el("span", { class: "path" }, row.label),
    el(
      "div",
      { class: "progress", style: "position:relative" },
      el("span", {
        style:
          `position:absolute;inset:0 auto 0 0;width:38%;background:${fill};` +
          `animation:ps-indeterminate ${INDETERMINATE_SECONDS}s ease-in-out infinite;` +
          `animation-delay:-${phase}s`,
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
  else if (st === "running")
    // A download-only pass has no queued local changes, so "0 change(s) pending" would read
    // wrong; the daemon's `syncing` flag is what put us in this state, so say that instead.
    subline = pending ? `${pending} change(s) pending` : "reconciling with Proton Drive";

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
  const transfers = matrix.showTransfers ? transferRows(planSummary, pending, resp?.activity) : [];
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
  // fix as activity.js/history.js (review findings on #108/#109). Healthy entries get the plain
  // "sync" label (neutral colour): status_history has no per-action vocabulary, and showing the
  // engine-internal `auto_link` tag here read as a leftover placeholder.
  const rows = (resp?.status_history ?? [])
    .slice()
    .reverse()
    .map((entry) => ({
      action: entry.last_error != null ? "error" : "sync",
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
