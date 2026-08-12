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

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import { serve } from "./serve.mjs";
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
import { resolveFixture } from "../../src/js/fixtures/frames.js";
import { OWES_BOX, OWES_FIT } from "./frame-classes.mjs";
import { isKnown, unmetDeviations, classifyUnstamped, KNOWN_DEVIATIONS } from "./known-deviations.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
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
/**
 * Colour comparisons dropped on the light frames because the prototype's answer is the page's, not
 * the frame's — `{ [label]: count }`. DEVIATIONS.md §58b; `fromPage` in extract.mjs argues which
 * properties qualify and why the list is five long rather than "everything inherited".
 *
 * COUNTED AND PRINTED rather than silently skipped. This is a gate that stops comparing 628 values
 * on the theme with the least drawn ground truth, and a reader who cannot see that number cannot
 * judge what "0 failures" on a `12a` frame is worth.
 */
const pageColourSkips = {};
const unmappedFrames = [];
/** Frames that DO carry a `fids` map and stamped none of it — built, and rendering nothing. */
const blankFrames = [];
/**
 * `{ frame, slot, key }` for every slot a frame DRAWS and the running app never stamped.
 *
 * EVERY frame, not every mapped one: collected above the unmapped-frame bail-out, so a screen that
 * stamps nothing at all lands here rather than in the non-failing "screen not built" list.
 */
const unstamped = [];

/**
 * How far each index axis is probed below. Measured, not chosen: raising it to 30 reaches not one
 * drawn key that 10 does not, across every factory slot of every mapped frame.
 */
const PROBE_DEPTH = 10;

/**
 * Every node key a factory fid slot can produce, over a numeric index grid.
 *
 * `check-fixtures.mjs` probes `value(i, 0, 0)` for i = 0…9, which answers the question IT asks —
 * "does this slot resolve in ANY frame that declares it". This one is per frame and stricter, so it
 * probes the whole 10³ grid: `sideRowNote(s, i)` is keyed by side AND row, and a single-axis probe
 * reaches only row 0 of each side. Measured: the grid finds 39 drawn-but-unstamped slots where the
 * single axis finds 33, and all six extra are further rows of clusters the axis already found — so
 * it completes findings rather than widening them. Three axes because S3's fact strip is keyed by
 * column, then card, then fact; arguments a shorter factory ignores cost nothing.
 *
 * TWO THINGS IT STILL CANNOT REACH, stated rather than solved, the way #247 stated the static-only
 * limit: an index past `PROBE_DEPTH`, and a factory wanting a non-numeric argument (none exist —
 * every fid factory is keyed by position). Both fail SAFE: the key is never produced, so the slot
 * is never reported, and this gate only ever accuses.
 *
 * @returns {string[]} deduplicated, and an array in BOTH branches — the guard used to answer `[]`
 *   where the body answered a `Set`, which iterates the same and would have surprised the first
 *   caller to reach for `.length`.
 */
function probeSlot(value) {
  if (typeof value !== "function") return [];
  const keys = new Set();
  for (let a = 0; a < PROBE_DEPTH; a++) {
    for (let b = 0; b < PROBE_DEPTH; b++) {
      for (let c = 0; c < PROBE_DEPTH; c++) {
        let key;
        try {
          key = value(a, b, c);
        } catch {
          continue; // a factory wanting arguments this grid cannot guess — the other cells cover it
        }
        // A factory says "no node here" by returning null — `rulePattern` does it off-index. That
        // is the factory's own way of leaving a slot undeclared, and it is not a finding.
        if (typeof key === "string") keys.add(key);
      }
    }
  }
  return [...keys];
}

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
  // FACTORY SLOTS TOO, which is the half #247 shipped without and #248 closed. A factory slot
  // (`row: (i) => …`) resolves to a different key per call, so it cannot be read off the map the way
  // a static one can — it has to be PROBED, and `probeSlot` argues the grid it probes over and the
  // two things that grid still cannot reach. Leaving them out was not free: 218 of the 838 slots
  // declared when this was measured (S8) are
  // factories, and a factory slot is by definition a repeated block — a row, a card, a fact, a path
  // — which is exactly the kind of thing a screen renders none of. It was also why #247 did not
  // catch the case it was built for: the compact panel declares `transferTrack`/`transferFill` as
  // factories, so the S8 regression (wiring the tray panel to `SyncActivity`, whose #98 gap removes
  // the progress fraction) passed it. It does not pass this.
  //
  // AND THE FRAME HAS TO DRAW THE NODE FOR IT TO BE A FINDING, which is what turns this from a
  // printout into a gate. Of the twelve slots the first version of this report listed, eight
  // resolved to a key that exists in NO node of the frame declaring it — `compactFids` is a factory
  // over four tree shapes and hands every frame the whole vocabulary, so `10a Settled` declaring
  // `meta` says the shape has a meta line, not that this panel draws one. `check-fixtures.mjs`
  // tolerates precisely that ("alive somewhere, not alive here") and argues why. Reporting them
  // here contradicted that gate and gave the list a permanent floor of benign noise, which is where
  // a real thirteenth entry would have gone to hide.
  //
  // What survives the filter is the honest finding: the frame draws a node, the app draws nothing
  // there. `known-deviations.mjs` sorts those into recorded and unexplained, and an unexplained one
  // fails the build.
  //
  // RUN BEFORE THE UNMAPPED-FRAME BAIL-OUT, and that ordering is the whole check. Below the
  // `continue` the gate had its own failure inverted: blank HALF a screen and the surviving stamps
  // put the frame in the mapped set, so the missing half is a finding — blank ALL of it and the
  // frame drops to `unmappedFrames`, a printout that says "screen not built", and the run stays
  // green. Measured rather than reasoned, and RE-measured on this build because #248 stamped five
  // more of that frame's nodes: making `7a Never synced` stamp nothing takes the run to `35/51 frames
  // mapped, 67457 assertions, 0 failures`, exit 0, with 1101 assertions gone and the frame's name
  // folded into a truncated `…` list. (It was 806 when #247 wrote this down.) That frame is the one
  // this mechanism exists for.
  //
  // Since S10 there are no unmapped frames left to cost anything: all 51 carry a `fids` map. The
  // clause it replaces was "all 15 unmapped frames are screens with no `fids` map at all, so they
  // declare nothing and produce no observations", and it stops being the reassurance it was — every
  // frame now declares slots, so every frame can now report one unstamped.
  const stampedKeys = new Set(seen.map((s) => s.key));
  // RESOLVED, not the raw registry entry. A light twin's mapping is its dark twin's (S10), so
  // reading `FIXTURES[label].fids` here found `undefined` on all seven `12a` frames and iterated
  // nothing — which is this gate's own failure mode, one level up: the frames were mapped, compared
  // and green, and the blocks they render nothing for were invisible. Caught by asking why
  // `12a Syncing light` reported none of the two #98 slots its twin reports.
  const declaredFids = resolveFixture(frame.label)?.fids ?? {};
  for (const [slot, value] of Object.entries(declaredFids)) {
    for (const key of typeof value === "string" ? [value] : probeSlot(value)) {
      if (!stampedKeys.has(key) && expected.has(key)) unstamped.push({ frame: frame.label, slot, key });
    }
  }

  if (!seen.length) {
    // TWO DIFFERENT THINGS LAND HERE, and calling both "screen not built" is the same comforting
    // mislabel this whole gate exists to remove. A frame with no `fids` map is a screen nobody has
    // written yet — the true state of most of S8–S11. A frame that HAS a mapping and stamped none
    // of it is a built screen rendering nothing, which is a failure however loudly its slots also
    // report. Separated so the informational line cannot describe the second as the first.
    //
    // A failure in its own right rather than left to the slot check, and it stays that way now that
    // the slot check covers factories too: a mapping whose every key sits past `PROBE_DEPTH`, or
    // behind a non-numeric argument, would stamp nothing and report nothing. Zero frames are in that
    // state — all 51 with a `fids` map stamp something — so this costs nothing and states the case
    // the probe cannot.
    if (Object.keys(declaredFids).length) blankFrames.push(frame.label);
    else unmappedFrames.push(frame.label);
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
    // A COLOUR THE PROTOTYPE NEVER SET IS NOT GROUND TRUTH FOR LIGHT. The frames are drawn on one
    // dark page whose wrapper carries `color:#F2F4F7`, so a `12a` node that declares no colour of its
    // own was extracted as the dark text tier — against which a correctly-light app fails on every
    // one, 142 times across the three compacts alone and not once for a real reason (DEVIATIONS
    // §58b). `fromPage` names those properties per node; the frame's own declared colours, which are
    // the light values the whole screen doc is about, are untouched and still exact.
    //
    // DARK FRAMES KEEP THEM. There the inherited value is accidentally correct — the app inherits
    // `#F2F4F7` too — so it is a real comparison and dropping it would trade a fixed light theme for
    // a weaker dark one. Same fixture, different reading, which is why `fromPage` is recorded for all
    // 51 and interpreted here.
    const fromPage = scheme === "light" && want.fromPage ? new Set(want.fromPage) : null;
    for (const prop of STYLE_PROPS) {
      if (fromPage?.has(prop)) {
        pageColourSkips[frame.label] = (pageColourSkips[frame.label] ?? 0) + 1;
        continue;
      }
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
// What the gate stopped comparing, in full, so "0 failures on a light frame" can be read for what it
// is. A light frame has less drawn ground truth than any other surface in the build, and this is the
// number that says how much less.
const skipTotal = Object.values(pageColourSkips).reduce((n, c) => n + c, 0);
if (skipTotal) {
  console.log(
    `  ${skipTotal} colour comparison(s) skipped on ${Object.keys(pageColourSkips).length} light frame(s) — ` +
      `the prototype draws them on a dark page and never set them (DEVIATIONS §58b):`,
  );
  for (const [label, count] of Object.entries(pageColourSkips).sort()) console.log(`    ${label} · ${count}`);
}
if (unmappedFrames.length) {
  // Not a failure: a frame with no `fids` map is a screen nobody has built yet, which is the true
  // state of most of S8–S11. Listed every run so "the gate is green" never gets confused with "the
  // gate looked at anything". A frame that HAS a mapping and stamped none of it is NOT in this
  // list — it is a failure, reported below.
  console.log(
    `  ${unmappedFrames.length} frames carry no data-fid yet (screen not built): ${unmappedFrames.slice(0, 6).join(", ")}${unmappedFrames.length > 6 ? ", …" : ""}`,
  );
}

// A block the app cannot draw yet, named and waiting on an issue. Printed like the deviations
// below and for the same reason: the reader has to be able to see what the gate is NOT comparing.
const { recorded: recordedUnstamped, unexplained, stale } = classifyUnstamped(unstamped);
if (recordedUnstamped.length) {
  console.log(
    `  ${recordedUnstamped.length} recorded unstamped slot(s) — the frame draws it, Phase 1 cannot:`,
  );
  for (const u of recordedUnstamped) console.log(`    ${u.frame} · ${u.slot} (${u.key}) — ${u.issue}`);
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

// The same clause for the same reason, on the other list. A row here stops being observed when the
// capability lands, when the prototype moves the node, or when the frame stops being mapped — and
// all three are a human's call rather than something to infer.
if (stale.length) {
  console.error(
    "\nRecorded unstamped slots that are no longer unstamped — delete them, or re-pin the key:\n",
  );
  for (const u of stale) {
    // `alsoUnstamped` narrows the causes without deciding between them: the same slot is unstamped
    // elsewhere on this frame, which is what a moved node looks like — and also what a run of
    // siblings gaining a member looks like. Naming the candidates is the honest version; picking one
    // of several as "where it went" is not. With none, an observation simply stopped arriving and
    // nothing here can tell the remaining causes apart, so all of them are named.
    const what = u.alsoUnstamped
      ? `no longer observed at ${u.key}, and the same slot is unstamped at ${u.alsoUnstamped.join(", ")} — re-pin the key if the node moved there`
      : `no longer observed — stamped now, or the fixture stopped declaring the slot, or the frame left index.json`;
    console.error(`  ${u.frame} · ${u.slot} — ${u.issue}\n      ${what}\n      ${u.why}`);
  }
  console.error(`\nfidelity:assert: ${stale.length} stale unstamped row(s) in known-deviations.mjs.`);
}

// A mapped screen that rendered NOTHING. Its slots normally report it too, but this is the case
// that must never be filed under "screen not built", so it is stated on its own terms.
if (blankFrames.length) {
  console.error("\nFrames with a fid mapping that stamped none of it — built, and rendering nothing:\n");
  for (const label of blankFrames) console.error(`  ${label}`);
  console.error(`\nfidelity:assert: ${blankFrames.length} frame(s) rendered nothing.`);
}

// THE TEETH. A frame draws this node, the app renders nothing there, and nothing on file says why.
// Either build the block or record it with the issue that blocks it — the one thing that must not
// happen is it going quiet, because a screen can render almost nothing and pass every other gate.
if (unexplained.length) {
  console.error("\nBlocks that render nothing, with no reason on file:\n");
  for (const u of unexplained) {
    console.error(
      `  ${u.frame} · ${u.slot} (${u.key})\n      the frame draws this node and the app never stamped it`,
    );
  }
  console.error(
    `\nfidelity:assert: ${unexplained.length} unexplained unstamped slot(s). Build the block, or add a KNOWN_UNSTAMPED row.`,
  );
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

if (failures.length || unmet.length || stale.length || unexplained.length || blankFrames.length)
  process.exit(1);
