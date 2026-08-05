// Vendors the design-v2 typefaces from node_modules into gui/src/fonts/.
//
// Why vendor rather than resolve at runtime: Tauri serves gui/src RAW (tauri.conf.json
// "frontendDist": "../src", no bundler, no beforeBuildCommand). Nothing rewrites a
// `node_modules/...` URL between here and the webview, and node_modules is not shipped, so a font
// that is not committed under gui/src/fonts/ simply does not exist at runtime. The @fontsource
// devDependencies are the provenance and the upgrade path; the committed .woff2 files are what the
// app loads. The CSP forbids external font hosts, and this is a desktop app that must render
// offline, so there is no CDN fallback either.
//
// Upgrading a typeface is a DELIBERATE act, not a dependency bump: new metrics move every
// measurement in docs/design-v2 and re-baseline the fidelity frames (F8). That is why this script
// is not wired into `npm run check` — a @fontsource bump that nobody syncs leaves the app on the
// font the frames were measured against, which is the safe outcome. Run `npm run fonts:sync` when
// you actually intend to move, and expect to regenerate the fidelity fixtures in the same PR.
//
//   npm run fonts:sync           copy (and report what changed)
//   npm run fonts:sync -- --check   exit 1 if the committed files differ from node_modules

import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const GUI = dirname(dirname(fileURLToPath(import.meta.url)));
const DEST = join(GUI, "src", "fonts");

// Weights are what the prototype actually uses, measured node-by-node over
// docs/design-v2/Drive Sync.dc.html — NOT what 01-foundations.md §2 claims:
//
//   sans  400 (504 text nodes) · 600 (244) · 500 (13)      700: zero uses
//   mono  400 (352)            · 600 (19)                  500: zero uses
//
// 400 sans is the single most common weight in the design and is absent from every prose list of
// "weights in use", so bundling only 500/600/700 would leave the majority of the UI rendering in
// system-ui with every downstream pixel assertion measuring the wrong typeface. 700 is bundled
// anyway because §2 names it: unused today, but a real face is cheaper than the synthetic bold a
// future `font-weight:700` would otherwise get, and its absence would be invisible to the style
// gate (`font-weight` computes to 700 whether or not a 700 face exists). Mono 500 is not bundled —
// nothing asks for it. See docs/design-v2/DEVIATIONS.md.
//
// Both latin and latin-ext subsets: the frames are all ASCII, but this app renders arbitrary
// FILENAMES. `unicode-range` means the ext face costs nothing until a path actually contains a
// character outside the latin subset, at which point the alternative is a system-font glyph in the
// middle of a filename.
const FONTS = [
  {
    pkg: "@fontsource/instrument-sans",
    stem: "instrument-sans",
    subsets: ["latin", "latin-ext"],
    weights: [400, 500, 600, 700],
  },
  {
    pkg: "@fontsource/ibm-plex-mono",
    stem: "ibm-plex-mono",
    subsets: ["latin", "latin-ext"],
    weights: [400, 600],
  },
];

const LICENSES = [
  { pkg: "@fontsource/instrument-sans", as: "OFL-instrument-sans.txt" },
  { pkg: "@fontsource/ibm-plex-mono", as: "OFL-ibm-plex-mono.txt" },
];

const check = process.argv.includes("--check");
const digest = (p) => createHash("sha256").update(readFileSync(p)).digest("hex");

/** @type {Array<{from: string, to: string}>} */
const jobs = [];
for (const font of FONTS) {
  for (const subset of font.subsets) {
    for (const weight of font.weights) {
      const name = `${font.stem}-${subset}-${weight}-normal.woff2`;
      jobs.push({ from: join(GUI, "node_modules", font.pkg, "files", name), to: join(DEST, name) });
    }
  }
}
for (const license of LICENSES) {
  jobs.push({ from: join(GUI, "node_modules", license.pkg, "LICENSE"), to: join(DEST, license.as) });
}

const missing = jobs.filter((j) => !existsSync(j.from));
if (missing.length) {
  console.error(`sync-fonts: ${missing.length} source file(s) not found — run \`npm install\` first:`);
  for (const j of missing) console.error(`  ${j.from}`);
  process.exit(1);
}

mkdirSync(DEST, { recursive: true });

const stale = jobs.filter((j) => !existsSync(j.to) || digest(j.from) !== digest(j.to));
if (check) {
  if (stale.length) {
    console.error(`sync-fonts: ${stale.length} vendored file(s) differ from node_modules:`);
    for (const j of stale) console.error(`  ${j.to}`);
    console.error("Run `npm run fonts:sync` — and re-baseline the fidelity frames in the same PR.");
    process.exit(1);
  }
  console.log(`sync-fonts: ${jobs.length} file(s) up to date.`);
} else {
  for (const j of stale) copyFileSync(j.from, j.to);
  console.log(`sync-fonts: ${stale.length} file(s) copied, ${jobs.length - stale.length} already current.`);
}
