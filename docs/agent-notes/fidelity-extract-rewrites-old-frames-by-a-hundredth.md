---
trigger: npm run fidelity:extract, frames/*.json, "the diff must be additions only", 4a-compact.json, 9a-review.json, fidelity:stale
depends_on: gui/tools/fidelity/extract.mjs, gui/tools/fidelity/check-stale.mjs
recorded: 2026-10-09
---

# `fidelity:extract` rewrites two existing frame files by 0.01px

**Symptom:** after adding a frame to the prototype and running `npm run fidelity:extract`, `git status` shows
the new `frames/*.json` files, an `index.json` that gained lines, and **two modified files nobody touched**:

```
 M gui/tools/fidelity/frames/4a-compact.json    "h": 53.97  ->  53.98
 M gui/tools/fidelity/frames/9a-review.json
```

**Cause (measured, 2026-10-09):** sub-pixel text layout differs between the machine that committed the files
and this one. Nothing in the prototype changed for those two frames; `check-stale.mjs` compares old files with
a tolerance and passes either way.

**Fix:** a frame-carrying PR's extract diff must be ADDITIONS ONLY (brief 4.7), so revert every modified frame
file except `index.json` straight after extracting:

```bash
for f in $(git diff --name-only -- tools/fidelity/frames | grep -v index.json); do git checkout -- "../$f"; done
```

(run from `gui/`). Then `git diff --numstat tools/fidelity/frames` should show only `index.json` with
insertions and zero deletions. Re-run it after every extract, because the noise comes back each time.
