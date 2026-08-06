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
import { STYLE_PROPS, SVG_ATTRS, compare, valueOf, LENGTH_TOLERANCE_PX, boxIsComparable } from "./props.mjs";
import { OWES_FIT } from "./frame-classes.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(HERE, "..", "..", "src");
const FRAMES = join(HERE, "frames");

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
const failures = [];
let asserted = 0;
let mapped = 0;
const unmappedFrames = [];

for (const entry of index) {
  const frame = JSON.parse(readFileSync(join(FRAMES, entry.file), "utf8"));
  const expected = new Map(frame.nodes.map((n) => [n.key, n]));

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

  const seen = await page.evaluate(
    (label, PROPS, ATTRS) => {
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
        for (const a of ATTRS) {
          const v = el.getAttribute(a);
          if (v != null) svgAttrs[a] = v;
        }
        const b = el.getBoundingClientRect();
        out.push({
          key,
          tag: el.tagName.toLowerCase(),
          styles,
          svgAttrs,
          box: { w: +b.width.toFixed(2), h: +b.height.toFixed(2) },
        });
      }
      return out;
    },
    frame.label,
    STYLE_PROPS,
    SVG_ATTRS,
  );

  if (!seen.length) {
    unmappedFrames.push(frame.label);
    continue;
  }
  mapped++;

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
      if (reason) failures.push({ frame: frame.label, key: node.key, prop, detail: reason });
    }
    // Size, as a border box in both documents — see the note on `width` in props.mjs. Skipped where
    // the node's text needs a glyph no bundled font provides; that width measures the machine.
    for (const side of boxIsComparable(want) ? ["w", "h"] : []) {
      asserted++;
      if (Math.abs(want.box[side] - node.box[side]) > LENGTH_TOLERANCE_PX) {
        failures.push({
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
      if (a !== b)
        failures.push({ frame: frame.label, key: node.key, prop: `@${attr}`, detail: `${a} vs ${b}` });
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
      failures.push({
        frame: frame.label,
        key: "(fit)",
        prop: "scroll",
        detail: `${fit.w}×${fit.h} exceeds 1040×764`,
      });
    }
    if (fit.overlap.length) {
      failures.push({
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

if (failures.length) {
  console.error("");
  for (const f of failures.slice(0, 40)) {
    console.error(`  ${f.frame} · ${f.key || "(root)"} · ${f.prop}\n      ${f.detail}`);
  }
  if (failures.length > 40) console.error(`  … and ${failures.length - 40} more`);
  console.error(`\nfidelity:assert: ${failures.length} failure(s).`);
  process.exit(1);
}
