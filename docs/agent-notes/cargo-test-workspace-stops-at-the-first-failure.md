---
trigger: cargo test --workspace, cargo test --workspace --all-targets --all-features
depends_on: Cargo.toml (workspace members), gui/gui-core, gui/src-tauri
recorded: 2026-08-18
---

# `cargo test --workspace` stops at the first failing *target*, so everything after it is unrun

**Symptom:** you break something shared — a `config.rs` rule, a wire type, an error message
`gui-core` matches on — run the whole suite, read

```
test result: FAILED. 596 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out
error: test failed, to rerun pass `-p proton-drive-sync-engine --lib`
```

and conclude the blast radius is one test. It is not. Cargo stopped at that binary.

**Measured, not assumed** (2026-08-18, one deliberately failing test appended to `src/lib.rs`):
the run printed exactly one `Running` line — `unittests src/lib.rs` — and exited. Not one
integration test in `tests/` ran, and neither `gui-core` nor `src-tauri` was reached. Cargo's
own hint names only the target it stopped on, which reads like a complete diagnosis.

**Fix:** when a change can reach both sides, run the GUI crate by name as part of the check,
not as a follow-up:

```bash
cargo test --workspace --all-targets --all-features
cargo test -p proton-sync-gui-core --lib
```

`--no-fail-fast` also runs everything and is fine for a one-off, but it buries the failures in
a long tail; naming the crate is what turns a green `gui-core` into an assertion instead of an
assumption. Note the same trap applies *within* the root crate — a failing `--lib` hides every
`tests/*.rs` integration binary, so `cargo test --test ipc_cli` is worth naming too when the
change touches the control protocol.

**Why it matters here specifically:** the two crates share more than types. `gui-core`'s
`ConfigDoc::validate` calls the engine's `validate_file_config_text`, and at least one
`gui-core` test matches the engine's error *text*
(`an_empty_root_is_refused_before_it_reaches_the_daemon`). An engine-side reword or rule change
is exactly the kind of edit whose second failure lands over there — and exactly the kind whose
first failure is the one you are already staring at.

**Related:** `--workspace` is required at all because the workspace root is itself a package, so
bare `cargo test` runs only the root crate (CLAUDE.md says so). This note is the next step: the
flag gets the GUI crates *scheduled*, not *reached*.
