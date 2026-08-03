// Delete-approval screen (S9, #90). Own ONLY this file. List `ctx.select.pendingDeletions()`
// (path, direction, entity, detected time) and wire `ctx.api.approve(target)` / `ctx.api.deny(target)`
// per-item and for "all". Explain the guard and that approving a remote delete removes the local
// file permanently. Do not edit screens.js, app.js, or other screen modules.

import { el, relativeTime } from "../components.js";
import { setPendingDeletions } from "../store.js";

// Transient, screen-local UI state — the last action's error, and which target ("<path>" or
// "all") is currently in flight (plus which verb, so only the clicked button's label changes).
// This belongs to this screen alone (not the shared store): it only needs to survive across
// this file's own re-renders, and disabling buttons mid-flight guards against double-firing a
// permanent deletion on a slow connection.
let lastError = null;
// `isAll` is part of the identity, not just `target`: the bulk buttons use the wire sentinel
// "all" as their target, which a pending deletion for a file literally *named* `all` also carries
// (with `isAll: false`). Matching on `isAll` too keeps a row click from lighting up the bulk
// button (and vice versa). `target` stays the wire selector ("all" ⇒ every item, else the path).
let busy = null; // { target: "<path>" | "all", verb: "approve" | "deny", isAll: bool } | null

// Acknowledged clicks: path → { verb: "approved" | "approved-paused" | "denied", at: epoch ms,
// fingerprint }. ("approved-paused" is the daemon-was-paused variant — the delete waits for a
// resume.) The daemon's ack
// round trip is near-instant, but the item does NOT leave `pending_deletions` on it: an
// approved deletion stays listed until the next reconcile pass actually executes it, and a
// denied one stays pending indefinitely (deny only revokes/declines — the file is still
// missing on one side). Meanwhile the 2 s status poll rewrites the store's list on every
// tick. So the lasting "your click landed" signal cannot live in the store or ride the busy
// flag alone; this map pins each acknowledged row to a visible confirmation state across
// those overwrites. Approved entries clear when the row leaves the daemon's list (or after a
// safety timeout, so an inert approval — e.g. a fingerprint that no longer matches — can't
// strand the row without buttons); denied entries clear after a short confirmation interval.
const acknowledged = new Map();
const APPROVED_ACK_SAFETY_MS = 90_000;
const DENIED_ACK_MS = 4_000;
let ackExpiryTimer = null;

function ackTtl(entry) {
  return entry.verb === "denied" ? DENIED_ACK_MS : APPROVED_ACK_SAFETY_MS;
}

/**
 * The live acknowledgment for this listed item, dropping it first if its display window has
 * expired — or if the item's fingerprint no longer matches the one the click acted on. A
 * changed fingerprint means this row is a NEW pending deletion at the same path (the
 * acknowledged one resolved while this screen wasn't rendering): it has not been approved or
 * denied, so it must show live buttons, not a stale pill.
 */
function acknowledgment(item) {
  const path = typeof item.path === "string" ? item.path : String(item.path ?? "");
  const entry = acknowledged.get(path);
  if (!entry) return null;
  if (Date.now() - entry.at >= ackTtl(entry)) {
    acknowledged.delete(path);
    return null;
  }
  if (entry.fingerprint != null && item.fingerprint != null && entry.fingerprint !== item.fingerprint) {
    acknowledged.delete(path);
    return null;
  }
  return entry;
}

// One shared timer wakes the screen when the soonest acknowledgment expires, so a "Deny
// recorded" pill clears itself even if no poll or click happens to re-render first.
function scheduleAckExpiry(ctx) {
  clearTimeout(ackExpiryTimer);
  ackExpiryTimer = null;
  let soonest = Infinity;
  const now = Date.now();
  for (const entry of acknowledged.values()) {
    soonest = Math.min(soonest, entry.at + ackTtl(entry) - now);
  }
  if (soonest !== Infinity) {
    ackExpiryTimer = setTimeout(() => nudgeRerender(ctx), Math.max(soonest, 0) + 50);
  }
}

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

async function runAndRefresh(ctx, target, verb, isAll) {
  busy = { target, verb, isAll };
  lastError = null;
  // Capture the affected items at click time, before any await: the poll may rewrite the
  // store's list mid-flight, an "all" acknowledgment must cover exactly the items the user
  // was looking at when they clicked, and each acknowledgment is pinned to the item's
  // fingerprint so it can never label a future same-path pending deletion.
  const snapshot = ctx.select.pendingDeletions();
  const affected = (isAll ? snapshot : snapshot.filter((item) => String(item.path) === target)).map(
    (item) => ({ path: String(item.path), fingerprint: item.fingerprint ?? null }),
  );
  nudgeRerender(ctx); // show the busy state immediately, before the round trip
  let confirmed = false;
  try {
    const payload =
      verb === "approve" ? await ctx.api.approve(target, !isAll) : await ctx.api.deny(target, !isAll);
    // The Tauri approve/deny commands NEVER reject on a daemon failure — a dead socket or a
    // dropped connection comes back as a RESOLVED payload with `error` set and no `response`.
    // A confirmation pill must only ever appear for an action the daemon durably recorded: a
    // false "✓ Deny recorded" would sit there while a still-standing approval deletes the
    // file it just promised was safe.
    if (!payload || payload.error != null || payload.response == null) {
      throw new Error((payload && payload.error) || "the daemon did not confirm the request");
    }
    const paused = payload.response.paused === true;
    const at = Date.now();
    const verbState = verb === "deny" ? "denied" : paused ? "approved-paused" : "approved";
    for (const { path, fingerprint } of affected) {
      acknowledged.set(path, { verb: verbState, at, fingerprint });
    }
    confirmed = true;
    if (verb === "approve" && !paused) {
      // Best-effort nudge: the approval is already durably recorded, but the daemon only
      // executes it on its next reconcile pass (up to its poll interval away). Asking for a
      // sync now makes the row disappear while the user is still watching; a failure here
      // changes nothing, so it is deliberately swallowed. Skipped while paused — the daemon
      // would skip the sync anyway, and the pill copy says the delete waits for resume.
      ctx.api.syncNow().catch(() => {});
    }
  } catch (e) {
    const who = isAll ? "all pending deletions" : target;
    lastError = `Couldn't ${verb} ${who}: ${e && e.message ? e.message : String(e)}`;
  } finally {
    busy = null;
    nudgeRerender(ctx);
  }
  if (confirmed) {
    // Refresh OUTSIDE the verb-attributed catch: the action itself succeeded, and a refresh
    // hiccup must not render as "Couldn't approve …" beside a success pill. The 2 s status
    // poll self-heals the list within a tick anyway.
    try {
      await refreshList(ctx);
    } catch (_) {
      /* poll self-heals */
    }
  }
}

function emptyCard(text) {
  return el("div", { class: "card" }, el("div", { class: "ledger-empty" }, text));
}

/**
 * The verb-aware label for one action button, so only the clicked button animates. `isAll`
 * disambiguates the bulk buttons from a per-row button whose path is literally `all` — both carry
 * the target string "all", so target + verb alone would cross-fire between them.
 */
function actionLabel(target, verb, isAll, idleLabel, busyLabel) {
  return busy && busy.target === target && busy.verb === verb && busy.isAll === isAll ? busyLabel : idleLabel;
}

/** The lasting post-ack confirmation shown in place of the action buttons. */
function acknowledgmentPill(entry) {
  const denied = entry.verb === "denied";
  const label = denied
    ? "✓ Deny recorded — nothing deleted"
    : entry.verb === "approved-paused"
      ? "✓ Approved — deletes when sync resumes"
      : "✓ Approved — deleting…";
  return el(
    "div",
    {
      class: "mono" + (denied ? "" : " dir-destructive"),
      style:
        "flex:none;align-self:center;font-size:var(--fs-meta);font-weight:600" +
        (denied ? ";color:var(--muted)" : ""),
    },
    label,
  );
}

function renderRow(ctx, item) {
  const path = typeof item.path === "string" ? item.path : String(item.path ?? "");
  const isDirectory = item.entity_kind === "directory";
  const copy = directionCopy(item.direction, item.entity_kind);
  // Any in-flight approve/deny disables ALL row actions (not just this row's / "all"), so a second
  // click can't overwrite `busy` and let the first request's `finally` re-enable the UI while the
  // second is still running — which would allow overlapping destructive actions.
  const anyBusy = busy != null;
  const ack = acknowledgment(item);

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
    ack
      ? acknowledgmentPill(ack)
      : el(
          "div",
          { style: "display:flex;gap:8px;flex:none" },
          el(
            "button",
            {
              class: "btn danger",
              disabled: anyBusy,
              onClick: () => runAndRefresh(ctx, path, "approve", false),
            },
            actionLabel(path, "approve", false, "Approve", "Approving…"),
          ),
          el(
            "button",
            {
              class: "btn",
              disabled: anyBusy,
              onClick: () => runAndRefresh(ctx, path, "deny", false),
            },
            actionLabel(path, "deny", false, "Deny", "Denying…"),
          ),
        ),
  );
}

export function renderDeletions(container, ctx) {
  const { select } = ctx;
  const st = select.daemonState();
  const items = select.pendingDeletions();

  // An acknowledged row that has left the daemon's list is done (the approved deletion
  // executed, or the item resolved some other way) — drop its entry so the map can't grow
  // across a long session.
  const listed = new Set(items.map((item) => String(item.path)));
  for (const path of acknowledged.keys()) {
    if (!listed.has(path)) acknowledged.delete(path);
  }
  scheduleAckExpiry(ctx);

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
            onClick: () => runAndRefresh(ctx, "all", "approve", true),
          },
          actionLabel("all", "approve", true, "Approve all", "Approving all…"),
        ),
        el(
          "button",
          {
            class: "btn",
            disabled: allBusy,
            onClick: () => runAndRefresh(ctx, "all", "deny", true),
          },
          actionLabel("all", "deny", true, "Deny all", "Denying all…"),
        ),
      ),
    ),
  );

  for (const item of items) children.push(renderRow(ctx, item));

  container.replaceChildren(...children);
}
