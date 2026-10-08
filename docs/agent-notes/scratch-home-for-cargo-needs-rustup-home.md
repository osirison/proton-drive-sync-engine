---
trigger: running cargo (test, clippy, build) with HOME pointed at a scratch directory so the suite cannot reach the real per-user state; "rustup could not choose a version of cargo to run"
depends_on: rustup, tests/common/mod.rs
recorded: 2026-10-08
---

# A scratch `HOME` for a cargo run must keep `RUSTUP_HOME` and `CARGO_HOME`

**Symptom:** `cargo test --workspace ...` run with `HOME=<scratch>` exits at once with

```
error: rustup could not choose a version of cargo to run, because one wasn't specified explicitly, and no default is configured.
```

and the log has no test output at all.

**Cause:** rustup finds its toolchains under `$HOME/.rustup` and the registry under `$HOME/.cargo`.
A scratch `HOME` has neither.

**Fix:** export both before changing `HOME`:

```bash
export RUSTUP_HOME=/home/<user>/.rustup CARGO_HOME=/home/<user>/.cargo
export HOME=<scratch>/home XDG_RUNTIME_DIR=<scratch>/run XDG_STATE_HOME=<scratch>/state \
       XDG_DATA_HOME=<scratch>/data XDG_CONFIG_HOME=<scratch>/config XDG_CACHE_HOME=<scratch>/cache
chmod 700 <scratch>/run
```

The tests that spawn a binary are sandboxed by `tests/common` on their own; the scratch variables
are a second layer for the lib tests and for anything a new test forgets.
