// Delete-approval screen (S9, #90). Own ONLY this file. List `ctx.select.pendingDeletions()`
// (path, direction, entity, detected time) and wire `ctx.api.approve(target)` / `ctx.api.deny(target)`
// per-item and for "all". Explain the guard and that approving a remote delete removes the local
// file permanently. Do not edit screens.js, app.js, or other screen modules.

import { el, relativeTime } from "../components.js";
import { setPendingDeletions } from "../store.js";

// Transient, screen-local UI state — the last action's error, and which target ("<path>" or
// "all") is currently in flight. This belongs to this screen alone (not the shared store): it
// only needs to survive across this file's own re-renders, and disabling buttons mid-flight
// guards against double-firing a permanent deletion on a slow connection.
let lastError = null;
let busy = null;

/**
 * What approving THIS item actually does, in plain words. `direction` mirrors the engine's
 * `DeleteDirection` (src/sync.rs):
 *   - "local"  = propagate a deletion that already happened on Proton Drive onto the local disk
 *                (`DeleteDirection::Local` → `LocalDelete`). Approving removes the LOCAL copy.
 *   - "remote" = propagate a deletion that already happened locally onto Proton Drive
 *                (`DeleteDirection::Remote` → `RemoteDelete`). Approving removes the PROTON DRIVE
 *                copy.
 * Verified against the `DeleteDirection` doc comments in src/sync.rs — do not swap these.
 *
 * The two sides are NOT equally reversible, and the copy says so instead of calling both
 * "permanent" outright: `LocalDelete` (src/daemon.rs) calls `fs::remove_file`/`remove_dir_all`
 * directly — genuinely gone, no trash involved. `RemoteDelete` (src/proton.rs `ProtonClient::
 * delete`) shells `proton-drive filesystem trash <path>` — it moves the item to Proton Drive's
 * own Trash, so it vanishes from the synced Drive immediately but is technically recoverable
 * there until emptied. Overstating the remote case as un-trashed permanent deletion would be a
 * factual error on the one screen where that error matters most.
 */
function directionCopy(direction, entityKind) {
  const thing = entityKind === "directory" ? "folder" : "file";
  const andContents = entityKind === "directory" ? ", and everything inside it," : "";
  if (direction === "local") {
    return {
      target: "this computer",
      detail: `Already deleted on Proton Drive. Approving permanently deletes the matching ${thing}${andContents} on THIS COMPUTER — removed straight from disk, not moved to any trash.`,
    };
  }
  if (direction === "remote") {
    return {
      target: "Proton Drive",
      detail: `Already deleted on this computer. Approving moves the matching ${thing}${andContents} to PROTON DRIVE'S TRASH — it disappears from your synced Drive immediately; recovering it means opening Proton Drive's own Trash yourself before it gets emptied.`,
    };
  }
  return {
    target: "an unrecognized location",
    detail: `Unrecognized deletion direction "${direction}" — treat this as destructive and review carefully before approving.`,
  };
}

async function refreshList(ctx) {
  // Triggers the store's `emit()`, which re-renders every subscribed screen — including this one
  // — so the approved/denied item drops out of the list without any local re-render bookkeeping.
  setPendingDeletions(await ctx.api.listPendingDeletions());
}

// Re-publishes the list unchanged, purely to make the store re-`emit()`. That routes back through
// app.js's own `render()` (which re-renders whichever screen is currently active), instead of this
// module poking `container` directly — so if the user switches tabs while an approve/deny is still
// in flight, the busy/error redraw lands on the active screen instead of clobbering it with this
// screen's content.
function nudgeRerender(ctx) {
  setPendingDeletions(ctx.select.pendingDeletions());
}

async function runAndRefresh(ctx, target, verb) {
  busy = target;
  lastError = null;
  nudgeRerender(ctx); // show the busy state immediately, before the round trip
  try {
    if (verb === "approve") await ctx.api.approve(target);
    else await ctx.api.deny(target);
    await refreshList(ctx);
  } catch (e) {
    const who = target === "all" ? "all pending deletions" : target;
    lastError = `Couldn't ${verb} ${who}: ${e && e.message ? e.message : String(e)}`;
  } finally {
    busy = null;
    nudgeRerender(ctx);
  }
}

function emptyCard(text) {
  return el("div", { class: "card" }, el("div", { class: "ledger-empty" }, text));
}

function renderRow(ctx, item) {
  const path = typeof item.path === "string" ? item.path : String(item.path ?? "");
  const isDirectory = item.entity_kind === "directory";
  const copy = directionCopy(item.direction, item.entity_kind);
  // Any in-flight approve/deny disables ALL row actions (not just this row's / "all"), so a second
  // click can't overwrite `busy` and let the first request's `finally` re-enable the UI while the
  // second is still running — which would allow overlapping destructive actions.
  const anyBusy = busy != null;

  return el(
    "div",
    {
      class: "card",
      style: "margin-bottom:10px;display:flex;justify-content:space-between;align-items:flex-start;gap:14px",
    },
    el(
      "div",
      { style: "min-width:0;flex:1" },
      el("div", { class: "mono", style: "word-break:break-all;font-size:var(--fs-body)" }, path),
      el(
        "div",
        { class: "mono", style: "margin-top:4px;font-size:var(--fs-meta);color:var(--muted)" },
        `${isDirectory ? "directory" : "file"} · detected ${relativeTime(item.detected_epoch_secs)} · target of this approval: ${copy.target}`,
      ),
      el(
        "div",
        { class: "dir-destructive", style: "margin-top:8px;font-size:var(--fs-body);font-weight:600" },
        copy.detail,
      ),
    ),
    el(
      "div",
      { style: "display:flex;gap:8px;flex:none" },
      el(
        "button",
        {
          class: "btn danger",
          disabled: anyBusy,
          onClick: () => runAndRefresh(ctx, path, "approve"),
        },
        busy === path ? "Approving…" : "Approve",
      ),
      el(
        "button",
        {
          class: "btn",
          disabled: anyBusy,
          onClick: () => runAndRefresh(ctx, path, "deny"),
        },
        "Deny",
      ),
    ),
  );
}

export function renderDeletions(container, ctx) {
  const { select } = ctx;
  const st = select.daemonState();
  const items = select.pendingDeletions();

  const children = [
    el(
      "div",
      { style: "margin-bottom:14px" },
      el("div", { style: "font-size:var(--fs-section);font-weight:600" }, "Delete approvals"),
      el(
        "div",
        { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted);margin-top:4px" },
        "The daemon withholds deletions here instead of applying them automatically. Nothing " +
          "listed below happens until you approve it — read the direction on each item first: " +
          "local approvals delete the file from this computer immediately, straight from disk, " +
          "no trash. Proton Drive approvals move the file to Proton Drive's Trash and remove it " +
          "from your synced Drive immediately.",
      ),
    ),
  ];

  if (lastError) {
    children.push(
      el(
        "div",
        { class: "card dir-destructive", style: "margin-bottom:14px;font-size:var(--fs-body)" },
        lastError,
      ),
    );
  }

  if (st === "unreachable") {
    children.push(emptyCard("Daemon unreachable — can't fetch pending deletions."));
    container.replaceChildren(...children);
    return;
  }

  if (items.length === 0) {
    children.push(emptyCard("No deletions are awaiting approval."));
    container.replaceChildren(...children);
    return;
  }

  const localCount = items.filter((i) => i.direction === "local").length;
  const remoteCount = items.filter((i) => i.direction === "remote").length;
  const allBusy = busy != null;

  children.push(
    el(
      "div",
      {
        class: "card",
        style:
          "margin-bottom:14px;display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap",
      },
      el(
        "div",
        { class: "mono", style: "font-size:var(--fs-meta);color:var(--muted)" },
        `${items.length} pending — ${localCount} would delete locally, ${remoteCount} would delete on Proton Drive`,
      ),
      el(
        "div",
        { style: "display:flex;gap:8px" },
        el(
          "button",
          {
            class: "btn danger",
            disabled: allBusy,
            onClick: () => runAndRefresh(ctx, "all", "approve"),
          },
          busy === "all" ? "Approving all…" : "Approve all",
        ),
        el(
          "button",
          {
            class: "btn",
            disabled: allBusy,
            onClick: () => runAndRefresh(ctx, "all", "deny"),
          },
          "Deny all",
        ),
      ),
    ),
  );

  for (const item of items) children.push(renderRow(ctx, item));

  container.replaceChildren(...children);
}
