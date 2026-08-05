// Guards the three F1 invariants that nothing else can catch, because CSS has no build step here
// and a broken one is invisible until someone opens the app in the affected theme.
//
//   1. THE TWO LIGHT BLOCKS AGREE. tokens.css declares the light palette twice — once under
//      `@media (prefers-color-scheme: light)` for the system default, once under
//      `:root[data-theme="light"]` so the ⋯ toggle beats the media query in both directions. Plain
//      CSS cannot share one declaration list between a media block and a selector, so the copies
//      are literal. A token edited in one and not the other means the toggle and the system
//      preference render different apps, which is not something a screenshot review would notice.
//
//   2. NEITHER THEME IS MISSING A TOKEN. Any token whose dark value contains a colour literal must
//      have a light value. Seven of the eleven screens have no drawn light frame (12-light-theme.md
//      "Frames drawn"), so a token that silently keeps its dark value in light is exactly the class
//      of bug S10 is least able to find.
//
//   3. RAW HEX LIVES IN A TOKEN FILE. The F1 definition of done. A colour written into a screen is
//      a colour that will not follow the theme. Two files may carry one: tokens.css, and the
//      legacy-tokens.css shim that F4 deletes.
//
//   Plus: every @font-face src in base.css resolves to a committed file. A typo there is not an
//   error anywhere — the face just never loads and the app renders in system-ui, which moves every
//   measurement the fidelity gate asserts.

import { readdirSync, readFileSync, existsSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const GUI = dirname(dirname(fileURLToPath(import.meta.url)));
const SRC = join(GUI, "src");
const TOKENS = join(SRC, "styles", "tokens.css");
const BASE = join(SRC, "styles", "base.css");

// The only files allowed to carry a raw colour. legacy-tokens.css is the v1 palette shim, deleted
// in F4 along with app.css/components.css — see its header.
const COLOUR_FILES = new Set([TOKENS, join(SRC, "styles", "legacy-tokens.css")]);

const errors = [];
const css = readFileSync(TOKENS, "utf8");

// ---- 1 + 2: the token blocks ------------------------------------------------------------------
/** Pull `--name: value;` pairs out of the block that starts at the given index. */
function declarationsAt(source, openBraceIndex) {
  let depth = 0;
  let i = openBraceIndex;
  for (; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) break;
  }
  const body = source.slice(openBraceIndex + 1, i);
  const out = new Map();
  for (const m of body.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g))
    out.set(m[1], m[2].trim().replace(/\s+/g, " "));
  return out;
}

function blockAfter(marker, label) {
  const at = css.indexOf(marker);
  if (at < 0) {
    errors.push(`tokens.css: could not find the ${label} block (looked for \`${marker}\`)`);
    return new Map();
  }
  return declarationsAt(css, css.indexOf("{", at));
}

const dark = blockAfter("\n:root {", "dark :root");
const lightMedia = blockAfter(':root:not([data-theme="dark"]) {', "system-preference light");
const lightExplicit = blockAfter(':root[data-theme="light"] {', "explicit light");

for (const [name, value] of lightMedia) {
  const other = lightExplicit.get(name);
  if (other === undefined)
    errors.push(`tokens.css: ${name} is in the media-query light block but not in :root[data-theme="light"]`);
  else if (other !== value)
    errors.push(`tokens.css: ${name} differs between the two light blocks — "${value}" vs "${other}"`);
}
for (const name of lightExplicit.keys()) {
  if (!lightMedia.has(name))
    errors.push(`tokens.css: ${name} is in :root[data-theme="light"] but not in the media-query light block`);
}

const HAS_COLOUR = /#[0-9a-f]{3,8}\b|\brgba?\(/i;
for (const [name, value] of dark) {
  if (name === "color-scheme") continue;
  if (HAS_COLOUR.test(value) && !lightExplicit.has(name)) {
    errors.push(`tokens.css: ${name} carries a colour in dark ("${value}") but has no light value`);
  }
}
for (const name of lightExplicit.keys()) {
  if (name !== "color-scheme" && !dark.has(name))
    errors.push(`tokens.css: ${name} is declared for light only — every token needs a dark value`);
}

// ---- 3: raw colour outside the token files ----------------------------------------------------
// `url(#id)` is an SVG reference, not a colour, and gradient ids are frequently hex-shaped.
const HEX = /(?<!url\()#[0-9a-fA-F]{3,8}\b/g;
const RGB = /\brgba?\(\s*\d/g;

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) {
      if (entry === "fonts" || entry === "assets") continue; // binaries and shipped artwork
      walk(p, out);
    } else if (/\.(css|js|mjs|html)$/.test(entry)) out.push(p);
  }
  return out;
}

// Comments are blanked rather than removed so reported line numbers still point at the source line.
// Without this, a GitHub issue reference in prose (`review findings on #108/#109` — overview.js)
// reads as two colours, and a gate whose first output is a false positive does not survive a week.
function blankComments(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/<!--[\s\S]*?-->/g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/(^|[^:])\/\/[^\n]*/g, (m, lead) => lead + " ".repeat(m.length - lead.length));
}

for (const file of walk(SRC)) {
  if (COLOUR_FILES.has(file)) continue;
  const text = blankComments(readFileSync(file, "utf8"));
  for (const [pattern, kind] of [
    [HEX, "hex colour"],
    [RGB, "rgb()/rgba() colour"],
  ]) {
    pattern.lastIndex = 0;
    for (const m of text.matchAll(pattern)) {
      const line = text.slice(0, m.index).split("\n").length;
      errors.push(
        `${relative(GUI, file)}:${line}: raw ${kind} \`${m[0]}\` — put it in tokens.css and reference the token`,
      );
    }
  }
}

// ---- bonus: every bundled face resolves -------------------------------------------------------
const baseCss = readFileSync(BASE, "utf8");
for (const m of baseCss.matchAll(/url\("([^"]+\.woff2)"\)/g)) {
  const p = resolve(dirname(BASE), m[1]);
  if (!existsSync(p))
    errors.push(`base.css: @font-face src does not exist: ${m[1]} — run \`npm run fonts:sync\``);
}

// ---- report -----------------------------------------------------------------------------------
if (errors.length) {
  for (const e of errors) console.error(e);
  console.error(`\ncheck-tokens: ${errors.length} problem(s).`);
  process.exit(1);
}
const allowed = [...COLOUR_FILES].map((f) => relative(SRC, f)).join(", ");
console.log(
  `check-tokens: ok — ${dark.size} tokens, ${lightExplicit.size} themed, both light blocks agree, no raw colour outside ${allowed}.`,
);
