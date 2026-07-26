// Plan-preview screen (S4, #85). Own ONLY this file. Run `ctx.api.runDryRun()` → { report,
// requires_delete_gate, files_at_risk }. Render the summary grid + one row per action (destructive
// rows tinted + sorted first). Arm the typed-DELETE gate ONLY when `requires_delete_gate` is true
// (never for a purge-only plan); name the `files_at_risk`. Do not edit screens.js/app.js/others.

import { screenPlaceholder } from "../components.js";

export function renderPlan(container, _ctx) {
  screenPlaceholder(container, "Plan preview", "S4 · #85 — dry-run review + DELETE gate");
}
