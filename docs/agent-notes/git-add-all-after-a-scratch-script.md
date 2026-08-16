---
trigger: git add -A, git commit -a, python3 scratch script, heredoc edit
depends_on: .gitignore
recorded: 2026-08-16
---

# A scratch script that fails leaves itself behind, and `git add -A` commits it

**Symptom:** a PR diff contains a file nobody meant to ship —
`.claude/scratchNN.py`, `gui/tools/*.tmp.mjs`, a one-off patch script. It is not
referenced by anything and no gate mentions it. In the case that produced this
note it reached a reviewer, who correctly asked why unused tooling was in-tree.

**Cause:** the cleanup was chained to the script's own success:

```bash
python3 .claude/scratch16.py && command rm -f .claude/scratch16.py && cargo build
```

Every one of these scripts starts with `assert s.count(old) == 1` — a guard that
fires exactly when a previous edit or `cargo fmt` has already reflowed the target.
On that failure the script exits non-zero, `&&` short-circuits, **the `rm` never
runs**, and the next `git add -A` takes the file in. The failure is loud and the
leak is silent, so attention goes to the assertion and the file is forgotten.

**Fix — either, and preferably both:**

- Separate the cleanup from the outcome, so it runs whether or not the edit
  applied: `python3 x.py; command rm -f x.py` (`;`, not `&&`).
- Write scratch scripts somewhere already ignored, or check the diff before
  committing. `git diff --name-only origin/main...HEAD` before every
  `git add -A` is one command and shows the whole set — the leak is obvious in a
  26-line list and invisible in a 26-file commit.

Note `.claude/` is **tracked** in this repo (it carries `commands/` and `skills/`),
so a file dropped there is staged like any other. It is not a scratch directory.

**Why it was not obvious:** `cargo fmt`, `clippy -D warnings`, the test suites and
the whole fidelity gate all pass with a stray Python file in the tree — nothing
compiles it, lints it or reads it. The only checks that can see it are a human
reading the diff and a reviewer reading the file list.

**Related:** a `.tmp.mjs` probe under `gui/tools/` has the same shape and the same
fix (`puppeteer-measurement-scripts.md` says to delete it afterwards — this is why
that sentence is there).
