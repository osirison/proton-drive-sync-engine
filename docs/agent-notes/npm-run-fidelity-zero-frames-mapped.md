---
trigger: npm run fidelity, node tools/fidelity/assert.mjs, fidelity:assert
depends_on: gui/tools/fidelity/assert.mjs, gui/tools/fidelity/serve.mjs, gui/src/js/app.js
recorded: 2026-08-16
---

# `0/N frames mapped` means the app threw at load — probe for the page error

**Symptom:** `npm run fidelity` reports

```
fidelity:assert — 0/51 frames mapped, 5 assertions, 11 failures
  ...
  100 stale deviation(s) in known-deviations.mjs.
Frames with a fid mapping that stamped none of it — built, and rendering nothing:
  10a Glyph states
  10a In situ
  ... (most of the frame list)
```

and **names no error**. The deviation list, the stale-deviation count and the
"rendering nothing" list are all downstream noise: nothing was stamped because
nothing rendered.

**Fix:** do not read the deviation output. Get the page error directly with a
throwaway probe under `gui/tools/` (it must live there to resolve `puppeteer` —
see `puppeteer-measurement-scripts.md`):

```js
// gui/tools/probe.tmp.mjs — delete after use, it is not a gate.
import puppeteer from "puppeteer";
import { serve } from "./fidelity/serve.mjs";

const { port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
page.on("pageerror", (error) => console.log("PAGEERROR:", error.message));
await page.goto(`http://127.0.0.1:${port}/index.html?frame=${encodeURIComponent("5a Plan")}`, {
  waitUntil: "networkidle0",
});
await browser.close();
process.exit(0);
```

`node tools/probe.tmp.mjs` answers in ~10 seconds where the gate takes ~4 minutes,
and prints the real cause (in the case that produced this note:
`Identifier 'progress' has already been declared` — a `const` shadowing the
function parameter it was named after).

**The precondition that avoids it entirely:** re-run `npm run check` after **every**
edit under `gui/src/js`, including edits made only to satisfy the fidelity gate.
`npm run check` runs eslint, which reports a parse error as a fatal lint failure in
seconds. It is easy to run `check` once, then keep editing JS while iterating on
`fidelity`, and hand a syntactically broken app to a four-minute browser gate.

**Why it was not obvious:** every other failure mode of this gate is reported per
node with a frame, a key and a property, so the instinct is to read the list. A
total render failure produces the *longest* output of any failure mode while
containing the least information about its own cause. The one line that
distinguishes it is the ratio at the top: `0/51 frames mapped` against a healthy
run's `51/51 frames mapped, 96500 assertions, 0 failures`.

`serve()` returns `{ port, server }`, not `{ port, close }` — call
`process.exit(0)` rather than trying to close it.
