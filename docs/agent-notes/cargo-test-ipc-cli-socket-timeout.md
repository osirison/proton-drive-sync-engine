---
trigger: cargo test --test ipc_cli, timed out waiting for daemon socket, wait_for_socket, daemon stderr empty, ipc_cli CI failure
depends_on: tests/ipc_cli.rs, src/bin/proton-syncd.rs, .github/workflows/ci.yml (rust job)
recorded: 2026-09-09
---

# `ipc_cli` timing out on the daemon socket in CI is a runner-speed flake, and it has a tell

**Symptom:** the `rust` job fails and several `tests/ipc_cli.rs` tests report

```
thread 'unix_tests::status_is_answered_while_a_sync_is_blocked_mid_transfer' panicked at tests/ipc_cli.rs:
timed out waiting for daemon socket at /tmp/.tmpjo0sig/daemon.sock
daemon stderr: <empty>
```

**Measured** (2026-09-09, commit `3c2d78f`, run 34342061082): three of fifteen failed this way in
CI, the same three tests passed with the other twelve locally in 0.97s, and **re-running the
identical commit was green**. The commit changed no Rust and no `Cargo.toml` — ten comment and
documentation edits in shell, YAML, packaging and markdown.

**The tell that separates this from a real failure.** `wait_for_socket` has two panics and they mean
opposite things:

- *"proton-syncd exited before binding socket: {status}"* — the daemon **died**. Real. Its stderr is
  printed with it and will say why.
- *"timed out waiting for daemon socket"* with **empty stderr** — the daemon is still alive and has
  printed nothing. The deadline is a flat 5s polled every 25ms, and fifteen tests each spawning a
  freshly built debug `proton-syncd` on a contended runner is enough to miss it.

Empty stderr is doing the work in that second reading. A daemon that failed for a real reason logs
to stderr (`tracing` writes there; stdout is reserved for dry-run JSON), so silence means it had not
got far enough to have an opinion.

**Do not go looking at the user-global lock.** It is the obvious suspect — every daemon this user
runs contends on one `flock`, per #23 — and it is already ruled out in the harness. `DaemonProcess`
sets `XDG_STATE_HOME` to the test's own tempdir, under the comment beginning *"Isolate the
user-global single-instance lock per test"*, precisely so parallel tests do not serialise on it.

**What to do:** re-run the job (`gh run rerun <run-id> --failed`) and run the target locally
(`cargo test --test ipc_cli`) before touching anything. Two greens on the unchanged commit is the
evidence; a diff that contains no `.rs` and no `Cargo.*` is the corroboration and takes one command:

```bash
git diff <last-green>..<red> --name-only | grep -E '\.rs$|Cargo\.(toml|lock)$'
```

If that grep prints nothing, the Rust binary under test is byte-identical to the one that passed and
the failure cannot be the change.

**Not fixed here.** Raising the 5s deadline or serialising the spawns would make it rarer; neither
was attempted, and the flake is pre-existing rather than owned by any one branch.
