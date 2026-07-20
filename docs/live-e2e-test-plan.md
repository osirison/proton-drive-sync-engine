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
| Nested files | SD-01, SD-02, SD-03, SD-04, and SD-08 safe create behavior are covered. SD-05 recursive directory deletion is implemented and covered by deterministic daemon/planner tests (subtree-proof propagation; ambiguous subtrees fall back to recreate) | Recursive directory *trash* behavior has been live-executed against a real Proton Drive account (`tests/proton_live.rs::mutating_live_e2e_trashes_nested_folder_contents`) and passes; none outstanding |
| Steady-state files | SS-01 through SS-06 are covered through daemon and planner tests. Remote-only rename convergence is covered as a local move when Proton ID and SHA-1 evidence are unique. SS-07 and SS-08 local-to-remote rename and move execute via the verified `filesystem rename` / `filesystem move` CLI contract and are covered by fake-CLI, daemon, and live E2E tests. SD-06/SD-07 directory-level rename/move is now implemented for the remote-to-local direction, keyed on a backfilled Proton ID with the same uniqueness guarantee as file rename, and covered by deterministic planner and daemon tests, plus a live E2E test (`tests/proton_live.rs::mutating_live_e2e_verifies_directory_rename_and_move`) and a directory-id-exposure live test (`mutating_live_e2e_verifies_directory_id_exposure`), both now executed against a real account and passing | None outstanding for file-level rename/move. Local-to-remote directory rename/move remains a documented non-goal because directories have no content hash to match against; remote-to-local directory rename/move is deterministically tested and live-verified |
| Conflict files | CF-01, CF-02, CF-03, and SD-09 type-clash behavior are covered through daemon and planner tests. A local directory clashing with a same-named remote file keeps the local directory, downloads the remote file as a `.proton-cloud` sidecar with its own conflict-tracked index row, and does not re-trigger on later reconciles once resolved. A live conflict scenario (`tests/proton_live.rs::mutating_live_e2e_resolves_conflict_by_downloading_a_sidecar_copy`) captures real diverging local/remote state, plans it with `plan_sync_entities`, and executes the resulting sidecar download against the real service - now executed and passing after fixing two critical bugs the first run surfaced (see Resolved Risks) | None outstanding at the planner/daemon level for the local-directory/remote-file clash. The reverse clash (a local file where the remote holds a directory of the same name) still uses the original non-mutating skip. SD-09's sidecar resolution has not yet been exercised against a real Proton Drive account |
| Edge cases | EG-01 is covered by the empty-file SHA-1 test. EG-02 is covered by a dedicated CLI-argument-safety test (`proton::tests::rename_or_move_passes_spaces_and_special_characters_through_a_single_argument`) proving a filename with spaces and shell metacharacters passes through as a single unmangled argument, plus a planner-level test (`sync::tests::local_file_rename_with_spaces_and_special_characters_plans_verified_remote_move`) proving rename detection itself is unaffected by special characters | None outstanding |
| Control IPC | IC-01 and IC-02 are covered by the IPC integration test plus watcher regression tests | A live daemon pause/resume test with real Proton traffic remains pending |
| Interruption safety | Failed side-effect handling verifies that successful early actions are not committed after a later upload failure. An IPC process test covers failed upload behavior across the daemon and control CLI boundary. A real `SIGINT` sent to the daemon process during a blocked upload is covered by a deterministic integration test (`tests/ipc_cli.rs::sigint_during_blocked_upload_exits_cleanly_without_partial_index_state`): it proves the daemon reaches a clean, tightly bounded shutdown (asserted well under the fake CLI's configured command timeout, not just eventually), commits no partial index state, and releases its lockfile | The test uses a fake CLI blocked on a sentinel file, not a real large upload against the live Proton Drive service; see the note below the table for why this is sufficient |
| Mutating live gate | Unit tests validate the opt-in gate, disposable-root prefix, and unique run-folder naming. All seven live E2E tests in `tests/proton_live.rs` - read-only smoke, upload/download/rename/move/delete, directory id exposure, folder-trash, directory rename/move, and conflict resolution - have been executed against a real, disposable Proton Drive folder and pass | A live pause/resume test remains unwritten |

SIGINT note: the daemon's main loop runs its reconcile step via `tokio::task::block_in_place`, so a `SIGINT` is only *acted on* by that blocking call once its own polling loop notices a shared cancellation flag. A separate, always-running task sets that flag the instant the signal is delivered (independent of whatever the blocking call is doing), and `run_once`'s polling loop (`src/proton.rs`) checks the flag every `CANCELLATION_POLL_INTERVAL` (100ms), so in practice the stuck call is interrupted and its whole process group is killed within about one poll interval - not only once the call's own `CommandPolicy` timeout would otherwise have fired. The deterministic test asserts this tightened bound directly (elapsed time, not just eventual completion) and already proves the properties that matter for data safety: the daemon does not hang for the stuck call's full timeout window, no partial index state is committed, and the lockfile is released. A live, large-upload variant of this scenario would exercise the same code path and is not expected to behave differently; it has not been run live because it offers no additional coverage over the deterministic test while consuming real account quota and time.

## Full Matrix Gaps

All seven live E2E tests in `tests/proton_live.rs` have now been executed against a real, disposable Proton Drive account and pass. The remaining gaps, consistent with the per-row "Remaining Gap" column above:

* Add a safety-gated mutating live test for the pause/resume scenario; every other live-testable scenario (upload, download, rename, move, delete, directory id/trash/rename, conflict, and `SIGINT` interruption) is now covered, live-verified where applicable.
* B-04's conflict sidecar is not yet a first-class schema row distinct from the primary path's record; see the Bootstrap files row.
* SD-09's local-directory/remote-file clash resolution (sidecar download, permanent directory ownership) is deterministically tested but has not yet been exercised against a real Proton Drive account; the reverse clash (local file vs. remote directory) still uses the original non-mutating skip.
* Local-to-remote directory rename/move remains a documented non-goal because directories have no content hash to serve as identity evidence; see `docs/rename-detection-design.md`.
* SD-07/SD-08 lack scenario-specific tests pinning a nested-path rename and a nested-path modify-then-upload; the shared, path-agnostic code paths are otherwise covered by initial-creation nested tests.
* IC-01's "file watching continues while paused" behavior is structurally guaranteed by the code (the watcher never checks the pause flag) but has no test asserting the scenario directly.

## Resolved Risks

* **CLI subprocess stdin inheritance (fixed):** `spawn_once` in `src/proton.rs` previously spawned the `proton-drive` CLI without configuring its stdin, so it inherited whatever stdin the calling process had. When that stdin was a live interactive terminal, the Node.js-based CLI kept an event-loop handle open waiting for input that would never arrive, hanging every caller until `CommandPolicy`'s timeout forcibly killed the child. This was found via live E2E execution (fast fake-CLI tests never exercise real subprocess stdio inheritance) and fixed by always setting `.stdin(Stdio::null())`. A permanent regression test (`proton::tests::spawned_commands_do_not_inherit_an_open_stdin`) now guards against a recurrence.
* **Silent upload skip on same-path revisions (fixed):** the real `proton-drive filesystem upload` command prompts interactively for a conflict-resolution strategy whenever the destination already has a same-named file. With the non-interactive stdin this daemon correctly configures, that prompt saw immediate EOF and the CLI silently skipped the file while still exiting 0 - so every revision of an already-synced file was silently failing to reach the remote while the daemon recorded the path as synced. Found via the live conflict test's first execution and root-caused with direct CLI diagnostics. Fixed by always passing `--file-conflict-strategy replace` in `upload()`. Regression test: `proton::tests::upload_passes_a_file_conflict_strategy_to_avoid_the_interactive_prompt`.
* **Conflict sidecar download overwriting the original file (fixed):** `filesystem download` always names its output after the remote source's own basename, not the caller's requested destination basename. Conflict resolution intentionally downloads into a differently-named `.proton-cloud` sidecar so the original conflicting file is preserved; because `download()` trusted the CLI's basename choice, the sidecar download was actually landing on top of (overwriting) the original local file it was supposed to protect. Found immediately after fixing the upload bug above, while re-running the same live conflict test. Fixed by always staging downloads through a private scratch directory and moving the single result into place with `fs::rename`. Regression tests: `proton::tests::download_stages_through_a_scratch_directory_and_moves_the_result_into_place`, `proton::tests::download_to_a_sidecar_name_does_not_clobber_a_file_matching_the_remote_basename`, `proton::tests::download_fails_cleanly_when_the_cli_produces_no_file`.

## Implementation Sequence

1. Keep the existing ignored read-only smoke test as the first live gate.
2. Add helper code that refuses mutating tests unless every safety gate passes.
3. Add one upload/download round-trip test against a unique run folder.
4. Add delete scenarios only after upload/download cleanup is reliable.
5. Add conflict coverage last because it depends on stable remote fixture creation.

## Cleanup Rules

Each test records every remote ID it creates. Cleanup deletes only those recorded IDs and should run even if assertions fail. When cleanup fails, the test output must print the disposable run folder path and remote IDs so the operator can clean up manually.
