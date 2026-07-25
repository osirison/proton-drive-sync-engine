// Shared components + classifiers (F3). Screens compose these; they never re-implement them.

/** Tiny DOM builder. `props.class`, `on<Event>` handlers, boolean/scalar attrs; children flattened. */
export function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, val] of Object.entries(props || {})) {
    if (key === "class") node.className = val;
    else if (key.startsWith("on") && typeof val === "function")
      node.addEventListener(key.slice(2).toLowerCase(), val);
    else if (val === true) node.setAttribute(key, "");
    else if (val !== false && val != null) node.setAttribute(key, String(val));
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}

const EM_DASH = "—";
export const dash = (v) => (v == null ? EM_DASH : String(v));

/** Map an engine action name (13-variant vocab) to a direction colour class stem. */
export function directionForAction(action) {
  switch (action) {
    case "upload":
    case "create_remote_directory":
      return "upload";
    case "download":
    case "create_local_directory":
      return "download";
    case "remote_delete":
    case "local_delete":
    case "purge":
    case "error":
      return "destructive";
    case "conflict":
    case "type_conflict":
      return "conflict";
    default: // move_local, move_remote, auto_link, skip_unsupported, sync…
      return "neutral";
  }
}

/** Map an action to a filter-chip category. */
export function ledgerCategory(action) {
  switch (action) {
    case "upload":
    case "create_remote_directory":
      return "uploads";
    case "download":
    case "create_local_directory":
      return "downloads";
    case "move_local":
    case "move_remote":
      return "moves";
    case "conflict":
    case "type_conflict":
      return "conflicts";
    case "skip_unsupported":
      return "skipped";
    case "error":
      return "errors";
    default:
      return "other";
  }
}

const dirClass = { upload: "dir-upload", download: "dir-download", destructive: "dir-destructive", conflict: "dir-upload", neutral: "dir-neutral" };

/** The hexagon status widget. `opts`: { pending, uploading, downloading, paused, idle }. */
export function renderHexagon(opts = {}) {
  const { pending = null, uploading = false, downloading = false, paused = false, idle = false } = opts;
  const wrap = el("div", { class: "hexagon" });
  if (paused) wrap.classList.add("is-muted");
  if ((uploading || downloading) && !paused) wrap.classList.add("is-spinning");

  const showArcs = !idle; // idle / unreachable → track only
  // Flat-top hexagon track + two semicircle arcs (amber up, cyan down).
  wrap.innerHTML = `
    <svg viewBox="0 0 96 96" aria-hidden="true">
      <polygon class="arc-track" points="92,48 70,86.1 26,86.1 4,48 26,9.9 70,9.9"></polygon>
      ${showArcs ? '<path class="arc-up" d="M 8 48 A 40 40 0 0 1 88 48"></path>' : ""}
      ${showArcs ? '<path class="arc-down" d="M 88 48 A 40 40 0 0 1 8 48"></path>' : ""}
    </svg>
    <div class="center">${dash(pending)}</div>`;
  return wrap;
}

/** The four stat tiles. `counters` values may be null → em-dash. */
export function renderStatTiles(counters) {
  const tiles = [
    { label: "Pending", field: "pending_changes", value: counters.pending_changes },
    { label: "Conflicts", field: "conflicts", value: counters.conflicts },
    { label: "Destructive", field: "destructive_actions", value: counters.destructive_actions },
    { label: "Skipped", field: "skipped_unsupported", value: counters.skipped_unsupported },
  ];
  return el(
    "div",
    { class: "stat-strip" },
    tiles.map((t) =>
      el(
        "div",
        { class: "stat-tile" },
        el("div", { class: "value" }, dash(t.value)),
        el("div", { class: "name" }, t.label),
        el("div", { class: "field mono" }, t.field),
      ),
    ),
  );
}

const CHIP_ORDER = ["all", "uploads", "downloads", "moves", "conflicts", "skipped", "errors"];

/**
 * The activity ledger. `opts`: { rows: [{action, path, meta}], filter, onFilter, provenance,
 * emptyText }. Chip counts derive from the rows actually present; the body shows the filtered set.
 */
export function renderLedger(opts = {}) {
  const { rows = [], filter = "all", onFilter = () => {}, provenance = "", emptyText = "No activity." } = opts;

  const counts = { all: rows.length };
  for (const key of CHIP_ORDER) if (key !== "all") counts[key] = 0;
  for (const row of rows) {
    const cat = ledgerCategory(row.action);
    if (cat in counts) counts[cat] += 1;
  }

  const header = el(
    "div",
    { class: "ledger-header" },
    CHIP_ORDER.map((key) =>
      el(
        "button",
        { class: `chip${key === filter ? " active" : ""}`, onClick: () => onFilter(key) },
        `${key} ${counts[key] ?? 0}`,
      ),
    ),
    provenance ? el("span", { class: "provenance" }, provenance) : null,
  );

  const visible =
    filter === "all" ? rows : rows.filter((r) => ledgerCategory(r.action) === filter);

  const body = el("div", { class: "ledger-body scroll-y" });
  if (visible.length === 0) {
    body.append(el("div", { class: "ledger-empty" }, emptyText));
  } else {
    for (const row of visible) {
      const dir = directionForAction(row.action);
      body.append(
        el(
          "div",
          { class: "ledger-row" },
          el("span", { class: `action mono ${dirClass[dir] || "dir-neutral"}` }, row.action),
          el("span", { class: "path" }, row.path || ""),
          el("span", { class: "meta" }, row.meta || ""),
        ),
      );
    }
  }

  return el("div", { class: "ledger" }, header, body);
}

/** Relative time from a unix epoch-seconds value. */
export function relativeTime(epochSecs) {
  if (epochSecs == null) return EM_DASH;
  const delta = Math.floor(Date.now() / 1000) - epochSecs;
  if (delta < 60) return `${Math.max(0, delta)}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}
