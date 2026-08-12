---
trigger: node <script>.mjs, puppeteer.launch, import puppeteer, headless measurement, npm run fidelity
depends_on: gui/package.json, gui/tools/fidelity/serve.mjs
recorded: 2026-08-12
---

# A script that imports puppeteer must live under `gui/`

**Symptom:** an ad-hoc measurement script written to a scratch directory
(`$CLAUDE_JOB_DIR/tmp`, `/tmp`) dies immediately:

```
Error [ERR_MODULE_NOT_FOUND]: Cannot find package 'puppeteer' imported from
/home/…/tmp/measure-something.mjs
```

**Fix:** put the script inside the `gui/` tree — `gui/tools/<name>.tmp.mjs` works —
and run it from there. Delete it afterwards; it is not a gate.

`puppeteer` is a devDependency of `gui/package.json` and is installed into
`gui/node_modules`. Node resolves a bare specifier by walking `node_modules`
upward **from the importing file**, so a script outside `gui/` never sees it. The
repository root has no `node_modules` and no `package.json`.

**Why it was not obvious:** the working directory is irrelevant here — running
`node /tmp/x.mjs` from inside `gui/` still fails, because resolution follows the
file's own path and not `process.cwd()`. The scratch directory is otherwise the
right place for temporary files (parallel jobs share `/tmp` and clobber each
other), so the instinct to write there is correct and the failure is surprising.

**Useful with it:** `gui/tools/fidelity/serve.mjs` exports `serve()`, which puts a
static server over `gui/src` on an ephemeral port — the same one the fidelity gate
and the tray-glyph renderer use. Import that rather than writing a fourth static
handler, and load `http://127.0.0.1:${port}/index.html`: the app is ES modules and
a module graph over `file://` hits cross-origin rules the real webview never does.

The window is `1040×764` at `deviceScaleFactor: 1` (`assert.mjs`).
