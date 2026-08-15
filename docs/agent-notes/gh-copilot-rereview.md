---
trigger: gh api pulls requested_reviewers, copilot re-review, Running Copilot Code Review
depends_on: repository Copilot code review settings
recorded: 2026-08-15
---

# Copilot re-review CAN be triggered from the CLI

**Symptom:** Copilot auto-reviews when a PR is opened, but a later push (for example a
merge from `main`) does not trigger a new review, leaving the newest review pinned to an
older commit — which fails the "latest review is at the PR head SHA" rule.

**Fix:**

```bash
gh api -X POST repos/<owner>/<repo>/pulls/<N>/requested_reviewers \
  -f "reviewers[]=copilot-pull-request-reviewer[bot]"
```

Confirm with `gh run list --branch <branch>`: a `Running Copilot Code Review` run appears
at the current head SHA within ~1 minute.

**Why it was not obvious:** the call **looks like it failed**. It returns 201 with the
full PR object whose `requested_reviewers` is `[]`, and `gh pr view --json reviewRequests`
also stays empty, so every observable says the reviewer was not added. The review runs
anyway. Verify by the workflow run and by the new review's `commit_id`
(`gh api repos/<owner>/<repo>/pulls/<N>/reviews --jq '.[] | "\(.user.login) \(.commit_id)"'`),
never by the requested-reviewers list.
