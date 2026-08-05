// Plan-preview screen (S4, #85). Own ONLY this file. Run `ctx.api.runDryRun()` → { report,
// requires_delete_gate, files_at_risk }. Render the summary grid + one row per action (destructive
// rows tinted + sorted first). Arm the typed-DELETE gate ONLY when `requires_delete_gate` is true
// (never for a purge-only plan); name the `files_at_risk`. Do not edit screens.js/app.js/others.

import { el, dash, directionForAction } from "../components.js";
import { setStatus } from "../store.js";

// ---- module-level state (this screen's own re-render loop; see header note in the task brief) ----
// `runDryRun()` shells the daemon and can be slow, so it is never called from `renderPlan` itself —
// only from the button handler below. The result is cached here so the shell's 2 s poll (which
// re-invokes `renderPlan` on every store change) redraws the SAME cached report instead of
// re-shelling the daemon on every tick.
let lastContainer = null;
let lastCtx = null;

let loading = false; // a runDryRun() call is in flight
let loadError = null; // last runDryRun() failure (or "no report" from the mock), if any
let payload = null; // last successful { report: { summary, plan }, requires_delete_gate, files_at_risk }

let gateInput = ""; // the typed-DELETE confirmation text
let applying = false; // a syncNow() call is in flight
let applyOutcome = null; // { state, message, isError } after the last Apply, or null

// `report.plan`, destructive-first (stable sort), computed once per successful `runPreview()` —
// NOT inside `paint()`, which re-runs on every DELETE-gate keystroke. Re-sorting the full plan on
// every keypress would be O(n log n) per key for large plans; caching it here keeps `paint()` O(n).
let sortedPlanRows = [];

// Mirrors components.js's private `dirClass` (not exported) so plan rows/tiles use the exact same
// direction → CSS class mapping as the shared ledger (including "conflict" reading as amber, not a
// dedicated colour). `directionForAction` itself — the actual classification logic — is imported
// and reused, not reimplemented; only this trivial class-name lookup is duplicated.
const DIR_CLASS = {
  upload: "dir-upload",
  download: "dir-download",
  destructive: "dir-destructive",
  conflict: "dir-upload",
  neutral: "dir-neutral",
};

const SUMMARY_FIELDS = [
  { field: "total", label: "Total" },
  { field: "uploads", label: "Uploads", action: "upload" },
  { field: "downloads", label: "Downloads", action: "download" },
  { field: "remote_directories_created", label: "Remote dirs created", action: "create_remote_directory" },
  { field: "local_directories_created", label: "Local dirs created", action: "create_local_directory" },
  { field: "local_moves", label: "Local moves", action: "move_local" },
  { field: "remote_moves", label: "Remote moves", action: "move_remote" },
  { field: "auto_links", label: "Auto-links", action: "auto_link" },
  { field: "conflicts", label: "Conflicts", action: "conflict" },
  { field: "type_conflicts", label: "Type conflicts", action: "type_conflict" },
  { field: "remote_deletes", label: "Remote deletes", action: "remote_delete" },
  { field: "local_deletes", label: "Local deletes", action: "local_delete" },
  { field: "purges", label: "Purges", action: "purge" },
  { field: "skipped_unsupported", label: "Skipped", action: "skip_unsupported" },
  // Aggregate counter (destructive_actions = remote_deletes + local_deletes + purges), no single
  // SyncAction of its own — force the red tint directly rather than through directionForAction.
  { field: "destructive_actions", label: "Destructive", forceClass: "dir-destructive" },
];

function isGateArmed() {
  return gateInput.trim().toUpperCase() === "DELETE";
}

function formatFilesAtRisk(files) {
  if (!files || files.length === 0) return "";
  if (files.length <= 3) return files.join(", ");
  return `${files.slice(0, 3).join(", ")}, and ${files.length - 3} more`;
}

// Stable sort: destructive rows (remote_delete / local_delete / purge, per directionForAction)
// float to the top; everything else keeps the daemon's original order. Called once per plan (see
// `sortedPlanRows`), not per render.
function sortPlanRows(plan) {
  return plan
    .map((row, index) => ({ row, index }))
    .sort((a, b) => {
      const da = directionForAction(a.row.action) === "destructive" ? 0 : 1;
      const db = directionForAction(b.row.action) === "destructive" ? 0 : 1;
      return da - db || a.index - b.index;
    })
    .map((entry) => entry.row);
}

// ---- data ----

async function runPreview() {
  const ctx = lastCtx;
  loading = true;
  loadError = null;
  paint();
  try {
    const result = await ctx.api.runDryRun();
    if (!result || !result.report) {
      loadError = ctx.api.isMock()
        ? "Dry-run preview isn't available in browser-preview mode — the mock doesn't implement run_dry_run. Run inside the Tauri app to see a real plan."
        : "The daemon returned no plan report.";
    } else {
      payload = result;
      sortedPlanRows = sortPlanRows(result.report?.plan ?? []);
      // A fresh plan may have a different destructive set than the one just reviewed — require
      // the gate to be typed again rather than carrying over a stale confirmation.
      gateInput = "";
      applyOutcome = null;
    }
  } catch (e) {
    loadError = e && e.message ? e.message : String(e);
  } finally {
    loading = false;
    paint();
  }
}

async function applyPlan() {
  const ctx = lastCtx;
  if (!payload || applying) return;
  const gated = payload.requires_delete_gate === true;
  if (gated && !isGateArmed()) return; // inert until armed — never bypass the typed check
  applying = true;
  applyOutcome = null;
  paint();
  try {
    const result = await ctx.api.syncNow();
    setStatus(result); // publish app-wide so the pill/footer reflect this immediately
    applyOutcome = {
      state: result?.state ?? null,
      message: result?.response?.message ?? null,
      isError: !!result?.error,
      error: result?.error ?? null,
    };
  } catch (e) {
    applyOutcome = {
      state: null,
      message: null,
      isError: true,
      error: e && e.message ? e.message : String(e),
    };
  } finally {
    applying = false;
    // Consumed: applying again (destructive or not) requires a fresh gate arm.
    gateInput = "";
    paint();
  }
}

// ---- rendering ----

function headerSection() {
  const hasPayload = !!payload;
  return el(
    "div",
    { class: "card" },
    el(
      "div",
      { style: "display:flex;align-items:center;gap:14px;flex-wrap:wrap" },
      el(
        "div",
        { style: "flex:1;min-width:0" },
        el("div", { style: "font-size:var(--fs-section);font-weight:600" }, "Sync plan preview"),
        el(
          "div",
          { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:4px" },
          "proton-syncd --dry-run · shells the daemon, index read-only · nothing changes until you apply",
        ),
      ),
      el(
        "button",
        {
          class: `btn ${hasPayload ? "" : "primary"}`,
          disabled: loading,
          onClick: () => runPreview(),
        },
        loading ? "Running…" : hasPayload ? "Re-run preview" : "Run plan preview",
      ),
    ),
  );
}

function loadingCard() {
  return el(
    "div",
    { class: "card", style: "margin-top:var(--gap-card)" },
    el(
      "div",
      { class: "ledger-empty" },
      "Running proton-syncd --dry-run… this shells the daemon and can take a while.",
    ),
  );
}

function errorCard() {
  return el(
    "div",
    {
      class: "card",
      style: "margin-top:var(--gap-card);display:flex;align-items:center;gap:14px;flex-wrap:wrap",
    },
    el("div", { class: "dir-destructive", style: "flex:1;min-width:0;font-size:var(--fs-body)" }, loadError),
    el("button", { class: "btn", disabled: loading, onClick: () => runPreview() }, "Retry"),
  );
}

function emptyPromptCard() {
  return el(
    "div",
    { class: "card", style: "margin-top:var(--gap-card)" },
    el(
      "div",
      { class: "ledger-empty" },
      "Nothing previewed yet. Run a plan preview to see what the next sync would do before it happens.",
    ),
  );
}

function summaryTile(spec, summary) {
  const value = summary ? summary[spec.field] : null;
  const dirClass = spec.forceClass ?? (DIR_CLASS[directionForAction(spec.action)] || "dir-neutral");
  return el(
    "div",
    { class: "stat-tile" },
    el("div", { class: `value ${dirClass}` }, dash(value)),
    el("div", { class: "name" }, spec.label),
    el("div", { class: "field mono" }, spec.field),
  );
}

function summaryGrid(summary) {
  return el(
    "div",
    {
      class: "stat-strip",
      style: "grid-template-columns:repeat(5, minmax(0,1fr));margin-top:var(--gap-card)",
    },
    SUMMARY_FIELDS.map((spec) => summaryTile(spec, summary)),
  );
}

function extraForRow(row) {
  if (row.destination_path) return `→ ${row.destination_path}`;
  if (row.conflict_path) return `→ ${row.conflict_path}`;
  return "";
}

function planRow(row) {
  const dir = directionForAction(row.action);
  const cls = DIR_CLASS[dir] || "dir-neutral";
  const isDestructive = dir === "destructive";
  const extra = extraForRow(row);
  return el(
    "div",
    {
      style:
        "display:flex;gap:14px;align-items:center;padding:8px 12px;border-bottom:1px solid var(--border-soft)" +
        (isDestructive ? ";background:var(--danger-tint)" : ""),
    },
    el("span", { class: `mono ${cls}`, style: "width:112px;flex:none;font-weight:600" }, row.action),
    el(
      "span",
      {
        class: "mono",
        style: "flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap",
      },
      row.path,
      extra ? el("span", { style: "color:var(--muted-2);margin-left:6px" }, extra) : null,
    ),
    el("span", { class: "mono", style: "width:72px;flex:none;color:var(--muted)" }, dash(row.entity_kind)),
    el(
      "span",
      {
        class: "mono",
        style:
          "width:150px;flex:none;color:var(--muted-2);overflow:hidden;text-overflow:ellipsis;white-space:nowrap",
      },
      dash(row.remote_id),
    ),
  );
}

// `sorted` is precomputed by `sortPlanRows` in `runPreview` and cached in `sortedPlanRows` — this
// function only renders, it never re-sorts (see the module-level state note on `sortedPlanRows`).
function planRowsCard(sorted) {
  const header = el(
    "div",
    {
      class: "mono",
      style:
        "display:flex;gap:14px;padding:8px 12px;font-size:var(--fs-meta);color:var(--faint);border-bottom:1px solid var(--border)",
    },
    el("span", { style: "width:112px;flex:none" }, "action"),
    el("span", { style: "flex:1" }, "path"),
    el("span", { style: "width:72px;flex:none" }, "entity"),
    el("span", { style: "width:150px;flex:none" }, "remote_id"),
  );

  const body = el(
    "div",
    { class: "scroll-y", style: "max-height:44vh" },
    sorted.length === 0
      ? el("div", { class: "ledger-empty" }, "Nothing to do — the plan is empty.")
      : sorted.map(planRow),
  );

  return el(
    "div",
    { class: "card", style: "margin-top:var(--gap-card);padding:0;overflow:hidden" },
    header,
    body,
  );
}

function gateInputField(disabled) {
  const input = el("input", {
    type: "text",
    class: "mono",
    value: gateInput,
    placeholder: "type DELETE",
    autocomplete: "off",
    spellcheck: "false",
    disabled,
    style:
      "width:130px;font-size:11.5px;letter-spacing:.08em;text-transform:uppercase;padding:8px 10px;" +
      `border-radius:var(--radius-control);background:var(--bg);color:var(--text-1);outline:none;` +
      `border:1px solid ${isGateArmed() ? "var(--danger-fill)" : "var(--border)"}`,
    onInput: (e) => {
      gateInput = e.target.value;
      paint();
    },
    onKeydown: (e) => {
      if (e.key === "Enter" && isGateArmed() && !applying) applyPlan();
    },
  });
  input.dataset.gateInput = "true";
  return input;
}

function gateBanner(requiresGate, summary, filesAtRisk) {
  if (requiresGate) {
    const named = formatFilesAtRisk(filesAtRisk);
    return el(
      "div",
      {
        style:
          "margin-top:var(--gap-card);display:flex;align-items:center;gap:14px;flex-wrap:wrap;padding:12px 16px;" +
          "border-radius:var(--radius-card);background:var(--danger-tint);border:1px solid var(--danger-fill)",
      },
      el("span", { style: "font-size:15px;color:var(--danger-text)" }, "▲"),
      el(
        "div",
        { style: "flex:1;min-width:0;font-size:12.5px;line-height:1.5;color:var(--danger-text)" },
        // Count is `files_at_risk.length` — the gated remote_delete/local_delete set — NOT
        // `summary.destructive_actions`, which also counts purges (index-only, no user data lost)
        // and would overstate how many files this warning is actually about.
        el("strong", {}, `${filesAtRisk?.length ?? "some"} permanent deletion(s) in this plan. `),
        `Applying will permanently remove: ${named || "the affected file(s)"}.`,
      ),
      gateInputField(applying),
    );
  }
  // Not gated: still say so plainly when the plan contains display-destructive rows (purges) so
  // it's clear why Apply is enabled directly rather than implying nothing destructive happened.
  const purges = summary?.purges ?? 0;
  if (purges > 0) {
    return el(
      "div",
      {
        style:
          "margin-top:var(--gap-card);padding:10px 16px;border-radius:var(--radius-card);" +
          "background:var(--row);border:1px solid var(--border);font-size:12px;color:var(--muted)",
      },
      `This plan only purges ${purges} stale index record(s) — no user data (local files or Proton Drive files) is deleted, ` +
        "so Apply is enabled directly without typing DELETE.",
    );
  }
  return null;
}

function applyFooter(requiresGate, filesAtRisk) {
  const armed = isGateArmed();
  const canApply = !applying && (requiresGate ? armed : true);
  const btnClass = requiresGate && armed ? "btn danger" : requiresGate ? "btn" : "btn primary";

  let hint;
  if (applying) {
    hint = "Applying…";
  } else if (requiresGate && armed) {
    hint = `Armed — this will permanently remove: ${formatFilesAtRisk(filesAtRisk) || "the affected file(s)"}.`;
  } else if (requiresGate) {
    hint = "Type DELETE above to enable Apply.";
  } else {
    hint = "No delete confirmation needed for this plan.";
  }

  return el(
    "div",
    { style: "margin-top:var(--gap-card);display:flex;align-items:center;gap:12px;flex-wrap:wrap" },
    el(
      "span",
      {
        class: `mono ${requiresGate && armed ? "dir-destructive" : ""}`,
        style: "font-size:var(--fs-meta);color:var(--muted)",
      },
      hint,
    ),
    el(
      "button",
      { class: `${btnClass}`, style: "margin-left:auto", disabled: !canApply, onClick: () => applyPlan() },
      applying ? "Applying…" : "Apply",
    ),
  );
}

function applyOutcomeCard() {
  if (!applyOutcome) return null;
  const { state, message, isError, error } = applyOutcome;
  return el(
    "div",
    {
      class: `card ${isError ? "dir-destructive" : ""}`,
      style: "margin-top:var(--gap-card);font-size:var(--fs-body)",
    },
    el(
      "div",
      { style: "font-weight:600" },
      isError ? "Apply failed" : "Apply triggered a fresh reconcile pass",
    ),
    el(
      "div",
      { class: "mono", style: "margin-top:4px;font-size:var(--fs-meta);color:var(--muted)" },
      isError ? error || "unknown error" : `state: ${dash(state)} · message: ${message || "(no message)"}`,
    ),
    !isError
      ? el(
          "div",
          { style: "margin-top:6px;font-size:var(--fs-meta);color:var(--muted)" },
          "The real reconcile ran as a separate pass from the plan you reviewed, so it may have executed a " +
            "slightly different plan. Run the preview again to see the current one.",
        )
      : null,
  );
}

// Rebuilds the screen body into `lastContainer`. This is the ONE place that touches the DOM, so
// every trigger — the button, a keystroke in the gate input, and the shell's 2 s poll re-invoking
// `renderPlan` — goes through it, which is what keeps the typed-DELETE input's focus/cursor intact
// across a poll tick landing mid-keystroke (captured below, restored after the rebuild).
function paint() {
  if (!lastContainer) return;

  const active = document.activeElement;
  const wasGateFocused = !!(active && active.dataset && active.dataset.gateInput === "true");
  const selStart = wasGateFocused ? active.selectionStart : null;
  const selEnd = wasGateFocused ? active.selectionEnd : null;

  const nodes = [headerSection()];

  if (loadError) nodes.push(errorCard());

  if (loading && !payload) {
    nodes.push(loadingCard());
  } else if (!payload) {
    if (!loadError) nodes.push(emptyPromptCard());
  } else {
    const { report, requires_delete_gate, files_at_risk } = payload;
    const summary = report?.summary ?? null;
    const requiresGate = requires_delete_gate === true;

    const banner = gateBanner(requiresGate, summary, files_at_risk);
    if (banner) nodes.push(banner);
    nodes.push(summaryGrid(summary));
    nodes.push(planRowsCard(sortedPlanRows));
    nodes.push(applyFooter(requiresGate, files_at_risk));
    nodes.push(applyOutcomeCard());
  }

  lastContainer.replaceChildren(...nodes);

  if (wasGateFocused) {
    const restored = lastContainer.querySelector('[data-gate-input="true"]');
    if (restored) {
      restored.focus();
      try {
        restored.setSelectionRange(selStart, selEnd);
      } catch (_) {
        /* input may not support selection ranges in some environments — focus alone is fine */
      }
    }
  }
}

export function renderPlan(container, ctx) {
  lastContainer = container;
  lastCtx = ctx;
  paint();
}
