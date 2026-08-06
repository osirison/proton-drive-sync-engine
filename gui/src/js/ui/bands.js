// Bands (F5). The tinted, bordered blocks that interrupt a screen to say something is waiting, is
// about to be lost, or needs agreeing to.
//
// MEASURED out of the fixtures for the tints and boxes, and out of the prototype for the padding
// and the internal rhythm. Three things the census settled:
//
//   · TONE AND DENSITY ARE INDEPENDENT, the same shape controls.js found for kind and size. Ten
//     band surfaces across three hues, and the alphas move per SITE, not per hue: the compact panel
//     steps every one down a notch (crimson .04→.035, .30→.26; red .06→.05, .38→.32). F5 builds the
//     four full-density bands; the compact pair belongs to F6. DEVIATIONS §52a.
//   · NO BAND IS EVER A SOLID FILL. Every one of the ten is a translucent tint over the screen's
//     own surface. The only solid red in the app is the armed `Delete permanently` button, and a
//     band that filled would read as that button's larger sibling.
//   · THE WARM ONE IS NOT A QUIET CRIMSON. `7a Activity quiet`'s never-synced band is amber because
//     nothing in it is at risk — the screen's own closing line is "Nothing here is at risk, it's
//     just not backed up." Four skipped temp files in a crimson band would be a lie about severity.
//
// AND NO BAND HOLDS A DESTRUCTIVE ACTION. `11a Rules` states it for banners — *Delete. Discard a
// version. Approve all. Anything irreversible* never gets a button in one — and the four bands here
// hold the same line: they route you to the screen that owns the decision. `Review`, `Compare`,
// `Show them`, `Leave it alone`. Nothing here deletes anything.

import { el } from "./el.js";
import { button } from "./controls.js";
import { dot } from "./rows.js";

/**
 * Which tokens each tone reaches for. Only the four F5 builds — see the module note above for the
 * six the census found that belong to other work packages.
 */
const TONE = {
  /** `2a Needs you`. The band that says a person is needed, and how many times. */
  attention: { bg: "var(--decision-band-bg)", border: "var(--decision-band-border)" },
  /** `5a Plan`. Something in this plan will not come back. */
  destructive: { bg: "var(--destructive-bg)", border: "var(--destructive-border)" },
  /** `7a Activity quiet`. Present, not backed up, not at risk. */
  warn: { bg: "var(--warn-band-bg)", border: "var(--warn-band-border)" },
  /** `9a Consent`. The one thing to agree to before continuous sync starts. */
  consent: { bg: "var(--decision-card-bg)", border: "var(--decision-card-border)" },
};

function toneOf(tone) {
  const found = TONE[tone];
  if (!found) throw new Error(`bands: unknown tone "${tone}". Known: ${Object.keys(TONE).join(", ")}`);
  return found;
}

/** The tinted box every band is. Layout and padding come from the class the caller's builder sets. */
function shell(tone, cls, children) {
  const node = el("div", { class: `band ${cls}` }, children);
  const { bg, border } = toneOf(tone);
  node.style.setProperty("--band-bg", bg);
  node.style.setProperty("--band-border", border);
  return node;
}

/**
 * `2a Needs you`'s attention band — ONE box holding every waiting item, split by internal rules.
 * Not one band per item: two conflicts and a deletion queue are one interruption, and three stacked
 * boxes would read as three.
 *
 * The band is ADDITIVE OVER the syncing screen and never a replacement for it (S1). Sync carries on
 * around whatever is waiting; that is the whole promise the copy makes.
 *
 * Each item's dot carries the severity, and the two drawn ones differ in kind rather than in
 * brightness: a conflict is an outlined `decision` ring, a deletion is a solid `destructive` fill.
 * An outline is a choice still open, a fill is a thing that will happen.
 */
export function attentionBand({ items = [] } = {}) {
  return shell(
    "attention",
    "band-attention",
    items.map((item) =>
      el(
        "div",
        { class: "band-item" },
        dot({ tone: item.tone ?? "decision", size: 7 }),
        el(
          "div",
          { class: "band-item-body" },
          el("div", { class: "band-item-title" }, item.title),
          item.note ? el("div", { class: "band-item-note" }, item.note) : null,
        ),
        item.action ?? null,
      ),
    ),
  );
}

/**
 * A single-row band: a mark, a sentence, and the button that takes you where the decision lives.
 * `5a Plan`'s destructive band and `7a Activity quiet`'s never-synced band.
 *
 * Every value here moves with the tone rather than with a size — padding, radius, gap, both type
 * sizes and both text colours all differ between the two. That is not inconsistency in the design;
 * a band that says *one file gets deleted for good* is doing a different job from one that says
 * *four files are never synced*, and it is drawn a size louder in every dimension.
 *
 * `mark` takes a built node rather than a name: `5a Plan` uses F2's 34px warning hexagon
 * (`renderHexagon({ size: 34, state: "warning" })`) and `7a Activity quiet` a plain `⊘` glyph.
 * Nothing is gained by teaching this module to build either one.
 *
 * NOT `8a Deletions tab` or `11a Rules`, both of which are tinted the same way and are neither of
 * these. The first is controls.js's `radioCard` wearing a destructive tone; the second is a prose
 * callout on a spec sheet, with no glyph and no button at all.
 */
export function noticeBand({ tone = "destructive", mark = null, title, note = null, action = null } = {}) {
  toneOf(tone);
  return shell(tone, `band-notice band-notice-${tone}`, [
    mark,
    el(
      "div",
      { class: "band-notice-body" },
      el("div", { class: "band-notice-title" }, title),
      note ? el("div", { class: "band-notice-note" }, note) : null,
    ),
    action,
  ]);
}

/**
 * `9a Consent`'s panel. A heading, the sentence that explains what agreeing means, and a divided
 * footer holding the checkbox.
 *
 * The only band with no button, and that is the point: CONSENT COMES AFTER THE MERGE and continuous
 * sync does not begin until the box is checked (S7). The action that proceeds lives in the footer
 * action bar, outside this panel, so nothing inside the tint can be clicked by reflex.
 *
 * `footer` takes a built control — controls.js's `checkbox` is the drawn one.
 */
export function consentPanel({ title, body, footer = null } = {}) {
  return shell("consent", "band-consent", [
    el("div", { class: "band-consent-title" }, title),
    el("div", { class: "band-consent-body" }, body),
    footer ? el("div", { class: "band-consent-footer" }, footer) : null,
  ]);
}

/**
 * The never-synced band's `⊘`, amber at 13px.
 *
 * A builder rather than a documented class, so the one place that knows the glyph and the one place
 * that knows its colour are the same place. It is a bare character, not an icon: the design's other
 * inline marks (`→`, `←`, `＋`, `↷`) are too, and an SVG here would be the only exception.
 */
export function warnGlyph(glyph = "⊘") {
  return el("span", { class: "band-glyph" }, glyph);
}

/**
 * The decision button inside an attention band item — `Compare`, `Review`.
 *
 * A builder rather than a note telling the caller to pass `padding:"8px 15px"`, for the same reason
 * `keepButton` and `trashButton` are builders: controls.js writes padding INLINE, so no rule in
 * `bands.css` can correct a caller who reaches for a plain `button()`. Where the stylesheet cannot
 * fix it, the geometry has to live somewhere a caller cannot get wrong.
 *
 * Always a `decision`, never a `destructive` — see the module note. A band routes; it does not act.
 */
export function bandButton({ label, onClick = null } = {}) {
  return button({
    kind: "decision",
    size: "small",
    label,
    onClick,
    padding: "8px 15px",
    radius: "var(--r-8)",
    fontSize: "12.5px",
    class: "band-action",
  });
}
