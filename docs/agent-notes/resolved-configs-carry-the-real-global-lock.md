---
trigger: a test that resolves a config file (config::resolve_runtime_configs / resolve_runtime_config) and then builds a Daemon from it; a test that spawns proton-syncd or proton-sync (CARGO_BIN_EXE, DaemonProcess, common::sandboxed); a poison check on a flag or config refusal; control socket missing, proton-sync.sock gone; a spawned proton-syncd with events_driven on (secret-tool, the desktop keyring, D-Bus)
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
| the signed-in Proton session | `events_driven` is ON by default, and then the daemon runs `secret-tool lookup service ch.proton.drive/drive-sdk-cli account auth-session` (the desktop keyring, over D-Bus) and calls the events API with it through `curl` | the bus address is removed (`DBUS_SESSION_BUS_ADDRESS`, `DBUS_SYSTEM_BUS_ADDRESS`); stub `secret-tool` and `curl` are first on `PATH`; and a daemon that leaves `events_driven` on is refused |

The last two rows are the ones no path variable fixes. The CLI row: a daemon with no `--proton-cli`
and no `proton_cli` in its config would sync the real account (one integration test, the
per-pair-flag refusal, had exactly that gap). The session row was found by the second review of
#102 phase 4c: naming a fake CLI does **not** stop the event session, which is a separate path to
the same account. A poisoned run that became a daemon with a two-pair config that did not set
`events_driven = false` reached `secret-tool` three times, and the reach was answered by the
sandbox's stub `secret-tool`, not the real one: the sandbox's stub directory is first on `PATH`,
the bus address is removed, and the run failed at its bound with the stub's record. The real
keyring was not read. So the sandbox cuts it three ways at once, because each
layer is what the one before it needs to fail: no bus to ask, stubs where the tools would be, and a
refusal before the spawn.

**Where it lives, once.** `tests/common/mod.rs` — `syncd(dir)` / `sync_cli(dir)` (the two binaries,
sandboxed; the only places a test names them), `sandboxed(program, dir)` / `sandbox(&mut command, dir)`
(`HOME` and the five XDG variables, the bus addresses removed, the stub tools first on `PATH`),
`run_bounded(command, limit)` (kills after the bound; output to temp files, not pipes; **fails a run
that reached a stub tool**, since the stub made a real-session read harmlessly fail and the test
would otherwise pass degraded), `spawn_logging(command, stderr_path)` (a daemon, stderr kept for the
wait helpers; it returns a `LoggedChild`, which derefs to `Child` and **fails the test on drop if the
process reached a stub**), `run_other_tool(command)` (the one way to run `kill`, `pgrep` or the live
tests' real CLI; it refuses the crate's own binaries) and `refuse_the_real_cli(command)` (a `proton-syncd` panics before it spawns unless it
names a fake CLI — `--proton-cli`, or a `--config` with `proton_cli` — **and** turns events off:
`--no-events-driven`, or `events_driven = false` in the config, in *every* `[[pair]]` table when it
has them, and never beside `--events-driven`, which wins over the file; a test about the session itself opts in by name with `opt_in_to_events_driven`, and none
does today). The stubs append what they were asked to `<sandbox dir>/.sandbox-stub-hits`.

**What the spawn guard catches, exactly.** `tests/test_isolation.rs` pins each of those, and its
scan fails any file under `tests/` **other than `common/mod.rs` and `test_isolation.rs` itself**
(`test_isolation.rs` is exempt because its probe trees contain every form on purpose) that has, on a
line not starting with `//`:

- the literal `CARGO_BIN_` + `EXE_` (`Command::new(env!(...))`, a variable bound first, `option_env!`,
  `std::env::var`);
- a string literal **ending in** `proton-syncd` or `proton-sync` (`"proton-syncd"`,
  `"/home/me/.cargo/bin/proton-syncd"`): a bare name resolves through `PATH` to the user's installed
  binary, which is the **live daemon** on a developer's machine. `.proton-sync.toml` and
  `proton-sync-gui` are longer names and are not matched;
- a `Command` launch: `.spawn(`, `.output(`, `.status(` or `.exec(`, also as `Command::spawn(&mut c)`,
  and with the name and the parenthesis on different lines;
- `Command as <name>`, which would hide the type from the forms above;
- `common::DAEMON_FILE_NAME` / `CONTROL_CLI_FILE_NAME` (the names, for a test comparing a manifest) in
  a file that also mentions `Command`.

A line inside a `/* */` block comment that holds a form is reported: the scan does not understand
block comments, so it fails closed. It does **not** catch a binary reached without its name (a path
built from `CARGO_MANIFEST_DIR` and `target/`, the name split with `concat!`, a copy of the binary), a
launch wrapped in a function that holds none of the forms itself, or `cargo run --bin`, and it follows
no data flow. A new test starts a binary through `common`, runs any other program with
`common::run_other_tool`, or that test fails. (The first version matched the text
`Command::new(env!("CARGO_BIN_EXE_` and was bypassed by binding the name to a variable first; the
second missed a bare name, `.output()`/`.status()`, `Command::spawn(&mut c)` and a split `.spawn`.)

A caller that clears the environment after `sandboxed` (the install-script sandbox in
`tests/scripts.rs` does, to build a minimal one) loses the stubs and the stub log, and builds its
`PATH` with `common::path_with_stubs`.

**Poisoning a refusal check.** Break the check, run only the test about it, and make sure the run
cannot become a daemon: sandboxed, `--proton-cli` a fake, events off, bounded. Never run a poisoned
`sandboxed` against a test that spawns `proton-syncd`; poison it against `test_isolation` (which
spawns `sh`). The incident's own poison, re-run on the sandbox as it is now: with every guard in place the
run fails at its 20 s bound and touches nothing outside its temp directory; with the events refusal
and `events_driven = false` removed as well, the poisoned daemon reaches the stub `secret-tool`
(and the failure message quotes it) instead of the keyring. Compare `/run/user/<uid>` before and
after, and check that the only `proton-syncd` running is the live one.

**A fake CLI that waits must not outlive its test.** `tests/ipc_cli.rs`'s blocking-upload fake used to
wait for a release file that never came, in a process group of its own (the daemon starts every CLI
child that way, so killing the daemon does not reach it): 59 of them, up to 1.3 days old, were found
on one machine. It now leaves when its parent is gone and after a minute regardless, and the tests
that use it kill its group on drop. Check after a full run with a bracketed pattern, which cannot
match its own `pgrep`: `pgrep -fc '[f]ake-blocking-upload-proton-drive'` should print `0`.

**Lib tests are safe by construction**, not by this helper: they inject a fake `ProtonClient`, use
`test_config` (every path in the tempdir) or repoint the lock as in section 1, and pin the event-source
factory off the keyring (`cargo-test-daemon-events-keyring.md`). A binary-level test is the only kind
that needs `common`.

**Not covered, by design:** `tests/proton_live.rs`, `events_live.rs` and `events_identity_live.rs` are
`#[ignore]` and use the real CLI and session on purpose (read-only unless `PROTON_SYNC_LIVE_WRITE=1`).
