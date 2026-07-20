---
title: Live End-to-End Test Plan
description: Safety-gated plan for opt-in Proton Drive live sync validation
---

## Live End-to-End Test Plan

Live end-to-end tests validate the sync daemon against an authenticated Proton Drive account. They must be opt-in, isolated to disposable folders, and safe to skip in normal CI.

## Goals

* Validate list, upload, download, and delete behavior through the real `proton-drive` CLI
* Exercise daemon reconciliation rather than only the low-level Proton client wrapper
* Keep normal contributor test runs free from network calls and remote mutations
* Make destructive test scope explicit before any file is created or deleted remotely

## Required Safety Gates

Live E2E tests should run only when all of these conditions are true:

* `PROTON_SYNC_LIVE_E2E=1` is set.
* `PROTON_SYNC_LIVE_REMOTE_ROOT` points to a disposable remote folder dedicated to this test suite.
* The remote root basename starts with `proton-sync-e2e-`.
* The test creates a unique child folder for each run and deletes only files whose IDs were discovered from that child folder.
* The test writes local data under a temporary directory created by the test process.

## Environment Variables

| Variable | Required | Purpose |
| -------- | -------- | ------- |
| `PROTON_SYNC_LIVE_E2E` | Yes | Must be `1` to enable mutating live tests |
| `PROTON_SYNC_LIVE_REMOTE_ROOT` | Yes | Disposable Proton Drive root for live tests |
| `PROTON_SYNC_LIVE_CLI` | No | Custom path to the `proton-drive` executable |
| `PROTON_SYNC_LIVE_TIMEOUT_SECS` | No | Command timeout for live Proton operations |
| `PROTON_SYNC_LIVE_LIST_ATTEMPTS` | No | Retry attempts for read-only listings |

## Test Matrix

| Scenario | Initial State | Expected Result |
| -------- | ------------- | --------------- |
| Read-only smoke | Remote root contains zero or more files | Listing succeeds and parsed file IDs are non-empty |
| Local upload | New local file under the test temp directory | Remote listing shows the uploaded file under the unique run folder |
| Remote download | New remote file under the unique run folder | Daemon downloads the file to the local root and indexes it as synced |
| Local delete | Indexed local file is removed after sync | Daemon deletes only the matching remote file ID |
| Remote delete | Indexed remote file is removed after sync | Daemon removes only the matching local file |
| Conflict | Both sides change the same indexed file | Daemon preserves local content and downloads a `.proton-cloud` sidecar |

## Current Automated Coverage

Normal `cargo test` runs simulated daemon reconciliation tests with an injected Proton client. These tests validate the local filesystem, the recorded remote operation, and the SQLite index together for the file-level parts of the matrix.

| Matrix Area | Current Coverage | Remaining Gap |
| ----------- | ---------------- | ------------- |
| Bootstrap files | B-01, B-02, B-03, and B-04 file behavior are covered through daemon tests | B-04 records the primary path as `conflict`; the schema does not yet store a separate sidecar row |
| Nested files | SD-01, SD-02, SD-03, SD-04, and SD-08 safe create behavior are covered. SD-05 recursive directory deletion is implemented and covered by deterministic daemon/planner tests (subtree-proof propagation; ambiguous subtrees fall back to recreate) | Recursive directory *trash* behavior against the real Proton Drive service is not yet live-verified (the mutating live E2E test exercises file-level delete, not a directory subtree) |
| Steady-state files | SS-01 through SS-06 are covered through daemon and planner tests. Remote-only rename convergence is covered as a local move when Proton ID and SHA-1 evidence are unique. SS-07 and SS-08 local-to-remote rename and move execute via the verified `filesystem rename` / `filesystem move` CLI contract and are covered by fake-CLI, daemon, and live E2E tests | None outstanding for file-level rename/move; SD-06/SD-07 directory-level rename/move still fall back to today's non-destructive behavior |
| Conflict files | CF-01, CF-02, CF-03, and SD-09 type-clash no-mutation behavior are covered through daemon and planner tests | A richer type-conflict resolution workflow remains future work |
| Edge cases | EG-01 is covered by the empty-file SHA-1 test; EG-02 is covered by path-safe command construction patterns | A dedicated special-character rename test still needs rename support |
| Control IPC | IC-01 and IC-02 are covered by the IPC integration test plus watcher regression tests | A live daemon pause/resume test with real Proton traffic remains pending |
| Interruption safety | Failed side-effect handling verifies that successful early actions are not committed after a later upload failure. An IPC process test covers failed upload behavior across the daemon and control CLI boundary. A real `SIGINT` sent to the daemon process during a blocked upload is covered by a deterministic integration test (`tests/ipc_cli.rs::sigint_during_blocked_upload_exits_cleanly_without_partial_index_state`): it proves the daemon reaches a clean, bounded shutdown, commits no partial index state, and releases its lockfile | The test uses a fake CLI blocked on a sentinel file, not a real large upload against the live Proton Drive service; see the note below the table for why this is sufficient |
| Mutating live gate | Unit tests validate the opt-in gate, disposable-root prefix, and unique run-folder naming. The mutating live E2E test (`tests/proton_live.rs::mutating_live_e2e_exercises_upload_download_rename_move_delete`) has been executed against a real, disposable Proton Drive folder and passed end-to-end (upload, download, rename in place, move into a subfolder, delete) | Conflict-scenario and pause/resume live tests remain pending; directory-subtree live trash behavior is untested |

SIGINT note: the daemon's main loop runs its reconcile step via `tokio::task::block_in_place`, so a `SIGINT` delivered while a proton-drive call is genuinely stuck is only observed once that call returns control to the loop - in practice, once the stuck call's own `CommandPolicy` timeout kills it. The deterministic test validates exactly this real, bounded behavior (not instantaneous mid-syscall interruption) and already proves the properties that matter for data safety: the daemon does not hang forever, no partial index state is committed, and the lockfile is released. A live, large-upload variant of this scenario would exercise the same code path and is not expected to behave differently; it has not been run live because it offers no additional coverage over the deterministic test while consuming real account quota and time.

## Full Matrix Gaps

Full matrix coverage still requires these implementation milestones before the corresponding E2E tests can be honest end-to-end validations:

* Live-verify recursive folder trash semantics against the real Proton Drive service (deterministic subtree-proof propagation is already implemented and tested; only the live folder-trash call itself is unverified).
* Verify the Proton CLI command contract for parent-folder identifiers before enforcing targeted parent IDs for nested uploads and moves.
* Add safety-gated mutating live tests for the conflict and pause/resume scenarios; upload, download, rename, move, delete, and `SIGINT` interruption are now covered (the last two deterministically, the first five live).

## Resolved Risks

* **CLI subprocess stdin inheritance (fixed):** `spawn_once` in `src/proton.rs` previously spawned the `proton-drive` CLI without configuring its stdin, so it inherited whatever stdin the calling process had. When that stdin was a live interactive terminal, the Node.js-based CLI kept an event-loop handle open waiting for input that would never arrive, hanging every caller until `CommandPolicy`'s timeout forcibly killed the child. This was found via live E2E execution (fast fake-CLI tests never exercise real subprocess stdio inheritance) and fixed by always setting `.stdin(Stdio::null())`. A permanent regression test (`proton::tests::spawned_commands_do_not_inherit_an_open_stdin`) now guards against a recurrence.

## Implementation Sequence

1. Keep the existing ignored read-only smoke test as the first live gate.
2. Add helper code that refuses mutating tests unless every safety gate passes.
3. Add one upload/download round-trip test against a unique run folder.
4. Add delete scenarios only after upload/download cleanup is reliable.
5. Add conflict coverage last because it depends on stable remote fixture creation.

## Cleanup Rules

Each test records every remote ID it creates. Cleanup deletes only those recorded IDs and should run even if assertions fail. When cleanup fails, the test output must print the disposable run folder path and remote IDs so the operator can clean up manually.
