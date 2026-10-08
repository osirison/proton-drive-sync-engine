---
trigger: FakeDaemon, gui_core::testing, a GUI test that hangs after its last assertion, cargo test hangs forever, test that swaps one fake daemon for another at the same socket path
depends_on: gui/gui-core/src/testing.rs, gui/src-tauri/src/commands/selection_tests.rs
recorded: 2026-10-08
---

# A `FakeDaemon` hangs its test at the END if the directory holding its socket is dropped first

**Symptom:** every assertion in a GUI test passes, the last line runs, and then `cargo test` sits for
ever on that one test (`test … ... ` with no `ok`). It looks like a deadlock in the code under test.
Nothing in the output names the cause, and the stall detector of whatever is running the suite is
the first thing to notice.

**Cause (measured, one `eprintln!` before and after each step):** `FakeDaemon::drop` wakes its accept
loop by connecting to its own socket and then `join`s the thread. If the socket has been deleted by
then — because the `TempDir` it lives in was dropped first — the connect fails, the accept loop never
sees the stop flag, and `join` waits for ever.

**How it happens:** a test that needs a daemon at a path it controls builds
`FakeDaemon::multi_pair(…).in_dir(dir.path())` and keeps `dir` beside it. Locals drop in the REVERSE of their
declaration, and so do locals bound by a destructuring pattern:

```rust
let Downgrade { h, socket_dir: _dir } = downgraded();   // h declared first, _dir second
// … end of scope: _dir (the directory) drops BEFORE h (the daemon) → hang
```

**Fix:** keep the two in one struct whose fields are declared daemon-first, and never destructure it
(`let d = downgraded(); let h = &d.h;`). Struct fields drop in declaration order. `selection_tests.rs`'s
`Downgrade` is the example, with the reason on the type.

**Same family:** swapping one fake for another at the same socket (a daemon "downgraded in the
meantime") needs the first dropped — which removes its socket file — before the second binds; use one
short directory for both and `drop(first)` explicitly.
