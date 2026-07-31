---
title: Development
description: Building, the validation suite, focused and live tests, the testing pattern, and how to contribute changes.
sidebar:
  order: 3
---

## Build

```bash
cargo build --all-targets
```

The workspace is one library crate (`proton_drive_sync_engine`) backing `proton-syncd` and
`proton-sync`, plus the desktop-app members under `gui/`. Edition 2024 is required.

## Validation suite

Run this before opening or updating a pull request — CI enforces it:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

`clippy` runs with `-D warnings`, so any warning is a build failure. Keep `cargo fmt` clean.

## Focused tests

```bash
cargo test sync::tests          # planner unit tests (src/sync.rs)
cargo test proton::tests        # remote JSON parsing / CLI wrapper
cargo test events::tests        # volume-events delta parsing + refresh logic
cargo test reconstruct::tests   # base ⊕ delta remote-map reconstruction
cargo test daemon::tests        # reconciliation with injected fakes
cargo test --test dry_run_cli   # dry-run integration test
cargo test --test ipc_cli       # control-socket integration test
cargo test computes_empty_file_sha1   # a single test by name substring
```

## The testing pattern

The daemon is generic over its client (`Daemon<C: ProtonClient>`), so tests inject **fake
`ProtonClient` implementations** — including one that fails uploads to exercise the
checkpoint-commit invariant (completed actions stay committed; the failed action is never
recorded) — and reconciliation is tested without a real Proton Drive.
The planner in `src/sync.rs` is **pure** and tested directly with in-memory maps.

**Prefer adding regression tests next to the logic you change**: planner tests in
`src/sync.rs`, and boundary tests where external paths first enter the engine (remote JSON
parsing, local scanning, conflict targets). When you change a safety boundary, add a test at
the boundary that first accepts the external path.

## Live tests (opt-in, `#[ignore]`)

Several tests exercise a real authenticated Proton Drive account. They are `#[ignore]` by
default and mostly read-only. The read-only smoke test:

```bash
PROTON_SYNC_LIVE_REMOTE_ROOT=/Drive/RemoteFolder \
  cargo test --test proton_live --all-features -- --ignored
# Set PROTON_SYNC_LIVE_CLI=/path/to/proton-drive if it isn't on PATH.
```

There are also `#[ignore]` live checks for the volume-events detection path
(`tests/events_live.rs`) and an id-identity gate (`tests/events_identity_live.rs`) that must
pass before trusting event-driven reconcile on a given account. Mutating live scenarios must
follow the safety gates in
[`docs/live-e2e-test-plan.md`](https://github.com/osirison/proton-drive-sync-engine/blob/main/docs/live-e2e-test-plan.md).

## Where changes usually start

- **Sync behavior** almost always starts in `src/sync.rs` (the pure planner). Add planner
  regression tests there.
- **Runtime/loop behavior** is in `src/daemon.rs`.
- **Remote parsing** is in `src/proton.rs`; **change detection** in `src/events.rs` and
  `src/reconstruct.rs`.
- The desktop app's data layer is `gui/gui-core` (pure Rust, unit-testable standalone); the
  Tauri shell is `gui/src-tauri`; the webview frontend is `gui/src`.

See the repo's `CLAUDE.md` for the architecture map and the invariants to preserve, and the
[ADRs](/concepts/architecture/#design-records) for the non-obvious decisions.

## Contributing

Format, lint, and test before pushing. Keep changes focused, add tests near the code you
touch, and preserve the documented invariants — commit-after-side-effects, path-safety at
boundaries, non-destructive-on-unknown-digests, and selective-sync-applies-everywhere.
