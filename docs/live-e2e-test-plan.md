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

## Implementation Sequence

1. Keep the existing ignored read-only smoke test as the first live gate.
2. Add helper code that refuses mutating tests unless every safety gate passes.
3. Add one upload/download round-trip test against a unique run folder.
4. Add delete scenarios only after upload/download cleanup is reliable.
5. Add conflict coverage last because it depends on stable remote fixture creation.

## Cleanup Rules

Each test records every remote ID it creates. Cleanup deletes only those recorded IDs and should run even if assertions fail. When cleanup fails, the test output must print the disposable run folder path and remote IDs so the operator can clean up manually.
