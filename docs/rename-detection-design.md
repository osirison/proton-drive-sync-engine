---
title: Rename Detection Design
description: Design notes for adding safe rename detection to the sync planner
---

## Rename Detection Design

Rename detection should reduce unnecessary delete and upload work when a file moves without content changes. It must never infer destructive remote changes from ambiguous state.

## Current Behavior

The planner keys local, remote, and base index records by relative path. It now has a conservative path-transition pass for file renames:

* A remote-only rename can converge locally when the old local file is unchanged, the remote destination is absent locally, and the Proton ID plus SHA-1 evidence identify one destination.
* A local-only rename executes as a real remote rename/move via the Proton CLI (`filesystem rename` / `filesystem move`, including the combined move-then-rename case), then purges the old index row and upserts the new path once the CLI call succeeds.
* Ambiguous same-hash candidates are not inferred as renames.

## Design Constraints

* Rename inference must require strong evidence: matching SHA-1, size, and a single unambiguous source and destination.
* Remote entries without usable SHA-1 digests cannot participate in rename inference.
* Cross-directory renames must remain inside the configured sync root and respect include and exclude filters.
* Conflicts take precedence over rename inference when either side also changed content.
* The first implementation supports remote-only rename convergence as a local filesystem move because it requires no Proton mutation.
* Local-to-remote rename execution uses the verified Proton CLI move/rename command contract (`ProtonClient::rename_or_move`); it never invents remote mutation from ambiguous evidence, and it only executes once the same strong-evidence matching used for remote-only renames identifies a unique source and destination.

## Index Changes

The `file_index` table stores path, entity kind, size, mtime, optional hash, Proton ID, and status. Rename inference currently groups file records and current entities by SHA-1 plus Proton ID evidence. Directory records do not participate because they have no content hash.

Future schema changes should consider adding:

* `last_seen_local_path` for diagnostic history
* `last_seen_remote_path` when Proton exposes reliable remote path moves
* `content_key` derived from size plus SHA-1 for faster candidate lookup

## Planner Algorithm

1. Scan local entities, remote entities, and base records by relative path.
2. Before ordinary path-based planning, look for base file paths that moved on exactly one side.
3. Plan `move_local` when the remote destination has the same Proton ID and SHA-1 as the base, the local source is unchanged, and the destination is unique.
4. Plan `MoveRemote` (via `plan_local_rename_as_remote_move`) when the local destination is unique and remote mutation is required to converge.
5. Reject ambiguous candidate groups and fall back to normal path-based planning.
6. Keep existing conflict behavior when either side changed content or exposes unknown hash state.

## Execution Model

The first safe execution path avoids remote mutation. Remote-only renames execute as local filesystem moves, then purge the old index row and upsert the new path after the local move succeeds. Local-to-remote renames execute via `ProtonClient::rename_or_move`, which issues `filesystem rename` (parent unchanged), `filesystem move` (name unchanged), or a move followed by a rename (both parent and name changed); the daemon purges the old index row and upserts the new path only after the CLI call succeeds, preserving the same no-partial-commit invariant used elsewhere.

Directory deletes propagate recursively only when every descendant under the directory resolves to a deletion-consistent outcome (the whole subtree is missing on one side with no diverging descendant); ambiguous subtrees keep the non-destructive recreate fallback instead of guessing.

## Test Plan

* Unit-test remote-only rename planning with unchanged local state and stable Proton ID evidence.
* Unit-test local-only rename planning and execution (rename-only, move-only, and combined move+rename) against a fake Proton CLI.
* Unit-test ambiguous same-hash candidates and confirm they fall back to current behavior.
* Unit-test filtered paths so excluded rename candidates are ignored.
* Add a daemon reconciliation test that updates the index only after the local move, or the remote `rename_or_move` call, succeeds.
* A live E2E scenario (`tests/proton_live.rs`, `mutating_live_e2e_exercises_upload_download_rename_move_delete`) exercises rename and move against a real, disposable remote folder; it is `#[ignore]`-gated and requires explicit opt-in env vars (see `docs/live-e2e-test-plan.md`).
