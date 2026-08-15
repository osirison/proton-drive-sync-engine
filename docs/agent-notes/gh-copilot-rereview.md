---
trigger: gh api pulls requested_reviewers, copilot re-review, Running Copilot Code Review, resolveReviewThread, suppressed comments
depends_on: repository Copilot code review settings, gh auth (repo scope)
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

**The GraphQL form, when you want the request to be observable.** `requestReviews` with
`botIds` (not `userIds` — the reviewer is a Bot) does show up in `reviewRequests`, so a
poll loop can wait on "is a review pending" rather than on a workflow run:

```bash
PR_ID=$(gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){id}}}' --jq '.data.repository.pullRequest.id')

# The bot's node id, read off any review it has already left in this repo:
BOT_ID=$(gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){reviews(first:5){nodes{author{... on Bot {id}}}}}}}' \
  --jq '[.data.repository.pullRequest.reviews.nodes[].author.id|select(.)][0]')

gh api graphql -f query="mutation{requestReviews(input:{
  pullRequestId:\"$PR_ID\", botIds:[\"$BOT_ID\"], union:true}){
  pullRequest{reviewRequests(first:5){nodes{requestedReviewer{
    ... on Bot {login} ... on User {login}}}}}}}"
```

`union: true` matters — without it the mutation **replaces** the whole reviewer set,
dropping any human reviewer already requested. Read `BOT_ID` fresh rather than hardcoding
it; it is a GitHub-side id, not something this repo controls.

**Order matters: request LAST.** A push **clears** a pending bot review request —
`reviewRequests` goes back to `[]` and no review is posted for the new head. So the
sequence is always *finish pushing, then request*, never the other way round. A request
made before the last commit looks accepted and then silently evaporates. Batch polish
commits for the same reason: every push costs a round.

**Suppressed comments are in the review BODY, not the comments endpoint.** A round can
report "generated no new comments" and still carry findings, inside a
`<details><summary>Suppressed comments (N)</summary>` block in the review body.
`gh api pulls/N/comments` never returns them, so a check that counts only inline comments
reads such a round as clean:

```bash
gh api repos/<owner>/<repo>/pulls/<N>/reviews \
  --jq '.[] | select(.user.login=="copilot-pull-request-reviewer[bot]") | .body' \
  | grep -A25 -i 'suppressed comments'
```

They have been real every time in this repo. There is no thread to reply to either —
answer in a normal PR comment (`gh pr comment --body-file`) quoting the file and line.

**Replied-to is not resolved.** "No unresolved comments" is a mechanical gate that a reply
does not satisfy; resolve the threads:

```bash
gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){reviewThreads(first:20){nodes{id isResolved}}}}}'
gh api graphql -f query='mutation{resolveReviewThread(input:{threadId:"PRRT_…"}){
  thread{isResolved}}}'
```

**Shell trap when replying.** A reply body full of backticked identifiers must not go
through a double-quoted `-f body="…"`: the shell runs the backticks as command
substitution and the identifiers vanish from the posted comment, silently and with a
`command not found` on stderr that is easy to miss. Use `--body-file`, `--input` with a
JSON file, or a single-quoted heredoc.
