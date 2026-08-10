// The style and fit gates (F8). Serves gui/src, opens each frame's fixture URL at exactly
// 1040×764, and compares every mapped node's computed styles against frames/*.json.
//
// HOW A NODE IS MAPPED. The app marks a node with `data-fid="<frame-label>:<node-key>"`, where the
// key is the path extract.mjs derived. There is no automatic correspondence and there cannot be:
// the app's tree is not the prototype's tree (F4 wraps the app mark in a button the frames draw
// bare), so a human decides which app node stands for which drawn node and says so in an attribute.
//
// WHAT THIS CAN CHECK TODAY, and why that is not a disappointment. Every route is a placeholder
// until S1–S11 land, so the app renders a header, a body and a footer — and those are exactly what
// F4 built, so those are what this asserts. The gate is built, proven against a deliberately-wrong
// value, and wired into CI now; it gains teeth one screen at a time, and each S-task's definition
// of done is "my frames pass". Building it after the screens would mean eleven screens written
// against no gate at all.
//
// WHAT IT CANNOT COVER, ever, and says so rather than implying otherwise:
//   · the seven screens with no drawn light frame — S10 asserts those against 12-light-theme.md's
//     mapping table, which is prose, not a drawn artefact;
//   · whether an animation LOOKS right. Only the declaration is comparable — `animation-name`,
//     duration and delay. A wrong easing that parses is invisible here.
//   · native tray rendering and the desktop's own notification chrome. Neither is a webview.

import { createServer } from "node:http";
import { readFileSync, statSync } from "node:fs";
import { dirname, extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import {
  STYLE_PROPS,
  SVG_ATTRS,
  COLOUR_ATTRS,
  compare,
  compareSvgAttr,
  valueOf,
  LENGTH_TOLERANCE_PX,
  boxComparability,
} from "./props.mjs";
import { FIXTURES } from "../../src/js/fixtures/frames.js";
import { OWES_BOX, OWES_FIT } from "./frame-classes.mjs";
import { isKnown, unmetDeviations, KNOWN_DEVIATIONS } from "./known-deviations.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(HERE, "..", "..", "src");
const FRAMES = join(HERE, "frames");

/**
 * The frames that must contain no hue — every surface whose whole message is "nothing to report".
 *
 * Not derived from the label: `10a Settled` is a tray panel and `2a Compact settled` a 360px one,
 * and both are the rule as much as the 1040 window is. A settled surface is a judgement about what
 * the frame SAYS, which is what makes it a list.
 */
const SETTLED_FRAMES = new Set([
  "2a Settled",
  "12a Settled light",
  "2a Compact settled",
  "12a Compact settled light",
  "10a Settled",
]);

/**
 * Saturation above which a colour counts as a hue — **HSV**, `(max − min) / max`.
 *
 * TWO MEASUREMENTS SHAPE THIS, and the second one is why it is HSV and not HSL.
 *
 * First: "no colour" cannot be tested as "grey". This design's neutral ramp is deliberately COOL and
 * not one tier of it is achromatic — `#828B98` and `#6D7783` both spread 22 channel values, so a
 * plain `max − min` test flags the entire palette. It has to be a saturation.
 *
 * Second: the obvious saturation is the wrong one. Under HSL, lightness divides out the chroma, so a
 * near-white or near-black neutral reads as vividly coloured — light's own `--surface` is `#FAF8F5`,
 * a five-value warm tint, and HSL calls it **0.33 saturated**, more than any threshold that catches
 * `#22D3EE` could allow. It failed `12a Compact settled light` on the surface the whole light theme
 * is painted on. HSV divides by `max` instead, which is stable at both ends: `#FAF8F5` is 0.02,
 * `#C9D0DA` 0.08, and the darkest neutrals — `#23262D`, `#2E323A` — top out at 0.22.
 *
 * Every accent in either palette is at least 0.85: `#22D3EE` 0.86, `#B23F14` 0.89, `#FF9F1C` 0.89,
 * `#0E7490` 0.90, `#BE123C` 0.94. A gap from 0.22 to 0.85 is not a knob to tune.
 */
const HUE_LIMIT = 0.5;

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".png": "image/png",
};

// Served rather than opened as file:// because the app is ES modules and a module graph over
// file:// hits cross-origin rules the real webview never does. Tauri serves gui/src the same way.
function serve() {
  return new Promise((ready) => {
    const server = createServer((req, res) => {
      const rel = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(
        /^(\.\.[/\\])+/,
        "",
      );
      let file = join(SRC, rel);
      try {
        if (statSync(file).isDirectory()) file = join(file, "index.html");
      } catch {
        res.writeHead(404).end("not found");
        return;
      }
      try {
        const body = readFileSync(file);
        res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" }).end(body);
      } catch {
        res.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => ready({ server, port: server.address().port }));
  });
}

const { server, port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
// Exactly the window. `deviceScaleFactor:1` so a length is a CSS pixel and nothing is rounded twice.
await page.setViewport({ width: 1040, height: 764, deviceScaleFactor: 1 });

const index = JSON.parse(readFileSync(join(FRAMES, "index.json"), "utf8"));

// A hardcoded label list that nothing cross-checks is a gate that can switch itself off: re-extract
// the prototype under a renamed frame and `SETTLED_FRAMES.has(label)` quietly goes false, with the
// run printing exactly what it printed before. The same argument as the stale-deviation check, one
// gate over.
const unknownSettled = [...SETTLED_FRAMES].filter((label) => !index.some((e) => e.label === label));
if (unknownSettled.length) {
  console.error(
    `fidelity:assert: SETTLED_FRAMES names ${unknownSettled.length} frame(s) that are not in scope — ` +
      `the hue gate would skip them silently: ${unknownSettled.join(", ")}`,
  );
  process.exit(1);
}

const failures = [];
const deviations = [];
/** Route a mismatch to the failure list, or to the recorded-deviation list if one names it. */
const record = (row) => (isKnown(row.frame, row.key, row.prop, row.detail) ? deviations : failures).push(row);
let asserted = 0;
let mapped = 0;
const unmappedFrames = [];
/** Per frame: the slots its fixture declares that the running app never stamped. */
const deadSlots = [];

for (const entry of index) {
  const frame = JSON.parse(readFileSync(join(FRAMES, entry.file), "utf8"));
  const expected = new Map(frame.nodes.map((n) => [n.key, n]));
  const boxComparable = boxComparability(frame.nodes);

  // PIN THE COLOUR SCHEME. tokens.css publishes the light palette under
  // `@media (prefers-color-scheme: light)`, and a headless browser's default for that query is a
  // property of the platform — dark on this developer's box, light on ubuntu-latest. Unpinned, the
  // gate compared a dark frame against a light app and produced 187 colour failures in CI that no
  // developer could reproduce. The frame decides: the `12a` set IS the light theme, everything else
  // is drawn dark.
  const scheme = frame.label.startsWith("12a") ? "light" : "dark";
  await page.emulateMediaFeatures([
    { name: "prefers-color-scheme", value: scheme },
    // `prefers-reduced-motion` is the same hazard, found by auditing rather than by CI. FOUR of the
    // app's stylesheets answer it and every one sets `animation: none` or `transition: none` — and
    // animation-name/duration/delay/timing-function are all asserted properties. The prototype has
    // no reduced-motion rules anywhere (DEVIATIONS.md §30), so a runner that reported `reduce` would
    // compare a motionless app against animated ground truth and fail exactly as the colour scheme
    // did. Both values are legitimate app states; the gate has to say which one it is looking at.
    { name: "prefers-reduced-motion", value: "no-preference" },
    // `forced-colors` is NOT pinned: puppeteer rejects it as an unsupported media feature. Nothing
    // in the app answers it today, so nothing is unpinned in practice — but the first stylesheet
    // that does will need a way to fix it, and this is where to look.
  ]);

  await page.goto(`http://127.0.0.1:${port}/index.html?frame=${encodeURIComponent(frame.label)}`, {
    waitUntil: "networkidle0",
  });
  // The ⋯ menu persists an explicit choice in localStorage, and `:root[data-theme]` beats the media
  // query by design — so a stray value would silently override the emulation above. localStorage
  // survives navigation within a browser context, so clear it and reload rather than clearing after
  // the app has already read it.
  const hadTheme = await page.evaluate(() => {
    const saved = localStorage.getItem("theme");
    localStorage.removeItem("theme");
    return saved;
  });
  if (hadTheme) await page.reload({ waitUntil: "networkidle0" });
  // The shell renders on its first status poll; wait for it rather than racing it.
  await page.waitForSelector("#app-root > *", { timeout: 5000 }).catch(() => {});

  // --- the hue gate. `03-main-screen.md`: "No seam. No colour anywhere. This is the rule made
  // visible — if a screen has nothing to report it must contain no hue at all." It is a line item on
  // the design's own acceptance checklist, assigned to this harness, and nothing implemented it
  // until a settled screen existed to point it at.
  if (SETTLED_FRAMES.has(frame.label)) {
    const hues = await page.evaluate((limit) => {
      // Only what actually PAINTS. `outline-color`, `caret-color`, `text-decoration-color` and
      // `column-rule-color` all default to `currentColor`, so including them reports one text colour
      // five times and buries the finding; a border colour paints only if the border has width.
      const parse = (v) => {
        const m = /rgba?\(([^)]+)\)/.exec(v);
        if (!m) return null;
        const p = m[1]
          .split(/[\s,/]+/)
          .filter(Boolean)
          .map(Number);
        return p.length < 3 ? null : { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
      };
      // HSV saturation — chroma over the brightest channel. See HUE_LIMIT for why not HSL, which
      // reads light's own near-white surface as a third saturated.
      const saturation = ({ r, g, b }) => {
        const max = Math.max(r, g, b);
        return max === 0 ? 0 : (max - Math.min(r, g, b)) / max;
      };
      const out = [];
      const check = (el, prop, value) => {
        for (const raw of value.match(/rgba?\([^)]+\)/g) ?? []) {
          const c = parse(raw);
          if (!c || c.a === 0) continue;
          const s = saturation(c);
          if (s > limit) {
            out.push(
              `${el.tagName.toLowerCase()}${el.getAttribute("class") ? `.${el.getAttribute("class").split(" ")[0]}` : ""} ${prop} ${raw} (saturation ${s.toFixed(2)})`,
            );
          }
        }
      };
      for (const el of document.querySelectorAll("#app-root, #app-root *")) {
        const cs = getComputedStyle(el);
        check(el, "color", cs.color);
        check(el, "background-color", cs.backgroundColor);
        if (cs.backgroundImage !== "none") check(el, "background-image", cs.backgroundImage);
        for (const side of ["top", "right", "bottom", "left"]) {
          if (parseFloat(cs.getPropertyValue(`border-${side}-width`)) > 0) {
            check(el, `border-${side}-color`, cs.getPropertyValue(`border-${side}-color`));
          }
        }
        for (const p of ["fill", "stroke", "stop-color"]) {
          const v = cs.getPropertyValue(p);
          if (v && v !== "none") check(el, p, v);
        }
      }
      return [...new Set(out)];
    }, HUE_LIMIT);
    for (const detail of hues) {
      record({ frame: frame.label, key: "(hue)", prop: "saturation", detail });
    }
    asserted++;
  }

  const seen = await page.evaluate(
    (label, PROPS, ATTRS, COLOUR) => {
      // Frozen exactly as extract.mjs freezes the prototype, and in the same evaluation as the
      // measurement so nothing can advance in between. Seeking rather than disabling, because
      // animation-name/duration/delay/timing-function are themselves asserted.
      // PAUSE FIRST, THEN SEEK. Seeking a running animation and pausing afterwards leaves a gap in
      // which the compositor can advance it: `opacity` under `breathe` read 0.45 on one run and
      // 0.450015 on the next, from the same machine seconds apart. Pausing first makes the seek final.
      for (const animation of document.getAnimations()) {
        animation.pause();
        animation.currentTime = 0;
      }
      const out = [];
      for (const el of document.querySelectorAll(`[data-fid^="${CSS.escape(label)}:"]`)) {
        const key = el.getAttribute("data-fid").slice(label.length + 1);
        const cs = getComputedStyle(el);
        const styles = {};
        for (const p of PROPS) styles[p] = cs.getPropertyValue(p);
        const svgAttrs = {};
        // The colour attributes are ALSO read as the engine computes them, because the app writes
        // `var(--token)` where the prototype writes a hex — see COLOUR_ATTRS in props.mjs.
        const svgComputed = {};
        for (const a of ATTRS) {
          const v = el.getAttribute(a);
          if (v != null) svgAttrs[a] = v;
          if (COLOUR.includes(a)) svgComputed[a] = cs.getPropertyValue(a);
        }
        const b = el.getBoundingClientRect();
        out.push({
          key,
          tag: el.tagName.toLowerCase(),
          styles,
          svgAttrs,
          svgComputed,
          box: { w: +b.width.toFixed(2), h: +b.height.toFixed(2) },
        });
      }
      return out;
    },
    frame.label,
    STYLE_PROPS,
    SVG_ATTRS,
    [...COLOUR_ATTRS],
  );

  if (!seen.length) {
    unmappedFrames.push(frame.label);
    continue;
  }
  mapped++;

  // A DECLARED SLOT THE APP NEVER STAMPS IS DEAD, and this is the check that would have caught S5's
  // never-synced dialog rendering an empty body while every gate stayed green. `check-fixtures.mjs`
  // already fails on a declared slot whose KEY exists in no frame — the complement, a slot whose key
  // is real but which nothing ever reaches, is invisible to it and to the comparison below, because
  // an unstamped node is simply not compared.
  //
  // The project's convention is already "declare only what you stamp": `hexRect`/`hexNumeral` were
  // removed for exactly this reason. So this is that convention made enforceable rather than a new
  // rule. A slot deliberately left for a state this frame does not draw belongs undeclared, the way
  // S4 leaves `5a Checking`'s progress line and `5a Plan`'s two G3 buttons undeclared.
  // STATIC KEYS ONLY, and that is a real limit rather than an implementation detail. A factory slot
  // (`row: (i) => …`) resolves to a different key per call, so deciding whether it was "reached"
  // would mean inverting the factory or guessing its arity — and a wrong guess reports a live slot
  // as dead. So factory slots are OUT of this report: `ruleRow`, `kvRow`, `passRow` and `door` are
  // not covered by it, and a screen that stopped rendering its rows would not show up here.
  const declared = new Set(
    Object.entries(FIXTURES[frame.label]?.fids ?? {})
      .filter(([, key]) => typeof key === "string")
      .map(([slot]) => slot),
  );
  const stampedKeys = new Set(seen.map((s) => s.key));
  const dead = [...declared].filter((slot) => !stampedKeys.has(FIXTURES[frame.label].fids[slot]));
  if (dead.length) deadSlots.push({ frame: frame.label, slots: dead });

  for (const node of seen) {
    const want = expected.get(node.key);
    if (!want) {
      // Loud on purpose. A data-fid naming a key that no longer exists is the failure mode the
      // path-based key scheme accepts, and silently skipping it would make the gate lie.
      failures.push({
        frame: frame.label,
        key: node.key,
        prop: "(mapping)",
        detail: "no such node key in the fixture — was the prototype re-extracted?",
      });
      continue;
    }
    for (const prop of STYLE_PROPS) {
      const reason = compare(prop, valueOf(want.styles, prop), node.styles[prop]);
      asserted++;
      if (reason) record({ frame: frame.label, key: node.key, prop, detail: reason });
    }
    // Size, as a border box in both documents — see the note on `width` in props.mjs. Skipped where
    // the node's text needs a glyph no bundled font provides (that width measures the machine), and
    // skipped whole for a content crop, whose every box is an artefact of the width it was drawn at
    // — see `OWES_BOX`.
    for (const side of OWES_BOX(entry.kind) && boxComparable(want) ? ["w", "h"] : []) {
      asserted++;
      if (Math.abs(want.box[side] - node.box[side]) > LENGTH_TOLERANCE_PX) {
        record({
          frame: frame.label,
          key: node.key,
          prop: `box.${side}`,
          detail: `${want.box[side]} vs ${node.box[side]}`,
        });
      }
    }
    for (const attr of SVG_ATTRS) {
      const a = want.svgAttrs?.[attr];
      const b = node.svgAttrs[attr];
      if (a === undefined && b === undefined) continue;
      asserted++;
      const reason = compareSvgAttr(attr, a, b, node.svgComputed?.[attr]);
      if (reason) record({ frame: frame.label, key: node.key, prop: `@${attr}`, detail: reason });
    }
  }

  // --- the fit gate. Only a full window owes it: a notification is sized by the desktop and a
  // content crop is a piece of one.
  if (OWES_FIT(entry.kind)) {
    const fit = await page.evaluate(() => {
      const root = document.documentElement;
      const footer = document.querySelector(".footer-nav, .footer-action-bar");
      const overlap = [];
      if (footer) {
        const f = footer.getBoundingClientRect();
        for (const el of document.querySelectorAll("#app-root *")) {
          if (footer.contains(el) || el.contains(footer)) continue;
          const b = el.getBoundingClientRect();
          if (!b.width || !b.height) continue;
          // A descendant painting over the footer is the bug 02-shell.md says was found twice.
          if (b.bottom > f.top + 0.5 && b.top < f.bottom) overlap.push(el.className || el.tagName);
        }
      }
      return { w: root.scrollWidth, h: root.scrollHeight, overlap: [...new Set(overlap)].slice(0, 5) };
    });
    if (fit.w > 1040 || fit.h > 764) {
      record({
        frame: frame.label,
        key: "(fit)",
        prop: "scroll",
        detail: `${fit.w}×${fit.h} exceeds 1040×764`,
      });
    }
    if (fit.overlap.length) {
      record({
        frame: frame.label,
        key: "(fit)",
        prop: "footer",
        detail: `painting over the footer: ${fit.overlap.join(", ")}`,
      });
    }
  }
}

await browser.close();
server.close();

console.log(
  `fidelity:assert — ${mapped}/${index.length} frames mapped, ${asserted} assertions, ${failures.length} failures`,
);
if (unmappedFrames.length) {
  // Not a failure: a frame with no `data-fid` anywhere is a screen nobody has built yet, which is
  // the true state of S1–S11. Listed every run so "the gate is green" never gets confused with
  // "the gate looked at anything".
  console.log(
    `  ${unmappedFrames.length} frames carry no data-fid yet (screen not built): ${unmappedFrames.slice(0, 6).join(", ")}${unmappedFrames.length > 6 ? ", …" : ""}`,
  );
}

if (deadSlots.length) {
  console.log("");
  console.log("Declared fid slots the app never stamped — dead mappings, or a block that renders nothing:");
  for (const d of deadSlots) console.log(`  ${d.frame}: ${d.slots.join(", ")}`);
}

// Printed every run, in full, and never folded into the pass count. A recorded deviation is a
// difference the build KNOWS about — the reader has to be able to see how many, and which.
if (deviations.length) {
  console.log(`  ${deviations.length} recorded Phase-1 deviation(s), each waiting on an open issue:`);
  for (const d of deviations) {
    const row = KNOWN_DEVIATIONS.find((k) => k.frame === d.frame && k.key === d.key);
    console.log(`    ${d.frame} · ${d.key} · ${d.prop} (${d.detail}) — ${row?.issue}`);
  }
}

// An entry that stopped failing is a lie about the build, so it fails it. This is the clause that
// keeps the list above from turning into somewhere failures go to be forgotten.
// REPORTED, NOT EXITED ON, so the failure list below still prints. The two arrive together on
// exactly the run that matters: the commit that closes #207 both settles the deviation AND is the
// most likely to break the nodes it touches, and exiting here would print "delete this row" and
// swallow every real failure underneath it.
const unmet = unmetDeviations();
if (unmet.length) {
  console.error("\nRecorded deviations that no longer fail — delete them, or re-pin their measurement:\n");
  for (const d of unmet) console.error(`  ${d.frame} · ${d.key} · ${d.prop} — ${d.issue}\n      ${d.why}`);
  console.error(`\nfidelity:assert: ${unmet.length} stale deviation(s) in known-deviations.mjs.`);
}

if (failures.length) {
  console.error("");
  // 40 by default so a broken build prints a page rather than a screenful of scrollback.
  // `FIDELITY_SHOW=200` is for developing a screen, where the first forty are all one cause.
  // Anything that is not a positive number falls back rather than silencing the report: `Number("")`
  // is 0 and `Number("x")` is NaN, and both would print nothing while the count still said dozens.
  const asked = Number(process.env.FIDELITY_SHOW);
  const SHOW = Number.isFinite(asked) && asked > 0 ? asked : 40;
  for (const f of failures.slice(0, SHOW)) {
    console.error(`  ${f.frame} · ${f.key || "(root)"} · ${f.prop}\n      ${f.detail}`);
  }
  if (failures.length > SHOW) console.error(`  … and ${failures.length - SHOW} more`);
  console.error(`\nfidelity:assert: ${failures.length} failure(s).`);
}

if (failures.length || unmet.length) process.exit(1);
