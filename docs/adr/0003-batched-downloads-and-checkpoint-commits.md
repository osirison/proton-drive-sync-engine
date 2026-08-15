# ADR 0003 — Batched downloads + per-action checkpoint commits

- **Status:** Accepted (amended in part by #136 — a failed action no longer ends the pass; see the
  amendment note below)
- **Date:** 2026-07-31
- **Relations:** amends ADR 0002 in part (approval consumption moves from the end-of-pass
  transaction into the executing delete's checkpoint transaction)

## Context

A bulk download (bootstrap against a populated remote, or a large remote drop) spent hours in
two compounding pathologies:

1. **One subprocess per file.** Every planned `Download` shelled its own `proton-drive
   filesystem download` invocation — process spawn, CLI startup (session/SQLite load), one
   network round trip of overhead per file — even though the CLI accepts **multiple remote
   `path...` arguments per invocation**. Parallelism is not an option (the CLI's shared SQLite
   session store is not concurrency-safe, #23), so amortizing the per-invocation cost is the
   only lever.
2. **All-or-nothing commit.** `execute_plan_and_commit` executed the whole plan and committed
   every `IndexMutation` in one transaction at the end. Any failure — one `You need to login
   first` blip on file 4,990 of 5,000, a timeout, a daemon shutdown — discarded the index
   record of *everything* the pass had already done. Files already downloaded stayed on disk
   and were re-adopted by hash (`AutoLink`) next pass, so no bytes were re-transferred, but the
   pass-level work (including the O(folders) remote walk that precedes execution) restarted
   from zero, and completed uploads/moves/deletes were re-derived from scratch. Observed live
   on 2026-07-29: a bootstrap of 5,399 downloads failed every ~29-minute pass for hours (#134)
   before a config fix let one pass finally complete.

## Decision

1. **Batch downloads through the CLI's multi-path form.** `ProtonClient` gains
   `download_many(&[DownloadRequest]) -> Vec<AppResult<()>>` with a default implementation that
   loops over `download` (test doubles unaffected). The concrete client executes one
   `filesystem download <remote>... <scratchDir>` invocation when the batch preconditions hold
   — ≥ 2 files, one shared destination directory, destination basenames exactly the remote
   basenames, no duplicates — and falls back to per-file downloads otherwise. Staging keeps the
   single-download contract: one private scratch directory, one rename per file into place,
   nothing else touched. The per-invocation timeout scales with batch size (every file keeps
   its configured per-command budget).

2. **Per-file results, digest-verified salvage.** A batch reports per-file outcomes. On a
   successful CLI exit, a file missing from staging (e.g. silently skipped) fails only that
   file. On a failed exit (error/timeout/cancel), files whose staged content hashes to the
   remote's claimed SHA-1 are still moved into place and reported `Ok` — a partial file can
   never match its digest, and a file without a claimed digest is never salvaged.

3. **The executor chunks consecutive downloads.** A run of adjacent planned `Download` actions
   is grouped by destination directory (plan order preserved) and chunked to
   `download_batch_size` (flag `--download-batch-size N`, file `download_batch_size = N`,
   default **25**, `1` disables batching byte-for-byte). Only adjacent downloads regroup, so
   ordering relative to every non-download action is unchanged.

4. **Checkpoint commits.** Every action that performed a side effect — and every download chunk
   — commits its accumulated `IndexMutation`s immediately in its own transaction
   (`commit_checkpoint`). Index-only mutations (`AutoLink`, `Purge`) accumulate into the next
   checkpoint or the final commit, so an adoption-heavy pass stays a few large transactions.
   Delete-approval consumptions ride in the same checkpoint as the deletion's index purge.
   The direction of the old invariant is untouched: **an index write never precedes its side
   effect**; what changed is that *completed* work is now durable immediately.

5. **The event cursor stays pass-scoped.** The cursor asserts "every remote change up to this
   event has been applied" — true only when the whole plan landed. It commits exclusively in
   the final transaction of a fully-successful pass, never in a checkpoint, and continues to be
   withheld whenever a delete-approval gate withheld an action.

## Consequences

- A failure or shutdown late in a multi-hour pass loses at most the current chunk's unfinished
  files plus unexecuted actions; everything else is recorded and never re-derived. Retry passes
  shrink monotonically.
- Bulk downloads spawn ~N/25 subprocesses instead of N, removing per-file CLI startup and
  session-load overhead. The remote listing walk is unchanged (O(folders), #18 tracks the real
  fix).
- A failed pass can now leave the index *partially advanced* relative to the plan — by design.
  Each committed mutation reflects a completed side effect, so the index remains a truthful
  baseline; the next pass replans only the remainder. Tests that asserted the old "earlier
  successes are not recorded" behavior were inverted deliberately
  (`reconcile_checkpoints_*` in `src/daemon.rs`).
- Conflict-sidecar downloads never batch (their destination basename differs from the remote's
  by design), and non-downloadable files never reach the executor as `Download` actions.

## Amendment 2026-08-15 (#136) — a failed action no longer ends the pass

This ADR left execution fail-fast: a failed action (or a failed file in a chunk) aborted the pass
after its checkpoints, so "unexecuted actions" above meant *everything ordered after the failure*.
That was the remaining half of the same incident this ADR cites — one poisoned file starved every
action behind it, on every pass, indefinitely.

The executor now rolls back the failed action's queued mutations, records it as a `FailedItem`, and
continues with its successors; the pass returns `PassOutcome::Partial { failed }`, a third outcome
enumerated at every branch on pass outcome. Nothing above is weakened: an index write still never
precedes its side effect, the failed action is still never recorded (now structurally, by
truncating to the pre-action mutation length), and the event cursor still advances only in the
final commit of a pass that landed everything — item failures join withheld deletes and vanished
nodes as a cause to hold it. Failed paths are re-queued into `pending_changes` so the next
event-driven pass cannot idle-skip them. A shutdown and a wholly-unproductive pass (20 consecutive
failures with no success) stay pass-fatal. See the partial-pass invariant in `CLAUDE.md`.
