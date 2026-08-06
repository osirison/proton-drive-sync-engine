// The seam (F3) — the second load-bearing primitive of design-v2, and the idea the whole redesign
// is built on: a 1px vertical hairline at `left:50%` separating THIS COMPUTER (left) from PROTON
// DRIVE (right). 01-foundations.md §5.
//
// EVERY NUMBER HERE WAS MEASURED out of docs/design-v2/Drive Sync.dc.html — laid out in a real
// engine and read off `getBoundingClientRect`, not transcribed from the prose. The prototype draws
// 22 seams, 20 of them in scope across 16 distinct product sites (four sites have a light twin that
// matches its dark original stop for stop). Where doc and frame disagree the frame wins for
// per-element geometry and colour (IMPLEMENTATION-PLAN.md §1.3 rule 2); every departure is written
// up in docs/design-v2/DEVIATIONS.md §32–§39. Four of those departures change what gets built:
//
//   · IT DOES TOUCH AN EDGE. §5 says "it never touches an edge". Six of the 20 run at full colour
//     into one end — the seam fades only where it actually STOPS, not where it is handed to the
//     block below. `5a Plan safe` draws the point twice: two elements, overlapping by 66px, the
//     lower one with no top fade, reading as one unbroken line across the gap between two blocks.
//     Hence three gradient shapes, not one, and `null` rather than `100` to express a cut end.
//   · THE STOPS ARE NOT A FUNCTION OF HEIGHT. 508px→26/78 and 514px→10/90 (fade-in 132px vs 51px);
//     543px→30/70 and 544px→26/74. Both pairs are the same symmetric shape, 6px and 1px apart.
//     SEAM_SITES is the authority; seamStops() is a documented fallback for heights nobody drew.
//   · A MASK NEEDS `position`, NOT JUST A BACKGROUND. §5 rule 3 says background + padding and warns
//     that z-index alone is not enough. That is the wrong half of the problem: the seam is
//     absolutely positioned, so it paints in step 8 of the stacking context, ABOVE the backgrounds
//     and the text of static siblings however late in the DOM they sit. All 34 in-scope masks are
//     positioned — 17 on themselves, 17 inside a `position:relative` wrapper — and the one drawn
//     mask that is not (`1a Compact`, round one, out of scope) ships a hairline straight through
//     "Syncing 3 changes". See DEVIATIONS.md §36.
//   · THERE IS A SECOND SEAM COLOUR AND NO TOKEN CARRIED IT. The compact panel, the tray panel and
//     the checking dialog draw `#23262D`, where every 1040 window draws `#2A2E36` — but BOTH become
//     `#D9D5CE` in light. `var(--border)` is right in dark and wrong in light, so F3 adds
//     `--seam-panel`. DEVIATIONS.md §35 and §17.
//
// The seam is NOT the conflict diff gutter. `3a Conflict diff` builds that from
// `grid-template-columns:1fr 1px 1fr` over a `#0D0E11` panel — a flat 1px column with no gradient
// and no fade. S2 should not reach for renderSeam() there.

/** The invariant part of the construction: all 22 drawn seams are exactly this box. */
const SEAM_BOX = "position:absolute;left:50%;width:1px";

/** The stop pair on `--seam-gradient`, on §5's own worked example, and the median of the 10 symmetric sites. */
export const DEFAULT_FADE_IN = 26;
export const DEFAULT_FADE_OUT = 74;

/**
 * The drawn envelope. §5 gives "10–30% in, 70–90% out"; measured, the in-stops run 10–30 (so the
 * doc is right at both ends) but the out-stops run 70–100, because of the cut form the doc does not
 * describe. seamStops() clamps into the in-stop envelope; nothing clamps a value taken from
 * SEAM_SITES, which is measurement rather than judgement.
 */
export const FADE_IN_MIN = 10;
export const FADE_IN_MAX = 30;

/**
 * Every seam the prototype draws, keyed by what it IS rather than by its frame label — a screen
 * knows it is rendering the main hero, not that the hero happens to be called `2a Syncing`. The
 * label is on each row for the F8 fidelity fixtures, which do need it.
 *
 * `fadeIn: null` / `fadeOut: null` mean the line is CUT at that end: full colour to the boundary,
 * no fade. `height` is present on the two sites the design sizes explicitly instead of pinning both
 * ends. `line` defaults to `var(--seam)`; the three panel sites override it.
 *
 * `top`/`bottom` are reproduced because several are NEGATIVE and that is the whole trick — the seam
 * deliberately overflows its container to reach the block below (`2a Syncing` runs 150px past the
 * bottom of a 394px hero, down through the transfer rows). A screen that clips its seam to its own
 * padding box has not drawn this design.
 */
export const SEAM_SITES = {
  /** `2a Syncing` · `12a Syncing light` — 394px hero, seam runs 150px on into the transfer rows. */
  mainHero: { top: 0, bottom: -150, fadeIn: 26, fadeOut: 74, frames: ["2a Syncing", "12a Syncing light"] },
  /** `2a Needs you` — the attention band below shortens the run; the only asymmetric symmetric pair. */
  mainHeroAttention: { top: 0, bottom: -114, fadeIn: 26, fadeOut: 78, frames: ["2a Needs you"] },
  /** `4a Deletions` · `12a Deletions light` — a full-height list; the shortest fade in the design. */
  deletionsList: {
    top: 0,
    bottom: 14,
    fadeIn: 10,
    fadeOut: 90,
    frames: ["4a Deletions", "12a Deletions light"],
  },
  /** `3a Conflict` · `12a Conflict light` — starts below the title block, not at the content top. */
  conflictBody: {
    top: 56,
    bottom: 8,
    fadeIn: 12,
    fadeOut: 88,
    frames: ["3a Conflict", "12a Conflict light"],
  },
  /** `2a Compact syncing` · `12a Compact syncing light` — 360px panel. */
  compactPanel: {
    top: 14,
    bottom: 8,
    fadeIn: 30,
    fadeOut: 70,
    line: "var(--seam-panel)",
    frames: ["2a Compact syncing", "12a Compact syncing light"],
  },
  /** `10a Syncing` — the tray panel. Same construction as the compact panel, 2px longer. */
  trayPanel: {
    top: 14,
    bottom: 6,
    fadeIn: 30,
    fadeOut: 70,
    line: "var(--seam-panel)",
    frames: ["10a Syncing"],
  },
  /** `5a Checking` — the 522px dry-run dialog. Panel colour despite being a window, not a panel. */
  checkingDialog: {
    top: 60,
    bottom: 60,
    fadeIn: 30,
    fadeOut: 70,
    line: "var(--seam-panel)",
    frames: ["5a Checking"],
  },
  /** `5a Plan` — the totals block. Cut at the bottom: the list below continues the thought. */
  planTotals: { top: 8, bottom: 0, fadeIn: 18, fadeOut: null, frames: ["5a Plan"] },
  /** `5a Plan safe` upper half of the continuation pair — overflows 40px past a 300px hero. */
  planSafeHero: { top: 24, bottom: -40, fadeIn: 26, fadeOut: null, frames: ["5a Plan safe"] },
  /** `5a Plan safe` lower half — starts 40px ABOVE its block, no top fade, so the joint is seamless. */
  planSafeList: { top: -40, bottom: 0, fadeIn: null, fadeOut: 82, frames: ["5a Plan safe"] },
  /** `7a Activity quiet` — cut at the bottom into the two-column summary. */
  activityQuiet: { top: 6, bottom: 0, fadeIn: 20, fadeOut: null, frames: ["7a Activity quiet"] },
  /** `7a File lookup` — cut, and overflowing 24px past its block. */
  fileLookup: { top: 66, bottom: -24, fadeIn: 14, fadeOut: null, frames: ["7a File lookup"] },
  /** `8a Settings` — the folder-pair row only, sized explicitly rather than pinned to the panel. */
  settingsPair: { top: 44, height: 86, fadeIn: 26, fadeOut: 74, frames: ["8a Settings"] },
  /** `9a Folders` — likewise explicit: the seam covers the two folder cards, not the page. */
  onboardingFolders: { top: 0, height: 250, fadeIn: 22, fadeOut: 78, frames: ["9a Folders"] },
  /** `9a Review` — cut, overflowing 56px into the two summary columns below. */
  onboardingReview: { top: 104, bottom: -56, fadeIn: 18, fadeOut: null, frames: ["9a Review"] },
  /** `9a First sync` — the 602×542 onboarding window. Note it keeps `var(--seam)`, not the panel colour. */
  firstSync: { top: 40, bottom: 110, fadeIn: 24, fadeOut: 76, frames: ["9a First sync"] },
};

const px = (n) => (n === 0 ? "0" : `${n}px`);

/**
 * The gradient, in one of the three shapes the prototype draws.
 *
 *   both ends fade   `S, L a%, L b%, S`     10 sites, 14 drawn — the form §5 documents
 *   bottom cut       `S, L a%, L 100%`       5 sites — 9a Review, 7a Activity quiet, 7a File
 *                                            lookup, 5a Plan, 5a Plan safe (hero)
 *   top cut          `L, L b%, S`            1 site  — 5a Plan safe (list)
 *
 * Emitting four stops always and writing 100 into one of them reproduces the bottom-cut form by
 * accident and gets the top-cut form wrong: `S, L 0%, L 82%, S` still fades UP from the surface
 * over the first pixel row, which is exactly the visible joint the pair exists to remove.
 *
 * The default returns the `--seam-gradient` token rather than an identical literal, so the single
 * most common form (mainHero, settingsPair and their light twins) has one place to retune.
 */
export function seamGradient({
  line = "var(--seam)",
  surface = "var(--surface)",
  fadeIn = DEFAULT_FADE_IN,
  fadeOut = DEFAULT_FADE_OUT,
} = {}) {
  if (fadeIn == null && fadeOut == null) return line; // both ends cut: a flat hairline, undrawn
  if (
    line === "var(--seam)" &&
    surface === "var(--surface)" &&
    fadeIn === DEFAULT_FADE_IN &&
    fadeOut === DEFAULT_FADE_OUT
  ) {
    return "var(--seam-gradient)";
  }
  if (fadeIn == null) return `linear-gradient(${line}, ${line} ${fadeOut}%, ${surface})`;
  if (fadeOut == null) return `linear-gradient(${surface}, ${line} ${fadeIn}%, ${line} 100%)`;
  return `linear-gradient(${surface}, ${line} ${fadeIn}%, ${line} ${fadeOut}%, ${surface})`;
}

/**
 * The per-height stop calculator, for a block at a height the design never drew.
 *
 * It does not interpolate SEAM_SITES, because there is nothing to interpolate: the drawn stops are
 * not a function of height and two clean pairs prove it (508→26/78 vs 514→10/90, six pixels apart
 * with a 132px fade against a 51px one; 543→30/70 vs 544→26/74, one pixel apart). Nor does it
 * follow §5's "fade over roughly the top and bottom eighth" — 12.5% matches two of the ten
 * symmetric sites, whose median is 26% (the mode is a 3–3 tie between 26 and 30, so the median is
 * the honest summary — and 26 is also what §5's own example gradient and --seam-gradient carry).
 * Frame wins (rule 2).
 *
 * So the default is flat 26/74 at any height, and `height` earns its place only through `fadePx`:
 * CSS gradient stops are percentages of the painted box, and a flex-derived seam's box is not known
 * until layout, so converting an intent of "about 40px of fade" into stops is the one genuinely
 * height-dependent thing there is to compute. The result is clamped into the drawn 10–30% envelope
 * — outside it the line either fails to reach full colour or reads as a hard edge.
 *
 * Reproducing a drawn frame? Quote SEAM_SITES. This is for everything else.
 */
export function seamStops(height, { fadePx = null } = {}) {
  if (fadePx == null) return { fadeIn: DEFAULT_FADE_IN, fadeOut: DEFAULT_FADE_OUT };
  if (!(height > 0))
    throw new Error(
      `seam: seamStops needs a positive height to convert ${fadePx}px into stops, got ${height}`,
    );
  const pct = Math.round((fadePx / height) * 100);
  const fadeIn = Math.min(FADE_IN_MAX, Math.max(FADE_IN_MIN, pct));
  return { fadeIn, fadeOut: 100 - fadeIn };
}

/**
 * Build a seam. Returns a bare `<div>` — every drawn seam is the FIRST child of a `position:relative`
 * block and nothing wraps it, which matters because the DOM order is half of rule 3: siblings that
 * come later and are positioned paint over it, and that is how the masks work.
 *
 * `site` names a row of SEAM_SITES and supplies top/bottom/height/stops/colour; anything passed
 * alongside it wins, so a screen can quote the drawn geometry and still override one end.
 *
 * `aria-hidden` matches the hexagon: the seam carries meaning by position, and a screen reader gets
 * that meaning from the column headings ("This computer" / "Proton Drive"), not from a hairline.
 */
export function renderSeam(opts = {}) {
  const site = opts.site ? SEAM_SITES[opts.site] : null;
  if (opts.site && !site) {
    throw new Error(`seam: unknown site "${opts.site}". Drawn sites: ${Object.keys(SEAM_SITES).join(", ")}`);
  }

  // `??` and destructuring defaults are both wrong for the stops: `null` is a MEANING here (the end
  // is cut, full colour to the boundary), not an absent value, so either would silently promote the
  // six cut ends back to a 74% fade-out. The roundtrip harness caught exactly that. Presence of the
  // key is the test; `undefined` is the only "not given".
  const pick = (key, fallback) => {
    if (opts[key] !== undefined) return opts[key];
    if (site && key in site) return site[key];
    return fallback;
  };

  const top = pick("top", 0);
  const fadeIn = pick("fadeIn", DEFAULT_FADE_IN);
  const fadeOut = pick("fadeOut", DEFAULT_FADE_OUT);
  const line = pick("line", "var(--seam)");
  const surface = pick("surface", "var(--surface)");
  const cls = opts.class !== undefined ? opts.class : "seam";
  const style = opts.style ?? null;

  // Written out rather than defaulted in the destructuring, because `height` and `bottom` are
  // alternatives and a default expression cannot see whether the OTHER one was passed. Whichever
  // the caller gives wins over the site row, including when the site row uses the other form —
  // `8a Settings` sizes the box, a screen reusing its stops may want to pin both ends instead.
  if (opts.height != null && opts.bottom != null) {
    throw new Error(
      "seam: pass either `height` or `bottom`, not both — the design sizes the box or pins the end, never both",
    );
  }
  let height = null;
  let bottom = null;
  if (opts.height != null) height = opts.height;
  else if (opts.bottom != null) bottom = opts.bottom;
  else if (site?.height != null) height = site.height;
  else bottom = site?.bottom ?? 0;

  const node = document.createElement("div");
  node.setAttribute("aria-hidden", "true");
  if (cls) node.className = cls;
  node.setAttribute(
    "style",
    [
      SEAM_BOX,
      `top:${px(top)}`,
      height != null ? `height:${px(height)}` : `bottom:${px(bottom ?? 0)}`,
      `background:${seamGradient({ line, surface, fadeIn, fadeOut })}`,
      style,
    ]
      .filter(Boolean)
      .join(";"),
  );
  return node;
}

// ------------------------------------------------------------------------------- rule 3: masks ---

/**
 * Side padding. The frames use 14, 16 and 18 and no function of font-size reproduces all of them —
 * the headline tiers are clean (≥28px→18, 17–22px→16, ≤15px→14) but four sub-labels take their
 * block's padding instead of their own tier (`9a Review` 14px→18, `2a Syncing` 13px→16). It is a
 * property of the centred block, chosen by eye, so it stays a parameter with a mid-band default and
 * the screens quote their frame. DEVIATIONS.md §37.
 */
export const MASK_PAD = 16;

/**
 * The mask, as an inline style string.
 *
 * Three parts, and dropping any one of them puts a hairline through the glyphs:
 *
 *   `background`  the surface behind the text. Buttons already carry their own fill (`2a Syncing`'s
 *                 Pause is `--panel-raised`, not the surface), so pass `surface:null` and keep it.
 *   `padding`     so the line is hidden a little before and after the text, not clipped to the ink.
 *                 Vertical padding is 0 on headlines and 2px on the small mono sub-labels.
 *   `position`    THE ONE §5 OMITS. The seam is `position:absolute`, so it paints with the
 *                 positioned descendants — above the background of any static sibling and above its
 *                 text, whatever the DOM order. Every masked node in the round-two frames is
 *                 positioned, directly or through a wrapper; the one that is not is `1a Compact`,
 *                 and it ships the bug. Pass `position:false` ONLY when a positioned wrapper
 *                 already sits between this node and the seam's containing block — which is what
 *                 17 of the 34 drawn masks do.
 */
export function maskStyle({ pad = MASK_PAD, padY = 0, surface = "var(--surface)", position = true } = {}) {
  return [
    surface ? `background:${surface}` : null,
    pad == null ? null : `padding:${px(padY)} ${px(pad)}`,
    position ? "position:relative" : null,
  ]
    .filter(Boolean)
    .join(";");
}

/**
 * Wear the mask: apply maskStyle() to an element that is already built. Declarations are set one at
 * a time rather than appended to the style attribute, so calling this on a node that already has
 * inline styles (every button in the design does) overrides only what the mask owns.
 */
export function seamMask(node, opts = {}) {
  if (!node) return node;
  for (const decl of maskStyle(opts).split(";")) {
    const at = decl.indexOf(":");
    if (at < 1) continue; // maskStyle can legitimately return "" — a button masking with its own fill
    node.style.setProperty(decl.slice(0, at), decl.slice(at + 1));
  }
  return node;
}

// ------------------------------------------------------------------------ rules 2 and 3: audit ---

/**
 * How much of what is behind this element does its background hide? `0` for none, `1` for all.
 *
 * Read out of the computed value rather than compared against the literal `rgba(0, 0, 0, 0)` that a
 * browser serialises `transparent` to: tools/check-tokens.mjs reads that literal as a raw colour and
 * fails the build on it. Alpha lives in the fourth argument of the `rgba()` form, and *only* there —
 * an engine serialises an opaque colour as `rgb(r, g, b)`, so a "does it end in `, 0)`" test reads
 * pure black, and every other zero-blue colour, as transparent.
 *
 * A background-image counts as 1. It usually is — the only one in the frames is the seam itself —
 * but a gradient can be transparent at any stop and resolving that means rasterising. Recorded
 * rather than guessed: if a screen ever masks with a gradient, this is where the audit goes blind.
 */
const RGBA_ALPHA = /^rgba\(\s*[\d.]+\s*,\s*[\d.]+\s*,\s*[\d.]+\s*,\s*([\d.]+)\s*\)$/;

function backgroundOpacity(styles) {
  if (styles.backgroundImage !== "none") return 1;
  const bg = styles.backgroundColor;
  if (bg === "transparent") return 0;
  const alpha = RGBA_ALPHA.exec(bg);
  return alpha ? Number(alpha[1]) : 1; // rgb(...) has no alpha channel: opaque
}

/**
 * Check the live DOM against the two rules that are about PLACEMENT rather than construction, so a
 * screen can fail loudly in development instead of shipping a hairline through a headline. F8's
 * fidelity harness is the real gate; this is the same test, available from a console and from a
 * unit test, and it is what "all four rules enforced" means for a component that cannot see its own
 * surroundings at build time.
 *
 * Rule 1 is deliberately NOT checked. "Drawn when it means something" over-predicts what the design
 * actually draws — `2a Compact needs you`, `4a Compact` and `3a Conflicts cleared` all present a
 * two-sided decision and draw no seam at all, and `7a File pending` masks its hexagon for a seam
 * that is not there. The 16 rows of SEAM_SITES are the authority; a rule-1 check would report the
 * design's own screens as violations. DEVIATIONS.md §32.
 *
 * Rule 4 (direction is carried by position first) is not checkable here either: it is a claim about
 * which COLUMN a row is in, which only the screen knows.
 *
 * `selector` exists so the harness can point this at hand-written markup — the prototype's own
 * seams carry no class, and being able to run the audit against the frames is what proved the rule
 * on `1a Compact` in the first place.
 *
 * Returns a list of problems, `[]` when clean. Never throws on DOM shape: a screen mid-render is a
 * normal thing to hand it.
 */
export function auditSeams(root = document.body, { selector = ".seam" } = {}) {
  const problems = [];
  const rootBox = root.getBoundingClientRect();
  // Collected once. The inner loop is already O(seams × elements) and every iteration forces a
  // style resolve; re-walking the tree per seam on top of that is pure waste on the screens most
  // worth auditing, which are the ones with two seams and a few hundred nodes.
  const candidates = [...root.querySelectorAll("*")];

  for (const seam of root.querySelectorAll(selector)) {
    const box = seam.getBoundingClientRect();
    if (!box.height) continue;
    const centre = box.left + box.width / 2;

    for (const other of candidates) {
      if (other === seam || seam.contains(other) || other.contains(seam)) continue;
      const b = other.getBoundingClientRect();
      if (!b.width || !b.height) continue;
      if (b.top >= box.bottom || b.bottom <= box.top) continue;

      const styles = getComputedStyle(other);
      const cover = backgroundOpacity(styles);

      // Rule 2 — the seam stops above any full-width band. A band spans the window, so anything
      // narrower is a card or a column and is allowed to sit beside the line. The check is not
      // limited to opaque bands: every band in the prototype is drawn transparent with a 1px
      // `border-top`, and one the line runs THROUGH is still the overlap the rule forbids.
      if (b.width >= rootBox.width - 4 && (cover > 0 || parseFloat(styles.borderTopWidth) > 0)) {
        problems.push({
          rule: 2,
          seam,
          element: other,
          message: `seam overlaps a full-width band by ${Math.round(Math.min(box.bottom, b.bottom) - Math.max(box.top, b.top))}px — it must terminate above it`,
        });
        continue;
      }

      // Rule 3 — anything centred on the seam masks it.
      //
      // The subject is anything that CLAIMS to mask: a background is what a screen reaches for, so
      // a background of any opacity is the signal. An element with none is not attempting the mask
      // and is not reported — otherwise every centred flex wrapper in the design is a violation,
      // and a check that cries wolf is a check nobody runs. Given the claim, two things can be
      // wrong with it, and both are invisible until someone looks at the pixels.
      const straddles = b.left < centre - 1 && b.right > centre + 1;
      if (!straddles || cover === 0) continue;

      // (a) it does not actually cover. `--decision-bg` is rgba(255, 107, 107, .05) — a real token
      // that hides nothing; a hairline under 50% alpha is a hairline at half strength.
      if (cover < 1) {
        problems.push({
          rule: 3,
          seam,
          element: other,
          message: `an element centred on the seam has a background of only ${cover} opacity — the line shows through it`,
        });
        continue;
      }

      // (b) it covers, but paints underneath. The rule §5 does not state.
      let node = other;
      let positioned = false;
      while (node && node !== seam.offsetParent) {
        if (getComputedStyle(node).position !== "static") {
          positioned = true;
          break;
        }
        node = node.parentElement;
      }
      if (!positioned) {
        problems.push({
          rule: 3,
          seam,
          element: other,
          message:
            "an opaque element is centred on the seam but neither it nor a wrapper is positioned — the line will paint over it (see 1a Compact)",
        });
      }
    }
  }
  return problems;
}

/**
 * Fade a seam in or out over 320ms (01-foundations.md §7: "320ms ease-out for the seam and its
 * columns"). The transition itself lives in styles/seam.css, because reduced motion needs a media
 * query and an inline style cannot carry one.
 *
 * The node is left in the DOM at `opacity:0` rather than removed: a removed node cannot transition,
 * and the screen owns when — or whether — to take it out afterwards. Under reduced motion the
 * transition is dropped but the seam still appears and disappears; the preference is about movement,
 * not about hiding a state boundary the design uses to carry direction.
 */
export function setSeamVisible(node, visible) {
  if (!node) return node;
  node.style.opacity = visible ? "1" : "0";
  return node;
}
