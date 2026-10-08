// The documentation screenshots — the app's own screens, rendered from its own fixtures.
//
// WHY THIS AND NOT A CAMERA ON A RUNNING APP. A screenshot lifted from a live desktop carries the
// photographer's real Proton Drive: their folder names, their file names, their account. Those
// images then live in a public README for as long as the repository does. Rendering `?frame=<label>`
// instead means the pixels come from the SAME frontend Tauri serves — `gui/src`, raw, no bundler —
// driven by the SAME datasets `tools/fidelity/assert.mjs` compares against the drawings. So the
// picture in the docs is the product, and the data in it is nobody's.
//
// IT IS THE WHOLE WINDOW, NOT A CROP. The viewport is 1040x764 because that is what
// `src-tauri/tauri.conf.json` declares and the window is `resizable: false` — so a frame at that
// size is the app at every size it has. A crop would be a composition; this is a screenshot.
//
// WHAT IS DELIBERATELY NOT HERE. The `10a In situ` and `11a` frames draw a desktop panel and
// freedesktop notifications — surfaces the DESKTOP paints, which the prototype mocks in HTML so the
// design can be reviewed. Publishing those as screenshots would present a drawing as the product.
// They stay in the fidelity harness, where a mock is the correct ground truth, and out of the docs.
//
// NOT A GATE, AND NOT IN CI. `glyphs:check` byte-compares because a tray icon is a BUILD INPUT that
// must not drift from the sheet it came from. A screenshot is an illustration: it drifts by a
// Chromium version bumping its antialiasing, which is not a defect and must not fail anyone's build.
// Run `npm run screenshots` by hand after a visible UI change, look at the result, and commit it.
//
//   npm run screenshots                                  the published set, into website/src/assets
//   npm run screenshots -- --out DIR                     the published set, into DIR instead
//   npm run screenshots -- --out DIR --frame "10a Two folders" [--frame ...]
//                                                        just those frames, into DIR
//
// `--out` exists so a frame can be rendered for REVIEW — a pull request's new frames, say — without
// publishing mock pixels to the website. `--frame` names any frame in the registry, and it REQUIRES
// `--out`: a frame nobody chose to publish must not land in `website/src/assets` by default. A panel
// (a compact frame) is clipped to its own box plus the shadow around it, since the page is a 1040px
// window and the panel 362px of it; every other frame is the whole window, as always.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import { armClock } from "./fidelity/clock-pin.mjs";
import { serve } from "./fidelity/serve.mjs";
import { FIXTURES } from "../src/js/fixtures/frames.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");

/**
 * ONE COPY, UNDER `website/src/assets`. Both readers reach it from there: Starlight resolves a
 * relative `src/assets` image through the build, so it survives the site's base path (the docs are
 * served under a project sub-path on Pages), and GitHub renders the README's relative path straight
 * out of the tree. A second copy at the repo root would be the shape of drift this codebase keeps
 * finding bugs in — two files that agree on the day someone checked.
 */
const PUBLISHED_OUT = resolve(REPO, "website", "src", "assets", "screenshots");

/** `--out DIR` and `--frame LABEL` (repeatable), `--out=DIR` and `--frame=LABEL` too. */
function parseArgs(argv) {
  const options = { out: null, frames: [] };
  for (let at = 0; at < argv.length; at += 1) {
    const [flag, inline] = argv[at].split(/=(.*)/s);
    if (flag !== "--out" && flag !== "--frame") {
      console.error(
        `screenshots: unknown argument ${JSON.stringify(argv[at])} (known: --out DIR, --frame LABEL)`,
      );
      process.exit(2);
    }
    const value = inline ?? argv[(at += 1)];
    if (!value) {
      console.error(`screenshots: ${flag} needs a value`);
      process.exit(2);
    }
    if (flag === "--out") options.out = resolve(value);
    else options.frames.push(value);
  }
  if (options.frames.length && !options.out) {
    console.error(
      "screenshots: --frame renders frames nobody published, so it needs --out DIR — it will not write them " +
        "into website/src/assets/screenshots",
    );
    process.exit(2);
  }
  return options;
}

const { out: requestedOut, frames: requestedFrames } = parseArgs(process.argv.slice(2));
const OUT = requestedOut ?? PUBLISHED_OUT;

/**
 * The published set: `[frame label, file stem, caption]`.
 *
 * The caption is here rather than in the pages so that a frame and the sentence describing it are
 * chosen together — and it is what the alt text is built from, which is the only description a
 * reader using a screen reader gets.
 */
const PUBLISHED_SHOTS = [
  ["2a Settled", "main-settled", "The main window with everything in sync"],
  ["2a Syncing", "main-syncing", "The main window during a sync pass, with the live transfer queue"],
  ["2a Needs you", "main-needs-you", "The main window when something needs a decision"],
  ["3a Conflict", "conflicts", "The conflicts screen, showing both sides of a file edited in two places"],
  ["4a Deletions", "deletions", "The deletions screen, holding deletions back until they are approved"],
  ["5a Plan", "plan", "The plan screen, previewing every action before anything is applied"],
  ["6a Activity passes", "activity", "The activity screen, listing recent sync passes"],
  ["8a Settings", "settings", "The settings screen"],
  ["9a Folders", "onboarding", "Onboarding: choosing the local folder and the Proton Drive folder"],
  ["12a Settled light", "main-settled-light", "The main window in the light theme"],
];

/** A file stem for a frame label that has no published one: `10a Two folders` -> `10a-two-folders`. */
const stemOf = (label) => label.replace(/[^a-z0-9]+/gi, "-").toLowerCase();

const SHOTS = requestedFrames.length
  ? requestedFrames.map((label) => [label, stemOf(label), label])
  : PUBLISHED_SHOTS;

const unknown = SHOTS.map(([label]) => label).filter((label) => !(label in FIXTURES));
if (unknown.length) {
  // A renamed frame must not become a silently missing screenshot: `?frame=` falls through to the
  // generic mock for a label it does not know, so the run would succeed and publish the wrong
  // screen. The same argument as `assert.mjs`'s `unknownSettled` check, one tool over.
  console.error(
    `screenshots: ${unknown.length} label(s) name no fixture — a renamed frame would otherwise ` +
      `publish the generic mock as if it were the screen: ${unknown.join(", ")}`,
  );
  process.exit(1);
}

const { server, port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
// `deviceScaleFactor: 2` — the opposite choice from the fidelity gate, for the opposite reason.
// There a length must be one CSS pixel so nothing rounds twice; here the file is looked at on a
// hidpi display, and 1x text renders soft.
await page.setViewport({ width: 1040, height: 764, deviceScaleFactor: 2 });

mkdirSync(OUT, { recursive: true });

for (const [label, stem] of SHOTS) {
  // PIN THE COLOUR SCHEME. A headless browser's `prefers-color-scheme` is a property of the machine
  // it runs on, so an unpinned run renders whatever the developer's desktop happens to be set to and
  // the light screenshot is light only by luck. The frame decides, exactly as in `assert.mjs`: the
  // `12a` set IS the light theme and everything else is drawn dark.
  await page.emulateMediaFeatures([
    { name: "prefers-color-scheme", value: label.startsWith("12a") ? "light" : "dark" },
    // `no-preference`, so the screenshots show the app a user with default settings sees. Four
    // stylesheets answer `reduce` with `animation: none`, which would publish the syncing screen
    // with its mark stopped — a real state, but not the ordinary one.
    { name: "prefers-reduced-motion", value: "no-preference" },
  ]);
  // THE CLOCK, PINNED as every fidelity gate pins it: a frame that prints an absolute time (`since
  // 15:17`) would otherwise publish whatever the machine read when it was run, and two runs would
  // commit two different images of one screen. Armed before the first navigation; it survives the
  // reload below.
  await armClock(page);
  const url = `http://127.0.0.1:${port}/index.html?frame=${encodeURIComponent(label)}`;
  await page.goto(url, { waitUntil: "networkidle0" });
  // The ⋯ menu persists an explicit choice in `localStorage`, and `:root[data-theme]` beats the
  // media query by design — so a value left behind by an earlier page in this browser context would
  // silently override the emulation above. Clear it and reload rather than clearing after the app
  // has already read it.
  const hadTheme = await page.evaluate(() => {
    const saved = localStorage.getItem("theme");
    localStorage.removeItem("theme");
    return saved;
  });
  if (hadTheme) await page.reload({ waitUntil: "networkidle0" });
  // The shell renders on its first status poll; wait for it rather than racing it.
  await page.waitForSelector("#app-root > *", { timeout: 5000 });
  // Let the entry transitions finish. `networkidle0` says the module graph has loaded, not that the
  // seam has faded in — screenshotting before it settles catches the app mid-animation.
  await new Promise((done) => {
    setTimeout(done, 600);
  });
  const file = join(OUT, `${stem}.png`);
  // A panel is 362px in a 1040px window: clip to it and the shadow around it, or the picture is
  // mostly empty page. Everything else is the whole window.
  const panel = FIXTURES[label]?.panel ? await page.$("#app-root > .compact-panel") : null;
  const box = await panel?.boundingBox();
  const margin = 28;
  const clip = box
    ? {
        x: Math.max(0, box.x - margin),
        y: Math.max(0, box.y - margin),
        width: Math.min(1040, box.width + 2 * margin),
        height: Math.min(764, box.height + 2 * margin),
      }
    : undefined;
  writeFileSync(file, await page.screenshot({ type: "png", ...(clip ? { clip } : {}) }));
  console.log(`  ${label}  ->  ${file.startsWith(`${REPO}/`) ? file.replace(`${REPO}/`, "") : file}`);
}

await browser.close();
server.close();
console.log(
  `screenshots — wrote ${SHOTS.length} images to ${OUT.startsWith(`${REPO}/`) ? OUT.replace(`${REPO}/`, "") : OUT}`,
);
