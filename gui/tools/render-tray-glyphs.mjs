// The five symbolic SVGs the desktop loads for the tray icon (S8).
//
// ONE GEOMETRY SOURCE, AND THE LINK IS NOT "THE SAME FUNCTION". These files are produced by opening
// `?frame=10a Glyph states` — the very sheet `assert.mjs` compares node by node against the drawing
// — and lifting the rendered marks out of it. So the icon on a panel and the mark in the gate are
// the same node, not two drawings that agree on the day someone checked. A hand-authored second copy
// of the hexagon would be a drawing nothing compares, in the one surface of the app where a mistake
// is least visible: a 16px icon that a person sees for a second at a time and never inspects.
//
// WHY FILES AT ALL. `tray-icon`'s Linux backend rasterises to a temp PNG and hands the path to
// libappindicator, so nothing about a theme can reach it. S8 replaces that with a hand-rolled
// StatusNotifierItem, which names its icon (`IconName`) and where to find it (`IconThemePath`) —
// and a name is what lets the DESKTOP do the drawing, at its own size and in its own colour.
//
// THREE TRANSFORMS, each one a thing an icon file cannot inherit from a webview:
//
//   1. `var(--hex-glyph-fg)` becomes `currentColor`, under the Breeze `ColorScheme-Text`
//      convention. VERIFIED ON A REAL PLASMA SESSION rather than assumed: an item whose SVG
//      declared `#ff00ff` rendered WHITE in the panel, at the same pixels a plain magenta SVG had
//      rendered magenta a minute earlier. That measurement is the whole reason the mono forms are
//      what ship — `10-tray.md`'s "state is carried by fill, not hue" stops being a design principle
//      and becomes the mechanism, because the desktop is going to pick the hue.
//   2. `--line-inert` — the syncing glyph's track, and the one mono value that is NOT the foreground
//      — becomes `currentColor` at reduced opacity. A recoloured icon has exactly one colour, so a
//      second literal would either survive recolouring (and clash with whatever the panel chose) or
//      be recoloured to match the segment (and vanish). Opacity is the only form of "dimmer than the
//      foreground" that means the same thing against any panel. DEVIATIONS §82d.
//   3. The animation goes. `renderHexagon` puts `animation:hexup 2.4s linear infinite` in a style
//      attribute; there is no CSS engine behind a tray icon and the SNI protocol has no notion of a
//      moving icon. The syncing glyph ships as its segment frozen where the sheet draws it, and the
//      motion the design asks for lives in the panel, which is a real webview. §82e.
//
// Run `npm run glyphs` to rewrite the files, `npm run glyphs:check` to fail when they are stale.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import { serve } from "./fidelity/serve.mjs";
import { TRAY_GLYPH_STATES } from "../src/js/ui/hexagon.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, "..", "src-tauri", "icons", "tray");

/** The icon-theme name each form is published under. `IconName` in the SNI item. */
export const ICON_NAME = {
  settled: "proton-sync-uptodate-symbolic",
  syncing: "proton-sync-syncing-symbolic",
  needsYou: "proton-sync-attention-symbolic",
  paused: "proton-sync-paused-symbolic",
  unreachable: "proton-sync-offline-symbolic",
};

/**
 * The track's share of the foreground, and it is measured rather than chosen: `#3E454E` against
 * `#E8EBF0` on the `#191C21` the sheet draws on. Compositing the track over that background at
 * alpha a gives 0x19 + a(0xE8 − 0x19) = 0x3E on the red channel, so a ≈ 0.16 — but the same
 * arithmetic on green and blue lands at 0.19 and 0.20, because the two colours are not the same hue.
 * 0.18 is the middle of the three and within a channel value of all of them.
 *
 * The important part is that this is the ratio the DRAWING has, not a value that looked right: the
 * recoloured icon reproduces the sheet's contrast between track and segment whatever colour the
 * panel picks.
 */
const TRACK_OPACITY = 0.18;

const check = process.argv.includes("--check");

const { server, port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
await page.setViewport({ width: 700, height: 700, deviceScaleFactor: 1 });
await page.goto(`http://127.0.0.1:${port}/?frame=${encodeURIComponent("10a Glyph states")}`, {
  waitUntil: "networkidle0",
});

/**
 * Lift the ten marks out of the sheet, resolving every colour to what the ENGINE computed.
 *
 * The marks are stamped `data-fid` by the fixture, so they are addressed the same way the gate
 * addresses them — `glyph` slot, index 0..9, mono in the even positions. Reading the attribute back
 * would give `var(--hex-glyph-fg)`, which means nothing in a standalone file; `getComputedStyle`
 * gives `rgb(232, 235, 240)`, which is comparable against the token and therefore mappable.
 */
const marks = await page.evaluate(() => {
  // A DESCRIPTION, NOT MARKUP. The first version returned `clone.outerHTML` and the files it wrote
  // did not reproduce on another machine: `glyphs:check` passed here and failed all five on CI.
  // Serialising an SVG subtree is the browser's decision — whether a namespace is re-declared on the
  // root, how an empty element is closed, how a number is spelled — and none of that is design.
  //
  // So the page returns the tree as data and THIS FILE writes the string, from a fixed attribute
  // order. What crosses the boundary is what the design decided; the spelling is ours.
  const KEEP = [
    "d",
    "viewBox",
    "cx",
    "cy",
    "r",
    "fill",
    "stroke",
    "stroke-width",
    "stroke-dasharray",
    "stroke-linecap",
    "stroke-linejoin",
    "opacity",
  ];
  const out = [];
  for (const svg of document.querySelectorAll(".glyph-cell svg")) {
    const nodes = [];
    for (const node of [svg, ...svg.querySelectorAll("*")]) {
      const computed = getComputedStyle(node);
      const attrs = {};
      for (const name of KEEP) {
        // The two colour attributes are read as the ENGINE COMPUTED them: the app writes
        // `stroke="var(--hex-glyph-fg)"`, which means nothing in a standalone file, and
        // `getComputedStyle` gives `rgb(232, 235, 240)` — comparable against the token, and
        // therefore mappable onto `currentColor` below.
        if (name === "stroke" || name === "fill") {
          if (node.hasAttribute(name)) attrs[name] = computed[name];
          continue;
        }
        // THE PAUSED MARK'S `opacity: .45` IS NOT DECORATION — `10-tray.md` lists it in the
        // construction table beside the dash, and it is half of what makes that form the paused
        // one. It arrives via `svg.style`, not an attribute, so it has to be read from the computed
        // value or it is silently lost (it was, once: the icon came out at full strength and no
        // longer read as dimmed). Opacity is also the one property that survives recolouring, being
        // alpha rather than hue.
        if (name === "opacity") {
          if (Number(computed.opacity) < 1) attrs.opacity = String(Number(computed.opacity));
          continue;
        }
        if (node.hasAttribute(name)) attrs[name] = node.getAttribute(name);
      }
      nodes.push({ tag: node.tagName.toLowerCase(), attrs });
    }
    out.push({
      nodes,
      fg: getComputedStyle(document.documentElement).getPropertyValue("--hex-glyph-fg").trim(),
    });
  }
  return out;
});

if (marks.length !== TRAY_GLYPH_STATES.length * 2) {
  throw new Error(
    `render-tray-glyphs: expected ${TRAY_GLYPH_STATES.length * 2} marks on the sheet, found ${marks.length}. ` +
      "The sheet draws each form twice (mono, colour) — if that changed, this tool is reading the wrong nodes.",
  );
}

await browser.close();
server.close();

/** `#e8ebf0` → `rgb(232, 235, 240)`, so a token can be compared against a computed value. */
function toRgb(hex) {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) throw new Error(`render-tray-glyphs: --hex-glyph-fg is not a 6-digit hex: ${hex}`);
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

/** One element, spelled by us. Attribute order is `KEEP`'s order, so it is the same everywhere. */
const tag = (node, extra = {}) => {
  const attrs = { ...node.attrs, ...extra };
  const pairs = Object.entries(attrs).map(([name, value]) => ` ${name}="${value}"`);
  return `<${node.tag}${pairs.join("")}/>`;
};

const files = new Map();
TRAY_GLYPH_STATES.forEach((state, row) => {
  // The MONO form — the even cell. The colour form exists on the sheet to show what a desktop that
  // allows hue would add; what ships is the one that survives a desktop that does not.
  const { nodes, fg } = marks[row * 2];
  const foreground = toRgb(fg);
  const [root, ...children] = nodes;

  // Every colour is now an `rgb(...)` literal, and there are exactly two possibilities: the
  // foreground, or the syncing track. Anything else means a form started drawing in a colour this
  // tool does not know how to make symbolic, and guessing would ship an icon with a hardcoded hue.
  const track = [
    ...new Set(
      children
        .flatMap((node) => [node.attrs.stroke, node.attrs.fill])
        .filter((c) => c && c !== "none" && c !== foreground),
    ),
  ];
  if (track.length > 1) {
    throw new Error(
      `render-tray-glyphs: the ${state} glyph draws ${track.length} colours that are not the foreground ` +
        `(${track.join(", ")}). A symbolic icon has one colour plus opacity — see the header.`,
    );
  }

  const body = children
    .map((node) => {
      const attrs = { class: "ColorScheme-Text", ...node.attrs };
      for (const name of ["stroke", "fill"]) {
        if (attrs[name] === foreground) attrs[name] = "currentColor";
      }
      // The track becomes the foreground at reduced alpha — see the header. A second literal
      // colour cannot survive recolouring: it would either clash with whatever the panel chose or
      // be recoloured to match the segment and disappear.
      if (track.length === 1 && attrs.stroke === track[0]) {
        attrs.stroke = "currentColor";
        attrs["stroke-opacity"] = TRACK_OPACITY;
      }
      return tag({ ...node, attrs });
    })
    .join("");

  // The Breeze stylesheet block. KDE rewrites the declaration inside `#current-color-scheme` to the
  // panel's own text colour, which is what `currentColor` above then resolves to. A desktop that
  // does not know the convention simply gets the declared value, which is the design's own
  // foreground — so the fallback is correct rather than merely safe.
  const style =
    '<defs><style type="text/css" id="current-color-scheme">' +
    `.ColorScheme-Text { color: ${fg}; }` +
    "</style></defs>";

  // The sheet renders at 20 (see `fixtures/tray.js`); the file declares 16, which is the size
  // `10-tray.md` names and the bottom of its range. Neither number is geometry — the viewBox is —
  // so this is what the desktop starts from before it scales to its own panel height.
  const rootAttrs = Object.entries({
    xmlns: "http://www.w3.org/2000/svg",
    width: 16,
    height: 16,
    ...root.attrs,
  })
    .map(([name, value]) => ` ${name}="${value}"`)
    .join("");
  const svg = `<svg${rootAttrs}>${style}${body}</svg>`;

  const header =
    `<!-- GENERATED by gui/tools/render-tray-glyphs.mjs from the ${state} mark on \`10a Glyph states\`.\n` +
    "     Do not edit: run `npm run glyphs`. `npm run glyphs:check` fails the build when this is stale. -->\n";
  files.set(join(OUT, `${ICON_NAME[state]}.svg`), `${header}${svg}\n`);
});

mkdirSync(OUT, { recursive: true });
const stale = [];
for (const [path, body] of files) {
  // A missing file is stale, not an error — that is the first run, and the `--check` branch below
  // has to report it rather than crash.
  let current;
  try {
    current = readFileSync(path, "utf8");
  } catch {
    current = null;
  }
  if (current === body) continue;
  stale.push(path.replace(`${resolve(HERE, "..", "..")}/`, ""));
  if (!check) writeFileSync(path, body);
}

if (check && stale.length) {
  console.error(
    `\nglyphs:check — ${stale.length} tray icon(s) do not match what ui/hexagon.js draws:\n` +
      stale.map((p) => `  ${p}`).join("\n") +
      "\n\nThe icon a desktop loads and the mark the fidelity gate compares are supposed to be the same\n" +
      "drawing. Run `npm run glyphs` and commit the result.\n",
  );
  process.exit(1);
}
console.log(
  check
    ? `glyphs:check — ${files.size} tray icons match ui/hexagon.js`
    : `glyphs — wrote ${stale.length} of ${files.size} tray icons to src-tauri/icons/tray`,
);
