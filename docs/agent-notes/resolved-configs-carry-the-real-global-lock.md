---
trigger: a test that resolves a config file (config::resolve_runtime_configs / resolve_runtime_config) and then builds a Daemon from it
depends_on: src/config.rs (ProcessValues), src/paths.rs (default_global_lock_path), src/daemon.rs (Daemon::from_pairs)
recorded: 2026-10-08
---

# A resolved config carries the REAL per-user lock, and a daemon built from it takes it

**Symptom:** `Daemon::from_pairs(resolved.pairs, ...)` in a test fails with `cannot start: ... Only one
proton-syncd may run per user account` when a real daemon is running on the machine — and passes on a
machine without one, and in CI. It reads as a flaky test.

**Cause (read from the code; the failure itself was not reproduced, because reproducing it would
take the real lock):** `DaemonConfig::global_lock_path` is "not user-overridable": the
single-instance guarantee must key on one fixed per-user path whatever flags are given, so
`config::resolve_runtime_configs` fills it from `paths::default_global_lock_path()`
(`$XDG_STATE_HOME/<app dir>/<lock name>`, else under `~/.local/state`). A hand-built test config
(`test_config`) points it into the test's tempdir; a **resolved** one does not. `from_pairs` takes that
lock for the life of the daemon, which is the very lock the real daemon holds.

**Fix:** after resolving, before constructing, repoint it:

```rust
for config in &mut configs {
    config.global_lock_path = directory.path().join("global.lock");
}
```

`the_daemon_wide_halves_of_every_pair_config_agree` (`src/daemon.rs`) does this. Resolution itself
(`resolve_runtime_configs`) only *computes* the path — it never opens it — so a config test that stops
at the `RuntimeConfigs` is safe. A test that spawns the **binary** is safe too if it sets
`XDG_STATE_HOME` to its tempdir, as `DaemonProcess::spawn_with_config` does.

**Why it is easy to miss:** the failure needs a live daemon on the machine running the tests, and
that machine is the developer's own.
