// The hexagon (F2) — one of the two load-bearing primitives of design-v2. Everything else on a
// screen is composition; getting this wrong is, per 01-foundations.md §6, "the single easiest way
// to make the redesign look off-brand".
//
// EVERY NUMBER HERE WAS MEASURED out of docs/design-v2/Drive Sync.dc.html, node by node, across all
// 53 in-scope hexagons — not transcribed from the prose. Where the two disagree the frame wins for
// per-element values (IMPLEMENTATION-PLAN.md §1.3 rule 2) and the difference is recorded in
// docs/design-v2/DEVIATIONS.md §20–§31. Several of those disagreements are load-bearing:
//
//   · The size→stroke relation is a LOOKUP TABLE, not a formula. It is not single-valued (80px is
//     drawn at both 4.4 and 4.6) and not monotonic (48→5.4 but the smaller 44→5), so no function
//     can reproduce it. Never interpolate: an unlisted size throws rather than guessing.
//   · The seam mask (`fill`) is ORTHOGONAL to state. 18 in-scope marks carry one, including settled
//     marks at 52/80/88 and a needs-you mark at 44, while `2a Settled` carries none — so it is a
//     caller flag, not a property of `syncing`. Two masked frames have no seam element at all.
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
 * 49.5275 / 49.4783. See DEVIATIONS.md §23.
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
// design draws at 34uu (11a Grouped vs 3a Conflict diff, same 34px mark).
const NUMERAL = {
  168: { syncing: { size: 21, y: 68 }, needs: { size: 21, y: 68 } },
  116: { syncing: { size: 20, y: 69 }, needs: { size: 20, y: 69 } },
  72: { syncing: { size: 24, y: 69 }, needs: { size: 26, y: 69 } },
  64: { syncing: { size: 26, y: 70 }, needs: { size: 26, y: 70 } },
  44: { syncing: { size: 30, y: 72 }, needs: { size: 30, y: 72 } },
  34: { syncing: { size: 34, y: 74 }, needs: { size: 34, y: 74 } },
};

const MONO_STACK = "'IBM Plex Mono',ui-monospace,monospace";

/**
 * Numeral metrics per rendered mark, so updateHexagon can REBUILD a <text> node it removed rather
 * than only mutate one that survives. A WeakMap rather than a data attribute: the frames carry no
 * such attribute, and F8 compares attributes node for node.
 */
const numeralMeta = new WeakMap();

function numeralNode({ y, fontSize, fill }, value) {
  return svgEl(
    "text",
    {
      x: 60,
      y,
      "text-anchor": "middle",
      style: `font-family:${MONO_STACK};font-size:${fontSize}px;font-weight:600;fill:${fill}`,
    },
    String(value),
  );
}

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
 * `flexNone` IS A PER-SITE PROPERTY, not a property of the mark — corrected in F6, which was the
 * first task to put a hexagon in front of the fidelity gate. This function used to emit `flex:none`
 * unconditionally on the strength of "a bare <svg> in a flex row shrinks, and every frame that sits
 * in one declares it". Censused across the 53 in-scope marks, TEN declare it and forty-three do not,
 * and `flex-shrink` is an asserted property — so the unconditional form failed every compact frame
 * and would have failed every dialog and panel after them.
 *
 * The ten are the marks sharing a flex ROW with text, and NONE OF THEM IS BUILT YET — they belong to
 * screens that do not exist, so there is no caller passing `flexNone: true` today and this list is a
 * standing instruction rather than a description of the code:
 *
 *   `3a Conflict diff` (S2) · `5a Plan` (S4) · `8a Save refused` (S6) · `9a CLI missing`,
 *   `9a Review` (S7) · `11a Grouped`, `11a Outage`, the three `11a In situ` banners (S9).
 *
 * Each of those must pass `flexNone: true` when its screen lands; every other mark must not, and the
 * fidelity gate says which is which the moment a frame is mapped.
 */
export function renderHexagon(opts = {}) {
  // Before the destructuring, not after: several defaults below call strokeForSize(size) or key
  // off it, and a default expression is evaluated during destructuring — so a missing size
  // otherwise surfaces as "no stroke width measured for size undefinedpx".
  if (!opts.size) throw new Error("hexagon: size is required");

  const {
    size,
    state = "settled",
    tone = "decision",
    family = "window",
    numeral = null,
    direction = "both",
    dryRun = false,
    masked = false,
    flexNone = false,
    mono = false,
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

  const maskFill = masked ? "var(--surface)" : null;

  /**
   * THE TRAY GLYPH IS A DIFFERENT DRAWING, not the in-window mark shrunk, and `family: "tray"` is
   * what says so. S8 is its first caller — F2 wrote the syncing branch below in anticipation and
   * nothing had ever passed the flag, so this is the first time the difference is drawn.
   *
   * At 20px the outline IS the mark: there is no check inside it and no numeral over it, so it has
   * to be the FOREGROUND colour. The in-window settled and paused marks stroke their outline with
   * `--hex-settled-track`/`--hex-paused-track` — the dark ring a check or a pair of bars sits
   * inside — and a glyph drawn at that value is a near-invisible grey on a dark panel. All ten
   * marks on `10a Glyph states` stroke at `--hex-glyph-fg` or at their tone; not one is a track
   * colour. DEVIATIONS §82a.
   */
  const glyph = family === "tray";
  const glyphFg = "var(--hex-glyph-fg)";
  /**
   * The monochrome column of the sheet — every form at one colour.
   *
   * This is the property `10-tray.md` calls load-bearing ("state is carried by fill, not hue"): a
   * tray icon may be forced single-colour by the desktop, so each form has to survive losing its
   * hue. `mono` is therefore only meaningful on the glyph; in the window the tone IS the message
   * and there is nothing to fall back to.
   */
  const toned = (colour) => (glyph && mono ? glyphFg : colour);
  const svgStyle = [`width:${size}px`, `height:${size}px`, flexNone ? "flex:none" : null, style]
    .filter(Boolean)
    .join(";");
  const svg = svgEl("svg", { viewBox: HEX_VIEWBOX, style: svgStyle, class: cls, "aria-hidden": "true" });

  const children = [];
  switch (state) {
    case "settled": {
      // Both columns of the sheet draw this one at `--hex-glyph-fg`, so it does not go through
      // `toned` — there is no hue to lose, which is the whole of the "Up to date" form.
      children.push(body(strokeWidth, glyph ? glyphFg : "var(--hex-settled-track)", maskFill));
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

      // The tray glyph's track is its own colour, not the in-window one: 10a Glyph states draws the
      // colour form's track at #2A2E36 (--hex-glyph-track) and the monochrome form's at #3E454E
      // (--line-inert), against #191C21 in every panel. A tray icon may be forced single-colour by
      // the desktop, which is why the mono form exists at all (10-tray.md).
      const track =
        family === "tray"
          ? mono
            ? "var(--line-inert)"
            : "var(--hex-glyph-track)"
          : "var(--hex-syncing-track)";

      children.push(
        // NO DEFS AT ALL in the monochrome form, rather than defs nothing references. `10a Glyph
        // states` draws the mono syncing glyph as two paths and the colour one as defs + two paths,
        // and the gate compares node for node — an unreferenced `<defs>` is a node the drawing does
        // not have.
        glyph && mono
          ? null
          : svgEl("defs", {}, gradient(upId, "up"), twoWay ? gradient(dnId, "down") : null),
        body(strokeWidth, track, maskFill),
        svgEl("path", {
          d: HEX_PATH,
          fill: "none",
          // The travelling segment is the one mark in the set whose colour form is a GRADIENT, so
          // the monochrome fallback is not a paler version of it — it is a flat foreground stroke.
          stroke: toned(`url(#${upId})`),
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
        body(strokeWidth, toned(TONE[tone].stroke), maskFill),
        // The filled centre is what `10-tray.md` means by adding MASS rather than a badge, and it
        // is why this form survives the monochrome column: the shape changes, not just the hue.
        svgEl("circle", { cx: 60, cy: 60, r: dotRadius, fill: toned(TONE[tone].stroke) }),
      );
      break;

    case "paused":
      // opacity belongs on the ROOT, not the track: 10a Paused dims the bars too. §6 writes it
      // inside the Track cell, which reads as a path property (DEVIATIONS.md §26).
      svg.style.opacity = size <= 20 ? ".45" : ".55";
      children.push(
        svgEl("path", {
          d: HEX_PATH,
          fill: "none",
          // Like settled: both columns draw it at the foreground colour. The state is carried by
          // the interrupted outline and the .45 opacity above, which is what survives losing hue.
          stroke: glyph ? glyphFg : "var(--hex-paused-track)",
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
        body(strokeWidth, toned("var(--destructive)"), maskFill),
        // No fill and no stroke-linejoin, matching the frames exactly.
        svgEl("path", {
          d: strikePath,
          stroke: toned("var(--destructive)"),
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
  const measured = NUMERAL[size]?.[state === "syncing" ? "syncing" : "needs"];
  if (numeralY != null || numeralSize != null || measured) {
    numeralMeta.set(svg, {
      y: numeralY ?? measured?.y,
      fontSize: numeralSize ?? measured?.size,
      fill: numeralTone ?? (state === "syncing" ? "var(--hex-numeral)" : TONE[tone].numeral),
    });
  }
  if (numeral != null) {
    const metrics = numeralMeta.get(svg);
    if (metrics?.y == null || metrics.fontSize == null) {
      throw new Error(`hexagon: no numeral metrics measured at ${size}px — pass numeralSize and numeralY`);
    }
    svg.append(numeralNode(metrics, numeral));
  }

  return svg;
}

// ------------------------------------------------------------------ the tray glyph ----

/**
 * THE FIVE FORMS, and there are five (`10-tray.md`: "Only five forms exist. A solid filled hexagon
 * is not a state — it was drawn that way by mistake during design and corrected").
 *
 * In the order `10a Glyph states` lays them down the sheet, which is also the order they are worth
 * reading in: the resting shape, then the one that moves, then the one that adds mass, then the two
 * that say nothing is moving.
 *
 * Each entry is the ARGUMENTS that make that form, not a second drawing of it. The whole point is
 * that S8 has one geometry source: the specimen sheet, the panel and the SVG files the desktop
 * loads all come through `renderHexagon` from this table, so a form cannot be right in the app and
 * wrong in the icon the tray actually shows.
 */
export const TRAY_GLYPH_STATES = ["settled", "syncing", "needsYou", "paused", "unreachable"];

const GLYPH_FORM = {
  settled: { state: "settled" },
  // ONE segment, not two. `renderHexagon` defaults `direction` to "both" and pushes a second
  // animated path with its own gradient; `10a Glyph states` draws the syncing glyph as a track and
  // a single travelling segment, with one `<linearGradient>` in its defs. At 16px two segments
  // chasing each other read as noise rather than as motion, which is the same reason the dash is
  // 70 230 here and 62 238 in the window.
  syncing: { state: "syncing", direction: "up" },
  // `needsDot`, not `needsNumeral`: a count does not survive being shrunk to 16px, and the design
  // replaces it with mass. `dotRadius` falls out of the size (17 at ≤20px), so it is not passed.
  needsYou: { state: "needsDot", tone: "decision" },
  paused: { state: "paused" },
  unreachable: { state: "unreachable" },
};

/**
 * One tray glyph.
 *
 * `size` defaults to 20 because that is what the sheet draws — `10-tray.md` says "rendered 15–20px",
 * a range, and `10a Glyph states` renders every one of the ten at exactly 20. The range is what the
 * desktop may do with the SVG; 20 is what the design measured.
 *
 * `mono` picks the sheet's left column: every form at one colour, which is what a desktop that
 * forces its tray icons monochrome will show. It is not a degraded version — all five forms are
 * distinguishable in it, and that is the property the whole set is built to have.
 */
export function trayGlyph({ state, mono = false, size = 20 } = {}) {
  const form = GLYPH_FORM[state];
  if (!form) {
    throw new Error(
      `hexagon: "${state}" is not one of the five tray glyphs (${TRAY_GLYPH_STATES.join(", ")}). ` +
        "10-tray.md is explicit that only five forms exist — a sixth is a drawing mistake, not a state.",
    );
  }
  return renderHexagon({ ...form, size, family: "tray", mono });
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
    // Removed, not blanked. renderHexagon emits no <text> at all when numeral == null, and an empty
    // one is a different DOM shape — which the F8 gate compares node for node, and which would also
    // leave a stale text node behind if the count later came back.
    if (opts.numeral == null) {
      text?.remove();
    } else if (text) {
      text.textContent = String(opts.numeral);
    } else {
      // The node was removed by an earlier null (or never drawn), and a count has come back.
      // Rebuilding it here is the whole point of the WeakMap: re-rendering the mark instead would
      // restart both animations, which is exactly what this function exists to avoid.
      //
      // Throws rather than no-opping when there are no metrics. A mark at a size the design never
      // draws a numeral at (104px, say — neither 5a Checking nor 4a Armed has one) has nothing to
      // rebuild from, and inventing a font-size and y here would be the same guess strokeForSize
      // refuses to make. Failing silently would leave a mark that simply never shows its count.
      const metrics = numeralMeta.get(node);
      if (!metrics) {
        throw new Error(
          "hexagon: this mark has no numeral metrics — no numeral is drawn at its size in any frame. Pass numeralSize and numeralY to renderHexagon if it needs one.",
        );
      }
      node.append(numeralNode(metrics, opts.numeral));
    }
  }
  if (opts.style != null) node.setAttribute("style", opts.style);
}
