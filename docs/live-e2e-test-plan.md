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
| Nested files | SD-01, SD-02, SD-03, SD-04, and SD-08 safe create behavior are covered | Recursive directory deletion is still disabled until subtree proof and live folder-trash behavior are verified |
| Steady-state files | SS-01 through SS-06 are covered through daemon and planner tests. Remote-only rename convergence is covered as a local move when Proton ID and SHA-1 evidence are unique | Local-to-remote rename and move execution remains blocked until Proton CLI move or rename command semantics are verified |
| Conflict files | CF-01, CF-02, CF-03, and SD-09 type-clash no-mutation behavior are covered through daemon and planner tests | A richer type-conflict resolution workflow remains future work |
| Edge cases | EG-01 is covered by the empty-file SHA-1 test; EG-02 is covered by path-safe command construction patterns | A dedicated special-character rename test still needs rename support |
| Control IPC | IC-01 and IC-02 are covered by the IPC integration test plus watcher regression tests | A live daemon pause/resume test with real Proton traffic remains pending |
| Interruption safety | Failed side-effect handling verifies that successful early actions are not committed after a later upload failure. An IPC process test covers failed upload behavior across the daemon and control CLI boundary | A real `SIGINT` during a large live upload remains pending |
| Mutating live gate | Unit tests validate the opt-in gate, disposable-root prefix, and unique run-folder naming | Actual upload, download, delete, conflict, and cleanup live tests are still pending |

## Full Matrix Gaps

Full matrix coverage still requires these implementation milestones before the corresponding E2E tests can be honest end-to-end validations:

* Verify recursive folder trash semantics and subtree state before enabling SD-05 or any destructive directory propagation.
* Verify the Proton CLI command contract for remote rename and move before implementing local-to-remote SS-07, SS-08, SD-06, and SD-07 execution.
* Verify the Proton CLI command contract for parent-folder identifiers before enforcing targeted parent IDs for nested uploads and moves.
* Add safety-gated mutating live tests before marking live upload, download, delete, conflict, pause/resume, and `SIGINT` scenarios as complete.

## Implementation Sequence

1. Keep the existing ignored read-only smoke test as the first live gate.
2. Add helper code that refuses mutating tests unless every safety gate passes.
3. Add one upload/download round-trip test against a unique run folder.
4. Add delete scenarios only after upload/download cleanup is reliable.
5. Add conflict coverage last because it depends on stable remote fixture creation.

## Cleanup Rules

Each test records every remote ID it creates. Cleanup deletes only those recorded IDs and should run even if assertions fail. When cleanup fails, the test output must print the disposable run folder path and remote IDs so the operator can clean up manually.
