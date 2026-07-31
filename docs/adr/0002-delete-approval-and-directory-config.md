# ADR 0002 — Directional delete-approval guard + per-directory config

- **Status:** Accepted (amended in part by ADR 0003 — approval consumption now rides in the
  executing delete's checkpoint transaction, not the end-of-pass transaction)
- **Date:** 2026-07-25

## Context

The engine executes two data-losing actions during reconcile: `RemoteDelete` (propagate a local
deletion by trashing the file on Proton Drive) and `LocalDelete` (propagate a remote deletion by
**permanently** removing the file from local disk). The only prior guard is planner-level: a
deletion propagates only when the other side is unchanged (the delete/edit safeguard). There was no
way for a user to require explicit consent before a deletion actually runs, and no way to configure
policy differently for different folders.

We want: a guard that (1) is **on by default**, (2) lets the user **approve** withheld deletions
per item, (3) can be **turned off per directory and inherited by subdirectories**, taking effect
immediately, and (4) rides on a config layer built to be **extensible**.

## Decision

1. **Directional guard, two independent booleans** (`delete_approval.remote`, `delete_approval.local`),
   both defaulting to `true`. They gate exactly `RemoteDelete` and `LocalDelete`. `Purge` (index-only
   cleanup, both sides already gone) destroys no data and is never gated.

2. **Execution-time gate on the `Daemon`, not the planner.** The planner stays pure; the gate filters
   the plan in `execute_plan_and_commit` before the loop, mirroring where `paused` lives. A withheld
   deletion skips both its side effect and its index mutation, so it re-plans (still pending) next
   pass — the same shape as a mid-plan failure, preserving commit-after-side-effects.

3. **Per-item approvals persisted, pending recomputed from ground truth.** A `delete_approvals` table
   stores standing approvals keyed by `(path, direction)` and pinned to a `fingerprint` (a file's
   last-synced SHA-1, or a directory's `proton_id`). A guarded deletion executes only if a matching
   approval exists, which is then consumed in the post-success transaction. The *pending* set is
   recomputed each pass from the current plan, so it never goes stale; if the entity changes, the
   fingerprint no longer matches and the old approval is inert.

4. **Hierarchical, machine-local `.proton-sync.toml`.** A settings file in any directory applies to
   that directory and everything beneath it; the nearest file wins, unset options inherit, and the
   daemon-wide default is the bottom of the chain (`src/dirconfig.rs`). The file is ignored by scan,
   plan, base-index filter, and watcher (`index::should_ignore_path`), so it is never synced. A
   malformed/unreadable file is ignored and the guard stays on (fail-safe). It is re-read every
   reconcile, so edits take effect on the next pass (≤ events-poll interval, or immediately via
   `syncnow`).

5. **Cursor hold when withholding (event-driven correctness).** If a pass withholds any deletion, the
   volume-event cursor is **not** advanced. A withheld `LocalDelete` originates from a remote-delete
   event; if the cursor advanced past it, `reconstruct_remote` (which overlays only the new delta onto
   the surviving baseline) would stop re-deriving it and the pending item would vanish until the next
   full-tree resync. Holding the cursor keeps every pending deletion re-derived from ground truth, so
   the queue stays fresh and an approval applies promptly. The cursor resumes advancing the first pass
   with nothing withheld.

## Options considered

| Option | Verdict | Why |
| ------ | ------- | --- |
| Guard + toggle only (no per-item queue) | Rejected | Coarser than asked: allowing one delete would require disabling the guard for a whole subtree. |
| Per-item queue only (no per-directory toggle) | Rejected | No way to opt a trusted subtree out; every delete would need a manual approval forever. |
| **Both: directional guard + per-item approvals + per-directory files** | **Chosen** | Matches the request: default-safe, per-item consent, and per-subtree opt-out with inheritance. |
| Re-inject withheld deletes from a persisted mark (instead of holding the cursor) | Rejected for v1 | Re-injecting a stored action risks acting on stale state; re-deriving from ground truth each pass is safer. |
| Sync the settings file across machines | Rejected | A safety policy must not be silently disable-able by a remote-authored file; machine-local matches `.sync`. |

## Consequences

- **Positive:** no deletion runs without consent by default; policy is per-subtree and inheritable;
  the config layer generalizes to future per-directory settings (add an `Option` field to
  `DirectorySettings` + a resolved field to `EffectiveSettings`).
- **Negative / risks:** a deletion left pending indefinitely pins the event cursor and grows the
  replay window; the periodic full-tree resync keeps correctness regardless, and the window collapses
  once the last pending deletion is approved or its subtree is opted out. Approvals are consumed only
  after the delete applies, so a deletion that fails mid-plan retries next pass (consistent with the
  no-auto-retry convention for side effects, since the whole plan re-derives).

## Invariants preserved

Commit-after-side-effects, path-safety at boundaries (the guard only acts on already-planned,
already-validated paths, and `approve` only targets currently-pending items), selective-sync
everywhere, and non-destructive-on-unknown-digests (the planner is untouched).
