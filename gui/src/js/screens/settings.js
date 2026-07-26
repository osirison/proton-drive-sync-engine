// Settings screen (S6, #87). Own ONLY this file. Read via `ctx.api.readConfig()`, save via
// `ctx.api.writeConfig(update)` (only changed fields; the writer preserves comments/daemon-only keys
// and is rejected if the daemon parser would refuse it). Selective sync = folder tree ⇄ raw
// include/exclude globs with live match counts. Say "saving restarts the daemon"; force a dry run
// before a root change. Do not edit screens.js, app.js, or other screen modules.

import { screenPlaceholder } from "../components.js";

export function renderSettings(container, _ctx) {
  screenPlaceholder(container, "Settings", "S6 · #87 — config editor + selective sync");
}
