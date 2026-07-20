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
* A directory renamed or moved on the remote side can converge locally when the old local directory is still present, the remote destination is absent locally, and its Proton ID uniquely matches the base index record's `proton_id`. Every descendant path under the directory is rewritten to match, rather than being independently replanned as an upload, download, or delete.

## Design Constraints

* Rename inference must require strong evidence: matching SHA-1, size, and a single unambiguous source and destination.
* Remote entries without usable SHA-1 digests cannot participate in rename inference.
* Cross-directory renames must remain inside the configured sync root and respect include and exclude filters.
* Conflicts take precedence over rename inference when either side also changed content.
* The first implementation supports remote-only rename convergence as a local filesystem move because it requires no Proton mutation.
* Local-to-remote rename execution uses the verified Proton CLI move/rename command contract (`ProtonClient::rename_or_move`); it never invents remote mutation from ambiguous evidence, and it only executes once the same strong-evidence matching used for remote-only renames identifies a unique source and destination.
* Directory rename and move detection is intentionally one-directional: only remote-to-local convergence is implemented. See "Directory Rename and Move Detection" for the rationale.

## Index Changes

The `file_index` table stores path, entity kind, size, mtime, optional hash, Proton ID, and status. Rename inference for files groups file records and current entities by SHA-1 plus Proton ID evidence. Directory records have no content hash, so directory rename inference relies solely on `proton_id` equality; a directory record must already carry a `proton_id` for rename detection to consider it.

A directory's `proton_id` is backfilled once it becomes known: when an already-tracked directory's base record has no `proton_id` (or a stale one) and the current remote listing exposes one, the planner emits `SyncAction::AutoLink` to update the index, without re-emitting once the record already matches. This backfill is a prerequisite for directory rename detection, since a directory created locally-first has no `proton_id` until a reconcile observes it on the remote side.

Future schema changes should consider adding:

* `last_seen_local_path` for diagnostic history
* `last_seen_remote_path` when Proton exposes reliable remote path moves
* `content_key` derived from size plus SHA-1 for faster candidate lookup

## Planner Algorithm

1. Scan local entities, remote entities, and base records by relative path.
2. Before ordinary path-based planning, look for base file paths that moved on exactly one side.
3. Plan `move_local` when the remote destination has the same Proton ID and SHA-1 as the base, the local source is unchanged, and the destination is unique.
4. Plan `MoveRemote` (via `plan_local_rename_as_remote_move`) when the local destination is unique and remote mutation is required to converge.
5. For a base directory path not yet claimed by a file-move match, plan `move_local` (via `plan_remote_directory_move`) when the local directory is still present, the remote path is absent, and exactly one remote directory exposes the same `proton_id` as the base record.
6. When a directory move is planned, suppress ordinary planning for every strict descendant of both the old and new directory paths, since the descendants move with their parent rather than being independently replanned.
7. Reject ambiguous candidate groups and fall back to normal path-based planning.
8. Keep existing conflict behavior when either side changed content or exposes unknown hash state.

## Execution Model

The first safe execution path avoids remote mutation. Remote-only renames execute as local filesystem moves, then purge the old index row and upsert the new path after the local move succeeds. Local-to-remote renames execute via `ProtonClient::rename_or_move`, which issues `filesystem rename` (parent unchanged), `filesystem move` (name unchanged), or a move followed by a rename (both parent and name changed); the daemon purges the old index row and upserts the new path only after the CLI call succeeds, preserving the same no-partial-commit invariant used elsewhere.

Directory deletes propagate recursively only when every descendant under the directory resolves to a deletion-consistent outcome (the whole subtree is missing on one side with no diverging descendant); ambiguous subtrees keep the non-destructive recreate fallback instead of guessing.

A directory move executes as a single top-level operation, `fs::rename` for the remote-to-local case, rather than moving descendants individually. After the top-level move succeeds, every descendant's index row is rewritten in place: the old descendant path is purged and a new record is upserted at the corresponding new path, carrying forward the descendant's existing size, mtime, hash, Proton ID, and sync status unchanged. All of these mutations, the directory's own purge and upsert plus every descendant's purge and upsert, are queued together and only committed in the single transaction at the end of reconciliation, so a later unrelated action failing in the same pass leaves the whole set uncommitted rather than applying it partially.

## Directory Rename and Move Detection

Directory rename and move detection is one-directional by design: only a directory renamed or moved on the remote side converges locally. Renaming or moving a directory locally does not currently converge remotely; it recreates the directory remotely instead, following the existing non-destructive fallback.

This asymmetry follows from where each side's identity evidence comes from. A file's identity is verifiable on both sides through its SHA-1 content hash, so a local file rename can be matched against a unique remote candidate by hash and Proton ID. A directory has no content hash. An empty, or even a populated, local directory carries no signal that distinguishes "this is the renamed version of a previously tracked directory" from "this is coincidentally a new directory," short of comparing entire descendant subtrees (relative paths and hashes) between the old and new locations. That comparison is a substantially more complex and riskier algorithm than the single-entity identity matching used everywhere else in the planner, and a wrong match would misattribute an entire subtree's history rather than a single file.

The remote side does not share this limitation: Proton Drive assigns a persistent id to each folder that survives renames, exactly like a file's remote id. Remote-to-local directory move detection reuses the same uniqueness-or-nothing matching used for file renames, keyed on `proton_id` equality instead of a content hash, so it carries the same safety guarantee: it only fires when exactly one remote directory exposes the base record's `proton_id`, and it falls back to the existing non-destructive recreate behavior for zero or multiple candidates, or when the base record has no `proton_id` yet.

Local-to-remote directory rename detection remains a documented gap rather than an attempted heuristic. A future implementation would need a subtree-shape comparison design, matching a "new" local directory to a "missing" base directory by comparing descendant relative paths and hashes as a set, which is out of scope until that design work happens.

## Test Plan

* Unit-test remote-only rename planning with unchanged local state and stable Proton ID evidence.
* Unit-test local-only rename planning and execution (rename-only, move-only, and combined move+rename) against a fake Proton CLI.
* Unit-test ambiguous same-hash candidates and confirm they fall back to current behavior.
* Unit-test filtered paths so excluded rename candidates are ignored.
* Add a daemon reconciliation test that updates the index only after the local move, or the remote `rename_or_move` call, succeeds.
* Unit-test directory Proton ID backfill: an unlinked directory gets backfilled once a matching remote id appears, an already-linked directory does not re-emit, and backfilling one directory's id never touches a different directory's record.
* Unit-test remote-to-local directory move planning: a clean rename in place, a move to a new parent with a nested file and a nested subdirectory rewritten and suppressed correctly, ambiguous id candidates falling back to recreate, and a directory without a `proton_id` falling back to recreate.
* Add a daemon reconciliation test that moves a local directory and rewrites every descendant index row, and a test confirming a later unrelated action failing in the same reconcile leaves an already-executed directory move's index mutations uncommitted.
* A live E2E scenario (`tests/proton_live.rs`, `mutating_live_e2e_exercises_upload_download_rename_move_delete`) exercises rename and move against a real, disposable remote folder; it is `#[ignore]`-gated and requires explicit opt-in env vars (see `docs/live-e2e-test-plan.md`).
