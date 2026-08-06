// The window chrome (F4): header, status chip, ⋯ menu, footer nav, footer action bar.
// `02-shell.md` — "Every window shares this skeleton. Build it once."
//
// MEASURED out of every in-scope 1040 frame by rendering the prototype and reading the laid-out
// boxes, the same method F3 used for the seam and for the same reason: a 52px header is a number
// you can read out of the source, but "does this screen have a footer nav" is a fact about the
// tree, and the padding that distinguishes four otherwise-identical footers is layout. Departures
// are in DEVIATIONS.md §40–§45. The load-bearing ones:
//
//   · THE DOORS ARE NOT ON EVERY SCREEN. Each in-scope 1040 frame carries either the four doors or
//     a footer action bar — 13 to 6, never both. See routes.js.
//   · THE HEADER DIMS OFF THE CHIP, NOT THE SCREEN. `02-shell.md` says the mark is dimmed "on
//     settled/secondary screens", which is a judgement call per screen. Measured, it is exactly
//     `chip === "idle"`: all six idle frames dim, all fourteen non-idle frames do not.
//   · ONBOARDING DROPS THE ⋯ BUTTON, not just the chip. Both `9a` frames have four header slots.
//   · THE DECISION RING IS 1px, not the 2px §2 states — in both themes.

import { el } from "./el.js";

// ------------------------------------------------------------------------- the status chip ----

/**
 * The five variants of `02-shell.md` §"Status chip", plus the sixth the doc describes in prose but
 * leaves out of the table: onboarding's `step N of 2`.
 *
 * `dot: null` is a real value, not a missing one — `rehearsal` is text only, and so is `step`.
 * Written as an explicit key so it cannot be confused with "not measured yet".
 */
const CHIP = {
  idle: { border: null, text: "var(--text-label)", dot: { fill: "var(--dot-inert)" } },
  syncing: {
    border: "var(--border-chrome)",
    text: "var(--text-3)",
    dot: { fill: "var(--up-to)", blip: true },
  },
  // Two attention variants that differ ONLY in their dot: a decision is a ring (something to open),
  // a deletion is solid (something that will happen). Same border, same text colour.
  decisions: {
    border: "var(--chip-attention-border)",
    text: "var(--decision-text)",
    dot: { ring: "var(--decision)" },
  },
  deletions: {
    border: "var(--chip-attention-border)",
    text: "var(--decision-text)",
    dot: { fill: "var(--destructive)" },
  },
  rehearsal: { border: "var(--border-chrome)", text: "var(--text-3)", dot: null },
  step: { border: null, text: "var(--text-5)", dot: null },
};

/** Is this chip the quiet one? Drives the header's dimming — see renderHeader. */
export const isQuietChip = (variant) => variant === "idle";

/**
 * `padding:5px 12px`, `border-radius:99px`, mono 11px, `gap:7px`, 6px dot. The dot measures 8.8px
 * on the syncing frames only because `blip` scales it 1.5× and the reading caught it mid-cycle; it
 * is a 6px dot in the markup, in every variant that has one.
 */
export function statusChip(variant, text) {
  const spec = CHIP[variant];
  if (!spec)
    throw new Error(
      `chrome: unknown status-chip variant "${variant}". Known: ${Object.keys(CHIP).join(", ")}`,
    );
  const chip = el("span", { class: `chip chip-${variant}`, "data-variant": variant });
  if (spec.border) chip.style.border = `1px solid ${spec.border}`;
  chip.style.color = spec.text;
  if (spec.dot) {
    const dot = el("span", { class: "chip-dot" + (spec.dot.blip ? " chip-dot-blip" : "") });
    // A ring is transparent with a 1px stroke; a filled dot has no border at all. Modelled as two
    // shapes rather than one with a colour swap, because the ring's meaning is "open this".
    if (spec.dot.ring) dot.style.border = `1px solid ${spec.dot.ring}`;
    else dot.style.background = spec.dot.fill;
    chip.append(dot);
  }
  chip.append(el("span", { class: "chip-text" }, text));
  return chip;
}

// ------------------------------------------------------------------------------- the header ----

/**
 * 52px, `flex:none`, `padding:0 20px`, `gap:12px`, **no bottom border** — the header floats on the
 * surface. Slots: mark · name · flex spacer · chip · ⋯.
 *
 * `onMenu == null` drops the ⋯ button entirely, which is what onboarding draws. Passing a handler
 * that no-ops would leave a dead 30×30 target in a flow that has no menu.
 */
export function renderHeader({ chip = "idle", chipText = "idle", onMenu = null, onHome = null } = {}) {
  const quiet = isQuietChip(chip);
  const mark = el("img", {
    class: "app-mark" + (quiet ? " is-quiet" : ""),
    src: "assets/icon.svg",
    alt: "",
    width: 20,
    height: 20,
  });
  const name = el("span", { class: "app-name" + (quiet ? " is-quiet" : "") }, "Proton Drive Sync");

  // The mark is the home affordance, and on Settings/Plan/onboarding it is one of only two ways
  // back — so it has to be a real button, keyboard-reachable like everything else in a desktop app.
  // That wraps one node the frames draw bare; the button is sized to exactly 20×20 with no padding
  // so the header's own 12px gaps are unmoved. DEVIATIONS.md §42.
  const home = onHome
    ? el(
        "button",
        {
          class: "app-home",
          onClick: onHome,
          "aria-label": "Back to the main screen",
          title: "Back to the main screen",
        },
        mark,
      )
    : mark;

  return el(
    "header",
    { class: "shell-header" },
    home,
    name,
    el("span", { class: "shell-spacer" }),
    statusChip(chip, chipText),
    onMenu
      ? el("button", { class: "menu-btn", onClick: onMenu, "aria-label": "More", title: "More" }, "⋯")
      : null,
  );
}

/**
 * Patch a rendered header across a poll. Same reason as updateFooterNav: a rebuild every 2s makes
 * the ⋯ button unfocusable and restarts the syncing dot's `blip` mid-cycle, which is precisely the
 * failure `updateHexagon` was built to avoid one primitive earlier.
 *
 * The chip node is REPLACED only when its variant changes, and its text patched otherwise — so a
 * count ticking from 2 to 3 does not restart the animation on a dot that has one.
 *
 * Returns false when the header's shape changed (the ⋯ or the home button appearing or
 * disappearing), which is the caller's signal to rebuild.
 */
export function updateHeader(
  header,
  { chip = "idle", chipText = "idle", hasMenu = true, hasHome = false } = {},
) {
  if (Boolean(header.querySelector(".menu-btn")) !== hasMenu) return false;
  if (Boolean(header.querySelector("button.app-home")) !== hasHome) return false;

  const quiet = isQuietChip(chip);
  header.querySelector(".app-mark").classList.toggle("is-quiet", quiet);
  header.querySelector(".app-name").classList.toggle("is-quiet", quiet);

  const current = header.querySelector(".chip");
  if (current.dataset.variant === chip) {
    const text = current.querySelector(".chip-text");
    if (text.textContent !== chipText) text.textContent = chipText;
  } else {
    current.replaceWith(statusChip(chip, chipText));
  }
  return true;
}

// --------------------------------------------------------------------------- the footer nav ----

/**
 * Four variants, measured. Distinguished only by padding and by whether the mono line beneath is
 * drawn — `02-shell.md` gives `0 40px 18–22px` / `padding-top:14–20px` as ranges, and
 * IMPLEMENTATION-PLAN §1.3 conflict 7 already reads them as per-frame. All four are drawn, so this
 * is a table and not a range to pick from, exactly like the seam's `-114`/`-150` pair (§33a).
 */
const FOOTER_NAV = {
  /** `2a Settled` · `2a Syncing` + light twins — the only ones with the mono line. */
  withLine: { bottom: 22, top: 20, line: true },
  /** `2a Needs you` — the attention band takes the line's place. */
  banded: { bottom: 20, top: 16, line: false },
  /** `3a` · `4a` · `6a` + light twins. The majority variant, and absent from the doc. */
  standard: { bottom: 18, top: 15, line: false },
  /** `7a Activity quiet` · `7a File lookup` — one pixel tighter, and the one the doc names. */
  tight: { bottom: 18, top: 14, line: false },
};

/**
 * The four doors. 13px/400, `gap:34px`, centred, `border-top:1px var(--divider)`.
 *
 * `order` is FOOTER_ORDER and nothing may reorder it. The design's testing checklist carries
 * "Footer's four doors never move or reorder" as a line item — it is the promise that replaces the
 * v1 sidebar, and a user who learns where Settings is must never have to look again.
 *
 * There are no badge counts here. A waiting decision is announced by the status chip and the
 * attention band; putting a number in navigation is what the v1 sidebar did.
 */
export function renderFooterNav({
  order,
  active = null,
  labels,
  onNavigate,
  variant = "standard",
  line = null,
}) {
  const spec = FOOTER_NAV[variant];
  if (!spec)
    throw new Error(
      `chrome: unknown footer-nav variant "${variant}". Known: ${Object.keys(FOOTER_NAV).join(", ")}`,
    );
  if (line != null && !spec.line) {
    throw new Error(
      `chrome: footer-nav variant "${variant}" is not drawn with the mono line — use "withLine"`,
    );
  }

  const bar = el("div", { class: "footer-nav-bar" });
  bar.style.paddingTop = `${spec.top}px`;
  for (const id of order) {
    bar.append(
      el(
        "button",
        {
          class: "door" + (id === active ? " is-active" : ""),
          // The route id, so the shell can patch the active door across a poll instead of
          // rebuilding the footer — see updateFooterNav.
          "data-route": id,
          onClick: () => onNavigate(id),
          "aria-current": id === active ? "page" : null,
        },
        labels[id],
      ),
    );
  }

  const nav = el(
    "nav",
    { class: "footer-nav" },
    bar,
    spec.line && line ? el("div", { class: "footer-line" }, line) : null,
  );
  nav.style.paddingBottom = `${spec.bottom}px`;
  return nav;
}

/**
 * Patch a rendered footer nav across a poll: the active door, and the mono line's text.
 *
 * This exists for the same reason `updateHexagon` does, and its absence was a worse bug than the
 * one that motivated that. The shell re-renders on every status poll (~2s); rebuilding the footer
 * destroys the button the user is standing on, so **keyboard focus dropped to `<body>` within 1.2
 * seconds** of tabbing to a door — measured, not theorised. `14-behaviour-and-state.md`: "Every
 * control must be keyboard-reachable — this is a desktop app."
 *
 * Returns false when the change is structural (a different padding variant, or the line appearing
 * or disappearing), which is the caller's signal to rebuild instead.
 */
export function updateFooterNav(nav, { active = null, variant = "standard", line = null } = {}) {
  const spec = FOOTER_NAV[variant];
  if (!spec) return false;
  if (`${spec.bottom}px` !== nav.style.paddingBottom) return false;
  const lineNode = nav.querySelector(".footer-line");
  if (Boolean(spec.line && line) !== Boolean(lineNode)) return false;

  for (const door of nav.querySelectorAll("[data-route]")) {
    const isActive = door.dataset.route === active;
    door.classList.toggle("is-active", isActive);
    if (isActive) door.setAttribute("aria-current", "page");
    else door.removeAttribute("aria-current");
  }
  if (lineNode && lineNode.textContent !== line) lineNode.textContent = line;
  return true;
}

// -------------------------------------------------------------------- the footer action bar ----

/**
 * `padding:14px 32px`, `border-top:1px var(--divider)`, `gap:12px`. Order is always consequence
 * text left → `flex:1` → secondary → primary, and the primary is disabled until the screen's gate
 * passes.
 *
 * `bottom` exists because onboarding draws `14px 32px 18px`: it has no footer nav beneath it, so
 * the bar carries the window's own bottom margin. Every other screen with an action bar sits above
 * nothing either — but draws 14. Measured, not derived; both are in the frames.
 *
 * `9a Review` puts its secondary (`Back`) in the LEFT slot instead of beside the primary, which is
 * the one frame that departs from the stated order. `consequence` accepts a node for that reason.
 */
export function renderActionBar({
  consequence = null,
  tone = "quiet",
  secondary = null,
  primary = null,
  bottom = 14,
} = {}) {
  const left =
    consequence == null
      ? null
      : consequence.nodeType
        ? consequence
        : el("span", { class: `bar-consequence tone-${tone}` }, consequence);
  const bar = el(
    "div",
    { class: "footer-action-bar" },
    left,
    el("span", { class: "shell-spacer" }),
    secondary,
    primary,
  );
  if (bottom !== 14) bar.style.paddingBottom = `${bottom}px`;
  return bar;
}

// ------------------------------------------------------------------------------ placeholder ----

/**
 * The body of a screen whose S-task has not built it yet. F4 replaces the seven v1 screens with
 * this: they were styled entirely by app.css/components.css, which this commit deletes, so leaving
 * them in place would ship unstyled markup rather than an honest "not built yet".
 */
export function screenPlaceholder(title, issue) {
  return el(
    "div",
    { class: "screen-placeholder" },
    el("h2", {}, title),
    el("p", {}, "This screen is built by its own task on the shared foundation (F1–F4)."),
    issue ? el("div", { class: "issue mono" }, issue) : null,
  );
}
