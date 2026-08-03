// Onboarding flow (S8, #89). First-run FULL-WINDOW TAKEOVER — not a normal SCREENS entry. Invoked
// directly by app.js's one hook when `store.select.daemonState() === "firstRun"`, in place of
// whatever tab is active. Own ONLY this file (+ the one surgical hook in app.js). Same pattern as
// settings.js/plan.js: module-level step/loading/error state + a local `paint()` that rebuilds into
// `container` — the shell's 2 s poll re-invokes `renderOnboarding` on every store change, so this
// module's own state (not the store) is authoritative for where the user is in the 4-step flow.
//
// The 4 steps (design §3.8, options 1f/1g): 1 verify the proton-drive CLI, 2 choose the folder
// pair, 3 review the dry-run plan, 4 start the service. Safety-critical copy requirement: say
// plainly that the FIRST pass is a non-destructive merge, while still requiring an explicit
// acknowledgement that deletions later propagate BOTH ways — nothing reaches "start service"
// without that checkbox AND a completed plan review (see `isReviewed`/step 3 gate below).

import { el, dash, directionForAction } from "../components.js";
import { setStatus } from "../store.js";

// ---- module-level state (survives the shell's re-render loop; see header note) -----------------
let step = 1; // 1 CLI check, 2 folders, 3 review plan, 4 start service

let lastContainer = null;
let lastCtx = null;

// Step 1: CLI presence/auth check.
// status: "idle" | "checking" | "ok" | "authExpired" | "unknown" ("unknown" = couldn't positively
// verify — the user is told so honestly and allowed to continue with a warning, per the task brief).
let cli = { status: "idle", message: null };

// Step 2: folder-pair draft (loaded from read_config, written back on Next).
let cfgLoaded = false;
let cfgLoading = false;
let cfgLoadError = null;
let cfgMeta = null; // { path, exists }, display-only
let draft = { local_root: "", remote_root: "" };
let saving = false;
let saveError = null;

// Step 3: dry-run review. status: "idle" | "running" | "done".
let dryRun = { status: "idle", payload: null, error: null, unavailable: false, note: null };
let ack = false; // the explicit "deletions propagate both ways" acknowledgement checkbox

// Step 4: start service.
let recheck = { status: "idle", state: null, error: null };

function messageOf(e) {
  return e && e.message ? e.message : String(e);
}

// A completed, non-errored dry-run attempt — success OR an honest "unavailable" note both count as
// "reviewed" (the user saw what there was to see); a real failure does not, so it can't be used to
// unlock the acknowledgement checkbox.
function isReviewed() {
  return dryRun.status === "done" && !dryRun.error;
}

// ---- step 1: CLI check ---------------------------------------------------------------------

async function runCliCheck() {
  const ctx = lastCtx;
  cli = { status: "checking", message: null };
  paint();

  let status = null;
  try {
    status = await ctx.api.getStatus();
  } catch (_) {
    status = null;
  }
  const state = status?.state ?? null;

  if (state === "authExpired") {
    cli = {
      status: "authExpired",
      message: status?.response?.last_error || status?.error || "The proton-drive sign-in looks expired.",
    };
    paint();
    return;
  }
  if (state && state !== "unreachable") {
    cli = { status: "ok", message: "Daemon reachable and responding — no Proton sign-in errors reported." };
    paint();
    return;
  }

  // Daemon unreachable (or the mock, which has no real daemon or CLI at all): a probe here would
  // either be meaningless (mock) or need a real CLI/session to mean anything, so say so honestly
  // rather than fabricate a "verified" result.
  if (ctx.api.isMock()) {
    cli = {
      status: "unknown",
      message:
        "Browser-preview mode has no real daemon or CLI to check. You can continue here to see the " +
        "flow, but run inside the Tauri app for a real verification.",
    };
    paint();
    return;
  }

  // The daemon's control socket isn't reachable, but `list_remote` shells `proton-drive` directly
  // (bypassing the daemon) — a real, independent check of whether the CLI itself is present and
  // authenticated even before anything is running.
  try {
    await ctx.api.listRemote();
    cli = {
      status: "ok",
      message:
        "The sync daemon isn't running yet, but the proton-drive CLI itself responded — it looks " +
        "present and authenticated.",
    };
  } catch (e) {
    cli = {
      status: "unknown",
      message:
        `Couldn't confirm the CLI is authenticated (${messageOf(e)}). You can continue, but ` +
        'run "proton-drive login" first if the next steps fail.',
    };
  }
  paint();
}

function step1Glyph(status) {
  if (status === "ok") return { icon: "✓", color: "var(--download-text)" };
  if (status === "authExpired") return { icon: "✕", color: "var(--danger-text)" };
  if (status === "unknown") return { icon: "▲", color: "var(--warn-text)" };
  return { icon: "…", color: "var(--muted)" };
}

function step1Headline(status) {
  if (status === "ok") return "The CLI looks good";
  if (status === "authExpired") return "Proton sign-in expired";
  if (status === "unknown") return "Couldn't fully verify";
  return "Checking the proton-drive CLI…";
}

function renderStep1() {
  const { icon, color } = step1Glyph(cli.status);
  const checking = cli.status === "checking";

  const card = el(
    "div",
    { class: "card status-card" },
    el(
      "div",
      { style: `font-size:28px;line-height:1;width:40px;text-align:center;flex:none;color:${color}` },
      icon,
    ),
    el(
      "div",
      { style: "flex:1;min-width:0" },
      el("div", { class: "headline" }, step1Headline(cli.status)),
      el("div", { class: "mono subline" }, cli.message || "Calling get_status()…"),
    ),
    el(
      "div",
      { class: "actions" },
      el(
        "button",
        { class: "btn", disabled: checking, onClick: () => runCliCheck() },
        checking ? "Checking…" : "Re-check",
      ),
      cli.status === "authExpired" ? el("div", { class: "cmd-hint mono" }, "proton-drive login") : null,
    ),
  );

  const canProceed = cli.status === "ok" || cli.status === "unknown";
  const hint =
    cli.status === "unknown"
      ? "Continuing without a confirmed check — you'll see the real picture once the dry-run plan runs in step 3."
      : cli.status === "authExpired"
        ? 'Sign in with "proton-drive login", then re-check.'
        : "";

  return [
    card,
    footerNav({
      backDisabled: true,
      hint,
      nextLabel: "Next",
      nextDisabled: !canProceed,
      onNext: () => {
        step = 2;
        if (!cfgLoaded && !cfgLoading && !cfgLoadError) startLoad();
        else paint();
      },
    }),
  ];
}

// ---- step 2: folder pair ---------------------------------------------------------------------

const inputStyle =
  "width:100%;box-sizing:border-box;padding:8px 10px;border-radius:var(--radius-control);" +
  "border:1px solid var(--border);background:var(--row);color:var(--text-1);" +
  "font-family:var(--font-mono);font-size:var(--fs-control)";

function startLoad() {
  const ctx = lastCtx;
  cfgLoading = true;
  cfgLoadError = null;
  paint();
  ctx.api
    .readConfig()
    .then((cfg) => {
      cfgMeta = { path: cfg.path, exists: cfg.exists };
      draft = { local_root: cfg.local_root || "", remote_root: cfg.remote_root || "" };
      cfgLoaded = true;
      cfgLoading = false;
      paint();
    })
    .catch((e) => {
      cfgLoadError = messageOf(e);
      cfgLoading = false;
      paint();
    });
}

function canProceedFolders() {
  return draft.local_root.trim() !== "" && draft.remote_root.trim() !== "";
}

// DOM refs captured during the current step-2 `renderStep2()`, used to patch the Next button and
// the required-fields hint directly on every keystroke — see `folderField`'s `onInput`. Without
// this, typing would leave both stuck at their initial (disabled/shown) state: `renderOnboarding`
// deliberately skips the shell's poll-triggered full repaint while an `<input>` here has focus (so
// the cursor doesn't get yanked mid-keystroke), and nothing else would ever re-run `canProceedFolders()`.
let folderRefs = {};

function refreshFolderDerived() {
  const ok = canProceedFolders();
  if (folderRefs.nextBtn) folderRefs.nextBtn.disabled = !ok || saving;
  if (folderRefs.hint) folderRefs.hint.textContent = ok ? "" : "Both fields are required.";
}

async function goToReview() {
  if (!canProceedFolders() || saving) return;
  const ctx = lastCtx;
  saving = true;
  saveError = null;
  paint();
  try {
    await ctx.api.writeConfig({ local_root: draft.local_root.trim(), remote_root: draft.remote_root.trim() });
    if (cfgMeta) cfgMeta.exists = true;
    saving = false;
    step = 3;
    // A fresh folder pair invalidates any previously reviewed plan — require a fresh review and a
    // fresh acknowledgement rather than carrying over a stale one (mirrors plan.js's own DELETE-gate
    // reset when a fresh plan comes in).
    dryRun = { status: "idle", payload: null, error: null, unavailable: false, note: null };
    ack = false;
    paint();
    runDryRunCheck();
  } catch (e) {
    saving = false;
    saveError = messageOf(e);
    paint();
  }
}

function folderField(label, hint, key) {
  return el(
    "div",
    { style: "margin-bottom:12px" },
    el(
      "label",
      {
        class: "mono",
        style:
          "display:block;font-size:var(--fs-label);text-transform:uppercase;" +
          "letter-spacing:var(--tracking-label);color:var(--muted);margin-bottom:4px",
      },
      label,
    ),
    el("input", {
      type: "text",
      value: draft[key],
      style: inputStyle,
      onInput: (e) => {
        draft[key] = e.target.value;
        refreshFolderDerived();
      },
    }),
    hint
      ? el(
          "div",
          { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted-2);margin-top:4px" },
          hint,
        )
      : null,
  );
}

function renderStep2() {
  folderRefs = {};
  if (!cfgLoaded) {
    const body = cfgLoadError
      ? el(
          "div",
          { class: "card dir-destructive" },
          el("div", {}, `Couldn't load configuration: ${cfgLoadError}`),
          el("button", { class: "btn", style: "margin-top:10px", onClick: () => startLoad() }, "Retry"),
        )
      : el("div", { class: "card ledger-empty" }, "Loading configuration…");
    return [
      body,
      footerNav({
        onBack: () => {
          step = 1;
          paint();
        },
        nextDisabled: true,
      }),
    ];
  }

  const card = el(
    "div",
    { class: "card" },
    el(
      "div",
      { style: "font-size:var(--fs-section);font-weight:600;margin-bottom:6px" },
      "Choose the folder pair",
    ),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-bottom:14px" },
      "The local folder and the Proton Drive folder that stay in sync. You can change these later on the Settings screen.",
    ),
    folderField("Local root", "A folder on this computer, e.g. ~/ProtonDrive", "local_root"),
    folderField("Remote root", "Path on Proton Drive, e.g. /Drive/RemoteFolder", "remote_root"),
    saveError
      ? el(
          "div",
          { class: "dir-destructive", style: "font-weight:600;margin-top:4px" },
          `Couldn't save: ${saveError}`,
        )
      : null,
  );

  // Built inline (not via the shared `footerNav`) so the Next button and hint can be captured into
  // `folderRefs` and patched directly by `refreshFolderDerived` on every keystroke, instead of only
  // updating on the next full repaint (see the comment on `folderRefs`).
  const hintSpan = el(
    "span",
    { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-right:auto" },
    canProceedFolders() ? "" : "Both fields are required.",
  );
  folderRefs.hint = hintSpan;
  const nextBtn = el(
    "button",
    { class: "btn primary", disabled: !canProceedFolders() || saving, onClick: () => goToReview() },
    saving ? "Saving…" : "Next",
  );
  folderRefs.nextBtn = nextBtn;
  const footer = el(
    "div",
    { class: "card", style: "display:flex;align-items:center;gap:12px" },
    hintSpan,
    el(
      "button",
      {
        class: "btn",
        onClick: () => {
          step = 1;
          paint();
        },
      },
      "Back",
    ),
    nextBtn,
  );

  return [card, footer];
}

// ---- step 3: review the dry-run plan -----------------------------------------------------------

async function runDryRunCheck() {
  const ctx = lastCtx;
  dryRun = { status: "running", payload: dryRun.payload, error: null, unavailable: false, note: null };
  paint();
  try {
    const result = await ctx.api.runDryRun();
    if (!result || !result.report) {
      dryRun = {
        status: "done",
        payload: null,
        error: null,
        unavailable: true,
        note: ctx.api.isMock()
          ? "Dry-run preview isn't available in browser-preview mode — the mock doesn't implement run_dry_run. Run inside the Tauri app to see a real plan."
          : "The daemon returned no plan report.",
      };
    } else {
      dryRun = { status: "done", payload: result, error: null, unavailable: false, note: null };
    }
  } catch (e) {
    dryRun = { status: "done", payload: null, error: messageOf(e), unavailable: false, note: null };
  }
  ack = false; // a fresh review always requires a fresh acknowledgement
  paint();
}

const SUMMARY_TILES = [
  { field: "uploads", label: "Uploads", cls: "dir-upload" },
  { field: "downloads", label: "Downloads", cls: "dir-download" },
  { field: "conflicts", label: "Conflicts", cls: "dir-neutral" },
  { field: "skipped_unsupported", label: "Skipped", cls: "dir-neutral" },
  { field: "destructive_actions", label: "Destructive", cls: "dir-destructive" },
];

function summaryTiles(summary) {
  return el(
    "div",
    { class: "stat-strip", style: "grid-template-columns:repeat(5, minmax(0,1fr))" },
    SUMMARY_TILES.map((spec) =>
      el(
        "div",
        { class: "stat-tile" },
        el("div", { class: `value ${spec.cls}` }, dash(summary ? summary[spec.field] : null)),
        el("div", { class: "name" }, spec.label),
        el("div", { class: "field mono" }, spec.field),
      ),
    ),
  );
}

// Mirrors plan.js's own DIR_CLASS mapping (components.js's `dirClass` isn't exported): "conflict"
// reads as amber (dir-upload), same as everywhere else the ledger renders a direction color.
const DIR_CLASS = {
  upload: "dir-upload",
  download: "dir-download",
  destructive: "dir-destructive",
  conflict: "dir-upload",
  neutral: "dir-neutral",
};

function planRow(row) {
  const dir = directionForAction(row.action);
  const cls = DIR_CLASS[dir] || "dir-neutral";
  const extra = row.destination_path
    ? `→ ${row.destination_path}`
    : row.conflict_path
      ? `→ ${row.conflict_path}`
      : "";
  return el(
    "div",
    {
      style:
        "display:flex;gap:14px;align-items:center;padding:7px 12px;border-bottom:1px solid var(--border-soft)",
    },
    el("span", { class: `mono ${cls}`, style: "width:104px;flex:none;font-weight:600" }, row.action),
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
  );
}

function riskBanner(payload) {
  const filesAtRisk = payload?.files_at_risk ?? [];
  if (!payload?.requires_delete_gate && filesAtRisk.length === 0) return null;
  const named =
    filesAtRisk.length <= 3
      ? filesAtRisk.join(", ")
      : `${filesAtRisk.slice(0, 3).join(", ")}, and ${filesAtRisk.length - 3} more`;
  return el(
    "div",
    { class: "card dir-destructive", style: "font-weight:600" },
    `Unexpected on a first pass: this plan includes ${filesAtRisk.length || "some"} deletion(s)${named ? ` — ${named}` : ""}. ` +
      "Review carefully before continuing; the first sync is supposed to only merge, never delete.",
  );
}

function ackRow() {
  const enabled = isReviewed();
  return el(
    "label",
    { style: "display:flex;align-items:flex-start;gap:10px;cursor:pointer" },
    el("input", {
      type: "checkbox",
      checked: ack,
      disabled: !enabled,
      style: "margin-top:3px;flex:none",
      onChange: (e) => {
        ack = e.target.checked;
        paint();
      },
    }),
    el(
      "div",
      {},
      el(
        "div",
        { class: "dir-destructive", style: "font-size:var(--fs-body);font-weight:600" },
        "I understand that once syncing is running, deletions propagate in both directions — deleting a file on either side deletes it on the other.",
      ),
      !enabled
        ? el(
            "div",
            { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:4px" },
            "Run the dry-run review below first.",
          )
        : null,
    ),
  );
}

function renderStep3() {
  const nodes = [
    el(
      "div",
      { class: "card" },
      el(
        "div",
        { style: "font-size:var(--fs-section);font-weight:600;margin-bottom:6px" },
        "Review what the first sync will do",
      ),
      el(
        "div",
        { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted)" },
        "proton-syncd --dry-run · shells the daemon, index read-only · nothing changes until you start syncing",
      ),
    ),
  ];

  if (dryRun.status === "running" && !dryRun.payload) {
    nodes.push(
      el(
        "div",
        { class: "card ledger-empty" },
        "Running proton-syncd --dry-run… this shells the daemon and can take a while.",
      ),
    );
  } else if (dryRun.error) {
    nodes.push(
      el(
        "div",
        { class: "card dir-destructive", style: "display:flex;align-items:center;gap:14px;flex-wrap:wrap" },
        el("div", { style: "flex:1;min-width:0" }, dryRun.error),
        el("button", { class: "btn", onClick: () => runDryRunCheck() }, "Retry"),
      ),
    );
  } else if (dryRun.unavailable) {
    nodes.push(
      el(
        "div",
        { class: "card" },
        el("div", { class: "ledger-empty" }, dryRun.note),
        el(
          "button",
          { class: "btn", style: "margin-top:8px", onClick: () => runDryRunCheck() },
          "Re-run dry run",
        ),
      ),
    );
  } else if (dryRun.payload) {
    const { report, files_at_risk, requires_delete_gate } = dryRun.payload;
    const summary = report?.summary ?? null;
    const plan = report?.plan ?? [];
    const risk = riskBanner({ files_at_risk, requires_delete_gate });
    if (risk) nodes.push(risk);
    nodes.push(summaryTiles(summary));
    nodes.push(
      el(
        "div",
        { class: "card", style: "padding:0;overflow:hidden" },
        el(
          "div",
          {
            class: "mono",
            style:
              "padding:8px 12px;font-size:var(--fs-meta);color:var(--faint);border-bottom:1px solid var(--border);display:flex",
          },
          el("span", {}, `${plan.length} planned action(s)`),
        ),
        el(
          "div",
          { class: "scroll-y", style: "max-height:32vh" },
          plan.length === 0
            ? el("div", { class: "ledger-empty" }, "Nothing to do — the plan is empty.")
            : plan.map(planRow),
        ),
        el(
          "div",
          { style: "padding:8px 12px;display:flex" },
          el("button", { class: "btn", onClick: () => runDryRunCheck() }, "Re-run dry run"),
        ),
      ),
    );
  } else {
    nodes.push(el("div", { class: "card ledger-empty" }, "Preparing the dry-run preview…"));
  }

  nodes.push(el("div", { class: "card" }, ackRow()));

  const canStart = isReviewed() && ack;
  const hint = !isReviewed()
    ? "Waiting on the dry-run review."
    : !ack
      ? "Check the acknowledgement above to continue."
      : "";

  nodes.push(
    footerNav({
      onBack: () => {
        step = 2;
        paint();
      },
      hint,
      nextLabel: "Continue to start service",
      nextDisabled: !canStart,
      // Disabled buttons don't fire clicks, but re-check the gate here too — belt-and-suspenders on
      // the one safety-critical transition, matching plan.js's own `applyPlan` convention of never
      // trusting the disabled attribute alone for a destructive/gated action.
      onNext: () => {
        if (!isReviewed() || !ack) return;
        step = 4;
        paint();
      },
    }),
  );

  return nodes;
}

// ---- step 4: start service ---------------------------------------------------------------------

async function recheckNow() {
  const ctx = lastCtx;
  recheck = { status: "checking", state: null, error: null };
  paint();
  try {
    const result = await ctx.api.getStatus();
    // Publish app-wide so the pill/footer/nav reflect it immediately, and — the actual point of this
    // step — so the shell hands off out of onboarding the moment the daemon state moves past
    // "firstRun", without waiting for the next background poll tick.
    setStatus(result);
    recheck = { status: "done", state: result?.state ?? null, error: null };
  } catch (e) {
    recheck = { status: "done", state: null, error: messageOf(e) };
  }
  paint();
}

function renderStep4() {
  const card = el(
    "div",
    { class: "card" },
    el(
      "div",
      { style: "font-size:var(--fs-section);font-weight:600;margin-bottom:6px" },
      "Start the service",
    ),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-bottom:12px" },
      "Nothing has synced yet. Start the daemon to run the first pass you just reviewed:",
    ),
    el(
      "div",
      {
        class: "mono",
        style:
          "font-size:var(--fs-body);padding:10px 12px;background:var(--row);border-radius:var(--radius-control)",
      },
      "systemctl --user start proton-syncd",
    ),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:10px" },
      "This screen switches over automatically once the daemon reports its first sync — no need to close it.",
    ),
  );

  const statusLine = (() => {
    if (recheck.status === "checking") return "Checking…";
    if (recheck.status === "idle") return "";
    if (recheck.error) return `Couldn't reach the daemon: ${recheck.error}`;
    if (recheck.state && recheck.state !== "firstRun")
      return `Connected — state: ${recheck.state}. Handing off…`;
    return "Still first run — waiting for the first sync to complete.";
  })();

  const checkCard = el(
    "div",
    { class: "card", style: "display:flex;align-items:center;gap:14px;flex-wrap:wrap" },
    el(
      "span",
      { class: "mono", style: "flex:1;min-width:0;font-size:var(--fs-meta);color:var(--muted)" },
      statusLine || "Not checked yet.",
    ),
    el(
      "button",
      { class: "btn primary", disabled: recheck.status === "checking", onClick: () => recheckNow() },
      recheck.status === "checking" ? "Checking…" : "Check status now",
    ),
  );

  return [
    card,
    checkCard,
    footerNav({
      onBack: () => {
        step = 3;
        paint();
      },
    }),
  ];
}

// ---- shared chrome (stepper header, safety banner, back/next footer) ----------------------------

const STEP_LABELS = ["CLI check", "Folders", "Review plan", "Start service"];

function stepperHeader() {
  const segments = STEP_LABELS.map((label, idx) => {
    const n = idx + 1;
    const done = n < step;
    const active = n === step;
    const barColor = done || active ? "var(--upload-fill)" : "var(--border)";
    const textColor = active ? "var(--upload-text)" : done ? "var(--muted)" : "var(--muted-2)";
    return el(
      "div",
      { style: "flex:1;min-width:0" },
      el("div", { style: `height:3px;border-radius:2px;background:${barColor}` }),
      el(
        "div",
        { class: "mono", style: `font-size:10px;color:${textColor};margin-top:7px` },
        `${n} · ${label}`,
      ),
    );
  });

  return el(
    "div",
    { class: "card" },
    el(
      "div",
      { style: "display:flex;align-items:center;justify-content:space-between;margin-bottom:12px" },
      el("div", { style: "font-size:var(--fs-section);font-weight:600" }, "Set up Proton Drive Sync"),
      el(
        "span",
        { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted)" },
        `step ${step} of 4`,
      ),
    ),
    el("div", { style: "display:flex;gap:8px" }, segments),
  );
}

function safetyBanner() {
  return el(
    "div",
    { class: "safety-banner", style: "border-radius:var(--radius-card);border:1px solid var(--warn-border)" },
    el("span", { class: "glyph" }, "▲"),
    el(
      "span",
      {},
      "The first sync is a non-destructive merge: local-only files upload, remote-only files download, matching files link automatically, and anything that differs is kept as both copies. Nothing is deleted on this first pass.",
    ),
  );
}

function footerNav({
  backDisabled = false,
  onBack,
  hint = "",
  nextLabel = "Next",
  nextDisabled = true,
  onNext,
} = {}) {
  return el(
    "div",
    { class: "card", style: "display:flex;align-items:center;gap:12px" },
    el(
      "span",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-right:auto" },
      hint,
    ),
    el("button", { class: "btn", disabled: backDisabled, onClick: onBack }, "Back"),
    onNext
      ? el("button", { class: "btn primary", disabled: nextDisabled, onClick: onNext }, nextLabel)
      : null,
  );
}

// ---- paint / mount --------------------------------------------------------------------------

function paint() {
  if (!lastContainer) return;
  const body =
    step === 1 ? renderStep1() : step === 2 ? renderStep2() : step === 3 ? renderStep3() : renderStep4();
  lastContainer.replaceChildren(stepperHeader(), safetyBanner(), ...body);
}

export function renderOnboarding(container, ctx) {
  lastContainer = container;
  lastCtx = ctx;

  if (cli.status === "idle") runCliCheck();
  if (step >= 2 && !cfgLoaded && !cfgLoading && !cfgLoadError) startLoad();

  // The shell re-invokes this on every 2 s poll tick. A full repaint would `replaceChildren` the
  // whole step body — including whatever `<input>` currently has focus in step 2's folder fields —
  // and pop the cursor out mid-keystroke (same concern/fix as settings.js). Our own `onInput`
  // handlers already keep `draft` in sync without a repaint; skip only this externally-triggered one.
  if (container.contains && document.activeElement && container.contains(document.activeElement)) return;

  paint();
}
