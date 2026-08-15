---
trigger: gh api pulls/N/requested_reviewers, gh pr edit --add-reviewer Copilot, re-request Copilot review, copilot-pull-request-reviewer
depends_on: gh auth (repo scope)
recorded: 2026-08-15
---

# Copilot re-reviews only through the GraphQL `botIds` field

**Symptom:** Copilot auto-reviews the **first** push to a PR and then goes quiet.
Later pushes get no new review, so the "Copilot's latest review is at the PR head
SHA" gate can never be met. Asking for one over REST looks like it worked:

```bash
gh api --method POST repos/OWNER/REPO/pulls/N/requested_reviewers \
  -f "reviewers[]=Copilot"                      # 201, and requested_reviewers is []
  -f "reviewers[]=copilot-pull-request-reviewer[bot]"   # same
```

Both return the full PR object with `"requested_reviewers": []`. Nothing was
requested and nothing errored — the reviewer is a **Bot**, and the REST endpoint's
`reviewers[]` only accepts Users.

**Fix:** GraphQL `requestReviews`, passing the bot through `botIds` (not `userIds`):

```bash
PR_ID=$(gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){id}}}' --jq '.data.repository.pullRequest.id')

# The bot's node id, read off any review it already left on any PR in the repo:
BOT_ID=$(gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){reviews(first:5){nodes{author{... on Bot {id}}}}}}}' \
  --jq '[.data.repository.pullRequest.reviews.nodes[].author.id|select(.)][0]')

gh api graphql -f query="mutation{requestReviews(input:{
  pullRequestId:\"$PR_ID\", botIds:[\"$BOT_ID\"], union:true}){
  pullRequest{reviewRequests(first:5){nodes{requestedReviewer{
    ... on Bot {login} ... on User {login}}}}}}}"
```

`union: true` matters — without it the mutation **replaces** the whole reviewer
set, dropping any human reviewer already requested.

At the time of writing `BOT_ID` for `copilot-pull-request-reviewer` in this repo is
`BOT_kgDOCnlnWA`. Read it fresh rather than trusting that: it is a GitHub-side id,
not something this repo controls.

**Verifying it landed:** REST `requested_reviewers` stays `[]` for bots even when
the request is live. Check GraphQL instead:

```bash
gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){reviewRequests(first:5){nodes{requestedReviewer{
    ... on Bot {login}}}}}}}'
```

**Related gates.** "No unresolved comments" is mechanical and replying does not
satisfy it — resolve the threads:

```bash
gh api graphql -f query='query{repository(owner:"OWNER",name:"REPO"){
  pullRequest(number:N){reviewThreads(first:20){nodes{id isResolved}}}}}'
gh api graphql -f query='mutation{resolveReviewThread(input:{threadId:"PRRT_…"}){
  thread{isResolved}}}'
```

**Bash gotcha when replying.** A reply body full of backticked identifiers must not
go through a double-quoted `-f body="…"`: the shell runs the backticks as command
substitution and the identifiers vanish from the posted comment, silently. Write
the body to a JSON file and use `--input`, or a single-quoted heredoc.

**Order matters: request LAST.** A push **clears** a pending bot review request —
`reviewRequests` goes back to `[]` and no review is ever posted. So the sequence is
always *finish pushing, then request*, never the other way round. A request made
before the last commit looks accepted and then silently evaporates.

**Cheapest prevention:** batch polish commits. Every push restarts the review
cycle, the auto-review only fires on the first one, and each push voids whatever
request was outstanding.
