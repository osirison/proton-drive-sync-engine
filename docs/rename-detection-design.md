---
title: Rename Detection Design
description: Design notes for adding safe rename detection to the sync planner
---

## Rename Detection Design

Rename detection should reduce unnecessary delete and upload work when a file moves without content changes. It must never infer destructive remote changes from ambiguous state.

## Current Behavior

The planner keys local, remote, and base index records by relative path. When a synced local file moves from `old.txt` to `new.txt`, the current planner sees one missing path and one new path. That becomes a remote delete plus an upload, even when the SHA-1 hash is unchanged.

## Design Constraints

* Rename inference must require strong evidence: matching SHA-1, size, and a single unambiguous source and destination.
* Remote entries without usable SHA-1 digests cannot participate in rename inference.
* Cross-directory renames must remain inside the configured sync root and respect include and exclude filters.
* Conflicts take precedence over rename inference when either side also changed content.
* The first implementation should support local-only renames before remote rename operations are added.

## Index Changes

The current `file_index` table stores path, size, mtime, hash, Proton ID, and status. Local rename detection can start without a schema migration by grouping base records and current local files by SHA-1 plus file size. Remote rename support will likely need either a Proton move command or a way to update indexed paths after detecting a remote path move.

Future schema changes should consider adding:

* `last_seen_local_path` for diagnostic history
* `last_seen_remote_path` when Proton exposes reliable remote path moves
* `content_key` derived from size plus SHA-1 for faster candidate lookup

## Planner Algorithm

1. Compute ordinary path-based local and remote deltas.
2. Collect local rename candidates where one base path is missing locally and one new local path has the same SHA-1 and size.
3. Reject candidate groups with more than one missing source or more than one new destination.
4. Replace the source path remote-delete and destination upload actions with one local rename action when the remote side is unchanged.
5. Keep existing conflict behavior when the remote side changed or has unknown hash state.

## Execution Model

The first safe execution path should avoid remote mutation. For local-only renames, update the index path from the old relative path to the new relative path after validating that the remote record still points at the same Proton ID and hash. A later remote rename feature can add a Proton client method only after the CLI command contract is verified.

## Test Plan

* Unit-test single local rename planning with unchanged remote state.
* Unit-test ambiguous same-hash candidates and confirm they fall back to current behavior.
* Unit-test filtered paths so excluded rename candidates are ignored.
* Add a daemon reconciliation test that updates the index only after the rename action succeeds.
* Add a live E2E rename scenario only after the Proton CLI move or rename behavior is verified in a disposable remote folder.
