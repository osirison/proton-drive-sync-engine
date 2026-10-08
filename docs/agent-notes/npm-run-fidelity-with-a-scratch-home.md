---
trigger: npm run fidelity, HOME=<scratch dir>, PUPPETEER_CACHE_DIR, "Could not find Chrome", isolated GUI test run
depends_on: gui/package.json, gui/tools/fidelity/assert.mjs
recorded: 2026-10-08
---

# `npm run fidelity` under a scratch `HOME` cannot find Chrome

**Symptom:** the GUI test isolation recipe points `HOME` (and the `XDG_*` directories) at a scratch
directory so nothing reaches the developer's real config, socket or keyring. `npm run check` passes
under it. `npm run fidelity` then dies before drawing a frame:

```
Error: Could not find Chrome (ver. 153.0.8010.36). ...
 2. your cache path is incorrectly configured (which is: <scratch HOME>/.cache/puppeteer).
    at async file:///.../gui/tools/fidelity/assert.mjs:88:17
```

**Cause (measured):** puppeteer looks for its browser under `$HOME/.cache/puppeteer`. The message names
exactly that path with `XDG_CACHE_HOME` pointed somewhere else, so it follows `HOME` and not the XDG
variable. The scratch `HOME` has no browser in it.

**Fix:** leave the scratch `HOME` and name the real cache for this one tool:

```bash
PUPPETEER_CACHE_DIR=$REAL_HOME/.cache/puppeteer npm run fidelity
```

**Why that is safe:** the gate is headless, serves `gui/src` from an ephemeral local port and draws
fixtures. It starts no daemon, opens no socket of the app's and reads no account. The cache is read,
never written. The same variable serves `fidelity:stale`, `fidelity:determinism` and
`glyphs:check`; `npm run check` does not need it.
