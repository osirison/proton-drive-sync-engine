---
trigger: cargo test -p proton-sync-gui, restart_service_impl, start_service_impl, poison verification
depends_on: gui/src-tauri/src/commands.rs
recorded: 2026-08-16
---

# A test around `restart_service_impl` can start the developer's real daemon

**Symptom:** none, which is the problem. `cargo test -p proton-sync-gui --lib` passes, and
`systemctl --user status proton-syncd` shows a unit that was started by the test run —
on a machine where the user had deliberately stopped it, the daemon is now syncing.

**Cause:** `restart_service_impl` falls through to `start_service_impl`, which shells
`systemctl --user start proton-syncd`. The unit is a real one on any machine this
project is developed on (`~/.config/systemd/user/proton-syncd.service`, installed by
`setup.sh`), so the call succeeds and the daemon comes up. A `tempfile::tempdir()`
config path does not prevent it: the systemd branch is tried FIRST and the config path is
only consulted when systemd fails.

**Where it bites:** poison verification. Breaking the `only_if_running` branch and re-running
the test is exactly the check the process asks for — and that broken build is one whose test
reaches `start_service_impl`. The assertion fails as intended and the daemon starts as a side
effect, silently.

**Fix / the rule:** a branch that decides whether a subprocess runs gets a **pure predicate**
beside it, and the poison check targets the predicate:

```rust
fn restart_plan(only_if_running: bool, probe: DaemonProbe) -> RestartPlan { … }
```

`a_save_never_starts_a_service_that_was_not_running` asserts the truth table with no I/O at
all, so breaking it and re-running proves the decision without ever reaching a `Command`.
The test that drives `restart_service_impl` itself stays — it is the wiring — but it is only
ever green-path: with `only_if_running: true` and a socket nothing is listening on, the early
return happens before any spawn.

**Run the pure test ALONE while the poison is in, and name it in full.** A poisoned
`restart_plan` is exactly the build in which the green-path wiring test stops being green-path:
with the decision broken, `restart_service_impl` walks past its early return and reaches the
spawn. So filter to the one test, and the filter needs the module path — the tests live in
`commands::socket_tests`, not `commands::tests`, and a bare name with `--exact` silently
matches nothing (`0 passed; 73 filtered out`, which reads like a pass):

```bash
cargo test -p proton-sync-gui --lib \
  commands::socket_tests::a_save_never_starts_a_service_that_was_not_running -- --exact
# `cargo test -p gui-core` does not exist either — the package is `proton-sync-gui-core`.
```

**Check afterwards anyway:** `systemctl --user status proton-syncd` reports `Active: … since`,
so an untouched daemon's start time is older than the test run. That is how this one was
confirmed harmless — twice now (#320 and #335).
