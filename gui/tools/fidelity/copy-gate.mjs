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
  // S2's meta line names what sort of file this is, and only ONE of the three answers is drawn.
  // `3a Conflict` is a text conflict, so `a plain text file` is in a frame; the other two describe
  // states no `3a` frame draws — a type conflict (the frames put `photos/trip` in the queue list,
  // never open) and a file the pair could not read as text (binary, too large, or vanished, which
  // `ConflictSide` cannot tell apart). The deck has no wording for either, so these are the two
  // sentences S2 wrote rather than measured. DEVIATIONS §74.
  //
  // `kindFolder` NEARLY MOVED OUT OF THIS TABLE, and the near-miss is worth the four lines. The
  // queue list on `3a Conflict diff` does draw `photos/trip · a folder here, a file there`, which
  // looks like the drawn instance this row denies — it is not. That string is `typeConflict`, the
  // list wording. `kindFolder` is the META LINE, the slot that says `a plain text file` under the
  // filename on the card, and no frame opens a type conflict card. Rewriting this one to match the
  // queue's text would have made the gate green by pointing two deck entries at one drawn node,
  // which is the exact failure a verbatim copy gate exists to prevent.
  ["CONFLICTS.kindFolder", "no frame opens a type conflict — the deck has no card copy for one"],
  ["CONFLICTS.kindBinary", "no frame opens a conflict whose pair has no text"],
  // S3's two Phase-1 consequences. Both are sentences the deck does not have because the frame
  // draws the state they replace: `4a Deletions`' permanent card is a FOLDER whose consequence is
  // the subtree aggregate (#208), and no frame draws a permanent FILE deletion at all.
  //
  // They are new sentences rather than the drawn one with the number omitted, which would leave
  // "Deleting this removes from this computer" — see the note on `folderConsequence` in copy.js.
  // The drawn form stays in the deck and stays gated (it is in DRAWN below), so the day #208 lands
  // the wording it has to match is still checked against the frame it came from.
  [
    "DELETIONS.folderConsequenceUnknown",
    "the drawn sentence needs the subtree aggregate (#208) — this is what Phase 1 says instead",
  ],
  ["DELETIONS.fileConsequence", "no frame draws a permanent deletion of a single file"],
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
  // The C-item templates (C3, C5). Each was a constant until the capability that varies it landed,
  // and each is listed here in the same commit — a template that leaves this table leaves the gate
  // silently, which is the regression the paragraph above records from S1.
  //
  // The arguments are read off the frames the same way every other row's are: `3a Conflict`'s two
  // cards are a one-line change where Proton's also gained a line at the end, and `3a Conflict
  // diff` counts two differing lines against three identical ones.
  [
    "CONFLICTS.versionDiff",
    ["mine", { quoted: "buy milk", extraAtEnd: 0, otherwiseSame: true }],
    "3a Conflict",
  ],
  [
    "CONFLICTS.versionDiff",
    ["theirs", { quoted: "buy oat milk", extraAtEnd: 1, otherwiseSame: false }],
    "3a Conflict",
  ],
  ["CONFLICTS.diffSummary", [2], "3a Conflict diff"],
  ["CONFLICTS.diffCounts", [2, 3], "3a Conflict diff"],
  // S2. The metadata row and the meta line are counted off the pair, so their numbers are the
  // frame's own: `41 bytes` / `4 lines` on the left card, `a plain text file` under the filename.
  // `edited 14:38` is deliberately absent — it renders from an mtime through a local clock, so it
  // moves with the timezone and the hour of the run, which is the one input a gate may not depend
  // on (`fixtures/conflicts.js` names it; DEVIATIONS §74).
  ["CONFLICTS.meta", ["a plain text file"], "3a Conflict"],
  ["CONFLICTS.lineCount", [4], "3a Conflict"],
  ["CONFLICTS.keepBothSub", ["todo.proton-cloud.txt"], "3a Conflict"],
  // The placeholder is drawn on the LEFT only, because the drawn pair puts the extra line on
  // Proton's side. The mirror — `not in Proton's version` — is reachable the moment yours has a
  // line Proton's does not, and no frame draws it; it is checked by `diff.test.js` instead.
  ["CONFLICTS.absentLine", ["mine"], "3a Conflict diff"],
  ["CONFLICTS.clearedSub", [{ total: 3, keptBoth: 2, tookProton: 1 }], "3a Conflicts cleared"],
  ["ONBOARDING.cliMissingBody", [{ id: "debian", name: "Debian" }], "9a CLI missing"],
  ["ONBOARDING.cliInstallCommand", [{ id: "debian", name: "Debian" }], "9a CLI missing"],
  // S3. Nine rows, and SIX OF THEM ARE STRINGS THAT WERE ALREADY DRAWN AND NEVER CHECKED — the
  // deck's own Deletions section lists the four facts (`deleted on Proton 22m ago`, `last opened
  // Mar 2024`, `deleted here 6m ago`, `last edited Jan 2026`) and F7 left them out of copy.js
  // entirely. Nothing caught it because this gate reads copy.js and the frames; a sentence missing
  // from the MODULE is invisible to it. `title` and `armedBody` were constants and are templates
  // now (a live queue has a count in the first and a path in the second), which is the transition
  // the note above records as the way a sentence leaves the gate silently.
  //
  // Every argument is read off the frame, as ever. Two of them name numbers Phase 1 cannot produce
  // (#208) — that is fine and is the point: this table asserts that the DECK still says what the
  // FRAME draws, which stays true and stays worth checking while the app draws the fallback.
  ["DELETIONS.title", [2], "4a Deletions"],
  ["DELETIONS.folderConsequence", ["1,204 photos, 8.4 GB"], "4a Deletions"],
  ["DELETIONS.deletedOnProton", ["22m ago"], "4a Deletions"],
  ["DELETIONS.deletedHere", ["6m ago"], "4a Deletions"],
  ["DELETIONS.lastOpened", ["Mar 2024"], "4a Deletions"],
  ["DELETIONS.lastEdited", ["Jan 2026"], "4a Deletions"],
  ["DELETIONS.armedTitle", ["1,204 photos"], "4a Armed"],
  ["DELETIONS.armedBody", ["photos/2019", "8.4 GB"], "4a Armed"],
  ["DELETIONS.compact.title", [2], "4a Compact"],
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

// Render each drawn template and require its result IN THE FRAME THE TABLE NAMES — not merely
// somewhere in scope, which is what the first version did by ignoring the third tuple element
// (Copilot caught it). The label is the table's whole claim: these arguments were read off THAT
// frame, and a check that accepts any frame lets the claim rot while staying green. Verified by
// pointing one entry at the wrong frame and watching the build fail.
//
// A path that no longer names a function, or a label that is not an in-scope frame, is a build
// failure and not a skip: the point is that this table cannot become a list of things nobody checks.
const templateErrors = [];
const drawnChecks = [];
for (const [path, args, label] of DRAWN) {
  const fn = at(path);
  const shown = `${path}(${args.map((a) => JSON.stringify(a)).join(", ")})`;
  if (typeof fn !== "function") {
    templateErrors.push(
      `${shown} is ${fn === undefined ? "not in the deck" : `a ${typeof fn}`}, not a template`,
    );
    continue;
  }
  if (!frameText.has(label)) {
    templateErrors.push(`${shown} names frame "${label}", which is not an in-scope frame`);
    continue;
  }
  drawnChecks.push([shown, fn(...args), label]);
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

// The templates, each against ITS OWN frame rather than against all of them.
for (const [shown, text, label] of drawnChecks) {
  if (frameText.get(label).includes(text)) found.push(shown);
  else missing.push([`${shown} — expected in ${label}`, text]);
}

console.log(
  `fidelity:copy — ${found.length}/${strings.length - NOT_DRAWN.size + drawnChecks.length} drawn strings matched ` +
    `(${drawnChecks.length} of them templates rendered at the arguments their own frame draws), ` +
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
