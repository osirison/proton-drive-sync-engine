// The dialog layer (F5). The last of the four modules, and the only one that is not a pure
// addition: the other three build nodes a screen drops into place, this one changes how the shell
// renders.
//
// WHAT F4 ALREADY OWNS, and what this must not duplicate. `app.js` has `openOverlay`/`closeOverlay`,
// the focus-key machinery that returns focus to whatever opened a dialog, and the `Escape` branch
// with its precedence chain — menu, then overlay, then the screen's own `shell:cancel`. A dialog
// that adds its own `Escape` listener gives one keypress two effects: the dialog closes AND the
// screen behind it cancels. So there is no key handling here beyond the Tab cycle. `Esc` reaches
// this layer through F4, or it does not reach it at all.
//
// WHAT F4 COULD NOT BUILD. There was no layer: `app.js` swapped the body wholesale when the route
// changed, so an "overlay" replaced the screen instead of stacking over it. F4's own note on the
// `details` route says what it wanted — *"5a/6a draw it as a panel over the screen you were on, not
// as a destination. Clicking it must not lose your place."* This module is that, and the render
// change it needs is one branch, because F4 already keys the header and footer off `route` rather
// than off the overlay.
//
// NOT EVERY OVERLAY IS A DIALOG, which is the finding that shaped the routing. Of the seven routes
// F4 marked `kind: "overlay"`, only three are drawn as something floating over a screen — `details`
// (`6a Details`), `neverSynced` (`7a Never synced`) and `saveRefused` (`8a Save refused`), each a
// standalone surface with no app header and no footer doors. The other four are full 1042x766
// windows that keep the header and the four doors and replace only the content area: `3a Conflict`,
// `4a Deletions`, `4a Armed`, and the onboarding takeover. Those need no scrim, no trap and no layer
// — the body swap F4 already does is exactly right for them. DEVIATIONS §57.
//
// SIZES ARE WRITTEN AS DRAWN, not as declared. §48 established that the prototype's surfaces come
// out 2px larger than their nominal size because they do not opt into `border-box` — but four of
// the ten dialogs DO opt in, so `520` is drawn 522 while `600` is drawn 600. There is no offset to
// apply, only a number to read off the frame. §48a.

import { el } from "./el.js";
import { button } from "./controls.js";

/**
 * Tones. `plain` is every dialog that is telling you something; `refusal` is the two that are
 * telling you something DID NOT HAPPEN, and they carry a crimson border for it — `8a Save refused`
 * and `9a CLI missing`. Nothing in the prose mentions this; both frames draw it.
 *
 * It is a border and never a fill. A refusal is not a destructive act — "nothing was saved, your
 * old settings are still running" is the reassurance the copy leads with — so the dialog is edged
 * in crimson rather than tinted like a band.
 */
const TONE = {
  plain: "var(--border)",
  quiet: "var(--border-subtle)",
  refusal: "var(--decision-card-border)",
};

/**
 * A dialog surface, with its scrim.
 *
 * `width` and `height` are the DRAWN numbers. `height: null` lets the dialog size to its content,
 * which is what the four `box-sizing:border-box` frames do.
 *
 * `label` names the dialog for a screen reader when it has no visible title; `labelledBy` points at
 * the id of the one it does have. EXACTLY ONE, enforced rather than documented: ARIA gives
 * `aria-labelledby` precedence, so passing both is not an error that surfaces — it is a `label` that
 * silently does nothing, and the caller who wrote it believes the dialog is named the other way.
 *
 * Neither is a throw. A modal announcing itself as "dialog" and nothing else is an accessibility
 * defect rather than a degraded experience, and for `8a Save refused` it would drop the entire
 * reason the thing opened. Better to fail in the one place that can still be fixed cheaply.
 */
export function dialog({
  width,
  height = null,
  tone = "plain",
  padding = null,
  label = null,
  labelledBy = null,
  children = null,
} = {}) {
  if (!width) throw new Error("dialog: width is required — write the drawn number, see §48a");
  const border = TONE[tone];
  if (!border) throw new Error(`dialog: unknown tone "${tone}". Known: ${Object.keys(TONE).join(", ")}`);
  if (!label && !labelledBy)
    throw new Error("dialog: needs a `label` or a `labelledBy` — see the note above");
  if (label && labelledBy)
    throw new Error("dialog: pass `label` OR `labelledBy`, not both — aria-labelledby would win silently");

  const surface = el(
    "div",
    {
      class: `dialog dialog-${tone}`,
      role: "dialog",
      "aria-modal": "true",
      "aria-label": label,
      "aria-labelledby": labelledBy,
      // Focusable so the trap has somewhere to put focus when a dialog opens with no control in it
      // — `5a Checking` has exactly one button and `4a Empty` has none at all. -1 keeps it out of
      // the Tab order itself.
      tabindex: "-1",
    },
    children,
  );
  surface.style.setProperty("--dialog-border", border);
  surface.style.width = `${width}px`;
  if (height != null) surface.style.height = `${height}px`;
  if (padding != null) surface.style.padding = padding;

  return el("div", { class: "dialog-scrim" }, surface);
}

/**
 * The title row: a heading, an optional sub-line, and the ✕.
 *
 * Two rungs, and they track the width family rather than anything semantic — `6a Details` at 520 is
 * `20px 22px` with a 16px title, `7a Never synced` at 600 is `22px 24px` with an 18px title over a
 * sub-line. The ✕ is identical in both, down to the 7px radius.
 */
export function dialogHead({ title, subtitle = null, onClose = null, size = "compact", id = null } = {}) {
  const heading = el("div", { class: "dialog-title", id }, title);
  return el(
    "div",
    { class: `dialog-head dialog-head-${size}` },
    subtitle
      ? el("div", { class: "dialog-headings" }, heading, el("div", { class: "dialog-subtitle" }, subtitle))
      : heading,
    onClose ? closeButton({ onClick: onClose }) : null,
  );
}

/**
 * The scrolling middle of a dialog, and the ruled row at its foot.
 *
 * `padding` and `margin` are inline rather than a rung per dialog, because the two frames that draw
 * them agree on nothing except the border: `7a Never synced` is `0 24px` over a 20px gap with an
 * 18px-deep foot, `6a Details` is `16px 22px 0` over 12px with a 16px-deep foot. Two rungs for two
 * callers is a table pretending to be a pattern; what genuinely repeats — `flex:1`, the divider,
 * the 12px gap — is in `dialog.css`, and the rest is measured per dialog at the call site.
 */
export function dialogBody({ padding = null, marginTop = null, children = [] } = {}) {
  const node = el("div", { class: "dialog-body" }, children);
  if (padding != null) node.style.padding = padding;
  if (marginTop != null) node.style.marginTop = marginTop;
  return node;
}

export function dialogFoot({ padding = null, marginTop = null, children = [] } = {}) {
  const node = el("div", { class: "dialog-foot" }, children);
  if (padding != null) node.style.padding = padding;
  if (marginTop != null) node.style.marginTop = marginTop;
  return node;
}

/**
 * The ✕. A `secondary` at a size nothing else in the app uses — 26x26 at `--r-7` — so it overrides
 * rather than earning a rung on controls.js's ladder.
 *
 * It carries a real label because `✕` announces as nothing useful. Esc closes the dialog too, via
 * F4; this is the pointer affordance and the Tab-reachable one.
 */
export function closeButton({ onClick = null, label = "Close" } = {}) {
  return button({
    kind: "secondary",
    size: "icon",
    label: "✕",
    onClick,
    padding: "1px 6px",
    radius: "var(--r-7)",
    fontSize: "12px",
    class: "dialog-close",
    "aria-label": label,
  });
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Trap Tab inside the dialog and put focus in it. Returns a detach function.
 *
 * ONLY Tab. Escape belongs to F4 — see the module note. Shift+Tab wraps the other way, because a
 * trap that only works forwards sends the first control's backwards-Tab to the browser chrome.
 *
 * Focus goes to the first focusable control IN DOM ORDER, or to the surface itself when there is
 * none. No preference logic, and the design makes that come out right on its own: the two dialogs
 * with a ✕ (`6a Details`, `7a Never synced`) are informational, so opening on "dismiss this" is
 * both conventional and the likeliest next action — while `8a Save refused` and `9a CLI missing`,
 * the two that ask you to choose, have no ✕ at all and open on `Go back and fix it`. A dialog that
 * wants some other control focused should order it first, not ask this for an exception.
 *
 * Nothing here restores focus on close — `closeOverlay` in app.js already does, through a key that
 * survives the opener being rebuilt. Doing it here as well would fight it.
 */
export function focusTrap(scrim) {
  const surface = scrim.querySelector(".dialog") ?? scrim;
  const list = () => [...surface.querySelectorAll(FOCUSABLE)].filter((n) => n.offsetParent !== null);

  const first = list()[0];
  (first ?? surface).focus();

  const onKeydown = (e) => {
    if (e.key !== "Tab") return;
    const items = list();
    if (!items.length) {
      e.preventDefault();
      return;
    }
    const edge = e.shiftKey ? items[0] : items[items.length - 1];
    // `document.activeElement` rather than e.target: the surface itself can hold focus, and it is
    // not in `items`, so comparing against the list would never match and the wrap never fire.
    if (document.activeElement === edge || !surface.contains(document.activeElement)) {
      e.preventDefault();
      (e.shiftKey ? items[items.length - 1] : items[0]).focus();
    }
  };

  surface.addEventListener("keydown", onKeydown);
  return () => surface.removeEventListener("keydown", onKeydown);
}
