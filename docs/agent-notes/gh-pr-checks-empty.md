---
trigger: gh pr checks, gh pr view --json statusCheckRollup, gh run list --branch
depends_on: .github/workflows/ci.yml, .github/workflows/audit.yml
recorded: 2026-08-15
---

# `no checks reported` usually means the PR conflicts, not that CI is slow

**Symptom:** minutes after opening a PR, `gh pr checks <N>` says

```
no checks reported on the '<branch>' branch
```

and `gh run list --branch <branch>` shows only the Copilot review run — while a
sibling branch pushed at the same minute has CI, CodeQL and Security audit runs.

**Fix:** check `gh pr view <N> --json mergeable`. On `CONFLICTING`, merge `origin/main`
into the branch and push; every `pull_request` workflow starts within seconds of the
conflict clearing.

Every workflow here triggers on `pull_request`, which GitHub runs against the
`refs/pull/<N>/merge` ref. That ref cannot be computed while the PR conflicts, so no
run is ever created — the PR is not queued, it is skipped.

**Why it was not obvious:** the PR page shows no error, the checks section is simply
empty, and it looks identical to a busy runner queue. Waiting does not fix it. The
conflict itself can appear *after* the PR is opened, when another branch merges first,
so a PR that had checks can stop getting them.
