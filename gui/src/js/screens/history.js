// History screen (S5, #86). Own ONLY this file. Render a reverse-chronological list of
// `ctx.select.response().status_history` (coloured dot, mono time, label, mono summary). State the
// 20-entry / restart-persisted limit and point to `journalctl --user -u proton-syncd` for more.
// Do not edit screens.js, app.js, or other screen modules.

import { screenPlaceholder } from "../components.js";

export function renderHistory(container, _ctx) {
  screenPlaceholder(container, "History", "S5 · #86 — status history (last 20)");
}
