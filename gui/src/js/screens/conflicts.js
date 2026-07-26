// Conflicts screen (S3, #84). Own ONLY this file. Rail of conflicted files + side-by-side compare;
// four staged choices applied together via `ctx.api.resolveConflict(conflict, choice)`; text files
// get a line-level diff, binaries show size/time only. Read the set from `ctx.select.conflicts()`
// (the single unresolved-set selector). Do not edit screens.js, app.js, or other screen modules.

import { screenPlaceholder } from "../components.js";

export function renderConflicts(container, _ctx) {
  screenPlaceholder(container, "Conflicts", "S3 · #84 — conflict resolution");
}
