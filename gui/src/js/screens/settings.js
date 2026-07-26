// Settings screen (S6, #87). Own ONLY this file. Read via `ctx.api.readConfig()`, save via
// `ctx.api.writeConfig(update)` (only changed fields; the writer preserves comments/daemon-only keys
// and is rejected if the daemon parser would refuse it). Selective sync v1 = raw include/exclude
// globs (see the note on `selectiveSyncSection` below for why the folder-tree view isn't here yet).
// Do not edit screens.js, app.js, or other screen modules.

import { el, dash } from "../components.js";

// ---- module-level form state -------------------------------------------------------------
// Kept here (not in the shared store) per the F1 pattern: this screen owns a draft that must
// survive the shell's 2s poll re-render without being clobbered by a fresh `readConfig()` on every
// paint. `loaded` gates the one-time fetch; `original` is the last-known-persisted baseline used
// both to diff for the save payload and to detect the "did the user touch a root" case.
let loaded = false;
let loading = false;
let loadError = null;
let meta = null; // display-only, never diffed/sent: { path, exists }
let original = null; // normalized draft baseline (see `toDraft`), refreshed on load/successful save
let form = null; // editable draft: local_root, remote_root, scan_interval_secs, events_driven,
// include, exclude, proton_cli, proton_timeout_secs, proton_list_attempts,
// delete_approval_remote, delete_approval_local

let saving = false;
let saveError = null;
let saveNotice = null; // { text, rootChanged } set after a successful save

// Transient text-buffers for the "add a pattern" inputs — not part of the diffed draft, cleared
// once the pattern is committed into form.include / form.exclude.
let newIncludeText = "";
let newExcludeText = "";

// DOM refs captured during the current `paint()`, used to update Save-button/dirty-note/root-warning
// state directly on every keystroke instead of rebuilding (and re-focusing) the whole form — see
// `refreshDerived`.
let refs = {};

const TEXT_FIELDS = ["local_root", "remote_root", "proton_cli"];
const NUMBER_FIELDS = ["scan_interval_secs", "proton_timeout_secs", "proton_list_attempts"];
const BOOL_FIELDS = ["events_driven", "delete_approval_remote", "delete_approval_local"];
const LIST_FIELDS = ["include", "exclude"];

const inputStyle =
  "width:100%;box-sizing:border-box;padding:8px 10px;border-radius:var(--radius-control);" +
  "border:1px solid var(--border);background:var(--row);color:var(--text-1);" +
  "font-family:var(--font-mono);font-size:var(--fs-control)";

// `read_config`'s bool fields are `Option<bool>` on the Rust side — a raw config file that never
// set the key comes back `null`, not the engine's actual resolved runtime default. Falling back to
// `false` there would misrepresent a security-relevant guard (delete-approval) as off when the
// daemon treats it as on. These mirror the documented defaults (CLAUDE.md / src/config.rs): both
// delete-approval directions default true, and events-driven reconcile is on by default.
const BOOL_DEFAULTS = { events_driven: true, delete_approval_remote: true, delete_approval_local: true };

function toDraft(cfg) {
  const draft = {};
  for (const k of TEXT_FIELDS) draft[k] = cfg[k];
  for (const k of NUMBER_FIELDS) draft[k] = cfg[k];
  for (const k of BOOL_FIELDS) draft[k] = cfg[k] == null ? BOOL_DEFAULTS[k] : cfg[k];
  for (const k of LIST_FIELDS) draft[k] = Array.isArray(cfg[k]) ? [...cfg[k]] : [];
  return draft;
}

function arraysEqual(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/** Only the fields that actually differ from `original` — this is exactly the `writeConfig`
 * payload, so it's both the dirty-check and the save-diff in one place. Blank/unparseable number
 * fields are skipped (not sent, not counted as a change) rather than silently coerced to 0. */
function buildUpdate() {
  const update = {};
  for (const k of [...TEXT_FIELDS, ...BOOL_FIELDS]) {
    if (form[k] !== original[k]) update[k] = form[k];
  }
  for (const k of NUMBER_FIELDS) {
    const raw = form[k];
    const str = raw == null ? "" : String(raw).trim();
    if (str === "") continue;
    const n = Number(str);
    if (Number.isFinite(n) && n !== original[k]) update[k] = n;
  }
  for (const k of LIST_FIELDS) {
    if (!arraysEqual(form[k], original[k])) update[k] = [...form[k]];
  }
  return update;
}

function computeDirty() {
  return Object.keys(buildUpdate()).length > 0;
}

function isRootDirty() {
  return form.local_root !== original.local_root || form.remote_root !== original.remote_root;
}

function messageOf(e) {
  return e && e.message ? e.message : String(e);
}

/** Surgical update after a field edit: toggle the Save/Discard buttons, the "unsaved changes"
 * note, and the root-change warning WITHOUT rebuilding the DOM — a full `paint()` on every
 * keystroke would recreate the focused `<input>` and drop focus/cursor mid-type. Structural edits
 * (toggle a switch, add/remove a glob, load/save complete) still go through `paint()`. */
function refreshDerived() {
  const dirty = computeDirty();
  if (refs.saveBtn) refs.saveBtn.disabled = !dirty || saving;
  if (refs.discardBtn) refs.discardBtn.disabled = !dirty || saving;
  if (refs.dirtyNote) refs.dirtyNote.style.display = dirty ? "" : "none";
  if (refs.rootWarning) refs.rootWarning.style.display = isRootDirty() ? "" : "none";
  // A stale "Saved…" banner reads oddly next to "Unsaved changes" once the user edits again after
  // a save. Hide it visually (surgical — no repaint, so no focus loss); `saveNotice`/`saveError`
  // themselves are cleared for real at the start of the next `handleSave`.
  if (refs.statusBox) refs.statusBox.style.display = dirty ? "none" : "";
}

// ---- small builders -------------------------------------------------------------------------

function sectionTitle(text) {
  return el("div", { style: "font-size:var(--fs-section);font-weight:600;margin-bottom:12px" }, text);
}

function fieldRow(label, hint, control) {
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
    control,
    hint ? el("div", { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted-2);margin-top:4px" }, hint) : null,
  );
}

function textInput(container, ctx, key, kind, opts = {}) {
  const props = {
    type: kind,
    value: form[key],
    style: inputStyle,
    onInput: (e) => {
      form[key] = e.target.value;
      refreshDerived();
    },
  };
  if (kind === "number") {
    props.min = opts.min ?? "1";
    props.step = "1";
  }
  return el("input", props);
}

function toggleRow(container, ctx, key, label, description) {
  return el(
    "div",
    { style: "display:flex;align-items:flex-start;gap:10px;margin-bottom:12px" },
    el("input", {
      type: "checkbox",
      checked: !!form[key],
      style: "margin-top:3px;flex:none",
      onChange: (e) => {
        form[key] = e.target.checked;
        paint(container, ctx);
      },
    }),
    el(
      "div",
      {},
      el("div", { style: "font-size:var(--fs-body)" }, label),
      description
        ? el("div", { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted-2);margin-top:2px" }, description)
        : null,
    ),
  );
}

function bufferFor(key) {
  return key === "include" ? newIncludeText : newExcludeText;
}
function setBuffer(key, value) {
  if (key === "include") newIncludeText = value;
  else newExcludeText = value;
}

function addPattern(container, ctx, key) {
  const text = bufferFor(key).trim();
  if (!text) return;
  if (!form[key].includes(text)) form[key].push(text);
  setBuffer(key, "");
  paint(container, ctx);
}

function patternListEditor(container, ctx, key, label) {
  const list = form[key];
  const rows = list.map((pattern, idx) =>
    el(
      "div",
      { style: "display:flex;align-items:center;gap:8px;padding:4px 0;border-bottom:1px solid var(--border-soft)" },
      el("span", { class: "mono", style: "flex:1;font-size:var(--fs-control);word-break:break-all" }, pattern),
      el(
        "button",
        {
          class: "btn",
          style: "padding:3px 9px;font-size:11px",
          onClick: () => {
            form[key].splice(idx, 1);
            paint(container, ctx);
          },
        },
        "Remove",
      ),
    ),
  );

  return el(
    "div",
    {},
    el(
      "div",
      {
        class: "mono",
        style:
          "font-size:var(--fs-label);text-transform:uppercase;letter-spacing:var(--tracking-label);" +
          "color:var(--muted);margin-bottom:6px",
      },
      `${label} (${list.length})`,
    ),
    list.length
      ? el("div", {}, rows)
      : el("div", { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted-2);margin-bottom:4px" }, "No patterns."),
    el(
      "div",
      { style: "display:flex;gap:8px;margin-top:8px" },
      el("input", {
        type: "text",
        placeholder: key === "include" ? "e.g. docs/**" : "e.g. *.tmp",
        style: `${inputStyle};flex:1`,
        value: bufferFor(key),
        onInput: (e) => setBuffer(key, e.target.value),
        onKeydown: (e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            addPattern(container, ctx, key);
          }
        },
      }),
      el("button", { class: "btn", onClick: () => addPattern(container, ctx, key) }, "Add"),
    ),
  );
}

// ---- sections ---------------------------------------------------------------------------------

function foldersSection(container, ctx) {
  const rootWarning = el(
    "div",
    {
      class: "dir-destructive",
      style: `margin-top:6px;font-size:var(--fs-meta);font-weight:600;${isRootDirty() ? "" : "display:none"}`,
    },
    "Changing a root re-bootstraps the index from scratch on the next reconcile. Preview a plan " +
      "before restarting the daemon.",
    el(
      "div",
      { style: "margin-top:6px" },
      el("button", { class: "btn", onClick: () => ctx.actions.setTab("plan") }, "Preview plan"),
    ),
  );
  refs.rootWarning = rootWarning;

  return el(
    "div",
    { class: "card", style: "margin-bottom:14px" },
    sectionTitle("Folders"),
    fieldRow("Local root", null, textInput(container, ctx, "local_root", "text")),
    fieldRow("Remote root", "Path on Proton Drive, e.g. /Drive/RemoteFolder", textInput(container, ctx, "remote_root", "text")),
    rootWarning,
  );
}

function scheduleSection(container, ctx) {
  return el(
    "div",
    { class: "card", style: "margin-bottom:14px" },
    sectionTitle("Schedule"),
    fieldRow(
      "Scan interval (seconds)",
      "Periodic full-scan fallback interval.",
      textInput(container, ctx, "scan_interval_secs", "number", { min: "1" }),
    ),
    toggleRow(
      container,
      ctx,
      "events_driven",
      "Event-driven reconcile",
      "Use Proton's volume-events stream for fast incremental reconcile between full scans. Off " +
        "restores the byte-identical snapshot-only path.",
    ),
  );
}

// v1 ships only the raw include/exclude glob-list editors below. The design's folder-tree view
// (checking a folder writes an exclude glob, design §3.6 / handoff options 5a+1k) is a real
// planned enhancement, not cut for scope reasons alone — it needs a local-directory listing that
// the daemon doesn't expose over IPC today (`read_config` only returns the resolved glob arrays,
// not a directory tree to check boxes against), so it's deferred rather than faked here.
function selectiveSyncSection(container, ctx) {
  return el(
    "div",
    { class: "card", style: "margin-bottom:14px" },
    sectionTitle("Selective sync"),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-bottom:12px" },
      "Glob patterns matched against paths relative to the sync root. Exclude always beats include " +
        'when a path matches both. ".sync/" (the engine\'s own state directory) is always ignored, ' +
        "regardless of these lists.",
    ),
    patternListEditor(container, ctx, "include", "Include"),
    el("div", { style: "height:14px" }),
    patternListEditor(container, ctx, "exclude", "Exclude"),
  );
}

function cliSection(container, ctx) {
  return el(
    "div",
    { class: "card", style: "margin-bottom:14px" },
    sectionTitle("CLI"),
    fieldRow(
      "proton-drive executable",
      "Path, or a bare command resolved via PATH.",
      textInput(container, ctx, "proton_cli", "text"),
    ),
    fieldRow("Command timeout (seconds)", null, textInput(container, ctx, "proton_timeout_secs", "number", { min: "1" })),
    fieldRow(
      "List retry attempts",
      "Read-only remote listings retry on failure; uploads/downloads/deletes never do.",
      textInput(container, ctx, "proton_list_attempts", "number", { min: "1" }),
    ),
  );
}

function deleteApprovalSection(container, ctx) {
  return el(
    "div",
    { class: "card", style: "margin-bottom:14px" },
    sectionTitle("Delete approval"),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-bottom:8px" },
      "When on, the daemon withholds a matching deletion until you approve it on the Deletions " +
        "screen instead of applying it automatically. Both default to on (protected).",
    ),
    toggleRow(
      container,
      ctx,
      "delete_approval_remote",
      "Guard remote deletes",
      "Require approval before removing a file on Proton Drive because it was deleted locally.",
    ),
    toggleRow(
      container,
      ctx,
      "delete_approval_local",
      "Guard local deletes",
      "Require approval before removing a local file because it was deleted on Proton Drive.",
    ),
  );
}

function serviceSection() {
  const path = meta?.path;
  const exists = meta?.exists;
  return el(
    "div",
    { class: "card", style: "margin-bottom:14px" },
    sectionTitle("Service"),
    fieldRow(
      "Config file",
      exists === false ? "Doesn't exist yet — saving will create it." : null,
      el("div", { class: "mono", style: "font-size:var(--fs-control);word-break:break-all" }, dash(path)),
    ),
    el(
      "div",
      { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted)" },
      "Saving writes this file only — the running daemon keeps its already-loaded config until " +
        "it's restarted:",
    ),
    el("div", { class: "mono", style: "font-size:var(--fs-control);margin-top:4px" }, "systemctl --user restart proton-syncd"),
  );
}

function statusBanners(container, ctx) {
  const nodes = [];
  if (saveError) {
    nodes.push(
      el("div", { class: "card dir-destructive", style: "margin-bottom:14px;font-weight:600" }, `Couldn't save: ${saveError}`),
    );
  }
  if (saveNotice) {
    nodes.push(
      el(
        "div",
        { class: "card", style: "margin-bottom:14px" },
        el("div", { style: "font-weight:600" }, saveNotice.text),
        saveNotice.rootChanged
          ? el(
              "div",
              { class: "dir-destructive", style: "margin-top:8px;font-weight:600" },
              "You changed a sync root — restarting will re-bootstrap the index from scratch. Preview " +
                "a plan before restarting.",
            )
          : null,
        saveNotice.rootChanged
          ? el(
              "button",
              { class: "btn", style: "margin-top:8px", onClick: () => ctx.actions.setTab("plan") },
              "Preview plan",
            )
          : null,
      ),
    );
  }
  return nodes;
}

function footerBar(container, ctx) {
  const dirty = computeDirty();

  const dirtyNote = el(
    "span",
    { class: "mono", style: `font-size:var(--fs-meta);color:var(--muted);margin-right:auto;${dirty ? "" : "display:none"}` },
    "Unsaved changes",
  );
  refs.dirtyNote = dirtyNote;

  const discardBtn = el(
    "button",
    {
      class: "btn",
      disabled: !dirty || saving,
      onClick: () => {
        form = toDraft(original);
        saveError = null;
        paint(container, ctx);
      },
    },
    "Discard changes",
  );
  refs.discardBtn = discardBtn;

  const saveBtn = el(
    "button",
    { class: "btn primary", disabled: !dirty || saving, onClick: () => handleSave(container, ctx) },
    saving ? "Saving…" : "Save changes",
  );
  refs.saveBtn = saveBtn;

  return el(
    "div",
    { class: "card", style: "display:flex;align-items:center;gap:12px" },
    dirtyNote,
    discardBtn,
    saveBtn,
  );
}

// ---- load / save --------------------------------------------------------------------------

function startLoad(container, ctx) {
  loading = true;
  loadError = null;
  paint(container, ctx);
  ctx.api
    .readConfig()
    .then((cfg) => {
      meta = { path: cfg.path, exists: cfg.exists };
      original = toDraft(cfg);
      form = toDraft(cfg);
      loaded = true;
      loading = false;
      paint(container, ctx);
    })
    .catch((e) => {
      loadError = messageOf(e);
      loading = false;
      paint(container, ctx);
    });
}

async function handleSave(container, ctx) {
  const update = buildUpdate();
  if (Object.keys(update).length === 0) return;
  saving = true;
  saveError = null;
  saveNotice = null;
  paint(container, ctx);
  try {
    await ctx.api.writeConfig(update);
    const rootChanged = "local_root" in update || "remote_root" in update;
    // Promote the saved draft to the new baseline (merge only the sent keys, so untouched fields —
    // and the daemon-only keys this screen never reads at all — are left alone). Dirty resets
    // naturally since `form` now equals `original` on every field this screen tracks.
    original = { ...original, ...update };
    if (meta) meta.exists = true;
    saveNotice = {
      text: "Saved. Restart the daemon for changes to take effect: systemctl --user restart proton-syncd",
      rootChanged,
    };
  } catch (e) {
    // Do NOT touch `original`/`form` here — the draft stays exactly as the user left it, and stays
    // dirty, so Save remains enabled and no edits are lost.
    saveError = messageOf(e);
  } finally {
    saving = false;
    paint(container, ctx);
  }
}

// ---- paint ----------------------------------------------------------------------------------

function paint(container, ctx) {
  refs = {};

  if (!loaded) {
    container.replaceChildren(
      loadError
        ? el(
            "div",
            { class: "card dir-destructive" },
            el("div", {}, `Couldn't load configuration: ${loadError}`),
            el("button", { class: "btn", style: "margin-top:10px", onClick: () => startLoad(container, ctx) }, "Retry"),
          )
        : el("div", { class: "card mono" }, "Loading configuration…"),
    );
    return;
  }

  const statusBox = el("div", {}, ...statusBanners(container, ctx));
  refs.statusBox = statusBox;

  const children = [
    el(
      "div",
      { style: "margin-bottom:14px" },
      el("div", { style: "font-size:var(--fs-section);font-weight:600" }, "Settings"),
      el(
        "div",
        { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:4px" },
        "Edits the daemon's config file directly. Save writes only the fields you changed here — " +
          "comments and daemon-only keys are preserved, and the write is rejected if the daemon's " +
          "own parser would refuse the result.",
      ),
    ),
    statusBox,
    foldersSection(container, ctx),
    scheduleSection(container, ctx),
    selectiveSyncSection(container, ctx),
    cliSection(container, ctx),
    deleteApprovalSection(container, ctx),
    serviceSection(),
    footerBar(container, ctx),
  ];

  container.replaceChildren(...children);
  refreshDerived();
}

export function renderSettings(container, ctx) {
  if (!loaded && !loading) {
    startLoad(container, ctx);
    return;
  }
  // The shell re-invokes every screen's render on its 2s status poll. A full `paint()` here would
  // `replaceChildren` the whole form — including whatever `<input>` currently has focus — and pop
  // the cursor out to <body> mid-keystroke. Our own edit handlers already keep the form correct
  // without a full repaint (`refreshDerived`) or repaint deliberately on a discrete action (toggle,
  // add/remove pattern); skip only this externally-triggered repaint while focus is inside our form.
  if (container.contains && document.activeElement && container.contains(document.activeElement)) {
    return;
  }
  paint(container, ctx);
}
