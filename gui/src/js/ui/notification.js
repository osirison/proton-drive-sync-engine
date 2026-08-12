// The notification banner (S9) — the four events that interrupt, and the one rule about buttons.
//
// TWO CONSUMERS, ONE BUILDER. `bannerFor` is pure and returns a spec; `renderBanner` draws it (the
// `11a` frames, and the fallback chrome for a desktop with no notification server), and
// `payloadFor` flattens it for `org.freedesktop.Notifications`. A banner the desktop draws and a
// banner we draw are then the same sentence by construction, not by inspection.
//
// THE HARD RULE IS ENFORCED HERE, not in a review. `11-notifications.md`: "Delete. Discard a
// version. Approve all. Anything irreversible needs a window where you can see what you're losing."
// Every action goes through `safeActions`, which throws on a destructive label or id — so a banner
// that offered one would fail at construction, in every test that builds it.

import { el } from "./el.js";
import { renderHexagon } from "./hexagon.js";
import { NOTIFY, TRAY } from "./copy.js";
import { notifyTime, plural } from "./format.js";
import { fid } from "../fixtures/frames.js";

/**
 * The only actions a banner may offer. An id not in here throws, which makes this the list to edit
 * when the design gains one — and the place a reviewer looks to see that it did not gain `delete`.
 */
export const SAFE_ACTIONS = new Set(["keep", "review", "compare", "later", "retry", "open"]);

/**
 * The deck's own destructive vocabulary, from the hard rule's three sentences plus the words the
 * deletions screen uses for the same acts. Matched against both the id and the label: a `Delete
 * permanently` button reaching this surface is the failure the whole screen exists to prevent.
 */
const DESTRUCTIVE =
  /\b(delete|deleting|deleted|discard|approve|remove|removing|trash|permanently|overwrite|replace)\b/i;

/** Whether a word belongs to something irreversible. Exported so the test can prove it catches. */
export const isDestructive = (text) => DESTRUCTIVE.test(String(text ?? ""));

function safeActions(actions) {
  for (const action of actions) {
    if (!SAFE_ACTIONS.has(action.id)) {
      throw new Error(`notification: "${action.id}" is not a safe banner action`);
    }
    if (isDestructive(action.id) || isDestructive(action.label)) {
      throw new Error(`notification: destructive action "${action.label}" may not appear in a banner`);
    }
  }
  // Exactly the drawn shape: a leading action and at most one behind it. `firstSync` passes none.
  if (actions.length > 2) throw new Error("notification: a banner draws at most two actions");
  return actions;
}

const primary = (id, label) => ({ id, label, role: "primary" });
const secondary = (id, label) => ({ id, label, role: "secondary" });

// ------------------------------------------------------------------------------ builders ----

/**
 * One banner spec per event. `body` is a list of segments so the deletion sentence can carry its
 * path in mono inside itself, which is how the frame breaks its text nodes.
 *
 * `quiet` is the body's tier: `#C9D0DA` normally, `#99A2AE` "when nothing is wrong"
 * (`11-notifications.md` §Banner spec). Only the first-sync banner is quiet.
 */
const BUILDERS = {
  /**
   * Something would be deleted permanently. `paths` are the queued permanent deletions.
   *
   * Grouping puts the count in the TITLE here and not in the mark — that is what the frame draws,
   * and it is the one banner whose count is a number of files rather than of decisions. The body
   * names the first path; naming all of them would be the list `Review` opens.
   */
  deletion: ({ paths, entity = null }) => ({
    kind: "deletion",
    icon: { state: "needsDot", tone: "destructive" },
    // `entity` is the queue's own `entity_kind` where every item agrees on one — a folder deletion
    // saying "1 file" would be wrong about the thing it is asking you to keep. Null where they
    // differ, which falls back to the deck's `file`/`files`.
    title: NOTIFY.deletionTitle(paths.length, entity ? plural(paths.length, entity, `${entity}s`) : null),
    body: [{ mono: paths[0] }, { text: NOTIFY.deletionBodyAfter }],
    actions: safeActions([primary("keep", NOTIFY.deletionKeep), secondary("review", NOTIFY.deletionReview)]),
  }),

  /**
   * A file changed on both sides. One conflict draws the path; several draw the count as the
   * hexagon's numeral — `11-notifications.md` §Grouping, and the reason a second banner is never
   * stacked.
   */
  conflict: ({ paths }) =>
    paths.length === 1
      ? {
          kind: "conflict",
          icon: { state: "needsDot", tone: "decision" },
          title: NOTIFY.conflictTitle(paths[0]),
          body: [{ text: NOTIFY.conflictBody }],
          actions: safeActions([
            primary("compare", NOTIFY.conflictCompare),
            secondary("later", NOTIFY.later),
          ]),
        }
      : {
          kind: "conflict",
          // 44uu at a 34px mark, where the rest of the design draws 34uu. Measured off `11a Grouped`.
          icon: {
            state: "needsNumeral",
            tone: "decision",
            numeral: paths.length,
            numeralSize: 44,
            numeralY: 74,
          },
          title: NOTIFY.groupedTitle(paths.length),
          body: [{ text: NOTIFY.groupedBody(paths.length * 2) }],
          actions: safeActions([primary("review", NOTIFY.groupedAction), secondary("later", NOTIFY.later)]),
        },

  /** The first sync finished. The one good-news trigger, and the only banner with no actions. */
  firstSync: ({ files = null, size = null }) => ({
    kind: "firstSync",
    icon: { state: "settled" },
    title: NOTIFY.firstSyncTitle,
    body: [{ text: NOTIFY.firstSyncBody(files, size) }],
    quiet: true,
    actions: safeActions([]),
  }),

  /**
   * Nothing has synced for a day. `changes` is null when the count is unknown.
   *
   * TWO BODIES FOR ONE TITLE. The frame draws the expired-session sentence and the doc gives the
   * trigger three causes — "an outage, expired session, or full disk" — so saying "Proton Drive is
   * asking you to sign in again" about a full disk would put a false statement in the banner whose
   * job is to be believed. The other two causes take the deck's own unreachable sentence, which is
   * already drawn (`10a Offline`) and already gated. Both open with the reassurance.
   */
  outage: ({ changes = null, cause = "auth" }) => ({
    kind: "outage",
    icon: { state: "unreachable" },
    title: NOTIFY.outageTitle,
    body: [{ text: cause === "auth" ? NOTIFY.outageBody(changes) : TRAY.unreachableBody(changes) }],
    actions: safeActions([primary("retry", NOTIFY.outageRetry), secondary("open", NOTIFY.outageOpen)]),
  }),
};

/** Build the spec for one event. Throws on an unknown kind rather than drawing an empty banner. */
export function bannerFor(event) {
  const build = BUILDERS[event?.kind];
  if (!build) throw new Error(`notification: unknown event "${event?.kind}"`);
  return build(event);
}

/** The four kinds, in the order `11-notifications.md` lists them. Used by the policy and the tests. */
export const EVENT_KINDS = Object.keys(BUILDERS);

/** Flatten a spec for a notification server: no markup, no segments. */
export function payloadFor(spec) {
  return {
    kind: spec.kind,
    summary: spec.title,
    body: spec.body.map((part) => part.text ?? part.mono).join(""),
    // The tray already ships these five names as symbolic SVGs, so the banner and the indicator
    // resolve the same icon — `11-notifications.md`: "so the banner and the tray agree".
    icon: TRAY_ICON[spec.kind],
    actions: spec.actions.map(({ id, label }) => ({ id, label })),
  };
}

/**
 * Which tray glyph each banner shows. Deletion and outage share the struck/filled destructive pair
 * the tray draws for `unreachable`; a conflict is the attention form. `icons.rs` owns the names.
 */
const TRAY_ICON = {
  deletion: "proton-sync-attention-symbolic",
  conflict: "proton-sync-attention-symbolic",
  firstSync: "proton-sync-uptodate-symbolic",
  outage: "proton-sync-offline-symbolic",
};

// -------------------------------------------------------------------------------- drawing ----

/**
 * Draw a banner. `at` is an epoch — the header renders it relative, so a spec is not a rendering.
 *
 * `width` is opt-in because the frames disagree deliberately: the three `11a In situ` banners fill a
 * 372px column and declare nothing, while `11a Outage`/`11a Grouped` are drawn at 520. A banner is
 * sized by the chrome around it.
 */
export function renderBanner(spec, { at = null, width = null, onAction = null, index = 0 } = {}) {
  const mark = renderHexagon({
    size: 34,
    family: "notification",
    flexNone: true,
    style: "margin-top:1px",
    ...spec.icon,
  });
  fid(mark, "bannerMark", index);
  // Read off the mark rather than predicted from the state: a form that stopped emitting its circle
  // stamps one fewer node and the gate reports the absence. Same discipline as the glyph sheet.
  mark.querySelectorAll("path").forEach((path, j) => fid(path, "bannerMarkPath", index, j));
  const dot = mark.querySelector("circle");
  if (dot) fid(dot, "bannerMarkDot", index);
  const numeral = mark.querySelector("text");
  if (numeral) fid(numeral, "bannerMarkNumeral", index);

  const time = fid(el("span", { class: "notify-time" }, notifyTime(at)), "bannerTime", index);
  const title = fid(el("div", { class: "notify-title" }, spec.title), "bannerTitle", index);
  const body = fid(
    el(
      "div",
      { class: ["notify-body", spec.quiet ? "is-quiet" : null].filter(Boolean).join(" ") },
      ...spec.body.map((part) =>
        part.mono ? fid(el("span", { class: "notify-path" }, part.mono), "bannerPath", index) : part.text,
      ),
    ),
    "bannerBody",
    index,
  );

  const head = fid(
    el(
      "div",
      { class: "notify-head" },
      mark,
      fid(
        el(
          "div",
          { class: "notify-text" },
          fid(
            el(
              "div",
              { class: "notify-meta" },
              fid(el("span", { class: "notify-app" }, NOTIFY.app), "bannerApp", index),
              fid(el("span", { class: "notify-spacer" }), "bannerSpacer", index),
              time,
            ),
            "bannerMeta",
            index,
          ),
          title,
          body,
        ),
        "bannerText",
        index,
      ),
    ),
    "bannerHead",
    index,
  );

  // No actions row at all when there are none — `11a In situ`'s third banner has one child, not an
  // empty second one, and the frame's node keys say so (`div`, not `div[0]`).
  const actions = spec.actions.length
    ? fid(
        el(
          "div",
          { class: "notify-actions" },
          ...spec.actions.map((action, j) =>
            fid(
              el(
                "button",
                {
                  class: ["notify-action", action.role === "primary" ? "is-primary" : null]
                    .filter(Boolean)
                    .join(" "),
                  type: "button",
                  onClick: onAction ? () => onAction(action.id, spec) : null,
                },
                action.label,
              ),
              "bannerAction",
              index,
              j,
            ),
          ),
        ),
        "bannerActions",
        index,
      )
    : null;

  return fid(
    el("div", { class: "notify-banner", style: width ? `width:${width}px` : null }, head, actions),
    "banner",
    index,
  );
}
