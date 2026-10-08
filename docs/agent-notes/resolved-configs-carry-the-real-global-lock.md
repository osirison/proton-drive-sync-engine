---
trigger: a test that resolves a config file (config::resolve_runtime_configs / resolve_runtime_config) and then builds a Daemon from it; a test that spawns proton-syncd or proton-sync (CARGO_BIN_EXE, DaemonProcess, common::sandboxed); a poison check on a flag or config refusal; control socket missing, proton-sync.sock gone
depends_on: src/config.rs (ProcessValues), src/paths.rs (default_global_lock_path, default_socket_path), src/daemon.rs (Daemon::from_pairs), tests/common/mod.rs, tests/test_isolation.rs
recorded: 2026-10-08
---

# A test reaches the machine's REAL per-user state unless it is cut off from it

Two shapes of one hazard: a daemon built **in the test process** from a resolved config takes the real
per-user lock, and a daemon **spawned as a binary** binds the real control socket. The second one has
already happened (below).

## 1. A resolved config carries the REAL per-user lock, and a daemon built from it takes it

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
at the `RuntimeConfigs` is safe.

**Why it is easy to miss:** the failure needs a live daemon on the machine running the tests, and
that machine is the developer's own.

## 2. A test that spawns the binary must be sandboxed, bounded, and given a fake CLI

**The incident (#102 phase 4c, 2026-10-08).** While poison-testing the "accept `--pair` without a
preview" check, a test ran `proton-syncd --config <temp> --pair b` with no `--socket-path`, no
`XDG_RUNTIME_DIR` override and an unbounded wait. With the check removed it was no longer refused: it
became a real daemon, bound the default socket `$XDG_RUNTIME_DIR/proton-sync.sock` — **replacing the
live daemon's** — and deleted it again on exit. The live daemon kept running with no control socket
until it was restarted. Nothing failed: the test got the wrong answer, loudly, but only after the damage.

**The rule.** A test that is meant to show a run is *refused* or *previewed* fails by starting a real
daemon, so the sandbox is not for the tests that need it; it is for the one that is about to be wrong.
A daemon reaches, and a test must cut it off from:

| Reach | Where it points | Redirected by |
|---|---|---|
| control socket | `$XDG_RUNTIME_DIR/proton-sync.sock` | `XDG_RUNTIME_DIR` (and still pass `--socket-path`) |
| user-global lock | `$XDG_STATE_HOME/...` | `XDG_STATE_HOME` |
| local-delete trash | `$XDG_DATA_HOME/Trash` | `XDG_DATA_HOME` |
| the above's fallback, and `~` | `$HOME` | `HOME` |
| the real `proton-drive` CLI | `proton-drive` on `PATH`, signed in through the desktop keyring | **nothing** — name a fake |

The last row is the one no environment variable fixes: the keyring session is reached over D-Bus, so a
daemon with no `--proton-cli` and no `proton_cli` in its config would sync the real account. One
integration test (the per-pair-flag refusal) had exactly that gap.

**Where it lives, once.** `tests/common/mod.rs` — `sandboxed(program, dir)` (the four variables),
`run_bounded(command, limit)` (kills after the bound; output to temp files, not pipes),
`spawn_logging(command, stderr_path)` (a daemon, stderr kept for the wait helpers) and
`refuse_the_real_cli(command)` (a `proton-syncd` with neither `--proton-cli` nor a `--config` that sets
`proton_cli` panics before it spawns). `tests/test_isolation.rs` pins each of those and **refuses any
test file that writes `Command::new(env!("CARGO_BIN_EXE_...` or `.spawn()` by hand**. A new test spawns
through `common`, or that test fails.

**Poisoning a refusal check.** Break the check, run only the test about it, and make sure the run
cannot become a daemon: sandboxed, `--proton-cli` a fake, bounded. Never run a poisoned `sandboxed`
against a test that spawns `proton-syncd`; poison it against `test_isolation` (which spawns `sh`).

**Lib tests are safe by construction**, not by this helper: they inject a fake `ProtonClient`, use
`test_config` (every path in the tempdir) or repoint the lock as in section 1, and pin the event-source
factory off the keyring (`cargo-test-daemon-events-keyring.md`). A binary-level test is the only kind
that needs `common`.

**Not covered, by design:** `tests/proton_live.rs`, `events_live.rs` and `events_identity_live.rs` are
`#[ignore]` and use the real CLI and session on purpose (read-only unless `PROTON_SYNC_LIVE_WRITE=1`).
