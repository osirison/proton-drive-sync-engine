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

import { mkdirSync, writeFileSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import puppeteer from "puppeteer";
import { OUT_OF_SCOPE, classify } from "./frame-classes.mjs";
import { STYLE_PROPS, SVG_ATTRS, INITIAL } from "./props.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const PROTOTYPE = join(REPO, "docs", "design-v2", "Drive Sync.dc.html");
// Overridable so check-stale.mjs can re-extract somewhere else and compare, rather than overwriting
// the committed fixtures to find out whether they needed overwriting.
const OUT_DIR = process.argv[2] ? resolve(process.argv[2]) : join(HERE, "frames");
const SRC_STYLES = join(REPO, "gui", "src", "styles");

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

// FREEZE EVERY ANIMATION BEFORE MEASURING. Otherwise the harness records animation PHASE as ground
// truth: `opacity` under `breathe` sampled at 0.82 one run and 0.79 the next, and a `blip` dot
// measured 8.8px wide because getBoundingClientRect includes the 1.5x transform mid-cycle. Both are
// real values; neither is the design.
//
// The Web Animations API rather than a `animation-play-state: paused` stylesheet, because the
// declarations themselves are asserted — animation-name, duration, delay and timing-function are all
// in STYLE_PROPS, and overriding them to measure them is the mistake that would make the gate agree
// with itself. Seeking to 0 pins the first keyframe without touching what is declared.

const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
// Wide enough that no frame is squeezed by the viewport. Computed styles do not depend on scroll
// position, so the whole document is read in one pass.
await page.setViewport({ width: 1400, height: 1000 });
await page.goto(pathToFileURL(PROTOTYPE).href, { waitUntil: "networkidle0" });

// THE PROTOTYPE HAS NO @font-face. It names 'Instrument Sans' and 'IBM Plex Mono' and falls back to
// whatever the machine happens to have — which on a machine that has neither is a different font on
// Fedora than on ubuntu-latest. Without this the fixtures encode the EXTRACTING MACHINE'S fallback,
// every text node's box is wrong by a few pixels somewhere else, and CI can never agree with a
// developer. It is the failure CI caught on the harness's first run.
//
// F1 bundled these faces for exactly this reason — "font metrics move every measurement in
// docs/design-v2, so no fidelity assertion is meaningful until F1 lands" — so the fix is to give the
// prototype the same faces the app loads, from the same files, and measure once they are ready.
//
// ONLY the @font-face blocks. base.css also carries the global `box-sizing: border-box` reset, and
// injecting that would quietly give the prototype the app's box model — erasing the very divergence
// DEVIATIONS.md §48 exists to record, and making the gate agree by changing the ground truth. The
// url()s are relative to base.css, so the tag is given a matching base href.
const FONT_FACES = readFileSync(join(SRC_STYLES, "base.css"), "utf8").match(/@font-face\s*\{[^}]*\}/g) ?? [];
if (FONT_FACES.length === 0) throw new Error("extract: no @font-face blocks found in base.css");
await page.addStyleTag({
  content: FONT_FACES.join("\n").replace(
    /url\(["']?\.\.\/fonts\//g,
    `url("${pathToFileURL(join(SRC_STYLES, "..", "fonts")).href}/`,
  ),
});
await page.evaluate(() => document.fonts.ready);
await page.evaluate(() => {
  for (const animation of document.getAnimations()) {
    animation.currentTime = 0;
    animation.pause();
  }
});

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
        // The whole subtree's text, recorded only when it differs from this node's own. The copy
        // deck's sentences are routinely split by an inline child — 25 <strong> elements mid-
        // paragraph — so "Yours has buy milk where Proton's..." is own-text "Yours has where
        // Proton's..." plus a <strong>buy milk</strong>. The copy gate needs the joined form; the
        // style gate needs to know which node actually holds the words. Record both.
        const fullText = (el.textContent || "").replace(/\s+/g, " ").trim();
        // User-visible strings that are NOT text nodes. A placeholder is copy — "Add a rule — e.g.
        // *.psd or scratch/**" appears nowhere in any textContent — and a gate that cannot see it
        // silently exempts every input in the design.
        const attrs = {};
        for (const a of ["placeholder", "value", "title", "aria-label", "alt"]) {
          const v = el.getAttribute(a);
          if (v) attrs[a] = v;
        }
        const box = el.getBoundingClientRect();
        return {
          key: keyOfNode(el, frame),
          tag: el.tagName.toLowerCase(),
          text: ownText || undefined,
          fullText: fullText && fullText !== ownText ? fullText : undefined,
          attrs: Object.keys(attrs).length ? attrs : undefined,
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
