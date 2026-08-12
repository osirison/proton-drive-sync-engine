// The contrast gate (S10) — the only check the seven undrawn light screens get.
//
// WHY IT EXISTS. Four of the eleven screens have a light frame drawn and are asserted against it,
// node by node, by `assert.mjs`. The other seven do not: `12-light-theme.md` says to "apply the
// table above mechanically — there are no light-specific layout decisions", so their light theme is
// a token swap with no drawn artefact behind it. `check-tokens.mjs` already proves the mechanical
// half — every token has both values, the two light blocks agree, and no raw colour lives outside
// `tokens.css` — so a token cannot simply be MISSING in light.
//
// What it cannot prove is that a light value is the RIGHT END OF ITS RAMP. `12-light-theme.md`'s
// first rule is that light is not an inversion: the accents move to the darker end, because the
// luminous ones read as pastel on white. Get that backwards on a token no frame draws — `#FFB84D`
// left where `#D97706` belongs — and every gate in this build stays green while a sentence turns
// unreadable on seven screens.
//
// SO IT COMPARES A NODE TO ITSELF ACROSS THE TWO THEMES, not to a standard.
//
// A fixed WCAG floor was the obvious design and it is the wrong one here, for a reason this repo
// already wrote down: `--text-5` is 4.33:1 in dark and `tokens.css` records that as a known,
// deliberate deviation; `--text-disabled` on a light surface is nearer 2:1 and is meant to be. A
// gate at 4.5 would fail the design as drawn, on frames that ARE drawn — and a gate that fails
// where the design is right is a gate that gets an exemption list and then gets ignored.
//
// The failure this is built for is asymmetric by construction: the dark theme is drawn, measured
// and asserted against 51 frames, so dark is the reference. A node that carries 8:1 in dark and
// 1.4:1 in light has had its token mapped to the wrong end of the ramp, whatever the absolute
// numbers are.
//
// AND PARITY ALONE IS NOT ENOUGH EITHER, which the first run measured rather than argued. The
// accents drop hardest of anything in the design and drop CORRECTLY: `--down-label` is `#22D3EE` on
// near-black, 10.89:1, and `#0E7490` on `#FAF8F5`, 5.05:1 — 46% of it, and both values are drawn,
// asserted node-for-node against eight light frames, and exactly what 12-light-theme.md's table
// asks for. A parity rule that fails those fails the design at its most deliberate point.
//
// So it takes BOTH: light has to be low in absolute terms AND much lower than dark. The two
// conditions never coincide in this design and the margins are not narrow — see the thresholds.

import { writeFileSync } from "node:fs";
import puppeteer from "puppeteer";
import { serve } from "./serve.mjs";
import { FIXTURES } from "../../src/js/fixtures/frames.js";

/**
 * The two thresholds, and a node fails only when it crosses BOTH.
 *
 * MEASURED ACROSS ALL 51 FRAMES IN BOTH THEMES, then placed in the gap. The full distribution is
 * `--report`; what matters is where its two extremes sit.
 *
 *   · The worst parity in the design is **0.46** — `--down-label`, 10.89:1 dark against 5.05:1
 *     light. Correct, drawn, and asserted. Everything below 0.55 parity is an accent arrow or a
 *     diff-gutter numeral, and the dimmest of them is 4.67:1.
 *   · The lowest light contrast in the design is **1.37:1** — a `·` separator in the diff gutter —
 *     and the quiet tail above it is disabled controls: `‹` at 1.76, `Save` at 1.86, `Delete` at
 *     2.32. `12a Conflict light` DRAWS that chevron at `#B9BEC6` and `assert.mjs` compares it
 *     exactly, so it is right on the strongest evidence this build has. Every row under 3:1 has a
 *     parity of 0.62 or better, because dark draws them just as quietly.
 *
 * The two populations are disjoint on opposite axes, so the conjunction has room on both: nothing
 * under 3:1 is under 0.62 parity, and nothing under 0.5 parity is under 4.74:1.
 *
 * A token left at its dark value lands in the corner neither population occupies. `--up-to`
 * (`#FFB84D`) not mapped to `#D97706` measures 1.62:1 on `#FAF8F5` against 11.45:1 on `#0A0B0D` —
 * 0.14 parity; `--decision` left at `#FF6B6B` is 2.62 against 7.09, 0.37. `S10_CONTRAST_POISON`
 * below reproduces one on demand, because a gate nobody has watched fail is a gate nobody should
 * believe: poisoned it reports 43 findings where the clean run reports none.
 */
const LIGHT_FLOOR = 3.0;
const PARITY_FLOOR = 0.5;

/** WCAG relative luminance. */
function luminance({ r, g, b }) {
  const f = (c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** WCAG contrast ratio, always ≥ 1. */
function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

const { server, port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
await page.setViewport({ width: 1040, height: 764, deviceScaleFactor: 1 });

/**
 * Every text node's colour and the colour actually behind it, for one rendered screen.
 *
 * COMPOSITING IS THE WHOLE JOB. A crimson card is `rgba(190,18,60,.03)` over a white panel over the
 * window surface, and the sentence inside it is `#374151` at some inherited opacity — so "the
 * background" is not a property any single element has. This walks up compositing every partly
 * transparent layer until it reaches an opaque one, and multiplies the opacity chain into the text.
 *
 * A node over a background IMAGE is skipped and counted rather than guessed at: a gradient has no
 * single colour, and the two the design uses are the up and down ramps, which are accents behind
 * nothing but their own hairline. Reported so the number is visible rather than implied.
 */
const READ_NODES = function nodesOfDocument() {
  const parse = (v) => {
    const m = /rgba?\(([^)]+)\)/.exec(v ?? "");
    if (!m) return null;
    const p = m[1]
      .split(/[\s,/]+/)
      .filter(Boolean)
      .map(Number);
    if (p.length < 3 || p.some((n) => !Number.isFinite(n))) return null;
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  // Source-over, WITH the alpha arithmetic. The first version returned `a: 1` unconditionally, which
  // reads correctly only when the bottom layer is already opaque — and the design stacks tints two
  // and three deep. `Move to Proton's Trash` is `rgba(190,18,60,.06)` inside a `rgba(190,18,60,.03)`
  // card, and forcing the alpha turned that pair into solid `#BE123C`: the same colour as the label
  // on it, reported as 1.00:1 in both themes. The gate accused the design of a bug it had itself.
  const over = (top, bottom) => {
    const a = top.a + bottom.a * (1 - top.a);
    if (a === 0) return { r: 0, g: 0, b: 0, a: 0 };
    const mix = (t, b) => (t * top.a + b * bottom.a * (1 - top.a)) / a;
    return { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a };
  };

  const out = [];
  const root = document.getElementById("app-root");
  if (!root) return out;
  // The canvas the whole window is painted on — the FIRST OPAQUE one of `<html>` and `<body>`, and
  // null if neither is.
  //
  // Not `?? body ?? black`, which is what this was and which is wrong in a way that matters here:
  // `base.css` puts `background: var(--surface)` on `body` and nothing on `html`, so
  // `getComputedStyle(html).backgroundColor` is `rgba(0, 0, 0, 0)` — a value `parse` returns happily
  // and `??` therefore accepts. Every ratio would then have been measured against transparent black.
  // It never fired, because the ancestor walk reaches `body` and stops there; it would have fired the
  // first time a screen was measured outside one. A tool whose fallback is wrong only while it is
  // unreachable is the same shape as the compositing bug above, and it is not worth keeping.
  const opaque = (c) => (c && c.a >= 1 ? c : null);
  const base =
    opaque(parse(getComputedStyle(document.documentElement).backgroundColor)) ??
    opaque(parse(getComputedStyle(document.body).backgroundColor));

  for (const el of [root, ...root.querySelectorAll("*")]) {
    // Own text only. A wrapper's `textContent` is its children's, and reading it here would compare
    // a paragraph's colour against a card's background for every ancestor it has.
    const text = [...el.childNodes]
      .filter((n) => n.nodeType === 3)
      .map((n) => n.textContent)
      .join("")
      .trim();
    const cs = getComputedStyle(el);
    // TEXT ONLY, and in SVG that means `<text>` — the numeral inside a syncing hexagon — read off
    // `fill` rather than `color`.
    //
    // NOT every SVG node with a fill, which is what the first version took. A hexagon body carries
    // `fill: var(--surface)` to MASK the seam behind it: being exactly the colour it sits on is the
    // entire job, and the gate reported all nine of them as text that had vanished. Strokes are out
    // for the neighbouring reason — `--hex-syncing-track` is an inert ring drawn a shade off the
    // surface on purpose, and a legibility gate has no opinion about a track.
    //
    // Stated rather than hidden: a glyph whose STROKE is mapped to the wrong end of its ramp is not
    // covered here. `assert.mjs` compares strokes exactly, on eight drawn light frames, which is a
    // stronger check than this one — it is the seven UNDRAWN screens that have only this.
    if (!text) continue;
    const svgText = el.namespaceURI === "http://www.w3.org/2000/svg";
    // A mark drawn with a gradient has no single colour to compare; the same argument as a
    // background image, one property over.
    const fg = parse(svgText ? cs.fill : cs.color);
    if (!fg) continue;

    let bg = null;
    let alpha = Number(cs.opacity);
    let image = false;
    for (let node = el; node; node = node.parentElement) {
      const s = getComputedStyle(node);
      if (node !== el) alpha *= Number(s.opacity);
      if (s.backgroundImage && s.backgroundImage !== "none") {
        image = true;
        break;
      }
      const layer = parse(s.backgroundColor);
      if (!layer || layer.a === 0) continue;
      bg = bg == null ? layer : over(bg, layer);
      if (bg.a >= 1) break;
    }
    if (image) {
      out.push({ image: true });
      continue;
    }
    // NOTHING OPAQUE ANYWHERE ABOVE IT. Reported and counted rather than guessed at, exactly as a
    // background image is: there is no colour behind this text that the tool can name, and inventing
    // one is how the first version reported 1.00:1 on a button that was fine.
    if ((bg == null || bg.a < 1) && base == null) {
      out.push({ unresolved: true });
      continue;
    }
    bg = bg == null ? base : bg.a >= 1 ? bg : over(bg, base);
    // The opacity chain applies to the text as painted, so fold it into the foreground before
    // compositing. `--app-mark-quiet` is .65/.75 and is exactly this case.
    //
    // The chain stops where the background does, which is an approximation and a deliberate one: an
    // ancestor with opacity ABOVE the first opaque layer fades the text and that layer together, so
    // most of the effect cancels. Nothing in this design has one — the only two `opacity` users are
    // the header mark and the settled glow — so it is a limit rather than an error, and it would
    // want measuring the first time a screen fades a whole panel.
    const painted = over({ ...fg, a: fg.a * alpha }, bg);
    out.push({
      // `getAttribute`, not `className`: on an SVG element `className` is an `SVGAnimatedString`
      // and stringifies to `[object Object]`, which is what four rows of the first report said.
      key: el.getAttribute("data-fid") || el.getAttribute("class") || el.tagName.toLowerCase(),
      text: text.slice(0, 42),
      fg: [painted.r, painted.g, painted.b].map(Math.round),
      bg: [bg.r, bg.g, bg.b].map(Math.round),
    });
  }
  return out;
}.toString();

const labels = Object.keys(FIXTURES).sort();
const findings = [];
const report = [];
let compared = 0;
let skippedImage = 0;
let unresolved = 0;

for (const label of labels) {
  const seen = {};
  for (const scheme of ["dark", "light"]) {
    await page.emulateMediaFeatures([
      { name: "prefers-color-scheme", value: scheme },
      { name: "prefers-reduced-motion", value: "no-preference" },
    ]);
    await page.goto(`http://127.0.0.1:${port}/index.html?frame=${encodeURIComponent(label)}`, {
      waitUntil: "networkidle0",
    });
    // The ⋯ menu persists a choice that beats the media query, exactly as it does in assert.mjs —
    // and a stray value here would compare one theme against itself and report perfect parity.
    const had = await page.evaluate(() => {
      const saved = localStorage.getItem("theme");
      localStorage.removeItem("theme");
      return saved;
    });
    if (had) await page.reload({ waitUntil: "networkidle0" });
    await page.waitForSelector("#app-root > *", { timeout: 5000 }).catch(() => {});
    // PROVE IT FAILS. `S10_CONTRAST_POISON=1` leaves one accent at its DARK value in the light pass —
    // `--up-label`, `#FF9F1C`, which is precisely "the luminous end left where the darker end
    // belongs" — and the run must then exit 1 naming the eyebrows that carry it. A gate nobody has
    // watched fail is a gate nobody should believe, and this one's thresholds were placed in a gap
    // measured from passing data, which is exactly the way to build a check that can only ever pass.
    // `!important` because a custom property declared on `:root` loses to `tokens.css`'s
    // `:root:not([data-theme="dark"])`, which is one selector more specific.
    if (scheme === "light" && process.env.S10_CONTRAST_POISON) {
      await page.addStyleTag({ content: ":root { --up-label: #ff9f1c !important; }" });
    }
    seen[scheme] = await page.evaluate(`(${READ_NODES})()`);
  }

  // POSITIONAL PAIRING, and it holds because the two runs render the same fixture through the same
  // code: light changes which value a token resolves to and nothing about which nodes exist. A
  // length mismatch would mean the theme changed the TREE, which is itself the finding — so it is
  // reported rather than tolerated.
  if (seen.dark.length !== seen.light.length) {
    findings.push({
      label,
      key: "(tree)",
      what: `${seen.dark.length} text nodes in dark against ${seen.light.length} in light — a theme must not change which nodes exist`,
    });
    continue;
  }

  for (const [i, d] of seen.dark.entries()) {
    const l = seen.light[i];
    if (d.image || l.image) {
      skippedImage++;
      continue;
    }
    if (d.unresolved || l.unresolved) {
      unresolved++;
      continue;
    }
    const rgb = ([r, g, b]) => ({ r, g, b });
    const dark = contrast(rgb(d.fg), rgb(d.bg));
    const light = contrast(rgb(l.fg), rgb(l.bg));
    compared++;
    report.push({ label, key: l.key, text: l.text, dark, light, parity: light / dark });
    if (light < LIGHT_FLOOR && light / dark < PARITY_FLOOR) {
      findings.push({
        label,
        key: l.key,
        what:
          `"${l.text}" carries ${dark.toFixed(2)}:1 in dark and ${light.toFixed(2)}:1 in light ` +
          `(${Math.round((light / dark) * 100)}% of it) — a token mapped to the wrong end of its ramp`,
      });
    }
  }
}

await browser.close();
server.close();

// `--report` prints the whole distribution, which is how `PARITY_FLOOR` was chosen rather than
// guessed. It is not part of the gate; the gate is the two thresholds above.
if (process.argv.includes("--report")) {
  report.sort((a, b) => a.parity - b.parity);
  const path = process.env.CONTRAST_REPORT ?? "contrast-report.tsv";
  writeFileSync(
    path,
    ["frame\tnode\ttext\tdark\tlight\tparity"]
      .concat(
        report.map(
          (r) =>
            `${r.label}\t${r.key}\t${r.text}\t${r.dark.toFixed(2)}\t${r.light.toFixed(2)}\t${r.parity.toFixed(3)}`,
        ),
      )
      .join("\n") + "\n",
  );
  console.log(`  wrote ${report.length} rows to ${path}`);
  for (const r of report.slice(0, 12)) {
    console.log(
      `    ${r.parity.toFixed(2)}  ${r.dark.toFixed(2)} → ${r.light.toFixed(2)}  ${r.label} · ${r.text}`,
    );
  }
}

console.log(
  `fidelity:contrast — ${compared} text nodes across ${labels.length} frames read in both themes, ` +
    `${skippedImage} over a gradient and ${unresolved} over nothing opaque (no single colour to ` +
    `compare), ${findings.length} failures`,
);

if (findings.length) {
  console.error("");
  for (const f of findings) console.error(`  ${f.label} · ${f.key}\n      ${f.what}`);
  console.error(`\nfidelity:contrast: ${findings.length} failure(s).`);
  process.exit(1);
}
