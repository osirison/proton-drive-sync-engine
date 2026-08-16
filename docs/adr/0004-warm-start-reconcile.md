# ADR 0004 — Warm-start reconcile (skip the full remote walk on boot)

- **Status:** Accepted
- **Date:** 2026-08-04
- **Relations:** relaxes the "Startup snapshots first" invariant established alongside ADR 0001
  (remote change detection via volume events); reuses that ADR's persisted event cursor and
  `reconstruct_remote(base ⊕ delta)` machinery.

## Context

Every process restart paid for a **full-tree remote walk** — `bootstrap_reconcile` lists every
remote folder with a separate, sequential `proton-drive filesystem list` subprocess (parallelism
is impossible; the CLI's shared SQLite is not concurrency-safe, #23). On a large Drive that is
minutes of work on every reboot, and a daemon launched by a systemd user service restarts often.

The cost was **not** the local side: the scan is already an rsync-style `(mtime, size)`
quick-check that reuses each unchanged file's stored SHA-1, so an unchanged tree costs `stat`
calls, not re-hashing. And the *remote* side already had everything needed to go incremental on a
restart — a durably-persisted per-volume event cursor and `reconstruct_remote`, which overlays the
event delta onto the last-known remote view at O(changes) cost. Steady-state passes already used
it.

The only reason the first pass after boot still full-walked was the **local** side, encoded as the
"Startup snapshots first" invariant: `notify` never replays pre-existing files, so a fresh
process's `pending_changes` is empty. A naive incremental first pass would find an empty remote
delta *and* an empty `pending_changes`, take the idle fast-path, skip the local scan entirely, and
**strand a file edited while the daemon was down** until the next full walk. The bootstrap's full
local scan was the safety net.

That safety net does not actually require the *remote* full walk — only the *local* one.

## Decision

Introduce a **warm start**: the first reconcile after boot runs the event-driven incremental path
(cursor replay + `reconstruct_remote`, no O(folders) walk) **but forces the full local stat-walk**
even when the delta is empty. The forced scan preserves the offline-edit safety property; dropping
the remote walk is the whole win. Reboot cost falls from O(all remote folders) to O(remote changes
since the last sync).

A warm start is attempted on the first pass only when **all** hold, else the pass bootstraps
(today's behavior, byte-for-byte):

1. `warm_start` enabled (default on) with event-driven detection and a usable event source.
2. A volume id and a stored cursor exist (there is something to replay from). The volume comes from
   a baseline composed `proton_id`, or — when the index carries none, e.g. an all-Proton-native
   remote — from the sole stored cursor row, whose `scope_id` *is* the volume (#32). Either way a
   real cursor must exist, so the gate cannot engage without one.
3. The cursor is **fresh** — `now - updated_at ≤ warm_start_max_cursor_age` (default 7 days).
4. The across-restart floor is not due — `warm_starts_since_full_walk < warm_start_full_walk_every`
   (default 30).

Any doubt during the pass (no cursor, server refresh, events error, unresolvable node, a `Created`
node its parent listing has not caught up with yet) falls back
to a bootstrap exactly as a steady-state incremental pass does.

Two safety bounds, both new surface:

- **Cursor-age gate** (`warm_start_max_cursor_age`, default 7 days; `0` disables). Guards the one
  thing we cannot verify from the client: that Proton signals a *refresh* — rather than silently
  truncating the delta — for a cursor past its event-retention window. A boot after long downtime
  full-walks instead of trusting a possibly-expired cursor. Note this measures the last cursor
  *advance*, not the last successful pass (the idle path only advances the cursor when the latest
  event id changed), so a totally-idle volume left past the window then rebooted takes an
  unnecessary — but always safe — full walk.

- **Every-N-warm-starts floor** (`warm_start_full_walk_every`, default 30; `0` disables via the
  same `u64::MAX` sentinel as `events_full_scan_every`). A self-healing full walk every N warm
  starts, so drift the event stream can't correct (a missed event, server-side compaction, index
  drift) is bounded. The counter is **persisted** across restarts in a `warm_start_state` table —
  distinct from the in-run `incremental_passes_since_full_scan`, which drives the *in-run* periodic
  resync (`events_full_scan_every`, off by default). This floor heals across **reboots**; a
  long-running process that never restarts still relies on `events_full_scan_every` for an in-run
  full walk.

**Forcing a full walk on demand.** `proton-syncd --full-walk` forces this boot's first pass to
bootstrap (a process-lifetime flag, sticky across a failed first pass so the requested walk still
happens on retry). `proton-sync resync` latches a `force_full_walk` flag consumed by the next pass
(first or steady-state), for self-healing suspected drift while running.

**Failure stickiness.** `is_first_reconcile` clears **only on success**. A failed first pass
retries as a first pass, so it never drops into the steady-state idle fast-path (which would skip
the local scan) before the startup local scan has succeeded once.

**Mid-life event-source reacquisition** (keyring unlocked after a degraded boot) still forces a
full walk by reseeding the in-run resync floor — but **only on a mid-life pass**, not the first
one. Steady-state incremental has no cursor-age gate, so replaying a stale cross-process cursor
there would miss changes; on the first pass, `first_reconcile`'s own cursor-age gate already
covers that, so the reseed is skipped (also avoiding an overflow when a warm start then increments
the `u64::MAX`-seeded counter).

## Consequences

- A restart with a recent, valid cursor now converges in O(changes) instead of re-walking the
  whole remote tree. The offline-edit guarantee is unchanged: the forced local scan still catches
  it, and a stale cursor / disabled feature / any fallback still full-walks.
- Warm start is on by default. Opt out with `--no-warm-start` / `warm_start = false` for the exact
  pre-change behavior. `--full-walk` and `proton-sync resync` force a walk when wanted.
- New persisted state (`warm_start_state`) and a new IPC command (`resync`). An older daemon
  rejects `resync` as an unknown command (the client is simply newer).
- **A restart is no longer a full walk, so any cursor anchored *after* a walk is permanent.**
  Before this ADR, "the next restart re-snapshots" was the standing repair for a cursor that
  over-claimed. It is gone: a restart replays the persisted cursor, `events_full_scan_every`
  defaults to off, and `warm_start_full_walk_every` is 30 restarts away. That is why the bootstrap
  reads its cursor *before* the walk (#294) and why the last arm that did not — the first-ever
  bootstrap, which had nothing stored to name its volume with — now names it from a targeted
  listing of the remote root instead of from the finished snapshot (#303).

## Alternatives considered

- **Parallelize the remote walk.** Blocked by the CLI's non-concurrency-safe SQLite (#23).
- **A total/Merkle tree hash** to detect "did anything change." Subsumed by the existing per-file
  `(mtime, size, sha1)` index (which identifies *which* file changed, and cannot be computed
  without walking the tree anyway); and Proton exposes no per-folder hash/dirty-flag, so it offers
  nothing on the remote side either — the event cursor is the only sub-full-walk remote lever.
- **Trust the cursor unconditionally on restart.** Rejected: unverifiable server behavior on an
  expired cursor could silently truncate the delta. The cursor-age gate makes correctness
  independent of it.
