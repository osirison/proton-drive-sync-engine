---
trigger: launching target/debug/proton-sync-gui or the release binary to drive it, "Start the sync service", tray Start row, Restart syncing, restart_service, start_service_impl, scratch XDG_RUNTIME_DIR
depends_on: gui/src-tauri/src/commands.rs (start_service_impl, restart_service_impl), gui/src-tauri/src/tray.rs (start_service_in_background)
recorded: 2026-10-09
---

# A GUI launched in a scratch environment still shells `systemctl` and `proton-syncd` by name

`gui-isolated-from-the-real-daemon.md` isolates the GUI's paths, socket and bus. It does not cover the
two programs the GUI runs by bare name, looked up on `PATH`. A scratch environment with the real `PATH`
leaves both pointing at the developer's installed ones.

**Confirmed by reading `commands.rs` (not run):** `start_service_impl` runs `systemctl --user start
proton-syncd` first, and when that does not succeed it spawns `proton-syncd --config <path>`. The
window's `Start the sync service`, the tray's `Start` row (`start_service_in_background` ->
`start_service_and_adopt`) and a Settings save that restarts a running daemon (`restart_service_impl`)
all reach it. The tray row and the button need only a daemon that does not answer, which is what an
isolated environment with no socket gives you.

**What can go wrong:**

- `systemctl --user start proton-syncd` may reach the developer's real user manager. **Inferred, not
  run:** scratch `XDG_*` variables move the runtime directory, but a `DBUS_SESSION_BUS_ADDRESS` carried
  through unchanged (the recipe in `gui-isolated-from-the-real-daemon.md`, needed for the tray to
  register) is a way for `systemctl --user` to find the real manager. Do not run it to find out.
- If `systemctl` fails, the installed `proton-syncd` starts against the scratch config, with the
  developer's `proton-drive` and keyring session if the environment does not hide them.

**Precondition:** put a scratch directory first on `PATH` holding a `systemctl` that logs its arguments
and exits 1, and a `proton-syncd` that logs and exits 0, before launching the GUI. Check the logs
afterwards: an empty `systemctl` log is how you know the click did not reach it. Add `proton-drive`,
`secret-tool` and `xdg-open` stubs the same way if the run can reach them (`tests/common/mod.rs` does
this for the engine's tests, see `resolved-configs-carry-the-real-global-lock.md`).

For `cargo test -p proton-sync-gui`, the rule is the other note's: no test calls `start_service_impl`
(`gui-tests-that-shell-systemctl.md`).
