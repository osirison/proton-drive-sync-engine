---
trigger: cargo run --example, cargo build --example, rm -rf examples
depends_on: examples/, tests/example_assets.rs
recorded: 2026-08-15
---

# `examples/` is shipped packaging, not a scratch directory

**Symptom:** five tests fail at once with `No such file or directory` —
`example_config_resolves_to_valid_daemon_config`,
`example_systemd_service_points_at_example_config_location`,
`example_systemd_installer_has_safe_defaults`,
`release_asset_manifest_points_at_existing_distribution_assets`,
`release_archive_helper_has_expected_packaging_contract`.

**Fix:** `git checkout -- examples/`. The directory is tracked and holds the
shipped `proton-sync.toml`, the systemd unit and installer, and the release
packaging assets; `tests/example_assets.rs` asserts on all of them (the config
one *resolves* it through `resolve_runtime_config`, so a new daemon config key
should be documented there and that test proves the file still loads).

**Why it was not obvious:** `examples/` is also Cargo's own convention for
throwaway binaries, so writing `examples/probe.rs` to check some library
behaviour with `cargo run --example probe` is the natural move — and cleaning it
up with `rm -rf examples` then deletes five tracked files. Nothing in the
directory hints that it is load-bearing, and the tests that break name paths
rather than the deletion.

For a throwaway probe, use the session scratchpad with a separate
`cargo new` project, or a `#[test]` behind `--nocapture`. If you do add anything
under `examples/`, delete the single file rather than the directory, and run
`git status` before any `rm -rf` inside the repo.
