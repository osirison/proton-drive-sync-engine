// Delete-approval screen (S9, #90). Own ONLY this file. List `ctx.select.pendingDeletions()`
// (path, direction, entity, detected time) and wire `ctx.api.approve(target)` / `ctx.api.deny(target)`
// per-item and for "all". Explain the guard and that approving a remote delete removes the local
// file permanently. Do not edit screens.js, app.js, or other screen modules.

import { screenPlaceholder } from "../components.js";

export function renderDeletions(container, _ctx) {
  screenPlaceholder(container, "Delete approvals", "S9 · #90 — review withheld deletions (approve/deny)");
}
