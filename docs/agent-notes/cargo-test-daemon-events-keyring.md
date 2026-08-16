# `cargo test daemon::tests` — an `events_driven` test can silently acquire a **live** event source

Applies to any test that builds a `Daemon` with `events_driven = true`:

```bash
cargo test --workspace --all-targets --all-features   # or: cargo test daemon::tests
```

## The precondition nothing states

`Daemon::with_client` (`src/daemon.rs`, `impl<C: ProtonClient> Daemon<C>` → `with_client_and_event_source`)
installs a **default** `event_source_factory`. Its body is a closure over `config.events_driven`, and
when that is true it calls `CliKeyringSession::from_cli_keyring()` — which shells `secret-tool` and
reads the **real OS keyring** for the logged-in `proton-drive` CLI's session:

```rust
// src/daemon.rs, in with_client_and_event_source (grep: `let event_source_factory`)
Box::new(move || {
    if !events_driven { return None; }
    match CliKeyringSession::from_cli_keyring() {
        Ok(session) => Some(Box::new(EventsClient::new(...)) as Box<dyn EventSource>),
        Err(_) => None,
    }
})
```

`reacquire_event_source_if_needed` calls that factory on **every pass**, so a test does not have to
ask for an event source to get one.

## What goes wrong

On a machine where the `proton-drive` CLI is logged in and the desktop keyring is unlocked
(`DBUS_SESSION_BUS_ADDRESS` set), a test that sets `events_driven = true` and does not override the
factory acquires a **live** event source part-way through. Then:

- a "degraded session" test exercises the *non*-degraded path, and asserts nothing it thinks it does;
- an events test can pass for the wrong reason, or make real network calls;
- the same test **passes in CI** (no keyring, no session) and **fails locally**, or the inverse —
  which reads as flakiness rather than as a missing stub.

`test_config` sets `events_driven: false`, so only a test that deliberately turns it on is exposed.

## Fix

Override the factory immediately after building the daemon, before the first pass:

```rust
let mut daemon = Daemon::with_client(config, client).expect("daemon");
daemon.event_source_factory = Box::new(|| None);
```

Existing precedents in `src/daemon.rs` (grep `event_source_factory = Box::new(|| None)`) — including
`a_degraded_session_still_reconciles_on_the_scan_interval` and the plan/apply tests. A test that
*wants* a source injects a fake one instead
(`daemon.event_source_factory = Box::new(|| Some(Box::new(FakeEventSource::new("cursor-0"))));`).

Rule of thumb: **if a daemon test sets `events_driven = true`, the very next line decides what the
event source is.** Leaving it to the default makes the test's meaning depend on the machine.
