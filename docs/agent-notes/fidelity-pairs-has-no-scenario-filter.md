---
trigger: poisoning page code and rerunning fidelity:pairs, "the whole gate takes minutes", check-pair-routing.mjs, a poison round over a dozen scenarios
depends_on: gui/tools/fidelity/check-pair-routing.mjs
recorded: 2026-10-11
---

# `fidelity:pairs` runs every scenario, and has no way to run a few

**Symptom:** a poison round over the page code (remove a line of `app.js`, rerun the gate, restore it) costs the
whole gate each time: about 120 scenarios, several minutes on a busy machine, ten times over.

**Cause:** `check-pair-routing.mjs` is one module of top-level `await scenario(...)` calls that run in file order. No
flag, environment variable or name filter selects some of them.

**Fix (throwaway, never committed):** copy the gate next to itself with a guard in `scenario()`, run that copy, and
delete it before the commit:

```js
async function scenario(name, run) {
  if (process.env.PAIRS_ONLY && !new RegExp(process.env.PAIRS_ONLY).test(name)) return;
  const before = failures.length;
  …
```

```bash
PAIRS_ONLY='Proton folder|confirmation' node tools/fidelity/check-pair-routing-subset.mjs
```

The copy must sit in `tools/fidelity/` (it imports its neighbours by relative path). Helpers and the `Bridge` are
declared outside the scenarios, so a skipped scenario costs nothing and breaks nothing after it.

**Two cautions:**

- The gate serves `gui/src` live from the working tree. A poison applied to `app.js` while a full run is going
  changes that run. Poison only while nothing else is using the gate.
- The full gate still has to pass unpoisoned at the end; the subset proves a poison is caught, not that the page is
  right.
