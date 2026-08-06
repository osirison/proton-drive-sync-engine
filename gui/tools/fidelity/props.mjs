// What the style gate compares, and how closely. Shared by extract.mjs (which records these off the
// prototype) and assert.mjs (which reads them off the app), so the two can never drift apart —
// a property recorded but not asserted is a silent hole in the gate.

/**
 * The property list from the F8 issue. Longhand only: `border`, `padding` and `margin` shorthands
 * serialise inconsistently between engines and collapse four sides into one string, which turns a
 * one-side failure into an unreadable diff.
 */
export const STYLE_PROPS = [
  // type
  "font-family",
  "font-size",
  "font-weight",
  "letter-spacing",
  "line-height",
  "text-transform",
  "text-align",
  // colour
  "color",
  "background-color",
  "background-image",
  "opacity",
  // box
  "border-top-width",
  "border-right-width",
  "border-bottom-width",
  "border-left-width",
  "border-top-color",
  "border-right-color",
  "border-bottom-color",
  "border-left-color",
  "border-top-style",
  "border-right-style",
  "border-bottom-style",
  "border-left-style",
  "border-top-left-radius",
  "border-top-right-radius",
  "border-bottom-right-radius",
  "border-bottom-left-radius",
  "padding-top",
  "padding-right",
  "padding-bottom",
  "padding-left",
  "margin-top",
  "margin-right",
  "margin-bottom",
  "margin-left",
  "width",
  "height",
  // layout
  "display",
  "flex-direction",
  "flex-grow",
  "flex-shrink",
  "flex-basis",
  "align-items",
  "justify-content",
  "gap",
  "grid-template-columns",
  "position",
  "top",
  "right",
  "bottom",
  "left",
  "overflow",
  // motion — the declaration only. Whether it LOOKS right is not checkable here; see the README.
  "animation-name",
  "animation-duration",
  "animation-delay",
  "animation-timing-function",
  "animation-iteration-count",
];

/** SVG presentation attributes, read as attributes rather than computed styles. */
export const SVG_ATTRS = [
  "d",
  "viewBox",
  "stroke",
  "stroke-width",
  "stroke-dasharray",
  "stroke-linecap",
  "stroke-linejoin",
  "fill",
  "x",
  "y",
  "cx",
  "cy",
  "r",
  "rx",
  "width",
  "height",
  "text-anchor",
];

/**
 * Properties whose computed value is a fixed constant when nothing sets them, and that constant.
 * A property at its initial value is OMITTED from `frames/*.json`, and assert.mjs reads a missing
 * property as this.
 *
 * The point of checking the fixtures in is that a prototype edit shows up as a reviewable diff. At
 * ~60 properties across 1999 nodes the recorded-everything form is 4.4 MB of mostly `margin-top:
 * 0px`, which is not a diff anyone reads. Omitting the defaults is lossless — "absent" has exactly
 * one meaning and it is written here — and turns the fixtures into something a human can scan.
 *
 * Inherited properties (`color`, `font-*`, `letter-spacing`, `text-align`) are deliberately NOT in
 * this table: their computed value depends on an ancestor, so there is no constant to compare
 * against and "absent" could not be resolved.
 */
export const INITIAL = {
  "background-color": "rgba(0, 0, 0, 0)",
  "background-image": "none",
  "border-top-width": "0px",
  "border-right-width": "0px",
  "border-bottom-width": "0px",
  "border-left-width": "0px",
  "border-top-style": "none",
  "border-right-style": "none",
  "border-bottom-style": "none",
  "border-left-style": "none",
  "border-top-left-radius": "0px",
  "border-top-right-radius": "0px",
  "border-bottom-right-radius": "0px",
  "border-bottom-left-radius": "0px",
  "padding-top": "0px",
  "padding-right": "0px",
  "padding-bottom": "0px",
  "padding-left": "0px",
  "margin-top": "0px",
  "margin-right": "0px",
  "margin-bottom": "0px",
  "margin-left": "0px",
  display: "block",
  "flex-direction": "row",
  "flex-grow": "0",
  "flex-shrink": "1",
  "flex-basis": "auto",
  "align-items": "normal",
  "justify-content": "normal",
  gap: "normal",
  "grid-template-columns": "none",
  position: "static",
  top: "auto",
  right: "auto",
  bottom: "auto",
  left: "auto",
  overflow: "visible",
  opacity: "1",
  "text-transform": "none",
  "animation-name": "none",
  "animation-duration": "0s",
  "animation-delay": "0s",
  "animation-timing-function": "ease",
  "animation-iteration-count": "1",
};

/** Resolve a recorded value, filling in the initial for a property the fixture omitted. */
export const valueOf = (styles, prop) => styles[prop] ?? INITIAL[prop];

/** Lengths compare at ±0.5px; everything else is exact after normalisation. */
export const LENGTH_TOLERANCE_PX = 0.5;

const LENGTH_PROP = /(width|height|top|right|bottom|left|size|spacing|radius|gap|padding|margin|basis)$/;

/**
 * `line-height:normal` is a WILDCARD, not a value.
 *
 * The prototype sets a line-height on very few nodes, so most of them compute to `normal`, which an
 * engine resolves to a number derived from the font's own metrics. Comparing that number across two
 * documents means comparing font loading, not design — and F1 already found that font metrics move
 * every measurement in this design. Treating it as an exact value makes the gate noisy on day one
 * and teaches everyone to ignore it, which is worse than not having it.
 */
export const isWildcard = (prop, value) => prop === "line-height" && value === "normal";

/**
 * Compare one property. Returns null when they agree, or a short reason when they do not.
 *
 * Colours are normalised by the engine already — both sides come out of `getComputedStyle`, so both
 * are `rgb()`/`rgba()` and compare as strings. `font-family` compares on the FIRST name only: the
 * fallback stack is a deployment detail, and the prototype's stack and the app's differ by design.
 */
export function compare(prop, expected, actual) {
  if (expected === actual) return null;
  if (isWildcard(prop, expected) || isWildcard(prop, actual)) return null;

  if (prop === "font-family") {
    const first = (s) =>
      String(s)
        .split(",")[0]
        .trim()
        .replace(/^["']|["']$/g, "");
    return first(expected) === first(actual) ? null : `${first(expected)} vs ${first(actual)}`;
  }

  if (LENGTH_PROP.test(prop)) {
    const px = (s) => (/^-?[\d.]+px$/.test(s) ? parseFloat(s) : NaN);
    const a = px(expected);
    const b = px(actual);
    if (!Number.isNaN(a) && !Number.isNaN(b)) {
      return Math.abs(a - b) <= LENGTH_TOLERANCE_PX ? null : `${expected} vs ${actual}`;
    }
  }

  return `${expected} vs ${actual}`;
}
