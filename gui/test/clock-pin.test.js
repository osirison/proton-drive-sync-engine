// The clock the fidelity gates load pages at (#102 phase 5a-2, K1).
//
// A fidelity page load freezes `Date.now()` at whatever the machine's clock reads when `clock.js`
// runs, so two loads of one URL freeze at two instants and an ABSOLUTE time a frame prints from the
// clock (`since 15:17`) differs between them whenever a minute falls in the gap. `clock-pin.mjs`
// installs one instant in the page BEFORE `clock.js` reads it. This file proves that with the real
// `clock.js` and `format.js`, and without a browser, by running the page's own installer against a
// stand-in global and loading `clock.js` under it — twice, with the real clock 61 seconds apart.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { installClock, PINNED_NOW_MS, SKEW_STEP_MS } from "../tools/fidelity/clock-pin.mjs";
import { clock } from "../src/js/ui/format.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const CLOCK_JS = pathToFileURL(join(HERE, "..", "src", "js", "fixtures", "clock.js")).href;

/** A `Date` of its own, so installing a clock on it (and `clock.js` assigning `Date.now`) leaves Node's alone. */
const standIn = () => ({ Date: class StandInDate extends Date {} });

let loads = 0;
/**
 * One "page load": install the clock the way the page would, then run `clock.js` against it exactly as
 * a `?frame=` page does, and hand back what a fixture can read afterwards. The real clock reads
 * `skewMs` later than the one before it.
 */
async function loadPage({ pinned, skewMs }) {
  const scope = standIn();
  installClock(pinned ? PINNED_NOW_MS : null, skewMs, scope);
  const realDate = globalThis.Date;
  const realLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  globalThis.Date = scope.Date;
  globalThis.location = { search: "?frame=9a%20Consent" };
  try {
    // A fresh specifier per load, or the module cache returns the first load's frozen instant.
    loads += 1;
    const module = await import(`${CLOCK_JS}?load=${loads}`);
    const lastSync = module.ago(60);
    return { lastSync, printed: `since ${clock(lastSync)}`, now: scope.Date.now() };
  } finally {
    globalThis.Date = realDate;
    if (realLocation) Object.defineProperty(globalThis, "location", realLocation);
    else delete globalThis.location;
  }
}

test("a pinned page reads one instant, whatever the real clock says when it loads", async () => {
  const first = await loadPage({ pinned: true, skewMs: 0 });
  const second = await loadPage({ pinned: true, skewMs: SKEW_STEP_MS });
  const third = await loadPage({ pinned: true, skewMs: 2 * SKEW_STEP_MS });
  assert.equal(first.now, PINNED_NOW_MS);
  assert.deepEqual(second, first, "a load 61s later froze the same instant and printed the same time");
  assert.deepEqual(third, first);
  assert.equal(first.lastSync, Math.floor(PINNED_NOW_MS / 1000) - 60, "`ago(60)` is sixty seconds before it");
});

test("the printed absolute time is the part that moved, and the pin is what stops it", async () => {
  // THE POSITIVE CONTROL. Without the pin, two loads 61s apart cross a minute boundary on every
  // possible start second, so the time `9a Consent` prints differs — which is the CI failure,
  // reproduced deterministically instead of one run in three. If this ever passes, the test above
  // has stopped being able to fail.
  const first = await loadPage({ pinned: false, skewMs: 0 });
  const second = await loadPage({ pinned: false, skewMs: SKEW_STEP_MS });
  assert.notEqual(second.lastSync, first.lastSync);
  assert.notEqual(second.printed, first.printed, `${first.printed} vs ${second.printed}`);
});

test("every way a script asks for the time answers the pinned instant, and a date built from a value is untouched", () => {
  const scope = standIn();
  installClock(PINNED_NOW_MS, 0, scope);
  const { Date: PinnedDate } = scope;
  assert.equal(PinnedDate.now(), PINNED_NOW_MS);
  assert.equal(new PinnedDate().getTime(), PINNED_NOW_MS);
  assert.equal(new PinnedDate(0).getTime(), 0, "an explicit value is still that value");
  assert.equal(new PinnedDate("2020-01-02T03:04:05Z").getTime(), Date.UTC(2020, 0, 2, 3, 4, 5));
  assert.equal(new PinnedDate(2020, 0, 2).getFullYear(), 2020);
  assert.equal(PinnedDate.UTC(2020, 0, 2), Date.UTC(2020, 0, 2), "statics are the real ones");
  assert.equal(
    PinnedDate(),
    new Date(PINNED_NOW_MS).toString(),
    "`Date()` called bare is the string form of now",
  );
  assert.ok(new PinnedDate() instanceof PinnedDate);
  assert.ok(new PinnedDate() instanceof scope.Date);
  assert.equal(Object.prototype.toString.call(new PinnedDate()), "[object Date]");
});

test("without a pin the real clock is only moved, never frozen", async () => {
  const scope = standIn();
  installClock(null, 5_000, scope);
  const before = Date.now();
  const read = scope.Date.now();
  assert.ok(read >= before + 5_000 && read < before + 5_000 + 1_000, `${read} vs ${before}`);
  assert.ok(Math.abs(new scope.Date().getTime() - read) < 1_000);
});

test("every script that opens a ?frame= page arms the clock first", () => {
  // A gate that loads frames and forgets this is back to failing one run in three, and nothing
  // else would say so. The list is read from the directories, so a NEW script cannot opt out by being
  // absent from it. `tools/` holds the generators (screenshots, tray glyphs) beside the gates in
  // `tools/fidelity/`: they load the same pages and read the same clock.
  const dirs = [join(HERE, "..", "tools", "fidelity"), join(HERE, "..", "tools")];
  const loaders = dirs.flatMap((dir) =>
    readdirSync(dir)
      .filter((file) => file.endsWith(".mjs"))
      .map((file) => join(dir, file))
      .filter((path) => {
        const source = readFileSync(path, "utf8");
        return /page\.goto\(/.test(source) && /\?frame=/.test(source);
      }),
  );
  assert.deepEqual(
    loaders.map((path) => relative(join(HERE, "..", "tools"), path)).sort(),
    [
      "fidelity/assert.mjs",
      "fidelity/check-contrast.mjs",
      "fidelity/check-n1-identity.mjs",
      "render-screenshots.mjs",
      "render-tray-glyphs.mjs",
    ],
    "the set of scripts that render a frame changed — decide whether the new one needs the pin",
  );
  for (const path of loaders) {
    const source = readFileSync(path, "utf8");
    assert.match(source, /from "\.\/(fidelity\/)?clock-pin\.mjs"/, `${path} must import the clock pin`);
    const armAt = source.indexOf("armClock(page");
    const gotoAt = source.indexOf("page.goto(");
    assert.ok(armAt !== -1 && armAt < gotoAt, `${path} must arm the clock before it first navigates`);
  }
});
