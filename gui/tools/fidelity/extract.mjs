// Turn the drawn prototype into checked-in ground truth: one JSON per in-scope frame, a normalised
// node tree of {key, tag, text, styles, svgAttrs}. `npm run fidelity:extract` regenerates them, and
// the diff is the review — a prototype edit that moves a number shows up as a line, not a surprise.
//
// IT RENDERS RATHER THAN PARSES, which is a departure from the F8 issue's wording and the whole
// reason the output is usable. assert.mjs reads the app's styles off `getComputedStyle`, so the
// prototype's have to come from the same place or the two sides are not comparable: a parse gives
// you `padding:0 20px` where the app gives you four longhand pixel values, and every cascade,
// inheritance and default is missing. Recorded in DEVIATIONS.md §47.
//
// The engine is Chromium (puppeteer). The app's real runtime is WebKitGTK and the issue asks for
// Playwright's WebKit, which is the right instinct — but it cannot be installed here (it wants
// libicu74/libjpeg-turbo8 through sudo), and a gate nobody can run locally is a gate nobody
// develops. `launch()` is a parameter for exactly this reason; see the README.

import { mkdirSync, writeFileSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import puppeteer from "puppeteer";
import { OUT_OF_SCOPE, classify } from "./frame-classes.mjs";
import { STYLE_PROPS, SVG_ATTRS, INITIAL } from "./props.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const PROTOTYPE = join(REPO, "docs", "design-v2", "Drive Sync.dc.html");
const OUT_DIR = join(HERE, "frames");

/**
 * The node key. Path-based, one segment per level, `tag` plus an index among same-tag element
 * siblings when there is more than one — `header/img`, `div[1]/svg`, `nav/div/button[2]`.
 *
 * WHY PATHS AND NOT NAMES. The F8 issue flags this as a decision to make before writing this file,
 * and offers a hand-maintained mapping as the alternative. Paths win on one argument: the app's tree
 * is NOT the prototype's tree and never will be — F4 already wraps the app mark in a button the
 * frames draw bare — so a key can only ever be an identifier that a human attaches to the app node
 * they judge to correspond. Given that, it should at least be derivable, diffable and unambiguous,
 * which a hand-maintained list of names is not.
 *
 * The cost is stated in the issue and accepted: a prototype edit that changes structure renames
 * keys, and the `frames/*.json` diff will show it. That is a review, not a silent break — assert.mjs
 * fails loudly on a `data-fid` that matches no key.
 */
function keyOf(node, stopAt) {
  const parts = [];
  let el = node;
  while (el && el !== stopAt) {
    const parent = el.parentElement;
    if (!parent) break;
    const sameTag = [...parent.children].filter((c) => c.tagName === el.tagName);
    const tag = el.tagName.toLowerCase();
    parts.unshift(sameTag.length > 1 ? `${tag}[${sameTag.indexOf(el)}]` : tag);
    el = parent;
  }
  return parts.join("/");
}

const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
// Wide enough that no frame is squeezed by the viewport. Computed styles do not depend on scroll
// position, so the whole document is read in one pass.
await page.setViewport({ width: 1400, height: 1000 });
await page.goto(pathToFileURL(PROTOTYPE).href, { waitUntil: "networkidle0" });

const frames = await page.evaluate(
  (OOS, PROPS, ATTRS, keySrc, INITIALS) => {
    const keyOfNode = new Function(`return ${keySrc}`)();
    const out = [];

    for (const frame of document.querySelectorAll("[data-screen-label]")) {
      const label = frame.getAttribute("data-screen-label");
      if (OOS.includes(label)) continue;

      const record = (el) => {
        const cs = getComputedStyle(el);
        const styles = {};
        for (const p of PROPS) {
          const v = cs.getPropertyValue(p);
          // Omitted when it equals the initial value — see INITIAL in props.mjs. Lossless, and the
          // difference between a 4.4 MB dump and a fixture a human can read the diff of.
          if (v !== INITIALS[p]) styles[p] = v;
        }
        const svgAttrs = {};
        for (const a of ATTRS) {
          const v = el.getAttribute(a);
          if (v != null) svgAttrs[a] = v;
        }
        // Own text only — the concatenation of every descendant's text is not this node's copy, and
        // the copy gate needs to know which node actually says a sentence.
        const ownText = [...el.childNodes]
          .filter((n) => n.nodeType === 3)
          .map((n) => n.textContent)
          .join("")
          .replace(/\s+/g, " ")
          .trim();
        const box = el.getBoundingClientRect();
        return {
          key: keyOfNode(el, frame),
          tag: el.tagName.toLowerCase(),
          text: ownText || undefined,
          box: { w: +box.width.toFixed(2), h: +box.height.toFixed(2) },
          styles,
          svgAttrs: Object.keys(svgAttrs).length ? svgAttrs : undefined,
        };
      };

      // THE ROOT IS INCLUDED DELIBERATELY. DEVIATIONS.md's caveat on the F1 method: a frame is not
      // its own descendant, so walking only descendants made the window's own border invisible and
      // shipped a wrong `--border-subtle` to the light theme before §8a caught it. `keyOf` returns
      // "" for the root, which is its key.
      const nodes = [record(frame), ...[...frame.querySelectorAll("*")].map(record)];
      const fb = frame.getBoundingClientRect();
      out.push({
        label,
        width: +fb.width.toFixed(2),
        height: +fb.height.toFixed(2),
        nodes,
      });
    }
    return out;
  },
  [...OUT_OF_SCOPE],
  STYLE_PROPS,
  SVG_ATTRS,
  keyOf.toString(),
  INITIAL,
);

await browser.close();

// Rewritten wholesale so a frame that disappears from the prototype disappears from here too —
// a stale fixture asserting a screen nobody draws any more is worse than no fixture.
rmSync(OUT_DIR, { recursive: true, force: true });
mkdirSync(OUT_DIR, { recursive: true });

const safe = (label) => label.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
const index = [];
for (const frame of frames) {
  const kind = classify(frame.label, frame.width);
  const file = `${safe(frame.label)}.json`;
  writeFileSync(
    join(OUT_DIR, file),
    JSON.stringify(
      { label: frame.label, kind, width: frame.width, height: frame.height, nodes: frame.nodes },
      null,
      2,
    ) + "\n",
  );
  index.push({ label: frame.label, kind, file, nodes: frame.nodes.length });
}
index.sort((a, b) => a.label.localeCompare(b.label));
writeFileSync(join(OUT_DIR, "index.json"), JSON.stringify(index, null, 2) + "\n");

const byKind = index.reduce((acc, f) => ((acc[f.kind] = (acc[f.kind] ?? 0) + 1), acc), {});
console.log(
  `fidelity:extract — ${index.length} frames, ${index.reduce((n, f) => n + f.nodes, 0)} nodes ` +
    `(${Object.entries(byKind)
      .map(([k, n]) => `${n} ${k}`)
      .join(", ")}) -> ${readdirSync(OUT_DIR).length} files`,
);
if (index.length !== 51) {
  console.error(`fidelity:extract: expected 51 in-scope frames, got ${index.length}`);
  process.exit(1);
}
