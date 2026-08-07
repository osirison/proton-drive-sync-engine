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
  // NOT `width`/`height`. The prototype does not opt into `border-box` and base.css opts the app
  // in globally, so the SAME element reports `width:1000px` in one document and `width:1040px` in
  // the other while occupying an identical 1040px on screen. Comparing the computed property
  // measures the box model, not the design. Size is compared through `box` (getBoundingClientRect,
  // always the border box) which is model-independent and is what a person actually sees.
  // See frame-classes.mjs BORDER_BOX_INSET and DEVIATIONS.md §48.
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

/**
 * SVG presentation attributes, read as attributes rather than computed styles — with the exception
 * of the two that carry a COLOUR. See `compareSvgAttr`.
 */
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
 * The two SVG attributes that hold a colour, and therefore cannot be compared as strings.
 *
 * ADDED IN F6, which was the first task to put a hexagon in front of this gate. The prototype writes
 * `stroke="#2E323A"`; the app writes `stroke="var(--hex-settled-track)"` and MUST, because
 * `tokens.css` is the only file allowed to carry a raw colour (`check-tokens.mjs` enforces it) and
 * because the light theme is a token swap — a literal hex here would be a mark that never changes
 * theme. Compared literally, those two can never be equal, so every hexagon in all 51 frames would
 * have failed and the honest fix would have looked like "stop theming the mark".
 *
 * So a colour attribute is compared the way every style property already is: as what the ENGINE
 * computes. `fill` and `stroke` are CSS properties, `var()` resolves in a presentation attribute,
 * and `getComputedStyle` returns `rgb(46, 50, 58)` on both sides. The fixture's recorded hex is
 * converted here rather than re-extracted, so `frames/*.json` is untouched.
 *
 * A `url(#id)` reference is equal to any other. The id names a gradient the app must make unique per
 * instance — `10a Glyph states` puts ten marks on one page and a fixed id makes the tenth resolve to
 * the first one's defs — so the id itself is not design. What the gradient CONTAINS is, and its
 * stops are driven from tokens; that is asserted on the stop nodes, not here.
 */
export const COLOUR_ATTRS = new Set(["stroke", "fill"]);

const HEX_COLOUR = /^#([0-9a-f]{3,8})$/i;

/** `#2E323A` / `#fff` / `#0A0B0DFF` → the `rgb()`/`rgba()` form an engine serialises, or null. */
function hexToRgb(value) {
  const m = HEX_COLOUR.exec(String(value ?? ""));
  if (!m) return null;
  let hex = m[1];
  if (hex.length === 3 || hex.length === 4) hex = [...hex].map((c) => c + c).join("");
  if (hex.length !== 6 && hex.length !== 8) return null;
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
  if (hex.length === 6) return `rgb(${r}, ${g}, ${b})`;
  // Alpha serialises as a number, and an engine drops a trailing zero (`0.50` → `0.5`).
  const a = Number((parseInt(hex.slice(6, 8), 16) / 255).toFixed(2));
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/**
 * Compare one SVG attribute. `computed` is the app's `getComputedStyle` value for the same
 * attribute, and is only supplied — and only meaningful — for the two colour attributes.
 */
export function compareSvgAttr(attr, expected, actual, computed = null) {
  if (expected === actual) return null;
  if (!COLOUR_ATTRS.has(attr)) return `${expected} vs ${actual}`;

  const ref = (v) => /^url\(/.test(String(v ?? ""));
  if (ref(expected) && ref(actual)) return null;

  const want = hexToRgb(expected);
  if (want != null && computed != null && want === computed) return null;
  return `${expected} vs ${actual}`;
}

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

/** A node key's parent key. Keys are slash paths, so this is textual. `""` is the frame root. */
const parentKey = (key) => (key.includes("/") ? key.slice(0, key.lastIndexOf("/")) : "");

/**
 * Which nodes of a frame may have their measured box compared across machines.
 *
 * `node.unbundled` is set by extract.mjs for text needing a glyph the bundled faces do not cover —
 * determined from base.css's own `unicode-range` declarations, so it follows F1's subsets rather
 * than a hand-written block list. Those glyphs come from whatever the machine has installed and
 * their advance widths differ by whole pixels.
 *
 * An unbundled glyph does not only corrupt its own width — IT MOVES ITS NEIGHBOURS. `10a Syncing`
 * draws a filename and a `→` in one flex row: the arrow measured 12px here and 10.06px on
 * ubuntu-latest, and the 1.94px it gave up landed on the filename beside it, which contains nothing
 * but Latin. So the rule follows the layout: **a box is comparable only if no unbundled glyph
 * appears anywhere inside its PARENT's subtree** — covering the node, every sibling sharing flex or
 * grid space with it, and every ancestor whose size sums it.
 *
 * Everything else about those nodes is still asserted: colour, padding, font-size, position, border.
 * Only the size they happen to occupy is not.
 */
export function boxComparability(nodes) {
  const tainted = new Set();
  for (const node of nodes) {
    if (!node.unbundled) continue;
    for (let key = node.key; ; key = parentKey(key)) {
      tainted.add(key);
      if (key === "") break;
    }
  }
  return (node) => !tainted.has(parentKey(node.key)) && !tainted.has(node.key);
}

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
