# ADR 0006 — Shared-volume event scope (a change elsewhere on the volume is not a reason to walk)

- **Status:** Accepted (the live gate in `tests/events_scope_live.rs` has not been run against a
  real account yet; see "Live gate")
- **Date:** 2026-10-10
- **Relations:** closes ADR 0005 §8a and phase 6 (#456). Changes one line of ADR 0001's list of what
  causes a full walk ("an unresolvable node") and the first pass of ADR 0004's warm start, which no
  longer walks for a node that is not in the pair's folder. Reuses ADR 0001's event stream and
  `reconstruct_remote(base ⊕ delta)`, ADR 0003's rule that the cursor moves only in the final
  commit, and ADR 0005's rule that a pass touches only its own pair.

## Context

Proton reports changes **per volume**, not per folder, and a user's "My files" is usually one
volume. Every pair reads the whole volume's stream, so a delta holds every change anywhere on the
volume. The chain, read from the code before this change:

1. `fetch_event_delta` fetches every change on the volume, with no filter.
2. `TargetedResolver::resolve` placed a created or updated node by listing its parent, but only if
   the parent had an index row. Otherwise it listed the pair's remote root, and if the node was not
   there it returned an error.
3. `reconstruct_remote` turned that error into a fallback, and the pass walked the whole remote
   tree.

So any create or update outside a pair's folder cost that pair one full walk. Events carry no path
and no name, so there was no way to say "this is somewhere else" from the event alone, and no
`proton-drive` verb takes a node id (every one is path-addressed), so the CLI cannot place a node
either.

**Measured (2026-10-09 to 10-10, three pairs on one volume, `journalctl --user -u proton-syncd`,
read only).** Seven "not under any indexed parent or the remote root" fallbacks, caused by **two**
node ids:

| node | who walked for it |
| --- | --- |
| `mg_sxvf…` | Documents (about 957 folders), four times in a row: each GUI add restarted the daemon in the middle of the walk, the cursor never moved past the event, and the next boot's warm start met the same event again |
| `h3ZYV36…` | Photos, videos and Documents: one node, three pairs, three walks |

The walks measured 28 and 30 minutes for Documents and 5 to 6 seconds for the small pairs. Passes
run one at a time, so the small pairs waited out the large one's walk. The small pairs' walks followed
the large pair's activity, which is the case the issue said would make a fix worth building. Four
further fallbacks in the same window were `curl` timeouts and a 401; they are not this ADR's.

A second instance, inside one pair: a node created in a folder that was itself created in the same
delta could not be placed, because the new folder has no index row until the pass commits.

**The one hole.** The commit for an `Upload` or a `CreateRemoteDirectory` records no node id: the
client functions return `()` and the id arrives only from a later listing (the planner's `AutoLink`).
So a folder the daemon has just made has `proton_id = NULL` until the pass that consumes its own
`Created` event, and an event whose parent is unknown may be about *that* folder. Skipping it would
advance the cursor past a create nothing re-derives, which is #30.

## Decision

Build ADR 0005's candidate (a), **per pair, from the pair's own state only**, with the decision
moved into the pure `reconstruct_remote`.

1. **The resolver is two primitives**, `indexed_path(uid)` (the index) and `list(directory)` (one
   memoized single-directory listing). Which one answers an event is `reconstruct_remote`'s
   decision, so it is tested without a daemon.
2. **The pair's root uid is an input.** `reconstruct_remote` takes the composed uid of the node the
   pair's `remote_root` names. The root is a known parent (`""`). An event about the root itself
   always falls back: it is never skipped and never placed.
3. **The in-pass map comes before the index.** A parent created or moved earlier in the same delta
   is placed, which fixes the second instance for any pair.
4. **An event whose parent is unknown is deferred, and decided at the end of the delta.** It is
   dropped as "outside" only under **condition S**: the root uid is known and every directory in the
   final map carries a composed uid of this volume. Otherwise the pass falls back to a snapshot and
   the reason names the record that blocked it ("`docs` has no composed id").
5. **A removal of a node the pair does not track** is deferred the same way and dropped under the
   same rule extended to files (every record carries a composed uid). It needs no root uid: the
   removed node could only have been one of the records that lack an id.
6. **A tracked node whose event names an unknown parent** left the tree. Under S it is removed with
   its subtree, unless a later event for the same node was applied (it moved back). A tracked node
   whose event names *no* parent falls back: that cannot be a move.
7. **A directory that appears by an `Updated` event while untracked** (moved in from elsewhere, or
   restored from the trash) falls back, because volume events are per link and nothing describes
   what is inside it. This also closes a gap that existed before this change. The exception is a
   directory record that already exists at that very path without an id: that is the daemon's own
   folder being given its id, not a newcomer.
8. **Without a root uid, nothing changes.** Placement is the old rule (the index, then the root
   listing, then the error), the removal rule still skips when every record has an id, and the
   reason strings are the old ones.
9. **Learning the root uid.** `ProtonClient::remote_root_uid` returns it from the root listing's
   wrapper node when the CLI prints an id there, otherwise from the root's entry in its parent's
   listing. At most two listings. A full walk that leaves a cursor asks every time (a root that was
   deleted and made again is a different node) and stores the answer or clears what was stored; one
   that leaves none asks nothing, because nothing streams from it (an empty remote root names no
   volume, and such a pair walks on every poll). An incremental pass
   asks once per run, only when it has events to place and none is stored, which is how an index
   written before this change learns it without a walk. It is stored in a single-row table
   (`remote_root_node`) in the **final commit**, beside the cursor, cleared by `reset-index`, and
   cleared when the remote root is found missing. Unknown means nothing is skipped.
10. **No other pair's index is read.** ADR 0005 §8a's second part (consult every pair's index on the
    volume) is not built. With S, a node is outside because *this* pair's tree is fully named, not
    because another pair claims it; the extra reads would couple the pairs, add a second connection
    per pair and make a pass read state that `PairPass` exists to make unreachable.
11. **Resuming a pair queues a pass.** `resume` on a pair that was paused queues the same explicit
    `Sync` job `syncnow` does, so a pair no longer waits for its next timer after a long pause (in
    the log, the pass came 20 seconds after the resume; on a 300-second interval it would be
    minutes). A pair that was not paused queues nothing. An unavailable pair gets its retry at once.

Two log lines are new. `event-driven pass skipped changes outside this folder` (with a count) when
anything was dropped, and the fallback reason `cannot tell whether node N is outside this folder:
<path> has no composed id` when S failed.

## Soundness

An event dropped under S is never a change inside the pair's tree that nothing later re-derives.
**Claim and argument.** Let an event name node N with parent P. If N is inside the tree, P is the
root or a directory Q of the tree.

- P is the root: the root uid is known, so P is a known parent and the event was not deferred.
- P is a directory Q of the tree: under S every directory in the final map has its uid in the
  in-pass map, so if Q is in the final map the event was not deferred. So Q is not in the final map.
  Every way that can happen while Q is inside the tree:

| # | Q is inside the tree but absent from the final map because… | closed by |
| --- | --- | --- |
| 1 | Q is a folder this pair created and has no id yet | Q's own `Created` event is in this delta (placed, so Q is in the map with an id: contradiction), or in an earlier delta (that pass placed Q and `AutoLink` recorded its id in the commit the cursor advanced in: contradiction), or has not arrived (Q is in the map **without** an id, S fails, the pass walks) |
| 2 | Q was created earlier in the same delta | placed before any event under it (in-order delivery), so it is in the in-pass map |
| 3 | Q is excluded by selective sync | it is absent from the filtered base and a full walk would not list N either, which is the standard the old skip of an untracked removal already used |
| 4 | Q was moved in from elsewhere, or restored from the trash | in this delta: its `Updated` event falls back (point 7). In an earlier delta: that pass fell back and walked, so Q and its subtree are indexed |
| 5 | Q is a descendant of a moved-in directory | no events exist for it; the fallback in 4 walks them |
| 6 | the root itself was renamed, moved or trashed | an event about the root falls back |
| 7 | the index is incomplete for another reason | after a full walk every inside, non-excluded directory has a composed id (listings are all-or-nothing, #59). `Keep` and `reset-index` latch a full walk, a partial pass holds the cursor, and a legacy raw id or an empty placeholder counts as missing, so S fails |
| 8 | another client creates something inside a folder this pair just created | that folder is case 1 or 2 |
| 9 | Q was removed or moved out earlier in the delta | N left with it; a later event for N describes an outside node |

Each row contradicts "deferred and dropped" or ends in a fallback.

The assumption the whole argument uses, and ADR 0001's cascade already used: **events arrive in the
order things happened**. A foreign event can sit before the event that names the daemon's own folder;
that is why the decision waits for the end of the delta.

Not claimed: descendants of a directory renamed *within* the tree keep their old paths in the
reconstructed map. That was already true and this change neither fixes nor depends on it. A pinning
test is listed under follow-ups.

## Consequences

- **Cost.** A foreign event costs no `proton-drive` call and one pass over the pair's map (hash
  lookups) when something was deferred. Before: one root listing and a full walk. A bootstrap pays
  up to two more listings to learn the root uid.
- **The cursor policy is unchanged.** A skipped event is "applied" by being outside. The five causes
  that hold the cursor (withheld delete, vanished node, failed item, skipped destructive row, a
  folder that vanished) still do, whatever else the pass skipped. #30, the bootstrap's pre-walk
  anchoring (#294, #303), selective sync and the plan pass (which still full-walks and learns
  nothing) are untouched.
- **What still walks.** A fetch error or a server refresh; a `Created` node absent from its known
  parent's listing (#30); an event about the pair's own root; a directory moved in or restored; a
  duplicate id in the baseline; a tree with a directory that has no composed id (the window between
  the daemon making a folder and its own event arriving); a pair that cannot learn its root uid.
- **A new invariant** for CLAUDE.md: an event the pair cannot place is skipped only when the pair's
  tree is fully named.
- **What to watch on the live log** after the upgrade: zero "not under any indexed parent" lines for
  foreign nodes; `warm start completed` for every pair at every boot, a GUI add included; the skip
  line on the small pairs whenever Documents uploads, and the reverse; and the rate of `cannot tell
  whether node … is outside` (each one on Documents is a 28-minute walk).
- **A residual, contrived:** the root uid names a node and `remote_root` names a path. If an
  ancestor of the folder is renamed and someone makes a different folder at the old path before the
  pair notices, no event is about the old root, so the pair keeps the old uid until its next walk and
  skips events inside the new folder. The window is one poll plus that person's speed; a walk
  (restart, `resync`, the scheduled sweep) re-asks. A check of the uid before dropping anything
  would close it at the price of one listing per pass that drops events; not built.
- **Not measured.** Whether the real CLI prints an id on the root listing's wrapper node (the
  second source covers it if not), and whether Proton sends an event for the root folder when a
  child is added (point 2 would then read every upload as "the root changed" and walk). The live
  gate reports both.

## Alternatives considered

- **(b) leave it.** Measured: 28-minute walks per foreign node per restart, and the small pairs
  waiting behind them.
- **Record the node id at commit time from the CLI's output.** `upload` and `create-folder` print
  text the engine does not parse and has no fixture of; reverse-engineering it is not available.
- **List the parent right after every `CreateRemoteDirectory` and `Upload`.** One child per created
  item on every pass that creates, to save a rare walk.
- **Consult every pair's index.** Adds nothing S does not already say and couples the pairs (see
  point 10).
- **Skip on "not in the root listing" without S.** Unsound: that is the hole.
- **Learn the root uid from events** (the `ParentLinkID` of a node found in the root listing). The
  listing is current and the delta is past: a node created under X and moved into the root after the
  delta's end is in the listing while its newest event names X, and the pair would record X as its
  root. A listing-based source has no such window.
- **Backfill ids by name before failing S** (list the parent of each id-less folder and take the
  uid of the entry at that path). Sound, and one child per distinct parent of an id-less folder only
  when S would otherwise fail. Held back until the live log shows how often S fails.

## Live gate

`tests/events_scope_live.rs`, in the shape of `events_identity_live.rs`, `#[ignore]`:

1. read only: the client learns the root uid, it is a composed uid, and its volume half is the
   events volume; the two sources agree when both can answer;
2. opt-in write (`PROTON_SYNC_LIVE_WRITE=1`): upload a probe at the root, read its `Created` event,
   assert `node_uid(volume, ParentLinkID)` is the root uid, and report whether the root itself got an
   event. The probe is deleted.

The skip is relied on once this has passed on a real account. If the first check fails a pair never
learns its root and nothing is skipped (the old behaviour); if the second fails, do not ship.

## Follow-ups

- Backfill ids by name (above), if the S-failure rate on the live log is worth it.
- An empty pair (a remote root with no children) cannot name its volume and walks on every poll; the
  root uid names the volume and would end that. Its own issue.
- A test that pins the stale paths of a renamed directory's descendants, before anyone touches it.
