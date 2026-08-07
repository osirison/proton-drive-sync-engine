// The fixture gate (F9) — the registry describes every in-scope frame, and describes it usefully.
//
// WHY THIS EXISTS AT ALL. `assert.mjs` counts a frame as mapped only when a node in the rendered app
// carries a `data-fid`, so a fixture cannot inflate that number — which is right, and it is also why
// nothing was checking the fixtures themselves. F9's deliverable is "every in-scope frame
// reproducible", and without this gate that claim rests on someone having counted to 51 once.
//
// It has to have teeth, because label-set equality alone passes on `{ "3a Conflict": {} }` — 51
// labels, 51 empty objects, a green gate and not one reproducible frame. So four checks:
//
//   1. the label set IS `frames/index.json`'s, in both directions;
//   2. every entry carries a payload of the shape its frame CLASS implies, and it is not empty;
//   3. no `fids` slot is dead — its node key exists in at least one frame declaring it;
//   4. no fixture module reads the wall clock except through `clock.js`.
//
// (3) is the one that earns its keep long before S10. `assert.mjs` catches a stale key only for
// nodes the app actually stamps, at runtime, in a browser — so a mapping written for a node the
// screen no longer renders is invisible until someone re-extracts the prototype. This reads the
// mapping against the frame directly, in Node, in milliseconds. Its exact rule, and why it is
// "alive somewhere" rather than "alive here", is argued where it is implemented.
//
// Runs in the `frontend` CI job rather than `fidelity`: it needs no browser, and a gate that can run
// in the cheap job should.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { FIXTURES, resolveFixture } from "../../src/js/fixtures/frames.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const FRAMES = join(HERE, "frames");
const SRC_FIXTURES = join(HERE, "..", "..", "src", "js", "fixtures");

const index = JSON.parse(readFileSync(join(FRAMES, "index.json"), "utf8"));
const failures = [];
const fail = (label, what) => failures.push({ label, what });

// ---- 1. the label set ----------------------------------------------------------------------

const drawn = new Map(index.map((e) => [e.label, e]));
const registered = new Set(Object.keys(FIXTURES));

for (const label of drawn.keys()) {
  if (!registered.has(label)) fail(label, "in-scope frame with no fixture — F9 is not complete without it");
}
for (const label of registered) {
  if (!drawn.has(label)) {
    // Almost always a typo in a long label rather than a genuinely invented frame, so say which
    // frames are nearby instead of only that this one is wrong.
    const near = [...drawn.keys()].filter((l) => l.split(" ")[0] === label.split(" ")[0]);
    fail(label, `no such frame in index.json${near.length ? ` — nearest: ${near.join(", ")}` : ""}`);
  }
}

// ---- 2. a payload of the right shape --------------------------------------------------------

// What a frame class implies its fixture must carry. The `window`/`dialog`/`crop` classes are all
// screens the daemon drives, so all three want a status payload; `compact` is the 362px panel;
// `notification` is a banner the desktop sizes; `specimen` is scenery around an artefact and has no
// payload at all, only a note saying which task draws it.
const REQUIRED = {
  window: "status",
  dialog: "status",
  crop: "status",
  compact: "panel",
  notification: "notification",
  specimen: "specimen",
};

// `sameAs` — the light frames say "same data, swapped tokens" rather than copying it — is resolved
// by IMPORTING the app's own resolver rather than reimplementing it here. The first version of this
// file had a second copy that returned the twin and dropped the entry's own keys, which agrees with
// `frames.js` only while no `sameAs` entry carries an override. The first `{ sameAs: "2a Settled",
// status: {…} }` S10 writes would have been shape-checked against the wrong object, and a gate that
// disagrees with the thing it gates is worse than no gate. One rule, one implementation.

const nonEmpty = (v) =>
  v != null && (typeof v !== "object" || (Array.isArray(v) ? v.length > 0 : Object.keys(v).length > 0));

for (const [label, entry] of Object.entries(FIXTURES)) {
  const spec = drawn.get(label);
  if (!spec) continue; // already reported above
  if (!nonEmpty(entry)) {
    fail(label, "empty fixture — a label with no data reproduces nothing");
    continue;
  }
  if (entry.sameAs && !FIXTURES[entry.sameAs]) {
    fail(label, `sameAs: "${entry.sameAs}" names no fixture`);
    continue;
  }
  const resolved = resolveFixture(label);
  if (!resolved) {
    fail(label, `sameAs chain does not resolve (cycle, or a missing link)`);
    continue;
  }
  const want = REQUIRED[spec.kind];
  if (!want) {
    fail(label, `frame class "${spec.kind}" has no required shape — add one to REQUIRED`);
    continue;
  }
  if (!nonEmpty(resolved[want])) {
    fail(
      label,
      `a ${spec.kind} frame must carry a non-empty \`${want}\` (has: ${Object.keys(resolved).join(", ") || "nothing"})`,
    );
  }
}

// ---- 3. no fids slot is dead ------------------------------------------------------------------

// THE RULE IS "ALIVE SOMEWHERE", NOT "ALIVE HERE", and getting that wrong is what the first version
// of this check did. Measured, not assumed:
//
// `compactFids` in fixtures/frames.js is a factory over four tree SHAPES, so it hands every frame in
// a shape the whole slot vocabulary — `meta`, `action`, `subBreak`, `hexRect`, `hexNumeral` — and
// each frame draws a subset. `fid()` only stamps a slot the screen actually renders, so a slot this
// frame has no node for is inert, not wrong. Requiring every declared slot to resolve per frame
// flagged 23 of 180 keys, and every one of the 23 was a slot that frame legitimately does not draw:
// `meta` failed on four frames and resolved on `10a Offline`, the only compact that draws a retry
// line; `hexRect` resolved only on `10a Paused`, whose mark is two bars. The signal was clean and
// the verdict was wrong.
//
// So a slot is dead only when its key exists in NO frame that declares it. That still catches the
// failure the path-based key scheme accepts — a prototype edit renames a subtree, and the key goes
// stale in every frame at once — with no false positive on a factory that over-declares by design.
// The per-frame version of this check is `assert.mjs`, which fails loudly on a STAMPED key that no
// longer exists; that is the half that needs a browser, and this is the half that does not.

const slotSites = new Map(); // "slot → key" → { alive, frames[] }
let fidKeysChecked = 0;

for (const [label, entry] of Object.entries(FIXTURES)) {
  const spec = drawn.get(label);
  if (!spec || !entry.fids) continue;
  const frame = JSON.parse(readFileSync(join(FRAMES, spec.file), "utf8"));
  const keys = new Set(frame.nodes.map((n) => n.key));

  const note = (slot, produced) => {
    fidKeysChecked++;
    const id = `${slot} → ${produced[0]}`;
    const site = slotSites.get(id) ?? { alive: false, frames: [] };
    site.alive ||= produced.some((k) => keys.has(k));
    site.frames.push(label);
    slotSites.set(id, site);
  };

  for (const [slot, value] of Object.entries(entry.fids)) {
    if (typeof value === "string") note(slot, [value]);
    else if (typeof value === "function") {
      // A factory slot (`door: (i) => …`) covers a run of siblings, and nothing here knows how many
      // the screen draws. Probe a plausible range and treat the slot as alive if any index resolves.
      const probes = [];
      for (let i = 0; i < 10; i++) {
        try {
          const key = value(i, 0);
          if (typeof key === "string") probes.push(key);
        } catch {
          /* a factory wanting arguments this probe cannot guess — the other indices cover it */
        }
      }
      if (probes.length) note(slot, probes);
    }
  }
}

for (const [id, site] of slotSites) {
  if (!site.alive) {
    fail(
      site.frames.join(", "),
      `fids.${id} — that node key exists in none of the ${site.frames.length} frame(s) declaring it. ` +
        `A renamed subtree, or a mapping written against a re-extracted prototype.`,
    );
  }
}

// ---- 4. the clock convention ----------------------------------------------------------------

// `clock.js` is the single place a fixture may read the wall clock, and its own header says a
// fixture reaching for `new Date()` is a bug. Cheap to state, cheap to check, and the failure it
// prevents is a frame that reproduces on the machine that wrote it and nowhere else.
for (const file of readdirSync(SRC_FIXTURES)) {
  if (!file.endsWith(".js") || file === "clock.js" || file === "preview.js") continue;
  const source = readFileSync(join(SRC_FIXTURES, file), "utf8");
  for (const [pattern, what] of [
    [/\bnew Date\b/, "new Date()"],
    [/\bMath\.random\b/, "Math.random()"],
  ]) {
    if (pattern.test(source)) {
      fail(file, `uses ${what} — the only clock a fixture may read is clock.js's \`ago()\` (see its header)`);
    }
  }
}

// ---- report ----------------------------------------------------------------------------------

const withFids = Object.values(FIXTURES).filter((f) => f.fids).length;
console.log(
  `fidelity:fixtures — ${registered.size}/${index.length} frames have a dataset, ` +
    `${withFids} carry a fids map (${fidKeysChecked} keys checked), ${failures.length} failures`,
);

if (failures.length) {
  console.error("");
  for (const f of failures) console.error(`  ${f.label}\n      ${f.what}`);
  console.error(`\nfidelity:fixtures: ${failures.length} failure(s).`);
  process.exit(1);
}
