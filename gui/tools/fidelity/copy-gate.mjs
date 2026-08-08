// The copy gate (F8) — every fixed string in ui/copy.js appears verbatim somewhere in the drawn
// frames.
//
// The F8 issue frames this as asserting the app's DOM against the deck. That version needs screens,
// and the screens are S1–S11. This one needs neither: it checks the MODULE against the FRAMES, both
// of which exist now, and it catches the same bug class the issue names — a smart quote typed
// straight, a "pending" where the deck says "waiting", a sentence quietly reworded. It is the
// direction that can run today, and when the screens land assert.mjs checks the third side of the
// triangle (app renders what copy.js says).
//
// Only string constants are checked. A template like `Syncing ${n} changes` cannot be compared
// verbatim without inventing the number, and inventing it would assert the fixture's data rather
// than the design's words.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as COPY from "../../src/js/ui/copy.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const FRAMES = join(HERE, "frames");

// Strings the deck carries that are deliberately NOT drawn in any frame, with the reason. Anything
// not on this list must appear, or the gate fails.
const NOT_DRAWN = new Map([
  // 11-notifications.md quotes all four notification bodies; their frames are the 11a set, which the
  // extractor records — but the deck's own Tray section duplicates strings that only ever render in
  // a native menu, which is not a webview and has no DOM to extract.
  ["TRAY.open", "native tray menu — no DOM"],
  ["TRAY.syncNow", "native tray menu — no DOM"],
  ["TRAY.pause", "native tray menu — no DOM"],
  ["TRAY.resume", "native tray menu — no DOM"],
  ["TRAY.tryAgain", "native tray menu — no DOM"],
  ["TRAY.closeWindow", "native tray menu — no DOM"],
  ["TRAY.closeWindowSub", "native tray menu — no DOM"],
  ["TRAY.quit", "native tray menu — no DOM"],
  ["TRAY.quitSub", "native tray menu — no DOM"],
  // The deck's Activity section carries this under "Quiet:", and its only frame is `6a Quiet` —
  // one of the two demoted tide-chart Activity frames that IMPLEMENTATION-PLAN §1.2 puts out of
  // scope. So the deck outlived the drawing. Kept in copy.js because 14-behaviour-and-state.md's
  // empty-state table still specifies it ("Activity › files: `Nothing has moved in the last hour.`
  // + flat line"), and S5 will need it. DEVIATIONS.md §49.
  ["ACTIVITY.nothingRecent", "only drawn in `6a Quiet`, which is out of scope"],
]);

/**
 * Templates, WITH the arguments the frame draws them at.
 *
 * The header's rule — "a template cannot be compared verbatim without inventing the number" — is
 * right about inventing and wrong about the number, and S1 is where the difference started to cost
 * something. `2a Needs you`'s band renders from live counts, so `One file changed on both sides` had
 * to become `conflictTitle(n)`; as a constant it was gated, and as a template it silently left the
 * gate. Three of the deck's sentences would have gone quiet in the commit that first drew them.
 *
 * The argument is not invented here — it is READ OFF the frame, exactly as every fixture value is:
 * `2a Needs you` draws one conflict and two deletions, `2a Syncing` draws three changes. So the
 * rendered string is ground truth on both sides and the comparison is the same one every constant
 * gets. What stays out is a template no frame draws (`MAIN.settledSub` needs a relative time, and
 * `since()` against a real clock is exactly the input a gate may not depend on).
 *
 * A path here that is not a function, or that renders a string no frame contains, fails the build.
 */
const DRAWN = [
  ["CHROME.chips.waiting", [3], "2a Needs you"],
  ["CHROME.chips.step", [1], "9a Folders"],
  ["MAIN.syncing", [3], "2a Syncing"],
  ["MAIN.otherWaiting", [3], "2a Needs you"],
  // Drawn on a NOTIFICATION rather than on the main screen: `11a Outage` is the only place the
  // design writes this sentence, and S1's sign-in hero quotes it because there is no second one.
  ["MAIN.authExpiredSub", [61], "11a Outage"],
  ["MAIN.band.conflictTitle", [1], "2a Needs you"],
  ["MAIN.band.conflictSub", ["notes/todo.txt"], "2a Needs you"],
  ["MAIN.band.deletionTitle", [2], "2a Needs you"],
  ["MAIN.band.deletionSub", [1, 1], "2a Needs you"],
  ["MAIN.compact.needYou", [3], "2a Compact needs you"],
];

/** Every own-text string in every frame, and which frames said it. */
const saidBy = new Map();
for (const file of readdirSync(FRAMES)) {
  if (file === "index.json" || !file.endsWith(".json")) continue;
  const frame = JSON.parse(readFileSync(join(FRAMES, file), "utf8"));
  for (const node of frame.nodes) {
    // Own text, the joined subtree text (a sentence split by an inline <strong>), and every
    // user-visible attribute — a placeholder is copy just as much as a paragraph is.
    for (const said of [node.text, node.fullText, ...Object.values(node.attrs ?? {})]) {
      if (!said) continue;
      if (!saidBy.has(said)) saidBy.set(said, []);
      saidBy.get(said).push(frame.label);
    }
  }
}
// Also index the concatenated text of each frame, so a sentence split across inline children (a
// <strong> mid-paragraph — the deck has 25 of them) is still found.
const frameText = new Map();
for (const file of readdirSync(FRAMES)) {
  if (file === "index.json" || !file.endsWith(".json")) continue;
  const frame = JSON.parse(readFileSync(join(FRAMES, file), "utf8"));
  // Each piece is whitespace-normalised on its own and then joined with a separator that cannot
  // occur in copy, so a match can never span two unrelated nodes. (A control character here trips
  // eslint's no-control-regex; the pilcrow is printable, absent from the deck, and just as unique.)
  frameText.set(
    frame.label,
    frame.nodes
      .flatMap((n) => [n.fullText ?? n.text ?? "", ...Object.values(n.attrs ?? {})])
      .map((t) => t.replace(/\s+/g, " ").trim())
      .filter(Boolean)
      .join(" ¶ "),
  );
}

/** Walk the exported constants, collecting `PATH -> string` for every fixed string. */
const strings = [];
const walk = (value, path) => {
  if (typeof value === "string") strings.push([path, value]);
  else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value)) walk(v, `${path}.${k}`);
  }
  // functions are templates — see the header
};
for (const [group, value] of Object.entries(COPY)) walk(value, group);

/** Resolve a dotted path into the deck. Returns undefined rather than throwing on a bad segment. */
const at = (path) => path.split(".").reduce((node, key) => (node == null ? node : node[key]), COPY);

// Render each drawn template and hand it to the same comparison the constants get. A path that no
// longer names a function is a build failure and not a skip: the whole point is that this table
// cannot rot into a list of things nobody checks.
const templateErrors = [];
for (const [path, args] of DRAWN) {
  const fn = at(path);
  if (typeof fn !== "function") {
    templateErrors.push(
      `${path} is ${fn === undefined ? "not in the deck" : `a ${typeof fn}`}, not a template`,
    );
    continue;
  }
  strings.push([`${path}(${args.map((a) => JSON.stringify(a)).join(", ")})`, fn(...args)]);
}

const missing = [];
const found = [];
for (const [path, text] of strings) {
  if (NOT_DRAWN.has(path)) continue;
  if (saidBy.has(text)) {
    found.push(path);
    continue;
  }
  const inFrame = [...frameText.entries()].find(([, all]) => all.includes(text));
  if (inFrame) {
    found.push(path);
    continue;
  }
  missing.push([path, text]);
}

console.log(
  `fidelity:copy — ${found.length}/${strings.length - NOT_DRAWN.size} drawn strings matched ` +
    `(${DRAWN.length} of them templates rendered at the arguments the frame draws), ` +
    `${NOT_DRAWN.size} exempt (native tray), ${missing.length} missing`,
);

if (templateErrors.length) {
  console.error("\nEntries in the drawn-template table that are no longer templates:\n");
  for (const problem of templateErrors) console.error(`  ${problem}`);
}

if (missing.length) {
  console.error("\nStrings in ui/copy.js that no in-scope frame contains:\n");
  for (const [path, text] of missing) {
    console.error(`  ${path}`);
    console.error(`    "${text}"`);
    // The most likely cause by far, so say it rather than making everyone rediscover it.
    const straightened = text.replace(/[’‘]/g, "'").replace(/[“”]/g, '"').replace(/—/g, "-");
    if (
      straightened !== text &&
      (saidBy.has(straightened) || [...frameText.values()].some((t) => t.includes(straightened)))
    ) {
      console.error("    ^ the frame has this with STRAIGHT quotes/dashes — the deck's are typographic");
    }
  }
  console.error(`\nfidelity:copy: ${missing.length} string(s) do not match the frames.`);
}

if (missing.length || templateErrors.length) process.exit(1);
