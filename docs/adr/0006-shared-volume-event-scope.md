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
   always falls back: it is never skipped and never placed. **A pair with include rules has no root
   uid for this purpose**: the folders between the root and a matching file are not sync entities and
   have no index row, so the index does not hold the tree's folders and "not any folder I know" says
   nothing. Such a pair is decided as before this change (the daemon does not even ask the client
   which node the root is); point 8 says exactly how.
3. **The in-pass map comes before the index, for a pair that knows its root.** A parent created or
   moved earlier in the same delta is placed, which fixes the second instance. A pair that does not
   know its root does not ask the in-pass map at all. The map places a node from the end-state
   listing, so it can hold a path that a later event of the delta takes away: the first version
   asked it for every pair, and a folder made outside the tree, a file made in it, the old holder
   of the name trashed and the folder moved in came out `Complete` without the file (found by the
   second review, 233 in 1.9 million small histories; before this change the same history walked).
   Such a pair keeps the old order, the index and then the root listing, so a create under a folder
   created in the same delta still walks for it. The map is still asked one thing: a parent it holds
   at a path the index does not say (the delta placed it, or moved it) cannot be looked up through the
   index, whose path for it is stale and may belong to another folder by now, so that child walks.
   The model test found it under an include rule, one history in 3 million; the old code had the same
   stale lookup.
4. **An event whose parent is unknown is deferred, and decided at the end of the delta, if its node
   is not one the pair holds.** It is dropped as "outside" only under **condition S**, which has
   four parts: the root uid is known (and the pair has no include rules); every directory in the
   final map carries a composed uid of this volume; **no record of the baseline carries a real id
   that is not one of this volume's** (a raw id from an older index, another volume's id: such a
   record can be the node the event is about, moved out, and it is read from the baseline because a
   different node may have taken its path since); and **every record sits directly in the root or
   in a directory of the map**, read over the baseline as well as the final map. The last part is
   what makes "the tree's folders are the directories I hold" true: a record under a folder with no
   row (an index written before folders were rows, or a folder an exclude rule has since hidden)
   could be sitting in exactly the folder the event names. Otherwise the pass falls back to a
   snapshot and the reason names the record that blocked it ("`docs` has no composed id",
   "`docs/x.md` is in a folder the index has no record of"). A record with **no** id yet (`None` or
   empty: the daemon uploaded it last pass) does not block a file's event: its own `Created` event
   names it, and an event of a node that left cannot come before that one. A directory with no id
   does block, because it can be the unknown parent itself.
   **An event that names no parent is never deferred**: nothing says where its node is (a node
   restored from the trash can arrive that way), so the pass walks.
5. **A removal of a node the pair does not track** is deferred the same way and dropped under the
   same rule extended to files (every record carries a composed uid, and no baseline record has an
   older id). It needs no root uid: the removed node could only have been one of the records that
   lack an id.
6. **A tracked node whose event names a parent the pair does not hold walks.** It left the tree, or
   it went into a folder the pair cannot see into (no row, nothing recorded beneath it), and the
   event does not say which. The first version read it as "left": the node was taken out of the map,
   and a file moved into such a folder came out of the next plan as deleted remotely, with the
   local copy removed when the guard was off. A real move out of the tree therefore costs one walk;
   that was accepted, because the other reading loses a file. "Tracked" includes a node an earlier
   event of the same delta took out of the map (an update the end-state listing could not find where
   it said, a node the listing put over its record): its next event is the one that says where it
   went. The same walk covers an event whose parent was taken out of the map earlier in the delta:
   the index still holds that parent's old path, and the path may belong to another folder by now
   (found by the second review's fuzz, one in 8 million histories). An update whose node is absent
   from its parent's listing drops the node **with what was beneath it**: events are per link, so
   nothing else says its children went with it. A directory that moves back in is `Updated` while
   untracked, so it falls back (point 7).
7. **A directory that appears by an `Updated` event while untracked** (moved in from elsewhere, or
   restored from the trash) falls back, because volume events are per link and nothing describes
   what is inside it. This also closes a gap that existed before this change. The exception is a
   directory record that already exists at that very path without an id: that is the daemon's own
   folder being given its id, not a newcomer.
8. **Without a root uid, placement is the old rule.** The index, then the root listing, then the
   error; the in-pass map is not asked (point 3); the removal rule still skips when every record has
   an id; the reason strings are the old ones. That is also the whole of what an include-rule pair
   does. Three things differ from the code before this change. Two can only add a walk: a directory
   that appears by an update falls back (point 7), and a parent that an earlier event of the same
   delta took out of the map, placed or moved is not resolved through its stale index path (points 3
   and 6). The third removes the entries beneath a folder whose update cannot find it (point 6),
   which the old code left in the map. A delta of nothing but dropped removals is also an idle pass for such a pair
   (point 12): it drops untracked removals as outside, as it always did.
9. **Learning the root uid.** `ProtonClient::remote_root_uid` returns it from the root listing's
   wrapper node when the CLI prints an id there, otherwise from the root's entry in its parent's
   listing. At most two listings. A full walk that leaves a cursor asks every time (a root that was
   deleted and made again is a different node) and stores the answer or clears what was stored; one
   that leaves none asks nothing, because nothing streams from it (an empty remote root names no
   volume, and such a pair walks on every poll). An incremental pass
   asks once per run, only when it has events to place and none is stored, which is how an index
   written before this change learns it without a walk. It is stored in a single-row table
   (`remote_root_node`) in the **final commit**, beside the cursor, **together with the
   `remote_root` path it was learned for** (`config::remote_root_comparison_key`, so `/Drive/A` and
   `Drive/A` are one). It answers only for that path: Settings can re-point `remote_root` over a
   surviving index, and the old folder's node would make every event in the new folder read as
   outside it. It is cleared by `reset-index` and when the remote root is found missing. Unknown
   means nothing is skipped.
10. **No other pair's index is read.** ADR 0005 §8a's second part (consult every pair's index on the
    volume) is not built. With S, a node is outside because *this* pair's tree is fully named, not
    because another pair claims it; the extra reads would couple the pairs, add a second connection
    per pair and make a pass read state that `PairPass` exists to make unreachable.
11. **Resuming a pair queues a pass.** `resume` on a pair that was paused queues the same explicit
    `Sync` job `syncnow` does, so a pair no longer waits for its next timer after a long pause (in
    the log, the pass came 20 seconds after the resume; on a 300-second interval it would be
    minutes). A pair that was not paused queues nothing. An unavailable pair gets its retry at once.

12. **A delta whose every event was outside is an idle pass.** `reconstruct_remote` counts the events
    it dropped as outside; when that is every event in the delta, the map is still the baseline and
    the pass is what an empty delta is: the cursor moves, nothing is scanned, planned or written. A
    reported local change (`pending_changes`), a pending deletion or a forced local scan still make
    it a real pass, exactly as they do for an empty delta. Before this, a poll that saw only other
    folders' events cost a stat-walk of the whole local tree. The node learned on the way is recorded
    all the same, since the commit that normally records it is not reached.

Three log lines are new. `event-driven pass skipped changes outside this folder` (with a count) when
anything was dropped and something else was not, `event-driven pass idle; the only changes were
outside this folder` when everything was, and the fallback reason `cannot tell whether node N is
outside this folder: <what blocked S>` when S failed.

## Soundness

An event dropped under S is never a change inside the pair's tree that nothing later re-derives.
**Claim and argument.** Let an event name node N with parent P. If N is inside the tree, P is the
root or a directory Q of the tree. (The argument is for a pair without include rules; a pair with
them skips nothing.)

First, the event is only deferred if N is not a node the pair holds. The ways N could be one without
the map saying so, and what closes each: the event names **no** parent (it walks, point 4); N is
tracked and names a parent the pair does not hold (it walks, point 6); N is a record held under an
id no event can carry, such as a raw id of an older index or another volume's id (S fails, point 4,
read from the baseline); N is a record the daemon uploaded last pass and that has no id yet (its own
`Created` event names it, and by in-order delivery an event of a node that left comes after that one,
so at the end of the delta the record is named or the event was not about it).

Then:

- P is the root: the root uid is known, so P is a known parent and the event was not deferred.
- P is a directory Q of the tree: under S every directory in the final map has its uid in the
  in-pass map, so if Q is in the final map the event was not deferred. So Q is not in the final map.
  The ways found that can happen while Q is inside the tree, and what closes each. The list is the
  result of the model test below and of two reviews, not a proof that it is complete:

| # | Q is inside the tree but absent from the final map because… | closed by |
| --- | --- | --- |
| 1 | Q is a folder this pair created and has no id yet | Q's own `Created` event is in this delta (placed, so Q is in the map with an id: contradiction), or in an earlier delta (that pass placed Q and `AutoLink` recorded its id in the commit the cursor advanced in: contradiction), or has not arrived (Q is in the map **without** an id, S fails, the pass walks) |
| 2 | Q was created earlier in the same delta | placed before any event under it (in-order delivery), so it is in the in-pass map |
| 3 | Q is excluded by an exclude rule | it is absent from the filtered base and a full walk would not list N either, which is the standard the old skip of an untracked removal already used. An **include** rule is not this row: it leaves the folders on the way to a match without rows although a walk lists them, so such a pair has no root uid for this purpose (point 2) |
| 4 | Q was moved in from elsewhere, or restored from the trash | in this delta: its `Updated` event falls back (point 7). In an earlier delta: that pass fell back and walked, so Q and its subtree are indexed |
| 5 | Q is a descendant of a moved-in directory | no events exist for it; the fallback in 4 walks them |
| 6 | the root itself was renamed, moved or trashed | an event about the root falls back |
| 7 | the index is incomplete for another reason | after a full walk **under rules without include patterns** every inside, non-excluded directory has a row with a composed id (listings are all-or-nothing, #59). `Keep` and `reset-index` latch a full walk, a partial pass holds the cursor, and a legacy raw id or an empty placeholder counts as missing, so S fails. A folder with no row but a record beneath it (an index older than folder rows, or a rule changed since the walk) fails S's last part. A folder with no row *and nothing beneath it* is invisible to the index: **see row 11** |
| 8 | another client creates something inside a folder this pair just created | that folder is case 1 or 2 |
| 9 | Q was removed or moved out earlier in the delta | N left with it. An event that names Q as its parent after that walks (point 6): the index's old path for Q is not asked, because it may belong to another folder by now |
| 10 | Q's name is taken by another node later in the delta | a tracked node's move out is a walk (point 6), so nothing is removed by a stale path |
| 11 | Q has no row and nothing recorded beneath it (it arises only when rules were loosened or the index predates folder rows, and no walk has run since) | **Closed:** a node the pair holds that moves *into* Q names a parent the pair does not hold, so it walks (point 6); before, it was read as having left, and its local copy was deleted. **Not closed:** a node the pair does *not* hold that is created in Q, restored into Q or moved into Q from elsewhere, and an update to a node already inside Q. Those events name a parent the pair has never heard of and are dropped as elsewhere on the volume. `resync` heals it, and the old code healed it by accident, with a walk for any unknown parent |

Each row contradicts "deferred and dropped", ends in a fallback, or is the one named as not closed
(the second half of row 11).

The assumption the whole argument uses, and ADR 0001's cascade already used: **events arrive in the
order things happened**. A foreign event can sit before the event that names the daemon's own folder;
that is why the decision waits for the end of the delta.

Not claimed: descendants of a directory renamed *within* the tree keep their old paths in the
reconstructed map. That was already true and this change neither fixes nor depends on it. A pinning
test is listed under follow-ups.

Also not claimed, and older than this change: the listing a pass reads is the tree as it is *now*,
which can be ahead of the delta. If it places a node at a path whose previous holder is only trashed
by an event after the delta ends, the holder's record is overwritten, the next pass reads its trash as
news about someone else's node, and the holder's children stay in the map until the next walk. The
planner's view is then a stale file, not a lost one, so nothing is deleted in the meantime. The model
test finds it once in about 190,000 two-pass histories (seed 215742) and skips that shape on purpose;
the code before this change overwrote the record the same way.

Also older than this change and equally true without a root uid, found by the model test and left
alone because an include-rule pair is decided as it was: with include rules, a folder that has no row
and holds matching files is trashed or deleted by a single event for its own link, so its files stay
in the map; and the same folder moved into the tree, or restored, brings files nothing describes.
Both are closed by walking whenever such an event arrives and a record sits in a folder with no row;
that costs include-rule pairs a walk per foreign deletion, so it is not done here.

## The model test

`src/reconstruct/model_tests.rs` checks the soundness claim against a model instead of an argument.
It keeps the real remote tree, generates a random history (create, modify, rename, move in, out and
within, trash, restore, delete, folders the daemon made itself whose records have no id), turns it
into the event delta, hands `reconstruct_remote` the baseline from before it and a resolver that
lists the tree as it is at the end, and compares a `Complete` map with the tree a filtered full walk
would list. A fallback is always acceptable; a complete and different map is a change the pass would
lose. It runs with no rules, an exclude rule, three include rule sets, and an index with some folder
rows missing, and once more with the history cut into two passes. Four further shapes came with the
second review: names that another node takes at once, trashes undone at once and nodes deleted
without a trash; events that lose their parent id; records held under the ids of an older index; and
folders with no row and nothing beneath them. Seeded and deterministic: a few seconds by default,
more with `RECONSTRUCT_FUZZ_SCENARIOS=300000 cargo test --lib reconstruct::model_tests --
--nocapture` (`RECONSTRUCT_FUZZ_FROM=<seed>` starts at a seed, to replay one). At the review of the
first version, it found the two defects fixed then (include rules, and a move out applied by a stale
path): 281 wrong maps in 60,000 scenarios without rules, thousands with include rules. Against the
first fixes the new shapes found 338 wrong maps in 10,000 histories with lost parent ids, 247 in
20,000 with older ids and 28 in 20,000 with folders that have no row; it reports none in 300,000
scenarios per configuration now. What the generator leaves out is listed at the top of the file.

## Consequences

- **Cost.** A foreign event costs no `proton-drive` call and one pass over the pair's map (hash
  lookups) when something was deferred, and a delta of only foreign events costs no local scan
  either (point 12). Before: one root listing and a full walk, then a stat-walk of the local tree. A
  bootstrap pays up to two more listings to learn the root uid, except for a pair with include rules.
- **The cursor policy is unchanged.** A skipped event is "applied" by being outside. The five causes
  that hold the cursor (withheld delete, vanished node, failed item, skipped destructive row, a
  folder that vanished) still do, whatever else the pass skipped. #30, the bootstrap's pre-walk
  anchoring (#294, #303), selective sync and the plan pass (which still full-walks and learns
  nothing) are untouched.
- **What still walks.** A fetch error or a server refresh; a `Created` node absent from its known
  parent's listing (#30); an event about the pair's own root; a directory moved in or restored; a
  duplicate id in the baseline; a tree with a directory that has no composed id (the window between
  the daemon making a folder and its own event arriving); a record in a folder with no row; a
  baseline record under an older id; an event that names no parent; **a node the pair holds that
  moves out of the tree** (one walk per move out: the same event is a move into a folder the pair
  has no row for, and the pair cannot tell them apart); an event naming a parent that an earlier
  event of the same delta took out; a pair that cannot learn its root uid; **every foreign event of
  a pair with include rules**, as before.
- **A new invariant** for CLAUDE.md: an event the pair cannot place is skipped only when the pair's
  tree is fully named and fully held (point 4), and never for a pair with include rules.
- **What to watch on the live log** after the upgrade: zero "not under any indexed parent" lines for
  foreign nodes (except from a pair with include rules); `warm start completed` for every pair at
  every boot, a GUI add included; the skip or idle line on the small pairs whenever Documents
  uploads, and the reverse; and the rate of `cannot tell whether node … is outside` (each one on
  Documents is a 28-minute walk).
- **A residual, contrived:** the root uid names a node and `remote_root` names a path. If an
  ancestor of the folder is renamed and someone makes a different folder at the old path before the
  pair notices, no event is about the old root, so the pair keeps the old uid until its next walk and
  skips events inside the new folder. The window is one poll plus that person's speed; a walk
  (restart, `resync`, the scheduled sweep) re-asks. A check of the uid before dropping anything
  would close it at the price of one listing per pass that drops events; not built.
- **What the include-rule exclusion costs:** none of this helps a pair with include rules, a shape
  the examples configuration documents. Such a pair keeps walking for every foreign event, as before.
  Giving the folders on the way to a match index rows would change the index for every such pair and
  is not part of this.
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
2. opt-in write (`PROTON_SYNC_LIVE_WRITE=1`): upload a probe at the root, read its `Created` event
   and assert `node_uid(volume, ParentLinkID)` is the root uid; rename the probe within the root,
   read the `Updated` event that follows and assert the same of it (a pair that already holds the
   node meets an edit or a rename far more often than a creation, and an event with a missing or
   different parent would be dropped as foreign); and report whether the root itself got an event for
   either. The probe is trashed at the end and also when an assertion fails (a drop guard). It is
   never restored from the trash, and the trash is not emptied. The run prints no ids.

```bash
PROTON_SYNC_EVENTS_VOLUME=<volumeId> \
PROTON_SYNC_LIVE_REMOTE_ROOT=/my-files/Videos \
PROTON_SYNC_LIVE_WRITE=1 \
  cargo test --test events_scope_live -- --ignored --nocapture
```

The skip is relied on once this has passed on a real account. If the first check fails a pair never
learns its root and nothing is skipped (the old behaviour); if the second fails, do not ship.

## Follow-ups

- Backfill ids by name (above), if the S-failure rate on the live log is worth it.
- An empty pair (a remote root with no children) cannot name its volume and walks on every poll; the
  root uid names the volume and would end that. Its own issue.
- A test that pins the stale paths of a renamed directory's descendants, before anyone touches it.
