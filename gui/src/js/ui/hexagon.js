// The hexagon (F2) — one of the two load-bearing primitives of design-v2. Everything else on a
// screen is composition; getting this wrong is, per 01-foundations.md §6, "the single easiest way
// to make the redesign look off-brand".
//
// EVERY NUMBER HERE WAS MEASURED out of docs/design-v2/Drive Sync.dc.html, node by node, across all
// 53 in-scope hexagons — not transcribed from the prose. Where the two disagree the frame wins for
// per-element values (IMPLEMENTATION-PLAN.md §1.3 rule 2) and the difference is recorded in
// docs/design-v2/DEVIATIONS.md §20–§35. Several of those disagreements are load-bearing:
//
//   · The size→stroke relation is a LOOKUP TABLE, not a formula. It is not single-valued (80px is
//     drawn at both 4.4 and 4.6) and not monotonic (48→5.4 but the smaller 44→5), so no function
//     can reproduce it. Never interpolate: an unlisted size throws rather than guessing.
//   · The seam mask (`fill`) is ORTHOGONAL to state. Six settled/needs-you frames carry it and
//     seven syncing frames do not, so it is a caller flag, not a property of `syncing`.
//   · There is NO crimson hero. `2a Needs you` at 168px is byte-identical to `2a Syncing` but for
//     its gradient ids — 03-main-screen.md: "the count in the hexagon is transfers, not decisions".
//     The crimson mark exists only at ≤72px.
//   · The strike and warning-bar paths carry ONLY stroke-linecap — no `fill`, no `stroke-linejoin`.
//     Emitting `fill="none"` there is visually identical and fails the F8 style gate.
//
// Nothing is nested inside the mark: the outline IS the animation track. No ring, no circle, no
// inner arc, and no solid fill — 10-tray.md is explicit that only five forms exist.

const SVG_NS = "http://www.w3.org/2000/svg";

/**
 * SVG element builder. `el()` in components.js cannot be used here and cannot be extended to:
 * it calls document.createElement, which yields an HTML-namespace element that inspects correctly
 * and renders NOTHING, and its `class` branch assigns to node.className — readonly on a real
 * SVGElement, so it throws under ESM strict mode. Importing it would also close an ESM cycle once
 * components.js re-exports from here (import-x/no-cycle is an error for exactly that reason).
 */
function svgEl(tag, attrs = {}, ...children) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, val] of Object.entries(attrs)) {
    if (val == null || val === false) continue;
    node.setAttribute(key, String(val));
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}

// ---------------------------------------------------------------- geometry ----

export const HEX_VIEWBOX = "0 0 120 120";
export const HEX_PATH = "M60 9.4 L103.1 33.8 L103.1 86.3 L60 110.6 L16.9 86.3 L16.9 33.8 Z";

/**
 * 303.0115, not the 297 in 01-foundations.md §6 — that figure is 6 × 49.5275, which assumes a
 * REGULAR hexagon. This one is not: the two vertical sides are 52.5 units and the four slants
 * 49.5275 / 49.4783. See DEVIATIONS.md §24.
 *
 * The dash arrays are not tuned against it in any case. They sum to DASH_PERIOD, which equals the
 * stroke-dashoffset travel in the keyframes — that identity is what makes the loop seamless, since
 * one cycle advances the pattern exactly one period. The 3.0115-unit remainder leaves a permanent
 * stub of "on" at the path start (the top vertex). It is there in every drawn frame; retuning the
 * arrays to the true perimeter would remove it and break the F8 gate against those frames.
 */
export const HEX_PERIMETER = 303.0115;
export const DASH_PERIOD = 300;

const CHECK = "M49 60 L57 68 L72 52";
const STRIKE = "M40 40 L80 80";
const STRIKE_SMALL = "M38 38 L82 82"; // ≤20px — 10-tray.md's longer strike, so it reads at 16px

// ------------------------------------------------------------ stroke widths ----

// Measured per size. `window` is the in-window/product default; `tray` (10a) and `notification`
// (11a) are the two families that draw the same size at a different weight. A size drawn at two
// widths inside one family (80px: 9a Review 4.6 vs 4a Empty 4.4) has no default — the caller must
// pass `strokeWidth`, because picking one silently mis-draws the other.
const STROKE = {
  168: { window: 3.4 },
  116: { window: 4.4 },
  104: { window: 4.4, warning: 4.6 }, // 5a Checking 4.4 · 4a Armed 4.6
  96: { window: 4.2 }, // below §6's stated 4.4 floor — 3a Conflicts cleared
  88: { window: 4.4 },
  80: {}, // genuinely two-valued: 4.4 (4a Empty) / 4.6 (9a Review)
  76: { window: 4.6 },
  72: { window: 4.5, tray: 4.6 },
  64: { window: 5 }, // 4a Compact — absent from §6's size list
  52: { window: 5.4 },
  48: { window: 5.4 }, // 7a File pending — absent from §6's size list
  44: { window: 5 },
  34: { window: 6, notification: 7 },
  20: { tray: 9, window: 9 },
  15: { tray: 9, window: 9 },
  14: { tray: 9, window: 9 },
  13: { window: 12 },
};

/** The settled check is drawn at the track's width everywhere except the 168px hero (3.6 vs 3.4). */
const CHECK_STROKE = { 168: 3.6 };

/**
 * Resolve a stroke width. Throws rather than interpolating: §6 claims "the mark should read as the
 * same weight at every size", but as drawn the rendered stroke falls 4.76px → 1.05px while the
 * relative weight rises 2.8% → 10% (an optical-compensation ramp, not a constant), and the best
 * power-law fit still errs 17–18% — far outside the ±0.5px the fidelity gate allows. A guessed
 * width would be a plausible wrong number, which is worse than a loud failure.
 */
export function strokeForSize(size, family = "window") {
  const entry = STROKE[size];
  if (!entry) {
    throw new Error(
      `hexagon: no stroke width measured for size ${size}px. Sizes in the design: ${Object.keys(STROKE).join(", ")}. Pass an explicit strokeWidth to use another size.`,
    );
  }
  const width = entry[family] ?? entry.window;
  if (width == null) {
    throw new Error(
      `hexagon: size ${size}px is drawn at more than one width (${Object.values(STROKE[size]).join(", ") || "4.4 and 4.6"}) and has no default — pass an explicit strokeWidth.`,
    );
  }
  return width;
}

// ------------------------------------------------------------------ numerals ----

// font-size is in USER UNITS on the 120 viewBox, so it renders at size/120 × the value. It is not a
// function of size: at 72px the syncing numeral is 24uu and the needs-you numeral 26uu (the numeral
// is larger when it is the only thing inside the mark). `y` is optically centred, and drifts down
// as the mark shrinks. Both are overridable — 11a Grouped draws 44uu at a size the rest of the
// design draws at 34uu (DEVIATIONS.md §29).
const NUMERAL = {
  168: { syncing: { size: 21, y: 68 }, needs: { size: 21, y: 68 } },
  116: { syncing: { size: 20, y: 69 }, needs: { size: 20, y: 69 } },
  72: { syncing: { size: 24, y: 69 }, needs: { size: 26, y: 69 } },
  64: { syncing: { size: 26, y: 70 }, needs: { size: 26, y: 70 } },
  44: { syncing: { size: 30, y: 72 }, needs: { size: 30, y: 72 } },
  34: { syncing: { size: 34, y: 74 }, needs: { size: 34, y: 74 } },
};

const MONO_STACK = "'IBM Plex Mono',ui-monospace,monospace";

// --------------------------------------------------------------- gradients ----

// SVG ids are document-scoped, so a fixed id makes the second syncing hexagon on a screen resolve
// to the first one's gradient. The prototype never hits this (no frame draws two), but the app
// will: 10a Glyph states puts ten marks on one page. Monotonic, not random — F8 asserts `stroke`,
// and a random id makes that assertion unstable across runs. Kept non-hex-shaped so the raw-colour
// scan in tools/check-tokens.mjs cannot read `#face` as a colour.
let instanceCount = 0;

/**
 * One defs pair serves BOTH themes: `stop-color` resolves var(), verified by rendering the light
 * and dark palettes and sampling the pixels. 12-light-theme.md offers "duplicate the defs per theme
 * or drive the stops from CSS variables" — this is the second, so the theme toggle needs no
 * re-render and no node surgery. The inline-style form is used rather than the presentation
 * attribute because CSS-property support for var() is the safer bet in WebKit.
 *
 * Note this must use the four raw stop tokens: --up-gradient/--down-gradient are CSS
 * linear-gradient() values, and a CSS gradient cannot be an SVG stroke.
 */
function gradient(id, direction) {
  const up = direction === "up";
  return svgEl(
    "linearGradient",
    { id, x1: "0", y1: up ? "0" : "1", x2: "1", y2: up ? "1" : "0" },
    svgEl("stop", { offset: "0%", style: `stop-color:var(--${up ? "up-from" : "down-from"})` }),
    svgEl("stop", { offset: "100%", style: `stop-color:var(--${up ? "up-to" : "down-to"})` }),
  );
}

// ------------------------------------------------------------------- states ----

const TONE = {
  decision: { stroke: "var(--decision)", numeral: "var(--decision-text)" },
  destructive: { stroke: "var(--destructive)", numeral: "var(--destructive-text)" },
};

/** The body outline. `fill` carries the seam mask, or a warning tint, or nothing. */
function body(strokeWidth, stroke, fill) {
  return svgEl("path", {
    d: HEX_PATH,
    fill: fill ?? "none",
    stroke,
    "stroke-width": strokeWidth,
    "stroke-linejoin": "round",
  });
}

/**
 * Build the mark. Returns the <svg> itself, with no wrapper: all 53 in-scope hexagons sit directly
 * in their screen's flex or grid, and the 168px ones carry position:relative precisely so they
 * stack over the settled glow. A wrapper would break that.
 *
 * `flex:none` is emitted because a bare <svg> in a flex row shrinks, and every frame that sits in
 * one declares it.
 */
export function renderHexagon(opts = {}) {
  const {
    size,
    state = "settled",
    tone = "decision",
    family = "window",
    numeral = null,
    direction = "both",
    dryRun = false,
    masked = false,
    numeralTone = null,
    strokeWidth = strokeForSize(size, state === "warning" && STROKE[size]?.warning ? "warning" : family),
    numeralSize = null,
    numeralY = null,
    checkPath = CHECK,
    strikePath = size <= 20 ? STRIKE_SMALL : STRIKE,
    dotRadius = size <= 20 ? 17 : 15,
    tint = null,
    class: cls = null,
    style = null,
  } = opts;

  if (!size) throw new Error("hexagon: size is required");

  const maskFill = masked ? "var(--surface)" : null;
  const svgStyle = [`width:${size}px`, `height:${size}px`, "flex:none", style].filter(Boolean).join(";");
  const svg = svgEl("svg", { viewBox: HEX_VIEWBOX, style: svgStyle, class: cls, "aria-hidden": "true" });

  const children = [];
  switch (state) {
    case "settled": {
      children.push(body(strokeWidth, "var(--hex-settled-track)", maskFill));
      // Dropped below 20px: the tray settled glyph is a bare outline, and so is the 13px bullet.
      // Reusing the panel construction there ships a checkmark that is not in the design.
      if (size > 20) {
        children.push(
          svgEl("path", {
            d: checkPath,
            fill: "none",
            stroke: "var(--hex-settled-check)",
            "stroke-width": CHECK_STROKE[size] ?? strokeWidth,
            "stroke-linecap": "round",
            "stroke-linejoin": "round",
          }),
        );
      }
      break;
    }

    case "syncing": {
      const n = ++instanceCount;
      const upId = `hex-up-${n}`;
      const dnId = `hex-dn-${n}`;
      const twoWay = direction === "both";
      const dash = dryRun ? "40 260" : family === "tray" ? "70 230" : "62 238";
      const upDur = dryRun ? "2.4s" : family === "tray" ? "2.4s" : "3.2s";
      const dnDur = dryRun ? "3.2s" : "4.4s";

      children.push(
        svgEl("defs", {}, gradient(upId, "up"), twoWay ? gradient(dnId, "down") : null),
        body(strokeWidth, "var(--hex-syncing-track)", maskFill),
        svgEl("path", {
          d: HEX_PATH,
          fill: "none",
          stroke: `url(#${upId})`,
          "stroke-width": strokeWidth,
          "stroke-dasharray": dash,
          "stroke-linecap": "round",
          "stroke-linejoin": "round",
          style: `animation:hexup ${upDur} linear infinite`,
        }),
      );
      if (twoWay) {
        // Negative delay = -(down duration)/2, so the two segments rarely sit on the same edge.
        // The frames write it literally; see updateHexagon for why it must stay literal.
        children.push(
          svgEl("path", {
            d: HEX_PATH,
            fill: "none",
            stroke: `url(#${dnId})`,
            "stroke-width": strokeWidth,
            "stroke-dasharray": dash,
            "stroke-linecap": "round",
            "stroke-linejoin": "round",
            style: `animation:hexdn ${dnDur} linear infinite;animation-delay:-${dryRun ? "1.6" : "2.2"}s`,
          }),
        );
      }
      break;
    }

    case "needsNumeral":
      children.push(body(strokeWidth, TONE[tone].stroke, maskFill));
      break;

    case "needsDot":
      children.push(
        body(strokeWidth, TONE[tone].stroke, maskFill),
        svgEl("circle", { cx: 60, cy: 60, r: dotRadius, fill: TONE[tone].stroke }),
      );
      break;

    case "paused":
      // opacity belongs on the ROOT, not the track: 10a Paused dims the bars too. §6 writes it
      // inside the Track cell, which reads as a path property (DEVIATIONS.md §32).
      svg.style.opacity = size <= 20 ? ".45" : ".55";
      children.push(
        svgEl("path", {
          d: HEX_PATH,
          fill: "none",
          stroke: "var(--hex-paused-track)",
          "stroke-width": strokeWidth,
          "stroke-linejoin": "round",
          "stroke-dasharray": size <= 20 ? "24 24" : "14 12",
        }),
      );
      if (size > 20) {
        // Pure fills, no stroke. Absent from the tray form entirely.
        for (const x of [51, 64]) {
          children.push(
            svgEl("rect", { x, y: 49, width: 5.5, height: 22, rx: 2.5, fill: "var(--hex-paused-bars)" }),
          );
        }
      }
      break;

    case "unreachable":
      children.push(
        body(strokeWidth, "var(--destructive)", maskFill),
        // No fill and no stroke-linejoin, matching the frames exactly.
        svgEl("path", {
          d: strikePath,
          stroke: "var(--destructive)",
          "stroke-width": strokeWidth,
          "stroke-linecap": "round",
        }),
      );
      break;

    case "warning": {
      // The sixth construction. It REPLACES the numeral or check rather than layering over a state,
      // and is built on the decision or destructive outline — never on settled/syncing/paused.
      const colour = TONE[tone].stroke;
      const barWidth = size >= 104 ? 6 : 8;
      const destructive = tone === "destructive";
      children.push(
        // The tint is the only fill in the set that is neither `none` nor a surface colour. Written
        // through the channel token so it themes; it is NOT --destructive-bg, which is .06.
        body(strokeWidth, colour, tint ?? (destructive ? "rgba(var(--destructive-rgb), 0.08)" : null)),
        svgEl("path", {
          d: destructive && size < 104 ? "M60 36 L60 66" : size >= 104 ? "M60 38 L60 66" : "M60 38 L60 64",
          stroke: colour,
          "stroke-width": barWidth,
          "stroke-linecap": "round",
        }),
        svgEl("circle", {
          cx: 60,
          cy: destructive && size < 104 ? 80 : 79,
          r: size >= 104 ? 3.6 : 4.6,
          fill: colour,
        }),
      );
      break;
    }

    case "outline":
      children.push(body(strokeWidth, opts.stroke ?? "var(--line-inert)", maskFill));
      break;

    default:
      throw new Error(`hexagon: unknown state "${state}"`);
  }

  svg.append(...children.filter(Boolean));

  // The numeral is appended last so it paints over the outline. `numeral == null` renders NO text
  // node at all — 5a Checking and 7a File pending animate with none, and the design has no
  // em-dash placeholder (unlike the v1 widget this replaces).
  if (numeral != null) {
    const key = state === "syncing" ? "syncing" : "needs";
    const metrics = NUMERAL[size]?.[key];
    if (!metrics && (numeralSize == null || numeralY == null)) {
      throw new Error(`hexagon: no numeral metrics measured at ${size}px — pass numeralSize and numeralY`);
    }
    const fill = numeralTone ?? (state === "syncing" ? "var(--hex-numeral)" : TONE[tone].numeral);
    svg.append(
      svgEl(
        "text",
        {
          x: 60,
          y: numeralY ?? metrics.y,
          "text-anchor": "middle",
          style: `font-family:${MONO_STACK};font-size:${numeralSize ?? metrics.size}px;font-weight:600;fill:${fill}`,
        },
        String(numeral),
      ),
    );
  }

  return svg;
}

/**
 * Patch a rendered mark in place, without touching the animated paths.
 *
 * This exists because of a bug the v1 widget already shipped and worked around: the shell rebuilds
 * the screen on every status poll (~2s) and replaceChildren() destroys the SVG, which restarts both
 * CSS animations from 0% — the old spinner "visibly jerked back and never completed a rotation".
 * The v1 fix was a wall-clock negative animation-delay, but that would make the delay a computed
 * value, and the F8 gate asserts animation-delay against the frames' literal -2.2s.
 *
 * Holding the node across polls resolves both: the literal delay stays correct AND the animation
 * runs uninterrupted. It is therefore a constraint on the screens (F4/F5) — they must call this
 * rather than re-rendering — which is why it ships with the component that needs it.
 */
export function updateHexagon(node, opts = {}) {
  if (!node) return;
  if (opts.numeral !== undefined) {
    const text = node.querySelector("text");
    if (text) text.textContent = opts.numeral == null ? "" : String(opts.numeral);
  }
  if (opts.style != null) node.setAttribute("style", opts.style);
}
