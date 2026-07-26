// Activity screen (S2, #83). Own ONLY this file. Build the full-height activity ledger with the
// filter chips promoted to the header, reusing `renderLedger` from ../components.js. Rows come from
// `ctx.select.response().status_history` (and error rows from `last_error`); chip counts derive from
// the visible rows. Do not edit screens.js, app.js, or other screen modules.

import { screenPlaceholder } from "../components.js";

export function renderActivity(container, _ctx) {
  screenPlaceholder(container, "Activity", "S2 · #83 — full activity ledger");
}
