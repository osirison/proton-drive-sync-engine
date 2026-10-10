---
trigger: UPDATE_GOLDEN, never_synced_payloads, never-synced-payloads.json, npm run fidelity:pairs
depends_on: gui/test/never-synced-payloads.json, gui/src-tauri/src/commands/selection_tests.rs, gui/tools/fidelity/check-pair-routing.mjs
recorded: 2026-10-10
---

# `gui/test/never-synced-payloads.json` is written by a Rust test and replayed by a page gate

**Symptom:** `cargo test -p proton-sync-gui --lib never_synced_payloads` fails with "this build sends different
payloads than gui/test/never-synced-payloads.json holds", after a change to `gui_core::state` (a state, a rank, a
field of `PairState`) or to the fake daemon (`gui_core::testing`).

**What it is:** the status payloads the real command layer builds for three folders that have not finished a pass
(`documents`, `photos`, `videos`), at two moments — the default folder mid-pass (`running`) and nothing popped yet
(`starting`) — one payload per folder the reply describes. `fidelity:pairs` replays them into the real page
(`Bridge`'s `replay`), so the page scenarios about a folder that has not had its turn are shown what Rust says and
not a stand-in's guess at it.

**Fix, when the change is intended:**

```bash
UPDATE_GOLDEN=1 cargo test -p proton-sync-gui --lib never_synced_payloads
(cd gui && npx prettier --write test/never-synced-payloads.json)
git diff gui/test/never-synced-payloads.json        # read it: the diff is the wire change
(cd gui && npm run fidelity:pairs)                   # the page still says what it should
```

Without `UPDATE_GOLDEN` the test only compares, so a stale file is a failure and never a silent overwrite. The test also
asserts the facts the page scenarios rely on (which folder is `queued`, who it waits for), so a regeneration that lost
them cannot pass. Run it alone: it needs no daemon and no display, only the fake daemon over a temp socket.

**Why it was not obvious:** the file is JSON in `gui/test/`, next to the menu corpus (`tray-menu-corpus.json`, which is
hand-written and read by both languages), so it reads as a fixture to edit. It is generated; an edit by hand is
overwritten by the next regeneration and compared against the build's output in between.
