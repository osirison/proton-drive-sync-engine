// Conflicts screen (S3, #84). Own ONLY this file. Rail of conflicted files + side-by-side compare
// (local orange-tinted, Proton cyan-tinted); a line-level diff for text, size+time only for binary
// or large files. Four staged choices applied together via `ctx.api.resolveConflict`; nothing is
// written until Apply. The unresolved set comes from `ctx.select.conflicts()` — the single source.

import { el, relativeTime, dash } from "../components.js";
import { setConflicts } from "../store.js";

// Screen-local state (survives this file's re-renders; the shell has no inputs here to lose focus).
let selectedPath = null; // original path of the selected conflict
const staged = {}; // originalPath -> Resolution ("keep_mine" | "use_proton" | "keep_both" | "decide_later")
const pairCache = {}; // originalPath -> { loading?: true, error?: string, pair?: {original, sidecar} }
let applying = false;
let applyError = null;
let container_ = null;
let ctx_ = null;

const CHOICES = [
  { key: "keep_mine", label: "Keep mine", hint: "delete the sidecar; your local file uploads next pass" },
  { key: "use_proton", label: "Use Proton's", hint: "replace your file with Proton's copy" },
  { key: "keep_both", label: "Keep both", hint: "rename yours to name.local.ext, keep Proton's too" },
  { key: "decide_later", label: "Decide later", hint: "nothing written; still counts as outstanding" },
];
const choiceLabel = (key) => CHOICES.find((c) => c.key === key)?.label ?? key;

const basename = (p) => (p || "").split("/").filter(Boolean).pop() || p || "";
const MAX_DIFF_LINES = 1200; // above this, skip the O(n·m) LCS and show plain panes

export function renderConflicts(container, ctx) {
  container_ = container;
  ctx_ = ctx;
  paint();
}

function selectConflict(path) {
  selectedPath = path;
  loadPair(path);
  paint();
}

async function loadPair(path) {
  if (pairCache[path]?.pair || pairCache[path]?.loading) return;
  const conflict = ctx_.select.conflicts().find((c) => c.original === path);
  if (!conflict) return;
  pairCache[path] = { loading: true };
  paint();
  try {
    pairCache[path] = { pair: await ctx_.api.readConflictPair(conflict) };
  } catch (e) {
    pairCache[path] = { error: e && e.message ? e.message : String(e) };
  }
  paint();
}

function stageChoice(path, choiceKey) {
  staged[path] = choiceKey;
  // Auto-advance to the next not-yet-staged conflict (design: auto-advance after a choice).
  const list = ctx_.select.conflicts();
  const next = list.find((c) => !staged[c.original] && c.original !== path);
  if (next) selectConflict(next.original);
  else paint();
}

async function applyStaged() {
  const list = ctx_.select.conflicts();
  const toApply = list.filter((c) => staged[c.original] && staged[c.original] !== "decide_later");
  if (applying || toApply.length === 0) return;
  applying = true;
  applyError = null;
  paint();
  try {
    for (const conflict of toApply) {
      await ctx_.api.resolveConflict(conflict, staged[conflict.original]);
      delete staged[conflict.original];
      delete pairCache[conflict.original];
    }
    // Refresh the unresolved set from disk; the store emit re-renders this screen.
    setConflicts(await ctx_.api.scanConflicts());
    selectedPath = null;
  } catch (e) {
    applyError = e && e.message ? e.message : String(e);
  } finally {
    applying = false;
    paint();
  }
}

// ---- rendering ----
function paint() {
  if (!container_) return;
  const conflicts = ctx_.select.conflicts();

  if (ctx_.select.daemonState() === "unreachable") {
    container_.replaceChildren(
      el("div", { class: "card" }, el("div", { class: "ledger-empty" }, "Daemon unreachable — can't list conflicts.")),
    );
    return;
  }
  if (conflicts.length === 0) {
    container_.replaceChildren(
      el("div", { class: "card" }, el("div", { class: "ledger-empty" }, "No conflicts. Nothing needs your attention here.")),
    );
    return;
  }

  const layout = el(
    "div",
    { style: "display:flex;gap:var(--gap-card);height:100%;min-height:0" },
    rail(conflicts),
    comparePanel(conflicts),
  );
  container_.replaceChildren(layout);
}

function rail(conflicts) {
  const rows = conflicts.map((c) => {
    const isSel = c.original === selectedPath;
    const choice = staged[c.original];
    return el(
      "button",
      {
        class: "nav-row" + (isSel ? " active" : ""),
        style: "flex-direction:column;align-items:flex-start;gap:2px",
        onClick: () => selectConflict(c.original),
      },
      el("span", { style: "font-size:var(--fs-control);font-weight:600" }, basename(c.original)),
      el("span", { class: "mono", style: "font-size:10.5px;color:var(--muted)" }, c.original),
      choice
        ? el("span", { class: "chip", style: "margin-top:2px" }, choiceLabel(choice))
        : null,
    );
  });
  return el(
    "div",
    {
      style: `width:var(--dim-conflict-rail);flex:none;display:flex;flex-direction:column;min-height:0;border-right:1px solid var(--border);padding-right:10px`,
    },
    el("div", { class: "label", style: "padding:4px 8px" }, `Conflicts (${conflicts.length})`),
    el("div", { class: "scroll-y", style: "display:flex;flex-direction:column;gap:4px;min-height:0" }, rows),
  );
}

function comparePanel(conflicts) {
  const conflict = conflicts.find((c) => c.original === selectedPath);
  const children = [];

  if (!conflict) {
    children.push(el("div", { class: "card" }, el("div", { class: "ledger-empty" }, "Select a conflict to review.")));
    return el("div", { style: "flex:1;min-width:0;display:flex;flex-direction:column;min-height:0" }, ...children);
  }

  // choice buttons
  const current = staged[conflict.original];
  const buttons = CHOICES.map((c) =>
    el(
      "button",
      {
        class: "btn" + (current === c.key ? " primary" : ""),
        style: "font-size:var(--fs-meta)",
        title: c.hint,
        onClick: () => stageChoice(conflict.original, c.key),
      },
      c.label,
    ),
  );
  children.push(
    el(
      "div",
      { style: "display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:10px" },
      el("div", { class: "mono", style: "font-weight:600;flex:1;min-width:0;word-break:break-all" }, conflict.original),
      ...buttons,
    ),
  );
  if (current) {
    children.push(
      el(
        "div",
        { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-bottom:10px" },
        `Staged: ${choiceLabel(current)} — ${CHOICES.find((c) => c.key === current).hint}. Nothing is written until you Apply.`,
      ),
    );
  }

  // compare body
  const cache = pairCache[conflict.original];
  if (!cache || cache.loading) {
    children.push(el("div", { class: "card" }, el("div", { class: "ledger-empty" }, "Loading…")));
  } else if (cache.error) {
    children.push(el("div", { class: "card dir-destructive" }, `Couldn't read files: ${cache.error}`));
  } else {
    children.push(compareBody(cache.pair));
  }

  children.push(applyFooter(conflicts));
  return el("div", { style: "flex:1;min-width:0;display:flex;flex-direction:column;min-height:0" }, ...children);
}

function paneHeader(title, tint, side) {
  return el(
    "div",
    { style: `padding:6px 10px;font-size:var(--fs-meta);font-weight:600;background:${tint};color:${side}` },
    title,
  );
}

function metaLine(s) {
  if (!s.exists) return "does not exist";
  const when = s.mtime_epoch_secs != null ? relativeTime(s.mtime_epoch_secs) : dash(null);
  return `${s.size} bytes · ${when}`;
}

function compareBody(pair) {
  const local = pair.original;
  const remote = pair.sidecar;
  const bothText = local.text != null && remote.text != null;

  const header = el(
    "div",
    { style: "display:grid;grid-template-columns:1fr 1fr;gap:0;border:1px solid var(--border);border-radius:var(--radius-tile) var(--radius-tile) 0 0;overflow:hidden" },
    paneHeader(`Your version · ${metaLine(local)}`, "var(--diff-local)", "var(--upload-text)"),
    paneHeader(`Proton's version · ${metaLine(remote)}`, "var(--diff-remote)", "var(--download-text)"),
  );

  let body;
  if (!bothText) {
    // Binary / large / missing: size + time only, never a fabricated preview.
    body = el(
      "div",
      { style: "display:grid;grid-template-columns:1fr 1fr;border:1px solid var(--border);border-top:none" },
      el("div", { class: "mono", style: "padding:14px;font-size:var(--fs-meta);color:var(--muted);border-right:1px solid var(--border)" },
        local.binary_or_large ? "Binary or large file — no preview." : local.exists ? "(text unavailable)" : "does not exist"),
      el("div", { class: "mono", style: "padding:14px;font-size:var(--fs-meta);color:var(--muted)" },
        remote.binary_or_large ? "Binary or large file — no preview." : remote.exists ? "(text unavailable)" : "does not exist"),
    );
  } else {
    body = diffTable(local.text, remote.text);
  }
  return el("div", { class: "scroll-y", style: "min-height:0;flex:1" }, header, body);
}

/** Longest-common-subsequence line diff → side-by-side rows; changed rows tinted + coloured line
 * numbers. Falls back to a plain paired view for very large files (bounds the O(n·m) table). */
function diffTable(localText, remoteText) {
  const a = localText.replace(/\n$/, "").split("\n");
  const b = remoteText.replace(/\n$/, "").split("\n");
  const rows = a.length > MAX_DIFF_LINES || b.length > MAX_DIFF_LINES ? plainRows(a, b) : lcsRows(a, b);

  const cell = (text, no, side, changed) => {
    const tint = changed ? (side === "left" ? "var(--diff-local)" : "var(--diff-remote)") : "transparent";
    const numColor = changed ? (side === "left" ? "var(--upload-text)" : "var(--download-text)") : "var(--muted-2)";
    return el(
      "div",
      { style: `display:flex;gap:8px;background:${tint};padding:0 8px` },
      el("span", { class: "mono", style: `width:34px;flex:none;text-align:right;color:${numColor};font-size:11px;user-select:none` }, no == null ? "" : String(no)),
      el("span", { class: "mono", style: "font-size:11px;white-space:pre-wrap;word-break:break-word;flex:1" }, text == null ? "" : text),
    );
  };

  const left = el("div", {}, ...rows.map((r) => cell(r.left, r.leftNo, "left", r.changed)));
  const right = el("div", { style: "border-left:1px solid var(--border)" }, ...rows.map((r) => cell(r.right, r.rightNo, "right", r.changed)));
  return el(
    "div",
    { style: "display:grid;grid-template-columns:1fr 1fr;border:1px solid var(--border);border-top:none;border-radius:0 0 var(--radius-tile) var(--radius-tile);overflow:hidden" },
    left,
    right,
  );
}

function lcsRows(a, b) {
  const n = a.length, m = b.length;
  const dp = Array.from({ length: n + 1 }, () => new Int32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const rows = [];
  let i = 0, j = 0, li = 1, ri = 1;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      rows.push({ left: a[i], right: b[j], leftNo: li++, rightNo: ri++, changed: false });
      i++; j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      rows.push({ left: a[i], right: null, leftNo: li++, rightNo: null, changed: true });
      i++;
    } else {
      rows.push({ left: null, right: b[j], leftNo: null, rightNo: ri++, changed: true });
      j++;
    }
  }
  while (i < n) rows.push({ left: a[i], right: null, leftNo: li++, rightNo: null, changed: true }), i++;
  while (j < m) rows.push({ left: null, right: b[j], leftNo: null, rightNo: ri++, changed: true }), j++;
  return rows;
}

function plainRows(a, b) {
  const rows = [];
  const max = Math.max(a.length, b.length);
  for (let k = 0; k < max; k++) {
    rows.push({
      left: k < a.length ? a[k] : null,
      right: k < b.length ? b[k] : null,
      leftNo: k < a.length ? k + 1 : null,
      rightNo: k < b.length ? k + 1 : null,
      changed: a[k] !== b[k],
    });
  }
  return rows;
}

function applyFooter(conflicts) {
  const pendingWrites = conflicts.filter((c) => staged[c.original] && staged[c.original] !== "decide_later").length;
  const nodes = [
    el("span", { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted)" },
      pendingWrites === 0 ? "No changes staged." : `${pendingWrites} file operation(s) staged — none written yet.`),
  ];
  if (applyError) {
    nodes.push(el("span", { class: "dir-destructive mono", style: "font-size:var(--fs-meta)" }, `Apply failed: ${applyError}`));
  }
  nodes.push(
    el(
      "button",
      { class: "btn primary", style: "margin-left:auto", disabled: applying || pendingWrites === 0, onClick: () => applyStaged() },
      applying ? "Applying…" : "Apply",
    ),
  );
  return el(
    "div",
    { style: "display:flex;align-items:center;gap:12px;margin-top:10px;padding-top:10px;border-top:1px solid var(--border-soft)" },
    ...nodes,
  );
}
